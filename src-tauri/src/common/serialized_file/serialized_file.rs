/*
 * serialized_file.rs - Unity SerializedFile Parser
 *
 * Fully mimics AssetStudio's SerializedFile.cs implementation.
 * Parses Unity serialized file header, TypeTree, and object information.
 *
 * Also includes ObjectReader (modeled after AssetStudio ObjectReader)
 * for reading object data.
 */

use std::borrow::Cow;
use std::io::{Cursor, Read, Seek, SeekFrom};

use crate::common::serialized_file::reader_utils::SerializedFileReaderUtils;
use crate::common::serialized_file::type_tree_readers::TypeTreeReaders;

// ================================================================
// Version number constants (corresponding values from AssetStudio SerializedFileFormatVersion)
// ================================================================

#[allow(dead_code)]
pub(crate) mod format_version {
    pub const UNKNOWN_2: u32 = 2;
    pub const UNKNOWN_3: u32 = 3;
    pub const UNKNOWN_5: u32 = 5;
    pub const UNKNOWN_6: u32 = 6;
    pub const UNKNOWN_7: u32 = 7;
    pub const UNKNOWN_8: u32 = 8;
    pub const UNKNOWN_9: u32 = 9;
    pub const UNKNOWN_10: u32 = 10;
    pub const HAS_SCRIPT_TYPE_INDEX: u32 = 11;
    pub const UNKNOWN_12: u32 = 12;
    pub const HAS_TYPE_TREE_HASHES: u32 = 13;
    pub const UNKNOWN_14: u32 = 14;
    pub const SUPPORTS_STRIPPED_OBJECT: u32 = 15;
    pub const REFACTORED_CLASS_ID: u32 = 16;
    pub const REFACTOR_TYPE_DATA: u32 = 17;
    pub const UNKNOWN_18: u32 = 18;
    pub const TYPE_TREE_NODE_WITH_TYPE_FLAGS: u32 = 19;
    pub const SUPPORTS_REF_OBJECT: u32 = 20;
    pub const STORES_TYPE_DEPENDENCIES: u32 = 21;
    pub const LARGE_FILES_SUPPORT: u32 = 22;
}

// ================================================================
// SerializedFileHeader
// ================================================================

// Add allow dead_code to all serialized file structs since they
// store full Unity format data for completeness/debugging

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SerializedFileHeader {
    pub metadata_size: u32,
    pub file_size: u64,
    pub version: u32,
    pub data_offset: u64,
    pub endianess: u8,
}

// ================================================================
// TypeTreeNode
// ================================================================

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TypeTreeNode {
    pub level: u8,
    pub type_name: String,
    pub name: String,
    pub byte_size: i32,
    pub index: i32,
    pub type_flags: i32,
    pub version: i32,
    pub meta_flag: i32,
    /// Offset of type name in the string buffer within the TypeTree blob
    pub(crate) type_str_offset: u32,
    /// Offset of field name in the string buffer within the TypeTree blob
    pub(crate) name_str_offset: u32,
}

// ================================================================
// TypeTree
// ================================================================

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TypeTree {
    pub nodes: Vec<TypeTreeNode>,
    pub string_buffer: Vec<u8>,
}

// ================================================================
// SerializedType
// ================================================================

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SerializedType {
    pub class_id: i32,
    pub is_stripped_type: bool,
    pub script_type_index: i16,
    pub script_id: Vec<u8>,
    pub old_type_hash: Vec<u8>,
    pub type_tree: TypeTree,
    pub klass_name: String,
    pub name_space: String,
    pub asm_name: String,
    pub type_dependencies: Vec<i32>,
}

// ================================================================
// ObjectInfo
// ================================================================

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ObjectInfo {
    pub path_id: i64,
    pub byte_start: u64,
    pub byte_size: u32,
    pub type_id: i32,
    pub class_id: u16,
    pub serialized_type: Option<SerializedType>,
    pub serialized_type_index: Option<usize>,
    pub script_type_index: Option<i16>,
    pub is_destroyed: u16,
    pub stripped: u8,
}

// ================================================================
// FileIdentifier
// ================================================================

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FileIdentifier {
    pub guid: [u8; 16],
    pub file_type: i32,
    pub path_name: String,
    pub file_name: String,
}

