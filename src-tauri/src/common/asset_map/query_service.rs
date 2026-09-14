use crate::common::asset_map::asset_index::AssetDatabase;
use crate::common::asset_map::asset_map_types::{
    AssetSearchIndexStatus, MapAssetClassStat, MapAssetQueryOptions, MapAssetQueryResult,
    MapAssetSummary, MapSummary,
};
use crate::common::bundle_file::bundle_types::BundleFileEntry;
use crate::common::task::task_context::TaskContext;
use crate::common::task::task_logger::TaskLogger;
use crate::utils::time_utils::TimeUtils;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub struct AssetMapQueryService;

impl AssetMapQueryService {
    pub fn exists_with_cache_root(workspace: &Path, cache_root: Option<&Path>) -> bool {
        AssetDatabase::exists_with_cache_root(workspace, cache_root)
    }

    pub fn summary_with_cache_root(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Option<MapSummary> {
        let db = AssetDatabase::open_with_cache_root(workspace, cache_root).ok()?;
        let built_at = db.built_at().ok()?;
        Some(MapSummary {
            built_at,
            built_at_formatted: Self::format_built_at(built_at),
            bundle_count: db.bundle_count().ok()?,
            asset_count: db.asset_count().ok()?,
            parsed_count: db.parsed_count().ok()?,
            cancelled: db.build_cancelled().ok()?,
        })
    }

    pub fn bundle_paths_for_classes_with_cache_root(
        workspace: &Path,
        class_names: &[String],
        cache_root: Option<&Path>,
    ) -> Result<HashMap<String, Vec<String>>, String> {
        let db = AssetDatabase::open_with_cache_root(workspace, cache_root)?;
        let mut result = HashMap::new();
        for class_name in class_names {
            result.insert(
                class_name.clone(),
                db.find_bundle_paths_by_class(class_name)?,
            );
        }
        Ok(result)
    }

    pub fn query_assets_with_cache_root(
        workspace: &Path,
        options: &MapAssetQueryOptions,
        cache_root: Option<&Path>,
    ) -> Result<MapAssetQueryResult, String> {
        Self::query_assets_with_cache_root_cancelable(
            workspace,
            options,
            cache_root,
            Arc::new(AtomicBool::new(false)),
        )
    }

    pub fn query_assets_with_cache_root_cancelable(
        workspace: &Path,
        options: &MapAssetQueryOptions,
        cache_root: Option<&Path>,
        cancel_token: Arc<AtomicBool>,
    ) -> Result<MapAssetQueryResult, String> {
        let db = AssetDatabase::open_with_cache_root(workspace, cache_root)?;
        let total = db.store().count_assets_by_options(options, &cancel_token)?;
        let assets = db
            .store()
            .query_assets_by_options(options, &cancel_token)?
            .into_iter()
            .map(|row| MapAssetSummary {
                path: row.asset_path,
                name: row.asset.asset_name,
                class_name: row.asset.class_name,
                class_id: row.asset.class_id,
                path_id: row.asset.path_id.to_string(),
                byte_size: row.asset.byte_size,
                source_bundle_path: row.asset.bundle_path,
            })
            .collect::<Vec<_>>();
        let end = options.offset.saturating_add(assets.len()).min(total);
        Ok(MapAssetQueryResult {
            assets,
            total,
            has_more: end < total,
            offset: options.offset,
            limit: options.limit.max(1),
        })
    }

    pub fn asset_search_index_status_with_cache_root(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<AssetSearchIndexStatus, String> {
        let db = AssetDatabase::open_with_cache_root(workspace, cache_root)?;
        db.store().asset_search_index_status()
    }

    pub fn build_asset_search_index_with_cache_root(
        workspace: &Path,
        cache_root: Option<&Path>,
        task_ctx: &TaskContext,
    ) -> Result<AssetSearchIndexStatus, String> {
        let db = AssetDatabase::open_with_cache_root(workspace, cache_root)?;
        let status = db.store().asset_search_index_status()?;
        if status.ready {
            task_ctx.success(format!(
                "All Assets trigram search index already ready: {}/{} assets",
                status.indexed_count, status.asset_count
            ));
            return Ok(status);
        }
        let status = db.store().build_asset_search_index(Some(task_ctx))?;
        Self::invalidate_query_cache();
        Ok(status)
    }

    pub fn all_asset_class_stats_with_cache_root(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<Vec<MapAssetClassStat>, String> {
        let db = AssetDatabase::open_with_cache_root(workspace, cache_root)?;
        Ok(db
            .class_stats_for_all_assets()?
            .into_iter()
            .map(|(name, count)| MapAssetClassStat { name, count })
            .collect())
    }

    pub fn bundle_asset_class_stats_with_cache_root(
        workspace: &Path,
        bundle_path: &Path,
        cache_root: Option<&Path>,
    ) -> Result<Vec<MapAssetClassStat>, String> {
        let db = AssetDatabase::open_with_cache_root(workspace, cache_root)?;
        let mut stats = db
            .class_stats_for_bundle(&bundle_path.to_string_lossy())?
            .into_iter()
            .map(|(name, count)| MapAssetClassStat { name, count })
            .collect::<Vec<_>>();
        stats.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(stats)
    }

    pub fn cached_bundle_files_with_cache_root(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<Vec<BundleFileEntry>, String> {
        let db = AssetDatabase::open_with_cache_root(workspace, cache_root)?;
        let mut entries = db
            .get_bundle_infos()?
            .into_iter()
            .map(|row| BundleFileEntry {
                name: Path::new(&row.path)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("")
                    .to_string(),
                path: row.path,
                size: row.file_size,
                modified_ms: row.modified_ms,
                modified: Self::format_built_at(row.modified_ms / 1000),
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    pub fn clear_with_cache_root(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<(), String> {
        match cache_root {
            Some(root) => {
                let cache_dir = AssetDatabase::workspace_cache_dir(root, workspace);
                if cache_dir.exists() {
                    TaskLogger::info(
                        "clear_map",
                        "Clear Map",
                        &format!("Deleting entire workspace cache: {}", cache_dir.display()),
                    );
                    Self::remove_dir_all_with_retry(&cache_dir)?;
                } else {
                    TaskLogger::info(
                        "clear_map",
                        "Clear Map",
                        "workspace cache dir does not exist, nothing to delete",
                    );
                }
            }
            None => {
                // Fallback: delete only the asset index db
                if AssetDatabase::exists_with_cache_root(workspace, cache_root) {
                    AssetDatabase::delete_files_with_cache_root(workspace, cache_root)?;
                }
            }
        }
        Self::invalidate_query_cache();
        Ok(())
    }

    pub fn format_built_at(unix_secs: u64) -> String {
        let sys_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix_secs);
        TimeUtils::format_time(sys_time)
    }

    pub fn invalidate_query_cache() {}

    /// Delete a directory with retry logic to handle transient file locks
    /// (e.g. SQLite WAL/SHM files still held by the OS after connection close on Windows).
    fn remove_dir_all_with_retry(dir: &Path) -> Result<(), String> {
        let mut last_err = None;
        for attempt in 0..5 {
            match std::fs::remove_dir_all(dir) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last_err = Some(e);
                    if attempt < 4 {
                        std::thread::sleep(std::time::Duration::from_millis(200 * (attempt + 1)));
                    }
                }
            }
        }
        Err(format!(
            "Failed to delete cache dir {} after 5 attempts: {}",
            dir.display(),
            last_err.unwrap()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalidate_query_cache_is_stable() {
        AssetMapQueryService::invalidate_query_cache();
    }
}
