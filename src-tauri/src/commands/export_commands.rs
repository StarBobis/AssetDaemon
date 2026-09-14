/*
 * export_commands.rs - batch export Tauri commands.
 *
 * Provides export entry points called from frontend:
 *  - start_export_batch: start batch export (spawn_blocking + Channel progress)
 *  - cancel_export: cancel an ongoing export
 *
 * Follows Soul.md convention: commands separated from service classes.
 */

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use tauri::ipc::Channel;

use crate::common::asset_map::asset_map_service::AssetMapService;
use crate::common::asset_map::asset_map_types::{
    AssetSearchIndexStatus, MapAssetClassStat, MapAssetQueryOptions, MapAssetQueryResult,
    MapSummary,
};
use crate::common::asset_map::query_service::AssetMapQueryService;
use crate::common::command_types::{DependencyExportAssetRef, DependencyExportResult};
use crate::common::export::common_types::{ExportJob, ExportOptions, ExportProgress, ExportReport};
use crate::common::export::dependency_asset_selector::DependencyAssetSelector;
use crate::common::export::export_service::ExportService;
use crate::common::mesh::animator_preview_workflow::AnimatorPreviewWorkflow;
use crate::common::mesh::game_object_preview_workflow::GameObjectPreviewWorkflow;
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::task::task_runner::TaskRunner;

static RUNNING_ASSET_SEARCH_INDEX_BUILDS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn asset_map_cache_root_path(value: Option<String>) -> Result<std::path::PathBuf, String> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(std::path::PathBuf::from)
        .ok_or_else(|| {
            "AssetMap cache root is required; configure the global AssetMap cache folder"
                .to_string()
        })
}

/// Start a batch export task.
///
/// Runs ExportService::run_batch in spawn_blocking;
/// ExportService registers with TaskManager internally and checks the cancel token.
/// Pushes progress events to the frontend via Channel, returns ExportReport on completion.
#[tauri::command]
pub async fn start_export_batch(
    job: ExportJob,
    progress: Channel<ExportProgress>,
) -> Result<ExportReport, String> {
    let job_id = job.job_id.clone();
    let asset_count = job.assets.len();
    TaskRunner::run_blocking_with_id(
        job_id,
        "Batch Export",
        format!("Received export request: {} assets", asset_count),
        None,
        move |ctx| {
            Ok(ExportService::run_batch_with_cancel_token(
                job,
                progress,
                ctx.cancel_token(),
            ))
        },
    )
    .await
}

/// Cancel a specified export task.
#[tauri::command]
pub fn cancel_export(job_id: String) -> Result<(), String> {
    ExportService::cancel_job(&job_id);
    Ok(())
}

/// Resolve exportable assets related to a Mesh or Material.
///
/// This is intentionally a light AssetMap query used before export so the UI can let the user
/// choose individual related assets without eagerly parsing or exporting Bundles.
/// GameObject dependency export uses GameObjectPreviewWorkflow directly; do not mirror that
/// parsing here with a stale AssetMap graph approximation.
#[tauri::command]
pub async fn list_dependency_export_assets(
    bundle_path: String,
    path_id: String,
    class_name: String,
    workspace_dir: String,
    asset_map_cache_root: Option<String>,
) -> Result<Vec<DependencyExportAssetRef>, String> {
    TaskRunner::run_blocking(
        "list_dependency_export_assets",
        "Resolve Related Export Assets",
        format!(
            "Resolve related export assets: {} {} (path_id={})",
            class_name, bundle_path, path_id
        ),
        Some("Related export assets ready".to_string()),
        move |ctx| {
            let path_id_i64 = path_id
                .parse::<i64>()
                .map_err(|e| format!("Invalid path_id: {}", e))?;
            let cache_root = asset_map_cache_root_path(asset_map_cache_root)?.into_boxed_path();
            ctx.info(format!(
                "Opening AssetMap for dependency lookup: workspace={}, cache_root={}",
                workspace_dir,
                cache_root.display()
            ));
            ctx.info(format!(
                "Resolving {} dependencies from AssetMap: {} (path_id={})",
                class_name, bundle_path, path_id_i64
            ));
            let assets = DependencyAssetSelector::select_for_workspace_with_task(
                std::path::Path::new(&workspace_dir),
                Some(cache_root.as_ref()),
                &bundle_path,
                path_id_i64,
                &class_name,
                &ctx,
            )?;
            ctx.info(format!(
                "Dependency lookup complete: {} exportable related assets",
                assets.len()
            ));
            Ok(assets)
        },
    )
    .await
}

