use crate::common::command_types::DumpResult;
use crate::common::task::task_runner::TaskRunner;
use crate::service::asset_dump_service::AssetDumpService;

/**
 * Get TypeTree dump data for the specified asset.
 *
 * Reads directly from TypeTree + ObjectReader, preserves type names,
 * outputs AssetStudio-style structured text.
 * Falls back to raw byte hex dump when TypeTree is unavailable.
 */
#[tauri::command]
pub async fn get_asset_dump(bundle_path: String, path_id: String) -> Result<DumpResult, String> {
    TaskRunner::run_blocking(
        "asset_dump",
        "Asset Dump",
        format!("Dump asset: {} (path_id={})", bundle_path, path_id),
        Some("Asset dump ready".to_string()),
        move |ctx| AssetDumpService::get_dump(&ctx, &bundle_path, &path_id),
    )
    .await
}
