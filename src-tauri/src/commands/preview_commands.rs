use std::path::PathBuf;

use crate::common::preview::asset_preview_service::AssetPreviewService;
use crate::common::preview::preview_types::AssetPreviewResult;
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::task::task_runner::TaskRunner;

#[tauri::command]
pub async fn get_asset_typed_preview(
    bundle_path: String,
    path_id: String,
    workspace_dir: Option<String>,
    asset_map_cache_root: Option<String>,
    progress: tauri::ipc::Channel<ProgressPayload>,
) -> Result<AssetPreviewResult, String> {
    let path_id = path_id
        .parse::<i64>()
        .map_err(|e| format!("Invalid path_id: {}", e))?;
    let workspace = workspace_dir
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let cache_root = asset_map_cache_root
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);

    TaskRunner::run_blocking(
        "asset_preview",
        "Asset Preview",
        format!("Preview asset: {} (path_id={})", bundle_path, path_id),
        Some("Asset preview ready".to_string()),
        move |ctx| {
            ctx.progress("preview", 0, 1, "Building asset preview");
            ctx.check_cancelled()?;
            let asset_map_cache_root = workspace.as_ref().and(cache_root.as_deref());
            let result = AssetPreviewService::preview(
                &bundle_path,
                path_id,
                workspace.as_deref(),
                asset_map_cache_root,
                Some(&progress),
            )?;
            ctx.progress("preview", 1, 1, "Asset preview ready");
            Ok(result)
        },
    )
    .await
}
