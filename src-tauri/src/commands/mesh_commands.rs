/*
 * Mesh commands get Mesh info, extract geometry data.
 *
 * This file only contains #[tauri::command] functions; actual logic delegates to MeshService.
 * Follows Soul.md convention: commands separated from service classes.
 */

use crate::common::asset_map::repository::AssetMapRepository;
use crate::common::bundle_file::asset_bundle::AssetBundleLoader;
use crate::common::bundle_file::asset_bundle::ObjectHandle;
use crate::common::command_types::{
    AnimatorPreviewGlbResult, MeshPreviewDiffuseTextureResult, MeshPreviewTextureCandidate,
    PreviewAnimationClipRef, PreviewAnimatorRef, PreviewSelectionOptions,
};
use crate::common::export::export_animation_utils::ExportAnimationUtils;
use crate::common::mesh::animator_preview_workflow::AnimatorPreviewWorkflow;
use crate::common::mesh::game_object_preview_workflow::GameObjectPreviewWorkflow;
use crate::common::mesh::mesh_preview_workflow::MeshPreviewWorkflow;
use crate::common::mesh::mesh_types::{MeshGeometry, MeshInfo};
use crate::common::mesh::preview_texture_resolver::MeshPreviewTextureResolver;
use crate::common::mesh::workspace_utils::WorkspaceUtils;
use crate::common::scan::scan_types::{IncrementalPreviewPayload, ProgressPayload};
use crate::common::task::task_runner::TaskRunner;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct ClearPreviewCacheResult {
    pub removed_paths: usize,
    pub removed_bytes: u64,
    pub details: Vec<String>,
}

#[derive(Default)]
struct PreviewCacheCleaner {
    scheduled: HashSet<PathBuf>,
    removed_paths: usize,
    removed_bytes: u64,
    details: Vec<String>,
}

impl PreviewCacheCleaner {
    fn clear_all(cache_dir: &Path) -> Result<ClearPreviewCacheResult, String> {
        let mut cleaner = Self::default();
        for root_name in preview_cache_root_names() {
            cleaner.schedule_existing(cache_dir.join(root_name));
        }
        cleaner.remove_scheduled()?;
        Ok(cleaner.into_result())
    }

    fn clear_asset(
        cache_dir: &Path,
        bundle_path: &str,
        path_id: i64,
    ) -> Result<ClearPreviewCacheResult, String> {
        let mut cleaner = Self::default();
        for root_name in preview_cache_root_names() {
            let root = cache_dir.join(root_name);
            cleaner.scan_preview_root(&root, bundle_path, path_id)?;
        }
        cleaner.remove_scheduled()?;
        Ok(cleaner.into_result())
    }