/// Export Mesh and its associated assets (Material + Texture2D).
///
/// Prefer pre-built AssetMap for second-level queries; fall back to real-time scan when no Map.
/// Texture export uses TextureReader version-aware parsing, supports streaming textures (.resS).
#[tauri::command]
pub async fn export_mesh_with_dependencies(
    bundle_path: String,
    path_id: String,
    workspace_dirs: Vec<String>,
    output_dir: String,
    progress: Channel<crate::common::scan::scan_types::ProgressPayload>,
    asset_map_cache_root: Option<String>,
    options: Option<ExportOptions>,
) -> Result<DependencyExportResult, String> {
    let cache_root = asset_map_cache_root_path(asset_map_cache_root)?;
    TaskRunner::run_blocking(
        "export_mesh_deps",
        "Export Mesh Dependencies",
        format!(
            "Export mesh dependencies: {} (path_id={})",
            bundle_path, path_id
        ),
        None,
        move |ctx| {
            let cancel_token = ctx.cancel_token();
            let res = ExportService::export_mesh_with_dependencies(
                bundle_path,
                path_id,
                workspace_dirs,
                output_dir,
                progress,
                &cancel_token,
                ctx.task_id(),
                Some(cache_root.as_path()),
                options.unwrap_or_default(),
            );

            match &res {
                Ok(r) => {
                    ctx.success(format!(
                        "Mesh export complete: {} ({} vertices, {} triangles, {} materials, {} textures)",
                        r.output_path, r.vertex_count, r.triangle_count,
                        r.material_count, r.texture_count
                    ));
                }
                Err(e) => {
                    ctx.error(format!("Mesh export failed: {}", e));
                }
            }
            res
        },
    )
    .await
}

/// Export Material and its associated textures.
///
/// Parse Material with the AssetStudio layout parser to extract texture PPtr references,
/// resolve Texture2D through the preview dependency path and export PNG sidecar files.
/// Also export Material properties as JSON.
#[tauri::command]
pub async fn export_material_with_dependencies(
    bundle_path: String,
    path_id: String,
    workspace_dirs: Vec<String>,
    output_dir: String,
    progress: Channel<crate::common::scan::scan_types::ProgressPayload>,
    asset_map_cache_root: Option<String>,
    options: Option<ExportOptions>,
) -> Result<DependencyExportResult, String> {
    let cache_root = asset_map_cache_root_path(asset_map_cache_root)?;
    TaskRunner::run_blocking(
        "export_material_deps",
        "Export Material Dependencies",
        format!(
            "Export Material and associated textures: {} (path_id={})",
            bundle_path, path_id
        ),
        None,
        move |ctx| {
            let cancel_token = ctx.cancel_token();
            let res = ExportService::export_material_with_dependencies(
                bundle_path,
                path_id,
                workspace_dirs,
                output_dir,
                progress,
                &cancel_token,
                ctx.task_id(),
                Some(cache_root.as_path()),
                options.unwrap_or_default(),
            );

            match &res {
                Ok(r) => {
                    ctx.success(format!(
                        "Material export complete: {} ({} materials, {} textures)",
                        r.output_path, r.material_count, r.texture_count
                    ));
                }
                Err(e) => {
                    ctx.error(format!("Material export failed: {}", e));
                }
            }
            res
        },
    )
    .await
}