// ================================================================
// SerializedFile
// ================================================================

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SerializedFile {
    pub header: SerializedFileHeader,
    pub unity_version: String,
    pub version: [i32; 4], // [major, minor, build, type]
    pub m_types: Vec<SerializedType>,
    pub big_id_enabled: i32,
    pub m_objects: Vec<ObjectInfo>,
    pub m_script_types: Vec<LocalSerializedObjectIdentifier>,
    pub m_externals: Vec<FileIdentifier>,
    pub m_ref_types: Vec<SerializedType>,
    pub user_information: String,
    /// Target platform (used for Object/EditorExtension conditional field determination)
    pub target_platform: i32,
    /// Raw data (for subsequent reading of object bytes)
    raw_data: Vec<u8>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct LocalSerializedObjectIdentifier {
    pub local_serialized_file_index: i32,
    pub local_identifier_in_file: i64,
}

impl SerializedFile {
    /// Parse SerializedFile from byte array
    #[allow(dead_code)]
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        // Simple entry without logs, delegates to logged version
        Self::parse_with_logs(data, &mut vec![])
    }

    /// Parse SerializedFile from owned bytes without copying the raw object data.
    ///
    /// BuildMap already owns each decompressed stream file as a `Vec<u8>`. Taking ownership here
    /// avoids cloning large SerializedFile payloads just so later object readers can slice them.
    pub fn parse_owned(data: Vec<u8>) -> Result<Self, String> {
        Self::parse_owned_with_logs(data, &mut vec![])
    }

    /// Parse SerializedFile from byte array while collecting step-by-step diagnostic logs
    pub fn parse_with_logs(data: &[u8], logs: &mut Vec<String>) -> Result<Self, String> {
        Self::parse_cow_with_logs(Cow::Borrowed(data), logs)
    }

    pub fn parse_owned_with_logs(data: Vec<u8>, logs: &mut Vec<String>) -> Result<Self, String> {
        Self::parse_cow_with_logs(Cow::Owned(data), logs)
    }

    fn parse_cow_with_logs(data: Cow<'_, [u8]>, logs: &mut Vec<String>) -> Result<Self, String> {
        let total_len = data.len();
        logs.push(format!(
            "SerializedFile::parse_with_logs: starting parse, total bytes={}",
            total_len
        ));

        let mut reader = Cursor::new(data.as_ref());

        // ============================================================
        // Read Header -- UnityFS internal data uses Big-Endian
        // ============================================================
        let metadata_size = SerializedFileReaderUtils::read_u32(&mut reader)?;
        let file_size = SerializedFileReaderUtils::read_u32(&mut reader)? as u64;
        let version = SerializedFileReaderUtils::read_u32(&mut reader)?;
        let data_offset = SerializedFileReaderUtils::read_u32(&mut reader)? as u64;
        logs.push(format!(
            "  Header: metadata_size={}, file_size={}, version={}, data_offset={}",
            metadata_size, file_size, version, data_offset
        ));

        // Validate version: reasonable range 1~30, values outside this indicate not a valid SerializedFile
        if version < 1 || version > 30 {
            let err = format!(
                "SerializedFile version={} is outside reasonable range (1~30), may not be a valid SerializedFile",
                version
            );
            logs.push(format!("  {}", err));
            return Err(err);
        }

        let mut header = SerializedFileHeader {
            metadata_size,
            file_size,
            version,
            data_offset,
            endianess: 0,
        };

        let _file_endianess: u8;
        // Endianness flag controls metadata/object tables. The extended v22 header that
        // follows this flag is still part of the big-endian file header, matching AssetStudio.
        #[allow(unused_assignments)]
        let mut is_le = false;
        if version >= 9 {
            header.endianess = SerializedFileReaderUtils::read_u8(&mut reader)?;
            let _reserved = SerializedFileReaderUtils::read_bytes(&mut reader, 3)?;
            _file_endianess = header.endianess;
            logs.push(format!(
                "  Endianess: version>=9, endian={}",
                header.endianess
            ));
            is_le = header.endianess == 0;
        } else {
            let seek_pos = file_size - metadata_size as u64;
            logs.push(format!(
                "  version<9, jumping to metadata position: offset={}",
                seek_pos
            ));
            reader
                .seek(SeekFrom::Start(seek_pos))
                .map_err(|e| format!("Seek error: {}", e))?;
            _file_endianess = SerializedFileReaderUtils::read_u8(&mut reader)?;
            header.endianess = _file_endianess;
            is_le = _file_endianess == 0;
        }

        // Large files support -- only present in version >= 22 (Unity 2020.1+)
        if version >= format_version::LARGE_FILES_SUPPORT {
            let pos_before = (reader.position() as usize).min(total_len);
            let remaining = total_len.saturating_sub(pos_before);
            let dump_len = remaining.min(64);
            let dump_bytes: Vec<String> = data.as_ref()[pos_before..pos_before + dump_len]
                .iter()
                .map(|b| format!("{:02X}", b))
                .collect();
            logs.push(format!(
                "  v>=22: reading Large file header (offset {}, hex: [{}])",
                pos_before,
                dump_bytes.join(" ")
            ));

            header.metadata_size = SerializedFileReaderUtils::read_u32_be(&mut reader)?;
            header.file_size = SerializedFileReaderUtils::read_i64_be(&mut reader)? as u64;
            header.data_offset = SerializedFileReaderUtils::read_i64_be(&mut reader)? as u64;
            let _unknown = SerializedFileReaderUtils::read_i64_be(&mut reader)?;
            logs.push(format!(
                "  Large file: metadata_size={}, file_size={}, data_offset={}",
                header.metadata_size, header.file_size, header.data_offset
            ));
        }

        // Validate after the v22+ extended header has replaced the initial 32-bit fields.
        if header.file_size != total_len as u64 {
            let err = format!(
                "SerializedFile file_size={} does not match actual file size {}, may not be a valid SerializedFile",
                header.file_size, total_len
            );
            logs.push(format!("  {}", err));
            return Err(err);
        }

        if header.data_offset > total_len as u64 {
            let err = format!(
                "SerializedFile data_offset={} exceeds file size {}, may not be a valid SerializedFile",
                header.data_offset, total_len
            );
            logs.push(format!("  {}", err));
            return Err(err);
        }

        // Create endianness-aware function pointers (for all subsequent multi-byte reads)
        let ri32: fn(&mut dyn Read) -> Result<i32, String> = if is_le {
            SerializedFileReaderUtils::read_i32_le
        } else {
            SerializedFileReaderUtils::read_i32_be
        };
        let ru32: fn(&mut dyn Read) -> Result<u32, String> = if is_le {
            SerializedFileReaderUtils::read_u32_le
        } else {
            SerializedFileReaderUtils::read_u32_be
        };
        let ri64: fn(&mut dyn Read) -> Result<i64, String> = if is_le {
            SerializedFileReaderUtils::read_i64_le
        } else {
            SerializedFileReaderUtils::read_i64_be
        };
        let _ru64: fn(&mut dyn Read) -> Result<u64, String> = if is_le {
            SerializedFileReaderUtils::read_u64_le
        } else {
            SerializedFileReaderUtils::read_u64_be
        };
        let ru16: fn(&mut dyn Read) -> Result<u16, String> = if is_le {
            SerializedFileReaderUtils::read_u16_le
        } else {
            SerializedFileReaderUtils::read_u16_be
        };
        let ri16: fn(&mut dyn Read) -> Result<i16, String> = if is_le {
            SerializedFileReaderUtils::read_i16_le
        } else {
            SerializedFileReaderUtils::read_i16_be
        };

        // ============================================================
        // Read Metadata
        // ============================================================
        // Note: Endianness is always little-endian for our purposes
        // (Unity on most platforms uses LE)

        let mut unity_version = String::from("2.5.0f5");
        if version >= 7 {
            unity_version = SerializedFileReaderUtils::read_string_to_null(&mut reader)?;
        }
        logs.push(format!("  Unity version: '{}'", unity_version));

        let version_arr = SerializedFileReaderUtils::parse_version(&unity_version);
        logs.push(format!("  Version array: [{:?}]", version_arr));

        // Target platform (version >= 8)
        let mut target_platform = -2i32; // default NoTarget
        if version >= 8 {
            target_platform = ri32(&mut reader)?;
            logs.push(format!("  Target platform: {}", target_platform));
        }

        // Enable TypeTree (version >= 13)
        let mut enable_type_tree = true;
        if version >= format_version::HAS_TYPE_TREE_HASHES {
            enable_type_tree = SerializedFileReaderUtils::read_bool(&mut reader)?;
            logs.push(format!("  Enable TypeTree: {}", enable_type_tree));
        }

        // ============================================================
        // Read Types
        // ============================================================
        let type_count = ri32(&mut reader)?;
        logs.push(format!("  Type count: {}", type_count));
        let safe_type_cap = (type_count.max(0) as usize).min(100_000);
        let mut m_types = Vec::with_capacity(safe_type_cap);
        for i in 0..type_count {
            match Self::read_serialized_type(&mut reader, version, enable_type_tree, false, is_le) {
                Ok(st) => m_types.push(st),
                Err(e) => {
                    logs.push(format!("  Type[{}] parse failed: {}", i, e));
                    return Err(format!("Failed to parse Type[{}]: {}", i, e));
                }
            }
        }
        logs.push(format!(
            "  Types parsing completed: {} items",
            m_types.len()
        ));

        // Big ID (version >= 7 && version < 14)
        let big_id_enabled = if version >= 7 && version < 14 {
            let bid = ri32(&mut reader)?;
            logs.push(format!("  Big ID enabled: {}", bid));
            bid
        } else {
            0
        };

        // ============================================================
        // Read Objects
        // ============================================================
        let object_count = ri32(&mut reader)?;
        logs.push(format!("  Object count: {}", object_count));
        let safe_obj_cap = (object_count.max(0) as usize).min(1_000_000);
        let mut m_objects = Vec::with_capacity(safe_obj_cap);

        for _obj_idx in 0..object_count {
            let mut object_info = ObjectInfo {
                path_id: 0,
                byte_start: 0,
                byte_size: 0,
                type_id: 0,
                class_id: 0,
                serialized_type: None,
                serialized_type_index: None,
                script_type_index: None,
                is_destroyed: 0,
                stripped: 0,
            };

            // Path ID
            if big_id_enabled != 0 {
                object_info.path_id = ri64(&mut reader)?;
            } else if version < 14 {
                object_info.path_id = ri32(&mut reader)? as i64;
            } else {
                SerializedFileReaderUtils::align_stream(&mut reader, 4)?;
                object_info.path_id = ri64(&mut reader)?;
            }

            // Byte start
            if version >= format_version::LARGE_FILES_SUPPORT {
                object_info.byte_start = ri64(&mut reader)? as u64;
            } else {
                object_info.byte_start = ru32(&mut reader)? as u64;
            }
            object_info.byte_start += header.data_offset;

            object_info.byte_size = ru32(&mut reader)?;
            object_info.type_id = ri32(&mut reader)?;

            // Class ID
            if version < format_version::REFACTORED_CLASS_ID {
                object_info.class_id = ru16(&mut reader)?;
                object_info.serialized_type_index = m_types
                    .iter()
                    .position(|t| t.class_id == object_info.type_id);
            } else {
                let type_idx = object_info.type_id as usize;
                if type_idx < m_types.len() {
                    object_info.class_id = m_types[type_idx].class_id as u16;
                    object_info.serialized_type_index = Some(type_idx);
                }
            }

            // Is destroyed
            if version < 11 {
                // HasScriptTypeIndex
                object_info.is_destroyed = ru16(&mut reader)?;
            }

            // Script type index (version 11..13) -- corresponds to AssetStudio HasScriptTypeIndex
            if version >= format_version::HAS_SCRIPT_TYPE_INDEX
                && version < format_version::REFACTOR_TYPE_DATA
            {
                let script_type_index = ri16(&mut reader)?;
                object_info.script_type_index = Some(script_type_index);
                if let Some(type_idx) = object_info.serialized_type_index {
                    if let Some(serialized_type) = m_types.get_mut(type_idx) {
                        serialized_type.script_type_index = script_type_index;
                    }
                }
            }

            if version == format_version::SUPPORTS_STRIPPED_OBJECT
                || version == format_version::REFACTORED_CLASS_ID
            {
                object_info.stripped = SerializedFileReaderUtils::read_u8(&mut reader)?;
            }

            m_objects.push(object_info);
        }
        logs.push(format!(
            "  Objects reading completed: {} items",
            m_objects.len()
        ));

        // ============================================================
        // Read Script Types (version >= 11)
        // ============================================================
        let mut m_script_types = Vec::new();
        if version >= 11 {
            let script_count = ri32(&mut reader)?;
            logs.push(format!("  Script types count: {}", script_count));
            for _ in 0..script_count {
                let local_file_idx = ri32(&mut reader)?;
                let local_id = if version < 14 {
                    ri32(&mut reader)? as i64
                } else {
                    SerializedFileReaderUtils::align_stream(&mut reader, 4)?;
                    ri64(&mut reader)?
                };
                m_script_types.push(LocalSerializedObjectIdentifier {
                    local_serialized_file_index: local_file_idx,
                    local_identifier_in_file: local_id,
                });
            }
        }

        // ============================================================
        // Read Externals
        // ============================================================
        let externals_count = ri32(&mut reader)?;
        // Safety check: prevent corrupted data from causing capacity overflow or infinite loop
        if externals_count < 0 || externals_count > 10_000 {
            return Err(format!(
                "Abnormal external reference count ({}), file may be corrupted",
                externals_count
            ));
        }
        let mut m_externals = Vec::with_capacity(externals_count as usize);
        for _ in 0..externals_count {
            if version >= 6 {
                let _temp_empty = SerializedFileReaderUtils::read_string_to_null(&mut reader)?;
            }
            let mut file_type = 0;
            let guid = if version >= 5 {
                let bytes = SerializedFileReaderUtils::read_bytes(&mut reader, 16)?;
                let mut arr = [0u8; 16];
                arr.copy_from_slice(&bytes);
                file_type = ri32(&mut reader)?;
                arr
            } else {
                [0u8; 16]
            };
            let path_name = SerializedFileReaderUtils::read_string_to_null(&mut reader)?;
            let file_name = path_name
                .replace('\\', "/")
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string();

            m_externals.push(FileIdentifier {
                guid,
                file_type,
                path_name,
                file_name,
            });
        }
        logs.push(format!(
            "  Externals reading completed: {} items",
            m_externals.len()
        ));

        // ============================================================
        // Read Ref Types (version >= 19?)
        // ============================================================
        let mut m_ref_types = Vec::new();
        if version >= format_version::SUPPORTS_REF_OBJECT {
            let ref_types_count = ri32(&mut reader)?;
            logs.push(format!("  Ref types count: {}", ref_types_count));
            for i in 0..ref_types_count {
                match Self::read_serialized_type(
                    &mut reader,
                    version,
                    enable_type_tree,
                    true,
                    is_le,
                ) {
                    Ok(st) => m_ref_types.push(st),
                    Err(e) => {
                        logs.push(format!("  Ref type[{}] parse failed: {}", i, e));
                        return Err(format!("Failed to parse Ref type[{}]: {}", i, e));
                    }
                }
            }
            logs.push(format!(
                "  Ref types parsing completed: {} items",
                m_ref_types.len()
            ));
        }

        // ============================================================
        // User Information (version >= 5)
        // ============================================================
        let user_information = if version >= 5 {
            match SerializedFileReaderUtils::read_string_to_null(&mut reader) {
                Ok(s) => {
                    logs.push(format!(
                        "  User information: '{}' ({} bytes)",
                        if s.len() > 80 {
                            format!("{}...", &s[..80])
                        } else {
                            s.clone()
                        },
                        s.len()
                    ));
                    s
                }
                Err(e) => {
                    logs.push(format!("  User information read failed: {}", e));
                    return Err(e);
                }
            }
        } else {
            String::new()
        };

        logs.push(format!(
            "SerializedFile parsing completed: {} objects, unity_version='{}'",
            m_objects.len(),
            unity_version
        ));
        drop(reader);

        Ok(SerializedFile {
            header,
            unity_version,
            version: version_arr,
            m_types,
            big_id_enabled,
            m_objects,
            m_script_types,
            m_externals,
            m_ref_types,
            user_information,
            target_platform,
            raw_data: data.into_owned(),
        })
    }

    #[allow(dead_code)]
    fn read_serialized_type<R: Read + Seek>(
        reader: &mut R,
        header_version: u32,
        enable_type_tree: bool,
        is_ref_type: bool,
        is_le: bool,
    ) -> Result<SerializedType, String> {
        let class_id = if is_le {
            SerializedFileReaderUtils::read_i32_le(reader)?
        } else {
            SerializedFileReaderUtils::read_i32_be(reader)?
        };

        let is_stripped_type = if header_version >= format_version::REFACTORED_CLASS_ID {
            SerializedFileReaderUtils::read_bool(reader)?
        } else {
            false
        };

        let script_type_index = if header_version >= format_version::REFACTOR_TYPE_DATA {
            if is_le {
                SerializedFileReaderUtils::read_i16_le(reader)?
            } else {
                SerializedFileReaderUtils::read_i16_be(reader)?
            }
        } else {
            -1
        };

        let mut script_id = Vec::new();
        let mut old_type_hash = Vec::new();

        if header_version >= format_version::HAS_TYPE_TREE_HASHES {
            // HasTypeTreeHashes (version >= 13)
            if is_ref_type && script_type_index >= 0 {
                script_id = SerializedFileReaderUtils::read_bytes(reader, 16)?;
            } else if (header_version < format_version::REFACTORED_CLASS_ID && class_id < 0)
                || (header_version >= format_version::REFACTORED_CLASS_ID && class_id == 114)
            {
                script_id = SerializedFileReaderUtils::read_bytes(reader, 16)?;
            }
            old_type_hash = SerializedFileReaderUtils::read_bytes(reader, 16)?;
        }

        let mut type_tree = TypeTree {
            nodes: Vec::new(),
            string_buffer: Vec::new(),
        };

        let mut klass_name = String::new();
        let mut name_space = String::new();
        let mut asm_name = String::new();
        let mut type_dependencies = Vec::new();

        if enable_type_tree {
            type_tree = TypeTreeReaders::read(reader, header_version, is_le)?;
            if header_version >= 21 {
                if is_ref_type {
                    klass_name = SerializedFileReaderUtils::read_string_to_null(reader)?;
                    name_space = SerializedFileReaderUtils::read_string_to_null(reader)?;
                    asm_name = SerializedFileReaderUtils::read_string_to_null(reader)?;
                } else {
                    type_dependencies = SerializedFileReaderUtils::read_i32_array(reader, is_le)?;
                }
            }
        }

        Ok(SerializedType {
            class_id,
            is_stripped_type,
            script_type_index,
            script_id,
            old_type_hash,
            type_tree,
            klass_name,
            name_space,
            asm_name,
            type_dependencies,
        })
    }

    /// Get raw byte data for an object
    pub fn object_bytes(&self, obj: &ObjectInfo) -> Result<&[u8], String> {
        let start = obj.byte_start as usize;
        let end = start
            .checked_add(obj.byte_size as usize)
            .ok_or_else(|| {
                format!(
                    "Object byte range overflow (path_id={}, start={}, size={})",
                    obj.path_id, start, obj.byte_size
                )
            })?;
        if end > self.raw_data.len() {
            return Err(format!(
                "Object bytes out of bounds (path_id={}, start={}, size={}, data_len={})",
                obj.path_id,
                start,
                obj.byte_size,
                self.raw_data.len()
            ));
        }
        Ok(&self.raw_data[start..end])
    }

    pub fn object_serialized_type<'a>(&'a self, obj: &'a ObjectInfo) -> Option<&'a SerializedType> {
        obj.serialized_type_index
            .and_then(|idx| self.m_types.get(idx))
            .or(obj.serialized_type.as_ref())
    }
}

