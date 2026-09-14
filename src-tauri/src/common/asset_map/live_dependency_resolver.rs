use std::collections::HashMap;
use std::path::Path;

use tauri::ipc::Channel;

use crate::common::asset_map::asset_index::AssetDatabase;
use crate::common::bundle_file::asset_bundle::{AssetBundle, AssetBundleLoader};
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::unity_dependency::external_resolver::UnityExternalResolver;
use crate::common::unity_dependency::material_texture_extractor::UnityMaterialTextureExtractor;
use crate::exporter::material_info::MaterialInfo;
use crate::utils::unity_object_name_utils::UnityObjectNameUtils;

pub struct LiveDependencyResolver {
    bundle_cache: HashMap<String, AssetBundle>,
}

impl LiveDependencyResolver {
    pub fn new() -> Self {
        Self {
            bundle_cache: HashMap::new(),
        }
    }

    pub fn resolve_material_textures(
        &mut self,
        db: &AssetDatabase,
        material_bundle_path: &str,
        material_path_id: i64,
        progress: &Channel<ProgressPayload>,
    ) -> Option<(MaterialInfo, Vec<(i64, String, String)>)> {
        let bundle = Self::load_bundle_cached(&mut self.bundle_cache, material_bundle_path)?;
        let (sf_index, material_name, material_info) = {
            let (sf_index, sf, obj) = bundle.assets.iter().enumerate().find_map(|(idx, sf)| {
                sf.objects
                    .iter()
                    .find(|obj| obj.path_id == material_path_id && obj.class_id == 21)
                    .map(|obj| (idx, sf, obj))
            })?;

            let material_name = UnityObjectNameUtils::peek_object_name(&sf.inner, &obj.inner)
                .ok()
                .flatten()
                .unwrap_or_else(|| format!("material_{}", material_path_id));
            let material_info = Self::parse_material(
                bundle,
                material_path_id,
                &material_name,
                material_bundle_path,
                progress,
            )?;
            (sf_index, material_name, material_info)
        };

        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG] live resolver parsed Material '{}' with {} texture slot(s)",
                material_name,
                material_info.textures.len()
            ),
        });

        let bundle = Self::load_bundle_cached(&mut self.bundle_cache, material_bundle_path)?;
        let externals = bundle
            .assets
            .get(sf_index)
            .map(|sf| sf.inner.m_externals.clone())
            .unwrap_or_default();

        let mut texture_refs = Vec::new();
        for slot in &material_info.textures {
            let Some((file_id, path_id)) =
                UnityMaterialTextureExtractor::decode_texture_ref(&slot.relative_path)
            else {
                continue;
            };
            let target_bundle_path = if file_id == 0 {
                Some(material_bundle_path.to_string())
            } else {
                let external = externals.get(file_id.checked_sub(1)? as usize)?;
                let name = if external.file_name.is_empty() {
                    external.path_name.as_str()
                } else {
                    external.file_name.as_str()
                };
                Self::resolve_external_bundle(db, name)
            };
            let Some(target_bundle_path) = target_bundle_path else {
                continue;
            };
            let Some(texture_name) = self.texture_name(&target_bundle_path, path_id) else {
                continue;
            };
            texture_refs.push((path_id, target_bundle_path, texture_name));
        }

        Some((material_info, texture_refs))
    }

    fn parse_material(
        bundle: &AssetBundle,
        material_path_id: i64,
        material_name: &str,
        material_bundle_path: &str,
        progress: &Channel<ProgressPayload>,
    ) -> Option<MaterialInfo> {
        let (_sf_index, sf, obj) = bundle.assets.iter().enumerate().find_map(|(idx, sf)| {
            sf.objects
                .iter()
                .find(|obj| obj.path_id == material_path_id && obj.class_id == 21)
                .map(|obj| (idx, sf, obj))
        })?;

        let (material_info, _) = UnityMaterialTextureExtractor::extract(
            &sf.inner,
            &obj.inner,
            material_name,
            material_bundle_path,
        )?;

        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG] live resolver Material parser produced {} texture slot(s)",
                material_info.textures.len()
            ),
        });
        Some(material_info)
    }

    fn load_bundle_cached<'a>(
        bundle_cache: &'a mut HashMap<String, AssetBundle>,
        bundle_path: &str,
    ) -> Option<&'a AssetBundle> {
        if !bundle_cache.contains_key(bundle_path) {
            let bundle =
                AssetBundleLoader::load_bundle_serialized_only(Path::new(bundle_path)).ok()?;
            bundle_cache.insert(bundle_path.to_string(), bundle);
        }
        bundle_cache.get(bundle_path)
    }

    fn texture_name(&mut self, bundle_path: &str, path_id: i64) -> Option<String> {
        let bundle = Self::load_bundle_cached(&mut self.bundle_cache, bundle_path)?;
        bundle.assets.iter().find_map(|sf| {
            sf.objects
                .iter()
                .find(|obj| obj.path_id == path_id && matches!(obj.class_id, 28 | 213 | 271))
                .map(|obj| {
                    UnityObjectNameUtils::peek_object_name(&sf.inner, &obj.inner)
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| format!("texture_{}", path_id))
                })
        })
    }

    fn resolve_external_bundle(db: &AssetDatabase, name: &str) -> Option<String> {
        UnityExternalResolver::resolve_from_db(db, name)
    }
}
