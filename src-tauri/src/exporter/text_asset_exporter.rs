/*
 * text_asset_exporter.rs - TextAsset export utility class.
 */

use std::fs;
use std::path::Path;

pub struct TextAssetExporter;

impl TextAssetExporter {
    /// Export TextAsset.
    /// - "txt": UTF-8 text
    /// - "json": JSON formatted or wrap {"raw": ...}
    /// - "bytes": raw bytes
    pub fn export(raw_data: &[u8], format: &str, output_path: &Path) -> Result<u64, String> {
        let content = String::from_utf8_lossy(raw_data);
        match format {
            "json" => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                    let pretty =
                        serde_json::to_string_pretty(&v).map_err(|e| format!("JSON: {}", e))?;
                    fs::write(output_path, pretty).map_err(|e| format!("write: {}", e))?;
                } else {
                    let w = serde_json::json!({"raw": content.as_ref()});
                    fs::write(
                        output_path,
                        serde_json::to_string_pretty(&w).map_err(|e| format!("JSON: {}", e))?,
                    )
                    .map_err(|e| format!("write: {}", e))?;
                }
            }
            "bytes" => {
                fs::write(output_path, raw_data).map_err(|e| format!("write: {}", e))?;
            }
            _ => {
                fs::write(output_path, content.as_bytes()).map_err(|e| format!("write: {}", e))?;
            }
        }
        Ok(fs::metadata(output_path)
            .map_err(|e| format!("stat: {}", e))?
            .len())
    }
}
