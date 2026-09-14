/*
 * glb_exporter.rs -- glTF 2.0 binary exporter for mesh dependency export.
 *
 * Produces a geometry-first GLB with PBR materials and sidecar texture URI references.
 * Texture image bytes stay outside the GLB so exported DDS files remain replaceable.
 */

use std::fs;
use std::path::Path;

use crate::exporter::animation_clip_exporter::{AnimationPath, GlbAnimation};
use crate::exporter::material_info::MaterialInfo;
use crate::exporter::mesh_exporter::MeshAttributes;
use crate::exporter::model_context::{GlbSkeleton, ModelContextResolver};
use crate::utils::format_utils::FormatUtils;

const GLB_MAGIC: u32 = 0x46546C67;
const GLB_VERSION: u32 = 2;
const CHUNK_TYPE_JSON: u32 = 0x4E4F534A;
const CHUNK_TYPE_BIN: u32 = 0x004E4942;
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;

pub struct GlbExporter;

pub struct GlbSceneMesh<'a> {
    pub name: &'a str,
    pub attrs: MeshAttributes<'a>,
    pub skeleton: Option<&'a GlbSkeleton>,
}

struct SkinData {
    joints: Vec<u16>,
    weights: Vec<f32>,
    inverse_bind_matrices: Vec<f32>,
    export_joints: Vec<usize>,
    export_local_matrices: Vec<[f32; 16]>,
    export_parents: Vec<Option<usize>>,
    joint_node_map: Vec<Option<usize>>,
    skeleton_root: Option<usize>,
    rest_pose_source: &'static str,
}

struct ExportSkinJointMap {
    export_joints: Vec<usize>,
    export_local_matrices: Vec<[f32; 16]>,
    export_parents: Vec<Option<usize>>,
    joint_node_map: Vec<Option<usize>>,
    source_to_export: Vec<u16>,
    inverse_bind_matrices: Vec<f32>,
    skeleton_root: Option<usize>,
}

struct MorphTargetData {
    name: String,
    positions: Vec<f32>,
    normals: Vec<f32>,
    tangents: Vec<f32>,
}

#[derive(Default)]
struct MaterialTextureSet {
    images: Vec<serde_json::Value>,
    textures: Vec<serde_json::Value>,
    texture_indices: std::collections::HashMap<String, usize>,
}

