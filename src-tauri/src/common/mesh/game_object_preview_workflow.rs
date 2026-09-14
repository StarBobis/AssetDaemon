/*
 * GameObject-rooted model preview workflow.
 *
 * This follows the AssetStudio model-export shape: start from a GameObject,
 * walk its Transform subtree through AssetMap relations, collect renderable
 * Mesh references, then generate a preview GLB.
 */

use crate::common::asset_map::asset_index::AssetDatabase;
use crate::common::asset_map::repository::AssetMapRepository;
use crate::common::bundle_file::asset_bundle::AssetBundleLoader;
use crate::common::command_types::{
    AnimatorPreviewGlbResult, DependencyExportResult, MeshPreviewTextureCandidate,
    PreviewMeshPartRef,
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
use crate::common::scan::scan_types::{IncrementalPreviewPayload, ProgressPayload};
use crate::common::task::task_context::TaskContext;
use crate::common::task::task_logger::TaskLogger;
#[cfg(test)]
use crate::common::unity_dependency::external_resolver::UnityExternalResolver;
use crate::exporter::glb_exporter::{GlbExporter, GlbSceneMesh};
use crate::exporter::mesh_exporter::{MeshAttributes, MeshBlendShape, MeshSubMesh};
use crate::exporter::model_context::{GlbSkeleton, ModelContextResolver};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::collections::HashSet;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Instant;

use super::mesh_types::MeshGeometry;

pub struct GameObjectPreviewWorkflow;

struct GameObjectGlbMesh {
    name: String,
    geometry: MeshGeometry,
    skeleton: Option<GlbSkeleton>,
    sub_meshes: Vec<MeshSubMesh>,
    blend_shapes: Vec<MeshBlendShape>,
}

impl GameObjectGlbMesh {
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

impl GameObjectPreviewWorkflow {
    pub fn extract_preview_glb(
        ctx: TaskContext,
        bundle_path: String,
        path_id: String,
        workspace_dir: String,
        cache_dir: String,
        asset_map_cache_root: Option<String>,
        options: PreviewSelectionOptions,
        progress: tauri::ipc::Channel<ProgressPayload>,
        incremental: Option<tauri::ipc::Channel<IncrementalPreviewPayload>>,
    ) -> Result<AnimatorPreviewGlbResult, String> {
        extract_game_object_preview_glb_blocking(
            ctx,
            bundle_path,
            path_id,
            workspace_dir,
            cache_dir,
            asset_map_cache_root,
            options,
            progress,
            incremental,
        )
    }

    pub fn export_with_dependencies(
        ctx: TaskContext,
        bundle_path: String,
        path_id: String,
        workspace_dir: String,
        output_dir: String,
        asset_map_cache_root: Option<String>,
        progress: tauri::ipc::Channel<ProgressPayload>,
        options: ExportOptions,
    ) -> Result<DependencyExportResult, String> {
        let task_id = ctx.task_id().to_string();
        TaskLogger::progress(
            &task_id,
            "Export GameObject Dependencies",
            "Export GameObject",
            1,
            1,
            "Exporting GameObject related meshes, skeleton, animations, materials and textures...",
        );
        let preview_options = PreviewSelectionOptions {
            models: true,
            textures: options.include_materials && options.include_textures,
            animations: options.include_animations,
            selected_animation_clip: None,
            eager_animations: options.include_animations,
        };
        let result = extract_game_object_preview_glb_blocking(
            ctx,
            bundle_path,
            path_id,
            workspace_dir,
            output_dir,
            asset_map_cache_root,
            preview_options,
            progress,
            None,
        )?;
        let texture_paths = result
            .texture_candidates
            .iter()
            .map(|candidate| candidate.png_path.clone())
            .collect::<Vec<_>>();
        TaskLogger::success(
            &task_id,
            "Export GameObject Dependencies",
            &format!(
                "GameObject export complete: {} ({} vertices, {} triangles, {} materials, {} textures, {} joints, {} animations)",
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

fn extract_game_object_preview_glb_blocking(
    ctx: TaskContext,
    bundle_path: String,
    path_id: String,
    workspace_dir: String,
    cache_dir: String,
    asset_map_cache_root: Option<String>,
    options: PreviewSelectionOptions,
    progress: tauri::ipc::Channel<ProgressPayload>,
    incremental: Option<tauri::ipc::Channel<IncrementalPreviewPayload>>,
) -> Result<AnimatorPreviewGlbResult, String> {
    let task_id = ctx.task_id().to_string();
    let cancel_token = ctx.cancel_token();
    let total_started = Instant::now();

    let result = (|| -> Result<AnimatorPreviewGlbResult, String> {
        check_cancelled(&task_id, &cancel_token)?;
        if !options.models {
            return Err("Model preview disabled by user selection".to_string());
        }
        let setup_started = Instant::now();
        let game_object_path_id = path_id
            .parse::<i64>()
            .map_err(|e| format!("Invalid path_id: {}", e))?;
        let workspace_path = Path::new(&workspace_dir);
        let cache_root_buf = asset_map_cache_root
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                "AssetMap cache root is required for GameObject model preview".to_string()
            })?;
        let cache_root = Some(cache_root_buf.as_path());
        if !AssetMapRepository::exists(workspace_path, cache_root) {
            return Err("AssetMap is required for GameObject model preview".to_string());
        }
        let db = AssetMapRepository::open(workspace_path, cache_root)?;
        emit_log(
            &task_id,
            &progress,
            "game_object_glb",
            format!("AssetMap opened in {} ms", elapsed_ms(setup_started)),
        );

        progress
            .send(ProgressPayload {
                step: "game_object_glb".into(),
                message: "Resolving GameObject hierarchy for GLB preview...".into(),
            })
            .ok();
        TaskLogger::progress(
            &task_id,
            "GameObject Preview",
            "Resolve Hierarchy",
            1,
            5,
            "Resolving GameObject hierarchy for GLB preview...",
        );

        let game_object_name = db
            .find_by_bundle_and_path_id(&bundle_path, game_object_path_id)
            .ok()
            .flatten()
            .filter(|row| row.class_name == "GameObject")
            .map(|row| row.asset_name)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| format!("game_object_{}", game_object_path_id));

        // Compute cache path early so we can skip expensive work on cache hit.
        let safe_name = safe_preview_file_stem(
            &game_object_name,
            &format!("game_object_{}", game_object_path_id),
        );
        let output_root = Path::new(&cache_dir).join("game_object_preview_glb");
        let output_folder = output_root.join(&safe_name);
        let selected_glb_name = preview_glb_name(&safe_name, &options);
        let glb_path_candidate = output_folder.join(format!("{}.glb", selected_glb_name));
        let cache_meta_path = output_folder.join(preview_cache_file_name(&options));

        // Fast path: return cached preview if GLB and metadata are present.
        if options.selected_animation_clip.is_none()
            && glb_path_candidate.exists()
            && cache_meta_path.exists()
        {
            if let Ok(json) = fs::read_to_string(&cache_meta_path) {
                if let Ok(cached) = serde_json::from_str::<GameObjectPreviewCache>(&json) {
                    if cached.bundle_path == bundle_path
                        && cached.path_id == game_object_path_id
                        && cached.options.matches(&options)
                    {
                        emit_log(
                            &task_id,
                            &progress,
                            "game_object_glb",
                            format!(
                                "Using cached GameObject preview GLB: {}",
                                glb_path_candidate.display()
                            ),
                        );
                        let mesh_part_names =
                            cached_mesh_part_names(&db, &bundle_path, game_object_path_id);
                        let (cached_mesh_refs, _, _) = AnimatorPreviewHierarchy::collect_mesh_refs_with_root_fallback(
                            &db,
                            &bundle_path,
                            game_object_path_id,
                        );
                        let mesh_parts = mesh_parts_from_refs(&cached_mesh_refs);
                        return Ok(AnimatorPreviewGlbResult {
                            success: true,
                            error: String::new(),
                            glb_path: glb_path_candidate.to_string_lossy().to_string(),
                            mesh_count: cached.mesh_count,
                            material_count: cached.material_count,
                            texture_count: cached.texture_count,
                            skeleton_joint_count: cached.skeleton_joint_count,
                            animation_count: cached.animation_count,
                            animation_names: Vec::new(),
                            mesh_part_names,
                            mesh_parts,
                            vertex_count: cached.vertex_count,
                            triangle_count: cached.triangle_count,
                            texture_candidates: cached.texture_candidates,
                        });
                    }
                }
            }
        }

        let (mesh_refs, diagnostics, has_particle_components) =
            AnimatorPreviewHierarchy::collect_mesh_refs_with_root_fallback(&db, &bundle_path, game_object_path_id);
        AnimatorPreviewHierarchy::emit_mesh_ref_diagnostics(
            &diagnostics,
            Some(&progress),
            Some(&task_id),
        );
        if mesh_refs.is_empty() {
            if has_particle_components {
                return Err(
                    "Particle System prefab has no static Mesh geometry: particles are generated \
                     at runtime (billboards/trails/default quad), so a 3D Mesh preview is not \
                     applicable"
                        .to_string(),
                );
            }
            return Err("No Mesh references found under GameObject hierarchy".to_string());
        }
        emit_log(
            &task_id,
            &progress,
            "game_object_glb",
            format!(
                "GameObject hierarchy resolved in {} ms: meshRefs={}, diagnostics={}",
                elapsed_ms(setup_started),
                mesh_refs.len(),
                diagnostics.len()
            ),
        );

        fs::create_dir_all(&output_folder)
            .map_err(|e| format!("Failed to create GameObject preview cache directory: {}", e))?;

        progress
            .send(ProgressPayload {
                step: "mesh".into(),
                message: format!(
                    "Parsing {} GameObject Mesh reference(s)...",
                    mesh_refs.len()
                ),
            })
            .ok();
        TaskLogger::progress(
            &task_id,
            "GameObject Preview",
            "Parse Meshes",
            2,
            5,
            &format!(
                "Parsing {} GameObject Mesh reference(s)...",
                mesh_refs.len()
            ),
        );

        let cache_path = Some(Path::new(&cache_dir));
        let mesh_phase_started = Instant::now();
        let mut merged = AnimatorPreviewMeshes::empty_geometry();
        let mut accepted_mesh_refs = Vec::<AnimatorMeshRef>::new();
        let mut skipped = Vec::<String>::new();
        let mut skinned_preview = false;
        let mut glb_meshes = Vec::<GameObjectGlbMesh>::new();
        let mut total_skeleton_joints = 0usize;
        for (index, mesh_ref) in mesh_refs.iter().enumerate() {
            check_cancelled(&task_id, &cancel_token)?;
            progress
                .send(ProgressPayload {
                    step: "mesh".into(),
                    message: format!(
                        "Parsing GameObject Mesh {}/{}: path_id={} ({})",
                        index + 1,
                        mesh_refs.len(),
                        mesh_ref.mesh_path_id,
                        bundle_display_name(&mesh_ref.mesh_bundle_path)
                    ),
                })
                .ok();
            let result = AnimatorPreviewMeshes::parse_full_mesh(
                &mesh_ref.mesh_bundle_path,
                mesh_ref.mesh_path_id,
                &progress,
                cache_path,
            );
            match result {
                Ok(mut geometry) if geometry.success && !geometry.vertices.is_empty() => {
                    let part = AnimatorPreviewMeshes::mesh_part(mesh_ref, &geometry);
                    let mut mesh_skeleton = None::<GlbSkeleton>;
                    if has_complete_skin_data(&geometry) {
                        match resolve_game_object_skeleton(
                            &db,
                            mesh_ref,
                            &geometry,
                            &progress,
                            &task_id,
                            &cancel_token,
                        ) {
                            Ok(resolved_skeleton) => {
                                total_skeleton_joints += resolved_skeleton.joints.len();
                                mesh_skeleton = Some(resolved_skeleton.clone());
                                skinned_preview = true;
                                AnimatorPreviewMeshes::merge_glb_geometry(
                                    &mut merged,
                                    geometry.clone(),
                                )?;
                            }
                            Err(error) => {
                                emit_log(
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
                                        mesh_ref.transform_bundle_path,
                                        mesh_ref.transform_path_id,
                                        error
                                    ));
                                }
                                AnimatorPreviewMeshes::merge_preview_geometry(
                                    &mut merged,
                                    geometry.clone(),
                                );
                            }
                        }
                    } else {
                        if let Err(error) = AnimatorPreviewHierarchy::bake_mesh_transform(
                            &db,
                            mesh_ref,
                            &mut geometry,
                        ) {
                            skipped.push(format!(
                                "transform {}:{} ({})",
                                mesh_ref.transform_bundle_path, mesh_ref.transform_path_id, error
                            ));
                        }
                        AnimatorPreviewMeshes::merge_preview_geometry(
                            &mut merged,
                            geometry.clone(),
                        );
                    }
                    merged.parts.push(part);
                    glb_meshes.push(GameObjectGlbMesh::new(
                        mesh_ref.mesh_name.clone(),
                        geometry,
                        mesh_skeleton,
                    ));
                    accepted_mesh_refs.push(mesh_ref.clone());
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

        if accepted_mesh_refs.is_empty() {
            return Err(format!(
                "No GameObject Mesh could be parsed. Details: {}",
                skipped.join(" | ")
            ));
        }
        normalize_merged_geometry(&mut merged);
        emit_log(
            &task_id,
            &progress,
            "mesh",
            format!(
                "Mesh merge phase done in {} ms: accepted={}, skipped={}, skinned={}, vertices={}, triangles={}",
                elapsed_ms(mesh_phase_started),
                accepted_mesh_refs.len(),
                skipped.len(),
                skinned_preview,
                merged.vertex_count,
                merged.triangle_count
            ),
        );

        let scene_meshes = glb_meshes
            .iter()
            .map(|mesh| GlbSceneMesh {
                name: &mesh.name,
                attrs: mesh.attrs(),
                skeleton: mesh.skeleton.as_ref(),
            })
            .collect::<Vec<_>>();
        let mesh_part_names = mesh_part_names_from_refs(&accepted_mesh_refs);
        let mesh_parts = mesh_parts_from_refs(&accepted_mesh_refs);
        let mesh_only_glb_started = Instant::now();
        let mesh_only_glb_name = format!("{}_mesh", safe_name);
        let mesh_only_glb_path = GlbExporter::export_multi_skinned_scene_data_flat(
            &output_folder,
            &mesh_only_glb_name,
            &scene_meshes,
            None,
        )?;
        emit_log(
            &task_id,
            &progress,
            "glb",
            format!(
                "Mesh-only GLB write done in {} ms: {}",
                elapsed_ms(mesh_only_glb_started),
                mesh_only_glb_path
            ),
        );

        // Incremental: mesh phase done and immediately renderable.
        send_incremental(
            &incremental,
            "mesh",
            Some(mesh_only_glb_path),
            Some(merged.vertex_count),
            Some(merged.triangle_count),
            Some(accepted_mesh_refs.len()),
            None,
            None,
            Some(total_skeleton_joints),
            None,
            None,
            Some(mesh_part_names.clone()),
            Some(mesh_parts.clone()),
            None,
        );

        let mut materials = Vec::new();
        let mut texture_candidates = Vec::new();
        if options.textures {
            check_cancelled(&task_id, &cancel_token)?;
            progress
                .send(ProgressPayload {
                    step: "materials".into(),
                    message: "Resolving GameObject preview materials and PNG textures...".into(),
                })
                .ok();
            TaskLogger::progress(
                &task_id,
                "GameObject Preview",
                "Resolve Materials",
                3,
                5,
                "Resolving GameObject preview materials and PNG textures...",
            );

            let material_mesh_refs = accepted_mesh_refs
                .iter()
                .map(|mesh_ref| AnimatorPreviewMeshRef {
                    mesh_bundle_path: mesh_ref.mesh_bundle_path.clone(),
                    mesh_path_id: mesh_ref.mesh_path_id,
                    renderer_bundle_path: mesh_ref.renderer_bundle_path.clone(),
                    renderer_path_id: mesh_ref.renderer_path_id,
                })
                .collect::<Vec<_>>();
            let materials_phase_started = Instant::now();
            let (resolved_materials, texture_refs) =
                AnimatorPreviewMaterials::resolve(&db, &material_mesh_refs, &progress);
            materials = resolved_materials;
            emit_log(
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
                "GameObject Preview",
                &cancel_token,
            );
            emit_log(
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
            check_cancelled(&task_id, &cancel_token)?;
        } else {
            emit_log(
                &task_id,
                &progress,
                "textures",
                "Texture preview disabled by user; skipping Material and Texture2D parsing"
                    .to_string(),
            );
        }

        // ---- Generate GLB early (before animation scan) so the viewer loads immediately ----
        let glb_phase_started = Instant::now();
        let textured_glb_name = preview_glb_name(&safe_name, &options);
        let glb_path = GlbExporter::export_multi_skinned_scene_data_flat(
            &output_folder,
            &textured_glb_name,
            &scene_meshes,
            if materials.is_empty() {
                None
            } else {
                Some(&materials)
            },
        )?;
        emit_log(
            &task_id,
            &progress,
            "glb",
            format!(
                "Textured GLB write done in {} ms: {}",
                elapsed_ms(glb_phase_started),
                glb_path
            ),
        );

        // Incremental: textures/materials phase done with a textured GLB.
        send_incremental(
            &incremental,
            "textures",
            Some(glb_path.clone()),
            Some(merged.vertex_count),
            Some(merged.triangle_count),
            Some(accepted_mesh_refs.len()),
            Some(materials.len()),
            Some(texture_candidates.len()),
            Some(total_skeleton_joints),
            None,
            None,
            Some(mesh_part_names.clone()),
            Some(mesh_parts.clone()),
            Some(texture_candidates.clone()),
        );
        let latest_glb_path = glb_path.clone();

        // ---- Resolve only a user-selected AnimationClip after the base preview is visible ----
        let animation_phase_started = Instant::now();
        let mut animations_by_mesh = vec![Vec::new(); glb_meshes.len()];
        let mut total_animation_count = 0usize;
        let mut latest_glb_path = latest_glb_path;
        if let Some(selected_clip) = options.selected_animation_clip.as_ref() {
            check_cancelled(&task_id, &cancel_token)?;
            let clip_path_id = selected_clip
                .path_id
                .parse::<i64>()
                .map_err(|e| format!("Invalid selected AnimationClip path_id: {}", e))?;
            emit_log(
                &task_id,
                &progress,
                "animation",
                format!(
                    "Parsing selected AnimationClip only: {} path_id={}",
                    selected_clip.bundle_path, clip_path_id
                ),
            );
            let avatar_tos_paths = ExportAnimationUtils::preview_avatar_tos_path_map(
                &db,
                "GameObject",
                &bundle_path,
                game_object_path_id,
            );
            let mut extended_skeletons: Vec<Option<GlbSkeleton>> = vec![None; glb_meshes.len()];
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
                    .map_err(|e| format!("Failed to load GameObject bundle for skeleton extension: {}", e))?;
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
                total_animation_count += parsed.len();
                animations_by_mesh[mesh_index] = parsed;
                extended_skeletons[mesh_index] = Some(extended_skeleton);
            }
            if total_animation_count == 0 {
                emit_log(
                    &task_id,
                    &progress,
                    "animation",
                    "Selected AnimationClip produced no playable channels for this GameObject skeleton"
                        .to_string(),
                );
            }
            let animated_scene_meshes = glb_meshes
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
            let animated_glb_name = preview_animated_glb_name(&safe_name, &options);
            latest_glb_path =
                GlbExporter::export_multi_skinned_scene_data_with_per_mesh_animations_flat(
                    &output_folder,
                    &animated_glb_name,
                    &animated_scene_meshes,
                    if materials.is_empty() {
                        None
                    } else {
                        Some(&materials)
                    },
                    Some(animations_by_mesh.as_slice()),
                )?;
            emit_log(
                &task_id,
                &progress,
                "animation",
                format!(
                    "Selected AnimationClip GLB write done in {} ms: {}",
                    elapsed_ms(animation_phase_started),
                    latest_glb_path
                ),
            );
        } else if options.animations {
            emit_log(
                &task_id,
                &progress,
                "animation",
                "AnimationClip scan is deferred until a preview drawer clip is selected"
                    .to_string(),
            );
        } else {
            emit_log(
                &task_id,
                &progress,
                "animation",
                "Animation preview disabled by user; skipping AnimationClip scan".to_string(),
            );
        }
        let animation_names = animations_by_mesh
            .iter()
            .flat_map(|animations| animations.iter())
            .map(|animation| animation.name.clone())
            .collect::<Vec<_>>();
        send_incremental(
            &incremental,
            "complete",
            Some(latest_glb_path.clone()),
            Some(merged.vertex_count),
            Some(merged.triangle_count),
            Some(accepted_mesh_refs.len()),
            Some(materials.len()),
            Some(texture_candidates.len()),
            Some(total_skeleton_joints),
            Some(total_animation_count),
            Some(animation_names.clone()),
            Some(mesh_part_names.clone()),
            Some(mesh_parts.clone()),
            Some(texture_candidates.clone()),
        );

        if !skipped.is_empty() {
            progress
                .send(ProgressPayload {
                    step: "glb".into(),
                    message: format!(
                        "GameObject GLB skipped {} Mesh reference(s): {}",
                        skipped.len(),
                        skipped.join(" | ")
                    ),
                })
                .ok();
        }

        let done_message = format!(
            "GameObject GLB preview ready: {} Mesh(es), {} vertices, {} triangles, {} materials, {} PNG textures, {} joints, {} animation(s)",
            accepted_mesh_refs.len(),
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
        TaskLogger::success(&task_id, "GameObject Preview", &done_message);

        // Write persistent cache metadata alongside the GLB
        if options.selected_animation_clip.is_none() {
            let cache_meta = GameObjectPreviewCache {
                bundle_path: bundle_path.clone(),
                path_id: game_object_path_id,
                options: options.clone().into(),
                mesh_count: accepted_mesh_refs.len(),
                material_count: materials.len(),
                texture_count: texture_candidates.len(),
                skeleton_joint_count: total_skeleton_joints,
                animation_count: total_animation_count,
                vertex_count: merged.vertex_count,
                triangle_count: merged.triangle_count,
                texture_candidates: texture_candidates.clone(),
            };
            let cache_path = output_folder.join(preview_cache_file_name(&options));
            if let Ok(json) = serde_json::to_string(&cache_meta) {
                let _ = fs::write(&cache_path, &json);
            }
        }

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
            glb_path: latest_glb_path,
            mesh_count: accepted_mesh_refs.len(),
            material_count: materials.len(),
            texture_count: texture_candidates.len(),
            skeleton_joint_count: total_skeleton_joints,
            animation_count: total_animation_count,
            animation_names,
            mesh_part_names,
            mesh_parts,
            vertex_count: merged.vertex_count,
            triangle_count: merged.triangle_count,
            texture_candidates,
        })
    })();

    if cancel_token.load(Ordering::SeqCst) {
        TaskLogger::warn(
            &task_id,
            "GameObject Preview",
            "GameObject GLB preview cancelled",
        );
    } else if let Err(error) = &result {
        TaskLogger::error(&task_id, "GameObject Preview", error);
    }
    result
}

fn normalize_merged_geometry(geometry: &mut MeshGeometry) {
    geometry.success = true;
    geometry.vertex_count = geometry.vertices.len() / 3;
    geometry.triangle_count = geometry.indices.len() / 3;
    if geometry.normals.len() < geometry.vertex_count * 3 {
        geometry.normals.clear();
    }
    if geometry.uvs.len() < geometry.vertex_count * 2 {
        geometry.uvs.clear();
    }
}

fn has_complete_skin_data(geometry: &MeshGeometry) -> bool {
    geometry.bone_weights.len() >= geometry.vertex_count * 4
        && geometry.bone_indices.len() >= geometry.vertex_count * 4
        && geometry.bind_poses.len() >= 16
}

fn optional_vertex_f32(values: &[f32], vertex_count: usize, stride: usize) -> Option<&[f32]> {
    (values.len() >= vertex_count * stride).then_some(values)
}

fn resolve_game_object_skeleton(
    db: &AssetDatabase,
    primary_mesh_ref: &AnimatorMeshRef,
    merged: &MeshGeometry,
    progress: &tauri::ipc::Channel<ProgressPayload>,
    task_id: &str,
    cancel_token: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<GlbSkeleton, String> {
    let skeleton_phase_started = Instant::now();
    emit_log(
        task_id,
        progress,
        "skeleton",
        format!(
            "Skeleton primary Mesh: {} path_id={} renderer={} path_id={}",
            bundle_display_name(&primary_mesh_ref.mesh_bundle_path),
            primary_mesh_ref.mesh_path_id,
            bundle_display_name(&primary_mesh_ref.renderer_bundle_path),
            primary_mesh_ref.renderer_path_id
        ),
    );
    let primary_bundle =
        AssetBundleLoader::load_bundle(Path::new(&primary_mesh_ref.mesh_bundle_path))
            .map_err(|e| format!("Failed to load skeleton source Bundle: {}", e))?;
    let resolved_skeleton = AnimatorPreviewSkeleton::resolve(AnimatorPreviewSkeletonRequest {
        mesh_bundle: &primary_bundle,
        mesh_path_id: primary_mesh_ref.mesh_path_id,
        mesh_bundle_path: &primary_mesh_ref.mesh_bundle_path,
        preferred_renderer: Some((
            &primary_mesh_ref.renderer_bundle_path,
            primary_mesh_ref.renderer_path_id,
        )),
        db: Some(db),
        bind_pose_count: merged.bind_poses.len() / 16,
        bone_name_hashes: &merged.bone_name_hashes,
        root_bone_name_hash: merged.root_bone_name_hash,
        progress,
        task_id,
        cancel_token,
    })?;
    let (skeleton, skeleton_bundle_path) = resolved_skeleton.ok_or_else(|| {
        "No SkinnedMeshRenderer/Bones found for GameObject dependency export".to_string()
    })?;
    emit_log(
        task_id,
        progress,
        "skeleton",
        format!(
            "Skeleton phase done in {} ms: joints={}, skinJoints={}, source={}",
            elapsed_ms(skeleton_phase_started),
            skeleton.joints.len(),
            skeleton.skin_joints.len(),
            skeleton_bundle_path
                .as_deref()
                .map(bundle_display_name)
                .unwrap_or("local mesh bundle")
        ),
    );
    Ok(skeleton)
}

#[cfg(test)]
fn game_object_animation_roots(
    db: &AssetDatabase,
    root_bundle_path: &str,
    root_game_object_path_id: i64,
    mesh_refs: &[AnimatorMeshRef],
) -> Vec<(String, i64)> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    push_animation_game_object_root(
        db,
        &mut result,
        &mut seen,
        root_bundle_path,
        root_game_object_path_id,
    );
    for mesh_ref in mesh_refs {
        push_animation_transform_ancestors(
            db,
            &mut result,
            &mut seen,
            &mesh_ref.transform_bundle_path,
            mesh_ref.transform_path_id,
        );
    }
    result
}

#[cfg(test)]
fn push_animation_game_object_root(
    db: &AssetDatabase,
    result: &mut Vec<(String, i64)>,
    seen: &mut HashSet<(String, i64)>,
    bundle_path: &str,
    game_object_path_id: i64,
) {
    if game_object_path_id == 0 {
        return;
    }
    let key = (bundle_path.to_string(), game_object_path_id);
    if seen.insert(key.clone()) {
        result.push(key);
    }
    for relation in db
        .find_relations_by_bundle_and_target(
            "transform_gameobject",
            bundle_path,
            game_object_path_id,
        )
        .unwrap_or_default()
    {
        push_animation_transform_ancestors(
            db,
            result,
            seen,
            &relation.bundle_path,
            relation.source_path_id,
        );
    }
}

#[cfg(test)]
fn push_animation_transform_ancestors(
    db: &AssetDatabase,
    result: &mut Vec<(String, i64)>,
    seen: &mut HashSet<(String, i64)>,
    bundle_path: &str,
    transform_path_id: i64,
) {
    if transform_path_id == 0 {
        return;
    }

    let mut current_bundle = bundle_path.to_string();
    let mut current_transform = transform_path_id;
    let mut seen_transforms = HashSet::new();
    while current_transform != 0
        && seen_transforms.insert((current_bundle.clone(), current_transform))
    {
        for relation in db
            .find_relations_by_bundle_and_source(
                "transform_gameobject",
                &current_bundle,
                current_transform,
            )
            .unwrap_or_default()
        {
            let key = (relation.bundle_path.clone(), relation.target_path_id);
            if relation.target_path_id != 0 && seen.insert(key.clone()) {
                result.push(key);
            }
        }

        let Some(parent_relation) = db
            .find_relations_by_bundle_and_source(
                "transform_parent",
                &current_bundle,
                current_transform,
            )
            .unwrap_or_default()
            .into_iter()
            .find(|relation| relation.target_path_id != 0)
        else {
            break;
        };
        current_bundle = UnityExternalResolver::resolve_relation_row(db, &parent_relation);
        current_transform = parent_relation.target_path_id;
    }
}

fn check_cancelled(
    task_id: &str,
    cancel_token: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    if cancel_token.load(Ordering::SeqCst) {
        TaskLogger::warn(
            task_id,
            "GameObject Preview",
            "GameObject preview cancelled",
        );
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
    TaskLogger::info(task_id, "GameObject Preview", &message);
}

/// Send an incremental preview update through the optional channel.
fn send_incremental(
    incremental: &Option<tauri::ipc::Channel<IncrementalPreviewPayload>>,
    phase: &str,
    glb_path: Option<String>,
    vertex_count: Option<usize>,
    triangle_count: Option<usize>,
    mesh_count: Option<usize>,
    material_count: Option<usize>,
    texture_count: Option<usize>,
    skeleton_joint_count: Option<usize>,
    animation_count: Option<usize>,
    animation_names: Option<Vec<String>>,
    mesh_part_names: Option<Vec<String>>,
    mesh_parts: Option<Vec<PreviewMeshPartRef>>,
    texture_candidates: Option<Vec<MeshPreviewTextureCandidate>>,
) {
    if let Some(ch) = incremental {
        let _ = ch.send(IncrementalPreviewPayload {
            phase: phase.to_string(),
            glb_path,
            vertex_count,
            triangle_count,
            mesh_count,
            material_count,
            texture_count,
            skeleton_joint_count,
            animation_count,
            animation_names,
            mesh_part_names,
            mesh_parts,
            texture_candidates,
        });
    }
}

fn cached_mesh_part_names(
    db: &AssetDatabase,
    bundle_path: &str,
    game_object_path_id: i64,
) -> Vec<String> {
    let (mesh_refs, _, _) =
        AnimatorPreviewHierarchy::collect_mesh_refs_with_root_fallback(db, bundle_path, game_object_path_id);
    mesh_part_names_from_refs(&mesh_refs)
}

fn mesh_part_names_from_refs(mesh_refs: &[AnimatorMeshRef]) -> Vec<String> {
    mesh_refs
        .iter()
        .map(|mesh_ref| mesh_ref.mesh_name.trim())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn mesh_parts_from_refs(mesh_refs: &[AnimatorMeshRef]) -> Vec<PreviewMeshPartRef> {
    mesh_refs
        .iter()
        .map(|mesh_ref| PreviewMeshPartRef {
            name: mesh_ref.mesh_name.trim().to_string(),
            bundle_path: mesh_ref.mesh_bundle_path.clone(),
            path_id: mesh_ref.mesh_path_id.to_string(),
            class_name: "Mesh".to_string(),
        })
        .collect()
}

/// Persistent cache metadata for GameObject preview GLB results.
/// Written alongside the GLB file so subsequent previews skip re-extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GameObjectPreviewCache {
    bundle_path: String,
    path_id: i64,
    #[serde(default)]
    options: CachedPreviewSelectionOptions,
    mesh_count: usize,
    material_count: usize,
    texture_count: usize,
    skeleton_joint_count: usize,
    animation_count: usize,
    vertex_count: usize,
    triangle_count: usize,
    #[serde(default)]
    texture_candidates: Vec<MeshPreviewTextureCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedPreviewSelectionOptions {
    #[serde(default = "default_cached_preview_option_enabled")]
    models: bool,
    #[serde(default = "default_cached_preview_option_enabled")]
    textures: bool,
    #[serde(default = "default_cached_preview_option_enabled")]
    animations: bool,
}

fn default_cached_preview_option_enabled() -> bool {
    true
}

impl Default for CachedPreviewSelectionOptions {
    fn default() -> Self {
        Self {
            models: true,
            textures: true,
            animations: true,
        }
    }
}

impl CachedPreviewSelectionOptions {
    fn matches(&self, options: &PreviewSelectionOptions) -> bool {
        self.models == options.models
            && self.textures == options.textures
            && self.animations == options.animations
    }
}

impl From<PreviewSelectionOptions> for CachedPreviewSelectionOptions {
    fn from(value: PreviewSelectionOptions) -> Self {
        Self {
            models: value.models,
            textures: value.textures,
            animations: value.animations,
        }
    }
}

fn preview_glb_name(safe_name: &str, options: &PreviewSelectionOptions) -> String {
    if options.textures {
        format!("{}_textures", safe_name)
    } else {
        format!("{}_mesh", safe_name)
    }
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

fn preview_cache_file_name(options: &PreviewSelectionOptions) -> String {
    format!(
        "preview_cache_v5_m{}_t{}_a{}.json",
        if options.models { 1 } else { 0 },
        if options.textures { 1 } else { 0 },
        if options.animations { 1 } else { 0 },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::asset_map::asset_index::{MapBundleWriteRows, RelationWriteRow};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn game_object_animation_roots_include_parent_game_objects_for_animators() {
        let workspace = temp_workspace("game-object-animation-parent-roots");
        std::fs::create_dir_all(&workspace).unwrap();

        let db = AssetDatabase::open(&workspace).unwrap();
        let bundle = workspace.join("model.bundle").to_string_lossy().to_string();

        db.insert_many_map_bundle_write_rows(&[MapBundleWriteRows {
            bundle_path: bundle.clone(),
            md5: "model".to_string(),
            file_size: 1,
            modified_ms: 1,
            unity_version: "2019.4".to_string(),
            asset_count: 6,
            assets: Vec::new(),
            containers: Vec::new(),
            externals: Vec::new(),
            internal_names: Vec::new(),
            relations: vec![
                relation(&bundle, "transform_gameobject", 100, 10, "m_GameObject"),
                relation(&bundle, "transform_gameobject", 200, 20, "m_GameObject"),
                relation(&bundle, "transform_parent", 200, 100, "m_Father"),
                relation(&bundle, "transform_gameobject", 300, 30, "m_GameObject"),
                relation(&bundle, "transform_parent", 300, 200, "m_Father"),
            ],
        }])
        .unwrap();

        let mesh_refs = vec![AnimatorMeshRef {
            mesh_bundle_path: bundle.clone(),
            mesh_path_id: 1,
            mesh_name: "mesh".to_string(),
            renderer_bundle_path: bundle.clone(),
            renderer_path_id: 2,
            transform_bundle_path: bundle.clone(),
            transform_path_id: 300,
        }];

        assert_eq!(
            game_object_animation_roots(&db, &bundle, 30, &mesh_refs),
            vec![
                (bundle.clone(), 30),
                (bundle.clone(), 20),
                (bundle.clone(), 10)
            ]
        );

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn mesh_part_names_from_refs_uses_real_mesh_names() {
        let refs = vec![mesh_ref("body_L3"), mesh_ref("hair_01"), mesh_ref(" ")];

        assert_eq!(
            mesh_part_names_from_refs(&refs),
            vec!["body_L3".to_string(), "hair_01".to_string()]
        );
    }

    fn mesh_ref(mesh_name: &str) -> AnimatorMeshRef {
        AnimatorMeshRef {
            mesh_bundle_path: "bundle".to_string(),
            mesh_path_id: 1,
            mesh_name: mesh_name.to_string(),
            renderer_bundle_path: "bundle".to_string(),
            renderer_path_id: 2,
            transform_bundle_path: "bundle".to_string(),
            transform_path_id: 3,
        }
    }

    fn relation(
        bundle_path: &str,
        relation_type: &str,
        source_path_id: i64,
        target_path_id: i64,
        field_path: &str,
    ) -> RelationWriteRow {
        RelationWriteRow {
            bundle_path: bundle_path.to_string(),
            relation_type: relation_type.to_string(),
            source_path_id,
            target_path_id,
            source_name: String::new(),
            target_name: String::new(),
            file_id: 0,
            field_path: field_path.to_string(),
            target_bundle_path: String::new(),
        }
    }

    fn temp_workspace(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("assetfinder-{}-{}", name, nanos))
    }
}
