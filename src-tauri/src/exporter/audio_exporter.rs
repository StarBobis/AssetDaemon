/*
 * audio_exporter.rs - AudioClip export utility class.
 */

use crate::common::bundle_file::asset_bundle::AssetBundle;
use crate::common::bundle_file::bundle_extractor::BundleExtractor;
use crate::common::export::common_types::ExportFormat;
use crate::common::serialized_file::object_reader::ObjectReader;
use crate::common::serialized_file::serialized_file::SerializedFile;
use std::fs;
use std::path::{Path, PathBuf};

pub struct AudioExporter;

impl AudioExporter {
    /// Export AudioClip payload bytes. WAV conversion is intentionally rejected until a decoder exists.
    pub fn export(
        raw_data: &[u8],
        serialized_file: &SerializedFile,
        bundle: &AssetBundle,
        bundle_path: &Path,
        format: &ExportFormat,
        output_path: &Path,
    ) -> Result<u64, String> {
        if matches!(format, ExportFormat::Wav) {
            return Err(
                "AudioClip WAV conversion is not implemented; export raw audio payload instead"
                    .to_string(),
            );
        }

        let audio_data = Self::extract_payload(raw_data, serialized_file, bundle, bundle_path)?;
        fs::write(output_path, audio_data).map_err(|e| format!("write: {}", e))?;
        Ok(fs::metadata(output_path)
            .map_err(|e| format!("stat: {}", e))?
            .len())
    }

    fn extract_payload(
        raw_data: &[u8],
        serialized_file: &SerializedFile,
        bundle: &AssetBundle,
        bundle_path: &Path,
    ) -> Result<Vec<u8>, String> {
        let mut reader = ObjectReader::new(raw_data, serialized_file);
        let _name = reader.try_read_aligned_string()?;

        if reader.version[0] >= 5 {
            reader.skip(4 * 4); // load type, channels, frequency, bits per sample
            reader.skip(4); // length
            reader.skip(1); // is tracker format
            reader.align();
            reader.skip(4); // subsound index
            reader.skip(3); // preload, load in background, legacy 3D
            reader.align();
            let source = reader.try_read_aligned_string()?;
            let offset = reader.try_read_i64()?;
            let size = reader.try_read_i64()?;
            let _compression_format = reader.try_read_i32()?;

            if size < 0 {
                return Err("AudioClip has invalid negative payload size".to_string());
            }
            let size = size as usize;
            if size == 0 {
                return Err("AudioClip has no payload data".to_string());
            }
            if source.is_empty() {
                return Self::slice_inline(raw_data, reader.pos, size);
            }
            return Self::read_streamed_payload(bundle, bundle_path, &source, offset, size);
        }

        reader.skip(4); // format
        reader.skip(4); // type
        reader.skip(2); // 3D, use hardware
        reader.align();
        if reader.version[0] >= 4 || reader.version[0] == 3 && reader.version[1] >= 2 {
            reader.skip(4); // stream flag
            let size = reader.try_read_i32()?;
            if size <= 0 {
                return Err("AudioClip has no payload data".to_string());
            }
            let size = size as usize;
            if reader.remaining() >= size {
                return Self::slice_inline(raw_data, reader.pos, size);
            }
            let offset = reader.try_read_u32()? as i64;
            let source = bundle_path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| format!("{}.resS", name))
                .unwrap_or_else(|| ".resS".to_string());
            return Self::read_streamed_payload(bundle, bundle_path, &source, offset, size);
        }

        let size = reader.try_read_i32()?;
        if size <= 0 {
            return Err("AudioClip has no payload data".to_string());
        }
        Self::slice_inline(raw_data, reader.pos, size as usize)
    }

    fn slice_inline(raw_data: &[u8], offset: usize, size: usize) -> Result<Vec<u8>, String> {
        let end = offset
            .checked_add(size)
            .ok_or_else(|| "AudioClip payload range overflowed".to_string())?;
        raw_data
            .get(offset..end)
            .map(|data| data.to_vec())
            .ok_or_else(|| {
                format!(
                    "AudioClip inline payload range {}..{} exceeds object size {}",
                    offset,
                    end,
                    raw_data.len()
                )
            })
    }

    fn read_streamed_payload(
        bundle: &AssetBundle,
        bundle_path: &Path,
        source: &str,
        offset: i64,
        size: usize,
    ) -> Result<Vec<u8>, String> {
        if offset < 0 {
            return Err("AudioClip has invalid negative stream offset".to_string());
        }
        let offset = offset as usize;
        let source_name = Path::new(source)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(source);

        if let Some(data) = bundle
            .nodes
            .iter()
            .find(|node| {
                node.name == source
                    || Path::new(&node.name)
                        .file_name()
                        .and_then(|name| name.to_str())
                        == Some(source_name)
            })
            .and_then(|node| bundle.extract_node_data(node).ok())
        {
            return Self::slice_inline(&data, offset, size);
        }

        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(parent) = bundle_path.parent() {
            candidates.push(parent.join(source));
            candidates.push(parent.join(source_name));
        }
        candidates.push(
            BundleExtractor::get_extract_dir(bundle_path, &std::env::temp_dir()).join(source),
        );
        candidates.push(
            BundleExtractor::get_extract_dir(bundle_path, &std::env::temp_dir()).join(source_name),
        );

        for candidate in candidates {
            if let Ok(data) = fs::read(&candidate) {
                return Self::slice_inline(&data, offset, size);
            }
        }

        Err(format!(
            "AudioClip streamed payload '{}' was not found",
            source
        ))
    }
}
