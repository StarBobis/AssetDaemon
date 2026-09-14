/*
 * AssetDaemon Application Entry.
 *
 * Module Declarations:
 * - commands:   All #[tauri::command] functions are centralized in this directory
 * - workspace:  Workspace-level asset map, export, and dependency orchestration
 * - unity:      Unity asset parsing (mesh_helper, mesh_asset_studio, mesh_heuristic_parser)
 * - common:     General utilities (asset_bundle, serialized_file, texture_reader, etc.)
 * - decrypt:    Game-specific decryptors
 *
 * Follows Soul.md conventions:
 * - Utilities use struct + impl organization, each .rs file contains only one utility class
 * - File names use snake_case
 * - Tauri commands are placed separately under src/commands/
 */

mod commands;
mod common;
mod decrypt;
mod exporter;
mod service;
mod unity;
mod utils;

/**
 * Application main entry class.
 *
 * Follows Soul.md conventions, wrapping the entry function in struct + impl.
 * Responsible for initializing Tauri plugins and registering all command handlers.
 */
pub struct Application;

impl Application {
    #[cfg_attr(mobile, tauri::mobile_entry_point)]
    pub fn run() {
        tauri::Builder::default()
            .setup(|app| {
                use tauri::Manager;
                // Install the global panic hook as early as possible so every
                // background-task panic is written to a durable on-disk log.
                let logs_dir = app
                    .path()
                    .app_data_dir()
                    .unwrap_or_else(|_| std::env::temp_dir().join("assetdaemon"))
                    .join("logs");
                crate::common::panic_reporter::PanicReporter::install(
                    logs_dir,
                    app.handle().clone(),
                );
                Ok(())
            })
            .plugin(tauri_plugin_opener::init())
            .plugin(tauri_plugin_dialog::init())
            .plugin(tauri_plugin_fs::init())
            .plugin(tauri_plugin_store::Builder::default().build())
            .invoke_handler(tauri::generate_handler![
                commands::decrypt_command::decrypt_gf2_files,
                commands::decrypt_command::decrypt_naraka_bladepoint_files,
                commands::decrypt_command::decrypt_the_magic_blade_files,
                commands::bundle_commands::scan_bundle_files,
                commands::bundle_commands::collect_bundle_paths,
                commands::bundle_commands::parse_bundle_metadata,
                commands::bundle_commands::get_scene_hierarchy,
                commands::batch_scan_command::batch_scan_types,
                commands::bundle_commands::extract_node_to_file,
                commands::mesh_commands::get_mesh_info,
                commands::mesh_commands::extract_mesh_geometry,
                commands::mesh_commands::extract_animator_geometry,
                commands::mesh_commands::extract_animator_preview_glb,
                commands::mesh_commands::extract_game_object_preview_glb,
                commands::mesh_commands::clear_asset_preview_cache,
                commands::mesh_commands::clear_all_preview_cache,
                commands::mesh_commands::extract_bundle_to_cache,
                commands::mesh_commands::resolve_mesh_preview_diffuse_texture,
                commands::mesh_commands::resolve_mesh_preview_texture_candidates,
                commands::mesh_commands::resolve_preview_animators,
                commands::mesh_commands::resolve_preview_animation_clips,
                commands::preview_commands::get_asset_typed_preview,
                commands::bundle_commands::export_mesh_preview,
                commands::texture_commands::export_texture_preview,
                commands::dump_commands::get_asset_dump,
                commands::export_commands::start_export_batch,
                commands::export_commands::cancel_export,
                commands::export_commands::list_dependency_export_assets,
                commands::export_commands::export_mesh_with_dependencies,
                commands::export_commands::export_material_with_dependencies,
                commands::export_commands::export_game_object_with_dependencies,
                commands::export_commands::export_animator_with_dependencies,
                commands::export_commands::build_asset_map,
                commands::export_commands::clear_asset_map,
                commands::export_commands::get_map_summary,
                commands::export_commands::get_map_bundle_paths_by_classes,
                commands::export_commands::query_map_assets,
                commands::export_commands::get_asset_search_index_status,
                commands::export_commands::build_asset_search_index,
                commands::export_commands::get_map_asset_class_stats,
                commands::export_commands::get_map_bundle_asset_class_stats,
                commands::task_commands::register_log_channel,
                commands::task_commands::stop_all_tasks,
                commands::task_commands::get_running_tasks,
            ])
            .run(tauri::generate_context!())
            .expect("error while running tauri application");
    }
}
