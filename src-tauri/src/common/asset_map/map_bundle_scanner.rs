use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::common::asset_map::asset_map_types::{AssetEntry, BundleEntry, BundleRelationEntry};
use crate::common::asset_map::unity_relation_collector::UnityRelationCollector;
use crate::common::bundle_file::asset_bundle::AssetBundleLoader;
use crate::common::bundle_file::bundle_file::BundleFile;
use crate::common::serialized_file::serialized_file::SerializedFile;
use crate::unity::classes::registry::UnityClassParser;
use crate::utils::unity_object_name_utils::UnityObjectNameUtils;

pub struct MapBundleScanner;

impl MapBundleScanner {
    pub fn scan_from_path(
        bundle_path: &Path,
        file_size: u64,
        md5: String,
    ) -> Result<BundleEntry, String> {
        Self::scan_from_path_with_relations(bundle_path, file_size, md5, false)
    }

    pub fn scan_from_path_with_relations(
        bundle_path: &Path,
        file_size: u64,
        md5: String,
        include_relations: bool,
    ) -> Result<BundleEntry, String> {
        let bundle_file =
            BundleFile::load_filtered(bundle_path, |path| Self::is_serialized_candidate(path))?;
        Self::scan_bundle_file(bundle_path, file_size, md5, bundle_file, include_relations)
    }

    pub fn scan_serialized_file_from_path(
        file_path: &Path,
        file_size: u64,
        md5: String,
    ) -> Result<BundleEntry, String> {
        Self::scan_serialized_file_from_path_with_relations(file_path, file_size, md5, false)
    }

    pub fn scan_serialized_file_from_path_with_relations(
        file_path: &Path,
        file_size: u64,
        md5: String,
        include_relations: bool,
    ) -> Result<BundleEntry, String> {
        let data = std::fs::read(file_path)
            .map_err(|e| format!("Failed to read '{}': {}", file_path.display(), e))?;
        Self::scan_serialized_file_bytes(file_path, file_size, md5, data, include_relations)
    }

    #[allow(dead_code)]
    pub fn scan_from_bytes(
        bundle_path: &Path,
        data: &[u8],
        file_size: u64,
        md5: String,
    ) -> Result<BundleEntry, String> {
        Self::scan_from_bytes_with_relations(bundle_path, data, file_size, md5, false)
    }

    pub fn scan_from_bytes_with_relations(
        bundle_path: &Path,
        data: &[u8],
        file_size: u64,
        md5: String,
        include_relations: bool,
    ) -> Result<BundleEntry, String> {
        let bundle_file =
            BundleFile::parse_filtered(data, |path| Self::is_serialized_candidate(path))?;
        Self::scan_bundle_file(bundle_path, file_size, md5, bundle_file, include_relations)
    }

    pub fn scan_serialized_file_from_bytes(
        file_path: &Path,
        data: Vec<u8>,
        file_size: u64,
        md5: String,
    ) -> Result<BundleEntry, String> {
        Self::scan_serialized_file_from_bytes_with_relations(file_path, data, file_size, md5, false)
    }

    pub fn scan_serialized_file_from_bytes_with_relations(
        file_path: &Path,
        data: Vec<u8>,
        file_size: u64,
        md5: String,
        include_relations: bool,
    ) -> Result<BundleEntry, String> {
        Self::scan_serialized_file_bytes(file_path, file_size, md5, data, include_relations)
    }

    fn scan_serialized_file_bytes(
        file_path: &Path,
        file_size: u64,
        md5: String,
        data: Vec<u8>,
        include_relations: bool,
    ) -> Result<BundleEntry, String> {
        let stream_file_name = file_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut internal_names = Vec::new();
        Self::push_internal_name(&mut internal_names, &stream_file_name, "file_name");
        Self::push_internal_name(&mut internal_names, &stream_file_name, "serialized_file");

        let sf = SerializedFile::parse_owned(data)?;
        let unity_version = sf.unity_version.clone();
        Self::scan_serialized_file(
            file_path,
            file_size,
            md5,
            unity_version,
            stream_file_name,
            sf,
            internal_names,
            include_relations,
        )
    }

