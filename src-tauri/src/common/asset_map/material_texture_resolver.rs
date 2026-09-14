use crate::common::asset_map::asset_index::{AssetDatabase, AssetRow};
use crate::common::asset_map::live_dependency_resolver::LiveDependencyResolver;
use crate::common::bundle_file::asset_bundle::AssetBundleLoader;
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::serialized_file::serialized_file::FileIdentifier;
use crate::common::unity_dependency::external_resolver::UnityExternalResolver;
use crate::common::unity_dependency::material_texture_extractor::UnityMaterialTextureExtractor;
use crate::exporter::material_info::{MaterialInfo, TextureSlot};
use crate::unity::classes::object::PPtr;
use crate::unity::classes::registry::UnityClassParser;
use crate::unity::relations::UnityRelationKind;
use crate::unity::type_tree::unity_value::UnityValue;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tauri::ipc::Channel;

pub struct MaterialTextureResolver;

struct DecodedTextureRef<'a> {
    file_id: Option<i32>,
    path_id: i64,
    source_bundle: Option<&'a str>,
}

impl MaterialTextureResolver {
    /// Look up Materials and Texture2D associated with a Mesh from the file index.
    /// Internally maintains a Bundle cache so each bundle is loaded and decompressed only once.
    pub fn resolve_dependencies(
        db: &AssetDatabase,
        mesh_bundle_path: &str,
        mesh_path_id: i64,
        mesh_asset_name: &str,
        progress: &Channel<ProgressPayload>,
    ) -> Result<(Vec<MaterialInfo>, Vec<(i64, String, String)>), String> {
        let mut visited = HashSet::new();
        Self::resolve_dependencies_inner(
            db,
            mesh_bundle_path,
            mesh_path_id,
            mesh_asset_name,
            &mut visited,
            progress,
        )
    }

    fn resolve_dependencies_inner(
        db: &AssetDatabase,
        mesh_bundle_path: &str,
        mesh_path_id: i64,
        mesh_asset_name: &str,
        visited: &mut HashSet<(String, i64)>,
        progress: &Channel<ProgressPayload>,
    ) -> Result<(Vec<MaterialInfo>, Vec<(i64, String, String)>), String> {
        if !visited.insert((mesh_bundle_path.to_string(), mesh_path_id)) {
            return Ok((Vec::new(), Vec::new()));
        }

        let _ = progress.send(ProgressPayload {
            step: "lookup".into(),
            message: "Querying AssetMap index...".into(),
        });

        let exact_material_refs =
            Self::find_mesh_material_refs(db, mesh_bundle_path, mesh_path_id, progress);
        let (materials, texture_refs) = if !exact_material_refs.is_empty() {
            let _ = progress.send(ProgressPayload {
                step: "lookup".into(),
                message: format!(
                    "Resolved {} exact material reference(s) from mesh renderer chain",
                    exact_material_refs.len()
                ),
            });
            Self::resolve_material_refs(db, &exact_material_refs, mesh_asset_name, progress)?
        } else {
            (Vec::new(), Vec::new())
        };

        if !materials.is_empty() {
            if texture_refs.is_empty() {
                if let Some(borrowed) = Self::try_borrow_same_name_mesh_textures(
                    db,
                    mesh_bundle_path,
                    mesh_asset_name,
                    visited,
                    progress,
                )? {
                    return Ok(borrowed);
                }
            }
            let _ = progress.send(ProgressPayload {
                step: "lookup".into(),
                message: format!(
                    "Found {} Materials, {} Texture2Ds",
                    materials.len(),
                    texture_refs.len()
                ),
            });
            return Ok((materials, texture_refs));
        }

        let identity_material_refs = Self::find_mesh_identity_material_refs(
            db,
            mesh_bundle_path,
            mesh_path_id,
            mesh_asset_name,
            progress,
        );
        if !identity_material_refs.is_empty() {
            let _ = progress.send(ProgressPayload {
                step: "lookup".into(),
                message: format!(
                    "Resolved {} identity material reference(s) from mesh name/path stem",
                    identity_material_refs.len()
                ),
            });
            let (identity_materials, identity_texture_refs) = Self::resolve_material_refs(
                db,
                &identity_material_refs,
                mesh_asset_name,
                progress,
            )?;
            if !identity_materials.is_empty() {
                if identity_texture_refs.is_empty() {
                    if let Some(borrowed) = Self::try_borrow_same_name_mesh_textures(
                        db,
                        mesh_bundle_path,
                        mesh_asset_name,
                        visited,
                        progress,
                    )? {
                        return Ok(borrowed);
                    }
                }
                let _ = progress.send(ProgressPayload {
                    step: "lookup".into(),
                    message: format!(
                        "Found {} Materials, {} Texture2Ds",
                        identity_materials.len(),
                        identity_texture_refs.len()
                    ),
                });
                return Ok((identity_materials, identity_texture_refs));
            }
        }

        let _ = progress.send(ProgressPayload {
            step: "lookup".into(),
            message: format!(
                "Found {} Materials, {} Texture2Ds",
                materials.len(),
                texture_refs.len()
            ),
        });

        let fallback_refs = Self::find_container_texture_candidates_for_material(
            db,
            mesh_bundle_path,
            mesh_asset_name,
            progress,
        );
        if fallback_refs.is_empty() {
            if let Some(borrowed) = Self::try_borrow_same_name_mesh_textures(
                db,
                mesh_bundle_path,
                mesh_asset_name,
                visited,
                progress,
            )? {
                return Ok(borrowed);
            }
            return Ok((materials, texture_refs));
        }

        let mut fallback_material = MaterialInfo {
            name: format!("{}_container_textures", mesh_asset_name),
            ..Default::default()
        };
        Self::append_fallback_texture_slots(&mut fallback_material, &fallback_refs);
        let _ = progress.send(ProgressPayload {
            step: "lookup".into(),
            message: format!(
                "Found {} Texture2Ds from Mesh Bundle container fallback",
                fallback_refs.len()
            ),
        });

        Ok((vec![fallback_material], fallback_refs))
    }

