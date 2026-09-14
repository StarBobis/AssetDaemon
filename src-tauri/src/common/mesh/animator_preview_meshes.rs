/*
 * Mesh parsing and geometry merging for Animator preview workflows.
 */

use crate::common::bundle_file::asset_bundle::{AssetBundleLoader, ObjectHandle};
use crate::common::mesh::mesh_service::MeshService;
use crate::common::mesh::mesh_types::{MeshGeometry, MeshGeometryPart, MeshSubMeshInfo};
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::task::task_logger::TaskLogger;
use crate::exporter::mesh_exporter::{MeshAttributes, MeshBlendShape, MeshSubMesh};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Instant;

use super::animator_preview_hierarchy::AnimatorMeshRef;

pub struct AnimatorPreviewMeshes;

impl AnimatorPreviewMeshes {
    pub fn parse_preview_mesh(
        bundle_path: &str,
        mesh_path_id: i64,
        progress: &tauri::ipc::Channel<ProgressPayload>,
        cache_path: Option<&Path>,
    ) -> Result<MeshGeometry, String> {
        extract_single_mesh_for_animator(bundle_path, mesh_path_id, progress, cache_path)
    }

    pub fn parse_full_mesh(
        bundle_path: &str,
        mesh_path_id: i64,
        progress: &tauri::ipc::Channel<ProgressPayload>,
        cache_path: Option<&Path>,
    ) -> Result<MeshGeometry, String> {
        extract_single_mesh_full_for_animator(bundle_path, mesh_path_id, progress, cache_path)
    }

    pub fn parse_full_meshes_parallel(
        mesh_refs: &[AnimatorMeshRef],
        progress: &tauri::ipc::Channel<ProgressPayload>,
        cache_path: Option<&Path>,
        task_id: &str,
        cancel_token: &Arc<AtomicBool>,
    ) -> Result<HashMap<(String, i64), MeshGeometry>, String> {
        let mut unique_refs = Vec::<AnimatorMeshRef>::new();
        let mut seen = HashSet::<(String, i64)>::new();
        for mesh_ref in mesh_refs {
            let key = (mesh_ref.mesh_bundle_path.clone(), mesh_ref.mesh_path_id);
            if seen.insert(key) {
                unique_refs.push(mesh_ref.clone());
            }
        }

        if unique_refs.is_empty() {
            return Ok(HashMap::new());
        }

        let worker_count = thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(4)
            .min(unique_refs.len())
            .min(8)
            .max(1);
        emit_log(
            task_id,
            progress,
            "mesh",
            format!(
                "Mesh parse plan: {} reference(s), {} unique Mesh(es), {} worker(s)",
                mesh_refs.len(),
                unique_refs.len(),
                worker_count
            ),
        );

        let started = Instant::now();
        let next_index = AtomicUsize::new(0);
        let (tx, rx) = mpsc::channel::<ParsedAnimatorMeshResult>();
        let mut parsed_cache = HashMap::new();
        let mut failures = Vec::<String>::new();
        let mut completed = 0usize;

        thread::scope(|scope| {
            for _ in 0..worker_count {
                let tx = tx.clone();
                let progress = progress.clone();
                let unique_refs = &unique_refs;
                let next_index = &next_index;
                let cancel_token = Arc::clone(cancel_token);
                scope.spawn(move || loop {
                    if cancel_token.load(Ordering::SeqCst) {
                        break;
                    }
                    let index = next_index.fetch_add(1, Ordering::SeqCst);
                    if index >= unique_refs.len() {
                        break;
                    }
                    let mesh_ref = &unique_refs[index];
                    let mesh_started = Instant::now();
                    let result = extract_single_mesh_full_for_animator(
                        &mesh_ref.mesh_bundle_path,
                        mesh_ref.mesh_path_id,
                        &progress,
                        cache_path,
                    );
                    let _ = tx.send(ParsedAnimatorMeshResult {
                        index,
                        key: (mesh_ref.mesh_bundle_path.clone(), mesh_ref.mesh_path_id),
                        result,
                        elapsed_ms: elapsed_ms(mesh_started),
                    });
                });
            }
            drop(tx);

            for item in rx {
                completed += 1;
                match item.result {
                    Ok(geometry) => {
                        emit_log(
                            task_id,
                            progress,
                            "mesh",
                            format!(
                                "Mesh parsed {}/{} (ref #{}) in {} ms: {} path_id={}, vertices={}, triangles={}, skinVertices={}, bones={}, blendShapes={}",
                                completed,
                                unique_refs.len(),
                                item.index + 1,
                                item.elapsed_ms,
                                bundle_display_name(&item.key.0),
                                item.key.1,
                                geometry.vertex_count,
                                geometry.triangle_count,
                                geometry.bone_weights.len() / 4,
                                geometry.bind_poses.len() / 16,
                                geometry.blend_shapes.len()
                            ),
                        );
                        parsed_cache.insert(item.key, geometry);
                    }
                    Err(error) => {
                        failures.push(format!("{}:{} ({})", item.key.0, item.key.1, error));
                        emit_log(
                            task_id,
                            progress,
                            "mesh",
                            format!(
                                "Mesh parse failed in {} ms: {} path_id={} ({})",
                                item.elapsed_ms,
                                bundle_display_name(&item.key.0),
                                item.key.1,
                                error
                            ),
                        );
                    }
                }
            }
        });

        check_cancelled(task_id, cancel_token)?;

        emit_log(
            task_id,
            progress,
            "mesh",
            format!(
                "Mesh parse phase done in {} ms: {} parsed, {} failed",
                elapsed_ms(started),
                parsed_cache.len(),
                failures.len()
            ),
        );

        Ok(parsed_cache)
    }

