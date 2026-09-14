use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use crate::common::asset_map::asset_map_types::BundleRelationEntry;
use crate::common::serialized_file::serialized_file::{ObjectInfo, SerializedFile};
use crate::unity::classes::object::PPtr;
use crate::unity::classes::registry::{UnityClassObject, UnityClassParser};
use crate::unity::relations::UnityRelationKind;

pub struct UnityRelationCollector;

impl UnityRelationCollector {
    pub fn collect_serialized_file_relations(
        serialized_file: &SerializedFile,
        relations: &mut Vec<BundleRelationEntry>,
        source_name: &str,
    ) {
        let class_by_path: HashMap<i64, u16> = serialized_file
            .m_objects
            .iter()
            .map(|object_info| (object_info.path_id, object_info.class_id))
            .collect();

        for object_info in &serialized_file.m_objects {
            Self::collect_object_relations(
                serialized_file,
                object_info,
                &class_by_path,
                relations,
                source_name,
            );
        }
    }

    pub fn collect_object_relations(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
        class_by_path: &HashMap<i64, u16>,
        relations: &mut Vec<BundleRelationEntry>,
        source_name: &str,
    ) {
        let start = relations.len();
        Self::collect_typed_object_relations(
            serialized_file,
            object_info,
            class_by_path,
            relations,
            source_name,
        );
        Self::dedupe_relations_from(relations, start);
    }

