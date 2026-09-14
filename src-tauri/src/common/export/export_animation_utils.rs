/*
 * export_animation_utils.rs -- animation discovery helpers for GLB export.
 *
 * Unity scenes usually connect a Mesh to AnimationClips through this chain:
 * Mesh <- SkinnedMeshRenderer -> GameObject -> Animator -> AnimatorController -> AnimationClip.
 * The GLB exporter needs the final AnimationClip bundles before it can parse channels.
 */

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use crate::common::asset_map::asset_index::{AssetDatabase, RelationRow};
use crate::common::bundle_file::asset_bundle::{AssetBundle, AssetBundleLoader};
use crate::common::command_types::{PreviewAnimationClipRef, PreviewAnimatorRef};
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::task::task_logger::TaskLogger;
use crate::common::unity_dependency::external_resolver::UnityExternalResolver;
use crate::exporter::animation_clip_exporter::{AnimationClipExporter, GlbAnimation};
use crate::exporter::mesh_exporter::MeshBlendShape;
use crate::exporter::model_context::{GlbSkeleton, ModelContextResolver};
use crate::unity::classes::object::PPtr;
use crate::unity::classes::registry::{UnityClassObject, UnityClassParser};
use crate::unity::relations::UnityRelationKind;
use crate::unity::type_tree::unity_value::UnityValue;

