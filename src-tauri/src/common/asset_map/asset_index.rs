#![allow(dead_code)]

use crate::common::asset_map::map_bundle_scanner::MapBundleScanner;
use crate::common::asset_map::parse_cache_store::AssetMapParseCacheStore;
use crate::common::asset_map::path_matcher::BundlePathMatcher;
use crate::common::asset_map::sqlite_store::{SqliteBuildSession, SqliteStore};
use crate::common::bundle_file::bundle_types::BundleFileEntry;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

// ============================================================
// Public data types — kept identical for API compatibility.
// These structs are used throughout the crate and by the
// frontend-facing Tauri commands.
// ============================================================

#[derive(Debug, Clone)]
pub struct AssetRow {
    pub bundle_path: String,
    pub path_id: i64,
    pub class_id: i32,
    pub class_name: String,
    pub asset_name: String,
    pub byte_size: u32,
}

#[derive(Debug, Clone)]
pub struct AssetDisplayRow {
    pub asset: AssetRow,
    pub asset_path: String,
}

#[derive(Debug, Clone)]
pub struct TextureCandidateRow {
    pub asset: AssetRow,
    pub asset_path: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ContainerRow {
    pub bundle_path: String,
    pub asset_path: String,
    pub path_id: i64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RelationRow {
    pub bundle_path: String,
    pub relation_type: String,
    pub source_path_id: i64,
    pub target_path_id: i64,
    pub source_name: String,
    pub target_name: String,
    pub file_id: i32,
    pub field_path: String,
    pub target_bundle_path: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BundleInfoRow {
    pub path: String,
    pub md5: String,
    pub file_size: u64,
    pub modified_ms: u64,
    pub unity_version: String,
    pub asset_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetWriteRow {
    pub bundle_path: String,
    pub path_id: i64,
    pub class_id: i32,
    pub class_name: String,
    pub asset_name: String,
    pub byte_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerWriteRow {
    pub bundle_path: String,
    pub asset_path: String,
    pub path_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalWriteRow {
    pub bundle_path: String,
    pub sf_index: usize,
    pub file_id: i32,
    pub path_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleInternalNameWriteRow {
    pub bundle_path: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationWriteRow {
    pub bundle_path: String,
    pub relation_type: String,
    pub source_path_id: i64,
    pub target_path_id: i64,
    pub source_name: String,
    pub target_name: String,
    pub file_id: i32,
    pub field_path: String,
    pub target_bundle_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapBundleWriteRows {
    pub bundle_path: String,
    pub md5: String,
    pub file_size: u64,
    pub modified_ms: u64,
    pub unity_version: String,
    pub asset_count: usize,
    pub assets: Vec<AssetWriteRow>,
    pub containers: Vec<ContainerWriteRow>,
    pub externals: Vec<ExternalWriteRow>,
    pub internal_names: Vec<BundleInternalNameWriteRow>,
    pub relations: Vec<RelationWriteRow>,
}

// ============================================================
// AssetDatabase — SQLite-backed facade
// ============================================================

pub struct AssetDatabase {
    workspace_path: PathBuf,
    cache_root: Option<PathBuf>,
    store: SqliteStore,
}

pub struct QueryInterruptWatcher {
    done: Arc<AtomicBool>,
}

#[derive(Clone)]
struct ParseCacheRelationSnapshot {
    key: String,
    rows: Vec<RelationRow>,
}

static PARSE_CACHE_RELATION_SNAPSHOT: Lazy<Mutex<Option<ParseCacheRelationSnapshot>>> =
    Lazy::new(|| Mutex::new(None));

impl Drop for QueryInterruptWatcher {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
    }
}

impl AssetDatabase {
    // ----- lifecycle -----

    pub fn open(workspace: &Path) -> Result<Self, String> {
        Self::open_with_cache_root(workspace, None)
    }

    pub fn open_with_cache_root(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<Self, String> {
        let store = SqliteStore::open(workspace, cache_root)?;
        if let Some(root) = cache_root {
            fs::create_dir_all(Self::workspace_cache_dir(root, workspace))
                .map_err(|e| format!("Failed to create asset index cache dir: {}", e))?;
        }
        Ok(Self {
            workspace_path: workspace.to_path_buf(),
            cache_root: cache_root.map(Path::to_path_buf),
            store,
        })
    }

    pub fn open_for_rebuild(workspace: &Path) -> Result<Self, String> {
        Self::open_for_rebuild_with_cache_root(workspace, None)
    }

    pub fn open_for_rebuild_with_cache_root(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<Self, String> {
        Self::open_with_cache_root(workspace, cache_root)
    }

    pub fn workspace_path(&self) -> &Path {
        &self.workspace_path
    }

    pub fn cache_root(&self) -> Option<&Path> {
        self.cache_root.as_deref()
    }

    pub fn workspace_cache_dir(cache_root: &Path, workspace: &Path) -> PathBuf {
        cache_root.join(Self::workspace_cache_key(workspace))
    }

    pub fn workspace_cache_key(workspace: &Path) -> String {
        let workspace_text = workspace.to_string_lossy();
        let display_name = workspace
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("workspace");
        let safe_name = Self::sanitize_cache_component(display_name);
        let hash = {
            use md5::Digest;
            let mut hasher = md5::Md5::new();
            hasher.update(workspace_text.as_bytes());
            format!("{:032x}", hasher.finalize())
        };
        format!("{}-{}", safe_name, hash)
    }

    pub fn sanitize_cache_component(value: &str) -> String {
        let sanitized: String = value
            .chars()
            .map(|ch| match ch {
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
                ch if ch.is_control() => '_',
                ch => ch,
            })
            .collect();
        let trimmed = sanitized.trim_matches([' ', '.']).trim();
        if trimmed.is_empty() {
            "workspace".to_string()
        } else {
            trimmed.chars().take(80).collect()
        }
    }

    // ----- storage management -----

    pub fn delete_files(workspace: &Path) -> Result<(), String> {
        Self::delete_files_with_cache_root(workspace, None)
    }

    pub fn delete_files_with_cache_root(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<(), String> {
        SqliteStore::delete_db(workspace, cache_root)
    }

    pub fn exists(workspace: &Path) -> bool {
        Self::exists_with_cache_root(workspace, None)
    }

    pub fn exists_with_cache_root(workspace: &Path, cache_root: Option<&Path>) -> bool {
        SqliteStore::exists(workspace, cache_root)
    }

    // ----- query helpers -----

    pub fn spawn_interrupt_watcher(&self, _cancel_token: Arc<AtomicBool>) -> QueryInterruptWatcher {
        QueryInterruptWatcher {
            done: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn drop_secondary_indexes_for_rebuild(&self) -> Result<(), String> {
        self.store.drop_indexes()
    }

    pub fn create_secondary_indexes_for_rebuild(&self) -> Result<(), String> {
        self.store.create_indexes()
    }

    pub fn begin_batch(&self) -> Result<(), String> {
        Ok(())
    }

    pub fn commit_batch(&self) -> Result<(), String> {
        Ok(())
    }

    pub fn finish_incremental_build(&self, cancelled: bool) -> Result<(), String> {
        self.store
            .update_build_meta("cancelled", &cancelled.to_string())
    }

    pub fn analyze(&self) -> Result<(), String> {
        self.store.optimize()
    }

    pub fn clear(&self) -> Result<(), String> {
        Self::delete_files_with_cache_root(&self.workspace_path, self.cache_root())
    }

    pub fn file_size(&self) -> u64 {
        self.store.file_size()
    }

    pub fn built_at(&self) -> Result<u64, String> {
        self.store.get_build_meta_i64("built_at").map(|v| v as u64)
    }

    pub fn build_cancelled(&self) -> Result<bool, String> {
        self.store.get_build_meta_bool("cancelled")
    }

    pub fn asset_count(&self) -> Result<usize, String> {
        self.store
            .get_build_meta_i64("asset_count")
            .map(|v| v as usize)
    }

    pub fn parsed_count(&self) -> Result<usize, String> {
        self.store
            .get_build_meta_i64("parsed_count")
            .map(|v| v as usize)
    }

    pub fn bundle_count(&self) -> Result<usize, String> {
        self.store
            .get_build_meta_i64("bundle_count")
            .map(|v| v as usize)
    }

    // ----- bundle queries -----

    pub fn get_bundle_infos(&self) -> Result<Vec<BundleInfoRow>, String> {
        self.store.get_bundle_infos()
    }

    pub fn find_bundle_paths_by_class(&self, class_name: &str) -> Result<Vec<String>, String> {
        self.store.find_bundle_paths_by_class(class_name)
    }

    // ----- asset lookups -----

    pub fn find_by_bundle_and_path_id(
        &self,
        bundle_path: &str,
        path_id: i64,
    ) -> Result<Option<AssetRow>, String> {
        self.store.find_by_bundle_and_path_id(bundle_path, path_id)
    }

    pub fn find_by_bundle(&self, bundle_path: &str) -> Result<Vec<AssetRow>, String> {
        self.store.find_by_bundle(bundle_path)
    }

    pub fn find_by_path_id(&self, path_id: i64) -> Result<Vec<AssetRow>, String> {
        self.store.find_by_path_id(path_id)
    }

    pub fn find_by_class(&self, class_name: &str) -> Result<Vec<AssetRow>, String> {
        self.store.find_by_class(class_name)
    }

    pub fn find_by_class_and_bundle(
        &self,
        class_name: &str,
        bundle_path: &str,
    ) -> Result<Vec<AssetRow>, String> {
        self.store.find_by_class_and_bundle(class_name, bundle_path)
    }

    /// Find Mesh assets with an exact matching name in other bundles of the
    /// workspace. Used by the same-name mesh texture borrow fallback so mesh
    /// previews can reuse textures resolved from duplicated mesh copies.
    pub fn find_mesh_copies_by_name(
        &self,
        mesh_name: &str,
        exclude_bundle_path: &str,
        limit: usize,
    ) -> Result<Vec<AssetDisplayRow>, String> {
        self.store
            .find_mesh_copies_by_name(mesh_name, exclude_bundle_path, limit)
    }

    // ----- container / external / internal name lookups -----

    pub fn get_containers(&self, bundle_path: &str) -> Result<Vec<ContainerRow>, String> {
        self.store.get_containers(bundle_path)
    }

    pub fn find_externals_by_bundle(
        &self,
        bundle_path: &str,
    ) -> Result<Vec<ExternalWriteRow>, String> {
        self.store.find_externals_by_bundle(bundle_path)
    }

    pub fn find_bundle_by_internal_name(&self, name: &str) -> Result<Option<String>, String> {
        self.store.find_bundle_by_internal_name(name)
    }

    // ----- relation queries -----

    pub fn get_relations_by_bundle(&self, bundle_path: &str) -> Result<Vec<RelationRow>, String> {
        if let Some(rows) = self.cached_bundle_rows(bundle_path)? {
            return Ok(Self::relations_from_cached_rows(&rows, |_| true));
        }
        self.store.get_relations_by_bundle(bundle_path)
    }

    pub fn get_relations_by_type(
        &self,
        bundle_path: &str,
        relation_type: &str,
    ) -> Result<Vec<RelationRow>, String> {
        if let Some(rows) = self.cached_bundle_rows(bundle_path)? {
            return Ok(Self::relations_from_cached_rows(&rows, |relation| {
                relation.relation_type == relation_type
            }));
        }
        self.store.get_relations_by_type(bundle_path, relation_type)
    }

    pub fn find_relations_by_bundle_and_source(
        &self,
        relation_type: &str,
        bundle_path: &str,
        source_path_id: i64,
    ) -> Result<Vec<RelationRow>, String> {
        if let Some(rows) = self.cached_bundle_rows(bundle_path)? {
            return Ok(Self::relations_from_cached_rows(&rows, |relation| {
                relation.relation_type == relation_type && relation.source_path_id == source_path_id
            }));
        }
        self.store
            .find_relations_by_bundle_and_source(bundle_path, relation_type, source_path_id)
    }

    pub fn find_relations_by_bundle_and_target(
        &self,
        relation_type: &str,
        bundle_path: &str,
        target_path_id: i64,
    ) -> Result<Vec<RelationRow>, String> {
        if let Some(rows) = self.cached_bundle_rows(bundle_path)? {
            let local = Self::relations_from_cached_rows(&rows, |relation| {
                relation.relation_type == relation_type
                    && relation.target_path_id == target_path_id
                    && relation.target_bundle_path.is_empty()
            });
            if !local.is_empty() {
                return Ok(local);
            }

            return Ok(Self::relations_from_cached_rows(&rows, |relation| {
                relation.relation_type == relation_type
                    && relation.target_path_id == target_path_id
                    && (relation.target_bundle_path.is_empty()
                        || relation.target_bundle_path == bundle_path)
            }));
        }
        self.store
            .find_relations_by_bundle_and_target(bundle_path, relation_type, target_path_id)
    }

    pub fn find_relations_by_target(
        &self,
        target_bundle_path: &str,
        target_path_id: i64,
    ) -> Result<Vec<RelationRow>, String> {
        let cached = self.cached_relation_snapshot(false)?;
        if !cached.is_empty() {
            return Ok(cached
                .iter()
                .filter(|relation| {
                    relation.target_path_id == target_path_id
                        && relation.target_bundle_path == target_bundle_path
                })
                .cloned()
                .collect());
        }

        self.store
            .find_relations_by_target(target_bundle_path, target_path_id)
    }

    pub fn find_relations_by_type_prefix(
        &self,
        bundle_path: &str,
        prefix: &str,
    ) -> Result<Vec<RelationRow>, String> {
        if let Some(rows) = self.cached_bundle_rows(bundle_path)? {
            return Ok(Self::relations_from_cached_rows(&rows, |relation| {
                relation.relation_type.starts_with(prefix)
            }));
        }
        self.store
            .find_relations_by_type_prefix(bundle_path, prefix)
    }

    fn cached_bundle_rows(&self, bundle_path: &str) -> Result<Option<MapBundleWriteRows>, String> {
        let Some(bundle) = self.store.get_bundle_info(bundle_path)? else {
            return Ok(None);
        };
        let parse_cache =
            AssetMapParseCacheStore::open(&self.workspace_path, self.cache_root.as_deref())?;
        if let Some(rows) = parse_cache.get_valid(
            bundle_path,
            bundle.file_size,
            bundle.modified_ms,
            &bundle.md5,
            true,
        )? {
            return Ok(Some(rows));
        }

        let rows = self.parse_bundle_rows_with_relations(&bundle)?;
        parse_cache.put_many_refs(&[&rows], true)?;
        Self::invalidate_relation_snapshot();
        Ok(Some(rows))
    }

    fn cached_relation_snapshot(&self, generate_missing: bool) -> Result<Vec<RelationRow>, String> {
        let bundles = self.store.get_bundle_infos()?;
        let snapshot_key =
            Self::relation_snapshot_key(&self.workspace_path, self.cache_root.as_deref(), &bundles);
        if let Ok(cache) = PARSE_CACHE_RELATION_SNAPSHOT.lock() {
            if let Some(snapshot) = cache.as_ref() {
                if snapshot.key == snapshot_key {
                    return Ok(snapshot.rows.clone());
                }
            }
        }

        let parse_cache =
            AssetMapParseCacheStore::open(&self.workspace_path, self.cache_root.as_deref())?;
        let mut cached_rows = parse_cache.get_valid_many(&bundles, true)?;
        if generate_missing && cached_rows.len() < bundles.len() {
            let cached_paths = cached_rows
                .iter()
                .map(|rows| rows.bundle_path.clone())
                .collect::<std::collections::HashSet<_>>();
            let mut generated = Vec::new();
            for bundle in bundles
                .iter()
                .filter(|bundle| !cached_paths.contains(&bundle.path))
            {
                generated.push(self.parse_bundle_rows_with_relations(bundle)?);
            }
            let cache_refs = generated.iter().collect::<Vec<_>>();
            parse_cache.put_many_refs(&cache_refs, true)?;
            cached_rows.extend(generated);
        }

        let rows = cached_rows
            .iter()
            .flat_map(|bundle_rows| Self::relations_from_cached_rows(bundle_rows, |_| true))
            .collect::<Vec<_>>();

        if let Ok(mut cache) = PARSE_CACHE_RELATION_SNAPSHOT.lock() {
            *cache = Some(ParseCacheRelationSnapshot {
                key: snapshot_key,
                rows: rows.clone(),
            });
        }

        Ok(rows)
    }

    fn relations_from_cached_rows(
        rows: &MapBundleWriteRows,
        keep: impl Fn(&RelationWriteRow) -> bool,
    ) -> Vec<RelationRow> {
        rows.relations
            .iter()
            .filter(|relation| keep(relation))
            .map(|relation| RelationRow {
                bundle_path: rows.bundle_path.clone(),
                relation_type: relation.relation_type.clone(),
                source_path_id: relation.source_path_id,
                target_path_id: relation.target_path_id,
                source_name: relation.source_name.clone(),
                target_name: relation.target_name.clone(),
                file_id: relation.file_id,
                field_path: relation.field_path.clone(),
                target_bundle_path: relation.target_bundle_path.clone(),
            })
            .collect()
    }

    fn relation_snapshot_key(
        workspace: &Path,
        cache_root: Option<&Path>,
        bundles: &[BundleInfoRow],
    ) -> String {
        let mut key = format!(
            "{}|{}|{}",
            workspace.display(),
            cache_root
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            bundles.len()
        );
        for bundle in bundles {
            key.push('|');
            key.push_str(&bundle.path);
            key.push(':');
            key.push_str(&bundle.file_size.to_string());
            key.push(':');
            key.push_str(&bundle.modified_ms.to_string());
            key.push(':');
            key.push_str(&bundle.md5);
        }
        key
    }

    fn invalidate_relation_snapshot() {
        if let Ok(mut cache) = PARSE_CACHE_RELATION_SNAPSHOT.lock() {
            *cache = None;
        }
    }

    fn parse_bundle_rows_with_relations(
        &self,
        bundle: &BundleInfoRow,
    ) -> Result<MapBundleWriteRows, String> {
        let all_bundle_paths = self
            .store
            .get_bundle_infos()?
            .into_iter()
            .map(|row| row.path)
            .collect::<Vec<_>>();
        let path_match_cache = BundlePathMatcher::build_cache(&all_bundle_paths);
        let path = Path::new(&bundle.path);
        let entry = if Self::looks_like_serialized_file(&bundle.path, bundle.file_size)? {
            MapBundleScanner::scan_serialized_file_from_path_with_relations(
                path,
                bundle.file_size,
                bundle.md5.clone(),
                true,
            )?
        } else {
            MapBundleScanner::scan_from_path_with_relations(
                path,
                bundle.file_size,
                bundle.md5.clone(),
                true,
            )?
        };

        Ok(Self::write_rows_from_bundle_entry(
            BundleFileEntry {
                name: path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_string(),
                path: bundle.path.clone(),
                size: bundle.file_size,
                modified_ms: bundle.modified_ms,
                modified: String::new(),
            },
            entry,
            &path_match_cache,
        ))
    }

    fn write_rows_from_bundle_entry(
        file_entry: BundleFileEntry,
        bundle_entry: crate::common::asset_map::asset_map_types::BundleEntry,
        path_match_cache: &crate::common::asset_map::path_matcher::BundlePathMatchCache,
    ) -> MapBundleWriteRows {
        let bundle_path = file_entry.path.clone();
        let asset_count = bundle_entry.assets.len();
        let assets = bundle_entry
            .assets
            .into_iter()
            .map(|asset| AssetWriteRow {
                bundle_path: bundle_path.clone(),
                path_id: asset.path_id,
                class_id: asset.class_id,
                class_name: asset.class_name.into_owned(),
                asset_name: asset.asset_name,
                byte_size: asset.byte_size,
            })
            .collect();
        let containers = bundle_entry
            .containers
            .iter()
            .map(|(asset_path, path_id)| ContainerWriteRow {
                bundle_path: bundle_path.clone(),
                asset_path: asset_path.clone(),
                path_id: *path_id,
            })
            .collect();
        let externals = bundle_entry
            .externals
            .iter()
            .map(|(sf_index, file_id, path_name)| ExternalWriteRow {
                bundle_path: bundle_path.clone(),
                sf_index: *sf_index,
                file_id: *file_id,
                path_name: path_name.clone(),
            })
            .collect();
        let internal_names = bundle_entry
            .internal_names
            .iter()
            .map(|(name, kind)| BundleInternalNameWriteRow {
                bundle_path: bundle_path.clone(),
                name: name.clone(),
                kind: kind.clone(),
            })
            .collect();
        let target_bundle_path_by_file_id: HashMap<i32, String> = bundle_entry
            .externals
            .iter()
            .filter_map(|(_, file_id, path_name)| {
                BundlePathMatcher::match_assetstudio_file_name(
                    path_name,
                    &path_match_cache.bundle_path_by_file_name,
                )
                .or_else(|| {
                    BundlePathMatcher::match_path(
                        path_name,
                        &path_match_cache.exact_bundle_path_by_normalized_path,
                        &path_match_cache.bundle_path_by_file_name,
                        &path_match_cache.normalized_bundle_paths,
                    )
                })
                .map(|matched_bundle_path| (*file_id, matched_bundle_path))
            })
            .collect();
        let relations = bundle_entry
            .relations
            .into_iter()
            .map(|relation| {
                let target_bundle_path = BundlePathMatcher::relation_target_bundle_path(
                    &bundle_path,
                    &relation,
                    &target_bundle_path_by_file_id,
                    &path_match_cache.exact_bundle_path_by_normalized_path,
                    &path_match_cache.bundle_path_by_file_name,
                    &path_match_cache.normalized_bundle_paths,
                );
                RelationWriteRow {
                    bundle_path: bundle_path.clone(),
                    relation_type: relation.relation_type.into_owned(),
                    source_path_id: relation.source_path_id,
                    target_path_id: relation.target_path_id,
                    source_name: relation.source_name,
                    target_name: relation.target_name,
                    file_id: relation.file_id,
                    field_path: relation.field_path.into_owned(),
                    target_bundle_path,
                }
            })
            .collect();

        MapBundleWriteRows {
            bundle_path,
            md5: bundle_entry.md5,
            file_size: file_entry.size,
            modified_ms: file_entry.modified_ms,
            unity_version: bundle_entry.unity_version,
            asset_count,
            assets,
            containers,
            externals,
            internal_names,
            relations,
        }
    }

    fn looks_like_serialized_file(path: &str, file_size: u64) -> Result<bool, String> {
        if file_size < 20 {
            return Ok(false);
        }
        let mut file = fs::File::open(path).map_err(|e| format!("open '{}': {}", path, e))?;
        let mut header = [0u8; 20];
        use std::io::Read;
        file.read_exact(&mut header)
            .map_err(|e| format!("read serialized header '{}': {}", path, e))?;
        let metadata_size_be = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let file_size_be = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
        let version_be = u32::from_be_bytes([header[8], header[9], header[10], header[11]]);
        let data_offset_be = u32::from_be_bytes([header[12], header[13], header[14], header[15]]);
        Ok((6..=30).contains(&version_be)
            && metadata_size_be > 0
            && metadata_size_be < file_size as u32
            && (file_size_be == file_size as u32 || file_size_be == 0)
            && data_offset_be < file_size as u32)
    }

    // ----- class stats -----

    pub fn class_stats_for_all_assets(&self) -> Result<Vec<(String, usize)>, String> {
        self.store.class_stats_for_all_assets()
    }

    pub fn class_stats_for_bundle(
        &self,
        bundle_path: &str,
    ) -> Result<Vec<(String, usize)>, String> {
        self.store.class_stats_for_bundle(bundle_path)
    }

    // ----- paginated queries -----

    pub fn count_assets_for_all_assets(
        &self,
        class_name: Option<&str>,
        search: Option<&str>,
        cancel_token: &Arc<AtomicBool>,
    ) -> Result<usize, String> {
        self.store.count_assets(class_name, search, cancel_token)
    }

    pub fn query_assets_for_all_assets(
        &self,
        class_name: Option<&str>,
        search: Option<&str>,
        offset: usize,
        limit: usize,
        cancel_token: &Arc<AtomicBool>,
    ) -> Result<Vec<AssetDisplayRow>, String> {
        self.store
            .query_assets(class_name, search, offset, limit, cancel_token)
    }

    // ----- texture / animation search -----

    pub fn search_texture_candidates_by_text(
        &self,
        search_terms: &[String],
        limit: usize,
    ) -> Result<Vec<TextureCandidateRow>, String> {
        self.store
            .search_texture_candidates_by_text(search_terms, limit)
    }

    pub fn search_texture_candidates_by_prefixes(
        &self,
        prefixes: &[String],
        limit: usize,
    ) -> Result<Vec<TextureCandidateRow>, String> {
        self.store
            .search_texture_candidates_by_prefixes(prefixes, limit)
    }

    pub fn search_material_candidates_by_identity(
        &self,
        identities: &[String],
        limit: usize,
    ) -> Result<Vec<AssetDisplayRow>, String> {
        self.store
            .search_material_candidates_by_identity(identities, limit)
    }

    pub fn find_animation_clips_by_text_tokens(
        &self,
        tokens: &[&str],
        bundle_path: Option<&str>,
    ) -> Result<Vec<AssetDisplayRow>, String> {
        let token_strings = tokens
            .iter()
            .map(|token| (*token).to_string())
            .collect::<Vec<_>>();
        self.store
            .find_animation_clips_by_text_tokens(&token_strings, bundle_path)
    }

    // ----- write operations (used by build process) -----

    pub fn insert_many_map_bundle_write_rows(
        &self,
        items: &[MapBundleWriteRows],
    ) -> Result<usize, String> {
        let mut written = 0usize;
        for rows in items {
            written += self.store.insert_bundle(rows)?;
        }
        let cache_rows = items.iter().collect::<Vec<_>>();
        AssetMapParseCacheStore::open(&self.workspace_path, self.cache_root.as_deref())?
            .put_many_refs(&cache_rows, true)?;
        Self::invalidate_relation_snapshot();
        Ok(written)
    }

    pub fn insert_many_map_bundle_write_rows_for_rebuild(
        &self,
        items: &[MapBundleWriteRows],
    ) -> Result<usize, String> {
        let mut written = 0usize;
        for rows in items {
            written += self.store.insert_bundle(rows)?;
        }
        let cache_rows = items.iter().collect::<Vec<_>>();
        AssetMapParseCacheStore::open(&self.workspace_path, self.cache_root.as_deref())?
            .put_many_refs(&cache_rows, true)?;
        Self::invalidate_relation_snapshot();
        Ok(written)
    }

    pub fn begin_batch_incremental(&self) -> Result<HashMap<String, BundleInfoRow>, String> {
        Ok(self
            .get_bundle_infos()?
            .into_iter()
            .map(|row| (row.path.clone(), row))
            .collect())
    }

    pub fn delete_bundle(&self, bundle_path: &str) -> Result<(), String> {
        self.store.delete_bundle(bundle_path)
    }

    pub fn delete_bundles(&self, bundle_paths: &[&str]) -> Result<(), String> {
        for path in bundle_paths {
            self.store.delete_bundle(path)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn replace_bundle_entry(
        &self,
        bundle_path: &str,
        md5: &str,
        file_size: u64,
        modified_ms: u64,
        unity_version: &str,
        assets: &[(&str, i64, i32, &str, &str, u32)],
        containers: &[(&str, &str, i64)],
        externals: &[(&str, usize, i32, &str)],
        relations: &[(
            &str,
            &str,
            i64,
            i64,
            &str,
            &str,
            i32,
            &str,
            std::borrow::Cow<'_, str>,
        )],
    ) -> Result<(), String> {
        let asset_rows: Vec<AssetWriteRow> = assets
            .iter()
            .map(|(bp, pid, cid, cn, an, bs)| AssetWriteRow {
                bundle_path: bp.to_string(),
                path_id: *pid,
                class_id: *cid,
                class_name: cn.to_string(),
                asset_name: an.to_string(),
                byte_size: *bs,
            })
            .collect();
        let container_rows: Vec<ContainerWriteRow> = containers
            .iter()
            .map(|(bp, ap, pid)| ContainerWriteRow {
                bundle_path: bp.to_string(),
                asset_path: ap.to_string(),
                path_id: *pid,
            })
            .collect();
        let external_rows: Vec<ExternalWriteRow> = externals
            .iter()
            .map(|(bp, si, fid, pn)| ExternalWriteRow {
                bundle_path: bp.to_string(),
                sf_index: *si,
                file_id: *fid,
                path_name: pn.to_string(),
            })
            .collect();
        let relation_rows: Vec<RelationWriteRow> = relations
            .iter()
            .map(|(bp, rt, sp, tp, sn, tn, fid, fp, tbp)| RelationWriteRow {
                bundle_path: bp.to_string(),
                relation_type: rt.to_string(),
                source_path_id: *sp,
                target_path_id: *tp,
                source_name: sn.to_string(),
                target_name: tn.to_string(),
                file_id: *fid,
                field_path: fp.to_string(),
                target_bundle_path: tbp.to_string(),
            })
            .collect();

        let rows = MapBundleWriteRows {
            bundle_path: bundle_path.to_string(),
            md5: md5.to_string(),
            file_size,
            modified_ms,
            unity_version: unity_version.to_string(),
            asset_count: asset_rows.len(),
            assets: asset_rows,
            containers: container_rows,
            externals: external_rows,
            internal_names: Vec::new(),
            relations: relation_rows,
        };

        self.store.insert_or_replace_bundle(&rows)?;
        AssetMapParseCacheStore::open(&self.workspace_path, self.cache_root.as_deref())?
            .put_many_refs(&[&rows], true)?;
        Self::invalidate_relation_snapshot();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn replace_bundle_entry_no_transaction(
        &self,
        bundle_path: &str,
        md5: &str,
        file_size: u64,
        modified_ms: u64,
        unity_version: &str,
        assets: &[(&str, i64, i32, &str, &str, u32)],
        containers: &[(&str, &str, i64)],
        externals: &[(&str, usize, i32, &str)],
        relations: &[(
            &str,
            &str,
            i64,
            i64,
            &str,
            &str,
            i32,
            &str,
            std::borrow::Cow<'_, str>,
        )],
    ) -> Result<(), String> {
        self.replace_bundle_entry(
            bundle_path,
            md5,
            file_size,
            modified_ms,
            unity_version,
            assets,
            containers,
            externals,
            relations,
        )
    }

    pub fn with_immediate_transaction<F>(&self, f: F) -> Result<(), String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        f()
    }

    // ----- build session access -----

    pub fn create_build_session(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<SqliteBuildSession, String> {
        SqliteBuildSession::create(workspace, cache_root)
    }

    pub fn store(&self) -> &SqliteStore {
        &self.store
    }
}

// ============================================================
// Utility helpers (previously at bottom of file)
// ============================================================

fn current_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