impl GlbExporter {
    #[allow(dead_code)]
    pub fn export_multi_skinned_scene_data(
        output_root: &Path,
        mesh_name: &str,
        meshes: &[GlbSceneMesh<'_>],
        materials: Option<&[MaterialInfo]>,
    ) -> Result<String, String> {
        Self::export_multi_skinned_scene_data_with_animations(
            output_root,
            mesh_name,
            meshes,
            materials,
            None,
        )
    }

    pub fn export_multi_skinned_scene_data_flat(
        output_folder: &Path,
        mesh_name: &str,
        meshes: &[GlbSceneMesh<'_>],
        materials: Option<&[MaterialInfo]>,
    ) -> Result<String, String> {
        Self::export_multi_skinned_scene_data_with_animation_sets_to_folder(
            output_folder,
            mesh_name,
            meshes,
            materials,
            None,
            None,
        )
    }

    pub fn export_multi_skinned_scene_data_with_per_mesh_animations_flat(
        output_folder: &Path,
        mesh_name: &str,
        meshes: &[GlbSceneMesh<'_>],
        materials: Option<&[MaterialInfo]>,
        animations_by_mesh: Option<&[Vec<GlbAnimation>]>,
    ) -> Result<String, String> {
        Self::export_multi_skinned_scene_data_with_animation_sets_to_folder(
            output_folder,
            mesh_name,
            meshes,
            materials,
            animations_by_mesh,
            None,
        )
    }

    #[allow(dead_code)]
    pub fn export_multi_skinned_scene_data_with_animations(
        output_root: &Path,
        mesh_name: &str,
        meshes: &[GlbSceneMesh<'_>],
        materials: Option<&[MaterialInfo]>,
        animations: Option<&[GlbAnimation]>,
    ) -> Result<String, String> {
        Self::export_multi_skinned_scene_data_with_animation_sets(
            output_root,
            mesh_name,
            meshes,
            materials,
            None,
            animations,
        )
    }

    #[allow(dead_code)]
    pub fn export_multi_skinned_scene_data_with_per_mesh_animations(
        output_root: &Path,
        mesh_name: &str,
        meshes: &[GlbSceneMesh<'_>],
        materials: Option<&[MaterialInfo]>,
        animations_by_mesh: Option<&[Vec<GlbAnimation>]>,
    ) -> Result<String, String> {
        Self::export_multi_skinned_scene_data_with_animation_sets(
            output_root,
            mesh_name,
            meshes,
            materials,
            animations_by_mesh,
            None,
        )
    }

    #[allow(dead_code)]
    fn export_multi_skinned_scene_data_with_animation_sets(
        output_root: &Path,
        mesh_name: &str,
        meshes: &[GlbSceneMesh<'_>],
        materials: Option<&[MaterialInfo]>,
        animations_by_mesh: Option<&[Vec<GlbAnimation>]>,
        animations: Option<&[GlbAnimation]>,
    ) -> Result<String, String> {
        let folder = output_root.join(mesh_name);
        fs::create_dir_all(&folder).map_err(|e| format!("create GLB folder: {}", e))?;
        let glb_path = folder.join(format!("{}.glb", mesh_name));
        let data = Self::build_multi_skinned_scene_data_with_animation_sets(
            mesh_name,
            meshes,
            materials,
            animations_by_mesh,
            animations,
        )?;
        fs::write(&glb_path, data).map_err(|e| format!("write GLB: {}", e))?;
        Ok(glb_path.to_string_lossy().to_string())
    }

    fn export_multi_skinned_scene_data_with_animation_sets_to_folder(
        output_folder: &Path,
        mesh_name: &str,
        meshes: &[GlbSceneMesh<'_>],
        materials: Option<&[MaterialInfo]>,
        animations_by_mesh: Option<&[Vec<GlbAnimation>]>,
        animations: Option<&[GlbAnimation]>,
    ) -> Result<String, String> {
        fs::create_dir_all(output_folder).map_err(|e| format!("create GLB folder: {}", e))?;
        let glb_path = output_folder.join(format!("{}.glb", mesh_name));
        let data = Self::build_multi_skinned_scene_data_with_animation_sets(
            mesh_name,
            meshes,
            materials,
            animations_by_mesh,
            animations,
        )?;
        fs::write(&glb_path, data).map_err(|e| format!("write GLB: {}", e))?;
        Ok(glb_path.to_string_lossy().to_string())
    }

    #[allow(dead_code)]
    pub fn build_multi_skinned_scene_data(
        _mesh_name: &str,
        meshes: &[GlbSceneMesh<'_>],
        materials: Option<&[MaterialInfo]>,
    ) -> Result<Vec<u8>, String> {
        Self::build_multi_skinned_scene_data_with_animations(_mesh_name, meshes, materials, None)
    }

    #[allow(dead_code)]
    pub fn build_multi_skinned_scene_data_with_animations(
        _mesh_name: &str,
        meshes: &[GlbSceneMesh<'_>],
        materials: Option<&[MaterialInfo]>,
        animations: Option<&[GlbAnimation]>,
    ) -> Result<Vec<u8>, String> {
        Self::build_multi_skinned_scene_data_with_animation_sets(
            _mesh_name, meshes, materials, None, animations,
        )
    }

    fn build_multi_skinned_scene_data_with_animation_sets(
        _mesh_name: &str,
        meshes: &[GlbSceneMesh<'_>],
        materials: Option<&[MaterialInfo]>,
        animations_by_mesh: Option<&[Vec<GlbAnimation>]>,
        animations: Option<&[GlbAnimation]>,
    ) -> Result<Vec<u8>, String> {
        if meshes.is_empty() {
            return Err("No meshes for multi-skinned GLB".to_string());
        }

        let material_texture_set = Self::build_material_texture_set(materials);
        let materials_json =
            Self::build_materials(materials, &material_texture_set.texture_indices);
        let mut bin = Vec::<u8>::new();
        let mut buffer_views = Vec::<serde_json::Value>::new();
        let mut accessors = Vec::<serde_json::Value>::new();
        let mut meshes_json = Vec::<serde_json::Value>::new();
        let mut nodes = Vec::<serde_json::Value>::new();
        let mut scene_nodes = Vec::<usize>::new();
        let mut skins_json = Vec::<serde_json::Value>::new();
        let mut animation_json = Vec::<serde_json::Value>::new();

        for (scene_mesh_index, scene_mesh) in meshes.iter().enumerate() {
            let attrs = &scene_mesh.attrs;
            let vertices = attrs.vertices;
            let indices = attrs.indices;
            let vertex_count = vertices.len() / 3;
            if vertices.len() < 3 || vertices.len() % 3 != 0 {
                return Err(format!(
                    "{} vertices: {} floats",
                    scene_mesh.name,
                    vertices.len()
                ));
            }

            let mesh_animations = animations_by_mesh
                .and_then(|sets| sets.get(scene_mesh_index).map(Vec::as_slice))
                .or(animations)
                .unwrap_or(&[]);
            let mut skin =
                Self::build_skin_data(attrs, vertex_count, scene_mesh.skeleton, mesh_animations);
            let mesh_bake_matrix = if skin.is_some() {
                scene_mesh
                    .skeleton
                    .and_then(Self::mesh_node_matrix_for_baking)
            } else {
                None
            };
            let positions =
                Self::bake_positions_if_needed(Self::flip_x(vertices, 3), mesh_bake_matrix);
            let converted_indices = Self::flip_triangles(indices);
            let position_accessor = Self::push_f32_accessor(
                &mut bin,
                &mut buffer_views,
                &mut accessors,
                &positions,
                vertex_count,
                "VEC3",
                ARRAY_BUFFER,
            );
            let index_accessor = Self::push_u32_accessor(
                &mut bin,
                &mut buffer_views,
                &mut accessors,
                &converted_indices,
                ELEMENT_ARRAY_BUFFER,
            );
            let primitive_ranges = Self::primitive_ranges(attrs, converted_indices.len());
            let mut split_index_accessors = Vec::with_capacity(primitive_ranges.len());
            for (start, count, _) in &primitive_ranges {
                split_index_accessors.push(Self::push_u32_accessor(
                    &mut bin,
                    &mut buffer_views,
                    &mut accessors,
                    &converted_indices[*start..(*start + *count)],
                    ELEMENT_ARRAY_BUFFER,
                ));
            }
            let normals = attrs
                .normals
                .filter(|n| n.len() >= vertices.len())
                .map(|n| Self::bake_normals_if_needed(Self::flip_x(n, 3), mesh_bake_matrix));
            let normal_accessor = normals.as_ref().map(|data| {
                Self::push_f32_accessor(
                    &mut bin,
                    &mut buffer_views,
                    &mut accessors,
                    data,
                    vertex_count,
                    "VEC3",
                    ARRAY_BUFFER,
                )
            });
            let uvs = attrs
                .uvs
                .filter(|u| u.len() >= vertex_count * 2)
                .map(|u| Self::normalize_uv0_attribute(u, vertex_count));
            let uv_accessor = uvs.as_ref().map(|data| {
                Self::push_f32_accessor(
                    &mut bin,
                    &mut buffer_views,
                    &mut accessors,
                    data,
                    vertex_count,
                    "VEC2",
                    ARRAY_BUFFER,
                )
            });
            let tangents = attrs
                .tangents
                .filter(|t| t.len() >= vertex_count * 4)
                .map(|t| {
                    let tangents = Self::normalize_vertex_f32_attribute(t, vertex_count, 4);
                    Self::bake_tangents_if_needed(Self::flip_x(&tangents, 4), mesh_bake_matrix)
                });
            let tangent_accessor = tangents.as_ref().map(|data| {
                Self::push_f32_accessor(
                    &mut bin,
                    &mut buffer_views,
                    &mut accessors,
                    data,
                    vertex_count,
                    "VEC4",
                    ARRAY_BUFFER,
                )
            });
            let colors = attrs
                .colors
                .filter(|c| !Self::has_base_color_texture(materials) && c.len() >= vertex_count * 4)
                .map(|c| Self::normalize_vertex_f32_attribute(c, vertex_count, 4));
            let color_accessor = colors.as_ref().map(|data| {
                Self::push_f32_accessor(
                    &mut bin,
                    &mut buffer_views,
                    &mut accessors,
                    data,
                    vertex_count,
                    "VEC4",
                    ARRAY_BUFFER,
                )
            });
            if let (Some(data), Some(matrix)) = (skin.as_mut(), mesh_bake_matrix) {
                Self::remove_baked_mesh_matrix_from_inverse_binds(data, &matrix);
            }
            let joint_accessor = skin.as_ref().map(|data| {
                Self::push_u16_accessor(
                    &mut bin,
                    &mut buffer_views,
                    &mut accessors,
                    &data.joints,
                    vertex_count,
                    "VEC4",
                    ARRAY_BUFFER,
                )
            });
            let weight_accessor = skin.as_ref().map(|data| {
                Self::push_f32_accessor(
                    &mut bin,
                    &mut buffer_views,
                    &mut accessors,
                    &data.weights,
                    vertex_count,
                    "VEC4",
                    ARRAY_BUFFER,
                )
            });

            let mut primitive_attrs = serde_json::Map::new();
            primitive_attrs.insert("POSITION".to_string(), serde_json::json!(position_accessor));
            if let Some(accessor) = normal_accessor {
                primitive_attrs.insert("NORMAL".to_string(), serde_json::json!(accessor));
            }
            if let Some(accessor) = uv_accessor {
                primitive_attrs.insert("TEXCOORD_0".to_string(), serde_json::json!(accessor));
            }
            if let Some(accessor) = tangent_accessor {
                primitive_attrs.insert("TANGENT".to_string(), serde_json::json!(accessor));
            }
            if let Some(accessor) = color_accessor {
                primitive_attrs.insert("COLOR_0".to_string(), serde_json::json!(accessor));
            }
            if let Some(accessor) = joint_accessor {
                primitive_attrs.insert("JOINTS_0".to_string(), serde_json::json!(accessor));
            }
            if let Some(accessor) = weight_accessor {
                primitive_attrs.insert("WEIGHTS_0".to_string(), serde_json::json!(accessor));
            }

            let mut primitives = if primitive_ranges.is_empty() {
                vec![serde_json::json!({
                    "attributes": primitive_attrs.clone(),
                    "indices": index_accessor,
                    "mode": 4
                })]
            } else {
                primitive_ranges
                    .iter()
                    .enumerate()
                    .map(|(primitive_index, (_, _, topology))| {
                        serde_json::json!({
                            "attributes": primitive_attrs.clone(),
                            "indices": split_index_accessors[primitive_index],
                            "mode": Self::gltf_mode(*topology)
                        })
                    })
                    .collect::<Vec<_>>()
            };
            if !materials_json.is_empty() {
                let primitive_count = primitives.len();
                for (primitive_index, primitive) in primitives.iter_mut().enumerate() {
                    let material_index = Self::material_index_for_primitive(
                        primitive_index,
                        primitive_count,
                        materials,
                        materials_json.len(),
                    );
                    primitive["material"] = serde_json::json!(material_index);
                }
            }
            let gltf_mesh_index = meshes_json.len();
            let mut mesh_json = serde_json::json!({
                "name": scene_mesh.name,
                "primitives": primitives
            });
            let morph_targets = Self::build_morph_targets(attrs, vertex_count);
            if !morph_targets.is_empty() {
                mesh_json["weights"] =
                    serde_json::json!(Self::zero_morph_weights(morph_targets.len()));
                mesh_json["extras"] = serde_json::json!({
                    "targetNames": morph_targets.iter().map(|target| target.name.clone()).collect::<Vec<_>>()
                });
            }
            meshes_json.push(mesh_json);

            let mesh_node_index = nodes.len();
            let mesh_parent = if mesh_bake_matrix.is_some() {
                None
            } else {
                scene_mesh.skeleton.and_then(|skeleton| {
                    skeleton
                        .mesh_parent
                        .and_then(|parent| skeleton.joints.get(parent))
                })
            };
            let mut mesh_node = if let Some(parent) = mesh_parent {
                serde_json::json!({
                    "name": scene_mesh.name,
                    "mesh": gltf_mesh_index,
                    "translation": [-parent.translation[0], parent.translation[1], parent.translation[2]],
                    "rotation": [parent.rotation[0], -parent.rotation[1], -parent.rotation[2], parent.rotation[3]],
                    "scale": parent.scale,
                    "extras": {
                        "unityRendererTransformPathId": parent.transform_path_id,
                        "unityRendererTransformPath": parent.path
                    }
                })
            } else {
                serde_json::json!({
                    "name": scene_mesh.name,
                    "mesh": gltf_mesh_index
                })
            };
            scene_nodes.push(mesh_node_index);
            nodes.push(serde_json::Value::Null);

            if let (Some(skin), Some(skeleton)) = (skin.as_ref(), scene_mesh.skeleton) {
                let joint_node_offset = nodes.len();
                for (export_index, joint_index) in skin.export_joints.iter().copied().enumerate() {
                    let joint = &skeleton.joints[joint_index];
                    let children =
                        Self::export_joint_children(skin, export_index, joint_node_offset);
                    let mut node = Self::joint_node_json(
                        joint,
                        Some(skin.export_local_matrices[export_index]),
                        skin.rest_pose_source,
                    );
                    if !children.is_empty() {
                        node["children"] = serde_json::json!(children);
                    }
                    nodes.push(node);
                }
                let joints = (0..skin.export_joints.len())
                    .map(|joint| joint_node_offset + joint)
                    .collect::<Vec<_>>();
                let inverse_bind_accessor = Self::push_f32_accessor_no_minmax(
                    &mut bin,
                    &mut buffer_views,
                    &mut accessors,
                    &skin.inverse_bind_matrices,
                    skin.export_joints.len(),
                    "MAT4",
                    ARRAY_BUFFER,
                );
                let skeleton_root = skin
                    .skeleton_root
                    .filter(|root| *root < skin.export_joints.len())
                    .map(|root| joint_node_offset + root)
                    .unwrap_or(joint_node_offset);
                let skin_index = skins_json.len();
                skins_json.push(serde_json::json!({
                    "joints": joints,
                    "skeleton": skeleton_root,
                    "inverseBindMatrices": inverse_bind_accessor
                }));
                mesh_node["skin"] = serde_json::json!(skin_index);
                for (export_index, parent) in skin.export_parents.iter().enumerate() {
                    if parent.is_none() {
                        scene_nodes.push(joint_node_offset + export_index);
                    }
                }
                let mesh_animation_json = Self::build_animations(
                    mesh_animations,
                    &mut bin,
                    &mut buffer_views,
                    &mut accessors,
                    nodes.len(),
                    Some((
                        joint_node_offset,
                        skin.joint_node_map.as_slice(),
                        skin.skeleton_root,
                    )),
                );
                Self::append_animation_json(&mut animation_json, mesh_animation_json);
            }
            nodes[mesh_node_index] = mesh_node;
        }

        Self::align_bin(&mut bin);
        let bin_len = bin.len();
        let mut root = serde_json::json!({
            "asset": { "version": "2.0", "generator": "AssetDaemon" },
            "scene": 0,
            "scenes": [{ "nodes": scene_nodes }],
            "nodes": nodes,
            "meshes": meshes_json,
            "accessors": accessors,
            "bufferViews": buffer_views,
            "buffers": [{ "byteLength": bin_len }]
        });
        if !materials_json.is_empty() {
            root["materials"] = serde_json::json!(materials_json);
        }
        if !material_texture_set.images.is_empty() {
            root["images"] = serde_json::json!(material_texture_set.images);
            root["textures"] = serde_json::json!(material_texture_set.textures);
            root["samplers"] = serde_json::json!([{
                "magFilter": 9729,
                "minFilter": 9987,
                "wrapS": 10497,
                "wrapT": 10497
            }]);
        }
        if !skins_json.is_empty() {
            root["skins"] = serde_json::json!(skins_json);
        }
        if !animation_json.is_empty() {
            root["animations"] = serde_json::json!(animation_json);
        }

        Self::write_glb(root, bin)
    }

    #[allow(dead_code)]
    pub fn export_with_split_submeshes_scene_data(
        output_root: &Path,
        mesh_name: &str,
        attrs: &MeshAttributes,
        materials: Option<&[MaterialInfo]>,
        skeleton: Option<&GlbSkeleton>,
        animations: Option<&[GlbAnimation]>,
    ) -> Result<String, String> {
        let folder = output_root.join(mesh_name);
        fs::create_dir_all(&folder).map_err(|e| format!("create GLB folder: {}", e))?;
        let glb_path = folder.join(format!("{}.glb", mesh_name));
        let data = Self::build_with_split_submeshes_scene_data(
            mesh_name, attrs, materials, skeleton, animations,
        )?;
        fs::write(&glb_path, data).map_err(|e| format!("write GLB: {}", e))?;
        Ok(glb_path.to_string_lossy().to_string())
    }

    #[allow(dead_code)]
    pub fn build_with_split_submeshes_scene_data(
        mesh_name: &str,
        attrs: &MeshAttributes,
        materials: Option<&[MaterialInfo]>,
        skeleton: Option<&GlbSkeleton>,
        animations: Option<&[GlbAnimation]>,
    ) -> Result<Vec<u8>, String> {
        Self::build_scene_data(mesh_name, attrs, materials, skeleton, animations, true)
    }

    pub fn export_with_scene_data(
        output_root: &Path,
        mesh_name: &str,
        attrs: &MeshAttributes,
        materials: Option<&[MaterialInfo]>,
        skeleton: Option<&GlbSkeleton>,
        animations: Option<&[GlbAnimation]>,
    ) -> Result<String, String> {
        let folder = output_root.join(mesh_name);
        fs::create_dir_all(&folder).map_err(|e| format!("create GLB folder: {}", e))?;
        let glb_path = folder.join(format!("{}.glb", mesh_name));
        let data = Self::build_with_scene_data(mesh_name, attrs, materials, skeleton, animations)?;
        fs::write(&glb_path, data).map_err(|e| format!("write GLB: {}", e))?;
        Ok(glb_path.to_string_lossy().to_string())
    }

    pub fn build_with_scene_data(
        mesh_name: &str,
        attrs: &MeshAttributes,
        materials: Option<&[MaterialInfo]>,
        skeleton: Option<&GlbSkeleton>,
        animations: Option<&[GlbAnimation]>,
    ) -> Result<Vec<u8>, String> {
        Self::build_scene_data(mesh_name, attrs, materials, skeleton, animations, false)
    }

    fn build_scene_data(
        mesh_name: &str,
        attrs: &MeshAttributes,
        materials: Option<&[MaterialInfo]>,
        skeleton: Option<&GlbSkeleton>,
        animations: Option<&[GlbAnimation]>,
        split_submeshes_to_nodes: bool,
    ) -> Result<Vec<u8>, String> {
        let vertices = attrs.vertices;
        let indices = attrs.indices;
        let vertex_count = vertices.len() / 3;
        if vertices.len() < 3 || vertices.len() % 3 != 0 {
            return Err(format!("Vertices: {} floats", vertices.len()));
        }

        let mut skin =
            Self::build_skin_data(attrs, vertex_count, skeleton, animations.unwrap_or(&[]));
        let mesh_bake_matrix = if skin.is_some() {
            skeleton.and_then(Self::mesh_node_matrix_for_baking)
        } else {
            None
        };
        let positions = Self::bake_positions_if_needed(Self::flip_x(vertices, 3), mesh_bake_matrix);
        let normals = attrs
            .normals
            .filter(|n| n.len() >= vertices.len())
            .map(|n| Self::bake_normals_if_needed(Self::flip_x(n, 3), mesh_bake_matrix));
        let uvs = attrs
            .uvs
            .filter(|u| u.len() >= vertex_count * 2)
            .map(|u| Self::normalize_uv0_attribute(u, vertex_count));
        let tangents = attrs
            .tangents
            .filter(|t| t.len() >= vertex_count * 4)
            .map(|t| {
                let tangents = Self::normalize_vertex_f32_attribute(t, vertex_count, 4);
                Self::bake_tangents_if_needed(Self::flip_x(&tangents, 4), mesh_bake_matrix)
            });
        let should_export_vertex_colors = !Self::has_base_color_texture(materials);
        let colors = attrs
            .colors
            .filter(|c| should_export_vertex_colors && c.len() >= vertex_count * 4)
            .map(|c| Self::normalize_vertex_f32_attribute(c, vertex_count, 4));
        if let (Some(data), Some(matrix)) = (skin.as_mut(), mesh_bake_matrix) {
            Self::remove_baked_mesh_matrix_from_inverse_binds(data, &matrix);
        }
        let converted_indices = Self::flip_triangles(indices);

        let mut bin = Vec::<u8>::new();
        let mut buffer_views = Vec::<serde_json::Value>::new();
        let mut accessors = Vec::<serde_json::Value>::new();

        let position_accessor = Self::push_f32_accessor(
            &mut bin,
            &mut buffer_views,
            &mut accessors,
            &positions,
            vertex_count,
            "VEC3",
            ARRAY_BUFFER,
        );
        let primitive_ranges = Self::primitive_ranges(attrs, converted_indices.len());
        let mut index_accessors = Vec::with_capacity(primitive_ranges.len());
        for (start, count, _) in &primitive_ranges {
            index_accessors.push(Self::push_u32_accessor(
                &mut bin,
                &mut buffer_views,
                &mut accessors,
                &converted_indices[*start..(*start + *count)],
                ELEMENT_ARRAY_BUFFER,
            ));
        }

        let normal_accessor = normals.as_ref().map(|data| {
            Self::push_f32_accessor(
                &mut bin,
                &mut buffer_views,
                &mut accessors,
                data,
                vertex_count,
                "VEC3",
                ARRAY_BUFFER,
            )
        });
        let uv_accessor = uvs.as_ref().map(|data| {
            Self::push_f32_accessor(
                &mut bin,
                &mut buffer_views,
                &mut accessors,
                data,
                vertex_count,
                "VEC2",
                ARRAY_BUFFER,
            )
        });
        let tangent_accessor = tangents.as_ref().map(|data| {
            Self::push_f32_accessor(
                &mut bin,
                &mut buffer_views,
                &mut accessors,
                data,
                vertex_count,
                "VEC4",
                ARRAY_BUFFER,
            )
        });
        let color_accessor = colors.as_ref().map(|data| {
            Self::push_f32_accessor(
                &mut bin,
                &mut buffer_views,
                &mut accessors,
                data,
                vertex_count,
                "VEC4",
                ARRAY_BUFFER,
            )
        });
        let joint_accessor = skin.as_ref().map(|data| {
            Self::push_u16_accessor(
                &mut bin,
                &mut buffer_views,
                &mut accessors,
                &data.joints,
                vertex_count,
                "VEC4",
                ARRAY_BUFFER,
            )
        });
        let weight_accessor = skin.as_ref().map(|data| {
            Self::push_f32_accessor(
                &mut bin,
                &mut buffer_views,
                &mut accessors,
                &data.weights,
                vertex_count,
                "VEC4",
                ARRAY_BUFFER,
            )
        });
        let inverse_bind_accessor = skin.as_ref().map(|data| {
            Self::push_f32_accessor_no_minmax(
                &mut bin,
                &mut buffer_views,
                &mut accessors,
                &data.inverse_bind_matrices,
                data.export_joints.len(),
                "MAT4",
                ARRAY_BUFFER,
            )
        });
        let morph_targets = Self::build_morph_targets(attrs, vertex_count);
        let morph_target_accessors: Vec<serde_json::Value> = morph_targets
            .iter()
            .map(|target| {
                let position = Self::push_f32_accessor_no_minmax(
                    &mut bin,
                    &mut buffer_views,
                    &mut accessors,
                    &Self::bake_vectors_if_needed(
                        Self::flip_x(&target.positions, 3),
                        mesh_bake_matrix,
                    ),
                    vertex_count,
                    "VEC3",
                    ARRAY_BUFFER,
                );
                let normal = if target.normals.len() >= vertex_count * 3 {
                    Some(Self::push_f32_accessor_no_minmax(
                        &mut bin,
                        &mut buffer_views,
                        &mut accessors,
                        &Self::bake_normals_if_needed(
                            Self::flip_x(&target.normals, 3),
                            mesh_bake_matrix,
                        ),
                        vertex_count,
                        "VEC3",
                        ARRAY_BUFFER,
                    ))
                } else {
                    None
                };
                let tangent = if target.tangents.len() >= vertex_count * 3 {
                    Some(Self::push_f32_accessor_no_minmax(
                        &mut bin,
                        &mut buffer_views,
                        &mut accessors,
                        &Self::bake_vectors_if_needed(
                            Self::flip_x(&target.tangents, 3),
                            mesh_bake_matrix,
                        ),
                        vertex_count,
                        "VEC3",
                        ARRAY_BUFFER,
                    ))
                } else {
                    None
                };
                let mut map = serde_json::Map::new();
                map.insert("POSITION".to_string(), serde_json::json!(position));
                if let Some(accessor) = normal {
                    map.insert("NORMAL".to_string(), serde_json::json!(accessor));
                }
                if let Some(accessor) = tangent {
                    map.insert("TANGENT".to_string(), serde_json::json!(accessor));
                }
                serde_json::Value::Object(map)
            })
            .collect();

        let material_texture_set = Self::build_material_texture_set(materials);
        let materials_json =
            Self::build_materials(materials, &material_texture_set.texture_indices);
        let mut attributes = serde_json::Map::new();
        attributes.insert("POSITION".to_string(), serde_json::json!(position_accessor));
        if let Some(accessor) = normal_accessor {
            attributes.insert("NORMAL".to_string(), serde_json::json!(accessor));
        }
        if let Some(accessor) = uv_accessor {
            attributes.insert("TEXCOORD_0".to_string(), serde_json::json!(accessor));
        }
        if let Some(accessor) = tangent_accessor {
            attributes.insert("TANGENT".to_string(), serde_json::json!(accessor));
        }
        if let Some(accessor) = color_accessor {
            attributes.insert("COLOR_0".to_string(), serde_json::json!(accessor));
        }
        if let Some(accessor) = joint_accessor {
            attributes.insert("JOINTS_0".to_string(), serde_json::json!(accessor));
        }
        if let Some(accessor) = weight_accessor {
            attributes.insert("WEIGHTS_0".to_string(), serde_json::json!(accessor));
        }

        let primitives: Vec<serde_json::Value> = primitive_ranges
            .iter()
            .enumerate()
            .map(|(i, (_, _, topology))| {
                let mut primitive = serde_json::json!({
                    "attributes": attributes.clone(),
                    "indices": index_accessors[i],
                    "mode": Self::gltf_mode(*topology)
                });
                if !materials_json.is_empty() {
                    primitive["material"] = serde_json::json!(Self::material_index_for_primitive(
                        i,
                        primitive_ranges.len(),
                        materials,
                        materials_json.len(),
                    ));
                }
                if !morph_target_accessors.is_empty() {
                    primitive["targets"] = serde_json::json!(morph_target_accessors);
                }
                primitive
            })
            .collect();

        let mesh_count = if split_submeshes_to_nodes {
            primitives.len().max(1)
        } else {
            1
        };
        let mesh_node_count = mesh_count;

        let (nodes, scene_nodes, skins_json) = if let Some(skin) = skin.as_ref() {
            let Some(skeleton) = skeleton else {
                return Err("Internal skin state missing resolved skeleton".to_string());
            };
            let first_joint_node = mesh_node_count;
            let joints = (0..skin.export_joints.len())
                .map(|joint| first_joint_node + joint)
                .collect::<Vec<_>>();
            let mesh_parent = if mesh_bake_matrix.is_some() {
                None
            } else {
                skeleton
                    .mesh_parent
                    .and_then(|parent| skeleton.joints.get(parent))
            };
            let mut nodes = Vec::with_capacity(mesh_node_count + skin.export_joints.len());
            for mesh_index in 0..mesh_node_count {
                let node_name = if split_submeshes_to_nodes {
                    format!("{}_submesh_{}", mesh_name, mesh_index)
                } else {
                    mesh_name.to_string()
                };
                let mesh_node_json = if let Some(parent) = mesh_parent {
                    serde_json::json!({
                        "name": node_name,
                        "mesh": mesh_index,
                        "skin": 0,
                        "translation": [-parent.translation[0], parent.translation[1], parent.translation[2]],
                        "rotation": [parent.rotation[0], -parent.rotation[1], -parent.rotation[2], parent.rotation[3]],
                        "scale": parent.scale,
                        "extras": {
                            "unityRendererTransformPathId": parent.transform_path_id,
                            "unityRendererTransformPath": parent.path
                        }
                    })
                } else {
                    serde_json::json!({ "name": node_name, "mesh": mesh_index, "skin": 0 })
                };
                nodes.push(mesh_node_json);
            }
            for (export_index, joint_index) in skin.export_joints.iter().copied().enumerate() {
                let joint = &skeleton.joints[joint_index];
                let children = Self::export_joint_children(skin, export_index, first_joint_node);
                let mut node = Self::joint_node_json(
                    joint,
                    Some(skin.export_local_matrices[export_index]),
                    skin.rest_pose_source,
                );
                if !children.is_empty() {
                    node["children"] = serde_json::json!(children);
                }
                nodes.push(node);
            }
            let skeleton_root = skin
                .skeleton_root
                .filter(|root| *root < skin.export_joints.len())
                .map(|root| first_joint_node + root)
                .unwrap_or(first_joint_node);
            let mut scene_nodes = (0..mesh_node_count).collect::<Vec<_>>();
            for (export_index, parent) in skin.export_parents.iter().enumerate() {
                if parent.is_none() {
                    let node_index = first_joint_node + export_index;
                    if !scene_nodes.contains(&node_index) {
                        scene_nodes.push(node_index);
                    }
                }
            }
            let mut skin_json = serde_json::json!({
                "joints": joints,
                "skeleton": skeleton_root
            });
            if let Some(accessor) = inverse_bind_accessor {
                skin_json["inverseBindMatrices"] = serde_json::json!(accessor);
            }
            if let Some(root_hash) = attrs.root_bone_name_hash {
                skin_json["extras"] = serde_json::json!({
                    "unityRootBoneNameHash": root_hash
                });
            }
            (nodes, scene_nodes, vec![skin_json])
        } else {
            let nodes = (0..mesh_node_count)
                .map(|mesh_index| {
                    let node_name = if split_submeshes_to_nodes {
                        format!("{}_submesh_{}", mesh_name, mesh_index)
                    } else {
                        mesh_name.to_string()
                    };
                    serde_json::json!({ "name": node_name, "mesh": mesh_index })
                })
                .collect::<Vec<_>>();
            (nodes, (0..mesh_node_count).collect::<Vec<_>>(), Vec::new())
        };

        let meshes_json = if split_submeshes_to_nodes {
            primitives
                .iter()
                .enumerate()
                .map(|(index, primitive)| {
                    let mut mesh_json = serde_json::json!({
                        "name": format!("{}_submesh_{}", mesh_name, index),
                        "primitives": [primitive.clone()]
                    });
                    if !morph_targets.is_empty() {
                        mesh_json["weights"] = serde_json::json!(Self::zero_morph_weights(morph_targets.len()));
                        mesh_json["extras"] = serde_json::json!({
                            "targetNames": morph_targets.iter().map(|target| target.name.clone()).collect::<Vec<_>>()
                        });
                    }
                    mesh_json
                })
                .collect::<Vec<_>>()
        } else {
            let mut mesh_json = serde_json::json!({ "name": mesh_name, "primitives": primitives });
            if !morph_targets.is_empty() {
                mesh_json["weights"] =
                    serde_json::json!(Self::zero_morph_weights(morph_targets.len()));
                mesh_json["extras"] = serde_json::json!({
                    "targetNames": morph_targets.iter().map(|target| target.name.clone()).collect::<Vec<_>>()
                });
            }
            vec![mesh_json]
        };

        let animations_json = Self::build_animations(
            animations.unwrap_or(&[]),
            &mut bin,
            &mut buffer_views,
            &mut accessors,
            nodes.len(),
            skin.as_ref().map(|skin| {
                (
                    mesh_node_count,
                    skin.joint_node_map.as_slice(),
                    skin.skeleton_root,
                )
            }),
        );

        Self::align_bin(&mut bin);
        let bin_len = bin.len();

        let mut root = serde_json::json!({
            "asset": { "version": "2.0", "generator": "AssetDaemon" },
            "scene": 0,
            "scenes": [{ "nodes": scene_nodes }],
            "nodes": nodes,
            "meshes": meshes_json,
            "accessors": accessors,
            "bufferViews": buffer_views,
            "buffers": [{ "byteLength": bin_len }]
        });
        if !materials_json.is_empty() {
            root["materials"] = serde_json::json!(materials_json);
        }
        if !material_texture_set.images.is_empty() {
            root["images"] = serde_json::json!(material_texture_set.images);
            root["textures"] = serde_json::json!(material_texture_set.textures);
            root["samplers"] = serde_json::json!([{
                "magFilter": 9729,
                "minFilter": 9987,
                "wrapS": 10497,
                "wrapT": 10497
            }]);
        }
        if !skins_json.is_empty() {
            root["skins"] = serde_json::json!(skins_json);
        }
        if !animations_json.is_empty() {
            root["animations"] = serde_json::json!(animations_json);
        }

        Self::write_glb(root, bin)
    }

    fn build_material_texture_set(materials: Option<&[MaterialInfo]>) -> MaterialTextureSet {
        let mut texture_set = MaterialTextureSet::default();
        let Some(materials) = materials else {
            return texture_set;
        };

        for material in materials {
            for slot in &material.textures {
                if slot.relative_path.is_empty() || slot.relative_path.starts_with('#') {
                    continue;
                }
                let image_index = texture_set.images.len();
                let texture_index = texture_set.textures.len();
                texture_set.images.push(serde_json::json!({
                    "uri": Self::uri_encode_path(&slot.relative_path),
                    "extras": {
                        "unitySlotName": slot.slot_name,
                        "unityUsage": slot.usage,
                        "sourceFileName": slot.file_name
                    }
                }));
                texture_set.textures.push(serde_json::json!({
                    "sampler": 0,
                    "source": image_index
                }));
                texture_set.texture_indices.insert(
                    Self::material_texture_key(&material.name, &slot.slot_name),
                    texture_index,
                );
            }
        }

        texture_set
    }

    fn build_materials(
        materials: Option<&[MaterialInfo]>,
        texture_indices: &std::collections::HashMap<String, usize>,
    ) -> Vec<serde_json::Value> {
        let Some(materials) = materials else {
            return Vec::new();
        };
        materials
            .iter()
            .map(|mat| {
                let base_color_texture = Self::find_material_texture_index(
                    texture_indices,
                    &mat.name,
                    Self::base_color_slot_names(),
                );
                let base_color_factor = if base_color_texture.is_some() {
                    [1.0, 1.0, 1.0, 1.0]
                } else {
                    mat.diffuse_color
                };
                let mut pbr = serde_json::json!({
                    "baseColorFactor": base_color_factor,
                    "metallicFactor": 0.0,
                    "roughnessFactor": 1.0
                });
                if let Some(texture_index) = base_color_texture {
                    pbr["baseColorTexture"] = serde_json::json!({ "index": texture_index });
                }

                let mut material = serde_json::json!({
                    "name": mat.name,
                    "pbrMetallicRoughness": pbr
                });
                if let Some(texture_index) = Self::find_material_texture_index(
                    texture_indices,
                    &mat.name,
                    &["_BumpMap", "_NormalMap"],
                ) {
                    material["normalTexture"] = serde_json::json!({ "index": texture_index });
                }
                if let Some(texture_index) = Self::find_material_texture_index(
                    texture_indices,
                    &mat.name,
                    &["_OcclusionMap"],
                ) {
                    material["occlusionTexture"] = serde_json::json!({ "index": texture_index });
                }
                if let Some(texture_index) = Self::find_material_texture_index(
                    texture_indices,
                    &mat.name,
                    &["_EmissionMap"],
                ) {
                    material["emissiveTexture"] = serde_json::json!({ "index": texture_index });
                }
                if let Some(texture_index) = Self::find_material_texture_index(
                    texture_indices,
                    &mat.name,
                    &["_SpecGlossMap", "_MetallicGlossMap", "_RMOTex"],
                ) {
                    material["extras"] = serde_json::json!({
                        "unityPackedTexture": {
                            "index": texture_index,
                            "note": "Unity packed texture is referenced as metadata because glTF core material channels do not map it losslessly."
                        }
                    });
                }
                let has_emissive = mat.emissive_color[0] > 0.0
                    || mat.emissive_color[1] > 0.0
                    || mat.emissive_color[2] > 0.0;
                if has_emissive {
                    material["emissiveFactor"] = serde_json::json!([
                        mat.emissive_color[0],
                        mat.emissive_color[1],
                        mat.emissive_color[2]
                    ]);
                }
                material
            })
            .collect()
    }

    fn find_material_texture_index(
        texture_indices: &std::collections::HashMap<String, usize>,
        material_name: &str,
        slot_names: &[&str],
    ) -> Option<usize> {
        slot_names.iter().find_map(|slot_name| {
            texture_indices
                .get(&Self::material_texture_key(material_name, slot_name))
                .copied()
        })
    }

    fn material_index_for_primitive(
        primitive_index: usize,
        primitive_count: usize,
        materials: Option<&[MaterialInfo]>,
        material_count: usize,
    ) -> usize {
        if material_count == 0 {
            return 0;
        }
        if primitive_count <= 1 {
            return Self::preferred_single_primitive_material_index(materials)
                .unwrap_or(0)
                .min(material_count - 1);
        }
        primitive_index.min(material_count - 1)
    }

    fn preferred_single_primitive_material_index(
        materials: Option<&[MaterialInfo]>,
    ) -> Option<usize> {
        let materials = materials?;
        materials
            .iter()
            .enumerate()
            .find_map(|(index, material)| {
                (!Self::is_helper_material_name(&material.name)
                    && material.textures.iter().any(|slot| {
                        !slot.relative_path.is_empty()
                            && !slot.relative_path.starts_with('#')
                            && Self::is_base_color_slot(&slot.slot_name, &slot.usage)
                    }))
                .then_some(index)
            })
            .or_else(|| {
                materials.iter().enumerate().find_map(|(index, material)| {
                    (!Self::is_helper_material_name(&material.name)
                        && !material.textures.is_empty())
                    .then_some(index)
                })
            })
    }

    fn base_color_slot_names() -> &'static [&'static str] {
        &["_Base", "_BaseMap", "_MainTex", "_BaseColorMap"]
    }

    fn is_base_color_slot(slot_name: &str, usage: &str) -> bool {
        usage.eq_ignore_ascii_case("DiffuseColor")
            || Self::base_color_slot_names()
                .iter()
                .any(|candidate| slot_name.eq_ignore_ascii_case(candidate))
    }

    fn has_base_color_texture(materials: Option<&[MaterialInfo]>) -> bool {
        let Some(materials) = materials else {
            return false;
        };
        materials.iter().any(|material| {
            !Self::is_helper_material_name(&material.name)
                && material.textures.iter().any(|slot| {
                    !slot.relative_path.is_empty()
                        && !slot.relative_path.starts_with('#')
                        && Self::is_base_color_slot(&slot.slot_name, &slot.usage)
                })
        })
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

    fn material_texture_key(material_name: &str, slot_name: &str) -> String {
        format!("{}::{}", material_name, slot_name)
    }

    fn uri_encode_path(path: &str) -> String {
        path.replace('\\', "/")
            .split('/')
            .map(Self::uri_encode_segment)
            .collect::<Vec<_>>()
            .join("/")
    }

    fn uri_encode_segment(segment: &str) -> String {
        let mut encoded = String::new();
        for byte in segment.as_bytes() {
            let is_unreserved =
                byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'.' | b'_' | b'~');
            if is_unreserved {
                encoded.push(*byte as char);
            } else {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
        encoded
    }

    fn push_f32_accessor(
        bin: &mut Vec<u8>,
        buffer_views: &mut Vec<serde_json::Value>,
        accessors: &mut Vec<serde_json::Value>,
        data: &[f32],
        count: usize,
        accessor_type: &str,
        target: u32,
    ) -> usize {
        Self::align_bin(bin);
        let offset = bin.len();
        for value in data {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        let view_index = buffer_views.len();
        buffer_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": data.len() * 4,
            "target": target
        }));
        let accessor_index = accessors.len();
        accessors.push(serde_json::json!({
            "bufferView": view_index,
            "componentType": 5126,
            "count": count,
            "type": accessor_type,
            "min": Self::minmax(data, Self::type_stride(accessor_type), true),
            "max": Self::minmax(data, Self::type_stride(accessor_type), false)
        }));
        accessor_index
    }

    fn push_f32_accessor_no_minmax(
        bin: &mut Vec<u8>,
        buffer_views: &mut Vec<serde_json::Value>,
        accessors: &mut Vec<serde_json::Value>,
        data: &[f32],
        count: usize,
        accessor_type: &str,
        target: u32,
    ) -> usize {
        Self::align_bin(bin);
        let offset = bin.len();
        for value in data {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        let view_index = buffer_views.len();
        buffer_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": data.len() * 4,
            "target": target
        }));
        let accessor_index = accessors.len();
        accessors.push(serde_json::json!({
            "bufferView": view_index,
            "componentType": 5126,
            "count": count,
            "type": accessor_type
        }));
        accessor_index
    }

    fn push_u16_accessor(
        bin: &mut Vec<u8>,
        buffer_views: &mut Vec<serde_json::Value>,
        accessors: &mut Vec<serde_json::Value>,
        data: &[u16],
        count: usize,
        accessor_type: &str,
        target: u32,
    ) -> usize {
        Self::align_bin(bin);
        let offset = bin.len();
        for value in data {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        let view_index = buffer_views.len();
        buffer_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": data.len() * 2,
            "target": target
        }));
        let accessor_index = accessors.len();
        accessors.push(serde_json::json!({
            "bufferView": view_index,
            "componentType": 5123,
            "count": count,
            "type": accessor_type
        }));
        accessor_index
    }

    fn push_u32_accessor(
        bin: &mut Vec<u8>,
        buffer_views: &mut Vec<serde_json::Value>,
        accessors: &mut Vec<serde_json::Value>,
        data: &[u32],
        target: u32,
    ) -> usize {
        Self::align_bin(bin);
        let offset = bin.len();
        for value in data {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        let view_index = buffer_views.len();
        buffer_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": data.len() * 4,
            "target": target
        }));
        let accessor_index = accessors.len();
        accessors.push(serde_json::json!({
            "bufferView": view_index,
            "componentType": 5125,
            "count": data.len(),
            "type": "SCALAR"
        }));
        accessor_index
    }

    fn build_animations(
        animations: &[GlbAnimation],
        bin: &mut Vec<u8>,
        buffer_views: &mut Vec<serde_json::Value>,
        accessors: &mut Vec<serde_json::Value>,
        node_count: usize,
        skeleton_node_map: Option<(usize, &[Option<usize>], Option<usize>)>,
    ) -> Vec<serde_json::Value> {
        let mut result = Vec::new();
        for animation in animations {
            let mut samplers = Vec::new();
            let mut channels = Vec::new();
            for channel in &animation.channels {
                if channel.times.is_empty() || channel.values.is_empty() {
                    continue;
                }
                let output_type = match channel.path {
                    AnimationPath::Rotation => "VEC4",
                    AnimationPath::Translation | AnimationPath::Scale => "VEC3",
                    AnimationPath::Weights => "SCALAR",
                };
                let stride = match channel.path {
                    AnimationPath::Weights => channel.values.len() / channel.times.len(),
                    _ => Self::type_stride(output_type),
                };
                if stride == 0 || channel.values.len() != channel.times.len() * stride {
                    continue;
                }
                let Some(target_node) =
                    Self::animation_target_node(channel.path, channel.node, skeleton_node_map)
                else {
                    continue;
                };
                if target_node >= node_count {
                    continue;
                }
                let output_count = match channel.path {
                    AnimationPath::Weights => channel.values.len(),
                    _ => channel.times.len(),
                };
                let input = Self::push_f32_accessor(
                    bin,
                    buffer_views,
                    accessors,
                    &channel.times,
                    channel.times.len(),
                    "SCALAR",
                    ARRAY_BUFFER,
                );
                let output = Self::push_f32_accessor_no_minmax(
                    bin,
                    buffer_views,
                    accessors,
                    &channel.values,
                    output_count,
                    output_type,
                    ARRAY_BUFFER,
                );
                let sampler_index = samplers.len();
                samplers.push(serde_json::json!({
                    "input": input,
                    "output": output,
                    "interpolation": "LINEAR"
                }));
                channels.push(serde_json::json!({
                    "sampler": sampler_index,
                    "target": {
                        "node": target_node,
                        "path": Self::animation_path_name(channel.path)
                    }
                }));
            }
            if !samplers.is_empty() {
                result.push(serde_json::json!({
                    "name": animation.name,
                    "samplers": samplers,
                    "channels": channels
                }));
            }
        }
        result
    }

    fn animation_path_name(path: AnimationPath) -> &'static str {
        match path {
            AnimationPath::Translation => "translation",
            AnimationPath::Rotation => "rotation",
            AnimationPath::Scale => "scale",
            AnimationPath::Weights => "weights",
        }
    }

    fn append_animation_json(
        target: &mut Vec<serde_json::Value>,
        animations: Vec<serde_json::Value>,
    ) {
        for animation in animations {
            if let Some(existing) = target
                .iter_mut()
                .find(|existing| existing.get("name") == animation.get("name"))
            {
                let base_sampler = existing
                    .get("samplers")
                    .and_then(|value| value.as_array())
                    .map(|items| items.len())
                    .unwrap_or(0);
                if let (Some(existing_samplers), Some(new_samplers)) = (
                    existing.get_mut("samplers").and_then(|value| value.as_array_mut()),
                    animation.get("samplers").and_then(|value| value.as_array()),
                ) {
                    existing_samplers.extend(new_samplers.iter().cloned());
                }
                if let (Some(existing_channels), Some(new_channels)) = (
                    existing.get_mut("channels").and_then(|value| value.as_array_mut()),
                    animation.get("channels").and_then(|value| value.as_array()),
                ) {
                    existing_channels.extend(new_channels.iter().cloned().map(|mut channel| {
                        if let Some(sampler) = channel.get("sampler").and_then(|value| value.as_u64()) {
                            channel["sampler"] = serde_json::json!(sampler as usize + base_sampler);
                        }
                        channel
                    }));
                }
                continue;
            }
            target.push(animation);
        }
    }

    fn animation_target_node(
        path: AnimationPath,
        node: usize,
        skeleton_node_map: Option<(usize, &[Option<usize>], Option<usize>)>,
    ) -> Option<usize> {
        match path {
            AnimationPath::Weights => Some(0),
            _ => skeleton_node_map.and_then(|(offset, map, _)| {
                map.get(node)
                    .copied()
                    .flatten()
                    .map(|export| offset + export)
            }),
        }
    }

    fn write_glb(json_value: serde_json::Value, bin: Vec<u8>) -> Result<Vec<u8>, String> {
        let json = serde_json::to_string(&json_value).map_err(|e| format!("GLB JSON: {}", e))?;
        let json_chunk = FormatUtils::align4_json(json.as_bytes());
        let total_len = 12 + 8 + json_chunk.len() + 8 + bin.len();
        let mut out = Vec::with_capacity(total_len);
        out.extend_from_slice(&GLB_MAGIC.to_le_bytes());
        out.extend_from_slice(&GLB_VERSION.to_le_bytes());
        out.extend_from_slice(&(total_len as u32).to_le_bytes());
        out.extend_from_slice(&(json_chunk.len() as u32).to_le_bytes());
        out.extend_from_slice(&CHUNK_TYPE_JSON.to_le_bytes());
        out.extend_from_slice(&json_chunk);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&CHUNK_TYPE_BIN.to_le_bytes());
        out.extend_from_slice(&bin);
        Ok(out)
    }

    fn align_bin(bin: &mut Vec<u8>) {
        let padding = (4 - bin.len() % 4) % 4;
        bin.extend(std::iter::repeat(0).take(padding));
    }

    fn normalize_vertex_f32_attribute(
        data: &[f32],
        vertex_count: usize,
        stride: usize,
    ) -> Vec<f32> {
        data.iter().take(vertex_count * stride).copied().collect()
    }

    fn normalize_uv0_attribute(data: &[f32], vertex_count: usize) -> Vec<f32> {
        if data.len() >= vertex_count * 4 {
            let mut out = Vec::with_capacity(vertex_count * 2);
            for uv in data.chunks_exact(4).take(vertex_count) {
                out.push(uv[0]);
                out.push(1.0 - uv[1]);
            }
            out
        } else {
            data.chunks_exact(2)
                .take(vertex_count)
                .flat_map(|uv| [uv[0], 1.0 - uv[1]])
                .collect()
        }
    }

    fn flip_x(data: &[f32], stride: usize) -> Vec<f32> {
        data.iter()
            .enumerate()
            .map(|(i, value)| if i % stride == 0 { -*value } else { *value })
            .collect()
    }

    fn mesh_node_matrix_for_baking(skeleton: &GlbSkeleton) -> Option<[f32; 16]> {
        skeleton.mesh_parent.and_then(|parent| {
            let matrix = skeleton.joints.get(parent).map(|parent| {
                Self::compose_trs(
                    [
                        -parent.translation[0],
                        parent.translation[1],
                        parent.translation[2],
                    ],
                    [
                        parent.rotation[0],
                        -parent.rotation[1],
                        -parent.rotation[2],
                        parent.rotation[3],
                    ],
                    parent.scale,
                )
            })?;
            (!Self::mat4_near(&matrix, &Self::identity_mat4(), 0.0001)).then_some(matrix)
        })
    }

    fn bake_positions_if_needed(data: Vec<f32>, matrix: Option<[f32; 16]>) -> Vec<f32> {
        let Some(matrix) = matrix else {
            return data;
        };
        data.chunks_exact(3)
            .flat_map(|point| Self::transform_point(&matrix, [point[0], point[1], point[2]]))
            .collect()
    }

    fn bake_vectors_if_needed(data: Vec<f32>, matrix: Option<[f32; 16]>) -> Vec<f32> {
        let Some(matrix) = matrix else {
            return data;
        };
        data.chunks_exact(3)
            .flat_map(|point| Self::transform_vector(&matrix, [point[0], point[1], point[2]]))
            .collect()
    }

    fn bake_normals_if_needed(data: Vec<f32>, matrix: Option<[f32; 16]>) -> Vec<f32> {
        let Some(matrix) = matrix else {
            return data;
        };
        let Some(inverse) = Self::invert_mat4(&matrix) else {
            return data;
        };
        let normal_matrix = Self::transpose_mat4(&inverse);
        data.chunks_exact(3)
            .flat_map(|normal| {
                Self::normalize_vec3(Self::transform_vector(
                    &normal_matrix,
                    [normal[0], normal[1], normal[2]],
                ))
            })
            .collect()
    }

    fn bake_tangents_if_needed(data: Vec<f32>, matrix: Option<[f32; 16]>) -> Vec<f32> {
        let Some(matrix) = matrix else {
            return data;
        };
        let Some(inverse) = Self::invert_mat4(&matrix) else {
            return data;
        };
        let normal_matrix = Self::transpose_mat4(&inverse);
        data.chunks_exact(4)
            .flat_map(|tangent| {
                let transformed = Self::normalize_vec3(Self::transform_vector(
                    &normal_matrix,
                    [tangent[0], tangent[1], tangent[2]],
                ));
                [transformed[0], transformed[1], transformed[2], tangent[3]]
            })
            .collect()
    }

    fn remove_baked_mesh_matrix_from_inverse_binds(skin: &mut SkinData, matrix: &[f32; 16]) {
        let Some(inverse_baked) = Self::invert_mat4(matrix) else {
            return;
        };
        let mut converted = Vec::with_capacity(skin.inverse_bind_matrices.len());
        for inverse_bind in skin.inverse_bind_matrices.chunks_exact(16) {
            let mut bind = [0.0f32; 16];
            bind.copy_from_slice(inverse_bind);
            converted.extend_from_slice(&Self::mul_mat4(&bind, &inverse_baked));
        }
        skin.inverse_bind_matrices = converted;
    }

    fn transform_point(matrix: &[f32; 16], point: [f32; 3]) -> [f32; 3] {
        [
            matrix[0] * point[0] + matrix[4] * point[1] + matrix[8] * point[2] + matrix[12],
            matrix[1] * point[0] + matrix[5] * point[1] + matrix[9] * point[2] + matrix[13],
            matrix[2] * point[0] + matrix[6] * point[1] + matrix[10] * point[2] + matrix[14],
        ]
    }

    fn transform_vector(matrix: &[f32; 16], vector: [f32; 3]) -> [f32; 3] {
        [
            matrix[0] * vector[0] + matrix[4] * vector[1] + matrix[8] * vector[2],
            matrix[1] * vector[0] + matrix[5] * vector[1] + matrix[9] * vector[2],
            matrix[2] * vector[0] + matrix[6] * vector[1] + matrix[10] * vector[2],
        ]
    }

    fn normalize_vec3(vector: [f32; 3]) -> [f32; 3] {
        let len = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
        if len <= f32::EPSILON {
            vector
        } else {
            [vector[0] / len, vector[1] / len, vector[2] / len]
        }
    }

    fn flip_triangles(indices: &[u32]) -> Vec<u32> {
        if indices.len() < 3 {
            return indices.to_vec();
        }
        indices
            .chunks(3)
            .filter(|tri| tri.len() == 3)
            .flat_map(|tri| [tri[0], tri[2], tri[1]])
            .collect()
    }

    fn minmax(data: &[f32], stride: usize, is_min: bool) -> serde_json::Value {
        let mut values = vec![if is_min { f32::MAX } else { f32::MIN }; stride];
        for (i, value) in data.iter().enumerate() {
            let channel = i % stride;
            if is_min && *value < values[channel] {
                values[channel] = *value;
            }
            if !is_min && *value > values[channel] {
                values[channel] = *value;
            }
        }
        serde_json::json!(values)
    }

    fn type_stride(accessor_type: &str) -> usize {
        match accessor_type {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT4" => 16,
            _ => 1,
        }
    }

    fn skeleton_with_skin_joints_from_bone_hashes(
        skeleton: &GlbSkeleton,
        bone_name_hashes: &[u32],
    ) -> Option<GlbSkeleton> {
        if bone_name_hashes.is_empty() {
            return None;
        }
        let joint_by_hash = ModelContextResolver::transform_path_by_unity_hash(skeleton);
        let mut skin_joints = Vec::with_capacity(bone_name_hashes.len());
        for hash in bone_name_hashes {
            let joint_index = joint_by_hash.get(hash).copied()?;
            if joint_index >= skeleton.joints.len() {
                return None;
            }
            skin_joints.push(joint_index);
        }
        if skin_joints == skeleton.skin_joints {
            return None;
        }
        let mut reordered = skeleton.clone();
        reordered.skin_joints = skin_joints;
        Some(reordered)
    }

    fn build_skin_data(
        attrs: &MeshAttributes,
        vertex_count: usize,
        skeleton: Option<&GlbSkeleton>,
        animations: &[GlbAnimation],
    ) -> Option<SkinData> {
        let weights = attrs.bone_weights?;
        let joints = attrs.bone_indices?;
        let skeleton = skeleton?;
        if weights.len() < vertex_count * 4 || joints.len() < vertex_count * 4 {
            return None;
        }

        let bind_pose_count = attrs.bind_poses.map(|m| m.len() / 16).unwrap_or(0);
        let hash_ordered_skeleton = attrs
            .bone_name_hashes
            .filter(|hashes| bind_pose_count > 0 && hashes.len() == bind_pose_count)
            .and_then(|hashes| Self::skeleton_with_skin_joints_from_bone_hashes(skeleton, hashes));
        let skeleton = hash_ordered_skeleton.as_ref().unwrap_or(skeleton);
        let joint_count = if skeleton.skin_joints.is_empty() {
            0
        } else {
            skeleton.skin_joints.len()
        };
        if joint_count == 0 {
            return None;
        }

        let mut normalized_weights = weights[..vertex_count * 4].to_vec();
        for chunk in normalized_weights.chunks_mut(4) {
            let sum: f32 = chunk.iter().sum();
            if sum > f32::EPSILON {
                for value in chunk {
                    *value /= sum;
                }
            }
        }

        let use_bind_pose_rest = bind_pose_count > 0
            && attrs
                .bind_poses
                .is_some_and(|bind_poses| !Self::all_bind_poses_are_identity(bind_poses));
        let source_inverse_bind_matrices = if use_bind_pose_rest {
            Self::inverse_bind_matrices_from_bind_pose_order(attrs.bind_poses?, joint_count)
                .or_else(|| {
                    Self::inverse_bind_matrices_from_transform_rest_pose(skeleton, joint_count)
                })?
        } else {
            Self::inverse_bind_matrices_from_transform_rest_pose(skeleton, joint_count)?
        };
        let joint_local_matrices = if use_bind_pose_rest {
            Self::joint_local_matrices_from_bind_pose_order(
                skeleton,
                &source_inverse_bind_matrices,
                joint_count,
            )
        } else {
            None
        };
        let export_skin = Self::build_export_skin_joint_map(
            skeleton,
            &source_inverse_bind_matrices,
            joint_local_matrices.as_deref(),
            joint_count,
            animations,
        )?;
        let mut remapped_joints = joints[..vertex_count * 4].to_vec();
        for joint in &mut remapped_joints {
            *joint = export_skin
                .source_to_export
                .get(*joint as usize)
                .copied()
                .unwrap_or(0);
        }
        Some(SkinData {
            joints: remapped_joints,
            weights: normalized_weights,
            inverse_bind_matrices: export_skin.inverse_bind_matrices,
            export_joints: export_skin.export_joints,
            export_local_matrices: export_skin.export_local_matrices,
            export_parents: export_skin.export_parents,
            joint_node_map: export_skin.joint_node_map,
            skeleton_root: export_skin.skeleton_root,
            rest_pose_source: if use_bind_pose_rest {
                "bindPose"
            } else {
                "transform"
            },
        })
    }

    fn build_export_skin_joint_map(
        skeleton: &GlbSkeleton,
        source_inverse_bind_matrices: &[f32],
        joint_local_matrices: Option<&[[f32; 16]]>,
        joint_count: usize,
        animations: &[GlbAnimation],
    ) -> Option<ExportSkinJointMap> {
        if source_inverse_bind_matrices.len() < joint_count * 16 {
            return None;
        }

        let mut export_joints = Vec::with_capacity(joint_count);
        let mut export_source_indices = Vec::with_capacity(joint_count);
        let mut export_index_by_joint = vec![None; skeleton.joints.len()];
        for (source_index, joint_index) in skeleton
            .skin_joints
            .iter()
            .copied()
            .take(joint_count)
            .enumerate()
        {
            if joint_index >= skeleton.joints.len() {
                return None;
            }
            if export_index_by_joint[joint_index].is_some() {
                continue;
            }
            let export_index = export_joints.len();
            export_joints.push(joint_index);
            export_source_indices.push(Some(source_index));
            export_index_by_joint[joint_index] = Some(export_index as u16);
        }
        Self::append_animation_target_joints(
            skeleton,
            animations,
            &mut export_joints,
            &mut export_source_indices,
            &mut export_index_by_joint,
        );
        if export_joints.is_empty() {
            return None;
        }

        let rest_world = if let Some(local) = joint_local_matrices {
            let mut world = vec![None; skeleton.joints.len()];
            for index in 0..skeleton.joints.len() {
                Self::resolve_joint_world(index, skeleton, local, &mut world)?;
            }
            world
                .into_iter()
                .map(|matrix| matrix.unwrap_or_else(Self::identity_mat4))
                .collect::<Vec<_>>()
        } else {
            Self::joint_world_matrices_from_transform_rest_pose(skeleton)
        };

        let mut export_parents = Vec::with_capacity(export_joints.len());
        let mut export_local_matrices = Vec::with_capacity(export_joints.len());
        for joint_index in &export_joints {
            let parent_export =
                Self::nearest_export_parent(skeleton, *joint_index, &export_index_by_joint);
            let parent_world = parent_export
                .and_then(|parent| {
                    let parent_joint = export_joints.get(parent as usize).copied()?;
                    rest_world.get(parent_joint).copied()
                })
                .unwrap_or_else(Self::identity_mat4);
            let joint_world = *rest_world.get(*joint_index)?;
            let local = Self::mul_mat4(&Self::invert_mat4(&parent_world)?, &joint_world);
            export_parents.push(parent_export.map(|parent| parent as usize));
            export_local_matrices.push(local);
        }

        let mut inverse_bind_matrices = Vec::with_capacity(export_joints.len() * 16);
        for (export_index, source_index) in export_source_indices.into_iter().enumerate() {
            if let Some(source_index) = source_index {
                let start = source_index * 16;
                inverse_bind_matrices
                    .extend_from_slice(&source_inverse_bind_matrices[start..start + 16]);
            } else {
                let joint_index = export_joints.get(export_index).copied()?;
                inverse_bind_matrices
                    .extend_from_slice(&Self::invert_mat4(rest_world.get(joint_index)?)?);
            }
        }

        let mut source_to_export = Vec::with_capacity(joint_count);
        for joint_index in skeleton.skin_joints.iter().copied().take(joint_count) {
            source_to_export.push(export_index_by_joint.get(joint_index)?.as_ref().copied()?);
        }

        let mut joint_node_map = vec![None; skeleton.joints.len()];
        for (export_index, joint_index) in export_joints.iter().copied().enumerate() {
            joint_node_map[joint_index] = Some(export_index);
        }
        let skeleton_root = skeleton
            .skeleton_root
            .and_then(|root| export_index_by_joint.get(root).copied().flatten())
            .map(|index| index as usize)
            .or_else(|| {
                export_parents
                    .iter()
                    .enumerate()
                    .find_map(|(index, parent)| parent.is_none().then_some(index))
            });

        Some(ExportSkinJointMap {
            export_joints,
            export_local_matrices,
            export_parents,
            joint_node_map,
            source_to_export,
            inverse_bind_matrices,
            skeleton_root,
        })
    }

    fn append_animation_target_joints(
        skeleton: &GlbSkeleton,
        animations: &[GlbAnimation],
        export_joints: &mut Vec<usize>,
        export_source_indices: &mut Vec<Option<usize>>,
        export_index_by_joint: &mut [Option<u16>],
    ) {
        for animation in animations {
            for channel in &animation.channels {
                if matches!(channel.path, AnimationPath::Weights) {
                    continue;
                }
                let mut current = Some(channel.node);
                while let Some(joint_index) = current {
                    if joint_index >= skeleton.joints.len() {
                        break;
                    }
                    if export_index_by_joint[joint_index].is_none() {
                        let export_index = export_joints.len();
                        export_joints.push(joint_index);
                        export_source_indices.push(None);
                        export_index_by_joint[joint_index] = Some(export_index as u16);
                    }
                    current = skeleton.joints[joint_index].parent;
                }
            }
        }
    }

    fn nearest_export_parent(
        skeleton: &GlbSkeleton,
        joint_index: usize,
        export_index_by_joint: &[Option<u16>],
    ) -> Option<u16> {
        let mut current = skeleton
            .joints
            .get(joint_index)
            .and_then(|joint| joint.parent);
        while let Some(parent) = current {
            if let Some(export_index) = export_index_by_joint.get(parent).copied().flatten() {
                return Some(export_index);
            }
            current = skeleton.joints.get(parent).and_then(|joint| joint.parent);
        }
        None
    }

    fn export_joint_children(
        skin: &SkinData,
        parent_export_index: usize,
        joint_node_offset: usize,
    ) -> Vec<usize> {
        skin.export_parents
            .iter()
            .enumerate()
            .filter_map(|(child_index, parent)| {
                (*parent == Some(parent_export_index)).then_some(joint_node_offset + child_index)
            })
            .collect()
    }

    fn joint_node_json(
        joint: &crate::exporter::model_context::GlbJoint,
        local_matrix: Option<[f32; 16]>,
        rest_pose_source: &'static str,
    ) -> serde_json::Value {
        if let Some(matrix) = local_matrix {
            if let Some((translation, rotation, scale)) = Self::decompose_trs(&matrix) {
                return serde_json::json!({
                    "name": joint.name,
                    "translation": translation,
                    "rotation": rotation,
                    "scale": scale,
                    "extras": {
                        "unityTransformPathId": joint.transform_path_id,
                        "unityTransformPath": joint.path,
                        "restPoseSource": rest_pose_source
                    }
                });
            }

            return serde_json::json!({
                "name": joint.name,
                "matrix": matrix,
                "extras": {
                    "unityTransformPathId": joint.transform_path_id,
                    "unityTransformPath": joint.path,
                    "restPoseSource": rest_pose_source
                }
            });
        }

        serde_json::json!({
            "name": joint.name,
            "translation": [-joint.translation[0], joint.translation[1], joint.translation[2]],
            "rotation": [joint.rotation[0], -joint.rotation[1], -joint.rotation[2], joint.rotation[3]],
            "scale": joint.scale,
            "extras": {
                "unityTransformPathId": joint.transform_path_id,
                "unityTransformPath": joint.path,
                "restPoseSource": rest_pose_source
            }
        })
    }

    fn inverse_bind_matrices_from_bind_pose_order(
        bind_poses: &[f32],
        joint_count: usize,
    ) -> Option<Vec<f32>> {
        let bind_pose_count = bind_poses.len() / 16;
        if bind_pose_count == 0 || bind_pose_count < joint_count {
            return None;
        }

        let convert = Self::axis_flip_mat4();
        let mut inverse_bind_matrices = Vec::with_capacity(joint_count * 16);
        for skin_order in 0..joint_count {
            let start = skin_order * 16;
            let mut bind_pose = [0.0f32; 16];
            bind_pose.copy_from_slice(&bind_poses[start..start + 16]);
            let bind_pose = Self::transpose_mat4(&bind_pose);
            let converted = Self::mul_mat4(&Self::mul_mat4(&convert, &bind_pose), &convert);
            inverse_bind_matrices.extend_from_slice(&converted);
        }
        Some(inverse_bind_matrices)
    }

    fn joint_local_matrices_from_bind_pose_order(
        skeleton: &GlbSkeleton,
        inverse_bind_matrices: &[f32],
        joint_count: usize,
    ) -> Option<Vec<[f32; 16]>> {
        if inverse_bind_matrices.len() < joint_count * 16 {
            return None;
        }

        let mesh_world = Self::mesh_node_matrix_from_skeleton(skeleton);
        let mut desired_world = vec![None; skeleton.joints.len()];
        for skin_order in 0..joint_count {
            let joint_index = skeleton.skin_joints.get(skin_order).copied()?;
            if joint_index >= skeleton.joints.len() {
                return None;
            }
            let start = skin_order * 16;
            let mut inverse_bind = [0.0f32; 16];
            inverse_bind.copy_from_slice(&inverse_bind_matrices[start..start + 16]);
            let bind_from_mesh = Self::invert_mat4(&inverse_bind)?;
            desired_world[joint_index] = Some(Self::mul_mat4(&mesh_world, &bind_from_mesh));
        }

        let original_local = Self::joint_local_matrices_from_transform_rest_pose(skeleton);
        let mut local = vec![None; skeleton.joints.len()];
        let mut world = vec![None; skeleton.joints.len()];
        for index in 0..skeleton.joints.len() {
            Self::resolve_joint_local_from_bind_pose(
                index,
                skeleton,
                &original_local,
                &desired_world,
                &mut local,
                &mut world,
            )?;
        }
        Some(
            local
                .into_iter()
                .map(|matrix| matrix.unwrap_or_else(Self::identity_mat4))
                .collect(),
        )
    }

    fn resolve_joint_local_from_bind_pose(
        index: usize,
        skeleton: &GlbSkeleton,
        original_local: &[[f32; 16]],
        desired_world: &[Option<[f32; 16]>],
        local: &mut [Option<[f32; 16]>],
        world: &mut [Option<[f32; 16]>],
    ) -> Option<[f32; 16]> {
        if index >= original_local.len() {
            return None;
        }
        if let Some(matrix) = world[index] {
            return Some(matrix);
        }

        let parent_world =
            if let Some(parent) = skeleton.joints.get(index).and_then(|joint| joint.parent) {
                Self::resolve_joint_local_from_bind_pose(
                    parent,
                    skeleton,
                    original_local,
                    desired_world,
                    local,
                    world,
                )?
            } else {
                Self::identity_mat4()
            };

        let local_matrix = if let Some(target_world) = desired_world.get(index).copied().flatten() {
            let inverse_parent = Self::invert_mat4(&parent_world)?;
            Self::mul_mat4(&inverse_parent, &target_world)
        } else {
            original_local[index]
        };
        let world_matrix = Self::mul_mat4(&parent_world, &local_matrix);
        local[index] = Some(local_matrix);
        world[index] = Some(world_matrix);
        Some(world_matrix)
    }

    fn inverse_bind_matrices_from_transform_rest_pose(
        skeleton: &GlbSkeleton,
        joint_count: usize,
    ) -> Option<Vec<f32>> {
        let world = Self::joint_world_matrices_from_transform_rest_pose(skeleton);
        let mesh_world = skeleton
            .mesh_parent
            .and_then(|idx| world.get(idx).copied())
            .unwrap_or_else(Self::identity_mat4);

        let mut inverse_bind_matrices = Vec::with_capacity(joint_count * 16);
        for joint_index in skeleton.skin_joints.iter().copied().take(joint_count) {
            let joint_world = world
                .get(joint_index)
                .copied()
                .unwrap_or_else(Self::identity_mat4);
            let inverse_joint_world = Self::invert_mat4(&joint_world)?;
            let inverse_bind = Self::mul_mat4(&inverse_joint_world, &mesh_world);
            inverse_bind_matrices.extend_from_slice(&inverse_bind);
        }
        Some(inverse_bind_matrices)
    }

    fn joint_world_matrices_from_transform_rest_pose(skeleton: &GlbSkeleton) -> Vec<[f32; 16]> {
        let local = Self::joint_local_matrices_from_transform_rest_pose(skeleton);
        let mut world = vec![None; skeleton.joints.len()];
        for index in 0..skeleton.joints.len() {
            Self::resolve_joint_world(index, skeleton, &local, &mut world);
        }
        world
            .into_iter()
            .map(|matrix| matrix.unwrap_or_else(Self::identity_mat4))
            .collect()
    }

    fn joint_local_matrices_from_transform_rest_pose(skeleton: &GlbSkeleton) -> Vec<[f32; 16]> {
        skeleton
            .joints
            .iter()
            .map(|joint| {
                Self::compose_trs(
                    [
                        -joint.translation[0],
                        joint.translation[1],
                        joint.translation[2],
                    ],
                    [
                        joint.rotation[0],
                        -joint.rotation[1],
                        -joint.rotation[2],
                        joint.rotation[3],
                    ],
                    joint.scale,
                )
            })
            .collect()
    }

    fn mesh_node_matrix_from_skeleton(skeleton: &GlbSkeleton) -> [f32; 16] {
        skeleton
            .mesh_parent
            .and_then(|parent| skeleton.joints.get(parent))
            .map(|parent| {
                Self::compose_trs(
                    [
                        -parent.translation[0],
                        parent.translation[1],
                        parent.translation[2],
                    ],
                    [
                        parent.rotation[0],
                        -parent.rotation[1],
                        -parent.rotation[2],
                        parent.rotation[3],
                    ],
                    parent.scale,
                )
            })
            .unwrap_or_else(Self::identity_mat4)
    }

    fn all_bind_poses_are_identity(bind_poses: &[f32]) -> bool {
        bind_poses.chunks_exact(16).all(|matrix| {
            matrix.iter().enumerate().all(|(index, value)| {
                let expected = if index % 5 == 0 { 1.0 } else { 0.0 };
                (*value - expected).abs() < 0.0001
            })
        })
    }

    fn mat4_near(actual: &[f32; 16], expected: &[f32; 16], epsilon: f32) -> bool {
        actual
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| (actual - expected).abs() <= epsilon)
    }

