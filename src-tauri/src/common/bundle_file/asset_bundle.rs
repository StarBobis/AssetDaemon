/*
 * asset_bundle.rs -- Unity AssetBundle loader and aggregate model
 *
 * Transition module: migrates the dependency of the original code on the unity_asset crate to our own implementation.
 * As modules are gradually migrated to fully manual parsing, the adapter code in this file will be removed.
 */

use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;

use crate::common::bundle_file::bundle_file::BundleFile;
use crate::common::bundle_file::res_s::ResS;
use crate::common::serialized_file::object_reader::ObjectReader;
use crate::common::serialized_file::serialized_file;
use crate::unity::classes::registry::{UnityClassObject, UnityClassParser};
use crate::unity::type_tree::type_tree_reader_utils::TypeTreeReaderUtils;
use crate::unity::type_tree::unity_value::UnityValue;
use crate::utils::cursor_reader_utils::CursorReaderUtils;
use crate::utils::unity_object_name_utils::UnityObjectNameUtils;

// ================================================================
// Re-exports (compatible with existing use statements)
// ================================================================
// AssetBundle loader wraps old free function API into struct + impl
// ================================================================

/**
 * AssetBundle loader.
 *
 * Wraps the original free functions (load_bundle, load_bundle_with_logs, get_class_name)
 * as static methods on a struct, following Soul.md convention: struct + impl organization, no free functions.
 */
pub struct AssetBundleLoader;

impl AssetBundleLoader {
    /// Load either a UnityFS Bundle or a loose Unity SerializedFile.
    pub fn load_unity_file(path: &Path) -> Result<AssetBundle, String> {
        if Self::looks_like_loose_serialized_file(path)? {
            return Self::load_serialized_file_with_logs(path, &mut vec![]);
        }
        Self::load_bundle(path)
    }

    /// Load either a UnityFS Bundle or a loose Unity SerializedFile, skipping resource payloads for Bundles.
    pub fn load_unity_file_serialized_only(path: &Path) -> Result<AssetBundle, String> {
        if Self::looks_like_loose_serialized_file(path)? {
            return Self::load_serialized_file_with_logs(path, &mut vec![]);
        }
        Self::load_bundle_serialized_only(path)
    }

    /// Load either a UnityFS Bundle or a loose Unity SerializedFile with diagnostic logs.
    pub fn load_unity_file_with_logs(
        path: &Path,
        logs: &mut Vec<String>,
    ) -> Result<AssetBundle, String> {
        if Self::looks_like_loose_serialized_file(path)? {
            return Self::load_serialized_file_with_logs(path, logs);
        }
        Self::load_bundle_with_logs(path, logs)
    }

    /// Load a Bundle file (compatible with unity_asset::load_bundle)
    pub fn load_bundle(path: &Path) -> Result<AssetBundle, String> {
        let bundle_file = BundleFile::load(path)?;
        Ok(AssetBundle::from(bundle_file))
    }

    /// Load only serialized files, skipping resource payloads such as .resS.
    ///
    /// This is intended for metadata/object lookup and interactive previews where
    /// resource bytes should be loaded later by exact StreamingInfo ranges.
    pub fn load_bundle_serialized_only(path: &Path) -> Result<AssetBundle, String> {
        let bundle_file = BundleFile::load_filtered(path, AssetBundle::is_serialized_file_path)?;
        Ok(AssetBundle::from(bundle_file))
    }

    /// Load a Bundle from an in-memory byte array (avoids double disk reads).
    ///
    /// When building AssetMap, file data is already read into `Vec<u8>`,
    /// parsing directly from here eliminates one `fs::read()` I/O overhead.
    #[allow(dead_code)]
    pub fn load_bundle_from_bytes(data: &[u8]) -> Result<AssetBundle, String> {
        let bundle_file = BundleFile::parse(data)?;
        Ok(AssetBundle::from(bundle_file))
    }

    /// Load a Bundle from bytes while skipping resource payloads such as .resS.
    pub fn load_bundle_serialized_only_from_bytes(data: &[u8]) -> Result<AssetBundle, String> {
        let bundle_file = BundleFile::parse_filtered(data, AssetBundle::is_serialized_file_path)?;
        Ok(AssetBundle::from(bundle_file))
    }

