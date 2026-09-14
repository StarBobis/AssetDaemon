/*
 * Skeleton resolution for Animator preview GLB generation.
 */

use crate::common::asset_map::asset_index::AssetDatabase;
use crate::common::bundle_file::asset_bundle::{AssetBundle, AssetBundleLoader};
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::task::task_logger::TaskLogger;
use crate::exporter::model_context::{GlbSkeleton, ModelContextResolver};
use crate::unity::relations::UnityRelationKind;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Instant;

pub struct AnimatorPreviewSkeletonRequest<'a> {
    pub mesh_bundle: &'a AssetBundle,
    pub mesh_path_id: i64,
    pub mesh_bundle_path: &'a str,
    pub preferred_renderer: Option<(&'a str, i64)>,
    pub db: Option<&'a AssetDatabase>,
    pub bind_pose_count: usize,
    pub bone_name_hashes: &'a [u32],
    pub root_bone_name_hash: Option<u32>,
    pub progress: &'a tauri::ipc::Channel<ProgressPayload>,
    pub task_id: &'a str,
    pub cancel_token: &'a Arc<AtomicBool>,
}

pub struct AnimatorPreviewSkeleton;

impl AnimatorPreviewSkeleton {
    pub fn resolve(
        request: AnimatorPreviewSkeletonRequest<'_>,
    ) -> Result<Option<(GlbSkeleton, Option<String>)>, String> {
        let phase_started = Instant::now();
        emit_log(
            request.task_id,
            request.progress,
            "skeleton",
            format!(
                "Skeleton input: mesh={}, bindPoseCount={}, boneNameHashes={}, rootBoneHash={:?}",
                request.mesh_path_id,
                request.bind_pose_count,
                request.bone_name_hashes.len(),
                request.root_bone_name_hash
            ),
        );

        if let Some((renderer_bundle_path, renderer_path_id)) = request
            .preferred_renderer
            .filter(|(_, renderer_path_id)| *renderer_path_id != 0)
        {
            let renderer_started = Instant::now();
            emit_log(
                request.task_id,
                request.progress,
                "skeleton",
                format!(
                    "Trying Animator hierarchy renderer first: {} renderer_path_id={}",
                    bundle_display_name(renderer_bundle_path),
                    renderer_path_id
                ),
            );
            let preferred_result = if renderer_bundle_path == request.mesh_bundle_path {
                ModelContextResolver::resolve_skeleton_for_renderer_with_map(
                    request.mesh_bundle,
                    request.mesh_bundle_path,
                    renderer_path_id,
                    request.mesh_path_id,
                    request.mesh_bundle_path,
                    request.db,
                    request.bind_pose_count,
                    request.bone_name_hashes,
                    request.root_bone_name_hash,
                )
            } else {
                let renderer_bundle = AssetBundleLoader::load_bundle(Path::new(
                    renderer_bundle_path,
                ))
                .map_err(|e| format!("Failed to load renderer Bundle for skeleton: {}", e))?;
                ModelContextResolver::resolve_skeleton_for_renderer_with_map(
                    &renderer_bundle,
                    renderer_bundle_path,
                    renderer_path_id,
                    request.mesh_path_id,
                    request.mesh_bundle_path,
                    request.db,
                    request.bind_pose_count,
                    request.bone_name_hashes,
                    request.root_bone_name_hash,
                )
            };
            if let Some(skeleton) = preferred_result {
                emit_log(
                    request.task_id,
                    request.progress,
                    "skeleton",
                    format!(
                        "Skeleton resolved from Animator hierarchy renderer in {} ms: joints={}, skinJoints={}, roots={}",
                        elapsed_ms(renderer_started),
                        skeleton.joints.len(),
                        skeleton.skin_joints.len(),
                        skeleton.roots.len()
                    ),
                );
                let source = (renderer_bundle_path != request.mesh_bundle_path)
                    .then(|| renderer_bundle_path.to_string());
                return Ok(Some((skeleton, source)));
            }
            emit_log(
                request.task_id,
                request.progress,
                "skeleton",
                format!(
                    "Animator hierarchy renderer skeleton miss in {} ms",
                    elapsed_ms(renderer_started)
                ),
            );
        }

        let local_started = Instant::now();
        let local_renderer_rows = request
            .db
            .map(|db| {
                db.find_relations_by_bundle_and_target(
                    UnityRelationKind::RENDERER_MESH,
                    request.mesh_bundle_path,
                    request.mesh_path_id,
                )
                .unwrap_or_default()
                .into_iter()
                .filter(|row| row.bundle_path == request.mesh_bundle_path)
                .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        emit_log(
            request.task_id,
            request.progress,
            "skeleton",
            format!(
                "Local skeleton renderer candidates from AssetMap: {}",
                local_renderer_rows.len()
            ),
        );
        if !local_renderer_rows.is_empty() {
            for (index, row) in local_renderer_rows.iter().enumerate() {
                check_cancelled(request.task_id, request.cancel_token)?;
                let renderer_started = Instant::now();
                emit_log(
                    request.task_id,
                    request.progress,
                    "skeleton",
                    format!(
                        "Local renderer skeleton lookup {}/{}: renderer_path_id={}",
                        index + 1,
                        local_renderer_rows.len(),
                        row.source_path_id
                    ),
                );
                if let Some(skeleton) =
                    ModelContextResolver::resolve_skeleton_for_renderer_with_hashes(
                        request.mesh_bundle,
                        row.source_path_id,
                        request.mesh_path_id,
                        request.bind_pose_count,
                        request.bone_name_hashes,
                        request.root_bone_name_hash,
                    )
                {
                    emit_log(
                        request.task_id,
                        request.progress,
                        "skeleton",
                        format!(
                            "Skeleton resolved from local renderer in {} ms: renderer_path_id={}, joints={}, skinJoints={}, roots={}",
                            elapsed_ms(renderer_started),
                            row.source_path_id,
                            skeleton.joints.len(),
                            skeleton.skin_joints.len(),
                            skeleton.roots.len()
                        ),
                    );
                    return Ok(Some((skeleton, None)));
                }
                emit_log(
                    request.task_id,
                    request.progress,
                    "skeleton",
                    format!(
                        "Local renderer skeleton miss in {} ms: renderer_path_id={}",
                        elapsed_ms(renderer_started),
                        row.source_path_id
                    ),
                );
            }
        }

        emit_log(
            request.task_id,
            request.progress,
            "skeleton",
            if local_renderer_rows.is_empty() {
                "No exact local renderer relation; falling back to full local SkinnedMeshRenderer scan..."
                    .to_string()
            } else {
                "Exact local renderer relation(s) missed; falling back to full local SkinnedMeshRenderer scan..."
                    .to_string()
            },
        );
        let full_scan_started = Instant::now();
        if let Some(skeleton) = ModelContextResolver::resolve_skeleton_for_mesh_with_hashes(
            request.mesh_bundle,
            request.mesh_path_id,
            request.bind_pose_count,
            request.bone_name_hashes,
            request.root_bone_name_hash,
        ) {
            emit_log(
                request.task_id,
                request.progress,
                "skeleton",
                format!(
                    "Skeleton resolved by full local scan in {} ms: joints={}, skinJoints={}, roots={}",
                    elapsed_ms(full_scan_started),
                    skeleton.joints.len(),
                    skeleton.skin_joints.len(),
                    skeleton.roots.len()
                ),
            );
            return Ok(Some((skeleton, None)));
        }
        emit_log(
            request.task_id,
            request.progress,
            "skeleton",
            format!(
                "Full local skeleton scan missed in {} ms",
                elapsed_ms(full_scan_started)
            ),
        );
        emit_log(
            request.task_id,
            request.progress,
            "skeleton",
            format!(
                "Local skeleton lookup missed in {} ms, scanning AssetMap renderer candidates...",
                elapsed_ms(local_started)
            ),
        );

        let mut candidate_bundles = Vec::<(String, i64)>::new();
        let mut renderer_relation_count = 0usize;
        let mut mesh_filter_relation_count = 0usize;
        if let Some(db) = request.db {
            let renderer_rows = db
                .find_relations_by_bundle_and_target(
                    UnityRelationKind::RENDERER_MESH,
                    request.mesh_bundle_path,
                    request.mesh_path_id,
                )
                .unwrap_or_default();
            renderer_relation_count = renderer_rows.len();
            for row in renderer_rows {
                if row.bundle_path != request.mesh_bundle_path
                    && !candidate_bundles
                        .iter()
                        .any(|(path, _)| path == &row.bundle_path)
                {
                    candidate_bundles.push((row.bundle_path, row.source_path_id));
                }
            }

            let mesh_filter_rows = db
                .find_relations_by_bundle_and_target(
                    UnityRelationKind::MESH_FILTER_MESH,
                    request.mesh_bundle_path,
                    request.mesh_path_id,
                )
                .unwrap_or_default();
            mesh_filter_relation_count = mesh_filter_rows.len();
            for row in mesh_filter_rows {
                if row.bundle_path != request.mesh_bundle_path
                    && !candidate_bundles
                        .iter()
                        .any(|(path, _)| path == &row.bundle_path)
                {
                    candidate_bundles.push((row.bundle_path, 0));
                }
            }
        }

        emit_log(
            request.task_id,
            request.progress,
            "skeleton",
            format!(
                "Skeleton candidate bundles: {} (renderer_mesh relations={}, mesh_filter_mesh relations={})",
                candidate_bundles.len(),
                renderer_relation_count,
                mesh_filter_relation_count
            ),
        );

        let worker_count = thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(4)
            .min(candidate_bundles.len())
            .min(8)
            .max(1);
        if !candidate_bundles.is_empty() {
            emit_log(
                request.task_id,
                request.progress,
                "skeleton",
                format!(
                    "Skeleton candidate scan uses {} worker(s), detail is capped after first 12 misses",
                    worker_count
                ),
            );
        }

        let scan = CandidateScan {
            candidate_bundles,
            worker_count,
            mesh_path_id: request.mesh_path_id,
            bind_pose_count: request.bind_pose_count,
            bone_name_hashes: request.bone_name_hashes,
            root_bone_name_hash: request.root_bone_name_hash,
            task_id: request.task_id,
            progress: request.progress,
            cancel_token: request.cancel_token,
        };
        if let Some(resolved_skeleton) = scan.run() {
            return Ok(Some(resolved_skeleton));
        }
        check_cancelled(request.task_id, request.cancel_token)?;

        emit_log(
            request.task_id,
            request.progress,
            "skeleton",
            format!(
                "Skeleton resolve failed in {} ms",
                elapsed_ms(phase_started)
            ),
        );
        Ok(None)
    }
}

struct CandidateScan<'a> {
    candidate_bundles: Vec<(String, i64)>,
    worker_count: usize,
    mesh_path_id: i64,
    bind_pose_count: usize,
    bone_name_hashes: &'a [u32],
    root_bone_name_hash: Option<u32>,
    task_id: &'a str,
    progress: &'a tauri::ipc::Channel<ProgressPayload>,
    cancel_token: &'a Arc<AtomicBool>,
}

impl CandidateScan<'_> {
    fn run(self) -> Option<(GlbSkeleton, Option<String>)> {
        let next_index = AtomicUsize::new(0);
        let stop_scan = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel::<SkeletonCandidateScanResult>();
        let mut checked = 0usize;
        let mut missing = 0usize;
        let mut load_failed = 0usize;
        let mut quiet_candidates = 0usize;
        let mut pending = HashMap::<usize, SkeletonCandidateScanResult>::new();
        let mut next_ordered_index = 0usize;
        let mut resolved_skeleton = None::<(GlbSkeleton, Option<String>)>;

        thread::scope(|scope| {
            for _ in 0..self.worker_count {
                let tx = tx.clone();
                let candidate_bundles = &self.candidate_bundles;
                let next_index = &next_index;
                let cancel_token = Arc::clone(self.cancel_token);
                let stop_scan = Arc::clone(&stop_scan);
                scope.spawn(move || loop {
                    if cancel_token.load(Ordering::SeqCst) || stop_scan.load(Ordering::SeqCst) {
                        break;
                    }
                    let index = next_index.fetch_add(1, Ordering::SeqCst);
                    if index >= candidate_bundles.len() {
                        break;
                    }
                    let (bundle_path, renderer_path_id) = candidate_bundles[index].clone();
                    let candidate_started = Instant::now();
                    let path = Path::new(&bundle_path);
                    if !path.exists() {
                        let _ = tx.send(SkeletonCandidateScanResult {
                            index,
                            bundle_path,
                            renderer_path_id,
                            skeleton: None,
                            renderer_count: 0,
                            elapsed_ms: elapsed_ms(candidate_started),
                            load_ms: 0,
                            scan_ms: 0,
                            missing: true,
                            error: None,
                        });
                        continue;
                    }

                    let load_started = Instant::now();
                    let candidate = match AssetBundleLoader::load_bundle(path) {
                        Ok(bundle) => bundle,
                        Err(error) => {
                            let _ = tx.send(SkeletonCandidateScanResult {
                                index,
                                bundle_path,
                                renderer_path_id,
                                skeleton: None,
                                renderer_count: 0,
                                elapsed_ms: elapsed_ms(candidate_started),
                                load_ms: elapsed_ms(load_started),
                                scan_ms: 0,
                                missing: false,
                                error: Some(error.to_string()),
                            });
                            continue;
                        }
                    };
                    let load_ms = elapsed_ms(load_started);
                    let renderer_count = candidate
                        .assets
                        .iter()
                        .map(|sf| sf.objects.iter().filter(|obj| obj.class_id == 137).count())
                        .sum::<usize>();
                    let scan_started = Instant::now();
                    let skeleton = if renderer_path_id != 0 {
                        ModelContextResolver::resolve_skeleton_for_renderer_with_hashes(
                            &candidate,
                            renderer_path_id,
                            self.mesh_path_id,
                            self.bind_pose_count,
                            self.bone_name_hashes,
                            self.root_bone_name_hash,
                        )
                    } else {
                        ModelContextResolver::resolve_skeleton_for_mesh_with_hashes(
                            &candidate,
                            self.mesh_path_id,
                            self.bind_pose_count,
                            self.bone_name_hashes,
                            self.root_bone_name_hash,
                        )
                    };
                    let _ = tx.send(SkeletonCandidateScanResult {
                        index,
                        bundle_path,
                        renderer_path_id,
                        skeleton,
                        renderer_count,
                        elapsed_ms: elapsed_ms(candidate_started),
                        load_ms,
                        scan_ms: elapsed_ms(scan_started),
                        missing: false,
                        error: None,
                    });
                });
            }
            drop(tx);

            for item in rx {
                checked += 1;
                pending.insert(item.index, item);
                while let Some(item) = pending.remove(&next_ordered_index) {
                    if item.missing {
                        missing += 1;
                        if next_ordered_index < 12 {
                            emit_log(
                                self.task_id,
                                self.progress,
                                "skeleton",
                                format!(
                                    "Skeleton candidate {}/{} missing on disk: {}",
                                    next_ordered_index + 1,
                                    self.candidate_bundles.len(),
                                    item.bundle_path
                                ),
                            );
                        }
                    } else if let Some(error) = item.error {
                        load_failed += 1;
                        if next_ordered_index < 12 {
                            emit_log(
                                self.task_id,
                                self.progress,
                                "skeleton",
                                format!(
                                    "Skeleton candidate load failed in {} ms: {} ({})",
                                    item.load_ms,
                                    bundle_display_name(&item.bundle_path),
                                    error
                                ),
                            );
                        }
                    } else if let Some(skeleton) = item.skeleton {
                        emit_log(
                            self.task_id,
                            self.progress,
                            "skeleton",
                            format!(
                                "Skeleton resolved from {} renderer_path_id={} in {} ms (load={} ms, scan={} ms): renderers={}, joints={}, skinJoints={}, roots={}",
                                bundle_display_name(&item.bundle_path),
                                item.renderer_path_id,
                                item.elapsed_ms,
                                item.load_ms,
                                item.scan_ms,
                                item.renderer_count,
                                skeleton.joints.len(),
                                skeleton.skin_joints.len(),
                                skeleton.roots.len()
                            ),
                        );
                        resolved_skeleton = Some((skeleton, Some(item.bundle_path)));
                        stop_scan.store(true, Ordering::SeqCst);
                        break;
                    } else if next_ordered_index < 12 {
                        emit_log(
                            self.task_id,
                            self.progress,
                            "skeleton",
                            format!(
                                "Skeleton candidate miss {}/{}: {} renderer_path_id={} in {} ms (load={} ms, scan={} ms, renderers={})",
                                next_ordered_index + 1,
                                self.candidate_bundles.len(),
                                bundle_display_name(&item.bundle_path),
                                item.renderer_path_id,
                                item.elapsed_ms,
                                item.load_ms,
                                item.scan_ms,
                                item.renderer_count
                            ),
                        );
                    } else {
                        quiet_candidates += 1;
                        if quiet_candidates % 25 == 0 {
                            emit_log(
                                self.task_id,
                                self.progress,
                                "skeleton",
                                format!(
                                    "Skeleton scan still running: {} candidate bundle(s) checked, {} hidden detail line(s)",
                                    checked, quiet_candidates
                                ),
                            );
                        }
                    }
                    next_ordered_index += 1;
                }
                if resolved_skeleton.is_some() {
                    break;
                }
            }
        });

        if resolved_skeleton.is_none() {
            emit_log(
                self.task_id,
                self.progress,
                "skeleton",
                format!(
                    "Skeleton candidate scan failed: checked={}, missing={}, loadFailed={}, hiddenDetails={}",
                    checked, missing, load_failed, quiet_candidates
                ),
            );
        }

        resolved_skeleton
    }
}

struct SkeletonCandidateScanResult {
    index: usize,
    bundle_path: String,
    renderer_path_id: i64,
    skeleton: Option<GlbSkeleton>,
    renderer_count: usize,
    elapsed_ms: u128,
    load_ms: u128,
    scan_ms: u128,
    missing: bool,
    error: Option<String>,
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
