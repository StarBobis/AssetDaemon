/*
 * type_tree.rs -- Unity type system-agnostic value types and TypeTree reader.
 *
 * UnityValue enum represents arbitrary values parsed from Unity TypeTree,
 * TypeTreeReaderUtils is responsible for recursively reading TypeTree structures.
 *
 * Follows Soul.md specification: struct + enum + impl organization.
 */

use std::collections::HashMap;

// ================================================================
// UnityValue -- TypeTree value type (compatible with unity_asset::UnityValue)
// ================================================================

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum UnityValue {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<UnityValue>),
    Object(HashMap<String, UnityValue>),
    Ptr { file_id: i32, path_id: i64 },
}

impl UnityValue {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            UnityValue::Integer(i) => Some(*i),
            UnityValue::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            UnityValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn variant_name(&self) -> &'static str {
        match self {
            UnityValue::Null => "Null",
            UnityValue::Bool(_) => "Bool",
            UnityValue::Integer(_) => "Integer",
            UnityValue::Float(_) => "Float",
            UnityValue::String(_) => "String",
            UnityValue::Bytes(_) => "Bytes",
            UnityValue::Array(_) => "Array",
            UnityValue::Object(_) => "Object",
            UnityValue::Ptr { .. } => "Ptr",
        }
    }

    /// Attempt to get underlying byte slice reference (Bytes variant only)
    pub fn to_u8_slice(&self) -> Option<&[u8]> {
        match self {
            UnityValue::Bytes(b) => Some(b.as_slice()),
            _ => None,
        }
    }

    /// Attempt to get Vec<u8> representation (Bytes -> clone, Array(Integer) -> element-wise conversion)
    pub fn to_u8_vec(&self) -> Vec<u8> {
        match self {
            UnityValue::Bytes(b) => b.clone(),
            UnityValue::Array(a) => a
                .iter()
                .filter_map(|v| match v {
                    UnityValue::Integer(i) => Some(*i as u8),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        }
    }
}

// ================================================================