    fn scan_bundle_file(
        bundle_path: &Path,
        file_size: u64,
        md5: String,
        bundle_file: BundleFile,
        include_relations: bool,
    ) -> Result<BundleEntry, String> {
        let mut assets: Vec<AssetEntry> = Vec::new();
        let mut containers: HashMap<String, i64> = HashMap::new();
        let mut externals: Vec<(usize, i32, String)> = Vec::new();
        let mut internal_names: Vec<(String, String)> = Vec::new();
        let mut relations: Vec<BundleRelationEntry> = Vec::new();
        let mut unity_version = String::new();

        for node in &bundle_file.directory_info {
            Self::push_internal_name(&mut internal_names, &node.path, "node");
        }

        for (sf_idx, stream_file) in bundle_file.file_list.into_iter().enumerate() {
            let stream_path = stream_file.path;
            let stream_file_name = stream_file.file_name;
            Self::push_internal_name(&mut internal_names, &stream_path, "serialized_file");
            Self::push_internal_name(&mut internal_names, &stream_file_name, "file_name");

            let sf = match SerializedFile::parse_owned(stream_file.data) {
                Ok(sf) => sf,
                Err(_) => continue,
            };

            if unity_version.is_empty() {
                unity_version = sf.unity_version.clone();
            }

            Self::scan_serialized_file_into(
                bundle_path,
                sf_idx,
                &stream_file_name,
                &sf,
                &mut assets,
                &mut containers,
                &mut externals,
                &mut relations,
                include_relations,
            );
        }

        Ok(BundleEntry {
            path: bundle_path.to_string_lossy().to_string(),
            md5,
            file_size,
            unity_version,
            asset_count: assets.len(),
            assets,
            containers,
            externals,
            internal_names,
            relations,
        })
    }

    fn scan_serialized_file(
        file_path: &Path,
        file_size: u64,
        md5: String,
        unity_version: String,
        stream_file_name: String,
        sf: SerializedFile,
        internal_names: Vec<(String, String)>,
        include_relations: bool,
    ) -> Result<BundleEntry, String> {
        let mut assets = Vec::new();
        let mut containers = HashMap::new();
        let mut externals = Vec::new();
        let mut relations = Vec::new();

        Self::scan_serialized_file_into(
            file_path,
            0,
            &stream_file_name,
            &sf,
            &mut assets,
            &mut containers,
            &mut externals,
            &mut relations,
            include_relations,
        );

        Ok(BundleEntry {
            path: file_path.to_string_lossy().to_string(),
            md5,
            file_size,
            unity_version,
            asset_count: assets.len(),
            assets,
            containers,
            externals,
            internal_names,
            relations,
        })
    }

