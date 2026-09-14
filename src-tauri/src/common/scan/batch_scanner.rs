/*
 * Batch Scanner class -- Provides Bundle file type scanning and MD5 caching capabilities.
 *
 * Follows Soul.md conventions: struct + impl organization, no free functions.
 */

use crate::common::bundle_file::asset_bundle::AssetBundleLoader;
use crate::common::scan::scan_types::FileTypeResult;
use crate::utils::hash_utils::HashUtils;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{collections::HashMap, path::Path};

/**
 * Batch type scanner class.
 *
 * Uses MD5 caching to accelerate repeated scans, supports type parsing for individual files.
 */
pub struct BatchScanner;

impl BatchScanner {
    /**
     * Scan a single file for types, using cache to skip already parsed files.
     *
     * First compute MD5 and compare with cache; if same, skip parsing.
     * If different, parse the Bundle and collect all asset types.
     */
    pub fn scan_single_file_cancelable(
        path: &str,
        cached: &HashMap<String, String>,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<FileTypeResult, String> {
        // Compute MD5 and compare with cache
        let current_md5 = match HashUtils::file_md5_cancelable(path, cancel_token) {
            Ok(md5) => md5,
            Err(error) if error.to_ascii_lowercase().contains("cancel") => {
                return Err(error);
            }
            Err(_) => {
                return Ok(FileTypeResult {
                    path: path.to_string(),
                    md5: String::new(),
                    types: vec![],
                });
            }
        };
        if cancel_token.is_some_and(|token| token.load(Ordering::SeqCst)) {
            return Err("Task cancelled".to_string());
        }

        // MD5 matches, skip parsing
        if let Some(cached_md5) = cached.get(path) {
            if *cached_md5 == current_md5 {
                return Ok(FileTypeResult {
                    path: path.to_string(),
                    md5: current_md5,
                    types: vec![],
                });
            }
        }
        if cancel_token.is_some_and(|token| token.load(Ordering::SeqCst)) {
            return Err("Task cancelled".to_string());
        }

        let bundle_path = Path::new(path);
        let mut types: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        if let Ok(bundle) = AssetBundleLoader::load_unity_file(bundle_path) {
            if cancel_token.is_some_and(|token| token.load(Ordering::SeqCst)) {
                return Err("Task cancelled".to_string());
            }
            for file in &bundle.assets {
                for obj in &file.objects {
                    if cancel_token.is_some_and(|token| token.load(Ordering::SeqCst)) {
                        return Err("Task cancelled".to_string());
                    }
                    let class_name = AssetBundleLoader::get_class_name(obj.class_id)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| String::from("Unknown"));
                    if seen.insert(class_name.clone()) {
                        types.push(class_name);
                    }
                }
            }
        }

        Ok(FileTypeResult {
            path: path.to_string(),
            md5: current_md5,
            types,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::AtomicBool;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn scan_single_file_stops_when_cancelled_before_hashing() {
        let path = temp_file("batch-scan-cancel.bundle");
        fs::write(&path, b"not a real bundle").unwrap();
        let cancel_token = Arc::new(AtomicBool::new(true));

        let result = BatchScanner::scan_single_file_cancelable(
            &path.to_string_lossy(),
            &HashMap::new(),
            Some(&cancel_token),
        );

        let _ = fs::remove_file(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_ascii_lowercase().contains("cancel"));
    }

    fn temp_file(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("assetfinder-{}-{}", nonce, name))
    }
}
