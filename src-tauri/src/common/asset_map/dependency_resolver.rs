use crate::common::asset_map::asset_index::AssetDatabase;
use crate::common::asset_map::asset_map_service::AssetMapService;
use crate::exporter::material_info::MaterialInfo;
use tauri::ipc::Channel;

use crate::common::scan::scan_types::ProgressPayload;

pub struct AssetMapDependencyResolver;

impl AssetMapDependencyResolver {
    pub fn resolve_mesh_textures(
        db: &AssetDatabase,
        bundle_path: &str,
        path_id: i64,
        class_name: &str,
        progress: &Channel<ProgressPayload>,
    ) -> Result<(Vec<MaterialInfo>, Vec<(i64, String, String)>), String> {
        AssetMapService::resolve_dependencies(db, bundle_path, path_id, class_name, progress)
    }

    pub fn resolve_material_refs(
        db: &AssetDatabase,
        material_refs: &[(i64, String)],
        mesh_asset_name: &str,
        progress: &Channel<ProgressPayload>,
    ) -> Result<(Vec<MaterialInfo>, Vec<(i64, String, String)>), String> {
        crate::common::asset_map::material_texture_resolver::MaterialTextureResolver::resolve_material_refs(
            db,
            material_refs,
            mesh_asset_name,
            progress,
        )
    }

    pub fn find_mesh_material_refs(
        db: &AssetDatabase,
        bundle_path: &str,
        path_id: i64,
        progress: &Channel<ProgressPayload>,
    ) -> Vec<(i64, String)> {
        crate::common::asset_map::material_texture_resolver::MaterialTextureResolver::find_mesh_material_refs(
            db,
            bundle_path,
            path_id,
            progress,
        )
    }

    pub fn resolve_material_textures(
        db: &AssetDatabase,
        bundle_path: &str,
        path_id: i64,
        progress: &Channel<ProgressPayload>,
    ) -> Result<(MaterialInfo, Vec<(i64, String, String)>), String> {
        AssetMapService::resolve_material_textures(db, bundle_path, path_id, progress)
    }
}
