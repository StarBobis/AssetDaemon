/*
 * export_service.rs Batch export orchestration service.
 *
 * Responsible for:
 *  - Receiving ExportJob, iterating through asset list
 *  - Routing to the correct exporter by class_name
 *  - Sending progress events via tauri::ipc::Channel
 *  - Supporting cancellation (AtomicBool)
 *  - Generating export report
 *
 * Phase 1: Use existing exporters (TextureExporter::decode_pixels, GlbExporter),
 *           New types (AudioClip, Font, etc.) use raw dump.
 * Phase 2: Extract TextureService, add multi-format and OBJ support.
 *
 * Follows Soul.md convention: struct + impl organization, no free functions.
 */

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tauri::ipc::Channel;

use crate::common::asset_map::repository::AssetMapRepository;
use crate::common::bundle_file::asset_bundle::{AssetBundle, AssetBundleLoader};
use crate::common::command_types::DependencyExportResult;
use crate::common::export::common_types::{
    ExportAssetRef, ExportFormat, ExportFormatResolver, ExportGroupBy, ExportJob, ExportOptions,
    ExportProgress, ExportReport, ExportResult,
};
use crate::common::export::export_animation_utils::ExportAnimationUtils;
use crate::common::mesh::animator_preview_materials::{
    AnimatorPreviewMaterials, AnimatorPreviewMeshRef,
};
use crate::common::mesh::animator_preview_workflow::AnimatorPreviewWorkflow;
use crate::common::mesh::game_object_preview_workflow::GameObjectPreviewWorkflow;
use crate::common::mesh::mesh_service::MeshService;
use crate::common::preview::asset_preview_service::AssetPreviewService;
use crate::common::serialized_file::object_reader::ObjectReader;
use crate::common::serialized_file::serialized_file;
use crate::common::task::task_context::TaskContext;
use crate::common::task::task_logger::TaskLogger;
use crate::common::task::task_manager::TaskManager;
use crate::exporter::animation_clip_exporter::GlbAnimation;
use crate::exporter::audio_exporter::AudioExporter;
use crate::exporter::dds_exporter::DDSExporter;
use crate::exporter::font_exporter::FontExporter;
use crate::exporter::glb_exporter::GlbExporter;
use crate::exporter::material_info::MaterialInfo;
use crate::exporter::mesh_exporter::{MeshAttributes, MeshBlendShape, MeshExporter, MeshSubMesh};
use crate::exporter::model_context::{GlbSkeleton, ModelContextResolver};
use crate::exporter::mono_behaviour_exporter::MonoBehaviourExporter;
use crate::exporter::raw_exporter::RawExporter;
use crate::exporter::text_asset_exporter::TextAssetExporter;
use crate::service::sprite_preview_service::SpritePreviewService;
use crate::unity::classes::registry::UnityClassObject;
use crate::utils::time_utils::TimeUtils;

pub struct ExportService;

impl ExportService {
    pub(crate) fn safe_asset_file_name(name: &str, fallback: &str, extension: &str) -> String {
        let mut out = name
            .chars()
            .map(|ch| match ch {
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
                ch if ch.is_control() => '_',
                ch => ch,
            })
            .collect::<String>();
        out = out.trim().trim_matches('.').to_string();
        if out.is_empty() {
            out = fallback.to_string();
        }
        let ext = extension.trim_start_matches('.');
        if !out.to_lowercase().ends_with(&format!(".{}", ext)) {
            out.push('.');
            out.push_str(ext);
        }
        out
    }

    pub(crate) fn safe_name_part(name: &str) -> String {
        name.chars()
            .map(|ch| match ch {
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
                ch if ch.is_control() => '_',
                ch => ch,
            })
            .collect::<String>()
            .trim()
            .trim_matches('.')
            .trim_matches('_')
            .to_string()
    }

    pub(crate) fn safe_path_segments(path: &str) -> Vec<String> {
        Path::new(path)
            .components()
            .filter_map(|component| match component {
                std::path::Component::Normal(part) => {
                    let safe = Self::safe_name_part(&part.to_string_lossy());
                    (!safe.is_empty()).then_some(safe)
                }
                _ => None,
            })
            .collect()
    }

    fn emit_dependency_progress(
        task_id: &str,
        task_label: &str,
        step: &str,
        current: usize,
        total: usize,
        message: &str,
    ) {
        TaskLogger::progress(task_id, task_label, step, current, total.max(1), message);
    }

    // ================================================================
    // Cancel token management (delegated to TaskManager global task system)
    // ================================================================

    /// Cancel the specified export job.
    pub fn cancel_job(job_id: &str) {
        TaskManager::cancel(job_id);
    }

    // ================================================================
    // Main Entry
    // ================================================================

    pub fn run_batch_with_cancel_token(
        job: ExportJob,
        progress_channel: Channel<ExportProgress>,
        cancel_token: Arc<AtomicBool>,
    ) -> ExportReport {
        let start_time = Instant::now();
        let total = job.assets.len();
        let mut results: Vec<ExportResult> = Vec::with_capacity(total);

        TaskLogger::info(
            &job.job_id,
            "batch_export",
            &format!(
                "Starting batch export: {} assets -> {}",
                total, job.options.output_dir
            ),
        );

        // Create output directory
        if let Err(e) = fs::create_dir_all(&job.options.output_dir) {
            let _ = progress_channel.send(ExportProgress {
                job_id: job.job_id.clone(),
                done: 0,
                total,
                current_asset: String::new(),
                current_step: format!("Failed to create directory: {}", e),
                completed: Vec::new(),
                cancelled: false,
            });
            TaskLogger::error(
                &job.job_id,
                "batch_export",
                &format!("Failed to create output directory: {}", e),
            );
            return ExportReport {
                job_id: job.job_id,
                total,
                succeeded: 0,
                failed: total,
                results: Vec::new(),
                output_directory: job.options.output_dir,
                duration_ms: start_time.elapsed().as_millis() as u64,
            };
        }

        let mut bundle_cache: HashMap<String, AssetBundle> = HashMap::new();

        for (index, asset_ref) in job.assets.iter().enumerate() {
            // Cancellation check
            if cancel_token.load(Ordering::SeqCst) {
                TaskLogger::warn(
                    &job.job_id,
                    "batch_export",
                    &format!("Export cancelled ({}/{})", index, total),
                );
                let _ = progress_channel.send(ExportProgress {
                    job_id: job.job_id.clone(),
                    done: index,
                    total,
                    current_asset: String::new(),
                    current_step: "Cancelled".to_string(),
                    completed: results.clone(),
                    cancelled: true,
                });
                break;
            }

            let asset_start = Instant::now();
            let _ = progress_channel.send(ExportProgress {
                job_id: job.job_id.clone(),
                done: index,
                total,
                current_asset: asset_ref.asset_name.clone(),
                current_step: "Exporting...".to_string(),
                completed: results.clone(),
                cancelled: false,
            });

            // Determine output format
            let format = Self::resolve_format(&asset_ref.class_name, &job.options);

            if let Some(result) = Self::try_export_preview_dependency_asset(
                asset_ref,
                &format,
                &job.options,
                &cancel_token,
                &job.job_id,
                &asset_start,
            ) {
                results.push(result);
                continue;
            }

            // Load bundle (cached)
            let bundle = match Self::load_bundle_cached(&asset_ref.bundle_path, &mut bundle_cache) {
                Ok(b) => b,
                Err(e) => {
                    results.push(Self::make_fail_result(asset_ref, format, e, &asset_start));
                    continue;
                }
            };

            // Build output path
            let output_path = match Self::build_output_path(&job.options, asset_ref, &format) {
                Ok(p) => p,
                Err(e) => {
                    results.push(Self::make_fail_result(asset_ref, format, e, &asset_start));
                    continue;
                }
            };

            let split_submesh_candidate = job.options.split_submeshes
                && asset_ref.class_name == "Mesh"
                && matches!(format, ExportFormat::Glb);

            // Skip existing file (unless overwrite requested). Split SubMesh export checks each
            // generated part inside the mesh exporter because the exact part count is mesh data.
            if output_path.exists() && !job.options.overwrite_existing && !split_submesh_candidate {
                let meta = fs::metadata(&output_path).ok();
                results.push(ExportResult {
                    asset_name: asset_ref.asset_name.clone(),
                    class_name: asset_ref.class_name.clone(),
                    path_id: asset_ref.path_id.clone(),
                    output_file: output_path.to_string_lossy().to_string(),
                    format: format.clone(),
                    success: true,
                    error: None,
                    byte_size: meta.map(|m| m.len()).unwrap_or(0),
                    duration_ms: asset_start.elapsed().as_millis() as u64,
                });
                continue;
            }

            // Ensure parent directory
            if let Some(parent) = output_path.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    results.push(Self::make_fail_result(
                        asset_ref,
                        format,
                        format!("Failed to create directory: {}", e),
                        &asset_start,
                    ));
                    continue;
                }
            }

