use crate::common::serialized_file::object_reader::ObjectReader;
use crate::common::serialized_file::serialized_file::{ObjectInfo, SerializedFile};
use crate::unity::classes::object::{PPtr, UnityObjectHeader};
use crate::unity::classes::registry::UnityClassParser;
use crate::unity::type_tree::unity_value::UnityValue;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SpriteMaskObject {
    pub header: UnityObjectHeader,
    pub game_object: PPtr,
    pub sprite: Option<PPtr>,
    pub alpha_cutoff: Option<f32>,
    pub is_custom_range_active: Option<bool>,
    pub front_sorting_layer_id: Option<i64>,
    pub front_sorting_order: Option<i64>,
    pub back_sorting_layer_id: Option<i64>,
    pub back_sorting_order: Option<i64>,
}

impl SpriteMaskObject {
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

        let game_object = find_pptr_field(&value, "m_GameObject").unwrap_or(PPtr {
            file_id: 0,
            path_id: 0,
        });
        let sprite = find_pptr_field(&value, "m_Sprite").filter(|pptr| !pptr.is_null());

        Ok(Self {
            header,
            game_object,
            sprite,
            alpha_cutoff: find_number_field(&value, "m_AlphaCutoff").map(|value| value as f32),
            is_custom_range_active: find_bool_field(&value, "m_IsCustomRangeActive"),
            front_sorting_layer_id: find_integer_field(&value, "m_FrontSortingLayerID"),
            front_sorting_order: find_integer_field(&value, "m_FrontSortingOrder"),
            back_sorting_layer_id: find_integer_field(&value, "m_BackSortingLayerID"),
            back_sorting_order: find_integer_field(&value, "m_BackSortingOrder"),
        })
    }

    fn read_binary(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
        header: UnityObjectHeader,
    ) -> Result<Self, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        read_editor_extension_base(&mut reader);
        let game_object = read_pptr(&mut reader);
        let _enabled = reader.read_u8();
        reader.align();
        let sprite = read_pptr(&mut reader);
        let alpha_cutoff = (reader.remaining() >= 4).then(|| reader.read_f32());
        let is_custom_range_active = if reader.remaining() >= 1 {
            let value = reader.read_bool();
            reader.align();
            Some(value)
        } else {
            None
        };
        let front_sorting_layer_id = (reader.remaining() >= 4).then(|| reader.read_i32() as i64);
        let front_sorting_order = (reader.remaining() >= 4).then(|| reader.read_i32() as i64);
        let back_sorting_layer_id = (reader.remaining() >= 4).then(|| reader.read_i32() as i64);
        let back_sorting_order = (reader.remaining() >= 4).then(|| reader.read_i32() as i64);

        Ok(Self {
            header,
            game_object,
            sprite: (!sprite.is_null()).then_some(sprite),
            alpha_cutoff,
            is_custom_range_active,
            front_sorting_layer_id,
            front_sorting_order,
            back_sorting_layer_id,
            back_sorting_order,
        })
    }
}

fn read_pptr(reader: &mut ObjectReader) -> PPtr {
    PPtr {
        file_id: reader.read_i32(),
        path_id: reader.read_path_id(),
    }
}

fn read_editor_extension_base(reader: &mut ObjectReader) {
    if reader.target_platform != -2 {
        return;
    }
    let _object_hide_flags = reader.read_u32();
    let _prefab_parent = read_pptr(reader);
    let _prefab_internal = read_pptr(reader);
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
    match find_field(value, field_name)? {
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

fn find_integer_field(value: &UnityValue, field_name: &str) -> Option<i64> {
    find_field(value, field_name).and_then(UnityValue::as_i64)
}

fn find_number_field(value: &UnityValue, field_name: &str) -> Option<f64> {
    match find_field(value, field_name)? {
        UnityValue::Float(value) => Some(*value),
        UnityValue::Integer(value) => Some(*value as f64),
        _ => None,
    }
}

fn find_bool_field(value: &UnityValue, field_name: &str) -> Option<bool> {
    match find_field(value, field_name)? {
        UnityValue::Bool(value) => Some(*value),
        UnityValue::Integer(value) => Some(*value != 0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn extracts_sprite_mask_fields_from_typetree_shape() {
        let value = UnityValue::Object(HashMap::from([
            ("m_GameObject".to_string(), ptr(0, 700)),
            ("m_Sprite".to_string(), ptr(0, 900)),
            ("m_AlphaCutoff".to_string(), UnityValue::Float(0.45)),
            ("m_IsCustomRangeActive".to_string(), UnityValue::Bool(true)),
            (
                "m_FrontSortingLayerID".to_string(),
                UnityValue::Integer(100),
            ),
            ("m_FrontSortingOrder".to_string(), UnityValue::Integer(2)),
            ("m_BackSortingLayerID".to_string(), UnityValue::Integer(50)),
            ("m_BackSortingOrder".to_string(), UnityValue::Integer(-1)),
        ]));

        assert_eq!(
            find_pptr_field(&value, "m_GameObject").unwrap().path_id,
            700
        );
        assert_eq!(find_pptr_field(&value, "m_Sprite").unwrap().path_id, 900);
        assert_eq!(find_number_field(&value, "m_AlphaCutoff").unwrap(), 0.45);
        assert_eq!(find_bool_field(&value, "m_IsCustomRangeActive"), Some(true));
        assert_eq!(
            find_integer_field(&value, "m_FrontSortingLayerID"),
            Some(100)
        );
        assert_eq!(find_integer_field(&value, "m_BackSortingOrder"), Some(-1));
    }

    fn ptr(file_id: i32, path_id: i64) -> UnityValue {
        UnityValue::Ptr { file_id, path_id }
    }
}
