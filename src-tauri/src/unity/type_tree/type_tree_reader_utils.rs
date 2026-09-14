/*
 * type_tree_reader_utils.rs -- Unity TypeTree value reader utility class.
 */

use crate::common::serialized_file::object_reader::ObjectReader;
use crate::common::serialized_file::serialized_file;
use crate::unity::type_tree::unity_value::UnityValue;
use std::collections::HashMap;

// TypeTree value reader utility class
// Follows Soul.md specification: struct + impl organization, no free functions
// ================================================================

/**
 * TypeTree value reader utility class.
 *
 * Parses the TypeTree structure of a Unity SerializedFile, recursively reading
 * binary data according to type definitions into UnityValue enum values.
 * All methods are static, hold no state, and are pure utility methods.
 */
pub struct TypeTreeReaderUtils;

impl TypeTreeReaderUtils {
    /// Read object data starting from the TypeTree root node
    pub fn read_typetree_value(
        reader: &mut ObjectReader,
        type_tree: &serialized_file::TypeTree,
        _sf: &serialized_file::SerializedFile,
    ) -> Result<UnityValue, String> {
        if type_tree.nodes.is_empty() {
            return Ok(UnityValue::Bytes(reader.data[reader.pos..].to_vec()));
        }

        // Start reading from root node
        let root = &type_tree.nodes[0];
        Self::read_node(reader, type_tree, root, 0)
    }

    /// Recursively read the value of a single TypeTree node
    fn read_node(
        reader: &mut ObjectReader,
        type_tree: &serialized_file::TypeTree,
        node: &serialized_file::TypeTreeNode,
        node_idx: usize,
    ) -> Result<UnityValue, String> {
        let value = Self::read_node_value(reader, type_tree, node, node_idx)?;
        // Unity aligns the stream to 4 bytes after nodes whose meta_flag carries
        // kAlignBytesFlag (e.g. bool followed by a vector). Skipping this desyncs
        // every subsequent field; align() is a no-op when already aligned.
        if node.meta_flag & 0x4000 != 0 {
            reader.align();
        }
        Ok(value)
    }