    fn scan_preview_root(
        &mut self,
        root: &Path,
        bundle_path: &str,
        path_id: i64,
    ) -> Result<(), String> {
        if !root.exists() {
            return Ok(());
        }
        let path_id_text = path_id.to_string();
        for entry in fs::read_dir(root).map_err(|e| {
            format!(
                "Failed to read preview cache root {}: {}",
                root.display(),
                e
            )
        })? {
            let entry = entry.map_err(|e| format!("Failed to read preview cache entry: {}", e))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|e| format!("Failed to stat preview cache entry: {}", e))?;
            if file_type.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains(&path_id_text) {
                    self.schedule_existing(path.clone());
                }
                self.scan_preview_metadata(root, &path, bundle_path, path_id)?;
            } else if is_preview_cache_meta(&path)
                && preview_cache_meta_matches(&path, bundle_path, path_id)?
            {
                self.schedule_cache_folder_and_outputs(root, &path);
            }
        }
        Ok(())
    }

    fn scan_preview_metadata(
        &mut self,
        root: &Path,
        dir: &Path,
        bundle_path: &str,
        path_id: i64,
    ) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|e| {
            format!(
                "Failed to read preview cache folder {}: {}",
                dir.display(),
                e
            )
        })? {
            let entry = entry.map_err(|e| format!("Failed to read preview cache entry: {}", e))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|e| format!("Failed to stat preview cache entry: {}", e))?;
            if file_type.is_dir() {
                self.scan_preview_metadata(root, &path, bundle_path, path_id)?;
            } else if is_preview_cache_meta(&path)
                && preview_cache_meta_matches(&path, bundle_path, path_id)?
            {
                self.schedule_cache_folder_and_outputs(root, &path);
            }
        }
        Ok(())
    }

    fn schedule_cache_folder_and_outputs(&mut self, root: &Path, meta_path: &Path) {
        if let Some(cache_folder) = meta_path.parent() {
            self.schedule_existing(cache_folder.to_path_buf());
            if let Some(safe_name) = cache_folder.file_name().and_then(|value| value.to_str()) {
                for suffix in ["mesh", "textures", "animated", "mesh_animated"] {
                    self.schedule_existing(root.join(format!("{}_{}", safe_name, suffix)));
                }
            }
        }
    }

    fn schedule_existing(&mut self, path: PathBuf) {
        if path.exists() {
            self.scheduled.insert(path);
        }
    }

    fn remove_scheduled(&mut self) -> Result<(), String> {
        let mut paths = self.scheduled.iter().cloned().collect::<Vec<_>>();
        paths.sort_by_key(|path| path.components().count());
        for path in paths {
            if !path.exists() {
                continue;
            }
            let bytes = path_size(&path).unwrap_or(0);
            if path.is_dir() {
                fs::remove_dir_all(&path).map_err(|e| {
                    format!(
                        "Failed to remove preview cache folder {}: {}",
                        path.display(),
                        e
                    )
                })?;
            } else {
                fs::remove_file(&path).map_err(|e| {
                    format!(
                        "Failed to remove preview cache file {}: {}",
                        path.display(),
                        e
                    )
                })?;
            }
            self.removed_paths += 1;
            self.removed_bytes += bytes;
            self.details.push(path.to_string_lossy().to_string());
        }
        Ok(())
    }

    fn into_result(self) -> ClearPreviewCacheResult {
        ClearPreviewCacheResult {
            removed_paths: self.removed_paths,
            removed_bytes: self.removed_bytes,
            details: self.details,
        }
    }
}

fn preview_cache_root_names() -> [&'static str; 2] {
    ["game_object_preview_glb", "animator_preview_glb"]
}

fn is_preview_cache_meta(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|name| name.starts_with("preview_cache") && name.ends_with(".json"))
        .unwrap_or(false)
}

