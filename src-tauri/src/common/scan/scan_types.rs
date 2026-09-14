/*
 * scan_types.rs - Batch scan related shared types.
 *
 * Split from types.rs, follows Soul.md conventions: each .rs contains related types only.
 */

use serde::{Deserialize, Serialize};

use crate::common::command_types::{MeshPreviewTextureCandidate, PreviewMeshPartRef};

/// Type scan result for a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTypeResult {
    pub path: String,
    pub md5: String,
    pub types: Vec<String>,
}

/// Scan progress event
#[derive(Debug, Clone, Serialize)]
pub struct ScanProgress {
    pub done: usize,
    pub total: usize,
    pub current: String,
}

/// Extraction progress event (Mesh geometry extraction stages)
#[derive(Debug, Clone, Serialize)]
pub struct ProgressPayload {
    pub step: String,
    pub message: String,
}

/// Incremental preview result sent progressively after each extraction phase.
/// Frontend receives these via a separate channel and updates the preview UI immediately.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncrementalPreviewPayload {
    /// Which phase just completed: "mesh", "textures", "materials", "animation", "glb_generated"
    pub phase: String,
    /// GLB file path (set when a new GLB is available)
    #[serde(default)]
    pub glb_path: Option<String>,
    /// Cumulative counts so far
    #[serde(default)]
    pub vertex_count: Option<usize>,
    #[serde(default)]
    pub triangle_count: Option<usize>,
    #[serde(default)]
    pub mesh_count: Option<usize>,
    #[serde(default)]
    pub material_count: Option<usize>,
    #[serde(default)]
    pub texture_count: Option<usize>,
    #[serde(default)]
    pub skeleton_joint_count: Option<usize>,
    #[serde(default)]
    pub animation_count: Option<usize>,
    #[serde(default)]
    pub animation_names: Option<Vec<String>>,
    #[serde(default)]
    pub mesh_part_names: Option<Vec<String>>,
    #[serde(default)]
    pub mesh_parts: Option<Vec<PreviewMeshPartRef>>,
    #[serde(default)]
    pub texture_candidates: Option<Vec<MeshPreviewTextureCandidate>>,
}

/// Partial preview result sent progressively after each extraction phase.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct IncrementalPreviewData {
    /// Which phase just completed: "mesh", "textures", "materials", "animation", "glb_generated"
    pub phase: String,
    /// GLB file path (set when a new GLB is available)
    #[serde(default)]
    pub glb_path: Option<String>,
    /// Cumulative counts so far
    #[serde(default)]
    pub vertex_count: Option<usize>,
    #[serde(default)]
    pub triangle_count: Option<usize>,
    #[serde(default)]
    pub mesh_count: Option<usize>,
    #[serde(default)]
    pub material_count: Option<usize>,
    #[serde(default)]
    pub texture_count: Option<usize>,
    #[serde(default)]
    pub skeleton_joint_count: Option<usize>,
    #[serde(default)]
    pub animation_count: Option<usize>,
    #[serde(default)]
    pub animation_names: Option<Vec<String>>,
    #[serde(default)]
    pub mesh_part_names: Option<Vec<String>>,
    #[serde(default)]
    pub mesh_parts: Option<Vec<PreviewMeshPartRef>>,
}