    /// Load a Bundle file and collect step-by-step diagnostic logs
    pub fn load_bundle_with_logs(
        path: &Path,
        logs: &mut Vec<String>,
    ) -> Result<AssetBundle, String> {
        logs.push(format!(
            "load_bundle_with_logs: start loading '{}'",
            path.display()
        ));
        let bundle_file = BundleFile::load_with_logs(path, logs).map_err(|e| {
            logs.push(format!("BundleFile::load failed: {}", e));
            e
        })?;
        logs.push(format!(
            "BundleFile::load success: {} nodes, {} files",
            bundle_file.directory_info.len(),
            bundle_file.file_list.len()
        ));
        Ok(AssetBundle::from_with_logs(bundle_file, logs))
    }

    /// Load a loose Unity SerializedFile, such as a CAB-* file extracted from a UnityFS bundle.
    pub fn load_serialized_file_with_logs(
        path: &Path,
        logs: &mut Vec<String>,
    ) -> Result<AssetBundle, String> {
        logs.push(format!(
            "load_serialized_file_with_logs: start loading '{}'",
            path.display()
        ));
        let data = fs::read(path)
            .map_err(|e| format!("Failed to read file '{}': {}", path.display(), e))?;
        let size = data.len() as u64;
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let first_bytes = data
            .iter()
            .take(16)
            .map(|b| format!("{:02X}", b))
            .collect::<Vec<_>>()
            .join(" ");
        let sf = SerializedFile::parse_with_logs(&data, &name, logs)?;
        logs.push(format!(
            "Loose SerializedFile parse OK: {} objects, unity_version='{}'",
            sf.objects.len(),
            sf.unity_version
        ));
        Ok(AssetBundle::from_loose_serialized_file(
            path,
            data,
            size,
            first_bytes,
            sf,
            logs,
        ))
    }

    /// Get class name (compatible with unity_asset::get_class_name)
    pub fn get_class_name(type_id: i32) -> Option<&'static str> {
        crate::utils::class_name_utils::ClassNameUtils::get_class_name(type_id)
    }

    fn looks_like_loose_serialized_file(path: &Path) -> Result<bool, String> {
        let file_size = fs::metadata(path)
            .map_err(|e| format!("Failed to stat '{}': {}", path.display(), e))?
            .len();
        if file_size < 20 {
            return Ok(false);
        }

        let mut file = fs::File::open(path)
            .map_err(|e| format!("Failed to open '{}': {}", path.display(), e))?;
        let mut header = [0u8; 48];
        let bytes_read = file
            .read(&mut header)
            .map_err(|e| format!("Failed to read file header '{}': {}", path.display(), e))?;
        if bytes_read < 20 {
            return Ok(false);
        }

        let mut serialized_file_size = u32::from_be_bytes(header[4..8].try_into().unwrap()) as u64;
        let version = u32::from_be_bytes(header[8..12].try_into().unwrap());
        let mut data_offset = u32::from_be_bytes(header[12..16].try_into().unwrap()) as u64;

        if !(1..=30).contains(&version) {
            return Ok(false);
        }
        if version >= 22 {
            if bytes_read < 40 {
                return Ok(false);
            }
            serialized_file_size = i64::from_be_bytes(header[24..32].try_into().unwrap()) as u64;
            data_offset = i64::from_be_bytes(header[32..40].try_into().unwrap()) as u64;
        }

        Ok(serialized_file_size == file_size && data_offset <= file_size)
    }
}

// ================================================================
// AssetBundle Aggregate Type
// ================================================================

pub struct AssetBundle {
    pub nodes: Vec<NodeInfo>,
    pub assets: Vec<SerializedFile>,
    pub asset_names: Vec<Option<String>>,
    pub files_diag: Vec<crate::common::bundle_file::bundle_types::FileDiag>,
    /// Full-process step-by-step parsing logs
    #[allow(dead_code)]
    pub parse_logs: Vec<String>,
    /// Resource files (.resS etc.), indexed by file name (lowercase)
    #[allow(dead_code)]
    pub resources: HashMap<String, ResS>,
    inner: BundleFile,
}

pub struct NodeInfo {
    pub name: String,
    pub offset: u64,
    pub size: u64,
}

impl NodeInfo {
    /// Check if the node is a file (not a directory)
    /// All nodes in a Bundle are files, this method always returns true
    pub fn is_file(&self) -> bool {
        true
    }
}

impl AssetBundle {
    pub fn is_serialized_file_path(path: &str) -> bool {
        let lower = path.to_ascii_lowercase();
        !lower.ends_with(".ress") && !lower.ends_with(".resource") && !lower.ends_with(".res")
    }

    fn is_resource_payload_path(path: &str) -> bool {
        let lower = path.to_ascii_lowercase();
        lower.ends_with(".ress") || lower.ends_with(".resource") || lower.ends_with(".res")
    }

