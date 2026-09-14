/*
 * mono_behaviour_exporter.rs - MonoBehaviour JSON export utility class.
 */

use crate::common::serialized_file::object_reader::ObjectReader;
use crate::common::serialized_file::serialized_file;
use std::fs;
use std::path::Path;

pub struct MonoBehaviourExporter;

impl MonoBehaviourExporter {
    /// Export MonoBehaviour as JSON.
    /// Prefer TypeTree structured export, fall back to base64 when no TypeTree.
    pub fn export_with_typetree(
        raw_data: &[u8],
        sf: &serialized_file::SerializedFile,
        type_tree: &serialized_file::TypeTree,
        output_path: &Path,
    ) -> Result<u64, String> {
        use crate::utils::dump_utils::DumpUtils;
        let mut reader = ObjectReader::new(raw_data, sf);
        let value = DumpUtils::export_typetree_json(&mut reader, type_tree, &Default::default())?;
        let text = serde_json::to_string_pretty(&value).map_err(|e| format!("JSON: {}", e))?;
        fs::write(output_path, text).map_err(|e| format!("write: {}", e))?;
        Ok(fs::metadata(output_path)
            .map_err(|e| format!("stat: {}", e))?
            .len())
    }

    /// Fallback when no TypeTree: base64 encode raw data.
    pub fn export_raw(raw_data: &[u8], path_id: i64, output_path: &Path) -> Result<u64, String> {
        let wrapped = serde_json::json!({
            "class": "MonoBehaviour", "path_id": path_id,
            "note": "No TypeTree, raw base64",
            "raw_base64": crate::exporter::raw_exporter::RawExporter::base64_encode(raw_data)
        });
        let text = serde_json::to_string_pretty(&wrapped).map_err(|e| format!("JSON: {}", e))?;
        fs::write(output_path, text).map_err(|e| format!("write: {}", e))?;
        Ok(fs::metadata(output_path)
            .map_err(|e| format!("stat: {}", e))?
            .len())
    }
}
