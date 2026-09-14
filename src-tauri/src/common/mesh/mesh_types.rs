/*
 * mesh_types.rs - Mesh related shared types.
 *
 * Split from types.rs, follows Soul.md conventions: each .rs contains related types only.
 */

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MeshSubMeshInfo {
    pub index_start: usize,
    pub index_count: usize,
    pub topology: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MeshBlendShapeInfo {
    pub name: String,
    pub weight: f32,
    pub delta_vertices: Vec<f32>,
    pub delta_normals: Vec<f32>,
    pub delta_tangents: Vec<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MeshGeometryPart {
    pub name: String,
    pub bundle_path: String,
    pub path_id: i64,
    pub vertices: Vec<f32>,
    #[serde(default)]
    pub normals: Vec<f32>,
    #[serde(default)]
    pub uvs: Vec<f32>,
    pub indices: Vec<u32>,
    #[serde(default)]
    pub sub_meshes: Vec<MeshSubMeshInfo>,
    pub vertex_count: usize,
    pub triangle_count: usize,
}

/// Summary info for a Mesh asset
#[derive(Debug, Clone, Serialize)]
pub struct MeshInfo {
    pub byte_size: usize,
    pub guessed_vertex_count: usize,
    pub guessed_triangle_count: usize,
    pub has_indices: bool,
    pub parse_success: bool,
}

/// Extracted Mesh geometry data + metadata
///
/// Contains vertex/index arrays as well as vertex/triangle count metadata,
/// so the frontend doesn't need to call get_mesh_info again,
/// avoiding redundant bundle loading.
#[derive(Debug, Clone, Serialize)]
pub struct MeshGeometry {
    pub vertices: Vec<f32>,
    #[serde(default)]
    pub normals: Vec<f32>,
    #[serde(default)]
    pub uvs: Vec<f32>,
    #[serde(default, skip_serializing)]
    pub tangents: Vec<f32>,
    #[serde(default, skip_serializing)]
    pub colors: Vec<f32>,
    #[serde(default, skip_serializing)]
    pub bone_weights: Vec<f32>,
    #[serde(default, skip_serializing)]
    pub bone_indices: Vec<u16>,
    #[serde(default, skip_serializing)]
    pub bind_poses: Vec<f32>,
    #[serde(default, skip_serializing)]
    pub bone_name_hashes: Vec<u32>,
    #[serde(default, skip_serializing)]
    pub root_bone_name_hash: Option<u32>,
    #[serde(default)]
    pub sub_meshes: Vec<MeshSubMeshInfo>,
    #[serde(default)]
    pub parts: Vec<MeshGeometryPart>,
    #[serde(default, skip_serializing)]
    pub blend_shapes: Vec<MeshBlendShapeInfo>,
    pub indices: Vec<u32>,
    pub success: bool,
    pub error: String,
    /// Vertex count (computed directly from parse result)
    pub vertex_count: usize,
    /// Triangle count (computed directly from parse result)
    pub triangle_count: usize,
}
