/*
 * raw_exporter.rs - Generic raw byte export utility class (fallback).
 */

use std::fs;
use std::path::Path;

pub struct RawExporter;

impl RawExporter {
    /// Export raw bytes of any asset.
    pub fn export(raw_data: &[u8], output_path: &Path) -> Result<u64, String> {
        fs::write(output_path, raw_data).map_err(|e| format!("write: {}", e))?;
        Ok(fs::metadata(output_path)
            .map_err(|e| format!("stat: {}", e))?
            .len())
    }

    /// Simple base64 encoding (no external dependencies).
    pub fn base64_encode(data: &[u8]) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for chunk in data.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = *chunk.get(1).unwrap_or(&0) as u32;
            let b2 = *chunk.get(2).unwrap_or(&0) as u32;
            let n = (b0 << 16) | (b1 << 8) | b2;
            s.push(CHARS[((n >> 18) & 0x3F) as usize] as char);
            s.push(CHARS[((n >> 12) & 0x3F) as usize] as char);
            s.push(if chunk.len() > 1 {
                CHARS[((n >> 6) & 0x3F) as usize] as char
            } else {
                '='
            });
            s.push(if chunk.len() > 2 {
                CHARS[(n & 0x3F) as usize] as char
            } else {
                '='
            });
        }
        s
    }
}
