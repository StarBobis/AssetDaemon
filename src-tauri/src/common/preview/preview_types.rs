use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AssetPreviewResult {
    pub class_name: String,
    pub path_id: String,
    pub name: String,
    pub unity_version: String,
    pub byte_size: u32,
    pub sections: Vec<PreviewSection>,
    pub relations: Vec<PreviewRelation>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewSection {
    pub title: String,
    pub kind: String,
    pub rows: Vec<PreviewRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewRow {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewRelation {
    pub relation_type: String,
    pub field_path: String,
    pub direction: String,
    pub bundle_path: String,
    pub path_id: String,
    pub class_name: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct PreviewAssetRef {
    pub bundle_path: String,
    pub path_id: i64,
    pub class_name: String,
    pub name: String,
}