    /// Recursively read the raw value of a single TypeTree node (no post-align)
    fn read_node_value(
        reader: &mut ObjectReader,
        type_tree: &serialized_file::TypeTree,
        node: &serialized_file::TypeTreeNode,
        node_idx: usize,
    ) -> Result<UnityValue, String> {
        let type_str = &node.type_name;
        let _name = &node.name;

        match type_str.as_str() {
            "bool" => Ok(UnityValue::Bool(reader.read_bool())),
            "UInt8" | "char" | "byte" => Ok(UnityValue::Integer(reader.read_u8() as i64)),
            "SInt8" => Ok(UnityValue::Integer(reader.read_u8() as i8 as i64)),

            // 2-byte integers
            "SInt16" | "short" => Ok(UnityValue::Integer(reader.read_i16() as i64)),
            "UInt16" | "unsigned short" => Ok(UnityValue::Integer(reader.read_u16() as i64)),

            // 4-byte integers - "int" in Unity is SInt32 (signed)
            "SInt32" | "int" => Ok(UnityValue::Integer(reader.read_i32() as i64)),
            "UInt32" | "unsigned int" => Ok(UnityValue::Integer(reader.read_u32() as i64)),

            // 8-byte integers
            "SInt64" | "long long" => Ok(UnityValue::Integer(reader.read_i64())),
            "UInt64" | "unsigned long long" => Ok(UnityValue::Integer(reader.read_u64() as i64)),

            "float" => Ok(UnityValue::Float(reader.read_f32() as f64)),
            "double" => Ok(UnityValue::Float(reader.read_f64())),

            "string" => {
                let len = reader.read_i32();
                if len <= 0 {
                    reader.align();
                    return Ok(UnityValue::String(String::new()));
                }
                let len_usize = len as usize;
                if len_usize > reader.remaining() {
                    reader.align();
                    return Ok(UnityValue::String(String::new()));
                }
                let end = reader.pos + len_usize;
                let s = String::from_utf8_lossy(&reader.data[reader.pos..end]).to_string();
                reader.pos = end;
                reader.align();
                Ok(UnityValue::String(s))
            }

            ty if Self::is_pptr_type(ty) => {
                let file_id = reader.read_i32();
                // PPtr pathID: Unity < v14 -> 32-bit, >= v14 -> 64-bit
                let path_id = if reader.header_version < 14 {
                    reader.read_i32() as i64
                } else {
                    reader.read_i64()
                };
                Ok(UnityValue::Ptr { file_id, path_id })
            }

            _ => {
                if type_str.starts_with("vector") || type_str == "Array" {
                    let count = reader.read_i32() as usize;
                    if count > 100000 {
                        return Ok(UnityValue::Array(Vec::new()));
                    }
                    // Resolve vector's element node
                    let element_node_idx = Self::resolve_vector_element_node(type_tree, node_idx)?;
                    let child_node = &type_tree.nodes[element_node_idx];
                    let mut items = Vec::with_capacity(count);
                    for _ in 0..count {
                        items.push(Self::read_node(
                            reader,
                            type_tree,
                            child_node,
                            element_node_idx,
                        )?);
                    }
                    reader.align();
                    Ok(UnityValue::Array(items))
                } else {
                    // "map" type: in Unity TypeTree, "map" is equivalent to vector<Pair<K,V>>,
                    // with the same binary format (size + N elements).
                    // However, TypeTree wraps "map" in an Array -> data -> element layer,
                    // requiring traversal through the wrapper to find the actual element node.
                    // "pair" type: 2 fixed child fields "first"/"second",
                    // read by position to avoid key misalignment from unresolved CommonString.
                    if type_str == "pair" {
                        reader.align();
                        let children = Self::find_children(type_tree, node_idx);
                        if children.len() < 2 {
                            return Err("pair type missing child nodes".into());
                        }
                        let first_node = &type_tree.nodes[children[0]];
                        let second_node = &type_tree.nodes[children[1]];
                        let first = Self::read_node(reader, type_tree, first_node, children[0])?;
                        let second = Self::read_node(reader, type_tree, second_node, children[1])?;
                        let mut map = HashMap::new();
                        map.insert("first".to_string(), first);
                        map.insert("second".to_string(), second);
                        return Ok(UnityValue::Object(map));
                    }

                    if type_str == "map" {
                        let count = reader.read_i32() as usize;
                        if count > 100000 {
                            return Ok(UnityValue::Array(Vec::new()));
                        }
                        // Find the Array child node under map, then locate the element type corresponding to "data"
                        let children = Self::find_children(type_tree, node_idx);
                        let array_idx = children
                            .iter()
                            .find(|&&i| type_tree.nodes[i].type_name == "Array")
                            .copied()
                            .ok_or_else(|| "Cannot find Array child node under map".to_string())?;
                        let element_node_idx =
                            Self::resolve_vector_element_node(type_tree, array_idx)?;
                        let child_node = &type_tree.nodes[element_node_idx];
                        let mut items = Vec::with_capacity(count);
                        for _ in 0..count {
                            items.push(Self::read_node(
                                reader,
                                type_tree,
                                child_node,
                                element_node_idx,
                            )?);
                        }
                        reader.align();
                        Ok(UnityValue::Array(items))
                    } else {
                        // Complex type: align to 4 bytes, then read child fields
                        // -- Iteration 3: handle cases where TypeTree type_name cannot be resolved --
                        // When type_name is CommonString_N, it means the CommonString table is missing that index,
                        // requiring type inference based on child node structure.
                        let is_unresolved_type = type_str.starts_with("CommonString_");
                        let children = Self::find_children(type_tree, node_idx);

                        if is_unresolved_type && children.is_empty() {
                            // Unresolved type with no children -> leaf type (e.g. float, int, string)
                            // Infer actual type based on byte_size and read
                            let byte_size = node.byte_size as usize;
                            if byte_size == 0 || byte_size > reader.remaining() {
                                return Ok(UnityValue::Null);
                            }
                            let end = reader.pos + byte_size;
                            let bytes = reader.data[reader.pos..end].to_vec();
                            reader.pos = end;
                            reader.align();
                            // For common sizes, try to interpret as specific types
                            match byte_size {
                                4 => {
                                    // May be float or int32
                                    let val = f32::from_le_bytes([
                                        bytes[0], bytes[1], bytes[2], bytes[3],
                                    ]);
                                    Ok(UnityValue::Float(val as f64))
                                }
                                8 => {
                                    let val = f64::from_le_bytes([
                                        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
                                        bytes[6], bytes[7],
                                    ]);
                                    Ok(UnityValue::Float(val))
                                }
                                1 => Ok(UnityValue::Bool(bytes[0] != 0)),
                                _ => Ok(UnityValue::Bytes(bytes)),
                            }
                        } else if is_unresolved_type && !children.is_empty() {
                            // Has children -> attempt to infer if it is map-like structure
                            // Check ALL children for Array type (internal array of map<K,V>)
                            let array_child_idx = children
                                .iter()
                                .find(|&&i| type_tree.nodes[i].type_name == "Array")
                                .copied();
                            if let Some(array_idx) = array_child_idx {
                                // Read as map format: size + N elements
                                reader.align();
                                let count = reader.read_i32() as usize;
                                if count > 100000 {
                                    return Ok(UnityValue::Array(Vec::new()));
                                }
                                let element_node_idx =
                                    Self::resolve_vector_element_node(type_tree, array_idx)?;
                                let child_node = &type_tree.nodes[element_node_idx];
                                let mut items = Vec::with_capacity(count);
                                for _ in 0..count {
                                    items.push(Self::read_node(
                                        reader,
                                        type_tree,
                                        child_node,
                                        element_node_idx,
                                    )?);
                                }
                                reader.align();
                                return Ok(UnityValue::Array(items));
                            } else if node.byte_size < 0 {
                                // byte_size < 0 -> variable-length container type. Infer based on name and child structure:
                                // - m_TexEnvs/m_Floats/m_Colors -> map (count + N * (FastPropertyName + value))
                                // - 2 children -> pair (no count prefix, read 2 fields directly)
                                // - 1 child that recursively resolves to pair -> pair wrapper (delegate to child)
                                // - others -> fall back to standard struct path
                                let is_property_map = node.name == "m_TexEnvs"
                                    || node.name == "m_Floats"
                                    || node.name == "m_Colors";
                                if is_property_map && !children.is_empty() {
                                    // Material property maps: count + N * (FastPropertyName + value)
                                    // key (FastPropertyName) is implicit in the map, needs manual reading
                                    reader.align();
                                    let count_raw = reader.read_i32();
                                    let count = count_raw as usize;
                                    if count > 100000 {
                                        return Ok(UnityValue::Array(Vec::new()));
                                    }
                                    let mut items = Vec::with_capacity(count);
                                    for _ in 0..count {
                                        // Read key: FastPropertyName = aligned_string + hash(i32)
                                        let key_len = reader.read_i32();
                                        let key_str = if key_len > 0
                                            && (key_len as usize) <= reader.remaining()
                                        {
                                            let end = reader.pos + key_len as usize;
                                            let s = String::from_utf8_lossy(
                                                &reader.data[reader.pos..end],
                                            )
                                            .to_string();
                                            reader.pos = end;
                                            reader.align();
                                            // Skip hash (i32)
                                            s
                                        } else {
                                            reader.align();
                                            String::new()
                                        };
                                        let mut val_map = HashMap::new();
                                        for &child_idx in &children {
                                            let child = &type_tree.nodes[child_idx];
                                            let val = Self::read_node(
                                                reader, type_tree, child, child_idx,
                                            )?;
                                            val_map.insert(child.name.clone(), val);
                                        }
                                        // Construct pair: { "first": key_str, "second": value_object }
                                        let mut pair = HashMap::new();
                                        pair.insert(
                                            "first".to_string(),
                                            UnityValue::String(key_str),
                                        );
                                        pair.insert(
                                            "second".to_string(),
                                            UnityValue::Object(val_map),
                                        );
                                        items.push(UnityValue::Object(pair));
                                    }
                                    return Ok(UnityValue::Array(items));
                                }
                                if children.len() == 2 {
                                    // Direct pair structure: {first, second}
                                    reader.align();
                                    let first = Self::read_node(
                                        reader,
                                        type_tree,
                                        &type_tree.nodes[children[0]],
                                        children[0],
                                    )?;
                                    let second = Self::read_node(
                                        reader,
                                        type_tree,
                                        &type_tree.nodes[children[1]],
                                        children[1],
                                    )?;
                                    let mut map = HashMap::new();
                                    map.insert("first".to_string(), first);
                                    map.insert("second".to_string(), second);
                                    return Ok(UnityValue::Object(map));
                                }
                                if children.len() == 1 && Self::is_pair_like(type_tree, children[0])
                                {
                                    // Pair wrapper: skip the wrapper, delegate directly to child
                                    reader.align();
                                    return Self::read_node(
                                        reader,
                                        type_tree,
                                        &type_tree.nodes[children[0]],
                                        children[0],
                                    );
                                }
                                // For other byte_size < 0 types, fall back to standard struct path
                                reader.align();
                                let mut map = HashMap::new();
                                for child_idx in &children {
                                    let child = &type_tree.nodes[*child_idx];
                                    let val =
                                        Self::read_node(reader, type_tree, child, *child_idx)?;
                                    map.insert(child.name.clone(), val);
                                }
                                return Ok(UnityValue::Object(map));
                            }
                            // Otherwise follow standard complex type path (struct/object)
                            reader.align();
                            let mut map = HashMap::new();
                            for child_idx in children {
                                let child = &type_tree.nodes[child_idx];
                                let val = Self::read_node(reader, type_tree, child, child_idx)?;
                                map.insert(child.name.clone(), val);
                            }
                            Ok(UnityValue::Object(map))
                        } else {
                            // Normal complex type path
                            reader.align();
                            let mut map = HashMap::new();
                            for child_idx in children {
                                let child = &type_tree.nodes[child_idx];
                                let val = Self::read_node(reader, type_tree, child, child_idx)?;
                                map.insert(child.name.clone(), val);
                            }
                            Ok(UnityValue::Object(map))
                        }
                    }
                }
            }
        }
    }