    /// Fallback for duplicated mesh assets: when the mesh's own bundle carries
    /// no texture references at all, borrow preview textures from other bundles
    /// that contain a Mesh asset with the exact same, sufficiently specific name
    /// (Unity duplicates shared assets across bundles, and the copies usually
    /// keep the real material chain).
    ///
    /// The mesh name must be specific enough to avoid cross-borrowing generic
    /// rig parts (Body, Hand_*, Chef_*, NPC_*, ...) whose textures differ per
    /// character bundle. Only runs after every other resolution path failed, so
    /// it can never shadow an exact match.
    fn try_borrow_same_name_mesh_textures(
        db: &AssetDatabase,
        mesh_bundle_path: &str,
        mesh_asset_name: &str,
        visited: &mut HashSet<(String, i64)>,
        progress: &Channel<ProgressPayload>,
    ) -> Result<Option<(Vec<MaterialInfo>, Vec<(i64, String, String)>)>, String> {
        if !Self::is_specific_borrow_mesh_name(mesh_asset_name) {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] same-name mesh texture borrow skipped for generic mesh name '{}'",
                    mesh_asset_name
                ),
            });
            return Ok(None);
        }

        let copies = db.find_mesh_copies_by_name(mesh_asset_name, mesh_bundle_path, 3)?;
        if copies.is_empty() {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] same-name mesh texture borrow found no Mesh copies of '{}' in other bundles",
                    mesh_asset_name
                ),
            });
            return Ok(None);
        }

        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG] same-name mesh texture borrow probing {} Mesh copy/copies of '{}' in other bundles",
                copies.len(),
                mesh_asset_name
            ),
        });

        for copy in copies {
            let copy_key = (copy.asset.bundle_path.clone(), copy.asset.path_id);
            if visited.contains(&copy_key) {
                continue;
            }
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] same-name mesh texture borrow probing copy path_id={} in {}",
                    copy.asset.path_id, copy.asset.bundle_path
                ),
            });
            match Self::resolve_dependencies_inner(
                db,
                &copy.asset.bundle_path,
                copy.asset.path_id,
                &copy.asset.asset_name,
                visited,
                progress,
            ) {
                Ok((materials, texture_refs)) if !texture_refs.is_empty() => {
                    let _ = progress.send(ProgressPayload {
                        step: "lookup".into(),
                        message: format!(
                            "Borrowed {} Texture2D candidate(s) from same-name Mesh copy path_id={} in {}",
                            texture_refs.len(),
                            copy.asset.path_id,
                            copy.asset.bundle_path
                        ),
                    });
                    return Ok(Some((materials, texture_refs)));
                }
                Ok(_) => {}
                Err(error) => {
                    let _ = progress.send(ProgressPayload {
                        step: "diag".into(),
                        message: format!(
                            "[DIAG] same-name mesh texture borrow failed for copy path_id={} in {}: {}",
                            copy.asset.path_id, copy.asset.bundle_path, error
                        ),
                    });
                }
            }
        }

        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG] same-name mesh texture borrow found no resolvable copies of '{}'",
                mesh_asset_name
            ),
        });
        Ok(None)
    }

    /// Whether a mesh name is specific enough for the same-name texture borrow
    /// fallback. Generic rig/body part names are reused across many character
    /// bundles with different materials, so borrowing their textures would show
    /// the wrong character.
    fn is_specific_borrow_mesh_name(mesh_name: &str) -> bool {
        let normalized = mesh_name
            .to_lowercase()
            .replace(['-', ' ', '.'], "_")
            .trim_matches('_')
            .to_string();
        if normalized.len() < 8 {
            return false;
        }
        let structured = normalized.contains('_')
            || normalized.chars().any(|ch| ch.is_ascii_digit());
        if !structured {
            return false;
        }
        const GENERIC_PREFIXES: &[&str] = &[
            "body",
            "head",
            "hair",
            "hand_",
            "face",
            "skin",
            "cloth",
            "weapon",
            "hat_",
            "cap_",
            "chef_",
            "npc_",
            "quad",
            "combined_mesh",
            "tail",
            "arm_",
            "leg_",
            "foot_",
            "boot_",
            "glove_",
            "eye_",
            "ear_",
            "nose_",
            "mouth_",
            "tooth_",
            "cube",
            "sphere",
            "capsule",
            "plane",
            "cylinder",
            "torch_",
            "mesh",
        ];
        !GENERIC_PREFIXES
            .iter()
            .any(|prefix| normalized.starts_with(prefix))
    }

    pub fn resolve_material_refs(
        db: &AssetDatabase,
        material_refs: &[(i64, String)],
        mesh_asset_name: &str,
        progress: &Channel<ProgressPayload>,
    ) -> Result<(Vec<MaterialInfo>, Vec<(i64, String, String)>), String> {
        let mut materials: Vec<MaterialInfo> = Vec::new();
        let mut texture_refs: Vec<(i64, String, String)> = Vec::new();
        let mut bundle_cache: HashMap<
            String,
            crate::common::bundle_file::asset_bundle::AssetBundle,
        > = HashMap::new();
        let mut live_resolver = LiveDependencyResolver::new();

        for (material_path_id, material_bundle_path) in material_refs {
            let asset = db
                .find_by_bundle_and_path_id(material_bundle_path, *material_path_id)
                .ok()
                .flatten();
            let material_name = asset
                .as_ref()
                .map(|row| row.asset_name.clone())
                .unwrap_or_else(|| format!("material_{}", material_path_id));
            let _ = progress.send(ProgressPayload {
                step: "lookup".into(),
                message: format!(
                    "Material resolved: name='{}', path_id={}, bundle={}",
                    material_name, material_path_id, material_bundle_path
                ),
            });
            let parse_result = Self::load_bundle_cached(&mut bundle_cache, material_bundle_path)
                .and_then(|bundle| {
                    Self::parse_material_from_map(
                        bundle,
                        *material_path_id,
                        &material_name,
                        material_bundle_path,
                        progress,
                    )
                });
            let (mut mat, fid_map) = parse_result.unwrap_or_else(|| {
                progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "Material '{}' could not be parsed from its Bundle (missing object or unsupported layout); using defaults",
                        material_name
                    ),
                }).ok();
                (MaterialInfo {
                    name: material_name.clone(),
                    ..Default::default()
                }, HashMap::new())
            });

            let resolved_texture_count_before = texture_refs.len();
            for tex_slot in &mat.textures {
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG] Material '{}' texture slot: slot='{}', usage={}, ref={}",
                        mat.name, tex_slot.slot_name, tex_slot.usage, tex_slot.relative_path
                    ),
                });
                if let Some(texture_ref) = Self::decode_texture_ref(&tex_slot.relative_path) {
                    let pid = texture_ref.path_id;
                    let source_bundle_owned = texture_ref.source_bundle.map(|s| s.to_string());
                    let target_bundle =
                        match texture_ref.file_id.or_else(|| fid_map.get(&pid).copied()) {
                            Some(file_id) if file_id != 0 => Self::resolve_external_bundle_path(
                                db,
                                material_bundle_path,
                                file_id,
                                progress,
                                None,
                            ),
                            _ => source_bundle_owned,
                        };
                    let tex_asset =
                        Self::find_texture_asset(db, pid, target_bundle.as_deref(), progress);
                    if let Some(ta) = tex_asset {
                        texture_refs.push((pid, ta.bundle_path.clone(), ta.asset_name.clone()));
                    }
                }
            }

            let resolved_texture_count_for_material = texture_refs
                .len()
                .saturating_sub(resolved_texture_count_before);
            if resolved_texture_count_for_material == 0
                && Self::should_try_texture_name_fallback_after_resolution(&mat)
            {
                let fallback_refs = Self::find_texture_candidates_by_material_name(
                    db,
                    &mat.name,
                    mesh_asset_name,
                    progress,
                );
                Self::append_fallback_texture_slots(&mut mat, &fallback_refs);
                for (pid, bundle_path, texture_name) in fallback_refs {
                    texture_refs.push((pid, bundle_path, texture_name));
                }
            } else if resolved_texture_count_for_material == 0
                && !Self::should_try_texture_name_fallback_after_resolution(&mat)
            {
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG] texture name fallback skipped for material='{}': Material already has explicit texture PPtrs",
                        mat.name
                    ),
                });
            }

            if texture_refs.len() == resolved_texture_count_before {
                if let Some((live_material, live_textures)) = live_resolver
                    .resolve_material_textures(
                        db,
                        material_bundle_path,
                        *material_path_id,
                        progress,
                    )
                {
                    if mat.textures.is_empty() && !live_material.textures.is_empty() {
                        mat = live_material;
                    }
                    for (pid, bundle_path, texture_name) in live_textures {
                        texture_refs.push((pid, bundle_path, texture_name));
                    }
                }
            }
            if texture_refs.len() == resolved_texture_count_before {
                let container_refs = Self::find_container_texture_candidates_for_material(
                    db,
                    material_bundle_path,
                    mesh_asset_name,
                    progress,
                );
                Self::append_fallback_texture_slots(&mut mat, &container_refs);
                for (pid, bundle_path, texture_name) in container_refs {
                    texture_refs.push((pid, bundle_path, texture_name));
                }
            }
            materials.push(mat);
        }

        Ok((materials, texture_refs))
    }

    fn find_mesh_identity_material_refs(
        db: &AssetDatabase,
        mesh_bundle_path: &str,
        mesh_path_id: i64,
        mesh_asset_name: &str,
        progress: &Channel<ProgressPayload>,
    ) -> Vec<(i64, String)> {
        let mut identities = Vec::new();
        identities.extend(Self::mesh_identity_candidates(mesh_asset_name));
        if let Some(mesh_container_path) =
            Self::find_mesh_container_asset_path(db, mesh_bundle_path, mesh_path_id)
        {
            identities.extend(Self::mesh_identity_candidates(&mesh_container_path));
        }
        identities.sort();
        identities.dedup();
        if identities.is_empty() {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] mesh identity material lookup skipped for mesh_path_id={} because no identity tokens were derived",
                    mesh_path_id
                ),
            });
            return Vec::new();
        }

        let candidates = match db.search_material_candidates_by_identity(&identities, 64) {
            Ok(rows) => rows,
            Err(error) => {
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG] mesh identity material query failed for {:?}: {}",
                        identities, error
                    ),
                });
                return Vec::new();
            }
        };

        let mut scored = candidates
            .into_iter()
            .filter_map(|row| {
                let asset = row.asset;
                let score = Self::mesh_identity_material_score(
                    &asset.asset_name,
                    &row.asset_path,
                    &identities,
                );
                (score > 0).then_some((score, asset.path_id, asset.bundle_path, asset.asset_name))
            })
            .collect::<Vec<_>>();

        scored.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.3.cmp(&right.3))
                .then_with(|| left.1.cmp(&right.1))
        });

        let refs = scored
            .into_iter()
            .take(16)
            .map(|(_, path_id, bundle_path, _)| (path_id, bundle_path))
            .collect::<Vec<_>>();

        if refs.is_empty() {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] mesh identity material lookup found no exact material match for mesh_path_id={} with identities {:?}",
                    mesh_path_id, identities
                ),
            });
        } else {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] mesh identity material lookup matched {} Material candidate(s) for mesh_path_id={}",
                    refs.len(),
                    mesh_path_id
                ),
            });
            for (path_id, bundle_path) in refs.iter().take(8) {
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG]   identity material candidate: path_id={}, bundle={}",
                        path_id, bundle_path
                    ),
                });
            }
        }

        refs
    }

    fn find_mesh_container_asset_path(
        db: &AssetDatabase,
        bundle_path: &str,
        mesh_path_id: i64,
    ) -> Option<String> {
        db.get_containers(bundle_path)
            .ok()?
            .into_iter()
            .find(|container| container.path_id == mesh_path_id)
            .map(|container| container.asset_path)
    }

    fn mesh_identity_candidates(value: &str) -> Vec<String> {
        let stem = Self::mesh_identity_stem(value);
        if stem.is_empty() {
            return Vec::new();
        }

        let mut identities = vec![stem.clone()];
        let mut cursor = stem.as_str();
        for suffix in [
            "_mesh",
            "_model",
            "_geo",
            "_geometry",
            "_lod0",
            "_lod1",
            "_lod2",
            "_lod3",
            "_01",
            "_02",
        ] {
            if let Some(stripped) = cursor.strip_suffix(suffix) {
                cursor = stripped;
                if cursor.len() >= 4 {
                    identities.push(cursor.to_string());
                }
            }
        }

        identities.sort();
        identities.dedup();
        identities
    }

    fn mesh_identity_stem(value: &str) -> String {
        let normalized = value.replace('\\', "/");
        let file_name = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
        let file_name = file_name.trim();
        if file_name.is_empty() {
            return String::new();
        }
        let file_stem = file_name
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or(file_name);
        file_stem
            .to_lowercase()
            .replace(['\\', '/', '-', ' ', '.'], "_")
            .trim_matches('_')
            .to_string()
    }

    fn mesh_identity_material_score(
        material_name: &str,
        asset_path: &str,
        identities: &[String],
    ) -> i32 {
        let material_name = Self::mesh_identity_stem(material_name);
        let material_path_stem = Self::mesh_identity_stem(asset_path);
        let mut best = 0;
        for identity in identities {
            if material_name == *identity {
                best = best.max(1000 + identity.len() as i32);
            }
            if material_path_stem == *identity {
                best = best.max(950 + identity.len() as i32);
            }
        }
        best
    }

    /// Start from a Material, parse it with the AssetStudio layout parser and look up referenced Texture2D.
    ///
    /// Unlike `resolve_dependencies`, this method starts directly from the Material,
    /// skips the Container matching strategy, and reads the Material class layout's
    /// `m_SavedProperties.m_TexEnvs` to get texture PPtr references,
    /// then locates Texture2D assets via the AssetMap DB.
    ///
    /// Returns `(MaterialInfo, Vec<(path_id, bundle_path, asset_name)>)`.
    pub fn resolve_material_textures(
        db: &AssetDatabase,
        material_bundle_path: &str,
        material_path_id: i64,
        progress: &Channel<ProgressPayload>,
    ) -> Result<(MaterialInfo, Vec<(i64, String, String)>), String> {
        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG] resolve_material_textures entry: bundle={}, path_id={}",
                material_bundle_path, material_path_id
            ),
        });

        let mut bundle_cache: HashMap<
            String,
            crate::common::bundle_file::asset_bundle::AssetBundle,
        > = HashMap::new();
        let mut texture_refs: Vec<(i64, String, String)> = Vec::new();

        // Get the Material's asset name from the DB
        let db_lookup = db.find_by_bundle_and_path_id(material_bundle_path, material_path_id);
        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG] DB lookup Material (bundle={}, path_id={}): {}",
                material_bundle_path,
                material_path_id,
                match &db_lookup {
                    Ok(Some(r)) => format!("found, class={}, name={}", r.class_name, r.asset_name),
                    Ok(None) => "not found".into(),
                    Err(e) => format!("query error: {}", e),
                }
            ),
        });

        let material_asset_name = db_lookup
            .ok()
            .flatten()
            .map(|r| r.asset_name)
            .unwrap_or_else(|| format!("material_{}", material_path_id));

        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!("[DIAG] Material asset name: {}", material_asset_name),
        });

        // Load the Bundle containing the Material and parse with the AssetStudio layout parser.
        let bundle_loaded = Self::load_bundle_cached(&mut bundle_cache, material_bundle_path);
        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG] Load Bundle '{}': {}",
                material_bundle_path,
                if bundle_loaded.is_some() {
                    "success"
                } else {
                    "failed"
                }
            ),
        });

        let parse_result = bundle_loaded.and_then(|bundle| {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] Bundle has {} SerializedFiles, looking for Material path_id={}...",
                    bundle.assets.len(),
                    material_path_id
                ),
            });
            Self::parse_material_from_map(
                bundle,
                material_path_id,
                &material_asset_name,
                material_bundle_path,
                progress,
            )
        });

        let tex_count_in_mat = parse_result
            .as_ref()
            .map(|(m, _)| m.textures.len())
            .unwrap_or(0);

        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG] parse_material_from_map result: {} ({} texture refs)",
                if parse_result.is_some() {
                    "success"
                } else {
                    "failed"
                },
                tex_count_in_mat
            ),
        });

        let (mut fbx_mat, fid_map) = parse_result.unwrap_or_else(|| {
            progress
                .send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG] Material '{}' could not be parsed from its Bundle (missing object or unsupported layout); using defaults",
                        material_asset_name
                    ),
                })
                .ok();
            (MaterialInfo {
                name: material_asset_name.clone(),
                ..Default::default()
            }, HashMap::new())
        });

        // Look up Texture2D from Material's texture references
        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG] Looking up Texture2D assets for {} texture references...",
                fbx_mat.textures.len()
            ),
        });

        for tex_slot in &fbx_mat.textures {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG]   texture ref: slot='{}', usage={}, file_name={}, relative_path={}",
                    tex_slot.slot_name, tex_slot.usage, tex_slot.file_name, tex_slot.relative_path
                ),
            });

            match Self::decode_texture_ref(&tex_slot.relative_path) {
                Some(texture_ref) => {
                    let pid = texture_ref.path_id;
                    let _ = progress.send(ProgressPayload {
                        step: "diag".into(),
                        message: format!(
                            "[DIAG]     decoded: file_id={:?}, path_id={}, source_bundle={:?}",
                            texture_ref.file_id, pid, texture_ref.source_bundle
                        ),
                    });
                    // Check for cross-bundle reference (file_id != 0)
                    let source_bundle_owned = texture_ref.source_bundle.map(|s| s.to_string());
                    let target_bundle = match texture_ref
                        .file_id
                        .or_else(|| fid_map.get(&pid).copied())
                    {
                        Some(file_id) if file_id != 0 => {
                            let resolved = Self::resolve_external_bundle_path(
                                db,
                                material_bundle_path,
                                file_id,
                                progress,
                                None,
                            );
                            match resolved {
                                Some(bp) => {
                                    let _ = progress.send(ProgressPayload {
                                        step: "diag".into(),
                                        message: format!(
                                            "[DIAG]     Cross-bundle ref: file_id={}, target Bundle={}",
                                            file_id, bp
                                        ),
                                    });
                                    Some(bp)
                                }
                                None => {
                                    let _ = progress.send(ProgressPayload {
                                        step: "diag".into(),
                                        message: format!(
                                            "[DIAG]     Cross-bundle ref: file_id={} unresolved in AssetMap; skipping texture lookup",
                                            file_id
                                        ),
                                    });
                                    None
                                }
                            }
                        }
                        _ => source_bundle_owned, // file_id=0 or not in fid_map
                    };

                    let tex_asset =
                        Self::find_texture_asset(db, pid, target_bundle.as_deref(), progress);
                    match &tex_asset {
                        Some(ta) => {
                            let _ = progress.send(ProgressPayload {
                                step: "diag".into(),
                                message: format!(
                                    "[DIAG]     Found texture: path_id={}, name={}, bundle={}, class={}",
                                    pid, ta.asset_name, ta.bundle_path, ta.class_name
                                ),
                            });
                            texture_refs.push((pid, ta.bundle_path.clone(), ta.asset_name.clone()));
                        }
                        None => {
                            let _ = progress.send(ProgressPayload {
                                step: "diag".into(),
                                message: format!(
                                    "[DIAG]     Texture not found: path_id={} (target_bundle={:?})",
                                    pid, target_bundle
                                ),
                            });
                        }
                    }
                }
                None => {
                    let _ = progress.send(ProgressPayload {
                        step: "diag".into(),
                        message: format!(
                            "[DIAG]     Cannot decode relative_path: {}",
                            tex_slot.relative_path
                        ),
                    });
                }
            }
        }

        if tex_count_in_mat > 0 && texture_refs.is_empty() {
            progress
                .send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG] Material '{}' has {} texture path_ids not indexed in DB",
                        material_asset_name, tex_count_in_mat
                    ),
                })
                .ok();
        }

        // If no textures were parsed from Material layout, try searching containers in the same bundle.
        if texture_refs.is_empty() {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: "[DIAG] No texture refs parsed from Material, searching Bundle containers for Texture2D/Sprite...".into(),
            });
            let container_refs = Self::find_container_texture_candidates_for_material(
                db,
                material_bundle_path,
                &material_asset_name,
                progress,
            );
            Self::append_fallback_texture_slots(&mut fbx_mat, &container_refs);
            texture_refs.extend(container_refs);
            if !texture_refs.is_empty() {
                progress
                    .send(ProgressPayload {
                        step: "diag".into(),
                        message: format!(
                            "[DIAG] Found {} Texture2Ds in containers",
                            texture_refs.len()
                        ),
                    })
                    .ok();
            }
        }

        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG] resolve_material_textures done: Material='{}', found {} Texture2Ds",
                material_asset_name,
                texture_refs.len()
            ),
        });

        Ok((fbx_mat, texture_refs))
    }
    /// Look up a texture asset inside a concrete Bundle resolved from AssetMap relations.
    ///
    /// Unity path_id values are only unique within a SerializedFile, so callers must resolve
    /// file_id/path_name to a Bundle path in the index before asking for the Texture2D row.
    fn find_texture_asset(
        db: &AssetDatabase,
        path_id: i64,
        target_bundle: Option<&str>,
        progress: &Channel<ProgressPayload>,
    ) -> Option<AssetRow> {
        let Some(bundle_path) = target_bundle else {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG]       find_texture_asset: path_id={} has no resolved target Bundle; trying global path_id fallback",
                    path_id
                ),
            });
            return Self::find_texture_asset_global_fallback(db, path_id, progress);
        };

        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG]       find_texture_asset: searching bundle={}, path_id={}",
                bundle_path, path_id
            ),
        });

        match db.find_by_bundle_and_path_id(bundle_path, path_id) {
            Ok(Some(row)) => {
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG]         find_by_bundle_and_path_id found: class={}, name={}",
                        row.class_name, row.asset_name
                    ),
                });
                if row.class_name == "Texture2D" || row.class_name == "Sprite" {
                    return Some(row);
                }
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG]         resolved row class={} is not Texture2D/Sprite",
                        row.class_name
                    ),
                });
            }
            Ok(None) => {
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: "[DIAG]         path_id not found in resolved target Bundle".into(),
                });
            }
            Err(e) => {
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!("[DIAG]         target Bundle query error: {}", e),
                });
            }
        }

        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG]       find_texture_asset: path_id={} was not resolved from AssetMap; not scanning Bundles during query",
                path_id
            ),
        });
        Self::find_texture_asset_global_fallback(db, path_id, progress)
    }

    fn find_texture_asset_global_fallback(
        db: &AssetDatabase,
        path_id: i64,
        progress: &Channel<ProgressPayload>,
    ) -> Option<AssetRow> {
        let mut candidates = db
            .find_by_path_id(path_id)
            .ok()?
            .into_iter()
            .filter(|row| row.class_name == "Texture2D" || row.class_name == "Sprite")
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG]       find_texture_asset: global path_id fallback found no Texture2D/Sprite rows for path_id={}",
                    path_id
                ),
            });
            return None;
        }

        candidates.sort_by(|left, right| {
            left.class_name
                .cmp(&right.class_name)
                .then_with(|| left.bundle_path.cmp(&right.bundle_path))
                .then_with(|| left.asset_name.cmp(&right.asset_name))
                .then_with(|| left.byte_size.cmp(&right.byte_size))
        });
        let chosen = candidates.remove(0);
        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG]       find_texture_asset: global path_id fallback chose {} '{}' from {} candidate(s)",
                chosen.class_name,
                chosen.bundle_path,
                candidates.len() + 1
            ),
        });
        Some(chosen)
    }

    /// Fallback for games where Material PPtrs point to CAB names absent from the extracted set.
    ///
    /// This still uses AssetMap only: it searches Texture2D/Sprite names and container paths
    /// using conservative tokens derived from the Material/Mesh name.
    fn find_texture_candidates_by_material_name(
        db: &AssetDatabase,
        material_name: &str,
        mesh_asset_name: &str,
        progress: &Channel<ProgressPayload>,
    ) -> Vec<(i64, String, String)> {
        let token_sets = Self::texture_fallback_token_sets(material_name, mesh_asset_name);
        if token_sets.is_empty() {
            return Vec::new();
        }

        let mut candidates = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for terms in token_sets {
            let rows = match db.search_texture_candidates_by_text(&terms, 64) {
                Ok(rows) => rows,
                Err(e) => {
                    let _ = progress.send(ProgressPayload {
                        step: "diag".into(),
                        message: format!(
                            "[DIAG] texture name fallback query failed for {:?}: {}",
                            terms, e
                        ),
                    });
                    continue;
                }
            };

            let mut scored = rows
                .into_iter()
                .filter_map(|row| {
                    let asset = row.asset;
                    if !seen.insert((asset.bundle_path.clone(), asset.path_id)) {
                        return None;
                    }
                    let score = Self::texture_name_fallback_score(
                        &asset.asset_name,
                        &row.asset_path,
                        material_name,
                        mesh_asset_name,
                    );
                    (score > 0).then_some((
                        score,
                        asset.path_id,
                        asset.bundle_path,
                        asset.asset_name,
                    ))
                })
                .collect::<Vec<_>>();
            scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.3.cmp(&right.3)));
            candidates.extend(
                scored
                    .into_iter()
                    .take(24)
                    .map(|(_, path_id, bundle_path, asset_name)| {
                        (path_id, bundle_path, asset_name)
                    }),
            );
            if !candidates.is_empty() {
                Self::expand_texture_fallback_family(db, &mut candidates, &mut seen, progress);
                break;
            }
        }

        if !candidates.is_empty() {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] texture name fallback matched {} Texture2D candidate(s) for material='{}', mesh='{}'",
                    candidates.len(),
                    material_name,
                    mesh_asset_name
                ),
            });
            for (path_id, bundle_path, name) in candidates.iter().take(16) {
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG]   fallback texture: name='{}', path_id={}, bundle={}",
                        name, path_id, bundle_path
                    ),
                });
            }
        }

        candidates
    }

    fn expand_texture_fallback_family(
        db: &AssetDatabase,
        candidates: &mut Vec<(i64, String, String)>,
        seen: &mut std::collections::HashSet<(String, i64)>,
        progress: &Channel<ProgressPayload>,
    ) {
        let family_terms = candidates
            .iter()
            .filter_map(|(_, _, texture_name)| Self::texture_family_stem(texture_name))
            .filter(|stem| Self::is_specific_texture_fallback_stem(stem))
            .collect::<Vec<_>>();
        if family_terms.is_empty() {
            return;
        }

        let mut added = 0usize;
        for family_term in family_terms {
            let rows = match db.search_texture_candidates_by_text(&[family_term.clone()], 64) {
                Ok(rows) => rows,
                Err(e) => {
                    let _ = progress.send(ProgressPayload {
                        step: "diag".into(),
                        message: format!(
                            "[DIAG] texture family fallback query failed for '{}': {}",
                            family_term, e
                        ),
                    });
                    continue;
                }
            };

            let mut family_rows = rows
                .into_iter()
                .filter_map(|row| {
                    let asset = row.asset;
                    if !seen.insert((asset.bundle_path.clone(), asset.path_id)) {
                        return None;
                    }
                    let score =
                        Self::texture_family_fallback_score(&asset.asset_name, &row.asset_path);
                    (score > 0).then_some((
                        score,
                        asset.path_id,
                        asset.bundle_path,
                        asset.asset_name,
                    ))
                })
                .collect::<Vec<_>>();
            family_rows
                .sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.3.cmp(&right.3)));
            for (_, path_id, bundle_path, asset_name) in family_rows.into_iter().take(48) {
                candidates.push((path_id, bundle_path, asset_name));
                added += 1;
            }
        }

        if added > 0 {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] texture family fallback added {} related Texture2D candidate(s)",
                    added
                ),
            });
        }
    }

    fn texture_fallback_token_sets(material_name: &str, mesh_asset_name: &str) -> Vec<Vec<String>> {
        let mut sets = Vec::new();
        for name in [material_name, mesh_asset_name] {
            for stem in Self::texture_fallback_name_stems(name) {
                if Self::is_specific_texture_fallback_stem(&stem) {
                    sets.push(vec![stem]);
                }
            }
        }
        sets.sort();
        sets.dedup();
        sets
    }

    fn should_try_texture_name_fallback_after_resolution(material: &MaterialInfo) -> bool {
        material.textures.is_empty()
            || material
                .textures
                .iter()
                .any(|slot| slot.relative_path.starts_with('#'))
    }

    fn find_container_texture_candidates_for_material(
        db: &AssetDatabase,
        bundle_path: &str,
        context_name: &str,
        progress: &Channel<ProgressPayload>,
    ) -> Vec<(i64, String, String)> {
        let context_prefixes = Self::texture_fallback_prefixes(context_name);
        if context_prefixes.is_empty() {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] Bundle container texture fallback skipped for generic context='{}'",
                    context_name
                ),
            });
            return Vec::new();
        }

        let containers = match db.get_containers(bundle_path) {
            Ok(rows) => rows,
            Err(error) => {
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG] Bundle container texture fallback query failed for {}: {}",
                        bundle_path, error
                    ),
                });
                Vec::new()
            }
        };
        if containers.is_empty() {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] Bundle container texture fallback found no container entries for {}",
                    bundle_path
                ),
            });
        }

        let mut seen = HashSet::new();
        let mut candidates = Vec::new();
        for container in containers {
            if !Self::container_path_looks_like_texture(&container.asset_path) {
                continue;
            }
            let asset = match db.find_by_bundle_and_path_id(bundle_path, container.path_id) {
                Ok(Some(row)) if row.class_name == "Texture2D" || row.class_name == "Sprite" => row,
                Ok(_) => continue,
                Err(error) => {
                    let _ = progress.send(ProgressPayload {
                        step: "diag".into(),
                        message: format!(
                            "[DIAG] Bundle container texture fallback asset lookup failed: {}",
                            error
                        ),
                    });
                    continue;
                }
            };
            if !seen.insert((asset.bundle_path.clone(), asset.path_id)) {
                continue;
            }
            let score = Self::container_texture_fallback_score(
                &asset.asset_name,
                &container.asset_path,
                &context_prefixes,
            );
            if score <= 0 {
                continue;
            }
            candidates.push((score, asset.path_id, asset.bundle_path, asset.asset_name));
        }

        Self::append_assetmap_prefix_texture_candidates(
            db,
            &context_prefixes,
            &mut candidates,
            &mut seen,
            progress,
        );

        candidates.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.3.cmp(&right.3))
                .then_with(|| left.1.cmp(&right.1))
        });

        let refs = candidates
            .into_iter()
            .take(32)
            .map(|(_, path_id, bundle_path, asset_name)| (path_id, bundle_path, asset_name))
            .collect::<Vec<_>>();

        if !refs.is_empty() {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] Mesh-name texture fallback matched {} Texture2D/Sprite candidate(s) for context='{}'",
                    refs.len(),
                    context_name
                ),
            });
            for (path_id, bundle_path, texture_name) in refs.iter().take(16) {
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG]   mesh-name fallback texture: name='{}', path_id={}, bundle={}",
                        texture_name, path_id, bundle_path
                    ),
                });
            }
        } else {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] Mesh-name texture fallback found no prefix-matched Texture2D/Sprite candidates for context='{}'",
                    context_name
                ),
            });
        }

        refs
    }

    fn container_path_looks_like_texture(asset_path: &str) -> bool {
        let lower = asset_path.to_lowercase();
        lower.contains("/texture/")
            || lower.contains("/textures/")
            || lower.contains("\\texture\\")
            || lower.contains("\\textures\\")
            || lower.contains("tex_")
            || lower.contains("_tex")
            || Self::path_has_texture_extension(&lower)
    }

    fn path_has_texture_extension(path: &str) -> bool {
        [
            ".png", ".jpg", ".jpeg", ".tga", ".dds", ".ktx", ".pvr", ".exr", ".hdr", ".psd",
            ".tif", ".tiff", ".bmp",
        ]
        .iter()
        .any(|ext| path.ends_with(ext))
    }

    fn append_assetmap_prefix_texture_candidates(
        db: &AssetDatabase,
        context_prefixes: &[String],
        candidates: &mut Vec<(i32, i64, String, String)>,
        seen: &mut HashSet<(String, i64)>,
        progress: &Channel<ProgressPayload>,
    ) {
        let rows = match db.search_texture_candidates_by_prefixes(context_prefixes, 256) {
            Ok(rows) => rows,
            Err(error) => {
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG] AssetMap prefix texture fallback query failed for {:?}: {}",
                        context_prefixes, error
                    ),
                });
                return;
            }
        };
        for row in rows {
            let asset = row.asset;
            if !seen.insert((asset.bundle_path.clone(), asset.path_id)) {
                continue;
            }
            let score = Self::container_texture_fallback_score(
                &asset.asset_name,
                &row.asset_path,
                context_prefixes,
            );
            if score <= 0 {
                continue;
            }
            candidates.push((
                score + 40,
                asset.path_id,
                asset.bundle_path,
                asset.asset_name,
            ));
        }
    }

    fn container_texture_fallback_score(
        texture_name: &str,
        asset_path: &str,
        context_tokens: &[String],
    ) -> i32 {
        let prefix_score =
            Self::texture_candidate_prefix_match_score(texture_name, asset_path, context_tokens);
        if prefix_score == 0 {
            return 0;
        }
        let haystack = format!("{} {}", texture_name, asset_path)
            .to_lowercase()
            .replace(['-', ' ', '.'], "_");
        let mut score = 100 + prefix_score;
        if Self::path_has_texture_extension(&asset_path.to_lowercase()) {
            score += 120;
        }
        if haystack.contains("/textures/") || haystack.contains("\\textures\\") {
            score += 80;
        }
        if haystack.contains("/texture/") || haystack.contains("\\texture\\") {
            score += 70;
        }
        for token in context_tokens {
            if Self::is_specific_texture_fallback_stem(token) && haystack.contains(token) {
                score += 240 + token.len() as i32;
            }
        }
        for token in [
            "_d",
            "_diff",
            "diffuse",
            "basecolor",
            "albedo",
            "_base",
            "_n",
            "_nx",
            "normal",
            "bump",
            "_s",
            "spec",
            "mask",
            "rough",
            "mrav",
            "_ao",
            "occlusion",
            "emiss",
            "wall",
        ] {
            if haystack.contains(token) {
                score += 30;
            }
        }
        for token in ["preview", "icon", "thumbnail", "editor"] {
            if haystack.contains(token) {
                score -= 120;
            }
        }
        score
    }

    fn texture_candidate_prefix_match_score(
        texture_name: &str,
        asset_path: &str,
        context_prefixes: &[String],
    ) -> i32 {
        let values = Self::texture_candidate_match_values(texture_name, asset_path);
        let mut best = 0;
        for prefix in context_prefixes
            .iter()
            .filter(|prefix| Self::is_specific_texture_fallback_stem(prefix))
        {
            for (value_index, value) in values.iter().enumerate() {
                if Self::normalized_prefix_match(value, prefix) {
                    let source_bonus = if value_index == 0 { 260 } else { 180 };
                    best = best.max(source_bonus + prefix.len() as i32);
                }
            }
        }
        best
    }

    #[cfg(test)]
    fn texture_candidate_matches_context_prefix(
        texture_name: &str,
        asset_path: &str,
        context_name: &str,
    ) -> bool {
        let prefixes = Self::texture_fallback_prefixes(context_name);
        Self::texture_candidate_prefix_match_score(texture_name, asset_path, &prefixes) > 0
    }

    fn texture_candidate_match_values(texture_name: &str, asset_path: &str) -> Vec<String> {
        let mut values = Vec::new();
        if !texture_name.trim().is_empty() {
            values.push(Self::normalize_texture_match_value(texture_name));
        }
        let file_name = asset_path
            .replace('\\', "/")
            .rsplit('/')
            .next()
            .unwrap_or(asset_path)
            .to_string();
        if !file_name.trim().is_empty() {
            let file_stem = file_name
                .rsplit_once('.')
                .map(|(stem, _)| stem)
                .unwrap_or(file_name.as_str());
            values.push(Self::normalize_texture_match_value(file_stem));
        }
        values.retain(|value| !value.is_empty());
        values.sort();
        values.dedup();
        values
    }

    fn normalize_texture_match_value(value: &str) -> String {
        value
            .to_lowercase()
            .replace(['\\', '/', '-', ' ', '.'], "_")
            .trim_matches('_')
            .to_string()
    }

    fn normalized_prefix_match(value: &str, prefix: &str) -> bool {
        if value == prefix {
            return true;
        }
        value
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('_'))
    }

    fn append_fallback_texture_slots(
        material: &mut MaterialInfo,
        texture_refs: &[(i64, String, String)],
    ) {
        let mut slot_counts = material.textures.iter().fold(
            std::collections::HashMap::<String, usize>::new(),
            |mut counts, slot| {
                *counts.entry(slot.slot_name.clone()).or_insert(0) += 1;
                counts
            },
        );
        for (path_id, bundle_path, texture_name) in texture_refs {
            let (slot_name, usage) = Self::infer_texture_slot_from_name(texture_name);
            let count = slot_counts.entry(slot_name.to_string()).or_insert(0);
            *count += 1;
            let effective_slot_name = if *count == 1 {
                slot_name.to_string()
            } else {
                format!("{}#{}", slot_name, *count)
            };
            material.textures.push(TextureSlot {
                slot_name: effective_slot_name,
                usage: usage.to_string(),
                file_name: format!("tex_{}.png", path_id),
                relative_path: UnityMaterialTextureExtractor::encode_texture_ref(
                    0,
                    *path_id,
                    bundle_path,
                ),
            });
        }
    }

    fn infer_texture_slot_from_name(texture_name: &str) -> (&'static str, &'static str) {
        let name = texture_name.to_lowercase().replace(['-', ' ', '.'], "_");
        if Self::texture_name_has_token(&name, &["_n", "_nx", "_normal", "_bump", "_nrm"]) {
            ("_BumpMap", "NormalMap")
        } else if Self::texture_name_has_token(&name, &["_ao", "_occlusion"]) {
            ("_OcclusionMap", "OcclusionMap")
        } else if Self::texture_name_has_token(&name, &["_em", "_emiss", "_emission"]) {
            ("_EmissionMap", "EmissiveColor")
        } else if Self::texture_name_has_token(&name, &["_rough", "_roughness"]) {
            ("_RoughnessMap", "RoughnessMap")
        } else if Self::texture_name_has_token(&name, &["_metal", "_metallic"]) {
            ("_MetallicGlossMap", "MetallicMap")
        } else if Self::texture_name_has_token(&name, &["_mask", "_m"]) {
            ("_MaskMap", "MaskMap")
        } else if Self::texture_name_has_token(&name, &["_line", "_outline"]) {
            ("_LineMap", "LineMap")
        } else if Self::texture_name_has_token(&name, &["_skin"]) {
            ("_SkinMap", "SkinMap")
        } else if Self::texture_name_has_token(&name, &["_other"]) {
            ("_OtherMap", "OtherMap")
        } else if Self::texture_name_has_token(&name, &["_s", "_spec", "_specular", "_mrav"]) {
            ("_SpecGlossMap", "SpecularColor")
        } else {
            ("_MainTex", "DiffuseColor")
        }
    }

    fn texture_name_has_token(name: &str, tokens: &[&str]) -> bool {
        tokens.iter().any(|token| {
            if token.starts_with('_') {
                name.ends_with(token) || name.contains(&format!("{}_", token))
            } else {
                name.contains(token)
            }
        })
    }

    fn is_specific_texture_fallback_stem(stem: &str) -> bool {
        let trimmed = stem.trim_matches('_');
        let underscore_count = trimmed.chars().filter(|ch| *ch == '_').count();
        if trimmed.len() < 8 && underscore_count < 2 {
            return false;
        }
        if matches!(
            trimmed,
            "body" | "head" | "hair" | "face" | "hand" | "skin" | "cloth" | "weapon"
        ) {
            return false;
        }
        underscore_count > 0 || trimmed.chars().any(|ch| ch.is_ascii_digit()) || trimmed.len() >= 12
    }

    fn texture_fallback_name_stems(name: &str) -> Vec<String> {
        let normalized = name.to_lowercase().replace(['-', ' ', '.'], "_");
        let mut out = Vec::new();
        if !normalized.is_empty() {
            out.push(normalized.clone());
        }

        let mut stem = normalized.as_str();
        for suffix in [
            "_outline_p5r",
            "_outline_ink",
            "_outline",
            "_ink",
            "_p5r",
            "_mat",
            "_material",
            "_02",
            "_01",
        ] {
            if let Some(stripped) = stem.strip_suffix(suffix) {
                stem = stripped;
                if stem.len() >= 4 {
                    out.push(stem.to_string());
                }
            }
        }
        if stem == "male_body" {
            out.push("male_body01".to_string());
        }
        if stem == "female_body" {
            out.push("female_body01".to_string());
        }
        out.sort_by_key(|value| std::cmp::Reverse(value.len()));
        out.dedup();
        out
    }

    fn texture_fallback_prefixes(name: &str) -> Vec<String> {
        let mut prefixes = Self::texture_fallback_name_stems(name)
            .into_iter()
            .filter(|stem| Self::is_specific_texture_fallback_stem(stem))
            .collect::<Vec<_>>();
        prefixes.sort_by_key(|value| std::cmp::Reverse(value.len()));
        prefixes.dedup();
        prefixes
    }

    fn texture_family_stem(texture_name: &str) -> Option<String> {
        let normalized = texture_name
            .to_lowercase()
            .replace(['-', ' ', '.'], "_")
            .trim_matches('_')
            .to_string();
        let suffixes = [
            "_d",
            "_diff",
            "_diffuse",
            "_albedo",
            "_basecolor",
            "_base",
            "_n",
            "_nx",
            "_normal",
            "_bump",
            "_s",
            "_spec",
            "_specular",
            "_m",
            "_mask",
            "_mrav",
            "_rough",
            "_roughness",
            "_ao",
            "_occlusion",
            "_em",
            "_emiss",
            "_emission",
        ];
        suffixes
            .iter()
            .find_map(|suffix| normalized.strip_suffix(suffix).map(|stem| stem.to_string()))
            .filter(|stem| !stem.is_empty())
    }

    fn texture_name_fallback_score(
        texture_name: &str,
        asset_path: &str,
        material_name: &str,
        mesh_asset_name: &str,
    ) -> i32 {
        let haystack = format!("{} {}", texture_name, asset_path).to_lowercase();
        let mut score = 10;
        for stem in Self::texture_fallback_name_stems(material_name)
            .into_iter()
            .chain(Self::texture_fallback_name_stems(mesh_asset_name))
        {
            if Self::is_specific_texture_fallback_stem(&stem) && haystack.contains(&stem) {
                score += 300 + stem.len() as i32;
            }
        }
        for token in [
            "_d",
            "_d.",
            "_diff",
            "diffuse",
            "basecolor",
            "albedo",
            "_base",
        ] {
            if haystack.contains(token) {
                score += 120;
            }
        }
        for token in ["_tx", "_main"] {
            if haystack.contains(token) {
                score += 40;
            }
        }
        for token in [
            "_n", "_nx", "normal", "bump", "mask", "spec", "rough", "mrav", "skin", "line",
            "other", "emiss", "_em",
        ] {
            if haystack.contains(token) {
                score -= 180;
            }
        }
        score
    }

    fn texture_family_fallback_score(texture_name: &str, asset_path: &str) -> i32 {
        let haystack = format!("{} {}", texture_name, asset_path).to_lowercase();
        let mut score = 10;
        for token in [
            "_d",
            "_diff",
            "diffuse",
            "basecolor",
            "albedo",
            "_base",
            "_n",
            "_nx",
            "normal",
            "bump",
            "_s",
            "spec",
            "mask",
            "rough",
            "mrav",
            "_ao",
            "occlusion",
            "emiss",
        ] {
            if haystack.contains(token) {
                score += 80;
            }
        }
        for token in ["lod", "preview", "icon", "thumbnail"] {
            if haystack.contains(token) {
                score -= 160;
            }
        }
        score
    }

    /// Resolve a PPtr file_id to the corresponding target Bundle path.
    ///
    /// In Unity, PPtr.file_id semantics:
    ///   - 0  Same SerializedFile (current Bundle)
    ///   - >0 m_externals[file_id - 1] external reference
    ///   - -1 Global resource (Resources)
    ///
    /// This method queries the externals table to get the path_name, then tries to match
    /// it to a scanned Bundle file path. Matching strategies (by priority):
    ///   1. Exact match (path_name == bundle_path)
    ///   2. Suffix match (bundle_path ends with path_name)
    ///   3. Filename match (path_name's filename == bundle_path's filename)
    fn resolve_external_bundle_path(
        db: &AssetDatabase,
        source_bundle_path: &str,
        file_id: i32,
        progress: &Channel<ProgressPayload>,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Option<String> {
        let _started = Instant::now();
        if file_id == -1 {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: "[DIAG]    resolve_external: file_id=-1 global resource, cannot locate a specific Bundle".into(),
            });
            return None; // Global resource, can't pinpoint a specific Bundle
        }
        if file_id <= 0 {
            return None;
        }

        // Query externals table for path_name
        let externals = db
            .find_externals_by_bundle(source_bundle_path)
            .ok()
            .unwrap_or_default()
            .into_iter()
            .map(|external| (external.file_id, external.path_name))
            .collect::<Vec<_>>();
        let external_count = externals.len();
        let path_name = externals
            .iter()
            .find(|(fid, _)| *fid == file_id)
            .map(|(_, name)| name.as_str());

        let path_name = match path_name {
            Some(name) => name,
            None => {
                let available_file_ids = externals
                    .iter()
                    .take(12)
                    .map(|(fid, name)| format!("{}:'{}'", fid, name))
                    .collect::<Vec<_>>()
                    .join(", ");
                let likely_bad_pptr = external_count == 0 || file_id as usize > external_count;
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG]    resolve_external: file_id={} not found in externals (bundle={}, externals={}, available=[{}])",
                        file_id, source_bundle_path, external_count, available_file_ids
                    ),
                });
                if likely_bad_pptr {
                    let _ = progress.send(ProgressPayload {
                        step: "diag".into(),
                        message: format!(
                            "[DIAG]    cause hint: file_id={} is outside the m_Externals index range for this Bundle; this usually means the Material PPtr was read from the wrong byte offset or Build Map did not index this SerializedFile's externals",
                            file_id
                        ),
                    });
                } else {
                    let _ = progress.send(ProgressPayload {
                        step: "diag".into(),
                        message: format!(
                            "[DIAG]    cause hint: file_id={} is within the expected external range but missing from AssetMap; rebuild AssetMap may be needed if the Bundle changed",
                            file_id
                        ),
                    });
                }
                return None;
            }
        };

        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG]    resolve_external: file_id={} path_name='{}'",
                file_id, path_name
            ),
        });

        if path_name.is_empty() {
            return None;
        }

        // Try to match against scanned Bundles
        let bundle_infos = db.get_bundle_infos().ok().unwrap_or_default();
        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG]    resolve_external: matching against {} indexed Bundles",
                bundle_infos.len()
            ),
        });
        let cab_name = UnityExternalResolver::archive_cab_name(path_name);

        // Strategy 4: archive:/CAB-xxx/... match against bundle internal stream/node names
        if let Some(ref cab) = cab_name {
            if let Some(token) = cancel_token {
                if token.load(Ordering::SeqCst) {
                    let _ = progress.send(ProgressPayload {
                        step: "diag".into(),
                        message: "[DIAG]    resolve_external: cancelled before CAB lookup".into(),
                    });
                    return None;
                }
            }
            match db.find_bundle_by_internal_name(cab) {
                Ok(Some(bundle_path)) => {
                    let _ = progress.send(ProgressPayload {
                        step: "diag".into(),
                        message: format!(
                            "[DIAG]    resolve_external: CAB internal-name index match {}",
                            bundle_path
                        ),
                    });
                    return Some(bundle_path);
                }
                Ok(None) => {
                    let _ = progress.send(ProgressPayload {
                        step: "diag".into(),
                        message: format!(
                            "[DIAG]    resolve_external: archive CAB '{}' was not found in internal-name index; AssetMap may be missing the Bundle that contains this CAB",
                            cab
                        ),
                    });
                }
                Err(e) => {
                    let _ = progress.send(ProgressPayload {
                        step: "diag".into(),
                        message: format!(
                            "[DIAG]    resolve_external: internal-name index query failed: {}",
                            e
                        ),
                    });
                }
            }
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG]    resolve_external: archive CAB '{}' has no indexed match; refusing full-bundle scan",
                    cab
                ),
            });
        }

        if let Some(bundle_path) = UnityExternalResolver::resolve_from_db(db, path_name) {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG]    resolve_external: shared resolver match {}",
                    bundle_path
                ),
            });
            return Some(bundle_path);
        }

        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG]    resolve_external: '{}' did not match any local Bundle",
                path_name
            ),
        });
        None
    }

    pub fn find_mesh_material_refs(
        db: &AssetDatabase,
        mesh_bundle_path: &str,
        mesh_path_id: i64,
        progress: &Channel<ProgressPayload>,
    ) -> Vec<(i64, String)> {
        let mut refs: Vec<(i64, String)> = Vec::new();
        let mut renderer_ids: Vec<i64> = Vec::new();
        let mut seen_renderer_ids: std::collections::HashSet<i64> =
            std::collections::HashSet::new();

        if let Ok(renderer_mesh_relations) = db.find_relations_by_bundle_and_target(
            UnityRelationKind::RENDERER_MESH,
            mesh_bundle_path,
            mesh_path_id,
        ) {
            for relation in renderer_mesh_relations {
                if seen_renderer_ids.insert(relation.source_path_id) {
                    renderer_ids.push(relation.source_path_id);
                }
            }
        }

        if let Ok(mesh_filter_relations) = db.find_relations_by_bundle_and_target(
            UnityRelationKind::MESH_FILTER_MESH,
            mesh_bundle_path,
            mesh_path_id,
        ) {
            for mesh_filter in mesh_filter_relations {
                let mesh_filter_go = db
                    .find_relations_by_bundle_and_source(
                        UnityRelationKind::MESH_FILTER_GAMEOBJECT,
                        mesh_bundle_path,
                        mesh_filter.source_path_id,
                    )
                    .unwrap_or_default()
                    .into_iter()
                    .next();
                let Some(game_object_id) = mesh_filter_go.map(|relation| relation.target_path_id)
                else {
                    continue;
                };
                for renderer_go in db
                    .find_relations_by_bundle_and_target(
                        UnityRelationKind::RENDERER_GAMEOBJECT,
                        mesh_bundle_path,
                        game_object_id,
                    )
                    .unwrap_or_default()
                {
                    if seen_renderer_ids.insert(renderer_go.source_path_id) {
                        renderer_ids.push(renderer_go.source_path_id);
                    }
                }
            }
        }

        for renderer_id in renderer_ids {
            for material in db
                .find_relations_by_bundle_and_source(
                    UnityRelationKind::RENDERER_MATERIAL,
                    mesh_bundle_path,
                    renderer_id,
                )
                .unwrap_or_default()
            {
                if material.target_path_id == 0 {
                    continue;
                }
                let bundle_path = if !material.target_bundle_path.is_empty() {
                    material.target_bundle_path.clone()
                } else if material.file_id == 0 {
                    material.bundle_path.clone()
                } else if let Some(resolved_bundle_path) = Self::resolve_external_bundle_path(
                    db,
                    &material.bundle_path,
                    material.file_id,
                    progress,
                    None,
                ) {
                    let _ = progress.send(ProgressPayload {
                        step: "diag".into(),
                        message: format!(
                            "[DIAG] renderer material relation source={} file_id={} path_id={} resolved via AssetMap externals to {}",
                            renderer_id, material.file_id, material.target_path_id, resolved_bundle_path
                        ),
                    });
                    resolved_bundle_path
                } else {
                    let _ = progress.send(ProgressPayload {
                        step: "diag".into(),
                        message: format!(
                            "[DIAG] renderer material relation source={} file_id={} path_id={} has no resolvable target Bundle in AssetMap",
                            renderer_id, material.file_id, material.target_path_id
                        ),
                    });
                    continue;
                };
                refs.push((material.target_path_id, bundle_path));
            }
        }

        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG] mesh_path_id={} DB renderer material refs={}",
                mesh_path_id,
                refs.len()
            ),
        });
        if refs.is_empty() {
            let live_refs =
                Self::find_mesh_material_refs_live(db, mesh_bundle_path, mesh_path_id, progress);
            if !live_refs.is_empty() {
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG] mesh_path_id={} live renderer material refs={}",
                        mesh_path_id,
                        live_refs.len()
                    ),
                });
                return live_refs;
            }
        }

        refs
    }

    fn find_mesh_material_refs_live(
        db: &AssetDatabase,
        mesh_bundle_path: &str,
        mesh_path_id: i64,
        progress: &Channel<ProgressPayload>,
    ) -> Vec<(i64, String)> {
        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG] DB relations empty; live scanning renderer/material PPtrs in {}",
                mesh_bundle_path
            ),
        });

        let bundle = match AssetBundleLoader::load_unity_file_serialized_only(std::path::Path::new(
            mesh_bundle_path,
        )) {
            Ok(bundle) => bundle,
            Err(error) => {
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG] live renderer/material scan failed to load Bundle: {}",
                        error
                    ),
                });
                return Vec::new();
            }
        };

        let mut game_object_by_mesh_filter = HashMap::<i64, i64>::new();
        let mut game_objects_with_mesh = HashSet::<i64>::new();
        let mut direct_renderers = Vec::<(usize, i64, Vec<PPtr>)>::new();
        let mut renderer_game_objects = Vec::<(usize, i64, i64, Vec<PPtr>)>::new();

        for (sf_index, sf) in bundle.assets.iter().enumerate() {
            let externals = sf.inner.m_externals.as_slice();
            for obj in &sf.objects {
                match obj.class_id {
                    33 => {
                        if let Ok(mesh_filter) =
                            UnityClassParser::parse_mesh_filter(&sf.inner, &obj.inner)
                        {
                            if let Some(mesh) = mesh_filter.mesh {
                                if Self::live_pptr_targets_asset(
                                    db,
                                    mesh_bundle_path,
                                    externals,
                                    &mesh,
                                    mesh_bundle_path,
                                    mesh_path_id,
                                ) {
                                    let game_object = mesh_filter.component.game_object;
                                    if game_object.file_id == 0 && game_object.path_id != 0 {
                                        game_objects_with_mesh.insert(game_object.path_id);
                                        game_object_by_mesh_filter
                                            .insert(obj.path_id, game_object.path_id);
                                    }
                                }
                            }
                        }
                    }
                    23 | 119 => {
                        if let Ok(renderer) =
                            UnityClassParser::parse_renderer(&sf.inner, &obj.inner)
                        {
                            if renderer.game_object.file_id == 0
                                && renderer.game_object.path_id != 0
                                && !renderer.materials.is_empty()
                            {
                                renderer_game_objects.push((
                                    sf_index,
                                    obj.path_id,
                                    renderer.game_object.path_id,
                                    renderer.materials,
                                ));
                            }
                        }
                    }
                    137 => {
                        if let Ok(skinned) =
                            UnityClassParser::parse_skinned_mesh_renderer(&sf.inner, &obj.inner)
                        {
                            let mesh = skinned.renderer.mesh;
                            let game_object = skinned.renderer.game_object;
                            let materials = skinned.renderer.materials;
                            if let Some(mesh) = mesh {
                                if Self::live_pptr_targets_asset(
                                    db,
                                    mesh_bundle_path,
                                    externals,
                                    &mesh,
                                    mesh_bundle_path,
                                    mesh_path_id,
                                ) {
                                    direct_renderers.push((
                                        sf_index,
                                        obj.path_id,
                                        materials.clone(),
                                    ));
                                }
                            }
                            if game_object.file_id == 0
                                && game_object.path_id != 0
                                && !materials.is_empty()
                            {
                                renderer_game_objects.push((
                                    sf_index,
                                    obj.path_id,
                                    game_object.path_id,
                                    materials,
                                ));
                            }
                        }
                    }
                    1 => {
                        if let Ok(game_object) =
                            UnityClassParser::parse_game_object(&sf.inner, &obj.inner)
                        {
                            if game_object.components.iter().any(|component| {
                                component.file_id == 0
                                    && game_object_by_mesh_filter.contains_key(&component.path_id)
                            }) {
                                game_objects_with_mesh.insert(obj.path_id);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let mut refs = Vec::<(i64, String)>::new();
        let mut seen = HashSet::<(i64, String)>::new();
        for (sf_index, renderer_path_id, materials) in direct_renderers {
            Self::push_live_material_refs(
                db,
                mesh_bundle_path,
                bundle
                    .assets
                    .get(sf_index)
                    .map(|sf| sf.inner.m_externals.as_slice())
                    .unwrap_or(&[]),
                renderer_path_id,
                &materials,
                &mut refs,
                &mut seen,
                progress,
            );
        }
        for (sf_index, renderer_path_id, game_object_id, materials) in renderer_game_objects {
            if !game_objects_with_mesh.contains(&game_object_id) {
                continue;
            }
            Self::push_live_material_refs(
                db,
                mesh_bundle_path,
                bundle
                    .assets
                    .get(sf_index)
                    .map(|sf| sf.inner.m_externals.as_slice())
                    .unwrap_or(&[]),
                renderer_path_id,
                &materials,
                &mut refs,
                &mut seen,
                progress,
            );
        }

        if refs.is_empty() {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] live renderer/material scan found no Material refs for mesh_path_id={}",
                    mesh_path_id
                ),
            });
        }
        refs
    }

    fn live_pptr_targets_asset(
        db: &AssetDatabase,
        source_bundle_path: &str,
        externals: &[FileIdentifier],
        pptr: &PPtr,
        target_bundle_path: &str,
        target_path_id: i64,
    ) -> bool {
        if pptr.is_null() || pptr.path_id != target_path_id {
            return false;
        }
        UnityExternalResolver::resolve_pptr(db, source_bundle_path, externals, pptr)
            .map(|bundle_path| bundle_path == target_bundle_path)
            .unwrap_or(false)
    }

    fn push_live_material_refs(
        db: &AssetDatabase,
        mesh_bundle_path: &str,
        externals: &[FileIdentifier],
        renderer_path_id: i64,
        materials: &[PPtr],
        refs: &mut Vec<(i64, String)>,
        seen: &mut HashSet<(i64, String)>,
        progress: &Channel<ProgressPayload>,
    ) {
        for material in materials {
            if material.is_null() {
                continue;
            }
            let Some(bundle_path) =
                UnityExternalResolver::resolve_pptr(db, mesh_bundle_path, externals, material)
            else {
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG] live renderer material source={} file_id={} path_id={} has no resolvable Bundle",
                        renderer_path_id, material.file_id, material.path_id
                    ),
                });
                continue;
            };
            if seen.insert((material.path_id, bundle_path.clone())) {
                refs.push((material.path_id, bundle_path.clone()));
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG] live renderer material source={} file_id={} path_id={} bundle={}",
                        renderer_path_id, material.file_id, material.path_id, bundle_path
                    ),
                });
            }
        }
    }

    /// Parse Material from a Bundle with the AssetStudio layout parser (called during dependency lookup).
    ///
    /// `material_bundle_path` is the disk path of the Bundle containing the Material,
    /// used as file context when parsing texture PPtrs to ensure subsequent texture
    /// lookups prefer the Material's own bundle (avoiding cross-bundle path_id collisions).
    fn parse_material_from_map(
        bundle: &crate::common::bundle_file::asset_bundle::AssetBundle,
        material_path_id: i64,
        material_name: &str,
        material_bundle_path: &str,
        progress: &Channel<ProgressPayload>,
    ) -> Option<(MaterialInfo, HashMap<i64, i32>)> {
        // Search all SerializedFiles in the Bundle for the Material object
        let search_result = bundle.assets.iter().find_map(|sf| {
            sf.objects
                .iter()
                .find(|o| o.path_id == material_path_id && o.class_id == 21)
                .map(|o| (sf, o))
        });

        let (sf, obj_info) = match search_result {
            Some(v) => v,
            None => {
                let _ = progress.send(ProgressPayload {
                    step: "lookup".into(),
                    message: format!(
                        "Material path_id={} was not found in the candidate Bundle",
                        material_path_id
                    ),
                });
                return None;
            }
        };

        let typed_material = UnityMaterialTextureExtractor::extract(
            &sf.inner,
            &obj_info.inner,
            material_name,
            material_bundle_path,
        );
        // A successful layout parse is kept even when it exposes zero texture
        // slots: textureless materials (e.g. baked Unity Default-Material) are
        // valid parse results, not failures, and callers rely on the parsed
        // identity for name-based texture fallbacks and material reporting.
        let layout_material = if let Some((material_info, file_id_by_path_id)) =
            typed_material.clone()
        {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] parse_material_from_map: AssetStudio layout parser succeeded for '{}' with {} texture slot(s)",
                    material_info.name,
                    material_info.textures.len()
                ),
            });
            for texture_slot in &material_info.textures {
                let decoded =
                    UnityMaterialTextureExtractor::decode_texture_ref(&texture_slot.relative_path);
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG]   typed slot '{}': file_id={}, path_id={}",
                        texture_slot.slot_name,
                        decoded.map(|(file_id, _)| file_id).unwrap_or(0),
                        decoded.map(|(_, path_id)| path_id).unwrap_or(0)
                    ),
                });
            }

            if !material_info.textures.is_empty() {
                let _ = progress.send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[DIAG] parse_material_from_map: using {} texture slot(s) from AssetStudio layout parser",
                        material_info.textures.len()
                    ),
                });
                return Some((material_info, file_id_by_path_id));
            }
            Some((material_info, file_id_by_path_id))
        } else {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] parse_material_from_map: AssetStudio layout parser failed for '{}'",
                    material_name
                ),
            });
            None
        };

        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG] parse_material_from_map: {} for '{}'; trying TypeTree material fallback",
                if layout_material.is_some() {
                    "layout parser produced no texture slots"
                } else {
                    "AssetStudio layout parser failed"
                },
                material_name
            ),
        });

        let typetree_result = Self::parse_material_from_typetree(
            sf,
            obj_info,
            material_name,
            material_bundle_path,
            progress,
        );
        if typetree_result.is_some() {
            return typetree_result;
        }

        layout_material
    }

    fn parse_material_from_typetree(
        sf: &crate::common::bundle_file::asset_bundle::SerializedFile,
        obj_info: &crate::common::bundle_file::asset_bundle::ObjectInfo,
        material_name: &str,
        material_bundle_path: &str,
        progress: &Channel<ProgressPayload>,
    ) -> Option<(MaterialInfo, HashMap<i64, i32>)> {
        let handle = crate::common::bundle_file::asset_bundle::ObjectHandle::new(sf, obj_info);
        let value = handle.read().ok()?;
        let mut file_id_by_path_id = HashMap::new();
        let textures = Self::extract_typetree_texture_slots(
            &value,
            material_bundle_path,
            &mut file_id_by_path_id,
        );
        if textures.is_empty() {
            let _ = progress.send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[DIAG] TypeTree material fallback found no texture slots for '{}'",
                    material_name
                ),
            });
            return None;
        }

        let _ = progress.send(ProgressPayload {
            step: "diag".into(),
            message: format!(
                "[DIAG] TypeTree material fallback found {} texture slot(s) for '{}'",
                textures.len(),
                material_name
            ),
        });
        Some((
            MaterialInfo {
                name: material_name.to_string(),
                textures,
                ..Default::default()
            },
            file_id_by_path_id,
        ))
    }

    fn extract_typetree_texture_slots(
        value: &UnityValue,
        material_bundle_path: &str,
        file_id_by_path_id: &mut HashMap<i64, i32>,
    ) -> Vec<crate::exporter::material_info::TextureSlot> {
        let Some(tex_envs) = Self::find_typetree_field(value, "m_TexEnvs") else {
            return Vec::new();
        };

        let mut slots = Vec::new();
        Self::collect_typetree_tex_env_slots(
            tex_envs,
            material_bundle_path,
            file_id_by_path_id,
            &mut slots,
        );
        slots
    }

    fn collect_typetree_tex_env_slots(
        value: &UnityValue,
        material_bundle_path: &str,
        file_id_by_path_id: &mut HashMap<i64, i32>,
        slots: &mut Vec<crate::exporter::material_info::TextureSlot>,
    ) {
        match value {
            UnityValue::Array(items) => {
                for item in items {
                    Self::collect_typetree_tex_env_slots(
                        item,
                        material_bundle_path,
                        file_id_by_path_id,
                        slots,
                    );
                }
            }
            UnityValue::Object(map) => {
                let slot_name = map
                    .get("first")
                    .and_then(UnityValue::as_str)
                    .unwrap_or("")
                    .to_string();
                let second = map.get("second").unwrap_or(value);
                if let Some((file_id, path_id)) = Self::find_first_ptr(second) {
                    if path_id == 0 {
                        return;
                    }
                    if file_id != 0 {
                        file_id_by_path_id.insert(path_id, file_id);
                    }
                    let slot_name = if slot_name.is_empty() {
                        format!("Texture_{}", slots.len())
                    } else {
                        slot_name
                    };
                    slots.push(crate::exporter::material_info::TextureSlot {
                        slot_name: slot_name.clone(),
                        usage: UnityMaterialTextureExtractor::slot_usage(&slot_name).to_string(),
                        file_name: format!("tex_{}.png", path_id),
                        relative_path: UnityMaterialTextureExtractor::encode_texture_ref(
                            file_id,
                            path_id,
                            material_bundle_path,
                        ),
                    });
                }
            }
            _ => {}
        }
    }

    fn find_typetree_field<'a>(value: &'a UnityValue, field_name: &str) -> Option<&'a UnityValue> {
        match value {
            UnityValue::Object(map) => map.get(field_name).or_else(|| {
                map.values()
                    .find_map(|child| Self::find_typetree_field(child, field_name))
            }),
            UnityValue::Array(items) => items
                .iter()
                .find_map(|child| Self::find_typetree_field(child, field_name)),
            _ => None,
        }
    }

    fn find_first_ptr(value: &UnityValue) -> Option<(i32, i64)> {
        match value {
            UnityValue::Ptr { file_id, path_id } => Some((*file_id, *path_id)),
            UnityValue::Object(map) => {
                if let Some(ptr) = map.get("m_Texture").and_then(Self::find_first_ptr) {
                    return Some(ptr);
                }
                map.values().find_map(Self::find_first_ptr)
            }
            UnityValue::Array(items) => items.iter().find_map(Self::find_first_ptr),
            _ => None,
        }
    }

    /// Extract PPtr fields from an encoded relative_path.
    ///
    /// Current format: `#<file_id>:<path_id>@<bundle_path>`
    /// New format: `#<path_id>@<bundle_path>`
    /// Old format: `#<path_id>` (backward compatible, no source_bundle)
    fn decode_texture_ref(relative_path: &str) -> Option<DecodedTextureRef<'_>> {
        let inner = relative_path.strip_prefix('#')?;
        let (id_part, source_bundle) = inner
            .split_once('@')
            .map(|(ids, bundle)| (ids, Some(bundle)))
            .unwrap_or((inner, None));
        let (file_id, path_id_part) = id_part
            .split_once(':')
            .map(|(file_id, path_id)| (file_id.parse::<i32>().ok(), path_id))
            .unwrap_or((None, id_part));
        let path_id = path_id_part.parse::<i64>().ok()?;
        Some(DecodedTextureRef {
            file_id,
            path_id,
            source_bundle,
        })
    }

    /// Get a Bundle from cache, or load and cache it on miss.
    fn load_bundle_cached<'a>(
        cache: &'a mut HashMap<String, crate::common::bundle_file::asset_bundle::AssetBundle>,
        bundle_path: &str,
    ) -> Option<&'a crate::common::bundle_file::asset_bundle::AssetBundle> {
        use std::collections::hash_map::Entry;
        let entry = match cache.entry(bundle_path.to_string()) {
            Entry::Occupied(e) => return Some(e.into_mut()),
            Entry::Vacant(e) => e,
        };
        let bp = std::path::Path::new(bundle_path);
        if !bp.exists() {
            return None;
        }
        let bundle = AssetBundleLoader::load_bundle(bp).ok()?;
        Some(entry.insert(bundle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::asset_map::asset_index::AssetRow;
    use std::collections::HashMap;

    #[test]
    fn texture_fallback_token_sets_ignore_generic_body_mesh_name() {
        let token_sets =
            MaterialTextureResolver::texture_fallback_token_sets("female_Body_ink", "body");

        assert!(token_sets.contains(&vec!["female_body".to_string()]));
        assert!(!token_sets.contains(&vec!["body".to_string()]));
    }

    #[test]
    fn texture_name_fallback_after_resolution_allows_empty_or_unresolved_slots() {
        let empty_material = MaterialInfo {
            name: "female_Body".to_string(),
            ..Default::default()
        };
        let unresolved_material = MaterialInfo {
            name: "female_Body".to_string(),
            textures: vec![crate::exporter::material_info::TextureSlot {
                slot_name: "_MainTex".to_string(),
                usage: "DiffuseColor".to_string(),
                file_name: "tex.png".to_string(),
                relative_path: "#7:-367321471771861468@bundle".to_string(),
            }],
            ..Default::default()
        };
        let resolved_material = MaterialInfo {
            name: "female_Body".to_string(),
            textures: vec![crate::exporter::material_info::TextureSlot {
                slot_name: "_MainTex".to_string(),
                usage: "DiffuseColor".to_string(),
                file_name: "tex.png".to_string(),
                relative_path: "tex.png".to_string(),
            }],
            ..Default::default()
        };

        assert!(
            MaterialTextureResolver::should_try_texture_name_fallback_after_resolution(
                &empty_material
            )
        );
        assert!(
            MaterialTextureResolver::should_try_texture_name_fallback_after_resolution(
                &unresolved_material
            )
        );
        assert!(
            !MaterialTextureResolver::should_try_texture_name_fallback_after_resolution(
                &resolved_material
            )
        );
    }

    #[test]
    fn texture_name_fallback_populates_material_slots() {
        let mut material = MaterialInfo {
            name: "ch_body03".to_string(),
            ..Default::default()
        };
        let texture_refs = vec![
            (11, "bundle".to_string(), "ch_body03_d".to_string()),
            (12, "bundle".to_string(), "ch_body03_n".to_string()),
            (13, "bundle".to_string(), "ch_body03_mrav".to_string()),
            (14, "bundle".to_string(), "ch_body03_ao".to_string()),
            (15, "bundle".to_string(), "ch_body03_mask".to_string()),
        ];

        MaterialTextureResolver::append_fallback_texture_slots(&mut material, &texture_refs);

        assert_eq!(material.textures.len(), 5);
        assert_eq!(material.textures[0].slot_name, "_MainTex");
        assert_eq!(material.textures[0].usage, "DiffuseColor");
        assert_eq!(material.textures[1].slot_name, "_BumpMap");
        assert_eq!(material.textures[1].usage, "NormalMap");
        assert_eq!(material.textures[2].slot_name, "_SpecGlossMap");
        assert_eq!(material.textures[2].usage, "SpecularColor");
        assert_eq!(material.textures[3].slot_name, "_OcclusionMap");
        assert_eq!(material.textures[3].usage, "OcclusionMap");
        assert_eq!(material.textures[4].slot_name, "_MaskMap");
        assert_eq!(material.textures[4].usage, "MaskMap");
        assert_eq!(material.textures[0].relative_path, "#0:11@bundle");
    }

    #[test]
    fn same_name_borrow_allows_specific_mesh_names_only() {
        for specific in [
            "Onion_Sliced",
            "Onion_Whole",
            "m_airballoon_rope_2units_01",
            "mat_airballoon_rope_2units_01",
            "food_sliced_onion_01",
            "ch_body03",
        ] {
            assert!(
                MaterialTextureResolver::is_specific_borrow_mesh_name(specific),
                "expected {specific:?} to be borrowable"
            );
        }
        for generic in [
            "Body",
            "Hand_Open_R",
            "Hat_Fancy",
            "Chef_Red",
            "NPC_Mike",
            "Knife",
            "Cleaver",
            "Quad(Clone)",
            "Combined Mesh (root: scene)",
            "Tail",
            "mesh",
            "Cube",
            "",
            "abc",
        ] {
            assert!(
                !MaterialTextureResolver::is_specific_borrow_mesh_name(generic),
                "expected {generic:?} to be rejected for borrowing"
            );
        }
    }

    #[test]
    fn texture_family_completion_keeps_existing_slots_and_adds_distinct_maps() {
        let mut material = MaterialInfo {
            name: "ch_body03".to_string(),
            textures: vec![crate::exporter::material_info::TextureSlot {
                slot_name: "_MainTex".to_string(),
                usage: "DiffuseColor".to_string(),
                file_name: "tex_11.png".to_string(),
                relative_path: "#0:11@bundle".to_string(),
            }],
            ..Default::default()
        };
        let texture_refs = vec![
            (11, "bundle".to_string(), "ch_body03_d".to_string()),
            (12, "bundle".to_string(), "ch_body03_n".to_string()),
            (13, "bundle".to_string(), "ch_body03_mask".to_string()),
        ];

        MaterialTextureResolver::append_fallback_texture_slots(&mut material, &texture_refs);

        assert_eq!(material.textures.len(), 4);
        assert_eq!(material.textures[0].slot_name, "_MainTex");
        assert_eq!(material.textures[1].slot_name, "_MainTex#2");
        assert_eq!(material.textures[2].slot_name, "_BumpMap");
        assert_eq!(material.textures[3].slot_name, "_MaskMap");
    }

    #[test]
    fn explicit_material_texture_slots_do_not_receive_family_fallback_slots() {
        let mut material = MaterialInfo {
            name: "ch_body03".to_string(),
            textures: vec![crate::exporter::material_info::TextureSlot {
                slot_name: "_MainTex".to_string(),
                usage: "DiffuseColor".to_string(),
                file_name: "tex_11.png".to_string(),
                relative_path: "#0:11@bundle".to_string(),
            }],
            ..Default::default()
        };
        let texture_refs = vec![
            (12, "bundle".to_string(), "ch_body03_n".to_string()),
            (13, "bundle".to_string(), "ch_body03_mask".to_string()),
        ];

        let resolved_texture_count_for_material = 1;
        if resolved_texture_count_for_material == 0
            && MaterialTextureResolver::should_try_texture_name_fallback_after_resolution(&material)
        {
            MaterialTextureResolver::append_fallback_texture_slots(&mut material, &texture_refs);
        }

        assert_eq!(material.textures.len(), 1);
        assert_eq!(material.textures[0].slot_name, "_MainTex");
        assert_eq!(material.textures[0].relative_path, "#0:11@bundle");
    }

    #[test]
    fn unresolved_explicit_material_texture_slots_can_receive_name_fallback_slots() {
        let material = MaterialInfo {
            name: "ch_body03".to_string(),
            textures: vec![crate::exporter::material_info::TextureSlot {
                slot_name: "Stealth".to_string(),
                usage: "Unknown".to_string(),
                file_name: "tex_missing.png".to_string(),
                relative_path: "#120:-3415679935937773568@bundle".to_string(),
            }],
            ..Default::default()
        };

        assert!(
            MaterialTextureResolver::should_try_texture_name_fallback_after_resolution(&material)
        );
    }

    #[test]
    fn container_texture_fallback_accepts_tga_container_paths() {
        assert!(MaterialTextureResolver::container_path_looks_like_texture(
            "assets/artdata/effects/efx_wanfa/chiji/wall/chijiwall001.tga"
        ));
        assert!(MaterialTextureResolver::container_path_looks_like_texture(
            "assets/artdata/entity/textures/bp_flag_01_d_ams.tga"
        ));
        assert!(!MaterialTextureResolver::container_path_looks_like_texture(
            "assets/artdata/effects/model/xianjing.fbx"
        ));
    }

    #[test]
    fn container_texture_fallback_scores_context_matches_first() {
        let context_tokens =
            MaterialTextureResolver::texture_fallback_name_stems("bp_flag_01_d_ams");

        let matching = MaterialTextureResolver::container_texture_fallback_score(
            "BP_flag_01_D_AMS",
            "assets/artdata/entity/textures/bp_flag_01_d_ams.tga",
            &context_tokens,
        );
        let unrelated = MaterialTextureResolver::container_texture_fallback_score(
            "chijiWall001",
            "assets/artdata/effects/efx_wanfa/chiji/wall/chijiwall001.tga",
            &context_tokens,
        );

        assert!(matching > unrelated);
    }

    #[test]
    fn container_texture_fallback_scores_short_structured_mesh_code() {
        let context_tokens = MaterialTextureResolver::texture_fallback_name_stems("X_TY_GJ");

        let matching = MaterialTextureResolver::container_texture_fallback_score(
            "X_TY_GJ_0006_N_01_AMS",
            "assets/artdata/character/textures/x_ty_gj_0006_n_01_ams.tga",
            &context_tokens,
        );
        let unrelated = MaterialTextureResolver::container_texture_fallback_score(
            "M_TY_7043_Pants_N_01_AMS",
            "assets/artdata/character/textures/m_ty_7043_pants_n_01_ams.tga",
            &context_tokens,
        );

        assert!(matching > unrelated);
    }

    #[test]
    fn mesh_name_texture_fallback_requires_prefix_match() {
        assert!(
            MaterialTextureResolver::texture_candidate_matches_context_prefix(
                "Cat_Parts_0013_Head_D_01_AMS",
                "assets/artdata/character/npc/textures/cat_parts_0013_head_d_01_ams.tga",
                "Cat_Parts_0013_Head",
            )
        );
        assert!(
            !MaterialTextureResolver::texture_candidate_matches_context_prefix(
                "ChaHu_0001_D_01_AMS",
                "assets/artdata/scene/textures/chahu_0001_d_01_ams.tga",
                "Cat_Parts_0013_Head",
            )
        );
        assert!(
            !MaterialTextureResolver::texture_candidate_matches_context_prefix(
                "body_d_01_ams",
                "assets/artdata/character/textures/body_d_01_ams.tga",
                "body",
            )
        );
    }

    #[test]
    fn mesh_name_texture_fallback_does_not_match_mid_string_mentions() {
        assert!(
            !MaterialTextureResolver::texture_candidate_matches_context_prefix(
                "Other_Cat_Parts_0013_Head_D_01_AMS",
                "assets/artdata/character/npc/textures/other_cat_parts_0013_head_d_01_ams.tga",
                "Cat_Parts_0013_Head",
            )
        );
    }

    #[test]
    fn texture_ref_decoder_preserves_explicit_file_id() {
        let decoded = MaterialTextureResolver::decode_texture_ref(
            "#7:-367321471771861468@D:\\NarakaAssets\\c\\o",
        )
        .expect("decode current format");
        assert_eq!(decoded.file_id, Some(7));
        assert_eq!(decoded.path_id, -367321471771861468);
        assert_eq!(decoded.source_bundle, Some("D:\\NarakaAssets\\c\\o"));

        let legacy = MaterialTextureResolver::decode_texture_ref("#811233593949903557@bundle")
            .expect("decode legacy bundle format");
        assert_eq!(legacy.file_id, None);
        assert_eq!(legacy.path_id, 811233593949903557);
        assert_eq!(legacy.source_bundle, Some("bundle"));
    }

    #[test]
    fn renderer_ids_are_collected_in_discovery_order() {
        let mut renderer_ids = Vec::new();
        let mut seen_renderer_ids = std::collections::HashSet::new();
        for id in [30, 10, 30, 20] {
            if seen_renderer_ids.insert(id) {
                renderer_ids.push(id);
            }
        }

        assert_eq!(renderer_ids, vec![30, 10, 20]);
    }

    #[test]
    fn live_material_ref_collection_dedupes_local_renderer_materials() {
        let workspace = std::env::temp_dir().join(format!(
            "assetfinder_live_material_refs_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let db = AssetDatabase::open(&workspace).unwrap();
        let progress = tauri::ipc::Channel::<ProgressPayload>::new(|_| Ok(()));
        let bundle_path = workspace.join("bundle.ab").to_string_lossy().to_string();
        let materials = vec![
            PPtr {
                file_id: 0,
                path_id: 21,
            },
            PPtr {
                file_id: 0,
                path_id: 21,
            },
            PPtr {
                file_id: 0,
                path_id: 22,
            },
        ];
        let mut refs = Vec::new();
        let mut seen = HashSet::new();

        MaterialTextureResolver::push_live_material_refs(
            &db,
            &bundle_path,
            &[],
            137,
            &materials,
            &mut refs,
            &mut seen,
            &progress,
        );

        assert_eq!(
            refs,
            vec![(21, bundle_path.clone()), (22, bundle_path.clone())]
        );

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn global_fallback_prefers_texture_like_rows_before_mesh_rows() {
        let mut rows = vec![
            AssetRow {
                bundle_path: "b3".to_string(),
                path_id: 7,
                class_id: 43,
                class_name: "Mesh".to_string(),
                asset_name: "mesh".to_string(),
                byte_size: 30,
            },
            AssetRow {
                bundle_path: "b2".to_string(),
                path_id: 7,
                class_id: 28,
                class_name: "Texture2D".to_string(),
                asset_name: "tex".to_string(),
                byte_size: 20,
            },
            AssetRow {
                bundle_path: "b1".to_string(),
                path_id: 7,
                class_id: 213,
                class_name: "Sprite".to_string(),
                asset_name: "spr".to_string(),
                byte_size: 10,
            },
        ];

        rows.retain(|row| row.class_name == "Texture2D" || row.class_name == "Sprite");
        rows.sort_by(|left, right| {
            left.class_name
                .cmp(&right.class_name)
                .then_with(|| left.bundle_path.cmp(&right.bundle_path))
                .then_with(|| left.asset_name.cmp(&right.asset_name))
                .then_with(|| left.byte_size.cmp(&right.byte_size))
        });

        assert_eq!(rows[0].class_name, "Sprite");
        assert_eq!(rows[1].class_name, "Texture2D");
    }

    #[test]
    fn texture_family_stem_strips_common_texture_suffixes() {
        assert_eq!(
            MaterialTextureResolver::texture_family_stem("female_body01_safe_n").as_deref(),
            Some("female_body01_safe")
        );
        assert_eq!(
            MaterialTextureResolver::texture_family_stem("female_body01_safe_d").as_deref(),
            Some("female_body01_safe")
        );
        assert_eq!(
            MaterialTextureResolver::texture_family_stem("female_body01_safe"),
            None
        );
    }

    #[test]
    fn typetree_material_fallback_extracts_tex_env_pptrs() {
        let tex_envs = UnityValue::Array(vec![
            UnityValue::Object(HashMap::from([
                (
                    "first".to_string(),
                    UnityValue::String("_MainTex".to_string()),
                ),
                (
                    "second".to_string(),
                    UnityValue::Object(HashMap::from([(
                        "m_Texture".to_string(),
                        UnityValue::Ptr {
                            file_id: 2,
                            path_id: 42,
                        },
                    )])),
                ),
            ])),
            UnityValue::Object(HashMap::from([
                (
                    "first".to_string(),
                    UnityValue::String("_BumpMap".to_string()),
                ),
                (
                    "second".to_string(),
                    UnityValue::Object(HashMap::from([(
                        "m_Texture".to_string(),
                        UnityValue::Ptr {
                            file_id: 0,
                            path_id: 0,
                        },
                    )])),
                ),
            ])),
        ]);
        let root = UnityValue::Object(HashMap::from([(
            "m_SavedProperties".to_string(),
            UnityValue::Object(HashMap::from([("m_TexEnvs".to_string(), tex_envs)])),
        )]));
        let mut file_id_by_path_id = HashMap::new();

        let slots = MaterialTextureResolver::extract_typetree_texture_slots(
            &root,
            "bundle-a",
            &mut file_id_by_path_id,
        );

        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].slot_name, "_MainTex");
        assert_eq!(slots[0].usage, "DiffuseColor");
        assert_eq!(slots[0].relative_path, "#2:42@bundle-a");
        assert_eq!(file_id_by_path_id.get(&42), Some(&2));
    }
}
