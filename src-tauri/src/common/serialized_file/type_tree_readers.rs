/*
 * type_tree_readers.rs - Unity TypeTree metadata readers.
 *
 * SerializedFile chooses between the legacy recursive TypeTree format and the
 * modern blob TypeTree format. Both readers normalize into the same TypeTree.
 */

use std::io::{Read, Seek};

use crate::common::serialized_file::reader_utils::SerializedFileReaderUtils;
use crate::common::serialized_file::serialized_file::{format_version, TypeTree, TypeTreeNode};

pub struct TypeTreeReaders;

impl TypeTreeReaders {
    pub fn read<R: Read + Seek>(
        reader: &mut R,
        header_version: u32,
        is_le: bool,
    ) -> Result<TypeTree, String> {
        if Self::uses_blob_format(header_version) {
            BlobTypeTreeReader::read(reader, header_version, is_le)
        } else {
            LegacyRecursiveTypeTreeReader::read(reader, header_version, is_le)
        }
    }

    fn uses_blob_format(header_version: u32) -> bool {
        header_version >= format_version::UNKNOWN_12 || header_version == format_version::UNKNOWN_10
    }
}

pub struct LegacyRecursiveTypeTreeReader;

impl LegacyRecursiveTypeTreeReader {
    pub fn read<R: Read>(
        reader: &mut R,
        header_version: u32,
        is_le: bool,
    ) -> Result<TypeTree, String> {
        let mut tree = TypeTree {
            nodes: Vec::new(),
            string_buffer: Vec::new(),
        };
        Self::read_node(reader, header_version, 0, &mut tree.nodes, is_le)?;
        Ok(tree)
    }

    fn read_node<R: Read>(
        reader: &mut R,
        header_version: u32,
        level: i32,
        nodes: &mut Vec<TypeTreeNode>,
        is_le: bool,
    ) -> Result<(), String> {
        let ri32: fn(&mut dyn Read) -> Result<i32, String> = if is_le {
            SerializedFileReaderUtils::read_i32_le
        } else {
            SerializedFileReaderUtils::read_i32_be
        };

        let type_name = SerializedFileReaderUtils::read_string_to_null(reader)?;
        let name = SerializedFileReaderUtils::read_string_to_null(reader)?;
        let byte_size = ri32(reader)?;
        if header_version == format_version::UNKNOWN_2 {
            let _variable_count = ri32(reader)?;
        }
        let index = if header_version != format_version::UNKNOWN_3 {
            ri32(reader)?
        } else {
            0
        };
        let type_flags = ri32(reader)?;
        let version = ri32(reader)?;
        let meta_flag = if header_version != format_version::UNKNOWN_3 {
            ri32(reader)?
        } else {
            0
        };

        nodes.push(TypeTreeNode {
            level: level as u8,
            type_name,
            name,
            byte_size,
            index,
            type_flags,
            version,
            meta_flag,
            type_str_offset: 0,
            name_str_offset: 0,
        });

        let children_count = ri32(reader)?;
        for _ in 0..children_count {
            Self::read_node(reader, header_version, level + 1, nodes, is_le)?;
        }
        Ok(())
    }
}

pub struct BlobTypeTreeReader;

impl BlobTypeTreeReader {
    pub fn read<R: Read + Seek>(
        reader: &mut R,
        header_version: u32,
        is_le: bool,
    ) -> Result<TypeTree, String> {
        let ri32: fn(&mut dyn Read) -> Result<i32, String> = if is_le {
            SerializedFileReaderUtils::read_i32_le
        } else {
            SerializedFileReaderUtils::read_i32_be
        };
        let ru16: fn(&mut dyn Read) -> Result<u16, String> = if is_le {
            SerializedFileReaderUtils::read_u16_le
        } else {
            SerializedFileReaderUtils::read_u16_be
        };
        let ru32: fn(&mut dyn Read) -> Result<u32, String> = if is_le {
            SerializedFileReaderUtils::read_u32_le
        } else {
            SerializedFileReaderUtils::read_u32_be
        };
        let ru64: fn(&mut dyn Read) -> Result<u64, String> = if is_le {
            SerializedFileReaderUtils::read_u64_le
        } else {
            SerializedFileReaderUtils::read_u64_be
        };

        let number_of_nodes = ri32(reader)?;
        let string_buffer_size = ri32(reader)?;

        if number_of_nodes < 0 || number_of_nodes > 500_000 {
            return Err(format!(
                "Abnormal TypeTree node count ({}), file may be corrupted",
                number_of_nodes
            ));
        }

        let mut nodes = Vec::with_capacity(number_of_nodes as usize);
        for _ in 0..number_of_nodes {
            let version = ru16(reader)? as i32;
            let level = SerializedFileReaderUtils::read_u8(reader)?;
            let type_flags = SerializedFileReaderUtils::read_u8(reader)? as i32;
            let type_str_offset = ru32(reader)?;
            let name_str_offset = ru32(reader)?;
            let byte_size = ri32(reader)?;
            let index = ri32(reader)?;
            let meta_flag = ri32(reader)?;

            if header_version >= format_version::TYPE_TREE_NODE_WITH_TYPE_FLAGS {
                let _ref_type_hash = ru64(reader)?;
            }

            nodes.push(TypeTreeNode {
                level,
                type_name: String::new(),
                name: String::new(),
                byte_size,
                index,
                type_flags,
                version,
                meta_flag,
                type_str_offset,
                name_str_offset,
            });
        }

        let string_buffer =
            SerializedFileReaderUtils::read_bytes(reader, string_buffer_size as usize)?;

        for node in &mut nodes {
            node.type_name = Self::resolve_string(&string_buffer, node.type_str_offset);
            node.name = Self::resolve_string(&string_buffer, node.name_str_offset);
        }

        Ok(TypeTree {
            nodes,
            string_buffer,
        })
    }

