/*
 * Animator preview workflow for mesh-backed Animator assets.
 *
 * This module concentrates the AssetMap-driven Animator preview implementation
 * behind a small interface so Tauri command adapters stay thin.
 */

use crate::common::asset_map::repository::AssetMapRepository;
use crate::common::bundle_file::asset_bundle::AssetBundleLoader;
use crate::common::command_types::{
    AnimatorPreviewGlbResult, DependencyExportResult, PreviewMeshPartRef,
    PreviewSelectionOptions,
};
use crate::common::export::common_types::ExportOptions;
use crate::common::export::export_animation_utils::ExportAnimationUtils;
use crate::common::mesh::animator_preview_hierarchy::{AnimatorMeshRef, AnimatorPreviewHierarchy};
use crate::common::mesh::animator_preview_materials::{
    safe_preview_file_stem, AnimatorPreviewMaterials, AnimatorPreviewMeshRef,
};
use crate::common::mesh::animator_preview_meshes::AnimatorPreviewMeshes;
use crate::common::mesh::animator_preview_skeleton::{
    AnimatorPreviewSkeleton, AnimatorPreviewSkeletonRequest,
};
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::task::task_context::TaskContext;
use crate::common::task::task_logger::TaskLogger;
use crate::exporter::glb_exporter::{GlbExporter, GlbSceneMesh};
use crate::exporter::mesh_exporter::{MeshAttributes, MeshBlendShape, MeshSubMesh};
use crate::exporter::model_context::{GlbSkeleton, ModelContextResolver};
use crate::unity::classes::registry::UnityClassParser;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Instant;

use super::mesh_types::MeshGeometry;

pub struct AnimatorPreviewWorkflow;

struct AnimatorGlbMesh {
    name: String,
    geometry: MeshGeometry,
    skeleton: Option<GlbSkeleton>,
    sub_meshes: Vec<MeshSubMesh>,
    blend_shapes: Vec<MeshBlendShape>,
}

impl AnimatorGlbMesh {
    fn new(name: String, geometry: MeshGeometry, skeleton: Option<GlbSkeleton>) -> Self {
        let sub_meshes = geometry
            .sub_meshes
            .iter()
            .map(|sub_mesh| MeshSubMesh {
                index_start: sub_mesh.index_start,
                index_count: sub_mesh.index_count,
                topology: sub_mesh.topology,
            })
            .collect();
        let blend_shapes = geometry
            .blend_shapes
            .iter()
            .map(|shape| MeshBlendShape {
                name: shape.name.clone(),
                delta_vertices: shape.delta_vertices.clone(),
                delta_normals: shape.delta_normals.clone(),
                delta_tangents: shape.delta_tangents.clone(),
            })
            .collect();
        Self {
            name,
            geometry,
            skeleton,
            sub_meshes,
            blend_shapes,
        }
    }

    fn attrs(&self) -> MeshAttributes<'_> {
        let has_skin = self.skeleton.is_some() && has_complete_skin_data(&self.geometry);
        MeshAttributes {
            vertices: &self.geometry.vertices,
            indices: &self.geometry.indices,
            normals: optional_vertex_f32(&self.geometry.normals, self.geometry.vertex_count, 3),
            uvs: optional_vertex_f32(&self.geometry.uvs, self.geometry.vertex_count, 2),
            tangents: optional_vertex_f32(&self.geometry.tangents, self.geometry.vertex_count, 4),
            colors: optional_vertex_f32(&self.geometry.colors, self.geometry.vertex_count, 4),
            bone_weights: has_skin.then_some(&self.geometry.bone_weights),
            bone_indices: has_skin.then_some(&self.geometry.bone_indices),
            bind_poses: has_skin.then_some(&self.geometry.bind_poses),
            bone_name_hashes: if has_skin && !self.geometry.bone_name_hashes.is_empty() {
                Some(&self.geometry.bone_name_hashes)
            } else {
                None
            },
            root_bone_name_hash: has_skin
                .then_some(self.geometry.root_bone_name_hash)
                .flatten(),
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
}

impl AnimatorPreviewWorkflow {
    pub fn extract_geometry(
        ctx: TaskContext,
        bundle_path: String,
        path_id: String,
        workspace_dir: String,
        cache_dir: Option<String>,
        asset_map_cache_root: Option<String>,
        progress: tauri::ipc::Channel<ProgressPayload>,
    ) -> Result<MeshGeometry, String> {
        extract_animator_geometry_blocking(
            ctx,
            bundle_path,
            path_id,
            workspace_dir,
            cache_dir,
            asset_map_cache_root,
            progress,
        )
    }

    pub fn extract_preview_glb(
        ctx: TaskContext,
        bundle_path: String,
        path_id: String,
        workspace_dir: String,
        cache_dir: String,
        asset_map_cache_root: Option<String>,
        options: PreviewSelectionOptions,
        progress: tauri::ipc::Channel<ProgressPayload>,
    ) -> Result<AnimatorPreviewGlbResult, String> {
        extract_animator_preview_glb_blocking(
            ctx,
            bundle_path,
            path_id,
            workspace_dir,
            cache_dir,
            asset_map_cache_root,
            options,
            progress,
        )
    }

