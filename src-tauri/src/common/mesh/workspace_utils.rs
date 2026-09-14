/*
 * Workspace utility class - provides UnityValue deep extraction utility methods.
 *
 * Follows Soul.md convention: organized as struct + impl, no free functions.
 *
 * Deleted functions (dead code):
 *   - value_shape
 *   - deep_extract_floats
 *   - try_read_stream_data_legacy
 *
 * All Mesh parsing related logic has been migrated to mesh_asset_studio.rs (AssetStudio-style byte stream parsing)
 * and mesh_service.rs (TypeTree fallback path).
 */

use crate::unity::type_tree::unity_value::UnityValue;

/**
 * Workspace general utility class.
 *
 * Provides utility methods for deep extraction of u32 index arrays from UnityValue trees,
 * referenced by mesh_commands.rs and mesh_service.rs.
 */
pub struct WorkspaceUtils;

impl WorkspaceUtils {
    /// Deep extract u32 index array from UnityValue (recursively traverse Object/Array/Bytes)
    pub fn deep_extract_u32s(val: &UnityValue) -> Vec<u32> {
        match val {
            UnityValue::Array(arr) => {
                let ints: Vec<u32> = arr
                    .iter()
                    .filter_map(|v| match v {
                        UnityValue::Integer(i) => Some(*i as u32),
                        _ => None,
                    })
                    .collect();
                if ints.len() >= 3 {
                    return ints;
                }
                let bytes: Vec<u8> = arr
                    .iter()
                    .filter_map(|v| match v {
                        UnityValue::Integer(i) => Some(*i as u8),
                        _ => None,
                    })
                    .collect();
                if bytes.len() >= 6 {
                    let u16s: Vec<u32> = bytes
                        .chunks(2)
                        .filter(|c| c.len() == 2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]) as u32)
                        .collect();
                    if u16s.len() >= 3 {
                        return u16s;
                    }
                }
                for item in arr {
                    let r = Self::deep_extract_u32s(item);
                    if !r.is_empty() {
                        return r;
                    }
                }
                vec![]
            }
            UnityValue::Object(obj) => {
                for key in &["m_Data", "m_DataSize", "data", "m_IndexBuffer", "Array"] {
                    if let Some(sub) = obj.get(*key) {
                        let r = Self::deep_extract_u32s(sub);
                        if !r.is_empty() {
                            return r;
                        }
                    }
                }
                for (_k, sub) in obj.iter() {
                    let r = Self::deep_extract_u32s(sub);
                    if !r.is_empty() {
                        return r;
                    }
                }
                vec![]
            }
            UnityValue::Bytes(b) => {
                // A 32-bit index buffer is always 4-byte aligned; reading it as u16
                // (the previous flow, where the u16 branch always returned first for
                // len >= 6) silently produced garbage indices for 32-bit meshes that
                // lack an m_IndexFormat hint. Prefer the u32 interpretation for
                // 4-aligned buffers; a buffer with an odd u16 count (len % 4 == 2)
                // can only be u16.
                if b.len() >= 12 && b.len() % 4 == 0 {
                    let u32s: Vec<u32> = b
                        .chunks(4)
                        .filter(|c| c.len() == 4)
                        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                        .collect();
                    if u32s.len() >= 3 {
                        return u32s;
                    }
                }
                if b.len() >= 6 {
                    let u16s: Vec<u32> = b
                        .chunks(2)
                        .filter(|c| c.len() == 2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]) as u32)
                        .collect();
                    if u16s.len() >= 3 {
                        return u16s;
                    }
                }
                vec![]
            }
            _ => vec![],
        }
    }

    /// Deep extract u32 with format hint (is_16bit=true prioritizes u16 parsing)
    pub fn deep_extract_u32s_with_format(val: &UnityValue, is_16bit: bool) -> Vec<u32> {
        if let UnityValue::Bytes(b) = val {
            if is_16bit && b.len() >= 6 {
                let u: Vec<_> = b
                    .chunks(2)
                    .filter(|c| c.len() == 2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]) as u32)
                    .collect();
                if u.len() >= 3 {
                    return u;
                }
            }
            if !is_16bit && b.len() >= 12 && b.len() % 4 == 0 {
                let u: Vec<_> = b
                    .chunks(4)
                    .filter(|c| c.len() == 4)
                    .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                if u.len() >= 3 {
                    return u;
                }
            }
        }
        Self::deep_extract_u32s(val)
    }
}
