use std::collections::HashMap;

use crate::common::asset_map::asset_index::{AssetDatabase, RelationRow};
use crate::common::serialized_file::serialized_file::FileIdentifier;
use crate::unity::classes::object::PPtr;

/// 统一跨 bundle 引用解析工具。
///
/// 提供两个核心方法：
/// - `resolve_pptr()` — 基于 PPtr + SerializedFile externals 表解析
/// - `resolve_relation_row()` — 基于预存的 RelationRow 解析
///
/// 所有散落在各模块的重复实现应当统一调用此处。
pub struct UnityExternalResolver;

impl UnityExternalResolver {
    /// 统一解析 PPtr 引用的目标 bundle 路径。
    ///
    /// - `file_id == 0` → 同一文件，直接返回 `source_bundle_path`
    /// - `file_id > 0`  → 查 `m_externals[file_id - 1]`，先用 `file_name`（非空时），
    ///   再用 `path_name`，最后调用 `resolve_from_db()` 做模糊匹配
    pub fn resolve_pptr(
        db: &AssetDatabase,
        source_bundle_path: &str,
        externals: &[FileIdentifier],
        pptr: &PPtr,
    ) -> Option<String> {
        if pptr.is_null() {
            return None;
        }
        if pptr.file_id == 0 {
            return Some(source_bundle_path.to_string());
        }
        let external = externals.get(pptr.file_id.checked_sub(1)? as usize)?;
        let name = if external.file_name.is_empty() {
            external.path_name.as_str()
        } else {
            external.file_name.as_str()
        };
        Self::resolve_from_db(db, name)
    }

    /// 统一解析 RelationRow 目标 bundle 路径。
    ///
    /// 查询顺序：
    /// 1. `target_bundle_path` 已缓存 → 直接返回
    /// 2. `file_id == 0` → 同一文件，返回 `bundle_path`
    /// 3. `file_id > 0`  → `resolve_from_db(relation.target_name)`
    /// 4. 兜底 → 返回 `bundle_path`
    pub fn resolve_relation_row(db: &AssetDatabase, relation: &RelationRow) -> String {
        if !relation.target_bundle_path.is_empty() {
            return relation.target_bundle_path.clone();
        }
        if relation.file_id == 0 {
            return relation.bundle_path.clone();
        }
        Self::resolve_from_db(db, &relation.target_name)
            .unwrap_or_else(|| relation.bundle_path.clone())
    }

    pub fn match_path(
        external_name: &str,
        exact_bundle_path_by_normalized_path: &HashMap<String, String>,
        bundle_path_by_file_name: &HashMap<String, String>,
        normalized_bundle_paths: &[(String, String)],
    ) -> Option<String> {
        if external_name.is_empty() {
            return None;
        }

        if let Some(file_name) = Self::assetstudio_file_name_hint(external_name) {
            if let Some(bundle_path) = bundle_path_by_file_name.get(&file_name) {
                return Some(bundle_path.clone());
            }
        }

        let candidates = Self::external_name_candidates(external_name);
        for candidate in &candidates {
            if let Some(bundle_path) = exact_bundle_path_by_normalized_path.get(candidate) {
                return Some(bundle_path.clone());
            }
        }

        for candidate in &candidates {
            if let Some(file_name) = Self::lower_file_name(candidate) {
                if let Some(bundle_path) = bundle_path_by_file_name.get(&file_name) {
                    return Some(bundle_path.clone());
                }
            }
        }

        for candidate in &candidates {
            if let Some((_, original_path)) =
                normalized_bundle_paths.iter().find(|(bundle_path, _)| {
                    bundle_path.ends_with(candidate) || bundle_path.contains(candidate)
                })
            {
                return Some(original_path.clone());
            }
        }

        None
    }

    pub fn match_assetstudio_file_name(
        external_name: &str,
        bundle_path_by_file_name: &HashMap<String, String>,
    ) -> Option<String> {
        let file_name = Self::assetstudio_file_name_hint(external_name)?;
        bundle_path_by_file_name.get(&file_name).cloned()
    }

    pub fn resolve_from_db(db: &AssetDatabase, external_name: &str) -> Option<String> {
        if let Some(file_name) = Self::assetstudio_file_name_hint(external_name) {
            if let Some(cab) = Self::cab_name_from_candidate(&file_name) {
                if let Ok(Some(bundle_path)) = db.find_bundle_by_internal_name(cab) {
                    return Some(bundle_path);
                }
            }

            if let Ok(bundle_infos) = db.get_bundle_infos() {
                if let Some(info) = bundle_infos.iter().find(|info| {
                    Self::lower_file_name(&info.path)
                        .map(|bundle_file_name| bundle_file_name == file_name)
                        .unwrap_or(false)
                }) {
                    return Some(info.path.clone());
                }
            }
        }

        if let Some(cab) = Self::archive_cab_name(external_name) {
            if let Ok(Some(bundle_path)) = db.find_bundle_by_internal_name(&cab) {
                return Some(bundle_path);
            }
        }

        let bundle_infos = db.get_bundle_infos().ok()?;
        let candidates = Self::external_name_candidates(external_name);

        for candidate in &candidates {
            if let Some(info) = bundle_infos
                .iter()
                .find(|info| Self::normalize_path(&info.path) == *candidate)
            {
                return Some(info.path.clone());
            }
        }

        for candidate in &candidates {
            if let Some(info) = bundle_infos.iter().find(|info| {
                let bp = Self::normalize_path(&info.path);
                bp.ends_with(candidate) || bp.contains(candidate)
            }) {
                return Some(info.path.clone());
            }
        }

        for candidate in &candidates {
            if let Some(info) = bundle_infos.iter().find(|info| {
                Self::lower_file_name(&info.path)
                    .map(|file_name| file_name == *candidate)
                    .unwrap_or(false)
            }) {
                return Some(info.path.clone());
            }
        }

        None
    }