// ================================================================
// Backward compatibility re-exports
// ObjectReader and SerializedFileReaderUtils live in this module folder.
// ================================================================

#[cfg(test)]
mod tests {
    use super::*;

    struct SerializedFileFixture {
        bytes: Vec<u8>,
        data_offset: u32,
    }

    impl SerializedFileFixture {
        fn new(version: u32, unity_version: &str) -> Self {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&0u32.to_be_bytes()); // metadata size placeholder
            bytes.extend_from_slice(&0u32.to_be_bytes()); // file size placeholder
            bytes.extend_from_slice(&version.to_be_bytes());
            bytes.extend_from_slice(&0u32.to_be_bytes()); // data offset placeholder
            bytes.push(0); // little endian metadata/data fields
            bytes.extend_from_slice(&[0, 0, 0]);
            bytes.extend_from_slice(unity_version.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(&5i32.to_le_bytes()); // StandaloneWindows
            bytes.push(0); // disable TypeTree
            Self {
                bytes,
                data_offset: 0,
            }
        }

        fn push_type_v15(mut self, class_id: i32, old_hash: [u8; 16]) -> Self {
            self.bytes.extend_from_slice(&1i32.to_le_bytes());
            self.bytes.extend_from_slice(&class_id.to_le_bytes());
            self.bytes.extend_from_slice(&old_hash);
            self
        }

