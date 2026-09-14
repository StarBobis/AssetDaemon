use std::collections::HashMap;

use crate::common::serialized_file::serialized_file::{ObjectInfo, SerializedFile};
use crate::exporter::material_info::{MaterialInfo, TextureSlot};
use crate::unity::classes::registry::UnityClassParser;

pub struct UnityMaterialTextureExtractor;

impl UnityMaterialTextureExtractor {
    pub fn extract(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
        fallback_material_name: &str,
        material_bundle_path: &str,
    ) -> Option<(MaterialInfo, HashMap<i64, i32>)> {
        let material = UnityClassParser::parse_material(serialized_file, object_info).ok()?;
        let mut material_info = MaterialInfo {
            name: if material.name.is_empty() {
                fallback_material_name.to_string()
            } else {
                material.name
            },
            diffuse_color: material.diffuse_color,
            specular_color: material.specular_color,
            emissive_color: material.emissive_color,
            shininess: material.shininess,
            textures: Vec::new(),
        };
        let mut file_id_by_path_id = HashMap::new();

        for texture_slot in material.texture_slots {
            if texture_slot.texture.is_null() {
                continue;
            }
            if texture_slot.texture.file_id != 0 {
                file_id_by_path_id
                    .insert(texture_slot.texture.path_id, texture_slot.texture.file_id);
            }
            material_info.textures.push(TextureSlot {
                slot_name: texture_slot.slot_name.clone(),
                usage: Self::slot_usage(&texture_slot.slot_name).to_string(),
                file_name: format!("tex_{}.png", texture_slot.texture.path_id),
                relative_path: Self::encode_texture_ref(
                    texture_slot.texture.file_id,
                    texture_slot.texture.path_id,
                    material_bundle_path,
                ),
            });
        }

        Some((material_info, file_id_by_path_id))
    }

    pub fn slot_usage(slot_name: &str) -> &'static str {
        let base_slot_name = slot_name.split('#').next().unwrap_or(slot_name);
        match base_slot_name {
            "_MainTex" | "_Base" | "_BaseMap" | "_BaseColorMap" => "DiffuseColor",
            "_BumpMap" | "_NormalMap" => "NormalMap",
            "_SpecGlossMap" => "SpecularColor",
            "_EmissionMap" => "EmissiveColor",
            "_OcclusionMap" => "OcclusionMap",
            "_RoughnessMap" => "RoughnessMap",
            "_MetallicGlossMap" => "MetallicMap",
            "_MaskMap" => "MaskMap",
            "_LineMap" => "LineMap",
            "_SkinMap" => "SkinMap",
            "_OtherMap" => "OtherMap",
            _ => "Unknown",
        }
    }

    pub fn encode_texture_ref(file_id: i32, path_id: i64, source_bundle: &str) -> String {
        format!("#{}:{}@{}", file_id, path_id, source_bundle)
    }

    pub fn decode_texture_ref(relative_path: &str) -> Option<(i32, i64)> {
        let inner = relative_path.strip_prefix('#')?;
        let ptr = inner.split('@').next().unwrap_or(inner);
        if let Some((file_id, path_id)) = ptr.split_once(':') {
            Some((file_id.parse().ok()?, path_id.parse().ok()?))
        } else {
            Some((0, ptr.parse().ok()?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::UnityMaterialTextureExtractor;

    #[test]
    fn texture_ref_round_trips_explicit_file_id() {
        let encoded =
            UnityMaterialTextureExtractor::encode_texture_ref(7, -367321471771861468, "bundle");

        assert_eq!(
            UnityMaterialTextureExtractor::decode_texture_ref(&encoded),
            Some((7, -367321471771861468))
        );
    }
}
