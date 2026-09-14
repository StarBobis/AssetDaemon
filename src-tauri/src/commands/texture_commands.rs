use crate::common::command_types::TexturePreviewResult;
use crate::common::task::task_runner::TaskRunner;
use crate::service::texture_preview_service::TexturePreviewService;

#[tauri::command]
pub async fn export_texture_preview(
    bundle_path: String,
    path_id: String,
    cache_dir: String,
    max_preview_edge: Option<u32>,
) -> Result<TexturePreviewResult, String> {
    TaskRunner::run_blocking(
        "texture_preview",
        "Texture Preview",
        format!(
            "Export texture preview: {} (path_id={})",
            bundle_path, path_id
        ),
        Some("Texture preview ready".to_string()),
        move |ctx| {
            TexturePreviewService::export_preview(
                &ctx,
                &bundle_path,
                &path_id,
                &cache_dir,
                max_preview_edge,
            )
        },
    )
    .await
}