    pub fn external_name_candidates(external_name: &str) -> Vec<String> {
        let mut candidates = Vec::new();
        for part in external_name.split('|') {
            let normalized = Self::normalize_path(part);
            if normalized.is_empty() {
                continue;
            }

            Self::push_candidate(&mut candidates, normalized.clone());
            if let Some(file_name) = Self::lower_file_name(&normalized) {
                Self::push_candidate(&mut candidates, file_name);
            }

            if normalized.starts_with("archive:/") {
                for part in normalized
                    .split('/')
                    .filter(|part| !part.is_empty() && *part != "archive:")
                {
                    Self::push_candidate(&mut candidates, part.to_string());
                    Self::push_candidate(&mut candidates, format!("{}.ress", part));
                }
            }
        }

        candidates
    }

    pub fn archive_cab_name(external_name: &str) -> Option<String> {
        if let Some(file_name) = Self::assetstudio_file_name_hint(external_name) {
            if let Some(cab) = Self::cab_name_from_candidate(&file_name) {
                return Some(cab.to_string());
            }
        }

        Self::external_name_candidates(external_name)
            .into_iter()
            .find_map(|candidate| Self::cab_name_from_candidate(&candidate).map(str::to_string))
    }

    pub fn normalize_path(path_name: &str) -> String {
        path_name.replace('\\', "/").to_lowercase()
    }

    pub fn lower_file_name(path_name: &str) -> Option<String> {
        let normalized = path_name.replace('\\', "/");
        normalized
            .rsplit('/')
            .next()
            .filter(|file_name| !file_name.is_empty())
            .map(|file_name| file_name.to_lowercase())
    }

    pub fn assetstudio_file_name_hint(external_name: &str) -> Option<String> {
        for part in external_name.split('|') {
            let normalized = Self::normalize_path(part);
            if normalized.is_empty() || normalized.starts_with("archive:/") {
                continue;
            }
            if let Some(file_name) = Self::lower_file_name(&normalized) {
                return Some(file_name);
            }
        }

        external_name
            .split('|')
            .find_map(|part| Self::lower_file_name(part))
    }

    fn cab_name_from_candidate(candidate: &str) -> Option<&str> {
        if candidate.starts_with("cab-")
            && !candidate.ends_with(".ress")
            && candidate.len() > "cab-".len()
        {
            Some(candidate)
        } else {
            None
        }
    }

    fn push_candidate(candidates: &mut Vec<String>, candidate: String) {
        if !candidates.iter().any(|value| value == &candidate) {
            candidates.push(candidate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_external_candidates_include_assetstudio_file_name() {
        let candidates = UnityExternalResolver::external_name_candidates(
            "archive:/CAB-33809b1b7ad04d2934348dcdd16e19e0/CAB-33809b1b7ad04d2934348dcdd16e19e0",
        );

        assert!(candidates.contains(&"cab-33809b1b7ad04d2934348dcdd16e19e0".to_string()));
        assert!(candidates.contains(&"cab-33809b1b7ad04d2934348dcdd16e19e0.ress".to_string()));
    }

    #[test]
    fn external_candidates_accept_assetstudio_file_name_hint() {
        let candidates = UnityExternalResolver::external_name_candidates(
            "archive:/cab-parent/cab-parent|cab-9b19aaab47b044427b948ab46df5762f",
        );

        assert!(candidates.contains(&"cab-9b19aaab47b044427b948ab46df5762f".to_string()));
    }

    #[test]
    fn archive_cab_name_prefers_assetstudio_file_name_hint() {
        assert_eq!(
            UnityExternalResolver::archive_cab_name("archive:/cab-parent/cab-parent|cab-child"),
            Some("cab-child".to_string())
        );
        assert_eq!(
            UnityExternalResolver::archive_cab_name("cab-child|archive:/cab-parent/cab-parent"),
            Some("cab-child".to_string())
        );
    }

    #[test]
    fn match_path_prefers_assetstudio_file_name_hint_over_archive_parent() {
        let exact = HashMap::new();
        let mut by_file_name = HashMap::new();
        by_file_name.insert("cab-parent".to_string(), "parent-bundle".to_string());
        by_file_name.insert("cab-child".to_string(), "child-bundle".to_string());
        let normalized = Vec::new();

        assert_eq!(
            UnityExternalResolver::match_path(
                "archive:/cab-parent/cab-parent|cab-child",
                &exact,
                &by_file_name,
                &normalized,
            ),
            Some("child-bundle".to_string())
        );
    }
}
