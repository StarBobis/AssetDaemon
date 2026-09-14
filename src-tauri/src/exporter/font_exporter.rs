/*
 * font_exporter.rs - Font (TTF/OTF) export utility class.
 */

use std::fs;
use std::path::Path;

pub struct FontExporter;

impl FontExporter {
    /// Export Unity Font as .ttf / .otf (using the specified format hint).
    /// raw_data should be the m_FontData byte array.
    /// format is used to determine the preferred output format (TTF/OTF).
    pub fn export_with_format(
        raw_data: &[u8],
        format: &crate::common::export::common_types::ExportFormat,
        output_path: &Path,
    ) -> Result<u64, String> {
        // Current implementation is the same as export (font format determined by embedded data),
        // Reserved format parameter for future forced conversion based on user selection.
        let _ = format;
        Self::export(raw_data, output_path)
    }

    /// Export Unity Font as .ttf / .otf.
    /// raw_data should be the m_FontData byte array.
    pub fn export(raw_data: &[u8], output_path: &Path) -> Result<u64, String> {
        if raw_data.is_empty() {
            return Err("Font data is empty".to_string());
        }
        // Locate TTF/OTF magic: "OTTO", "true", 0x00010000, "ttcf"
        let start = raw_data
            .windows(4)
            .position(|w| w == b"OTTO" || w == b"true" || w == b"\x00\x01\x00\x00" || w == b"ttcf")
            .unwrap_or(0);
        fs::write(output_path, &raw_data[start..]).map_err(|e| format!("write: {}", e))?;
        Ok(fs::metadata(output_path)
            .map_err(|e| format!("stat: {}", e))?
            .len())
    }
}