    pub fn export_with_dependencies(
        ctx: TaskContext,
        bundle_path: String,
        path_id: String,
        workspace_dir: String,
        output_dir: String,
        asset_map_cache_root: Option<String>,
        options: ExportOptions,
        progress: tauri::ipc::Channel<ProgressPayload>,
    ) -> Result<DependencyExportResult, String> {
        let task_id = ctx.task_id().to_string();
        TaskLogger::progress(
            &task_id,
            "Export Animator Dependencies",
            "Export Animator",
            1,
            1,
            "Exporting Animator preview GLB and related textures...",
        );
        let preview_options = PreviewSelectionOptions {
            models: true,
            textures: options.include_materials && options.include_textures,
            animations: options.include_animations,
            selected_animation_clip: None,
            eager_animations: options.include_animations,
        };
        let result = extract_animator_preview_glb_blocking(
            ctx,
            bundle_path,
            path_id,
            workspace_dir,
            output_dir,
            asset_map_cache_root,
            preview_options,
            progress,
        )?;
        let texture_paths = result
            .texture_candidates
            .iter()
            .map(|candidate| candidate.png_path.clone())
            .collect::<Vec<_>>();
        TaskLogger::success(
            &task_id,
            "Export Animator Dependencies",
            &format!(
                "Animator export complete: {} ({} vertices, {} triangles, {} materials, {} textures, {} joints, {} animations)",
                result.glb_path,
                result.vertex_count,
                result.triangle_count,
                result.material_count,
                result.texture_count,
                result.skeleton_joint_count,
                result.animation_count
            ),
        );
        Ok(DependencyExportResult {
            output_path: result.glb_path.clone(),
            glb_path: result.glb_path,
            texture_paths,
            material_count: result.material_count,
            texture_count: result.texture_count,
            vertex_count: result.vertex_count,
            triangle_count: result.triangle_count,
        })
    }
}
fn extract_animator_geometry_blocking(
    ctx: TaskContext,
    bundle_path: String,
    path_id: String,
    workspace_dir: String,
    cache_dir: Option<String>,
    asset_map_cache_root: Option<String>,
    progress: tauri::ipc::Channel<ProgressPayload>,
) -> Result<MeshGeometry, String> {
    let task_id = ctx.task_id().to_string();
    let cancel_token = ctx.cancel_token();

    let result = (|| -> Result<MeshGeometry, String> {
        check_animator_preview_cancelled(&task_id, &cancel_token)?;
        let animator_path_id = path_id
            .parse::<i64>()
            .map_err(|e| format!("Invalid path_id: {}", e))?;
        let workspace_path = Path::new(&workspace_dir);
        let cache_root_buf = asset_map_cache_root
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                "AssetMap cache root is required for Animator model preview".to_string()
            })?;
        let cache_root = Some(cache_root_buf.as_path());
        if !AssetMapRepository::exists(workspace_path, cache_root) {
            return Err("AssetMap is required for Animator model preview".to_string());
        }
        let db = AssetMapRepository::open(workspace_path, cache_root)?;

        progress
            .send(ProgressPayload {
                step: "animator".into(),
                message: "Resolving Animator model hierarchy from AssetMap...".into(),
            })
            .ok();
        TaskLogger::progress(
            &task_id,
            "Animator Preview",
            "Resolve Hierarchy",
            1,
            3,
            "Resolving Animator model hierarchy from AssetMap...",
        );

        let root_game_object_id = animator_game_object_id(&bundle_path, animator_path_id)?;
        check_animator_preview_cancelled(&task_id, &cancel_token)?;
        if root_game_object_id == 0 {
            return Err("Animator has no GameObject reference".to_string());
        }

        let (mesh_refs, mesh_ref_diagnostics, has_particle_components) =
            AnimatorPreviewHierarchy::collect_mesh_refs_with_root_fallback(&db, &bundle_path, root_game_object_id);
        AnimatorPreviewHierarchy::emit_mesh_ref_diagnostics(
            &mesh_ref_diagnostics,
            Some(&progress),
            Some(&task_id),
        );
        if mesh_refs.is_empty() {
            if has_particle_components {
                return Err(
                    "Particle System hierarchy has no static Mesh geometry: particles are \
                     generated at runtime (billboards/trails/default quad), so a 3D Mesh \
                     preview is not applicable"
                        .to_string(),
                );
            }
            return Err("No Mesh references found under Animator hierarchy".to_string());
        }

        progress
            .send(ProgressPayload {
                step: "animator".into(),
                message: format!("Found {} Mesh reference(s) under Animator", mesh_refs.len()),
            })
            .ok();

        let cache_path = cache_dir.as_deref().map(Path::new);
        let mut merged = AnimatorPreviewMeshes::empty_geometry();
        let mut parsed_count = 0usize;
        let mut cache_hit_count = 0usize;
        let mut skipped = Vec::new();
        let mut parsed_mesh_cache: HashMap<(String, i64), MeshGeometry> = HashMap::new();
        let mut failed_mesh_cache: HashMap<(String, i64), String> = HashMap::new();

        for (index, mesh_ref) in mesh_refs.iter().enumerate() {
            check_animator_preview_cancelled(&task_id, &cancel_token)?;
            TaskLogger::progress(
                &task_id,
                "Animator Preview",
                "Parse Meshes",
                index + 1,
                mesh_refs.len(),
                &format!(
                    "Parsing Animator Mesh {}/{}: path_id={}",
                    index + 1,
                    mesh_refs.len(),
                    mesh_ref.mesh_path_id
                ),
            );
            progress
                .send(ProgressPayload {
                    step: "mesh".into(),
                    message: format!(
                        "Parsing Animator Mesh {}/{}: path_id={} ({})",
                        index + 1,
                        mesh_refs.len(),
                        mesh_ref.mesh_path_id,
                        Path::new(&mesh_ref.mesh_bundle_path)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or(&mesh_ref.mesh_bundle_path)
                    ),
                })
                .ok();

            let mesh_cache_key = (mesh_ref.mesh_bundle_path.clone(), mesh_ref.mesh_path_id);
            let mesh_result = if let Some(cached) = parsed_mesh_cache.get(&mesh_cache_key) {
                cache_hit_count += 1;
                progress
                    .send(ProgressPayload {
                        step: "mesh".into(),
                        message: format!(
                            "Reusing parsed Animator Mesh: path_id={} ({})",
                            mesh_ref.mesh_path_id,
                            Path::new(&mesh_ref.mesh_bundle_path)
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or(&mesh_ref.mesh_bundle_path)
                        ),
                    })
                    .ok();
                Ok(cached.clone())
            } else if let Some(cached_error) = failed_mesh_cache.get(&mesh_cache_key) {
                cache_hit_count += 1;
                Err(cached_error.clone())
            } else {
                let result = AnimatorPreviewMeshes::parse_preview_mesh(
                    &mesh_ref.mesh_bundle_path,
                    mesh_ref.mesh_path_id,
                    &progress,
                    cache_path,
                );
                match &result {
                    Ok(geometry) if geometry.success && !geometry.vertices.is_empty() => {
                        parsed_mesh_cache.insert(mesh_cache_key.clone(), geometry.clone());
                    }
                    Ok(geometry) => {
                        failed_mesh_cache.insert(mesh_cache_key.clone(), geometry.error.clone());
                    }
                    Err(error) => {
                        failed_mesh_cache.insert(mesh_cache_key.clone(), error.clone());
                    }
                }
                result
            };

            check_animator_preview_cancelled(&task_id, &cancel_token)?;
            match mesh_result {
                Ok(mut geometry) if geometry.success && !geometry.vertices.is_empty() => {
                    if let Err(error) =
                        AnimatorPreviewHierarchy::bake_mesh_transform(&db, mesh_ref, &mut geometry)
                    {
                        skipped.push(format!(
                            "transform {}:{} ({})",
                            mesh_ref.transform_bundle_path, mesh_ref.transform_path_id, error
                        ));
                    }
                    let part = AnimatorPreviewMeshes::mesh_part(mesh_ref, &geometry);
                    AnimatorPreviewMeshes::merge_preview_geometry(&mut merged, geometry);
                    merged.parts.push(part);
                    parsed_count += 1;
                }
                Ok(geometry) => skipped.push(format!(
                    "{}:{} ({})",
                    mesh_ref.mesh_bundle_path, mesh_ref.mesh_path_id, geometry.error
                )),
                Err(error) => skipped.push(format!(
                    "{}:{} ({})",
                    mesh_ref.mesh_bundle_path, mesh_ref.mesh_path_id, error
                )),
            }
        }

        if parsed_count == 0 {
            return Err(format!(
                "Failed to parse every Mesh under Animator: {}",
                skipped.join(" | ")
            ));
        }

        merged.success = true;
        merged.vertex_count = merged.vertices.len() / 3;
        merged.triangle_count = merged.indices.len() / 3;
        if merged.normals.len() < merged.vertex_count * 3 {
            merged.normals.clear();
        }
        if merged.uvs.len() < merged.vertex_count * 2 {
            merged.uvs.clear();
        }
        merged.error = if skipped.is_empty() {
            String::new()
        } else {
            format!(
                "Skipped {} Mesh(es): {}",
                skipped.len(),
                skipped.join(" | ")
            )
        };

        let done_message = format!(
            "Animator preview prepared {} Mesh part(s): {} vertices, {} triangles ({} unique Mesh parsed, {} cache hit).",
            parsed_count,
            merged.vertex_count,
            merged.triangle_count,
            parsed_mesh_cache.len(),
            cache_hit_count
        );
        progress
            .send(ProgressPayload {
                step: "done".into(),
                message: done_message.clone(),
            })
            .ok();
        TaskLogger::progress(
            &task_id,
            "Animator Preview",
            "Complete",
            3,
            3,
            &done_message,
        );

        Ok(merged)
    })();

    if cancel_token.load(Ordering::SeqCst) {
        TaskLogger::warn(&task_id, "Animator Preview", "Animator preview cancelled");
    } else {
        match &result {
            Ok(geometry) => TaskLogger::success(
                &task_id,
                "Animator Preview",
                &format!(
                    "Animator preview complete: {} Mesh part(s), {} vertices, {} triangles",
                    geometry.parts.len(),
                    geometry.vertex_count,
                    geometry.triangle_count
                ),
            ),
            Err(error) => TaskLogger::error(&task_id, "Animator Preview", error),
        }
    }
    result
}