    fn resolve_joint_world(
        index: usize,
        skeleton: &GlbSkeleton,
        local: &[[f32; 16]],
        world: &mut [Option<[f32; 16]>],
    ) -> Option<[f32; 16]> {
        if index >= local.len() {
            return None;
        }
        if let Some(matrix) = world[index] {
            return Some(matrix);
        }
        let matrix = if let Some(parent) = skeleton.joints.get(index).and_then(|joint| joint.parent)
        {
            let parent_world = Self::resolve_joint_world(parent, skeleton, local, world)?;
            Self::mul_mat4(&parent_world, &local[index])
        } else {
            local[index]
        };
        world[index] = Some(matrix);
        Some(matrix)
    }

    fn compose_trs(translation: [f32; 3], rotation: [f32; 4], scale: [f32; 3]) -> [f32; 16] {
        let [x, y, z, w] = rotation;
        let xx = x * x;
        let yy = y * y;
        let zz = z * z;
        let xy = x * y;
        let xz = x * z;
        let yz = y * z;
        let wx = w * x;
        let wy = w * y;
        let wz = w * z;

        [
            (1.0 - 2.0 * (yy + zz)) * scale[0],
            (2.0 * (xy + wz)) * scale[0],
            (2.0 * (xz - wy)) * scale[0],
            0.0,
            (2.0 * (xy - wz)) * scale[1],
            (1.0 - 2.0 * (xx + zz)) * scale[1],
            (2.0 * (yz + wx)) * scale[1],
            0.0,
            (2.0 * (xz + wy)) * scale[2],
            (2.0 * (yz - wx)) * scale[2],
            (1.0 - 2.0 * (xx + yy)) * scale[2],
            0.0,
            translation[0],
            translation[1],
            translation[2],
            1.0,
        ]
    }