    /// Determine if node is pair-like (has exactly 2 children representing first/second).
    fn is_pair_like(type_tree: &serialized_file::TypeTree, node_idx: usize) -> bool {
        let node = &type_tree.nodes[node_idx];
        let parent_level = node.level;
        let mut count = 0usize;
        for i in (node_idx + 1)..type_tree.nodes.len() {
            let n = &type_tree.nodes[i];
            if n.level == parent_level + 1 {
                count += 1;
            } else if n.level <= parent_level {
                break;
            }
        }
        count == 2
    }

    /// Determine if type name is a PPtr reference type.
    fn is_pptr_type(type_name: &str) -> bool {
        type_name == "PPtr" || type_name == "PPtr<Object>" || type_name.starts_with("PPtr<")
    }

    /// Resolve the actual element node index for vector/Array.
    fn resolve_vector_element_node(
        type_tree: &serialized_file::TypeTree,
        vector_node_idx: usize,
    ) -> Result<usize, String> {
        // There are several common shapes here:
        // 1. vector -> Array -> data -> <element>
        // 2. vector -> Array -> <element data>
        // 3. Array  -> data -> <element>
        // 4. Array  -> <element data>
        //
        // Real MonoBehaviour typetrees also commonly encode the vector node's direct child as
        // `Array` rather than a child literally named `data`, so we first normalize to the
        // immediate Array/data wrapper node, then resolve the actual element beneath it.
        let node = &type_tree.nodes[vector_node_idx];

        let wrapper_idx = if node.type_name == "Array" {
            vector_node_idx
        } else if let Some(idx) = Self::find_children_by_name(type_tree, vector_node_idx, "data")
            .first()
            .copied()
        {
            idx
        } else if let Some(idx) = Self::find_children(type_tree, vector_node_idx)
            .into_iter()
            .find(|&idx| type_tree.nodes[idx].type_name == "Array")
        {
            idx
        } else {
            return Err(format!(
                "vector node {} missing Array/data child node",
                vector_node_idx
            ));
        };

        let wrapper_node = &type_tree.nodes[wrapper_idx];
        if wrapper_node.type_name == "Array" {
            let grand_children = Self::find_children_by_name(type_tree, wrapper_idx, "data");
            if let Some(idx) = grand_children.first() {
                return Ok(*idx);
            }
            let all_grand_children = Self::find_children(type_tree, wrapper_idx);
            if let Some(idx) = all_grand_children
                .iter()
                .copied()
                .find(|&idx| type_tree.nodes[idx].name != "size")
                .or_else(|| all_grand_children.first().copied())
            {
                return Ok(idx);
            }
        }

        Ok(wrapper_idx)
    }

