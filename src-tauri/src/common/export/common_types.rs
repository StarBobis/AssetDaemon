/*
 * export_types.rs - Shared type definitions for batch export functionality.
 *
 * Contains the export format enum, export task descriptions, progress events, and export reports.
 * These types are shared between the commands layer and export_service.
 *
 * Follows Soul.md conventions: organized via struct + enum, no free functions.
 */

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ================================================================
// ExportFormat - Supported export formats
// ================================================================

/// Output formats supported per asset type.
/// Maps to AssetStudio's export format dropdown.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    // ---- Textures / Sprites (Unity native DDS, lossless direct output) ----
    Dds,
    // ---- Sprites (cropped visual output matching preview) ----
    Png,
    // ---- Texture decoded formats ----
    Tga,

    // ---- Meshes ----
    Glb,
    Obj,

    // ---- Audio ----
    Wav,
    // Mp3,    // Future: needs encoder
    // Ogg,    // Future: needs encoder

    // ---- Text assets ----
    Txt,
    Json,
    Bytes,

    // ---- Fonts ----
    Ttf,
    Otf,

    // ---- Shaders ----
    Shader,
    Bin,

    // ---- MonoBehaviour ----
    JsonTree,

    // ---- Video ----
    Ogv,
    BinRaw,

    // ---- Generic fallback ----
    Raw,
}

#[cfg(test)]
mod tests {
    use super::{ExportFormat, ExportFormatResolver};

    #[test]
    fn audio_clip_defaults_to_raw_until_wav_conversion_exists() {
        assert_eq!(
            ExportFormatResolver::default_formats_for_class("AudioClip").first(),
            Some(&ExportFormat::Raw)
        );
    }

    #[test]
    fn game_object_defaults_to_glb_like_preview() {
        assert_eq!(
            ExportFormatResolver::default_formats_for_class("GameObject"),
            vec![ExportFormat::Glb, ExportFormat::JsonTree, ExportFormat::Raw]
        );
    }
}

impl ExportFormat {
    /// Get the file extension (without dot).
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Dds => "dds",
            Self::Png => "png",
            Self::Tga => "tga",
            Self::Glb => "glb",
            Self::Obj => "obj",
            Self::Wav => "wav",
            Self::Txt => "txt",
            Self::Json => "json",
            Self::Bytes => "bytes",
            Self::Ttf => "ttf",
            Self::Otf => "otf",
            Self::Shader => "shader",
            Self::Bin => "bin",
            Self::JsonTree => "json", // TypeTree JSON (MonoBehaviour) - same suffix as Json but different use case
            Self::Ogv => "ogv",
            Self::BinRaw => "bin",
            Self::Raw => "dat",
        }
    }
}

// ================================================================
// ExportGroupBy - Export file organization method
// ================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportGroupBy {
    /// All files flat in the output directory
    None,
    /// Group by asset type folder: output/Texture2D/, output/Mesh/, ...
    ByType,
    /// Group by Unity container path: output/Assets/Resources/...
    ByContainer,
}

// ================================================================
// ExportOptions - Export options
// ================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOptions {
    /// Absolute path to output directory
    pub output_dir: String,
    /// File organization method
    pub group_by: ExportGroupBy,
    /// Whether to overwrite existing files
    pub overwrite_existing: bool,
    /// Whether to generate a CSV/JSON export report
    pub generate_report: bool,
    /// Whether GLB-style heuristic export may resolve and attach Materials.
    #[serde(default = "default_include_heuristic_content")]
    pub include_materials: bool,
    /// Whether GLB-style heuristic export may resolve, export, and attach textures.
    #[serde(default = "default_include_heuristic_content")]
    pub include_textures: bool,
    /// Whether GLB-style heuristic export may resolve and embed skeleton/skin data.
    #[serde(default = "default_include_heuristic_content")]
    pub include_skeleton: bool,
    /// Whether GLB-style heuristic export may scan for and embed AnimationClip data.
    #[serde(default = "default_include_heuristic_content")]
    pub include_animations: bool,
    /// Whether Mesh GLB export writes each SubMesh as its own file.
    #[serde(default)]
    pub split_submeshes: bool,
    /// Workspace directories used by preview dependency resolution.
    #[serde(default)]
    pub workspace_dirs: Vec<String>,
    /// Global AssetMap cache root used by preview dependency resolution.
    #[serde(default)]
    pub asset_map_cache_root: Option<String>,
    /// Asset type -> format override: class_name -> ExportFormat
    /// Unspecified types use the default format for that type
    pub format_overrides: HashMap<String, ExportFormat>,
}