/// Export GameObject and its associated model assets.
///
/// The export follows the GameObject preview model path: collect child Mesh renderers,
/// resolve shared skeleton and related animations, export PNG sidecar textures, then generate GLB.
#[tauri::command]
pub async fn export_game_object_with_dependencies(
    bundle_path: String,
    path_id: String,
    workspace_dir: String,
    output_dir: String,
    progress: Channel<crate::common::scan::scan_types::ProgressPayload>,
    asset_map_cache_root: Option<String>,
    options: Option<ExportOptions>,
) -> Result<DependencyExportResult, String> {
    TaskRunner::run_blocking(
        "export_game_object_deps",
        "Export GameObject Dependencies",
        format!(
            "Export GameObject dependencies: {} (path_id={})",
            bundle_path, path_id
        ),
        None,
        move |ctx| {
            GameObjectPreviewWorkflow::export_with_dependencies(
                ctx,
                bundle_path,
                path_id,
                workspace_dir,
                output_dir,
                asset_map_cache_root,
                progress,
                options.unwrap_or_default(),
            )
        },
    )
    .await
}

/// Export Animator and its associated model assets.
///
/// The export intentionally reuses the Animator preview GLB workflow so skeleton binding,
/// materials, textures, and animation clips cannot drift from what the viewer shows.
#[tauri::command]
pub async fn export_animator_with_dependencies(
    bundle_path: String,
    path_id: String,
    workspace_dir: String,
    output_dir: String,
    progress: Channel<crate::common::scan::scan_types::ProgressPayload>,
    asset_map_cache_root: Option<String>,
    options: Option<ExportOptions>,
) -> Result<DependencyExportResult, String> {
    TaskRunner::run_blocking(
        "export_animator_deps",
        "Export Animator Dependencies",
        format!(
            "Export Animator dependencies: {} (path_id={})",
            bundle_path, path_id
        ),
        None,
        move |ctx| {
            AnimatorPreviewWorkflow::export_with_dependencies(
                ctx,
                bundle_path,
                path_id,
                workspace_dir,
                output_dir,
                asset_map_cache_root,
                options.unwrap_or_default(),
                progress,
            )
        },
    )
    .await
}

/// Build the lightweight search index (AssetMap).
///
/// Scans Unity files for searchable asset rows and high-value typed relations.
/// Full dependency walks are resolved on demand by export/preview flows.
#[tauri::command]
pub async fn build_asset_map(
    workspace_dir: String,
    progress: Channel<ProgressPayload>,
    memory_limit_bytes: u64,
    asset_map_cache_root: Option<String>,
    skip_ress_list_build: bool,
) -> Result<MapSummary, String> {
    let ws = std::path::PathBuf::from(workspace_dir);
    let cache_root = asset_map_cache_root_path(asset_map_cache_root)?;
    TaskRunner::run_blocking(
        "build_map",
        "Build Search Index",
        format!("Starting lightweight asset index build: {}", ws.display()),
        None,
        move |ctx| {
            let cancel_token = ctx.cancel_token();
            let summary = AssetMapService::build_with_task_id(
                &ws,
                &progress,
                &cancel_token,
                ctx.task_id(),
                memory_limit_bytes,
                Some(cache_root.as_path()),
                skip_ress_list_build,
            )
            .map_err(|e| format!("Failed to build AssetMap: {}", e))?;

            if summary.cancelled {
                ctx.warn(format!(
                    "Search index build stopped: saved {} bundles, {} assets, {} parsed",
                    summary.bundle_count, summary.asset_count, summary.parsed_count
                ));
            } else {
                ctx.success(format!(
                    "Search index build complete: {} bundles, {} assets, {} parsed",
                    summary.bundle_count, summary.asset_count, summary.parsed_count
                ));
            }
            Ok(summary)
        },
    )
    .await
}

/// Clear the AssetMap under the current workspace.
#[tauri::command]
pub async fn clear_asset_map(
    workspace_dir: String,
    asset_map_cache_root: Option<String>,
) -> Result<(), String> {
    TaskRunner::run_blocking(
        "clear_asset_map",
        "Clear AssetMap",
        format!("Clearing AssetMap: {}", workspace_dir),
        Some("AssetMap cleared".to_string()),
        move |_| {
            let ws = std::path::Path::new(&workspace_dir);
            let cache_root = asset_map_cache_root_path(asset_map_cache_root)?;
            AssetMapQueryService::clear_with_cache_root(ws, Some(cache_root.as_path()))
        },
    )
    .await
}

