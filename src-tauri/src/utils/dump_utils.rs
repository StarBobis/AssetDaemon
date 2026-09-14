/*
 * Dump Format Utility Class - Formats Unity TypeTree into AssetStudio-style indented text.
 *
 * Operates directly on TypeTree + ObjectReader, preserving type names,
 * Output format such as: int m_SomeInt: 42
 *              string m_Name: "hello"
 *              PPtr<GameObject> m_GameObject: {fileID: 0, pathID: 456}
 *
 * Type reading logic is a 1:1 replica of AssetStudio TypeTreeHelper / SerializedTypeHelper.
 *
 * Follows Soul.md convention: organized as struct + impl, no free functions.
 */

use crate::common::serialized_file::object_reader::ObjectReader;
use crate::common::serialized_file::serialized_file;

/**
 * Dump format parameters.
 */
#[derive(Clone, Copy)]
pub struct FormatParams {
    /// Maximum recursion depth
    pub max_depth: usize,
    /// Maximum number of array items to display
    pub max_array_items: usize,
    /// Maximum display length for strings
    pub max_string_len: usize,
}

impl Default for FormatParams {
    fn default() -> Self {
        Self {
            max_depth: 20,
            max_array_items: 100,
            max_string_len: 500,
        }
    }
}

/**
 * Dump format state (tracks truncation).
 */
#[derive(Default)]
pub struct FormatState {
    /// Whether truncation occurred
    pub truncated: bool,
    /// Number of truncated items
    pub truncated_items: usize,
}

/**
 * Dump Format Utility Class.
 *
 * Formats Unity TypeTree into AssetStudio-style indented text.
 * Reads type names and field names directly from TypeTree nodes, reads values from ObjectReader.
 */
pub struct DumpUtils;

impl DumpUtils {
    /// Generates dump text starting from the TypeTree root node.
    ///
    /// Skips the root node itself (container), outputs directly from its children (level 1).
    pub fn format_typetree_dump(
        reader: &mut ObjectReader,
        type_tree: &serialized_file::TypeTree,
        params: &FormatParams,
        state: &mut FormatState,
    ) -> Result<String, String> {
        if type_tree.nodes.is_empty() {
            return Ok(String::new());
        }

        let children = Self::find_children(type_tree, 0);
        let mut lines: Vec<String> = Vec::new();

        for child_idx in children {
            let line = Self::format_node(reader, type_tree, child_idx, 0, 0, params, state)?;
            lines.push(line);
        }

        // Separate fields with blank lines
        Ok(lines.join("\n"))
    }

