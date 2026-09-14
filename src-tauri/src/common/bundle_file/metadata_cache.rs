use crate::common::asset_map::asset_map_service::AssetMapService;
use crate::common::bundle_file::bundle_types::BundleMeta;
use std::path::Path;

pub struct BundleMetadataCache;

impl BundleMetadataCache {
    pub fn try_load_with_workspace(
        bundle_path: &Path,
        workspace: &Path,
        cache_root: Option<&Path>,
    ) -> Option<BundleMeta> {
        AssetMapService::try_load_cached_meta_with_workspace(bundle_path, workspace, cache_root)
    }
}
