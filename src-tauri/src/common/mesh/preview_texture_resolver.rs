use crate::common::asset_map::asset_index::AssetDatabase;
use crate::common::asset_map::dependency_resolver::AssetMapDependencyResolver;
use crate::common::scan::scan_types::ProgressPayload;
use crate::exporter::material_info::MaterialInfo;
use tauri::ipc::Channel;

pub struct MeshPreviewTextureResolver;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MeshTextureDependencyManifest {
    pub materials: Vec<MaterialInfo>,
    pub texture_candidates: Vec<MeshTextureDependencyCandidate>,
}

#[derive(Debug, Clone)]
pub struct MeshTextureDependencyCandidate {
    pub texture_path_id: i64,
    pub texture_bundle_path: String,
    pub texture_name: String,
    pub material_name: String,
    pub material_index: usize,
    pub slot_name: String,
    pub usage: String,
}

impl MeshPreviewTextureResolver {
    pub fn resolve_manifest(
        db: &AssetDatabase,
        mesh_path_id: i64,
        mesh_name: &str,
        mesh_bundle_path: &str,
        progress: &Channel<ProgressPayload>,
    ) -> Result<MeshTextureDependencyManifest, String> {
        let (materials, texture_refs) = AssetMapDependencyResolver::resolve_mesh_textures(
            db,
            mesh_bundle_path,
            mesh_path_id,
            mesh_name,
            progress,
        )?;

        let mut texture_candidates = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (path_id, bundle_path, texture_name) in texture_refs {
            if !seen.insert((path_id, bundle_path.clone())) {
                continue;
            }
            let (material_name, material_index, slot_name, usage) =
                Self::slot_info_for_texture(&materials, path_id, &bundle_path);
            texture_candidates.push(MeshTextureDependencyCandidate {
                texture_path_id: path_id,
                texture_bundle_path: bundle_path,
                texture_name,
                material_name,
                material_index,
                slot_name,
                usage,
            });
        }

        texture_candidates.sort_by(|left, right| {
            let left_score = Self::default_texture_score(
                &left.material_name,
                &left.slot_name,
                &left.usage,
                &left.texture_name,
            );
            let right_score = Self::default_texture_score(
                &right.material_name,
                &right.slot_name,
                &right.usage,
                &right.texture_name,
            );
            right_score
                .cmp(&left_score)
                .then_with(|| left.texture_name.cmp(&right.texture_name))
        });

        Ok(MeshTextureDependencyManifest {
            materials,
            texture_candidates,
        })
    }

    pub fn find_texture_candidates(
        db: &AssetDatabase,
        mesh_path_id: i64,
        mesh_name: &str,
        mesh_bundle_path: &str,
        progress: &Channel<ProgressPayload>,
    ) -> Vec<(i64, String, String, String, usize, String, String)> {
        let manifest =
            match Self::resolve_manifest(db, mesh_path_id, mesh_name, mesh_bundle_path, progress) {
                Ok(manifest) => manifest,
                Err(e) => {
                    progress
                        .send(ProgressPayload {
                            step: "diffuse".into(),
                            message: format!("Texture candidate dependency lookup failed: {}", e),
                        })
                        .ok();
                    return Vec::new();
                }
            };

        manifest
            .texture_candidates
            .into_iter()
            .map(|candidate| {
                (
                    candidate.texture_path_id,
                    candidate.texture_bundle_path,
                    candidate.texture_name,
                    candidate.material_name,
                    candidate.material_index,
                    candidate.slot_name,
                    candidate.usage,
                )
            })
            .collect()
    }

    fn slot_info_for_texture(
        materials: &[MaterialInfo],
        texture_path_id: i64,
        texture_bundle_path: &str,
    ) -> (String, usize, String, String) {
        for (material_index, material) in materials.iter().enumerate() {
            for slot in &material.textures {
                if Self::texture_ref_matches(
                    &slot.relative_path,
                    texture_path_id,
                    texture_bundle_path,
                ) {
                    return (
                        material.name.clone(),
                        material_index,
                        slot.slot_name.clone(),
                        slot.usage.clone(),
                    );
                }
            }
        }
        ("".to_string(), 0, "".to_string(), "".to_string())
    }