    /// Create AssetBundle (no detailed logs)
    fn from(bundle_file: BundleFile) -> Self {
        Self::from_with_logs(bundle_file, &mut vec![])
    }

    /// Create AssetBundle while collecting step-by-step diagnostic logs
    fn from_with_logs(bundle_file: BundleFile, logs: &mut Vec<String>) -> Self {
        logs.push(format!(
            "AssetBundle::from_with_logs: {} nodes, {} file streams",
            bundle_file.directory_info.len(),
            bundle_file.file_list.len()
        ));

        let nodes: Vec<NodeInfo> = bundle_file
            .directory_info
            .iter()
            .map(|n| NodeInfo {
                name: n.path.clone(),
                offset: n.offset as u64,
                size: n.size as u64,
            })
            .collect();

        // Parse each file as SerializedFile (skipping .resS and other resource files)
        let mut assets = Vec::new();
        let mut asset_names = Vec::new();
        let mut resources: HashMap<String, ResS> = HashMap::new();
        let mut files_diag = Vec::new();
        for (file_idx, stream_file) in bundle_file.file_list.iter().enumerate() {
            let first_bytes: String = stream_file
                .data
                .iter()
                .take(16)
                .map(|b| format!("{:02X}", b))
                .collect::<Vec<_>>()
                .join(" ");

            logs.push(format!(
                "  [file {}] name='{}', size={}B, first_bytes=[{}]",
                file_idx,
                stream_file.path,
                stream_file.data.len(),
                first_bytes
            ));

            // Identify and store .resS and other resource files
            // Unity Bundle .resS files always end with the .resS extension
            let is_resource_file = Self::is_resource_payload_path(&stream_file.path);
            if is_resource_file {
                let path_key = stream_file.path.to_lowercase();
                let file_name_key = stream_file.file_name.to_lowercase();
                resources.insert(
                    path_key,
                    ResS::new(stream_file.path.clone(), stream_file.data.clone()),
                );
                resources.insert(
                    file_name_key,
                    ResS::new(stream_file.file_name.clone(), stream_file.data.clone()),
                );
                logs.push(format!(
                    "  [file {}] identified as resource file, added to resource index",
                    file_idx
                ));
                files_diag.push(crate::common::bundle_file::bundle_types::FileDiag {
                    name: stream_file.path.clone(),
                    size_bytes: stream_file.data.len(),
                    first_bytes_hex: first_bytes,
                    parse_ok: true,
                    parse_error: String::new(),
                });
                continue;
            }

            let (ok, err) =
                match SerializedFile::parse_with_logs(&stream_file.data, &stream_file.path, logs) {
                    Ok(sf) => {
                        logs.push(format!(
                            "  [file {}] parse OK: {} objects, unity_version='{}'",
                            file_idx,
                            sf.objects.len(),
                            sf.unity_version
                        ));
                        assets.push(sf);
                        asset_names.push(Some(stream_file.path.clone()));
                        (true, String::new())
                    }
                    Err(e) => {
                        logs.push(format!("  [file {}] parse FAIL: {}", file_idx, e));
                        (false, e)
                    }
                };
            files_diag.push(crate::common::bundle_file::bundle_types::FileDiag {
                name: stream_file.path.clone(),
                size_bytes: stream_file.data.len(),
                first_bytes_hex: first_bytes,
                parse_ok: ok,
                parse_error: err,
            });
        }

        let all_logs = logs.clone();
        AssetBundle {
            nodes,
            assets,
            asset_names,
            resources,
            files_diag,
            parse_logs: all_logs,
            inner: bundle_file,
        }
    }