        fn push_type_v16(mut self, class_id: i32, old_hash: [u8; 16]) -> Self {
            self.bytes.extend_from_slice(&1i32.to_le_bytes());
            self.bytes.extend_from_slice(&class_id.to_le_bytes());
            self.bytes.push(1); // m_IsStrippedType
            self.bytes.extend_from_slice(&old_hash);
            self
        }

        fn push_type_v17(mut self, class_id: i32, old_hash: [u8; 16]) -> Self {
            self.bytes.extend_from_slice(&1i32.to_le_bytes());
            self.bytes.extend_from_slice(&class_id.to_le_bytes());
            self.bytes.push(1); // m_IsStrippedType
            self.bytes.extend_from_slice(&(-1i16).to_le_bytes()); // m_ScriptTypeIndex
            self.bytes.extend_from_slice(&old_hash);
            self
        }

        fn push_object_v15(mut self) -> Self {
            self.bytes.extend_from_slice(&1i32.to_le_bytes());
            self.align4();
            self.bytes.extend_from_slice(&123i64.to_le_bytes()); // pathID
            self.bytes.extend_from_slice(&0u32.to_le_bytes()); // byteStart relative to dataOffset
            self.bytes.extend_from_slice(&4u32.to_le_bytes()); // byteSize
            self.bytes.extend_from_slice(&28i32.to_le_bytes()); // typeID
            self.bytes.extend_from_slice(&28u16.to_le_bytes()); // classID
            self.bytes.extend_from_slice(&7i16.to_le_bytes()); // object scriptTypeIndex
            self.bytes.push(1); // stripped
            self
        }

