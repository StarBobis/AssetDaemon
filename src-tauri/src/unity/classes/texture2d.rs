/*
 * texture_reader.rs -Unity Texture2D Binary Field Parser
 *
 * Extracts duplicated Texture2D version-aware field parsing logic from
 * texture_commands.rs and export_service.rs, centralized here for unified maintenance.
 *
 * Based on AssetStudio's Texture2D.cs (ObjectReader reader) implementation,
 * fully parses all version branch fields of the inheritance chain from
 * Object -> EditorExtension -> NamedObject -> Texture -> Texture2D.
 *
 * Follows Soul.md convention: organized as struct + impl, no free functions.
 */

use crate::common::bundle_file::res_s::ResS;
use crate::common::serialized_file::object_reader::ObjectReader;
use crate::unity::classes::object::UnityObjectHeader;
use std::collections::HashMap;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Texture2DObject {
    pub header: UnityObjectHeader,
    pub data: Texture2DInfo,
}

/// Texture2D parse result (includes all fields + decoded pixel data)
#[derive(Debug, Clone)]
pub struct Texture2DInfo {
    pub width: u32,
    pub height: u32,
    pub complete_image_size: i32,
    pub texture_format: i32,
    pub mip_count: i32,
    pub is_readable: bool,
    pub streaming_mipmaps: bool,
    pub image_count: i32,
    pub texture_dimension: i32,
    pub filter_mode: i32,
    pub aniso: i32,
    pub wrap_u: i32,
    pub wrap_v: i32,
    pub color_space: i32,
    pub image_data_size: u32,
    pub pixel_data: Vec<u8>,
}

/**
 * Texture2D field reader.
 *
 * Encapsulates complete version-aware parsing of the Unity Texture2D serialization format,
 * handling both inline texture data and streaming (.resS) data extraction.
 */
pub struct TextureReader;