    fn from_loose_serialized_file(
        path: &Path,
        data: Vec<u8>,
        size: u64,
        first_bytes_hex: String,
        serialized_file: SerializedFile,
        logs: &mut Vec<String>,
    ) -> Self {
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let unity_version = serialized_file.unity_version.clone();
        let object_count = serialized_file.objects.len();
        logs.push(format!(
            "AssetBundle::from_loose_serialized_file: name='{}', size={}B, objects={}",
            name, size, object_count
        ));
        let header = crate::common::bundle_file::bundle_file::BundleHeader {
            signature: "SerializedFile".to_string(),
            version: serialized_file.inner.header.version,
            unity_version,
            unity_revision: String::new(),
            size: size as i64,
            compressed_blocks_info_size: 0,
            uncompressed_blocks_info_size: 0,
            flags: 0,
        };
        let node = crate::common::bundle_file::bundle_file::DirectoryNode {
            offset: 0,
            size: size as i64,
            flags: 0,
            path: name.clone(),
        };
        let stream_file = crate::common::bundle_file::bundle_file::StreamFile {
            path: name.clone(),
            file_name: name.clone(),
            data,
        };
        let bundle_file = BundleFile {
            header,
            blocks_info: Vec::new(),
            directory_info: vec![node],
            file_list: vec![stream_file],
        };
        let resources = Self::load_sibling_resources(path, logs);
        let all_logs = logs.clone();
        AssetBundle {
            nodes: vec![NodeInfo {
                name: name.clone(),
                offset: 0,
                size,
            }],
            assets: vec![serialized_file],
            asset_names: vec![Some(name.clone())],
            resources,
            files_diag: vec![crate::common::bundle_file::bundle_types::FileDiag {
                name,
                size_bytes: size.min(usize::MAX as u64) as usize,
                first_bytes_hex,
                parse_ok: true,
                parse_error: String::new(),
            }],
            parse_logs: all_logs,
            inner: bundle_file,
        }
    }

    fn load_sibling_resources(path: &Path, logs: &mut Vec<String>) -> HashMap<String, ResS> {
        let mut resources = HashMap::new();
        let Some(parent) = path.parent() else {
            return resources;
        };
        let Ok(entries) = fs::read_dir(parent) else {
            return resources;
        };
        for entry in entries.flatten() {
            let resource_path = entry.path();
            if !resource_path.is_file() {
                continue;
            }
            let resource_name = resource_path
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_default();
            if !Self::is_resource_payload_path(&resource_name) {
                continue;
            }
            let Ok(data) = fs::read(&resource_path) else {
                continue;
            };
            logs.push(format!(
                "  sibling resource loaded: '{}' ({}B)",
                resource_path.display(),
                data.len()
            ));
            let resource = ResS::new(resource_name.clone(), data.clone());
            resources.insert(resource_name.to_ascii_lowercase(), resource);
            resources.insert(
                resource_path.to_string_lossy().to_ascii_lowercase(),
                ResS::new(resource_path.to_string_lossy().to_string(), data),
            );
        }
        resources
    }

    pub fn size(&self) -> u64 {
        self.inner.size()
    }

    pub fn is_compressed(&self) -> bool {
        self.inner.is_compressed()
    }

    /// Extract node data
    pub fn extract_node_data(&self, node: &NodeInfo) -> Result<Vec<u8>, String> {
        for stream_file in &self.inner.file_list {
            if stream_file.path == node.name || stream_file.file_name == node.name {
                return Ok(stream_file.data.clone());
            }
        }
        Err(format!("Node '{}' not found", node.name))
    }

    /// Parse an object by path_id into a strongly typed Unity class object.
    pub fn parse_object_by_path_id(&self, path_id: i64) -> Result<UnityClassObject, String> {
        for sf in &self.assets {
            if let Some(obj) = sf.objects.iter().find(|obj| obj.path_id == path_id) {
                return UnityClassParser::parse_object(&sf.inner, &obj.inner, &self.resources);
            }
        }
        Err(format!("Object with path_id={} not found", path_id))
    }
}

