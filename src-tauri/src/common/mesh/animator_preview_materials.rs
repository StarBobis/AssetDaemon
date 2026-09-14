/*
 * Material and texture export support for Animator preview GLB generation.
 */

use crate::common::asset_map::asset_index::AssetDatabase;
use crate::common::asset_map::dependency_resolver::AssetMapDependencyResolver;
use crate::common::bundle_file::asset_bundle::{AssetBundle, AssetBundleLoader};
use crate::common::command_types::MeshPreviewTextureCandidate;
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::task::task_logger::TaskLogger;
use crate::common::unity_dependency::external_resolver::UnityExternalResolver;
use crate::exporter::material_info::MaterialInfo;
use crate::exporter::texture_exporter::TextureExporter;
use crate::unity::classes::registry::UnityClassObject;
use crate::unity::classes::texture2d::Texture2DInfo;
use crate::unity::classes::texture_format::TextureFormatConst;
use crate::unity::relations::UnityRelationKind;
use crate::utils::hash_utils::HashUtils;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AnimatorPreviewMeshRef {
    pub mesh_bundle_path: String,
    pub mesh_path_id: i64,
    pub renderer_bundle_path: String,
    pub renderer_path_id: i64,
}

pub struct AnimatorPreviewMaterials;

#[derive(Debug, Clone)]
struct TextureOccurrence {
    path_id: i64,
    bundle_path: String,
    texture_name: String,
    material_name: String,
    material_index: usize,
    slot_index: Option<usize>,
    slot_name: String,
    usage: String,
}