    /// Get all direct child node indices for the specified node in TypeTree
    fn find_children(type_tree: &serialized_file::TypeTree, parent_idx: usize) -> Vec<usize> {
        let parent_level = type_tree.nodes[parent_idx].level;
        let mut children = Vec::new();
        for i in (parent_idx + 1)..type_tree.nodes.len() {
            let node = &type_tree.nodes[i];
            if node.level == parent_level + 1 {
                children.push(i);
            } else if node.level <= parent_level {
                break;
            }
        }
        children
    }

    /// Find all direct child nodes with the specified name.
    fn find_children_by_name(
        type_tree: &serialized_file::TypeTree,
        parent_idx: usize,
        name: &str,
    ) -> Vec<usize> {
        let parent_level = type_tree.nodes[parent_idx].level;
        let mut result = Vec::new();
        for i in (parent_idx + 1)..type_tree.nodes.len() {
            let node = &type_tree.nodes[i];
            if node.level == parent_level + 1 {
                if node.name == name {
                    result.push(i);
                }
            } else if node.level <= parent_level {
                break;
            }
        }

        if !result.is_empty() {
            return result;
        }

        // Fallback for unresolved CommonString names: "data" is usually the second direct
        // child under vector/Array, while "size" is the first.
        let children = Self::find_children(type_tree, parent_idx);
        match name {
            "data" => children.get(1).copied().into_iter().collect(),
            "size" => children.first().copied().into_iter().collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TypeTreeReaderUtils;
    use crate::common::serialized_file::serialized_file::{TypeTree, TypeTreeNode};

    fn node(level: u8, type_name: &str, name: &str) -> TypeTreeNode {
        TypeTreeNode {
            level,
            type_name: type_name.to_string(),
            name: name.to_string(),
            byte_size: 0,
            index: 0,
            type_flags: 0,
            version: 1,
            meta_flag: 0,
            type_str_offset: 0,
            name_str_offset: 0,
        }
    }

    #[test]
    fn resolve_vector_element_node_handles_array_child_without_named_data_grandchild() {
        let type_tree = TypeTree {
            nodes: vec![
                node(0, "Root", "Base"),
                node(1, "vector", "m_Items"),
                node(2, "Array", "data"),
                node(3, "int", "size"),
                node(3, "PPtr<Object>", "CommonString_106"),
                node(4, "int", "m_FileID"),
                node(4, "SInt64", "m_PathID"),
            ],
            string_buffer: Vec::new(),
        };

        let idx = TypeTreeReaderUtils::resolve_vector_element_node(&type_tree, 1)
            .expect("resolve vector element");
        assert_eq!(idx, 4);
    }

    #[test]
    fn resolve_vector_element_node_handles_vector_with_direct_array_child() {
        let type_tree = TypeTree {
            nodes: vec![
                node(0, "Root", "Base"),
                node(1, "vector", "buffFxGroup"),
                node(2, "Array", "Array"),
                node(3, "int", "size"),
                node(3, "PPtr<Object>", "data"),
                node(4, "int", "m_FileID"),
                node(4, "SInt64", "m_PathID"),
            ],
            string_buffer: Vec::new(),
        };

        let idx = TypeTreeReaderUtils::resolve_vector_element_node(&type_tree, 1)
            .expect("resolve vector element");
        assert_eq!(idx, 4);
    }

    #[test]
    fn read_node_aligns_after_flagged_bool_before_vector() {
        use crate::common::serialized_file::object_reader::ObjectReader;
        use crate::common::serialized_file::serialized_file::SerializedFile;
        use crate::unity::type_tree::unity_value::UnityValue;

        fn flagged_node(level: u8, type_name: &str, name: &str, meta_flag: i32) -> TypeTreeNode {
            TypeTreeNode {
                level,
                type_name: type_name.to_string(),
                name: name.to_string(),
                byte_size: 0,
                index: 0,
                type_flags: 0,
                version: 1,
                meta_flag,
                type_str_offset: 0,
                name_str_offset: 0,
            }
        }

        // Mirrors the ParticleSystemRenderer tail: bool (aligned) followed by a
        // UInt8 vector, then a trailing int. Without flag-driven alignment the
        // vector count is read from the bool padding and every field desyncs.
        let type_tree = TypeTree {
            nodes: vec![
                flagged_node(0, "ParticleSystemRenderer", "Base", 0),
                flagged_node(1, "bool", "m_UseCustomVertexStreams", 0x4000),
                flagged_node(1, "vector", "m_VertexStreams", 0x4000),
                flagged_node(2, "Array", "Array", 0x4000),
                flagged_node(3, "int", "size", 0),
                flagged_node(3, "UInt8", "data", 0),
                flagged_node(1, "int", "m_RenderMode", 0),
            ],
            string_buffer: Vec::new(),
        };

        let mut bytes = Vec::new();
        bytes.push(1u8); // m_UseCustomVertexStreams = true
        bytes.extend_from_slice(&[0, 0, 0]); // alignment padding
        bytes.extend_from_slice(&2i32.to_le_bytes()); // vector count
        bytes.extend_from_slice(&[7u8, 9u8]); // stream ids
        bytes.extend_from_slice(&[0, 0]); // alignment padding
        bytes.extend_from_slice(&4i32.to_le_bytes()); // m_RenderMode = Mesh

        fn empty_serialized_file() -> SerializedFile {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&0u32.to_be_bytes()); // metadata size placeholder
            bytes.extend_from_slice(&0u32.to_be_bytes()); // file size placeholder
            bytes.extend_from_slice(&17u32.to_be_bytes()); // format version
            bytes.extend_from_slice(&0u32.to_be_bytes()); // data offset placeholder
            bytes.push(0); // little endian
            bytes.extend_from_slice(&[0, 0, 0]);
            bytes.extend_from_slice(b"2017.4.8f1");
            bytes.push(0);
            bytes.extend_from_slice(&5i32.to_le_bytes()); // platform
            bytes.push(0); // disable TypeTree
            bytes.extend_from_slice(&0i32.to_le_bytes()); // types
            bytes.extend_from_slice(&0i32.to_le_bytes()); // objects
            bytes.extend_from_slice(&0i32.to_le_bytes()); // script types
            bytes.extend_from_slice(&0i32.to_le_bytes()); // externals
            bytes.push(0); // user information
            let data_offset = bytes.len() as u32;
            let metadata_size = data_offset - 20;
            bytes[0..4].copy_from_slice(&metadata_size.to_be_bytes());
            bytes[4..8].copy_from_slice(&data_offset.to_be_bytes());
            bytes[12..16].copy_from_slice(&data_offset.to_be_bytes());
            SerializedFile::parse(&bytes).expect("parse minimal serialized file")
        }

        let sf = empty_serialized_file();
        let mut reader = ObjectReader::new(&bytes, &sf);
        let value = TypeTreeReaderUtils::read_typetree_value(&mut reader, &type_tree, &sf)
            .expect("read typetree value");

        let UnityValue::Object(map) = value else {
            panic!("expected object value");
        };
        assert!(matches!(map.get("m_UseCustomVertexStreams"), Some(UnityValue::Bool(true))));
        assert!(
            matches!(map.get("m_VertexStreams"), Some(UnityValue::Array(items)) if items.len() == 2)
        );
        assert!(matches!(map.get("m_RenderMode"), Some(UnityValue::Integer(4))));
    }
}