    /// Recursively formats the value of a single TypeTree node.
    fn format_node(
        reader: &mut ObjectReader,
        type_tree: &serialized_file::TypeTree,
        node_idx: usize,
        indent: usize,
        depth: usize,
        params: &FormatParams,
        state: &mut FormatState,
    ) -> Result<String, String> {
        let node = &type_tree.nodes[node_idx];
        let prefix = " ".repeat(indent);
        let type_name = &node.type_name;
        let field_name = &node.name;

        // Depth limit
        if depth > params.max_depth {
            state.truncated = true;
            return Ok(format!(
                "{}{} {} = <max depth reached>",
                prefix, type_name, field_name
            ));
        }

        match type_name.as_str() {
            // ---- Boolean ----
            "bool" => {
                let val = reader.read_bool();
                Ok(format!("{}{} {} = {}", prefix, type_name, field_name, val))
            }

            // ---- 1-byte integer ----
            "UInt8" | "char" | "byte" => {
                let val = reader.read_u8();
                Ok(format!("{}{} {} = {}", prefix, type_name, field_name, val))
            }
            "SInt8" => {
                let val = reader.read_u8() as i8;
                Ok(format!("{}{} {} = {}", prefix, type_name, field_name, val))
            }

            // ---- 2-byte integer ----
            "SInt16" | "short" => {
                let val = reader.read_i16();
                Ok(format!("{}{} {} = {}", prefix, type_name, field_name, val))
            }
            "UInt16" | "unsigned short" => {
                let val = reader.read_u16();
                Ok(format!("{}{} {} = {}", prefix, type_name, field_name, val))
            }

            // ---- 4-byte integer ----
            "SInt32" | "int" => {
                let val = reader.read_i32();
                Ok(format!("{}{} {} = {}", prefix, type_name, field_name, val))
            }
            "UInt32" | "unsigned int" => {
                let val = reader.read_u32();
                Ok(format!("{}{} {} = {}", prefix, type_name, field_name, val))
            }

            // ---- 8-byte integer ----
            "SInt64" | "long long" => {
                let val = reader.read_i64();
                Ok(format!("{}{} {} = {}", prefix, type_name, field_name, val))
            }
            "UInt64" | "unsigned long long" => {
                let val = reader.read_u64();
                Ok(format!("{}{} {} = {}", prefix, type_name, field_name, val))
            }

            // ---- Float ----
            "float" => {
                let val = reader.read_f32();
                Ok(format!("{}{} {} = {}", prefix, type_name, field_name, val))
            }
            "double" => {
                let val = reader.read_f64();
                Ok(format!("{}{} {} = {}", prefix, type_name, field_name, val))
            }

            // ---- String (AlignedString) ----
            "string" => Self::format_string(reader, type_name, field_name, &prefix, params, state),

            // ---- PPtr Reference ----
            ty if Self::is_pptr_type(ty) => {
                let file_id = reader.read_i32();
                // PathID size depends on Unity version
                // version < 14 -> int (32-bit), version >= 14 -> long (64-bit)
                let path_id = if reader.header_version < 14 {
                    reader.read_i32() as i64
                } else {
                    reader.read_i64()
                };
                Ok(format!(
                    "{}{} {} = {{fileID: {}, pathID: {}}}",
                    prefix, type_name, field_name, file_id, path_id
                ))
            }

            // ---- Array / Vector ----
            ty if ty.starts_with("vector") || ty == "Array" => Self::format_vector(
                reader, type_tree, node_idx, &prefix, type_name, field_name, depth, params, state,
            ),

            // ---- Complex type (struct / class) ----
            _ => Self::format_complex(
                reader, type_tree, node_idx, &prefix, type_name, field_name, indent, depth, params,
                state,
            ),
        }
    }

    /// Formats a string field.
    fn format_string(
        reader: &mut ObjectReader,
        type_name: &str,
        field_name: &str,
        prefix: &str,
        params: &FormatParams,
        state: &mut FormatState,
    ) -> Result<String, String> {
        let len = reader.read_i32();
        if len <= 0 {
            // Empty string -- still need to align after reading
            reader.align();
            return Ok(format!("{}{} {} = \"\"", prefix, type_name, field_name));
        }

        let len_usize = len as usize;
        if len_usize > reader.remaining() {
            // Data out of bounds, skip and try to recover
            reader.align();
            state.truncated = true;
            return Ok(format!(
                "{}{} {} = \"<invalid string: len={} exceeds remaining={}>\"",
                prefix,
                type_name,
                field_name,
                len,
                reader.remaining()
            ));
        }

        let end = reader.pos + len_usize;
        // Safe: already checked remaining >= len_usize above
        let raw = &reader.data[reader.pos..end];
        let s = String::from_utf8_lossy(raw).to_string();
        reader.pos = end;
        reader.align();

        if s.len() > params.max_string_len {
            state.truncated = true;
            let truncated = &s[..params.max_string_len];
            Ok(format!(
                "{}{} {} = \"{}\"... (truncated, total={} chars)",
                prefix,
                type_name,
                field_name,
                truncated,
                s.len()
            ))
        } else {
            Ok(format!(
                "{}{} {} = \"{}\"",
                prefix, type_name, field_name, s
            ))
        }
    }