impl TextureReader {
    /**
     * Parses a Texture2D object and extracts pixel data.
     *
     * Fully reads all serialized fields of a Unity Texture2D (with version-aware branches),
     * and automatically loads streaming data from resources (.resS) based on StreamingInfo,
     * or extracts from inline data.
     *
     * @param reader       Initialized ObjectReader (pointing to the start of Texture2D raw bytes)
     * @param resources    bundle.resources (for .resS streaming texture lookup)
     * @return             Texture2DInfo (contains all parsed fields and pixel data)
     */
    pub fn read_texture(
        reader: &mut ObjectReader,
        resources: &HashMap<String, ResS>,
    ) -> Result<Texture2DInfo, String> {
        let v = reader.version;
        let is_notarget = reader.target_platform == -2; // BuildTarget.NoTarget

        // ==================================================================
        // Inheritance chain: Object -> EditorExtension -> NamedObject -> Texture -> Texture2D
        // ==================================================================

        // ---- Object (NoTarget platform only) ----
        if is_notarget {
            let _object_hide_flags = reader.read_u32();
        }

        // ---- EditorExtension (NoTarget platform only, 2xPPtr) ----
        if is_notarget {
            let _prefab_parent_file = reader.read_i32();
            let _prefab_parent_path = if reader.header_version < 14 {
                reader.read_i32() as i64
            } else {
                reader.read_i64()
            };
            let _prefab_internal_file = reader.read_i32();
            let _prefab_internal_path = if reader.header_version < 14 {
                reader.read_i32() as i64
            } else {
                reader.read_i64()
            };
        }

        // ---- NamedObject: m_Name (AlignedString) ----
        reader.read_aligned_string();

        // ---- Texture (2017.3+, AssetStudio Texture.cs) ----
        if v[0] > 2017 || (v[0] == 2017 && v[1] >= 3) {
            let _forced_fallback_format = reader.read_i32();
            let _downscale_fallback = reader.read_bool();
            if v[0] > 2020 || (v[0] == 2020 && v[1] >= 2) {
                let _is_alpha_channel_optional = reader.read_bool();
            }
            reader.align();
        }

        // ---- Texture2D specific fields ----
        let width = reader.read_i32() as u32;
        let height = reader.read_i32() as u32;
        let complete_image_size = reader.read_i32();

        if v[0] >= 2020 {
            let _mips_stripped = reader.read_i32();
        }

        let texture_format = reader.read_i32();

        let mip_count = if v[0] < 5 || (v[0] == 5 && v[1] < 2) {
            let _mipmap = reader.read_bool();
            0
        } else {
            reader.read_i32()
        };

        let has_readable_field = v[0] > 2 || (v[0] == 2 && v[1] >= 6);
        let is_readable = has_readable_field && reader.read_bool();

        if v[0] >= 2020 {
            let _is_pre_processed = reader.read_bool();
        }

        if v[0] > 2019 || (v[0] == 2019 && v[1] >= 3) {
            let _ignore_mipmap_limit = reader.read_bool();
            if v[0] >= 2022 {
                let _mipmap_limit_group_name = reader.read_aligned_string();
            }
        }

        if v[0] >= 3 {
            if v[0] < 5 || (v[0] == 5 && v[1] <= 4) {
                let _read_allowed = reader.read_bool();
            }
        }

        let has_streaming_mipmaps = v[0] > 2018 || (v[0] == 2018 && v[1] >= 2);
        let streaming_mipmaps = has_streaming_mipmaps && reader.read_bool();

        reader.align();

        if has_streaming_mipmaps {
            let _streaming_mipmaps_priority = reader.read_i32();
        }

        let image_header_pos = reader.pos;
        let mut image_count = reader.read_i32();
        let mut texture_dimension = reader.read_i32();
        if v[0] == 5 && !Self::is_sane_image_header(image_count, texture_dimension) {
            reader.pos = image_header_pos.saturating_add(4).min(reader.data.len());
            image_count = reader.read_i32();
            texture_dimension = reader.read_i32();
        }

        // GLTextureSettings
        let filter_mode = reader.read_i32();
        let aniso = reader.read_i32();
        let _mip_bias = reader.read_f32();
        let (wrap_u, wrap_v) = if v[0] >= 2017 {
            let wu = reader.read_i32();
            let wv = reader.read_i32();
            let _ww = reader.read_i32();
            (wu, wv)
        } else {
            let wm = reader.read_i32();
            (wm, wm)
        };

        if v[0] >= 3 {
            let _lightmap_format = reader.read_i32();
        }

        let color_space = if v[0] > 3 || (v[0] == 3 && v[1] >= 5) {
            reader.read_i32()
        } else {
            0
        };

        // (2020.2+) m_PlatformBlob (UInt8Array) + AlignStream
        if v[0] > 2020 || (v[0] == 2020 && v[1] >= 2) {
            let _platform_blob = reader.read_u8_array();
            reader.align();
        }

        // image_data_size + image_data or StreamingInfo
        let image_data_size = reader.read_i32() as usize;

        let pixel_data: Vec<u8> = if image_data_size == 0 && ((v[0] == 5 && v[1] >= 3) || v[0] > 5)
        {
            // Streaming texture: reads a fragment at given offset/size from .resS file
            let soff = if v[0] >= 2020 {
                reader.read_i64() as u64
            } else {
                reader.read_u32() as u64
            };
            let ssize = reader.read_u32() as usize;
            let sp = reader.read_aligned_string();
            let filename = sp.rsplit('/').next().unwrap_or(&sp).to_string();
            if ssize == 0 {
                return Err(format!("Streaming texture size=0: {}", filename));
            }
            let res = resources
                .get(&filename.to_lowercase())
                .ok_or_else(|| format!("Streaming resource not found: {}", filename))?;
            res.read(soff as usize, ssize)?.to_vec()
        } else if image_data_size > 0 {
            // Inline texture: reads directly from ObjectReader data
            if reader.pos + image_data_size > reader.data.len() {
                return Err(format!(
                    "Texture data out of bounds: image_data_size={} but remaining {} bytes",
                    image_data_size,
                    reader.data.len() - reader.pos
                ));
            }
            reader.data[reader.pos..reader.pos + image_data_size].to_vec()
        } else {
            return Err("Texture data is empty and no StreamingInfo".to_string());
        };

        Ok(Texture2DInfo {
            width,
            height,
            complete_image_size,
            texture_format,
            mip_count,
            is_readable,
            streaming_mipmaps,
            image_count,
            texture_dimension,
            filter_mode,
            aniso,
            wrap_u,
            wrap_v,
            color_space,
            image_data_size: image_data_size as u32,
            pixel_data,
        })
    }