    fn decompose_trs(m: &[f32; 16]) -> Option<([f32; 3], [f32; 4], [f32; 3])> {
        if m[3].abs() > 0.0001
            || m[7].abs() > 0.0001
            || m[11].abs() > 0.0001
            || (m[15] - 1.0).abs() > 0.0001
        {
            return None;
        }

        let col_len = |col: usize| -> f32 {
            let start = col * 4;
            (m[start] * m[start] + m[start + 1] * m[start + 1] + m[start + 2] * m[start + 2]).sqrt()
        };
        let scale = [col_len(0), col_len(1), col_len(2)];
        if scale.iter().any(|value| *value <= f32::EPSILON) {
            return None;
        }

        let mut r = [0.0f32; 9];
        r[0] = m[0] / scale[0];
        r[1] = m[1] / scale[0];
        r[2] = m[2] / scale[0];
        r[3] = m[4] / scale[1];
        r[4] = m[5] / scale[1];
        r[5] = m[6] / scale[1];
        r[6] = m[8] / scale[2];
        r[7] = m[9] / scale[2];
        r[8] = m[10] / scale[2];

        let dot01 = r[0] * r[3] + r[1] * r[4] + r[2] * r[5];
        let dot02 = r[0] * r[6] + r[1] * r[7] + r[2] * r[8];
        let dot12 = r[3] * r[6] + r[4] * r[7] + r[5] * r[8];
        if dot01.abs() > 0.001 || dot02.abs() > 0.001 || dot12.abs() > 0.001 {
            return None;
        }

        let trace = r[0] + r[4] + r[8];
        let rotation = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            [
                (r[5] - r[7]) / s,
                (r[6] - r[2]) / s,
                (r[1] - r[3]) / s,
                0.25 * s,
            ]
        } else if r[0] > r[4] && r[0] > r[8] {
            let s = (1.0 + r[0] - r[4] - r[8]).sqrt() * 2.0;
            [
                0.25 * s,
                (r[3] + r[1]) / s,
                (r[6] + r[2]) / s,
                (r[5] - r[7]) / s,
            ]
        } else if r[4] > r[8] {
            let s = (1.0 + r[4] - r[0] - r[8]).sqrt() * 2.0;
            [
                (r[3] + r[1]) / s,
                0.25 * s,
                (r[7] + r[5]) / s,
                (r[6] - r[2]) / s,
            ]
        } else {
            let s = (1.0 + r[8] - r[0] - r[4]).sqrt() * 2.0;
            [
                (r[6] + r[2]) / s,
                (r[7] + r[5]) / s,
                0.25 * s,
                (r[1] - r[3]) / s,
            ]
        };