    fn texture_ref_matches(
        relative_path: &str,
        texture_path_id: i64,
        texture_bundle_path: &str,
    ) -> bool {
        if Self::texture_ref_path_id(relative_path) != Some(texture_path_id) {
            return false;
        }
        let Some(bundle_path) = Self::texture_ref_bundle_path(relative_path) else {
            return true;
        };
        bundle_path.eq_ignore_ascii_case(texture_bundle_path)
    }

    fn texture_ref_path_id(relative_path: &str) -> Option<i64> {
        let inner = relative_path.strip_prefix('#')?;
        let id_part = inner.split('@').next().unwrap_or(inner);
        let path_id_part = id_part
            .split_once(':')
            .map(|(_, path_id)| path_id)
            .unwrap_or(id_part);
        path_id_part.parse::<i64>().ok()
    }

    fn texture_ref_bundle_path(relative_path: &str) -> Option<&str> {
        relative_path
            .split_once('@')
            .map(|(_, bundle_path)| bundle_path)
    }

    fn default_texture_score(
        material_name: &str,
        slot_name: &str,
        usage: &str,
        texture_name: &str,
    ) -> i32 {
        let haystack =
            format!("{} {} {} {}", material_name, slot_name, usage, texture_name).to_lowercase();
        let mut score = 0;
        if usage.eq_ignore_ascii_case("DiffuseColor") {
            score += 1000;
        }
        if Self::is_helper_material_name(material_name) {
            score -= 1400;
        }
        for token in [
            "_maintex",
            "_basemap",
            "_basecolormap",
            "diffuse",
            "albedo",
            "basecolor",
        ] {
            if haystack.contains(token) {
                score += 100;
            }
        }
        for token in ["_n.", "_n_", "_normal", "normal", "bump", "nrm"] {
            if haystack.contains(token) {
                score -= 700;
            }
        }
        for token in ["spec", "metal", "rough", "emiss", "mask", "ao"] {
            if haystack.contains(token) {
                score -= 300;
            }
        }
        for token in ["outline", "hatch", "_line", " line", "_ink"] {
            if haystack.contains(token) {
                score -= 500;
            }
        }
        score
    }

    fn is_helper_material_name(material_name: &str) -> bool {
        let name = material_name.to_lowercase();
        name == "lit"
            || name.contains("outline")
            || name.contains("hatch")
            || name.contains("shadow")
            || name.contains("stencil")
            || name.contains("debug")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exporter::material_info::{MaterialInfo, TextureSlot};

    #[test]
    fn slot_info_matches_texture_refs_with_explicit_file_id() {
        let materials = vec![MaterialInfo {
            name: "female_Body".to_string(),
            textures: vec![TextureSlot {
                slot_name: "_MainTex".to_string(),
                usage: "DiffuseColor".to_string(),
                file_name: "tex.dds".to_string(),
                relative_path: "#7:-367321471771861468@D:\\NarakaAssets\\c\\o".to_string(),
            }],
            ..Default::default()
        }];

        let (material_name, material_index, slot_name, usage) =
            MeshPreviewTextureResolver::slot_info_for_texture(
                &materials,
                -367321471771861468,
                "D:\\NarakaAssets\\c\\o",
            );

        assert_eq!(material_name, "female_Body");
        assert_eq!(material_index, 0);
        assert_eq!(slot_name, "_MainTex");
        assert_eq!(usage, "DiffuseColor");
    }

    #[test]
    fn default_texture_score_demotes_outline_hatch_materials() {
        let body_score = MeshPreviewTextureResolver::default_texture_score(
            "mat_plague_doctor_origin_1",
            "_Base",
            "DiffuseColor",
            "tex_plague_doctor_origin_1_col",
        );
        let outline_score = MeshPreviewTextureResolver::default_texture_score(
            "mat_default_character_outline",
            "_Base",
            "DiffuseColor",
            "tex_hatch_lines",
        );

        assert!(body_score > outline_score);
    }

    #[test]
    fn slot_info_does_not_match_same_path_id_from_different_bundle() {
        let materials = vec![MaterialInfo {
            name: "mat_a".to_string(),
            textures: vec![TextureSlot {
                slot_name: "_Base".to_string(),
                usage: "DiffuseColor".to_string(),
                file_name: "tex.dds".to_string(),
                relative_path: "#0:42@A.bundle".to_string(),
            }],
            ..Default::default()
        }];

        let (material_name, _, slot_name, _) =
            MeshPreviewTextureResolver::slot_info_for_texture(&materials, 42, "B.bundle");

        assert!(material_name.is_empty());
        assert!(slot_name.is_empty());
    }
}
