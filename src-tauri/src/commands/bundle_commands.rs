/*
 * Bundle commands -scan, parse metadata, extract nodes, export Mesh preview.
 *
 * This file only contains #[tauri::command] functions; actual logic delegates to corresponding service classes.
 * Follows Soul.md convention: commands separated from service classes.
 */

use crate::common::bundle_file::asset_bundle::AssetBundleLoader;
use std::fs;
use std::path::Path;
use tauri::ipc::Channel;

use crate::common::bundle_file::bundle_asset_summary_utils::BundleAssetSummaryUtils;
use crate::common::bundle_file::bundle_types::{
    BundleFileEntry, BundleMeta, BundleNodeInfo, BundleParseChannelResult, ExtractedNode,
};
use crate::common::bundle_file::bundle_utils::BundleFileCollector;
use crate::common::bundle_file::metadata_cache::BundleMetadataCache;
use crate::common::export::export_animation_utils::ExportAnimationUtils;
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::scene::hierarchy_service::HierarchyBuilder;
use crate::common::scene::scene_types::SceneHierarchyResult;
use crate::common::task::task_runner::TaskRunner;
use crate::exporter::glb_exporter::GlbExporter;
use crate::exporter::mesh_exporter::{MeshBlendShape, MeshExporter, MeshSubMesh};
use crate::exporter::model_context::ModelContextResolver;

/**
 * Scan a directory and list all Unity file candidates.
 */
#[tauri::command]
pub async fn scan_bundle_files(
    dir: String,
    progress: Channel<ProgressPayload>,
) -> Result<Vec<BundleFileEntry>, String> {
    TaskRunner::run_blocking(
        "scan_bundle_files",
        "Scan Bundle Files",
        format!("Scanning directory: {}", dir),
        None,
        move |ctx| {
            let dir_path = Path::new(&dir);
            if !dir_path.exists() {
                return Err(format!("Directory does not exist: {}", dir));
            }
            if !dir_path.is_dir() {
                return Err(format!("Path is not a directory: {}", dir));
            }
            let _ = progress.send(ProgressPayload {
                step: "scan".into(),
                message: format!("Scanning directory: {}", dir_path.display()),
            });
            ctx.progress(
                "scan",
                0,
                1,
                format!("Scanning directory: {}", dir_path.display()),
            );
            let mut entries: Vec<BundleFileEntry> = Vec::new();
            BundleFileCollector::collect_bundle_files_with_progress(
                dir_path,
                &mut entries,
                Some(&progress),
            )?;
            ctx.check_cancelled()?;
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            let _ = progress.send(ProgressPayload {
                step: "done".into(),
                message: format!("Found {} files", entries.len()),
            });
            ctx.progress("done", 1, 1, format!("Found {} files", entries.len()));
            ctx.success(format!("Scan complete: {} files", entries.len()));
            Ok(entries)
        },
    )
    .await
}

/**
 * Scan multiple paths (files and directories) to collect all Unity file candidates.
 *
 * Used for "select files" and "drag-drop import" -supports mixed file and folder paths.
 */
#[tauri::command]
pub async fn collect_bundle_paths(paths: Vec<String>) -> Result<Vec<BundleFileEntry>, String> {
    TaskRunner::run_blocking(
        "collect_bundle_paths",
        "Collect Bundle Paths",
        format!("Collecting Bundle paths from {} inputs", paths.len()),
        None,
        move |ctx| {
            let mut entries: Vec<BundleFileEntry> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let total = paths.len().max(1);

            for (index, raw) in paths.iter().enumerate() {
                ctx.check_cancelled()?;
                ctx.progress("collect", index + 1, total, format!("Collecting: {}", raw));
                let p = Path::new(raw);
                if !p.exists() {
                    continue;
                }

                if p.is_dir() {
                    BundleFileCollector::collect_bundle_files(p, &mut entries)?;
                } else if p.is_file() && !BundleFileCollector::is_resource_payload_path(p) {
                    let entry = match BundleFileCollector::collect_file_entry(p) {
                        Ok(entry) => entry,
                        Err(_) => continue,
                    };
                    if seen.insert(entry.path.clone()) {
                        entries.push(entry);
                    }
                }
            }

            entries.sort_by(|a, b| a.name.cmp(&b.name));
            ctx.success(format!("Collected {} Bundle paths", entries.len()));
            Ok(entries)
        },
    )
    .await
}