        Some(([m[12], m[13], m[14]], rotation, scale))
    }

    fn identity_mat4() -> [f32; 16] {
        [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]
    }

    fn axis_flip_mat4() -> [f32; 16] {
        [
            -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]
    }

    fn transpose_mat4(m: &[f32; 16]) -> [f32; 16] {
        let mut out = [0.0f32; 16];
        for row in 0..4 {
            for col in 0..4 {
                out[row + col * 4] = m[col + row * 4];
            }
        }
        out
    }

    fn mul_mat4(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
        let mut out = [0.0f32; 16];
        for row in 0..4 {
            for col in 0..4 {
                let mut sum = 0.0;
                for k in 0..4 {
                    sum += a[row + k * 4] * b[k + col * 4];
                }
                out[row + col * 4] = sum;
            }
        }
        out
    }

    fn invert_mat4(m: &[f32; 16]) -> Option<[f32; 16]> {
        let mut aug = [[0.0f32; 8]; 4];
        for row in 0..4 {
            for col in 0..4 {
                aug[row][col] = m[row + col * 4];
            }
            aug[row][4 + row] = 1.0;
        }

        for col in 0..4 {
            let mut pivot = col;
            for row in (col + 1)..4 {
                if aug[row][col].abs() > aug[pivot][col].abs() {
                    pivot = row;
                }
            }
            if aug[pivot][col].abs() <= f32::EPSILON {
                return None;
            }
            if pivot != col {
                aug.swap(pivot, col);
            }

            let divisor = aug[col][col];
            for value in &mut aug[col] {
                *value /= divisor;
            }

            for row in 0..4 {
                if row == col {
                    continue;
                }
                let factor = aug[row][col];
                for idx in 0..8 {
                    aug[row][idx] -= factor * aug[col][idx];
                }
            }
        }

        let mut out = [0.0f32; 16];
        for row in 0..4 {
            for col in 0..4 {
                out[row + col * 4] = aug[row][4 + col];
            }
        }
        Some(out)
    }

    fn primitive_ranges(attrs: &MeshAttributes, index_count: usize) -> Vec<(usize, usize, i32)> {
        let Some(sub_meshes) = attrs.sub_meshes else {
            return vec![(0, index_count, 0)];
        };
        let mut ranges: Vec<(usize, usize, i32)> = sub_meshes
            .iter()
            .filter_map(|sm| {
                if sm.index_count == 0 || sm.index_start >= index_count {
                    return None;
                }
                let count = sm.index_count.min(index_count - sm.index_start);
                Some((sm.index_start, count, sm.topology))
            })
            .collect();
        if ranges.is_empty() {
            ranges.push((0, index_count, 0));
        }
        ranges
    }

    fn gltf_mode(unity_topology: i32) -> u32 {
        match unity_topology {
            3 => 1,
            4 => 3,
            5 => 0,
            _ => 4,
        }
    }

    fn build_morph_targets(attrs: &MeshAttributes, vertex_count: usize) -> Vec<MorphTargetData> {
        let Some(blend_shapes) = attrs.blend_shapes else {
            return Vec::new();
        };
        blend_shapes
            .iter()
            .filter_map(|shape| {
                if shape.delta_vertices.len() < vertex_count * 3 {
                    return None;
                }
                Some(MorphTargetData {
                    name: shape.name.clone(),
                    positions: shape.delta_vertices[..vertex_count * 3].to_vec(),
                    normals: if shape.delta_normals.len() >= vertex_count * 3 {
                        shape.delta_normals[..vertex_count * 3].to_vec()
                    } else {
                        Vec::new()
                    },
                    tangents: if shape.delta_tangents.len() >= vertex_count * 3 {
                        shape.delta_tangents[..vertex_count * 3].to_vec()
                    } else {
                        Vec::new()
                    },
                })
            })
            .collect()
    }

    fn zero_morph_weights(count: usize) -> Vec<f32> {
        vec![0.0; count]
    }
}

