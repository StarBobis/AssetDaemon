use crate::common::asset_map::asset_map_types::BundleRelationEntry;
use crate::common::unity_dependency::external_resolver::UnityExternalResolver;
use std::collections::HashMap;

pub struct BundlePathMatchCache {
    pub exact_bundle_path_by_normalized_path: HashMap<String, String>,
    pub bundle_path_by_file_name: HashMap<String, String>,
    pub normalized_bundle_paths: Vec<(String, String)>,
}

pub struct BundlePathMatcher;

impl BundlePathMatcher {
    pub fn build_cache(bundle_paths: &[String]) -> BundlePathMatchCache {
        let mut exact_bundle_path_by_normalized_path: HashMap<String, String> =
            HashMap::with_capacity(bundle_paths.len());
        let mut bundle_path_by_file_name: HashMap<String, String> =
            HashMap::with_capacity(bundle_paths.len());
        let mut normalized_bundle_paths: Vec<(String, String)> =
            Vec::with_capacity(bundle_paths.len());

        for bundle_path in bundle_paths {
            let normalized_path = Self::normalize_path(bundle_path);
            exact_bundle_path_by_normalized_path
                .entry(normalized_path.clone())
                .or_insert_with(|| bundle_path.clone());

            if let Some(file_name) = Self::lower_file_name(bundle_path) {
                bundle_path_by_file_name
                    .entry(file_name)
                    .or_insert_with(|| bundle_path.clone());
            }

            normalized_bundle_paths.push((normalized_path, bundle_path.clone()));
        }

        BundlePathMatchCache {
            exact_bundle_path_by_normalized_path,
            bundle_path_by_file_name,
            normalized_bundle_paths,
        }
    }

    pub fn match_path(
        path_name: &str,
        exact_bundle_path_by_normalized_path: &HashMap<String, String>,
        bundle_path_by_file_name: &HashMap<String, String>,
        normalized_bundle_paths: &[(String, String)],
    ) -> Option<String> {
        UnityExternalResolver::match_path(
            path_name,
            exact_bundle_path_by_normalized_path,
            bundle_path_by_file_name,
            normalized_bundle_paths,
        )
    }

    pub fn match_assetstudio_file_name(
        path_name: &str,
        bundle_path_by_file_name: &HashMap<String, String>,
    ) -> Option<String> {
        UnityExternalResolver::match_assetstudio_file_name(path_name, bundle_path_by_file_name)
    }

    #[allow(dead_code)]
    pub fn external_name_candidates(path_name: &str) -> Vec<String> {
        UnityExternalResolver::external_name_candidates(path_name)
    }

    #[allow(dead_code)]
    pub fn archive_cab_name(path_name: &str) -> Option<String> {
        UnityExternalResolver::archive_cab_name(path_name)
    }

    pub fn relation_target_bundle_path(
        current_bundle_path: &str,
        relation: &BundleRelationEntry,
        target_bundle_path_by_file_id: &HashMap<i32, String>,
        exact_bundle_path_by_normalized_path: &HashMap<String, String>,
        bundle_path_by_file_name: &HashMap<String, String>,
        normalized_bundle_paths: &[(String, String)],
    ) -> String {
        let resolved = if !relation.target_bundle_path.is_empty() {
            relation.target_bundle_path.clone()
        } else if relation.file_id == 0 && relation.target_path_id != 0 {
            current_bundle_path.to_string()
        } else {
            target_bundle_path_by_file_id
                .get(&relation.file_id)
                .cloned()
                .or_else(|| {
                    Self::match_path(
                        relation.target_name.as_str(),
                        exact_bundle_path_by_normalized_path,
                        bundle_path_by_file_name,
                        normalized_bundle_paths,
                    )
                })
                .unwrap_or_default()
        };

        if resolved == current_bundle_path {
            String::new()
        } else {
            resolved
        }
    }

    pub fn normalize_path(path_name: &str) -> String {
        UnityExternalResolver::normalize_path(path_name)
    }

    pub fn lower_file_name(path_name: &str) -> Option<String> {
        UnityExternalResolver::lower_file_name(path_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_external_candidates_include_assetstudio_file_name() {
        let candidates = BundlePathMatcher::external_name_candidates(
            "archive:/CAB-33809b1b7ad04d2934348dcdd16e19e0/CAB-33809b1b7ad04d2934348dcdd16e19e0",
        );

        assert!(candidates.contains(&"cab-33809b1b7ad04d2934348dcdd16e19e0".to_string()));
        assert!(candidates.contains(&"cab-33809b1b7ad04d2934348dcdd16e19e0.ress".to_string()));
    }

    #[test]
    fn external_candidates_accept_assetstudio_file_name_hint() {
        let candidates = BundlePathMatcher::external_name_candidates(
            "archive:/cab-parent/cab-parent|cab-9b19aaab47b044427b948ab46df5762f",
        );

        assert!(candidates.contains(&"cab-9b19aaab47b044427b948ab46df5762f".to_string()));
    }

    #[test]
    fn assetstudio_file_name_match_uses_hint_before_path_name() {
        let mut by_file_name = HashMap::new();
        by_file_name.insert("cab-parent".to_string(), "parent-bundle".to_string());
        by_file_name.insert("cab-child".to_string(), "child-bundle".to_string());

        assert_eq!(
            BundlePathMatcher::match_assetstudio_file_name(
                "archive:/cab-parent/cab-parent|cab-child",
                &by_file_name
            ),
            Some("child-bundle".to_string())
        );
    }
}
