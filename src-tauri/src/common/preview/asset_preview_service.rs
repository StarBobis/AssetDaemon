use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use crate::common::asset_map::asset_index::{AssetDatabase, AssetRow, RelationRow};
use crate::common::asset_map::dependency_resolver::AssetMapDependencyResolver;
use crate::common::asset_map::repository::AssetMapRepository;
use crate::common::bundle_file::asset_bundle::{AssetBundle, AssetBundleLoader};
use crate::common::preview::preview_types::{
    AssetPreviewResult, PreviewAssetRef, PreviewRelation, PreviewRow, PreviewSection,
};
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::unity_dependency::external_resolver::UnityExternalResolver;
use crate::unity::classes::object::PPtr;
use crate::unity::classes::registry::{UnityClassObject, UnityClassParser};
use crate::unity::relations::UnityRelationKind;
use crate::unity::type_tree::unity_value::UnityValue;
use crate::utils::unity_object_name_utils::UnityObjectNameUtils;
use tauri::ipc::Channel;

pub struct AssetPreviewService;

impl AssetPreviewService {
    pub fn preview(
        bundle_path: &str,
        path_id: i64,
        workspace_dir: Option<&Path>,
        asset_map_cache_root: Option<&Path>,
        progress: Option<&Channel<ProgressPayload>>,
    ) -> Result<AssetPreviewResult, String> {
        let bundle_path_ref = Path::new(bundle_path);
        if !bundle_path_ref.exists() {
            return Err(format!("Bundle not found: {}", bundle_path));
        }

        let bundle = AssetBundleLoader::load_unity_file_serialized_only(bundle_path_ref)
            .map_err(|e| format!("Failed to load Unity file: {}", e))?;
        let (sf, bundle_object_info) = Self::find_object(&bundle, path_id)
            .ok_or_else(|| format!("Asset path_id={} not found", path_id))?;
        let object_info = &bundle_object_info.inner;
        let header = Self::basic_result(&sf.inner, object_info);
        let class_name = header.class_name.clone();
        let name = Self::object_name(&sf.inner, object_info).unwrap_or_else(|| header.name.clone());
        let db = workspace_dir
            .and_then(|workspace| AssetMapRepository::open(workspace, asset_map_cache_root).ok());

        let mut result = AssetPreviewResult {
            class_name: class_name.clone(),
            path_id: path_id.to_string(),
            name,
            unity_version: header.unity_version,
            byte_size: header.byte_size,
            sections: vec![PreviewSection {
                title: "Summary".to_string(),
                kind: "summary".to_string(),
                rows: vec![
                    row("Name", &header.name),
                    row("Class", &class_name),
                    row("PathID", path_id),
                    row("Unity", &sf.unity_version),
                    row("Size", format!("{} B", header.byte_size)),
                ],
            }],
            relations: Vec::new(),
            warnings: Vec::new(),
        };

        match class_name.as_str() {
            "Material" => Self::append_material_preview(
                &mut result,
                &bundle,
                bundle_path,
                &sf.inner,
                object_info,
                db.as_ref(),
                progress,
            ),
            "Animator" => Self::append_animator_preview(
                &mut result,
                bundle_path,
                &sf.inner,
                object_info,
                db.as_ref(),
            ),
            "Avatar" => Self::append_avatar_preview(
                &mut result,
                &sf.inner,
                object_info,
                bundle_path,
                db.as_ref(),
            ),
            "AnimationClip" => {
                Self::append_animation_clip_preview(&mut result, &sf.inner, object_info)
            }
            "Shader" => Self::append_shader_preview(&mut result, &sf.inner, object_info),
            "MonoBehaviour" => {
                Self::append_mono_behaviour_preview(&mut result, &sf.inner, object_info)
            }
            "SpriteMask" => Self::append_sprite_mask_preview(
                &mut result,
                &sf.inner,
                object_info,
                bundle_path,
                db.as_ref(),
            ),
            "GameObject" | "Transform" | "Renderer" | "SkinnedMeshRenderer" | "MeshFilter" => {
                Self::append_scene_object_preview(&mut result, &sf.inner, object_info)
            }
            _ => Self::append_typetree_summary(&mut result, &sf.inner, object_info),
        }

        if let Some(db) = db.as_ref() {
            Self::append_asset_map_relations(&mut result, db, bundle_path, path_id);
        } else {
            result.warnings.push(
                "AssetMap is not available; relation preview is limited to local object parsing."
                    .to_string(),
            );
        }

        Ok(result)
    }