        fn push_object_v16(mut self) -> Self {
            self.bytes.extend_from_slice(&1i32.to_le_bytes());
            self.align4();
            self.bytes.extend_from_slice(&456i64.to_le_bytes()); // pathID
            self.bytes.extend_from_slice(&0u32.to_le_bytes()); // byteStart relative to dataOffset
            self.bytes.extend_from_slice(&4u32.to_le_bytes()); // byteSize
            self.bytes.extend_from_slice(&0i32.to_le_bytes()); // typeID indexes m_Types
            self.bytes.extend_from_slice(&9i16.to_le_bytes()); // object scriptTypeIndex
            self.bytes.push(1); // stripped
            self
        }

        fn push_object_v22(mut self) -> Self {
            self.bytes.extend_from_slice(&1i32.to_le_bytes());
            self.align4();
            self.bytes.extend_from_slice(&456i64.to_le_bytes()); // pathID
            self.bytes.extend_from_slice(&0i64.to_le_bytes()); // byteStart relative to dataOffset
            self.bytes.extend_from_slice(&4u32.to_le_bytes()); // byteSize
            self.bytes.extend_from_slice(&0i32.to_le_bytes()); // typeID indexes m_Types
            self
        }

        fn finish(mut self) -> Vec<u8> {
            self.bytes.extend_from_slice(&0i32.to_le_bytes()); // script types
            self.bytes.extend_from_slice(&0i32.to_le_bytes()); // externals
            self.bytes.push(0); // user information

            self.finish_with_current_metadata()
        }

