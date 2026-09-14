/*
 * GF2 (Girls' Frontline 2: Exilium) AssetBundle Decryption Module
 *
 * This module implements the XOR decryption algorithm from nijinekoyo/GF2AssetBundleDecryption,
 * used to decrypt game resource files in .bundle format.
 *
 * Decryption logic:
 * 1. XOR the first 16 bytes of the file with the static key to obtain the file-specific key (FileKey)
 * 2. Use FileKey to XOR the first 32KB of the file (or the whole file if smaller than 32KB)
 * 3. Output the fully decrypted file (unmodified portions remain unchanged)
 *
 * Warning: All operations are read -> decrypt -> write new file, never polluting the original file.
 */

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

use crate::common::bundle_file::bundle_file::BundleFile;
use crate::common::task::task_logger::TaskLogger;

// ============================================================
// Constants
// ============================================================

/// Static key (ASCII: "UnityFS\0\0\0\0\x07\x35\x2E\x78\x2E")
///
/// This is the magic signature of Unity AssetBundle, with GF2 adding an XOR mask on top.
/// First use it to extract the file-specific key, then use the file-specific key to decrypt data.
const STATIC_KEY: [u8; 16] = [
    0x55, 0x6E, 0x69, 0x74, 0x79, 0x46, 0x53, 0x00, // "UnityFS\0"
    0x00, 0x00, 0x00, 0x07, 0x35, 0x2E, 0x78, 0x2E, // "\0\0\0\x07\x35\x2E\x78\x2E"
];

/// Maximum bytes for XOR decryption (32KB = 0x8000 bytes).
///
/// GF2 only encrypts the first 32KB of AssetBundle data; the rest is plaintext.
/// This is a performance consideration - decrypting the entire large file is unnecessary.
const MAX_DECRYPT_SIZE: usize = 0x1000 * 8;

/// Length of file-specific key (fixed 16 bytes)
const KEY_LENGTH: usize = 16;

/// Result summary returned after decryption completes
#[derive(Clone, serde::Serialize)]
pub struct DecryptResult {
    /// Total number of .bundle files scanned
    pub total: usize,
    /// Number of successfully decrypted files
    pub success: usize,
    /// Number of failed files
    pub failed: usize,
    /// List of failed files (with paths, for user troubleshooting)
    pub failed_files: Vec<String>,
    /// Elapsed time (seconds)
    pub elapsed_secs: f64,
    /// Whether the task was cancelled by the global task system.
    pub cancelled: bool,
}

// ============================================================
// Bundle Decryption Service - wraps the XOR decryption algorithm as a utility class
// ============================================================

/**
 * Bundle decryption service class.
 *
 * Encapsulates all logic for GF2 AssetBundle XOR decryption, including:
 * - Core XOR decryption algorithm
 * - Recursive .bundle file collection
 * - Single file decryption and writing
 * - Concurrent batch decryption
 *
 * All methods are static (no self), because the decryptor holds no state
 * and is a pure algorithm utility class. Follows Soul.md rules: no free functions,
 * must be organized through struct + impl.
 */
pub struct DecryptionService;

impl DecryptionService {
    /**
     * XOR operation: byte-wise XOR on the first min(len(a), len(b)) bytes of two byte arrays.
     *
     * This is the most basic XOR operation, used in both steps of GF2 decryption.
     * Result length = min(len(data), len(key)).
     *
     * @param data  Input data
     * @param key   Key
     * @return      XOR result
     */
    fn xor_bytes(data: &[u8], key: &[u8]) -> Vec<u8> {
        let size = data.len().min(key.len());
        let mut result = Vec::with_capacity(size);
        for i in 0..size {
            result.push(data[i] ^ key[i]);
        }
        result
    }