#[cfg(test)]
mod tests {
    use super::AnimationPath;
    use super::GlbExporter;
    use super::GlbSceneMesh;
    use crate::exporter::animation_clip_exporter::{GlbAnimation, GlbAnimationChannel};
    use crate::exporter::material_info::{MaterialInfo, TextureSlot};
    use crate::exporter::mesh_exporter::MeshAttributes;
    use crate::exporter::model_context::{GlbJoint, GlbSkeleton};

    #[test]
    fn build_with_scene_data_writes_animation_channels_when_animation_data_is_present() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let skeleton = GlbSkeleton {
            joints: vec![GlbJoint {
                name: "root".to_string(),
                path: "root".to_string(),
                transform_path_id: 1,
                parent: None,
                children: vec![],
                translation: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            }],
            skin_joints: vec![0],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let bind_poses = vec![
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let bone_weights = vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let bone_indices = vec![0u16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: &[0, 1, 2],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&bone_weights),
            bone_indices: Some(&bone_indices),
            bind_poses: Some(&bind_poses),
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };
        let animations = vec![GlbAnimation {
            name: "walk".to_string(),
            channels: vec![GlbAnimationChannel {
                node: 0,
                path: AnimationPath::Translation,
                times: vec![0.0, 1.0],
                values: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            }],
        }];

        let glb = GlbExporter::build_with_scene_data(
            "mesh",
            &attrs,
            None,
            Some(&skeleton),
            Some(&animations),
        )
        .expect("glb");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();

        assert_eq!(json["animations"][0]["name"], serde_json::json!("walk"));
        assert_eq!(
            json["animations"][0]["channels"][0]["target"],
            serde_json::json!({ "node": 1, "path": "translation" })
        );
    }