    pub fn empty_geometry() -> MeshGeometry {
        empty_mesh_geometry()
    }

    pub fn merge_preview_geometry(target: &mut MeshGeometry, source: MeshGeometry) {
        merge_mesh_geometry(target, source);
    }

    pub fn mesh_part(mesh_ref: &AnimatorMeshRef, geometry: &MeshGeometry) -> MeshGeometryPart {
        mesh_geometry_part(mesh_ref, geometry)
    }

    pub fn merge_glb_geometry(
        target: &mut MeshGeometry,
        source: MeshGeometry,
    ) -> Result<(), String> {
        merge_mesh_geometry_for_glb(target, source)
    }

    pub fn export_attributes(geometry: &MeshGeometry) -> AnimatorMeshExportAttributes<'_> {
        AnimatorMeshExportAttributes::new(geometry)
    }

    pub fn mesh_part_display_names(geometry: &MeshGeometry) -> Vec<String> {
        mesh_part_display_names(geometry)
    }
}

pub struct AnimatorMeshExportAttributes<'a> {
    geometry: &'a MeshGeometry,
    sub_meshes: Vec<MeshSubMesh>,
    blend_shapes: Vec<MeshBlendShape>,
}

impl<'a> AnimatorMeshExportAttributes<'a> {
    fn new(geometry: &'a MeshGeometry) -> Self {
        Self {
            geometry,
            sub_meshes: mesh_submeshes_for_export(geometry),
            blend_shapes: mesh_blend_shapes_for_export(geometry),
        }
    }

    pub fn mesh_attributes(&self) -> MeshAttributes<'_> {
        MeshAttributes {
            vertices: &self.geometry.vertices,
            indices: &self.geometry.indices,
            normals: optional_vertex_f32(&self.geometry.normals, self.geometry.vertex_count, 3),
            uvs: optional_vertex_f32(&self.geometry.uvs, self.geometry.vertex_count, 2),
            tangents: optional_vertex_f32(&self.geometry.tangents, self.geometry.vertex_count, 4),
            colors: optional_vertex_f32(&self.geometry.colors, self.geometry.vertex_count, 4),
            bone_weights: Some(&self.geometry.bone_weights),
            bone_indices: Some(&self.geometry.bone_indices),
            bind_poses: Some(&self.geometry.bind_poses),
            bone_name_hashes: if self.geometry.bone_name_hashes.is_empty() {
                None
            } else {
                Some(&self.geometry.bone_name_hashes)
            },
            root_bone_name_hash: self.geometry.root_bone_name_hash,
            sub_meshes: if self.sub_meshes.is_empty() {
                None
            } else {
                Some(&self.sub_meshes)
            },
            blend_shapes: if self.blend_shapes.is_empty() {
                None
            } else {
                Some(&self.blend_shapes)
            },
        }
    }

    pub fn morph_target_names(&self) -> Vec<String> {
        self.blend_shapes
            .iter()
            .map(|shape| shape.name.clone())
            .collect()
    }
}