impl AnimatorPreviewMaterials {
    pub fn resolve(
        db: &AssetDatabase,
        mesh_refs: &[AnimatorPreviewMeshRef],
        progress: &tauri::ipc::Channel<ProgressPayload>,
    ) -> (Vec<MaterialInfo>, Vec<(i64, String, String)>) {
        let mut materials = Vec::<MaterialInfo>::new();
        let mut texture_refs = Vec::<(i64, String, String)>::new();
        let mut material_contexts = Vec::<((i64, String), String)>::new();
        let mut seen_material_contexts = HashSet::<(i64, String, String)>::new();
        let mut seen_material_refs = HashSet::<(i64, String)>::new();

        for mesh_ref in mesh_refs {
            let mesh_name = db
                .find_by_bundle_and_path_id(&mesh_ref.mesh_bundle_path, mesh_ref.mesh_path_id)
                .ok()
                .flatten()
                .map(|row| row.asset_name)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("mesh_{}", mesh_ref.mesh_path_id));
            let mut renderer_material_refs = find_renderer_material_refs(db, mesh_ref, progress);
            if renderer_material_refs.is_empty() {
                renderer_material_refs = AssetMapDependencyResolver::find_mesh_material_refs(
                    db,
                    &mesh_ref.mesh_bundle_path,
                    mesh_ref.mesh_path_id,
                    progress,
                );
            }
            if !renderer_material_refs.is_empty() {
                for material_ref in renderer_material_refs {
                    seen_material_refs.insert(material_ref.clone());
                    let context_key = (material_ref.0, material_ref.1.clone(), mesh_name.clone());
                    if seen_material_contexts.insert(context_key) {
                        material_contexts.push((material_ref, mesh_name.clone()));
                    }
                }
                continue;
            }

            match AssetMapDependencyResolver::resolve_mesh_textures(
                db,
                &mesh_ref.mesh_bundle_path,
                mesh_ref.mesh_path_id,
                &mesh_name,
                progress,
            ) {
                Ok((mut mesh_materials, mesh_texture_refs)) => {
                    materials.append(&mut mesh_materials);
                    texture_refs.extend(mesh_texture_refs);
                }
                Err(error) => {
                    progress
                        .send(ProgressPayload {
                            step: "materials".into(),
                            message: format!(
                                "Material lookup skipped for Mesh {}:{} ({})",
                                mesh_ref.mesh_bundle_path, mesh_ref.mesh_path_id, error
                            ),
                        })
                        .ok();
                }
            }
        }
        if !material_contexts.is_empty() {
            progress
                .send(ProgressPayload {
                    step: "materials".into(),
                    message: format!(
                        "Resolving {} Material context(s) from {} unique Material reference(s) across {} Mesh renderer(s)",
                        material_contexts.len(),
                        seen_material_refs.len(),
                        mesh_refs.len()
                    ),
                })
                .ok();
            for (material_ref, mesh_name) in material_contexts {
                match AssetMapDependencyResolver::resolve_material_refs(
                    db,
                    &[material_ref.clone()],
                    &mesh_name,
                    progress,
                ) {
                    Ok((resolved_materials, resolved_texture_refs)) => {
                        materials.extend(resolved_materials);
                        texture_refs.extend(resolved_texture_refs);
                    }
                    Err(error) => {
                        progress
                            .send(ProgressPayload {
                                step: "materials".into(),
                                message: format!(
                                    "Material lookup failed for Mesh '{}' ({})",
                                    mesh_name, error
                                ),
                            })
                            .ok();
                    }
                }
            }
        }
        (materials, texture_refs)
    }

    pub fn resolve_material(
        db: &AssetDatabase,
        material_bundle_path: &str,
        material_path_id: i64,
        material_name: &str,
        progress: &tauri::ipc::Channel<ProgressPayload>,
    ) -> (MaterialInfo, Vec<(i64, String, String)>) {
        match AssetMapDependencyResolver::resolve_material_refs(
            db,
            &[(material_path_id, material_bundle_path.to_string())],
            material_name,
            progress,
        ) {
            Ok((materials, texture_refs)) => (
                materials
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| MaterialInfo {
                        name: material_name.to_string(),
                        ..Default::default()
                    }),
                texture_refs,
            ),
            Err(error) => {
                progress
                    .send(ProgressPayload {
                        step: "materials".into(),
                        message: format!(
                            "Material preview dependency lookup failed for '{}' ({})",
                            material_name, error
                        ),
                    })
                    .ok();
                (
                    MaterialInfo {
                        name: material_name.to_string(),
                        ..Default::default()
                    },
                    Vec::new(),
                )
            }
        }
    }

    pub fn export_textures(
        materials: &mut [MaterialInfo],
        texture_refs: &[(i64, String, String)],
        output_folder: &Path,
        progress: &tauri::ipc::Channel<ProgressPayload>,
        task_id: &str,
        task_label: &str,
        cancel_token: &Arc<AtomicBool>,
    ) -> Vec<MeshPreviewTextureCandidate> {
        let mut candidates = Vec::new();
        let texture_occurrences = texture_occurrences_from_materials(materials, texture_refs);
        let mut bundle_cache = HashMap::<String, AssetBundle>::new();
        let mut texture_cache = HashMap::<(String, i64), Texture2DInfo>::new();
        let mut png_cache = HashMap::<(String, i64), String>::new();
        let mut png_name_by_md5 = HashMap::<String, String>::new();
        let mut candidate_by_md5 = HashSet::<String>::new();
        let mut unique_png_count = 0usize;
        let mut md5_reuse_count = 0usize;
        TaskLogger::progress(
            task_id,
            task_label,
            "Export Textures",
            0,
            texture_occurrences.len().max(1),
            &format!(
                "Preparing {} preview texture occurrence(s)",
                texture_occurrences.len()
            ),
        );

        for (index, occurrence) in texture_occurrences.iter().enumerate() {
            if cancel_token.load(Ordering::SeqCst) {
                break;
            }
            TaskLogger::progress(
                task_id,
                task_label,
                "Export Textures",
                index + 1,
                texture_occurrences.len().max(1),
                &format!(
                    "Exporting texture {}/{}: {} (path_id={})",
                    index + 1,
                    texture_occurrences.len(),
                    occurrence.texture_name,
                    occurrence.path_id
                ),
            );
            progress
                .send(ProgressPayload {
                    step: "textures".into(),
                    message: format!(
                        "Exporting Animator preview texture {}/{}: {}",
                        index + 1,
                        texture_occurrences.len(),
                        occurrence.texture_name
                    ),
                })
                .ok();

            let slot_name = if occurrence.slot_name.is_empty() {
                "texture"
            } else {
                occurrence.slot_name.as_str()
            };
            let png_name = safe_texture_png_file_name(
                &occurrence.texture_name,
                occurrence.path_id,
                slot_name,
                index,
            );
            let png_path = output_folder.join(&png_name);
            match export_texture_to_png_file_cached(
                &mut bundle_cache,
                &mut texture_cache,
                &mut png_cache,
                &occurrence.bundle_path,
                occurrence.path_id,
                &png_path,
                cancel_token,
                task_id,
                task_label,
                progress,
            ) {
                Ok(()) => {
                    let md5 = match HashUtils::file_md5_cancelable(
                        &png_path.to_string_lossy(),
                        Some(cancel_token),
                    ) {
                        Ok(md5) => md5,
                        Err(error) => {
                            TaskLogger::warn(
                                task_id,
                                task_label,
                                &format!("Preview texture hash failed: {} ({})", png_name, error),
                            );
                            continue;
                        }
                    };
                    let (effective_png_name, reused_png, is_unique_candidate) =
                        register_preview_png_md5(
                            &mut png_name_by_md5,
                            &mut candidate_by_md5,
                            &md5,
                            &png_name,
                        );
                    let effective_png_path = output_folder.join(&effective_png_name);
                    if reused_png {
                        md5_reuse_count += 1;
                    } else {
                        unique_png_count += 1;
                    }
                    replace_material_texture_occurrence_path(
                        materials,
                        occurrence.material_index,
                        occurrence.slot_index,
                        occurrence.path_id,
                        &effective_png_name,
                    );
                    if is_unique_candidate {
                        candidates.push(animator_texture_candidate_from_cache(
                            &texture_cache,
                            &occurrence.bundle_path,
                            occurrence.path_id,
                            &occurrence.texture_name,
                            &effective_png_path,
                            Some(occurrence.material_name.as_str()),
                            slot_name,
                            Some(occurrence.usage.as_str()),
                            occurrence.material_index,
                        ));
                    }
                }
                Err(error) => {
                    TaskLogger::warn(
                        task_id,
                        task_label,
                        &format!(
                            "Preview texture skipped: {}:{} ({})",
                            occurrence.bundle_path, occurrence.path_id, error
                        ),
                    );
                    progress
                        .send(ProgressPayload {
                            step: "textures".into(),
                            message: format!(
                                "Preview texture skipped: {} (path_id={}, error={})",
                                occurrence.texture_name, occurrence.path_id, error
                            ),
                        })
                        .ok();
                }
            }
        }

        let summary = format!(
            "Texture occurrence export done: occurrences={}, candidates={}, uniquePng={}, md5Reused={}",
            texture_occurrences.len(),
            candidates.len(),
            unique_png_count,
            md5_reuse_count
        );
        TaskLogger::progress(
            task_id,
            task_label,
            "Export Textures",
            texture_occurrences.len(),
            texture_occurrences.len().max(1),
            &summary,
        );
        TaskLogger::info(task_id, task_label, &summary);
        progress
            .send(ProgressPayload {
                step: "textures".into(),
                message: summary,
            })
            .ok();

        candidates
    }
}