    fn append_material_preview(
        result: &mut AssetPreviewResult,
        _bundle: &AssetBundle,
        bundle_path: &str,
        sf: &crate::common::serialized_file::serialized_file::SerializedFile,
        object_info: &crate::common::serialized_file::serialized_file::ObjectInfo,
        db: Option<&AssetDatabase>,
        progress: Option<&Channel<ProgressPayload>>,
    ) {
        match UnityClassParser::parse_material(sf, object_info) {
            Ok(material) => {
                result.sections.push(PreviewSection {
                    title: "Material".to_string(),
                    kind: "properties".to_string(),
                    rows: vec![
                        row("Shader Textures", material.texture_slots.len()),
                        row("Diffuse", format_color(material.diffuse_color)),
                        row("Specular", format_color(material.specular_color)),
                        row("Emission", format_color(material.emissive_color)),
                        row("Shininess", format!("{:.3}", material.shininess)),
                    ],
                });

                let texture_rows = material
                    .texture_slots
                    .iter()
                    .map(|slot| {
                        row(
                            &slot.slot_name,
                            format!(
                                "file_id={}, path_id={}",
                                slot.texture.file_id, slot.texture.path_id
                            ),
                        )
                    })
                    .collect();
                result.sections.push(PreviewSection {
                    title: "Texture Slots".to_string(),
                    kind: "table".to_string(),
                    rows: texture_rows,
                });

                if let (Some(db), Some(progress)) = (db, progress) {
                    if let Ok((_material_info, textures)) =
                        AssetMapDependencyResolver::resolve_material_textures(
                            db,
                            bundle_path,
                            object_info.path_id,
                            progress,
                        )
                    {
                        result.sections.push(PreviewSection {
                            title: "Resolved Textures".to_string(),
                            kind: "relations".to_string(),
                            rows: textures
                                .iter()
                                .map(|(path_id, bundle, name)| {
                                    row(name, format!("{} @ {}", path_id, short_path(bundle)))
                                })
                                .collect(),
                        });
                    }
                }
            }
            Err(error) => {
                result
                    .warnings
                    .push(format!("Material parser failed: {}", error));
                Self::append_typetree_summary(result, sf, object_info);
            }
        }
    }

    fn append_animator_preview(
        result: &mut AssetPreviewResult,
        bundle_path: &str,
        sf: &crate::common::serialized_file::serialized_file::SerializedFile,
        object_info: &crate::common::serialized_file::serialized_file::ObjectInfo,
        db: Option<&AssetDatabase>,
    ) {
        match UnityClassParser::parse_animator(sf, object_info) {
            Ok(animator) => {
                result.sections.push(PreviewSection {
                    title: "Animator".to_string(),
                    kind: "properties".to_string(),
                    rows: vec![
                        row("GameObject", format_pptr(animator.component.game_object)),
                        row("Avatar", format_pptr(animator.avatar)),
                        row("Controller", format_pptr(animator.controller)),
                    ],
                });

                if let Some(db) = db {
                    let related = Self::collect_animator_model_refs(
                        db,
                        bundle_path,
                        object_info.path_id,
                        animator.component.game_object.path_id,
                    );
                    result.sections.push(PreviewSection {
                        title: "Model Candidates".to_string(),
                        kind: "relations".to_string(),
                        rows: vec![
                            row("GameObjects", related.game_objects.len()),
                            row("Renderers", related.renderers.len()),
                            row("Meshes", related.meshes.len()),
                            row("Materials", related.materials.len()),
                            row("Textures", related.textures.len()),
                        ],
                    });
                    Self::append_asset_refs_section(result, "Meshes", &related.meshes);
                    Self::append_asset_refs_section(result, "Materials", &related.materials);
                    Self::append_asset_refs_section(result, "Textures", &related.textures);
                }
            }
            Err(error) => result
                .warnings
                .push(format!("Animator parser failed: {}", error)),
        }
    }