use tauri::ipc::Channel;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AnimationClipRef {
    bundle_path: String,
    path_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ComponentRef {
    bundle_path: String,
    path_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ObjectRef {
    bundle_path: String,
    path_id: i64,
}

pub struct ExportAnimationUtils;

impl ExportAnimationUtils {
    /// Resolve Animator/Animation component references without parsing Meshes or clips.
    pub fn collect_preview_animators(
        asset_class_name: &str,
        bundle_path: &str,
        path_id: i64,
        database: &AssetDatabase,
    ) -> Vec<PreviewAnimatorRef> {
        Self::collect_preview_animators_with_progress(
            asset_class_name,
            bundle_path,
            path_id,
            database,
            None,
        )
    }

    pub fn collect_preview_animators_with_progress(
        asset_class_name: &str,
        bundle_path: &str,
        path_id: i64,
        database: &AssetDatabase,
        progress: Option<&Channel<ProgressPayload>>,
    ) -> Vec<PreviewAnimatorRef> {
        let mut component_refs = Vec::new();
        let mut seen = HashSet::new();

        Self::emit_preview_animator_log(
            progress,
            format!(
                "Animator lookup: class={}, path_id={}, bundle={}",
                asset_class_name,
                path_id,
                Path::new(bundle_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(bundle_path)
            ),
        );

        match asset_class_name {
            "Animator" | "Animation" => {
                Self::push_unique_component_ref(
                    &mut component_refs,
                    &mut seen,
                    ComponentRef {
                        bundle_path: bundle_path.to_string(),
                        path_id,
                    },
                );
            }
            "GameObject" => {
                Self::emit_preview_animator_log(
                    progress,
                    "Animator lookup: reading GameObject components".to_string(),
                );
                for component_ref in
                    Self::component_refs_for_game_object(database, bundle_path, path_id)
                {
                    Self::push_unique_component_ref(&mut component_refs, &mut seen, component_ref);
                }
                Self::emit_preview_animator_log(
                    progress,
                    format!(
                        "Animator lookup GameObject components done: {} component ref(s)",
                        component_refs.len()
                    ),
                );
            }
            "Mesh" => {
                for relation in Self::mesh_component_relations(database, bundle_path, path_id) {
                    let component_bundle_path = relation.bundle_path.clone();
                    let component_path_id = relation.source_path_id;
                    for game_object_id in Self::game_object_ids_for_component(
                        database,
                        &component_bundle_path,
                        component_path_id,
                    ) {
                        for component_ref in Self::component_refs_for_game_object(
                            database,
                            &component_bundle_path,
                            game_object_id,
                        ) {
                            Self::push_unique_component_ref(
                                &mut component_refs,
                                &mut seen,
                                component_ref,
                            );
                        }
                    }
                }
            }
            "Renderer" | "MeshRenderer" | "SkinnedMeshRenderer" | "MeshFilter" | "Transform" => {
                for game_object_id in
                    Self::game_object_ids_for_component(database, bundle_path, path_id)
                {
                    for component_ref in
                        Self::component_refs_for_game_object(database, bundle_path, game_object_id)
                    {
                        Self::push_unique_component_ref(
                            &mut component_refs,
                            &mut seen,
                            component_ref,
                        );
                    }
                }
            }
            _ => {}
        }

        let animators = Self::preview_animator_refs(database, &component_refs);
        Self::emit_preview_animator_log(
            progress,
            format!("Animator lookup complete: {} animator(s)", animators.len()),
        );
        animators
    }

    /// Resolve related AnimationClip references without parsing animation curves or rebuilding GLB.
    pub fn collect_preview_animation_clips(
        asset_class_name: &str,
        bundle_path: &str,
        path_id: i64,
        database: &AssetDatabase,
        progress: Option<&Channel<ProgressPayload>>,
    ) -> Vec<PreviewAnimationClipRef> {
        let mut clip_refs = Vec::new();
        let mut seen = HashSet::new();
        let mut seen_controllers = HashSet::new();

        match asset_class_name {
            "AnimationClip" => Self::push_unique_clip_ref(
                &mut clip_refs,
                &mut seen,
                AnimationClipRef {
                    bundle_path: bundle_path.to_string(),
                    path_id,
                },
            ),
            "Animator" | "Animation" => {
                Self::collect_clip_refs_from_component(
                    database,
                    bundle_path,
                    path_id,
                    &mut clip_refs,
                    &mut seen,
                    &mut seen_controllers,
                );
                Self::collect_clip_refs_from_component_object(
                    database,
                    bundle_path,
                    path_id,
                    &mut clip_refs,
                    &mut seen,
                    &mut seen_controllers,
                    progress,
                );
            }
            "GameObject" => {
                for component_ref in
                    Self::component_refs_for_game_object(database, bundle_path, path_id)
                {
                    Self::collect_clip_refs_from_component(
                        database,
                        &component_ref.bundle_path,
                        component_ref.path_id,
                        &mut clip_refs,
                        &mut seen,
                        &mut seen_controllers,
                    );
                    Self::collect_clip_refs_from_component_object(
                        database,
                        &component_ref.bundle_path,
                        component_ref.path_id,
                        &mut clip_refs,
                        &mut seen,
                        &mut seen_controllers,
                        progress,
                    );
                }
            }
            "Mesh" => Self::collect_clip_refs_related_to_mesh(
                database,
                bundle_path,
                path_id,
                &mut clip_refs,
                &mut seen,
                progress,
            ),
            "Renderer" | "MeshRenderer" | "SkinnedMeshRenderer" | "MeshFilter" | "Transform" => {
                for game_object_id in
                    Self::game_object_ids_for_component(database, bundle_path, path_id)
                {
                    for component_ref in
                        Self::component_refs_for_game_object(database, bundle_path, game_object_id)
                    {
                        Self::collect_clip_refs_from_component(
                            database,
                            &component_ref.bundle_path,
                            component_ref.path_id,
                            &mut clip_refs,
                            &mut seen,
                            &mut seen_controllers,
                        );
                        Self::collect_clip_refs_from_component_object(
                            database,
                            &component_ref.bundle_path,
                            component_ref.path_id,
                            &mut clip_refs,
                            &mut seen,
                            &mut seen_controllers,
                            progress,
                        );
                    }
                }
            }
            "AnimatorController" | "RuntimeAnimatorController" | "AnimatorOverrideController" => {
                Self::collect_clip_refs_from_controller_object(
                    database,
                    bundle_path,
                    path_id,
                    &mut clip_refs,
                    &mut seen,
                    &mut seen_controllers,
                );
                Self::collect_clip_refs_from_controller_legacy_relations(
                    database,
                    bundle_path,
                    path_id,
                    &mut clip_refs,
                    &mut seen,
                    &mut seen_controllers,
                );
            }
            _ => {}
        }

        Self::preview_clip_refs(database, &clip_refs)
    }

    /// Collect AnimationClips starting from an Animator or legacy Animation component.
    ///
    /// Animator preview already knows the selected Animator path_id, so this direct walk is
    /// more reliable than rediscovering the Animator from Mesh reverse relations.
    #[allow(dead_code)]
    pub fn extract_component_animations(
        component_bundle_path: &str,
        component_path_id: i64,
        database: Option<&AssetDatabase>,
        skeleton: &GlbSkeleton,
        blend_shapes: &[MeshBlendShape],
        progress: Option<&Channel<ProgressPayload>>,
    ) -> Vec<GlbAnimation> {
        let morph_target_names: Vec<String> = blend_shapes
            .iter()
            .map(|shape| shape.name.clone())
            .collect();
        let mut animations = Vec::new();
        let mut visited_bundle_paths = HashSet::new();

        Self::append_component_animations(
            &mut animations,
            &mut visited_bundle_paths,
            component_bundle_path,
            component_path_id,
            database,
            skeleton,
            &morph_target_names,
            progress,
        );

        animations
    }

    pub fn append_component_animations(
        animations: &mut Vec<GlbAnimation>,
        visited_bundle_paths: &mut HashSet<String>,
        component_bundle_path: &str,
        component_path_id: i64,
        database: Option<&AssetDatabase>,
        skeleton: &GlbSkeleton,
        morph_target_names: &[String],
        progress: Option<&Channel<ProgressPayload>>,
    ) {
        let animation_count_before = animations.len();
        if let Some(database) = database {
            let mut clip_refs = Vec::new();
            let mut seen = HashSet::new();
            let mut seen_controllers = HashSet::new();
            Self::collect_clip_refs_from_component(
                database,
                component_bundle_path,
                component_path_id,
                &mut clip_refs,
                &mut seen,
                &mut seen_controllers,
            );
            if clip_refs.is_empty() {
                Self::collect_clip_refs_from_component_object(
                    database,
                    component_bundle_path,
                    component_path_id,
                    &mut clip_refs,
                    &mut seen,
                    &mut seen_controllers,
                    progress,
                );
            }
            if let Some(progress) = progress {
                let _ = progress.send(ProgressPayload {
                    step: "animation".into(),
                    message: format!(
                        "Animator component exact AnimationClip refs: {}",
                        clip_refs.len()
                    ),
                });
            }
            if !clip_refs.is_empty() {
                Self::append_clip_ref_animations(
                    animations,
                    visited_bundle_paths,
                    &clip_refs,
                    skeleton,
                    morph_target_names,
                    progress,
                );
            }
            if animations.len() == animation_count_before {
                if let Some(progress) = progress {
                    let _ = progress.send(ProgressPayload {
                        step: "animation".into(),
                        message:
                            "Animator exact AnimationClip refs resolved no compatible channels"
                                .into(),
                    });
                }
            }
        }
    }

    /// Parse only one selected AnimationClip against the current skeleton.
    pub fn extract_selected_preview_animation(
        clip_bundle_path: &str,
        clip_path_id: i64,
        skeleton: &GlbSkeleton,
        morph_target_names: &[String],
        progress: Option<&Channel<ProgressPayload>>,
    ) -> Vec<GlbAnimation> {
        Self::extract_selected_preview_animation_with_hash_aliases(
            clip_bundle_path,
            clip_path_id,
            skeleton,
            morph_target_names,
            None,
            progress,
        )
    }

    /// Parse only one selected AnimationClip against the current skeleton, using optional
    /// Animator Avatar TOS binding hashes as aliases for GLB skeleton joints.
    pub fn extract_selected_preview_animation_with_hash_aliases(
        clip_bundle_path: &str,
        clip_path_id: i64,
        skeleton: &GlbSkeleton,
        morph_target_names: &[String],
        hash_aliases: Option<&HashMap<u32, usize>>,
        progress: Option<&Channel<ProgressPayload>>,
    ) -> Vec<GlbAnimation> {
        if !Path::new(clip_bundle_path).exists() {
            if let Some(progress) = progress {
                let _ = progress.send(ProgressPayload {
                    step: "animation".into(),
                    message: format!(
                        "Selected AnimationClip bundle not found: {}",
                        clip_bundle_path
                    ),
                });
            }
            return Vec::new();
        }
        let Ok(bundle) = AssetBundleLoader::load_bundle(Path::new(clip_bundle_path)) else {
            if let Some(progress) = progress {
                let _ = progress.send(ProgressPayload {
                    step: "animation".into(),
                    message: format!(
                        "Failed to load selected AnimationClip bundle: {}",
                        clip_bundle_path
                    ),
                });
            }
            return Vec::new();
        };
        let mut path_ids = HashSet::new();
        path_ids.insert(clip_path_id);
        let parsed = AnimationClipExporter::extract_from_bundle_path_ids_with_hash_aliases(
            &bundle,
            &path_ids,
            skeleton,
            morph_target_names,
            hash_aliases,
        );
        let total_channels: usize = parsed.iter().map(|a| a.channels.len()).sum();
        if let Some(progress) = progress {
            let hash_info = hash_aliases.map(|aliases| {
                let non_root = aliases.values().filter(|&&v| v != 0).count();
                format!("hashMap={} nonRoot={}", aliases.len(), non_root)
            }).unwrap_or_else(|| "hashMap=none".to_string());
            let extra = if total_channels > 0 {
                let non_root: Vec<_> = parsed.iter().flat_map(|a| a.channels.iter()).filter(|c| c.node != 0).take(5).map(|c| c.node).collect();
                format!(", {} ch, joints={} {}, non-root={:?}", total_channels, skeleton.joints.len(), hash_info, non_root)
            } else {
                format!(", 0 ch, joints={} {}", skeleton.joints.len(), hash_info)
            };
            let _ = progress.send(ProgressPayload {
                step: "animation".into(),
                message: format!(
                    "Selected AnimationClip parsed: {}:{} -> {} compatible animation(s){}",
                    clip_bundle_path,
                    clip_path_id,
                    parsed.len(),
                    extra
                ),
            });
        }
        parsed
    }

    /// Append only clips directly referenced by the selected component's controller chain.
    ///
    /// Animator preview should match AssetStudio: Animator -> RuntimeAnimatorController ->
    /// AnimatorController.m_AnimationClips. It must not fall back to scanning every clip in
    /// the component bundle, because that pollutes the preview with unrelated animations.
    pub fn append_component_animations_strict(
        animations: &mut Vec<GlbAnimation>,
        visited_bundle_paths: &mut HashSet<String>,
        component_bundle_path: &str,
        component_path_id: i64,
        database: Option<&AssetDatabase>,
        skeleton: &GlbSkeleton,
        morph_target_names: &[String],
        progress: Option<&Channel<ProgressPayload>>,
    ) {
        let Some(database) = database else {
            if let Some(progress) = progress {
                let _ = progress.send(ProgressPayload {
                    step: "animation".into(),
                    message: "Animator exact clip refs unavailable: AssetMap is not open".into(),
                });
            }
            return;
        };

        let mut clip_refs = Vec::new();
        let mut seen = HashSet::new();
        let mut seen_controllers = HashSet::new();
        Self::collect_clip_refs_from_component(
            database,
            component_bundle_path,
            component_path_id,
            &mut clip_refs,
            &mut seen,
            &mut seen_controllers,
        );
        if clip_refs.is_empty() {
            Self::collect_clip_refs_from_component_object(
                database,
                component_bundle_path,
                component_path_id,
                &mut clip_refs,
                &mut seen,
                &mut seen_controllers,
                progress,
            );
        }
        if let Some(progress) = progress {
            let _ = progress.send(ProgressPayload {
                step: "animation".into(),
                message: format!(
                    "Animator component exact AnimationClip refs: {}",
                    clip_refs.len()
                ),
            });
        }
        if clip_refs.is_empty() {
            if let Some(progress) = progress {
                let _ = progress.send(ProgressPayload {
                    step: "animation".into(),
                    message: "Animator has no resolvable exact AnimationClip refs".into(),
                });
            }
            return;
        }

        Self::append_clip_ref_animations(
            animations,
            visited_bundle_paths,
            &clip_refs,
            skeleton,
            morph_target_names,
            progress,
        );
    }

    /// Append clips from Animator/Animation components attached to one GameObject.
    ///
    /// GameObject model export starts from a hierarchy root, like AssetStudio's model
    /// converter. This helper bridges that root to the same exact component clip walk
    /// used by Animator preview without falling back to unrelated bundle-wide scans.
    pub fn append_game_object_animations_strict(
        animations: &mut Vec<GlbAnimation>,
        visited_bundle_paths: &mut HashSet<String>,
        game_object_bundle_path: &str,
        game_object_path_id: i64,
        database: Option<&AssetDatabase>,
        skeleton: &GlbSkeleton,
        morph_target_names: &[String],
        progress: Option<&Channel<ProgressPayload>>,
    ) {
        let Some(database) = database else {
            if let Some(progress) = progress {
                let _ = progress.send(ProgressPayload {
                    step: "animation".into(),
                    message: "GameObject animation refs unavailable: AssetMap is not open".into(),
                });
            }
            return;
        };

        let component_refs = Self::component_refs_for_game_object(
            database,
            game_object_bundle_path,
            game_object_path_id,
        );
        if let Some(progress) = progress {
            let _ = progress.send(ProgressPayload {
                step: "animation".into(),
                message: format!(
                    "GameObject Animator/Animation components: {}",
                    component_refs.len()
                ),
            });
        }

        let mut clip_refs = Vec::new();
        let mut seen = HashSet::new();
        let mut seen_controllers = HashSet::new();
        for component_ref in component_refs {
            let before = clip_refs.len();
            Self::collect_clip_refs_from_component(
                database,
                &component_ref.bundle_path,
                component_ref.path_id,
                &mut clip_refs,
                &mut seen,
                &mut seen_controllers,
            );
            if clip_refs.len() == before {
                Self::collect_clip_refs_from_component_object(
                    database,
                    &component_ref.bundle_path,
                    component_ref.path_id,
                    &mut clip_refs,
                    &mut seen,
                    &mut seen_controllers,
                    progress,
                );
            }
        }
        if let Some(progress) = progress {
            let _ = progress.send(ProgressPayload {
                step: "animation".into(),
                message: format!("GameObject exact AnimationClip refs: {}", clip_refs.len()),
            });
        }
        Self::append_clip_ref_animations(
            animations,
            visited_bundle_paths,
            &clip_refs,
            skeleton,
            morph_target_names,
            progress,
        );
    }

    /// Append clips reachable from a renderer's exact GameObject/bone transform hierarchy.
    ///
    /// Some character prefabs host Animator on the skeleton root rather than on the mesh
    /// subtree root. In those cases we can still prove the mapping through exact Unity refs:
    /// SkinnedMeshRenderer -> m_Bones / m_RootBone -> Transform ancestors -> GameObject -> Animator.
    pub fn append_renderer_animations_strict(
        animations: &mut Vec<GlbAnimation>,
        visited_bundle_paths: &mut HashSet<String>,
        renderer_bundle_path: &str,
        renderer_path_id: i64,
        database: Option<&AssetDatabase>,
        skeleton: &GlbSkeleton,
        morph_target_names: &[String],
        progress: Option<&Channel<ProgressPayload>>,
    ) {
        let Some(database) = database else {
            if let Some(progress) = progress {
                let _ = progress.send(ProgressPayload {
                    step: "animation".into(),
                    message: "Renderer exact animation refs unavailable: AssetMap is not open"
                        .into(),
                });
            }
            return;
        };

        let game_object_roots =
            Self::renderer_animation_game_objects(database, renderer_bundle_path, renderer_path_id);
        if let Some(progress) = progress {
            let _ = progress.send(ProgressPayload {
                step: "animation".into(),
                message: format!(
                    "Renderer bone hierarchy animation roots: {} GameObject candidate(s)",
                    game_object_roots.len()
                ),
            });
        }

        let mut clip_refs = Vec::new();
        let mut seen = HashSet::new();
        let mut seen_controllers = HashSet::new();
        for game_object_ref in game_object_roots {
            let component_refs = Self::component_refs_for_game_object(
                database,
                &game_object_ref.bundle_path,
                game_object_ref.path_id,
            );
            for component_ref in component_refs {
                let before = clip_refs.len();
                Self::collect_clip_refs_from_component(
                    database,
                    &component_ref.bundle_path,
                    component_ref.path_id,
                    &mut clip_refs,
                    &mut seen,
                    &mut seen_controllers,
                );
                if clip_refs.len() == before {
                    Self::collect_clip_refs_from_component_object(
                        database,
                        &component_ref.bundle_path,
                        component_ref.path_id,
                        &mut clip_refs,
                        &mut seen,
                        &mut seen_controllers,
                        progress,
                    );
                }
            }
        }

        if let Some(progress) = progress {
            let _ = progress.send(ProgressPayload {
                step: "animation".into(),
                message: format!(
                    "Renderer hierarchy exact AnimationClip refs: {}",
                    clip_refs.len()
                ),
            });
        }

        Self::append_clip_ref_animations(
            animations,
            visited_bundle_paths,
            &clip_refs,
            skeleton,
            morph_target_names,
            progress,
        );
    }

    /// Append AnimationClips whose real curve bindings are compatible with the current skeleton.
    ///
    /// This is still an exact Unity-data match: a clip is accepted only when its serialized
    /// bindings resolve to nodes on the current skeleton or to known blend-shape targets.
    /// It does not rely on names/tokens outside Unity's own animation binding data.
    pub fn append_skeleton_compatible_animations_strict(
        animations: &mut Vec<GlbAnimation>,
        visited_bundle_paths: &mut HashSet<String>,
        database: Option<&AssetDatabase>,
        skeleton: &GlbSkeleton,
        morph_target_names: &[String],
        progress: Option<&Channel<ProgressPayload>>,
    ) {
        let Some(database) = database else {
            if let Some(progress) = progress {
                let _ = progress.send(ProgressPayload {
                    step: "animation".into(),
                    message:
                        "Skeleton-compatible exact AnimationClip scan unavailable: AssetMap is not open"
                            .into(),
                });
            }
            return;
        };

        let Ok(bundle_paths) = database.find_bundle_paths_by_class("AnimationClip") else {
            return;
        };
        if let Some(progress) = progress {
            let _ = progress.send(ProgressPayload {
                step: "animation".into(),
                message: format!(
                    "Skeleton-compatible exact AnimationClip scan: {} bundle(s)",
                    bundle_paths.len()
                ),
            });
        }

        let mut scanned_bundles = 0usize;
        let mut matched_clips = 0usize;
        let total = bundle_paths.len();

        // Filter to bundles that haven't been visited yet
        let pending: Vec<&String> = bundle_paths
            .iter()
            .filter(|bp| !visited_bundle_paths.contains(bp.as_str()) && Path::new(bp).exists())
            .collect();

        if let Some(progress) = progress {
            let _ = progress.send(ProgressPayload {
                step: "animation".into(),
                message: format!(
                    "Scanning AnimationClip bundles in parallel: {} pending / {} total",
                    pending.len(),
                    total,
                ),
            });
        }

        // Parallel scan using std::thread::scope
        let results = Mutex::new(Vec::new());
        // Use a fixed parallelism target: 8 workers or the number of pending bundles, whichever is smaller.
        let worker_count = 8usize.min(pending.len()).max(1);
        let chunk_size = (pending.len() + worker_count - 1) / worker_count;
        std::thread::scope(|scope| {
            for chunk in pending.chunks(chunk_size) {
                let chunk = chunk.to_vec();
                let results_ref = &results;
                scope.spawn(|| {
                    let mut local_results: Vec<(String, Vec<GlbAnimation>)> = Vec::new();
                    for bundle_path in chunk {
                        TaskLogger::info(
                            "animation_scan",
                            "AnimationClip",
                            &format!("Scanning: {}", bundle_path),
                        );
                        let Ok(bundle) =
                            AssetBundleLoader::load_bundle_serialized_only(Path::new(bundle_path))
                        else {
                            TaskLogger::info(
                                "animation_scan",
                                "AnimationClip",
                                &format!("  -> skipped (cannot load): {}", bundle_path),
                            );
                            continue;
                        };
                        let parsed = AnimationClipExporter::extract_from_bundle(
                            &bundle,
                            skeleton,
                            morph_target_names,
                        );
                        TaskLogger::info(
                            "animation_scan",
                            "AnimationClip",
                            &format!("  -> {} clip(s) from: {}", parsed.len(), bundle_path),
                        );
                        if !parsed.is_empty() {
                            local_results.push((bundle_path.clone(), parsed));
                        }
                    }
                    results_ref.lock().unwrap().push(local_results);
                });
            }
        });

        // Merge results
        for thread_results in results.lock().unwrap().drain(..) {
            for (bundle_path, mut clips) in thread_results {
                visited_bundle_paths.insert(bundle_path);
                matched_clips += clips.len();
                scanned_bundles += 1;
                Self::append_unique_animations(animations, &mut clips);
            }
        }

        if let Some(progress) = progress {
            let _ = progress.send(ProgressPayload {
                step: "animation".into(),
                message: format!(
                    "Skeleton-compatible exact AnimationClip scan matched {} clip(s) across {} bundle(s)",
                    matched_clips, scanned_bundles
                ),
            });
        }
    }

    /// Collect exact animations that can be proven through Unity object relations.
    pub fn extract_related_animations(
        mesh_bundle: &AssetBundle,
        mesh_bundle_path: &str,
        mesh_path_id: i64,
        skeleton_bundle_path: Option<&str>,
        database: Option<&AssetDatabase>,
        skeleton: &GlbSkeleton,
        blend_shapes: &[MeshBlendShape],
        progress: Option<&Channel<ProgressPayload>>,
    ) -> Vec<GlbAnimation> {
        let _ = mesh_bundle;
        let _ = skeleton_bundle_path;
        let morph_target_names: Vec<String> = blend_shapes
            .iter()
            .map(|shape| shape.name.clone())
            .collect();
        let mut animations = Vec::new();
        let mut visited_bundle_paths = HashSet::new();

        Self::append_related_animations(
            &mut animations,
            &mut visited_bundle_paths,
            mesh_bundle_path,
            mesh_path_id,
            database,
            skeleton,
            &morph_target_names,
            progress,
        );

        animations
    }

    pub fn append_related_animations(
        animations: &mut Vec<GlbAnimation>,
        visited_bundle_paths: &mut HashSet<String>,
        mesh_bundle_path: &str,
        mesh_path_id: i64,
        database: Option<&AssetDatabase>,
        skeleton: &GlbSkeleton,
        morph_target_names: &[String],
        progress: Option<&Channel<ProgressPayload>>,
    ) {
        let Some(database) = database else {
            if let Some(progress) = progress {
                let _ = progress.send(ProgressPayload {
                    step: "animation".into(),
                    message: "Mesh exact AnimationClip refs unavailable: AssetMap is not open"
                        .into(),
                });
            }
            return;
        };

        let mut clip_refs = Vec::new();
        let mut seen_refs = HashSet::new();
        Self::collect_clip_refs_related_to_mesh(
            database,
            mesh_bundle_path,
            mesh_path_id,
            &mut clip_refs,
            &mut seen_refs,
            progress,
        );
        if let Some(progress) = progress {
            let _ = progress.send(ProgressPayload {
                step: "animation".into(),
                message: format!("Mesh exact AnimationClip refs: {}", clip_refs.len()),
            });
        }
        Self::append_clip_ref_animations(
            animations,
            visited_bundle_paths,
            &clip_refs,
            skeleton,
            morph_target_names,
            progress,
        );
    }

    fn collect_clip_refs_related_to_mesh(
        database: &AssetDatabase,
        mesh_bundle_path: &str,
        mesh_path_id: i64,
        result: &mut Vec<AnimationClipRef>,
        seen: &mut HashSet<AnimationClipRef>,
        progress: Option<&Channel<ProgressPayload>>,
    ) {
        let mesh_component_relations =
            Self::mesh_component_relations(database, mesh_bundle_path, mesh_path_id);
        for mesh_component_relation in mesh_component_relations {
            let component_bundle_path = mesh_component_relation.bundle_path.clone();
            let component_path_id = mesh_component_relation.source_path_id;
            let game_object_ids = Self::game_object_ids_for_component(
                database,
                &component_bundle_path,
                component_path_id,
            );

            for game_object_id in game_object_ids {
                let animator_refs = Self::component_refs_for_game_object(
                    database,
                    &component_bundle_path,
                    game_object_id,
                );
                for animator_ref in animator_refs {
                    let mut seen_controllers = HashSet::new();
                    Self::collect_clip_refs_from_component(
                        database,
                        &animator_ref.bundle_path,
                        animator_ref.path_id,
                        result,
                        seen,
                        &mut seen_controllers,
                    );
                    Self::collect_clip_refs_from_component_object(
                        database,
                        &animator_ref.bundle_path,
                        animator_ref.path_id,
                        result,
                        seen,
                        &mut seen_controllers,
                        progress,
                    );
                }
            }
        }
    }

    /// 查找直接引用目标 Mesh 的组件关系。
    ///
    /// SkinnedMeshRenderer 会通过 `renderer_mesh` 直接指向 Mesh；
    /// 普通 MeshRenderer 则由同一 GameObject 上的 MeshFilter 持有 `m_Mesh`。
    fn mesh_component_relations(
        database: &AssetDatabase,
        mesh_bundle_path: &str,
        mesh_path_id: i64,
    ) -> Vec<RelationRow> {
        let mut result = Vec::new();
        for relation_type in [
            UnityRelationKind::RENDERER_MESH,
            UnityRelationKind::MESH_FILTER_MESH,
        ] {
            let relations = database
                .find_relations_by_bundle_and_target(relation_type, mesh_bundle_path, mesh_path_id)
                .unwrap_or_default();
            result.extend(relations);
        }
        result
    }

    /// Find GameObjects attached to a Renderer or Animator component.
    fn game_object_ids_for_component(
        database: &AssetDatabase,
        component_bundle_path: &str,
        component_path_id: i64,
    ) -> Vec<i64> {
        let mut result = Vec::new();

        // 1) 优先查新显式类型（无需 field_path 匹配）
        for relation_type in [
            UnityRelationKind::ANIMATOR_GAMEOBJECT,
            UnityRelationKind::ANIMATION_GAMEOBJECT,
        ] {
            if let Ok(relations) = database.find_relations_by_bundle_and_source(
                relation_type,
                component_bundle_path,
                component_path_id,
            ) {
                for relation in relations {
                    result.push(relation.target_path_id);
                }
            }
        }
        if !result.is_empty() {
            result.sort_unstable();
            result.dedup();
            return result;
        }

        // 2) 回退：旧关系类型 + field_path 过滤
        for relation_type in [
            UnityRelationKind::RENDERER_GAMEOBJECT,
            UnityRelationKind::MESH_FILTER_GAMEOBJECT,
            "transform_gameobject",
            "monobehaviour_gameobject",
            "pptr",
        ] {
            for relation in database
                .find_relations_by_bundle_and_source(
                    relation_type,
                    component_bundle_path,
                    component_path_id,
                )
                .unwrap_or_default()
            {
                if !relation.field_path.ends_with("m_GameObject") {
                    continue;
                }
                result.push(relation.target_path_id);
            }
        }
        result.sort_unstable();
        result.dedup();
        result
    }

    /// Find Animator or Animation components attached to a GameObject.
    fn component_refs_for_game_object(
        database: &AssetDatabase,
        game_object_bundle_path: &str,
        game_object_path_id: i64,
    ) -> Vec<ComponentRef> {
        let mut result = Vec::new();
        let mut seen = HashSet::new();

        for relation in database
            .find_relations_by_bundle_and_source(
                "gameobject_component",
                game_object_bundle_path,
                game_object_path_id,
            )
            .unwrap_or_default()
        {
            let target_bundle_path = if relation.target_bundle_path.trim().is_empty() {
                relation.bundle_path.clone()
            } else {
                relation.target_bundle_path.clone()
            };
            if !Self::is_animation_component(database, &target_bundle_path, relation.target_path_id)
            {
                continue;
            }
            let component_ref = ComponentRef {
                bundle_path: target_bundle_path,
                path_id: relation.target_path_id,
            };
            if seen.insert(component_ref.clone()) {
                result.push(component_ref);
            }
        }

        if !result.is_empty() || !Path::new(game_object_bundle_path).exists() {
            return result;
        }

        let Ok(bundle) =
            AssetBundleLoader::load_bundle_serialized_only(Path::new(game_object_bundle_path))
        else {
            return result;
        };
        let Some((sf, obj)) = Self::find_object(&bundle, game_object_path_id) else {
            return result;
        };
        if obj.class_id != 1 {
            return result;
        }

        let component_pptrs = UnityClassParser::parse_game_object(&sf.inner, &obj.inner)
            .map(|game_object| game_object.components)
            .unwrap_or_else(|_| Self::typetree_pptr_array_field(sf, obj, "component"));
        for component in component_pptrs {
            if component.is_null() {
                continue;
            }
            let Some(component_bundle_path) =
                Self::resolve_pptr_bundle_path(database, game_object_bundle_path, sf, component)
            else {
                continue;
            };
            if !Self::is_animation_component(database, &component_bundle_path, component.path_id) {
                continue;
            }
            let component_ref = ComponentRef {
                bundle_path: component_bundle_path,
                path_id: component.path_id,
            };
            if seen.insert(component_ref.clone()) {
                result.push(component_ref);
            }
        }

        result
    }

    fn avatar_tos_hashes(value: &UnityValue) -> HashSet<u32> {
        let UnityValue::Object(map) = value else {
            return HashSet::new();
        };
        Self::unity_object_field_values(map, "m_TOS")
            .into_iter()
            .filter_map(|value| match value {
                UnityValue::Array(items) => Some(items),
                _ => None,
            })
            .flat_map(|items| items.iter())
            .filter_map(|item| match item {
                UnityValue::Object(map) => map
                    .get("first")
                    .and_then(UnityValue::as_i64)
                    .or_else(|| map.get("key").and_then(UnityValue::as_i64))
                    .filter(|key| *key >= 0)
                    .map(|key| key as u32),
                _ => None,
            })
            .collect()
    }

    fn avatar_tos_hashes_in_bundle(bundle: &AssetBundle, path_id: i64) -> HashSet<u32> {
        let Some((sf, obj)) = Self::find_object(bundle, path_id) else {
            return HashSet::new();
        };
        let Some(value) = UnityClassParser::read_typetree_value_public(&sf.inner, &obj.inner)
        else {
            return HashSet::new();
        };
        Self::avatar_tos_hashes(&value)
    }

    fn avatar_tos_hashes_for_ref(bundle_path: &str, path_id: i64) -> HashSet<u32> {
        if !Path::new(bundle_path).exists() {
            return HashSet::new();
        }
        let Ok(bundle) = AssetBundleLoader::load_bundle_serialized_only(Path::new(bundle_path))
        else {
            return HashSet::new();
        };
        let Some((sf, obj)) = Self::find_object(&bundle, path_id) else {
            return HashSet::new();
        };
        let Some(value) = UnityClassParser::read_typetree_value_public(&sf.inner, &obj.inner)
        else {
            return HashSet::new();
        };
        Self::avatar_tos_hashes(&value)
    }

    fn avatar_tos_path_map_for_ref(bundle_path: &str, path_id: i64) -> HashMap<u32, String> {
        if !Path::new(bundle_path).exists() {
            return HashMap::new();
        }
        let Ok(bundle) = AssetBundleLoader::load_bundle_serialized_only(Path::new(bundle_path))
        else {
            return HashMap::new();
        };
        let Some((sf, obj)) = Self::find_object(&bundle, path_id) else {
            return HashMap::new();
        };
        let Some(value) = UnityClassParser::read_typetree_value_public(&sf.inner, &obj.inner)
        else {
            return HashMap::new();
        };
        Self::avatar_tos_path_map(&value)
    }

    fn avatar_tos_path_map(value: &UnityValue) -> HashMap<u32, String> {
        let UnityValue::Object(map) = value else {
            return HashMap::new();
        };
        let mut result = HashMap::new();
        for value in Self::unity_object_field_values(map, "m_TOS") {
            let UnityValue::Array(items) = value else {
                continue;
            };
            for item in items {
                let UnityValue::Object(entry) = item else {
                    continue;
                };
                let Some(hash) = entry
                    .get("first")
                    .and_then(UnityValue::as_i64)
                    .or_else(|| entry.get("key").and_then(UnityValue::as_i64))
                    .filter(|key| *key >= 0)
                    .map(|key| key as u32)
                else {
                    continue;
                };
                let Some(path) = entry
                    .get("second")
                    .and_then(Self::unity_string_value)
                    .or_else(|| entry.get("value").and_then(Self::unity_string_value))
                    .or_else(|| entry.values().find_map(Self::unity_string_value))
                    .filter(|path| !path.is_empty())
                else {
                    continue;
                };
                result.entry(hash).or_insert(path);
            }
        }
        result
    }

    fn unity_string_value(value: &UnityValue) -> Option<String> {
        match value {
            UnityValue::String(value) => Some(value.clone()),
            UnityValue::Object(map) => map.values().find_map(Self::unity_string_value),
            UnityValue::Array(items) => items.iter().find_map(Self::unity_string_value),
            _ => None,
        }
    }

    /// Walk Animator/Animation PPtrs and collect exact AnimationClip objects.
    ///
    /// 优先使用新显式关系类型（ANIMATOR_CONTROLLER / CONTROLLER_ANIMATION_CLIP 等），
    /// 无结果时回退到旧的 "pptr" + field_path 字符串匹配（兼容旧 AssetMap 数据库）。
    fn collect_clip_refs_from_component(
        database: &AssetDatabase,
        component_bundle_path: &str,
        component_path_id: i64,
        result: &mut Vec<AnimationClipRef>,
        seen: &mut HashSet<AnimationClipRef>,
        seen_controllers: &mut HashSet<(String, i64)>,
    ) {
        // 1) 优先查新显式类型：Animator → Controller
        let mut found = false;
        if let Ok(explicit) = database.find_relations_by_bundle_and_source(
            UnityRelationKind::ANIMATOR_CONTROLLER,
            component_bundle_path,
            component_path_id,
        ) {
            for relation in explicit {
                found = true;
                Self::collect_clip_refs_from_controller_relation(
                    database,
                    &relation,
                    result,
                    seen,
                    seen_controllers,
                );
            }
        }
        // 2) 查新显式类型：Animation → default clip / clips
        if let Ok(explicit) = database.find_relations_by_bundle_and_source(
            UnityRelationKind::ANIMATION_DEFAULT_CLIP,
            component_bundle_path,
            component_path_id,
        ) {
            for relation in explicit {
                found = true;
                Self::push_clip_ref_from_relation(database, &relation, result, seen);
            }
        }
        if let Ok(explicit) = database.find_relations_by_bundle_and_source(
            UnityRelationKind::ANIMATION_CLIPS,
            component_bundle_path,
            component_path_id,
        ) {
            for relation in explicit {
                found = true;
                Self::push_clip_ref_from_relation(database, &relation, result, seen);
            }
        }
        // 3) 查新显式类型：AnimatorOverrideController
        if let Ok(explicit) = database.find_relations_by_bundle_and_source(
            UnityRelationKind::OVERRIDE_BASE_CONTROLLER,
            component_bundle_path,
            component_path_id,
        ) {
            for relation in explicit {
                found = true;
                Self::collect_clip_refs_from_controller_relation(
                    database,
                    &relation,
                    result,
                    seen,
                    seen_controllers,
                );
            }
        }
        if found {
            return;
        }

        Self::collect_clip_refs_from_component_object(
            database,
            component_bundle_path,
            component_path_id,
            result,
            seen,
            seen_controllers,
            None,
        );
        if result.is_empty() {
            Self::collect_clip_refs_from_component_legacy_relations(
                database,
                component_bundle_path,
                component_path_id,
                result,
                seen,
                seen_controllers,
            );
        }

        // 4) 回退：旧的 "pptr" + field_path 字符串匹配（兼容旧数据库）
    }

    /// Fallback for stale/incomplete AssetMap relation rows: parse the selected component
    /// directly, then follow the same AssetStudio controller chain.
    fn collect_clip_refs_from_component_object(
        database: &AssetDatabase,
        component_bundle_path: &str,
        component_path_id: i64,
        result: &mut Vec<AnimationClipRef>,
        seen: &mut HashSet<AnimationClipRef>,
        seen_controllers: &mut HashSet<(String, i64)>,
        progress: Option<&Channel<ProgressPayload>>,
    ) {
        if !Path::new(component_bundle_path).exists() {
            return;
        }
        let Ok(bundle) =
            AssetBundleLoader::load_bundle_serialized_only(Path::new(component_bundle_path))
        else {
            return;
        };
        let Some((sf, obj)) = Self::find_object(&bundle, component_path_id) else {
            return;
        };
        match obj.class_id {
            95 => {
                if let Ok(animator) = UnityClassParser::parse_animator(&sf.inner, &obj.inner) {
                    if let Some(progress) = progress {
                        let _ = progress.send(ProgressPayload {
                            step: "animation".into(),
                            message: format!(
                                "Animator object parser controller ref: file_id={}, path_id={}",
                                animator.controller.file_id, animator.controller.path_id
                            ),
                        });
                    }
                    let before = result.len();
                    Self::collect_clip_refs_from_controller_pptr(
                        database,
                        component_bundle_path,
                        sf,
                        animator.controller,
                        result,
                        seen,
                        seen_controllers,
                    );
                    if result.len() > before {
                        return;
                    }
                }
                if let Some(controller) = Self::typetree_direct_pptr_field(sf, obj, "m_Controller")
                    .or_else(|| Self::typetree_pptr_field(sf, obj, "m_Controller"))
                {
                    if let Some(progress) = progress {
                        let _ = progress.send(ProgressPayload {
                            step: "animation".into(),
                            message: format!(
                                "Animator TypeTree controller ref: file_id={}, path_id={}",
                                controller.file_id, controller.path_id
                            ),
                        });
                    }
                    Self::collect_clip_refs_from_controller_pptr(
                        database,
                        component_bundle_path,
                        sf,
                        controller,
                        result,
                        seen,
                        seen_controllers,
                    );
                }
            }
            111 => {
                if let Ok(animation) = UnityClassParser::parse_animation(&sf.inner, &obj.inner) {
                    Self::push_clip_ref_from_pptr(
                        database,
                        component_bundle_path,
                        sf,
                        animation.default_animation,
                        result,
                        seen,
                    );
                    for clip in animation.animations {
                        Self::push_clip_ref_from_pptr(
                            database,
                            component_bundle_path,
                            sf,
                            clip,
                            result,
                            seen,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    pub fn animator_avatar_tos_path_map(
        database: &AssetDatabase,
        component_bundle_path: &str,
        component_path_id: i64,
    ) -> HashMap<u32, String> {
        let Some((avatar_bundle_path, avatar_path_id)) =
            Self::animator_avatar_ref(database, component_bundle_path, component_path_id)
        else {
            return HashMap::new();
        };
        Self::avatar_tos_path_map_for_ref(&avatar_bundle_path, avatar_path_id)
    }

    pub fn animator_avatar_tos_hash_aliases(
        database: &AssetDatabase,
        component_bundle_path: &str,
        component_path_id: i64,
        skeleton: &GlbSkeleton,
    ) -> HashMap<u32, usize> {
        let tos_paths =
            Self::animator_avatar_tos_path_map(database, component_bundle_path, component_path_id);
        Self::avatar_tos_hash_aliases_for_skeleton(&tos_paths, skeleton)
    }

    pub fn preview_avatar_tos_path_map(
        database: &AssetDatabase,
        asset_class_name: &str,
        bundle_path: &str,
        path_id: i64,
    ) -> HashMap<u32, String> {
        let mut result = HashMap::new();
        for animator in
            Self::collect_preview_animators(asset_class_name, bundle_path, path_id, database)
        {
            let Ok(animator_path_id) = animator.path_id.parse::<i64>() else {
                continue;
            };
            result.extend(Self::animator_avatar_tos_path_map(
                database,
                &animator.bundle_path,
                animator_path_id,
            ));
        }
        result
    }

    pub fn filter_avatar_tos_paths_for_clip(
        clip_bundle_path: &str,
        clip_path_id: i64,
        avatar_tos_paths: &HashMap<u32, String>,
    ) -> HashMap<u32, String> {
        if avatar_tos_paths.is_empty() || !Path::new(clip_bundle_path).exists() {
            return HashMap::new();
        }
        let Ok(bundle) =
            AssetBundleLoader::load_bundle_serialized_only(Path::new(clip_bundle_path))
        else {
            return HashMap::new();
        };
        let Some((sf, obj)) = Self::find_object(&bundle, clip_path_id) else {
            return HashMap::new();
        };
        let binding_hashes = AnimationClipExporter::binding_path_hashes_from_clip_object(sf, obj);
        binding_hashes
            .into_iter()
            .filter_map(|hash| {
                avatar_tos_paths
                    .get(&hash)
                    .cloned()
                    .map(|path| (hash, path))
            })
            .collect()
    }

    pub fn avatar_tos_hash_aliases_for_skeleton(
        tos_paths: &HashMap<u32, String>,
        skeleton: &GlbSkeleton,
    ) -> HashMap<u32, usize> {
        let name_to_joint = ModelContextResolver::transform_path_by_name(skeleton);
        let fallback = (!skeleton.joints.is_empty()).then_some(0);
        tos_paths
            .iter()
            .map(|(hash, path)| {
                let joint = Self::avatar_tos_joint_index(path, &name_to_joint)
                    .or(fallback)
                    .unwrap_or(0);
                (*hash, joint)
            })
            .collect()
    }

    fn avatar_tos_joint_index(
        avatar_path: &str,
        name_to_joint: &HashMap<String, usize>,
    ) -> Option<usize> {
        let normalized = avatar_path.replace('\\', "/");
        let lower = normalized.to_lowercase();
        let mut suffix = lower.as_str();
        loop {
            if let Some(index) = name_to_joint.get(suffix).copied() {
                return Some(index);
            }
            let Some((_, next_suffix)) = suffix.split_once('/') else {
                break;
            };
            suffix = next_suffix;
        }
        None
    }

    /// Build a clip-specific hash→joint map by resolving each binding hash
    /// through the Avatar TOS or skeleton path CRC lookup.
    pub fn build_clip_hash_to_joint(
        clip_bundle_path: &str,
        clip_path_id: i64,
        skeleton: &GlbSkeleton,
        avatar_tos_paths: &HashMap<u32, String>,
    ) -> HashMap<u32, usize> {
        let binding_hashes = if Path::new(clip_bundle_path).exists() {
            AssetBundleLoader::load_bundle_serialized_only(Path::new(clip_bundle_path))
                .ok()
                .and_then(|bundle| {
                    let (sf, obj) = Self::find_object(&bundle, clip_path_id)?;
                    Some(AnimationClipExporter::binding_path_hashes_from_clip_object(
                        sf, obj,
                    ))
                })
                .unwrap_or_default()
        } else {
            HashSet::new()
        };

        let name_to_joint = ModelContextResolver::transform_path_by_name(skeleton);
        let hash_to_joint =
            ModelContextResolver::transform_path_by_unity_hash(skeleton);
        let root_fallback = (!skeleton.joints.is_empty()).then_some(0);

        let mut result: HashMap<u32, usize> = HashMap::new();
        for hash in &binding_hashes {
            // 1. Direct CRC match against skeleton paths
            if let Some(&joint) = hash_to_joint.get(hash) {
                result.insert(*hash, joint);
                continue;
            }
            // 2. TOS lookup: hash → path → name match in skeleton
            if let Some(tos_path) = avatar_tos_paths.get(hash) {
                if let Some(joint) = Self::avatar_tos_joint_index(tos_path, &name_to_joint)
                    .or(root_fallback)
                {
                    result.insert(*hash, joint);
                    continue;
                }
            }
            // 3. Ultimate fallback: root
            if let Some(joint) = root_fallback {
                result.insert(*hash, joint);
            }
        }

        result
    }

    fn animator_avatar_ref(
        database: &AssetDatabase,
        component_bundle_path: &str,
        component_path_id: i64,
    ) -> Option<(String, i64)> {
        if !Path::new(component_bundle_path).exists() {
            return None;
        }
        let bundle =
            AssetBundleLoader::load_bundle_serialized_only(Path::new(component_bundle_path))
                .ok()?;
        let (sf, obj) = Self::find_object(&bundle, component_path_id)?;
        if obj.class_id != 95 {
            return None;
        }
        let animator = UnityClassParser::parse_animator(&sf.inner, &obj.inner).ok()?;
        if animator.avatar.is_null() {
            return None;
        }
        let avatar_bundle_path =
            Self::resolve_pptr_bundle_path(database, component_bundle_path, sf, animator.avatar)?;
        Some((avatar_bundle_path, animator.avatar.path_id))
    }

    fn collect_clip_refs_from_component_legacy_relations(
        database: &AssetDatabase,
        component_bundle_path: &str,
        component_path_id: i64,
        result: &mut Vec<AnimationClipRef>,
        seen: &mut HashSet<AnimationClipRef>,
        seen_controllers: &mut HashSet<(String, i64)>,
    ) {
        let controller_relations = database
            .find_relations_by_bundle_and_source("pptr", component_bundle_path, component_path_id)
            .unwrap_or_default();
        for relation in controller_relations {
            if relation.field_path.ends_with("m_Controller") {
                Self::collect_clip_refs_from_controller_relation(
                    database,
                    &relation,
                    result,
                    seen,
                    seen_controllers,
                );
            } else if let Some(clip_ref) = Self::clip_ref_from_relation(database, &relation) {
                Self::push_unique_clip_ref(result, seen, clip_ref);
            }
        }
    }

    /// Resolve RuntimeAnimatorController or AnimatorOverrideController links into exact clips.
    ///
    /// 优先使用新显式关系类型（CONTROLLER_ANIMATION_CLIP / OVERRIDE_*），
    /// 无结果时回退到旧的 "pptr" + field_path 字符串匹配。
    fn collect_clip_refs_from_controller_relation(
        database: &AssetDatabase,
        controller_relation: &RelationRow,
        result: &mut Vec<AnimationClipRef>,
        seen: &mut HashSet<AnimationClipRef>,
        seen_controllers: &mut HashSet<(String, i64)>,
    ) {
        let Some(controller_bundle_path) =
            Self::relation_target_bundle_path(database, controller_relation)
        else {
            return;
        };
        if !seen_controllers.insert((
            controller_bundle_path.clone(),
            controller_relation.target_path_id,
        )) {
            return;
        }
        let controller_asset = database
            .find_by_bundle_and_path_id(&controller_bundle_path, controller_relation.target_path_id)
            .ok()
            .flatten();
        let Some(controller_asset) = controller_asset else {
            return;
        };

        if controller_asset.class_name == "AnimationClip" {
            Self::push_unique_clip_ref(
                result,
                seen,
                AnimationClipRef {
                    bundle_path: controller_bundle_path,
                    path_id: controller_relation.target_path_id,
                },
            );
            return;
        }

        match controller_asset.class_name.as_str() {
            "AnimatorController" | "RuntimeAnimatorController" => {
                // 1) 优先查新显式类型：Controller → AnimationClip
                let mut found = false;
                if let Ok(explicit) = database.find_relations_by_bundle_and_source(
                    UnityRelationKind::CONTROLLER_ANIMATION_CLIP,
                    &controller_bundle_path,
                    controller_relation.target_path_id,
                ) {
                    for relation in explicit {
                        found = true;
                        Self::push_clip_ref_from_relation(database, &relation, result, seen);
                    }
                }
                // 2) 如果没有新类型结果且关系类型本身就是 CONTROLLER_ANIMATION_CLIP，
                //    说明动画片段所在 bundle 与 controller 不同，用旧路径重试一次
                if !found {
                    Self::collect_clip_refs_from_controller_object(
                        database,
                        &controller_bundle_path,
                        controller_relation.target_path_id,
                        result,
                        seen,
                        seen_controllers,
                    );
                    if result.is_empty() {
                        Self::collect_clip_refs_from_controller_legacy_relations(
                            database,
                            &controller_bundle_path,
                            controller_relation.target_path_id,
                            result,
                            seen,
                            seen_controllers,
                        );
                    }
                }
            }
            "AnimatorOverrideController" => {
                // 1) 优先查新显式类型：OVERRIDE_CLIP_ORIGINAL / OVERRIDE_CLIP_OVERRIDE
                let mut override_refs = Vec::new();
                let mut overridden_original_refs = HashSet::new();
                let mut found_override = false;
                if let Ok(explicit) = database.find_relations_by_bundle_and_source(
                    UnityRelationKind::OVERRIDE_CLIP_ORIGINAL,
                    &controller_bundle_path,
                    controller_relation.target_path_id,
                ) {
                    for relation in explicit {
                        found_override = true;
                        if let Some(clip_ref) = Self::clip_ref_from_relation(database, &relation) {
                            overridden_original_refs.insert(clip_ref);
                        }
                    }
                }
                if let Ok(explicit) = database.find_relations_by_bundle_and_source(
                    UnityRelationKind::OVERRIDE_CLIP_OVERRIDE,
                    &controller_bundle_path,
                    controller_relation.target_path_id,
                ) {
                    for relation in explicit {
                        found_override = true;
                        if let Some(clip_ref) = Self::clip_ref_from_relation(database, &relation) {
                            override_refs.push(clip_ref);
                        }
                    }
                }
                if found_override {
                    if override_refs.is_empty() {
                        overridden_original_refs.clear();
                    }
                    for clip_ref in override_refs {
                        Self::push_unique_clip_ref(result, seen, clip_ref);
                    }
                    // 查 base controller
                    if let Ok(explicit) = database.find_relations_by_bundle_and_source(
                        UnityRelationKind::OVERRIDE_BASE_CONTROLLER,
                        &controller_bundle_path,
                        controller_relation.target_path_id,
                    ) {
                        for base_relation in explicit {
                            let mut base_refs = Vec::new();
                            let mut base_seen = seen.clone();
                            Self::collect_clip_refs_from_controller_relation(
                                database,
                                &base_relation,
                                &mut base_refs,
                                &mut base_seen,
                                seen_controllers,
                            );
                            for clip_ref in base_refs {
                                if !overridden_original_refs.contains(&clip_ref) {
                                    Self::push_unique_clip_ref(result, seen, clip_ref);
                                }
                            }
                        }
                    }
                    return;
                }

                // 2) 回退：旧的 "pptr" + field_path 字符串匹配
                Self::collect_clip_refs_from_controller_object(
                    database,
                    &controller_bundle_path,
                    controller_relation.target_path_id,
                    result,
                    seen,
                    seen_controllers,
                );
                if result.is_empty() {
                    Self::collect_clip_refs_from_controller_legacy_relations(
                        database,
                        &controller_bundle_path,
                        controller_relation.target_path_id,
                        result,
                        seen,
                        seen_controllers,
                    );
                }
            }
            _ => {}
        }
    }

    fn collect_clip_refs_from_controller_object(
        database: &AssetDatabase,
        controller_bundle_path: &str,
        controller_path_id: i64,
        result: &mut Vec<AnimationClipRef>,
        seen: &mut HashSet<AnimationClipRef>,
        seen_controllers: &mut HashSet<(String, i64)>,
    ) {
        if !Path::new(controller_bundle_path).exists() {
            return;
        }
        let Ok(bundle) =
            AssetBundleLoader::load_bundle_serialized_only(Path::new(controller_bundle_path))
        else {
            return;
        };
        let Some((controller_sf, controller_obj)) = Self::find_object(&bundle, controller_path_id)
        else {
            return;
        };

        match controller_obj.class_id {
            74 => Self::push_unique_clip_ref(
                result,
                seen,
                AnimationClipRef {
                    bundle_path: controller_bundle_path.to_string(),
                    path_id: controller_path_id,
                },
            ),
            91 => {
                let mut clips = Vec::new();
                if let Ok(UnityClassObject::AnimatorController(controller_object)) =
                    UnityClassParser::parse_object(
                        &controller_sf.inner,
                        &controller_obj.inner,
                        &bundle.resources,
                    )
                {
                    clips = controller_object.animation_clips;
                }
                if clips.is_empty() {
                    clips = Self::typetree_pptr_array_field(
                        controller_sf,
                        controller_obj,
                        "m_AnimationClips",
                    );
                }
                for clip in clips {
                    Self::push_clip_ref_from_pptr(
                        database,
                        controller_bundle_path,
                        controller_sf,
                        clip,
                        result,
                        seen,
                    );
                }
            }
            221 => {
                let mut overridden_original_refs = HashSet::new();
                let controller_pptr =
                    if let Ok(UnityClassObject::AnimatorOverrideController(override_controller)) =
                        UnityClassParser::parse_object(
                            &controller_sf.inner,
                            &controller_obj.inner,
                            &bundle.resources,
                        )
                    {
                        for clip in override_controller.clips {
                            let override_ref = Self::clip_ref_from_pptr(
                                database,
                                controller_bundle_path,
                                controller_sf,
                                clip.override_clip,
                            );
                            if override_ref.is_some() {
                                if let Some(original_ref) = Self::clip_ref_from_pptr(
                                    database,
                                    controller_bundle_path,
                                    controller_sf,
                                    clip.original_clip,
                                ) {
                                    overridden_original_refs.insert(original_ref);
                                }
                            }
                            if let Some(clip_ref) = override_ref {
                                Self::push_unique_clip_ref(result, seen, clip_ref);
                            }
                        }
                        Some(override_controller.controller)
                    } else {
                        let override_clips = Self::typetree_pptr_array_field(
                            controller_sf,
                            controller_obj,
                            "m_OverrideClip",
                        );
                        let override_refs = override_clips
                            .into_iter()
                            .filter_map(|clip| {
                                Self::clip_ref_from_pptr(
                                    database,
                                    controller_bundle_path,
                                    controller_sf,
                                    clip,
                                )
                            })
                            .collect::<Vec<_>>();
                        if !override_refs.is_empty() {
                            for clip in Self::typetree_pptr_array_field(
                                controller_sf,
                                controller_obj,
                                "m_OriginalClip",
                            ) {
                                if let Some(clip_ref) = Self::clip_ref_from_pptr(
                                    database,
                                    controller_bundle_path,
                                    controller_sf,
                                    clip,
                                ) {
                                    overridden_original_refs.insert(clip_ref);
                                }
                            }
                        }
                        for clip_ref in override_refs {
                            Self::push_unique_clip_ref(result, seen, clip_ref);
                        }
                        Self::typetree_direct_pptr_field(
                            controller_sf,
                            controller_obj,
                            "m_Controller",
                        )
                        .or_else(|| {
                            Self::typetree_pptr_field(controller_sf, controller_obj, "m_Controller")
                        })
                    };
                if let Some(controller_pptr) = controller_pptr {
                    let mut base_refs = Vec::new();
                    let mut base_seen = seen.clone();
                    Self::collect_clip_refs_from_controller_pptr(
                        database,
                        controller_bundle_path,
                        controller_sf,
                        controller_pptr,
                        &mut base_refs,
                        &mut base_seen,
                        seen_controllers,
                    );
                    for clip_ref in base_refs {
                        if !overridden_original_refs.contains(&clip_ref) {
                            Self::push_unique_clip_ref(result, seen, clip_ref);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn collect_clip_refs_from_controller_legacy_relations(
        database: &AssetDatabase,
        controller_bundle_path: &str,
        controller_path_id: i64,
        result: &mut Vec<AnimationClipRef>,
        seen: &mut HashSet<AnimationClipRef>,
        seen_controllers: &mut HashSet<(String, i64)>,
    ) {
        let controller_pptrs = database
            .find_relations_by_bundle_and_source("pptr", controller_bundle_path, controller_path_id)
            .unwrap_or_default();
        let mut override_refs = Vec::new();
        let mut overridden_original_refs = HashSet::new();
        let mut has_override_fields = false;

        for relation in &controller_pptrs {
            if relation.field_path.contains("m_Clips")
                && relation.field_path.ends_with("m_OriginalClip")
            {
                has_override_fields = true;
                if let Some(clip_ref) = Self::clip_ref_from_relation(database, relation) {
                    overridden_original_refs.insert(clip_ref);
                }
            } else if relation.field_path.contains("m_Clips")
                && relation.field_path.ends_with("m_OverrideClip")
            {
                has_override_fields = true;
                if let Some(clip_ref) = Self::clip_ref_from_relation(database, relation) {
                    override_refs.push(clip_ref);
                }
            }
        }

        if has_override_fields {
            if override_refs.is_empty() {
                overridden_original_refs.clear();
            }
            for clip_ref in override_refs {
                Self::push_unique_clip_ref(result, seen, clip_ref);
            }
            for relation in controller_pptrs
                .iter()
                .filter(|relation| relation.field_path.ends_with("m_Controller"))
            {
                let mut base_refs = Vec::new();
                let mut base_seen = seen.clone();
                Self::collect_clip_refs_from_controller_relation(
                    database,
                    relation,
                    &mut base_refs,
                    &mut base_seen,
                    seen_controllers,
                );
                for clip_ref in base_refs {
                    if !overridden_original_refs.contains(&clip_ref) {
                        Self::push_unique_clip_ref(result, seen, clip_ref);
                    }
                }
            }
            return;
        }

        for relation in controller_pptrs {
            if relation.field_path.contains("m_AnimationClips") {
                Self::push_clip_ref_from_relation(database, &relation, result, seen);
            } else if relation.field_path.ends_with("m_Controller") {
                Self::collect_clip_refs_from_controller_relation(
                    database,
                    &relation,
                    result,
                    seen,
                    seen_controllers,
                );
            }
        }
    }

    fn collect_clip_refs_from_controller_pptr(
        database: &AssetDatabase,
        source_bundle_path: &str,
        source_sf: &crate::common::bundle_file::asset_bundle::SerializedFile,
        controller: PPtr,
        result: &mut Vec<AnimationClipRef>,
        seen: &mut HashSet<AnimationClipRef>,
        seen_controllers: &mut HashSet<(String, i64)>,
    ) {
        if controller.is_null() {
            return;
        }
        let Some(controller_bundle_path) =
            Self::resolve_pptr_bundle_path(database, source_bundle_path, source_sf, controller)
        else {
            return;
        };
        if !seen_controllers.insert((controller_bundle_path.clone(), controller.path_id)) {
            return;
        }
        Self::collect_clip_refs_from_controller_object(
            database,
            &controller_bundle_path,
            controller.path_id,
            result,
            seen,
            seen_controllers,
        );
    }

    /// Resolve a PPtr relation and add the target object if it points at an AnimationClip.
    fn push_clip_ref_from_relation(
        database: &AssetDatabase,
        relation: &RelationRow,
        result: &mut Vec<AnimationClipRef>,
        seen: &mut HashSet<AnimationClipRef>,
    ) {
        if let Some(clip_ref) = Self::clip_ref_from_relation(database, relation) {
            Self::push_unique_clip_ref(result, seen, clip_ref);
        }
    }

    fn clip_ref_from_relation(
        database: &AssetDatabase,
        relation: &RelationRow,
    ) -> Option<AnimationClipRef> {
        let bundle_path = Self::relation_target_bundle_path(database, relation)?;
        let asset = database
            .find_by_bundle_and_path_id(&bundle_path, relation.target_path_id)
            .ok()
            .flatten()?;
        if asset.class_name != "AnimationClip" {
            return None;
        }
        Some(AnimationClipRef {
            bundle_path,
            path_id: relation.target_path_id,
        })
    }

    fn push_clip_ref_from_pptr(
        database: &AssetDatabase,
        source_bundle_path: &str,
        source_sf: &crate::common::bundle_file::asset_bundle::SerializedFile,
        clip: PPtr,
        result: &mut Vec<AnimationClipRef>,
        seen: &mut HashSet<AnimationClipRef>,
    ) {
        if let Some(clip_ref) =
            Self::clip_ref_from_pptr(database, source_bundle_path, source_sf, clip)
        {
            Self::push_unique_clip_ref(result, seen, clip_ref);
        }
    }

    fn clip_ref_from_pptr(
        database: &AssetDatabase,
        source_bundle_path: &str,
        source_sf: &crate::common::bundle_file::asset_bundle::SerializedFile,
        clip: PPtr,
    ) -> Option<AnimationClipRef> {
        if clip.is_null() {
            return None;
        }
        let bundle_path =
            Self::resolve_pptr_bundle_path(database, source_bundle_path, source_sf, clip)?;
        let asset = database
            .find_by_bundle_and_path_id(&bundle_path, clip.path_id)
            .ok()
            .flatten();
        if asset
            .as_ref()
            .is_some_and(|row| row.class_name == "AnimationClip")
            || Self::bundle_object_class_id(&bundle_path, clip.path_id) == Some(74)
        {
            return Some(AnimationClipRef {
                bundle_path,
                path_id: clip.path_id,
            });
        }
        None
    }

    fn is_animation_component(database: &AssetDatabase, bundle_path: &str, path_id: i64) -> bool {
        if let Some(asset) = database
            .find_by_bundle_and_path_id(bundle_path, path_id)
            .ok()
            .flatten()
        {
            return asset.class_name == "Animator" || asset.class_name == "Animation";
        }
        matches!(
            Self::bundle_object_class_id(bundle_path, path_id),
            Some(95 | 111)
        )
    }

    fn resolve_pptr_bundle_path(
        database: &AssetDatabase,
        source_bundle_path: &str,
        source_sf: &crate::common::bundle_file::asset_bundle::SerializedFile,
        pptr: PPtr,
    ) -> Option<String> {
        UnityExternalResolver::resolve_pptr(
            database,
            source_bundle_path,
            &source_sf.inner.m_externals,
            &pptr,
        )
    }

    fn find_object<'a>(
        bundle: &'a AssetBundle,
        path_id: i64,
    ) -> Option<(
        &'a crate::common::bundle_file::asset_bundle::SerializedFile,
        &'a crate::common::bundle_file::asset_bundle::ObjectInfo,
    )> {
        bundle.assets.iter().find_map(|sf| {
            sf.objects
                .iter()
                .find(|obj| obj.path_id == path_id)
                .map(|obj| (sf, obj))
        })
    }

    fn renderer_animation_game_objects(
        database: &AssetDatabase,
        renderer_bundle_path: &str,
        renderer_path_id: i64,
    ) -> Vec<ObjectRef> {
        if !Path::new(renderer_bundle_path).exists() {
            return Vec::new();
        }
        let Ok(bundle) =
            AssetBundleLoader::load_bundle_serialized_only(Path::new(renderer_bundle_path))
        else {
            return Vec::new();
        };
        let Some((sf, obj)) = Self::find_object(&bundle, renderer_path_id) else {
            return Vec::new();
        };

        let mut result = Vec::new();
        let mut seen = HashSet::new();
        match obj.class_id {
            137 => {
                if let Ok(UnityClassObject::SkinnedMeshRenderer(renderer)) =
                    UnityClassParser::parse_object(&sf.inner, &obj.inner, &bundle.resources)
                {
                    Self::push_game_object_with_transform_ancestors(
                        database,
                        &mut result,
                        &mut seen,
                        renderer_bundle_path,
                        renderer.renderer.game_object.path_id,
                    );
                    for bone in renderer.bones {
                        Self::push_transform_ancestor_game_objects_from_pptr(
                            database,
                            &mut result,
                            &mut seen,
                            renderer_bundle_path,
                            sf,
                            bone,
                        );
                    }
                    if let Some(root_bone) = renderer.root_bone {
                        Self::push_transform_ancestor_game_objects_from_pptr(
                            database,
                            &mut result,
                            &mut seen,
                            renderer_bundle_path,
                            sf,
                            root_bone,
                        );
                    }
                    return result;
                }
            }
            23 | 119 => {
                if let Ok(UnityClassObject::Renderer(renderer)) =
                    UnityClassParser::parse_object(&sf.inner, &obj.inner, &bundle.resources)
                {
                    Self::push_game_object_with_transform_ancestors(
                        database,
                        &mut result,
                        &mut seen,
                        renderer_bundle_path,
                        renderer.game_object.path_id,
                    );
                    return result;
                }
            }
            _ => {}
        }

        if let Some(game_object) = Self::typetree_pptr_field(sf, obj, "m_GameObject") {
            Self::push_game_object_with_transform_ancestors(
                database,
                &mut result,
                &mut seen,
                renderer_bundle_path,
                game_object.path_id,
            );
        }
        for bone in Self::typetree_pptr_array_field(sf, obj, "m_Bones") {
            Self::push_transform_ancestor_game_objects_from_pptr(
                database,
                &mut result,
                &mut seen,
                renderer_bundle_path,
                sf,
                bone,
            );
        }
        if let Some(root_bone) = Self::typetree_pptr_field(sf, obj, "m_RootBone") {
            Self::push_transform_ancestor_game_objects_from_pptr(
                database,
                &mut result,
                &mut seen,
                renderer_bundle_path,
                sf,
                root_bone,
            );
        }

        result
    }

    fn push_transform_ancestor_game_objects_from_pptr(
        database: &AssetDatabase,
        result: &mut Vec<ObjectRef>,
        seen: &mut HashSet<ObjectRef>,
        source_bundle_path: &str,
        source_sf: &crate::common::bundle_file::asset_bundle::SerializedFile,
        transform: PPtr,
    ) {
        if transform.is_null() {
            return;
        }
        let Some(transform_bundle_path) =
            Self::resolve_pptr_bundle_path(database, source_bundle_path, source_sf, transform)
        else {
            return;
        };
        Self::push_transform_ancestor_game_objects(
            database,
            result,
            seen,
            &transform_bundle_path,
            transform.path_id,
        );
    }

    fn push_game_object_with_transform_ancestors(
        database: &AssetDatabase,
        result: &mut Vec<ObjectRef>,
        seen: &mut HashSet<ObjectRef>,
        game_object_bundle_path: &str,
        game_object_path_id: i64,
    ) {
        if game_object_path_id == 0 {
            return;
        }
        let game_object_ref = ObjectRef {
            bundle_path: game_object_bundle_path.to_string(),
            path_id: game_object_path_id,
        };
        if seen.insert(game_object_ref.clone()) {
            result.push(game_object_ref);
        }
        for transform_ref in Self::transform_refs_for_game_object(
            database,
            game_object_bundle_path,
            game_object_path_id,
        ) {
            Self::push_transform_ancestor_game_objects(
                database,
                result,
                seen,
                &transform_ref.bundle_path,
                transform_ref.path_id,
            );
        }
    }

    fn push_transform_ancestor_game_objects(
        database: &AssetDatabase,
        result: &mut Vec<ObjectRef>,
        seen: &mut HashSet<ObjectRef>,
        transform_bundle_path: &str,
        transform_path_id: i64,
    ) {
        if transform_path_id == 0 {
            return;
        }

        let mut current_bundle_path = transform_bundle_path.to_string();
        let mut current_transform_path_id = transform_path_id;
        let mut seen_transforms = HashSet::new();
        while current_transform_path_id != 0
            && seen_transforms.insert((current_bundle_path.clone(), current_transform_path_id))
        {
            let mut advanced = false;
            for game_object_ref in Self::game_object_refs_for_transform(
                database,
                &current_bundle_path,
                current_transform_path_id,
            ) {
                if seen.insert(game_object_ref.clone()) {
                    result.push(game_object_ref);
                }
                advanced = true;
            }

            if let Some(parent_ref) = Self::parent_transform_ref(
                database,
                &current_bundle_path,
                current_transform_path_id,
            ) {
                current_bundle_path = parent_ref.bundle_path;
                current_transform_path_id = parent_ref.path_id;
                continue;
            }

            if advanced {
                break;
            }
            break;
        }
    }

    fn transform_refs_for_game_object(
        database: &AssetDatabase,
        game_object_bundle_path: &str,
        game_object_path_id: i64,
    ) -> Vec<ObjectRef> {
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        for relation in database
            .find_relations_by_bundle_and_target(
                "transform_gameobject",
                game_object_bundle_path,
                game_object_path_id,
            )
            .unwrap_or_default()
        {
            let object_ref = ObjectRef {
                bundle_path: relation.bundle_path,
                path_id: relation.source_path_id,
            };
            if seen.insert(object_ref.clone()) {
                result.push(object_ref);
            }
        }
        if !result.is_empty() {
            return result;
        }
        if !Path::new(game_object_bundle_path).exists() {
            return result;
        }
        let Ok(bundle) =
            AssetBundleLoader::load_bundle_serialized_only(Path::new(game_object_bundle_path))
        else {
            return result;
        };
        for sf in &bundle.assets {
            for obj in &sf.objects {
                if obj.class_id != 4 {
                    continue;
                }
                let Ok(transform) = UnityClassParser::parse_transform(&sf.inner, &obj.inner) else {
                    continue;
                };
                if transform.game_object.path_id != game_object_path_id {
                    continue;
                }
                let object_ref = ObjectRef {
                    bundle_path: game_object_bundle_path.to_string(),
                    path_id: obj.path_id,
                };
                if seen.insert(object_ref.clone()) {
                    result.push(object_ref);
                }
            }
        }
        result
    }

    fn game_object_refs_for_transform(
        database: &AssetDatabase,
        transform_bundle_path: &str,
        transform_path_id: i64,
    ) -> Vec<ObjectRef> {
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        for relation in database
            .find_relations_by_bundle_and_source(
                "transform_gameobject",
                transform_bundle_path,
                transform_path_id,
            )
            .unwrap_or_default()
        {
            let target_bundle_path = if relation.target_bundle_path.trim().is_empty() {
                relation.bundle_path.clone()
            } else {
                relation.target_bundle_path.clone()
            };
            let object_ref = ObjectRef {
                bundle_path: target_bundle_path,
                path_id: relation.target_path_id,
            };
            if object_ref.path_id != 0 && seen.insert(object_ref.clone()) {
                result.push(object_ref);
            }
        }
        if !result.is_empty() {
            return result;
        }
        if !Path::new(transform_bundle_path).exists() {
            return result;
        }
        let Ok(bundle) =
            AssetBundleLoader::load_bundle_serialized_only(Path::new(transform_bundle_path))
        else {
            return result;
        };
        let Some((sf, obj)) = Self::find_object(&bundle, transform_path_id) else {
            return result;
        };
        let Ok(transform) = UnityClassParser::parse_transform(&sf.inner, &obj.inner) else {
            return result;
        };
        if transform.game_object.path_id != 0 {
            result.push(ObjectRef {
                bundle_path: transform_bundle_path.to_string(),
                path_id: transform.game_object.path_id,
            });
        }
        result
    }

    fn parent_transform_ref(
        database: &AssetDatabase,
        transform_bundle_path: &str,
        transform_path_id: i64,
    ) -> Option<ObjectRef> {
        if let Some(relation) = database
            .find_relations_by_bundle_and_source(
                "transform_parent",
                transform_bundle_path,
                transform_path_id,
            )
            .unwrap_or_default()
            .into_iter()
            .find(|relation| relation.target_path_id != 0)
        {
            return Some(ObjectRef {
                bundle_path: if relation.target_bundle_path.trim().is_empty() {
                    relation.bundle_path
                } else {
                    relation.target_bundle_path
                },
                path_id: relation.target_path_id,
            });
        }
        if !Path::new(transform_bundle_path).exists() {
            return None;
        }
        let bundle =
            AssetBundleLoader::load_bundle_serialized_only(Path::new(transform_bundle_path))
                .ok()?;
        let (sf, obj) = Self::find_object(&bundle, transform_path_id)?;
        let transform = UnityClassParser::parse_transform(&sf.inner, &obj.inner).ok()?;
        if transform.father.is_null() {
            return None;
        }
        let bundle_path =
            Self::resolve_pptr_bundle_path(database, transform_bundle_path, sf, transform.father)?;
        Some(ObjectRef {
            bundle_path,
            path_id: transform.father.path_id,
        })
    }

    fn bundle_object_class_id(bundle_path: &str, path_id: i64) -> Option<i32> {
        let bundle = AssetBundleLoader::load_bundle_serialized_only(Path::new(bundle_path)).ok()?;
        let (_, obj) = Self::find_object(&bundle, path_id)?;
        Some(obj.class_id)
    }

    fn typetree_pptr_field(
        sf: &crate::common::bundle_file::asset_bundle::SerializedFile,
        obj: &crate::common::bundle_file::asset_bundle::ObjectInfo,
        field_name: &str,
    ) -> Option<PPtr> {
        let value = crate::common::bundle_file::asset_bundle::ObjectHandle::new(sf, obj)
            .read()
            .ok()?;
        let UnityValue::Object(map) = value else {
            return None;
        };
        Self::unity_object_field_values(&map, field_name)
            .into_iter()
            .find_map(Self::unity_value_pptr)
    }

    fn typetree_direct_pptr_field(
        sf: &crate::common::bundle_file::asset_bundle::SerializedFile,
        obj: &crate::common::bundle_file::asset_bundle::ObjectInfo,
        field_name: &str,
    ) -> Option<PPtr> {
        let value = crate::common::bundle_file::asset_bundle::ObjectHandle::new(sf, obj)
            .read()
            .ok()?;
        let UnityValue::Object(map) = value else {
            return None;
        };
        map.get(field_name).and_then(Self::unity_value_pptr)
    }

    fn typetree_pptr_array_field(
        sf: &crate::common::bundle_file::asset_bundle::SerializedFile,
        obj: &crate::common::bundle_file::asset_bundle::ObjectInfo,
        field_name: &str,
    ) -> Vec<PPtr> {
        let Ok(value) = crate::common::bundle_file::asset_bundle::ObjectHandle::new(sf, obj).read()
        else {
            return Vec::new();
        };
        let UnityValue::Object(map) = value else {
            return Vec::new();
        };
        let mut result = Vec::new();
        for value in Self::unity_object_field_values(&map, field_name) {
            Self::collect_unity_value_pptrs(value, &mut result);
        }
        result
    }

    fn unity_value_pptr(value: &UnityValue) -> Option<PPtr> {
        match value {
            UnityValue::Ptr { file_id, path_id } => Some(PPtr {
                file_id: *file_id,
                path_id: *path_id,
            }),
            UnityValue::Object(map) => Self::unity_object_pptr(map)
                .or_else(|| map.values().find_map(Self::unity_value_pptr)),
            _ => None,
        }
    }

    fn unity_object_pptr(map: &HashMap<String, UnityValue>) -> Option<PPtr> {
        let file_id = map
            .get("m_FileID")
            .or_else(|| map.get("fileID"))
            .and_then(UnityValue::as_i64)?;
        let path_id = map
            .get("m_PathID")
            .or_else(|| map.get("pathID"))
            .and_then(UnityValue::as_i64)?;
        Some(PPtr {
            file_id: file_id as i32,
            path_id,
        })
    }

    fn unity_object_field_values<'a>(
        map: &'a HashMap<String, UnityValue>,
        field_name: &str,
    ) -> Vec<&'a UnityValue> {
        let mut result = Vec::new();
        Self::collect_unity_object_field_values(map, field_name, &mut result);
        result
    }

    fn collect_unity_object_field_values<'a>(
        map: &'a HashMap<String, UnityValue>,
        field_name: &str,
        result: &mut Vec<&'a UnityValue>,
    ) {
        for (key, value) in map {
            if key == field_name {
                result.push(value);
            }
            match value {
                UnityValue::Object(child) => {
                    Self::collect_unity_object_field_values(child, field_name, result);
                }
                UnityValue::Array(items) => {
                    for item in items {
                        if let UnityValue::Object(child) = item {
                            Self::collect_unity_object_field_values(child, field_name, result);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_unity_value_pptrs(value: &UnityValue, result: &mut Vec<PPtr>) {
        match value {
            UnityValue::Ptr { file_id, path_id } => result.push(PPtr {
                file_id: *file_id,
                path_id: *path_id,
            }),
            UnityValue::Array(items) => {
                for item in items {
                    Self::collect_unity_value_pptrs(item, result);
                }
            }
            UnityValue::Object(map) => {
                if let Some(pptr) = Self::unity_object_pptr(map) {
                    result.push(pptr);
                } else {
                    for value in map.values() {
                        Self::collect_unity_value_pptrs(value, result);
                    }
                }
            }
            _ => {}
        }
    }

    /// Convert AssetMap relation target data into the concrete target bundle path.
    fn relation_target_bundle_path(
        database: &AssetDatabase,
        relation: &RelationRow,
    ) -> Option<String> {
        let bundle = UnityExternalResolver::resolve_relation_row(database, relation);
        if bundle.is_empty() {
            return None;
        }
        Some(bundle)
    }

    #[cfg(test)]
    fn relation_targets_bundle(relation: &RelationRow, mesh_bundle_path: &str) -> bool {
        if !relation.target_bundle_path.is_empty() {
            relation.target_bundle_path == mesh_bundle_path
        } else {
            relation.bundle_path == mesh_bundle_path
        }
    }

    /// Load only the exact AnimationClip objects referenced by an Animator/Animation component.
    fn append_clip_ref_animations(
        animations: &mut Vec<GlbAnimation>,
        visited_bundle_paths: &mut HashSet<String>,
        clip_refs: &[AnimationClipRef],
        skeleton: &GlbSkeleton,
        morph_target_names: &[String],
        progress: Option<&Channel<ProgressPayload>>,
    ) {
        let mut bundle_order = Vec::<&str>::new();
        let mut refs_by_bundle = std::collections::HashMap::<&str, HashSet<i64>>::new();
        for clip_ref in clip_refs {
            let is_new_bundle = !refs_by_bundle.contains_key(clip_ref.bundle_path.as_str());
            refs_by_bundle
                .entry(clip_ref.bundle_path.as_str())
                .or_default()
                .insert(clip_ref.path_id);
            if is_new_bundle {
                bundle_order.push(clip_ref.bundle_path.as_str());
            }
        }

        for bundle_path in bundle_order {
            let Some(path_ids) = refs_by_bundle.get(bundle_path) else {
                continue;
            };
            if !Path::new(bundle_path).exists() {
                continue;
            }
            let Ok(bundle) = AssetBundleLoader::load_bundle(Path::new(bundle_path)) else {
                continue;
            };
            let mut parsed = AnimationClipExporter::extract_from_bundle_path_ids(
                &bundle,
                &path_ids,
                skeleton,
                morph_target_names,
            );
            if let Some(progress) = progress {
                let _ = progress.send(ProgressPayload {
                    step: "glb".into(),
                    message: format!(
                        "Animation exact scan: {} requested clips in {}, {} parsed",
                        path_ids.len(),
                        bundle_path,
                        parsed.len()
                    ),
                });
            }
            if parsed.is_empty() {
                continue;
            }
            visited_bundle_paths.insert(bundle_path.to_string());
            Self::append_unique_animations(animations, &mut parsed);
        }
    }

    fn push_unique_clip_ref(
        result: &mut Vec<AnimationClipRef>,
        seen: &mut HashSet<AnimationClipRef>,
        clip_ref: AnimationClipRef,
    ) {
        if seen.insert(clip_ref.clone()) {
            result.push(clip_ref);
        }
    }

    fn push_unique_component_ref(
        result: &mut Vec<ComponentRef>,
        seen: &mut HashSet<ComponentRef>,
        component_ref: ComponentRef,
    ) {
        if seen.insert(component_ref.clone()) {
            result.push(component_ref);
        }
    }

    fn emit_preview_animator_log(
        progress: Option<&Channel<ProgressPayload>>,
        message: impl Into<String>,
    ) {
        if let Some(progress) = progress {
            let _ = progress.send(ProgressPayload {
                step: "animator".into(),
                message: message.into(),
            });
        }
    }

    fn preview_clip_refs(
        database: &AssetDatabase,
        clip_refs: &[AnimationClipRef],
    ) -> Vec<PreviewAnimationClipRef> {
        clip_refs
            .iter()
            .filter_map(|clip_ref| {
                let asset = database
                    .find_by_bundle_and_path_id(&clip_ref.bundle_path, clip_ref.path_id)
                    .ok()
                    .flatten();
                let is_animation_clip = asset
                    .as_ref()
                    .is_some_and(|row| row.class_name == "AnimationClip")
                    || Self::bundle_object_class_id(&clip_ref.bundle_path, clip_ref.path_id)
                        == Some(74);
                if !is_animation_clip {
                    return None;
                }
                Some(PreviewAnimationClipRef {
                    bundle_path: clip_ref.bundle_path.clone(),
                    path_id: clip_ref.path_id.to_string(),
                    class_name: "AnimationClip".to_string(),
                    name: asset
                        .as_ref()
                        .map(|row| row.asset_name.clone())
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| format!("AnimationClip_{}", clip_ref.path_id)),
                    byte_size: asset.as_ref().map(|row| row.byte_size).unwrap_or(0),
                })
            })
            .collect()
    }

    fn preview_animator_refs(
        database: &AssetDatabase,
        component_refs: &[ComponentRef],
    ) -> Vec<PreviewAnimatorRef> {
        component_refs
            .iter()
            .filter_map(|component_ref| {
                let asset = database
                    .find_by_bundle_and_path_id(&component_ref.bundle_path, component_ref.path_id)
                    .ok()
                    .flatten();
                let class_name = asset
                    .as_ref()
                    .map(|row| row.class_name.clone())
                    .filter(|class_name| class_name == "Animator" || class_name == "Animation")
                    .or_else(|| {
                        Self::bundle_object_class_id(
                            &component_ref.bundle_path,
                            component_ref.path_id,
                        )
                        .and_then(|class_id| match class_id {
                            95 => Some("Animator".to_string()),
                            111 => Some("Animation".to_string()),
                            _ => None,
                        })
                    })?;
                Some(PreviewAnimatorRef {
                    bundle_path: component_ref.bundle_path.clone(),
                    path_id: component_ref.path_id.to_string(),
                    class_name,
                    name: asset
                        .as_ref()
                        .map(|row| row.asset_name.clone())
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| format!("Animator_{}", component_ref.path_id)),
                    byte_size: asset.as_ref().map(|row| row.byte_size).unwrap_or(0),
                })
            })
            .collect()
    }

    fn append_unique_animations(
        animations: &mut Vec<GlbAnimation>,
        parsed: &mut Vec<GlbAnimation>,
    ) {
        let existing_names: HashSet<String> = animations
            .iter()
            .map(|animation| animation.name.clone())
            .collect();
        parsed.retain(|animation| !existing_names.contains(&animation.name));
        animations.append(parsed);
    }
}

#[cfg(test)]
mod tests {
    use super::{AnimationClipRef, ComponentRef, ExportAnimationUtils};
    use crate::common::asset_map::asset_index::{
        AssetDatabase, AssetWriteRow, ContainerWriteRow, MapBundleWriteRows, RelationRow,
        RelationWriteRow,
    };
    use crate::common::bundle_file::asset_bundle::{AssetBundleLoader, ObjectHandle};
    use crate::exporter::animation_clip_exporter::AnimationClipExporter;
    use crate::unity::classes::registry::UnityClassParser;
    use crate::unity::relations::UnityRelationKind;
    use crate::unity::type_tree::unity_value::UnityValue;
    use std::collections::HashMap;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn same_bundle_relation_targets_selected_mesh_bundle_without_target_path() {
        let relation = RelationRow {
            bundle_path: "model.bundle".to_string(),
            relation_type: UnityRelationKind::MESH_FILTER_MESH.to_string(),
            source_path_id: 10,
            target_path_id: 20,
            source_name: String::new(),
            target_name: String::new(),
            file_id: 0,
            field_path: "m_Mesh".to_string(),
            target_bundle_path: String::new(),
        };

        assert!(ExportAnimationUtils::relation_targets_bundle(
            &relation,
            "model.bundle"
        ));
        assert!(!ExportAnimationUtils::relation_targets_bundle(
            &relation,
            "other.bundle"
        ));
    }

    #[test]
    fn explicit_target_bundle_must_match_selected_mesh_bundle() {
        let relation = RelationRow {
            bundle_path: "renderer.bundle".to_string(),
            relation_type: UnityRelationKind::RENDERER_MESH.to_string(),
            source_path_id: 10,
            target_path_id: 20,
            source_name: String::new(),
            target_name: String::new(),
            file_id: 1,
            field_path: "m_Mesh".to_string(),
            target_bundle_path: "mesh.bundle".to_string(),
        };

        assert!(ExportAnimationUtils::relation_targets_bundle(
            &relation,
            "mesh.bundle"
        ));
        assert!(!ExportAnimationUtils::relation_targets_bundle(
            &relation,
            "renderer.bundle"
        ));
    }

    #[test]
    fn animator_component_collects_controller_clip_refs_without_unrelated_bundle_clips() {
        let workspace = temp_workspace("animator-exact-clip-refs");
        std::fs::create_dir_all(&workspace).unwrap();

        let db = AssetDatabase::open(&workspace).unwrap();
        let model_bundle = workspace.join("model.bundle").to_string_lossy().to_string();
        let anim_bundle = workspace.join("anim.bundle").to_string_lossy().to_string();

        db.insert_many_map_bundle_write_rows(&[
            MapBundleWriteRows {
                bundle_path: model_bundle.clone(),
                md5: "model".to_string(),
                file_size: 1,
                modified_ms: 1,
                unity_version: "2019.4".to_string(),
                asset_count: 4,
                assets: vec![
                    asset(&model_bundle, 10, 95, "Animator", "animator"),
                    asset(&model_bundle, 20, 91, "AnimatorController", "controller"),
                    asset(&model_bundle, 30, 74, "AnimationClip", "idle"),
                    asset(&model_bundle, 50, 74, "AnimationClip", "unrelated"),
                ],
                containers: Vec::new(),
                externals: Vec::new(),
                internal_names: Vec::new(),
                relations: vec![
                    relation(&model_bundle, 10, 20, "m_Controller", ""),
                    relation(&model_bundle, 20, 30, "m_AnimationClips", ""),
                    relation(&model_bundle, 20, 40, "m_AnimationClips", &anim_bundle),
                ],
            },
            MapBundleWriteRows {
                bundle_path: anim_bundle.clone(),
                md5: "anim".to_string(),
                file_size: 1,
                modified_ms: 1,
                unity_version: "2019.4".to_string(),
                asset_count: 1,
                assets: vec![asset(&anim_bundle, 40, 74, "AnimationClip", "run")],
                containers: Vec::new(),
                externals: Vec::new(),
                internal_names: Vec::new(),
                relations: Vec::new(),
            },
        ])
        .unwrap();

        let mut refs = Vec::new();
        let mut seen_refs = HashSet::new();
        let mut seen_controllers = HashSet::new();
        ExportAnimationUtils::collect_clip_refs_from_component(
            &db,
            &model_bundle,
            10,
            &mut refs,
            &mut seen_refs,
            &mut seen_controllers,
        );

        assert_eq!(
            refs,
            vec![
                AnimationClipRef {
                    bundle_path: model_bundle.clone(),
                    path_id: 30,
                },
                AnimationClipRef {
                    bundle_path: anim_bundle,
                    path_id: 40,
                },
            ]
        );

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn animator_override_controller_prefers_override_clips_and_keeps_unoverridden_base_clips() {
        let workspace = temp_workspace("animator-override-runtime-clips");
        std::fs::create_dir_all(&workspace).unwrap();

        let db = AssetDatabase::open(&workspace).unwrap();
        let bundle = workspace.join("model.bundle").to_string_lossy().to_string();

        db.insert_many_map_bundle_write_rows(&[MapBundleWriteRows {
            bundle_path: bundle.clone(),
            md5: "model".to_string(),
            file_size: 1,
            modified_ms: 1,
            unity_version: "2019.4".to_string(),
            asset_count: 7,
            assets: vec![
                asset(&bundle, 10, 95, "Animator", "animator"),
                asset(&bundle, 20, 221, "AnimatorOverrideController", "override"),
                asset(&bundle, 30, 91, "AnimatorController", "controller"),
                asset(&bundle, 40, 74, "AnimationClip", "base-idle"),
                asset(&bundle, 50, 74, "AnimationClip", "override-idle"),
                asset(&bundle, 60, 74, "AnimationClip", "override-run"),
                asset(&bundle, 70, 74, "AnimationClip", "base-walk"),
            ],
            containers: Vec::new(),
            externals: Vec::new(),
            internal_names: Vec::new(),
            relations: vec![
                relation(&bundle, 10, 20, "m_Controller", ""),
                relation(&bundle, 20, 30, "m_Controller", ""),
                relation(&bundle, 20, 40, "m_Clips.m_OriginalClip", ""),
                relation(&bundle, 20, 60, "m_Clips.m_OverrideClip", ""),
                relation(&bundle, 30, 40, "m_AnimationClips", ""),
                relation(&bundle, 30, 70, "m_AnimationClips", ""),
            ],
        }])
        .unwrap();

        let mut refs = Vec::new();
        let mut seen_refs = HashSet::new();
        let mut seen_controllers = HashSet::new();
        ExportAnimationUtils::collect_clip_refs_from_component(
            &db,
            &bundle,
            10,
            &mut refs,
            &mut seen_refs,
            &mut seen_controllers,
        );

        assert_eq!(
            refs,
            vec![
                AnimationClipRef {
                    bundle_path: bundle.clone(),
                    path_id: 60,
                },
                AnimationClipRef {
                    bundle_path: bundle.clone(),
                    path_id: 70,
                }
            ]
        );

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn game_object_component_lookup_finds_animator_and_animation_components() {
        let workspace = temp_workspace("game-object-animation-components");
        std::fs::create_dir_all(&workspace).unwrap();

        let db = AssetDatabase::open(&workspace).unwrap();
        let bundle = workspace.join("model.bundle").to_string_lossy().to_string();

        db.insert_many_map_bundle_write_rows(&[MapBundleWriteRows {
            bundle_path: bundle.clone(),
            md5: "model".to_string(),
            file_size: 1,
            modified_ms: 1,
            unity_version: "2019.4".to_string(),
            asset_count: 4,
            assets: vec![
                asset(&bundle, 10, 1, "GameObject", "root"),
                asset(&bundle, 20, 95, "Animator", "animator"),
                asset(&bundle, 30, 111, "Animation", "legacy"),
                asset(&bundle, 40, 23, "MeshRenderer", "renderer"),
            ],
            containers: Vec::new(),
            externals: Vec::new(),
            internal_names: Vec::new(),
            relations: vec![
                RelationWriteRow {
                    bundle_path: bundle.clone(),
                    relation_type: "gameobject_component".into(),
                    source_path_id: 10,
                    target_path_id: 20,
                    source_name: String::new(),
                    target_name: String::new(),
                    file_id: 0,
                    field_path: "".into(),
                    target_bundle_path: String::new(),
                },
                RelationWriteRow {
                    bundle_path: bundle.clone(),
                    relation_type: "gameobject_component".to_string(),
                    source_path_id: 10,
                    target_path_id: 30,
                    source_name: String::new(),
                    target_name: String::new(),
                    file_id: 0,
                    field_path: String::new(),
                    target_bundle_path: String::new(),
                },
                RelationWriteRow {
                    bundle_path: bundle.clone(),
                    relation_type: "gameobject_component".to_string(),
                    source_path_id: 10,
                    target_path_id: 40,
                    source_name: String::new(),
                    target_name: String::new(),
                    file_id: 0,
                    field_path: String::new(),
                    target_bundle_path: String::new(),
                },
            ],
        }])
        .unwrap();

        assert_eq!(
            ExportAnimationUtils::component_refs_for_game_object(&db, &bundle, 10),
            vec![
                ComponentRef {
                    bundle_path: bundle.clone(),
                    path_id: 20,
                },
                ComponentRef {
                    bundle_path: bundle.clone(),
                    path_id: 30,
                }
            ]
        );

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn game_object_component_lookup_uses_target_bundle_path_for_cross_bundle_animator() {
        let workspace = temp_workspace("game-object-cross-bundle-animator");
        std::fs::create_dir_all(&workspace).unwrap();

        let db = AssetDatabase::open(&workspace).unwrap();
        let model_bundle = workspace.join("model.bundle").to_string_lossy().to_string();
        let anim_bundle = workspace.join("anim.bundle").to_string_lossy().to_string();

        db.insert_many_map_bundle_write_rows(&[
            MapBundleWriteRows {
                bundle_path: model_bundle.clone(),
                md5: "model".to_string(),
                file_size: 1,
                modified_ms: 1,
                unity_version: "2019.4".to_string(),
                asset_count: 1,
                assets: vec![asset(&model_bundle, 10, 1, "GameObject", "root")],
                containers: Vec::new(),
                externals: Vec::new(),
                internal_names: Vec::new(),
                relations: vec![RelationWriteRow {
                    bundle_path: model_bundle.clone(),
                    relation_type: "gameobject_component".to_string(),
                    source_path_id: 10,
                    target_path_id: 20,
                    source_name: String::new(),
                    target_name: String::new(),
                    file_id: 1,
                    field_path: String::new(),
                    target_bundle_path: anim_bundle.clone(),
                }],
            },
            MapBundleWriteRows {
                bundle_path: anim_bundle.clone(),
                md5: "anim".to_string(),
                file_size: 1,
                modified_ms: 1,
                unity_version: "2019.4".to_string(),
                asset_count: 1,
                assets: vec![asset(&anim_bundle, 20, 95, "Animator", "animator")],
                containers: Vec::new(),
                externals: Vec::new(),
                internal_names: Vec::new(),
                relations: Vec::new(),
            },
        ])
        .unwrap();

        assert_eq!(
            ExportAnimationUtils::component_refs_for_game_object(&db, &model_bundle, 10),
            vec![ComponentRef {
                bundle_path: anim_bundle,
                path_id: 20,
            }]
        );

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn visual_part_prefab_does_not_guess_actor_prefab_animator() {
        let workspace = temp_workspace("visual-part-actor-animator");
        std::fs::create_dir_all(&workspace).unwrap();

        let db = AssetDatabase::open(&workspace).unwrap();
        let visual_bundle = workspace
            .join("visual.bundle")
            .to_string_lossy()
            .to_string();
        let actor_bundle = workspace.join("actor.bundle").to_string_lossy().to_string();

        db.insert_many_map_bundle_write_rows(&[
            MapBundleWriteRows {
                bundle_path: visual_bundle.clone(),
                md5: "visual".to_string(),
                file_size: 1,
                modified_ms: 1,
                unity_version: "2019.4".to_string(),
                asset_count: 1,
                assets: vec![asset(&visual_bundle, 10, 1, "GameObject", "visual")],
                containers: vec![container(
                    &visual_bundle,
                    10,
                    "assets/res/prefab/actor_visual_part/ch_f_japan_onmyoji/ch_f_japan_onmyoji_lv_s11.prefab",
                )],
                externals: Vec::new(),
                internal_names: Vec::new(),
                relations: Vec::new(),
            },
            MapBundleWriteRows {
                bundle_path: actor_bundle.clone(),
                md5: "actor".to_string(),
                file_size: 1,
                modified_ms: 1,
                unity_version: "2019.4".to_string(),
                asset_count: 2,
                assets: vec![
                    asset(&actor_bundle, 20, 1, "GameObject", "actor"),
                    asset(&actor_bundle, 30, 95, "Animator", "animator"),
                ],
                containers: vec![container(
                    &actor_bundle,
                    20,
                    "assets/res/prefab/actor/ch_f_japan_onmyoji.prefab",
                )],
                externals: Vec::new(),
                internal_names: Vec::new(),
                relations: vec![RelationWriteRow {
                    bundle_path: actor_bundle.clone(),
                    relation_type: "gameobject_component".to_string(),
                    source_path_id: 20,
                    target_path_id: 30,
                    source_name: String::new(),
                    target_name: String::new(),
                    file_id: 0,
                    field_path: String::new(),
                    target_bundle_path: String::new(),
                }],
            },
        ])
        .unwrap();

        let animators =
            ExportAnimationUtils::collect_preview_animators("GameObject", &visual_bundle, 10, &db);
        assert!(animators.is_empty());

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn mesh_lookup_does_not_guess_actor_prefab_when_direct_chain_is_missing() {
        let workspace = temp_workspace("mesh-actor-fallback");
        std::fs::create_dir_all(&workspace).unwrap();

        let db = AssetDatabase::open(&workspace).unwrap();
        let visual_bundle = workspace
            .join("visual.bundle")
            .to_string_lossy()
            .to_string();
        let actor_bundle = workspace.join("actor.bundle").to_string_lossy().to_string();

        db.insert_many_map_bundle_write_rows(&[
            MapBundleWriteRows {
                bundle_path: visual_bundle.clone(),
                md5: "visual".to_string(),
                file_size: 1,
                modified_ms: 1,
                unity_version: "2019.4".to_string(),
                asset_count: 1,
                assets: vec![asset(&visual_bundle, 10, 43, "Mesh", "body_mesh")],
                containers: vec![container(
                    &visual_bundle,
                    10,
                    "assets/res/prefab/actor_visual_part/ch_f_japan_onmyoji/ch_f_japan_onmyoji_lv_s11.prefab",
                )],
                externals: Vec::new(),
                internal_names: Vec::new(),
                relations: Vec::new(),
            },
            MapBundleWriteRows {
                bundle_path: actor_bundle.clone(),
                md5: "actor".to_string(),
                file_size: 1,
                modified_ms: 1,
                unity_version: "2019.4".to_string(),
                asset_count: 2,
                assets: vec![
                    asset(&actor_bundle, 20, 1, "GameObject", "actor"),
                    asset(&actor_bundle, 30, 95, "Animator", "animator"),
                ],
                containers: vec![container(
                    &actor_bundle,
                    20,
                    "assets/res/prefab/actor/ch_f_japan_onmyoji.prefab",
                )],
                externals: Vec::new(),
                internal_names: Vec::new(),
                relations: vec![RelationWriteRow {
                    bundle_path: actor_bundle.clone(),
                    relation_type: "gameobject_component".to_string(),
                    source_path_id: 20,
                    target_path_id: 30,
                    source_name: String::new(),
                    target_name: String::new(),
                    file_id: 0,
                    field_path: String::new(),
                    target_bundle_path: String::new(),
                }],
            },
        ])
        .unwrap();

        let animators = ExportAnimationUtils::collect_preview_animators(
            "Mesh",
            &visual_bundle,
            10,
            &db,
        );
        assert!(animators.is_empty());

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }


    #[test]
    fn mesh_related_clip_walk_collects_only_exact_component_chain_refs() {
        let workspace = temp_workspace("mesh-exact-animation-chain");
        std::fs::create_dir_all(&workspace).unwrap();

        let db = AssetDatabase::open(&workspace).unwrap();
        let model_bundle = workspace.join("model.bundle").to_string_lossy().to_string();
        let anim_bundle = workspace.join("anim.bundle").to_string_lossy().to_string();

        db.insert_many_map_bundle_write_rows(&[
            MapBundleWriteRows {
                bundle_path: model_bundle.clone(),
                md5: "model".to_string(),
                file_size: 1,
                modified_ms: 1,
                unity_version: "2019.4".to_string(),
                asset_count: 7,
                assets: vec![
                    asset(&model_bundle, 10, 43, "Mesh", "body_mesh"),
                    asset(
                        &model_bundle,
                        20,
                        137,
                        "SkinnedMeshRenderer",
                        "body_renderer",
                    ),
                    asset(&model_bundle, 30, 1, "GameObject", "body_go"),
                    asset(&model_bundle, 40, 95, "Animator", "body_animator"),
                    asset(&model_bundle, 50, 91, "AnimatorController", "controller"),
                    asset(&model_bundle, 60, 74, "AnimationClip", "idle"),
                    asset(&model_bundle, 70, 74, "AnimationClip", "unrelated"),
                ],
                containers: Vec::new(),
                externals: Vec::new(),
                internal_names: Vec::new(),
                relations: vec![
                    RelationWriteRow {
                        bundle_path: model_bundle.clone(),
                        relation_type: UnityRelationKind::RENDERER_MESH.to_string(),
                        source_path_id: 20,
                        target_path_id: 10,
                        source_name: String::new(),
                        target_name: String::new(),
                        file_id: 0,
                        field_path: "m_Mesh".to_string(),
                        target_bundle_path: String::new(),
                    },
                    RelationWriteRow {
                        bundle_path: model_bundle.clone(),
                        relation_type: UnityRelationKind::RENDERER_GAMEOBJECT.to_string(),
                        source_path_id: 20,
                        target_path_id: 30,
                        source_name: String::new(),
                        target_name: String::new(),
                        file_id: 0,
                        field_path: "m_GameObject".to_string(),
                        target_bundle_path: String::new(),
                    },
                    RelationWriteRow {
                        bundle_path: model_bundle.clone(),
                        relation_type: "gameobject_component".to_string(),
                        source_path_id: 30,
                        target_path_id: 20,
                        source_name: String::new(),
                        target_name: String::new(),
                        file_id: 0,
                        field_path: String::new(),
                        target_bundle_path: String::new(),
                    },
                    RelationWriteRow {
                        bundle_path: model_bundle.clone(),
                        relation_type: "gameobject_component".to_string(),
                        source_path_id: 30,
                        target_path_id: 40,
                        source_name: String::new(),
                        target_name: String::new(),
                        file_id: 0,
                        field_path: String::new(),
                        target_bundle_path: String::new(),
                    },
                    relation(&model_bundle, 40, 50, "m_Controller", ""),
                    relation(&model_bundle, 50, 60, "m_AnimationClips", ""),
                    relation(&model_bundle, 50, 80, "m_AnimationClips", &anim_bundle),
                ],
            },
            MapBundleWriteRows {
                bundle_path: anim_bundle.clone(),
                md5: "anim".to_string(),
                file_size: 1,
                modified_ms: 1,
                unity_version: "2019.4".to_string(),
                asset_count: 1,
                assets: vec![asset(&anim_bundle, 80, 74, "AnimationClip", "run")],
                containers: Vec::new(),
                externals: Vec::new(),
                internal_names: Vec::new(),
                relations: Vec::new(),
            },
        ])
        .unwrap();

        let mut refs = Vec::new();
        let mut seen = HashSet::new();
        ExportAnimationUtils::collect_clip_refs_related_to_mesh(
            &db,
            &model_bundle,
            10,
            &mut refs,
            &mut seen,
            None,
        );

        assert_eq!(
            refs,
            vec![
                AnimationClipRef {
                    bundle_path: model_bundle.clone(),
                    path_id: 60,
                },
                AnimationClipRef {
                    bundle_path: anim_bundle,
                    path_id: 80,
                }
            ]
        );

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn typetree_pptr_collection_recurses_nested_and_expanded_ptrs() {
        let nested = UnityValue::Object(HashMap::from([(
            "m_AnimationClips".to_string(),
            UnityValue::Object(HashMap::from([(
                "Array".to_string(),
                UnityValue::Array(vec![
                    UnityValue::Ptr {
                        file_id: 1,
                        path_id: 10,
                    },
                    UnityValue::Object(HashMap::from([
                        ("m_FileID".to_string(), UnityValue::Integer(2)),
                        ("m_PathID".to_string(), UnityValue::Integer(20)),
                    ])),
                ]),
            )])),
        )]));
        let UnityValue::Object(map) = nested else {
            unreachable!();
        };
        let mut refs = Vec::new();
        for value in ExportAnimationUtils::unity_object_field_values(&map, "m_AnimationClips") {
            ExportAnimationUtils::collect_unity_value_pptrs(value, &mut refs);
        }

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].file_id, 1);
        assert_eq!(refs[0].path_id, 10);
        assert_eq!(refs[1].file_id, 2);
        assert_eq!(refs[1].path_id, 20);
    }

    #[test]
    #[ignore = "diagnostic probe against local real Naraka bundle data"]
    fn real_actor_body_visual_cell_probe() {
        let bundle_path = std::env::var("ASSETFINDER_REAL_BUNDLE")
            .unwrap_or_else(|_| r"D:\NarakaTest\6\6\660019f01e306713".to_string());
        let path_ids = [
            8847075052430237404_i64,
            136854213105656408_i64,
            1466625627197402528_i64,
        ];

        let bundle =
            AssetBundleLoader::load_bundle_serialized_only(std::path::Path::new(&bundle_path))
                .expect("load real bundle");

        for path_id in path_ids {
            let (sf, obj) = ExportAnimationUtils::find_object(&bundle, path_id)
                .unwrap_or_else(|| panic!("object not found: {}", path_id));
            let value = match ObjectHandle::new(sf, obj).read() {
                Ok(value) => value,
                Err(err) => {
                    if let Some(serialized_type) = sf.inner.object_serialized_type(&obj.inner) {
                        println!("=== TypeTree dump for failing path_id={} ===", path_id);
                        for (index, node) in serialized_type.type_tree.nodes.iter().enumerate() {
                            println!(
                                "#{:03} level={} type='{}' name='{}' byte_size={}",
                                index, node.level, node.type_name, node.name, node.byte_size
                            );
                        }
                    }
                    panic!("ObjectHandle::read failed for {}: {}", path_id, err);
                }
            };
            let mut refs = Vec::new();
            collect_ptr_paths("", &value, &mut refs);

            println!("=== ActorBodyVisualCell path_id={} ===", path_id);
            println!("ptr_count={}", refs.len());
            for (field_path, file_id, target_path_id) in refs {
                println!(
                    "pptr field='{}' file_id={} path_id={}",
                    field_path, file_id, target_path_id
                );
            }
        }
    }

    #[test]
    #[ignore = "diagnostic BFS against local real Naraka asset graph"]
    fn real_actor_body_visual_cell_reaches_animator_by_exact_refs() {
        use std::collections::{HashSet, VecDeque};

        let workspace = std::path::PathBuf::from(
            r"C:\Users\Administrator\AppData\Local\com.administrator.assetdaemon\asset-map\NarakaTest__3d77a37bcdc0",
        );
        let db = AssetDatabase::open(&workspace).expect("open real asset map");
        let start_bundle = r"D:\NarakaTest\6\6\660019f01e306713".to_string();
        let starts = [
            -337566547204893701_i64,
            8847075052430237404_i64,
            -3781255134203538519_i64,
            136854213105656408_i64,
            -4274623719921159940_i64,
            1466625627197402528_i64,
        ];

        let mut queue = VecDeque::new();
        let mut seen = HashSet::new();
        for path_id in starts {
            queue.push_back((start_bundle.clone(), path_id, 0usize));
        }

        let mut animator_hits = Vec::new();
        while let Some((bundle_path, path_id, depth)) = queue.pop_front() {
            if depth > 4 || !seen.insert((bundle_path.clone(), path_id)) {
                continue;
            }
            let Some(asset) = db
                .find_by_bundle_and_path_id(&bundle_path, path_id)
                .expect("db lookup")
            else {
                continue;
            };
            if matches!(asset.class_name.as_str(), "Animator" | "Animation") {
                println!(
                    "hit class={} bundle={} path_id={} name={} depth={}",
                    asset.class_name, bundle_path, path_id, asset.asset_name, depth
                );
                animator_hits.push((bundle_path.clone(), path_id, asset.asset_name.clone()));
                continue;
            }

            for relation_type in ["gameobject_component", "pptr", "monobehaviour_gameobject"] {
                for relation in db
                    .find_relations_by_bundle_and_source(relation_type, &bundle_path, path_id)
                    .unwrap_or_default()
                {
                    if relation.target_path_id == 0 {
                        continue;
                    }
                    let target_bundle_path = if relation.target_bundle_path.trim().is_empty() {
                        relation.bundle_path.clone()
                    } else {
                        relation.target_bundle_path.clone()
                    };
                    println!(
                        "depth={} {}:{} --{}:{}--> {}:{}",
                        depth,
                        bundle_path,
                        path_id,
                        relation.relation_type,
                        relation.field_path,
                        target_bundle_path,
                        relation.target_path_id
                    );
                    queue.push_back((target_bundle_path, relation.target_path_id, depth + 1));
                }
            }
        }

        println!("animator_hits={:?}", animator_hits);
    }

    #[test]
    #[ignore = "diagnostic exact Animator->Controller->Clip walk on real bundle"]
    fn real_avatar_matched_animator_resolves_exact_controller_clips() {
        use crate::common::asset_map::repository::AssetMapRepository;

        let workspace = std::path::PathBuf::from(r"C:\Users\Administrator\Desktop\NarakaDecrypt");
        let cache_root = std::path::PathBuf::from(r"D:\AssetDaemonCacheFolder");
        let db =
            AssetMapRepository::open(&workspace, Some(&cache_root)).expect("open real asset map");
        let bundle_path = r"C:\Users\Administrator\Desktop\NarakaDecrypt\6\6\660019f01e306713";
        let game_object_path_id = -337566547204893701_i64;

        let clips = ExportAnimationUtils::collect_preview_animation_clips(
            "GameObject",
            bundle_path,
            game_object_path_id,
            &db,
            None,
        );
        println!("preview clip refs={}", clips.len());
        for clip in clips.iter().take(12) {
            println!(
                "preview clip {}:{} {}",
                clip.bundle_path, clip.path_id, clip.name
            );
        }
        assert!(
            !clips.is_empty(),
            "Animator should resolve exact controller clips"
        );
    }

    #[test]
    #[ignore = "diagnostic selected real Naraka AnimationClip against preview skeleton"]
    fn real_selected_visual_part_clip_exports_animation_channels() {
        use crate::common::asset_map::repository::AssetMapRepository;
        use crate::common::mesh::animator_preview_hierarchy::AnimatorPreviewHierarchy;
        use crate::common::mesh::animator_preview_meshes::AnimatorPreviewMeshes;
        use crate::common::mesh::animator_preview_skeleton::{
            AnimatorPreviewSkeleton, AnimatorPreviewSkeletonRequest,
        };
        use crate::common::task::task_context::TaskContext;
        use crate::exporter::model_context::ModelContextResolver;
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        let workspace = std::path::PathBuf::from(r"C:\Users\Administrator\Desktop\NarakaDecrypt");
        let cache_root = std::path::PathBuf::from(r"D:\AssetDaemonCacheFolder");
        let db =
            AssetMapRepository::open(&workspace, Some(&cache_root)).expect("open real asset map");
        let visual_bundle_path =
            r"C:\Users\Administrator\Desktop\NarakaDecrypt\6\6\660019f01e306713";
        let visual_path_id = -337566547204893701_i64;
        let selected_clip_bundle =
            r"C:\Users\Administrator\Desktop\NarakaDecrypt\8\b\8b85161a4bc2dc45";
        let selected_clip_path_id = 2409857663497606206_i64;

        let (mesh_refs, _, _) =
            AnimatorPreviewHierarchy::collect_mesh_refs(&db, visual_bundle_path, visual_path_id);
        assert_eq!(mesh_refs.len(), 12);
        let primary_ref = mesh_refs
            .iter()
            .find(|mesh_ref| mesh_ref.mesh_path_id == -5898882860443825091_i64)
            .or_else(|| mesh_refs.first())
            .expect("primary mesh ref");
        let geometry = AnimatorPreviewMeshes::parse_full_mesh(
            &primary_ref.mesh_bundle_path,
            primary_ref.mesh_path_id,
            &tauri::ipc::Channel::new(|_| Ok(())),
            None,
        )
        .expect("parse primary mesh");
        let primary_bundle =
            AssetBundleLoader::load_bundle(std::path::Path::new(&primary_ref.mesh_bundle_path))
                .expect("load primary bundle");
        let ctx = TaskContext::new("real_selected_clip", "Real selected clip probe");
        let cancel = Arc::new(AtomicBool::new(false));
        let skeleton = AnimatorPreviewSkeleton::resolve(AnimatorPreviewSkeletonRequest {
            mesh_bundle: &primary_bundle,
            mesh_path_id: primary_ref.mesh_path_id,
            mesh_bundle_path: &primary_ref.mesh_bundle_path,
            preferred_renderer: Some((
                &primary_ref.renderer_bundle_path,
                primary_ref.renderer_path_id,
            )),
            db: Some(&db),
            bind_pose_count: geometry.bind_poses.len() / 16,
            bone_name_hashes: &geometry.bone_name_hashes,
            root_bone_name_hash: geometry.root_bone_name_hash,
            progress: &tauri::ipc::Channel::new(|_| Ok(())),
            task_id: ctx.task_id(),
            cancel_token: &cancel,
        })
        .expect("resolve skeleton")
        .expect("skeleton")
        .0;
        let hash_map = ModelContextResolver::transform_path_by_unity_hash(&skeleton);
        let overlap = geometry
            .bone_name_hashes
            .iter()
            .filter(|hash| hash_map.contains_key(hash))
            .count();
        println!(
            "skeleton joints={} skin_joints={} skin_hashes={} mesh_hashes={} overlap={}",
            skeleton.joints.len(),
            skeleton.skin_joints.len(),
            skeleton.skin_joint_hashes.len(),
            geometry.bone_name_hashes.len(),
            overlap
        );
        let animations = ExportAnimationUtils::extract_selected_preview_animation(
            selected_clip_bundle,
            selected_clip_path_id,
            &skeleton,
            &[],
            None,
        );
        let avatar_tos_paths = ExportAnimationUtils::preview_avatar_tos_path_map(
            &db,
            "GameObject",
            visual_bundle_path,
            visual_path_id,
        );
        let selected_avatar_tos_paths = ExportAnimationUtils::filter_avatar_tos_paths_for_clip(
            selected_clip_bundle,
            selected_clip_path_id,
            &avatar_tos_paths,
        );
        let visual_animation_bundle =
            AssetBundleLoader::load_bundle(std::path::Path::new(visual_bundle_path))
                .expect("load visual bundle for animation skeleton");
        let extended_skeleton = ModelContextResolver::extend_skeleton_with_transform_paths(
            &visual_animation_bundle,
            &skeleton,
            &selected_avatar_tos_paths,
        );
        let hash_aliases = ExportAnimationUtils::avatar_tos_hash_aliases_for_skeleton(
            &avatar_tos_paths,
            &extended_skeleton,
        );
        let alias_animations =
            ExportAnimationUtils::extract_selected_preview_animation_with_hash_aliases(
                selected_clip_bundle,
                selected_clip_path_id,
                &extended_skeleton,
                &[],
                Some(&hash_aliases),
                None,
            );
        let selected_bundle = AssetBundleLoader::load_bundle_serialized_only(std::path::Path::new(
            selected_clip_bundle,
        ))
        .expect("load selected clip bundle serialized");
        let (selected_sf, selected_obj) =
            ExportAnimationUtils::find_object(&selected_bundle, selected_clip_path_id)
                .expect("selected clip object");
        let binding_hashes =
            AnimationClipExporter::binding_path_hashes_from_clip_object(selected_sf, selected_obj);
        let binding_overlap = binding_hashes
            .iter()
            .filter(|hash| hash_map.contains_key(hash))
            .count();
        let mesh_overlap = binding_hashes
            .iter()
            .filter(|hash| geometry.bone_name_hashes.contains(hash))
            .count();
        let animators = ExportAnimationUtils::collect_preview_animators(
            "GameObject",
            visual_bundle_path,
            visual_path_id,
            &db,
        );
        println!("preview animators={:#?}", animators);
        let avatar_hashes = animators
            .first()
            .map(|animator| {
                ExportAnimationUtils::animator_avatar_tos_path_map(
                    &db,
                    &animator.bundle_path,
                    animator.path_id.parse().unwrap_or(0),
                )
                .keys()
                .copied()
                .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let avatar_overlap = binding_hashes
            .iter()
            .filter(|hash| avatar_hashes.contains(hash))
            .count();
        let binding_alias_overlap = binding_hashes
            .iter()
            .filter(|hash| hash_aliases.contains_key(hash))
            .count();
        println!(
            "binding_hashes={} skeleton_overlap={} mesh_overlap={} avatar_overlap={} avatar_hashes={} binding_alias_overlap={}",
            binding_hashes.len(),
            binding_overlap,
            mesh_overlap,
            avatar_overlap,
            avatar_hashes.len(),
            binding_alias_overlap
        );
        for hash in binding_hashes.iter().take(16) {
            println!(
                "binding hash={} avatar_path={:?} alias={:?}",
                hash,
                avatar_tos_paths.get(hash),
                hash_aliases.get(hash)
            );
        }
        for path in avatar_tos_paths.values() {
            if path.contains("Weapon") || path == "body_dummy" {
                let suffixes = std::iter::successors(Some(path.as_str()), |suffix| {
                    suffix.split_once('/').map(|(_, rest)| rest)
                })
                .filter_map(|suffix| {
                    ModelContextResolver::transform_path_by_name(&skeleton)
                        .get(suffix)
                        .copied()
                        .map(|joint| (suffix, joint))
                })
                .collect::<Vec<_>>();
                if !suffixes.is_empty() {
                    println!("tos_path={} suffix_matches={:?}", path, suffixes);
                }
            }
        }
        let visual_bundle = AssetBundleLoader::load_bundle_serialized_only(std::path::Path::new(
            visual_bundle_path,
        ))
        .expect("load visual bundle serialized");
        let game_object_names = visual_bundle
            .assets
            .iter()
            .flat_map(|sf| {
                sf.objects
                    .iter()
                    .filter(|obj| obj.class_id == 1)
                    .filter_map(|obj| {
                        UnityClassParser::parse_game_object(&sf.inner, &obj.inner)
                            .ok()
                            .map(|go| (obj.path_id, go.name))
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<HashMap<_, _>>();
        let transforms = visual_bundle
            .assets
            .iter()
            .flat_map(|sf| {
                sf.objects
                    .iter()
                    .filter(|obj| obj.class_id == 4)
                    .filter_map(|obj| {
                        UnityClassParser::parse_transform(&sf.inner, &obj.inner)
                            .ok()
                            .map(|transform| {
                                let name = game_object_names
                                    .get(&transform.game_object.path_id)
                                    .cloned()
                                    .unwrap_or_else(|| format!("bone_{}", obj.path_id));
                                (obj.path_id, (name, transform.father.path_id))
                            })
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<HashMap<_, _>>();
        let mut transform_paths = HashMap::<String, i64>::new();
        for transform_id in transforms.keys().copied() {
            let mut current = transform_id;
            let mut names = Vec::new();
            let mut seen = HashSet::new();
            while seen.insert(current) {
                let Some((name, parent)) = transforms.get(&current) else {
                    break;
                };
                names.push(name.clone());
                if *parent == 0 {
                    break;
                }
                current = *parent;
            }
            names.reverse();
            transform_paths.insert(names.join("/"), transform_id);
        }
        for hash in &binding_hashes {
            if let Some(path) = avatar_tos_paths.get(hash) {
                println!(
                    "transform tree lookup path={} full={:?} last={:?}",
                    path,
                    transform_paths.get(path),
                    path.rsplit('/').next().and_then(|last| {
                        transform_paths
                            .iter()
                            .find_map(|(tree_path, id)| tree_path.ends_with(last).then_some(id))
                    })
                );
            }
        }
        println!(
            "selected animations={} channels={} alias_animations={} alias_channels={} aliases={}",
            animations.len(),
            animations
                .iter()
                .map(|animation| animation.channels.len())
                .sum::<usize>(),
            alias_animations.len(),
            alias_animations
                .iter()
                .map(|animation| animation.channels.len())
                .sum::<usize>(),
            hash_aliases.len()
        );
        assert!(
            alias_animations
                .iter()
                .any(|animation| !animation.channels.is_empty()),
            "selected AnimationClip should produce playable channels"
        );
    }

    #[test]
    #[ignore = "diagnostic avatar TOS matching against local real Naraka asset graph"]
    fn real_visual_part_mesh_bone_hashes_match_avatar_tos() {
        use crate::common::asset_map::repository::AssetMapRepository;
        use crate::unity::mesh_asset_studio::AssetStudioMeshParser;
        use crate::unity::type_tree::unity_value::UnityValue;

        fn find_field<'a>(value: &'a UnityValue, field_name: &str) -> Option<&'a UnityValue> {
            match value {
                UnityValue::Object(map) => map
                    .get(field_name)
                    .or_else(|| map.values().find_map(|child| find_field(child, field_name))),
                UnityValue::Array(items) => {
                    items.iter().find_map(|child| find_field(child, field_name))
                }
                _ => None,
            }
        }

        let workspace = std::path::PathBuf::from(r"C:\Users\Administrator\Desktop\NarakaDecrypt");
        let cache_root = std::path::PathBuf::from(r"D:\AssetDaemonCacheFolder");
        let db = AssetMapRepository::open(&workspace, Some(&cache_root)).expect("open db");
        let visual_bundle_path =
            r"C:\Users\Administrator\Desktop\NarakaDecrypt\6\6\660019f01e306713";
        let mesh_path_id = -5898882860443825091_i64;
        let visual_bundle = AssetBundleLoader::load_bundle_serialized_only(std::path::Path::new(
            visual_bundle_path,
        ))
        .expect("load visual bundle");
        let (mesh_sf, mesh_obj) =
            ExportAnimationUtils::find_object(&visual_bundle, mesh_path_id).expect("mesh object");
        let mesh_raw = mesh_sf.object_bytes(mesh_obj).expect("mesh bytes");
        let mut mesh_logs = Vec::new();
        let mesh = AssetStudioMeshParser::parse_mesh_preview_from_raw_with_logs(
            mesh_raw,
            &mesh_sf.inner.unity_version,
            &mut mesh_logs,
        )
        .expect("parse mesh");
        let mesh_hashes = mesh.bone_name_hashes.into_iter().collect::<HashSet<_>>();
        println!(
            "mesh_hashes={} root={:?}",
            mesh_hashes.len(),
            mesh.root_bone_name_hash
        );

        let candidate_bundles = [
            r"C:\Users\Administrator\Desktop\NarakaDecrypt\5\8\589302f25ec7b3dd",
            r"C:\Users\Administrator\Desktop\NarakaDecrypt\4\4\4443a14f4d24450e",
            r"C:\Users\Administrator\Desktop\NarakaDecrypt\a\4\a48d196b89cbe3e3",
        ];
        for bundle_path in candidate_bundles {
            let bundle =
                AssetBundleLoader::load_bundle_serialized_only(std::path::Path::new(bundle_path))
                    .expect("load candidate bundle");
            let mut scanned = 0usize;
            let mut matches = Vec::<(usize, i64, i64)>::new();
            for sf in &bundle.assets {
                for obj in &sf.objects {
                    if obj.class_id != 95 {
                        continue;
                    }
                    scanned += 1;
                    let Ok(animator) = UnityClassParser::parse_animator(&sf.inner, &obj.inner)
                    else {
                        continue;
                    };
                    if animator.avatar.is_null() {
                        continue;
                    }
                    let Some(avatar_bundle_path) = ExportAnimationUtils::resolve_pptr_bundle_path(
                        &db,
                        bundle_path,
                        sf,
                        animator.avatar,
                    ) else {
                        continue;
                    };
                    let avatar_bundle = if avatar_bundle_path == bundle_path {
                        None
                    } else {
                        Some(
                            AssetBundleLoader::load_bundle_serialized_only(std::path::Path::new(
                                &avatar_bundle_path,
                            ))
                            .expect("load avatar bundle"),
                        )
                    };
                    let avatar_source = avatar_bundle.as_ref().unwrap_or(&bundle);
                    let Some((avatar_sf, avatar_obj)) =
                        ExportAnimationUtils::find_object(avatar_source, animator.avatar.path_id)
                    else {
                        continue;
                    };
                    let Some(value) = UnityClassParser::read_typetree_value_public(
                        &avatar_sf.inner,
                        &avatar_obj.inner,
                    ) else {
                        continue;
                    };
                    let Some(UnityValue::Array(items)) = find_field(&value, "m_TOS") else {
                        continue;
                    };
                    let mut overlap = 0usize;
                    for item in items {
                        if let UnityValue::Object(map) = item {
                            let key = map
                                .get("first")
                                .and_then(UnityValue::as_i64)
                                .or_else(|| map.get("key").and_then(UnityValue::as_i64))
                                .unwrap_or(-1);
                            if key >= 0 && mesh_hashes.contains(&(key as u32)) {
                                overlap += 1;
                            }
                        }
                    }
                    if overlap > 0 {
                        matches.push((overlap, obj.path_id, animator.avatar.path_id));
                    }
                }
            }
            matches.sort_by_key(|(overlap, _, _)| std::cmp::Reverse(*overlap));
            println!(
                "bundle={} scanned_animators={} matches_top={:?}",
                bundle_path,
                scanned,
                matches.iter().take(10).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    #[ignore = "diagnostic full Animator lookup on local real Naraka visual part"]
    fn real_visual_part_collects_animator_by_avatar_tos_match() {
        use crate::common::asset_map::repository::AssetMapRepository;

        let workspace = std::path::PathBuf::from(r"C:\Users\Administrator\Desktop\NarakaDecrypt");
        let cache_root = std::path::PathBuf::from(r"D:\AssetDaemonCacheFolder");
        let db = AssetMapRepository::open(&workspace, Some(&cache_root)).expect("open db");
        let visual_bundle_path =
            r"C:\Users\Administrator\Desktop\NarakaDecrypt\6\6\660019f01e306713";
        let visual_path_id = -337566547204893701_i64;

        let animators = ExportAnimationUtils::collect_preview_animators(
            "GameObject",
            visual_bundle_path,
            visual_path_id,
            &db,
        );
        println!("animators={:#?}", animators);
        assert!(
            animators
                .iter()
                .any(|animator| animator.path_id == "-674761679598903054"),
            "expected Avatar TOS matched Animator to be returned"
        );
    }

    #[test]
    #[ignore = "diagnostic probe for real LXRendererAssistant typetree refs"]
    fn real_lx_renderer_assistant_probe() {
        let bundle_path = r"D:\NarakaTest\6\6\660019f01e306713";
        let path_ids = [
            3385591521407427188_i64,
            998809572162722029_i64,
            3156867076487740173_i64,
        ];
        let bundle =
            AssetBundleLoader::load_bundle_serialized_only(std::path::Path::new(bundle_path))
                .expect("load bundle");

        for path_id in path_ids {
            let (sf, obj) = ExportAnimationUtils::find_object(&bundle, path_id)
                .unwrap_or_else(|| panic!("assistant object not found: {}", path_id));
            let value = ObjectHandle::new(sf, obj)
                .read()
                .unwrap_or_else(|err| panic!("assistant read failed for {}: {}", path_id, err));
            let mut refs = Vec::new();
            collect_ptr_paths("", &value, &mut refs);
            println!("=== LXRendererAssistant path_id={} ===", path_id);
            println!("ptr_count={}", refs.len());
            for (field_path, file_id, target_path_id) in refs {
                println!(
                    "pptr field='{}' file_id={} path_id={}",
                    field_path, file_id, target_path_id
                );
            }
        }
    }

    #[test]
    #[ignore = "diagnostic probe for renderer bone hierarchy animation roots"]
    fn real_renderer_bone_hierarchy_animation_probe() {
        let db = AssetDatabase::open(std::path::Path::new(
            r"C:\Users\Administrator\AppData\Local\com.administrator.assetdaemon\asset-map\NarakaTest__3d77a37bcdc0",
        ))
        .expect("open db");
        let renderer_bundle = r"D:\NarakaTest\6\6\660019f01e306713";
        let renderer_path_id = -7899841259225299006_i64;

        let roots = ExportAnimationUtils::renderer_animation_game_objects(
            &db,
            renderer_bundle,
            renderer_path_id,
        );
        println!("renderer_roots={}", roots.len());
        for root in &roots {
            let asset = db
                .find_by_bundle_and_path_id(&root.bundle_path, root.path_id)
                .ok()
                .flatten();
            println!(
                "root bundle={} path_id={} class={:?} name={:?}",
                root.bundle_path,
                root.path_id,
                asset.as_ref().map(|row| row.class_name.as_str()),
                asset.as_ref().map(|row| row.asset_name.as_str())
            );
            let components = ExportAnimationUtils::component_refs_for_game_object(
                &db,
                &root.bundle_path,
                root.path_id,
            );
            println!("  components={}", components.len());
            for component in components {
                let component_asset = db
                    .find_by_bundle_and_path_id(&component.bundle_path, component.path_id)
                    .ok()
                    .flatten();
                println!(
                    "    component bundle={} path_id={} class={:?} name={:?}",
                    component.bundle_path,
                    component.path_id,
                    component_asset.as_ref().map(|row| row.class_name.as_str()),
                    component_asset.as_ref().map(|row| row.asset_name.as_str())
                );
                let mut clip_refs = Vec::new();
                let mut seen = HashSet::new();
                let mut seen_controllers = HashSet::new();
                ExportAnimationUtils::collect_clip_refs_from_component(
                    &db,
                    &component.bundle_path,
                    component.path_id,
                    &mut clip_refs,
                    &mut seen,
                    &mut seen_controllers,
                );
                ExportAnimationUtils::collect_clip_refs_from_component_object(
                    &db,
                    &component.bundle_path,
                    component.path_id,
                    &mut clip_refs,
                    &mut seen,
                    &mut seen_controllers,
                    None,
                );
                println!("    clip_refs={}", clip_refs.len());
                for clip_ref in clip_refs {
                    let clip_asset = db
                        .find_by_bundle_and_path_id(&clip_ref.bundle_path, clip_ref.path_id)
                        .ok()
                        .flatten();
                    println!(
                        "      clip bundle={} path_id={} name={:?}",
                        clip_ref.bundle_path,
                        clip_ref.path_id,
                        clip_asset.as_ref().map(|row| row.asset_name.as_str())
                    );
                }
            }
        }
    }

    fn asset(
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

    fn container(bundle_path: &str, path_id: i64, asset_path: &str) -> ContainerWriteRow {
        ContainerWriteRow {
            bundle_path: bundle_path.to_string(),
            asset_path: asset_path.to_string(),
            path_id,
        }
    }

    fn relation(
        bundle_path: &str,
        source_path_id: i64,
        target_path_id: i64,
        field_path: &str,
        target_bundle_path: &str,
    ) -> RelationWriteRow {
        RelationWriteRow {
            bundle_path: bundle_path.to_string(),
            relation_type: "pptr".to_string(),
            source_path_id,
            target_path_id,
            source_name: String::new(),
            target_name: String::new(),
            file_id: 0,
            field_path: field_path.to_string(),
            target_bundle_path: target_bundle_path.to_string(),
        }
    }

    fn temp_workspace(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("assetfinder-{}-{}", name, nanos))
    }

    fn collect_ptr_paths(prefix: &str, value: &UnityValue, out: &mut Vec<(String, i32, i64)>) {
        match value {
            UnityValue::Ptr { file_id, path_id } => {
                if *path_id != 0 {
                    out.push((prefix.to_string(), *file_id, *path_id));
                }
            }
            UnityValue::Object(map) => {
                for (key, child) in map {
                    let next = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{}.{}", prefix, key)
                    };
                    collect_ptr_paths(&next, child, out);
                }
            }
            UnityValue::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    let next = if prefix.is_empty() {
                        format!("[{}]", index)
                    } else {
                        format!("{}[{}]", prefix, index)
                    };
                    collect_ptr_paths(&next, child, out);
                }
            }
            _ => {}
        }
    }
}
