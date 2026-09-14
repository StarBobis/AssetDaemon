use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;

/// Bundle parse result used while building the file-backed asset map.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BundleEntry {
    pub path: String,
    pub md5: String,
    pub file_size: u64,
    pub unity_version: String,
    pub asset_count: usize,
    pub assets: Vec<AssetEntry>,
    pub containers: HashMap<String, i64>,
    pub externals: Vec<(usize, i32, String)>,
    pub internal_names: Vec<(String, String)>,
    pub relations: Vec<BundleRelationEntry>,
}

/// Single asset record used while building the file-backed asset map.
///
/// `class_name` 使用 `Cow<'static, str>` 避免为已知 class_id 重复堆分配。
#[derive(Debug, Clone)]
pub struct AssetEntry {
    pub path_id: i64,
    pub class_id: i32,
    pub class_name: Cow<'static, str>,
    pub asset_name: String,
    pub byte_size: u32,
}

/// Generic relationship record stored in the file-backed asset map.
///
/// `relation_type` 和 `field_path` 使用 `Cow<'static, str>` 而非 `String`，
/// 避免为 `&'static str` 常量（如 "gameobject_component"、"m_Component"）重复堆分配。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BundleRelationEntry {
    pub relation_type: Cow<'static, str>,
    pub source_path_id: i64,
    pub target_path_id: i64,
    pub source_name: String,
    pub target_name: String,
    pub file_id: i32,
    pub field_path: Cow<'static, str>,
    pub target_bundle_path: String,
}

/// Map Summary (lightweight struct for frontend state display)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapSummary {
    /// Build time (Unix seconds)
    pub built_at: u64,
    /// Build time (human-readable, e.g. "2024-01-15 14:30:00")
    pub built_at_formatted: String,
    /// Total number of bundles
    pub bundle_count: usize,
    /// Total number of assets
    pub asset_count: usize,
    /// Number of successfully parsed bundles
    pub parsed_count: usize,
    /// Whether the last build was stopped before all bundles were processed
    pub cancelled: bool,
}

/// Lightweight asset row returned to the All Assets tab from AssetMap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapAssetSummary {
    pub path: String,
    pub name: String,
    pub class_name: String,
    pub class_id: i32,
    pub path_id: String,
    pub byte_size: u32,
    pub source_bundle_path: String,
}

/// Asset type count row returned for the All Assets type filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapAssetClassStat {
    pub name: String,
    pub count: usize,
}

/// Status of the optional trigram FTS index used to accelerate All Assets text filters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetSearchIndexStatus {
    pub ready: bool,
    pub asset_count: usize,
    pub indexed_count: usize,
    pub indexed_asset_count: Option<usize>,
}

/// Query options accepted by the All Assets search command.
#[derive(Debug, Clone, Deserialize)]
pub struct MapAssetQueryOptions {
    pub search: String,
    pub class_names: Vec<String>,
    pub bundle_path: Option<String>,
    pub name_query: Option<String>,
    pub name_match_mode: Option<String>,
    pub name_include_queries: Option<Vec<String>>,
    pub name_exclude_query: Option<String>,
    pub name_exclude_queries: Option<Vec<String>>,
    pub bundle_query: Option<String>,
    pub path_id_query: Option<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub offset: usize,
    pub limit: usize,
    pub sort_by: Option<String>,
    pub sort_direction: Option<String>,
}

/// Paged search result returned by the All Assets search command.
#[derive(Debug, Clone, Serialize)]
pub struct MapAssetQueryResult {
    pub assets: Vec<MapAssetSummary>,
    pub total: usize,
    pub has_more: bool,
    pub offset: usize,
    pub limit: usize,
}