/**
 * Asynchronously parse metadata of a single .bundle file.
 *
 * Returns immediately; parse results are pushed asynchronously via Channel without blocking the frontend UI.
 * Called after the frontend "clicks a file" to display its contents.
 */
#[tauri::command]
pub async fn parse_bundle_metadata(
    path: String,
    on_result: Channel<BundleParseChannelResult>,
    workspace_dir: Option<String>,
    asset_map_cache_root: Option<String>,
) -> Result<(), String> {
    if !Path::new(&path).exists() {
        let _ = on_result.send(BundleParseChannelResult {
            success: false,
            meta: None,
            error: Some(format!("File does not exist: {}", path)),
        });
        return Ok(());
    }

    let file_name = Path::new(&path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    if BundleFileCollector::is_resource_payload_path(Path::new(&path)) {
        let message = format!(
            "{} is a Unity resource payload (.resS/.resource/.res), not a standalone SerializedFile/UnityFS Bundle. It is loaded through StreamingInfo from its matching CAB/assets file.",
            file_name
        );
        let _ = on_result.send(BundleParseChannelResult {
            success: false,
            meta: None,
            error: Some(message),
        });
        return Ok(());
    }

    let ctx = TaskRunner::create("parse_bundle", "ParseBundle");
    ctx.info(format!("Loading bundle metadata: {}", file_name));
    ctx.progress("metadata", 0, 2, "Checking AssetMap cache");

    TaskRunner::spawn_blocking_detached(ctx, move |ctx| {
        let bundle_path = Path::new(&path);

        // Prefer checking the AssetMap DB from Map build (second-level load, no reparse needed).
        // This can still read and serialize many rows, so it must stay off the async command thread.
        let workspace_path = workspace_dir
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(Path::new);
        let cache_root = asset_map_cache_root
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(Path::new);
        let cached_meta = match workspace_path {
            Some(workspace) if cache_root.is_some() => {
                BundleMetadataCache::try_load_with_workspace(bundle_path, workspace, cache_root)
            }
            _ => None,
        };

        if let Some(cached_meta) = cached_meta {
            let assets_len = cached_meta.assets.len();
            ctx.progress(
                "metadata",
                2,
                2,
                format!("Loaded from AssetMap cache: {} assets", assets_len),
            );
            ctx.success(format!("Cache hit: {} ({} assets)", file_name, assets_len));
            let _ = on_result.send(BundleParseChannelResult {
                success: true,
                meta: Some(cached_meta),
                error: None,
            });
            return Ok(());
        }

        if ctx.is_cancelled() {
            ctx.warn(format!("Metadata loading cancelled: {}", file_name));
            let _ = on_result.send(BundleParseChannelResult {
                success: false,
                meta: None,
                error: Some("Task cancelled".to_string()),
            });
            return Ok(());
        }

        ctx.progress("metadata", 1, 2, "AssetMap cache miss, parsing Bundle file");

        let mut parse_logs: Vec<String> = Vec::new();
        let result = AssetBundleLoader::load_unity_file_with_logs(bundle_path, &mut parse_logs)
            .map_err(|e| format!("Failed to parse Unity file '{}': {:?}", path, e));

        // Check cancellation
        if ctx.is_cancelled() {
            ctx.warn(format!("Parse cancelled: {}", file_name));
            let _ = on_result.send(BundleParseChannelResult {
                success: false,
                meta: None,
                error: Some("Task cancelled".to_string()),
            });
            return Ok(());
        }

        match result {
            Ok(bundle) => {
                let file_size = bundle.size();
                let compressed = bundle.is_compressed();
                let unity_version = bundle
                    .assets
                    .first()
                    .map(|file| file.unity_version.clone())
                    .unwrap_or_default();

                ctx.info(format!(
                    "Unity file loaded: {} bytes, Unity {}, compressed={}",
                    file_size, unity_version, compressed
                ));

                let nodes: Vec<BundleNodeInfo> = bundle
                    .nodes
                    .iter()
                    .map(|node| BundleNodeInfo {
                        name: node.name.clone(),
                        offset: node.offset,
                        size: node.size,
                    })
                    .collect();

                let assets = BundleAssetSummaryUtils::build_asset_summaries(&bundle);

                ctx.success(format!(
                    "Parse complete: {} assets ({} nodes)",
                    assets.len(),
                    nodes.len()
                ));

                let meta = BundleMeta {
                    path,
                    size: file_size,
                    compressed,
                    unity_version,
                    nodes,
                    assets,
                    files_diag: bundle.files_diag,
                    parse_logs,
                };
                let _ = on_result.send(BundleParseChannelResult {
                    success: true,
                    meta: Some(meta),
                    error: None,
                });
            }
            Err(e) => {
                ctx.error(format!("Parse failed: {}", e));
                let _ = on_result.send(BundleParseChannelResult {
                    success: false,
                    meta: None,
                    error: Some(e),
                });
            }
        }
        Ok(())
    });
    Ok(())
}

/**
 * Extract specified node data from a .bundle file to a file.
 *
 * Called after the user clicks a node in the node list; writes the raw node data to the output directory.
 */
#[tauri::command]
pub async fn extract_node_to_file(
    bundle_path: String,
    node_name: String,
    output_dir: String,
) -> Result<ExtractedNode, String> {
    TaskRunner::run_blocking(
        "extract_node",
        "Extract Node",
        format!("Extract node: {} from {}", node_name, bundle_path),
        None,
        move |ctx| {
            let bundle_path = Path::new(&bundle_path);
            let output_dir = Path::new(&output_dir);
            if !bundle_path.exists() {
                return Err(format!("File does not exist: {}", bundle_path.display()));
            }
            ctx.progress("load", 1, 4, "Loading Bundle");
            let bundle = AssetBundleLoader::load_bundle(bundle_path)
                .map_err(|e| format!("Failed to parse Bundle: {:?}", e))?;
            ctx.check_cancelled()?;
            ctx.progress("find", 2, 4, format!("Finding node: {}", node_name));
            let node = bundle
                .nodes
                .iter()
                .find(|n| n.name == node_name)
                .ok_or_else(|| format!("Node '{}' not found", node_name))?;
            let data = bundle
                .extract_node_data(node)
                .map_err(|e| format!("Failed to extract node data '{}': {:?}", node_name, e))?;
            ctx.check_cancelled()?;
            ctx.progress("write", 3, 4, "Writing node data");
            fs::create_dir_all(output_dir).map_err(|e| {
                format!(
                    "Cannot create output directory '{}': {}",
                    output_dir.display(),
                    e
                )
            })?;
            let output_path = output_dir.join(&node_name);
            let written = data.len() as u64;
            fs::write(&output_path, &data)
                .map_err(|e| format!("Failed to write file '{}': {}", output_path.display(), e))?;
            ctx.progress("done", 4, 4, format!("Extracted {} bytes", written));
            ctx.success(format!("Node extracted: {}", output_path.display()));
            Ok(ExtractedNode {
                file_path: output_path.to_string_lossy().to_string(),
                size: written,
            })
        },
    )
    .await
}

/**
 * Export mesh preview as GLB file.
 *
 * Prefer TypeTree extraction for geometry data (more accurate),
 * fall back to raw data heuristic parsing.
 */
#[tauri::command]
pub async fn export_mesh_preview(
    bundle_path: String,
    path_id: String,
    cache_dir: String,
) -> Result<String, String> {
    TaskRunner::run_blocking(
        "export_mesh_preview",
        "Export Mesh Preview",
        format!("Export mesh preview: {} (path_id={})", bundle_path, path_id),
        None,
        move |ctx| {
    let path_id_i64: i64 = path_id
        .parse()
        .map_err(|e| format!("Invalid path_id: {}", e))?;
    let bundle_path = Path::new(&bundle_path);
    let cache_dir = Path::new(&cache_dir);
    if !bundle_path.exists() {
        return Err(format!("File does not exist: {}", bundle_path.display()));
    }
    fs::create_dir_all(cache_dir).map_err(|e| format!("Cannot create cache directory: {}", e))?;
    ctx.progress("load", 1, 4, "Loading Bundle");

    // Prefer TypeTree extraction (call MeshService directly, not cross-command)
    let noop_channel =
        tauri::ipc::Channel::<crate::common::scan::scan_types::ProgressPayload>::new(|_| Ok(()));
    if let Ok(bundle) = AssetBundleLoader::load_bundle(bundle_path) {
        ctx.check_cancelled()?;
        ctx.progress("extract", 2, 4, "Extracting mesh geometry");
        let found = bundle.assets.iter().find_map(|sf| {
            sf.objects
                .iter()
                .find(|o| o.path_id == path_id_i64 && (o.class_id == 43 || o.class_id == 34))
                .map(|o| (sf, o))
        });
        if let Some((file, info)) = found {
            let handle = crate::common::bundle_file::asset_bundle::ObjectHandle::new(file, info);
            let raw_data = handle.raw_data().ok();
            if let Some(raw) = raw_data {
                let cache_path = std::env::temp_dir().join("assetdaemon_mesh_export");
                let _ = fs::create_dir_all(&cache_path);
                let _ = crate::common::bundle_file::bundle_extractor::BundleExtractor::extract_all_nodes(
                    &bundle, bundle_path, &cache_path,
                );
                let geo = crate::common::mesh::mesh_service::MeshService::extract_mesh_from_object(
                    &handle,
                    &bundle,
                    bundle_path,
                    &file.unity_version,
                    &raw,
                    &noop_channel,
                    Some(&cache_path),
                );
                if let Ok(geo) = geo {
                    if geo.success && !geo.vertices.is_empty() {
                        let sub_meshes: Vec<MeshSubMesh> = geo
                            .sub_meshes
                            .iter()
                            .map(|sm| MeshSubMesh {
                                index_start: sm.index_start,
                                index_count: sm.index_count,
                                topology: sm.topology,
                            })
                            .collect();
                        let blend_shapes: Vec<MeshBlendShape> = geo
                            .blend_shapes
                            .iter()
                            .map(|shape| MeshBlendShape {
                                name: shape.name.clone(),
                                delta_vertices: shape.delta_vertices.clone(),
                                delta_normals: shape.delta_normals.clone(),
                                delta_tangents: shape.delta_tangents.clone(),
                            })
                            .collect();
                        let attrs = crate::exporter::mesh_exporter::MeshAttributes {
                            vertices: &geo.vertices,
                            indices: &geo.indices,
                            normals: if geo.normals.len() >= geo.vertices.len() {
                                Some(&geo.normals)
                            } else {
                                None
                            },
                            uvs: if geo.uvs.len() >= 2 * geo.vertices.len() / 3 {
                                Some(&geo.uvs)
                            } else {
                                None
                            },
                            tangents: if geo.tangents.len() >= 4 * geo.vertices.len() / 3 {
                                Some(&geo.tangents)
                            } else {
                                None
                            },
                            colors: if geo.colors.len() >= 4 * geo.vertices.len() / 3 {
                                Some(&geo.colors)
                            } else {
                                None
                            },
                            bone_weights: if geo.bone_weights.len() >= 4 * geo.vertices.len() / 3 {
                                Some(&geo.bone_weights)
                            } else {
                                None
                            },
                            bone_indices: if geo.bone_indices.len() >= 4 * geo.vertices.len() / 3 {
                                Some(&geo.bone_indices)
                            } else {
                                None
                            },
                            bind_poses: if geo.bind_poses.len() >= 16 {
                                Some(&geo.bind_poses)
                            } else {
                                None
                            },
                            bone_name_hashes: if geo.bone_name_hashes.is_empty() {
                                None
                            } else {
                                Some(&geo.bone_name_hashes)
                            },
                            root_bone_name_hash: geo.root_bone_name_hash,
                            sub_meshes: if sub_meshes.is_empty() {
                                None
                            } else {
                                Some(&sub_meshes)
                            },
                            blend_shapes: if blend_shapes.is_empty() {
                                None
                            } else {
                                Some(&blend_shapes)
                            },
                        };
                        let skeleton = ModelContextResolver::resolve_skeleton_for_mesh_with_hashes(
                            &bundle,
                            path_id_i64,
                            geo.bind_poses.len() / 16,
                            &geo.bone_name_hashes,
                            geo.root_bone_name_hash,
                        );
                        let animations = skeleton
                            .as_ref()
                            .map(|skel| {
                                ExportAnimationUtils::extract_related_animations(
                                    &bundle,
                                    &bundle_path.to_string_lossy(),
                                    path_id_i64,
                                    None,
                                    None,
                                    skel,
                                    &blend_shapes,
                                    None,
                                )
                            })
                            .unwrap_or_default();
                        if let Ok(glb_data) = GlbExporter::build_with_scene_data(
                            &format!("mesh_{}", path_id_i64),
                            &attrs,
                            None,
                            skeleton.as_ref(),
                            if animations.is_empty() {
                                None
                            } else {
                                Some(&animations)
                            },
                        ) {
                            let node_name = format!("mesh_{}", path_id_i64);
                            let glb_name = format!("{}.glb", node_name);
                            let glb_path = cache_dir.join(&glb_name);
                            fs::write(&glb_path, &glb_data).map_err(|e| {
                                format!(
                                    "Failed to write mesh preview GLB '{}': {}",
                                    glb_path.display(),
                                    e
                                )
                            })?;
                            ctx.progress("done", 4, 4, "Mesh preview GLB ready");
                            ctx.success(format!("Mesh preview exported: {}", glb_path.display()));
                            return Ok(glb_path.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }

    // Fall back to raw data + heuristic parsing
    ctx.progress("fallback", 3, 4, "Using heuristic mesh export fallback");
    let bundle = AssetBundleLoader::load_bundle(bundle_path)
        .map_err(|e| format!("Failed to parse Bundle: {:?}", e))?;
    let (mesh_data, name) = bundle
        .assets
        .iter()
        .enumerate()
        .find_map(|(i, file)| {
            file.objects
                .iter()
                .find(|obj| obj.path_id == path_id_i64)
                .and_then(|obj| {
                    file.object_bytes(obj)
                        .ok()
                        .map(|bytes| (bytes.to_vec(), bundle.asset_names.get(i).cloned().flatten()))
                })
        })
        .ok_or_else(|| format!("Asset with path_id={} not found", path_id))?;
    if mesh_data.is_empty() {
        return Err("Mesh data is empty".to_string());
    }
    let node_name = name.unwrap_or_else(|| format!("mesh_{}", path_id_i64));
    let glb_name = format!("{}.glb", node_name);
    let glb_path = cache_dir.join(&glb_name);
    MeshExporter::mesh_data_to_glb(&mesh_data, &glb_path)
        .map_err(|e| format!("GLB export failed: {}", e))?;
    ctx.progress("done", 4, 4, "Mesh preview GLB ready");
    ctx.success(format!("Mesh preview exported: {}", glb_path.display()));
    Ok(glb_path.to_string_lossy().to_string())
        },
    )
    .await
}

/**
 * Get scene hierarchy tree (GameObject parent-child structure).
 *
 * Parse GameObject and Transform objects from all SerializedFiles in the Bundle,
 * build GameObject tree structure and return to frontend for rendering.
 */
#[tauri::command]
pub async fn get_scene_hierarchy(path: String) -> Result<SceneHierarchyResult, String> {
    TaskRunner::run_blocking(
        "scene_hierarchy",
        "Scene Hierarchy",
        format!("Loading scene hierarchy: {}", path),
        None,
        move |ctx| {
            let bundle_path = Path::new(&path);
            if !bundle_path.exists() {
                return Err(format!("File does not exist: {}", path));
            }

            let mut parse_logs: Vec<String> = Vec::new();
            ctx.progress("load", 1, 3, "Loading Bundle");
            let bundle =
                match AssetBundleLoader::load_bundle_with_logs(bundle_path, &mut parse_logs) {
                    Ok(b) => b,
                    Err(e) => {
                        return Err(format!("Failed to load Bundle '{}': {:?}", path, e));
                    }
                };

            ctx.check_cancelled()?;

            let mut diag: Vec<String> = Vec::new();
            ctx.progress("build", 2, 3, "Building hierarchy tree");
            let result = HierarchyBuilder::build_hierarchy(&bundle.assets, &mut diag);

            ctx.progress(
                "done",
                3,
                3,
                format!("Hierarchy tree: {} root nodes", result.roots.len()),
            );
            ctx.success(format!("Hierarchy tree: {} root nodes", result.roots.len()));
            Ok(result)
        },
    )
    .await
}