    fn append_avatar_preview(
        result: &mut AssetPreviewResult,
        sf: &crate::common::serialized_file::serialized_file::SerializedFile,
        object_info: &crate::common::serialized_file::serialized_file::ObjectInfo,
        bundle_path: &str,
        db: Option<&AssetDatabase>,
    ) {
        if let Some(value) = UnityClassParser::read_typetree_value_public(sf, object_info) {
            let tos_count = Self::find_field(&value, "m_TOS")
                .and_then(|v| match v {
                    UnityValue::Array(items) => Some(items.len()),
                    _ => None,
                })
                .unwrap_or(0);
            let skeleton_id_count = Self::find_field(&value, "m_ID")
                .and_then(|v| match v {
                    UnityValue::Array(items) => Some(items.len()),
                    _ => None,
                })
                .unwrap_or(0);
            let default_pose_count = Self::find_field(&value, "m_DefaultPose")
                .and_then(|pose| Self::find_field(pose, "m_X"))
                .and_then(|v| match v {
                    UnityValue::Array(items) => Some(items.len()),
                    _ => None,
                })
                .unwrap_or(0);
            result.sections.push(PreviewSection {
                title: "Avatar".to_string(),
                kind: "properties".to_string(),
                rows: vec![
                    row("TOS Bone Paths", tos_count),
                    row("Skeleton IDs", skeleton_id_count),
                    row("Default Pose XForms", default_pose_count),
                    row(
                        "Root Motion",
                        Self::find_field(&value, "m_RootMotionBoneIndex")
                            .and_then(UnityValue::as_i64)
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "?".to_string()),
                    ),
                ],
            });

            let bone_paths = Self::extract_tos_paths(&value, 48);
            if !bone_paths.is_empty() {
                result.sections.push(PreviewSection {
                    title: "Bone Paths".to_string(),
                    kind: "list".to_string(),
                    rows: bone_paths
                        .into_iter()
                        .map(|(hash, path)| row(hash, path))
                        .collect(),
                });
            }
        } else {
            result.warnings.push(
                "Avatar TypeTree is unavailable; only raw and AssetMap relations can be shown."
                    .to_string(),
            );
        }

