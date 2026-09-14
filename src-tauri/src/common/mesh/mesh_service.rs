/*
 * Mesh service class - provides Mesh data parsing, vertex extraction, etc.
 *
 * Follows Soul.md convention: struct + impl organization, no free functions.
 *
 * ?
 * Parsing strategy (priority):
 *
 *   1. [Primary] AssetStudio-style byte stream parsing (parse_mesh_from_raw)
 *      Reads directly from Mesh raw bytes field by field, same version branching as AssetStudio.
 *      When vertices are in external .resS (has_streaming=true):
 *        -> load .resS -> process_stream_vertex_data()
 *
 *   2. [Fallback] Heuristic TypeTree parsing (last resort)
 *      Only used when AssetStudio parsing fails.
 *
 * ?
 *
 * Complex streaming loading logic delegated to StreamDataLoader,
 * TypeTree fallback parsing delegated to TypeTreeMeshParser.
 */

use crate::common::bundle_file::asset_bundle::AssetBundle;
use crate::unity::mesh_asset_studio::AssetStudioMeshParser;
use crate::unity::type_tree::unity_value::UnityValue;
use std::path::Path;

use super::mesh_typetree_parser::TypeTreeMeshParser;
use super::stream_data_loader::StreamDataLoader;
use crate::common::mesh::mesh_types::{MeshBlendShapeInfo, MeshGeometry, MeshSubMeshInfo};
use crate::common::scan::scan_types::ProgressPayload;

/**
 * Mesh data processing service class.
 */
pub struct MeshService;

