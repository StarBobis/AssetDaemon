/*
 * bundle_types.rs - Bundle file / node / asset related shared types.
 *
 * Split from types.rs, follows Soul.md conventions: each .rs contains related types only.
 */

use serde::Serialize;

// ============================================================
// Bundle file / node / asset summaries
// ============================================================

/// A .bundle file entry in the filesystem
#[derive(Debug, Clone, Serialize)]
pub struct BundleFileEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified_ms: u64,
    pub modified: String,
}

/// Directory node info for a Bundle (corresponds to UnityFS DirectoryNode)
#[derive(Debug, Clone, Serialize)]
pub struct BundleNodeInfo {
    pub name: String,
    pub offset: u64,
    pub size: u64,
}

/// Summary info for an asset inside a Bundle
#[derive(Debug, Clone, serde::Serialize)]
pub struct AssetSummary {
    pub path: String,
    pub name: String,
    pub class_name: String,
    pub class_id: i32,
    #[serde(serialize_with = "I64StringSerializer::serialize")]
    pub path_id: i64,
    pub byte_size: u32,
}

/**
 * Serializer that serializes i64 as a string.
 *
 * A Serde custom serialization function that serializes i64 values as JSON strings
 * (rather than numbers) to avoid JavaScript-side precision loss.
 */
struct I64StringSerializer;

impl I64StringSerializer {
    fn serialize<S>(val: &i64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(val)
    }
}

/// Metadata returned after parsing a Bundle
#[derive(Debug, Clone, Serialize)]
pub struct BundleMeta {
    pub path: String,
    pub size: u64,
    pub compressed: bool,
    pub unity_version: String,
    pub nodes: Vec<BundleNodeInfo>,
    pub assets: Vec<AssetSummary>,
    /// File parse diagnostics (name, size, first 16 bytes hex, whether parsed as SerializedFile)
    pub files_diag: Vec<FileDiag>,
    /// Full step-by-step diagnostic logs (chronological record of each key step)
    #[serde(default)]
    pub parse_logs: Vec<String>,
}

/// Bundle parse result returned asynchronously via Channel
#[derive(Debug, Clone, Serialize)]
pub struct BundleParseChannelResult {
    pub success: bool,
    pub meta: Option<BundleMeta>,
    pub error: Option<String>,
}

/// Parse diagnostics for a single file inside a Bundle
#[derive(Debug, Clone, Serialize)]
pub struct FileDiag {
    pub name: String,
    pub size_bytes: usize,
    pub first_bytes_hex: String,
    pub parse_ok: bool,
    pub parse_error: String,
}

/// Info returned after extracting a single node from a Bundle
#[derive(Debug, Clone, Serialize)]
pub struct ExtractedNode {
    pub file_path: String,
    pub size: u64,
}