fn register_preview_png_md5(
    png_name_by_md5: &mut HashMap<String, String>,
    candidate_by_md5: &mut HashSet<String>,
    md5: &str,
    png_name: &str,
) -> (String, bool, bool) {
    let reused_png = png_name_by_md5.contains_key(md5);
    let effective_png_name = png_name_by_md5
        .entry(md5.to_string())
        .or_insert_with(|| png_name.to_string())
        .clone();
    let is_unique_candidate = candidate_by_md5.insert(md5.to_string());
    (effective_png_name, reused_png, is_unique_candidate)
}

fn find_renderer_material_refs(
    db: &AssetDatabase,
    mesh_ref: &AnimatorPreviewMeshRef,
    _progress: &tauri::ipc::Channel<ProgressPayload>,
) -> Vec<(i64, String)> {
    if mesh_ref.renderer_bundle_path.trim().is_empty() || mesh_ref.renderer_path_id == 0 {
        return Vec::new();
    }

    let mut refs = Vec::new();
    for material in db
        .find_relations_by_bundle_and_source(
            UnityRelationKind::RENDERER_MATERIAL,
            &mesh_ref.renderer_bundle_path,
            mesh_ref.renderer_path_id,
        )
        .unwrap_or_default()
    {
        if material.target_path_id == 0 {
            continue;
        }
        let bundle_path = if !material.target_bundle_path.is_empty() {
            resolve_external_name_or_path(db, &material.target_bundle_path)
                .unwrap_or_else(|| material.target_bundle_path.clone())
        } else if material.file_id == 0 {
            material.bundle_path.clone()
        } else {
            match db.find_externals_by_bundle(&material.bundle_path) {
                Ok(externals) => externals
                    .into_iter()
                    .find(|external| external.file_id == material.file_id)
                    .and_then(|external| resolve_external_name_or_path(db, &external.path_name)),
                Err(_) => None,
            }
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| material.bundle_path.clone())
        };
        refs.push((material.target_path_id, bundle_path));
    }
    refs
}

fn resolve_external_name_or_path(db: &AssetDatabase, external_name: &str) -> Option<String> {
    let trimmed = external_name.trim();
    if trimmed.is_empty() {
        return None;
    }
    if Path::new(trimmed).exists() {
        return Some(trimmed.to_string());
    }
    UnityExternalResolver::resolve_from_db(db, trimmed)
}

fn export_texture_to_png_file_cached(
    bundle_cache: &mut HashMap<String, AssetBundle>,
    texture_cache: &mut HashMap<(String, i64), Texture2DInfo>,
    png_cache: &mut HashMap<(String, i64), String>,
    bundle_path: &str,
    texture_path_id: i64,
    output_path: &Path,
    cancel_token: &Arc<AtomicBool>,
    task_id: &str,
    task_label: &str,
    progress: &tauri::ipc::Channel<ProgressPayload>,
) -> Result<(), String> {
    check_cancelled(cancel_token)?;
    let key = (bundle_path.to_string(), texture_path_id);
    if let Some(cached_png_path) = png_cache.get(&key) {
        if Path::new(cached_png_path).exists() {
            if cached_png_path != &output_path.to_string_lossy().to_string() {
                TaskLogger::progress(
                    task_id,
                    task_label,
                    "Export Textures",
                    1,
                    1,
                    &format!("Reusing cached Texture2D PNG: {}", output_path.display()),
                );
                std::fs::copy(cached_png_path, output_path)
                    .map_err(|e| format!("Failed to reuse cached Texture2D PNG: {}", e))?;
            }
            check_cancelled(cancel_token)?;
            return Ok(());
        }
    }
    if !bundle_cache.contains_key(bundle_path) {
        progress
            .send(ProgressPayload {
                step: "textures".into(),
                message: format!(
                    "Loading Texture2D bundle: {}",
                    bundle_display_name(bundle_path)
                ),
            })
            .ok();
        TaskLogger::info(
            task_id,
            task_label,
            &format!("Loading Texture2D bundle: {}", bundle_path),
        );
        let bundle = AssetBundleLoader::load_bundle(Path::new(bundle_path))
            .map_err(|e| format!("Failed to load Texture2D bundle: {}", e))?;
        bundle_cache.insert(bundle_path.to_string(), bundle);
    }
    check_cancelled(cancel_token)?;
    let bundle = bundle_cache
        .get(bundle_path)
        .ok_or_else(|| format!("Texture bundle cache miss: {}", bundle_path))?;
    if !texture_cache.contains_key(&key) {
        TaskLogger::progress(
            task_id,
            task_label,
            "Parse Texture",
            1,
            4,
            &format!("Parsing Texture2D object path_id={}", texture_path_id),
        );
        let info = parse_texture_from_bundle(bundle, texture_path_id)?;
        texture_cache.insert(key.clone(), info);
    }
    check_cancelled(cancel_token)?;
    let info = texture_cache
        .get(&key)
        .ok_or_else(|| format!("Texture cache miss: {}:{}", bundle_path, texture_path_id))?;
    export_texture_info_to_png_file(
        info,
        output_path,
        cancel_token,
        task_id,
        task_label,
        progress,
    )?;
    png_cache.insert(key, output_path.to_string_lossy().to_string());
    Ok(())
}