struct ParsedAnimatorMeshResult {
    index: usize,
    key: (String, i64),
    result: Result<MeshGeometry, String>,
    elapsed_ms: u128,
}

fn extract_single_mesh_for_animator(
    bundle_path: &str,
    mesh_path_id: i64,
    progress: &tauri::ipc::Channel<ProgressPayload>,
    cache_path: Option<&Path>,
) -> Result<MeshGeometry, String> {
    let bundle_path_ref = Path::new(bundle_path);
    if !bundle_path_ref.exists() {
        return Err(format!("Mesh bundle not found: {}", bundle_path));
    }
    let bundle = AssetBundleLoader::load_bundle_serialized_only(bundle_path_ref)
        .map_err(|e| format!("Load Mesh bundle failed: {}", e))?;
    let (sf, obj) = bundle
        .assets
        .iter()
        .find_map(|sf| {
            sf.objects
                .iter()
                .find(|obj| obj.path_id == mesh_path_id && obj.class_id == 43)
                .map(|obj| (sf, obj))
        })
        .ok_or_else(|| format!("Mesh path_id={} not found", mesh_path_id))?;
    let handle = ObjectHandle::new(sf, obj);
    let raw_data = handle.raw_data()?;
    MeshService::extract_mesh_preview_from_object(
        &handle,
        &bundle,
        bundle_path_ref,
        &sf.unity_version,
        &raw_data,
        progress,
        cache_path,
    )
}

fn extract_single_mesh_full_for_animator(
    bundle_path: &str,
    mesh_path_id: i64,
    progress: &tauri::ipc::Channel<ProgressPayload>,
    cache_path: Option<&Path>,
) -> Result<MeshGeometry, String> {
    let bundle_path_ref = Path::new(bundle_path);
    if !bundle_path_ref.exists() {
        return Err(format!("Mesh bundle not found: {}", bundle_path));
    }
    let bundle = AssetBundleLoader::load_bundle_serialized_only(bundle_path_ref)
        .map_err(|e| format!("Load Mesh bundle failed: {}", e))?;
    let (sf, obj) = bundle
        .assets
        .iter()
        .find_map(|sf| {
            sf.objects
                .iter()
                .find(|obj| obj.path_id == mesh_path_id && obj.class_id == 43)
                .map(|obj| (sf, obj))
        })
        .ok_or_else(|| format!("Mesh path_id={} not found", mesh_path_id))?;
    let handle = ObjectHandle::new(sf, obj);
    let raw_data = handle.raw_data()?;
    MeshService::extract_mesh_from_object(
        &handle,
        &bundle,
        bundle_path_ref,
        &sf.unity_version,
        &raw_data,
        progress,
        cache_path,
    )
}