    /// Formats vector/Array field.
    fn format_vector(
        reader: &mut ObjectReader,
        type_tree: &serialized_file::TypeTree,
        node_idx: usize,
        prefix: &str,
        type_name: &str,
        field_name: &str,
        depth: usize,
        params: &FormatParams,
        state: &mut FormatState,
    ) -> Result<String, String> {
        let count = reader.read_i32() as usize;

        // Guard: oversized array
        if count > 100_000 {
            return Ok(format!(
                "{}{} {} = <{} items, skipped for safety>",
                prefix, type_name, field_name, count
            ));
        }

        // Empty array
        if count == 0 {
            return Ok(format!(
                "{}{} {} = [] (0 items)",
                prefix, type_name, field_name
            ));
        }

        // Find array element node -- traverse vector's "data" child nodes
        // TypeTree structure: vector Array -> Array data -> <element_type>
        // Or directly: vector Array -> <element_type> data (some Unity versions)
        let element_node_idx = Self::resolve_vector_element_node(type_tree, node_idx)?;

        let display_count = count.min(params.max_array_items);
        let overflow = count.saturating_sub(params.max_array_items);

        if overflow > 0 {
            state.truncated = true;
            state.truncated_items += overflow;
        }

        let mut result = format!(
            "{}{} {} = ({} items)\n",
            prefix, type_name, field_name, count
        );

        for i in 0..display_count {
            let item_prefix = format!("{}  [{}] ", prefix, i);
            let child_line = Self::format_node_as_item(
                reader,
                type_tree,
                element_node_idx,
                &item_prefix,
                depth + 1,
                params,
                state,
            )?;
            result.push_str(&child_line);
            result.push('\n');
        }

        if overflow > 0 {
            result.push_str(&format!("{}  ... and {} more items\n", prefix, overflow));
        }

        Ok(result)
    }

    /// Formats complex type (struct / class / nested object).
    fn format_complex(
        reader: &mut ObjectReader,
        type_tree: &serialized_file::TypeTree,
        node_idx: usize,
        prefix: &str,
        type_name: &str,
        field_name: &str,
        indent: usize,
        depth: usize,
        params: &FormatParams,
        state: &mut FormatState,
    ) -> Result<String, String> {
        // AssetStudio: complex types align to 4 bytes before reading child fields
        reader.align();

        let children = Self::find_children(type_tree, node_idx);
        if children.is_empty() {
            return Ok(format!("{}{} {} = {{}}", prefix, type_name, field_name));
        }

        // First line: type name and field name, child fields indented 4 spaces after newline
        let mut result = format!("{}{} {}\n", prefix, type_name, field_name);
        let child_indent = indent + 4;

        for (i, child_idx) in children.iter().enumerate() {
            if i > 0 {
                result.push('\n');
            }
            let line = Self::format_node(
                reader,
                type_tree,
                *child_idx,
                child_indent,
                depth + 1,
                params,
                state,
            )?;
            result.push_str(&line);
        }

        // No align needed after complex type ends -- handled by caller (format_node)
        // But if inside an array, no alignment needed (array elements are contiguous)
        Ok(result)
    }

