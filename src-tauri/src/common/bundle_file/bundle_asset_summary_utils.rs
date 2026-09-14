use std::collections::HashMap;

use crate::common::bundle_file::asset_bundle::{AssetBundle, AssetBundleLoader};
use crate::common::bundle_file::bundle_types::AssetSummary;
use crate::utils::unity_object_name_utils::UnityObjectNameUtils;

pub struct BundleAssetSummaryUtils;

impl BundleAssetSummaryUtils {
    /// Extract AssetSummary list for all objects from AssetBundle serialized files.
    pub fn build_asset_summaries(bundle: &AssetBundle) -> Vec<AssetSummary> {
        let mut assets: Vec<AssetSummary> = Vec::new();
        for serialized_file in &bundle.assets {
            let container_entries = serialized_file
                .objects
                .iter()
                .find(|obj| obj.class_id == 142)
                .and_then(|asset_bundle_obj| {
                    serialized_file
                        .assetbundle_container_raw(asset_bundle_obj)
                        .ok()
                })
                .unwrap_or_default();

            let mut path_id_to_asset_path: HashMap<i64, String> = HashMap::new();
            for (asset_path, _file_id, path_id) in &container_entries {
                path_id_to_asset_path.insert(*path_id, asset_path.clone());
            }

            for obj in &serialized_file.objects {
                let class_name = AssetBundleLoader::get_class_name(obj.class_id)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| String::from("Unknown"));
                let asset_path = path_id_to_asset_path
                    .get(&obj.path_id)
                    .cloned()
                    .unwrap_or_else(|| format!("#{:x}", obj.path_id));
                let obj_name =
                    UnityObjectNameUtils::display_name(&serialized_file.inner, &obj.inner)
                        .ok()
                        .flatten()
                        .unwrap_or_default();

                assets.push(AssetSummary {
                    path: asset_path,
                    name: obj_name,
                    class_name,
                    class_id: obj.class_id,
                    path_id: obj.path_id,
                    byte_size: obj.byte_size,
                });
            }
        }
        assets
    }
}
