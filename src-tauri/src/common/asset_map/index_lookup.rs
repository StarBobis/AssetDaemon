use crate::common::asset_map::asset_index::{
    AssetDatabase, AssetRow, BundleInfoRow, ContainerRow, RelationRow,
};
use std::path::Path;

pub struct FileIndexLookup;

impl FileIndexLookup {
    pub fn get_bundle_infos(
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Result<Vec<BundleInfoRow>, String> {
        AssetDatabase::open_with_cache_root(workspace, cache_root)?.get_bundle_infos()
    }

    pub fn find_by_bundle_and_path_id(
        workspace: &Path,
        bundle_path: &str,
        path_id: i64,
        cache_root: Option<&Path>,
    ) -> Result<Option<AssetRow>, String> {
        AssetDatabase::open_with_cache_root(workspace, cache_root)?
            .find_by_bundle_and_path_id(bundle_path, path_id)
    }

    pub fn find_by_bundle(
        workspace: &Path,
        bundle_path: &str,
        cache_root: Option<&Path>,
    ) -> Result<Vec<AssetRow>, String> {
        AssetDatabase::open_with_cache_root(workspace, cache_root)?.find_by_bundle(bundle_path)
    }

    pub fn get_containers(
        workspace: &Path,
        bundle_path: &str,
        cache_root: Option<&Path>,
    ) -> Result<Vec<ContainerRow>, String> {
        AssetDatabase::open_with_cache_root(workspace, cache_root)?.get_containers(bundle_path)
    }

    pub fn find_externals_by_bundle(
        workspace: &Path,
        bundle_path: &str,
        cache_root: Option<&Path>,
    ) -> Result<Vec<(i32, String)>, String> {
        Ok(AssetDatabase::open_with_cache_root(workspace, cache_root)?
            .find_externals_by_bundle(bundle_path)?
            .into_iter()
            .map(|external| (external.file_id, external.path_name))
            .collect())
    }

    pub fn find_bundle_by_internal_name(
        workspace: &Path,
        name: &str,
        cache_root: Option<&Path>,
    ) -> Result<Option<String>, String> {
        AssetDatabase::open_with_cache_root(workspace, cache_root)?
            .find_bundle_by_internal_name(name)
    }

    pub fn find_relations_by_bundle_and_source(
        workspace: &Path,
        relation_type: &str,
        bundle_path: &str,
        source_path_id: i64,
        cache_root: Option<&Path>,
    ) -> Result<Vec<RelationRow>, String> {
        AssetDatabase::open_with_cache_root(workspace, cache_root)?
            .find_relations_by_bundle_and_source(relation_type, bundle_path, source_path_id)
    }

    pub fn find_relations_by_bundle_and_target(
        workspace: &Path,
        relation_type: &str,
        target_bundle_path: &str,
        target_path_id: i64,
        cache_root: Option<&Path>,
    ) -> Result<Vec<RelationRow>, String> {
        let db = AssetDatabase::open_with_cache_root(workspace, cache_root)?;
        let local_rows = db.find_relations_by_bundle_and_target(
            relation_type,
            target_bundle_path,
            target_path_id,
        )?;
        if !local_rows.is_empty() {
            return Ok(local_rows);
        }

        Ok(db
            .find_relations_by_target(target_bundle_path, target_path_id)?
            .into_iter()
            .filter(|row| row.relation_type == relation_type)
            .collect())
    }
}
