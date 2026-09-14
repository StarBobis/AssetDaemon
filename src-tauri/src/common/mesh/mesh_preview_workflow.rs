/*
 * Mesh preview workflow for extracting interactive Mesh geometry.
 *
 * Keeps Bundle loading, cancellation, progress logging, and preview-size
 * safeguards behind one small interface for the Tauri command adapter.
 */

use crate::common::bundle_file::asset_bundle::{AssetBundleLoader, ObjectHandle, SerializedFile};
use crate::common::mesh::mesh_service::MeshService;
use crate::common::mesh::mesh_types::MeshGeometry;
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::task::task_context::TaskContext;
use crate::common::task::task_logger::TaskLogger;
use std::path::Path;
use std::sync::atomic::Ordering;

const MAX_MESH_PREVIEW_VERTICES: usize = 300_000;
const MAX_MESH_PREVIEW_INDICES: usize = 900_000;

pub struct MeshPreviewWorkflow;

impl MeshPreviewWorkflow {
    pub fn extract_geometry(
        ctx: TaskContext,
        bundle_path: String,
        path_id: String,
        progress: tauri::ipc::Channel<ProgressPayload>,
        cache_dir: Option<String>,
    ) -> Result<MeshGeometry, String> {
        let task_id = ctx.task_id().to_string();
        let cancel_token = ctx.cancel_token();
        TaskLogger::progress(
            &task_id,
            "Extract Mesh",
            "Load Bundle",
            1,
            5,
            "Loading Bundle file...",
        );

        progress
            .send(ProgressPayload {
                step: "load".into(),
                message: "Loading Bundle file...".into(),
            })
            .ok();

        let path_id_i64: i64 = path_id
            .parse()
            .map_err(|e| format!("Invalid path_id: {}", e))?;
        let bundle_path = Path::new(&bundle_path);
        if !bundle_path.exists() {
            TaskLogger::error(&task_id, "Extract Mesh", "File does not exist");
            return Err(format!("File does not exist: {}", bundle_path.display()));
        }

        let bundle = match AssetBundleLoader::load_bundle_serialized_only(bundle_path) {
            Ok(b) => b,
            Err(e) => {
                TaskLogger::error(
                    &task_id,
                    "Extract Mesh",
                    &format!("Failed to parse Bundle: {}", e),
                );
                return Err(format!("Failed to parse Bundle: {:?}", e));
            }
        };

        if cancel_token.load(Ordering::SeqCst) {
            TaskLogger::warn(
                &task_id,
                "Extract Mesh",
                "Mesh geometry extraction cancelled",
            );
            return Err("Task cancelled".to_string());
        }

        let cache_path = cache_dir.as_ref().map(Path::new);
        progress
            .send(ProgressPayload {
                step: "load".into(),
                message: format!(
                    "Preview load complete: {} serialized file(s), resource payloads deferred",
                    bundle.assets.len()
                ),
            })
            .ok();
        TaskLogger::progress(
            &task_id,
            "Extract Mesh",
            "Find Mesh",
            2,
            5,
            "Finding Mesh object...",
        );

        progress
            .send(ProgressPayload {
                step: "find".into(),
                message: "Finding asset object...".into(),
            })
            .ok();
        TaskLogger::progress(
            &task_id,
            "Extract Mesh",
            "Find Mesh",
            3,
            5,
            "Finding Mesh object...",
        );

        let mut found_obj_idx: usize = 0;
        let mut found_file_ref: Option<&SerializedFile> = None;
        let mut same_path_id_classes: Vec<i32> = Vec::new();
        for file in &bundle.assets {
            for (idx, obj) in file.objects.iter().enumerate() {
                if obj.path_id == path_id_i64 {
                    same_path_id_classes.push(obj.class_id);
                    if obj.class_id == 43 {
                        found_obj_idx = idx;
                        found_file_ref = Some(file);
                        break;
                    }
                }
            }
            if found_file_ref.is_some() {
                break;
            }
        }

        let file = found_file_ref.ok_or_else(|| {
            TaskLogger::error(
                &task_id,
                "Extract Mesh",
                &format!(
                    "Mesh with path_id={} not found. Same path_id classes: {:?}",
                    path_id_i64, same_path_id_classes
                ),
            );
            format!(
                "Mesh with path_id={} not found. Same path_id classes: {:?}",
                path_id_i64, same_path_id_classes
            )
        })?;
        let info = &file.objects[found_obj_idx];
        let handle = ObjectHandle::new(file, info);
        progress
            .send(ProgressPayload {
                step: "find".into(),
                message: format!(
                    "Found Mesh object: class_id={}, byte_size={}B, same path_id classes={:?}",
                    info.class_id, info.byte_size, same_path_id_classes
                ),
            })
            .ok();

        if info.byte_size > 0 && info.byte_size <= 512 {
            let message = format!(
                "Empty Mesh: byte_size={}B, skip geometry parsing",
                info.byte_size
            );
            progress
                .send(ProgressPayload {
                    step: "skip".into(),
                    message: message.clone(),
                })
                .ok();
            TaskLogger::info(&task_id, "Extract Mesh", &message);
            return Ok(Self::empty_geometry_result("Empty Mesh".to_string(), 0, 0));
        }

        let unity_version = &file.unity_version;
        let raw_data = handle.raw_data().map_err(|e| {
            TaskLogger::error(
                &task_id,
                "Extract Mesh",
                &format!("Failed to get raw data: {}", e),
            );
            format!("Failed to get raw data: {}", e)
        })?;

        progress
            .send(ProgressPayload {
                step: "parse".into(),
                message: "Parsing Mesh geometry data...".into(),
            })
            .ok();
        TaskLogger::progress(
            &task_id,
            "Extract Mesh",
            "Parse Mesh",
            4,
            5,
            "Parsing Mesh geometry data...",
        );

        let result = MeshService::extract_mesh_preview_from_object(
            &handle,
            &bundle,
            bundle_path,
            unity_version,
            &raw_data,
            &progress,
            cache_path,
        )
        .map_err(|e| {
            TaskLogger::error(
                &task_id,
                "Extract Mesh",
                &format!("Mesh parse failed: {}", e),
            );
            e
        })?;

        let vc = result.vertex_count;
        let tc = result.triangle_count;
        if vc > MAX_MESH_PREVIEW_VERTICES || result.indices.len() > MAX_MESH_PREVIEW_INDICES {
            let message = format!(
                "Mesh is too large for interactive preview: {} vertices, {} indices. Export is still available.",
                vc,
                result.indices.len()
            );
            progress
                .send(ProgressPayload {
                    step: "skip".into(),
                    message: message.clone(),
                })
                .ok();
            TaskLogger::warn(&task_id, "Extract Mesh", &message);
            return Ok(Self::empty_geometry_result(message, vc, tc));
        }

        progress
            .send(ProgressPayload {
                step: "done".into(),
                message: format!("Extraction complete: {} vertices, {} triangles", vc, tc),
            })
            .ok();
        TaskLogger::progress(
            &task_id,
            "Extract Mesh",
            "Complete",
            5,
            5,
            &format!("Extraction complete: {} vertices, {} triangles", vc, tc),
        );
        TaskLogger::success(
            &task_id,
            "Extract Mesh",
            &format!(
                "Mesh geometry extraction complete: {} vertices, {} triangles",
                vc, tc
            ),
        );

        Ok(result)
    }

    fn empty_geometry_result(
        error: String,
        vertex_count: usize,
        triangle_count: usize,
    ) -> MeshGeometry {
        MeshGeometry {
            vertices: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            tangents: Vec::new(),
            colors: Vec::new(),
            bone_weights: Vec::new(),
            bone_indices: Vec::new(),
            bind_poses: Vec::new(),
            bone_name_hashes: Vec::new(),
            root_bone_name_hash: None,
            sub_meshes: Vec::new(),
            parts: Vec::new(),
            blend_shapes: Vec::new(),
            indices: Vec::new(),
            success: false,
            error,
            vertex_count,
            triangle_count,
        }
    }
}
