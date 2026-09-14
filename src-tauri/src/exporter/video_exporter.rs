/*
 * video_exporter.rs - VideoClip/MovieTexture raw export utility class.
 */

use std::fs;
use std::path::Path;

pub struct VideoExporter;

impl VideoExporter {
    /// Export raw video asset bytes.
    pub fn export(raw_data: &[u8], output_path: &Path) -> Result<u64, String> {
        fs::write(output_path, raw_data).map_err(|e| format!("write: {}", e))?;
        Ok(fs::metadata(output_path)
            .map_err(|e| format!("stat: {}", e))?
            .len())
    }
}