impl MeshService {
    fn summarize_mesh_diagnostics(
        props: Option<&std::collections::HashMap<String, UnityValue>>,
        bundle: &AssetBundle,
        bundle_path: &Path,
        raw_data: &[u8],
    ) -> Vec<String> {
        let mut diag = Vec::new();
        diag.push(format!("raw_data={}B", raw_data.len()));

        if let Some(props) = props {
            if let Some(UnityValue::Object(vd)) = props.get("m_VertexData") {
                let vertex_count = vd
                    .get("m_VertexCount")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let channel_count = vd
                    .get("m_Channels")
                    .and_then(|v| match v {
                        UnityValue::Array(items) => Some(items.len()),
                        _ => None,
                    })
                    .unwrap_or(0);
                let stream_count = vd
                    .get("m_Streams")
                    .and_then(|v| match v {
                        UnityValue::Array(items) => Some(items.len()),
                        _ => None,
                    })
                    .unwrap_or(0);
                let inline_data_size = vd
                    .get("m_DataSize")
                    .and_then(|value| value.to_u8_slice())
                    .map(|bytes| bytes.len())
                    .or_else(|| vd.get("m_DataSize").map(|value| value.to_u8_vec().len()))
                    .unwrap_or(0);
                diag.push(format!(
                    "VertexData: vertex_count={}, channels={}, streams={}, inline_data={}B",
                    vertex_count, channel_count, stream_count, inline_data_size
                ));
            } else {
                diag.push("VertexData: missing".to_string());
            }

            if let Some(sd) = props.get("m_StreamData") {
                let (stream_path, stream_offset, stream_size) =
                    StreamDataLoader::extract_stream_info(sd);
                diag.push(format!(
                    "StreamData: path='{}', offset={}, size={}",
                    if stream_path.is_empty() {
                        "(empty)"
                    } else {
                        stream_path.as_str()
                    },
                    stream_offset,
                    stream_size
                ));
            } else {
                diag.push("StreamData: missing".to_string());
            }

            if let Some(UnityValue::Object(cm)) = props.get("m_CompressedMesh") {
                let packed_vertices = cm
                    .get("m_Vertices")
                    .and_then(|v| match v {
                        UnityValue::Object(o) => o.get("m_NumItems").and_then(|n| n.as_i64()),
                        _ => None,
                    })
                    .unwrap_or(0);
                let packed_triangles = cm
                    .get("m_Triangles")
                    .and_then(|v| match v {
                        UnityValue::Object(o) => o.get("m_NumItems").and_then(|n| n.as_i64()),
                        _ => None,
                    })
                    .unwrap_or(0);
                diag.push(format!(
                    "CompressedMesh: vertices_items={}, triangle_items={}",
                    packed_vertices, packed_triangles
                ));
            }
        }

        let ress_nodes: Vec<String> = bundle
            .nodes
            .iter()
            .filter(|node| node.name.to_lowercase().ends_with(".ress"))
            .take(5)
            .map(|node| format!("{}({}B)", node.name, node.size))
            .collect();
        diag.push(format!(
            "Bundle resources: indexed_resS={}, resS_nodes={}",
            bundle.resources.len(),
            if ress_nodes.is_empty() {
                "none".to_string()
            } else {
                ress_nodes.join(", ")
            }
        ));

        let sibling_resources = bundle_path
            .parent()
            .and_then(|parent| std::fs::read_dir(parent).ok())
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        let path = entry.path();
                        let lower = path
                            .file_name()
                            .map(|name| name.to_string_lossy().to_lowercase())
                            .unwrap_or_default();
                        if lower.ends_with(".ress")
                            || lower.ends_with(".resource")
                            || lower.ends_with(".res")
                        {
                            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                            Some(format!("{}({}B)", lower, size))
                        } else {
                            None
                        }
                    })
                    .take(5)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        diag.push(format!(
            "Sibling resource files: {}",
            if sibling_resources.is_empty() {
                "none".to_string()
            } else {
                sibling_resources.join(", ")
            }
        ));

        diag
    }

    fn classify_mesh_failure(
        props: Option<&std::collections::HashMap<String, UnityValue>>,
        bundle: &AssetBundle,
        bundle_path: &Path,
        raw_data: &[u8],
    ) -> String {
        let summary = Self::summarize_mesh_diagnostics(props, bundle, bundle_path, raw_data);
        let mut reason = "Mesh vertex buffer is not readable".to_string();

        if let Some(props) = props {
            let vertex_count = props
                .get("m_VertexData")
                .and_then(|v| match v {
                    UnityValue::Object(vd) => vd.get("m_VertexCount").and_then(|n| n.as_i64()),
                    _ => None,
                })
                .unwrap_or(0);
            let (stream_path, _, stream_size) = props
                .get("m_StreamData")
                .map(StreamDataLoader::extract_stream_info)
                .unwrap_or_default();
            let has_res = !bundle.resources.is_empty()
                || bundle
                    .nodes
                    .iter()
                    .any(|node| node.name.to_lowercase().ends_with(".ress"));

            if vertex_count == 0 && stream_path.is_empty() && stream_size == 0 {
                reason = "疑似Empty Mesh".to_string();
            } else if stream_size > 0 && !has_res {
                reason = "External vertex stream is referenced, but no .resS resource is available"
                    .to_string();
            } else if stream_path.is_empty() || stream_size == 0 {
                reason = "StreamData is invalid: missing path or size".to_string();
            }
        }

        format!("{} | {}", reason, summary.join(" | "))
    }

    fn map_sub_meshes(
        sub_meshes: &[crate::unity::mesh_asset_studio::ParsedSubMesh],
    ) -> Vec<MeshSubMeshInfo> {
        sub_meshes
            .iter()
            .map(|sm| MeshSubMeshInfo {
                index_start: sm.index_start,
                index_count: sm.index_count,
                topology: sm.topology,
            })
            .collect()
    }

    fn map_blend_shapes(
        blend_shapes: &[crate::unity::mesh_asset_studio::ParsedBlendShape],
    ) -> Vec<MeshBlendShapeInfo> {
        blend_shapes
            .iter()
            .map(|shape| MeshBlendShapeInfo {
                name: shape.name.clone(),
                weight: shape.weight,
                delta_vertices: shape.delta_vertices.clone(),
                delta_normals: shape.delta_normals.clone(),
                delta_tangents: shape.delta_tangents.clone(),
            })
            .collect()
    }

    fn strip_preview_auxiliary_data(geometry: &mut MeshGeometry) {
        geometry.tangents.clear();
        geometry.colors.clear();
        geometry.bone_weights.clear();
        geometry.bone_indices.clear();
        geometry.bind_poses.clear();
        geometry.bone_name_hashes.clear();
        geometry.root_bone_name_hash = None;
        geometry.blend_shapes.clear();
    }

    /**
     * Extract geometry data (vertices + indices) from a Unity Mesh object.
     *
     * Process:
     *   1. AssetStudio byte stream parsing (primary)
     *   2. On failure -> fallback to TypeTree heuristic parsing
     *   3. Streaming data (.resS) -> load + process_stream_vertex_data
     *
     * @param handle       ObjectHandle (used for fallback TypeTree parsing)
     * @param bundle       AssetBundle (used for extracting .resS from nodes)
     * @param bundle_path  Bundle file path
     * @param unity_version Unity version string
     * @param raw_data     Mesh object raw bytes
     * @param progress     Progress push Channel
     * @param cache_dir    Optional cache directory
     * @return             MeshGeometry
     */
    pub fn extract_mesh_from_object(
        handle: &crate::common::bundle_file::asset_bundle::ObjectHandle,
        bundle: &AssetBundle,
        bundle_path: &Path,
        unity_version: &str,
        raw_data: &[u8],
        progress: &tauri::ipc::Channel<ProgressPayload>,
        cache_dir: Option<&Path>,
    ) -> Result<MeshGeometry, String> {
        Self::extract_mesh_from_object_internal(
            handle,
            bundle,
            bundle_path,
            unity_version,
            raw_data,
            progress,
            cache_dir,
            true,
        )
    }

    pub fn extract_mesh_preview_from_object(
        handle: &crate::common::bundle_file::asset_bundle::ObjectHandle,
        bundle: &AssetBundle,
        bundle_path: &Path,
        unity_version: &str,
        raw_data: &[u8],
        progress: &tauri::ipc::Channel<ProgressPayload>,
        cache_dir: Option<&Path>,
    ) -> Result<MeshGeometry, String> {
        let mut geometry = Self::extract_mesh_from_object_internal(
            handle,
            bundle,
            bundle_path,
            unity_version,
            raw_data,
            progress,
            cache_dir,
            false,
        )?;
        Self::strip_preview_auxiliary_data(&mut geometry);
        Ok(geometry)
    }

    fn extract_mesh_from_object_internal(
        handle: &crate::common::bundle_file::asset_bundle::ObjectHandle,
        bundle: &AssetBundle,
        bundle_path: &Path,
        unity_version: &str,
        raw_data: &[u8],
        progress: &tauri::ipc::Channel<ProgressPayload>,
        cache_dir: Option<&Path>,
        include_auxiliary_data: bool,
    ) -> Result<MeshGeometry, String> {
        // Step 1: AssetStudio byte stream parsing (primary)
        progress
            .send(ProgressPayload {
                step: "assetstudio".into(),
                message: format!(
                    "AssetStudio parsing Mesh ({}B, Unity {})...",
                    raw_data.len(),
                    unity_version
                ),
            })
            .ok();

        if raw_data.len() < 64 {
            return Self::fallback_typetree(
                handle,
                bundle,
                bundle_path,
                unity_version,
                raw_data,
                progress,
                cache_dir,
            );
        }

        let mut field_logs: Vec<String> = Vec::new();
        let parse_result = if include_auxiliary_data {
            AssetStudioMeshParser::parse_mesh_from_raw_with_logs(
                raw_data,
                unity_version,
                &mut field_logs,
            )
        } else {
            AssetStudioMeshParser::parse_mesh_preview_from_raw_with_logs(
                raw_data,
                unity_version,
                &mut field_logs,
            )
        };

        match parse_result {
            Ok(parsed) => {
                let indices = parsed.indices.clone();

                if parsed.has_streaming {
                    // -- Scenario A: vertices in external .resS --
                    return Self::handle_streaming(
                        &parsed,
                        indices,
                        bundle_path,
                        cache_dir,
                        bundle,
                        unity_version,
                        progress,
                        Some(handle),
                    );
                }

                // -- Scenario B: inline vertices are empty, but channel info may exist -> try loading .resS streaming data --
                if parsed.vertices.is_empty() {
                    // If the parser recognized channel info, try loading streaming data from .resS
                    if !parsed.channels.is_empty() && parsed.vertex_count > 0 {
                        progress.send(ProgressPayload { step: "streaming".into(), message: format!(
                            "AssetStudio parser has channel info (ch={}, vc={}), trying to load .resS streaming data...",
                            parsed.channels.len(), parsed.vertex_count
                        ) }).ok();
                        // Prefer TypeTree's StreamingInfo for correct offset/size
                        let (stream_path_tt, stream_offset_tt, stream_size_tt) =
                            StreamDataLoader::extract_streaming_info_from_typetree(Some(handle))
                                .unwrap_or_default();
                        let use_tt_info = !stream_path_tt.is_empty() && stream_size_tt > 0;
                        let res_data = if use_tt_info {
                            progress
                                .send(ProgressPayload {
                                    step: "diag".into(),
                                    message: format!(
                                        "TypeTree StreamingInfo: path='{}', offset={}, size={}",
                                        stream_path_tt, stream_offset_tt, stream_size_tt
                                    ),
                                })
                                .ok();
                            StreamDataLoader::load_stream_data(
                                &stream_path_tt,
                                stream_offset_tt,
                                stream_size_tt as u32,
                                bundle_path,
                                cache_dir,
                                bundle,
                                progress,
                            )
                        } else {
                            progress
                                .send(ProgressPayload {
                                    step: "diag".into(),
                                    message: "TypeTree has no usable StreamingInfo; skipping unsafe full .resS fallback for preview"
                                        .into(),
                                })
                                .ok();
                            Vec::new()
                        };
                        progress
                            .send(ProgressPayload {
                                step: "diag".into(),
                                message: format!(
                                    ".resS data read: {}B (empty={})",
                                    res_data.len(),
                                    res_data.is_empty()
                                ),
                            })
                            .ok();
                        if !res_data.is_empty() {
                            let stream_data = AssetStudioMeshParser::process_stream_vertex_data(
                                &parsed,
                                &res_data,
                                unity_version,
                            );
                            let nan_count = stream_data
                                .vertices
                                .iter()
                                .filter(|f| !f.is_finite())
                                .count();
                            progress.send(ProgressPayload { step: "diag".into(), message: format!(
                                "process_stream_vertex_data result: {} floats, {} NaN, skin_weights={}, skin_indices={}",
                                stream_data.vertices.len(), nan_count,
                                stream_data.bone_weights.len() / 4,
                                stream_data.bone_indices.len() / 4
                            ) }).ok();
                            if !stream_data.vertices.is_empty() {
                                let vc = stream_data.vertices.len() / 3;
                                let tc = parsed.indices.len() / 3;
                                progress.send(ProgressPayload { step: "done".into(), message: format!(
                                    "Streaming extraction complete: {} vertices, {} triangles", vc, tc
                                ) }).ok();
                                return Ok(MeshGeometry {
                                    vertices: stream_data.vertices,
                                    normals: stream_data.normals,
                                    uvs: stream_data.uvs,
                                    tangents: stream_data.tangents,
                                    colors: stream_data.colors,
                                    bone_weights: if stream_data.bone_weights.is_empty() {
                                        parsed.bone_weights
                                    } else {
                                        stream_data.bone_weights
                                    },
                                    bone_indices: if stream_data.bone_indices.is_empty() {
                                        parsed.bone_indices
                                    } else {
                                        stream_data.bone_indices
                                    },
                                    bind_poses: parsed.bind_poses,
                                    bone_name_hashes: parsed.bone_name_hashes,
                                    root_bone_name_hash: parsed.root_bone_name_hash,
                                    sub_meshes: Self::map_sub_meshes(&parsed.sub_meshes),
                                    parts: Vec::new(),
                                    blend_shapes: Self::map_blend_shapes(&parsed.blend_shapes),
                                    indices: parsed.indices,
                                    success: true,
                                    error: String::new(),
                                    vertex_count: vc,
                                    triangle_count: tc,
                                });
                            }
                        }
                    }
                    // Fallback to TypeTree parsing
                    let diag = format!(
                        "AssetStudio parsed {} indices but vertices are empty (ch={}, streams={}, vc={})",
                        indices.len(), parsed.channels.len(), parsed.streams.len(), parsed.vertex_count
                    );
                    progress
                        .send(ProgressPayload {
                            step: "diag".into(),
                            message: format!(
                                "AssetStudio Mesh summary: has_streaming={}, stream_path='{}', stream_offset={}, stream_size={}, inline_data={}B",
                                parsed.has_streaming,
                                if parsed.stream_path.is_empty() { "(empty)" } else { parsed.stream_path.as_str() },
                                parsed.stream_offset,
                                parsed.stream_size,
                                parsed.data_size_bytes.len()
                            ),
                        })
                        .ok();
                    progress
                        .send(ProgressPayload {
                            step: "diag".into(),
                            message: diag.clone(),
                        })
                        .ok();
                    return Self::fallback_typetree(
                        handle,
                        bundle,
                        bundle_path,
                        unity_version,
                        raw_data,
                        progress,
                        cache_dir,
                    );
                }

                let vc = parsed.vertices.len() / 3;
                let tc = indices.len() / 3;
                return Ok(MeshGeometry {
                    vertices: parsed.vertices,
                    normals: parsed.normals,
                    uvs: parsed.uvs,
                    tangents: parsed.tangents,
                    colors: parsed.colors,
                    bone_weights: parsed.bone_weights,
                    bone_indices: parsed.bone_indices,
                    bind_poses: parsed.bind_poses,
                    bone_name_hashes: parsed.bone_name_hashes,
                    root_bone_name_hash: parsed.root_bone_name_hash,
                    sub_meshes: Self::map_sub_meshes(&parsed.sub_meshes),
                    parts: Vec::new(),
                    blend_shapes: Self::map_blend_shapes(&parsed.blend_shapes),
                    indices,
                    success: true,
                    error: String::new(),
                    vertex_count: vc,
                    triangle_count: tc,
                });
            }
            Err(e) => {
                progress
                    .send(ProgressPayload {
                        step: "diag".into(),
                        message: format!("AssetStudio failed: {}, falling back to TypeTree", e),
                    })
                    .ok();
                return Self::fallback_typetree(
                    handle,
                    bundle,
                    bundle_path,
                    unity_version,
                    raw_data,
                    progress,
                    cache_dir,
                );
            }
        }
    }

    // ============================================================
    // Streaming Data Handling
    // ============================================================

    fn handle_streaming(
        parsed: &crate::unity::mesh_asset_studio::AssetStudioMesh,
        indices: Vec<u32>,
        bundle_path: &Path,
        cache_dir: Option<&Path>,
        bundle: &AssetBundle,
        unity_version: &str,
        progress: &tauri::ipc::Channel<ProgressPayload>,
        type_tree_handle: Option<&crate::common::bundle_file::asset_bundle::ObjectHandle>,
    ) -> Result<MeshGeometry, String> {
        // If binary parser's streaming info is clearly wrong (offset/size=0), try to get correct values from TypeTree
        let (real_path, real_offset, real_size) = if parsed.stream_offset == 0
            && parsed.stream_size == 0
        {
            let tt_result =
                StreamDataLoader::extract_streaming_info_from_typetree(type_tree_handle);
            if tt_result.is_some() {
                progress
                    .send(ProgressPayload {
                        step: "diag".into(),
                        message: format!(
                    "[handle_streaming] TypeTree StreamingInfo valid, will use TypeTree values"
                ),
                    })
                    .ok();
            } else {
                // Diagnostic: print TypeTree structure to investigate why extraction failed
                StreamDataLoader::diagnose_typetree_streaming(type_tree_handle, progress);
                progress.send(ProgressPayload { step: "diag".into(), message: format!(
                    "[handle_streaming] TypeTree StreamingInfo invalid (None), falling back to binary parser values: path='{}', off={}, sz={}",
                    parsed.stream_path, parsed.stream_offset, parsed.stream_size
                ) }).ok();
            }
            tt_result.map(|(p, o, s)| (p, o, s)).unwrap_or_else(|| {
                (
                    parsed.stream_path.clone(),
                    parsed.stream_offset,
                    parsed.stream_size as usize,
                )
            })
        } else {
            progress
                .send(ProgressPayload {
                    step: "diag".into(),
                    message: format!(
                "[handle_streaming] Binary parser StreamingInfo valid: path='{}', off={}, sz={}",
                parsed.stream_path, parsed.stream_offset, parsed.stream_size
            ),
                })
                .ok();
            (
                parsed.stream_path.clone(),
                parsed.stream_offset,
                parsed.stream_size as usize,
            )
        };

        // Diagnostic: cleaned path
        let clean_path_debug = StreamDataLoader::clean_stream_path(&real_path);
        progress
            .send(ProgressPayload {
                step: "diag".into(),
                message: format!(
                    "[handle_streaming] After cleanup path='{}', offset={}, size={}",
                    clean_path_debug, real_offset, real_size
                ),
            })
            .ok();

        let res_data = StreamDataLoader::load_stream_data(
            &real_path,
            real_offset as u64,
            real_size as u32,
            bundle_path,
            cache_dir,
            bundle,
            progress,
        );

        if res_data.is_empty() {
            return Err(format!(
                "Mesh vertex data is in external .resS ({} indices), but could not load (path='{}')",
                indices.len(), parsed.stream_path
            ));
        }

        progress
            .send(ProgressPayload {
                step: "processdata".into(),
                message: format!(
                    "Merging streaming data ({:.1} KB)...",
                    res_data.len() as f64 / 1024.0
                ),
            })
            .ok();

        let stream_data =
            AssetStudioMeshParser::process_stream_vertex_data(parsed, &res_data, unity_version);

        if stream_data.vertices.is_empty() {
            return Err(format!(
                "process_stream_vertex_data failed ({} indices, data={}B, vc={})",
                indices.len(),
                parsed.data_size_bytes.len(),
                parsed.vertex_count
            ));
        }

        let vc = stream_data.vertices.len() / 3;
        let tc = indices.len() / 3;
        progress.send(ProgressPayload { step: "done".into(), message: format!(
            "Extraction complete: {} vertices (streaming), {} triangles, skin_weights={}, skin_indices={}",
            vc, tc, stream_data.bone_weights.len() / 4, stream_data.bone_indices.len() / 4
        ) }).ok();

        Ok(MeshGeometry {
            vertices: stream_data.vertices,
            normals: stream_data.normals,
            uvs: stream_data.uvs,
            tangents: stream_data.tangents,
            colors: stream_data.colors,
            bone_weights: if stream_data.bone_weights.is_empty() {
                parsed.bone_weights.clone()
            } else {
                stream_data.bone_weights
            },
            bone_indices: if stream_data.bone_indices.is_empty() {
                parsed.bone_indices.clone()
            } else {
                stream_data.bone_indices
            },
            bind_poses: parsed.bind_poses.clone(),
            bone_name_hashes: parsed.bone_name_hashes.clone(),
            root_bone_name_hash: parsed.root_bone_name_hash,
            sub_meshes: Self::map_sub_meshes(&parsed.sub_meshes),
            parts: Vec::new(),
            blend_shapes: Self::map_blend_shapes(&parsed.blend_shapes),
            indices,
            success: true,
            error: String::new(),
            vertex_count: vc,
            triangle_count: tc,
        })
    }

    // ============================================================
    // TypeTree Fallback Parsing
    // ============================================================

    /**
     * Fall back to TypeTree heuristic parsing.
     *
     * Used when AssetStudio parsing fails. Uses TypeTree to read vertex data,
     * supports VertexData / CompressedMesh / m_Vertices and other formats.
     */
    fn fallback_typetree(
        handle: &crate::common::bundle_file::asset_bundle::ObjectHandle,
        bundle: &AssetBundle,
        bundle_path: &Path,
        unity_version: &str,
        raw_data: &[u8],
        progress: &tauri::ipc::Channel<ProgressPayload>,
        cache_dir: Option<&Path>,
    ) -> Result<MeshGeometry, String> {
        progress
            .send(ProgressPayload {
                step: "typetree".into(),
                message: "Fallback: TypeTree parsing...".into(),
            })
            .ok();

        let obj = match handle.read() {
            Ok(o) => o,
            Err(e) => return Err(format!("TypeTree parsing failed: {:?}", e)),
        };
        let props = match &obj {
            UnityValue::Object(map) => map,
            _ => {
                return Err(format!(
                    "TypeTree parse result is not Object: {:?}",
                    obj.variant_name()
                ))
            }
        };

        let mut vertices: Vec<f32> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut diag = Vec::new();

        // Step 1: Indices
        if let Some(val) = props.get("m_IndexBuffer") {
            let use_16bit = props
                .get("m_IndexFormat")
                .and_then(|v| v.as_i64())
                .map(|f| f == 0)
                .or_else(|| {
                    props
                        .get("m_Use16BitIndices")
                        .and_then(|v| v.as_i64())
                        .map(|f| f != 0)
                });
            if let Some(is_16bit) = use_16bit {
                indices = super::workspace_utils::WorkspaceUtils::deep_extract_u32s_with_format(
                    val, is_16bit,
                );
            } else {
                indices = super::workspace_utils::WorkspaceUtils::deep_extract_u32s(val);
            }
        }

        // Step 2: VertexData
        if vertices.is_empty() {
            if let Some(val) = props.get("m_VertexData") {
                if let Some(parsed) = TypeTreeMeshParser::parse_vertex_data(val) {
                    vertices = parsed;
                    diag.push(format!("VertexData: {} vertices", vertices.len() / 3));
                }
            }
        }

        // Step 3: CompressedMesh
        if vertices.is_empty() {
            if let Some(val) = props.get("m_CompressedMesh") {
                if let Some(parsed) = TypeTreeMeshParser::parse_compressed_mesh_vertices(val) {
                    vertices = parsed;
                    diag.push(format!("CompressedMesh: {} vertices", vertices.len() / 3));
                }
                if indices.is_empty() {
                    if let Some(parsed_idx) = TypeTreeMeshParser::parse_compressed_mesh_indices(val)
                    {
                        indices = parsed_idx;
                    }
                }
            }
        }

        // Step 4: m_Vertices
        if vertices.is_empty() {
            if let Some(val) = props.get("m_Vertices") {
                vertices = TypeTreeMeshParser::deep_extract_floats(val);
                if !vertices.is_empty() {
                    diag.push(format!("m_Vertices: {} vertices", vertices.len() / 3));
                }
            }
        }

        // Step 5: m_StreamData external resource - load raw bytes from .resS,
        //          then use VertexData channel info to correctly parse position components
        if vertices.is_empty() {
            if let Some(sd) = props.get("m_StreamData") {
                progress
                    .send(ProgressPayload {
                        step: "streaming".into(),
                        message: "Loading streaming vertex data from .resS...".into(),
                    })
                    .ok();
                // Try using TypeTree-parsed VertexData channel info + .resS raw bytes
                let stream_bytes =
                    StreamDataLoader::load_raw_stream_bytes(sd, bundle, bundle_path, cache_dir);
                if !stream_bytes.is_empty() {
                    if let Some(vd) = props.get("m_VertexData") {
                        if let Some(parsed) =
                            TypeTreeMeshParser::parse_vertex_data_from_stream(vd, &stream_bytes)
                        {
                            vertices = parsed;
                            diag.push(format!(
                                "StreamData+VertexData: {} vertices",
                                vertices.len() / 3
                            ));
                        }
                    }
                }
                if vertices.is_empty() {
                    // Fallback: read f32 directly
                    vertices =
                        StreamDataLoader::try_read_stream_data(sd, bundle, bundle_path, cache_dir);
                    if !vertices.is_empty() {
                        diag.push(format!("StreamData(raw): {} vertices", vertices.len() / 3));
                    } else {
                        let (stream_path, stream_offset, stream_size) =
                            StreamDataLoader::extract_stream_info(sd);
                        progress.send(ProgressPayload { step: "streaming".into(), message: format!(
                            "  StreamData: path='{}', offset={}, size={} - failed to read valid vertex data",
                            if stream_path.is_empty() { "(empty)" } else { stream_path.as_str() },
                            stream_offset,
                            stream_size
                        ) }).ok();
                    }
                }
            }
        }

        if vertices.is_empty() {
            let prop_names: Vec<_> = props.iter().map(|(k, _)| k.clone()).collect();
            let diagnosis = Self::classify_mesh_failure(Some(props), bundle, bundle_path, raw_data);
            progress
                .send(ProgressPayload {
                    step: "diag".into(),
                    message: format!("Mesh failure diagnosis: {}", diagnosis),
                })
                .ok();
            return Err(format!(
                "TypeTree fallback also could not extract vertices. Diagnosis: {} | Fields: {} | Unity version: {}",
                diagnosis,
                prop_names.join(", "),
                unity_version
            ));
        }

        let vertex_count = vertices.len() / 3;
        let triangle_count = if indices.len() >= 3 {
            indices.len() / 3
        } else {
            0
        };
        progress
            .send(ProgressPayload {
                step: "done".into(),
                message: format!(
                    "Fallback complete: {} vertices, {} triangles",
                    vertex_count, triangle_count
                ),
            })
            .ok();

        Ok(MeshGeometry {
            vertices,
            indices,
            normals: vec![],
            uvs: vec![],
            tangents: vec![],
            colors: vec![],
            bone_weights: vec![],
            bone_indices: vec![],
            bind_poses: vec![],
            bone_name_hashes: vec![],
            root_bone_name_hash: None,
            sub_meshes: vec![],
            parts: vec![],
            blend_shapes: vec![],
            success: true,
            error: diag.join(" | "),
            vertex_count,
            triangle_count,
        })
    }
}