fn extract_animator_preview_glb_blocking(
    ctx: TaskContext,
    bundle_path: String,
    path_id: String,
    workspace_dir: String,
    cache_dir: String,
    asset_map_cache_root: Option<String>,
    options: PreviewSelectionOptions,
    progress: tauri::ipc::Channel<ProgressPayload>,
) -> Result<AnimatorPreviewGlbResult, String> {
    let task_id = ctx.task_id().to_string();
    let cancel_token = ctx.cancel_token();
    let total_started = Instant::now();

    let result = (|| -> Result<AnimatorPreviewGlbResult, String> {
        check_animator_preview_cancelled(&task_id, &cancel_token)?;
        if !options.models {
            return Err("Model preview disabled by user selection".to_string());
        }
        let setup_started = Instant::now();
        let animator_path_id = path_id
            .parse::<i64>()
            .map_err(|e| format!("Invalid path_id: {}", e))?;
        let workspace_path = Path::new(&workspace_dir);
        let cache_root_buf = asset_map_cache_root
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                "AssetMap cache root is required for Animator GLB preview".to_string()
            })?;
        let cache_root = Some(cache_root_buf.as_path());
        if !AssetMapRepository::exists(workspace_path, cache_root) {
            return Err("AssetMap is required for Animator GLB preview".to_string());
        }
        let db = AssetMapRepository::open(workspace_path, cache_root)?;
        emit_animator_preview_log(
            &task_id,
            &progress,
            "animator_glb",
            format!("AssetMap opened in {} ms", elapsed_ms(setup_started)),
        );

        progress
            .send(ProgressPayload {
                step: "animator_glb".into(),
                message: "Resolving Animator hierarchy for GLB preview...".into(),
            })
            .ok();
        TaskLogger::progress(
            &task_id,
            "Animator Preview",
            "Resolve Hierarchy",
            1,
            6,
            "Resolving Animator hierarchy for GLB preview...",
        );

        let root_game_object_id = animator_game_object_id(&bundle_path, animator_path_id)?;
        if root_game_object_id == 0 {
            return Err("Animator has no GameObject reference".to_string());
        }
        let (mesh_refs, mesh_ref_diagnostics, has_particle_components) =
            AnimatorPreviewHierarchy::collect_mesh_refs_with_root_fallback(&db, &bundle_path, root_game_object_id);
        AnimatorPreviewHierarchy::emit_mesh_ref_diagnostics(
            &mesh_ref_diagnostics,
            Some(&progress),
            Some(&task_id),
        );
        if mesh_refs.is_empty() {
            if has_particle_components {
                return Err(
                    "Particle System hierarchy has no static Mesh geometry: particles are \
                     generated at runtime (billboards/trails/default quad), so a 3D Mesh \
                     preview is not applicable"
                        .to_string(),
                );
            }
            return Err("No Mesh references found under Animator hierarchy".to_string());
        }
        emit_animator_preview_log(
            &task_id,
            &progress,
            "animator_glb",
            format!(
                "Animator hierarchy resolved in {} ms: rootGameObject={}, meshRefs={}, diagnostics={}",
                elapsed_ms(setup_started),
                root_game_object_id,
                mesh_refs.len(),
                mesh_ref_diagnostics.len()
            ),
        );

        let animator_name = db
            .find_by_bundle_and_path_id(&bundle_path, animator_path_id)
            .ok()
            .flatten()
            .map(|row| row.asset_name)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| format!("animator_{}", animator_path_id));
        let safe_name =
            safe_preview_file_stem(&animator_name, &format!("animator_{}", animator_path_id));
        let output_root = Path::new(&cache_dir).join("animator_preview_glb");
        let output_folder = output_root.join(&safe_name);
        fs::create_dir_all(&output_folder)
            .map_err(|e| format!("Failed to create Animator preview cache directory: {}", e))?;

        progress
            .send(ProgressPayload {
                step: "mesh".into(),
                message: format!(
                    "Parsing {} Animator Mesh reference(s) with skin data...",
                    mesh_refs.len()
                ),
            })
            .ok();
        TaskLogger::progress(
            &task_id,
            "Animator Preview",
            "Parse Meshes",
            2,
            6,
            &format!(
                "Parsing {} Animator Mesh reference(s) with skin data...",
                mesh_refs.len()
            ),
        );

        let cache_path = Some(Path::new(&cache_dir));
        let mesh_phase_started = Instant::now();
        let parsed_cache = AnimatorPreviewMeshes::parse_full_meshes_parallel(
            &mesh_refs,
            &progress,
            cache_path,
            &task_id,
            &cancel_token,
        )?;

        progress
            .send(ProgressPayload {
                step: "skeleton".into(),
                message: "Resolving per-Mesh skeletons for Animator preview...".into(),
            })
            .ok();
        TaskLogger::progress(
            &task_id,
            "Animator Preview",
            "Resolve Skeleton",
            3,
            6,
            "Resolving per-Mesh skeletons for Animator preview...",
        );

        let skeleton_phase_started = Instant::now();
        let mut merged = AnimatorPreviewMeshes::empty_geometry();
        let mut skipped = Vec::<String>::new();
        let mut accepted_mesh_refs = Vec::<AnimatorMeshRef>::new();
        let mut glb_meshes = Vec::<AnimatorGlbMesh>::new();
        let mut skinned_mesh_count = 0usize;
        let mut static_mesh_count = 0usize;
        let mut total_skeleton_joints = 0usize;

        for mesh_ref in mesh_refs.iter() {
            check_animator_preview_cancelled(&task_id, &cancel_token)?;
            let key = (mesh_ref.mesh_bundle_path.clone(), mesh_ref.mesh_path_id);
            let Some(mut geometry) = parsed_cache.get(&key).cloned() else {
                skipped.push(format!(
                    "{}:{} (parse failed or missing from parse cache)",
                    mesh_ref.mesh_bundle_path, mesh_ref.mesh_path_id
                ));
                continue;
            };

            if !geometry.success || geometry.vertices.is_empty() {
                skipped.push(format!(
                    "{}:{} ({})",
                    mesh_ref.mesh_bundle_path, mesh_ref.mesh_path_id, geometry.error
                ));
                continue;
            }

            let mesh_name = parsed_cache
                .get(&key)
                .and_then(|geometry| geometry.parts.first().map(|part| part.name.clone()))
                .filter(|name| !name.trim().is_empty())
                .or_else(|| {
                    db.find_by_bundle_and_path_id(&mesh_ref.mesh_bundle_path, mesh_ref.mesh_path_id)
                        .ok()
                        .flatten()
                        .map(|row| row.asset_name)
                        .filter(|name| !name.trim().is_empty())
                })
                .unwrap_or_else(|| format!("mesh_{}", mesh_ref.mesh_path_id));

            let skeleton = if has_complete_skin_data(&geometry) {
                match resolve_animator_mesh_skeleton(
                    &db,
                    mesh_ref,
                    &geometry,
                    &progress,
                    &task_id,
                    &cancel_token,
                ) {
                    Ok(skeleton) => {
                        total_skeleton_joints += skeleton.joints.len();
                        skinned_mesh_count += 1;
                        Some(skeleton)
                    }
                    Err(error) => {
                        emit_animator_preview_log(
                            &task_id,
                            &progress,
                            "mesh",
                            format!(
                                "Skin resolve failed for {}:{} ({}); previewing it as static geometry",
                                mesh_ref.mesh_bundle_path, mesh_ref.mesh_path_id, error
                            ),
                        );
                        if let Err(error) = AnimatorPreviewHierarchy::bake_mesh_transform(
                            &db,
                            mesh_ref,
                            &mut geometry,
                        ) {
                            skipped.push(format!(
                                "transform {}:{} after skin fallback ({})",
                                mesh_ref.transform_bundle_path, mesh_ref.transform_path_id, error
                            ));
                        }
                        static_mesh_count += 1;
                        None
                    }
                }
            } else {
                emit_animator_preview_log(
                    &task_id,
                    &progress,
                    "mesh",
                    format!(
                        "{}:{} has no complete skin data (weights={}, indices={}, bindPoses={}); previewing it as static geometry",
                        mesh_ref.mesh_bundle_path,
                        mesh_ref.mesh_path_id,
                        geometry.bone_weights.len() / 4,
                        geometry.bone_indices.len() / 4,
                        geometry.bind_poses.len() / 16
                    ),
                );
                if let Err(error) =
                    AnimatorPreviewHierarchy::bake_mesh_transform(&db, mesh_ref, &mut geometry)
                {
                    skipped.push(format!(
                        "transform {}:{} ({})",
                        mesh_ref.transform_bundle_path, mesh_ref.transform_path_id, error
                    ));
                }
                static_mesh_count += 1;
                None
            };

            glb_meshes.push(AnimatorGlbMesh::new(mesh_name, geometry.clone(), skeleton));

            if merged.vertex_count == 0 {
                merged = geometry.clone();
            } else {
                AnimatorPreviewMeshes::merge_preview_geometry(&mut merged, geometry.clone());
            }
            accepted_mesh_refs.push(mesh_ref.clone());
        }
        let accepted = accepted_mesh_refs.len();
        emit_animator_preview_log(
            &task_id,
            &progress,
            "mesh",
            format!(
                "Mesh merge phase done in {} ms: accepted={}, skipped={}, skinned={}, static={}, vertices={}, triangles={}, perMeshSkeletonJoints={}",
                elapsed_ms(mesh_phase_started),
                accepted,
                skipped.len(),
                skinned_mesh_count,
                static_mesh_count,
                merged.vertex_count,
                merged.triangle_count,
                total_skeleton_joints
            ),
        );

        if accepted == 0 {
            return Err(format!(
                "No Animator Mesh could be parsed for GLB preview. Details: {}",
                skipped.join(" | ")
            ));
        }
        emit_animator_preview_log(
            &task_id,
            &progress,
            "skeleton",
            format!(
                "Skeleton phase done in {} ms: meshes={}, totalJoints={}",
                elapsed_ms(skeleton_phase_started),
                glb_meshes.len(),
                total_skeleton_joints
            ),
        );

        let mut materials = Vec::new();
        let mut texture_candidates = Vec::new();
        if options.textures {
            check_animator_preview_cancelled(&task_id, &cancel_token)?;
            progress
                .send(ProgressPayload {
                    step: "materials".into(),
                    message: "Resolving preview materials and PNG textures...".into(),
                })
                .ok();
            TaskLogger::progress(
                &task_id,
                "Animator Preview",
                "Resolve Materials",
                4,
                6,
                "Resolving preview materials and PNG textures...",
            );

            let materials_phase_started = Instant::now();
            let material_mesh_refs = accepted_mesh_refs
                .iter()
                .map(|mesh_ref| AnimatorPreviewMeshRef {
                    mesh_bundle_path: mesh_ref.mesh_bundle_path.clone(),
                    mesh_path_id: mesh_ref.mesh_path_id,
                    renderer_bundle_path: mesh_ref.renderer_bundle_path.clone(),
                    renderer_path_id: mesh_ref.renderer_path_id,
                })
                .collect::<Vec<_>>();
            let (resolved_materials, texture_refs) =
                AnimatorPreviewMaterials::resolve(&db, &material_mesh_refs, &progress);
            materials = resolved_materials;
            emit_animator_preview_log(
                &task_id,
                &progress,
                "materials",
                format!(
                    "Material lookup done in {} ms: materials={}, textureRefs={}",
                    elapsed_ms(materials_phase_started),
                    materials.len(),
                    texture_refs.len()
                ),
            );
            let texture_phase_started = Instant::now();
            texture_candidates = AnimatorPreviewMaterials::export_textures(
                &mut materials,
                &texture_refs,
                &output_folder,
                &progress,
                &task_id,
                "Animator Preview",
                &cancel_token,
            );
            emit_animator_preview_log(
                &task_id,
                &progress,
                "textures",
                format!(
                    "Texture export done in {} ms: exported={}/{}",
                    elapsed_ms(texture_phase_started),
                    texture_candidates.len(),
                    texture_refs.len()
                ),
            );
        } else {
            emit_animator_preview_log(
                &task_id,
                &progress,
                "textures",
                "Texture preview disabled by user; skipping Material and Texture2D parsing",
            );
        }

        let animation_phase_started = Instant::now();
        let mut animations_by_mesh = vec![Vec::new(); glb_meshes.len()];
        let mut extended_skeletons: Vec<Option<GlbSkeleton>> = vec![None; glb_meshes.len()];
        let mut total_animation_count = 0usize;
        let mut total_channel_count = 0usize;
        if let Some(selected_clip) = options.selected_animation_clip.as_ref() {
            check_animator_preview_cancelled(&task_id, &cancel_token)?;
            let clip_path_id = selected_clip
                .path_id
                .parse::<i64>()
                .map_err(|e| format!("Invalid selected AnimationClip path_id: {}", e))?;
            emit_animator_preview_log(
                &task_id,
                &progress,
                "animation",
                format!(
                    "Parsing selected AnimationClip only: {} path_id={}",
                    selected_clip.bundle_path, clip_path_id
                ),
            );
            let avatar_tos_paths = ExportAnimationUtils::animator_avatar_tos_path_map(
                &db,
                &bundle_path,
                animator_path_id,
            );
            for (mesh_index, glb_mesh) in glb_meshes.iter().enumerate() {
                let Some(skeleton) = glb_mesh.skeleton.as_ref() else {
                    continue;
                };
                let morph_target_names = glb_mesh
                    .blend_shapes
                    .iter()
                    .map(|shape| shape.name.clone())
                    .collect::<Vec<_>>();
                let clip_hash_to_joint = ExportAnimationUtils::build_clip_hash_to_joint(
                    &selected_clip.bundle_path,
                    clip_path_id,
                    skeleton,
                    &avatar_tos_paths,
                );
                let filtered_avatar_tos_paths = clip_hash_to_joint
                    .keys()
                    .filter_map(|hash| avatar_tos_paths.get(hash).cloned().map(|path| (*hash, path)))
                    .collect::<HashMap<_, _>>();
                let mesh_bundle = AssetBundleLoader::load_bundle(Path::new(&bundle_path))
                    .map_err(|e| format!("Failed to load Animator bundle for skeleton extension: {}", e))?;
                let extended_skeleton = ModelContextResolver::extend_skeleton_with_transform_paths(
                    &mesh_bundle,
                    skeleton,
                    &filtered_avatar_tos_paths,
                );
                let parsed =
                    ExportAnimationUtils::extract_selected_preview_animation_with_hash_aliases(
                        &selected_clip.bundle_path,
                        clip_path_id,
                        &extended_skeleton,
                        &morph_target_names,
                        Some(&clip_hash_to_joint),
                        Some(&progress),
                    );
                total_channel_count += parsed
                    .iter()
                    .map(|animation| animation.channels.len())
                    .sum::<usize>();
                total_animation_count += parsed.len();
                animations_by_mesh[mesh_index] = parsed;
                extended_skeletons[mesh_index] = Some(extended_skeleton);
            }
            if total_animation_count == 0 {
                emit_animator_preview_log(
                    &task_id,
                    &progress,
                    "animation",
                    "Selected AnimationClip produced no playable channels for this Animator skeleton",
                );
            }
        } else if options.animations && options.eager_animations {
            check_animator_preview_cancelled(&task_id, &cancel_token)?;
            emit_animator_preview_log(
                &task_id,
                &progress,
                "animation",
                "Resolving all Animator AnimationClips for export...",
            );
            for (mesh_index, glb_mesh) in glb_meshes.iter().enumerate() {
                let Some(skeleton) = glb_mesh.skeleton.as_ref() else {
                    continue;
                };
                let morph_target_names = glb_mesh
                    .blend_shapes
                    .iter()
                    .map(|shape| shape.name.clone())
                    .collect::<Vec<_>>();
                let parsed = ExportAnimationUtils::extract_component_animations(
                    &bundle_path,
                    animator_path_id,
                    Some(&db),
                    skeleton,
                    &glb_mesh.blend_shapes,
                    Some(&progress),
                );
                total_channel_count += parsed
                    .iter()
                    .map(|animation| animation.channels.len())
                    .sum::<usize>();
                total_animation_count += parsed.len();
                animations_by_mesh[mesh_index] = parsed;
                emit_animator_preview_log(
                    &task_id,
                    &progress,
                    "animation",
                    format!(
                        "Animator export animation parse for mesh {}: morphTargets={}, compatibleAnimations={}",
                        mesh_index + 1,
                        morph_target_names.len(),
                        animations_by_mesh[mesh_index].len()
                    ),
                );
            }
        } else if options.animations {
            emit_animator_preview_log(
                &task_id,
                &progress,
                "animation",
                "AnimationClip scan is deferred until a preview drawer clip is selected",
            );
        } else {
            emit_animator_preview_log(
                &task_id,
                &progress,
                "animation",
                "Animation preview disabled by user; skipping AnimationClip scan",
            );
        }
        progress
            .send(ProgressPayload {
                step: "animation".into(),
                message: format!(
                    "Animator animation total: {} clip(s), {} channel(s)",
                    total_animation_count, total_channel_count
                ),
            })
            .ok();
        emit_animator_preview_log(
            &task_id,
            &progress,
            "animation",
            format!(
                "Animation resolve done in {} ms: total clips={}, channels={}",
                elapsed_ms(animation_phase_started),
                total_animation_count,
                total_channel_count
            ),
        );
        let animation_names = animations_by_mesh
            .iter()
            .flat_map(|animations| animations.iter())
            .map(|animation| animation.name.clone())
            .collect::<Vec<_>>();

        progress
            .send(ProgressPayload {
                step: "glb".into(),
                message: "Generating Animator preview GLB...".into(),
            })
            .ok();
        TaskLogger::progress(
            &task_id,
            "Animator Preview",
            "Generate GLB",
            6,
            6,
            "Generating Animator preview GLB...",
        );
        let glb_phase_started = Instant::now();
        let scene_meshes = glb_meshes
            .iter()
            .enumerate()
            .map(|(mesh_index, mesh)| GlbSceneMesh {
                name: &mesh.name,
                attrs: mesh.attrs(),
                skeleton: extended_skeletons[mesh_index]
                    .as_ref()
                    .or(mesh.skeleton.as_ref()),
            })
            .collect::<Vec<_>>();
        let glb_name = if options.selected_animation_clip.is_some() {
            preview_animated_glb_name(&safe_name, &options)
        } else {
            safe_name.clone()
        };
        let glb_path = GlbExporter::export_multi_skinned_scene_data_with_per_mesh_animations_flat(
            &output_folder,
            &glb_name,
            &scene_meshes,
            if materials.is_empty() {
                None
            } else {
                Some(&materials)
            },
            if total_animation_count > 0 {
                Some(animations_by_mesh.as_slice())
            } else {
                None
            },
        )?;
        emit_animator_preview_log(
            &task_id,
            &progress,
            "glb",
            format!(
                "GLB write done in {} ms: {}",
                elapsed_ms(glb_phase_started),
                glb_path
            ),
        );

        if !skipped.is_empty() {
            progress
                .send(ProgressPayload {
                    step: "glb".into(),
                    message: format!(
                        "Animator GLB skipped {} Mesh reference(s): {}",
                        skipped.len(),
                        skipped.join(" | ")
                    ),
                })
                .ok();
        }

        let done_message = format!(
            "Animator GLB preview ready: {} Mesh(es), {} vertices, {} triangles, {} materials, {} PNG textures, {} joints, {} animation(s)",
            accepted,
            merged.vertex_count,
            merged.triangle_count,
            materials.len(),
            texture_candidates.len(),
            total_skeleton_joints,
            total_animation_count
        );
        let done_message = format!("{} (total {} ms)", done_message, elapsed_ms(total_started));
        progress
            .send(ProgressPayload {
                step: "done".into(),
                message: done_message.clone(),
            })
            .ok();
        TaskLogger::success(&task_id, "Animator Preview", &done_message);

        Ok(AnimatorPreviewGlbResult {
            success: true,
            error: if skipped.is_empty() {
                String::new()
            } else {
                format!(
                    "Skipped {} Mesh reference(s): {}",
                    skipped.len(),
                    skipped.join(" | ")
                )
            },
            glb_path,
            mesh_count: accepted,
            material_count: materials.len(),
            texture_count: texture_candidates.len(),
            skeleton_joint_count: total_skeleton_joints,
            animation_count: total_animation_count,
            animation_names,
            mesh_part_names: AnimatorPreviewMeshes::mesh_part_display_names(&merged),
            mesh_parts: accepted_mesh_refs
                .iter()
                .map(|mesh_ref| PreviewMeshPartRef {
                    name: mesh_ref.mesh_name.trim().to_string(),
                    bundle_path: mesh_ref.mesh_bundle_path.clone(),
                    path_id: mesh_ref.mesh_path_id.to_string(),
                    class_name: "Mesh".to_string(),
                })
                .collect(),
            vertex_count: merged.vertex_count,
            triangle_count: merged.triangle_count,
            texture_candidates,
        })
    })();

    if cancel_token.load(Ordering::SeqCst) {
        TaskLogger::warn(
            &task_id,
            "Animator Preview",
            "Animator GLB preview cancelled",
        );
    } else if let Err(error) = &result {
        TaskLogger::error(&task_id, "Animator Preview", error);
    }
    result
}