fn preview_cache_meta_matches(
    meta_path: &Path,
    bundle_path: &str,
    path_id: i64,
) -> Result<bool, String> {
    let Ok(json) = fs::read_to_string(meta_path) else {
        return Ok(false);
    };
    let Ok(value) = serde_json::from_str::<Value>(&json) else {
        return Ok(false);
    };
    let cached_bundle = value
        .get("bundle_path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let cached_path_id = value
        .get("path_id")
        .and_then(Value::as_i64)
        .or_else(|| value.get("path_id").and_then(Value::as_str)?.parse().ok())
        .unwrap_or(0);
    Ok(cached_path_id == path_id && same_cache_path(cached_bundle, bundle_path))
}

fn same_cache_path(left: &str, right: &str) -> bool {
    normalize_cache_path(left) == normalize_cache_path(right)
}

fn normalize_cache_path(path: &str) -> String {
    let normalized = path.replace('\\', "/").trim_end_matches('/').to_string();
    if cfg!(windows) {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

fn path_size(path: &Path) -> Result<u64, String> {
    let meta =
        fs::metadata(path).map_err(|e| format!("Failed to stat {}: {}", path.display(), e))?;
    if meta.is_file() {
        return Ok(meta.len());
    }
    let mut total = 0u64;
    for entry in fs::read_dir(path)
        .map_err(|e| format!("Failed to read folder {}: {}", path.display(), e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read folder entry: {}", e))?;
        total = total.saturating_add(path_size(&entry.path()).unwrap_or(0));
    }
    Ok(total)
}

#[cfg(test)]
mod preview_cache_cleaner_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_cache_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("assetfinder_{}_{}", name, nonce))
    }

    #[test]
    fn clear_asset_removes_game_object_meta_and_matching_glb_outputs() {
        let root = temp_cache_dir("preview_cache_clear_asset");
        let cache_root = root.join("game_object_preview_glb");
        let meta_dir = cache_root.join("ch_f_japan_onmyoji_lv_s11");
        fs::create_dir_all(&meta_dir).unwrap();
        fs::write(
            meta_dir.join("preview_cache_v2_m1_t1_a0.json"),
            r#"{"bundle_path":"D:\\NarakaTest\\6\\6\\660019f01e306713","path_id":-337566547204893701}"#,
        )
        .unwrap();
        fs::write(
            meta_dir.join("ch_f_japan_onmyoji_lv_s11_textures.glb"),
            b"stale glb",
        )
        .unwrap();
        fs::write(
            meta_dir.join("ch_f_japan_onmyoji_lv_s11_mesh.glb"),
            b"stale mesh glb",
        )
        .unwrap();

        let result = PreviewCacheCleaner::clear_asset(
            &root,
            "D:/NarakaTest/6/6/660019f01e306713",
            -337566547204893701,
        )
        .unwrap();

        assert_eq!(result.removed_paths, 1);
        assert!(!meta_dir.exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn clear_asset_still_removes_legacy_split_game_object_glb_output_folders() {
        let root = temp_cache_dir("preview_cache_clear_asset_legacy_split");
        let cache_root = root.join("game_object_preview_glb");
        let meta_dir = cache_root.join("ch_f_japan_onmyoji_lv_s11");
        let mesh_dir = cache_root.join("ch_f_japan_onmyoji_lv_s11_mesh");
        let textured_dir = cache_root.join("ch_f_japan_onmyoji_lv_s11_textures");
        fs::create_dir_all(&meta_dir).unwrap();
        fs::create_dir_all(&mesh_dir).unwrap();
        fs::create_dir_all(&textured_dir).unwrap();
        fs::write(
            meta_dir.join("preview_cache_v2_m1_t1_a0.json"),
            r#"{"bundle_path":"D:\\NarakaTest\\6\\6\\660019f01e306713","path_id":-337566547204893701}"#,
        )
        .unwrap();
        fs::write(
            textured_dir.join("ch_f_japan_onmyoji_lv_s11_textures.glb"),
            b"stale glb",
        )
        .unwrap();
        fs::write(
            mesh_dir.join("ch_f_japan_onmyoji_lv_s11_mesh.glb"),
            b"stale mesh glb",
        )
        .unwrap();

        let result = PreviewCacheCleaner::clear_asset(
            &root,
            "D:/NarakaTest/6/6/660019f01e306713",
            -337566547204893701,
        )
        .unwrap();

        assert_eq!(result.removed_paths, 3);
        assert!(!meta_dir.exists());
        assert!(!mesh_dir.exists());
        assert!(!textured_dir.exists());

        let _ = fs::remove_dir_all(&root);
    }
}

/**
 * Get metadata info of a Mesh asset.
 *
 * Parse the Mesh object using TypeTree, extract vertex count, triangle count, index info, etc.,
 * for display in the frontend properties panel.
 */
#[tauri::command]
pub async fn get_mesh_info(bundle_path: String, path_id: String) -> Result<MeshInfo, String> {
    TaskRunner::run_blocking(
        "mesh_info",
        "Mesh Info",
        format!("Read Mesh info: {} (path_id={})", bundle_path, path_id),
        Some("Mesh info ready".to_string()),
        move |ctx| {
            let path_id: i64 = path_id
                .parse()
                .map_err(|e| format!("Invalid path_id: {}", e))?;
            let bundle_path = Path::new(&bundle_path);
            if !bundle_path.exists() {
                return Err(format!("File does not exist: {}", bundle_path.display()));
            }
            let bundle = AssetBundleLoader::load_bundle(bundle_path)
                .map_err(|e| format!("Failed to parse Bundle: {:?}", e))?;
            ctx.check_cancelled()?;

            let mut found_obj_idx = 0usize;
            let mut found_file = None;
            for file in &bundle.assets {
                for (idx, obj) in file.objects.iter().enumerate() {
                    if obj.path_id == path_id {
                        found_obj_idx = idx;
                        found_file = Some(file);
                        break;
                    }
                }
                if found_file.is_some() {
                    break;
                }
            }
            let file =
                found_file.ok_or_else(|| format!("Asset with path_id={} not found", path_id))?;
            let info = &file.objects[found_obj_idx];

            let handle = ObjectHandle::new(file, info);
            let obj = handle
                .read()
                .map_err(|e| format!("TypeTree parse failed: {:?}", e))?;
            let props = match &obj {
                crate::unity::type_tree::unity_value::UnityValue::Object(map) => map,
                _ => return Err(format!("TypeTree parse result is not Object")),
            };

            let byte_size = info.byte_size as usize;
            let mut vertex_count = 0usize;
            let mut triangle_count = 0usize;
            let mut has_indices = false;

            if let Some(vd) = props.get("m_VertexData") {
                if let crate::unity::type_tree::unity_value::UnityValue::Object(vd_obj) = vd {
                    if let Some(vc) = vd_obj.get("m_VertexCount") {
                        vertex_count = vc.as_i64().unwrap_or(0) as usize;
                    }
                }
            }
            if let Some(sm) = props.get("m_SubMeshes") {
                if let crate::unity::type_tree::unity_value::UnityValue::Array(arr) = sm {
                    for sub in arr {
                        if let crate::unity::type_tree::unity_value::UnityValue::Object(so) = sub {
                            triangle_count +=
                                so.get("triangleCount")
                                    .or_else(|| so.get("indexCount"))
                                    .or_else(|| so.get("m_TriangleCount"))
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0) as usize;
                        }
                    }
                }
            }
            if triangle_count == 0 {
                if let Some(ib) = props.get("m_IndexBuffer") {
                    let idx = WorkspaceUtils::deep_extract_u32s(ib);
                    if !idx.is_empty() {
                        triangle_count = idx.len() / 3;
                    }
                }
            }
            if let Some(ib) = props.get("m_IndexBuffer") {
                has_indices = !WorkspaceUtils::deep_extract_u32s(ib).is_empty();
            }
            if vertex_count == 0 {
                if let Some(ic) = props
                    .get("m_IndexCount")
                    .or_else(|| props.get("m_VertexCount"))
                {
                    vertex_count = ic.as_i64().unwrap_or(0) as usize;
                }
            }

            Ok(MeshInfo {
                byte_size,
                guessed_vertex_count: vertex_count,
                guessed_triangle_count: triangle_count,
                has_indices,
                parse_success: vertex_count > 0,
            })
        },
    )
    .await
}

/**
 * Extract Mesh geometry data (vertices + indices) from Bundle.
 *
 * Parse and extract mesh vertex positions and triangle indices using TypeTree,
 * supports multiple storage formats: VertexData, CompressedMesh, m_Vertices, etc.
 * Automatically selects the appropriate parsing algorithm based on Unity version.
 *
 * Loads only serialized Bundle files first. External stream data is read later
 * by exact StreamingInfo ranges, avoiding full .resS extraction for previews.
 *
 * The progress Channel pushes realtime progress for each phase, displayed in the frontend log panel.
 */
#[tauri::command]
pub async fn extract_mesh_geometry(
    bundle_path: String,
    path_id: String,
    progress: tauri::ipc::Channel<ProgressPayload>,
    cache_dir: Option<String>,
) -> Result<MeshGeometry, String> {
    TaskRunner::run_blocking(
        "extract_mesh",
        "Extract Mesh",
        format!(
            "Extract Mesh geometry: {} (path_id={})",
            bundle_path, path_id
        ),
        None,
        move |ctx| {
            MeshPreviewWorkflow::extract_geometry(ctx, bundle_path, path_id, progress, cache_dir)
        },
    )
    .await
}

#[tauri::command]
pub async fn extract_animator_geometry(
    bundle_path: String,
    path_id: String,
    workspace_dir: String,
    cache_dir: Option<String>,
    asset_map_cache_root: Option<String>,
    progress: tauri::ipc::Channel<ProgressPayload>,
) -> Result<MeshGeometry, String> {
    TaskRunner::run_blocking(
        "extract_animator",
        "Animator Preview",
        format!(
            "Extract Animator preview geometry: {} (path_id={})",
            bundle_path, path_id
        ),
        None,
        move |ctx| {
            AnimatorPreviewWorkflow::extract_geometry(
                ctx,
                bundle_path,
                path_id,
                workspace_dir,
                cache_dir,
                asset_map_cache_root,
                progress,
            )
        },
    )
    .await
}

#[tauri::command]
pub async fn extract_animator_preview_glb(
    bundle_path: String,
    path_id: String,
    workspace_dir: String,
    cache_dir: String,
    asset_map_cache_root: Option<String>,
    options: Option<PreviewSelectionOptions>,
    progress: tauri::ipc::Channel<ProgressPayload>,
) -> Result<AnimatorPreviewGlbResult, String> {
    TaskRunner::run_blocking(
        "extract_animator_glb",
        "Animator Preview",
        format!(
            "Build Animator preview GLB: {} (path_id={})",
            bundle_path, path_id
        ),
        None,
        move |ctx| {
            AnimatorPreviewWorkflow::extract_preview_glb(
                ctx,
                bundle_path,
                path_id,
                workspace_dir,
                cache_dir,
                asset_map_cache_root,
                options.unwrap_or_default(),
                progress,
            )
        },
    )
    .await
}

#[tauri::command]
pub async fn extract_game_object_preview_glb(
    bundle_path: String,
    path_id: String,
    workspace_dir: String,
    cache_dir: String,
    asset_map_cache_root: Option<String>,
    options: Option<PreviewSelectionOptions>,
    progress: tauri::ipc::Channel<ProgressPayload>,
    incremental: tauri::ipc::Channel<IncrementalPreviewPayload>,
) -> Result<AnimatorPreviewGlbResult, String> {
    TaskRunner::run_blocking(
        "extract_game_object_glb",
        "GameObject Preview",
        format!(
            "Build GameObject preview GLB: {} (path_id={})",
            bundle_path, path_id
        ),
        None,
        move |ctx| {
            GameObjectPreviewWorkflow::extract_preview_glb(
                ctx,
                bundle_path,
                path_id,
                workspace_dir,
                cache_dir,
                asset_map_cache_root,
                options.unwrap_or_default(),
                progress,
                Some(incremental),
            )
        },
    )
    .await
}

#[tauri::command]
pub async fn clear_asset_preview_cache(
    bundle_path: String,
    path_id: String,
    cache_dir: String,
) -> Result<ClearPreviewCacheResult, String> {
    TaskRunner::run_blocking(
        "clear_asset_preview_cache",
        "Clear Preview Cache",
        format!("Clear preview cache: {} (path_id={})", bundle_path, path_id),
        Some("Preview cache cleared".to_string()),
        move |_ctx| {
            let path_id: i64 = path_id
                .parse()
                .map_err(|e| format!("Invalid path_id: {}", e))?;
            PreviewCacheCleaner::clear_asset(Path::new(&cache_dir), &bundle_path, path_id)
        },
    )
    .await
}

#[tauri::command]
pub async fn clear_all_preview_cache(cache_dir: String) -> Result<ClearPreviewCacheResult, String> {
    TaskRunner::run_blocking(
        "clear_all_preview_cache",
        "Clear Preview Cache",
        "Clear all preview cache".to_string(),
        Some("All preview cache cleared".to_string()),
        move |_ctx| PreviewCacheCleaner::clear_all(Path::new(&cache_dir)),
    )
    .await
}

#[tauri::command]
pub async fn extract_bundle_to_cache(
    bundle_path: String,
    cache_dir: String,
) -> Result<Vec<crate::common::bundle_file::bundle_types::ExtractedNode>, String> {
    TaskRunner::run_blocking(
        "extract_bundle_cache",
        "Extract Bundle Cache",
        format!("Extract bundle nodes to cache: {}", bundle_path),
        None,
        move |ctx| {
            let bundle_path = Path::new(&bundle_path);
            if !bundle_path.exists() {
                return Err(format!("File does not exist: {}", bundle_path.display()));
            }
            let cache_dir = Path::new(&cache_dir);

            ctx.progress("load", 1, 3, "Loading Bundle");
            let bundle = AssetBundleLoader::load_bundle(bundle_path)
                .map_err(|e| format!("Failed to parse Bundle: {:?}", e))?;
            ctx.check_cancelled()?;
            ctx.progress("extract", 2, 3, "Extracting Bundle nodes");
            let result =
                crate::common::bundle_file::bundle_extractor::BundleExtractor::extract_all_nodes(
                    &bundle,
                    bundle_path,
                    cache_dir,
                )?;
            ctx.progress(
                "done",
                3,
                3,
                format!("Extracted {} nodes", result.nodes.len()),
            );
            ctx.success(format!("Bundle cache ready: {} nodes", result.nodes.len()));

            Ok(result.nodes)
        },
    )
    .await
}

#[tauri::command]
pub async fn resolve_mesh_preview_diffuse_texture(
    bundle_path: String,
    path_id: String,
    workspace_dir: String,
    cache_dir: String,
    asset_map_cache_root: Option<String>,
    progress: tauri::ipc::Channel<ProgressPayload>,
) -> Result<Option<MeshPreviewDiffuseTextureResult>, String> {
    let candidates = resolve_mesh_preview_texture_candidates(
        bundle_path,
        path_id,
        workspace_dir,
        cache_dir,
        asset_map_cache_root,
        progress,
    )
    .await?;
    Ok(candidates
        .into_iter()
        .next()
        .map(|candidate| MeshPreviewDiffuseTextureResult {
            png_path: candidate.png_path,
            texture_path_id: candidate.texture_path_id,
            texture_name: candidate.texture_name,
            texture_bundle_path: candidate.texture_bundle_path,
            material_name: candidate.material_name,
            material_index: candidate.material_index,
            slot_name: candidate.slot_name,
            usage: candidate.usage,
        }))
}

#[tauri::command]
pub async fn resolve_mesh_preview_texture_candidates(
    bundle_path: String,
    path_id: String,
    workspace_dir: String,
    cache_dir: String,
    asset_map_cache_root: Option<String>,
    progress: tauri::ipc::Channel<ProgressPayload>,
) -> Result<Vec<MeshPreviewTextureCandidate>, String> {
    TaskRunner::run_blocking(
        "mesh_preview_textures",
        "Mesh Preview Textures",
        format!(
            "Resolve Mesh preview textures: {} (path_id={})",
            bundle_path, path_id
        ),
        None,
        move |ctx| {
            let _ = cache_dir;
            let mesh_path_id: i64 = path_id
                .parse()
                .map_err(|e| format!("Invalid path_id: {}", e))?;
            let bundle_path_ref = Path::new(&bundle_path);
            if !bundle_path_ref.exists() {
                return Err(format!(
                    "File does not exist: {}",
                    bundle_path_ref.display()
                ));
            }

            let workspace_path = Path::new(&workspace_dir);
            let cache_root_buf = asset_map_cache_root
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from);
            let Some(cache_root_buf) = cache_root_buf else {
                ctx.warn("AssetMap cache root not provided; build AssetMap before textured mesh preview");
                progress
                    .send(ProgressPayload {
                        step: "diffuse".into(),
                        message: "AssetMap cache root not provided; build AssetMap before textured mesh preview"
                            .into(),
                    })
                    .ok();
                return Ok(Vec::new());
            };
            let cache_root = Some(cache_root_buf.as_path());
            if !AssetMapRepository::exists(workspace_path, cache_root) {
                ctx.warn("AssetMap not found; build AssetMap before textured mesh preview");
                progress
                    .send(ProgressPayload {
                        step: "diffuse".into(),
                        message: "AssetMap not found; build AssetMap before textured mesh preview"
                            .into(),
                    })
                    .ok();
                return Ok(Vec::new());
            }
            ctx.progress("load", 1, 3, "Opening AssetMap");
            let db = AssetMapRepository::open(workspace_path, cache_root)?;
            ctx.check_cancelled()?;

            let mesh_asset_name = db
                .find_by_bundle_and_path_id(&bundle_path, mesh_path_id)
                .ok()
                .flatten()
                .map(|row| row.asset_name)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("mesh_{}", mesh_path_id));

            progress
                .send(ProgressPayload {
                    step: "diffuse".into(),
                    message: format!("Resolving related textures for Mesh '{}'", mesh_asset_name),
                })
                .ok();

            let candidates = MeshPreviewTextureResolver::find_texture_candidates(
                &db,
                mesh_path_id,
                &mesh_asset_name,
                &bundle_path,
                &progress,
            );
            ctx.progress(
                "resolve",
                2,
                3,
                format!("Resolved {} raw texture candidate(s)", candidates.len()),
            );
            ctx.check_cancelled()?;
            if candidates.is_empty() {
                progress
                    .send(ProgressPayload {
                        step: "diffuse".into(),
                        message: "No related Texture2D candidates found for Mesh preview".into(),
                    })
                    .ok();
                return Ok(Vec::new());
            }

            let mut preview_candidates = Vec::new();
            for (
                texture_path_id,
                texture_bundle_path,
                texture_name,
                material_name,
                material_index,
                slot_name,
                usage,
            ) in candidates
            {
                preview_candidates.push(MeshPreviewTextureCandidate {
                    png_path: String::new(),
                    texture_path_id: texture_path_id.to_string(),
                    texture_name,
                    texture_bundle_path,
                    material_name,
                    material_index,
                    slot_name,
                    usage,
                    width: 0,
                    height: 0,
                    texture_format: 0,
                    texture_format_name: String::new(),
                    byte_size: 0,
                    has_alpha: false,
                    color_space_name: String::new(),
                    wrap_mode_name: String::new(),
                    filter_mode_name: String::new(),
                });
            }

            progress
                .send(ProgressPayload {
                    step: "diffuse".into(),
                    message: format!(
                        "Resolved {} texture candidate(s) for Mesh preview",
                        preview_candidates.len()
                    ),
                })
                .ok();
            ctx.progress(
                "done",
                3,
                3,
                format!(
                    "Resolved {} texture candidate(s) for Mesh preview",
                    preview_candidates.len()
                ),
            );
            ctx.success(format!(
                "Mesh preview textures ready: {} candidate(s)",
                preview_candidates.len()
            ));

            Ok(preview_candidates)
        },
    )
    .await
}

