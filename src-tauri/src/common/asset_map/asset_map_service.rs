/*
 * asset_map_service.rs Global Asset Index (AssetMap) Service
 *
 * Uses a sharded file index to store AssetMap data.
 * Incremental updates: parallel parsing during build, results streamed into shard files.
 * Queries scan compact index files without loading bundle payloads into memory.
 *
 * Follows Soul.md conventions: organized via struct + impl, no free functions.
 */

use crate::common::asset_map::asset_index::{
    AssetDatabase, AssetWriteRow, BundleInternalNameWriteRow, ContainerWriteRow, ExternalWriteRow,
    MapBundleWriteRows, RelationWriteRow,
};
use crate::common::asset_map::asset_map_types::{
    AssetEntry, BundleEntry, BundleRelationEntry, MapAssetClassStat, MapAssetQueryOptions,
    MapAssetQueryResult, MapSummary,
};
use crate::common::asset_map::build_support::{AssetMapBuildSupport, BuildMapMemoryLimiter};
use crate::common::asset_map::map_bundle_scanner::MapBundleScanner;
use crate::common::asset_map::material_texture_resolver::MaterialTextureResolver;
use crate::common::asset_map::parse_cache_store::AssetMapParseCacheStore;
use crate::common::asset_map::path_matcher::BundlePathMatcher;
use crate::common::asset_map::query_service::AssetMapQueryService;
use crate::common::asset_map::unity_relation_collector::UnityRelationCollector;
use crate::common::bundle_file::asset_bundle::AssetBundleLoader;
use crate::common::bundle_file::bundle_types::BundleFileEntry;
use crate::common::bundle_file::bundle_utils::BundleFileCollector;
use crate::common::bundle_file::ress_list_cache::ResSListCache;
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::task::task_logger::TaskLogger;
use crate::exporter::material_info::MaterialInfo;
use crate::utils::time_utils::TimeUtils;
use crate::utils::unity_object_name_utils::UnityObjectNameUtils;
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Condvar;

/// 简单 I/O 并发限制信号量。
/// 限制同时读盘的 worker 数量，避免 HDD/网络盘上 24 线程争抢导致磁盘头抖动。
struct IoPermit {
    count: Mutex<u32>,
    cvar: Condvar,
}

impl IoPermit {
    fn new() -> Self {
        IoPermit {
            count: Mutex::new(0),
            cvar: Condvar::new(),
        }
    }

    /// Lock the permit counter, tolerating a poisoned mutex so a panicked worker
    /// cannot cascade into panics in every other worker.
    fn lock_count(&self) -> std::sync::MutexGuard<'_, u32> {
        self.count
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn acquire(&self, max: u32) {
        let mut count = self.lock_count();
        while *count >= max {
            // Condvar::wait re-acquires the mutex before returning, so the guard
            // inside PoisonError is already the locked guard; just unwrap it.
            count = self
                .cvar
                .wait(count)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        *count += 1;
    }

    fn release(&self) {
        let mut count = self.lock_count();
        *count = count.saturating_sub(1);
        self.cvar.notify_one();
    }
}

/// I/O 操作的生命周期 guard — drop 时自动 release。
struct IoGuard<'a> {
    permit: &'a IoPermit,
}

impl<'a> IoGuard<'a> {
    fn new(permit: &'a IoPermit, max: u32) -> Self {
        permit.acquire(max);
        IoGuard { permit }
    }
}

impl<'a> Drop for IoGuard<'a> {
    fn drop(&mut self) {
        self.permit.release();
    }
}
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;

pub struct AssetMapService;

struct AtomicCounterGuard<'a> {
    counter: &'a AtomicUsize,
}

impl<'a> AtomicCounterGuard<'a> {
    fn new(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self { counter }
    }
}

impl Drop for AtomicCounterGuard<'_> {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

struct AtomicElapsedGuard<'a> {
    counter: &'a AtomicUsize,
    start: Instant,
}

impl<'a> AtomicElapsedGuard<'a> {
    fn new(counter: &'a AtomicUsize) -> Self {
        Self {
            counter,
            start: Instant::now(),
        }
    }
}

impl Drop for AtomicElapsedGuard<'_> {
    fn drop(&mut self) {
        self.counter
            .fetch_add(self.start.elapsed().as_millis() as usize, Ordering::Relaxed);
    }
}

enum AssetMapBuildOutput {
    Parsed {
        rows: Arc<MapBundleWriteRows>,
        cache_after_parse: bool,
    },
}

impl AssetMapService {
    const MAP_WRITE_BATCH_BUNDLES: usize = 1024;
    const MAP_WRITE_BATCH_ASSETS: usize = 2_000_000;
    const MAP_WRITE_BATCH_RELATIONS: usize = 1_000_000;
    const BUILD_MAP_INCLUDE_RELATIONS: bool = false;

    /// Check whether the Map exists
    #[allow(dead_code)]
    pub fn exists(workspace: &Path) -> bool {
        Self::exists_with_cache_root(workspace, None)
    }

    pub fn exists_with_cache_root(workspace: &Path, cache_root: Option<&Path>) -> bool {
        AssetMapQueryService::exists_with_cache_root(workspace, cache_root)
    }

    fn emit_index_save_progress(
        progress: &Channel<ProgressPayload>,
        task_id: &str,
        current: usize,
        total: usize,
        written_assets: usize,
        pending_assets: usize,
    ) {
        let total = total.max(1);
        let current = current.min(total);
        let message = format!(
            "Index save progress: {}/{} bundles queued ({} committed assets, {} pending assets).",
            current, total, written_assets, pending_assets
        );
        TaskLogger::progress(task_id, "Build Map", "Save Index", current, total, &message);
        let _ = progress.send(ProgressPayload {
            step: "write".into(),
            message,
        });
    }

