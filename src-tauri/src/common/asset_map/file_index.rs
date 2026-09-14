use crate::common::asset_map::asset_index::{
    BundleInternalNameWriteRow, ContainerWriteRow, ExternalWriteRow, MapBundleWriteRows,
    RelationWriteRow,
};
use crate::common::asset_map::asset_map_types::{MapAssetClassStat, MapSummary};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const INDEX_DIR: &str = "asset_index";
const INDEX_VERSION: u32 = 3;
const RELATION_TARGET_WRITER_CACHE_SIZE: usize = 256;

static ASSET_RECORDS_CACHE: Lazy<Mutex<Option<AssetRecordsCache>>> = Lazy::new(|| Mutex::new(None));

#[derive(Debug, Clone)]
struct AssetRecordsCache {
    key: String,
    records: Arc<Vec<IndexedAssetRecord>>,
}

#[derive(Debug, Clone)]
pub struct AssetRecordsSnapshot {
    pub revision: String,
    pub records: Arc<Vec<IndexedAssetRecord>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetIndexCurrentPointer {
    pub version: u32,
    pub generation: String,
    pub published_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetIndexManifest {
    pub version: u32,
    pub built_at: u64,
    pub built_at_formatted: String,
    pub bundle_count: usize,
    pub asset_count: usize,
    pub parsed_count: usize,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetIndexBundleCatalogEntry {
    pub bundle_path: String,
    pub file_name: String,
    pub file_size: u64,
    pub modified_ms: u64,
    pub modified: String,
    pub unity_version: String,
    pub asset_count: usize,
    pub shard_rel_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedAssetRecord {
    pub bundle_path: String,
    pub path_id: i64,
    pub class_id: i32,
    pub class_name: String,
    pub asset_name: String,
    pub asset_path: String,
    pub byte_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileIndexBundleShard {
    #[serde(default = "FileIndexBundleShard::current_version")]
    pub version: u32,
    pub bundle_path: String,
    pub md5: String,
    pub file_size: u64,
    pub modified_ms: u64,
    pub unity_version: String,
    pub asset_count: usize,
    pub assets: Vec<IndexedAssetRecord>,
    pub containers: Vec<ContainerWriteRow>,
    pub externals: Vec<ExternalWriteRow>,
    pub internal_names: Vec<BundleInternalNameWriteRow>,
    pub relations: Vec<RelationWriteRow>,
}

impl FileIndexBundleShard {
    fn current_version() -> u32 {
        1
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepRelationShard {
    pub version: u32,
    pub bundle_path: String,
    pub file_size: u64,
    pub modified_ms: u64,
    pub fingerprint: String,
    pub relations: Vec<RelationWriteRow>,
}

impl DeepRelationShard {
    pub fn new(
        bundle_path: String,
        file_size: u64,
        modified_ms: u64,
        fingerprint: String,
        relations: Vec<RelationWriteRow>,
    ) -> Self {
        Self {
            version: 1,
            bundle_path,
            file_size,
            modified_ms,
            fingerprint,
            relations,
        }
    }

    pub fn matches(&self, bundle_path: &str, file_size: u64, modified_ms: u64) -> bool {
        self.bundle_path == bundle_path
            && self.file_size == file_size
            && self.modified_ms == modified_ms
    }
}

pub struct FileIndexBuildSession {
    paths: FileIndexPaths,
    previous_paths: Option<FileIndexPaths>,
    pointer_path: PathBuf,
    assets_writer: BufWriter<File>,
    relation_target_writers: HashMap<u16, BufWriter<File>>,
    bundle_catalog: Vec<AssetIndexBundleCatalogEntry>,
    class_stats: HashMap<String, usize>,
    class_bundle_paths: HashMap<String, BTreeSet<String>>,
    asset_count: usize,
}

#[derive(Debug, Clone)]
pub struct FileIndexPaths {
    pub control_dir: PathBuf,
    pub root_dir: PathBuf,
    pub bundles_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub bundle_catalog_path: PathBuf,
    pub class_stats_path: PathBuf,
    pub class_bundle_paths_path: PathBuf,
    pub assets_path: PathBuf,
    pub relation_targets_dir: PathBuf,
    pub deep_relations_dir: PathBuf,
}

impl FileIndexPaths {
    pub fn new(workspace: &Path, cache_root: Option<&Path>) -> Self {
        let control_dir = match cache_root {
            Some(root) => {
                crate::common::asset_map::asset_index::AssetDatabase::workspace_cache_dir(
                    root, workspace,
                )
                .join(INDEX_DIR)
            }
            None => workspace.join(INDEX_DIR),
        };
        if let Some(current_paths) = Self::current_from_control_dir(&control_dir) {
            current_paths
        } else {
            Self::from_root_dir(control_dir.clone(), control_dir)
        }
    }

    fn from_root_dir(control_dir: PathBuf, root_dir: PathBuf) -> Self {
        let bundles_dir = root_dir.join("bundles");
        let deep_relations_dir = root_dir.join("deep").join("relations");
        Self {
            control_dir,
            manifest_path: root_dir.join("manifest.json"),
            bundle_catalog_path: root_dir.join("bundle_catalog.json"),
            class_stats_path: root_dir.join("class_stats.json"),
            class_bundle_paths_path: root_dir.join("class_bundle_paths.json"),
            assets_path: root_dir.join("assets.jsonl"),
            relation_targets_dir: root_dir.join("relation_targets"),
            deep_relations_dir,
            root_dir,
            bundles_dir,
        }
    }

    fn control_dir(workspace: &Path, cache_root: Option<&Path>) -> PathBuf {
        match cache_root {
            Some(root) => {
                crate::common::asset_map::asset_index::AssetDatabase::workspace_cache_dir(
                    root, workspace,
                )
                .join(INDEX_DIR)
            }
            None => workspace.join(INDEX_DIR),
        }
    }

    fn generation(control_dir: &Path, generation: &str) -> Self {
        Self::from_root_dir(
            control_dir.to_path_buf(),
            control_dir.join("generations").join(generation),
        )
    }

    fn pointer_path(control_dir: &Path) -> PathBuf {
        control_dir.join("current.json")
    }

    fn current_from_control_dir(control_dir: &Path) -> Option<Self> {
        let pointer_path = Self::pointer_path(control_dir);
        let file = File::open(pointer_path).ok()?;
        let pointer: AssetIndexCurrentPointer =
            serde_json::from_reader(BufReader::new(file)).ok()?;
        let paths = Self::generation(control_dir, &pointer.generation);
        paths.manifest_path.exists().then_some(paths)
    }
}

impl FileIndexBuildSession {
    pub fn create(workspace: &Path, cache_root: Option<&Path>) -> Result<Self, String> {
        let control_dir = FileIndexPaths::control_dir(workspace, cache_root);
        let previous_paths_candidate = FileIndexPaths::new(workspace, cache_root);
        let previous_paths = previous_paths_candidate
            .manifest_path
            .exists()
            .then_some(previous_paths_candidate);
        fs::create_dir_all(control_dir.join("generations")).map_err(|e| {
            format!(
                "Failed to create asset index generation root {}: {}",
                control_dir.join("generations").display(),
                e
            )
        })?;
        let generation = format!(
            "gen-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let paths = FileIndexPaths::generation(&control_dir, &generation);
        if paths.root_dir.exists() {
            fs::remove_dir_all(&paths.root_dir).map_err(|e| {
                format!(
                    "Failed to clear asset index generation {}: {}",
                    paths.root_dir.display(),
                    e
                )
            })?;
        }
        fs::create_dir_all(&paths.bundles_dir).map_err(|e| {
            format!(
                "Failed to create asset index {}: {}",
                paths.bundles_dir.display(),
                e
            )
        })?;
        fs::create_dir_all(&paths.relation_targets_dir).map_err(|e| {
            format!(
                "Failed to create asset index {}: {}",
                paths.relation_targets_dir.display(),
                e
            )
        })?;
        fs::create_dir_all(&paths.deep_relations_dir).map_err(|e| {
            format!(
                "Failed to create deep relation cache {}: {}",
                paths.deep_relations_dir.display(),
                e
            )
        })?;
        if let Some(previous_paths) = &previous_paths {
            if previous_paths.deep_relations_dir.exists() {
                Self::copy_dir_recursive(
                    &previous_paths.deep_relations_dir,
                    &paths.deep_relations_dir,
                )?;
            }
        }
        let assets_writer = BufWriter::new(
            File::create(&paths.assets_path)
                .map_err(|e| format!("Failed to create {}: {}", paths.assets_path.display(), e))?,
        );
        Ok(Self {
            pointer_path: FileIndexPaths::pointer_path(&control_dir),
            paths,
            previous_paths,
            assets_writer,
            relation_target_writers: HashMap::new(),
            bundle_catalog: Vec::new(),
            class_stats: HashMap::new(),
            class_bundle_paths: HashMap::new(),
            asset_count: 0,
        })
    }

    pub fn append_bundle(&mut self, rows: &MapBundleWriteRows) -> Result<usize, String> {
        let shard_rel_path = format!(
            "bundles/{}.json",
            FileAssetIndex::bundle_key_for_path(&rows.bundle_path)
        );
        let shard_path = self.paths.root_dir.join(&shard_rel_path);
        let container_path_by_id = rows
            .containers
            .iter()
            .map(|row| (row.path_id, row.asset_path.clone()))
            .collect::<HashMap<_, _>>();
        let assets = rows
            .assets
            .iter()
            .map(|row| IndexedAssetRecord {
                bundle_path: row.bundle_path.clone(),
                path_id: row.path_id,
                class_id: row.class_id,
                class_name: row.class_name.clone(),
                asset_name: row.asset_name.clone(),
                asset_path: container_path_by_id
                    .get(&row.path_id)
                    .cloned()
                    .unwrap_or_else(|| format!("#{:x}", row.path_id)),
                byte_size: row.byte_size,
            })
            .collect::<Vec<_>>();

        let shard = FileIndexBundleShard {
            version: FileIndexBundleShard::current_version(),
            bundle_path: rows.bundle_path.clone(),
            md5: rows.md5.clone(),
            file_size: rows.file_size,
            modified_ms: rows.modified_ms,
            unity_version: rows.unity_version.clone(),
            asset_count: rows.asset_count,
            assets: assets.clone(),
            containers: rows.containers.clone(),
            externals: rows.externals.clone(),
            internal_names: rows.internal_names.clone(),
            relations: rows.relations.clone(),
        };

        let shard_file = File::create(&shard_path)
            .map_err(|e| format!("Failed to create {}: {}", shard_path.display(), e))?;
        serde_json::to_writer(BufWriter::new(shard_file), &shard)
            .map_err(|e| format!("Failed to write {}: {}", shard_path.display(), e))?;

        self.append_relation_target_rows(&rows.relations)?;

        for asset in assets {
            let line = serde_json::to_string(&asset)
                .map_err(|e| format!("Failed to serialize asset record: {}", e))?;
            self.assets_writer
                .write_all(line.as_bytes())
                .and_then(|_| self.assets_writer.write_all(b"\n"))
                .map_err(|e| format!("Failed to append asset index row: {}", e))?;
            *self
                .class_stats
                .entry(asset.class_name.clone())
                .or_insert(0) += 1;
            self.class_bundle_paths
                .entry(asset.class_name)
                .or_default()
                .insert(asset.bundle_path);
            self.asset_count += 1;
        }

        let file_name = Path::new(&rows.bundle_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        self.bundle_catalog.push(AssetIndexBundleCatalogEntry {
            bundle_path: rows.bundle_path.clone(),
            file_name,
            file_size: rows.file_size,
            modified_ms: rows.modified_ms,
            modified: String::new(),
            unity_version: rows.unity_version.clone(),
            asset_count: rows.asset_count,
            shard_rel_path,
        });

        Ok(rows.assets.len())
    }

    pub fn append_l1_shard(&mut self, shard: &FileIndexBundleShard) -> Result<usize, String> {
        let shard_rel_path = format!(
            "bundles/{}.json",
            FileAssetIndex::bundle_key_for_path(&shard.bundle_path)
        );
        let shard_path = self.paths.root_dir.join(&shard_rel_path);
        let mut l1_shard = shard.clone();
        l1_shard.version = FileIndexBundleShard::current_version();
        l1_shard.relations.clear();
        let shard_file = File::create(&shard_path)
            .map_err(|e| format!("Failed to create {}: {}", shard_path.display(), e))?;
        serde_json::to_writer(BufWriter::new(shard_file), &l1_shard)
            .map_err(|e| format!("Failed to write {}: {}", shard_path.display(), e))?;

        for asset in &l1_shard.assets {
            let line = serde_json::to_string(asset)
                .map_err(|e| format!("Failed to serialize asset record: {}", e))?;
            self.assets_writer
                .write_all(line.as_bytes())
                .and_then(|_| self.assets_writer.write_all(b"\n"))
                .map_err(|e| format!("Failed to append asset index row: {}", e))?;
            *self
                .class_stats
                .entry(asset.class_name.clone())
                .or_insert(0) += 1;
            self.class_bundle_paths
                .entry(asset.class_name.clone())
                .or_default()
                .insert(asset.bundle_path.clone());
            self.asset_count += 1;
        }

        let file_name = Path::new(&l1_shard.bundle_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        self.bundle_catalog.push(AssetIndexBundleCatalogEntry {
            bundle_path: l1_shard.bundle_path.clone(),
            file_name,
            file_size: l1_shard.file_size,
            modified_ms: l1_shard.modified_ms,
            modified: String::new(),
            unity_version: l1_shard.unity_version.clone(),
            asset_count: l1_shard.asset_count,
            shard_rel_path,
        });

        Ok(l1_shard.assets.len())
    }

    pub fn load_previous_bundle_catalog_map(
        &self,
    ) -> Result<HashMap<String, AssetIndexBundleCatalogEntry>, String> {
        let Some(paths) = &self.previous_paths else {
            return Ok(HashMap::new());
        };
        Ok(FileAssetIndex::load_bundle_catalog_from_paths(paths)?
            .into_iter()
            .map(|entry| (entry.bundle_path.clone(), entry))
            .collect())
    }

    pub fn previous_paths(&self) -> Option<FileIndexPaths> {
        self.previous_paths.clone()
    }

    pub fn finish(
        mut self,
        built_at: u64,
        built_at_formatted: String,
        parsed_count: usize,
        cancelled: bool,
    ) -> Result<MapSummary, String> {
        self.assets_writer
            .flush()
            .map_err(|e| format!("Failed to flush asset index rows: {}", e))?;
        for writer in self.relation_target_writers.values_mut() {
            writer
                .flush()
                .map_err(|e| format!("Failed to flush relation target index rows: {}", e))?;
        }

        self.bundle_catalog
            .sort_by(|left, right| left.file_name.cmp(&right.file_name));

        Self::write_json(&self.paths.bundle_catalog_path, &self.bundle_catalog)?;

        let mut class_stats = self
            .class_stats
            .into_iter()
            .map(|(name, count)| MapAssetClassStat { name, count })
            .collect::<Vec<_>>();
        class_stats.sort_by(|left, right| left.name.cmp(&right.name));
        Self::write_json(&self.paths.class_stats_path, &class_stats)?;

        let class_bundle_paths = self
            .class_bundle_paths
            .into_iter()
            .map(|(class_name, bundles)| (class_name, bundles.into_iter().collect::<Vec<_>>()))
            .collect::<HashMap<_, _>>();
        Self::write_json(&self.paths.class_bundle_paths_path, &class_bundle_paths)?;

        let summary = MapSummary {
            built_at,
            built_at_formatted: built_at_formatted.clone(),
            bundle_count: self.bundle_catalog.len(),
            asset_count: self.asset_count,
            parsed_count,
            cancelled,
        };
        let manifest = AssetIndexManifest {
            version: INDEX_VERSION,
            built_at,
            built_at_formatted,
            bundle_count: summary.bundle_count,
            asset_count: summary.asset_count,
            parsed_count,
            cancelled,
        };
        Self::write_json(&self.paths.manifest_path, &manifest)?;
        let pointer_path = self.pointer_path.clone();
        let generation = self
            .paths
            .root_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                format!(
                    "Invalid asset index generation path {}",
                    self.paths.root_dir.display()
                )
            })?
            .to_string();
        drop(self.assets_writer);
        drop(self.relation_target_writers);
        let pointer = AssetIndexCurrentPointer {
            version: 1,
            generation,
            published_at: built_at,
        };
        Self::write_json(&pointer_path, &pointer)?;
        FileAssetIndex::invalidate_memory_cache();
        let _ = Self::prune_old_generations(&self.paths.control_dir, &pointer.generation, 2);
        Ok(summary)
    }

    fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), String> {
        fs::create_dir_all(to).map_err(|e| format!("Failed to create {}: {}", to.display(), e))?;
        for entry in
            fs::read_dir(from).map_err(|e| format!("Failed to read {}: {}", from.display(), e))?
        {
            let entry = entry.map_err(|e| format!("Failed to read {}: {}", from.display(), e))?;
            let source_path = entry.path();
            let target_path = to.join(entry.file_name());
            let metadata = entry
                .metadata()
                .map_err(|e| format!("Failed to stat {}: {}", source_path.display(), e))?;
            if metadata.is_dir() {
                Self::copy_dir_recursive(&source_path, &target_path)?;
            } else if metadata.is_file() {
                fs::copy(&source_path, &target_path).map_err(|e| {
                    format!(
                        "Failed to copy {} -> {}: {}",
                        source_path.display(),
                        target_path.display(),
                        e
                    )
                })?;
            }
        }
        Ok(())
    }

    fn prune_old_generations(
        control_dir: &Path,
        active_generation: &str,
        keep_count: usize,
    ) -> Result<(), String> {
        let generations_dir = control_dir.join("generations");
        if !generations_dir.exists() {
            return Ok(());
        }
        let mut generations = Vec::new();
        for entry in fs::read_dir(&generations_dir)
            .map_err(|e| format!("Failed to read {}: {}", generations_dir.display(), e))?
        {
            let entry = entry
                .map_err(|e| format!("Failed to read {}: {}", generations_dir.display(), e))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            generations.push((name, path, modified));
        }
        generations.sort_by(|left, right| right.2.cmp(&left.2));

        let mut kept = 0usize;
        for (name, path, _) in generations {
            if name == active_generation {
                kept += 1;
                continue;
            }
            if kept < keep_count {
                kept += 1;
                continue;
            }
            let _ = fs::remove_dir_all(path);
        }
        Ok(())
    }

    fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
        let file = File::create(path)
            .map_err(|e| format!("Failed to create {}: {}", path.display(), e))?;
        serde_json::to_writer_pretty(BufWriter::new(file), value)
            .map_err(|e| format!("Failed to write {}: {}", path.display(), e))
    }

    fn append_relation_target_rows(&mut self, rows: &[RelationWriteRow]) -> Result<(), String> {
        for row in rows {
            let target_bundle_path = FileAssetIndex::relation_effective_target_bundle(row);
            let bucket_id = FileAssetIndex::relation_target_bucket_id(
                &row.relation_type,
                target_bundle_path,
                row.target_path_id,
            );
            if !self.relation_target_writers.contains_key(&bucket_id) {
                self.open_relation_target_writer(bucket_id)?;
            }
            let line = serde_json::to_string(row)
                .map_err(|e| format!("Failed to serialize relation target row: {}", e))?;
            let writer = self
                .relation_target_writers
                .get_mut(&bucket_id)
                .ok_or_else(|| format!("Missing relation target bucket writer {}", bucket_id))?;
            writer
                .write_all(line.as_bytes())
                .and_then(|_| writer.write_all(b"\n"))
                .map_err(|e| format!("Failed to append relation target index row: {}", e))?;
        }
        Ok(())
    }

    fn open_relation_target_writer(&mut self, bucket_id: u16) -> Result<(), String> {
        if self.relation_target_writers.len() >= RELATION_TARGET_WRITER_CACHE_SIZE {
            if let Some(evict_id) = self.relation_target_writers.keys().next().copied() {
                if let Some(mut writer) = self.relation_target_writers.remove(&evict_id) {
                    writer.flush().map_err(|e| {
                        format!(
                            "Failed to flush relation target bucket {:03x}: {}",
                            evict_id, e
                        )
                    })?;
                }
            }
        }
        let bucket_path = FileAssetIndex::relation_target_bucket_path(&self.paths, bucket_id);
        let writer = BufWriter::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&bucket_path)
                .map_err(|e| format!("Failed to open {}: {}", bucket_path.display(), e))?,
        );
        self.relation_target_writers.insert(bucket_id, writer);
        Ok(())
    }
}

pub struct FileAssetIndex;

impl FileAssetIndex {
    pub fn exists_with_cache_root(workspace: &Path, cache_root: Option<&Path>) -> bool {
        FileIndexPaths::new(workspace, cache_root)
            .manifest_path
            .exists()
    }

    pub fn clear_with_cache_root(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<(), String> {
        let control_dir = FileIndexPaths::control_dir(workspace, cache_root);
        if control_dir.exists() {
            fs::remove_dir_all(&control_dir)
                .map_err(|e| format!("Failed to delete {}: {}", control_dir.display(), e))?;
        }
        Self::invalidate_memory_cache();
        Ok(())
    }

    pub fn load_manifest(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<Option<AssetIndexManifest>, String> {
        let path = FileIndexPaths::new(workspace, cache_root).manifest_path;
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(Self::read_json(&path)?))
    }

    pub fn supports_relation_target_buckets(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<bool, String> {
        Ok(Self::load_manifest(workspace, cache_root)?
            .map(|manifest| manifest.version >= 3)
            .unwrap_or(false))
    }

    pub fn load_bundle_catalog(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<Vec<AssetIndexBundleCatalogEntry>, String> {
        Self::load_bundle_catalog_from_paths(&FileIndexPaths::new(workspace, cache_root))
    }

    pub fn load_bundle_catalog_from_paths(
        paths: &FileIndexPaths,
    ) -> Result<Vec<AssetIndexBundleCatalogEntry>, String> {
        let path = &paths.bundle_catalog_path;
        if !path.exists() {
            return Ok(Vec::new());
        }
        Self::read_json(&path)
    }

    pub fn load_bundle_catalog_map(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<HashMap<String, AssetIndexBundleCatalogEntry>, String> {
        Ok(Self::load_bundle_catalog(workspace, cache_root)?
            .into_iter()
            .map(|entry| (entry.bundle_path.clone(), entry))
            .collect())
    }

    pub fn load_class_stats(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<Vec<MapAssetClassStat>, String> {
        let path = FileIndexPaths::new(workspace, cache_root).class_stats_path;
        if !path.exists() {
            return Ok(Vec::new());
        }
        Self::read_json(&path)
    }

    pub fn load_class_bundle_paths(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<HashMap<String, Vec<String>>, String> {
        let path = FileIndexPaths::new(workspace, cache_root).class_bundle_paths_path;
        if !path.exists() {
            return Ok(HashMap::new());
        }
        Self::read_json(&path)
    }

    pub fn load_bundle_shard_by_path(
        workspace: &Path,
        bundle_path: &str,
        cache_root: Option<&Path>,
    ) -> Result<Option<FileIndexBundleShard>, String> {
        let catalog = Self::load_bundle_catalog_map(workspace, cache_root)?;
        let Some(entry) = catalog.get(bundle_path) else {
            return Ok(None);
        };
        let paths = FileIndexPaths::new(workspace, cache_root);
        let shard_path = paths.root_dir.join(&entry.shard_rel_path);
        Ok(Some(Self::read_json(&shard_path)?))
    }

    pub fn load_bundle_shard_by_catalog_entry(
        workspace: &Path,
        entry: &AssetIndexBundleCatalogEntry,
        cache_root: Option<&Path>,
    ) -> Result<FileIndexBundleShard, String> {
        let paths = FileIndexPaths::new(workspace, cache_root);
        Self::load_bundle_shard_from_paths(&paths, entry)
    }

    pub fn load_bundle_shard_from_paths(
        paths: &FileIndexPaths,
        entry: &AssetIndexBundleCatalogEntry,
    ) -> Result<FileIndexBundleShard, String> {
        Self::read_json(&paths.root_dir.join(&entry.shard_rel_path))
    }

    pub fn read_asset_records_arc(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<Arc<Vec<IndexedAssetRecord>>, String> {
        Ok(Self::read_asset_records_snapshot(workspace, cache_root)?.records)
    }

    pub fn read_asset_records_snapshot(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<AssetRecordsSnapshot, String> {
        let path = FileIndexPaths::new(workspace, cache_root).assets_path;
        if !path.exists() {
            return Ok(AssetRecordsSnapshot {
                revision: format!("{}:missing", path.display()),
                records: Arc::new(Vec::new()),
            });
        }
        let cache_key = Self::asset_records_cache_key(&path)?;
        if let Some(records) = Self::cached_asset_records(&cache_key)? {
            return Ok(AssetRecordsSnapshot {
                revision: cache_key,
                records,
            });
        }
        let file =
            File::open(&path).map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
            if line.trim().is_empty() {
                continue;
            }
            let record = serde_json::from_str(&line)
                .map_err(|e| format!("Failed to parse asset index row: {}", e))?;
            records.push(record);
        }
        let records = Arc::new(records);
        let mut cache = ASSET_RECORDS_CACHE
            .lock()
            .map_err(|_| "Failed to lock AssetMap asset record cache".to_string())?;
        *cache = Some(AssetRecordsCache {
            key: cache_key.clone(),
            records: records.clone(),
        });
        Ok(AssetRecordsSnapshot {
            revision: cache_key,
            records,
        })
    }

    pub fn invalidate_memory_cache() {
        if let Ok(mut cache) = ASSET_RECORDS_CACHE.lock() {
            *cache = None;
        }
    }

    fn cached_asset_records(
        cache_key: &str,
    ) -> Result<Option<Arc<Vec<IndexedAssetRecord>>>, String> {
        let cache = ASSET_RECORDS_CACHE
            .lock()
            .map_err(|_| "Failed to lock AssetMap asset record cache".to_string())?;
        Ok(cache
            .as_ref()
            .filter(|cache| cache.key == cache_key)
            .map(|cache| cache.records.clone()))
    }

    fn asset_records_cache_key(path: &Path) -> Result<String, String> {
        let metadata =
            fs::metadata(path).map_err(|e| format!("Failed to stat {}: {}", path.display(), e))?;
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        Ok(format!(
            "{}:{}:{}",
            path.display(),
            metadata.len(),
            modified_ms
        ))
    }

    pub fn read_relation_target_records(
        workspace: &Path,
        relation_type: &str,
        target_bundle_path: &str,
        target_path_id: i64,
        cache_root: Option<&Path>,
    ) -> Result<Vec<RelationWriteRow>, String> {
        let paths = FileIndexPaths::new(workspace, cache_root);
        let bucket_id =
            Self::relation_target_bucket_id(relation_type, target_bundle_path, target_path_id);
        let path = Self::relation_target_bucket_path(&paths, bucket_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file =
            File::open(&path).map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
            if line.trim().is_empty() {
                continue;
            }
            let record = serde_json::from_str(&line)
                .map_err(|e| format!("Failed to parse relation target index row: {}", e))?;
            records.push(record);
        }
        Ok(records)
    }

    pub fn read_deep_relation_shard(
        workspace: &Path,
        bundle_path: &str,
        cache_root: Option<&Path>,
    ) -> Result<Option<DeepRelationShard>, String> {
        let paths = FileIndexPaths::new(workspace, cache_root);
        let path = Self::deep_relation_path(&paths, bundle_path);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(Self::read_json(&path)?))
    }

    pub fn read_deep_relation_records(
        workspace: &Path,
        bundle_path: &str,
        file_size: u64,
        modified_ms: u64,
        cache_root: Option<&Path>,
    ) -> Result<Option<Vec<RelationWriteRow>>, String> {
        let Some(shard) = Self::read_deep_relation_shard(workspace, bundle_path, cache_root)?
        else {
            return Ok(None);
        };
        if !shard.matches(bundle_path, file_size, modified_ms) {
            return Ok(None);
        }
        Ok(Some(shard.relations))
    }

    pub fn write_deep_relation_records(
        workspace: &Path,
        bundle_path: &str,
        file_size: u64,
        modified_ms: u64,
        fingerprint: String,
        rows: &[RelationWriteRow],
        cache_root: Option<&Path>,
    ) -> Result<(), String> {
        let paths = FileIndexPaths::new(workspace, cache_root);
        fs::create_dir_all(&paths.deep_relations_dir).map_err(|e| {
            format!(
                "Failed to create deep relation cache {}: {}",
                paths.deep_relations_dir.display(),
                e
            )
        })?;
        let path = Self::deep_relation_path(&paths, bundle_path);
        let file = File::create(&path)
            .map_err(|e| format!("Failed to create {}: {}", path.display(), e))?;
        let shard = DeepRelationShard::new(
            bundle_path.to_string(),
            file_size,
            modified_ms,
            fingerprint,
            rows.to_vec(),
        );
        serde_json::to_writer(BufWriter::new(file), &shard)
            .map_err(|e| format!("Failed to write {}: {}", path.display(), e))
    }

    fn relation_effective_target_bundle(row: &RelationWriteRow) -> &str {
        if row.target_bundle_path.is_empty() {
            &row.bundle_path
        } else {
            &row.target_bundle_path
        }
    }

    fn relation_target_bucket_path(paths: &FileIndexPaths, bucket_id: u16) -> PathBuf {
        paths
            .relation_targets_dir
            .join(format!("{:03x}.jsonl", bucket_id))
    }

    fn relation_target_bucket_id(
        relation_type: &str,
        target_bundle_path: &str,
        target_path_id: i64,
    ) -> u16 {
        use md5::Digest;
        let mut hasher = md5::Md5::new();
        hasher.update(relation_type.as_bytes());
        hasher.update(b"\0");
        hasher.update(target_bundle_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(target_path_id.to_le_bytes());
        let digest = hasher.finalize();
        u16::from_le_bytes([digest[0], digest[1]]) & 0x0fff
    }

    pub fn bundle_key_for_path(bundle_path: &str) -> String {
        use md5::Digest;
        let mut hasher = md5::Md5::new();
        hasher.update(bundle_path.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    fn deep_relation_path(paths: &FileIndexPaths, bundle_path: &str) -> PathBuf {
        paths
            .deep_relations_dir
            .join(format!("{}.json", Self::bundle_key_for_path(bundle_path)))
    }

    fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
        let file =
            File::open(path).map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
        serde_json::from_reader(BufReader::new(file))
            .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))
    }
}