        if let Some(db) = db {
            let animators = lookup_relations_by_target(
                db,
                UnityRelationKind::ANIMATOR_AVATAR,
                bundle_path,
                object_info.path_id,
            )
            .into_iter()
            .filter_map(|r| Self::asset_ref(db, &r.bundle_path, r.source_path_id))
            .collect::<Vec<_>>();
            Self::append_asset_refs_section(result, "Animator Users", &animators);
        }
    }

    fn append_animation_clip_preview(
        result: &mut AssetPreviewResult,
        sf: &crate::common::serialized_file::serialized_file::SerializedFile,
        object_info: &crate::common::serialized_file::serialized_file::ObjectInfo,
    ) {
        if let Some(value) = UnityClassParser::read_typetree_value_public(sf, object_info) {
            let rows = vec![
                row(
                    "Sample Rate",
                    Self::find_field(&value, "m_SampleRate")
                        .and_then(UnityValue::as_i64)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "?".to_string()),
                ),
                row(
                    "Wrap Mode",
                    Self::find_field(&value, "m_WrapMode")
                        .and_then(UnityValue::as_i64)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "?".to_string()),
                ),
                row("Curves", Self::count_named_arrays(&value, "Curve")),
                row(
                    "Events",
                    Self::array_len_field(&value, "m_Events").unwrap_or(0),
                ),
            ];
            result.sections.push(PreviewSection {
                title: "AnimationClip".to_string(),
                kind: "properties".to_string(),
                rows,
            });
        }
    }

    fn append_shader_preview(
        result: &mut AssetPreviewResult,
        sf: &crate::common::serialized_file::serialized_file::SerializedFile,
        object_info: &crate::common::serialized_file::serialized_file::ObjectInfo,
    ) {
        if let Some(value) = UnityClassParser::read_typetree_value_public(sf, object_info) {
            let decompressed_size = Self::find_field(&value, "decompressedSize")
                .and_then(UnityValue::as_i64)
                .unwrap_or(0);
            let keyword_count = Self::array_len_field(&value, "m_Keywords").unwrap_or(0);
            let property_count = Self::array_len_field(&value, "m_Props").unwrap_or(0);
            result.sections.push(PreviewSection {
                title: "Shader".to_string(),
                kind: "properties".to_string(),
                rows: vec![
                    row("Decompressed Size", decompressed_size),
                    row("Keywords", keyword_count),
                    row("Properties", property_count),
                ],
            });
        }
    }

    fn append_mono_behaviour_preview(
        result: &mut AssetPreviewResult,
        sf: &crate::common::serialized_file::serialized_file::SerializedFile,
        object_info: &crate::common::serialized_file::serialized_file::ObjectInfo,
    ) {
        if let Some(value) = UnityClassParser::read_typetree_value_public(sf, object_info) {
            let script = Self::find_field(&value, "m_Script")
                .and_then(Self::ptr_from_value)
                .map(format_pptr)
                .unwrap_or_else(|| "?".to_string());
            result.sections.push(PreviewSection {
                title: "MonoBehaviour".to_string(),
                kind: "properties".to_string(),
                rows: vec![
                    row("Script", script),
                    row("Fields", Self::object_field_count(&value)),
                ],
            });
            let mut fields = Vec::new();
            Self::collect_scalar_fields(&value, "", &mut fields, 80);
            if !fields.is_empty() {
                result.sections.push(PreviewSection {
                    title: "Fields".to_string(),
                    kind: "fields".to_string(),
                    rows: fields,
                });
            }
        }
    }

    fn append_sprite_mask_preview(
        result: &mut AssetPreviewResult,
        sf: &crate::common::serialized_file::serialized_file::SerializedFile,
        object_info: &crate::common::serialized_file::serialized_file::ObjectInfo,
        bundle_path: &str,
        db: Option<&AssetDatabase>,
    ) {
        match UnityClassParser::parse_sprite_mask(sf, object_info) {
            Ok(mask) => {
                let sprite_text = mask
                    .sprite
                    .map(format_pptr)
                    .unwrap_or_else(|| "None".to_string());
                result.sections.push(PreviewSection {
                    title: "SpriteMask".to_string(),
                    kind: "properties".to_string(),
                    rows: vec![
                        row("GameObject", format_pptr(mask.game_object)),
                        row("Sprite", sprite_text),
                        row(
                            "Alpha Cutoff",
                            mask.alpha_cutoff
                                .map(|v| format!("{:.3}", v))
                                .unwrap_or_else(|| "?".to_string()),
                        ),
                        row(
                            "Custom Range",
                            mask.is_custom_range_active
                                .map(|v| if v { "Yes" } else { "No" }.to_string())
                                .unwrap_or_else(|| "?".to_string()),
                        ),
                        row(
                            "Front Sorting",
                            format_sorting(mask.front_sorting_layer_id, mask.front_sorting_order),
                        ),
                        row(
                            "Back Sorting",
                            format_sorting(mask.back_sorting_layer_id, mask.back_sorting_order),
                        ),
                    ],
                });

                if let (Some(db), Some(sprite)) = (db, mask.sprite) {
                    let refs = Self::asset_ref(db, bundle_path, sprite.path_id)
                        .into_iter()
                        .collect::<Vec<_>>();
                    Self::append_asset_refs_section(result, "Mask Sprite", &refs);
                }
            }
            Err(error) => {
                result
                    .warnings
                    .push(format!("SpriteMask parser failed: {}", error));
                Self::append_typetree_summary(result, sf, object_info);
            }
        }
    }

    fn append_scene_object_preview(
        result: &mut AssetPreviewResult,
        sf: &crate::common::serialized_file::serialized_file::SerializedFile,
        object_info: &crate::common::serialized_file::serialized_file::ObjectInfo,
    ) {
        match UnityClassParser::parse_object(sf, object_info, &HashMap::new()) {
            Ok(UnityClassObject::GameObject(go)) => {
                result.sections.push(PreviewSection {
                    title: "GameObject".to_string(),
                    kind: "properties".to_string(),
                    rows: vec![row("Components", go.components.len())],
                });
            }
            Ok(UnityClassObject::Transform(t)) => {
                result.sections.push(PreviewSection {
                    title: "Transform".to_string(),
                    kind: "properties".to_string(),
                    rows: vec![
                        row("GameObject", format_pptr(t.game_object)),
                        row("Parent", format_pptr(t.father)),
                        row("Children", t.children.len()),
                        row("Position", format_vec3(t.local_position)),
                        row("Scale", format_vec3(t.local_scale)),
                    ],
                });
            }
            Ok(UnityClassObject::Renderer(r)) => {
                result.sections.push(PreviewSection {
                    title: "Renderer".to_string(),
                    kind: "properties".to_string(),
                    rows: vec![
                        row("GameObject", format_pptr(r.game_object)),
                        row(
                            "Mesh",
                            r.mesh.map(format_pptr).unwrap_or_else(|| "?".to_string()),
                        ),
                        row("Materials", r.materials.len()),
                    ],
                });
            }
            Ok(UnityClassObject::SkinnedMeshRenderer(r)) => {
                result.sections.push(PreviewSection {
                    title: "SkinnedMeshRenderer".to_string(),
                    kind: "properties".to_string(),
                    rows: vec![
                        row("GameObject", format_pptr(r.renderer.game_object)),
                        row(
                            "Mesh",
                            r.renderer
                                .mesh
                                .map(format_pptr)
                                .unwrap_or_else(|| "?".to_string()),
                        ),
                        row("Materials", r.renderer.materials.len()),
                        row("Bones", r.bones.len()),
                        row(
                            "Root Bone",
                            r.root_bone
                                .map(format_pptr)
                                .unwrap_or_else(|| "?".to_string()),
                        ),
                    ],
                });
            }
            Ok(UnityClassObject::MeshFilter(f)) => {
                result.sections.push(PreviewSection {
                    title: "MeshFilter".to_string(),
                    kind: "properties".to_string(),
                    rows: vec![
                        row("GameObject", format_pptr(f.component.game_object)),
                        row(
                            "Mesh",
                            f.mesh.map(format_pptr).unwrap_or_else(|| "?".to_string()),
                        ),
                    ],
                });
            }
            Ok(_) => Self::append_typetree_summary(result, sf, object_info),
            Err(error) => result
                .warnings
                .push(format!("Scene parser failed: {}", error)),
        }
    }

    fn append_typetree_summary(
        result: &mut AssetPreviewResult,
        sf: &crate::common::serialized_file::serialized_file::SerializedFile,
        object_info: &crate::common::serialized_file::serialized_file::ObjectInfo,
    ) {
        if let Some(value) = UnityClassParser::read_typetree_value_public(sf, object_info) {
            result.sections.push(PreviewSection {
                title: "TypeTree".to_string(),
                kind: "properties".to_string(),
                rows: vec![
                    row("Root Type", value.variant_name()),
                    row("Top Fields", Self::object_field_count(&value)),
                    row("Arrays", Self::count_arrays(&value)),
                    row("Pointers", Self::count_ptrs(&value)),
                ],
            });
        } else {
            result.sections.push(PreviewSection {
                title: "Raw".to_string(),
                kind: "properties".to_string(),
                rows: vec![
                    row("TypeTree", "Unavailable"),
                    row("Raw Bytes", result.byte_size),
                ],
            });
        }
    }

    fn append_asset_map_relations(
        result: &mut AssetPreviewResult,
        db: &AssetDatabase,
        bundle_path: &str,
        path_id: i64,
    ) {
        let mut relations = Vec::new();
        for relation_type in [
            "gameobject_component",
            "transform_gameobject",
            "transform_parent",
            UnityRelationKind::MESH_FILTER_GAMEOBJECT,
            UnityRelationKind::MESH_FILTER_MESH,
            UnityRelationKind::RENDERER_GAMEOBJECT,
            UnityRelationKind::RENDERER_MESH,
            UnityRelationKind::RENDERER_MATERIAL,
            UnityRelationKind::ANIMATOR_GAMEOBJECT,
            UnityRelationKind::ANIMATOR_AVATAR,
            UnityRelationKind::ANIMATOR_CONTROLLER,
            UnityRelationKind::CONTROLLER_ANIMATION_CLIP,
            UnityRelationKind::OVERRIDE_BASE_CONTROLLER,
            UnityRelationKind::OVERRIDE_CLIP_ORIGINAL,
            UnityRelationKind::OVERRIDE_CLIP_OVERRIDE,
            UnityRelationKind::ANIMATION_GAMEOBJECT,
            UnityRelationKind::ANIMATION_DEFAULT_CLIP,
            UnityRelationKind::ANIMATION_CLIPS,
            "skinned_renderer_bone",
            "skinned_renderer_root_bone",
            "monobehaviour_gameobject",
            "monobehaviour_script",
            UnityRelationKind::SPRITE_MASK_GAMEOBJECT,
            UnityRelationKind::SPRITE_MASK_SPRITE,
        ] {
            relations.extend(
                lookup_relations_by_source(db, relation_type, bundle_path, path_id)
                    .into_iter()
                    .map(|r| Self::preview_relation(db, &r, "out")),
            );
            relations.extend(
                lookup_relations_by_target(db, relation_type, bundle_path, path_id)
                    .into_iter()
                    .map(|r| Self::preview_relation(db, &r, "in")),
            );
        }
        let mut seen = HashSet::new();
        result.relations = relations
            .into_iter()
            .filter(|r| {
                seen.insert(format!(
                    "{}:{}:{}:{}",
                    r.direction, r.relation_type, r.bundle_path, r.path_id
                ))
            })
            .take(300)
            .collect();
    }

    fn collect_animator_model_refs(
        db: &AssetDatabase,
        bundle_path: &str,
        animator_path_id: i64,
        root_game_object_id: i64,
    ) -> AnimatorModelRefs {
        let mut refs = AnimatorModelRefs::default();
        if root_game_object_id == 0 {
            return refs;
        }

        let root_transform_ids = lookup_relations_by_target(
            db,
            "transform_gameobject",
            bundle_path,
            root_game_object_id,
        )
        .into_iter()
        .map(|r| r.source_path_id)
        .collect::<Vec<_>>();

        let mut transform_ids = HashSet::new();
        let mut queue: VecDeque<i64> = root_transform_ids.into_iter().collect();
        while let Some(transform_id) = queue.pop_front() {
            if !transform_ids.insert(transform_id) {
                continue;
            }
            for child in
                lookup_relations_by_target(db, "transform_parent", bundle_path, transform_id)
            {
                queue.push_back(child.source_path_id);
            }
        }

        let mut game_object_ids = HashSet::from([root_game_object_id]);
        for transform_id in transform_ids {
            for relation in
                lookup_relations_by_source(db, "transform_gameobject", bundle_path, transform_id)
            {
                game_object_ids.insert(relation.target_path_id);
            }
        }

        refs.game_objects = game_object_ids
            .iter()
            .filter_map(|id| Self::asset_ref(db, bundle_path, *id))
            .collect();

        let mut renderer_ids = HashSet::new();
        let mut mesh_ids = HashSet::new();
        let mut material_ids: HashSet<(String, i64)> = HashSet::new();
        for go_id in game_object_ids {
            for component in
                lookup_relations_by_source(db, "gameobject_component", bundle_path, go_id)
            {
                if component.target_path_id == animator_path_id {
                    continue;
                }
                let Some(asset) =
                    Self::asset_ref(db, &component.bundle_path, component.target_path_id)
                else {
                    continue;
                };
                match asset.class_name.as_str() {
                    "SkinnedMeshRenderer" | "MeshRenderer" | "Renderer" => {
                        renderer_ids.insert(component.target_path_id);
                    }
                    "MeshFilter" => {
                        for mesh in lookup_relations_by_source(
                            db,
                            UnityRelationKind::MESH_FILTER_MESH,
                            &component.bundle_path,
                            component.target_path_id,
                        ) {
                            mesh_ids.insert((
                                Self::relation_target_bundle(db, &mesh),
                                mesh.target_path_id,
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }

        for renderer_id in renderer_ids {
            if let Some(renderer) = Self::asset_ref(db, bundle_path, renderer_id) {
                refs.renderers.push(renderer);
            }
            for mesh in lookup_relations_by_source(
                db,
                UnityRelationKind::RENDERER_MESH,
                bundle_path,
                renderer_id,
            ) {
                mesh_ids.insert((Self::relation_target_bundle(db, &mesh), mesh.target_path_id));
            }
            for material in lookup_relations_by_source(
                db,
                UnityRelationKind::RENDERER_MATERIAL,
                bundle_path,
                renderer_id,
            ) {
                material_ids.insert((
                    Self::relation_target_bundle(db, &material),
                    material.target_path_id,
                ));
            }
        }

        refs.meshes = mesh_ids
            .iter()
            .filter_map(|(bundle, id)| Self::asset_ref(db, bundle, *id))
            .collect();
        refs.materials = material_ids
            .iter()
            .filter_map(|(bundle, id)| Self::asset_ref(db, bundle, *id))
            .collect();

        let progress = dummy_progress();
        let mut texture_ids = HashSet::new();
        for material in &refs.materials {
            if let Ok((_info, textures)) = AssetMapDependencyResolver::resolve_material_textures(
                db,
                &material.bundle_path,
                material.path_id,
                &progress,
            ) {
                for (path_id, bundle_path, _name) in textures {
                    texture_ids.insert((bundle_path, path_id));
                }
            }
        }
        refs.textures = texture_ids
            .iter()
            .filter_map(|(bundle, id)| Self::asset_ref(db, bundle, *id))
            .collect();
        refs
    }

    fn relation_target_bundle(db: &AssetDatabase, relation: &RelationRow) -> String {
        UnityExternalResolver::resolve_relation_row(db, relation)
    }

    fn preview_relation(
        db: &AssetDatabase,
        relation: &RelationRow,
        direction: &str,
    ) -> PreviewRelation {
        let (bundle_path, path_id) = if direction == "out" {
            (
                Self::relation_target_bundle(db, relation),
                relation.target_path_id,
            )
        } else {
            (relation.bundle_path.clone(), relation.source_path_id)
        };
        let asset = Self::asset_ref(db, &bundle_path, path_id);
        PreviewRelation {
            relation_type: relation.relation_type.clone(),
            field_path: relation.field_path.clone(),
            direction: direction.to_string(),
            bundle_path,
            path_id: path_id.to_string(),
            class_name: asset
                .as_ref()
                .map(|a| a.class_name.clone())
                .unwrap_or_default(),
            name: asset.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
        }
    }

    fn asset_ref(db: &AssetDatabase, bundle_path: &str, path_id: i64) -> Option<PreviewAssetRef> {
        let row = db
            .find_by_bundle_and_path_id(bundle_path, path_id)
            .ok()
            .flatten()?;
        Some(Self::row_to_ref(row))
    }

    fn row_to_ref(row: AssetRow) -> PreviewAssetRef {
        PreviewAssetRef {
            bundle_path: row.bundle_path,
            path_id: row.path_id,
            class_name: row.class_name,
            name: row.asset_name,
        }
    }

    fn append_asset_refs_section(
        result: &mut AssetPreviewResult,
        title: &str,
        refs: &[PreviewAssetRef],
    ) {
        if refs.is_empty() {
            return;
        }
        result.sections.push(PreviewSection {
            title: title.to_string(),
            kind: "relations".to_string(),
            rows: refs
                .iter()
                .take(120)
                .map(|item| {
                    row(
                        if item.name.is_empty() {
                            &item.class_name
                        } else {
                            &item.name
                        },
                        format!(
                            "{} | {} | {}",
                            item.class_name,
                            item.path_id,
                            short_path(&item.bundle_path)
                        ),
                    )
                })
                .collect(),
        });
    }

    fn find_object<'a>(
        bundle: &'a AssetBundle,
        path_id: i64,
    ) -> Option<(
        &'a crate::common::bundle_file::asset_bundle::SerializedFile,
        &'a crate::common::bundle_file::asset_bundle::ObjectInfo,
    )> {
        bundle.assets.iter().find_map(|sf| {
            sf.objects
                .iter()
                .find(|object| object.path_id == path_id)
                .map(|object| (sf, object))
        })
    }

    fn basic_result(
        sf: &crate::common::serialized_file::serialized_file::SerializedFile,
        object_info: &crate::common::serialized_file::serialized_file::ObjectInfo,
    ) -> BasicPreviewHeader {
        let class_id = object_info.class_id as i32;
        let class_name = crate::utils::class_name_utils::ClassNameUtils::get_class_name(class_id)
            .unwrap_or("Unknown")
            .to_string();
        BasicPreviewHeader {
            name: Self::object_name(sf, object_info)
                .unwrap_or_else(|| format!("{}_{}", class_name, object_info.path_id)),
            class_name,
            unity_version: sf.unity_version.clone(),
            byte_size: object_info.byte_size,
        }
    }

    fn object_name(
        sf: &crate::common::serialized_file::serialized_file::SerializedFile,
        object_info: &crate::common::serialized_file::serialized_file::ObjectInfo,
    ) -> Option<String> {
        UnityObjectNameUtils::display_name(sf, object_info)
            .ok()
            .flatten()
            .filter(|name| !name.is_empty())
    }

    fn find_field<'a>(value: &'a UnityValue, field_name: &str) -> Option<&'a UnityValue> {
        match value {
            UnityValue::Object(map) => map.get(field_name).or_else(|| {
                map.values()
                    .find_map(|child| Self::find_field(child, field_name))
            }),
            UnityValue::Array(items) => items
                .iter()
                .find_map(|child| Self::find_field(child, field_name)),
            _ => None,
        }
    }

    fn array_len_field(value: &UnityValue, field_name: &str) -> Option<usize> {
        match Self::find_field(value, field_name)? {
            UnityValue::Array(items) => Some(items.len()),
            _ => None,
        }
    }

    fn ptr_from_value(value: &UnityValue) -> Option<PPtr> {
        match value {
            UnityValue::Ptr { file_id, path_id } => Some(PPtr {
                file_id: *file_id,
                path_id: *path_id,
            }),
            UnityValue::Object(map) => {
                if let (Some(file_id), Some(path_id)) = (map.get("m_FileID"), map.get("m_PathID")) {
                    return Some(PPtr {
                        file_id: file_id.as_i64()? as i32,
                        path_id: path_id.as_i64()?,
                    });
                }
                None
            }
            _ => None,
        }
    }

    fn extract_tos_paths(value: &UnityValue, limit: usize) -> Vec<(String, String)> {
        let Some(UnityValue::Array(items)) = Self::find_field(value, "m_TOS") else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|item| match item {
                UnityValue::Object(map) => {
                    let key = map
                        .get("first")
                        .and_then(UnityValue::as_i64)
                        .map(|v| v.to_string())
                        .or_else(|| {
                            map.get("key")
                                .and_then(UnityValue::as_i64)
                                .map(|v| v.to_string())
                        })?;
                    let value = map
                        .get("second")
                        .and_then(UnityValue::as_str)
                        .or_else(|| map.get("value").and_then(UnityValue::as_str))?;
                    Some((key, value.to_string()))
                }
                _ => None,
            })
            .take(limit)
            .collect()
    }

    fn object_field_count(value: &UnityValue) -> usize {
        match value {
            UnityValue::Object(map) => map.len(),
            _ => 0,
        }
    }

    fn count_arrays(value: &UnityValue) -> usize {
        match value {
            UnityValue::Array(items) => 1 + items.iter().map(Self::count_arrays).sum::<usize>(),
            UnityValue::Object(map) => map.values().map(Self::count_arrays).sum(),
            _ => 0,
        }
    }

    fn count_ptrs(value: &UnityValue) -> usize {
        match value {
            UnityValue::Ptr { path_id, .. } => usize::from(*path_id != 0),
            UnityValue::Array(items) => items.iter().map(Self::count_ptrs).sum(),
            UnityValue::Object(map) => map.values().map(Self::count_ptrs).sum(),
            _ => 0,
        }
    }

    fn count_named_arrays(value: &UnityValue, needle: &str) -> usize {
        match value {
            UnityValue::Object(map) => map
                .iter()
                .map(|(key, child)| {
                    usize::from(key.contains(needle) && matches!(child, UnityValue::Array(_)))
                        + Self::count_named_arrays(child, needle)
                })
                .sum(),
            UnityValue::Array(items) => items
                .iter()
                .map(|item| Self::count_named_arrays(item, needle))
                .sum(),
            _ => 0,
        }
    }

    fn collect_scalar_fields(
        value: &UnityValue,
        prefix: &str,
        rows: &mut Vec<PreviewRow>,
        limit: usize,
    ) {
        if rows.len() >= limit {
            return;
        }
        match value {
            UnityValue::Object(map) => {
                for (key, child) in map {
                    let next = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{}.{}", prefix, key)
                    };
                    Self::collect_scalar_fields(child, &next, rows, limit);
                    if rows.len() >= limit {
                        return;
                    }
                }
            }
            UnityValue::Bool(v) => rows.push(row(prefix, v)),
            UnityValue::Integer(v) => rows.push(row(prefix, v)),
            UnityValue::Float(v) => rows.push(row(prefix, format!("{:.4}", v))),
            UnityValue::String(v) => {
                if !v.is_empty() {
                    rows.push(row(prefix, v.chars().take(160).collect::<String>()));
                }
            }
            UnityValue::Ptr { file_id, path_id } if *path_id != 0 => rows.push(row(
                prefix,
                format!("file_id={}, path_id={}", file_id, path_id),
            )),
            UnityValue::Array(items) => {
                if !items.is_empty() {
                    rows.push(row(prefix, format!("Array[{}]", items.len())));
                }
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct AnimatorModelRefs {
    game_objects: Vec<PreviewAssetRef>,
    renderers: Vec<PreviewAssetRef>,
    meshes: Vec<PreviewAssetRef>,
    materials: Vec<PreviewAssetRef>,
    textures: Vec<PreviewAssetRef>,
}

struct BasicPreviewHeader {
    name: String,
    class_name: String,
    unity_version: String,
    byte_size: u32,
}

fn row(label: impl ToString, value: impl ToString) -> PreviewRow {
    PreviewRow {
        label: label.to_string(),
        value: value.to_string(),
    }
}

fn format_pptr(pptr: PPtr) -> String {
    if pptr.is_null() {
        "None".to_string()
    } else {
        format!("file_id={}, path_id={}", pptr.file_id, pptr.path_id)
    }
}

fn format_color(color: [f32; 4]) -> String {
    format!(
        "{:.3}, {:.3}, {:.3}, {:.3}",
        color[0], color[1], color[2], color[3]
    )
}

fn format_vec3(value: [f32; 3]) -> String {
    format!("{:.3}, {:.3}, {:.3}", value[0], value[1], value[2])
}

fn format_sorting(layer_id: Option<i64>, order: Option<i64>) -> String {
    match (layer_id, order) {
        (Some(layer_id), Some(order)) => format!("layer={}, order={}", layer_id, order),
        (Some(layer_id), None) => format!("layer={}, order=?", layer_id),
        (None, Some(order)) => format!("layer=?, order={}", order),
        (None, None) => "?".to_string(),
    }
}

fn short_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_string()
}

fn dummy_progress() -> Channel<ProgressPayload> {
    tauri::ipc::Channel::<ProgressPayload>::new(|_| Ok(()))
}

fn lookup_relations_by_source(
    db: &AssetDatabase,
    relation_type: &str,
    bundle_path: &str,
    source_path_id: i64,
) -> Vec<RelationRow> {
    db.find_relations_by_bundle_and_source(relation_type, bundle_path, source_path_id)
        .unwrap_or_default()
}

fn lookup_relations_by_target(
    db: &AssetDatabase,
    relation_type: &str,
    bundle_path: &str,
    target_path_id: i64,
) -> Vec<RelationRow> {
    db.find_relations_by_bundle_and_target(relation_type, bundle_path, target_path_id)
        .unwrap_or_default()
}