    /// Formats a child node as an array item.
    ///
    /// `item_prefix` format is "  [0] ", includes indentation and index.
    /// Simple type values follow directly after prefix, complex types create a new line.
    fn format_node_as_item(
        reader: &mut ObjectReader,
        type_tree: &serialized_file::TypeTree,
        node_idx: usize,
        item_prefix: &str,
        depth: usize,
        params: &FormatParams,
        state: &mut FormatState,
    ) -> Result<String, String> {
        let node = &type_tree.nodes[node_idx];
        let type_name = &node.type_name;

        if depth > params.max_depth {
            state.truncated = true;
            return Ok(format!("{}<max depth reached>", item_prefix));
        }

        match type_name.as_str() {
            // ---- Simple type: follows directly after [N] ----
            "bool" => {
                let val = reader.read_bool();
                Ok(format!("{}{} = {}", item_prefix, type_name, val))
            }
            "UInt8" | "char" | "byte" => {
                let val = reader.read_u8();
                Ok(format!("{}{} = {}", item_prefix, type_name, val))
            }
            "SInt8" => {
                let val = reader.read_u8() as i8;
                Ok(format!("{}{} = {}", item_prefix, type_name, val))
            }
            "SInt16" | "short" => {
                let val = reader.read_i16();
                Ok(format!("{}{} = {}", item_prefix, type_name, val))
            }
            "UInt16" | "unsigned short" => {
                let val = reader.read_u16();
                Ok(format!("{}{} = {}", item_prefix, type_name, val))
            }
            "SInt32" | "int" => {
                let val = reader.read_i32();
                Ok(format!("{}{} = {}", item_prefix, type_name, val))
            }
            "UInt32" | "unsigned int" => {
                let val = reader.read_u32();
                Ok(format!("{}{} = {}", item_prefix, type_name, val))
            }
            "SInt64" | "long long" => {
                let val = reader.read_i64();
                Ok(format!("{}{} = {}", item_prefix, type_name, val))
            }
            "UInt64" | "unsigned long long" => {
                let val = reader.read_u64();
                Ok(format!("{}{} = {}", item_prefix, type_name, val))
            }
            "float" => {
                let val = reader.read_f32();
                Ok(format!("{}{} = {}", item_prefix, type_name, val))
            }
            "double" => {
                let val = reader.read_f64();
                Ok(format!("{}{} = {}", item_prefix, type_name, val))
            }
            "string" => {
                let len = reader.read_i32();
                if len <= 0 {
                    reader.align();
                    return Ok(format!("{}{} = \"\"", item_prefix, type_name));
                }
                let len_usize = len as usize;
                if len_usize > reader.remaining() {
                    reader.align();
                    state.truncated = true;
                    return Ok(format!("{}{} = \"<invalid>\"", item_prefix, type_name));
                }
                let end = reader.pos + len_usize;
                let s = String::from_utf8_lossy(&reader.data[reader.pos..end]).to_string();
                reader.pos = end;
                reader.align();
                if s.len() > params.max_string_len {
                    state.truncated = true;
                    let truncated = &s[..params.max_string_len];
                    Ok(format!(
                        "{}{} = \"{}\"...",
                        item_prefix, type_name, truncated
                    ))
                } else {
                    Ok(format!("{}{} = \"{}\"", item_prefix, type_name, s))
                }
            }
            ty if Self::is_pptr_type(ty) => {
                let file_id = reader.read_i32();
                let path_id = if reader.header_version < 14 {
                    reader.read_i32() as i64
                } else {
                    reader.read_i64()
                };
                Ok(format!(
                    "{}{} = {{fileID: {}, pathID: {}}}",
                    item_prefix, type_name, file_id, path_id
                ))
            }
            ty if ty.starts_with("vector") || ty == "Array" => {
                // Nested array: read recursively
                let count = reader.read_i32() as usize;
                if count > 100_000 {
                    return Ok(format!(
                        "{}{} = <{} items, skipped>",
                        item_prefix, type_name, count
                    ));
                }
                if count == 0 {
                    return Ok(format!("{}{} = []", item_prefix, type_name));
                }
                let element_node_idx = Self::resolve_vector_element_node(type_tree, node_idx)?;
                let display_count = count.min(params.max_array_items);
                let overflow = count.saturating_sub(params.max_array_items);
                if overflow > 0 {
                    state.truncated = true;
                    state.truncated_items += overflow;
                }
                let mut result = format!("{}{} = ({} items)\n", item_prefix, type_name, count);
                for i in 0..display_count {
                    let child_prefix = format!("{}  [{}] ", item_prefix, i);
                    let child_line = Self::format_node_as_item(
                        reader,
                        type_tree,
                        element_node_idx,
                        &child_prefix,
                        depth + 1,
                        params,
                        state,
                    )?;
                    result.push_str(&child_line);
                    result.push('\n');
                }
                if overflow > 0 {
                    result.push_str(&format!(
                        "{}  ... and {} more items\n",
                        item_prefix, overflow
                    ));
                }
                Ok(result)
            }
            _ => {
                // Complex type: align then recurse
                reader.align();
                let children = Self::find_children(type_tree, node_idx);
                if children.is_empty() {
                    return Ok(format!("{}{} = {{}}", item_prefix, type_name));
                }
                let mut result = format!("{}{}\n", item_prefix, type_name);
                let child_indent = item_prefix.len() + 2;
                for (i, child_idx) in children.iter().enumerate() {
                    if i > 0 {
                        result.push('\n');
                    }
                    let line = Self::format_node(
                        reader,
                        type_tree,
                        *child_idx,
                        child_indent,
                        depth + 1,
                        params,
                        state,
                    )?;
                    result.push_str(&line);
                }
                Ok(result)
            }
        }
    }

