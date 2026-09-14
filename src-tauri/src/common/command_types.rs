/*
 * command_types.rs - Shared types returned by Tauri command handlers.
 *
 * These data structures live outside src/commands so command modules can stay focused on
 * #[tauri::command] entry points, while service code can reuse the same response shapes.
 */

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewSelectionOptions {
    #[serde(default = "default_preview_option_enabled")]
    pub models: bool,
    #[serde(default = "default_preview_option_enabled")]
    pub textures: bool,
    #[serde(default = "default_preview_option_enabled")]
    pub animations: bool,
    #[serde(default, alias = "selectedAnimationClip")]
    pub selected_animation_clip: Option<PreviewAnimationClipSelection>,
    #[serde(default)]
    pub eager_animations: bool,
}

fn default_preview_option_enabled() -> bool {
    true
}

impl Default for PreviewSelectionOptions {
    fn default() -> Self {
        Self {
            models: true,
            textures: true,
            animations: true,
            selected_animation_clip: None,
            eager_animations: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewAnimationClipSelection {
    pub bundle_path: String,
    pub path_id: String,
}

// ================================================================
// Dump results
// ================================================================

/**
 * Dump result - structured dump data returned to the frontend.
 */
#[derive(Debug, Clone, Serialize)]
pub struct DumpResult {
    /// Formatted TypeTree dump text
    pub dump_text: String,
    /// Whether truncation occurred (depth/array count/string length)
    pub truncated: bool,
    /// Number of truncated items
    pub truncated_items: usize,
    /// Whether a TypeTree exists (false = used hex dump)
    pub has_typetree: bool,
}

// ================================================================
// Texture preview results
// ================================================================

/// Texture export result (returns both path and metadata, for frontend hover info)
#[derive(Debug, Serialize)]
pub struct TexturePreviewResult {
    pub png_path: String,
    pub png_data_url: String,
    pub preview_width: u32,
    pub preview_height: u32,
    pub width: u32,
    pub height: u32,
    pub texture_format: i32,
    pub texture_format_name: String,
    pub complete_image_size: i32,
    pub mip_count: i32,
    pub image_count: i32,
    pub texture_dimension: i32,
    pub texture_dimension_name: String,
    pub byte_size: u32,
    pub has_alpha: bool,
    pub color_space: i32,
    pub color_space_name: String,
    pub filter_mode: i32,
    pub filter_mode_name: String,
    pub aniso: i32,
    pub wrap_u: i32,
    pub wrap_v: i32,
    pub wrap_mode_name: String,
    pub is_readable: bool,
    pub streaming_mipmaps: bool,
}

// ================================================================
// Mesh textured preview results
// ================================================================

/// Diffuse/BaseColor texture chosen for Mesh preview.
#[derive(Debug, Clone, Serialize)]
pub struct MeshPreviewDiffuseTextureResult {
    pub png_path: String,
    pub texture_path_id: String,
    pub texture_name: String,
    pub texture_bundle_path: String,
    pub material_name: String,
    pub material_index: usize,
    pub slot_name: String,
    pub usage: String,
}

/// Texture candidate shown in the Mesh preview texture picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshPreviewTextureCandidate {
    pub png_path: String,
    pub texture_path_id: String,
    pub texture_name: String,
    pub texture_bundle_path: String,
    pub material_name: String,
    pub material_index: usize,
    pub slot_name: String,
    pub usage: String,
    pub width: u32,
    pub height: u32,
    pub texture_format: i32,
    pub texture_format_name: String,
    pub byte_size: u32,
    pub has_alpha: bool,
    pub color_space_name: String,
    pub wrap_mode_name: String,
    pub filter_mode_name: String,
}

/// AnimationClip reference resolved for the progressive preview drawer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewAnimationClipRef {
    pub bundle_path: String,
    pub path_id: String,
    pub class_name: String,
    pub name: String,
    pub byte_size: u32,
}

/// Animator/Animation component reference resolved for the progressive preview drawer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewAnimatorRef {
    pub bundle_path: String,
    pub path_id: String,
    pub class_name: String,
    pub name: String,
    pub byte_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewMeshPartRef {
    pub name: String,
    pub bundle_path: String,
    pub path_id: String,
    pub class_name: String,
}

/// Animator preview GLB generated for the right-side interactive viewer.
#[derive(Debug, Clone, Serialize)]
pub struct AnimatorPreviewGlbResult {
    pub success: bool,
    pub error: String,
    pub glb_path: String,
    pub mesh_count: usize,
    pub material_count: usize,
    pub texture_count: usize,
    pub skeleton_joint_count: usize,
    pub animation_count: usize,
    pub animation_names: Vec<String>,
    pub mesh_part_names: Vec<String>,
    #[serde(default)]
    pub mesh_parts: Vec<PreviewMeshPartRef>,
    pub vertex_count: usize,
    pub triangle_count: usize,
    pub texture_candidates: Vec<MeshPreviewTextureCandidate>,
}

// ================================================================
// Mesh dependency export results
// ================================================================

/// Export result
#[derive(Debug, Clone, Serialize)]
pub struct DependencyExportResult {
    /// Exported primary file path
    pub output_path: String,
    /// Exported GLB file path. Empty for non-mesh dependency exports.
    pub glb_path: String,
    /// Exported texture file paths
    pub texture_paths: Vec<String>,
    /// Number of associated Materials
    pub material_count: usize,
    /// Number of associated Texture2Ds
    pub texture_count: usize,
    /// Vertex count
    pub vertex_count: usize,
    /// Triangle count
    pub triangle_count: usize,
}

/// One asset that can be offered in the "export related assets" picker.
#[derive(Debug, Clone, Serialize)]
pub struct DependencyExportAssetRef {
    pub bundle_path: String,
    pub path_id: String,
    pub class_name: String,
    pub asset_name: String,
    pub byte_size: u32,
}