        fn finish_large_header(mut self) -> Vec<u8> {
            self.bytes.extend_from_slice(&0i32.to_le_bytes()); // script types
            self.bytes.extend_from_slice(&0i32.to_le_bytes()); // externals
            self.bytes.extend_from_slice(&0i32.to_le_bytes()); // ref types
            self.bytes.push(0); // user information

            self.finish_with_current_metadata_large_header()
        }

        fn finish_with_external(mut self, path_name: &str) -> Vec<u8> {
            self.bytes.extend_from_slice(&0i32.to_le_bytes()); // script types
            self.bytes.extend_from_slice(&1i32.to_le_bytes()); // externals
            self.bytes.push(0); // temp empty string for version >= 6
            self.bytes.extend_from_slice(&[0x11; 16]); // guid
            self.bytes.extend_from_slice(&0i32.to_le_bytes()); // type
            self.bytes.extend_from_slice(path_name.as_bytes());
            self.bytes.push(0);
            self.bytes.push(0); // user information

            self.finish_with_current_metadata()
        }

        fn finish_with_current_metadata(mut self) -> Vec<u8> {
            self.data_offset = self.bytes.len() as u32;
            self.bytes.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

            let metadata_size = self.data_offset - 20;
            let file_size = self.bytes.len() as u32;
            self.bytes[0..4].copy_from_slice(&metadata_size.to_be_bytes());
            self.bytes[4..8].copy_from_slice(&file_size.to_be_bytes());
            self.bytes[12..16].copy_from_slice(&self.data_offset.to_be_bytes());
            self.bytes
        }