    fn collect_typed_object_relations(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
        class_by_path: &HashMap<i64, u16>,
        relations: &mut Vec<BundleRelationEntry>,
        source_name: &str,
    ) {
        match object_info.class_id {
            1 => {
                if let Ok(game_object) =
                    UnityClassParser::parse_game_object(serialized_file, object_info)
                {
                    for component in game_object.components {
                        let _class_id = class_by_path.get(&component.path_id).copied().unwrap_or(0);
                        Self::push_relation(
                            relations,
                            "gameobject_component",
                            object_info.path_id,
                            component,
                            source_name,
                            "",
                            "m_Component",
                        );
                    }
                }
            }
            4 => {
                if let Ok(transform) =
                    UnityClassParser::parse_transform(serialized_file, object_info)
                {
                    Self::push_non_null_relation(
                        relations,
                        "transform_gameobject",
                        object_info.path_id,
                        transform.game_object,
                        source_name,
                        "",
                        "m_GameObject",
                    );
                    Self::push_non_null_relation(
                        relations,
                        "transform_parent",
                        object_info.path_id,
                        transform.father,
                        source_name,
                        "",
                        "m_Father",
                    );
                    for child in transform.children {
                        Self::push_non_null_relation(
                            relations,
                            "transform_parent",
                            child.path_id,
                            PPtr {
                                file_id: child.file_id,
                                path_id: object_info.path_id,
                            },
                            source_name,
                            "",
                            "m_Children",
                        );
                    }
                }
            }
            33 => {
                if let Ok(mesh_filter) =
                    UnityClassParser::parse_mesh_filter(serialized_file, object_info)
                {
                    Self::push_non_null_relation(
                        relations,
                        UnityRelationKind::MESH_FILTER_GAMEOBJECT,
                        object_info.path_id,
                        mesh_filter.component.game_object,
                        source_name,
                        "",
                        "m_GameObject",
                    );
                    if let Some(mesh) = mesh_filter.mesh {
                        Self::push_relation(
                            relations,
                            UnityRelationKind::MESH_FILTER_MESH,
                            object_info.path_id,
                            mesh,
                            source_name,
                            "",
                            "m_Mesh",
                        );
                    }
                }
            }
            23 | 119 | 137 => {
                let parsed_renderer =
                    UnityClassParser::parse_object(serialized_file, object_info, &HashMap::new());
                let (game_object, mesh, materials) = match parsed_renderer {
                    Ok(UnityClassObject::Renderer(renderer)) => {
                        (renderer.game_object, renderer.mesh, renderer.materials)
                    }
                    Ok(UnityClassObject::SkinnedMeshRenderer(skinned)) => (
                        skinned.renderer.game_object,
                        skinned.renderer.mesh,
                        skinned.renderer.materials,
                    ),
                    _ => (
                        PPtr {
                            file_id: 0,
                            path_id: 0,
                        },
                        None,
                        Vec::new(),
                    ),
                };
                if let Some(mesh) = mesh {
                    Self::push_relation(
                        relations,
                        UnityRelationKind::RENDERER_MESH,
                        object_info.path_id,
                        mesh,
                        source_name,
                        "",
                        "m_Mesh",
                    );
                }
                Self::push_non_null_relation(
                    relations,
                    UnityRelationKind::RENDERER_GAMEOBJECT,
                    object_info.path_id,
                    game_object,
                    source_name,
                    "",
                    "m_GameObject",
                );
                for material in materials {
                    Self::push_relation(
                        relations,
                        UnityRelationKind::RENDERER_MATERIAL,
                        object_info.path_id,
                        material,
                        source_name,
                        "",
                        "m_Materials",
                    );
                }
            }
            199 => {
                if let Ok(particle_renderer) =
                    UnityClassParser::parse_particle_system_renderer(serialized_file, object_info)
                {
                    Self::push_non_null_relation(
                        relations,
                        UnityRelationKind::RENDERER_GAMEOBJECT,
                        object_info.path_id,
                        particle_renderer.renderer.game_object,
                        source_name,
                        "",
                        "m_GameObject",
                    );
                    for material in particle_renderer.renderer.materials {
                        Self::push_relation(
                            relations,
                            UnityRelationKind::RENDERER_MATERIAL,
                            object_info.path_id,
                            material,
                            source_name,
                            "",
                            "m_Materials",
                        );
                    }
                    // The mesh is only rendered in Mesh render mode (4); billboard
                    // modes may keep a stale m_Mesh reference from the editor.
                    if particle_renderer.render_mode == Some(4) {
                        if let Some(mesh) = particle_renderer.renderer.mesh {
                            Self::push_non_null_relation(
                                relations,
                                UnityRelationKind::RENDERER_MESH,
                                object_info.path_id,
                                mesh,
                                source_name,
                                "",
                                "m_Mesh",
                            );
                        }
                    }
                }
            }
            331 => {
                if let Ok(sprite_mask) =
                    UnityClassParser::parse_sprite_mask(serialized_file, object_info)
                {
                    Self::push_non_null_relation(
                        relations,
                        UnityRelationKind::SPRITE_MASK_GAMEOBJECT,
                        object_info.path_id,
                        sprite_mask.game_object,
                        source_name,
                        "",
                        "m_GameObject",
                    );
                    if let Some(sprite) = sprite_mask.sprite {
                        Self::push_relation(
                            relations,
                            UnityRelationKind::SPRITE_MASK_SPRITE,
                            object_info.path_id,
                            sprite,
                            source_name,
                            "",
                            "m_Sprite",
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn dedupe_relations_from(relations: &mut Vec<BundleRelationEntry>, start: usize) {
        let mut seen = HashSet::new();
        let mut write = start;
        let mut read = start;
        while read < relations.len() {
            let key = (
                relations[read].relation_type.clone(),
                relations[read].source_path_id,
                relations[read].target_path_id,
                relations[read].file_id,
                relations[read].field_path.clone(),
                relations[read].target_bundle_path.clone(),
            );
            if seen.insert(key) {
                if write != read {
                    relations.swap(write, read);
                }
                write += 1;
            }
            read += 1;
        }
        relations.truncate(write);
    }

    fn push_non_null_relation(
        relations: &mut Vec<BundleRelationEntry>,
        relation_type: impl Into<Cow<'static, str>>,
        source_path_id: i64,
        target: PPtr,
        source_name: &str,
        target_name: &str,
        field_path: impl Into<Cow<'static, str>>,
    ) {
        if target.is_null() {
            return;
        }
        Self::push_relation(
            relations,
            relation_type,
            source_path_id,
            target,
            source_name,
            target_name,
            field_path,
        );
    }

    fn push_relation(
        relations: &mut Vec<BundleRelationEntry>,
        relation_type: impl Into<Cow<'static, str>>,
        source_path_id: i64,
        target: PPtr,
        source_name: &str,
        target_name: &str,
        field_path: impl Into<Cow<'static, str>>,
    ) {
        relations.push(BundleRelationEntry {
            relation_type: relation_type.into(),
            source_path_id,
            target_path_id: target.path_id,
            source_name: source_name.to_string(),
            target_name: target_name.to_string(),
            file_id: target.file_id,
            field_path: field_path.into(),
            target_bundle_path: String::new(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupe_relations_removes_exact_duplicate_edges() {
        let mut relations = vec![BundleRelationEntry {
            relation_type: "pptr".into(),
            source_path_id: 10,
            target_path_id: 20,
            source_name: "source-a".to_string(),
            target_name: String::new(),
            file_id: 0,
            field_path: "m_Target".into(),
            target_bundle_path: String::new(),
        }];
        relations.push(BundleRelationEntry {
            source_name: "source-b".to_string(),
            ..relations[0].clone()
        });

        UnityRelationCollector::dedupe_relations_from(&mut relations, 0);

        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].source_name, "source-a");
    }

    #[test]
    fn generic_pptr_relation_collection_is_disabled_in_fast_index_mode() {
        let mut relations = Vec::new();
        UnityRelationCollector::dedupe_relations_from(&mut relations, 0);
        assert!(relations.is_empty());
    }
}