    #[test]
    fn build_with_scene_data_drops_animation_channels_targeting_missing_nodes() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: &[0, 1, 2],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: None,
            bone_indices: None,
            bind_poses: None,
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };
        let animations = vec![GlbAnimation {
            name: "bad".to_string(),
            channels: vec![GlbAnimationChannel {
                node: 99,
                path: AnimationPath::Translation,
                times: vec![0.0, 1.0],
                values: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            }],
        }];

        let glb = GlbExporter::build_with_scene_data("mesh", &attrs, None, None, Some(&animations))
            .expect("glb");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();

        assert!(json.get("animations").is_none());
    }

    #[test]
    fn build_with_scene_data_references_sidecar_textures_from_material_slots() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: &[0, 1, 2],
            normals: None,
            uvs: Some(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]),
            tangents: None,
            colors: Some(&[0.2, 0.3, 0.4, 1.0, 0.2, 0.3, 0.4, 1.0, 0.2, 0.3, 0.4, 1.0]),
            bone_weights: None,
            bone_indices: None,
            bind_poses: None,
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };
        let materials = vec![MaterialInfo {
            name: "body".to_string(),
            diffuse_color: [1.0, 1.0, 1.0, 1.0],
            specular_color: [0.0, 0.0, 0.0, 0.0],
            emissive_color: [0.0, 0.0, 0.0, 0.0],
            shininess: 0.0,
            textures: vec![
                TextureSlot {
                    slot_name: "_BaseMap".to_string(),
                    usage: "DiffuseColor".to_string(),
                    file_name: "body_BaseMap.dds".to_string(),
                    relative_path: "body_BaseMap.dds".to_string(),
                },
                TextureSlot {
                    slot_name: "_BumpMap".to_string(),
                    usage: "NormalMap".to_string(),
                    file_name: "body_BumpMap.dds".to_string(),
                    relative_path: "body_BumpMap.dds".to_string(),
                },
            ],
        }];

        let glb = GlbExporter::build_with_scene_data("mesh", &attrs, Some(&materials), None, None)
            .expect("glb");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();

        assert_eq!(
            json["images"][0]["uri"],
            serde_json::json!("body_BaseMap.dds")
        );
        assert_eq!(
            json["images"][1]["uri"],
            serde_json::json!("body_BumpMap.dds")
        );
        assert_eq!(
            json["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"],
            serde_json::json!(0)
        );
        assert_eq!(
            json["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"],
            serde_json::json!([1.0, 1.0, 1.0, 1.0])
        );
        assert_eq!(
            json["materials"][0]["normalTexture"]["index"],
            serde_json::json!(1)
        );
        assert_eq!(json["textures"][0]["source"], serde_json::json!(0));
        assert_eq!(json["samplers"][0]["wrapS"], serde_json::json!(10497));
        assert!(json["meshes"][0]["primitives"][0]["attributes"]
            .get("COLOR_0")
            .is_none());
    }

    #[test]
    fn single_primitive_prefers_textured_base_material_over_lit_placeholder() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: &[0, 1, 2],
            normals: None,
            uvs: Some(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]),
            tangents: None,
            colors: None,
            bone_weights: None,
            bone_indices: None,
            bind_poses: None,
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };
        let materials = vec![
            MaterialInfo {
                name: "Lit".to_string(),
                ..Default::default()
            },
            MaterialInfo {
                name: "mat_body".to_string(),
                textures: vec![TextureSlot {
                    slot_name: "_Base".to_string(),
                    usage: "Unknown".to_string(),
                    file_name: "body.png".to_string(),
                    relative_path: "body.png".to_string(),
                }],
                ..Default::default()
            },
        ];

        let glb = GlbExporter::build_with_scene_data("mesh", &attrs, Some(&materials), None, None)
            .expect("glb");
        let (json, _) = parse_glb_json_and_bin(&glb);

        assert_eq!(
            json["meshes"][0]["primitives"][0]["material"],
            serde_json::json!(1)
        );
        assert_eq!(
            json["materials"][1]["pbrMetallicRoughness"]["baseColorTexture"]["index"],
            serde_json::json!(0)
        );
    }

    #[test]
    fn build_with_scene_data_exports_uvs_in_gltf_texture_space() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: &[0, 1, 2],
            normals: None,
            uvs: Some(&[0.25, 0.0, 0.50, 0.25, 0.75, 1.0]),
            tangents: None,
            colors: None,
            bone_weights: None,
            bone_indices: None,
            bind_poses: None,
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };

        let glb =
            GlbExporter::build_with_scene_data("mesh", &attrs, None, None, None).expect("glb");
        let (json, bin) = parse_glb_json_and_bin(&glb);
        let uv_accessor_index = json["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"]
            .as_u64()
            .expect("uv accessor") as usize;
        let uv_values = read_f32_accessor(&json, bin, uv_accessor_index);

        assert_eq!(uv_values, vec![0.25, 1.0, 0.50, 0.75, 0.75, 0.0]);
    }

    #[test]
    fn build_with_scene_data_exports_only_uv0_from_four_component_uv_data() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: &[0, 1, 2],
            normals: None,
            uvs: Some(&[
                0.25, 0.0, -42.0, 47.0, 0.50, 0.25, -43.0, 38.0, 0.75, 1.0, -45.0, 32.0,
            ]),
            tangents: None,
            colors: None,
            bone_weights: None,
            bone_indices: None,
            bind_poses: None,
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };

        let glb =
            GlbExporter::build_with_scene_data("mesh", &attrs, None, None, None).expect("glb");
        let (json, bin) = parse_glb_json_and_bin(&glb);
        let uv_accessor_index = json["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"]
            .as_u64()
            .unwrap() as usize;
        let uv_accessor = &json["accessors"][uv_accessor_index];
        let uv_buffer_view =
            &json["bufferViews"][uv_accessor["bufferView"].as_u64().unwrap() as usize];
        let uv_values = read_f32_accessor(&json, bin, uv_accessor_index);

        assert_eq!(uv_accessor["type"], serde_json::json!("VEC2"));
        assert_eq!(uv_buffer_view["byteLength"], serde_json::json!(3 * 2 * 4));
        assert_eq!(uv_values, vec![0.25, 1.0, 0.50, 0.75, 0.75, 0.0]);
    }

    #[test]
    fn build_with_scene_data_writes_one_primitive_per_submesh() {
        use crate::exporter::mesh_exporter::{MeshAttributes, MeshSubMesh};

        let sub_meshes = vec![
            MeshSubMesh {
                index_start: 0,
                index_count: 3,
                topology: 0,
            },
            MeshSubMesh {
                index_start: 3,
                index_count: 3,
                topology: 0,
            },
        ];
        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0],
            indices: &[0, 1, 2, 1, 3, 2],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: None,
            bone_indices: None,
            bind_poses: None,
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: Some(&sub_meshes),
            blend_shapes: None,
        };

        let glb =
            GlbExporter::build_with_scene_data("mesh", &attrs, None, None, None).expect("glb");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();

        assert_eq!(json["meshes"][0]["primitives"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn split_submeshes_scene_data_writes_one_node_and_mesh_per_submesh() {
        use crate::exporter::mesh_exporter::{MeshAttributes, MeshSubMesh};

        let sub_meshes = vec![
            MeshSubMesh {
                index_start: 0,
                index_count: 3,
                topology: 0,
            },
            MeshSubMesh {
                index_start: 3,
                index_count: 3,
                topology: 0,
            },
        ];
        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0],
            indices: &[0, 1, 2, 1, 3, 2],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: None,
            bone_indices: None,
            bind_poses: None,
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: Some(&sub_meshes),
            blend_shapes: None,
        };

        let glb =
            GlbExporter::build_with_split_submeshes_scene_data("mesh", &attrs, None, None, None)
                .expect("glb");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();

        assert_eq!(json["meshes"].as_array().unwrap().len(), 2);
        assert_eq!(json["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(json["nodes"][0]["mesh"], serde_json::json!(0));
        assert_eq!(json["nodes"][1]["mesh"], serde_json::json!(1));
        assert_eq!(json["meshes"][0]["primitives"].as_array().unwrap().len(), 1);
        assert_eq!(json["meshes"][1]["primitives"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn split_submeshes_scene_data_keeps_skin_on_each_submesh_node() {
        use crate::exporter::mesh_exporter::{MeshAttributes, MeshSubMesh};

        let skeleton = GlbSkeleton {
            joints: vec![GlbJoint {
                name: "root".to_string(),
                path: "root".to_string(),
                transform_path_id: 1,
                parent: None,
                children: Vec::new(),
                translation: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            }],
            skin_joints: vec![0],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let sub_meshes = vec![
            MeshSubMesh {
                index_start: 0,
                index_count: 3,
                topology: 0,
            },
            MeshSubMesh {
                index_start: 3,
                index_count: 3,
                topology: 0,
            },
        ];
        let bind_poses = vec![
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let bone_weights = vec![
            1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
        ];
        let bone_indices = vec![0u16; 16];
        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0],
            indices: &[0, 1, 2, 1, 3, 2],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&bone_weights),
            bone_indices: Some(&bone_indices),
            bind_poses: Some(&bind_poses),
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: Some(&sub_meshes),
            blend_shapes: None,
        };

        let glb = GlbExporter::build_with_split_submeshes_scene_data(
            "mesh",
            &attrs,
            None,
            Some(&skeleton),
            None,
        )
        .expect("glb");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();

        assert_eq!(json["meshes"].as_array().unwrap().len(), 2);
        assert_eq!(json["nodes"][0]["skin"], serde_json::json!(0));
        assert_eq!(json["nodes"][1]["skin"], serde_json::json!(0));
        assert_eq!(json["nodes"][2]["name"], serde_json::json!("root"));
        assert_eq!(json["scenes"][0]["nodes"], serde_json::json!([0, 1, 2]));
    }

    #[test]
    fn multi_skinned_scene_data_keeps_one_skin_per_mesh() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let skeleton_a = GlbSkeleton {
            joints: vec![GlbJoint {
                name: "mesh_a_bone".to_string(),
                path: "mesh_a/mesh_a_bone".to_string(),
                transform_path_id: 101,
                parent: None,
                children: Vec::new(),
                translation: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            }],
            skin_joints: vec![0],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let skeleton_b = GlbSkeleton {
            joints: vec![GlbJoint {
                name: "mesh_b_bone".to_string(),
                path: "mesh_b/mesh_b_bone".to_string(),
                transform_path_id: 201,
                parent: None,
                children: Vec::new(),
                translation: [1.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            }],
            skin_joints: vec![0],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let bind_pose = vec![
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let bone_weights = vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let bone_indices = vec![0u16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let attrs_a = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: &[0, 1, 2],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&bone_weights),
            bone_indices: Some(&bone_indices),
            bind_poses: Some(&bind_pose),
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };
        let attrs_b = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0, 0.0],
            indices: &[0, 1, 2],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&bone_weights),
            bone_indices: Some(&bone_indices),
            bind_poses: Some(&bind_pose),
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };
        let scene_meshes = vec![
            GlbSceneMesh {
                name: "mesh_a",
                attrs: attrs_a,
                skeleton: Some(&skeleton_a),
            },
            GlbSceneMesh {
                name: "mesh_b",
                attrs: attrs_b,
                skeleton: Some(&skeleton_b),
            },
        ];

        let glb = GlbExporter::build_multi_skinned_scene_data("animator", &scene_meshes, None)
            .expect("glb");
        let (json, bin) = parse_glb_json_and_bin(&glb);
        let nodes = json["nodes"].as_array().unwrap();
        let skins = json["skins"].as_array().unwrap();

        assert_eq!(skins.len(), 2);
        assert_eq!(nodes[0]["mesh"], serde_json::json!(0));
        assert_eq!(nodes[0]["skin"], serde_json::json!(0));
        assert_eq!(nodes[2]["mesh"], serde_json::json!(1));
        assert_eq!(nodes[2]["skin"], serde_json::json!(1));
        assert_eq!(skins[0]["joints"], serde_json::json!([1]));
        assert_eq!(skins[1]["joints"], serde_json::json!([3]));
        assert_eq!(nodes[1]["name"], serde_json::json!("mesh_a_bone"));
        assert_eq!(nodes[3]["name"], serde_json::json!("mesh_b_bone"));

        let joints_a = json["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"]
            .as_u64()
            .unwrap() as usize;
        let joints_b = json["meshes"][1]["primitives"][0]["attributes"]["JOINTS_0"]
            .as_u64()
            .unwrap() as usize;
        assert_eq!(read_u16_accessor(&json, bin, joints_a), vec![0; 12]);
        assert_eq!(read_u16_accessor(&json, bin, joints_b), vec![0; 12]);
    }

    #[test]
    fn multi_skinned_scene_data_keeps_each_mesh_bind_pose_rest_pose_identity() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let skeleton_a = GlbSkeleton {
            joints: vec![GlbJoint {
                name: "mesh_a_bone".to_string(),
                path: "mesh_a/mesh_a_bone".to_string(),
                transform_path_id: 101,
                parent: None,
                children: Vec::new(),
                translation: [1.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            }],
            skin_joints: vec![0],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let skeleton_b = GlbSkeleton {
            joints: vec![GlbJoint {
                name: "mesh_b_bone".to_string(),
                path: "mesh_b/mesh_b_bone".to_string(),
                transform_path_id: 201,
                parent: None,
                children: Vec::new(),
                translation: [0.0, 3.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            }],
            skin_joints: vec![0],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let bind_poses_a = raw_bind_poses(&[gltf_translation_mat4(-5.0, 0.0, 0.0)]);
        let bind_poses_b = raw_bind_poses(&[gltf_translation_mat4(0.0, -7.0, 0.0)]);
        let bone_weights = vec![1.0, 0.0, 0.0, 0.0];
        let bone_indices = vec![0u16, 0, 0, 0];
        let attrs_a = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0],
            indices: &[0],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&bone_weights),
            bone_indices: Some(&bone_indices),
            bind_poses: Some(&bind_poses_a),
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };
        let attrs_b = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0],
            indices: &[0],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&bone_weights),
            bone_indices: Some(&bone_indices),
            bind_poses: Some(&bind_poses_b),
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };
        let scene_meshes = vec![
            GlbSceneMesh {
                name: "mesh_a",
                attrs: attrs_a,
                skeleton: Some(&skeleton_a),
            },
            GlbSceneMesh {
                name: "mesh_b",
                attrs: attrs_b,
                skeleton: Some(&skeleton_b),
            },
        ];

        let glb = GlbExporter::build_multi_skinned_scene_data("animator", &scene_meshes, None)
            .expect("glb");
        let (json, bin) = parse_glb_json_and_bin(&glb);

        assert_all_skin_rest_poses_identity(&json, bin);
    }

    #[test]
    fn split_submesh_animation_channels_target_skeleton_nodes_not_mesh_nodes() {
        use crate::exporter::mesh_exporter::{MeshAttributes, MeshSubMesh};

        let skeleton = GlbSkeleton {
            joints: vec![GlbJoint {
                name: "root".to_string(),
                path: "root".to_string(),
                transform_path_id: 1,
                parent: None,
                children: Vec::new(),
                translation: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            }],
            skin_joints: vec![0],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let sub_meshes = vec![
            MeshSubMesh {
                index_start: 0,
                index_count: 3,
                topology: 0,
            },
            MeshSubMesh {
                index_start: 3,
                index_count: 3,
                topology: 0,
            },
        ];
        let bind_poses = vec![
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let bone_weights = vec![
            1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
        ];
        let bone_indices = vec![0u16; 16];
        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0],
            indices: &[0, 1, 2, 1, 3, 2],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&bone_weights),
            bone_indices: Some(&bone_indices),
            bind_poses: Some(&bind_poses),
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: Some(&sub_meshes),
            blend_shapes: None,
        };
        let animations = vec![GlbAnimation {
            name: "walk".to_string(),
            channels: vec![GlbAnimationChannel {
                node: 0,
                path: AnimationPath::Translation,
                times: vec![0.0, 1.0],
                values: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            }],
        }];

        let glb = GlbExporter::build_with_split_submeshes_scene_data(
            "mesh",
            &attrs,
            None,
            Some(&skeleton),
            Some(&animations),
        )
        .expect("glb");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();

        assert_eq!(json["nodes"][0]["mesh"], serde_json::json!(0));
        assert_eq!(json["nodes"][1]["mesh"], serde_json::json!(1));
        assert_eq!(json["nodes"][2]["name"], serde_json::json!("root"));
        assert_eq!(
            json["animations"][0]["channels"][0]["target"],
            serde_json::json!({ "node": 2, "path": "translation" })
        );
    }

    #[test]
    fn skin_joint_indices_are_not_clamped_by_source_bind_pose_count() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let skeleton = GlbSkeleton {
            joints: vec![
                GlbJoint {
                    name: "root".to_string(),
                    path: "root".to_string(),
                    transform_path_id: 1,
                    parent: None,
                    children: vec![1],
                    translation: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "hair".to_string(),
                    path: "root/hair".to_string(),
                    transform_path_id: 2,
                    parent: Some(0),
                    children: Vec::new(),
                    translation: [0.0, 1.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            ],
            skin_joints: vec![0, 1],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let bind_poses = vec![
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let bone_weights = vec![1.0, 0.0, 0.0, 0.0];
        let bone_indices = vec![1u16, 1, 1, 1];
        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0],
            indices: &[0],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&bone_weights),
            bone_indices: Some(&bone_indices),
            bind_poses: Some(&bind_poses),
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };

        let glb = GlbExporter::build_with_scene_data("mesh", &attrs, None, Some(&skeleton), None)
            .expect("glb");
        let (json, bin) = parse_glb_json_and_bin(&glb);
        let joint_accessor_index = json["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"]
            .as_u64()
            .expect("joint accessor") as usize;
        let joint_values = read_u16_accessor(&json, bin, joint_accessor_index);

        assert_eq!(joint_values, vec![1, 1, 1, 1]);
    }

    #[test]
    fn skin_nodes_keep_transform_trs_when_bind_pose_is_at_origin() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let skeleton = GlbSkeleton {
            joints: vec![
                GlbJoint {
                    name: "parent".to_string(),
                    path: "parent".to_string(),
                    transform_path_id: 1,
                    parent: None,
                    children: vec![1],
                    translation: [2.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "child".to_string(),
                    path: "parent/child".to_string(),
                    transform_path_id: 2,
                    parent: Some(0),
                    children: vec![],
                    translation: [3.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            ],
            skin_joints: vec![0, 1],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let bind_poses = vec![
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0,
            0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let bone_weights = vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let bone_indices = vec![0u16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: &[0, 1, 2],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&bone_weights),
            bone_indices: Some(&bone_indices),
            bind_poses: Some(&bind_poses),
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };

        let glb = GlbExporter::build_with_scene_data("mesh", &attrs, None, Some(&skeleton), None)
            .expect("glb");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
        let nodes = json["nodes"].as_array().unwrap();

        assert_eq!(nodes[1]["translation"], serde_json::json!([-2.0, 0.0, 0.0]));
        assert_eq!(nodes[2]["translation"], serde_json::json!([-3.0, 0.0, 0.0]));
        assert!(nodes[1].get("matrix").is_none());
        assert_eq!(
            nodes[1]["extras"]["restPoseSource"],
            serde_json::json!("transform")
        );
        assert_eq!(json["skins"][0]["joints"], serde_json::json!([1, 2]));
    }

    #[test]
    fn inverse_bind_matrices_match_exported_transform_rest_pose() {
        let skeleton = GlbSkeleton {
            joints: vec![
                GlbJoint {
                    name: "mesh_node".to_string(),
                    path: "mesh_node".to_string(),
                    transform_path_id: 1,
                    parent: None,
                    children: vec![1],
                    translation: [5.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "bone".to_string(),
                    path: "mesh_node/bone".to_string(),
                    transform_path_id: 2,
                    parent: Some(0),
                    children: vec![],
                    translation: [2.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            ],
            skin_joints: vec![1],
            skin_joint_hashes: Vec::new(),
            mesh_parent: Some(0),
            skeleton_root: Some(1),
            roots: vec![0],
        };

        let inverse_bind =
            GlbExporter::inverse_bind_matrices_from_transform_rest_pose(&skeleton, 1)
                .expect("inverse bind");
        let joint_world = GlbExporter::joint_world_matrices_from_transform_rest_pose(&skeleton)[1];
        let mesh_world = GlbExporter::joint_world_matrices_from_transform_rest_pose(&skeleton)[0];
        let inverse_mesh_world = GlbExporter::invert_mat4(&mesh_world).expect("mesh inverse");
        let mut inverse_bind_matrix = [0.0f32; 16];
        inverse_bind_matrix.copy_from_slice(&inverse_bind[..16]);

        let skin_matrix = GlbExporter::mul_mat4(
            &GlbExporter::mul_mat4(&inverse_mesh_world, &joint_world),
            &inverse_bind_matrix,
        );

        assert_mat4_near(&skin_matrix, &GlbExporter::identity_mat4());
    }

    #[test]
    fn skin_joint_export_excludes_non_skin_parent_chain_and_preserves_unity_weight_mapping() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let skeleton = GlbSkeleton {
            joints: vec![
                GlbJoint {
                    name: "root".to_string(),
                    path: "root".to_string(),
                    transform_path_id: 1,
                    parent: None,
                    children: vec![1, 2],
                    translation: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "bone_a".to_string(),
                    path: "root/bone_a".to_string(),
                    transform_path_id: 2,
                    parent: Some(0),
                    children: vec![],
                    translation: [1.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "bone_b".to_string(),
                    path: "root/bone_b".to_string(),
                    transform_path_id: 3,
                    parent: Some(0),
                    children: vec![],
                    translation: [2.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            ],
            skin_joints: vec![2, 1],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let bind_poses = raw_bind_poses(&[
            gltf_translation_mat4(-2.0, 0.0, 0.0),
            gltf_translation_mat4(-1.0, 0.0, 0.0),
        ]);
        let bone_weights = vec![1.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let bone_indices = vec![0u16, 0, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0];
        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: &[0, 1, 2],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&bone_weights),
            bone_indices: Some(&bone_indices),
            bind_poses: Some(&bind_poses),
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };

        let glb = GlbExporter::build_with_scene_data("mesh", &attrs, None, Some(&skeleton), None)
            .expect("glb");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();

        assert_eq!(json["nodes"].as_array().unwrap().len(), 3);
        assert_eq!(json["skins"][0]["joints"], serde_json::json!([1, 2]));
        assert_eq!(json["skins"][0]["skeleton"], serde_json::json!(1));

        let bin_header_offset = 20 + json_len;
        let bin_len = u32::from_le_bytes(
            glb[bin_header_offset..bin_header_offset + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let bin_start = bin_header_offset + 8;
        let bin = &glb[bin_start..bin_start + bin_len];
        let joint_accessor = json["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"]
            .as_u64()
            .unwrap() as usize;
        let remapped_joints = read_u16_accessor(&json, bin, joint_accessor);

        assert_eq!(
            remapped_joints,
            vec![
                0, 0, 0, 0, // Unity bone index 0 => bone_b
                0, 1, 0, 0, // Unity bone index 1 => bone_a
                1, 0, 0, 0,
            ]
        );
    }

    #[test]
    fn skinned_mesh_transform_is_baked_into_geometry() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let skeleton = GlbSkeleton {
            joints: vec![
                GlbJoint {
                    name: "mesh_node".to_string(),
                    path: "mesh_node".to_string(),
                    transform_path_id: 1,
                    parent: None,
                    children: vec![1],
                    translation: [5.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [0.01, 0.01, 0.01],
                },
                GlbJoint {
                    name: "bone".to_string(),
                    path: "mesh_node/bone".to_string(),
                    transform_path_id: 2,
                    parent: Some(0),
                    children: vec![],
                    translation: [2.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            ],
            skin_joints: vec![1],
            skin_joint_hashes: Vec::new(),
            mesh_parent: Some(0),
            skeleton_root: Some(1),
            roots: vec![0],
        };
        let mesh_world =
            GlbExporter::compose_trs([-5.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.01, 0.01, 0.01]);
        let joint_world = GlbExporter::mul_mat4(
            &mesh_world,
            &GlbExporter::compose_trs([-2.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
        );
        let bind_poses = raw_bind_poses(&[GlbExporter::mul_mat4(
            &GlbExporter::invert_mat4(&joint_world).expect("joint inverse"),
            &mesh_world,
        )]);
        let bone_weights = vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let bone_indices = vec![0u16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: &[0, 1, 2],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&bone_weights),
            bone_indices: Some(&bone_indices),
            bind_poses: Some(&bind_poses),
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };

        let glb = GlbExporter::build_with_scene_data("mesh", &attrs, None, Some(&skeleton), None)
            .expect("glb");
        let (json, bin) = parse_glb_json_and_bin(&glb);
        let nodes = json["nodes"].as_array().unwrap();
        let position_accessor = json["meshes"][0]["primitives"][0]["attributes"]["POSITION"]
            .as_u64()
            .unwrap() as usize;
        let positions = read_f32_accessor(&json, bin, position_accessor);

        assert!(nodes[0].get("translation").is_none());
        assert!(nodes[0].get("scale").is_none());
        assert!((positions[0] + 5.0).abs() < 0.0001);
        assert!((positions[3] + 5.01).abs() < 0.0001);
        assert_eq!(nodes.len(), 2);
        assert_eq!(json["skins"][0]["joints"], serde_json::json!([1]));
        assert_eq!(json["skins"][0]["skeleton"], serde_json::json!(1));
        assert_eq!(json["scenes"][0]["nodes"], serde_json::json!([0, 1]));
        assert_all_skin_rest_poses_identity(&json, bin);
    }

    #[test]
    fn skin_joint_parenting_skips_non_skin_intermediate_nodes_without_long_root_bone() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let skeleton = GlbSkeleton {
            joints: vec![
                GlbJoint {
                    name: "model_root".to_string(),
                    path: "model_root".to_string(),
                    transform_path_id: 1,
                    parent: None,
                    children: vec![1],
                    translation: [0.0, -100.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "motion_root".to_string(),
                    path: "model_root/motion_root".to_string(),
                    transform_path_id: 2,
                    parent: Some(0),
                    children: vec![2],
                    translation: [0.0, 80.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "hips".to_string(),
                    path: "model_root/motion_root/hips".to_string(),
                    transform_path_id: 3,
                    parent: Some(1),
                    children: vec![3],
                    translation: [0.0, 20.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "head".to_string(),
                    path: "model_root/motion_root/hips/head".to_string(),
                    transform_path_id: 4,
                    parent: Some(2),
                    children: vec![],
                    translation: [0.0, 2.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            ],
            skin_joints: vec![2, 3],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let bind_pose = vec![
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0,
            0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let bone_weights = vec![1.0, 0.0, 0.0, 0.0];
        let bone_indices = vec![1u16, 0, 0, 0];
        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0],
            indices: &[0],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&bone_weights),
            bone_indices: Some(&bone_indices),
            bind_poses: Some(&bind_pose),
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };

        let glb = GlbExporter::build_with_scene_data("mesh", &attrs, None, Some(&skeleton), None)
            .expect("glb");
        let (json, _) = parse_glb_json_and_bin(&glb);
        let nodes = json["nodes"].as_array().unwrap();

        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[1]["name"], serde_json::json!("hips"));
        assert_eq!(nodes[2]["name"], serde_json::json!("head"));
        assert_eq!(nodes[1]["children"], serde_json::json!([2]));
        assert_eq!(json["skins"][0]["joints"], serde_json::json!([1, 2]));
        assert_eq!(nodes[1]["translation"], serde_json::json!([0.0, 0.0, 0.0]));
        assert_eq!(nodes[2]["translation"], serde_json::json!([0.0, 2.0, 0.0]));
    }

    #[test]
    fn animation_channels_keep_non_skin_parent_joints_exported() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let skeleton = GlbSkeleton {
            joints: vec![
                GlbJoint {
                    name: "root".to_string(),
                    path: "root".to_string(),
                    transform_path_id: 1,
                    parent: None,
                    children: vec![1],
                    translation: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "hips".to_string(),
                    path: "root/hips".to_string(),
                    transform_path_id: 2,
                    parent: Some(0),
                    children: vec![],
                    translation: [0.0, 1.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            ],
            skin_joints: vec![1],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let bind_pose = raw_bind_poses(&[GlbExporter::identity_mat4()]);
        let bone_weights = vec![1.0, 0.0, 0.0, 0.0];
        let bone_indices = vec![0u16, 0, 0, 0];
        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0],
            indices: &[0],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&bone_weights),
            bone_indices: Some(&bone_indices),
            bind_poses: Some(&bind_pose),
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };
        let animations = vec![GlbAnimation {
            name: "root_motion".to_string(),
            channels: vec![GlbAnimationChannel {
                node: 0,
                path: AnimationPath::Translation,
                times: vec![0.0, 1.0],
                values: vec![0.0, 0.0, 0.0, 0.0, 2.0, 0.0],
            }],
        }];

        let glb = GlbExporter::build_with_scene_data(
            "mesh",
            &attrs,
            None,
            Some(&skeleton),
            Some(&animations),
        )
        .expect("glb");
        let (json, _) = parse_glb_json_and_bin(&glb);

        assert_eq!(json["nodes"][1]["name"], serde_json::json!("hips"));
        assert_eq!(json["nodes"][2]["name"], serde_json::json!("root"));
        assert_eq!(json["nodes"][2]["children"], serde_json::json!([1]));
        assert_eq!(json["skins"][0]["joints"], serde_json::json!([1, 2]));
        assert_eq!(
            json["animations"][0]["name"],
            serde_json::json!("root_motion")
        );
        assert_eq!(
            json["animations"][0]["channels"][0]["target"],
            serde_json::json!({ "node": 2, "path": "translation" })
        );
    }

    #[test]
    fn inverse_bind_matrices_follow_bind_pose_order_not_joint_node_index() {
        let skeleton = GlbSkeleton {
            joints: vec![
                GlbJoint {
                    name: "root".to_string(),
                    path: "root".to_string(),
                    transform_path_id: 1,
                    parent: None,
                    children: vec![1, 2, 3],
                    translation: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "ancestor".to_string(),
                    path: "root/ancestor".to_string(),
                    transform_path_id: 2,
                    parent: Some(0),
                    children: vec![],
                    translation: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "bone_a".to_string(),
                    path: "root/bone_a".to_string(),
                    transform_path_id: 3,
                    parent: Some(0),
                    children: vec![],
                    translation: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "bone_b".to_string(),
                    path: "root/bone_b".to_string(),
                    transform_path_id: 4,
                    parent: Some(0),
                    children: vec![],
                    translation: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            ],
            skin_joints: vec![2, 3],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let bind_poses = raw_bind_poses(&[
            gltf_translation_mat4(-2.0, 0.0, 0.0),
            gltf_translation_mat4(-5.0, 0.0, 0.0),
        ]);

        let inverse_bind = GlbExporter::inverse_bind_matrices_from_bind_pose_order(&bind_poses, 2)
            .expect("inverse bind");

        assert_eq!(skeleton.skin_joints, vec![2, 3]);
        assert_eq!(inverse_bind.len(), 32);
        assert!((inverse_bind[12] - 2.0).abs() < 0.0001);
        assert!((inverse_bind[13] - 0.0).abs() < 0.0001);
        assert!((inverse_bind[14] - 0.0).abs() < 0.0001);
        assert!((inverse_bind[28] - 5.0).abs() < 0.0001);
        assert!((inverse_bind[29] - 0.0).abs() < 0.0001);
        assert!((inverse_bind[30] - 0.0).abs() < 0.0001);
    }

    #[test]
    fn skin_export_reorders_skeleton_skin_joints_to_mesh_bone_hash_order() {
        let skeleton = GlbSkeleton {
            joints: vec![
                GlbJoint {
                    name: "root".to_string(),
                    path: "root".to_string(),
                    transform_path_id: 1,
                    parent: None,
                    children: vec![1, 2],
                    translation: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "body".to_string(),
                    path: "root/body".to_string(),
                    transform_path_id: 2,
                    parent: Some(0),
                    children: vec![],
                    translation: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "hat".to_string(),
                    path: "root/hat".to_string(),
                    transform_path_id: 3,
                    parent: Some(0),
                    children: vec![],
                    translation: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            ],
            skin_joints: vec![1, 2],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let bind_poses = raw_bind_poses(&[
            gltf_translation_mat4(-3.0, 0.0, 0.0),
            gltf_translation_mat4(-7.0, 0.0, 0.0),
        ]);
        let bone_name_hashes = vec![
            unity_crc32("hat".as_bytes()),
            unity_crc32("body".as_bytes()),
        ];
        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0],
            indices: &[0],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&[1.0, 0.0, 0.0, 0.0]),
            bone_indices: Some(&[0, 0, 0, 0]),
            bind_poses: Some(&bind_poses),
            bone_name_hashes: Some(&bone_name_hashes),
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };

        let glb = GlbExporter::build_with_scene_data("mesh", &attrs, None, Some(&skeleton), None)
            .expect("glb");
        let (json, bin) = parse_glb_json_and_bin(&glb);

        assert_eq!(json["skins"][0]["joints"], serde_json::json!([1, 2]));
        let inverse_bind_accessor =
            json["skins"][0]["inverseBindMatrices"].as_u64().unwrap() as usize;
        let inverse_bind = read_f32_accessor(&json, bin, inverse_bind_accessor);
        assert!((inverse_bind[12] - 3.0).abs() < 0.0001);
        assert!((inverse_bind[28] - 7.0).abs() < 0.0001);

        let joint_accessor = json["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"]
            .as_u64()
            .unwrap() as usize;
        let joints = read_u16_accessor(&json, bin, joint_accessor);
        assert_eq!(&joints[..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn morph_target_default_weights_are_zero() {
        use crate::exporter::mesh_exporter::MeshBlendShape;

        let blend_shapes = vec![
            MeshBlendShape {
                name: "wide".to_string(),
                delta_vertices: vec![0.1, 0.0, 0.0],
                delta_normals: Vec::new(),
                delta_tangents: Vec::new(),
            },
            MeshBlendShape {
                name: "narrow".to_string(),
                delta_vertices: vec![-0.1, 0.0, 0.0],
                delta_normals: Vec::new(),
                delta_tangents: Vec::new(),
            },
        ];
        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0],
            indices: &[0],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: None,
            bone_indices: None,
            bind_poses: None,
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: Some(&blend_shapes),
        };

        let glb =
            GlbExporter::build_with_scene_data("mesh", &attrs, None, None, None).expect("glb");
        let (json, _) = parse_glb_json_and_bin(&glb);

        assert_eq!(json["meshes"][0]["weights"], serde_json::json!([0.0, 0.0]));
        assert_eq!(
            json["meshes"][0]["extras"]["targetNames"],
            serde_json::json!(["wide", "narrow"])
        );
    }

    #[test]
    fn single_primitive_material_prefers_non_helper_base_texture() {
        let materials = vec![
            MaterialInfo {
                name: "mat_default_character_outline".to_string(),
                textures: vec![TextureSlot {
                    slot_name: "_Base".to_string(),
                    usage: "DiffuseColor".to_string(),
                    file_name: "tex_hatch_lines.png".to_string(),
                    relative_path: "tex_hatch_lines.png".to_string(),
                }],
                ..Default::default()
            },
            MaterialInfo {
                name: "mat_plague_doctor_origin_1".to_string(),
                textures: vec![TextureSlot {
                    slot_name: "_Base".to_string(),
                    usage: "DiffuseColor".to_string(),
                    file_name: "tex_plague_doctor_origin_1_col.png".to_string(),
                    relative_path: "tex_plague_doctor_origin_1_col.png".to_string(),
                }],
                ..Default::default()
            },
        ];

        let index = GlbExporter::preferred_single_primitive_material_index(Some(&materials));

        assert_eq!(index, Some(1));
    }

    #[test]
    fn bind_pose_rest_pose_keeps_skin_matrix_identity_when_transform_rest_pose_differs() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let skeleton = GlbSkeleton {
            joints: vec![
                GlbJoint {
                    name: "root".to_string(),
                    path: "root".to_string(),
                    transform_path_id: 1,
                    parent: None,
                    children: vec![1],
                    translation: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "bone".to_string(),
                    path: "root/bone".to_string(),
                    transform_path_id: 2,
                    parent: Some(0),
                    children: Vec::new(),
                    translation: [2.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            ],
            skin_joints: vec![1],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let bind_poses = raw_bind_poses(&[gltf_translation_mat4(-5.0, 0.0, 0.0)]);
        let bone_weights = vec![1.0, 0.0, 0.0, 0.0];
        let bone_indices = vec![0u16, 0, 0, 0];
        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0],
            indices: &[0],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&bone_weights),
            bone_indices: Some(&bone_indices),
            bind_poses: Some(&bind_poses),
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };

        let glb = GlbExporter::build_with_scene_data("mesh", &attrs, None, Some(&skeleton), None)
            .expect("glb");
        let (json, bin) = parse_glb_json_and_bin(&glb);
        let nodes = json["nodes"].as_array().unwrap();
        let skins = json["skins"].as_array().unwrap();
        let skin = &skins[0];
        let mesh_world = json_node_world_matrix(nodes, 0);
        let joint_node = skin["joints"][0].as_u64().unwrap() as usize;
        let joint_world = json_node_world_matrix(nodes, joint_node);
        let inverse_mesh_world = GlbExporter::invert_mat4(&mesh_world).expect("mesh inverse");
        let inverse_bind_accessor = skin["inverseBindMatrices"].as_u64().unwrap() as usize;
        let inverse_bind = read_f32_accessor(&json, bin, inverse_bind_accessor);
        let mut inverse_bind_matrix = [0.0f32; 16];
        inverse_bind_matrix.copy_from_slice(&inverse_bind[..16]);
        let skin_matrix = GlbExporter::mul_mat4(
            &GlbExporter::mul_mat4(&inverse_mesh_world, &joint_world),
            &inverse_bind_matrix,
        );

        assert_eq!(
            nodes[joint_node]["extras"]["restPoseSource"],
            serde_json::json!("bindPose")
        );
        assert_mat4_near(&skin_matrix, &GlbExporter::identity_mat4());
    }

    #[test]
    fn skin_root_does_not_become_duplicate_scene_root() {
        use crate::exporter::mesh_exporter::MeshAttributes;

        let skeleton = GlbSkeleton {
            joints: vec![
                GlbJoint {
                    name: "model_root".to_string(),
                    path: "model_root".to_string(),
                    transform_path_id: 1,
                    parent: None,
                    children: vec![1],
                    translation: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                GlbJoint {
                    name: "hips".to_string(),
                    path: "model_root/hips".to_string(),
                    transform_path_id: 2,
                    parent: Some(0),
                    children: vec![],
                    translation: [0.0, 1.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            ],
            skin_joints: vec![1],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(1),
            roots: vec![0],
        };
        let bind_poses = raw_bind_poses(&[gltf_translation_mat4(0.0, -1.0, 0.0)]);
        let bone_weights = vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let bone_indices = vec![0u16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let attrs = MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: &[0, 1, 2],
            normals: None,
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: Some(&bone_weights),
            bone_indices: Some(&bone_indices),
            bind_poses: Some(&bind_poses),
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };

        let glb = GlbExporter::build_with_scene_data("mesh", &attrs, None, Some(&skeleton), None)
            .expect("glb");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();

        assert_eq!(json["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(json["skins"][0]["joints"], serde_json::json!([1]));
        assert_eq!(json["skins"][0]["skeleton"], serde_json::json!(1));
        assert_eq!(json["scenes"][0]["nodes"], serde_json::json!([0, 1]));
    }

    fn assert_mat4_near(actual: &[f32; 16], expected: &[f32; 16]) {
        for (i, (actual, expected)) in actual.iter().zip(expected.iter()).enumerate() {
            assert!(
                (actual - expected).abs() < 0.0001,
                "matrix mismatch at {}: {} != {}",
                i,
                actual,
                expected
            );
        }
    }

    fn gltf_translation_mat4(x: f32, y: f32, z: f32) -> [f32; 16] {
        [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, x, y, z, 1.0,
        ]
    }

    fn raw_bind_poses(matrices: &[[f32; 16]]) -> Vec<f32> {
        matrices
            .iter()
            .flat_map(GlbExporter::transpose_mat4)
            .collect()
    }

    fn assert_all_skin_rest_poses_identity(json: &serde_json::Value, bin: &[u8]) {
        let nodes = json["nodes"].as_array().unwrap();
        let skins = json["skins"].as_array().unwrap();
        for (mesh_node_index, node) in nodes.iter().enumerate() {
            let Some(skin_index) = node["skin"].as_u64().map(|value| value as usize) else {
                continue;
            };
            let skin = &skins[skin_index];
            let inverse_bind_accessor = skin["inverseBindMatrices"].as_u64().unwrap() as usize;
            let inverse_bind = read_f32_accessor(json, bin, inverse_bind_accessor);
            let mesh_world = json_node_world_matrix(nodes, mesh_node_index);
            let inverse_mesh_world = GlbExporter::invert_mat4(&mesh_world).expect("mesh inverse");
            for (skin_order, joint_node) in skin["joints"].as_array().unwrap().iter().enumerate() {
                let joint_node = joint_node.as_u64().unwrap() as usize;
                let joint_world = json_node_world_matrix(nodes, joint_node);
                let mut inverse_bind_matrix = [0.0f32; 16];
                inverse_bind_matrix
                    .copy_from_slice(&inverse_bind[skin_order * 16..skin_order * 16 + 16]);
                let skin_matrix = GlbExporter::mul_mat4(
                    &GlbExporter::mul_mat4(&inverse_mesh_world, &joint_world),
                    &inverse_bind_matrix,
                );
                assert_mat4_near(&skin_matrix, &GlbExporter::identity_mat4());
            }
        }
    }

    fn parse_glb_json_and_bin(glb: &[u8]) -> (serde_json::Value, &[u8]) {
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
        let bin_header_offset = 20 + json_len;
        let bin_len = u32::from_le_bytes(
            glb[bin_header_offset..bin_header_offset + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let bin_start = bin_header_offset + 8;
        (json, &glb[bin_start..bin_start + bin_len])
    }

    fn json_node_world_matrix(nodes: &[serde_json::Value], index: usize) -> [f32; 16] {
        let node = &nodes[index];
        let local = if let Some(matrix) = node["matrix"].as_array() {
            let mut out = [0.0f32; 16];
            for (index, value) in matrix.iter().enumerate().take(16) {
                out[index] = value.as_f64().unwrap() as f32;
            }
            out
        } else {
            let translation = json_vec3(node.get("translation"), [0.0, 0.0, 0.0]);
            let rotation = json_vec4(node.get("rotation"), [0.0, 0.0, 0.0, 1.0]);
            let scale = json_vec3(node.get("scale"), [1.0, 1.0, 1.0]);
            GlbExporter::compose_trs(translation, rotation, scale)
        };
        let parent = nodes.iter().enumerate().find_map(|(parent_index, parent)| {
            parent["children"]
                .as_array()
                .is_some_and(|children| {
                    children
                        .iter()
                        .any(|child| child.as_u64() == Some(index as u64))
                })
                .then_some(parent_index)
        });
        if let Some(parent) = parent {
            GlbExporter::mul_mat4(&json_node_world_matrix(nodes, parent), &local)
        } else {
            local
        }
    }

    fn json_vec3(value: Option<&serde_json::Value>, default: [f32; 3]) -> [f32; 3] {
        let Some(array) = value.and_then(|value| value.as_array()) else {
            return default;
        };
        [
            array
                .first()
                .and_then(|v| v.as_f64())
                .unwrap_or(default[0] as f64) as f32,
            array
                .get(1)
                .and_then(|v| v.as_f64())
                .unwrap_or(default[1] as f64) as f32,
            array
                .get(2)
                .and_then(|v| v.as_f64())
                .unwrap_or(default[2] as f64) as f32,
        ]
    }

    fn json_vec4(value: Option<&serde_json::Value>, default: [f32; 4]) -> [f32; 4] {
        let Some(array) = value.and_then(|value| value.as_array()) else {
            return default;
        };
        [
            array
                .first()
                .and_then(|v| v.as_f64())
                .unwrap_or(default[0] as f64) as f32,
            array
                .get(1)
                .and_then(|v| v.as_f64())
                .unwrap_or(default[1] as f64) as f32,
            array
                .get(2)
                .and_then(|v| v.as_f64())
                .unwrap_or(default[2] as f64) as f32,
            array
                .get(3)
                .and_then(|v| v.as_f64())
                .unwrap_or(default[3] as f64) as f32,
        ]
    }

    fn read_f32_accessor(json: &serde_json::Value, bin: &[u8], accessor_index: usize) -> Vec<f32> {
        let accessor = &json["accessors"][accessor_index];
        let buffer_view_index = accessor["bufferView"].as_u64().unwrap() as usize;
        let buffer_view = &json["bufferViews"][buffer_view_index];
        let byte_offset = buffer_view["byteOffset"].as_u64().unwrap_or(0) as usize
            + accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
        let count = accessor["count"].as_u64().unwrap() as usize;
        let component_count = match accessor["type"].as_str().unwrap() {
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT4" => 16,
            other => panic!("unsupported accessor type {}", other),
        };
        (0..count * component_count)
            .map(|index| {
                let start = byte_offset + index * 4;
                f32::from_le_bytes(bin[start..start + 4].try_into().unwrap())
            })
            .collect()
    }

    fn read_u16_accessor(json: &serde_json::Value, bin: &[u8], accessor_index: usize) -> Vec<u16> {
        let accessor = &json["accessors"][accessor_index];
        let buffer_view_index = accessor["bufferView"].as_u64().unwrap() as usize;
        let buffer_view = &json["bufferViews"][buffer_view_index];
        let byte_offset = buffer_view["byteOffset"].as_u64().unwrap_or(0) as usize
            + accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
        let count = accessor["count"].as_u64().unwrap() as usize;
        let component_count = match accessor["type"].as_str().unwrap() {
            "VEC4" => 4,
            other => panic!("unsupported accessor type {}", other),
        };
        (0..count * component_count)
            .map(|index| {
                let start = byte_offset + index * 2;
                u16::from_le_bytes(bin[start..start + 2].try_into().unwrap())
            })
            .collect()
    }

    fn unity_crc32(data: &[u8]) -> u32 {
        let mut value = 0xFFFF_FFFFu32;
        for byte in data {
            let mut table_value = ((value as u8) ^ *byte) as u32;
            for _ in 0..8 {
                table_value = if table_value & 1 != 0 {
                    (table_value >> 1) ^ 0xEDB8_8320
                } else {
                    table_value >> 1
                };
            }
            value = (value >> 8) ^ table_value;
        }
        !value
    }
}
