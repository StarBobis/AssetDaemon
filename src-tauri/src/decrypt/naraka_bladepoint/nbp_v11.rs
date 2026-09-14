/*
 * Naraka: Bladepoint v11 decrypt/conversion module.
 *
 * Ports the two QuickBMS scripts used for Naraka resources:
 * - naraka_convert_v11.bms: converts obfuscated UnityFS bundles into normal UnityFS bundles.
 * - naraka_data_extract.bms: extracts compressed StreamingAssets containers.
 *
 * Inputs are never modified. All outputs are written under the selected output directory while
 * preserving the source relative path. Existing target files are skipped.
 */

use std::{
    fs,
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

use flate2::read::GzDecoder;

use crate::common::task::task_logger::TaskLogger;

const TASK_LABEL: &str = "Decrypt Naraka";
const UNITYFS_MAGIC: &[u8] = b"UnityFS";
const OBFUSCATED_MAGIC: [u8; 7] = [0x15, 0x1E, 0x1C, 0x0D, 0x0D, 0x23, 0x21];
const PAD: usize = 0x1000;
const INFO_DIR_NEW1: usize = 50;
const INFO_DIR_NEW2: usize = 64;
const EDIT_OFFSET: usize = 30;
const INFO_FLAGS_LZ4: u32 = 0x43;
const BLOCK_COMPRESSION_LZ4HC: u16 = 3;
const MAX_UNCOMPRESSED_BLOCKS_INFO: usize = 512 * 1024 * 1024;
const MAX_FILE_SIZE: u64 = usize::MAX as u64;
const DATA_GZIP_PACKED_LIST: u32 = 0x0005_0001;
const DATA_GZIP_CHUNK_LIST: u32 = 0x0001_0001;
const DATA_LZMA86_CHUNK_LIST: u32 = 0x0001_0003;
const MAX_LZ4_MATCH_COPY: usize = 1_000_000;

#[derive(Clone, serde::Serialize)]
pub struct DecryptResult {
    pub total: usize,
    pub success: usize,
    pub failed: usize,
    pub failed_files: Vec<String>,
    pub elapsed_secs: f64,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputKind {
    ObfuscatedBundle,
    DataContainer,
    Unsupported,
}

#[derive(Debug)]
struct BundleHeaderInfo {
    signature: Vec<u8>,
    size: u64,
    compressed_blocks_info_size: u32,
    uncompressed_blocks_info_size: u32,
    header_fields_offset: usize,
    after_header_offset: usize,
}

#[derive(Debug)]
struct BundleShiftPlan {
    new_variant: u8,
    compressed_shift: u32,
    uncompressed_shift: u32,
    metadata_offset: usize,
}

#[derive(Debug)]
struct ProcessOutcome {
    written: usize,
    skipped: usize,
}

pub struct NarakaBladepointDecryptionService;

impl NarakaBladepointDecryptionService {
    pub fn decrypt_all_bundles(
        task_id: &str,
        cancel_token: &Arc<AtomicBool>,
        input_path: &str,
        output_path: &str,
        _max_concurrency: usize,
    ) -> DecryptResult {
        let start_time = Instant::now();
        let input_dir = Path::new(input_path);
        let output_dir = Path::new(output_path);

        if !input_dir.is_dir() {
            let message = format!(
                "Input directory does not exist or is not readable: {}",
                input_path
            );
            TaskLogger::error(task_id, TASK_LABEL, &message);
            return Self::failed_result(message, start_time.elapsed().as_secs_f64());
        }

        if let Err(e) = fs::create_dir_all(output_dir) {
            let message = format!(
                "Cannot create output directory '{}': {}",
                output_dir.display(),
                e
            );
            TaskLogger::error(task_id, TASK_LABEL, &message);
            return Self::failed_result(message, start_time.elapsed().as_secs_f64());
        }

        TaskLogger::info(
            task_id,
            TASK_LABEL,
            &format!("Scanning input directory: {}", input_dir.display()),
        );
        let candidates = match Self::collect_candidate_files(input_dir, task_id, cancel_token) {
            Ok(files) => files,
            Err(e) => {
                TaskLogger::error(task_id, TASK_LABEL, &e);
                return Self::failed_result(e, start_time.elapsed().as_secs_f64());
            }
        };

        if cancel_token.load(Ordering::SeqCst) {
            TaskLogger::warn(
                task_id,
                TASK_LABEL,
                &format!(
                    "Scan cancelled after finding {} compatible files",
                    candidates.len()
                ),
            );
            return DecryptResult {
                total: candidates.len(),
                success: 0,
                failed: 0,
                failed_files: vec![],
                elapsed_secs: start_time.elapsed().as_secs_f64(),
                cancelled: true,
            };
        }

        if candidates.is_empty() {
            TaskLogger::warn(task_id, TASK_LABEL, "No compatible Naraka files found");
            return DecryptResult {
                total: 0,
                success: 0,
                failed: 0,
                failed_files: vec![],
                elapsed_secs: start_time.elapsed().as_secs_f64(),
                cancelled: false,
            };
        }

        let total = candidates.len();
        TaskLogger::info(
            task_id,
            TASK_LABEL,
            &format!("Scan complete, found {} compatible files", total),
        );
        TaskLogger::progress(task_id, TASK_LABEL, "Decrypt Files", 0, total, "Ready");

        let mut completed = 0usize;
        let mut success = 0usize;
        let mut failed_files = Vec::new();

        for file_path in candidates {
            if cancel_token.load(Ordering::SeqCst) {
                break;
            }

            completed += 1;
            let display_name = Self::display_relative_path(input_dir, &file_path);
            match Self::process_file(input_dir, output_dir, &file_path, task_id, cancel_token) {
                Ok(outcome) => {
                    success += 1;
                    if outcome.written == 0 && outcome.skipped > 0 {
                        TaskLogger::info(
                            task_id,
                            TASK_LABEL,
                            &format!("Skipped existing {}", display_name),
                        );
                        TaskLogger::progress(
                            task_id,
                            TASK_LABEL,
                            "Decrypt Files",
                            completed,
                            total,
                            &format!("Skipped existing {}", display_name),
                        );
                    } else {
                        TaskLogger::info(
                            task_id,
                            TASK_LABEL,
                            &format!(
                                "Processed {} ({} written, {} skipped)",
                                display_name, outcome.written, outcome.skipped
                            ),
                        );
                        TaskLogger::progress(
                            task_id,
                            TASK_LABEL,
                            "Decrypt Files",
                            completed,
                            total,
                            &format!("Processed {}", display_name),
                        );
                    }
                }
                Err(error) => {
                    let message = format!("Failed {}: {}", display_name, error);
                    TaskLogger::error(task_id, TASK_LABEL, &message);
                    failed_files.push(message);
                    return DecryptResult {
                        total,
                        success,
                        failed: failed_files.len(),
                        failed_files,
                        elapsed_secs: start_time.elapsed().as_secs_f64(),
                        cancelled: false,
                    };
                }
            }
        }

        let cancelled = cancel_token.load(Ordering::SeqCst);
        if cancelled {
            TaskLogger::warn(
                task_id,
                TASK_LABEL,
                &format!("Decrypt cancelled after {} of {} files", completed, total),
            );
        }

        DecryptResult {
            total,
            success,
            failed: failed_files.len(),
            failed_files,
            elapsed_secs: start_time.elapsed().as_secs_f64(),
            cancelled,
        }
    }

    fn failed_result(message: String, elapsed_secs: f64) -> DecryptResult {
        DecryptResult {
            total: 0,
            success: 0,
            failed: 1,
            failed_files: vec![message],
            elapsed_secs,
            cancelled: false,
        }
    }

    fn collect_candidate_files(
        dir: &Path,
        task_id: &str,
        cancel_token: &Arc<AtomicBool>,
    ) -> Result<Vec<PathBuf>, String> {
        let mut files = Vec::new();
        Self::collect_candidate_files_recursive(dir, &mut files, task_id, cancel_token)?;
        Ok(files)
    }

    fn collect_candidate_files_recursive(
        dir: &Path,
        files: &mut Vec<PathBuf>,
        task_id: &str,
        cancel_token: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        if cancel_token.load(Ordering::SeqCst) {
            return Ok(());
        }

        let entries = fs::read_dir(dir)
            .map_err(|e| format!("Cannot read directory '{}': {}", dir.display(), e))?;
        for entry in entries {
            if cancel_token.load(Ordering::SeqCst) {
                return Ok(());
            }

            let entry = entry
                .map_err(|e| format!("Cannot read directory entry '{}': {}", dir.display(), e))?;
            let path = entry.path();
            if path.is_dir() {
                Self::collect_candidate_files_recursive(&path, files, task_id, cancel_token)?;
            } else {
                match Self::detect_input_kind(&path)? {
                    InputKind::Unsupported => {}
                    _ => {
                        files.push(path.clone());
                        if files.len() % 5000 == 0 {
                            TaskLogger::info(
                                task_id,
                                TASK_LABEL,
                                &format!(
                                    "Found {} compatible files, current: {}",
                                    files.len(),
                                    path.display()
                                ),
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn detect_input_kind(path: &Path) -> Result<InputKind, String> {
        let mut file =
            fs::File::open(path).map_err(|e| format!("Cannot open '{}': {}", path.display(), e))?;
        let mut header = [0u8; 8];
        let read = file
            .read(&mut header)
            .map_err(|e| format!("Cannot read '{}': {}", path.display(), e))?;
        if read < 4 {
            return Ok(InputKind::Unsupported);
        }

        let first_u32 = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        if first_u32 == 0x151E1C0D {
            return Ok(InputKind::ObfuscatedBundle);
        }

        let first_u32_le = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        if matches!(
            first_u32_le,
            DATA_GZIP_PACKED_LIST | DATA_LZMA86_CHUNK_LIST | DATA_GZIP_CHUNK_LIST
        ) {
            return Ok(InputKind::DataContainer);
        }

        Ok(InputKind::Unsupported)
    }

    fn process_file(
        input_dir: &Path,
        output_dir: &Path,
        input_path: &Path,
        task_id: &str,
        cancel_token: &Arc<AtomicBool>,
    ) -> Result<ProcessOutcome, String> {
        match Self::detect_input_kind(input_path)? {
            InputKind::ObfuscatedBundle => {
                let output_path = Self::relative_output_path(input_dir, output_dir, input_path)?;
                if output_path.exists() {
                    return Ok(ProcessOutcome {
                        written: 0,
                        skipped: 1,
                    });
                }

                let data = Self::read_whole_file(input_path)?;
                let converted = Self::convert_bundle_v11(&data)?;
                Self::write_new_file(&output_path, &converted)?;
                Ok(ProcessOutcome {
                    written: 1,
                    skipped: 0,
                })
            }
            InputKind::DataContainer => Self::extract_data_container(
                input_dir,
                output_dir,
                input_path,
                task_id,
                cancel_token,
            ),
            InputKind::Unsupported => {
                Err("This file is not compatible with the Naraka scripts".to_string())
            }
        }
    }

    fn relative_output_path(
        input_dir: &Path,
        output_dir: &Path,
        input_path: &Path,
    ) -> Result<PathBuf, String> {
        let relative = input_path.strip_prefix(input_dir).map_err(|e| {
            format!(
                "Cannot compute relative path for '{}' from '{}': {}",
                input_path.display(),
                input_dir.display(),
                e
            )
        })?;
        Ok(output_dir.join(relative))
    }

    fn display_relative_path(input_dir: &Path, input_path: &Path) -> String {
        input_path
            .strip_prefix(input_dir)
            .unwrap_or(input_path)
            .display()
            .to_string()
    }

    fn read_whole_file(path: &Path) -> Result<Vec<u8>, String> {
        let metadata =
            fs::metadata(path).map_err(|e| format!("Cannot stat '{}': {}", path.display(), e))?;
        if metadata.len() > MAX_FILE_SIZE {
            return Err(format!(
                "File is too large to process on this platform: {} bytes",
                metadata.len()
            ));
        }
        fs::read(path).map_err(|e| format!("Cannot read file '{}': {}", path.display(), e))
    }

    fn write_new_file(path: &Path, data: &[u8]) -> Result<(), String> {
        if path.exists() {
            return Ok(());
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "Cannot create output directory '{}': {}",
                    parent.display(),
                    e
                )
            })?;
        }

        let mut writer = io::BufWriter::new(
            fs::File::create(path)
                .map_err(|e| format!("Cannot create output file '{}': {}", path.display(), e))?,
        );
        writer
            .write_all(data)
            .map_err(|e| format!("Failed to write file '{}': {}", path.display(), e))?;
        writer
            .flush()
            .map_err(|e| format!("Failed to flush file '{}': {}", path.display(), e))
    }

    fn convert_bundle_v11(data: &[u8]) -> Result<Vec<u8>, String> {
        let header = Self::read_bundle_header(data)?;
        if header.signature != OBFUSCATED_MAGIC {
            return Err(
                "This file is not compatible with the bundle conversion script".to_string(),
            );
        }

        let plan = Self::build_shift_plan(data, &header)?;
        match plan.new_variant {
            0 => Self::convert_old_bundle(data, &header, &plan),
            1 => Self::convert_new1_bundle(data, &header, &plan),
            2 => Self::convert_new2_bundle(data, &header, &plan),
            _ => Err(format!(
                "Unsupported Naraka conversion variant {}",
                plan.new_variant
            )),
        }
    }

    fn read_bundle_header(data: &[u8]) -> Result<BundleHeaderInfo, String> {
        let mut offset = 0usize;
        let signature = Self::read_c_string_bytes(data, &mut offset, 32)?;
        let _version = Self::read_u32_be_at(data, offset)?;
        offset += 4;
        let _unity_version = Self::read_c_string_bytes(data, &mut offset, 256)?;
        let _unity_revision = Self::read_c_string_bytes(data, &mut offset, 256)?;
        let size = Self::read_u64_be_at(data, offset)?;
        let header_fields_offset = offset;
        offset += 8;
        let compressed_blocks_info_size = Self::read_u32_be_at(data, offset)?;
        offset += 4;
        let uncompressed_blocks_info_size = Self::read_u32_be_at(data, offset)?;
        offset += 4;
        let _flags = Self::read_u32_be_at(data, offset)?;
        offset += 4;

        Ok(BundleHeaderInfo {
            signature,
            size,
            compressed_blocks_info_size,
            uncompressed_blocks_info_size,
            header_fields_offset,
            after_header_offset: offset,
        })
    }

    fn build_shift_plan(data: &[u8], header: &BundleHeaderInfo) -> Result<BundleShiftPlan, String> {
        let file_size = data.len() as i128;
        let check2 = header.size as i128 - file_size;
        let mut new_variant = 0u8;
        let mut metadata_offset = header.after_header_offset;
        let (mut compressed_shift, mut uncompressed_shift) = if check2 == 0 {
            return Err("This bundle is not compatible with the script".to_string());
        } else if check2 == 0x1A {
            (180, 170)
        } else if check2 == 0x16 {
            (202, 202)
        } else {
            new_variant = 1;
            let ver = Self::read_u16_be_at(data, header.after_header_offset)?;
            metadata_offset = header.after_header_offset + 2;
            match ver {
                0x9 => (164, 156),
                0xF => (170, 190),
                0x14 => (240, 230),
                0x32 => {
                    let check3 = Self::read_u32_be_at(data, 0x40)?;
                    if check3 == 0 {
                        new_variant = 2;
                        (0, 0)
                    } else {
                        (150, 130)
                    }
                }
                _ => {
                    new_variant = 2;
                    (0, 0)
                }
            }
        };

        if new_variant == 2 {
            compressed_shift = 255;
            uncompressed_shift = 0;
            if header
                .compressed_blocks_info_size
                .saturating_sub(compressed_shift)
                < 32
            {
                compressed_shift = header.compressed_blocks_info_size.saturating_sub(32);
            }
        }

        Ok(BundleShiftPlan {
            new_variant,
            compressed_shift,
            uncompressed_shift,
            metadata_offset,
        })
    }

    fn convert_old_bundle(
        data: &[u8],
        header: &BundleHeaderInfo,
        plan: &BundleShiftPlan,
    ) -> Result<Vec<u8>, String> {
        let compressed_size = header
            .compressed_blocks_info_size
            .checked_sub(plan.compressed_shift)
            .ok_or_else(|| "Compressed BlocksInfo shift is larger than source value".to_string())?;
        let uncompressed_size = header
            .uncompressed_blocks_info_size
            .checked_sub(plan.uncompressed_shift)
            .ok_or_else(|| {
                "Uncompressed BlocksInfo shift is larger than source value".to_string()
            })?;

        let mut output = data.to_vec();
        Self::replace_magic_with_unityfs(&mut output)?;
        Self::write_u32_be(
            &mut output,
            header.header_fields_offset + 8,
            compressed_size,
        )?;
        Self::write_u32_be(
            &mut output,
            header.header_fields_offset + 12,
            uncompressed_size,
        )?;
        Ok(output)
    }

    fn convert_new1_bundle(
        data: &[u8],
        header: &BundleHeaderInfo,
        plan: &BundleShiftPlan,
    ) -> Result<Vec<u8>, String> {
        let compressed_size = header
            .compressed_blocks_info_size
            .checked_sub(plan.compressed_shift)
            .ok_or_else(|| "Compressed BlocksInfo shift is larger than source value".to_string())?
            as usize;
        let uncompressed_size = header
            .uncompressed_blocks_info_size
            .checked_sub(plan.uncompressed_shift)
            .ok_or_else(|| {
                "Uncompressed BlocksInfo shift is larger than source value".to_string()
            })? as usize;
        let metadata = Self::slice_checked(data, plan.metadata_offset, compressed_size)?;
        let blocks_info = Self::decompress_lz4(metadata, uncompressed_size)?;
        if blocks_info.len() != uncompressed_size {
            return Err(format!(
                "NEW=1 BlocksInfo size mismatch: got {} expected {}",
                blocks_info.len(),
                uncompressed_size
            ));
        }
        let patched_blocks_info = Self::patch_block_flags(blocks_info)?;
        let compressed_blocks_info = lz4_flex::block::compress(&patched_blocks_info);

        let data_offset = plan
            .metadata_offset
            .checked_add(compressed_size)
            .ok_or_else(|| "Data offset overflow".to_string())?;
        let body = Self::slice_checked(data, data_offset, data.len().saturating_sub(data_offset))?;

        let mut output =
            Vec::with_capacity(INFO_DIR_NEW1 + compressed_blocks_info.len() + body.len());
        output.extend_from_slice(Self::slice_checked(data, 0, INFO_DIR_NEW1)?);
        output.extend_from_slice(&compressed_blocks_info);
        output.extend_from_slice(body);

        Self::replace_magic_with_unityfs(&mut output)?;
        let file_size = output.len() as u64;
        Self::write_u64_be(&mut output, EDIT_OFFSET, file_size)?;
        Self::write_u32_be(
            &mut output,
            EDIT_OFFSET + 8,
            compressed_blocks_info.len() as u32,
        )?;
        Self::write_u32_be(
            &mut output,
            EDIT_OFFSET + 12,
            patched_blocks_info.len() as u32,
        )?;
        Self::write_u32_be(&mut output, EDIT_OFFSET + 16, INFO_FLAGS_LZ4)?;

        Ok(output)
    }

    fn convert_new2_bundle(
        data: &[u8],
        header: &BundleHeaderInfo,
        plan: &BundleShiftPlan,
    ) -> Result<Vec<u8>, String> {
        let shifted_compressed_size = header
            .compressed_blocks_info_size
            .checked_sub(plan.compressed_shift)
            .ok_or_else(|| "Compressed BlocksInfo shift is larger than source value".to_string())?
            as usize;
        let metadata_offset = Self::align_up(plan.metadata_offset, PAD);
        let first_test = metadata_offset
            .checked_add(shifted_compressed_size)
            .ok_or_else(|| "First metadata test offset overflow".to_string())?;
        let test_offset = Self::align_up(first_test, PAD).saturating_sub(3);

        let scan_start = if Self::read_three_at(data, test_offset)? != 0 {
            test_offset
                .checked_add(PAD)
                .ok_or_else(|| "NEW=2 marker scan offset overflow".to_string())?
        } else {
            test_offset
        };
        let metadata_end = Self::find_new2_metadata_end(data, metadata_offset, scan_start)?;
        let compressed_size = metadata_end
            .checked_sub(metadata_offset)
            .ok_or_else(|| "Computed metadata size underflow".to_string())?;
        let guessed_uncompressed = compressed_size
            .checked_mul(100)
            .ok_or_else(|| "Guessed BlocksInfo size overflow".to_string())?;
        if guessed_uncompressed > MAX_UNCOMPRESSED_BLOCKS_INFO {
            return Err(format!(
                "Guessed BlocksInfo size is too large: {} bytes",
                guessed_uncompressed
            ));
        }

        let metadata = Self::slice_checked(data, metadata_offset, compressed_size)?;
        let blocks_info = Self::decompress_lz4(metadata, guessed_uncompressed)?;
        let patched_blocks_info = Self::patch_block_flags(blocks_info)?;
        let blocks = Self::read_block_entries(&patched_blocks_info)?;

        let mut data_offset = metadata_end;
        while data_offset < data.len() && data[data_offset] == 0xFF {
            data_offset += 1;
        }
        data_offset = Self::align_up(data_offset, PAD);

        let block_count = blocks.len();
        let mut body = Vec::new();
        for (index, block) in blocks.iter().enumerate() {
            let block_size = block.compressed_size as usize;
            let block_end = data_offset
                .checked_add(block_size)
                .ok_or_else(|| "Data block offset overflow".to_string())?;
            if block_end <= data.len() {
                body.extend_from_slice(Self::slice_checked(data, data_offset, block_size)?);
            } else if index + 1 == block_count && data_offset <= data.len() {
                let available = data.len() - data_offset;
                body.extend_from_slice(Self::slice_checked(data, data_offset, available)?);
                body.resize(body.len() + (block_size - available), 0);
            } else {
                return Err(format!(
                    "Data block exceeds file size: block={}/{}, offset={}, len={}, file_size={}",
                    index + 1,
                    block_count,
                    data_offset,
                    block_size,
                    data.len()
                ));
            }
            data_offset = block_end;
            data_offset = Self::align_up(data_offset, PAD);
        }

        let compressed_blocks_info = lz4_flex::block::compress(&patched_blocks_info);
        let mut output =
            Vec::with_capacity(INFO_DIR_NEW2 + compressed_blocks_info.len() + body.len());
        output.extend_from_slice(Self::slice_checked(data, 0, INFO_DIR_NEW2)?);
        output.extend_from_slice(&compressed_blocks_info);
        output.extend_from_slice(&body);

        Self::replace_magic_with_unityfs(&mut output)?;
        let file_size = output.len() as u64;
        Self::write_u64_be(&mut output, EDIT_OFFSET, file_size)?;
        Self::write_u32_be(
            &mut output,
            EDIT_OFFSET + 8,
            compressed_blocks_info.len() as u32,
        )?;
        Self::write_u32_be(
            &mut output,
            EDIT_OFFSET + 12,
            patched_blocks_info.len() as u32,
        )?;
        Self::write_u32_be(&mut output, EDIT_OFFSET + 16, INFO_FLAGS_LZ4)?;
        Self::write_u16_be(&mut output, INFO_DIR_NEW1, 0)?;

        Ok(output)
    }

    fn find_new2_metadata_end(
        data: &[u8],
        metadata_offset: usize,
        scan_start: usize,
    ) -> Result<usize, String> {
        let mut candidates = Vec::new();
        let mut pos = scan_start;
        loop {
            let check = Self::read_three_at(data, pos)?;
            if matches!(check, 0x100 | 0x5000 | 0x726573 | 0xF0100) {
                let skip = match check {
                    0x100 | 0xF0100 => 4,
                    0x5000 => 2,
                    0x726573 => 8,
                    _ => 0,
                };
                let cursor_after_bms_backstep = pos;
                candidates.push(
                    cursor_after_bms_backstep
                        .checked_add(skip)
                        .ok_or_else(|| "Metadata end offset overflow".to_string())?,
                );
            }

            if pos == metadata_offset || pos == 0 {
                break;
            }
            pos -= 1;
        }

        candidates.extend(Self::find_new2_cab_candidates(
            data,
            metadata_offset,
            scan_start.saturating_add(PAD).min(data.len()),
        ));
        candidates.sort_unstable();
        candidates.dedup();

        for candidate in candidates.iter().copied() {
            if candidate <= metadata_offset || candidate > data.len() {
                continue;
            }
            let compressed_size = candidate - metadata_offset;
            let guessed_uncompressed = match compressed_size.checked_mul(100) {
                Some(value) if value <= MAX_UNCOMPRESSED_BLOCKS_INFO => value,
                _ => continue,
            };
            let Ok(decoded) =
                Self::decompress_lz4(&data[metadata_offset..candidate], guessed_uncompressed)
            else {
                continue;
            };
            if Self::is_complete_blocks_info(&decoded) {
                return Ok(candidate);
            }
        }

        Err("Cannot locate valid v11 BlocksInfo data".to_string())
    }

    fn find_new2_cab_candidates(data: &[u8], start: usize, end: usize) -> Vec<usize> {
        if start >= end || end > data.len() {
            return Vec::new();
        }

        let mut candidates = Vec::new();
        let mut pos = start;
        while pos + 4 <= end {
            if &data[pos..pos + 4] == b"CAB-" {
                let window_start = pos.saturating_sub(128).max(start + 1);
                candidates.extend(window_start..=pos);
            }
            pos += 1;
        }
        candidates
    }

    fn patch_block_flags(mut blocks_info: Vec<u8>) -> Result<Vec<u8>, String> {
        let mut cursor = BlocksInfoCursor::new(&blocks_info);
        cursor.skip(16)?;
        let block_count = cursor.read_u32_be()? as usize;
        if block_count > 200_000 {
            return Err(format!(
                "Abnormal block count in BlocksInfo: {}",
                block_count
            ));
        }

        let mut flag_offsets = Vec::with_capacity(block_count);
        for _ in 0..block_count {
            cursor.skip(8)?;
            let flag_offset = cursor.position();
            let flags = cursor.read_u16_be()?;
            if flags == 6 {
                flag_offsets.push(flag_offset);
            }
        }

        for offset in flag_offsets {
            Self::write_u16_be(&mut blocks_info, offset, BLOCK_COMPRESSION_LZ4HC)?;
        }
        Ok(blocks_info)
    }

    fn read_block_entries(blocks_info: &[u8]) -> Result<Vec<BlockEntry>, String> {
        let mut cursor = BlocksInfoCursor::new(blocks_info);
        cursor.skip(16)?;
        let block_count = cursor.read_u32_be()? as usize;
        if block_count > 200_000 {
            return Err(format!(
                "Abnormal block count in BlocksInfo: {}",
                block_count
            ));
        }

        let mut blocks = Vec::with_capacity(block_count);
        for _ in 0..block_count {
            let _uncompressed_size = cursor.read_u32_be()?;
            let compressed_size = cursor.read_u32_be()?;
            let _flags = cursor.read_u16_be()?;
            blocks.push(BlockEntry { compressed_size });
        }
        Ok(blocks)
    }

    fn extract_data_container(
        input_dir: &Path,
        output_dir: &Path,
        input_path: &Path,
        task_id: &str,
        cancel_token: &Arc<AtomicBool>,
    ) -> Result<ProcessOutcome, String> {
        let data = Self::read_whole_file(input_path)?;
        let mut offset = 0usize;
        let check = Self::read_u32_le_from(data.as_slice(), &mut offset)?;
        let relative = input_path.strip_prefix(input_dir).map_err(|e| {
            format!(
                "Cannot compute relative path for '{}' from '{}': {}",
                input_path.display(),
                input_dir.display(),
                e
            )
        })?;
        let file_stem = input_path
            .file_stem()
            .ok_or_else(|| format!("Cannot get file stem: {}", input_path.display()))?;
        let base_output_dir = output_dir
            .join(relative.parent().unwrap_or_else(|| Path::new("")))
            .join(format!("{}_extracted", file_stem.to_string_lossy()));

        let mut written = 0usize;
        let mut skipped = 0usize;

        match check {
            DATA_GZIP_PACKED_LIST => {
                let files = Self::read_u16_le_from(data.as_slice(), &mut offset)? as usize;
                TaskLogger::info(
                    task_id,
                    TASK_LABEL,
                    &format!(
                        "Extracting container {} ({} entries)",
                        input_path.display(),
                        files
                    ),
                );
                for i in 0..files {
                    if cancel_token.load(Ordering::SeqCst) {
                        break;
                    }
                    let _size1 = Self::read_u16_le_from(data.as_slice(), &mut offset)?;
                    let _size2 = Self::read_u16_le_from(data.as_slice(), &mut offset)?;
                    let _chunk = Self::read_u32_le_from(data.as_slice(), &mut offset)?;
                    let entry_offset =
                        Self::read_u32_le_from(data.as_slice(), &mut offset)? as usize;
                    let zsize = Self::read_u32_le_from(data.as_slice(), &mut offset)? as usize;
                    let size = Self::read_u32_le_from(data.as_slice(), &mut offset)? as usize;
                    let output_path = base_output_dir.join(format!("{:08}", i));
                    if output_path.exists() {
                        skipped += 1;
                        continue;
                    }
                    let compressed = Self::slice_checked(&data, entry_offset, zsize)?;
                    let decompressed = Self::decompress_gzip(compressed, size)?;
                    Self::write_new_file(&output_path, &decompressed)?;
                    written += 1;
                    Self::log_container_progress(
                        task_id,
                        input_path,
                        i + 1,
                        files,
                        written,
                        skipped,
                    );
                }
            }
            DATA_GZIP_CHUNK_LIST | DATA_LZMA86_CHUNK_LIST => {
                let files = Self::read_u32_le_from(data.as_slice(), &mut offset)? as usize;
                TaskLogger::info(
                    task_id,
                    TASK_LABEL,
                    &format!(
                        "Extracting container {} ({} entries)",
                        input_path.display(),
                        files
                    ),
                );
                for i in 0..files {
                    if cancel_token.load(Ordering::SeqCst) {
                        break;
                    }
                    let head = Self::read_u32_le_from(data.as_slice(), &mut offset)?;
                    let _chunk = head & 0xffff;
                    let _size1 = Self::read_u32_le_from(data.as_slice(), &mut offset)?;
                    let _size2 = Self::read_u32_le_from(data.as_slice(), &mut offset)?;
                    let entry_offset =
                        Self::read_u32_le_from(data.as_slice(), &mut offset)? as usize;
                    let size = Self::read_u32_le_from(data.as_slice(), &mut offset)? as usize;
                    let zsize = Self::read_u32_le_from(data.as_slice(), &mut offset)? as usize;
                    let output_path = base_output_dir.join(format!("{:08}", i));
                    if output_path.exists() {
                        skipped += 1;
                        continue;
                    }
                    let compressed = Self::slice_checked(&data, entry_offset, zsize)?;
                    let decompressed = if check == DATA_GZIP_CHUNK_LIST {
                        Self::decompress_gzip(compressed, size)?
                    } else {
                        Self::decompress_lzma_with_header(compressed, size)?
                    };
                    Self::write_new_file(&output_path, &decompressed)?;
                    written += 1;
                    Self::log_container_progress(
                        task_id,
                        input_path,
                        i + 1,
                        files,
                        written,
                        skipped,
                    );
                }
            }
            _ => return Err("This is not compatible file".to_string()),
        }

        Ok(ProcessOutcome { written, skipped })
    }

    fn log_container_progress(
        task_id: &str,
        input_path: &Path,
        current: usize,
        total: usize,
        written: usize,
        skipped: usize,
    ) {
        if current == total || current % 16 == 0 {
            TaskLogger::info(
                task_id,
                TASK_LABEL,
                &format!(
                    "Extracted {}/{} entries from {} ({} written, {} skipped)",
                    current,
                    total,
                    input_path.display(),
                    written,
                    skipped
                ),
            );
        }
    }

    fn decompress_lz4(data: &[u8], expected_size: usize) -> Result<Vec<u8>, String> {
        let mut output = vec![0u8; expected_size];
        match lz4_flex::block::decompress_into(data, &mut output) {
            Ok(written) => {
                output.truncate(written);
                Ok(output)
            }
            Err(strict_error) => Self::decompress_lz4_lenient(data, expected_size).map_err(|e| {
                format!(
                    "LZ4 decompression failed: {}; lenient fallback failed: {}",
                    strict_error, e
                )
            }),
        }
    }

    fn decompress_lz4_lenient(data: &[u8], output_limit: usize) -> Result<Vec<u8>, String> {
        let mut input_pos = 0usize;
        let mut output = Vec::new();

        while input_pos < data.len() {
            let token = data[input_pos];
            input_pos += 1;

            let literal_len = Self::read_lz4_len(data, &mut input_pos, (token >> 4) as usize)?;
            let literal_end = input_pos
                .checked_add(literal_len)
                .ok_or_else(|| "LZ4 literal length overflow".to_string())?;
            if literal_end > data.len() {
                return Err("LZ4 literal exceeds input".to_string());
            }
            if output.len().saturating_add(literal_len) > output_limit {
                return Err("LZ4 literal output exceeded safety limit".to_string());
            }
            output.extend_from_slice(&data[input_pos..literal_end]);
            input_pos = literal_end;

            if input_pos >= data.len() {
                break;
            }
            if input_pos + 2 > data.len() {
                if Self::is_complete_blocks_info(&output) {
                    break;
                }
                return Err("LZ4 match offset is truncated".to_string());
            }
            let offset = u16::from_le_bytes([data[input_pos], data[input_pos + 1]]) as usize;
            input_pos += 2;
            if offset == 0 || offset > output.len() {
                if Self::is_complete_blocks_info(&output) {
                    break;
                }
                return Err(format!("Invalid LZ4 match offset: {}", offset));
            }

            let match_len = match Self::read_lz4_len(data, &mut input_pos, (token & 0x0F) as usize)
            {
                Ok(len) => len + 4,
                Err(_) if input_pos >= data.len() || Self::is_complete_blocks_info(&output) => {
                    break
                }
                Err(e) => return Err(e),
            };
            if match_len > MAX_LZ4_MATCH_COPY {
                return Err(format!(
                    "LZ4 match length {} exceeds safety limit",
                    match_len
                ));
            }
            if output.len().saturating_add(match_len) > output_limit {
                return Err("LZ4 match output exceeded safety limit".to_string());
            }

            for _ in 0..match_len {
                let src = output.len() - offset;
                output.push(output[src]);
            }
        }

        Ok(output)
    }

    fn is_complete_blocks_info(data: &[u8]) -> bool {
        if data.len() < 20 {
            return false;
        }

        let mut cursor = BlocksInfoCursor::new(data);
        if cursor.skip(16).is_err() {
            return false;
        }

        let Ok(block_count) = cursor.read_u32_be() else {
            return false;
        };
        if block_count > 200_000 {
            return false;
        }
        for _ in 0..block_count {
            if cursor.skip(10).is_err() {
                return false;
            }
        }

        let Ok(node_count) = cursor.read_u32_be() else {
            return false;
        };
        if node_count > 100_000 {
            return false;
        }
        for _ in 0..node_count {
            if cursor.skip(20).is_err() {
                return false;
            }
            if cursor.read_c_string(1024).is_err() {
                return false;
            }
        }

        true
    }

    fn read_lz4_len(data: &[u8], input_pos: &mut usize, base: usize) -> Result<usize, String> {
        let mut len = base;
        if len == 15 {
            loop {
                if *input_pos >= data.len() {
                    return Err("LZ4 length extension is truncated".to_string());
                }
                let extra = data[*input_pos] as usize;
                *input_pos += 1;
                len = len
                    .checked_add(extra)
                    .ok_or_else(|| "LZ4 length overflow".to_string())?;
                if extra != 255 {
                    break;
                }
            }
        }
        Ok(len)
    }

    fn decompress_gzip(data: &[u8], expected_size: usize) -> Result<Vec<u8>, String> {
        let mut decoder = GzDecoder::new(data);
        let mut output = Vec::with_capacity(expected_size);
        decoder
            .read_to_end(&mut output)
            .map_err(|e| format!("GZip decompression failed: {}", e))?;
        if output.len() != expected_size {
            return Err(format!(
                "GZip decompressed size mismatch: wrote {} bytes but expected {} bytes",
                output.len(),
                expected_size
            ));
        }
        Ok(output)
    }

    fn decompress_lzma_with_header(data: &[u8], expected_size: usize) -> Result<Vec<u8>, String> {
        use lzma_rs::decompress::raw::{LzmaDecoder, LzmaParams, LzmaProperties};

        if data.len() < 13 {
            return Err("LZMA86 chunk header is truncated".to_string());
        }
        let props = data[0] as u32;
        if props >= 225 {
            return Err(format!("Invalid LZMA86 properties byte: {}", props));
        }

        let lc = props % 9;
        let lp = (props / 9) % 5;
        let pb = props / 45;
        let dict_size = Self::read_u32_le_at(data, 1)?.max(0x1000);
        let size_a = Self::read_u32_le_at(data, 5)?;
        let size_b = Self::read_u32_le_at(data, 9)?;
        let unpacked_size = if size_a == size_b {
            size_a as u64
        } else {
            Self::read_u64_le_at(data, 5)?
        };
        if unpacked_size != expected_size as u64 {
            return Err(format!(
                "LZMA86 header size mismatch: {} expected {}",
                unpacked_size, expected_size
            ));
        }

        let params = LzmaParams::new(
            LzmaProperties { lc, lp, pb },
            dict_size,
            Some(expected_size as u64),
        );
        let mut decoder = LzmaDecoder::new(params, None)
            .map_err(|e| format!("LZMA86 decoder setup failed: {}", e))?;
        let mut input = Cursor::new(&data[13..]);
        let mut output = Vec::with_capacity(expected_size);
        decoder
            .decompress(&mut input, &mut output)
            .map_err(|e| format!("LZMA86 decompression failed: {}", e))?;
        if output.len() != expected_size {
            return Err(format!(
                "LZMA86 decompressed size mismatch: wrote {} bytes but expected {} bytes",
                output.len(),
                expected_size
            ));
        }
        Ok(output)
    }

    fn replace_magic_with_unityfs(data: &mut [u8]) -> Result<(), String> {
        if data.len() < 7 {
            return Err("File is too small to patch UnityFS magic".to_string());
        }
        data[..UNITYFS_MAGIC.len()].copy_from_slice(UNITYFS_MAGIC);
        data[UNITYFS_MAGIC.len()] = 0;
        Ok(())
    }

    fn read_c_string_bytes(
        data: &[u8],
        offset: &mut usize,
        max_len: usize,
    ) -> Result<Vec<u8>, String> {
        let start = *offset;
        let limit = start
            .checked_add(max_len)
            .ok_or_else(|| "CString limit overflow".to_string())?
            .min(data.len());
        for pos in start..limit {
            if data[pos] == 0 {
                *offset = pos + 1;
                return Ok(data[start..pos].to_vec());
            }
        }

        Err(format!(
            "CString exceeded {} bytes without null terminator at offset {}",
            max_len, start
        ))
    }

    fn read_u16_be_at(data: &[u8], offset: usize) -> Result<u16, String> {
        let bytes = Self::slice_checked(data, offset, 2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn read_u32_be_at(data: &[u8], offset: usize) -> Result<u32, String> {
        let bytes = Self::slice_checked(data, offset, 4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_u64_be_at(data: &[u8], offset: usize) -> Result<u64, String> {
        let bytes = Self::slice_checked(data, offset, 8)?;
        Ok(u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn read_three_at(data: &[u8], offset: usize) -> Result<u32, String> {
        let bytes = Self::slice_checked(data, offset, 3)?;
        Ok(((bytes[0] as u32) << 16) | ((bytes[1] as u32) << 8) | bytes[2] as u32)
    }

    fn read_u16_le_at(data: &[u8], offset: usize) -> Result<u16, String> {
        let bytes = Self::slice_checked(data, offset, 2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn read_u32_le_at(data: &[u8], offset: usize) -> Result<u32, String> {
        let bytes = Self::slice_checked(data, offset, 4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_u64_le_at(data: &[u8], offset: usize) -> Result<u64, String> {
        let bytes = Self::slice_checked(data, offset, 8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn read_u16_le_from(data: &[u8], offset: &mut usize) -> Result<u16, String> {
        let value = Self::read_u16_le_at(data, *offset)?;
        *offset += 2;
        Ok(value)
    }

    fn read_u32_le_from(data: &[u8], offset: &mut usize) -> Result<u32, String> {
        let value = Self::read_u32_le_at(data, *offset)?;
        *offset += 4;
        Ok(value)
    }

    fn write_u16_be(data: &mut [u8], offset: usize, value: u16) -> Result<(), String> {
        let target = Self::slice_checked_mut(data, offset, 2)?;
        target.copy_from_slice(&value.to_be_bytes());
        Ok(())
    }

    fn write_u32_be(data: &mut [u8], offset: usize, value: u32) -> Result<(), String> {
        let target = Self::slice_checked_mut(data, offset, 4)?;
        target.copy_from_slice(&value.to_be_bytes());
        Ok(())
    }

    fn write_u64_be(data: &mut [u8], offset: usize, value: u64) -> Result<(), String> {
        let target = Self::slice_checked_mut(data, offset, 8)?;
        target.copy_from_slice(&value.to_be_bytes());
        Ok(())
    }

    fn slice_checked(data: &[u8], offset: usize, len: usize) -> Result<&[u8], String> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| "Slice offset overflow".to_string())?;
        if end > data.len() {
            return Err(format!(
                "Slice out of bounds: offset={}, len={}, file_size={}",
                offset,
                len,
                data.len()
            ));
        }
        Ok(&data[offset..end])
    }

    fn slice_checked_mut(data: &mut [u8], offset: usize, len: usize) -> Result<&mut [u8], String> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| "Mutable slice offset overflow".to_string())?;
        if end > data.len() {
            return Err(format!(
                "Mutable slice out of bounds: offset={}, len={}, file_size={}",
                offset,
                len,
                data.len()
            ));
        }
        Ok(&mut data[offset..end])
    }

    fn align_up(value: usize, alignment: usize) -> usize {
        (value + alignment - 1) & !(alignment - 1)
    }
}

#[derive(Debug)]
struct BlockEntry {
    compressed_size: u32,
}

struct BlocksInfoCursor<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> BlocksInfoCursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn position(&self) -> usize {
        self.offset
    }

    fn skip(&mut self, count: usize) -> Result<(), String> {
        self.offset = self
            .offset
            .checked_add(count)
            .ok_or_else(|| "BlocksInfo cursor overflow".to_string())?;
        if self.offset > self.data.len() {
            return Err(format!(
                "BlocksInfo cursor out of bounds: offset={}, size={}",
                self.offset,
                self.data.len()
            ));
        }
        Ok(())
    }

    fn read_u16_be(&mut self) -> Result<u16, String> {
        let bytes = NarakaBladepointDecryptionService::slice_checked(self.data, self.offset, 2)?;
        self.offset += 2;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn read_u32_be(&mut self) -> Result<u32, String> {
        let bytes = NarakaBladepointDecryptionService::slice_checked(self.data, self.offset, 4)?;
        self.offset += 4;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_c_string(&mut self, max_len: usize) -> Result<(), String> {
        let start = self.offset;
        let limit = start.saturating_add(max_len).min(self.data.len());
        for pos in start..limit {
            if self.data[pos] == 0 {
                self.offset = pos + 1;
                return Ok(());
            }
        }
        Err("BlocksInfo string terminator was not found".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::NarakaBladepointDecryptionService;
    use crate::common::bundle_file::bundle_file::BundleFile;
    use std::path::Path;

    #[test]
    fn patches_naraka_magic_to_unityfs() {
        let mut data = vec![0x15, 0x1E, 0x1C, 0x0D, 0x0D, 0x23, 0x21, 1, 2, 3];

        NarakaBladepointDecryptionService::replace_magic_with_unityfs(&mut data).unwrap();

        assert_eq!(&data[..8], b"UnityFS\0");
    }

    #[test]
    fn patches_block_flag_six_to_lz4hc() {
        let mut blocks_info = vec![0u8; 16];
        blocks_info.extend_from_slice(&1u32.to_be_bytes());
        blocks_info.extend_from_slice(&100u32.to_be_bytes());
        blocks_info.extend_from_slice(&50u32.to_be_bytes());
        blocks_info.extend_from_slice(&6u16.to_be_bytes());

        let patched = NarakaBladepointDecryptionService::patch_block_flags(blocks_info).unwrap();

        assert_eq!(u16::from_be_bytes([patched[28], patched[29]]), 3);
    }

    #[test]
    fn converts_reported_new2_sample_when_available() {
        let sample = Path::new(
            "D:\\Softs\\Steam\\steamapps\\common\\NARAKA BLADEPOINT\\NarakaBladepoint_Data\\StreamingAssets\\0\\0\\00027328d2eb5747",
        );
        if !sample.exists() {
            return;
        }

        let data = std::fs::read(sample).unwrap();
        let converted = NarakaBladepointDecryptionService::convert_bundle_v11(&data).unwrap();

        assert!(converted.starts_with(b"UnityFS\0"));
        BundleFile::parse_filtered(&converted, |_| false).unwrap();
    }

    #[test]
    fn converts_reported_large_new2_sample_when_available() {
        let sample = Path::new(
            "D:\\Softs\\Steam\\steamapps\\common\\NARAKA BLADEPOINT\\NarakaBladepoint_Data\\StreamingAssets\\0\\9\\09e8d16fd77642ac",
        );
        if !sample.exists() {
            return;
        }

        let data = std::fs::read(sample).unwrap();
        let converted = NarakaBladepointDecryptionService::convert_bundle_v11(&data).unwrap();

        assert!(converted.starts_with(b"UnityFS\0"));
        BundleFile::parse_filtered(&converted, |_| false).unwrap();
    }

    #[test]
    fn converts_reported_literal_oob_new2_sample_when_available() {
        let sample = Path::new(
            "D:\\Softs\\Steam\\steamapps\\common\\NARAKA BLADEPOINT\\NarakaBladepoint_Data\\StreamingAssets\\a\\a\\aa3d8d5da9fb13f1",
        );
        if !sample.exists() {
            return;
        }

        let data = std::fs::read(sample).unwrap();
        let converted = NarakaBladepointDecryptionService::convert_bundle_v11(&data).unwrap();

        assert!(converted.starts_with(b"UnityFS\0"));
        BundleFile::parse_filtered(&converted, |_| false).unwrap();
    }

    #[test]
    fn converts_reported_slice_oob_new2_sample_when_available() {
        let sample = Path::new(
            "D:\\Softs\\Steam\\steamapps\\common\\NARAKA BLADEPOINT\\NarakaBladepoint_Data\\StreamingAssets\\c\\f\\cf634062ad17ad4b",
        );
        if !sample.exists() {
            return;
        }

        let data = std::fs::read(sample).unwrap();
        let converted = NarakaBladepointDecryptionService::convert_bundle_v11(&data).unwrap();

        assert!(converted.starts_with(b"UnityFS\0"));
        BundleFile::parse_filtered(&converted, |_| false).unwrap();
    }

    #[test]
    fn converts_reported_mesh_bundle_when_available() {
        let sample = Path::new(
            "D:\\Softs\\Steam\\steamapps\\common\\NARAKA BLADEPOINT\\NarakaBladepoint_Data\\StreamingAssets\\3\\1\\312ba9c886d5c842",
        );
        if !sample.exists() {
            return;
        }

        let data = std::fs::read(sample).unwrap();
        let converted = NarakaBladepointDecryptionService::convert_bundle_v11(&data).unwrap();

        assert!(converted.starts_with(b"UnityFS\0"));
        BundleFile::parse_filtered(&converted, |_| true).unwrap();
    }
}