    /// Checks if a type name is a PPtr reference type.
    fn is_pptr_type(type_name: &str) -> bool {
        type_name.starts_with("PPtr")
            || type_name == "PPtr<Object>"
            || type_name.starts_with("PPtr<")
    }

    /// Resolves the actual element node index of a vector/Array.
    ///
    /// There are two structures for vector in TypeTree:
    /// 1. vector Array -> Array data -> <element_type> (two levels of data)
    /// 2. vector Array -> <element_type> data    (one level of data)
    ///
    /// This method returns the node index of the actual element type.
    fn resolve_vector_element_node(
        type_tree: &serialized_file::TypeTree,
        vector_node_idx: usize,
    ) -> Result<usize, String> {
        // Find direct child node named "data"
        let data_children = Self::find_children_by_name(type_tree, vector_node_idx, "data");
        if data_children.is_empty() {
            return Err(format!(
                "vector node {} missing 'data' child node",
                vector_node_idx
            ));
        }

        let data_idx = data_children[0];
        let data_node = &type_tree.nodes[data_idx];

        // If the "data" child node itself is of type "Array", the element is at its next level
        if data_node.type_name == "Array" {
            let grand_children = Self::find_children_by_name(type_tree, data_idx, "data");
            if let Some(gc_idx) = grand_children.first() {
                return Ok(*gc_idx);
            }
            // No deeper "data", use the first non-Array among all direct children of Array
            let all_grand_children = Self::find_children(type_tree, data_idx);
            if let Some(gc_idx) = all_grand_children.first() {
                return Ok(*gc_idx);
            }
        }

        Ok(data_idx)
    }

    /// Gets all direct child node indices of the specified node in TypeTree.
    fn find_children(type_tree: &serialized_file::TypeTree, parent_idx: usize) -> Vec<usize> {
        if parent_idx >= type_tree.nodes.len() {
            return Vec::new();
        }
        let parent_level = type_tree.nodes[parent_idx].level;
        let mut children = Vec::new();
        for i in (parent_idx + 1)..type_tree.nodes.len() {
            let node_level = type_tree.nodes[i].level;
            if node_level == parent_level + 1 {
                children.push(i);
            } else if node_level <= parent_level {
                break;
            }
        }
        children
    }

    /// Finds direct child nodes with the specified name in TypeTree.
    fn find_children_by_name(
        type_tree: &serialized_file::TypeTree,
        parent_idx: usize,
        name: &str,
    ) -> Vec<usize> {
        if parent_idx >= type_tree.nodes.len() {
            return Vec::new();
        }
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
        result
    }

    /// Exports TypeTree as structured JSON (used for file export of types like MonoBehaviour).
    ///
    /// Unlike `format_typetree_dump`, this method outputs `serde_json::Value`
    /// instead of formatted text dump.
    /// - PPtr references -> {"fileID": N, "pathID": N}
    /// - Nested objects -> nested JSON object
    /// - Arrays -> JSON array
    /// - Truncation protection reuses `FormatParams`
    pub fn export_typetree_json(
        reader: &mut ObjectReader,
        type_tree: &serialized_file::TypeTree,
        params: &FormatParams,
    ) -> Result<serde_json::Value, String> {
        use serde_json::Value;

        if type_tree.nodes.is_empty() {
            return Ok(Value::Null);
        }

        let children = Self::find_children(type_tree, 0);
        let mut map = serde_json::Map::new();

        for child_idx in children {
            let node = &type_tree.nodes[child_idx];
            let val = Self::read_node_json(reader, type_tree, child_idx, 0, params)?;
            map.insert(node.name.clone(), val);
        }

        Ok(Value::Object(map))
    }

