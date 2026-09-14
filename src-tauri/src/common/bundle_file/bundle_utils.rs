/*
 * Bundle file utilities - provides Unity file candidate directory scanning.
 *
 * Follows Soul.md conventions: organized via struct + impl, no free functions.
 */

use std::fs;
use std::path::Path;
use tauri::ipc::Channel;

use super::bundle_types::BundleFileEntry;
use super::ress_list_cache::ResSListCache;
use crate::common::scan::scan_types::ProgressPayload;
use crate::utils::time_utils::TimeUtils;

/**
 * Bundle file collector.
 *
 * Recursively scans a directory for Unity file candidates,
 * skipping loose resource payloads such as .resS/.resource/.res.
 * returns sorted file info list for frontend display.
 *
 * Overlaps with decrypt::girls_frontline2::gf2::DecryptionService::collect_bundle_files.
 * The latter only returns path lists (no metadata), used for the file decryption flow.
 */
pub struct BundleFileCollector;

impl BundleFileCollector {
    /**
     * Recursively scan a directory and collect Unity file candidate info.
     *
     * @param dir      The root directory to scan
     * @param results  Mutable reference to collect results into
     */
    pub fn collect_bundle_files(
        dir: &Path,
        results: &mut Vec<BundleFileEntry>,
    ) -> Result<(), String> {
        Self::collect_bundle_files_with_progress(dir, results, None)
    }

    pub fn collect_bundle_files_with_progress(
        dir: &Path,
        results: &mut Vec<BundleFileEntry>,
        progress: Option<&Channel<ProgressPayload>>,
    ) -> Result<(), String> {
        let mut include_candidate = |path: &Path| !Self::is_resource_payload_path(path);
        Self::collect_bundle_files_with_progress_filtered(
            dir,
            results,
            progress,
            &mut include_candidate,
        )
    }

    pub fn collect_bundle_files_filtered<F>(
        dir: &Path,
        results: &mut Vec<BundleFileEntry>,
        mut include_file: F,
    ) -> Result<(), String>
    where
        F: FnMut(&Path) -> bool,
    {
        Self::collect_bundle_files_with_progress_filtered(dir, results, None, &mut include_file)
    }

    fn collect_bundle_files_with_progress_filtered<F>(
        dir: &Path,
        results: &mut Vec<BundleFileEntry>,
        progress: Option<&Channel<ProgressPayload>>,
        include_file: &mut F,
    ) -> Result<(), String>
    where
        F: FnMut(&Path) -> bool + ?Sized,
    {
        let read_dir = fs::read_dir(dir)
            .map_err(|e| format!("Cannot read directory '{}': {}", dir.display(), e))?;
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path
                    .file_name()
                    .map(|name| {
                        name.to_string_lossy()
                            .eq_ignore_ascii_case(ResSListCache::DIR_NAME)
                    })
                    .unwrap_or(false)
                {
                    continue;
                }
                Self::collect_bundle_files_with_progress_filtered(
                    &path,
                    results,
                    progress,
                    include_file,
                )?;
            } else if path.is_file() && include_file(&path) {
                results.push(Self::collect_file_entry(&path)?);
                if results.len() % 1000 == 0 {
                    if let Some(progress) = progress {
                        let _ = progress.send(ProgressPayload {
                            step: "scan".into(),
                            message: format!(
                                "Found {} files, current: {}",
                                results.len(),
                                path.display()
                            ),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    pub fn collect_file_entry(path: &Path) -> Result<BundleFileEntry, String> {
        let metadata = fs::metadata(path)
            .map_err(|e| format!("Cannot read file info '{}': {}", path.display(), e))?;
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        let modified = match metadata.modified() {
            Ok(time) => TimeUtils::format_time(time),
            Err(_) => String::from("Unknown"),
        };
        Ok(BundleFileEntry {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            path: path.to_string_lossy().to_string(),
            size: metadata.len(),
            modified_ms,
            modified,
        })
    }

    pub fn is_resource_payload_path(path: &Path) -> bool {
        let lower = path.to_string_lossy().to_ascii_lowercase();
        lower.ends_with(".ress") || lower.ends_with(".resource") || lower.ends_with(".res")
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::BundleFileCollector;

    #[test]
    fn skips_resource_payload_files_by_default() {
        let root = std::env::temp_dir().join(format!(
            "assetfinder_bundle_scan_test_{}",
            std::process::id()
        ));
        let nested = root.join("nested");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("123"), b"UnityFS candidate").unwrap();
        fs::write(root.join("CAB-123.resS"), b"resource payload").unwrap();
        fs::write(nested.join("456.notbundle"), b"also candidate").unwrap();

        let mut entries = Vec::new();
        BundleFileCollector::collect_bundle_files(&root, &mut entries).unwrap();
        let mut names = entries
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        names.sort();

        assert_eq!(names, vec!["123".to_string(), "456.notbundle".to_string()]);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn filtered_collection_can_skip_resource_payloads() {
        let root = std::env::temp_dir().join(format!(
            "assetfinder_bundle_scan_filtered_test_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("bundle"), b"UnityFS candidate").unwrap();
        fs::write(root.join("CAB-123.resS"), b"resource payload").unwrap();

        let mut skipped = 0usize;
        let mut entries = Vec::new();
        BundleFileCollector::collect_bundle_files_filtered(&root, &mut entries, |path| {
            let include = !path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".ress");
            if !include {
                skipped += 1;
            }
            include
        })
        .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "bundle");
        assert_eq!(skipped, 1);

        let _ = fs::remove_dir_all(&root);
    }
}