#[tauri::command]
pub async fn resolve_preview_animators(
    bundle_path: String,
    path_id: String,
    class_name: String,
    workspace_dir: String,
    asset_map_cache_root: Option<String>,
    cache_dir: Option<String>,
    progress: tauri::ipc::Channel<ProgressPayload>,
) -> Result<Vec<PreviewAnimatorRef>, String> {
    TaskRunner::run_blocking(
        "preview_animators",
        "Preview Animators",
        format!(
            "Resolve preview Animators: {} (path_id={})",
            bundle_path, path_id
        ),
        None,
        move |ctx| {
            let path_id: i64 = path_id
                .parse()
                .map_err(|e| format!("Invalid path_id: {}", e))?;
            if !Path::new(&bundle_path).exists() {
                return Err(format!("File does not exist: {}", bundle_path));
            }

            // Cache check
            if let Some(ref cache_dir) = cache_dir {
                let cache_path = Path::new(cache_dir)
                    .join("game_object_preview_glb")
                    .join(format!("game_object_{}", path_id))
                    .join(format!("game_object_{}_animators.json", path_id));
                if cache_path.exists() {
                    if let Ok(json) = std::fs::read_to_string(&cache_path) {
                        if let Ok(cached) =
                            serde_json::from_str::<Vec<PreviewAnimatorRef>>(&json)
                        {
                            ctx.success(format!(
                                "Preview Animators ready (cached): {} animator(s)",
                                cached.len()
                            ));
                            return Ok(cached);
                        }
                    }
                }
            }

            let cache_root_buf = asset_map_cache_root
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from);
            let Some(cache_root_buf) = cache_root_buf else {
                ctx.warn("AssetMap cache root not provided; build AssetMap before Animator scan");
                progress
                    .send(ProgressPayload {
                        step: "animator".into(),
                        message:
                            "AssetMap cache root not provided; build AssetMap before Animator scan"
                                .into(),
                    })
                    .ok();
                return Ok(Vec::new());
            };
            let workspace_path = Path::new(&workspace_dir);
            let cache_root = Some(cache_root_buf.as_path());
            if !AssetMapRepository::exists(workspace_path, cache_root) {
                ctx.warn("AssetMap not found; build AssetMap before Animator scan");
                progress
                    .send(ProgressPayload {
                        step: "animator".into(),
                        message: "AssetMap not found; build AssetMap before Animator scan".into(),
                    })
                    .ok();
                return Ok(Vec::new());
            }

            ctx.progress("load", 1, 3, "Opening AssetMap");
            let db = AssetMapRepository::open(workspace_path, cache_root)?;
            ctx.check_cancelled()?;
            progress
                .send(ProgressPayload {
                    step: "animator".into(),
                    message: format!(
                        "Resolving Animator refs for {} path_id={}",
                        class_name, path_id
                    ),
                })
                .ok();

            let animators = ExportAnimationUtils::collect_preview_animators_with_progress(
                &class_name,
                &bundle_path,
                path_id,
                &db,
                Some(&progress),
            );

            // Save to cache
            if let Some(ref cache_dir) = cache_dir {
                if let Ok(json) = serde_json::to_string(&animators) {
                    let cache_path = Path::new(cache_dir)
                        .join("game_object_preview_glb")
                        .join(format!("game_object_{}", path_id));
                    let _ = std::fs::create_dir_all(&cache_path);
                    let _ = std::fs::write(
                        cache_path.join(format!("game_object_{}_animators.json", path_id)),
                        json,
                    );
                }
            }

            ctx.progress(
                "done",
                3,
                3,
                format!("Resolved {} Animator ref(s)", animators.len()),
            );
            ctx.success(format!(
                "Preview Animators ready: {} animator(s)",
                animators.len()
            ));
            Ok(animators)
        },
    )
    .await
}