    /// Recursively reads a node and returns a JSON Value.
    fn read_node_json(
        reader: &mut ObjectReader,
        type_tree: &serialized_file::TypeTree,
        node_idx: usize,
        depth: usize,
        params: &FormatParams,
    ) -> Result<serde_json::Value, String> {
        use serde_json::{json, Value};

        let node = &type_tree.nodes[node_idx];
        let type_name = &node.type_name;

        if depth > params.max_depth {
            return Ok(Value::String("<max depth reached>".to_string()));
        }

        match type_name.as_str() {
            "bool" => Ok(Value::Bool(reader.read_bool())),
            "UInt8" | "char" | "byte" => Ok(json!(reader.read_u8())),
            "SInt8" => Ok(json!(reader.read_u8() as i8)),
            "SInt16" | "short" => Ok(json!(reader.read_i16())),
            "UInt16" | "unsigned short" => Ok(json!(reader.read_u16())),
            "SInt32" | "int" => Ok(json!(reader.read_i32())),
            "UInt32" | "unsigned int" => Ok(json!(reader.read_u32())),
            "SInt64" | "long long" => Ok(json!(reader.read_i64())),
            "UInt64" | "unsigned long long" => Ok(json!(reader.read_u64())),
            "float" => Ok(json!(reader.read_f32())),
            "double" => Ok(json!(reader.read_f64())),
            "string" => {
                let len = reader.read_i32();
                if len <= 0 {
                    reader.align();
                    return Ok(Value::String(String::new()));
                }
                let len_usize = len as usize;
                if len_usize > reader.remaining() {
                    reader.align();
                    return Ok(Value::String("<invalid string>".to_string()));
                }
                let end = reader.pos + len_usize;
                let s = String::from_utf8_lossy(&reader.data[reader.pos..end]).to_string();
                reader.pos = end;
                reader.align();
                Ok(Value::String(s))
            }
            ty if Self::is_pptr_type(ty) => {
                let file_id = reader.read_i32();
                let path_id = if reader.header_version < 14 {
                    reader.read_i32() as i64
                } else {
                    reader.read_i64()
                };
                Ok(json!({"fileID": file_id, "pathID": path_id}))
            }
            ty if ty.starts_with("vector") || ty == "Array" => {
                let count = reader.read_i32() as usize;
                if count > 100_000 {
                    return Ok(Value::Array(Vec::new()));
                }
                let elem_idx = Self::resolve_vector_element_node(type_tree, node_idx)?;
                let mut items = Vec::with_capacity(count.min(params.max_array_items));
                let display = count.min(params.max_array_items);
                for _ in 0..display {
                    items.push(Self::read_node_json(
                        reader,
                        type_tree,
                        elem_idx,
                        depth + 1,
                        params,
                    )?);
                }
                Ok(Value::Array(items))
            }
            _ => {
                reader.align();
                let children = Self::find_children(type_tree, node_idx);
                let mut map = serde_json::Map::new();
                for child_idx in children {
                    let child = &type_tree.nodes[child_idx];
                    let val =
                        Self::read_node_json(reader, type_tree, child_idx, depth + 1, params)?;
                    map.insert(child.name.clone(), val);
                }
                Ok(Value::Object(map))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // Unit tests for helper methods like find_children need to construct TypeTreeNode,
    // but TypeTreeNode contains private fields (type_str_offset, name_str_offset),
    // cannot be constructed from the utils module. Integration tests use real bundle files.
}
