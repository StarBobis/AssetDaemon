use crate::common::asset_map::asset_index::AssetDatabase;
use std::path::Path;

pub struct AssetMapRepository;

impl AssetMapRepository {
    pub fn exists(workspace: &Path, cache_root: Option<&Path>) -> bool {
        AssetDatabase::exists_with_cache_root(workspace, cache_root)
    }

    pub fn open(workspace: &Path, cache_root: Option<&Path>) -> Result<AssetDatabase, String> {
        AssetDatabase::open_with_cache_root(workspace, cache_root)
    }

    pub fn open_first_existing(
        workspace_dirs: &[String],
        cache_root: Option<&Path>,
    ) -> Result<Option<AssetDatabase>, String> {
        for dir_str in workspace_dirs {
            let workspace = Path::new(dir_str);
            if Self::exists(workspace, cache_root) {
                return Self::open(workspace, cache_root).map(Some);
            }
        }
        Ok(None)
    }
}