    fn scan_serialized_file_into(
        _bundle_path: &Path,
        sf_idx: usize,
        _stream_file_name: &str,
        sf: &SerializedFile,
        assets: &mut Vec<AssetEntry>,
        containers: &mut HashMap<String, i64>,
        externals: &mut Vec<(usize, i32, String)>,
        relations: &mut Vec<BundleRelationEntry>,
        include_relations: bool,
    ) {
        if let Some(ab_obj) = sf.m_objects.iter().find(|o| o.class_id == 142) {
            if let Ok(asset_bundle) = UnityClassParser::parse_asset_bundle(sf, ab_obj) {
                for entry in asset_bundle.container {
                    let asset_path = entry.asset_path;
                    let path_id = entry.asset.path_id;
                    containers.insert(asset_path.clone(), path_id);
                }
            }
        }

        for (ext_idx, ext) in sf.m_externals.iter().enumerate() {
            if !ext.path_name.is_empty() {
                let target_name = Self::external_match_name(&ext.path_name, &ext.file_name);
                externals.push((sf_idx, ext_idx as i32 + 1, target_name.clone()));
            }
        }

        if include_relations {
            UnityRelationCollector::collect_serialized_file_relations(sf, relations, "");
        }

        let named_path_ids: HashSet<i64> = containers.values().copied().collect();
        for obj in &sf.m_objects {
            let class_id = obj.class_id as i32;
            let class_name: Cow<'static, str> = match AssetBundleLoader::get_class_name(class_id) {
                Some(name) => name.into(),
                None => format!("Unknown({})", obj.class_id).into(),
            };
            let has_container_path = named_path_ids.contains(&obj.path_id);
            let asset_name = if Self::should_read_name(class_id, has_container_path) {
                UnityObjectNameUtils::display_name(sf, obj)
                    .ok()
                    .flatten()
                    .unwrap_or_default()
            } else {
                String::new()
            };

            assets.push(AssetEntry {
                path_id: obj.path_id,
                class_id,
                class_name,
                asset_name,
                byte_size: obj.byte_size,
            });
        }
    }

    fn push_internal_name(rows: &mut Vec<(String, String)>, name: &str, kind: &str) {
        let normalized = name.replace('\\', "/").to_lowercase();
        if normalized.is_empty() {
            return;
        }
        rows.push((normalized.clone(), kind.to_string()));
        if let Some(file_name) = normalized.rsplit('/').next() {
            if !file_name.is_empty() && file_name != normalized {
                rows.push((file_name.to_string(), kind.to_string()));
            }
        }
    }

    fn external_match_name(path_name: &str, file_name: &str) -> String {
        if file_name.is_empty() {
            path_name.to_string()
        } else if path_name.eq_ignore_ascii_case(file_name) {
            path_name.to_string()
        } else {
            format!("{}|{}", path_name, file_name)
        }
    }

    fn is_serialized_candidate(path: &str) -> bool {
        let lower = path.to_ascii_lowercase();
        !lower.ends_with(".ress") && !lower.ends_with(".resource") && !lower.ends_with(".res")
    }

    fn should_read_name(class_id: i32, has_container_path: bool) -> bool {
        has_container_path || matches!(class_id, 28 | 43 | 74 | 95 | 111)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "assetfinder_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    struct SerializedFileFixture {
        bytes: Vec<u8>,
        data_offset: u32,
    }

    impl SerializedFileFixture {
        fn new() -> Self {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&0u32.to_be_bytes()); // metadata size placeholder
            bytes.extend_from_slice(&0u32.to_be_bytes()); // file size placeholder
            bytes.extend_from_slice(&15u32.to_be_bytes()); // version
            bytes.extend_from_slice(&0u32.to_be_bytes()); // data offset placeholder
            bytes.push(0); // little endian
            bytes.extend_from_slice(&[0, 0, 0]);
            bytes.extend_from_slice(b"2019.4.41f2\0");
            bytes.extend_from_slice(&19i32.to_le_bytes()); // target platform
            bytes.push(1); // enable type tree
            bytes.extend_from_slice(&0i32.to_le_bytes()); // type count
            bytes.extend_from_slice(&1i32.to_le_bytes()); // object count
            while bytes.len() % 4 != 0 {
                bytes.push(0);
            }
            bytes.extend_from_slice(&123i64.to_le_bytes()); // path id
            bytes.extend_from_slice(&0u32.to_le_bytes()); // byte start
            bytes.extend_from_slice(&4u32.to_le_bytes()); // byte size
            bytes.extend_from_slice(&28i32.to_le_bytes()); // type id
            bytes.extend_from_slice(&28u16.to_le_bytes()); // class id
            bytes.extend_from_slice(&0i16.to_le_bytes()); // script type index
            bytes.push(0); // stripped
            bytes.extend_from_slice(&0i32.to_le_bytes()); // script types
            bytes.extend_from_slice(&0i32.to_le_bytes()); // externals
            bytes.push(0); // user information

            Self {
                bytes,
                data_offset: 0,
            }
        }

        fn finish(mut self) -> Vec<u8> {
            self.data_offset = self.bytes.len() as u32;
            self.bytes.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
            let metadata_size = self.data_offset - 20;
            let file_size = self.bytes.len() as u32;
            self.bytes[0..4].copy_from_slice(&metadata_size.to_be_bytes());
            self.bytes[4..8].copy_from_slice(&file_size.to_be_bytes());
            self.bytes[12..16].copy_from_slice(&self.data_offset.to_be_bytes());
            self.bytes
        }
    }

    #[test]
    fn scans_loose_serialized_file_like_assetstudio_external_file() {
        let root = temp_path("loose-serialized");
        fs::create_dir_all(&root).unwrap();
        let file_path = root.join("cab-9b19aaab47b044427b948ab46df5762f");
        let bytes = SerializedFileFixture::new().finish();
        fs::write(&file_path, &bytes).unwrap();

        let entry = MapBundleScanner::scan_serialized_file_from_path(
            &file_path,
            bytes.len() as u64,
            "fingerprint".to_string(),
        )
        .expect("loose SerializedFile should scan");

        assert_eq!(entry.assets.len(), 1);
        assert_eq!(entry.assets[0].path_id, 123);
        assert!(entry.internal_names.iter().any(|(name, kind)| {
            name == "cab-9b19aaab47b044427b948ab46df5762f" && kind == "serialized_file"
        }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reads_gameobject_name_only_for_container_assets() {
        assert!(MapBundleScanner::should_read_name(1, true));
        assert!(!MapBundleScanner::should_read_name(1, false));
        assert!(MapBundleScanner::should_read_name(21, true));
        assert!(!MapBundleScanner::should_read_name(21, false));
        assert!(MapBundleScanner::should_read_name(43, false));
        assert!(MapBundleScanner::should_read_name(95, false));
        assert!(MapBundleScanner::should_read_name(111, false));
    }
}