fn parse_texture_from_bundle(
    bundle: &AssetBundle,
    texture_path_id: i64,
) -> Result<Texture2DInfo, String> {
    match bundle.parse_object_by_path_id(texture_path_id)? {
        UnityClassObject::Texture2D(texture) => Ok(texture.data),
        other => {
            return Err(format!(
                "Object path_id={} is {}, not Texture2D",
                texture_path_id,
                other.class_name()
            ))
        }
    }
}

pub(crate) fn export_texture_info_to_png_file(
    info: &Texture2DInfo,
    output_path: &Path,
    cancel_token: &Arc<AtomicBool>,
    task_id: &str,
    task_label: &str,
    progress: &tauri::ipc::Channel<ProgressPayload>,
) -> Result<(), String> {
    TaskLogger::progress(
        task_id,
        task_label,
        "Decode Texture",
        2,
        4,
        &format!(
            "Decoding Texture2D pixels: {}x{} format={}",
            info.width, info.height, info.texture_format
        ),
    );
    let rgba = TextureExporter::decode_pixels_to_rgba_cancelable(
        &info.pixel_data,
        info.texture_format,
        info.width,
        info.height,
        Some(cancel_token),
    )?;
    progress
        .send(ProgressPayload {
            step: "textures".into(),
            message: format!("Scaling Texture2D preview: {}x{}", info.width, info.height),
        })
        .ok();
    TaskLogger::progress(
        task_id,
        task_label,
        "Scale Texture",
        3,
        4,
        "Scaling Texture2D preview",
    );
    let display_rgba = TextureExporter::flip_rgba_vertical_cancelable(
        &rgba,
        info.width,
        info.height,
        Some(cancel_token),
    )?;
    let (preview_rgba, preview_width, preview_height) =
        TextureExporter::downscale_rgba_nearest_cancelable(
            &display_rgba,
            info.width,
            info.height,
            2048,
            Some(cancel_token),
        )?;
    TaskLogger::progress(
        task_id,
        task_label,
        "Encode Texture",
        4,
        4,
        &format!("Writing Texture2D PNG: {}", output_path.display()),
    );
    TextureExporter::save_png_cancelable(
        &preview_rgba,
        preview_width,
        preview_height,
        output_path,
        Some(cancel_token),
    )
}

fn check_cancelled(cancel_token: &Arc<AtomicBool>) -> Result<(), String> {
    if cancel_token.load(Ordering::SeqCst) {
        Err("Task cancelled".to_string())
    } else {
        Ok(())
    }
}