/// Query AssetMap summary info (exists, asset count, etc.)
#[tauri::command]
pub async fn get_map_summary(
    workspace_dir: String,
    asset_map_cache_root: Option<String>,
) -> Result<Option<MapSummary>, String> {
    let ws = std::path::PathBuf::from(workspace_dir);
    let cache_root = asset_map_cache_root_path(asset_map_cache_root)?;
    TaskRunner::run_blocking(
        "map_summary",
        "AssetMap Summary",
        format!("Query AssetMap summary: {}", ws.display()),
        Some("AssetMap summary ready".to_string()),
        move |_| {
            Ok(AssetMapQueryService::summary_with_cache_root(
                &ws,
                Some(cache_root.as_path()),
            ))
        },
    )
    .await
}

/// Query Bundle paths by asset class from the pre-built AssetMap.
#[tauri::command]
pub async fn get_map_bundle_paths_by_classes(
    workspace_dir: String,
    class_names: Vec<String>,
    asset_map_cache_root: Option<String>,
) -> Result<HashMap<String, Vec<String>>, String> {
    TaskRunner::run_blocking(
        "map_bundle_paths",
        "AssetMap Bundle Paths",
        format!(
            "Query Bundle paths for {} classes: {}",
            class_names.len(),
            workspace_dir
        ),
        Some("AssetMap Bundle paths ready".to_string()),
        move |_| {
            let ws = std::path::Path::new(&workspace_dir);
            let cache_root = asset_map_cache_root_path(asset_map_cache_root)?;
            if !AssetMapQueryService::exists_with_cache_root(ws, Some(cache_root.as_path())) {
                return Err("AssetMap is not built".to_string());
            }
            AssetMapQueryService::bundle_paths_for_classes_with_cache_root(
                ws,
                &class_names,
                Some(cache_root.as_path()),
            )
        },
    )
    .await
}

/// Query one page of assets from the pre-built AssetMap.
#[tauri::command]
pub async fn query_map_assets(
    workspace_dir: String,
    options: MapAssetQueryOptions,
    asset_map_cache_root: Option<String>,
) -> Result<MapAssetQueryResult, String> {
    let ws = std::path::PathBuf::from(workspace_dir);
    let cache_root = asset_map_cache_root_path(asset_map_cache_root)?;
    if !AssetMapQueryService::exists_with_cache_root(&ws, Some(cache_root.as_path())) {
        return Err("AssetMap is not built".to_string());
    }
    TaskRunner::run_blocking(
        "map_query_assets",
        "AssetMap Query",
        format!("Query AssetMap assets: {}", ws.display()),
        Some("AssetMap query ready".to_string()),
        move |ctx| {
            AssetMapQueryService::query_assets_with_cache_root_cancelable(
                &ws,
                &options,
                Some(cache_root.as_path()),
                ctx.cancel_token(),
            )
        },
    )
    .await
}

/// Read status for the optional trigram FTS index used by All Assets text filters.
#[tauri::command]
pub async fn get_asset_search_index_status(
    workspace_dir: String,
    asset_map_cache_root: Option<String>,
    silent: Option<bool>,
) -> Result<AssetSearchIndexStatus, String> {
    let ws = std::path::PathBuf::from(workspace_dir);
    let cache_root = asset_map_cache_root_path(asset_map_cache_root)?;
    if silent.unwrap_or(false) {
        if !AssetMapQueryService::exists_with_cache_root(&ws, Some(cache_root.as_path())) {
            return Err("AssetMap is not built".to_string());
        }
        return AssetMapQueryService::asset_search_index_status_with_cache_root(
            &ws,
            Some(cache_root.as_path()),
        );
    }
    TaskRunner::run_blocking(
        "asset_search_status",
        "AssetMap Search Status",
        format!("Query All Assets search index status: {}", ws.display()),
        Some("All Assets search index status ready".to_string()),
        move |_| {
            if !AssetMapQueryService::exists_with_cache_root(&ws, Some(cache_root.as_path())) {
                return Err("AssetMap is not built".to_string());
            }
            AssetMapQueryService::asset_search_index_status_with_cache_root(
                &ws,
                Some(cache_root.as_path()),
            )
        },
    )
    .await
}

