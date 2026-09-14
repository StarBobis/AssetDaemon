use std::collections::HashMap;

use crate::common::serialized_file::object_reader::ObjectReader;
use crate::common::serialized_file::serialized_file::{ObjectInfo, SerializedFile};
use crate::unity::classes::object::{PPtr, UnityObjectHeader};
use crate::unity::classes::registry::UnityClassParser;
use crate::unity::type_tree::unity_value::UnityValue;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SpriteObject {
    pub header: UnityObjectHeader,
    pub rect: RectF,
    pub pivot: Option<[f32; 2]>,
    pub pixels_to_units: Option<f32>,
    pub render_data_key: Option<String>,
    pub sprite_atlas: Option<PPtr>,
    pub render_data: Option<SpriteRenderData>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SpriteAtlasObject {
    pub header: UnityObjectHeader,
    pub render_data_map: Vec<SpriteAtlasRenderData>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SpriteAtlasRenderData {
    pub key: String,
    pub data: SpriteRenderData,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SpriteRenderData {
    pub texture: PPtr,
    pub texture_rect: RectF,
    pub texture_rect_offset: Option<[f32; 2]>,
    pub downscale_multiplier: f32,
    pub settings: SpriteSettings,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct SpriteSettings {
    pub raw: u32,
    pub packed: bool,
    pub packing_mode: u32,
    pub packing_rotation: u32,
    pub mesh_type: u32,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl SpriteObject {
    pub fn read(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
        header: UnityObjectHeader,
    ) -> Result<Self, String> {
        let Some(value) =
            UnityClassParser::read_typetree_value_public(serialized_file, object_info)
        else {
            return Self::read_binary(serialized_file, object_info, header);
        };
        let rect = find_field(&value, "m_Rect")
            .and_then(rect_from_value)
            .ok_or_else(|| "Sprite m_Rect is unavailable".to_string())?;
        let render_data_key = find_field(&value, "m_RenderDataKey").map(value_signature);
        let sprite_atlas = find_pptr_field(&value, "m_SpriteAtlas").filter(|pptr| !pptr.is_null());
        let render_data = find_field(&value, "m_RD").and_then(render_data_from_value);

        Ok(Self {
            header,
            rect,
            pivot: find_field(&value, "m_Pivot").and_then(vector2_from_value),
            pixels_to_units: find_number_field(&value, "m_PixelsToUnits").map(|v| v as f32),
            render_data_key,
            sprite_atlas,
            render_data,
        })
    }

    fn read_binary(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
        header: UnityObjectHeader,
    ) -> Result<Self, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        read_named_object_header(&mut reader);
        let rect = read_rect(&mut reader);
        let _offset = read_vec2(&mut reader);
        if reader.version[0] > 4 || (reader.version[0] == 4 && reader.version[1] >= 5) {
            read_vec4(&mut reader);
        }
        let pixels_to_units = reader.read_f32();
        let pivot = if reader.version[0] > 5
            || (reader.version[0] == 5 && reader.version[1] > 4)
            || (reader.version[0] == 5 && reader.version[1] == 4 && reader.version[2] >= 2)
        {
            Some(read_vec2(&mut reader))
        } else {
            Some([0.5, 0.5])
        };
        let _extrude = reader.read_u32();
        if reader.version[0] > 5 || (reader.version[0] == 5 && reader.version[1] >= 3) {
            let _is_polygon = reader.read_bool();
            reader.align();
        }
        if reader.version[0] >= 2017 {
            reader.skip(16);
            let _render_data_key_second = reader.read_i64();
            let tag_count = read_sane_count(&mut reader, "m_AtlasTags")?;
            for _ in 0..tag_count {
                reader.read_aligned_string();
            }
            let _sprite_atlas = read_pptr(&mut reader);
        }

        let render_data = read_sprite_render_data(&mut reader)?;
        Ok(Self {
            header,
            rect,
            pivot,
            pixels_to_units: Some(pixels_to_units),
            render_data_key: None,
            sprite_atlas: None,
            render_data: Some(render_data),
        })
    }
}

impl SpriteAtlasObject {
    pub fn read(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
        header: UnityObjectHeader,
    ) -> Result<Self, String> {
        let value = UnityClassParser::read_typetree_value_public(serialized_file, object_info)
            .ok_or_else(|| "SpriteAtlas TypeTree is unavailable".to_string())?;
        let render_data_map = find_field(&value, "m_RenderDataMap")
            .map(sprite_atlas_render_data_map_from_value)
            .unwrap_or_default();
        Ok(Self {
            header,
            render_data_map,
        })
    }
}

impl SpriteSettings {
    pub fn from_raw(raw: u32) -> Self {
        Self {
            raw,
            packed: (raw & 1) != 0,
            packing_mode: (raw >> 1) & 1,
            packing_rotation: (raw >> 2) & 0x0f,
            mesh_type: (raw >> 6) & 1,
        }
    }
}

fn sprite_atlas_render_data_map_from_value(value: &UnityValue) -> Vec<SpriteAtlasRenderData> {
    match value {
        UnityValue::Array(items) => items
            .iter()
            .filter_map(|item| {
                let UnityValue::Object(map) = item else {
                    return None;
                };
                let key = map.get("first").map(value_signature)?;
                let data = map.get("second").and_then(render_data_from_value)?;
                Some(SpriteAtlasRenderData { key, data })
            })
            .collect(),
        UnityValue::Object(map) => map
            .values()
            .find_map(|child| match child {
                UnityValue::Array(_) => Some(sprite_atlas_render_data_map_from_value(child)),
                _ => None,
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn render_data_from_value(value: &UnityValue) -> Option<SpriteRenderData> {
    let texture = child_pptr(value, "texture")?;
    let texture_rect = child_value(value, "textureRect").and_then(rect_from_value)?;
    let settings_raw = child_value(value, "settingsRaw")
        .and_then(settings_raw_from_value)
        .unwrap_or(0);
    Some(SpriteRenderData {
        texture,
        texture_rect,
        texture_rect_offset: child_value(value, "textureRectOffset").and_then(vector2_from_value),
        downscale_multiplier: child_value(value, "downscaleMultiplier")
            .and_then(number_from_value)
            .map(|v| v as f32)
            .unwrap_or(1.0),
        settings: SpriteSettings::from_raw(settings_raw),
    })
}

fn child_value<'a>(value: &'a UnityValue, field_name: &str) -> Option<&'a UnityValue> {
    match value {
        UnityValue::Object(map) => map.get(field_name),
        _ => None,
    }
}

fn child_pptr(value: &UnityValue, field_name: &str) -> Option<PPtr> {
    child_value(value, field_name).and_then(pptr_from_value)
}

fn find_field<'a>(value: &'a UnityValue, field_name: &str) -> Option<&'a UnityValue> {
    match value {
        UnityValue::Object(map) => map
            .get(field_name)
            .or_else(|| map.values().find_map(|child| find_field(child, field_name))),
        UnityValue::Array(items) => items.iter().find_map(|child| find_field(child, field_name)),
        _ => None,
    }
}

fn find_pptr_field(value: &UnityValue, field_name: &str) -> Option<PPtr> {
    find_field(value, field_name).and_then(pptr_from_value)
}

fn pptr_from_value(value: &UnityValue) -> Option<PPtr> {
    match value {
        UnityValue::Ptr { file_id, path_id } => Some(PPtr {
            file_id: *file_id,
            path_id: *path_id,
        }),
        UnityValue::Object(map) => {
            let file_id = map.get("m_FileID").and_then(UnityValue::as_i64)?;
            let path_id = map.get("m_PathID").and_then(UnityValue::as_i64)?;
            Some(PPtr {
                file_id: file_id as i32,
                path_id,
            })
        }
        _ => None,
    }
}

fn rect_from_value(value: &UnityValue) -> Option<RectF> {
    let UnityValue::Object(map) = value else {
        return None;
    };
    Some(RectF {
        x: object_number(map, "x").or_else(|| object_number(map, "m_X"))? as f32,
        y: object_number(map, "y").or_else(|| object_number(map, "m_Y"))? as f32,
        width: object_number(map, "width").or_else(|| object_number(map, "m_Width"))? as f32,
        height: object_number(map, "height").or_else(|| object_number(map, "m_Height"))? as f32,
    })
}

fn vector2_from_value(value: &UnityValue) -> Option<[f32; 2]> {
    let UnityValue::Object(map) = value else {
        return None;
    };
    Some([
        object_number(map, "x").or_else(|| object_number(map, "m_X"))? as f32,
        object_number(map, "y").or_else(|| object_number(map, "m_Y"))? as f32,
    ])
}

fn settings_raw_from_value(value: &UnityValue) -> Option<u32> {
    number_from_value(value).map(|v| v as u32).or_else(|| {
        find_field(value, "settingsRaw")
            .and_then(number_from_value)
            .map(|v| v as u32)
    })
}

fn object_number(map: &HashMap<String, UnityValue>, key: &str) -> Option<f64> {
    map.get(key).and_then(number_from_value)
}

fn find_number_field(value: &UnityValue, field_name: &str) -> Option<f64> {
    find_field(value, field_name).and_then(number_from_value)
}

fn number_from_value(value: &UnityValue) -> Option<f64> {
    match value {
        UnityValue::Float(value) => Some(*value),
        UnityValue::Integer(value) => Some(*value as f64),
        _ => None,
    }
}

fn value_signature(value: &UnityValue) -> String {
    match value {
        UnityValue::Null => "null".to_string(),
        UnityValue::Bool(value) => format!("b:{value}"),
        UnityValue::Integer(value) => format!("i:{value}"),
        UnityValue::Float(value) => format!("f:{value:.8}"),
        UnityValue::String(value) => format!("s:{value}"),
        UnityValue::Bytes(value) => format!("bytes:{}", hex_signature(value)),
        UnityValue::Ptr { file_id, path_id } => format!("ptr:{file_id}:{path_id}"),
        UnityValue::Array(items) => {
            let parts = items.iter().map(value_signature).collect::<Vec<_>>();
            format!("[{}]", parts.join(","))
        }
        UnityValue::Object(map) => {
            let mut entries = map
                .iter()
                .map(|(key, value)| (key.as_str(), value_signature(value)))
                .collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            let parts = entries
                .into_iter()
                .map(|(key, value)| format!("{key}:{value}"))
                .collect::<Vec<_>>();
            format!("{{{}}}", parts.join(","))
        }
    }
}

fn hex_signature(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

fn read_sprite_render_data(reader: &mut ObjectReader) -> Result<SpriteRenderData, String> {
    let texture = read_pptr(reader);
    if reader.version[0] > 5 || (reader.version[0] == 5 && reader.version[1] >= 2) {
        let _alpha_texture = read_pptr(reader);
    }
    if reader.version[0] >= 2019 {
        let count = read_sane_count(reader, "secondaryTextures")?;
        for _ in 0..count {
            let _texture = read_pptr(reader);
            reader.read_string_to_null();
        }
    }
    if reader.version[0] > 5 || (reader.version[0] == 5 && reader.version[1] >= 6) {
        let sub_mesh_count = read_sane_count(reader, "m_SubMeshes")?;
        for _ in 0..sub_mesh_count {
            read_sub_mesh(reader);
        }
        reader.read_u8_array();
        reader.align();
        skip_vertex_data(reader)?;
    } else {
        let vertex_count = read_sane_count(reader, "vertices")?;
        for _ in 0..vertex_count {
            read_vec3(reader);
            if reader.version[0] < 4 || (reader.version[0] == 4 && reader.version[1] <= 3) {
                read_vec2(reader);
            }
        }
        let index_count = read_sane_count(reader, "indices")?;
        reader.skip(index_count.saturating_mul(2));
        reader.align();
    }
    if reader.version[0] >= 2018 {
        let bindpose_count = read_sane_count(reader, "m_Bindpose")?;
        reader.skip(bindpose_count.saturating_mul(64));
        if reader.version[0] == 2018 && reader.version[1] < 2 {
            let source_skin_count = read_sane_count(reader, "m_SourceSkin")?;
            reader.skip(source_skin_count.saturating_mul(32));
        }
    }

    let texture_rect = read_rect(reader);
    let texture_rect_offset = Some(read_vec2(reader));
    if reader.version[0] > 5 || (reader.version[0] == 5 && reader.version[1] >= 6) {
        read_vec2(reader);
    }
    let settings = SpriteSettings::from_raw(reader.read_u32());
    if reader.version[0] > 4 || (reader.version[0] == 4 && reader.version[1] >= 5) {
        read_vec4(reader);
    }
    let downscale_multiplier = if reader.version[0] >= 2017 {
        reader.read_f32()
    } else {
        1.0
    };

    Ok(SpriteRenderData {
        texture,
        texture_rect,
        texture_rect_offset,
        downscale_multiplier,
        settings,
    })
}

fn read_named_object_header(reader: &mut ObjectReader) {
    if reader.target_platform == -2 {
        let _object_hide_flags = reader.read_u32();
        let _prefab_parent = read_pptr(reader);
        let _prefab_internal = read_pptr(reader);
    }
    reader.read_aligned_string();
}

fn read_pptr(reader: &mut ObjectReader) -> PPtr {
    PPtr {
        file_id: reader.read_i32(),
        path_id: reader.read_path_id(),
    }
}

fn read_rect(reader: &mut ObjectReader) -> RectF {
    RectF {
        x: reader.read_f32(),
        y: reader.read_f32(),
        width: reader.read_f32(),
        height: reader.read_f32(),
    }
}

fn read_vec2(reader: &mut ObjectReader) -> [f32; 2] {
    [reader.read_f32(), reader.read_f32()]
}

fn read_vec3(reader: &mut ObjectReader) -> [f32; 3] {
    [reader.read_f32(), reader.read_f32(), reader.read_f32()]
}

fn read_vec4(reader: &mut ObjectReader) -> [f32; 4] {
    [
        reader.read_f32(),
        reader.read_f32(),
        reader.read_f32(),
        reader.read_f32(),
    ]
}

fn read_sane_count(reader: &mut ObjectReader, field_name: &str) -> Result<usize, String> {
    let count = reader.read_i32();
    if !(0..=1_000_000).contains(&count) {
        return Err(format!("Invalid {} count: {}", field_name, count));
    }
    Ok(count as usize)
}

fn read_sub_mesh(reader: &mut ObjectReader) {
    reader.skip(12);
    if reader.version[0] < 4 {
        reader.skip(4);
    }
    if reader.version[0] > 2017 || (reader.version[0] == 2017 && reader.version[1] >= 3) {
        reader.skip(4);
    }
    if reader.version[0] >= 3 {
        reader.skip(8);
        read_vec3(reader);
        read_vec3(reader);
    }
}

fn skip_vertex_data(reader: &mut ObjectReader) -> Result<(), String> {
    if reader.version[0] < 2018 {
        reader.skip(4);
    }
    reader.skip(4);
    if reader.version[0] >= 4 {
        let channel_count = read_sane_count(reader, "m_Channels")?;
        reader.skip(channel_count.saturating_mul(4));
    }
    if reader.version[0] < 5 {
        let stream_count = if reader.version[0] < 4 {
            4
        } else {
            read_sane_count(reader, "m_Streams")?
        };
        let stream_size = if reader.version[0] < 4 { 16 } else { 12 };
        reader.skip(stream_count.saturating_mul(stream_size));
    }
    let data = reader.read_u8_array();
    reader.align();
    if data.is_empty() && reader.remaining() == 0 {
        return Err("Sprite VertexData is truncated".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sprite_settings_bits_like_assetstudio() {
        let settings = SpriteSettings::from_raw(0b01_0101);

        assert!(settings.packed);
        assert_eq!(settings.packing_mode, 0);
        assert_eq!(settings.packing_rotation, 5);
    }
}