    fn build_map_bundle_write_rows(
        file_entry: BundleFileEntry,
        bundle_entry: BundleEntry,
        exact_bundle_path_by_normalized_path: &HashMap<String, String>,
        bundle_path_by_file_name: &HashMap<String, String>,
        normalized_bundle_paths: &[(String, String)],
    ) -> MapBundleWriteRows {
        let bundle_path = file_entry.path;
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
            .into_iter()
            .map(|(asset_path, path_id)| ContainerWriteRow {
                bundle_path: bundle_path.clone(),
                asset_path,
                path_id,
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

        // Resolve external file_id targets once per bundle in parser workers.
        let target_bundle_path_by_file_id: HashMap<i32, String> = bundle_entry
            .externals
            .iter()
            .filter_map(|(_, file_id, path_name)| {
                BundlePathMatcher::match_assetstudio_file_name(path_name, bundle_path_by_file_name)
                    .or_else(|| {
                        BundlePathMatcher::match_path(
                            path_name,
                            exact_bundle_path_by_normalized_path,
                            bundle_path_by_file_name,
                            normalized_bundle_paths,
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
                    exact_bundle_path_by_normalized_path,
                    bundle_path_by_file_name,
                    normalized_bundle_paths,
                );
                RelationWriteRow {
                    bundle_path: bundle_path.clone(),
                    relation_type: relation.relation_type.into_owned(),
                    source_path_id: relation.source_path_id,
                    target_path_id: relation.target_path_id,
                    source_name: String::new(),
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

    /// Get Map summary (does not load full content).
    /// Data comes from the file index, returns in milliseconds.
    #[allow(dead_code)]
    pub fn summary(workspace: &Path) -> Option<MapSummary> {
        Self::summary_with_cache_root(workspace, None)
    }

    pub fn summary_with_cache_root(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Option<MapSummary> {
        AssetMapQueryService::summary_with_cache_root(workspace, cache_root)
    }

    /// Query Bundle paths that contain any of the given class names from AssetMap.
    #[allow(dead_code)]
    pub fn bundle_paths_for_classes(
        workspace: &Path,
        class_names: &[String],
    ) -> Result<HashMap<String, Vec<String>>, String> {
        Self::bundle_paths_for_classes_with_cache_root(workspace, class_names, None)
    }

    pub fn bundle_paths_for_classes_with_cache_root(
        workspace: &Path,
        class_names: &[String],
        cache_root: Option<&Path>,
    ) -> Result<HashMap<String, Vec<String>>, String> {
        AssetMapQueryService::bundle_paths_for_classes_with_cache_root(
            workspace,
            class_names,
            cache_root,
        )
    }

    /// Query one page of all AssetMap assets for the All Assets tab.
    #[allow(dead_code)]
    pub fn query_assets(
        workspace: &Path,
        options: &MapAssetQueryOptions,
    ) -> Result<MapAssetQueryResult, String> {
        Self::query_assets_with_cache_root(workspace, options, None)
    }

    pub fn query_assets_with_cache_root(
        workspace: &Path,
        options: &MapAssetQueryOptions,
        cache_root: Option<&Path>,
    ) -> Result<MapAssetQueryResult, String> {
        AssetMapQueryService::query_assets_with_cache_root(workspace, options, cache_root)
    }

    /// Query all AssetMap class statistics for the All Assets type filter.
    #[allow(dead_code)]
    pub fn all_asset_class_stats(workspace: &Path) -> Result<Vec<MapAssetClassStat>, String> {
        Self::all_asset_class_stats_with_cache_root(workspace, None)
    }

    pub fn all_asset_class_stats_with_cache_root(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<Vec<MapAssetClassStat>, String> {
        AssetMapQueryService::all_asset_class_stats_with_cache_root(workspace, cache_root)
    }

    /// Query class statistics for one Bundle in the AssetMap.
    #[allow(dead_code)]
    pub fn bundle_asset_class_stats_with_cache_root(
        workspace: &Path,
        bundle_path: &Path,
        cache_root: Option<&Path>,
    ) -> Result<Vec<MapAssetClassStat>, String> {
        AssetMapQueryService::bundle_asset_class_stats_with_cache_root(
            workspace,
            bundle_path,
            cache_root,
        )
    }

    /// Load the Bundle file list from the file index after Map build.
    #[allow(dead_code)]
    pub fn cached_bundle_files(workspace: &Path) -> Result<Vec<BundleFileEntry>, String> {
        Self::cached_bundle_files_with_cache_root(workspace, None)
    }

    #[allow(dead_code)]
    pub fn cached_bundle_files_with_cache_root(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<Vec<BundleFileEntry>, String> {
        AssetMapQueryService::cached_bundle_files_with_cache_root(workspace, cache_root)
    }

    /// Clear the AssetMap under the current workspace.
    #[allow(dead_code)]
    pub fn clear(workspace: &Path) -> Result<(), String> {
        Self::clear_with_cache_root(workspace, None)
    }

    pub fn clear_with_cache_root(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<(), String> {
        AssetMapQueryService::clear_with_cache_root(workspace, cache_root)
    }

    // ============================================================
    // Build Map (core) parallel version using all CPU cores
    // ============================================================

    /// Build AssetMap with an explicit task id for backend progress logging.
    pub fn build_with_task_id(
        workspace: &Path,
        progress: &Channel<ProgressPayload>,
        cancel_token: &Arc<AtomicBool>,
        task_id: &str,
        memory_limit_bytes: u64,
        cache_root: Option<&Path>,
        skip_ress_list_build: bool,
    ) -> Result<MapSummary, String> {
        let start = Instant::now();

        // Step 1: Collect all candidate files before building the file index.
        let _ = progress.send(ProgressPayload {
            step: "collect".into(),
            message: "Scanning workspace directory...".into(),
        });
        let mut bundles: Vec<BundleFileEntry> = Vec::new();
        let skipped_resource_payload_count = AtomicUsize::new(0);
        BundleFileCollector::collect_bundle_files_filtered(workspace, &mut bundles, |path| {
            let is_resource_payload = AssetMapBuildSupport::is_resource_payload_path(path);
            if is_resource_payload {
                skipped_resource_payload_count.fetch_add(1, Ordering::Relaxed);
            }
            !is_resource_payload
        })?;
        bundles.sort_by(|a, b| b.size.cmp(&a.size));
        AssetMapBuildSupport::interleave_large_and_small_bundles(&mut bundles);
        let total = bundles.len();
        let skipped_resource_payload_count = skipped_resource_payload_count.load(Ordering::Relaxed);
        let _ = progress.send(ProgressPayload {
            step: "collect".into(),
            message: format!(
                "Found {} candidate files (skipped {} resource payload files)",
                total, skipped_resource_payload_count
            ),
        });
        TaskLogger::info(
            task_id,
            "Build Map",
            &format!(
                "Found {} candidate files, skipped {} loose resource payload files (.resS/.resource/.res)",
                total, skipped_resource_payload_count
            ),
        );

        if total == 0 {
            return Err("No files found in workspace".to_string());
        }

        if skip_ress_list_build {
            let message = "Skipping ResSList cache preparation by user option.";
            TaskLogger::info(task_id, "Build Map", message);
            let _ = progress.send(ProgressPayload {
                step: "ress".into(),
                message: message.into(),
            });
        } else {
            let ress_start = Instant::now();
            let ress_summary =
                ResSListCache::prepare(workspace, &bundles, progress, cancel_token, task_id)?;
            TaskLogger::info(
                task_id,
                "Build Map",
                &format!(
                    "ResSList prepared in {:.2}s: {} bundles checked, {} extracted, {} reused, {} failed bundles, {} MB written.",
                    ress_start.elapsed().as_secs_f64(),
                    ress_summary.total_bundles,
                    ress_summary.extracted_files,
                    ress_summary.reused_files,
                    ress_summary.failed_bundles,
                    ress_summary.total_bytes_written / 1024 / 1024
                ),
            );
        }

        let bundle_paths: Vec<String> = bundles.iter().map(|bundle| bundle.path.clone()).collect();
        let path_match_cache = BundlePathMatcher::build_cache(&bundle_paths);
        let exact_bundle_path_by_normalized_path =
            path_match_cache.exact_bundle_path_by_normalized_path;
        let bundle_path_by_file_name = path_match_cache.bundle_path_by_file_name;
        let normalized_bundle_paths = path_match_cache.normalized_bundle_paths;

        let mut index_session = AssetDatabase::create_build_session(workspace, cache_root)?;
        let parse_cache = Arc::new(AssetMapParseCacheStore::open(workspace, cache_root)?);

        // Step 2: Parse all bundles in parallel (std::thread::scope dynamic work-stealing)
        let cpu_count = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let worker_count = AssetMapBuildSupport::parse_worker_count(cpu_count, total);
        let parse_memory_limit_bytes =
            AssetMapBuildSupport::normalize_memory_limit(memory_limit_bytes);
        let result_channel_capacity = AssetMapBuildSupport::result_channel_capacity(worker_count);
        let io_max_concurrent = AssetMapBuildSupport::io_worker_count(cpu_count, total);

        let _ = progress.send(ProgressPayload {
            step: "parse".into(),
            message: format!(
                "Parsing in parallel ({} bundles, {} threads, {} I/O slots, memory budget {:.1} GB, result queue {})...",
                total,
                worker_count,
                io_max_concurrent,
                parse_memory_limit_bytes as f64 / 1_073_741_824.0,
                result_channel_capacity
            ),
        });
        TaskLogger::info(
            task_id,
            "Build Map",
            &format!(
                "Parse scheduler: cpu={}, workers={}, io_slots={}, memory_budget={:.1} GB, result_queue={}",
                cpu_count,
                worker_count,
                io_max_concurrent,
                parse_memory_limit_bytes as f64 / 1_073_741_824.0,
                result_channel_capacity
            ),
        );

        let parsed_count = AtomicUsize::new(0);
        let cached_count = AtomicUsize::new(0);
        let skipped_count = AtomicUsize::new(0);
        let total_assets = AtomicUsize::new(0);
        let started_count = AtomicUsize::new(0);
        let next_index = AtomicUsize::new(0);
        let active_worker_count = AtomicUsize::new(0);
        let active_parsing_count = AtomicUsize::new(0);
        let active_worker_bytes = AtomicUsize::new(0);
        let queued_write_bundle_count = AtomicUsize::new(0);
        let queued_write_asset_count = AtomicUsize::new(0);
        let queued_write_relation_count = AtomicUsize::new(0);
        let fast_scan_count = AtomicUsize::new(0);
        let fallback_scan_count = AtomicUsize::new(0);
        let fallback_fail_count = AtomicUsize::new(0);
        let total_header_probe_ms = AtomicUsize::new(0);
        let total_memory_wait_ms = AtomicUsize::new(0);
        let total_read_ms = AtomicUsize::new(0);
        let total_parse_ms = AtomicUsize::new(0);
        let total_row_build_ms = AtomicUsize::new(0);
        let total_result_send_wait_ms = AtomicUsize::new(0);
        let total_flush_ms = AtomicUsize::new(0);
        let total_parse_cache_write_ms = AtomicUsize::new(0);
        let slowest_parse_ms = AtomicUsize::new(0);
        let slowest_parse_path = Mutex::new(String::new());

        let (result_tx, result_rx) =
            mpsc::sync_channel::<AssetMapBuildOutput>(result_channel_capacity);
        let cache_batch_capacity = result_channel_capacity.clamp(2, 16);
        let (parse_cache_tx, parse_cache_rx) =
            mpsc::sync_channel::<Vec<Arc<MapBundleWriteRows>>>(cache_batch_capacity);
        let parse_cache_write_error = Mutex::new(None::<String>);
        let parse_memory_limiter = BuildMapMemoryLimiter::new(parse_memory_limit_bytes as usize);

        let cancel_ref = &**cancel_token;
        let bundles_ref = &bundles;
        let next_index_ref = &next_index;
        let parsed_ref = &parsed_count;
        let cached_ref = &cached_count;
        let skipped_ref = &skipped_count;
        let total_assets_ref = &total_assets;
        let started_ref = &started_count;
        let active_worker_count_ref = &active_worker_count;
        let active_parsing_count_ref = &active_parsing_count;
        let active_worker_bytes_ref = &active_worker_bytes;
        let queued_write_bundle_count_ref = &queued_write_bundle_count;
        let queued_write_asset_count_ref = &queued_write_asset_count;
        let queued_write_relation_count_ref = &queued_write_relation_count;
        let fast_scan_count_ref = &fast_scan_count;
        let fallback_scan_count_ref = &fallback_scan_count;
        let fallback_fail_count_ref = &fallback_fail_count;
        let total_header_probe_ms_ref = &total_header_probe_ms;
        let total_memory_wait_ms_ref = &total_memory_wait_ms;
        let total_read_ms_ref = &total_read_ms;
        let total_parse_ms_ref = &total_parse_ms;
        let total_row_build_ms_ref = &total_row_build_ms;
        let total_result_send_wait_ms_ref = &total_result_send_wait_ms;
        let slowest_parse_ms_ref = &slowest_parse_ms;
        let slowest_parse_path_ref = &slowest_parse_path;
        let parse_memory_limiter_ref = &parse_memory_limiter;
        let parse_cache_ref = &parse_cache;
        let parse_cache_write_error_ref = &parse_cache_write_error;
        let exact_bundle_path_by_normalized_path_ref = &exact_bundle_path_by_normalized_path;
        let bundle_path_by_file_name_ref = &bundle_path_by_file_name;
        let normalized_bundle_paths_ref = &normalized_bundle_paths;

        // I/O 并发限制：限制同时读盘的线程数，避免磁盘头抖动
        let io_permit = IoPermit::new();
        let io_permit_ref = &io_permit;

        let mut changed_count: usize = 0;
        let mut written_asset_count: usize = 0;
        let mut write_error: Option<String> = None;

        std::thread::scope(|_scope| {
            let cache_writer_store = Arc::clone(&parse_cache);
            let cache_writer_error = parse_cache_write_error_ref;
            let cache_writer_total_ms = &total_parse_cache_write_ms;
            _scope.spawn(move || {
                // This scoped thread is never joined by hand, so a panic here
                // would be re-raised by thread::scope and kill the whole build.
                // Contain it and surface the failure as a normal write error.
                let cache_writer_result = std::panic::catch_unwind(
                    std::panic::AssertUnwindSafe(|| {
                        while let Ok(cache_rows) = parse_cache_rx.recv() {
                            let cache_write_start = Instant::now();
                            let cache_row_refs = cache_rows
                                .iter()
                                .map(|rows| rows.as_ref())
                                .collect::<Vec<_>>();
                            if let Err(e) = cache_writer_store
                                .put_many_refs(&cache_row_refs, Self::BUILD_MAP_INCLUDE_RELATIONS)
                            {
                                if let Ok(mut error) = cache_writer_error.lock() {
                                    *error = Some(e);
                                }
                                break;
                            }
                            cache_writer_total_ms.fetch_add(
                                cache_write_start.elapsed().as_millis() as usize,
                                Ordering::Relaxed,
                            );
                        }
                    }),
                );
                if let Err(payload) = cache_writer_result {
                    if let Ok(mut error) = cache_writer_error.lock() {
                        *error = Some(format!(
                            "parse cache writer panicked: {}",
                            crate::common::panic_reporter::PanicReporter::payload_to_string(
                                payload.as_ref()
                            )
                        ));
                    }
                }
            });

            for _worker_id in 0..worker_count {
                let progress_own = progress.clone();
                let result_tx_clone = result_tx.clone();
                _scope.spawn(move || {
                    loop {
                        if cancel_ref.load(Ordering::Relaxed) {
                            return;
                        }

                        let idx = next_index_ref.fetch_add(1, Ordering::Relaxed);
                        if idx >= total {
                            return;
                        }

                        let entry = &bundles_ref[idx];
                        let entry_path = entry.path.clone();
                        let entry_name = entry.name.clone();
                        let entry_name_for_err = entry_name.clone();
                        let entry_size = entry.size;

                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            let _active_worker_guard =
                                AtomicCounterGuard::new(active_worker_count_ref);
                            let bp = Path::new(&entry_path);
                            if cancel_ref.load(Ordering::Relaxed) {
                                return;
                            }

                            let d = started_ref.fetch_add(1, Ordering::Relaxed) + 1;

                            // I/O 并发限制：同一时刻最多 IO_MAX_CONCURRENT 个线程读盘
                            let header_probe_start = Instant::now();
                            let looks_like_unityfs = {
                                let _io_guard = IoGuard::new(io_permit_ref, io_max_concurrent);
                                match AssetMapBuildSupport::looks_like_unityfs_file(&entry_path) {
                                    Ok(value) => value,
                                    Err(e) => {
                                        total_header_probe_ms_ref.fetch_add(
                                            header_probe_start.elapsed().as_millis() as usize,
                                            Ordering::Relaxed,
                                        );
                                        skipped_ref.fetch_add(1, Ordering::Relaxed);
                                        let _ = progress_own.send(ProgressPayload {
                                            step: "parse".into(),
                                            message: format!("{} skipped ({})", entry_name, e),
                                        });
                                        return;
                                    }
                                }
                            };
                            let looks_like_serialized_file = if looks_like_unityfs {
                                false
                            } else {
                                let _io_guard = IoGuard::new(io_permit_ref, io_max_concurrent);
                                match AssetMapBuildSupport::looks_like_serialized_file(
                                    &entry_path,
                                    entry_size,
                                ) {
                                    Ok(value) => value,
                                    Err(e) => {
                                        total_header_probe_ms_ref.fetch_add(
                                            header_probe_start.elapsed().as_millis() as usize,
                                            Ordering::Relaxed,
                                        );
                                        skipped_ref.fetch_add(1, Ordering::Relaxed);
                                        let _ = progress_own.send(ProgressPayload {
                                            step: "parse".into(),
                                            message: format!("{} skipped ({})", entry_name, e),
                                        });
                                        return;
                                    }
                                }
                            };
                            total_header_probe_ms_ref.fetch_add(
                                header_probe_start.elapsed().as_millis() as usize,
                                Ordering::Relaxed,
                            );

                            if !looks_like_unityfs && !looks_like_serialized_file {
                                skipped_ref.fetch_add(1, Ordering::Relaxed);
                                return;
                            }

                            let content_fingerprint = AssetMapBuildSupport::content_fingerprint(
                                entry_size,
                                entry.modified_ms,
                                entry_size as usize,
                            );

                            if let Ok(Some(cached_rows)) = parse_cache_ref.get_valid(
                                &entry_path,
                                entry_size,
                                entry.modified_ms,
                                &content_fingerprint,
                                Self::BUILD_MAP_INCLUDE_RELATIONS,
                            ) {
                                cached_ref.fetch_add(1, Ordering::Relaxed);
                                total_assets_ref
                                    .fetch_add(cached_rows.assets.len(), Ordering::Relaxed);
                                queued_write_bundle_count_ref.fetch_add(1, Ordering::Relaxed);
                                queued_write_asset_count_ref
                                    .fetch_add(cached_rows.assets.len(), Ordering::Relaxed);
                                queued_write_relation_count_ref
                                    .fetch_add(cached_rows.relations.len(), Ordering::Relaxed);
                                let _send_wait_guard =
                                    AtomicElapsedGuard::new(total_result_send_wait_ms_ref);
                                let _ = result_tx_clone.send(AssetMapBuildOutput::Parsed {
                                    rows: Arc::new(cached_rows),
                                    cache_after_parse: false,
                                });
                                return;
                            }

                            let memory_budget_bytes =
                                AssetMapBuildSupport::estimate_parse_memory_budget(entry_size);
                            let memory_wait_start = Instant::now();
                            let memory_permit = match parse_memory_limiter_ref
                                .acquire(memory_budget_bytes, active_worker_bytes_ref)
                            {
                                Ok(permit) => permit,
                                Err(e) => {
                                    total_memory_wait_ms_ref.fetch_add(
                                        memory_wait_start.elapsed().as_millis() as usize,
                                        Ordering::Relaxed,
                                    );
                                    skipped_ref.fetch_add(1, Ordering::Relaxed);
                                    let _ = progress_own.send(ProgressPayload {
                                        step: "parse".into(),
                                        message: format!("{} skipped ({})", entry_name, e),
                                    });
                                    return;
                                }
                            };
                            total_memory_wait_ms_ref.fetch_add(
                                memory_wait_start.elapsed().as_millis() as usize,
                                Ordering::Relaxed,
                            );

                            if cancel_ref.load(Ordering::Relaxed) {
                                return;
                            }

                            if d == 1 || d % 25 == 0 || d == total {
                                TaskLogger::progress(
                                    task_id,
                                    "Build Map",
                                    "Parse Bundle",
                                    d,
                                    total,
                                    &format!("Parsing: {} ({} KB)", entry_name, entry_size / 1024),
                                );
                                let _ = progress_own.send(ProgressPayload {
                                    step: "parse".into(),
                                    message: format!("{} parsing...", entry_name),
                                });
                            }

                            let file_size = entry_size;

                            if cancel_ref.load(Ordering::Relaxed) {
                                return;
                            }

                            let active_parsing_guard =
                                AtomicCounterGuard::new(active_parsing_count_ref);
                            let parse_start = Instant::now();
                            let scan_result = match if looks_like_serialized_file {
                                MapBundleScanner::scan_serialized_file_from_path_with_relations(
                                    bp,
                                    file_size,
                                    content_fingerprint.clone(),
                                    Self::BUILD_MAP_INCLUDE_RELATIONS,
                                )
                            } else {
                                MapBundleScanner::scan_from_path_with_relations(
                                    bp,
                                    file_size,
                                    content_fingerprint.clone(),
                                    Self::BUILD_MAP_INCLUDE_RELATIONS,
                                )
                            } {
                                Ok(data) => {
                                    fast_scan_count_ref.fetch_add(1, Ordering::Relaxed);
                                    Ok(data)
                                }
                                Err(fast_error) => {
                                    if Self::should_skip_map_fallback(&fast_error) {
                                        fallback_fail_count_ref.fetch_add(1, Ordering::Relaxed);
                                        Err(format!(
                                            "{}; fallback skipped: fast scan already failed while decompressing a bundle block, so reading the full file and retrying would repeat the same decompression failure.",
                                            fast_error
                                        ))
                                    } else {
                                        let read_start = Instant::now();
                                        let fallback_read_result = {
                                            let _io_guard =
                                                IoGuard::new(io_permit_ref, io_max_concurrent);
                                            fs::read(&entry_path)
                                        };
                                        match fallback_read_result {
                                            Ok(file_data) => {
                                                total_read_ms_ref.fetch_add(
                                                    read_start.elapsed().as_millis() as usize,
                                                    Ordering::Relaxed,
                                                );
                                                let fallback_fingerprint =
                                                    content_fingerprint.clone();
                                                let fallback_result = if looks_like_serialized_file
                                                {
                                                    MapBundleScanner::scan_serialized_file_from_bytes_with_relations(
                                                        bp,
                                                        file_data,
                                                        file_size,
                                                        fallback_fingerprint,
                                                        Self::BUILD_MAP_INCLUDE_RELATIONS,
                                                    )
                                                } else {
                                                    AssetBundleLoader::load_bundle_serialized_only_from_bytes(
                                                        &file_data,
                                                    )
                                                    .map(|bundle| {
                                                        Self::extract_bundle_entry_with_relations(
                                                            bp,
                                                            &bundle,
                                                            file_size,
                                                            fallback_fingerprint,
                                                            Self::BUILD_MAP_INCLUDE_RELATIONS,
                                                        )
                                                    })
                                                };
                                                match fallback_result {
                                                    Ok(data) => {
                                                        fallback_scan_count_ref
                                                            .fetch_add(1, Ordering::Relaxed);
                                                        Ok(data)
                                                    }
                                                    Err(fallback_error) => {
                                                        fallback_fail_count_ref
                                                            .fetch_add(1, Ordering::Relaxed);
                                                        Err(format!(
                                                            "{}; fallback failed: {}",
                                                            fast_error, fallback_error
                                                        ))
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                fallback_fail_count_ref
                                                    .fetch_add(1, Ordering::Relaxed);
                                                Err(format!(
                                                    "{}; fallback read failed: Failed to read file '{}': {}",
                                                    fast_error, entry_path, e
                                                ))
                                            }
                                        }
                                    }
                                }
                            };
                            let parse_ms = parse_start.elapsed().as_millis() as usize;
                            drop(active_parsing_guard);
                            total_parse_ms_ref.fetch_add(parse_ms, Ordering::Relaxed);
                            let previous_slowest = slowest_parse_ms_ref.load(Ordering::Relaxed);
                            if parse_ms > previous_slowest
                                && slowest_parse_ms_ref
                                    .compare_exchange(
                                        previous_slowest,
                                        parse_ms,
                                        Ordering::Relaxed,
                                        Ordering::Relaxed,
                                    )
                                    .is_ok()
                            {
                                if let Ok(mut slowest_path) = slowest_parse_path_ref.lock() {
                                    *slowest_path = entry_path.clone();
                                }
                            }

                            match scan_result {
                                Ok(data) => {
                                    parsed_ref.fetch_add(1, Ordering::Relaxed);
                                    total_assets_ref
                                        .fetch_add(data.assets.len(), Ordering::Relaxed);
                                    let row_build_start = Instant::now();
                                    let write_rows = Self::build_map_bundle_write_rows(
                                        BundleFileEntry {
                                            name: entry_name,
                                            path: entry_path.clone(),
                                            size: entry_size,
                                            modified_ms: entry.modified_ms,
                                            modified: String::new(),
                                        },
                                        data,
                                        exact_bundle_path_by_normalized_path_ref,
                                        bundle_path_by_file_name_ref,
                                        normalized_bundle_paths_ref,
                                    );
                                    total_row_build_ms_ref.fetch_add(
                                        row_build_start.elapsed().as_millis() as usize,
                                        Ordering::Relaxed,
                                    );
                                    drop(memory_permit);
                                    queued_write_bundle_count_ref.fetch_add(1, Ordering::Relaxed);
                                    queued_write_asset_count_ref
                                        .fetch_add(write_rows.assets.len(), Ordering::Relaxed);
                                    queued_write_relation_count_ref
                                        .fetch_add(write_rows.relations.len(), Ordering::Relaxed);
                                    let _send_wait_guard =
                                        AtomicElapsedGuard::new(total_result_send_wait_ms_ref);
                                    let _ = result_tx_clone.send(AssetMapBuildOutput::Parsed {
                                        rows: Arc::new(write_rows),
                                        cache_after_parse: true,
                                    });
                                }
                                Err(e) => {
                                    skipped_ref.fetch_add(1, Ordering::Relaxed);
                                    let error_message =
                                        Self::describe_map_parse_failure(&entry_path, &e);
                                    let _ = progress_own.send(ProgressPayload {
                                        step: "parse".into(),
                                        message: format!(
                                            "{} skipped ({})",
                                            entry_name, error_message
                                        ),
                                    });
                                }
                            }
                        }));

                        if let Err(payload) = result {
                            skipped_ref.fetch_add(1, Ordering::Relaxed);
                            let panic_message =
                                crate::common::panic_reporter::PanicReporter::payload_to_string(
                                    payload.as_ref(),
                                );
                            let panic_snippet: String =
                                panic_message.chars().take(160).collect();
                            let panic_log_hint =
                                crate::common::panic_reporter::PanicReporter::session_log_path()
                                    .map(|path| {
                                        format!(" (details: {})", path.to_string_lossy())
                                    })
                                    .unwrap_or_default();
                            TaskLogger::progress(
                                task_id,
                                "Build Map",
                                "Parse Bundle",
                                started_ref.load(Ordering::Relaxed),
                                total,
                                &format!(
                                    "Panic skip: {} ({}){}",
                                    entry_name_for_err, panic_snippet, panic_log_hint
                                ),
                            );
                            let _ = progress_own.send(ProgressPayload {
                                step: "parse".into(),
                                message: format!(
                                    "{} parse panic ({}), skipped",
                                    entry_name_for_err, panic_snippet
                                ),
                            });
                        }
                    }
                });
            }

            drop(result_tx);

            let _ = progress.send(ProgressPayload {
                step: "write".into(),
                message: "Writing parsed bundles to file index as they finish...".into(),
            });
            Self::emit_index_save_progress(progress, task_id, 0, total, written_asset_count, 0);

            let mut pending: Vec<AssetMapBuildOutput> = Vec::new();
            let mut pending_assets: usize = 0;
            let mut pending_relations: usize = 0;
            let mut last_write_log_count: usize = 0;
            let mut last_save_progress_emit = Instant::now() - Duration::from_secs(5);
            let mut last_heartbeat = Instant::now();
            let mut last_diagnostic_started: usize = 0;
            let mut last_diagnostic_saved: usize = 0;
            let mut last_diagnostic_time = Instant::now();
            let mut last_flush_seconds = 0.0_f64;
            let mut last_flush_bundles = 0_usize;
            let mut last_flush_assets = 0_usize;
            let mut last_flush_relations = 0_usize;
            let mut flush_pending = |pending: &mut Vec<AssetMapBuildOutput>,
                                     pending_assets: &mut usize,
                                     pending_relations: &mut usize|
             -> Result<(usize, usize, usize, f64), String> {
                if pending.is_empty() {
                    return Ok((0, 0, 0, 0.0));
                }
                let batch_count = pending.len();
                let mut batch_assets = 0usize;
                let batch_relations = *pending_relations;
                let flush_start = Instant::now();
                let drained = std::mem::take(pending);
                let cache_rows: Vec<Arc<MapBundleWriteRows>> = drained
                    .iter()
                    .filter_map(|item| match item {
                        AssetMapBuildOutput::Parsed {
                            rows,
                            cache_after_parse: true,
                        } => Some(Arc::clone(rows)),
                        _ => None,
                    })
                    .collect();
                let cache_write_start = Instant::now();
                if !cache_rows.is_empty() {
                    parse_cache_tx
                        .send(cache_rows)
                        .map_err(|e| format!("queue parse cache write: {}", e))?;
                }
                total_parse_cache_write_ms.fetch_add(
                    cache_write_start.elapsed().as_millis() as usize,
                    Ordering::Relaxed,
                );
                for item in drained {
                    batch_assets += match item {
                        AssetMapBuildOutput::Parsed {
                            rows,
                            cache_after_parse: _,
                        } => index_session.append_bundle(rows)?,
                    };
                }
                let flush_elapsed = flush_start.elapsed();
                let flush_seconds = flush_elapsed.as_secs_f64();
                total_flush_ms.fetch_add(flush_elapsed.as_millis() as usize, Ordering::Relaxed);
                *pending_assets = 0;
                *pending_relations = 0;
                Ok((batch_count, batch_assets, batch_relations, flush_seconds))
            };

            loop {
                let build_output = match result_rx.recv_timeout(Duration::from_secs(5)) {
                    Ok(result) => result,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // 超时：如果已取消且有未刷数据，立即刷盘
                        if cancel_token.load(Ordering::Relaxed) && !pending.is_empty() {
                            if let Ok((bundles, assets, _, _)) = flush_pending(
                                &mut pending,
                                &mut pending_assets,
                                &mut pending_relations,
                            ) {
                                changed_count += bundles;
                                written_asset_count += assets;
                            }
                        }
                        if last_heartbeat.elapsed() >= Duration::from_secs(5) {
                            let _ = progress.send(ProgressPayload {
                                step: "parse".into(),
                                message: format!(
                                    "Build Map running: started {}/{} bundles, parsed {}, cached {}, skipped {}, pending write {} bundles ({} assets, {} relations), saved {} bundles.",
                                    started_count.load(Ordering::Relaxed),
                                    total,
                                    parsed_count.load(Ordering::Relaxed),
                                    cached_count.load(Ordering::Relaxed),
                                    skipped_count.load(Ordering::Relaxed),
                                    pending.len(),
                                    pending_assets,
                                    pending_relations,
                                    changed_count
                                ),
                            });
                            last_heartbeat = Instant::now();
                        }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };

                queued_write_bundle_count.fetch_sub(1, Ordering::Relaxed);
                let (asset_len, relation_len) = match &build_output {
                    AssetMapBuildOutput::Parsed { rows, .. } => {
                        (rows.assets.len(), rows.relations.len())
                    }
                };
                queued_write_asset_count.fetch_sub(asset_len, Ordering::Relaxed);
                queued_write_relation_count.fetch_sub(relation_len, Ordering::Relaxed);
                pending_assets += asset_len;
                pending_relations += relation_len;
                pending.push(build_output);
                if last_save_progress_emit.elapsed() >= Duration::from_millis(750) {
                    Self::emit_index_save_progress(
                        progress,
                        task_id,
                        changed_count + pending.len(),
                        total,
                        written_asset_count,
                        pending_assets,
                    );
                    last_save_progress_emit = Instant::now();
                }

                let should_flush = pending.len() >= Self::MAP_WRITE_BATCH_BUNDLES
                    || pending_assets >= Self::MAP_WRITE_BATCH_ASSETS
                    || pending_relations >= Self::MAP_WRITE_BATCH_RELATIONS
                    || cancel_token.load(Ordering::Relaxed);

                if should_flush {
                    match flush_pending(&mut pending, &mut pending_assets, &mut pending_relations) {
                        Ok((bundles, assets, relations, flush_seconds)) => {
                            changed_count += bundles;
                            written_asset_count += assets;
                            last_flush_seconds = flush_seconds;
                            last_flush_bundles = bundles;
                            last_flush_assets = assets;
                            last_flush_relations = relations;
                            if bundles > 0 && changed_count != last_write_log_count {
                                Self::emit_index_save_progress(
                                    progress,
                                    task_id,
                                    changed_count,
                                    total,
                                    written_asset_count,
                                    0,
                                );
                                last_save_progress_emit = Instant::now();
                                last_write_log_count = changed_count;
                            }
                        }
                        Err(e) => {
                            cancel_token.store(true, Ordering::Relaxed);
                            write_error = Some(e);
                            break;
                        }
                    }
                }

                if last_diagnostic_time.elapsed() >= Duration::from_secs(10) {
                    let diagnostic_seconds = last_diagnostic_time.elapsed().as_secs_f64().max(1.0);
                    let current_started = started_count.load(Ordering::Relaxed);
                    let parse_rate = (current_started.saturating_sub(last_diagnostic_started))
                        as f64
                        / diagnostic_seconds;
                    let write_rate = (changed_count.saturating_sub(last_diagnostic_saved)) as f64
                        / diagnostic_seconds;
                    let active_bytes_mb =
                        active_worker_bytes.load(Ordering::Relaxed) as f64 / 1_048_576.0;
                    let queued_assets = queued_write_asset_count.load(Ordering::Relaxed);
                    let queued_relations = queued_write_relation_count.load(Ordering::Relaxed);
                    let parsed_for_avg = parsed_count.load(Ordering::Relaxed).max(1);
                    let started_for_avg = current_started.max(1);
                    let header_probe_ms = total_header_probe_ms.load(Ordering::Relaxed);
                    let memory_wait_ms = total_memory_wait_ms.load(Ordering::Relaxed);
                    let fallback_read_ms = total_read_ms.load(Ordering::Relaxed);
                    let parse_ms_total = total_parse_ms.load(Ordering::Relaxed);
                    let row_build_ms = total_row_build_ms.load(Ordering::Relaxed);
                    let result_send_wait_ms = total_result_send_wait_ms.load(Ordering::Relaxed);
                    let flush_ms = total_flush_ms.load(Ordering::Relaxed);
                    let parse_cache_write_ms = total_parse_cache_write_ms.load(Ordering::Relaxed);
                    let slowest_path = slowest_parse_path
                        .lock()
                        .map(|path| path.clone())
                        .unwrap_or_default();
                    let message = format!(
                        "Build Map diagnostics: active_workers={}/{}, parsing_workers={}, active_input={:.1} MB, started={}/{}, parsed={}, cached={}, skipped={}, fast_scan={}, fallback_scan={}, fallback_fail={}, slowest_parse={} ms '{}', pending_writer={} bundles ({} assets, {} relations), channel_queue={} bundles ({} assets, {} relations), saved={} bundles, parse_rate={:.1} bundles/s, write_rate={:.1} bundles/s, stage_ms_total header_probe={} memory_wait={} fallback_read={} parse={} row_build={} result_send_wait={} parse_cache_write={} index_flush={}, stage_ms_avg header_probe={:.1} memory_wait={:.1} fallback_read={:.1} parse={:.1} row_build={:.1} result_send_wait={:.1}, last_flush={} bundles/{} assets/{} relations in {:.2}s.",
                        active_worker_count.load(Ordering::Relaxed),
                        worker_count,
                        active_parsing_count.load(Ordering::Relaxed),
                        active_bytes_mb,
                        current_started,
                        total,
                        parsed_count.load(Ordering::Relaxed),
                        cached_count.load(Ordering::Relaxed),
                        skipped_count.load(Ordering::Relaxed),
                        fast_scan_count.load(Ordering::Relaxed),
                        fallback_scan_count.load(Ordering::Relaxed),
                        fallback_fail_count.load(Ordering::Relaxed),
                        slowest_parse_ms.load(Ordering::Relaxed),
                        slowest_path,
                        pending.len(),
                        pending_assets,
                        pending_relations,
                        queued_write_bundle_count.load(Ordering::Relaxed),
                        queued_assets,
                        queued_relations,
                        changed_count,
                        parse_rate,
                        write_rate,
                        header_probe_ms,
                        memory_wait_ms,
                        fallback_read_ms,
                        parse_ms_total,
                        row_build_ms,
                        result_send_wait_ms,
                        parse_cache_write_ms,
                        flush_ms,
                        header_probe_ms as f64 / started_for_avg as f64,
                        memory_wait_ms as f64 / started_for_avg as f64,
                        fallback_read_ms as f64 / parsed_for_avg as f64,
                        parse_ms_total as f64 / parsed_for_avg as f64,
                        row_build_ms as f64 / parsed_for_avg as f64,
                        result_send_wait_ms as f64 / parsed_for_avg as f64,
                        last_flush_bundles,
                        last_flush_assets,
                        last_flush_relations,
                        last_flush_seconds
                    );
                    let _ = progress.send(ProgressPayload {
                        step: "diagnostics".into(),
                        message,
                    });
                    last_diagnostic_started = current_started;
                    last_diagnostic_saved = changed_count;
                    last_diagnostic_time = Instant::now();
                }
            }

            if write_error.is_none() {
                match flush_pending(&mut pending, &mut pending_assets, &mut pending_relations) {
                    Ok((bundles, assets, _relations, _flush_seconds)) => {
                        changed_count += bundles;
                        written_asset_count += assets;
                        if bundles > 0 && changed_count != last_write_log_count {
                            Self::emit_index_save_progress(
                                progress,
                                task_id,
                                changed_count,
                                total,
                                written_asset_count,
                                0,
                            );
                        }
                    }
                    Err(e) => {
                        cancel_token.store(true, Ordering::Relaxed);
                        write_error = Some(e);
                    }
                }
            }

            drop(flush_pending);
            drop(parse_cache_tx);
        });

        // Step 3: finalize file index
        if write_error.is_none() {
            if let Ok(mut error) = parse_cache_write_error.lock() {
                if let Some(e) = error.take() {
                    write_error = Some(e);
                }
            }
        }

        if let Some(e) = write_error {
            return Err(e);
        }

        let was_cancelled = cancel_token.load(Ordering::Relaxed);

        // --- 读取各阶段计时（仅在 build 结束时）---
        let header_probe_ms = total_header_probe_ms.load(Ordering::Relaxed);
        let memory_wait_ms = total_memory_wait_ms.load(Ordering::Relaxed);
        let fallback_read_ms = total_read_ms.load(Ordering::Relaxed);
        let parse_ms_total = total_parse_ms.load(Ordering::Relaxed);
        let row_build_ms = total_row_build_ms.load(Ordering::Relaxed);
        let result_send_wait_ms = total_result_send_wait_ms.load(Ordering::Relaxed);
        let flush_ms = total_flush_ms.load(Ordering::Relaxed);
        let parse_cache_write_ms = total_parse_cache_write_ms.load(Ordering::Relaxed);
        let parsed_for_avg = parsed_count.load(Ordering::Relaxed).max(1);
        let timing_report = format!(
            "Timing (total/avg per parsed bundle): header_probe={}/{}ms memory_wait={}/{}ms fallback_read={}/{}ms parse={}/{}ms row_build={}/{}ms result_send_wait={}/{}ms parse_cache_write={}/{}ms index_flush={}/{}ms",
            header_probe_ms, header_probe_ms / parsed_for_avg,
            memory_wait_ms, memory_wait_ms / parsed_for_avg,
            fallback_read_ms, fallback_read_ms / parsed_for_avg,
            parse_ms_total, parse_ms_total / parsed_for_avg,
            row_build_ms, row_build_ms / parsed_for_avg,
            result_send_wait_ms, result_send_wait_ms / parsed_for_avg,
            parse_cache_write_ms, parse_cache_write_ms / parsed_for_avg,
            flush_ms, flush_ms / parsed_for_avg,
        );

        if cancel_token.load(Ordering::Relaxed) {
            TaskLogger::warn(
                task_id,
                "Build Map",
                "Map build cancelled; completed bundles were saved.",
            );
        }

        let cached_count_val = cached_count.load(Ordering::Relaxed);
        let elapsed = start.elapsed().as_secs_f64();
        let parsed = parsed_count.load(Ordering::Relaxed);
        let cached = cached_count_val;
        let skipped = skipped_count.load(Ordering::Relaxed);
        let built_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let final_summary = index_session.finish(
            built_at,
            AssetMapQueryService::format_built_at(built_at),
            parsed + cached,
            was_cancelled,
        )?;
        AssetMapQueryService::invalidate_query_cache();
        let asset_count = final_summary.asset_count;
        let total_bundles = final_summary.bundle_count;

        if was_cancelled {
            TaskLogger::warn(
                task_id,
                "Build Map",
                &format!(
                "Map build stopped: saved {} parsed bundles ({} cache hits/{} skipped), {} assets",
                parsed, cached, skipped, asset_count
            ),
            );
            let _ = progress.send(ProgressPayload {
                step: "done".into(),
                message: format!("Map build stopped: saved {} parsed bundles ({} cache hits/{} skipped), {} assets, elapsed {}. {}",
                    parsed, cached, skipped, asset_count,
                    TimeUtils::format_duration(elapsed), timing_report),
            });
        } else {
            TaskLogger::success(
                task_id,
                "Build Map",
                &format!(
                    "Map build complete: {} bundles ({} parsed/{} cached/{} skipped), {} assets",
                    total_bundles, parsed, cached, skipped, asset_count
                ),
            );
            let _ = progress.send(ProgressPayload {
                step: "done".into(),
                message: format!("Map build complete: {} bundles ({} parsed/{} cached/{} skipped), {} assets, elapsed {}. {}",
                    total_bundles, parsed, cached, skipped, asset_count,
                    TimeUtils::format_duration(elapsed), timing_report),
            });
        }
        Ok(final_summary)
    }

    // ============================================================
    // Fast dependency lookup using the file index (called during export)
    // ============================================================

    /// Look up Materials and Texture2D associated with a Mesh from the file index.
    /// Internally maintains a Bundle cache so each bundle is loaded and decompressed only once.
    pub fn resolve_dependencies(
        db: &AssetDatabase,
        mesh_bundle_path: &str,
        mesh_path_id: i64,
        mesh_asset_name: &str,
        progress: &Channel<ProgressPayload>,
    ) -> Result<(Vec<MaterialInfo>, Vec<(i64, String, String)>), String> {
        MaterialTextureResolver::resolve_dependencies(
            db,
            mesh_bundle_path,
            mesh_path_id,
            mesh_asset_name,
            progress,
        )
    }

    /// Start from a Material, parse it with the AssetStudio layout parser and look up referenced Texture2D.
    ///
    /// Unlike `resolve_dependencies`, this method starts directly from the Material,
    /// skips the Container matching strategy, and reads the Material class layout's
    /// `m_SavedProperties.m_TexEnvs` to get texture PPtr references,
    /// then locates Texture2D assets via the AssetMap index.
    ///
    /// Returns `(MaterialInfo, Vec<(path_id, bundle_path, asset_name)>)`.
    pub fn resolve_material_textures(
        db: &AssetDatabase,
        material_bundle_path: &str,
        material_path_id: i64,
        progress: &Channel<ProgressPayload>,
    ) -> Result<(MaterialInfo, Vec<(i64, String, String)>), String> {
        MaterialTextureResolver::resolve_material_textures(
            db,
            material_bundle_path,
            material_path_id,
            progress,
        )
    }

    // ============================================================
    // Cache loading (fast path for parse_bundle_metadata)
    // ============================================================

    pub fn try_load_cached_meta_with_workspace(
        bundle_path: &Path,
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Option<crate::common::bundle_file::bundle_types::BundleMeta> {
        use crate::common::bundle_file::bundle_types::{AssetSummary, BundleMeta};

        let db = AssetDatabase::open_with_cache_root(workspace, cache_root).ok()?;
        let bundle_path_str = bundle_path.to_string_lossy().to_string();
        let assets = db.find_by_bundle(&bundle_path_str).ok()?;
        if assets.is_empty() {
            return None;
        }
        let container_path_by_id = db
            .get_containers(&bundle_path_str)
            .ok()?
            .into_iter()
            .map(|row| (row.path_id, row.asset_path))
            .collect::<HashMap<_, _>>();
        let unity_version = db
            .get_bundle_infos()
            .ok()?
            .into_iter()
            .find(|row| row.path == bundle_path_str)
            .map(|row| row.unity_version)
            .unwrap_or_default();

        let file_size = std::fs::metadata(bundle_path).map(|m| m.len()).unwrap_or(0);
        let summaries: Vec<AssetSummary> = assets
            .iter()
            .map(|a| AssetSummary {
                path: container_path_by_id
                    .get(&a.path_id)
                    .cloned()
                    .unwrap_or_default(),
                name: a.asset_name.clone(),
                class_name: a.class_name.clone(),
                class_id: a.class_id,
                path_id: a.path_id,
                byte_size: a.byte_size,
            })
            .collect();

        Some(BundleMeta {
            path: bundle_path.to_string_lossy().to_string(),
            size: file_size,
            compressed: false,
            unity_version,
            nodes: Vec::new(),
            assets: summaries,
            files_diag: Vec::new(),
            parse_logs: vec!["(Loaded from sqlite cache)".to_string()],
        })
    }

    // ============================================================
    // Internal: Bundle Parsing
    // ============================================================

    /// Parse a single Bundle: load file (heavy I/O + decompression)
    #[allow(dead_code)]
    fn load_bundle_file(
        bundle_path: &Path,
    ) -> Result<(crate::common::bundle_file::asset_bundle::AssetBundle, u64), String> {
        let file_size = fs::metadata(bundle_path).map(|m| m.len()).unwrap_or(0);
        let bundle = AssetBundleLoader::load_bundle(bundle_path)
            .map_err(|e| format!("Load failed: {}", e))?;
        Ok((bundle, file_size))
    }

    /// Extract the asset manifest from a Bundle (lightweight in-memory traversal).
    fn extract_bundle_entry_with_relations(
        bundle_path: &Path,
        bundle: &crate::common::bundle_file::asset_bundle::AssetBundle,
        file_size: u64,
        md5: String,
        include_relations: bool,
    ) -> BundleEntry {
        let unity_version = bundle
            .assets
            .first()
            .map(|sf| sf.unity_version.clone())
            .unwrap_or_default();

        let mut assets: Vec<AssetEntry> = Vec::new();
        let mut containers: HashMap<String, i64> = HashMap::new();
        let mut externals: Vec<(usize, i32, String)> = Vec::new();
        let mut internal_names: Vec<(String, String)> = Vec::new();
        let mut relations: Vec<BundleRelationEntry> = Vec::new();

        for node in &bundle.nodes {
            Self::push_internal_name(&mut internal_names, &node.name, "node");
        }

        for name in bundle.asset_names.iter().flatten() {
            Self::push_internal_name(&mut internal_names, name, "serialized_file");
        }

        for (sf_idx, sf) in bundle.assets.iter().enumerate() {
            Self::push_internal_name(&mut internal_names, &sf.name, "file_name");

            // Extract AssetBundle container info (class_id=142)
            if let Some(ab_obj) = sf.objects.iter().find(|o| o.class_id == 142) {
                if let Ok(entries) = sf.assetbundle_container_raw(ab_obj) {
                    for (asset_path, _file_id, path_id) in &entries {
                        containers.insert(asset_path.clone(), *path_id);
                    }
                }
            }
            // Extract m_externals external reference info
            for (ext_idx, ext) in sf.inner.m_externals.iter().enumerate() {
                if !ext.path_name.is_empty() {
                    let target_name = Self::external_match_name(&ext.path_name, &ext.file_name);
                    externals.push((sf_idx, ext_idx as i32 + 1, target_name.clone()));
                    // file_id in PPtr is 1-based: file_id=1 m_externals[0]
                }
            }
            let class_by_path: HashMap<i64, u16> = if include_relations {
                sf.objects
                    .iter()
                    .map(|object_info| (object_info.path_id, object_info.class_id as u16))
                    .collect()
            } else {
                HashMap::new()
            };
            for obj in &sf.objects {
                let class_name: Cow<'static, str> =
                    match AssetBundleLoader::get_class_name(obj.class_id) {
                        Some(name) => name.into(),
                        None => format!("Unknown({})", obj.class_id).into(),
                    };
                let obj_name = UnityObjectNameUtils::display_name(&sf.inner, &obj.inner)
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                if include_relations {
                    Self::collect_typed_relations(
                        sf,
                        obj,
                        &class_by_path,
                        &obj_name,
                        &mut relations,
                    );
                }
                assets.push(AssetEntry {
                    path_id: obj.path_id,
                    class_id: obj.class_id,
                    class_name,
                    asset_name: obj_name,
                    byte_size: obj.byte_size,
                });
            }
        }

        BundleEntry {
            path: bundle_path.to_string_lossy().to_string(),
            md5,
            file_size,
            unity_version,
            asset_count: assets.len(),
            assets,
            containers,
            externals,
            internal_names,
            relations,
        }
    }

    fn describe_map_parse_failure(path: &str, error: &str) -> String {
        let has_malformed_unityfs_prefix = fs::File::open(path)
            .and_then(|mut file| {
                let mut header = [0u8; 8];
                file.read_exact(&mut header)?;
                Ok(header)
            })
            .map(|header| header.starts_with(b"UnityFS\0"))
            .unwrap_or(false);

        if has_malformed_unityfs_prefix && error.contains("Failed to read unity_version") {
            format!(
                "{}; UnityFS header is malformed after the signature, so this bundle was not indexed. \
                 Re-run decryption with a supported algorithm/source file before rebuilding AssetMap.",
                error
            )
        } else {
            error.to_string()
        }
    }

    fn should_skip_map_fallback(error: &str) -> bool {
        error.contains("Failed to decompress block")
    }

    fn push_internal_name(rows: &mut Vec<(String, String)>, name: &str, kind: &str) {
        let normalized = name.replace('\\', "/").to_lowercase();
        if normalized.is_empty() {
            return;
        }
        rows.push((normalized.clone(), kind.to_string()));
        if let Some(file_name) = normalized.rsplit('/').next() {
            if !file_name.is_empty() && file_name != normalized {
                rows.push((file_name.to_string(), kind.to_string()));
            }
        }
    }

    fn external_match_name(path_name: &str, file_name: &str) -> String {
        if file_name.is_empty() {
            path_name.to_string()
        } else if path_name.eq_ignore_ascii_case(file_name) {
            path_name.to_string()
        } else {
            format!("{}|{}", path_name, file_name)
        }
    }

    fn collect_typed_relations(
        sf: &crate::common::bundle_file::asset_bundle::SerializedFile,
        obj: &crate::common::bundle_file::asset_bundle::ObjectInfo,
        class_by_path: &HashMap<i64, u16>,
        obj_name: &str,
        relations: &mut Vec<BundleRelationEntry>,
    ) {
        UnityRelationCollector::collect_object_relations(
            &sf.inner,
            &obj.inner,
            &class_by_path,
            relations,
            obj_name,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::AssetMapService;

    #[test]
    fn skips_fallback_for_block_decompression_failures() {
        assert!(AssetMapService::should_skip_map_fallback(
            "Failed to decompress block 0 (uncompressed=131072, type=3): LZ4 decompression failed"
        ));
    }

    #[test]
    fn keeps_fallback_for_non_decompression_failures() {
        assert!(!AssetMapService::should_skip_map_fallback(
            "Failed to read unity_version"
        ));
    }
}