    fn resolve_string(string_buffer: &[u8], offset_value: u32) -> String {
        let is_offset = (offset_value & 0x80000000) == 0;
        if is_offset {
            let start = offset_value as usize;
            if start < string_buffer.len() {
                let mut end = start;
                while end < string_buffer.len() && string_buffer[end] != 0 {
                    end += 1;
                }
                return String::from_utf8_lossy(&string_buffer[start..end]).to_string();
            }
            String::new()
        } else {
            let common_index = offset_value & 0x7FFFFFFF;
            match common_index {
                0 => "AABB",
                5 => "AnimationClip",
                19 => "AnimationCurve",
                34 => "AnimationState",
                49 => "Array",
                55 => "Base",
                60 => "BitField",
                69 => "bitset",
                76 => "bool",
                81 => "char",
                86 => "ColorRGBA",
                96 => "Component",
                106 => "data",
                111 => "deque",
                117 => "double",
                124 => "dynamic_array",
                138 => "FastPropertyName",
                155 => "first",
                161 => "float",
                167 => "Font",
                172 => "GameObject",
                183 => "Generic Mono",
                196 => "GradientNEW",
                208 => "GUID",
                213 => "GUIStyle",
                222 => "int",
                226 => "list",
                231 => "long long",
                241 => "map",
                245 => "Matrix4x4f",
                256 => "MdFour",
                263 => "MonoBehaviour",
                277 => "MonoScript",
                288 => "m_ByteSize",
                299 => "m_Curve",
                307 => "m_EditorClassIdentifier",
                331 => "m_EditorHideFlags",
                349 => "m_Enabled",
                359 => "m_ExtensionPtr",
                374 => "m_GameObject",
                387 => "m_Index",
                395 => "m_IsArray",
                405 => "m_IsStatic",
                416 => "m_MetaFlag",
                427 => "m_Name",
                434 => "m_ObjectHideFlags",
                452 => "m_PrefabInternal",
                469 => "m_PrefabParentObject",
                490 => "m_Script",
                499 => "m_StaticEditorFlags",
                519 => "m_Type",
                526 => "m_Version",
                536 => "Object",
                543 => "pair",
                548 => "PPtr<Component>",
                564 => "PPtr<GameObject>",
                581 => "PPtr<Material>",
                596 => "PPtr<MonoBehaviour>",
                616 => "PPtr<MonoScript>",
                633 => "PPtr<Object>",
                646 => "PPtr<Prefab>",
                659 => "PPtr<Sprite>",
                672 => "PPtr<TextAsset>",
                688 => "PPtr<Texture>",
                702 => "PPtr<Texture2D>",
                718 => "PPtr<Transform>",
                734 => "Prefab",
                741 => "Quaternionf",
                753 => "Rectf",
                759 => "RectInt",
                767 => "RectOffset",
                778 => "second",
                785 => "set",
                789 => "short",
                795 => "size",
                800 => "SInt16",
                807 => "SInt32",
                814 => "SInt64",
                821 => "SInt8",
                827 => "staticvector",
                840 => "string",
                847 => "TextAsset",
                857 => "TextMesh",
                866 => "Texture",
                874 => "Texture2D",
                884 => "Transform",
                894 => "TypelessData",
                907 => "UInt16",
                914 => "UInt32",
                921 => "UInt64",
                928 => "UInt8",
                934 => "unsigned int",
                947 => "unsigned long long",
                966 => "unsigned short",
                981 => "vector",
                988 => "Vector2f",
                997 => "Vector3f",
                1006 => "Vector4f",
                1015 => "m_ScriptingClassIdentifier",
                1042 => "Gradient",
                1051 => "Type*",
                1057 => "int2_storage",
                1070 => "int3_storage",
                1083 => "BoundsInt",
                1093 => "m_CorrespondingSourceObject",
                1121 => "m_PrefabInstance",
                1138 => "m_PrefabAsset",
                1152 => "FileSize",
                1161 => "Hash128",
                _ => return format!("CommonString_{}", common_index),
            }
            .to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn blob_reader_resolves_assetstudio_common_string_offsets() {
        assert_eq!(
            BlobTypeTreeReader::resolve_string(&[], 0x80000000 | 222),
            "int"
        );
        assert_eq!(
            BlobTypeTreeReader::resolve_string(&[], 0x80000000 | 427),
            "m_Name"
        );
        assert_eq!(
            BlobTypeTreeReader::resolve_string(&[], 0x80000000 | 633),
            "PPtr<Object>"
        );
    }

    #[test]
    fn legacy_reader_handles_v3_missing_index_and_meta_flag() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"int\0m_Test\0");
        bytes.extend_from_slice(&4i32.to_le_bytes()); // byteSize
        bytes.extend_from_slice(&0i32.to_le_bytes()); // typeFlags
        bytes.extend_from_slice(&1i32.to_le_bytes()); // version
        bytes.extend_from_slice(&0i32.to_le_bytes()); // childrenCount

        let mut cursor = Cursor::new(bytes);
        let tree =
            LegacyRecursiveTypeTreeReader::read(&mut cursor, format_version::UNKNOWN_3, true)
                .expect("legacy v3 TypeTree should parse");

        assert_eq!(tree.nodes.len(), 1);
        assert_eq!(tree.nodes[0].type_name, "int");
        assert_eq!(tree.nodes[0].name, "m_Test");
        assert_eq!(tree.nodes[0].index, 0);
        assert_eq!(tree.nodes[0].meta_flag, 0);
    }
}