        fn finish_with_current_metadata_large_header(mut self) -> Vec<u8> {
            self.bytes.splice(20..20, [0u8; 28]);
            self.data_offset = self.bytes.len() as u32;
            self.bytes.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

            let metadata_size = (self.data_offset - 48) as u32;
            let file_size = self.bytes.len() as u64;
            self.bytes[0..4].copy_from_slice(&0u32.to_be_bytes());
            self.bytes[4..8].copy_from_slice(&0u32.to_be_bytes());
            self.bytes[12..16].copy_from_slice(&0u32.to_be_bytes());
            self.bytes[20..24].copy_from_slice(&metadata_size.to_be_bytes());
            self.bytes[24..32].copy_from_slice(&file_size.to_be_bytes());
            self.bytes[32..40].copy_from_slice(&(self.data_offset as u64).to_be_bytes());
            self.bytes[40..48].copy_from_slice(&0u64.to_be_bytes());
            self.bytes
        }

        fn align4(&mut self) {
            while self.bytes.len() % 4 != 0 {
                self.bytes.push(0);
            }
        }
    }

    #[test]
    fn parses_v15_object_table_with_class_id_script_type_index_and_stripped_flag() {
        let bytes = SerializedFileFixture::new(15, "5.4.0f3")
            .push_type_v15(28, [0xA5; 16])
            .push_object_v15()
            .finish();

        let sf = SerializedFile::parse(&bytes).expect("v15 SerializedFile should parse");
        let obj = sf.m_objects.first().expect("object should be present");

        assert_eq!(sf.m_types[0].is_stripped_type, false);
        assert_eq!(sf.m_types[0].script_type_index, 7);
        assert_eq!(obj.path_id, 123);
        assert_eq!(obj.byte_start, sf.header.data_offset);
        assert_eq!(obj.byte_size, 4);
        assert_eq!(obj.type_id, 28);
        assert_eq!(obj.class_id, 28);
        assert_eq!(obj.script_type_index, Some(7));
        assert_eq!(obj.stripped, 1);
        assert_eq!(sf.object_bytes(obj).unwrap(), &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn parses_v16_type_and_object_without_refactor_type_data_field() {
        let bytes = SerializedFileFixture::new(16, "5.5.0f3")
            .push_type_v16(28, [0x5A; 16])
            .push_object_v16()
            .finish();

        let sf = SerializedFile::parse(&bytes).expect("v16 SerializedFile should parse");
        let obj = sf.m_objects.first().expect("object should be present");

        assert_eq!(sf.m_types[0].is_stripped_type, true);
        assert_eq!(sf.m_types[0].script_type_index, 9);
        assert_eq!(obj.path_id, 456);
        assert_eq!(obj.byte_start, sf.header.data_offset);
        assert_eq!(obj.byte_size, 4);
        assert_eq!(obj.type_id, 0);
        assert_eq!(obj.class_id, 28);
        assert_eq!(obj.script_type_index, Some(9));
        assert_eq!(obj.stripped, 1);
        assert_eq!(sf.object_bytes(obj).unwrap(), &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn external_file_name_uses_assetstudio_slash_semantics() {
        let bytes = SerializedFileFixture::new(15, "5.4.0f3")
            .push_type_v15(28, [0xA5; 16])
            .push_object_v15()
            .finish_with_external(
                "archive:/cab-9b19aaab47b044427b948ab46df5762f/cab-9b19aaab47b044427b948ab46df5762f",
            );

        let sf = SerializedFile::parse(&bytes).expect("SerializedFile should parse");
        let external = sf.m_externals.first().expect("external should be present");

        assert_eq!(external.file_name, "cab-9b19aaab47b044427b948ab46df5762f");
    }

    #[test]
    fn parses_v22_large_header_as_big_endian_before_little_endian_metadata() {
        let bytes = SerializedFileFixture::new(22, "2022.3.62f2")
            .push_type_v17(28, [0x5A; 16])
            .push_object_v22()
            .finish_large_header();

        let sf = SerializedFile::parse(&bytes).expect("v22 SerializedFile should parse");
        let obj = sf.m_objects.first().expect("object should be present");

        assert_eq!(sf.header.metadata_size, (sf.header.data_offset - 48) as u32);
        assert_eq!(sf.header.file_size, bytes.len() as u64);
        assert_eq!(sf.header.endianess, 0);
        assert_eq!(sf.unity_version, "2022.3.62f2");
        assert_eq!(obj.path_id, 456);
        assert_eq!(obj.byte_start, sf.header.data_offset);
        assert_eq!(obj.class_id, 28);
        assert_eq!(sf.object_bytes(obj).unwrap(), &[0xDE, 0xAD, 0xBE, 0xEF]);
    }
}
