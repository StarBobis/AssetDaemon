/*
 * ress_list_cache.rs -- workspace-local .resS cache.
 *
 * Build Map warms this cache by extracting .resS nodes from UnityFS bundles into
 * <workspace>/ResSList. Runtime stream readers can then resolve external resource
 * paths without reopening large bundle payloads.
 */

use crate::common::bundle_file::bundle_file::BundleFile;
use crate::common::bundle_file::bundle_types::BundleFileEntry;
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::task::task_logger::TaskLogger;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::ipc::Channel;

#[derive(Debug, Clone, Default)]
pub struct ResSListSummary {
    pub total_bundles: usize,
    pub scanned_bundles: usize,
    pub skipped_bundles: usize,
    pub extracted_files: usize,
    pub reused_files: usize,
    pub failed_bundles: usize,
    pub total_bytes_written: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResSListManifest {
    version: u32,
    entries: Vec<ResSListManifestEntry>,
}

impl Default for ResSListManifest {
    fn default() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResSListManifestEntry {
    key: String,
    file_name: String,
    size: u64,
    source_bundle: String,
    node_path: String,
}

pub struct ResSListCache;

impl ResSListCache {
    pub const DIR_NAME: &'static str = "ResSList";
    const MANIFEST_NAME: &'static str = "_manifest.json";

    pub fn dir_for_workspace(workspace: &Path) -> PathBuf {
        workspace.join(Self::DIR_NAME)
    }

    pub fn manifest_path_for_workspace(workspace: &Path) -> PathBuf {
        Self::dir_for_workspace(workspace).join(Self::MANIFEST_NAME)
    }

    pub fn prepare(
        workspace: &Path,
        bundles: &[BundleFileEntry],
        progress: &Channel<ProgressPayload>,
        cancel_token: &Arc<AtomicBool>,
        task_id: &str,
    ) -> Result<ResSListSummary, String> {
        let output_dir = Self::dir_for_workspace(workspace);
        fs::create_dir_all(&output_dir).map_err(|e| {
            format!(
                "Cannot create ResSList directory '{}': {}",
                output_dir.display(),
                e
            )
        })?;

        let mut summary = ResSListSummary {
            total_bundles: bundles.len(),
            ..ResSListSummary::default()
        };
        let mut entries_by_key: HashMap<String, ResSListManifestEntry> = HashMap::new();

        TaskLogger::progress(
            task_id,
            "Build Map",
            "Prepare ResSList",
            0,
            bundles.len().max(1),
            "Preparing workspace ResSList cache...",
        );
        let _ = progress.send(ProgressPayload {
            step: "ress".into(),
            message: "Preparing ResSList cache...".into(),
        });

        for (index, bundle) in bundles.iter().enumerate() {
            if cancel_token.load(Ordering::Relaxed) {
                return Err("Task cancelled".to_string());
            }

            let bundle_path = Path::new(&bundle.path);
            let current = index + 1;
            if !Self::looks_like_unityfs(bundle_path) {
                summary.skipped_bundles += 1;
                continue;
            }
            if current == 1 || current % 25 == 0 || current == bundles.len() {
                TaskLogger::progress(
                    task_id,
                    "Build Map",
                    "Prepare ResSList",
                    current,
                    bundles.len().max(1),
                    &format!(
                        "Scanning .resS nodes: {} ({}/{})",
                        bundle.name,
                        current,
                        bundles.len()
                    ),
                );
                let _ = progress.send(ProgressPayload {
                    step: "ress".into(),
                    message: format!("Scanning .resS nodes: {}", bundle.name),
                });
            }

            let include_output_dir = output_dir.clone();
            // Contain parser panics per bundle: one malformed file must not
            // abort the whole ResSList preparation (and the Build Map task).
            let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                BundleFile::load_filtered(bundle_path, |node_path| {
                    if !Self::is_ress_path(node_path) {
                        return false;
                    }
                    let file_name = Self::output_file_name(node_path);
                    !include_output_dir.join(file_name).exists()
                })
            }));

            let parsed = match parsed {
                Ok(Ok(bundle_file)) => bundle_file,
                Ok(Err(_)) => {
                    summary.failed_bundles += 1;
                    continue;
                }
                Err(payload) => {
                    summary.failed_bundles += 1;
                    let panic_message =
                        crate::common::panic_reporter::PanicReporter::payload_to_string(
                            payload.as_ref(),
                        );
                    let panic_snippet: String = panic_message.chars().take(160).collect();
                    TaskLogger::warn(
                        task_id,
                        "Build Map",
                        &format!(
                            "ResSList scan panicked for bundle '{}', skipped: {}",
                            bundle.name, panic_snippet
                        ),
                    );
                    continue;
                }
            };

            summary.scanned_bundles += 1;
            let mut bundle_had_ress = false;