fn merge_mesh_geometry_for_glb(
    target: &mut MeshGeometry,
    source: MeshGeometry,
) -> Result<(), String> {
    let previous_vertex_count = target.vertex_count;
    merge_mesh_geometry(target, source.clone());

    if target.tangents.len() == previous_vertex_count * 4
        && source.tangents.len() >= source.vertex_count * 4
    {
        target
            .tangents
            .extend_from_slice(&source.tangents[..source.vertex_count * 4]);
    } else if !target.tangents.is_empty() || !source.tangents.is_empty() {
        target.tangents.clear();
    }
    if target.colors.len() == previous_vertex_count * 4
        && source.colors.len() >= source.vertex_count * 4
    {
        target
            .colors
            .extend_from_slice(&source.colors[..source.vertex_count * 4]);
    } else if !target.colors.is_empty() || !source.colors.is_empty() {
        target.colors.clear();
    }
    if target.bone_weights.len() == previous_vertex_count * 4
        && source.bone_weights.len() >= source.vertex_count * 4
    {
        target
            .bone_weights
            .extend_from_slice(&source.bone_weights[..source.vertex_count * 4]);
    } else if previous_vertex_count == 0 {
        target.bone_weights = source.bone_weights[..source.vertex_count * 4].to_vec();
    } else {
        return Err("Cannot merge skinned Mesh: bone weights are incomplete".to_string());
    }
    if target.bone_indices.len() == previous_vertex_count * 4
        && source.bone_indices.len() >= source.vertex_count * 4
    {
        target
            .bone_indices
            .extend_from_slice(&source.bone_indices[..source.vertex_count * 4]);
    } else if previous_vertex_count == 0 {
        target.bone_indices = source.bone_indices[..source.vertex_count * 4].to_vec();
    } else {
        return Err("Cannot merge skinned Mesh: bone indices are incomplete".to_string());
    }
    if previous_vertex_count == 0 {
        target.bind_poses = source.bind_poses;
        target.bone_name_hashes = source.bone_name_hashes;
        target.root_bone_name_hash = source.root_bone_name_hash;
        target.blend_shapes = source.blend_shapes;
    }
    target.success = true;
    Ok(())
}

fn mesh_submeshes_for_export(geometry: &MeshGeometry) -> Vec<MeshSubMesh> {
    geometry
        .sub_meshes
        .iter()
        .map(|sub_mesh| MeshSubMesh {
            index_start: sub_mesh.index_start,
            index_count: sub_mesh.index_count,
            topology: sub_mesh.topology,
        })
        .collect()
}

fn mesh_blend_shapes_for_export(geometry: &MeshGeometry) -> Vec<MeshBlendShape> {
    geometry
        .blend_shapes
        .iter()
        .map(|shape| MeshBlendShape {
            name: shape.name.clone(),
            delta_vertices: shape.delta_vertices.clone(),
            delta_normals: shape.delta_normals.clone(),
            delta_tangents: shape.delta_tangents.clone(),
        })
        .collect()
}

fn optional_vertex_f32(values: &[f32], vertex_count: usize, stride: usize) -> Option<&[f32]> {
    (values.len() >= vertex_count * stride).then_some(values)
}

pub fn empty_mesh_geometry() -> MeshGeometry {
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
        error: String::new(),
        vertex_count: 0,
        triangle_count: 0,
    }
}

pub fn merge_mesh_geometry(target: &mut MeshGeometry, source: MeshGeometry) {
    let vertex_offset = (target.vertices.len() / 3) as u32;
    let index_offset = target.indices.len();
    let source_vertex_count = source.vertices.len() / 3;

    target.vertices.extend(source.vertices);
    if target.normals.len() == target.vertex_count * 3
        && source.normals.len() >= source_vertex_count * 3
    {
        target
            .normals
            .extend_from_slice(&source.normals[..source_vertex_count * 3]);
    } else if !target.normals.is_empty() || !source.normals.is_empty() {
        target.normals.clear();
    }
    if target.uvs.len() == target.vertex_count * 2 && source.uvs.len() >= source_vertex_count * 2 {
        target
            .uvs
            .extend_from_slice(&source.uvs[..source_vertex_count * 2]);
    } else if !target.uvs.is_empty() || !source.uvs.is_empty() {
        target.uvs.clear();
    }

    target.indices.extend(
        source
            .indices
            .into_iter()
            .map(|index| index + vertex_offset),
    );
    for sub_mesh in source.sub_meshes {
        target.sub_meshes.push(MeshSubMeshInfo {
            index_start: sub_mesh.index_start + index_offset,
            index_count: sub_mesh.index_count,
            topology: sub_mesh.topology,
        });
    }
    target.vertex_count = target.vertices.len() / 3;
    target.triangle_count = target.indices.len() / 3;
}