fn default_include_heuristic_content() -> bool {
    true
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            output_dir: String::new(),
            group_by: ExportGroupBy::None,
            overwrite_existing: false,
            generate_report: false,
            include_materials: true,
            include_textures: true,
            include_skeleton: true,
            include_animations: true,
            split_submeshes: false,
            workspace_dirs: Vec::new(),
            asset_map_cache_root: None,
            format_overrides: HashMap::new(),
        }
    }
}

// ================================================================
// ExportAssetRef - Reference to a single asset to export
// ================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportAssetRef {
    /// Absolute path to the bundle containing this asset
    pub bundle_path: String,
    /// Unity internal path_id (i64, serialized as string to avoid JS precision loss)
    pub path_id: String,
    /// Asset type name (e.g. "Texture2D", "Mesh")
    pub class_name: String,
    /// Asset instance name (m_Name field value)
    pub asset_name: String,
}

// ================================================================
// ExportJob - Batch export task
// ================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportJob {
    /// Task unique identifier (UUID generated by frontend)
    pub job_id: String,
    /// List of assets to export
    pub assets: Vec<ExportAssetRef>,
    /// Export options
    pub options: ExportOptions,
}

// ================================================================
// ExportResult - Export result for a single asset
// ================================================================

#[derive(Debug, Clone, Serialize)]
pub struct ExportResult {
    /// Asset instance name
    pub asset_name: String,
    /// Type name
    pub class_name: String,
    /// path_id string
    pub path_id: String,
    /// Actual written output file path
    pub output_file: String,
    /// Export format used
    pub format: ExportFormat,
    /// Whether it succeeded
    pub success: bool,
    /// Error message (on failure)
    pub error: Option<String>,
    /// Output file size in bytes
    pub byte_size: u64,
    /// Export duration (milliseconds)
    pub duration_ms: u64,
}

// ================================================================
// ExportProgress - Batch export progress event
// ================================================================

#[derive(Debug, Clone, Serialize)]
pub struct ExportProgress {
    /// Task ID
    pub job_id: String,
    /// Completed count
    pub done: usize,
    /// Total count
    pub total: usize,
    /// Currently processing asset name
    pub current_asset: String,
    /// Current step description
    pub current_step: String,
    /// Completed results (in order of completion)
    pub completed: Vec<ExportResult>,
    /// Whether cancelled
    pub cancelled: bool,
}

// ================================================================
// ExportReport - Batch export completion report
// ================================================================

#[derive(Debug, Clone, Serialize)]
pub struct ExportReport {
    /// Task ID
    pub job_id: String,
    /// Total assets
    pub total: usize,
    /// Succeeded count
    pub succeeded: usize,
    /// Failed count
    pub failed: usize,
    /// All results
    pub results: Vec<ExportResult>,
    /// Output directory
    pub output_directory: String,
    /// Total duration (milliseconds)
    pub duration_ms: u64,
}

// ================================================================
// Helper - Get default export format for an asset type
// ================================================================

/**
 * Export format query utility class.
 *
 * Encapsulates server-side default export format selection.
 * Follows Soul.md conventions: struct + impl organization, no free functions.
 */
pub struct ExportFormatResolver;

impl ExportFormatResolver {
    /// Return recommended default export format list for a Unity class_name.
    /// The first format is "most recommended", frontend selects it by default.
    pub fn default_formats_for_class(class_name: &str) -> Vec<ExportFormat> {
        match class_name {
            "Texture2D" => vec![ExportFormat::Dds, ExportFormat::Png, ExportFormat::Tga],
            "Sprite" | "SpriteMask" => vec![ExportFormat::Png],
            "Mesh" => vec![ExportFormat::Glb, ExportFormat::Obj],
            "GameObject" => vec![ExportFormat::Glb, ExportFormat::JsonTree, ExportFormat::Raw],
            "AudioClip" => vec![ExportFormat::Raw],
            "TextAsset" => vec![ExportFormat::Txt, ExportFormat::Json, ExportFormat::Bytes],
            "Font" => vec![ExportFormat::Ttf, ExportFormat::Otf],
            "Shader" => vec![ExportFormat::Shader, ExportFormat::Bin],
            "MonoBehaviour" => vec![ExportFormat::JsonTree],
            "AnimationClip"
            | "Avatar"
            | "RuntimeAnimatorController"
            | "AnimatorController"
            | "AnimatorOverrideController" => vec![ExportFormat::JsonTree, ExportFormat::Raw],
            "Animator" => vec![ExportFormat::Glb, ExportFormat::JsonTree, ExportFormat::Raw],
            "VideoClip" | "MovieTexture" => vec![ExportFormat::Ogv, ExportFormat::BinRaw],
            _ => vec![ExportFormat::Raw],
        }
    }
}