fn bundle_display_name(bundle_path: &str) -> &str {
    Path::new(bundle_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(bundle_path)
}

fn texture_occurrences_from_materials(
    materials: &[MaterialInfo],
    texture_refs: &[(i64, String, String)],
) -> Vec<TextureOccurrence> {
    let mut name_by_texture_ref = HashMap::<(String, i64), String>::new();
    let mut refs_by_path_id = HashMap::<i64, Vec<(String, String)>>::new();
    for (path_id, bundle_path, texture_name) in texture_refs {
        name_by_texture_ref
            .entry((bundle_path.clone(), *path_id))
            .or_insert_with(|| texture_name.clone());
        refs_by_path_id
            .entry(*path_id)
            .or_default()
            .push((bundle_path.clone(), texture_name.clone()));
    }

    let mut occurrences = Vec::new();
    for (material_index, material) in materials.iter().enumerate() {
        for (slot_index, slot) in material.textures.iter().enumerate() {
            let Some(decoded) = decode_texture_ref_for_preview(&slot.relative_path) else {
                continue;
            };
            let resolved = decoded
                .bundle_path
                .as_ref()
                .and_then(|bundle_path| {
                    name_by_texture_ref
                        .get(&(bundle_path.clone(), decoded.path_id))
                        .map(|texture_name| (bundle_path.clone(), texture_name.clone()))
                })
                .or_else(|| {
                    refs_by_path_id
                        .get(&decoded.path_id)
                        .and_then(|items| items.first())
                        .map(|(bundle_path, texture_name)| {
                            (bundle_path.clone(), texture_name.clone())
                        })
                });
            let Some((bundle_path, texture_name)) = resolved else {
                continue;
            };
            occurrences.push(TextureOccurrence {
                path_id: decoded.path_id,
                bundle_path,
                texture_name,
                material_name: material.name.clone(),
                material_index,
                slot_index: Some(slot_index),
                slot_name: slot.slot_name.clone(),
                usage: slot.usage.clone(),
            });
        }
    }

    if !occurrences.is_empty() {
        return occurrences;
    }

    texture_refs
        .iter()
        .enumerate()
        .map(
            |(index, (path_id, bundle_path, texture_name))| TextureOccurrence {
                path_id: *path_id,
                bundle_path: bundle_path.clone(),
                texture_name: texture_name.clone(),
                material_name: String::new(),
                material_index: index,
                slot_index: None,
                slot_name: String::new(),
                usage: String::new(),
            },
        )
        .collect()
}

fn animator_texture_candidate_from_cache(
    texture_cache: &HashMap<(String, i64), Texture2DInfo>,
    texture_bundle_path: &str,
    texture_path_id: i64,
    texture_name: &str,
    png_path: &Path,
    material_name: Option<&str>,
    slot_name: &str,
    usage: Option<&str>,
    material_index: usize,
) -> MeshPreviewTextureCandidate {
    let texture_data = texture_cache.get(&(texture_bundle_path.to_string(), texture_path_id));

    let (
        width,
        height,
        texture_format,
        texture_format_name,
        byte_size,
        has_alpha,
        color_space_name,
        wrap_mode_name,
        filter_mode_name,
    ) = if let Some(info) = texture_data {
        (
            info.width,
            info.height,
            info.texture_format,
            TextureFormatConst::format_name(info.texture_format).to_string(),
            info.image_data_size,
            TextureFormatConst::has_alpha(info.texture_format),
            TextureFormatConst::color_space_name(info.color_space).to_string(),
            TextureFormatConst::wrap_mode_name(info.wrap_u).to_string(),
            TextureFormatConst::filter_mode_name(info.filter_mode).to_string(),
        )
    } else {
        (
            0,
            0,
            0,
            String::new(),
            0,
            false,
            String::new(),
            String::new(),
            String::new(),
        )
    };

    MeshPreviewTextureCandidate {
        png_path: png_path.to_string_lossy().to_string(),
        texture_path_id: texture_path_id.to_string(),
        texture_name: texture_name.to_string(),
        texture_bundle_path: texture_bundle_path.to_string(),
        material_name: material_name.unwrap_or("").to_string(),
        material_index,
        slot_name: slot_name.to_string(),
        usage: usage.unwrap_or("").to_string(),
        width,
        height,
        texture_format,
        texture_format_name,
        byte_size,
        has_alpha,
        color_space_name,
        wrap_mode_name,
        filter_mode_name,
    }
}

fn replace_material_texture_occurrence_path(
    materials: &mut [MaterialInfo],
    material_index: usize,
    slot_index: Option<usize>,
    texture_path_id: i64,
    png_name: &str,
) {
    if let Some(slot_index) = slot_index {
        if let Some(slot) = materials
            .get_mut(material_index)
            .and_then(|material| material.textures.get_mut(slot_index))
        {
            if texture_ref_path_id(&slot.relative_path) == Some(texture_path_id) {
                slot.file_name = png_name.to_string();
                slot.relative_path = png_name.to_string();
            }
        }
        return;
    }

    for material in materials {
        for slot in &mut material.textures {
            if texture_ref_path_id(&slot.relative_path) == Some(texture_path_id) {
                slot.file_name = png_name.to_string();
                slot.relative_path = png_name.to_string();
            }
        }
    }
}

#[derive(Debug, Clone)]
struct DecodedPreviewTextureRef {
    path_id: i64,
    bundle_path: Option<String>,
}

fn decode_texture_ref_for_preview(relative_path: &str) -> Option<DecodedPreviewTextureRef> {
    let inner = relative_path.strip_prefix('#')?;
    let (id_part, bundle_path) = inner
        .split_once('@')
        .map(|(id_part, bundle)| {
            (
                id_part,
                (!bundle.trim().is_empty()).then(|| bundle.to_string()),
            )
        })
        .unwrap_or((inner, None));
    let path_id_part = id_part
        .split_once(':')
        .map(|(_, path_id)| path_id)
        .unwrap_or(id_part);
    Some(DecodedPreviewTextureRef {
        path_id: path_id_part.parse::<i64>().ok()?,
        bundle_path,
    })
}

fn texture_ref_path_id(relative_path: &str) -> Option<i64> {
    let inner = relative_path.strip_prefix('#')?;
    let id_part = inner.split('@').next().unwrap_or(inner);
    let path_id_part = id_part
        .split_once(':')
        .map(|(_, path_id)| path_id)
        .unwrap_or(id_part);
    path_id_part.parse::<i64>().ok()
}

pub fn safe_preview_file_stem(name: &str, fallback: &str) -> String {
    let value = name
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .trim_matches('_')
        .to_string();
    if value.is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

fn safe_texture_png_file_name(
    texture_name: &str,
    path_id: i64,
    slot_name: &str,
    occurrence_index: usize,
) -> String {
    let stem = safe_preview_file_stem(texture_name, &format!("tex_{}", path_id));
    let slot = safe_preview_file_stem(slot_name, "");
    if slot.is_empty() {
        format!("{}_{}_c{}.png", stem, path_id, occurrence_index + 1)
    } else {
        format!(
            "{}_{}_{}_c{}.png",
            stem,
            path_id,
            slot,
            occurrence_index + 1
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::asset_map::asset_index::{
        AssetDatabase, AssetWriteRow, BundleInternalNameWriteRow, ExternalWriteRow,
        MapBundleWriteRows, RelationWriteRow,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace(name: &str) -> std::path::PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "assetfinder_animator_materials_{}_{}",
            name, suffix
        ))
    }

    fn map_bundle(bundle_path: &str) -> MapBundleWriteRows {
        MapBundleWriteRows {
            bundle_path: bundle_path.to_string(),
            md5: bundle_path.to_string(),
            file_size: 1,
            modified_ms: 1,
            unity_version: "2019.4.41f2".to_string(),
            asset_count: 0,
            assets: Vec::new(),
            containers: Vec::new(),
            externals: Vec::new(),
            internal_names: Vec::new(),
            relations: Vec::new(),
        }
    }

    fn asset(bundle_path: &str, path_id: i64, class_id: i32, class_name: &str) -> AssetWriteRow {
        AssetWriteRow {
            bundle_path: bundle_path.to_string(),
            path_id,
            class_id,
            class_name: class_name.to_string(),
            asset_name: class_name.to_string(),
            byte_size: 1,
        }
    }

    fn asset_named(
        bundle_path: &str,
        path_id: i64,
        class_id: i32,
        class_name: &str,
        asset_name: &str,
    ) -> AssetWriteRow {
        AssetWriteRow {
            bundle_path: bundle_path.to_string(),
            path_id,
            class_id,
            class_name: class_name.to_string(),
            asset_name: asset_name.to_string(),
            byte_size: 1,
        }
    }

    #[test]
    fn renderer_context_finds_materials_when_mesh_lives_in_another_bundle() {
        let workspace = temp_workspace("cross_bundle_renderer_material");
        std::fs::create_dir_all(&workspace).unwrap();
        let db = AssetDatabase::open(&workspace).unwrap();
        let renderer_bundle = workspace
            .join("animator_bundle")
            .to_string_lossy()
            .to_string();
        let mesh_bundle = workspace.join("mesh_bundle").to_string_lossy().to_string();
        let material_bundle = workspace
            .join("material_bundle")
            .to_string_lossy()
            .to_string();

        let mut renderer_rows = map_bundle(&renderer_bundle);
        renderer_rows.assets = vec![asset(&renderer_bundle, 300, 137, "SkinnedMeshRenderer")];
        renderer_rows.relations = vec![RelationWriteRow {
            bundle_path: renderer_bundle.clone(),
            relation_type: UnityRelationKind::RENDERER_MATERIAL.into(),
            source_path_id: 300,
            target_path_id: 500,
            source_name: String::new(),
            target_name: String::new(),
            file_id: 0,
            field_path: "".into(),
            target_bundle_path: material_bundle.clone(),
        }];

        let mut mesh_rows = map_bundle(&mesh_bundle);
        mesh_rows.assets = vec![asset(&mesh_bundle, 400, 43, "Mesh")];

        let mut material_rows = map_bundle(&material_bundle);
        material_rows.assets = vec![asset(&material_bundle, 500, 21, "Material")];

        db.insert_many_map_bundle_write_rows(&[renderer_rows, mesh_rows, material_rows])
            .unwrap();

        let progress = tauri::ipc::Channel::<ProgressPayload>::new(|_| Ok(()));
        let refs = find_renderer_material_refs(
            &db,
            &AnimatorPreviewMeshRef {
                mesh_bundle_path: mesh_bundle,
                mesh_path_id: 400,
                renderer_bundle_path: renderer_bundle,
                renderer_path_id: 300,
            },
            &progress,
        );

        assert_eq!(refs, vec![(500, material_bundle)]);

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn renderer_context_resolves_assetstudio_archive_material_external() {
        let workspace = temp_workspace("renderer_material_archive_external");
        std::fs::create_dir_all(&workspace).unwrap();
        let db = AssetDatabase::open(&workspace).unwrap();
        let renderer_bundle = workspace
            .join("renderer_bundle")
            .to_string_lossy()
            .to_string();
        let material_bundle = workspace
            .join("material_bundle")
            .to_string_lossy()
            .to_string();

        let mut renderer_rows = map_bundle(&renderer_bundle);
        renderer_rows.assets = vec![asset(&renderer_bundle, 300, 137, "SkinnedMeshRenderer")];
        renderer_rows.externals = vec![ExternalWriteRow {
            bundle_path: renderer_bundle.clone(),
            sf_index: 0,
            file_id: 1,
            path_name: "archive:/cab-material/cab-material|cab-material".to_string(),
        }];
        renderer_rows.relations = vec![RelationWriteRow {
            bundle_path: renderer_bundle.clone(),
            relation_type: UnityRelationKind::RENDERER_MATERIAL.into(),
            source_path_id: 300,
            target_path_id: 500,
            source_name: String::new(),
            target_name: String::new(),
            file_id: 1,
            field_path: "m_Materials".into(),
            target_bundle_path: String::new(),
        }];

        let mut material_rows = map_bundle(&material_bundle);
        material_rows.assets = vec![asset(&material_bundle, 500, 21, "Material")];
        material_rows.internal_names = vec![BundleInternalNameWriteRow {
            bundle_path: material_bundle.clone(),
            name: "cab-material".to_string(),
            kind: "node".to_string(),
        }];

        db.insert_many_map_bundle_write_rows(&[renderer_rows, material_rows])
            .unwrap();

        let progress = tauri::ipc::Channel::<ProgressPayload>::new(|_| Ok(()));
        let refs = find_renderer_material_refs(
            &db,
            &AnimatorPreviewMeshRef {
                mesh_bundle_path: renderer_bundle.clone(),
                mesh_path_id: 400,
                renderer_bundle_path: renderer_bundle,
                renderer_path_id: 300,
            },
            &progress,
        );

        assert_eq!(refs, vec![(500, material_bundle)]);

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn resolve_deduplicates_material_refs_across_mesh_renderer_chains() {
        let workspace = temp_workspace("dedupe_mesh_material_refs");
        std::fs::create_dir_all(&workspace).unwrap();
        let db = AssetDatabase::open(&workspace).unwrap();
        let model_bundle = workspace.join("model_bundle").to_string_lossy().to_string();
        let material_bundle = workspace
            .join("material_bundle")
            .to_string_lossy()
            .to_string();

        let mut model_rows = map_bundle(&model_bundle);
        model_rows.assets = vec![
            asset(&model_bundle, 101, 43, "Mesh"),
            asset(&model_bundle, 102, 43, "Mesh"),
            asset(&model_bundle, 201, 137, "SkinnedMeshRenderer"),
            asset(&model_bundle, 202, 137, "SkinnedMeshRenderer"),
        ];
        model_rows.relations = vec![
            RelationWriteRow {
                bundle_path: model_bundle.clone(),
                relation_type: UnityRelationKind::RENDERER_MESH.into(),
                source_path_id: 10,
                target_path_id: 20,
                source_name: String::new(),
                target_name: String::new(),
                file_id: 0,
                field_path: "".into(),
                target_bundle_path: String::new(),
            },
            RelationWriteRow {
                bundle_path: model_bundle.clone(),
                relation_type: UnityRelationKind::RENDERER_MESH.to_string(),
                source_path_id: 202,
                target_path_id: 102,
                source_name: String::new(),
                target_name: String::new(),
                file_id: 0,
                field_path: String::new(),
                target_bundle_path: String::new(),
            },
            RelationWriteRow {
                bundle_path: model_bundle.clone(),
                relation_type: UnityRelationKind::RENDERER_MATERIAL.into(),
                source_path_id: 10,
                target_path_id: 20,
                source_name: String::new(),
                target_name: String::new(),
                file_id: 0,
                field_path: "".into(),
                target_bundle_path: material_bundle.clone(),
            },
            RelationWriteRow {
                bundle_path: model_bundle.clone(),
                relation_type: UnityRelationKind::RENDERER_MATERIAL.to_string(),
                source_path_id: 202,
                target_path_id: 500,
                source_name: String::new(),
                target_name: String::new(),
                file_id: 0,
                field_path: String::new(),
                target_bundle_path: material_bundle.clone(),
            },
        ];

        let mut material_rows = map_bundle(&material_bundle);
        material_rows.assets = vec![asset(&material_bundle, 500, 21, "SharedMaterial")];

        db.insert_many_map_bundle_write_rows(&[model_rows, material_rows])
            .unwrap();

        let progress = tauri::ipc::Channel::<ProgressPayload>::new(|_| Ok(()));
        let (materials, texture_refs) = AnimatorPreviewMaterials::resolve(
            &db,
            &[
                AnimatorPreviewMeshRef {
                    mesh_bundle_path: model_bundle.clone(),
                    mesh_path_id: 101,
                    renderer_bundle_path: String::new(),
                    renderer_path_id: 0,
                },
                AnimatorPreviewMeshRef {
                    mesh_bundle_path: model_bundle,
                    mesh_path_id: 102,
                    renderer_bundle_path: String::new(),
                    renderer_path_id: 0,
                },
            ],
            &progress,
        );

        assert_eq!(materials.len(), 1);
        assert_eq!(materials[0].name, "SharedMaterial");
        assert!(texture_refs.is_empty());

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn resolve_uses_each_mesh_name_when_shared_material_needs_texture_fallback() {
        let workspace = temp_workspace("mesh_context_texture_fallback");
        std::fs::create_dir_all(&workspace).unwrap();
        let db = AssetDatabase::open(&workspace).unwrap();
        let model_bundle = workspace.join("model_bundle").to_string_lossy().to_string();
        let material_bundle = workspace
            .join("missing_material_bundle")
            .to_string_lossy()
            .to_string();
        let texture_bundle = workspace
            .join("texture_bundle")
            .to_string_lossy()
            .to_string();

        let mut model_rows = map_bundle(&model_bundle);
        model_rows.assets = vec![
            asset_named(&model_bundle, 101, 43, "Mesh", "ch_body03_mesh"),
            asset_named(&model_bundle, 102, 43, "Mesh", "ch_hair03_mesh"),
            asset(&model_bundle, 201, 137, "SkinnedMeshRenderer"),
            asset(&model_bundle, 202, 137, "SkinnedMeshRenderer"),
        ];
        model_rows.relations = vec![
            RelationWriteRow {
                bundle_path: model_bundle.clone(),
                relation_type: UnityRelationKind::RENDERER_MATERIAL.into(),
                source_path_id: 201,
                target_path_id: 500,
                source_name: String::new(),
                target_name: String::new(),
                file_id: 0,
                field_path: String::new(),
                target_bundle_path: material_bundle.clone(),
            },
            RelationWriteRow {
                bundle_path: model_bundle.clone(),
                relation_type: UnityRelationKind::RENDERER_MATERIAL.into(),
                source_path_id: 202,
                target_path_id: 500,
                source_name: String::new(),
                target_name: String::new(),
                file_id: 0,
                field_path: String::new(),
                target_bundle_path: material_bundle.clone(),
            },
        ];

        let mut material_rows = map_bundle(&material_bundle);
        material_rows.assets = vec![asset_named(
            &material_bundle,
            500,
            21,
            "Material",
            "shared_missing_material",
        )];

        let mut texture_rows = map_bundle(&texture_bundle);
        texture_rows.assets = vec![
            asset_named(&texture_bundle, 901, 28, "Texture2D", "ch_body03_mesh_d"),
            asset_named(&texture_bundle, 902, 28, "Texture2D", "ch_hair03_mesh_d"),
        ];

        db.insert_many_map_bundle_write_rows(&[model_rows, material_rows, texture_rows])
            .unwrap();

        let progress = tauri::ipc::Channel::<ProgressPayload>::new(|_| Ok(()));
        let (materials, texture_refs) = AnimatorPreviewMaterials::resolve(
            &db,
            &[
                AnimatorPreviewMeshRef {
                    mesh_bundle_path: model_bundle.clone(),
                    mesh_path_id: 101,
                    renderer_bundle_path: model_bundle.clone(),
                    renderer_path_id: 201,
                },
                AnimatorPreviewMeshRef {
                    mesh_bundle_path: model_bundle.clone(),
                    mesh_path_id: 102,
                    renderer_bundle_path: model_bundle,
                    renderer_path_id: 202,
                },
            ],
            &progress,
        );

        let texture_names = texture_refs
            .iter()
            .map(|(_, _, name)| name.as_str())
            .collect::<HashSet<_>>();

        assert_eq!(materials.len(), 2);
        assert_eq!(texture_refs.len(), 2);
        assert!(texture_names.contains("ch_body03_mesh_d"));
        assert!(texture_names.contains("ch_hair03_mesh_d"));
        assert_eq!(
            materials
                .iter()
                .map(|material| material.textures.len())
                .sum::<usize>(),
            2
        );

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn texture_occurrences_keep_repeated_material_slots_until_content_hash_dedupe() {
        let materials = vec![MaterialInfo {
            name: "shared_mat".to_string(),
            textures: vec![
                crate::exporter::material_info::TextureSlot {
                    slot_name: "_MainTex".to_string(),
                    usage: "DiffuseColor".to_string(),
                    file_name: "tex_901.png".to_string(),
                    relative_path: "#0:901@bundle".to_string(),
                },
                crate::exporter::material_info::TextureSlot {
                    slot_name: "_MainTex#2".to_string(),
                    usage: "DiffuseColor".to_string(),
                    file_name: "tex_902.png".to_string(),
                    relative_path: "#0:902@bundle".to_string(),
                },
            ],
            ..Default::default()
        }];
        let texture_refs = vec![
            (901, "bundle".to_string(), "body_d".to_string()),
            (902, "bundle".to_string(), "body_d".to_string()),
        ];

        let occurrences = texture_occurrences_from_materials(&materials, &texture_refs);

        assert_eq!(occurrences.len(), 2);
        assert_eq!(occurrences[0].path_id, 901);
        assert_eq!(occurrences[0].slot_name, "_MainTex");
        assert_eq!(occurrences[1].path_id, 902);
        assert_eq!(occurrences[1].slot_name, "_MainTex#2");
    }

    #[test]
    fn preview_png_md5_registry_reuses_png_and_suppresses_duplicate_candidate() {
        let mut png_name_by_md5 = HashMap::new();
        let mut candidate_by_md5 = HashSet::new();

        let first = register_preview_png_md5(
            &mut png_name_by_md5,
            &mut candidate_by_md5,
            "same-md5",
            "first.png",
        );
        let second = register_preview_png_md5(
            &mut png_name_by_md5,
            &mut candidate_by_md5,
            "same-md5",
            "second.png",
        );

        assert_eq!(first, ("first.png".to_string(), false, true));
        assert_eq!(second, ("first.png".to_string(), true, false));
    }

    #[test]
    fn glb_preview_texture_export_writes_display_oriented_png() {
        let workspace = temp_workspace("display_oriented_png");
        std::fs::create_dir_all(&workspace).unwrap();
        let output_path = workspace.join("texture.png");
        let cancel_token = Arc::new(AtomicBool::new(false));
        let progress = tauri::ipc::Channel::<ProgressPayload>::new(|_| Ok(()));
        let info = Texture2DInfo {
            width: 2,
            height: 2,
            complete_image_size: 16,
            texture_format: TextureFormatConst::RGBA32,
            mip_count: 1,
            is_readable: true,
            streaming_mipmaps: false,
            image_count: 1,
            texture_dimension: 2,
            filter_mode: 1,
            aniso: 1,
            wrap_u: 1,
            wrap_v: 1,
            color_space: 1,
            image_data_size: 16,
            pixel_data: vec![
                10, 0, 0, 255, 20, 0, 0, 255, //
                30, 0, 0, 255, 40, 0, 0, 255,
            ],
        };

        export_texture_info_to_png_file(
            &info,
            &output_path,
            &cancel_token,
            "test",
            "Test",
            &progress,
        )
        .expect("texture export");

        let image = image::open(&output_path).expect("png").to_rgba8();
        assert_eq!(image.get_pixel(0, 0).0[0], 30);
        assert_eq!(image.get_pixel(1, 0).0[0], 40);
        assert_eq!(image.get_pixel(0, 1).0[0], 10);
        assert_eq!(image.get_pixel(1, 1).0[0], 20);

        let _ = std::fs::remove_dir_all(workspace);
    }
}