fn mesh_geometry_part(mesh_ref: &AnimatorMeshRef, geometry: &MeshGeometry) -> MeshGeometryPart {
    MeshGeometryPart {
        name: mesh_ref.mesh_name.clone(),
        bundle_path: mesh_ref.mesh_bundle_path.clone(),
        path_id: mesh_ref.mesh_path_id,
        vertices: geometry.vertices.clone(),
        normals: geometry.normals.clone(),
        uvs: geometry.uvs.clone(),
        indices: geometry.indices.clone(),
        sub_meshes: geometry.sub_meshes.clone(),
        vertex_count: geometry.vertex_count,
        triangle_count: geometry.triangle_count,
    }
}

fn mesh_part_display_names(geometry: &MeshGeometry) -> Vec<String> {
    let mut names = Vec::new();
    for part in &geometry.parts {
        let base_name = if part.name.trim().is_empty() {
            format!("mesh_{}", part.path_id)
        } else {
            part.name.trim().to_string()
        };
        let sub_mesh_count = part.sub_meshes.len().max(1);
        if sub_mesh_count == 1 {
            names.push(base_name);
        } else {
            for index in 0..sub_mesh_count {
                names.push(format!("{} / Submesh {}", base_name, index + 1));
            }
        }
    }
    names
}

fn check_cancelled(task_id: &str, cancel_token: &Arc<AtomicBool>) -> Result<(), String> {
    if cancel_token.load(Ordering::SeqCst) {
        TaskLogger::warn(task_id, "Animator Preview", "Animator preview cancelled");
        Err("Task cancelled".to_string())
    } else {
        Ok(())
    }
}

fn elapsed_ms(start: Instant) -> u128 {
    start.elapsed().as_millis()
}

fn bundle_display_name(bundle_path: &str) -> &str {
    Path::new(bundle_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(bundle_path)
}

fn emit_log(
    task_id: &str,
    progress: &tauri::ipc::Channel<ProgressPayload>,
    step: &str,
    message: impl Into<String>,
) {
    let message = message.into();
    progress
        .send(ProgressPayload {
            step: step.into(),
            message: message.clone(),
        })
        .ok();
    TaskLogger::info(task_id, "Animator Preview", &message);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(vertices: Vec<f32>, indices: Vec<u32>) -> MeshGeometry {
        let vertex_count = vertices.len() / 3;
        MeshGeometry {
            vertices,
            normals: vec![0.0; vertex_count * 3],
            uvs: vec![0.0; vertex_count * 2],
            tangents: Vec::new(),
            colors: Vec::new(),
            bone_weights: Vec::new(),
            bone_indices: Vec::new(),
            bind_poses: Vec::new(),
            bone_name_hashes: Vec::new(),
            root_bone_name_hash: None,
            sub_meshes: vec![MeshSubMeshInfo {
                index_start: 0,
                index_count: indices.len(),
                topology: 0,
            }],
            parts: Vec::new(),
            blend_shapes: Vec::new(),
            indices,
            success: true,
            error: String::new(),
            vertex_count,
            triangle_count: 0,
        }
    }

    #[test]
    fn merge_mesh_geometry_offsets_indices_and_submeshes() {
        let mut merged = empty_mesh_geometry();
        merge_mesh_geometry(&mut merged, geometry(vec![0.0, 0.0, 0.0], vec![0]));
        merge_mesh_geometry(
            &mut merged,
            geometry(vec![1.0, 0.0, 0.0, 2.0, 0.0, 0.0], vec![0, 1]),
        );

        assert_eq!(merged.vertices.len() / 3, 3);
        assert_eq!(merged.indices, vec![0, 1, 2]);
        assert_eq!(merged.sub_meshes.len(), 2);
        assert_eq!(merged.sub_meshes[0].index_start, 0);
        assert_eq!(merged.sub_meshes[1].index_start, 1);
        assert_eq!(merged.sub_meshes[1].index_count, 2);
    }
}
