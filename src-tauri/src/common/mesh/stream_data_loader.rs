/*
 * stream_data_loader.rs -- .resS stream data loader utility
 *
 * Extracted from mesh_service.rs, handles all data loading logic related to .resS resource files.
 * Includes: stream vertex data loading, StreamingInfo extraction, cache lookup, clean path resolution.
 *
 * Follows Soul.md convention: struct + impl organization, no free functions.
 */

use crate::common::bundle_file::asset_bundle::AssetBundle;
use crate::common::bundle_file::ress_list_cache::ResSListCache;
use crate::common::scan::scan_types::ProgressPayload;
use crate::unity::type_tree::unity_value::UnityValue;
use std::path::Path;

const MAX_PREVIEW_STREAM_BYTES: usize = 64 * 1024 * 1024;

/**
 * .resS stream data loader.
 *
 * Responsible for reading preview .resS stream ranges from the workspace ResSList cache.
 * Handles stream paths corrupted by cursor offset, extracts StreamingInfo from TypeTree.
 */
pub struct StreamDataLoader;

impl StreamDataLoader {
    // ============================================================
    // .resS Stream Data Loading
    // ============================================================

    /// Clean stream path corrupted by cursor offset, extract the valid part
    pub fn clean_stream_path(path: &str) -> String {
        // First extract the valid starting point: archive:/ or .resS
        let trimmed = if let Some(pos) = path.find("archive:/") {
            &path[pos..]
        } else {
            let lower = path.to_lowercase();
            if let Some(pos) = lower.find(".ress") {
                let start = path[..pos]
                    .rfind(|c: char| c == '/' || c == '\\')
                    .map(|p| p + 1)
                    .unwrap_or(0);
                &path[start..]
            } else {
                path
            }
        };
        // Truncate at ".resS", removing all trailing garbage characters (\0, corrupted data, etc.)
        let lower = trimmed.to_lowercase();
        if let Some(pos) = lower.find(".ress") {
            trimmed[..pos + 5].to_string()
        } else {
            trimmed.to_string()
        }
    }

    /// Load raw stream data bytes from .resS (for passing to TypeTree's parse_vertex_data_from_stream)
    pub fn load_raw_stream_bytes(
        sd: &UnityValue,
        _bundle: &AssetBundle,
        bundle_path: &Path,
        _cache_dir: Option<&Path>,
    ) -> Vec<u8> {
        let (res_path, offset, size) = Self::extract_stream_info(sd);
        if res_path.is_empty() || size == 0 {
            return vec![];
        }
        if size > MAX_PREVIEW_STREAM_BYTES {
            return vec![];
        }
        ResSListCache::read_range_from_bundle_path(bundle_path, &res_path, offset as usize, size)
            .unwrap_or_default()
    }

    /// Extract streaming info from the TypeTree-parsed m_StreamData value.
    /// Try multiple possible field name combinations (because TypeTree parsing may have variations).
    pub fn extract_stream_info(sd: &UnityValue) -> (String, u64, usize) {
        let obj = match sd {
            UnityValue::Object(o) => o,
            _ => return (String::new(), 0, 0),
        };
        let get_str = |key: &str| obj.get(key).and_then(|v| v.as_str()).map(|s| s.to_string());
        let get_i64 = |key: &str| obj.get(key).and_then(|v| v.as_i64());
        // Try common field name combinations
        let res_path = get_str("path")
            .or_else(|| get_str("m_Path"))
            .or_else(|| get_str("m_Source"))
            .unwrap_or_default();
        let offset = get_i64("offset")
            .or_else(|| get_i64("m_Offset"))
            .unwrap_or(0) as u64;
        let size = get_i64("size").or_else(|| get_i64("m_Size")).unwrap_or(0) as usize;
        (res_path, offset, size)
    }