/// Start rebuilding the optional All Assets trigram FTS index as a background task.
#[tauri::command]
pub async fn build_asset_search_index(
    workspace_dir: String,
    asset_map_cache_root: Option<String>,
) -> Result<(), String> {
    let ws = std::path::PathBuf::from(workspace_dir);
    let cache_root = asset_map_cache_root_path(asset_map_cache_root)?;
    if !AssetMapQueryService::exists_with_cache_root(&ws, Some(cache_root.as_path())) {
        return Err("AssetMap is not built".to_string());
    }

    let build_key = format!("{}|{}", ws.display(), cache_root.display());
    {
        let mut running = RUNNING_ASSET_SEARCH_INDEX_BUILDS
            .lock()
            .map_err(|_| "Asset search index task registry is poisoned".to_string())?;
        if !running.insert(build_key.clone()) {
            return Ok(());
        }
    }

    let ctx = TaskRunner::create("asset_search_index", "AssetMap Search Index");
    ctx.info(format!(
        "Start All Assets search index background build: {}",
        ws.display()
    ));
    TaskRunner::spawn_blocking_detached(ctx, move |ctx| {
        let result = AssetMapQueryService::build_asset_search_index_with_cache_root(
            &ws,
            Some(cache_root.as_path()),
            &ctx,
        )
        .map(|_| ());
        if let Ok(mut running) = RUNNING_ASSET_SEARCH_INDEX_BUILDS.lock() {
            running.remove(&build_key);
        }
        result
    });
    Ok(())
}

/// Query AssetMap class counts for the All Assets type filter.
#[tauri::command]
pub async fn get_map_asset_class_stats(
    workspace_dir: String,
    asset_map_cache_root: Option<String>,
) -> Result<Vec<MapAssetClassStat>, String> {
    TaskRunner::run_blocking(
        "map_asset_class_stats",
        "AssetMap Class Stats",
        format!("Query AssetMap class stats: {}", workspace_dir),
        Some("AssetMap class stats ready".to_string()),
        move |_| {
            let ws = std::path::Path::new(&workspace_dir);
            let cache_root = asset_map_cache_root_path(asset_map_cache_root)?;
            if !AssetMapQueryService::exists_with_cache_root(ws, Some(cache_root.as_path())) {
                return Err("AssetMap is not built".to_string());
            }
            AssetMapQueryService::all_asset_class_stats_with_cache_root(
                ws,
                Some(cache_root.as_path()),
            )
        },
    )
    .await
}

/// Query AssetMap class counts for one Bundle in the normal Asset List pane.
#[tauri::command]
pub async fn get_map_bundle_asset_class_stats(
    workspace_dir: String,
    bundle_path: String,
    asset_map_cache_root: Option<String>,
) -> Result<Vec<MapAssetClassStat>, String> {
    TaskRunner::run_blocking(
        "map_bundle_class_stats",
        "AssetMap Bundle Stats",
        format!("Query Bundle class stats: {}", bundle_path),
        Some("AssetMap Bundle stats ready".to_string()),
        move |_| {
            let ws = std::path::Path::new(&workspace_dir);
            let bundle = std::path::Path::new(&bundle_path);
            let cache_root = asset_map_cache_root_path(asset_map_cache_root)?;
            if !AssetMapQueryService::exists_with_cache_root(ws, Some(cache_root.as_path())) {
                return Err("AssetMap is not built".to_string());
            }
            AssetMapQueryService::bundle_asset_class_stats_with_cache_root(
                ws,
                bundle,
                Some(cache_root.as_path()),
            )
        },
    )
    .await
}