            // Execute export
            let result = Self::export_single(
                bundle,
                asset_ref,
                &format,
                &output_path,
                &asset_start,
                &job.options,
                &cancel_token,
                &job.job_id,
                total,
                index,
            );
            results.push(result);
        }

        // Generate report
        if job.options.generate_report {
            let _ = Self::write_csv_report(&job, &results, &start_time);
        }

        let succeeded = results.iter().filter(|r| r.success).count();
        let failed = total - succeeded;
        let duration_s = start_time.elapsed().as_secs_f64();
        if failed > 0 {
            TaskLogger::warn(
                &job.job_id,
                "batch_export",
                &format!(
                    "Batch export complete: {}/{} succeeded, {} failed, took {}",
                    succeeded,
                    total,
                    failed,
                    TimeUtils::format_duration(duration_s)
                ),
            );
        } else {
            TaskLogger::success(
                &job.job_id,
                "batch_export",
                &format!(
                    "Batch export complete: {}/{} all succeeded, took {}",
                    succeeded,
                    total,
                    TimeUtils::format_duration(duration_s)
                ),
            );
        }
        let _ = progress_channel.send(ExportProgress {
            job_id: job.job_id.clone(),
            done: total,
            total,
            current_asset: String::new(),
            current_step: format!("Complete: {}/{}", succeeded, total),
            completed: results.clone(),
            cancelled: false,
        });

        ExportReport {
            job_id: job.job_id,
            total,
            succeeded,
            failed: total - succeeded,
            results,
            output_directory: job.options.output_dir,
            duration_ms: start_time.elapsed().as_millis() as u64,
        }
    }

    // ================================================================
    // Internal Methods
    // ================================================================

    fn load_bundle_cached<'a>(
        path: &str,
        cache: &'a mut HashMap<String, AssetBundle>,
    ) -> Result<&'a AssetBundle, String> {
        // Check cache first, load on miss (because Result cannot be concisely expressed with entry API)
        if !cache.contains_key(path) {
            let b = AssetBundleLoader::load_bundle(Path::new(path))
                .map_err(|e| format!("Load failed '{}': {}", path, e))?;
            cache.insert(path.to_string(), b);
        }
        Ok(cache
            .get(path)
            .ok_or_else(|| format!("Cache anomaly: just inserted Bundle missing '{}'", path))?)
    }

    fn resolve_format(class_name: &str, options: &ExportOptions) -> ExportFormat {
        if class_name == "GameObject" {
            return ExportFormat::Glb;
        }
        options
            .format_overrides
            .get(class_name)
            .cloned()
            .unwrap_or_else(|| {
                ExportFormatResolver::default_formats_for_class(class_name)
                    .first()
                    .cloned()
                    .unwrap_or(ExportFormat::Raw)
            })
    }

    pub(crate) fn build_output_path(
        options: &ExportOptions,
        asset_ref: &ExportAssetRef,
        format: &ExportFormat,
    ) -> Result<PathBuf, String> {
        let dir = Path::new(&options.output_dir);
        let sub = match &options.group_by {
            ExportGroupBy::None => PathBuf::new(),
            ExportGroupBy::ByType => PathBuf::from(&asset_ref.class_name),
            ExportGroupBy::ByContainer => {
                let mut path = PathBuf::new();
                let parent = Path::new(&asset_ref.asset_name)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                for segment in Self::safe_path_segments(&parent) {
                    path.push(segment);
                }
                path
            }
        };
        let stem = Path::new(&asset_ref.asset_name)
            .file_stem()
            .map(|s| Self::safe_name_part(&s.to_string_lossy()))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("asset_{}", asset_ref.path_id));
        let file_name = Self::safe_asset_file_name(
            &stem,
            &format!("asset_{}", asset_ref.path_id),
            format.extension(),
        );
        let output_path = dir.join(sub).join(file_name);

        let base = dir
            .canonicalize()
            .or_else(|_| {
                fs::create_dir_all(dir).map_err(|e| std::io::Error::new(e.kind(), e))?;
                dir.canonicalize()
            })
            .map_err(|e| format!("Failed to resolve output directory: {}", e))?;
        let parent = output_path.parent().unwrap_or(dir);
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create output directory: {}", e))?;
        let resolved_parent = parent
            .canonicalize()
            .map_err(|e| format!("Failed to resolve output path: {}", e))?;
        if !resolved_parent.starts_with(&base) {
            return Err("Export path escaped the output directory".to_string());
        }
        Ok(output_path)
    }

    fn should_split_submesh_files(
        options: &ExportOptions,
        format: &ExportFormat,
        sub_meshes: &[MeshSubMesh],
    ) -> bool {
        options.split_submeshes && matches!(format, ExportFormat::Glb) && sub_meshes.len() > 1
    }

    fn split_submesh_output_path(output_path: &Path, submesh_index: usize) -> PathBuf {
        let stem = output_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("mesh");
        let extension = output_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("glb");
        if submesh_index == 0 {
            output_path.to_path_buf()
        } else {
            output_path.with_file_name(format!(
                "{}_submesh_{:03}.{}",
                stem,
                submesh_index + 1,
                extension
            ))
        }
    }

    fn single_submesh_attrs<'a>(
        attrs: &MeshAttributes<'a>,
        sub_mesh: &'a [MeshSubMesh; 1],
    ) -> MeshAttributes<'a> {
        MeshAttributes {
            vertices: attrs.vertices,
            indices: attrs.indices,
            normals: attrs.normals,
            uvs: attrs.uvs,
            tangents: attrs.tangents,
            colors: attrs.colors,
            bone_weights: attrs.bone_weights,
            bone_indices: attrs.bone_indices,
            bind_poses: attrs.bind_poses,
            bone_name_hashes: attrs.bone_name_hashes,
            root_bone_name_hash: attrs.root_bone_name_hash,
            sub_meshes: Some(&sub_mesh[..]),
            blend_shapes: attrs.blend_shapes,
        }
    }

    fn export_split_submesh_scene_files(
        mesh_name: &str,
        output_path: &Path,
        format: &ExportFormat,
        attrs: &MeshAttributes<'_>,
        sub_meshes: &[MeshSubMesh],
        materials: Option<&[MaterialInfo]>,
        skeleton: Option<&GlbSkeleton>,
        animations: Option<&[GlbAnimation]>,
        overwrite_existing: bool,
    ) -> Result<(u64, Vec<PathBuf>), String> {
        let Some(parent) = output_path.parent() else {
            return Err("Output path has no parent directory".to_string());
        };
        fs::create_dir_all(parent).map_err(|e| format!("create output folder: {}", e))?;

        let mut total_bytes = 0u64;
        let mut paths = Vec::with_capacity(sub_meshes.len());
        for (submesh_index, sub_mesh) in sub_meshes.iter().enumerate() {
            let part_path = Self::split_submesh_output_path(output_path, submesh_index);
            if part_path.exists() && !overwrite_existing {
                total_bytes += fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);
                paths.push(part_path);
                continue;
            }

            let part_name = format!("{}_submesh_{:03}", mesh_name, submesh_index + 1);
            let single_sub_mesh = [*sub_mesh];
            let part_attrs = Self::single_submesh_attrs(attrs, &single_sub_mesh);
            match format {
                ExportFormat::Glb => {
                    let material = materials
                        .and_then(|items| items.get(submesh_index))
                        .cloned()
                        .map(|item| vec![item])
                        .unwrap_or_default();
                    let glb_data = GlbExporter::build_with_scene_data(
                        &part_name,
                        &part_attrs,
                        if material.is_empty() {
                            None
                        } else {
                            Some(material.as_slice())
                        },
                        skeleton,
                        animations,
                    )?;
                    fs::write(&part_path, &glb_data).map_err(|e| format!("write GLB: {}", e))?;
                }

                _ => {
                    return Err(format!(
                        "SubMesh split does not support format {:?}",
                        format
                    ))
                }
            }
            total_bytes += fs::metadata(&part_path)
                .map_err(|e| format!("stat: {}", e))?
                .len();
            paths.push(part_path);
        }
        Ok((total_bytes, paths))
    }

    fn make_fail_result(
        asset_ref: &ExportAssetRef,
        format: ExportFormat,
        error: String,
        start: &Instant,
    ) -> ExportResult {
        ExportResult {
            asset_name: asset_ref.asset_name.clone(),
            class_name: asset_ref.class_name.clone(),
            path_id: asset_ref.path_id.clone(),
            output_file: String::new(),
            format,
            success: false,
            error: Some(error),
            byte_size: 0,
            duration_ms: start.elapsed().as_millis() as u64,
        }
    }

    fn try_export_preview_dependency_asset(
        asset_ref: &ExportAssetRef,
        format: &ExportFormat,
        options: &ExportOptions,
        cancel_token: &Arc<AtomicBool>,
        task_id: &str,
        start: &Instant,
    ) -> Option<ExportResult> {
        if !matches!(
            asset_ref.class_name.as_str(),
            "Mesh" | "Material" | "Animator" | "GameObject"
        ) {
            return None;
        }
        if matches!(asset_ref.class_name.as_str(), "Animator" | "GameObject")
            && !matches!(format, ExportFormat::Glb)
        {
            return None;
        }
        let Some(cache_root) = options
            .asset_map_cache_root
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        else {
            return None;
        };
        if options.workspace_dirs.is_empty() {
            return None;
        }

        let progress = Channel::<crate::common::scan::scan_types::ProgressPayload>::new(|_| Ok(()));
        let cache_root = Path::new(cache_root);
        let dependency_result = match asset_ref.class_name.as_str() {
            "Mesh" => Self::export_mesh_with_dependencies(
                asset_ref.bundle_path.clone(),
                asset_ref.path_id.clone(),
                options.workspace_dirs.clone(),
                options.output_dir.clone(),
                progress,
                cancel_token,
                task_id,
                Some(cache_root),
                options.clone(),
            ),
            "Material" => Self::export_material_with_dependencies(
                asset_ref.bundle_path.clone(),
                asset_ref.path_id.clone(),
                options.workspace_dirs.clone(),
                options.output_dir.clone(),
                progress,
                cancel_token,
                task_id,
                Some(cache_root),
                options.clone(),
            ),
            "Animator" => AnimatorPreviewWorkflow::export_with_dependencies(
                TaskContext::borrowed(
                    task_id.to_string(),
                    "Export Animator Dependencies",
                    cancel_token.clone(),
                ),
                asset_ref.bundle_path.clone(),
                asset_ref.path_id.clone(),
                options.workspace_dirs[0].clone(),
                options.output_dir.clone(),
                Some(cache_root.to_string_lossy().to_string()),
                options.clone(),
                progress,
            ),
            "GameObject" => GameObjectPreviewWorkflow::export_with_dependencies(
                TaskContext::borrowed(
                    task_id.to_string(),
                    "Export GameObject Dependencies",
                    cancel_token.clone(),
                ),
                asset_ref.bundle_path.clone(),
                asset_ref.path_id.clone(),
                options.workspace_dirs[0].clone(),
                options.output_dir.clone(),
                Some(cache_root.to_string_lossy().to_string()),
                progress,
                options.clone(),
            ),
            _ => unreachable!(),
        };

        Some(match dependency_result {
            Ok(result) => {
                let byte_size = fs::metadata(&result.output_path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                ExportResult {
                    asset_name: asset_ref.asset_name.clone(),
                    class_name: asset_ref.class_name.clone(),
                    path_id: asset_ref.path_id.clone(),
                    output_file: result.output_path,
                    format: format.clone(),
                    success: true,
                    error: None,
                    byte_size,
                    duration_ms: start.elapsed().as_millis() as u64,
                }
            }
            Err(error) => Self::make_fail_result(asset_ref, format.clone(), error, start),
        })
    }

    // ================================================================
    // Single Asset Export Routing
    // ================================================================

    fn export_single(
        bundle: &AssetBundle,
        asset_ref: &ExportAssetRef,
        format: &ExportFormat,
        output_path: &Path,
        start: &Instant,
        options: &ExportOptions,
        cancel_token: &Arc<AtomicBool>,
        task_id: &str,
        total_assets: usize,
        asset_index: usize,
    ) -> ExportResult {
        let path_id: i64 = match asset_ref.path_id.parse() {
            Ok(id) => id,
            Err(e) => {
                return Self::make_fail_result(
                    asset_ref,
                    format.clone(),
                    format!("Invalid path_id: {}", e),
                    start,
                )
            }
        };

        let result: Result<u64, String> = match asset_ref.class_name.as_str() {
            "Texture2D" => Self::export_texture(
                bundle,
                path_id,
                format,
                output_path,
                cancel_token,
                task_id,
                total_assets,
                asset_index,
            ),
            "Sprite" => {
                TaskLogger::progress(
                    task_id,
                    "Batch Export",
                    "Export Texture",
                    asset_index + 1,
                    total_assets.max(1),
                    &format!("Exporting Sprite PNG: {}", asset_ref.asset_name),
                );
                SpritePreviewService::export_sprite_png(
                    bundle,
                    path_id,
                    output_path,
                    Some(cancel_token),
                )
            }
            "SpriteMask" => {
                TaskLogger::progress(
                    task_id,
                    "Batch Export",
                    "Export Texture",
                    asset_index + 1,
                    total_assets.max(1),
                    &format!("Exporting SpriteMask PNG: {}", asset_ref.asset_name),
                );
                SpritePreviewService::export_sprite_mask_png(
                    bundle,
                    path_id,
                    output_path,
                    Some(cancel_token),
                )
            }
            "Mesh" => Self::export_mesh(
                bundle,
                &asset_ref.bundle_path,
                path_id,
                format,
                output_path,
                options,
            ),
            "TextAsset" => Self::export_text_asset(bundle, path_id, format, output_path),
            "AudioClip" => {
                Self::export_audio(bundle, &asset_ref.bundle_path, path_id, format, output_path)
            }
            "Font" => Self::export_font(bundle, path_id, format, output_path),
            "MonoBehaviour" => Self::export_mono(bundle, path_id, format, output_path),
            "AnimationClip"
            | "Animator"
            | "GameObject"
            | "Avatar"
            | "RuntimeAnimatorController"
            | "AnimatorController"
            | "AnimatorOverrideController" => {
                if matches!(format, ExportFormat::Raw) {
                    Self::export_typetree_or_raw(bundle, path_id, output_path)
                } else {
                    Self::export_typed_preview_json(&asset_ref.bundle_path, path_id, output_path)
                        .or_else(|_| Self::export_typetree_or_raw(bundle, path_id, output_path))
                }
            }
            "Shader" => Self::export_shader(bundle, path_id, output_path),
            "VideoClip" | "MovieTexture" => Self::export_video(bundle, path_id, output_path),
            _ => Self::export_raw(bundle, path_id, format, output_path),
        };

        match result {
            Ok(size) => ExportResult {
                asset_name: asset_ref.asset_name.clone(),
                class_name: asset_ref.class_name.clone(),
                path_id: asset_ref.path_id.clone(),
                output_file: output_path.to_string_lossy().to_string(),
                format: format.clone(),
                success: true,
                error: None,
                byte_size: size,
                duration_ms: start.elapsed().as_millis() as u64,
            },
            Err(e) => Self::make_fail_result(asset_ref, format.clone(), e, start),
        }
    }

    fn find_object_bytes<'a>(
        bundle: &'a AssetBundle,
        path_id: i64,
        class_ids: &[i32],
        type_name: &str,
    ) -> Result<(&'a [u8], &'a serialized_file::SerializedFile), String> {
        bundle
            .assets
            .iter()
            .filter_map(|sf| {
                sf.objects
                    .iter()
                    .find(|o| o.path_id == path_id && class_ids.contains(&o.class_id))
                    .and_then(|o| sf.object_bytes(o).ok().map(|b| (b, &sf.inner)))
            })
            .next()
            .ok_or_else(|| format!("Not found {}(path_id={})", type_name, path_id))
    }

    fn find_any_object_bytes<'a>(
        bundle: &'a AssetBundle,
        path_id: i64,
    ) -> Result<(&'a [u8], &'a serialized_file::SerializedFile), String> {
        bundle
            .assets
            .iter()
            .filter_map(|sf| {
                sf.objects
                    .iter()
                    .find(|o| o.path_id == path_id)
                    .and_then(|o| sf.object_bytes(o).ok().map(|b| (b, &sf.inner)))
            })
            .next()
            .ok_or_else(|| format!("Not found any object(path_id={})", path_id))
    }

    fn parse_texture_object(
        bundle: &AssetBundle,
        path_id: i64,
    ) -> Result<crate::unity::classes::texture2d::Texture2DObject, String> {
        match bundle.parse_object_by_path_id(path_id)? {
            UnityClassObject::Texture2D(texture) => Ok(texture),
            other => Err(format!(
                "Object path_id={} is {}, not Texture2D",
                path_id,
                other.class_name()
            )),
        }
    }

    // ================================================================
    // Texture2D / Sprite
    // ================================================================

    fn export_texture(
        bundle: &AssetBundle,
        path_id: i64,
        format: &ExportFormat,
        output_path: &Path,
        cancel_token: &Arc<AtomicBool>,
        task_id: &str,
        total_assets: usize,
        asset_index: usize,
    ) -> Result<u64, String> {
        if cancel_token.load(Ordering::SeqCst) {
            return Err("Task cancelled".to_string());
        }
        TaskLogger::progress(
            task_id,
            "Batch Export",
            "Export Texture",
            asset_index + 1,
            total_assets.max(1),
            &format!("Parsing Texture2D object path_id={}", path_id),
        );
        let info = Self::parse_texture_object(bundle, path_id)?.data;
        if cancel_token.load(Ordering::SeqCst) {
            return Err("Task cancelled".to_string());
        }

        // PNG / TGA / uncompressed DDS export: decode to display-oriented RGBA then encode.
        if *format == ExportFormat::Png || *format == ExportFormat::Tga {
            let rgba = crate::exporter::texture_exporter::TextureExporter::decode_pixels_to_display_rgba_cancelable(
                &info.pixel_data,
                info.texture_format,
                info.width,
                info.height,
                Some(cancel_token),
            )?;
            TaskLogger::progress(
                task_id,
                "Batch Export",
                "Export Texture",
                asset_index + 1,
                total_assets.max(1),
                &format!(
                    "Writing Texture2D {}: {}x{}",
                    if *format == ExportFormat::Tga {
                        "TGA"
                    } else {
                        "PNG"
                    },
                    info.width,
                    info.height,
                ),
            );
            if *format == ExportFormat::Tga {
                crate::exporter::texture_exporter::TextureExporter::save_tga_cancelable(
                    &rgba,
                    info.width,
                    info.height,
                    output_path,
                    Some(cancel_token),
                )?;
            } else {
                crate::exporter::texture_exporter::TextureExporter::save_png_cancelable(
                    &rgba,
                    info.width,
                    info.height,
                    output_path,
                    Some(cancel_token),
                )?;
            }
        } else {
            // DDS export: prefer display-oriented RGBA8 DDS so standalone viewers match PNG/TGA.
            // If a texture format cannot be decoded, fall back to the original lossless DDS path.
            TaskLogger::progress(
                task_id,
                "Batch Export",
                "Export Texture",
                asset_index + 1,
                total_assets.max(1),
                &format!(
                    "Writing Texture2D DDS: {}x{} format={}",
                    info.width, info.height, info.texture_format
                ),
            );
            match crate::exporter::texture_exporter::TextureExporter::decode_pixels_to_display_rgba_cancelable(
                &info.pixel_data,
                info.texture_format,
                info.width,
                info.height,
                Some(cancel_token),
            ) {
                Ok(rgba) => {
                    DDSExporter::export_rgba8_cancelable(
                        &rgba,
                        info.width,
                        info.height,
                        output_path,
                        Some(cancel_token),
                    )?;
                }
                Err(_) => {
                    DDSExporter::export_cancelable(
                        &info.pixel_data,
                        info.texture_format,
                        info.width,
                        info.height,
                        info.mip_count,
                        output_path,
                        Some(cancel_token),
                    )?;
                }
            }
        }
        Ok(fs::metadata(output_path)
            .map_err(|e| format!("stat: {}", e))?
            .len())
    }

    // ================================================================
    // Mesh
    // ================================================================

    fn export_mesh(
        bundle: &AssetBundle,
        bundle_path_str: &str,
        path_id: i64,
        format: &ExportFormat,
        output_path: &Path,
        options: &ExportOptions,
    ) -> Result<u64, String> {
        // 1. Find Mesh object
        let (file, info) = bundle
            .assets
            .iter()
            .filter_map(|sf| {
                sf.objects
                    .iter()
                    .find(|o| o.path_id == path_id && (o.class_id == 43 || o.class_id == 34))
                    .map(|o| (sf, o))
            })
            .next()
            .ok_or_else(|| format!("Not found Mesh with path_id={}", path_id))?;

        // 2. Extract bundle nodes to temp cache (.resS streaming vertex data)
        let temp_dir = std::env::temp_dir().join("assetdaemon_mesh_export");
        let _ = fs::create_dir_all(&temp_dir);
        let bp = std::path::Path::new(bundle_path_str);
        let cache_ok =
            crate::common::bundle_file::bundle_extractor::BundleExtractor::extract_all_nodes(
                bundle, bp, &temp_dir,
            )
            .is_ok();

        // 3. Create ObjectHandle
        let handle = crate::common::bundle_file::asset_bundle::ObjectHandle::new(file, info);
        let unity_version = &file.unity_version;
        let raw_data = handle
            .raw_data()
            .map_err(|e| format!("Get raw data failed: {}", e))?;

        // 4. AssetStudio parser (precise)
        use crate::common::mesh::mesh_service::MeshService;
        let noop = tauri::ipc::Channel::<crate::common::scan::scan_types::ProgressPayload>::new(
            |_| Ok(()),
        );
        let cache_opt: Option<&std::path::Path> = if cache_ok { Some(&temp_dir) } else { None };

        let geo = MeshService::extract_mesh_from_object(
            &handle,
            bundle,
            bp,
            unity_version,
            &raw_data,
            &noop,
            cache_opt,
        )
        .map_err(|e| format!("Mesh geometry extraction failed: {}", e))?;

        if !geo.success || geo.vertices.is_empty() {
            return Err(format!("Mesh extraction failed: {}", geo.error));
        }

        // 5. Export (with all attributes)
        let vertex_count = geo.vertices.len() / 3;
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
            normals: if geo.normals.len() >= vertex_count * 3 {
                Some(&geo.normals)
            } else {
                None
            },
            uvs: if geo.uvs.len() >= vertex_count * 2 {
                Some(&geo.uvs)
            } else {
                None
            },
            tangents: if geo.tangents.len() >= vertex_count * 4 {
                Some(&geo.tangents)
            } else {
                None
            },
            colors: if geo.colors.len() >= vertex_count * 4 {
                Some(&geo.colors)
            } else {
                None
            },
            bone_weights: if geo.bone_weights.len() >= vertex_count * 4 {
                Some(&geo.bone_weights)
            } else {
                None
            },
            bone_indices: if geo.bone_indices.len() >= vertex_count * 4 {
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
        let mesh_name = output_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Mesh");
        let needs_scene_data = matches!(format, ExportFormat::Glb);
        let skeleton = if needs_scene_data && options.include_skeleton {
            ModelContextResolver::resolve_skeleton_for_mesh_with_hashes(
                bundle,
                path_id,
                geo.bind_poses.len() / 16,
                &geo.bone_name_hashes,
                geo.root_bone_name_hash,
            )
        } else {
            None
        };
        let animations = if needs_scene_data && options.include_animations {
            skeleton
                .as_ref()
                .map(|skel| {
                    ExportAnimationUtils::extract_related_animations(
                        bundle,
                        bundle_path_str,
                        path_id,
                        None,
                        None,
                        skel,
                        &blend_shapes,
                        None,
                    )
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let animation_ref = if animations.is_empty() {
            None
        } else {
            Some(animations.as_slice())
        };
        if Self::should_split_submesh_files(options, format, &sub_meshes) {
            let (total_bytes, _paths) = Self::export_split_submesh_scene_files(
                mesh_name,
                output_path,
                format,
                &attrs,
                &sub_meshes,
                None,
                skeleton.as_ref(),
                animation_ref,
                options.overwrite_existing,
            )?;
            return Ok(total_bytes);
        }
        if output_path.exists() && !options.overwrite_existing {
            return Ok(fs::metadata(output_path)
                .map_err(|e| format!("stat: {}", e))?
                .len());
        }
        match format {
            ExportFormat::Glb => {
                let glb_data = GlbExporter::build_with_scene_data(
                    mesh_name,
                    &attrs,
                    None,
                    skeleton.as_ref(),
                    animation_ref,
                )?;
                fs::write(output_path, &glb_data).map_err(|e| format!("Write GLB: {}", e))?;
            }

            ExportFormat::Obj => {
                let obj_text = MeshExporter::build_obj_with_attributes(
                    &geo.vertices,
                    &geo.indices,
                    if geo.normals.len() >= geo.vertices.len() {
                        Some(&geo.normals)
                    } else {
                        None
                    },
                    if geo.uvs.len() >= 2 * geo.vertices.len() / 3 {
                        Some(&geo.uvs)
                    } else {
                        None
                    },
                    "",
                )?;
                fs::write(output_path, obj_text.as_bytes())
                    .map_err(|e| format!("Write OBJ: {}", e))?;
            }
            _ => return Err(format!("Mesh does not support format {:?}", format)),
        }
        Ok(fs::metadata(output_path)
            .map_err(|e| format!("stat: {}", e))?
            .len())
    }

    // ================================================================
    // TextAsset
    // ================================================================

    fn export_text_asset(
        bundle: &AssetBundle,
        path_id: i64,
        format: &ExportFormat,
        output_path: &Path,
    ) -> Result<u64, String> {
        let (raw, sf) = Self::find_object_bytes(bundle, path_id, &[49], "TextAsset")?;
        let mut reader = ObjectReader::new(raw, sf);
        let _name = reader.try_read_aligned_string()?;
        let script_data = reader.try_read_u8_array()?;
        let fmt_str = format!["{:?}", format].to_lowercase();
        TextAssetExporter::export(&script_data, &fmt_str, output_path)
    }

    fn export_audio(
        bundle: &AssetBundle,
        bundle_path: &str,
        path_id: i64,
        format: &ExportFormat,
        output_path: &Path,
    ) -> Result<u64, String> {
        let (raw, sf) = Self::find_object_bytes(bundle, path_id, &[83], "AudioClip")?;
        AudioExporter::export(raw, sf, bundle, Path::new(bundle_path), format, output_path)
    }

    fn export_font(
        bundle: &AssetBundle,
        path_id: i64,
        format: &ExportFormat,
        output_path: &Path,
    ) -> Result<u64, String> {
        let (raw, sf) = Self::find_object_bytes(bundle, path_id, &[128], "Font")?;
        let mut reader = ObjectReader::new(raw, sf);
        let _name = reader.try_read_aligned_string()?;
        let font_data = reader.try_read_u8_array()?;
        if font_data.is_empty() {
            RawExporter::export(raw, output_path)
        } else {
            // Format parameter passed to FontExporter to support auto-detection vs forced format selection
            FontExporter::export_with_format(&font_data, format, output_path)
                .or_else(|_| FontExporter::export(&font_data, output_path))
        }
    }

    fn export_mono(
        bundle: &AssetBundle,
        path_id: i64,
        _format: &ExportFormat,
        output_path: &Path,
    ) -> Result<u64, String> {
        let (sf, obj_info) = bundle
            .assets
            .iter()
            .filter_map(|sf| {
                sf.objects
                    .iter()
                    .find(|o| o.path_id == path_id && o.class_id == 114)
                    .map(|o| (sf, o))
            })
            .next()
            .ok_or_else(|| format!("Not found MonoBehaviour(path_id={})", path_id))?;
        let raw = sf.object_bytes(obj_info)?;
        let tt = sf
            .inner
            .object_serialized_type(&obj_info.inner)
            .map(|st| &st.type_tree)
            .filter(|tt| !tt.nodes.is_empty());
        match tt {
            Some(tt) => {
                MonoBehaviourExporter::export_with_typetree(raw, &sf.inner, tt, output_path)
            }
            None => MonoBehaviourExporter::export_raw(raw, path_id, output_path),
        }
    }

    fn export_shader(
        bundle: &AssetBundle,
        path_id: i64,
        output_path: &Path,
    ) -> Result<u64, String> {
        let (raw, _sf) = Self::find_any_object_bytes(bundle, path_id)?;
        crate::exporter::shader_exporter::ShaderExporter::export(raw, output_path)
    }

    fn export_typetree_or_raw(
        bundle: &AssetBundle,
        path_id: i64,
        output_path: &Path,
    ) -> Result<u64, String> {
        let (sf, obj_info) = bundle
            .assets
            .iter()
            .filter_map(|sf| {
                sf.objects
                    .iter()
                    .find(|o| o.path_id == path_id)
                    .map(|o| (sf, o))
            })
            .next()
            .ok_or_else(|| format!("Not found object(path_id={})", path_id))?;
        let raw = sf.object_bytes(obj_info)?;
        let tt = sf
            .inner
            .object_serialized_type(&obj_info.inner)
            .map(|st| &st.type_tree)
            .filter(|tt| !tt.nodes.is_empty());
        match tt {
            Some(tt) => {
                use crate::utils::dump_utils::DumpUtils;
                let mut reader = ObjectReader::new(raw, &sf.inner);
                let value = DumpUtils::export_typetree_json(&mut reader, tt, &Default::default())?;
                let text =
                    serde_json::to_string_pretty(&value).map_err(|e| format!("JSON: {}", e))?;
                fs::write(output_path, text).map_err(|e| format!("write: {}", e))?;
                Ok(fs::metadata(output_path)
                    .map_err(|e| format!("stat: {}", e))?
                    .len())
            }
            None => RawExporter::export(raw, output_path),
        }
    }

    fn export_typed_preview_json(
        bundle_path: &str,
        path_id: i64,
        output_path: &Path,
    ) -> Result<u64, String> {
        let preview = AssetPreviewService::preview(bundle_path, path_id, None, None, None)?;
        let json = serde_json::to_string_pretty(&preview)
            .map_err(|e| format!("Serialize typed preview JSON failed: {}", e))?;
        fs::write(output_path, json.as_bytes()).map_err(|e| format!("write: {}", e))?;
        Ok(fs::metadata(output_path)
            .map_err(|e| format!("stat: {}", e))?
            .len())
    }

    fn export_video(bundle: &AssetBundle, path_id: i64, output_path: &Path) -> Result<u64, String> {
        let (raw, _sf) = Self::find_any_object_bytes(bundle, path_id)?;
        crate::exporter::video_exporter::VideoExporter::export(raw, output_path)
    }

    fn export_raw(
        bundle: &AssetBundle,
        path_id: i64,
        _format: &ExportFormat,
        output_path: &Path,
    ) -> Result<u64, String> {
        let (raw, _sf) = Self::find_any_object_bytes(bundle, path_id)?;
        RawExporter::export(raw, output_path)
    }

    // ================================================================
    // Mesh related asset export (GLB + related textures)
    // ================================================================

    /// Export Mesh and its related assets (Material + Texture2D),
    /// generating a GLB file and accompanying PNG preview sidecar textures.
    ///
    /// Uses existing Texture2D object parsing in ExportService to ensure streaming textures (.resS) are loaded correctly.
    pub fn export_mesh_with_dependencies(
        bundle_path: String,
        path_id: String,
        workspace_dirs: Vec<String>,
        output_dir: String,
        progress: Channel<crate::common::scan::scan_types::ProgressPayload>,
        cancel_token: &Arc<AtomicBool>,
        task_id: &str,
        asset_map_cache_root: Option<&Path>,
        options: ExportOptions,
    ) -> Result<DependencyExportResult, String> {
        let path_id_i64: i64 = path_id
            .parse()
            .map_err(|e| format!("Invalid path_id: {}", e))?;
        let bundle_p = Path::new(&bundle_path);
        if !bundle_p.exists() {
            return Err(format!("File does not exist: {}", bundle_path));
        }

        //
        // Step 1: Load the Bundle containing the target Mesh
        //
        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: "load_mesh".into(),
                message: "Loading mesh bundle...".into(),
            })
            .ok();
        Self::emit_dependency_progress(
            task_id,
            "Export Mesh Dependencies",
            "Load Mesh",
            1,
            6,
            "Loading mesh bundle...",
        );

        let bundle = AssetBundleLoader::load_bundle(bundle_p)
            .map_err(|e| format!("Failed to load Bundle: {}", e))?;

        let found: Option<(
            &crate::common::bundle_file::asset_bundle::SerializedFile,
            &crate::common::bundle_file::asset_bundle::ObjectInfo,
        )> = bundle
            .assets
            .iter()
            .filter_map(|sf| {
                sf.objects
                    .iter()
                    .find(|o| o.path_id == path_id_i64 && (o.class_id == 43 || o.class_id == 34))
                    .map(|o| (sf, o))
            })
            .next();

        let (sf, obj_info) =
            found.ok_or_else(|| format!("Not found Mesh with path_id={}", path_id_i64))?;

        let handle = crate::common::bundle_file::asset_bundle::ObjectHandle::new(sf, obj_info);
        let mesh_name = handle
            .peek_name()
            .ok()
            .flatten()
            .unwrap_or_else(|| format!("mesh_{}", path_id_i64));
        let unity_version = sf.unity_version.clone();
        let raw_data = handle
            .raw_data()
            .map_err(|e| format!("Get raw data: {}", e))?;

        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: "load_mesh".into(),
                message: format!("Mesh: {} (Unity {})", mesh_name, unity_version),
            })
            .ok();

        if cancel_token.load(Ordering::SeqCst) {
            return Err("Task cancelled".to_string());
        }

        //
        // Step 2: Extract Mesh geometry data
        //
        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: "extract_geo".into(),
                message: "Extracting mesh geometry...".into(),
            })
            .ok();
        Self::emit_dependency_progress(
            task_id,
            "Export Mesh Dependencies",
            "Extract Geometry",
            2,
            6,
            "Extracting mesh geometry...",
        );

        let noop_ch = Channel::<crate::common::scan::scan_types::ProgressPayload>::new(|_| Ok(()));
        let cache_path = std::env::temp_dir().join("assetdaemon_mesh_export");
        let _ = fs::create_dir_all(&cache_path);
        let _ = crate::common::bundle_file::bundle_extractor::BundleExtractor::extract_all_nodes(
            &bundle,
            bundle_p,
            &cache_path,
        );

        let geo = MeshService::extract_mesh_from_object(
            &handle,
            &bundle,
            bundle_p,
            &unity_version,
            &raw_data,
            &noop_ch,
            Some(&cache_path),
        )
        .map_err(|e| format!("Mesh geometry extraction failed: {}", e))?;

        if !geo.success || geo.vertices.is_empty() {
            return Err(format!("Mesh geometry data is empty: {}", geo.error));
        }

        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: "extract_geo".into(),
                message: format!(
                    "Extracted {} vertices, {} triangles and {} submesh range(s)",
                    geo.vertex_count,
                    geo.triangle_count,
                    geo.sub_meshes.len()
                ),
            })
            .ok();
        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: "extract_geo".into(),
                message: format!(
                    "Skin data: weights={}, indices={}, bind poses={}, bone hashes={}",
                    geo.bone_weights.len() / 4,
                    geo.bone_indices.len() / 4,
                    geo.bind_poses.len() / 16,
                    geo.bone_name_hashes.len()
                ),
            })
            .ok();
        if (!geo.bind_poses.is_empty() || !geo.bone_name_hashes.is_empty())
            && (geo.bone_weights.len() < geo.vertex_count * 4
                || geo.bone_indices.len() < geo.vertex_count * 4)
        {
            progress.send(crate::common::scan::scan_types::ProgressPayload {
                step: "extract_geo".into(),
                message: format!(
                    "Skin warning: bind poses/hashes exist but vertex weights or indices are incomplete; GLB will not contain usable skinning unless both reach {}",
                    geo.vertex_count
                ),
            }).ok();
        }

        if cancel_token.load(Ordering::SeqCst) {
            return Err("Task cancelled".to_string());
        }

        let map_db = if options.include_materials
            || options.include_textures
            || options.include_skeleton
            || options.include_animations
        {
            match asset_map_cache_root {
                Some(cache_root) => {
                    AssetMapRepository::open_first_existing(&workspace_dirs, Some(cache_root))?
                }
                None => None,
            }
        } else {
            None
        };

        let (mut materials, texture_refs): (Vec<MaterialInfo>, Vec<(i64, String, String)>) = {
            let mut resolved_from_map: Option<(Vec<MaterialInfo>, Vec<(i64, String, String)>)> =
                None;
            if options.include_materials || options.include_textures {
                progress
                    .send(crate::common::scan::scan_types::ProgressPayload {
                        step: "resolve_deps".into(),
                        message: if options.include_materials && options.include_textures {
                            "Resolving related materials and textures...".into()
                        } else if options.include_textures {
                            "Resolving related textures with preview dependency manifest...".into()
                        } else {
                            "Resolving related materials without exporting textures...".into()
                        },
                    })
                    .ok();
                Self::emit_dependency_progress(
                    task_id,
                    "Export Mesh Dependencies",
                    "Resolve Dependencies",
                    3,
                    6,
                    if options.include_materials && options.include_textures {
                        "Resolving related materials and textures..."
                    } else if options.include_textures {
                        "Resolving related textures with preview dependency manifest..."
                    } else {
                        "Resolving related materials without exporting textures..."
                    },
                );
                if let Some(ref database) = map_db {
                    let asset_count = database.asset_count().unwrap_or(0);
                    let bundle_count = database.bundle_count().unwrap_or(0);
                    if asset_count > 0 && bundle_count > 0 {
                        let mesh_refs = vec![AnimatorPreviewMeshRef {
                            mesh_bundle_path: bundle_path.clone(),
                            mesh_path_id: path_id_i64,
                            renderer_bundle_path: bundle_path.clone(),
                            renderer_path_id: 0,
                        }];
                        let mut deps =
                            AnimatorPreviewMaterials::resolve(database, &mesh_refs, &progress);
                        if !options.include_textures {
                            deps.1.clear();
                            for material in &mut deps.0 {
                                material.textures.clear();
                            }
                        }
                        if !options.include_materials {
                            deps.0.clear();
                        }
                        if !deps.0.is_empty() || !deps.1.is_empty() {
                            resolved_from_map = Some(deps);
                        }
                    }
                }
            } else {
                progress
                    .send(crate::common::scan::scan_types::ProgressPayload {
                        step: "resolve_deps".into(),
                        message:
                            "Material and texture dependency resolution skipped by export options."
                                .into(),
                    })
                    .ok();
                Self::emit_dependency_progress(
                    task_id,
                    "Export Mesh Dependencies",
                    "Resolve Dependencies",
                    3,
                    6,
                    "Material and texture dependency resolution skipped.",
                );
            }

            if let Some(deps) = resolved_from_map {
                deps
            } else if options.include_materials || options.include_textures {
                let message = "AssetMap has no usable dependency match; exporting mesh geometry without related materials/textures.";
                progress
                    .send(crate::common::scan::scan_types::ProgressPayload {
                        step: "resolve_deps".into(),
                        message: message.into(),
                    })
                    .ok();
                TaskLogger::warn(task_id, "Export Mesh Dependencies", message);
                (Vec::new(), Vec::new())
            } else {
                (Vec::new(), Vec::new())
            }
        };

        let material_count = materials.len();
        let texture_count = texture_refs.len();

        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: "resolve_deps".into(),
                message: format!(
                    "Resolved {} materials and {} textures",
                    material_count, texture_count
                ),
            })
            .ok();

        if cancel_token.load(Ordering::SeqCst) {
            return Err("Task cancelled".to_string());
        }

        //
        // Step 4: Export related Texture2D as PNG sidecar files matching preview output
        //
        let output_root = Path::new(&output_dir);
        let mesh_folder = output_root.join(&mesh_name);
        fs::create_dir_all(&mesh_folder)
            .map_err(|e| format!("Failed to create output directory: {}", e))?;

        let exported_texture_paths = if options.include_textures {
            AnimatorPreviewMaterials::export_textures(
                &mut materials,
                &texture_refs,
                &mesh_folder,
                &progress,
                task_id,
                "Export Mesh Dependencies",
                cancel_token,
            )
            .iter()
            .map(|candidate| candidate.png_path.clone())
            .collect::<Vec<_>>()
        } else {
            progress
                .send(crate::common::scan::scan_types::ProgressPayload {
                    step: "textures".into(),
                    message: "Texture export skipped by export options.".into(),
                })
                .ok();
            Vec::new()
        };

        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: "textures".into(),
                message: format!("Exported {} textures", exported_texture_paths.len()),
            })
            .ok();
        Self::emit_dependency_progress(
            task_id,
            "Export Mesh Dependencies",
            "Export Textures",
            4,
            6,
            &format!("Exported {} textures", exported_texture_paths.len()),
        );

        if cancel_token.load(Ordering::SeqCst) {
            return Err("Task cancelled".to_string());
        }

        //
        // Step 5: Assemble material info (map texture paths to materials)
        //
        let _ = progress.send(crate::common::scan::scan_types::ProgressPayload {
            step: "materials".into(),
            message: format!("Preparing {} materials", materials.len()),
        });
        Self::emit_dependency_progress(
            task_id,
            "Export Mesh Dependencies",
            "Prepare Materials",
            5,
            6,
            &format!("Preparing {} materials", materials.len()),
        );
        let mesh_format = options
            .format_overrides
            .get("Mesh")
            .cloned()
            .unwrap_or(ExportFormat::Glb);
        if !matches!(mesh_format, ExportFormat::Glb) {
            return Err(format!(
                "Mesh dependency export does not support format {:?}",
                mesh_format
            ));
        }

        //
        // Step 6: Export primary mesh
        //
        let format_label = mesh_format.extension().to_uppercase();
        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: mesh_format.extension().into(),
                message: format!("Generating {}...", format_label),
            })
            .ok();
        Self::emit_dependency_progress(
            task_id,
            "Export Mesh Dependencies",
            &format!("Generate {}", format_label),
            6,
            6,
            &format!("Generating {}...", format_label),
        );

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
            normals: if geo.normals.len() >= geo.vertex_count * 3 {
                Some(&geo.normals)
            } else {
                None
            },
            uvs: if geo.uvs.len() >= geo.vertex_count * 2 {
                Some(&geo.uvs)
            } else {
                None
            },
            tangents: if geo.tangents.len() >= geo.vertex_count * 4 {
                Some(&geo.tangents)
            } else {
                None
            },
            colors: if geo.colors.len() >= geo.vertex_count * 4 {
                Some(&geo.colors)
            } else {
                None
            },
            bone_weights: if geo.bone_weights.len() >= geo.vertex_count * 4 {
                Some(&geo.bone_weights)
            } else {
                None
            },
            bone_indices: if geo.bone_indices.len() >= geo.vertex_count * 4 {
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

        let (skeleton, skeleton_bundle_path) = if options.include_skeleton {
            let resolved_skeleton = ModelContextResolver::resolve_skeleton_for_mesh_with_map(
                &bundle,
                path_id_i64,
                &bundle_path,
                map_db.as_ref(),
                geo.bind_poses.len() / 16,
                &geo.bone_name_hashes,
                geo.root_bone_name_hash,
            );
            match resolved_skeleton {
                Some((skeleton, skeleton_bundle_path)) => {
                    let _ = progress.send(crate::common::scan::scan_types::ProgressPayload {
                        step: mesh_format.extension().into(),
                        message: format!(
                            "Resolved skeleton: {} joints{}",
                            skeleton.joints.len(),
                            skeleton_bundle_path
                                .as_ref()
                                .map(|p| format!(" from {}", p))
                                .unwrap_or_default()
                        ),
                    });
                    (Some(skeleton), skeleton_bundle_path)
                }
                None => {
                    let _ = progress.send(crate::common::scan::scan_types::ProgressPayload {
                        step: mesh_format.extension().into(),
                        message: "No SkinnedMeshRenderer/Bones found for this Mesh in current bundle or AssetMap".into(),
                    });
                    (None, None)
                }
            }
        } else {
            let _ = progress.send(crate::common::scan::scan_types::ProgressPayload {
                step: mesh_format.extension().into(),
                message: "Skeleton resolution skipped by export options.".into(),
            });
            (None, None)
        };
        let animations = if options.include_animations {
            skeleton
                .as_ref()
                .map(|skel| {
                    ExportAnimationUtils::extract_related_animations(
                        &bundle,
                        &bundle_path,
                        path_id_i64,
                        skeleton_bundle_path.as_deref(),
                        map_db.as_ref(),
                        skel,
                        &blend_shapes,
                        Some(&progress),
                    )
                })
                .unwrap_or_default()
        } else {
            let _ = progress.send(crate::common::scan::scan_types::ProgressPayload {
                step: mesh_format.extension().into(),
                message: "Animation resolution skipped by export options.".into(),
            });
            Vec::new()
        };
        let animation_ref = if animations.is_empty() {
            None
        } else {
            Some(animations.as_slice())
        };
        let output_path = if Self::should_split_submesh_files(&options, &mesh_format, &sub_meshes) {
            let primary_path =
                mesh_folder.join(format!("{}.{}", mesh_name, mesh_format.extension()));
            let (_total_bytes, paths) = Self::export_split_submesh_scene_files(
                &mesh_name,
                &primary_path,
                &mesh_format,
                &attrs,
                &sub_meshes,
                if materials.is_empty() {
                    None
                } else {
                    Some(&materials)
                },
                skeleton.as_ref(),
                animation_ref,
                options.overwrite_existing,
            )?;
            paths
                .first()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_else(|| primary_path.to_string_lossy().to_string())
        } else {
            match mesh_format {
                ExportFormat::Glb => GlbExporter::export_with_scene_data(
                    output_root,
                    &mesh_name,
                    &attrs,
                    if materials.is_empty() {
                        None
                    } else {
                        Some(&materials)
                    },
                    skeleton.as_ref(),
                    animation_ref,
                )?,

                _ => unreachable!("mesh format is checked before export"),
            }
        };

        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: "done".into(),
                message: format!("Export complete: {}", output_path),
            })
            .ok();

        Ok(DependencyExportResult {
            output_path: output_path.clone(),
            glb_path: if mesh_format == ExportFormat::Glb {
                output_path.clone()
            } else {
                String::new()
            },
            texture_paths: exported_texture_paths,
            material_count,
            texture_count,
            vertex_count: geo.vertex_count,
            triangle_count: geo.triangle_count,
        })
    }

    /// Export Material and its related textures.
    ///
    /// Starting from Material:
    /// 1. Load the Bundle containing the Material
    /// 2. Resolve textures through the same dependency path used by preview
    /// 3. Export textures as PNG sidecar files with the preview texture exporter
    /// 5. Export Material properties as JSON
    ///
    /// No Mesh geometry data, no GLB generation.
    pub fn export_material_with_dependencies(
        bundle_path: String,
        path_id: String,
        workspace_dirs: Vec<String>,
        output_dir: String,
        progress: Channel<crate::common::scan::scan_types::ProgressPayload>,
        cancel_token: &Arc<AtomicBool>,
        task_id: &str,
        asset_map_cache_root: Option<&Path>,
        options: ExportOptions,
    ) -> Result<DependencyExportResult, String> {
        let path_id_i64: i64 = path_id
            .parse()
            .map_err(|e| format!("Invalid path_id: {}", e))?;
        let bundle_p = Path::new(&bundle_path);
        if !bundle_p.exists() {
            return Err(format!("File does not exist: {}", bundle_path));
        }

        //
        // Step 1: Load the Bundle containing the target Material
        //
        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: "load_material".into(),
                message: "Loading Bundle containing Material...".into(),
            })
            .ok();
        Self::emit_dependency_progress(
            task_id,
            "Export Material Dependencies",
            "Load Material",
            1,
            4,
            "Loading Bundle containing Material...",
        );

        let bundle = AssetBundleLoader::load_bundle(bundle_p)
            .map_err(|e| format!("Failed to load Bundle: {}", e))?;

        // Find Material object (class_id=21)
        let (sf, obj_info) = bundle
            .assets
            .iter()
            .filter_map(|sf| {
                sf.objects
                    .iter()
                    .find(|o| o.path_id == path_id_i64 && o.class_id == 21)
                    .map(|o| (sf, o))
            })
            .next()
            .ok_or_else(|| {
                let msg = format!(
                    "Not found Material with path_id={} (class_id=21)",
                    path_id_i64
                );
                msg
            })?;

        let handle = crate::common::bundle_file::asset_bundle::ObjectHandle::new(sf, obj_info);
        let mat_name = handle
            .peek_name()
            .ok()
            .flatten()
            .unwrap_or_else(|| format!("material_{}", path_id_i64));

        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: "load_material".into(),
                message: format!("Material: {}", mat_name),
            })
            .ok();

        if cancel_token.load(Ordering::SeqCst) {
            return Err("Task cancelled".to_string());
        }

        //
        // Step 2: Resolve Material textures with the same path used by preview.
        //
        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: "resolve_deps".into(),
                message: if options.include_textures {
                    "Resolving Material textures with preview dependency path...".into()
                } else {
                    "Texture dependency resolution skipped by export options.".into()
                },
            })
            .ok();
        Self::emit_dependency_progress(
            task_id,
            "Export Material Dependencies",
            "Resolve Dependencies",
            2,
            4,
            if options.include_textures {
                "Resolving Material textures with preview dependency path..."
            } else {
                "Texture dependency resolution skipped."
            },
        );

        let db = if options.include_textures {
            match asset_map_cache_root {
                Some(cache_root) => {
                    AssetMapRepository::open_first_existing(&workspace_dirs, Some(cache_root))?
                }
                None => None,
            }
        } else {
            None
        };

        let (mut materials, texture_refs): (Vec<MaterialInfo>, Vec<(i64, String, String)>) =
            if options.include_textures {
                match db {
                    Some(ref database) => {
                        let asset_count = database.asset_count().unwrap_or(0);
                        let bundle_count = database.bundle_count().unwrap_or(0);
                        let _ = progress.send(crate::common::scan::scan_types::ProgressPayload {
                            step: "map".into(),
                            message: format!(
                                "Database opened ({} asset entries, {} Bundles)",
                                asset_count, bundle_count
                            ),
                        });
                        let (material, texture_refs) = AnimatorPreviewMaterials::resolve_material(
                            database,
                            &bundle_path,
                            path_id_i64,
                            &mat_name,
                            &progress,
                        );
                        (vec![material], texture_refs)
                    }
                    None => {
                        return Err(
                            "AssetMap database not found, please build Map first".to_string()
                        );
                    }
                }
            } else {
                (
                    vec![MaterialInfo {
                        name: mat_name.clone(),
                        ..Default::default()
                    }],
                    Vec::new(),
                )
            };

        let texture_count = texture_refs.len();

        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: "resolve_deps".into(),
                message: format!(
                    "Material '{}': found {} Texture2D(s)",
                    mat_name, texture_count
                ),
            })
            .ok();

        if cancel_token.load(Ordering::SeqCst) {
            return Err("Task cancelled".to_string());
        }

        //
        // Step 3: Export related Texture2D as PNG sidecar files using preview exporter
        //
        let output_root = Path::new(&output_dir);
        let material_folder = output_root.join(&mat_name);
        fs::create_dir_all(&material_folder)
            .map_err(|e| format!("Failed to create output directory: {}", e))?;

        let texture_paths = if options.include_textures {
            AnimatorPreviewMaterials::export_textures(
                &mut materials,
                &texture_refs,
                &material_folder,
                &progress,
                task_id,
                "Export Material Dependencies",
                cancel_token,
            )
            .iter()
            .map(|candidate| candidate.png_path.clone())
            .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: "textures".into(),
                message: format!("Texture export complete: {} files", texture_paths.len()),
            })
            .ok();
        Self::emit_dependency_progress(
            task_id,
            "Export Material Dependencies",
            "Export Textures",
            3,
            4,
            &format!("Texture export complete: {} files", texture_paths.len()),
        );

        if cancel_token.load(Ordering::SeqCst) {
            return Err("Task cancelled".to_string());
        }

        //
        // Step 4: Export Material properties as JSON
        //
        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: "material_json".into(),
                message: "Exporting Material properties as JSON...".into(),
            })
            .ok();
        Self::emit_dependency_progress(
            task_id,
            "Export Material Dependencies",
            "Write Material",
            4,
            4,
            "Exporting Material properties as JSON...",
        );

        let json_path = material_folder.join(format!("{}_material.json", mat_name));
        let export_material = materials
            .into_iter()
            .next()
            .unwrap_or_else(|| MaterialInfo {
                name: mat_name.clone(),
                ..Default::default()
            });
        let json_content = serde_json::to_string_pretty(&export_material)
            .map_err(|e| format!("Serialize Material failed: {}", e))?;
        fs::write(&json_path, json_content.as_bytes())
            .map_err(|e| format!("Write Material JSON failed: {}", e))?;

        let json_path_str = json_path.to_string_lossy().to_string();

        progress
            .send(crate::common::scan::scan_types::ProgressPayload {
                step: "done".into(),
                message: format!("Export complete: {}", json_path_str),
            })
            .ok();

        Ok(DependencyExportResult {
            output_path: json_path_str,
            glb_path: String::new(),
            texture_paths,
            material_count: 1,
            texture_count,
            vertex_count: 0,
            triangle_count: 0,
        })
    }

    // ================================================================
    // CSV Report
    // ================================================================

    fn write_csv_report(
        job: &ExportJob,
        results: &[ExportResult],
        _start: &Instant,
    ) -> Result<(), String> {
        let path =
            Path::new(&job.options.output_dir).join(format!("export_report_{}.csv", job.job_id));
        let mut f = fs::File::create(&path).map_err(|e| format!("Create report: {}", e))?;
        writeln!(f, "name,class,path_id,format,success,error,output,bytes,ms")
            .map_err(|e| format!("Write report: {}", e))?;
        for r in results {
            writeln!(
                f,
                "{},{},{},{:?},{},{},{},{},{}",
                r.asset_name,
                r.class_name,
                r.path_id,
                r.format,
                r.success,
                r.error.as_deref().unwrap_or(""),
                r.output_file,
                r.byte_size,
                r.duration_ms
            )
            .map_err(|e| format!("Write report: {}", e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ExportService;
    use crate::common::export::common_types::{
        ExportAssetRef, ExportFormat, ExportGroupBy, ExportOptions,
    };
    use std::collections::HashMap;
    use std::fs;

    #[test]
    fn build_output_path_sanitizes_names_and_stays_under_output_dir() {
        let output_dir = std::env::temp_dir().join(format!(
            "assetdaemon-export-path-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&output_dir);
        fs::create_dir_all(&output_dir).expect("create output dir");

        let options = ExportOptions {
            output_dir: output_dir.to_string_lossy().to_string(),
            group_by: ExportGroupBy::ByContainer,
            overwrite_existing: false,
            generate_report: false,
            format_overrides: HashMap::new(),
            ..Default::default()
        };
        let asset_ref = ExportAssetRef {
            bundle_path: String::new(),
            path_id: "42".to_string(),
            class_name: "TextAsset".to_string(),
            asset_name: "..\\..\\evil:folder\\bad*name".to_string(),
        };

        let path = ExportService::build_output_path(&options, &asset_ref, &ExportFormat::Txt)
            .expect("build output path");
        let resolved_parent = path.parent().unwrap().canonicalize().unwrap();
        assert!(resolved_parent.starts_with(output_dir.canonicalize().unwrap()));
        assert_eq!(path.file_name().unwrap().to_string_lossy(), "bad_name.txt");

        let _ = fs::remove_dir_all(output_dir);
    }

    #[test]
    fn split_submesh_output_path_keeps_first_file_as_primary() {
        let base = std::path::Path::new(r"C:\exports\Hero.glb");

        assert_eq!(
            ExportService::split_submesh_output_path(base, 0)
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "Hero.glb"
        );
        assert_eq!(
            ExportService::split_submesh_output_path(base, 1)
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "Hero_submesh_002.glb"
        );
    }

    #[test]
    fn game_object_batch_export_ignores_stale_non_glb_override() {
        let mut options = ExportOptions::default();
        options.format_overrides =
            HashMap::from([("GameObject".to_string(), ExportFormat::JsonTree)]);

        assert_eq!(
            ExportService::resolve_format("GameObject", &options),
            ExportFormat::Glb
        );
    }

    #[test]
    #[ignore = "diagnostic full GameObject batch export on local Naraka asset graph"]
    fn real_game_object_batch_export_uses_preview_glb_workflow() {
        use crate::common::export::common_types::{ExportJob, ExportProgress};
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        let output_dir = std::env::temp_dir().join(format!(
            "assetdaemon-real-gameobject-export-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&output_dir);
        fs::create_dir_all(&output_dir).expect("create output dir");

        let mut format_overrides = HashMap::new();
        format_overrides.insert("GameObject".to_string(), ExportFormat::JsonTree);
        let job = ExportJob {
            job_id: "real_game_object_batch_export_uses_preview_glb_workflow".to_string(),
            assets: vec![ExportAssetRef {
                bundle_path: r"C:\Users\Administrator\Desktop\NarakaDecrypt\6\6\660019f01e306713"
                    .to_string(),
                path_id: "-337566547204893701".to_string(),
                class_name: "GameObject".to_string(),
                asset_name:
                    "assets/res/prefab/actor_visual_part/ch_f_japan_onmyoji/ch_f_japan_onmyoji_lv_s11.prefab"
                        .to_string(),
            }],
            options: ExportOptions {
                output_dir: output_dir.to_string_lossy().to_string(),
                workspace_dirs: vec![r"C:\Users\Administrator\Desktop\NarakaDecrypt".to_string()],
                asset_map_cache_root: Some(r"D:\AssetDaemonCacheFolder".to_string()),
                include_materials: true,
                include_textures: true,
                include_skeleton: true,
                include_animations: true,
                format_overrides,
                ..Default::default()
            },
        };
        let progress = tauri::ipc::Channel::<ExportProgress>::new(|_| Ok(()));
        let report = ExportService::run_batch_with_cancel_token(
            job,
            progress,
            Arc::new(AtomicBool::new(false)),
        );

        println!("{:#?}", report.results);
        assert_eq!(report.succeeded, 1);
        let output_file = &report.results[0].output_file;
        assert!(output_file.ends_with(".glb"), "{output_file}");
        assert!(std::path::Path::new(output_file).exists());

        let _ = fs::remove_dir_all(output_dir);
    }
}