    /// Extract correct StreamingInfo from TypeTree (when the binary parser gives incorrect values)
    pub fn extract_streaming_info_from_typetree(
        handle: Option<&crate::common::bundle_file::asset_bundle::ObjectHandle>,
    ) -> Option<(String, u64, usize)> {
        let handle = handle?;
        let obj = handle.read().ok()?;
        let props = match &obj {
            crate::unity::type_tree::unity_value::UnityValue::Object(map) => map,
            _ => return None,
        };
        let sd = match props.get("m_StreamData") {
            Some(v) => v,
            None => return None,
        };
        let sd_obj = match sd {
            crate::unity::type_tree::unity_value::UnityValue::Object(o) => o,
            _ => return None,
        };
        let path = sd_obj
            .get("path")
            .or_else(|| sd_obj.get("m_Path"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())?;
        let offset = sd_obj
            .get("offset")
            .or_else(|| sd_obj.get("m_Offset"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as u64;
        let size = sd_obj
            .get("size")
            .or_else(|| sd_obj.get("m_Size"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as usize;
        if path.is_empty() || size == 0 {
            return None;
        }
        Some((path, offset, size))
    }

    /// Try to read vertex floats from TypeTree-parsed m_StreamData
    pub fn try_read_stream_data(
        sd: &UnityValue,
        _bundle: &AssetBundle,
        bundle_path: &Path,
        _cache_dir: Option<&Path>,
    ) -> Vec<f32> {
        // Step 1: Extract path/offset/size from TypeTree-parsed m_StreamData Object
        let (res_path, offset, size) = Self::extract_stream_info(sd);

        if res_path.is_empty() || size == 0 || size > MAX_PREVIEW_STREAM_BYTES {
            return vec![];
        }

        if let Some(raw) = ResSListCache::read_range_from_bundle_path(
            bundle_path,
            &res_path,
            offset as usize,
            size,
        ) {
            if raw.len() > 12 {
                let floats: Vec<f32> = raw
                    .chunks(4)
                    .filter(|c| c.len() == 4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .filter(|f| f.is_finite())
                    .collect();
                if !floats.is_empty() {
                    return floats;
                }
            }
        }
        vec![]
    }

    /// Multi-layer attempt to load .resS stream data
    pub fn load_stream_data(
        stream_path: &str,
        stream_offset: u64,
        stream_size: u32,
        bundle_path: &Path,
        _cache_dir: Option<&Path>,
        _bundle: &AssetBundle,
        progress: &tauri::ipc::Channel<ProgressPayload>,
    ) -> Vec<u8> {
        // Clean path: remove leading garbage characters (caused by parser cursor offset)
        let clean_path = Self::clean_stream_path(stream_path);
        progress
            .send(ProgressPayload {
                step: "diag".into(),
                message: format!("[load_stream_data] raw path='{}'", stream_path),
            })
            .ok();
        progress
            .send(ProgressPayload {
                step: "diag".into(),
                message: format!("[load_stream_data] cleaned path='{}'", clean_path),
            })
            .ok();
        if clean_path.is_empty() {
            progress
                .send(ProgressPayload {
                    step: "diag".into(),
                    message: "[load_stream_data] clean_path is empty, returning empty".into(),
                })
                .ok();
            return vec![];
        }

        let offset = stream_offset as usize;
        let size = stream_size as usize;
        if size == 0 {
            progress
                .send(ProgressPayload {
                    step: "diag".into(),
                    message: "[load_stream_data] size=0, returning empty; preview reads explicit ResSList ranges only".into(),
                })
                .ok();
            return vec![];
        }
        if size > MAX_PREVIEW_STREAM_BYTES {
            progress
                .send(ProgressPayload {
                    step: "streaming".into(),
                    message: format!(
                        "[load_stream_data] refusing to load {}B stream for preview (limit {}B)",
                        size, MAX_PREVIEW_STREAM_BYTES
                    ),
                })
                .ok();
            return vec![];
        }
        progress
            .send(ProgressPayload {
                step: "diag".into(),
                message: format!("[load_stream_data] final offset={}, size={}", offset, size),
            })
            .ok();
        if size == 0 {
            progress
                .send(ProgressPayload {
                    step: "diag".into(),
                    message: "[load_stream_data] size=0, returning empty".into(),
                })
                .ok();
            return vec![];
        }

        // Layer 0: workspace ResSList prepared by Build Map.
        progress
            .send(ProgressPayload {
                step: "diag".into(),
                message: "[load_stream_data] trying Layer 0: workspace ResSList".into(),
            })
            .ok();
        if let Some(data) =
            ResSListCache::read_range_from_bundle_path(bundle_path, &clean_path, offset, size)
        {
            progress
                .send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                        "[load_stream_data] Layer 0 ResSList succeeded: {}B range",
                        data.len()
                    ),
                })
                .ok();
            return data;
        }

        progress
            .send(ProgressPayload {
                step: "diag".into(),
                message: "[load_stream_data] ResSList miss; preview does not fall back to cache/filesystem/bundle node reads".into(),
            })
            .ok();
        vec![]
    }

    /// Diagnostics: print m_StreamData field details from TypeTree to help debug extraction failures
    pub fn diagnose_typetree_streaming(
        handle: Option<&crate::common::bundle_file::asset_bundle::ObjectHandle>,
        progress: &tauri::ipc::Channel<ProgressPayload>,
    ) {
        let handle = match handle {
            Some(h) => h,
            None => {
                progress
                    .send(ProgressPayload {
                        step: "diag".into(),
                        message: "[TT] handle is None".into(),
                    })
                    .ok();
                return;
            }
        };
        let obj = match handle.read() {
            Ok(o) => o,
            Err(e) => {
                progress
                    .send(ProgressPayload {
                        step: "diag".into(),
                        message: format!("[TT] handle.read failed: {:?}", e),
                    })
                    .ok();
                return;
            }
        };
        let props = match &obj {
            crate::unity::type_tree::unity_value::UnityValue::Object(map) => map,
            _ => {
                progress
                    .send(ProgressPayload {
                        step: "diag".into(),
                        message: "[TT] top level is not Object".into(),
                    })
                    .ok();
                return;
            }
        };
        let top_keys: Vec<String> = props.keys().cloned().collect();
        progress
            .send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[TT] Mesh top-level fields({}): {:?}",
                    top_keys.len(),
                    &top_keys[..top_keys.len().min(20)]
                ),
            })
            .ok();

        match props.get("m_StreamData") {
            Some(sd) => match sd {
                crate::unity::type_tree::unity_value::UnityValue::Object(sd_obj) => {
                    let sd_keys: Vec<String> = sd_obj.keys().cloned().collect();
                    progress
                        .send(ProgressPayload {
                            step: "diag".into(),
                            message: format!(
                                "[TT] m_StreamData fields({}): {:?}",
                                sd_keys.len(),
                                sd_keys
                            ),
                        })
                        .ok();
                    for (k, v) in sd_obj.iter() {
                        Self::format_tt_value_for_diag(k, v, 0, progress);
                    }
                }
                _ => {
                    progress
                        .send(ProgressPayload {
                            step: "diag".into(),
                            message: "[TT] m_StreamData is not Object".into(),
                        })
                        .ok();
                }
            },
            None => {
                progress
                    .send(ProgressPayload {
                        step: "diag".into(),
                        message: format!("[TT] no m_StreamData field! All fields: {:?}", top_keys),
                    })
                    .ok();
            }
        }
    }