            for node in &parsed.directory_info {
                if !Self::is_ress_path(&node.path) {
                    continue;
                }
                bundle_had_ress = true;
                let file_name = Self::output_file_name(&node.path);
                let output_path = output_dir.join(&file_name);
                if output_path.exists() {
                    summary.reused_files += 1;
                    let size = fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);
                    Self::insert_manifest_entries(
                        &mut entries_by_key,
                        &node.path,
                        &file_name,
                        size,
                        &bundle.path,
                    );
                }
            }

            for stream_file in parsed.file_list {
                if !Self::is_ress_path(&stream_file.path) {
                    continue;
                }
                bundle_had_ress = true;
                let file_name = Self::output_file_name(&stream_file.path);
                let output_path = output_dir.join(&file_name);
                if output_path.exists() {
                    summary.reused_files += 1;
                    let size = fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);
                    Self::insert_manifest_entries(
                        &mut entries_by_key,
                        &stream_file.path,
                        &file_name,
                        size,
                        &bundle.path,
                    );
                    continue;
                }

                fs::write(&output_path, &stream_file.data).map_err(|e| {
                    format!("Cannot write .resS '{}': {}", output_path.display(), e)
                })?;
                summary.extracted_files += 1;
                summary.total_bytes_written += stream_file.data.len() as u64;
                Self::insert_manifest_entries(
                    &mut entries_by_key,
                    &stream_file.path,
                    &file_name,
                    stream_file.data.len() as u64,
                    &bundle.path,
                );
            }

            if !bundle_had_ress {
                summary.skipped_bundles += 1;
            }
        }

        let mut manifest = ResSListManifest {
            version: 1,
            entries: entries_by_key.into_values().collect(),
        };
        manifest.entries.sort_by(|a, b| a.key.cmp(&b.key));
        let manifest_json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| format!("Cannot serialize ResSList manifest: {}", e))?;
        fs::write(Self::manifest_path_for_workspace(workspace), manifest_json)
            .map_err(|e| format!("Cannot write ResSList manifest: {}", e))?;

        TaskLogger::progress(
            task_id,
            "Build Map",
            "Prepare ResSList",
            bundles.len().max(1),
            bundles.len().max(1),
            &format!(
                "ResSList ready: {} extracted, {} reused, {} failed bundles.",
                summary.extracted_files, summary.reused_files, summary.failed_bundles
            ),
        );
        let _ = progress.send(ProgressPayload {
            step: "ress".into(),
            message: format!(
                "ResSList ready: {} extracted, {} reused",
                summary.extracted_files, summary.reused_files
            ),
        });

        Ok(summary)
    }

    pub fn resolve_from_bundle_path(bundle_path: &Path, res_path: &str) -> Option<PathBuf> {
        let start = bundle_path.parent().unwrap_or(bundle_path);
        let ress_dir = Self::find_nearest_dir(start)?;
        Self::resolve_in_dir(&ress_dir, res_path)
    }

    pub fn read_range_from_bundle_path(
        bundle_path: &Path,
        res_path: &str,
        offset: usize,
        size: usize,
    ) -> Option<Vec<u8>> {
        let path = Self::resolve_from_bundle_path(bundle_path, res_path)?;
        Self::read_range(&path, offset, size)
    }

    pub fn read_range(path: &Path, offset: usize, size: usize) -> Option<Vec<u8>> {
        if size == 0 {
            return Some(Vec::new());
        }
        let mut file = fs::File::open(path).ok()?;
        let file_len = file.metadata().ok()?.len() as usize;
        if offset >= file_len {
            return None;
        }
        let read_size = size.min(file_len.saturating_sub(offset));
        if read_size == 0 {
            return None;
        }
        file.seek(SeekFrom::Start(offset as u64)).ok()?;
        let mut data = vec![0u8; read_size];
        file.read_exact(&mut data).ok()?;
        Some(data)
    }

    fn find_nearest_dir(start: &Path) -> Option<PathBuf> {
        for ancestor in start.ancestors() {
            let candidate = ancestor.join(Self::DIR_NAME);
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
        None
    }

    fn resolve_in_dir(ress_dir: &Path, res_path: &str) -> Option<PathBuf> {
        if !ress_dir.is_dir() {
            return None;
        }
        let keys = Self::lookup_keys(res_path);
        if keys.is_empty() {
            return None;
        }

        if let Some(manifest) = Self::read_manifest(ress_dir) {
            for key in &keys {
                if let Some(entry) = manifest.entries.iter().find(|entry| entry.key == *key) {
                    let path = ress_dir.join(&entry.file_name);
                    if path.exists() {
                        return Some(path);
                    }
                }
            }

            for key in &keys {
                if !Self::is_useful_contains_key(key) {
                    continue;
                }
                if let Some(entry) = manifest.entries.iter().find(|entry| {
                    entry.key.contains(key)
                        || key.contains(&entry.key)
                        || entry.file_name.to_ascii_lowercase().contains(key)
                }) {
                    let path = ress_dir.join(&entry.file_name);
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }

        for key in &keys {
            let direct = ress_dir.join(Self::output_file_name(key));
            if direct.exists() {
                return Some(direct);
            }
        }

        for key in &keys {
            if !Self::is_useful_contains_key(key) {
                continue;
            }
            if let Ok(read_dir) = fs::read_dir(ress_dir) {
                for entry in read_dir.flatten() {
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_ascii_lowercase())
                        .unwrap_or_default();
                    if name.contains(key) {
                        return Some(path);
                    }
                }
            }
        }

        None
    }

    fn read_manifest(ress_dir: &Path) -> Option<ResSListManifest> {
        let text = fs::read_to_string(ress_dir.join(Self::MANIFEST_NAME)).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn insert_manifest_entries(
        entries_by_key: &mut HashMap<String, ResSListManifestEntry>,
        node_path: &str,
        file_name: &str,
        size: u64,
        source_bundle: &str,
    ) {
        for key in Self::lookup_keys(node_path) {
            entries_by_key
                .entry(key.clone())
                .or_insert_with(|| ResSListManifestEntry {
                    key,
                    file_name: file_name.to_string(),
                    size,
                    source_bundle: source_bundle.to_string(),
                    node_path: node_path.to_string(),
                });
        }
    }

    fn lookup_keys(path: &str) -> Vec<String> {
        let mut keys = Vec::new();
        let clean = Self::normalize_key(path);
        if clean.is_empty() {
            return keys;
        }

        keys.push(clean.clone());
        let archive_path = clean.strip_prefix("archive:/").unwrap_or(&clean);
        keys.push(archive_path.to_string());

        let parts = archive_path
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if let Some(first) = parts.first() {
            keys.push((*first).to_string());
            if !first.ends_with(".ress") {
                keys.push(format!("{}.ress", first));
            }
        }
        if let Some(last) = parts.last() {
            keys.push((*last).to_string());
            if !last.ends_with(".ress") {
                keys.push(format!("{}.ress", last));
            }
        }

        let file_name = Path::new(archive_path)
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if !file_name.is_empty() {
            keys.push(file_name.clone());
            if !file_name.ends_with(".ress") {
                keys.push(format!("{}.ress", file_name));
            }
        }

        let mut seen = HashSet::new();
        keys.retain(|key| !key.is_empty() && seen.insert(key.clone()));
        keys
    }

    fn normalize_key(path: &str) -> String {
        path.trim()
            .trim_matches(char::from(0))
            .replace('\\', "/")
            .to_ascii_lowercase()
    }

    fn output_file_name(path: &str) -> String {
        let raw = Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.rsplit(['/', '\\']).next().unwrap_or(path).to_string());
        let mut out = String::with_capacity(raw.len().max(8));
        for ch in raw.chars() {
            let is_invalid = matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
                || ch.is_control();
            if is_invalid {
                out.push('_');
            } else {
                out.push(ch);
            }
        }
        let out = out.trim_matches(['.', ' ']).to_string();
        if out.is_empty() {
            "resource.resS".to_string()
        } else {
            out
        }
    }

    fn is_ress_path(path: &str) -> bool {
        path.to_ascii_lowercase().ends_with(".ress")
    }

    fn looks_like_unityfs(path: &Path) -> bool {
        let mut buf = [0u8; 7];
        fs::File::open(path)
            .and_then(|mut file| file.read_exact(&mut buf))
            .map(|_| &buf == b"UnityFS")
            .unwrap_or(false)
    }

    fn is_useful_contains_key(key: &str) -> bool {
        key.len() >= 4 && key != ".ress" && !key.ends_with('/')
    }
}

#[cfg(test)]
mod tests {
    use super::ResSListCache;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "assetfinder_ress_cache_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn resolves_manifest_by_archive_name_and_contains_fallback() {
        let root = temp_dir("resolve");
        let ress_dir = root.join(ResSListCache::DIR_NAME);
        fs::create_dir_all(&ress_dir).unwrap();
        fs::write(ress_dir.join("CAB-123456.resS"), b"abcdefghijkl").unwrap();
        fs::write(
            ress_dir.join("_manifest.json"),
            r#"{
  "version": 1,
  "entries": [
    {
      "key": "cab-123456.ress",
      "file_name": "CAB-123456.resS",
      "size": 12,
      "source_bundle": "bundle",
      "node_path": "CAB-123456.resS"
    }
  ]
}"#,
        )
        .unwrap();

        let bundle = root.join("nested").join("x.bundle");
        fs::create_dir_all(bundle.parent().unwrap()).unwrap();
        fs::write(&bundle, b"UnityFS").unwrap();

        let resolved = ResSListCache::resolve_from_bundle_path(&bundle, "archive:/CAB-123456/foo");
        assert_eq!(resolved, Some(ress_dir.join("CAB-123456.resS")));

        let data = ResSListCache::read_range_from_bundle_path(&bundle, "CAB-123456", 2, 4).unwrap();
        assert_eq!(data, b"cdef");

        let _ = fs::remove_dir_all(&root);
    }
}