// ================================================================
// SerializedFile Aggregate Type
// ================================================================

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SerializedFile {
    pub name: String,
    pub unity_version: String,
    pub objects: Vec<ObjectInfo>,
    pub inner: serialized_file::SerializedFile,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ObjectInfo {
    pub path_id: i64,
    pub type_id: i32,
    pub class_id: i32,
    pub byte_size: u32,
    pub inner: serialized_file::ObjectInfo,
}

impl SerializedFile {
    #[allow(dead_code)]
    pub fn parse(data: &[u8], name: &str) -> Result<Self, String> {
        Self::parse_with_logs(data, name, &mut vec![])
    }

    pub fn parse_with_logs(
        data: &[u8],
        name: &str,
        logs: &mut Vec<String>,
    ) -> Result<Self, String> {
        let inner = serialized_file::SerializedFile::parse_with_logs(data, logs)?;
        let unity_version = inner.unity_version.clone();
        let objects: Vec<ObjectInfo> = inner
            .m_objects
            .iter()
            .map(|o| ObjectInfo {
                path_id: o.path_id,
                type_id: o.type_id,
                class_id: o.class_id as i32,
                byte_size: o.byte_size,
                inner: o.clone(),
            })
            .collect();
        logs.push(format!(
            "  asset_bundle::SerializedFile: {} objects, unity_version='{}', name='{}'",
            objects.len(),
            unity_version,
            name
        ));
        Ok(SerializedFile {
            name: name.to_string(),
            unity_version,
            objects,
            inner,
        })
    }

    /// Get raw bytes of an object
    pub fn object_bytes(&self, obj: &ObjectInfo) -> Result<&[u8], String> {
        self.inner.object_bytes(&obj.inner)
    }

    /// Parse AssetBundle container to get pathasset_path, file_id, path_id) mapping
    /// PPtr pathID size is determined by header_version (<14: 32-bit, >=14: 64-bit)
    pub fn assetbundle_container_raw(
        &self,
        asset_bundle_obj: &ObjectInfo,
    ) -> Result<Vec<(String, i64, i64)>, String> {
        let obj_data = self.inner.object_bytes(&asset_bundle_obj.inner)?;
        let mut reader = Cursor::new(obj_data);
        let header_ver = self.inner.header.version;

        // AssetBundle object: first read m_Name (aligned string)
        let _name = CursorReaderUtils::read_aligned_string(&mut reader)?;

        // m_PreloadTable: Array of PPtr<Object>
        let preload_count = CursorReaderUtils::read_i32(&mut reader)?;
        for _ in 0..preload_count {
            let _file_id = CursorReaderUtils::read_i32(&mut reader)?;
            // PPtr pathID: version < 14 32-bit, >= 14 64-bit
            let _ = if header_ver < 14 {
                CursorReaderUtils::read_i32(&mut reader)? as i64
            } else {
                CursorReaderUtils::read_i64(&mut reader)?
            };
        }

        // m_Container: Array of (string, AssetInfo)
        let container_count = CursorReaderUtils::read_i32(&mut reader)?;
        let mut entries = Vec::with_capacity(container_count as usize);
        for _ in 0..container_count {
            let key = CursorReaderUtils::read_aligned_string(&mut reader)?;
            // AssetInfo: preloadIndex(i32) + preloadSize(i32) + asset(PPtr<Object>)
            let _preload_index = CursorReaderUtils::read_i32(&mut reader)?;
            let _preload_size = CursorReaderUtils::read_i32(&mut reader)?;
            let file_id = CursorReaderUtils::read_i32(&mut reader)? as i64;
            // PPtr pathID: version < 14 32-bit, >= 14 64-bit
            let path_id = if header_ver < 14 {
                CursorReaderUtils::read_i32(&mut reader)? as i64
            } else {
                CursorReaderUtils::read_i64(&mut reader)?
            };
            entries.push((key, file_id, path_id));
        }
        Ok(entries)
    }
}

// ================================================================
// ObjectHandle Aggregate Type
// ================================================================

pub struct ObjectHandle {
    serialized_file: serialized_file::SerializedFile,
    object_info: serialized_file::ObjectInfo,
}

impl ObjectHandle {
    /// Create ObjectHandle
    pub fn new(sf: &SerializedFile, obj: &ObjectInfo) -> Self {
        ObjectHandle {
            serialized_file: sf.inner.clone(),
            object_info: obj.inner.clone(),
        }
    }

    /// Read the object's TypeTree data (returns parsed HashMap)
    pub fn read(&self) -> Result<UnityValue, String> {
        let raw = self.serialized_file.object_bytes(&self.object_info)?;

        // Try to find the corresponding TypeTree
        let type_tree = self
            .serialized_file
            .object_serialized_type(&self.object_info)
            .map(|st| &st.type_tree);

        if let Some(tt) = type_tree {
            if !tt.nodes.is_empty() {
                let mut reader = ObjectReader::new(raw, &self.serialized_file);
                return TypeTreeReaderUtils::read_typetree_value(
                    &mut reader,
                    tt,
                    &self.serialized_file,
                );
            }
        }

        // No TypeTree: return raw bytes
        Ok(UnityValue::Bytes(raw.to_vec()))
    }

    /// Get raw bytes of the object
    pub fn raw_data(&self) -> Result<Vec<u8>, String> {
        self.serialized_file
            .object_bytes(&self.object_info)
            .map(|d| d.to_vec())
    }

    /// Read the object name (m_Name field)
    pub fn peek_name(&self) -> Result<Option<String>, String> {
        UnityObjectNameUtils::peek_object_name(&self.serialized_file, &self.object_info)
    }
}

// ================================================================
// UnityValue and TypeTreeReaderUtils live in unity::type_tree.
// ================================================================