    /// Recursively format TypeTree value for diagnostics
    fn format_tt_value_for_diag(
        key: &str,
        val: &crate::unity::type_tree::unity_value::UnityValue,
        depth: usize,
        progress: &tauri::ipc::Channel<ProgressPayload>,
    ) {
        let indent = "  ".repeat(depth + 1);
        match val {
            crate::unity::type_tree::unity_value::UnityValue::String(s) => {
                progress
                    .send(ProgressPayload {
                        step: "diag".into(),
                        message: format!("{}{} = String('{}')", indent, key, s),
                    })
                    .ok();
            }
            crate::unity::type_tree::unity_value::UnityValue::Integer(i) => {
                progress
                    .send(ProgressPayload {
                        step: "diag".into(),
                        message: format!("{}{} = Integer({})", indent, key, i),
                    })
                    .ok();
            }
            crate::unity::type_tree::unity_value::UnityValue::Float(f) => {
                progress
                    .send(ProgressPayload {
                        step: "diag".into(),
                        message: format!("{}{} = Float({})", indent, key, f),
                    })
                    .ok();
            }
            crate::unity::type_tree::unity_value::UnityValue::Bytes(b) => {
                progress
                    .send(ProgressPayload {
                        step: "diag".into(),
                        message: format!("{}{} = Bytes[{}]", indent, key, b.len()),
                    })
                    .ok();
            }
            crate::unity::type_tree::unity_value::UnityValue::Array(a) => {
                progress
                    .send(ProgressPayload {
                        step: "diag".into(),
                        message: format!("{}{} = Array[{}]", indent, key, a.len()),
                    })
                    .ok();
                for (i, item) in a.iter().enumerate().take(3) {
                    Self::format_tt_value_for_diag(&format!("[{}]", i), item, depth + 1, progress);
                }
            }
            crate::unity::type_tree::unity_value::UnityValue::Object(o) => {
                progress
                    .send(ProgressPayload {
                        step: "diag".into(),
                        message: format!("{}{} = Object{{}} ({} fields)", indent, key, o.len()),
                    })
                    .ok();
                for (k, v) in o.iter() {
                    Self::format_tt_value_for_diag(k, v, depth + 1, progress);
                }
            }
            _ => {
                progress
                    .send(ProgressPayload {
                        step: "diag".into(),
                        message: format!("{}{} = {:?}", indent, key, val.variant_name()),
                    })
                    .ok();
            }
        }
    }
}