    fn is_sane_image_header(image_count: i32, texture_dimension: i32) -> bool {
        (1..=1024).contains(&image_count) && (1..=6).contains(&texture_dimension)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unity::classes::texture_format::TextureFormatConst;

    fn push_i32(bytes: &mut Vec<u8>, value: i32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_f32(bytes: &mut Vec<u8>, value: f32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_aligned_string(bytes: &mut Vec<u8>, value: &str) {
        push_i32(bytes, value.len() as i32);
        bytes.extend_from_slice(value.as_bytes());
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
    }

    #[test]
    fn reads_unity_56_texture_with_legacy_padding_before_image_header() {
        let mut bytes = Vec::new();
        push_aligned_string(&mut bytes, "tex");
        push_i32(&mut bytes, 4); // m_Width
        push_i32(&mut bytes, 4); // m_Height
        push_i32(&mut bytes, 8); // m_CompleteImageSize
        push_i32(&mut bytes, TextureFormatConst::DXT1);
        push_i32(&mut bytes, 1); // m_MipCount
        bytes.push(0); // m_IsReadable
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        push_i32(&mut bytes, 0); // legacy/padding observed in some 5.6 no-typetree files
        push_i32(&mut bytes, 1); // m_ImageCount
        push_i32(&mut bytes, 2); // m_TextureDimension
        push_i32(&mut bytes, 0); // m_FilterMode
        push_i32(&mut bytes, 1); // m_Aniso
        push_f32(&mut bytes, 0.0); // m_MipBias
        push_i32(&mut bytes, 1); // m_WrapMode
        push_i32(&mut bytes, 0); // m_LightmapFormat
        push_i32(&mut bytes, 1); // m_ColorSpace
        push_i32(&mut bytes, 8); // image data size
        bytes.extend_from_slice(&[0; 8]);

        let mut reader = ObjectReader {
            data: &bytes,
            pos: 0,
            version: [5, 6, 3, 3],
            endian: 0,
            header_version: 17,
            target_platform: 19,
        };

        let info = TextureReader::read_texture(&mut reader, &HashMap::new())
            .expect("Texture2D should parse");

        assert_eq!(info.image_count, 1);
        assert_eq!(info.texture_dimension, 2);
        assert_eq!(info.image_data_size, 8);
        assert_eq!(info.pixel_data.len(), 8);
    }

    #[test]
    fn reads_unity_2022_mipmap_limit_group_name_before_streaming_fields() {
        let mut bytes = Vec::new();
        push_aligned_string(&mut bytes, "tex");

        push_i32(&mut bytes, 0); // m_ForcedFallbackFormat
        bytes.push(0); // m_DownscaleFallback
        bytes.push(0); // m_IsAlphaChannelOptional
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }

        push_i32(&mut bytes, 1); // m_Width
        push_i32(&mut bytes, 1); // m_Height
        push_i32(&mut bytes, 3); // m_CompleteImageSize
        push_i32(&mut bytes, 0); // m_MipsStripped
        push_i32(&mut bytes, TextureFormatConst::RGB24);
        push_i32(&mut bytes, 1); // m_MipCount
        bytes.push(0); // m_IsReadable
        bytes.push(0); // m_IsPreProcessed
        bytes.push(0); // m_IgnoreMipmapLimit
        push_aligned_string(&mut bytes, ""); // m_MipmapLimitGroupName
        bytes.push(0); // m_StreamingMipmaps
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }

        push_i32(&mut bytes, 0); // m_StreamingMipmapsPriority
        push_i32(&mut bytes, 1); // m_ImageCount
        push_i32(&mut bytes, 2); // m_TextureDimension
        push_i32(&mut bytes, 0); // m_FilterMode
        push_i32(&mut bytes, 1); // m_Aniso
        push_f32(&mut bytes, 0.0); // m_MipBias
        push_i32(&mut bytes, 1); // m_WrapU
        push_i32(&mut bytes, 1); // m_WrapV
        push_i32(&mut bytes, 1); // m_WrapW
        push_i32(&mut bytes, 0); // m_LightmapFormat
        push_i32(&mut bytes, 1); // m_ColorSpace
        push_i32(&mut bytes, 0); // m_PlatformBlob size
        push_i32(&mut bytes, 3); // image data size
        bytes.extend_from_slice(&[1, 2, 3]);

        let mut reader = ObjectReader {
            data: &bytes,
            pos: 0,
            version: [2022, 3, 62, 2],
            endian: 0,
            header_version: 22,
            target_platform: 19,
        };

        let info = TextureReader::read_texture(&mut reader, &HashMap::new())
            .expect("Texture2D should parse");

        assert_eq!(info.image_count, 1);
        assert_eq!(info.texture_dimension, 2);
        assert_eq!(info.image_data_size, 3);
        assert_eq!(info.pixel_data, vec![1, 2, 3]);
    }
}