    /**
     * Decrypt a single AssetBundle file's data.
     *
     * Algorithm steps:
     * 1. FileKey = xor_bytes(FileData[0..16], STATIC_KEY)
     * 2. Size = min(MAX_DECRYPT_SIZE, len(FileData))
     * 3. For i in 0..Size: FileData[i] ^= FileKey[i % 16]
     *
     * Note: The Go version does in-place modification,
     * we instead create a new Vec to return, avoiding accidental modification of input data.
     *
     * @param data  Original file data (will not be modified)
     * @return      Decrypted file data
     */
    pub fn decrypt_asset_bundle(data: &[u8]) -> Vec<u8> {
        /*
         * Step 1: Compute the file-specific key from the first 16 bytes of the file and the static key.
         * Each file's key is different because the first 16 bytes contain file-specific information.
         */
        let file_key = Self::xor_bytes(&data[..KEY_LENGTH.min(data.len())], &STATIC_KEY);

        /*
         * Step 2: Decrypt the first 32KB of data using the file-specific key.
         * The key is used cyclically (i % 16), similar to a Vigenere cipher.
         */
        let size = MAX_DECRYPT_SIZE.min(data.len());
        let mut decrypted = data.to_vec(); // Create a copy, don't pollute original data

        for i in 0..size {
            decrypted[i] ^= file_key[i % KEY_LENGTH];
        }

        decrypted
    }

    fn validate_decrypted_bundle(data: &[u8]) -> Result<(), String> {
        BundleFile::parse_filtered(data, |_| false)
            .map(|_| ())
            .map_err(|e| {
                format!(
                    "Decrypted output is not a readable UnityFS Bundle: {}. \
                     The input may use an unsupported encryption variant or may not be fully decrypted.",
                    e
                )
            })
    }

    /**
     * Recursively traverse a directory, collecting all file paths with the .bundle extension.
     *
     * Uses a breadth-first-style recursive traversal, first collecting files in the current directory,
     * then recursively processing subdirectories. Results are ordered by traversal order.
     *
     * Functionally overlaps with common::bundle_file::bundle_utils::BundleFileCollector::collect_bundle_files.
     * The latter additionally collects file metadata (size, modification time) for frontend display.
     * This method only returns a list of paths for the decryption flow.
     *
     * @param dir  The root directory to traverse
     * @return     Full paths of all .bundle files
     */
    fn collect_bundle_files_with_task(
        dir: &Path,
        task_id: &str,
        cancel_token: &Arc<AtomicBool>,
    ) -> Vec<PathBuf> {
        let mut bundle_files = Vec::new();
        TaskLogger::info(
            task_id,
            "Decrypt GF2",
            &format!("Scanning input directory: {}", dir.display()),
        );
        Self::collect_bundle_files_recursive_with_task(
            dir,
            &mut bundle_files,
            task_id,
            cancel_token,
        );
        bundle_files
    }