fn has_complete_skin_data(geometry: &MeshGeometry) -> bool {
    geometry.bone_weights.len() >= geometry.vertex_count * 4
        && geometry.bone_indices.len() >= geometry.vertex_count * 4
        && geometry.bind_poses.len() >= 16
}

fn optional_vertex_f32(values: &[f32], vertex_count: usize, stride: usize) -> Option<&[f32]> {
    (values.len() >= vertex_count * stride).then_some(values)
}

fn resolve_animator_mesh_skeleton(
    db: &crate::common::asset_map::asset_index::AssetDatabase,
    mesh_ref: &AnimatorMeshRef,
    geometry: &MeshGeometry,
    progress: &tauri::ipc::Channel<ProgressPayload>,
    task_id: &str,
    cancel_token: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<GlbSkeleton, String> {
    let mesh_bundle = AssetBundleLoader::load_bundle(Path::new(&mesh_ref.mesh_bundle_path))
        .map_err(|e| format!("Failed to load supplemental skeleton Mesh Bundle: {}", e))?;
    let resolved = AnimatorPreviewSkeleton::resolve(AnimatorPreviewSkeletonRequest {
        mesh_bundle: &mesh_bundle,
        mesh_path_id: mesh_ref.mesh_path_id,
        mesh_bundle_path: &mesh_ref.mesh_bundle_path,
        preferred_renderer: Some((&mesh_ref.renderer_bundle_path, mesh_ref.renderer_path_id)),
        db: Some(db),
        bind_pose_count: geometry.bind_poses.len() / 16,
        bone_name_hashes: &geometry.bone_name_hashes,
        root_bone_name_hash: geometry.root_bone_name_hash,
        progress,
        task_id,
        cancel_token,
    })?;
    resolved
        .map(|(skeleton, _)| skeleton)
        .ok_or_else(|| "No supplemental SkinnedMeshRenderer/Bones found".to_string())
}

fn animator_game_object_id(bundle_path: &str, animator_path_id: i64) -> Result<i64, String> {
    let bundle = AssetBundleLoader::load_bundle_serialized_only(Path::new(bundle_path))
        .map_err(|e| format!("Load Animator bundle failed: {}", e))?;
    let (sf, obj) = bundle
        .assets
        .iter()
        .find_map(|sf| {
            sf.objects
                .iter()
                .find(|obj| obj.path_id == animator_path_id && obj.class_id == 95)
                .map(|obj| (sf, obj))
        })
        .ok_or_else(|| format!("Animator path_id={} not found", animator_path_id))?;
    let animator = UnityClassParser::parse_animator(&sf.inner, &obj.inner)?;
    Ok(animator.component.game_object.path_id)
}

fn check_animator_preview_cancelled(
    task_id: &str,
    cancel_token: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
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

#[allow(dead_code)]
fn bundle_display_name(bundle_path: &str) -> &str {
    Path::new(bundle_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(bundle_path)
}

fn preview_animated_glb_name(safe_name: &str, options: &PreviewSelectionOptions) -> String {
    let clip_suffix = options
        .selected_animation_clip
        .as_ref()
        .map(|clip| clip.path_id.as_str())
        .unwrap_or("selected");
    if options.textures {
        format!("{}_clip_{}", safe_name, clip_suffix)
    } else {
        format!("{}_mesh_clip_{}", safe_name, clip_suffix)
    }
}

fn emit_animator_preview_log(
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

    #[test]
    fn static_animator_mesh_attrs_do_not_export_skin_data() {
        let geometry = minimal_geometry();
        let mesh = AnimatorGlbMesh::new("static_mesh".to_string(), geometry, None);

        let attrs = mesh.attrs();

        assert!(attrs.bone_weights.is_none());
        assert!(attrs.bone_indices.is_none());
        assert!(attrs.bind_poses.is_none());
        assert!(attrs.bone_name_hashes.is_none());
        assert!(attrs.root_bone_name_hash.is_none());
    }

    #[test]
    fn incomplete_animator_skin_data_is_treated_as_static_attrs() {
        let mut geometry = minimal_geometry();
        geometry.bone_indices = vec![0; geometry.vertex_count * 4];
        geometry.bind_poses = vec![0.0; 16];
        let mesh = AnimatorGlbMesh::new(
            "incomplete_skin_mesh".to_string(),
            geometry,
            Some(GlbSkeleton {
                joints: Vec::new(),
                skin_joints: Vec::new(),
                skin_joint_hashes: Vec::new(),
                mesh_parent: None,
                skeleton_root: None,
                roots: Vec::new(),
            }),
        );

        let attrs = mesh.attrs();

        assert!(attrs.bone_weights.is_none());
        assert!(attrs.bone_indices.is_none());
        assert!(attrs.bind_poses.is_none());
    }

    fn minimal_geometry() -> MeshGeometry {
        MeshGeometry {
            vertices: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
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
            indices: vec![0, 1, 2],
            success: true,
            error: String::new(),
            vertex_count: 3,
            triangle_count: 1,
        }
    }
}