#[tauri::command]
pub async fn resolve_preview_animation_clips(
    bundle_path: String,
    path_id: String,
    class_name: String,
    workspace_dir: String,
    asset_map_cache_root: Option<String>,
    cache_dir: Option<String>,
    progress: tauri::ipc::Channel<ProgressPayload>,
) -> Result<Vec<PreviewAnimationClipRef>, String> {
    TaskRunner::run_blocking(
        "preview_animation_clips",
        "Preview AnimationClips",
        format!(
            "Resolve preview AnimationClips: {} (path_id={})",
            bundle_path, path_id
        ),
        None,
        move |ctx| {
            let path_id: i64 = path_id
                .parse()
                .map_err(|e| format!("Invalid path_id: {}", e))?;
            if !Path::new(&bundle_path).exists() {
                return Err(format!("File does not exist: {}", bundle_path));
            }

            // Cache check
            if let Some(ref cache_dir) = cache_dir {
                let cache_path = Path::new(cache_dir)
                    .join("game_object_preview_glb")
                    .join(format!("game_object_{}", path_id))
                    .join(format!("game_object_{}_clips.json", path_id));
                if cache_path.exists() {
                    if let Ok(json) = std::fs::read_to_string(&cache_path) {
                        if let Ok(cached) =
                            serde_json::from_str::<Vec<PreviewAnimationClipRef>>(&json)
                        {
                            ctx.success(format!(
                                "Preview AnimationClips ready (cached): {} clip(s)",
                                cached.len()
                            ));
                            return Ok(cached);
                        }
                    }
                }
            }

            let cache_root_buf = asset_map_cache_root
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from);
            let Some(cache_root_buf) = cache_root_buf else {
                ctx.warn("AssetMap cache root not provided; build AssetMap before AnimationClip scan");
                progress
                    .send(ProgressPayload {
                        step: "animation".into(),
                        message:
                            "AssetMap cache root not provided; build AssetMap before AnimationClip scan"
                                .into(),
                    })
                    .ok();
                return Ok(Vec::new());
            };
            let workspace_path = Path::new(&workspace_dir);
            let cache_root = Some(cache_root_buf.as_path());
            if !AssetMapRepository::exists(workspace_path, cache_root) {
                ctx.warn("AssetMap not found; build AssetMap before AnimationClip scan");
                progress
                    .send(ProgressPayload {
                        step: "animation".into(),
                        message: "AssetMap not found; build AssetMap before AnimationClip scan"
                            .into(),
                    })
                    .ok();
                return Ok(Vec::new());
            }

            ctx.progress("load", 1, 3, "Opening AssetMap");
            let db = AssetMapRepository::open(workspace_path, cache_root)?;
            ctx.check_cancelled()?;
            progress
                .send(ProgressPayload {
                    step: "animation".into(),
                    message: format!(
                        "Resolving AnimationClip refs for {} path_id={}",
                        class_name, path_id
                    ),
                })
                .ok();

            let clips = ExportAnimationUtils::collect_preview_animation_clips(
                &class_name,
                &bundle_path,
                path_id,
                &db,
                Some(&progress),
            );

            // Save to cache
            if let Some(ref cache_dir) = cache_dir {
                if let Ok(json) = serde_json::to_string(&clips) {
                    let cache_path = Path::new(cache_dir)
                        .join("game_object_preview_glb")
                        .join(format!("game_object_{}", path_id));
                    let _ = std::fs::create_dir_all(&cache_path);
                    let _ = std::fs::write(
                        cache_path.join(format!("game_object_{}_clips.json", path_id)),
                        json,
                    );
                }
            }

            ctx.progress(
                "done",
                3,
                3,
                format!("Resolved {} AnimationClip ref(s)", clips.len()),
            );
            ctx.success(format!("Preview AnimationClips ready: {} clip(s)", clips.len()));
            Ok(clips)
        },
    )
    .await
}