    /**
     * Recursive helper for collect_bundle_files.
     *
     * Reads directory entries, recurses for directories, adds .bundle files to the list.
     */
    fn collect_bundle_files_recursive_with_task(
        dir: &Path,
        files: &mut Vec<PathBuf>,
        task_id: &str,
        cancel_token: &Arc<AtomicBool>,
    ) {
        if cancel_token.load(Ordering::SeqCst) {
            return;
        }

        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                TaskLogger::warn(
                    task_id,
                    "Decrypt GF2",
                    &format!("Cannot read directory '{}': {}", dir.display(), e),
                );
                return;
            }
        };

        for entry in entries.flatten() {
            if cancel_token.load(Ordering::SeqCst) {
                return;
            }

            let path = entry.path();
            if path.is_dir() {
                Self::collect_bundle_files_recursive_with_task(&path, files, task_id, cancel_token);
            } else if path.extension().map_or(false, |ext| ext == "bundle") {
                files.push(path.clone());
                if files.len() % 5000 == 0 {
                    TaskLogger::info(
                        task_id,
                        "Decrypt GF2",
                        &format!(
                            "Found {} .bundle files, current: {}",
                            files.len(),
                            path.display()
                        ),
                    );
                }
            }
        }
    }

    /**
     * Read, decrypt, and write a single .bundle file.
     *
     * Full flow: read original file -> XOR decrypt -> write to output directory.
     * Output file retains the original file name (without directory structure).
     *
     * @param input_path   Full path of the original file
     * @param output_dir   Output directory
     * @return             Ok(bytes written) or Err(error message)
     */
    fn decrypt_bundle_file(input_path: &Path, output_dir: &Path) -> Result<u64, String> {
        let file_name = input_path
            .file_name()
            .ok_or_else(|| format!("Cannot get file name: {}", input_path.display()))?;
        let output_path = output_dir.join(file_name);
        if output_path.exists() {
            return Ok(0);
        }

        /*
         * Step 1: Read original file data into memory.
         * .bundle files are usually not large (a few MB to tens of MB), reading at once is fine.
         * If very large files are encountered in the future, this can be changed to streaming reads.
         */
        let raw_data = fs::read(input_path)
            .map_err(|e| format!("Cannot read file '{}': {}", input_path.display(), e))?;

        /*
         * Step 2: Perform XOR decryption.
         * Note: DecryptionService::decrypt_asset_bundle copies data, does not modify the original raw_data.
         */
        let decrypted_data = Self::decrypt_asset_bundle(&raw_data);
        Self::validate_decrypted_bundle(&decrypted_data)?;

        /*
         * Step 3: Construct the output file path and write.
         * The output file name is the same as the original file name (file name only, without directory structure).
         */
        /*
         * Ensure the output directory exists (in case of extreme situations like premature deletion).
         * Under normal circumstances, the outer layer has already created the directory; this is defensive programming.
         */
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "Cannot create output directory '{}': {}",
                    parent.display(),
                    e
                )
            })?;
        }

        /*
         * Write the decrypted data.
         * Use a buffered writer to improve large file write performance.
         */
        let mut writer = io::BufWriter::new(fs::File::create(&output_path).map_err(|e| {
            format!(
                "Cannot create output file '{}': {}",
                output_path.display(),
                e
            )
        })?);

        writer
            .write_all(&decrypted_data)
            .map_err(|e| format!("Failed to write file '{}': {}", output_path.display(), e))?;

        writer
            .flush()
            .map_err(|e| format!("Failed to flush buffer '{}': {}", output_path.display(), e))?;

        // Return bytes written == original file size (decryption does not change size)
        Ok(raw_data.len() as u64)
    }

    // ============================================================
    // Public Entry Methods
    // ============================================================

    /**
     * Decrypt all .bundle files in the specified directory.
     *
     * This is the top-level entry method called by Tauri commands, responsible for:
     * 1. Scanning the input directory to collect all .bundle files
     * 2. Ensuring the output directory exists
     * 3. Using a Semaphore to control concurrency
     * 4. Pushing real-time progress through the Tauri event system
     * 5. Collecting and returning result statistics
     *
     * @param progress_channel   Tauri IPC Channel, for streaming progress to the frontend
     * @param input_path         Input directory path (containing .bundle files)
     * @param output_path        Output directory path (where decrypted files are written)
     * @param max_concurrency    Maximum concurrency (at least 1)
     * @return                   Decryption result summary
     */
    pub fn decrypt_all_bundles(
        task_id: &str,
        cancel_token: &Arc<AtomicBool>,
        input_path: &str,
        output_path: &str,
        max_concurrency: usize,
    ) -> DecryptResult {
        let start_time = Instant::now();

        let input_dir = Path::new(input_path);
        let output_dir = Path::new(output_path);

        if !input_dir.is_dir() {
            let message = format!(
                "Input directory does not exist or is not readable: {}",
                input_path
            );
            TaskLogger::error(task_id, "Decrypt GF2", &message);
            return DecryptResult {
                total: 0,
                success: 0,
                failed: 1,
                failed_files: vec![message],
                elapsed_secs: start_time.elapsed().as_secs_f64(),
                cancelled: false,
            };
        }

        /*
         * Step 1: Ensure the output directory exists.
         * create_dir_all is idempotent, it won't error if the directory already exists.
         */
        if let Err(e) = fs::create_dir_all(output_dir) {
            TaskLogger::error(
                task_id,
                "Decrypt GF2",
                &format!("Cannot create output directory: {}", e),
            );
            return DecryptResult {
                total: 0,
                success: 0,
                failed: 1,
                failed_files: vec![format!("Cannot create output directory: {}", e)],
                elapsed_secs: start_time.elapsed().as_secs_f64(),
                cancelled: false,
            };
        }

        /*
         * Step 2: Scan the input directory, collect all .bundle files.
         */
        let bundle_files = Self::collect_bundle_files_with_task(input_dir, task_id, cancel_token);

        if cancel_token.load(Ordering::SeqCst) {
            TaskLogger::warn(
                task_id,
                "Decrypt GF2",
                &format!(
                    "Scan cancelled after finding {} .bundle files",
                    bundle_files.len()
                ),
            );
            return DecryptResult {
                total: bundle_files.len(),
                success: 0,
                failed: 0,
                failed_files: vec![],
                elapsed_secs: start_time.elapsed().as_secs_f64(),
                cancelled: true,
            };
        }

        if bundle_files.is_empty() {
            TaskLogger::warn(task_id, "Decrypt GF2", "No .bundle files found");
            return DecryptResult {
                total: 0,
                success: 0,
                failed: 0,
                failed_files: vec![],
                elapsed_secs: start_time.elapsed().as_secs_f64(),
                cancelled: false,
            };
        }

        let total = bundle_files.len();
        let concurrency = max_concurrency.max(1); // At least 1 concurrent worker
        TaskLogger::info(
            task_id,
            "Decrypt GF2",
            &format!("Scan complete, found {} .bundle files", total),
        );
        TaskLogger::progress(task_id, "Decrypt GF2", "Decrypt Files", 0, total, "Ready");

        /*
         * Step 3: Process all files concurrently - Dynamic Work Stealing pattern.
         *
         * Design:
         * - Create N worker threads (N = min(concurrency, total))
         * - All threads share a Mutex<usize> as the "next file index to process"
         * - Threads atomically acquire an index, process the corresponding file, then continue to get the next one
         * - When index >= total, the thread exits
         *
         * Advantages of this pattern:
         * 1. Natural load balancing - faster threads process more files, slower threads process fewer
         * 2. No semaphore needed - fixed thread count naturally controls concurrency
         * 3. Lock-free contention - only brief lock operations when acquiring an index
         * 4. Deadlock-free - no complex resource dependencies
         */
        /*
         * Convert shared variables to references, so the move closure only copies pointers (Copy),
         * without moving the original variable, allowing subsequent loop iterations to capture them normally.
         *
         * All references point to function-local variables whose lifetime covers the std::thread::scope range,
         * so cross-thread borrowing is safe.
         */
        let next_index = Mutex::new(0usize);
        let completed_count = AtomicUsize::new(0);
        let results: Mutex<Vec<(String, Result<u64, String>)>> =
            Mutex::new(Vec::with_capacity(total));

        let next_index_ref = &next_index;
        let completed_count_ref = &completed_count;
        let results_ref = &results;
        let bundle_files_ref = &bundle_files;

        /*
         * Determine the number of worker threads:
         * Must not exceed the total file count (otherwise there would be idle threads), but at least 1 thread.
         */
        let worker_count = concurrency.min(total).max(1);

        /*
         * std::thread::scope is a safe scoped thread introduced in Rust 1.63.
         * Threads within scope can borrow external variables (no 'static required),
         * and all threads are guaranteed to finish before scope returns, no manual join needed.
         *
         * Use move closures to transfer Channel ownership into the thread,
         * while shared references (&Mutex, &Vec, &Path) implement Copy, safe across threads.
         */
        std::thread::scope(|scope| {
            for _worker_id in 0..worker_count {
                scope.spawn(move || {
                    loop {
                        if cancel_token.load(Ordering::SeqCst) {
                            break;
                        }

                        /*
                         * Atomically acquire the index of the next file to process.
                         * This operation briefly locks the Mutex, but the duration is very short (just integer increment).
                         */
                        let current_idx = {
                            let mut guard = next_index_ref.lock().unwrap();
                            let idx = *guard;
                            *guard = idx + 1;
                            idx
                        };

                        // All files have been assigned, current thread exits
                        if current_idx >= total {
                            break;
                        }

                        let file_path = &bundle_files_ref[current_idx];
                        let file_name = file_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();

                        // Execute single file decryption
                        let result = Self::decrypt_bundle_file(file_path, output_dir);
                        let completed = completed_count_ref.fetch_add(1, Ordering::SeqCst) + 1;
                        match &result {
                            Ok(0) => {
                                TaskLogger::info(
                                    task_id,
                                    "Decrypt GF2",
                                    &format!("Skipped existing {}", file_name),
                                );
                                TaskLogger::progress(
                                    task_id,
                                    "Decrypt GF2",
                                    "Decrypt Files",
                                    completed,
                                    total,
                                    &format!("Skipped existing {}", file_name),
                                );
                            }
                            Ok(_) => {
                                TaskLogger::info(
                                    task_id,
                                    "Decrypt GF2",
                                    &format!("Decrypted {}", file_name),
                                );
                                TaskLogger::progress(
                                    task_id,
                                    "Decrypt GF2",
                                    "Decrypt Files",
                                    completed,
                                    total,
                                    &format!("Decrypted {}", file_name),
                                );
                            }
                            Err(error) => {
                                TaskLogger::warn(
                                    task_id,
                                    "Decrypt GF2",
                                    &format!("Failed {}: {}", file_name, error),
                                );
                                TaskLogger::progress(
                                    task_id,
                                    "Decrypt GF2",
                                    "Decrypt Files",
                                    completed,
                                    total,
                                    &format!("Failed {}", file_name),
                                );
                            }
                        }

                        // Collect result into shared list
                        let mut guard = results_ref.lock().unwrap();
                        guard.push((file_name, result));
                    }
                });
            }
        });
        // All threads have completed at this point

        /*
         * Step 4: Aggregate results and return.
         */
        let guard = results.lock().unwrap();
        let success_count = guard.iter().filter(|(_, r)| r.is_ok()).count();
        let skipped_count = guard.iter().filter(|(_, r)| matches!(r, Ok(0))).count();
        let failed_files: Vec<String> = guard
            .iter()
            .filter_map(
                |(name, r)| {
                    if r.is_err() {
                        Some(name.clone())
                    } else {
                        None
                    }
                },
            )
            .collect();

        let elapsed = start_time.elapsed().as_secs_f64();
        if cancel_token.load(Ordering::SeqCst) {
            TaskLogger::warn(
                task_id,
                "Decrypt GF2",
                &format!(
                    "Decrypt cancelled after {} of {} files ({} skipped)",
                    guard.len(),
                    total,
                    skipped_count
                ),
            );
        }

        DecryptResult {
            total,
            success: success_count,
            failed: failed_files.len(),
            failed_files,
            elapsed_secs: elapsed,
            cancelled: cancel_token.load(Ordering::SeqCst),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DecryptionService;

    #[test]
    fn rejects_malformed_unityfs_after_decryption() {
        let mut data = b"UnityFS\0\0\0\0\x075.x.".to_vec();
        data.extend_from_slice(&[0x49, 0xE7, 0x2F, 0x74, 0x43, 0x4A, 0x61, 0x69, 0xC9, 0xC2]);

        let err = DecryptionService::validate_decrypted_bundle(&data).unwrap_err();

        assert!(err.contains("not a readable UnityFS Bundle"));
        assert!(err.contains("Failed to read unity_version"));
    }
}
