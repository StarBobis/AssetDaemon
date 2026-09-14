/*
 * Animator hierarchy traversal and transform baking for preview workflows.
 */

use crate::common::asset_map::asset_index::{AssetDatabase, RelationRow};
use crate::common::asset_map::asset_map_types::BundleRelationEntry;
use crate::common::asset_map::unity_relation_collector::UnityRelationCollector;
use crate::common::bundle_file::asset_bundle::AssetBundleLoader;
use crate::common::mesh::mesh_types::MeshGeometry;
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::task::task_logger::TaskLogger;
use crate::common::unity_dependency::external_resolver::UnityExternalResolver;
use crate::unity::classes::registry::UnityClassParser;
use crate::unity::relations::UnityRelationKind;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AnimatorMeshRef {
    pub(crate) mesh_bundle_path: String,
    pub(crate) mesh_path_id: i64,
    pub(crate) mesh_name: String,
    pub(crate) renderer_bundle_path: String,
    pub(crate) renderer_path_id: i64,
    pub(crate) transform_bundle_path: String,
    pub(crate) transform_path_id: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct AnimatorMeshRefDiagnostics {
    relation_type: String,
    renderer_bundle_path: String,
    renderer_path_id: i64,
    target_path_id: i64,
    file_id: i32,
    target_name: String,
    target_bundle_path: String,
    reason: String,
}

#[derive(Debug, Clone, Copy)]
struct LocalTransform {
    rotation: [f32; 4],
    position: [f32; 3],
    scale: [f32; 3],
}

pub(crate) struct AnimatorPreviewHierarchy;

impl AnimatorPreviewHierarchy {
    pub(crate) fn collect_mesh_refs(
        db: &AssetDatabase,
        bundle_path: &str,
        root_game_object_id: i64,
    ) -> (Vec<AnimatorMeshRef>, Vec<AnimatorMeshRefDiagnostics>, bool) {
        collect_animator_mesh_refs(db, bundle_path, root_game_object_id)
    }

    /// Collects Mesh references for a GameObject preview. AssetMap lists every
    /// GameObject of a prefab as a separate row, so the selected GameObject is
    /// often a leaf node whose subtree legitimately contains no renderers. When
    /// that happens, walk up the m_Father chain to the hierarchy root and
    /// collect Mesh references for the whole prefab instead.
    pub(crate) fn collect_mesh_refs_with_root_fallback(
        db: &AssetDatabase,
        bundle_path: &str,
        root_game_object_id: i64,
    ) -> (Vec<AnimatorMeshRef>, Vec<AnimatorMeshRefDiagnostics>, bool) {
        let (refs, diagnostics, has_particles) =
            collect_animator_mesh_refs(db, bundle_path, root_game_object_id);
        if !refs.is_empty() {
            return (refs, diagnostics, has_particles);
        }
        let Some(hierarchy_root_id) =
            find_hierarchy_root_game_object(db, bundle_path, root_game_object_id)
        else {
            return (refs, diagnostics, has_particles);
        };
        if hierarchy_root_id == root_game_object_id {
            return (refs, diagnostics, has_particles);
        }
        let (root_refs, root_diagnostics, root_has_particles) =
            collect_animator_mesh_refs(db, bundle_path, hierarchy_root_id);
        if root_refs.is_empty() {
            return (refs, diagnostics, has_particles);
        }
        TaskLogger::info(
            "collect_mesh_refs",
            "Animator Preview",
            &format!(
                "Selected GameObject subtree has no Mesh references; resolved hierarchy root GameObject {} with {} Mesh reference(s)",
                hierarchy_root_id,
                root_refs.len()
            ),
        );
        (root_refs, root_diagnostics, root_has_particles)
    }

    pub(crate) fn emit_mesh_ref_diagnostics(
        diagnostics: &[AnimatorMeshRefDiagnostics],
        progress: Option<&tauri::ipc::Channel<ProgressPayload>>,
        task_id: Option<&str>,
    ) {
        emit_animator_mesh_ref_diagnostics(diagnostics, progress, task_id);
    }

    pub(crate) fn bake_mesh_transform(
        db: &AssetDatabase,
        mesh_ref: &AnimatorMeshRef,
        geometry: &mut MeshGeometry,
    ) -> Result<(), String> {
        let chain = animator_transform_chain(db, mesh_ref)?;
        apply_transform_chain(geometry, &chain);
        Ok(())
    }
}

/// Resolves the topmost GameObject of the transform hierarchy that contains
/// the given GameObject, using transform_parent (child -> parent) relations.
/// Returns None when the GameObject has no transform relation in this bundle.
fn find_hierarchy_root_game_object(
    db: &AssetDatabase,
    bundle_path: &str,
    game_object_id: i64,
) -> Option<i64> {
    let transform_rels = db
        .find_relations_by_type_prefix(bundle_path, "transform_")
        .unwrap_or_default();
    let mut child_to_parent: HashMap<i64, i64> = HashMap::new();
    let mut own_transform: Option<i64> = None;
    for rel in &transform_rels {
        if rel.bundle_path != bundle_path {
            continue;
        }
        if rel.relation_type == "transform_parent" && rel.target_path_id != 0 {
            child_to_parent.insert(rel.source_path_id, rel.target_path_id);
        }
        if rel.relation_type == "transform_gameobject" && rel.target_path_id == game_object_id {
            own_transform = Some(rel.source_path_id);
        }
    }
    let mut current = own_transform?;
    let mut visited = HashSet::new();
    while let Some(&parent) = child_to_parent.get(&current) {
        if !visited.insert(current) {
            break;
        }
        current = parent;
    }
    transform_rels
        .iter()
        .find(|rel| {
            rel.relation_type == "transform_gameobject"
                && rel.bundle_path == bundle_path
                && rel.source_path_id == current
        })
        .map(|rel| rel.target_path_id)
}

fn collect_animator_mesh_refs(
    db: &AssetDatabase,
    bundle_path: &str,
    root_game_object_id: i64,
) -> (Vec<AnimatorMeshRef>, Vec<AnimatorMeshRefDiagnostics>, bool) {
    // Batch-load ALL transform relations for the bundle in one query,
    // then build in-memory maps to avoid per-node DB round-trips.
    let mut all_transform_rels = db
        .find_relations_by_type_prefix(bundle_path, "transform_")
        .unwrap_or_default();
    let has_transform_gameobject = all_transform_rels
        .iter()
        .any(|rel| rel.bundle_path == bundle_path && rel.relation_type == "transform_gameobject");
    let has_transform_parent = all_transform_rels
        .iter()
        .any(|rel| rel.bundle_path == bundle_path && rel.relation_type == "transform_parent");
    if !has_transform_gameobject || !has_transform_parent {
        if let Ok(live_relations) = collect_live_bundle_relations(bundle_path) {
            all_transform_rels.extend(
                live_relations
                    .iter()
                    .filter(|rel| rel.relation_type.starts_with("transform_"))
                    .cloned(),
            );
            TaskLogger::info(
                "collect_mesh_refs",
                "Animator Preview",
                &format!(
                    "AssetMap has no Transform relations for bundle; parsed live bundle relations: {}",
                    live_relations.len()
                ),
            );
        }
    }

    // Debug: log what we found
    let tx_parent_count = all_transform_rels
        .iter()
        .filter(|r| r.relation_type == "transform_parent" && r.bundle_path == bundle_path)
        .count();
    let tx_go_count = all_transform_rels
        .iter()
        .filter(|r| r.relation_type == "transform_gameobject" && r.bundle_path == bundle_path)
        .count();
    TaskLogger::info(
        "collect_mesh_refs",
        "Animator Preview",
        &format!(
            "transform_rels total={}, transform_parent={}, transform_gameobject={}, bundle_path={}, root_go={}",
            all_transform_rels.len(),
            tx_parent_count,
            tx_go_count,
            bundle_path,
            root_game_object_id
        ),
    );

    // Build parent→children map from transform_parent relations
    let mut parent_to_children: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut transform_to_game_object: HashMap<i64, (String, i64)> = HashMap::new();
    for rel in &all_transform_rels {
        if rel.relation_type == "transform_parent" && rel.bundle_path == bundle_path {
            parent_to_children
                .entry(rel.target_path_id)
                .or_default()
                .push(rel.source_path_id);
        }
        if rel.relation_type == "transform_gameobject" && rel.bundle_path == bundle_path {
            transform_to_game_object.insert(
                rel.source_path_id,
                (rel.bundle_path.clone(), rel.target_path_id),
            );
        }
    }

    // BFS the transform hierarchy using the in-memory map
    let root_transform_ids = all_transform_rels
        .iter()
        .filter(|rel| {
            rel.relation_type == "transform_gameobject"
                && rel.bundle_path == bundle_path
                && rel.target_path_id == root_game_object_id
        })
        .map(|rel| rel.source_path_id)
        .collect::<Vec<_>>();

    let mut transform_ids = HashSet::new();
    let mut queue: VecDeque<i64> = root_transform_ids.into_iter().collect();
    while let Some(transform_id) = queue.pop_front() {
        if !transform_ids.insert(transform_id) {
            continue;
        }
        if let Some(children) = parent_to_children.get(&transform_id) {
            for &child_id in children {
                queue.push_back(child_id);
            }
        }
    }

    // Build game_object → (transform_bundle, transform_path_id) map
    let mut game_object_transforms = HashMap::from([(
        root_game_object_id,
        bundle_transform_ref(bundle_path, *transform_ids.iter().next().unwrap_or(&0)),
    )]);
    for &transform_id in &transform_ids {
        if let Some((ref tx_bundle, game_obj_id)) = transform_to_game_object.get(&transform_id) {
            game_object_transforms
                .entry(*game_obj_id)
                .or_insert_with(|| bundle_transform_ref(tx_bundle, transform_id));
        }
    }
    let game_object_ids: Vec<i64> = game_object_transforms.keys().copied().collect();

    // Batch-load component relations in one query
    let mut all_component_rels = db
        .find_relations_by_type_prefix(bundle_path, "gameobject_component")
        .unwrap_or_default();
    if all_component_rels.is_empty() {
        if let Ok(live_relations) = collect_live_bundle_relations(bundle_path) {
            all_component_rels.extend(
                live_relations
                    .iter()
                    .filter(|rel| rel.relation_type == "gameobject_component")
                    .cloned(),
            );
        }
    }
    // Build game_object → components map (source=game_object, target=component)
    let mut go_components: HashMap<i64, Vec<&RelationRow>> = HashMap::new();
    for rel in &all_component_rels {
        if rel.relation_type == "gameobject_component"
            && rel.bundle_path == bundle_path
            && game_object_transforms.contains_key(&rel.source_path_id)
        {
            go_components
                .entry(rel.source_path_id)
                .or_default()
                .push(rel);
        }
    }

    // Batch-load mesh relations (renderer→mesh + mesh_filter→mesh).
    // Note: cannot use a single prefix "mesh_" because "renderer_mesh" doesn't start with "mesh_".
    let mut all_renderer_mesh_rels = db
        .find_relations_by_type_prefix(bundle_path, "renderer_mesh")
        .unwrap_or_default();
    let mut all_filter_mesh_rels = db
        .find_relations_by_type_prefix(bundle_path, "mesh_filter_mesh")
        .unwrap_or_default();
    if all_renderer_mesh_rels.is_empty() || all_filter_mesh_rels.is_empty() {
        if let Ok(live_relations) = collect_live_bundle_relations(bundle_path) {
            if all_renderer_mesh_rels.is_empty() {
                all_renderer_mesh_rels.extend(
                    live_relations
                        .iter()
                        .filter(|rel| rel.relation_type == UnityRelationKind::RENDERER_MESH)
                        .cloned(),
                );
            }
            if all_filter_mesh_rels.is_empty() {
                all_filter_mesh_rels.extend(
                    live_relations
                        .iter()
                        .filter(|rel| rel.relation_type == UnityRelationKind::MESH_FILTER_MESH)
                        .cloned(),
                );
            }
        }
    }
    let mut renderer_to_mesh: HashMap<i64, Vec<&RelationRow>> = HashMap::new();
    let mut filter_to_mesh: HashMap<i64, Vec<&RelationRow>> = HashMap::new();
    for rel in &all_renderer_mesh_rels {
        if rel.relation_type == UnityRelationKind::RENDERER_MESH {
            renderer_to_mesh
                .entry(rel.source_path_id)
                .or_default()
                .push(rel);
        }
    }
    for rel in &all_filter_mesh_rels {
        if rel.relation_type == UnityRelationKind::MESH_FILTER_MESH {
            filter_to_mesh
                .entry(rel.source_path_id)
                .or_default()
                .push(rel);
        }
    }

    let mut renderer_ids = HashSet::new();
    let mut mesh_filter_ids = HashSet::new();
    let mut mesh_refs = HashSet::new();
    let mut diagnostics = Vec::new();
    let mut particle_component_count = 0usize;

    for &game_object_id in &game_object_ids {
        let Some((ref transform_bundle_path, transform_path_id)) =
            game_object_transforms.get(&game_object_id)
        else {
            continue;
        };
        let Some(components) = go_components.get(&game_object_id) else {
            continue;
        };
        for component in components {
            let asset = db
                .find_by_bundle_and_path_id(&component.bundle_path, component.target_path_id)
                .ok()
                .flatten();
            let Some(asset) = asset else {
                continue;
            };
            match asset.class_name.as_str() {
                "SkinnedMeshRenderer" | "MeshRenderer" | "Renderer" => {
                    renderer_ids.insert((
                        component.bundle_path.clone(),
                        component.target_path_id,
                        transform_bundle_path.clone(),
                        *transform_path_id,
                    ));
                }
                "MeshFilter" => {
                    mesh_filter_ids.insert(component.target_path_id);
                    if let Some(mesh_rels) = filter_to_mesh.get(&component.target_path_id) {
                        for relation in mesh_rels {
                            if let Some((mesh_bundle_path, mesh_path_id, mesh_name)) =
                                resolve_mesh_relation_target(db, relation, Some(&mut diagnostics))
                            {
                                mesh_refs.insert(AnimatorMeshRef {
                                    mesh_bundle_path,
                                    mesh_path_id,
                                    mesh_name,
                                    renderer_bundle_path: String::new(),
                                    renderer_path_id: 0,
                                    transform_bundle_path: transform_bundle_path.clone(),
                                    transform_path_id: *transform_path_id,
                                });
                            }
                        }
                    }
                }
                // Particle systems usually have no static mesh geometry: billboard
                // visuals are generated by the simulation at runtime. But a
                // ParticleSystemRenderer in Mesh render mode references a real Mesh
                // through renderer_mesh relations, so register it as a renderer and
                // let the generic mesh resolution below pick it up.
                "ParticleSystemRenderer" => {
                    particle_component_count += 1;
                    renderer_ids.insert((
                        component.bundle_path.clone(),
                        component.target_path_id,
                        transform_bundle_path.clone(),
                        *transform_path_id,
                    ));
                }
                "ParticleSystem" | "TrailRenderer" | "LineRenderer" => {
                    particle_component_count += 1;
                }
                _ => {}
            }
        }
    }

    let renderer_ids_count = renderer_ids.len();
    let mesh_filter_ids_count = mesh_filter_ids.len();
    for (renderer_bundle, renderer_id, transform_bundle_path, transform_path_id) in renderer_ids {
        if let Some(mesh_rels) = renderer_to_mesh.get(&renderer_id) {
            for relation in mesh_rels {
                if let Some((mesh_bundle_path, mesh_path_id, mesh_name)) =
                    resolve_mesh_relation_target(db, relation, Some(&mut diagnostics))
                {
                    mesh_refs.insert(AnimatorMeshRef {
                        mesh_bundle_path,
                        mesh_path_id,
                        mesh_name,
                        renderer_bundle_path: renderer_bundle.clone(),
                        renderer_path_id: renderer_id,
                        transform_bundle_path: transform_bundle_path.clone(),
                        transform_path_id,
                    });
                }
            }
        }
    }

    let mut refs = mesh_refs.into_iter().collect::<Vec<_>>();
    if refs.is_empty() {
        TaskLogger::info(
            "collect_mesh_refs",
            "Animator Preview",
            &format!(
                "No mesh refs found: transform_ids={}, game_objects={}, renderer_ids={}, mesh_filter_ids={}, component_rels={}, renderer_mesh_rels={}, mesh_filter_mesh_rels={}, transform_rels_total={}, particle_components={}",
                transform_ids.len(),
                game_object_transforms.len(),
                renderer_ids_count,
                mesh_filter_ids_count,
                all_component_rels.len(),
                all_renderer_mesh_rels.len(),
                all_filter_mesh_rels.len(),
                all_transform_rels.len(),
                particle_component_count
            ),
        );
    }
    refs.sort_by(|left, right| {
        left.mesh_bundle_path
            .cmp(&right.mesh_bundle_path)
            .then_with(|| left.mesh_path_id.cmp(&right.mesh_path_id))
            .then_with(|| left.renderer_bundle_path.cmp(&right.renderer_bundle_path))
            .then_with(|| left.renderer_path_id.cmp(&right.renderer_path_id))
            .then_with(|| left.transform_bundle_path.cmp(&right.transform_bundle_path))
            .then_with(|| left.transform_path_id.cmp(&right.transform_path_id))
    });
    (refs, diagnostics, particle_component_count > 0)
}

fn bundle_transform_ref(bundle_path: &str, transform_path_id: i64) -> (String, i64) {
    (bundle_path.to_string(), transform_path_id)
}

fn collect_live_bundle_relations(bundle_path: &str) -> Result<Vec<RelationRow>, String> {
    let bundle = AssetBundleLoader::load_bundle_serialized_only(Path::new(bundle_path))?;
    let mut rows = Vec::new();
    for serialized_file in &bundle.assets {
        let mut relations = Vec::<BundleRelationEntry>::new();
        UnityRelationCollector::collect_serialized_file_relations(
            &serialized_file.inner,
            &mut relations,
            "",
        );
        rows.extend(relations.into_iter().map(|relation| RelationRow {
            bundle_path: bundle_path.to_string(),
            relation_type: relation.relation_type.into_owned(),
            source_path_id: relation.source_path_id,
            target_path_id: relation.target_path_id,
            source_name: relation.source_name,
            target_name: relation.target_name,
            file_id: relation.file_id,
            field_path: relation.field_path.into_owned(),
            target_bundle_path: relation.target_bundle_path,
        }));
    }
    dedupe_relation_rows(&mut rows);
    Ok(rows)
}

fn dedupe_relation_rows(rows: &mut Vec<RelationRow>) {
    let mut seen = HashSet::new();
    rows.retain(|row| {
        seen.insert((
            row.bundle_path.clone(),
            row.relation_type.clone(),
            row.source_path_id,
            row.target_path_id,
            row.file_id,
            row.field_path.clone(),
            row.target_bundle_path.clone(),
        ))
    });
}

fn relation_target_bundle(db: &AssetDatabase, relation: &RelationRow) -> String {
    UnityExternalResolver::resolve_relation_row(db, relation)
}

fn resolve_mesh_relation_target(
    db: &AssetDatabase,
    relation: &RelationRow,
    diagnostics: Option<&mut Vec<AnimatorMeshRefDiagnostics>>,
) -> Option<(String, i64, String)> {
    if relation.target_path_id == 0 {
        return None;
    }

    let primary_bundle_path = relation_target_bundle(db, relation);
    if let Some(asset) = db
        .find_by_bundle_and_path_id(&primary_bundle_path, relation.target_path_id)
        .ok()
        .flatten()
        .filter(|asset| asset.class_name == "Mesh")
    {
        return Some((
            primary_bundle_path,
            relation.target_path_id,
            mesh_asset_name(&asset.asset_name, relation.target_path_id),
        ));
    }

    if let Ok(rows) = db.find_by_path_id(relation.target_path_id) {
        let mesh_candidates = rows
            .into_iter()
            .filter(|asset| asset.class_name == "Mesh")
            .collect::<Vec<_>>();
        if mesh_candidates.len() == 1 {
            let asset = &mesh_candidates[0];
            if let Some(diagnostics) = diagnostics {
                diagnostics.push(animator_mesh_ref_diagnostic(
                    relation,
                    &primary_bundle_path,
                    format!(
                        "Corrected Mesh bundle via global path_id lookup: {}",
                        asset.bundle_path
                    ),
                ));
            }
            return Some((
                asset.bundle_path.clone(),
                asset.path_id,
                mesh_asset_name(&asset.asset_name, asset.path_id),
            ));
        }
        if mesh_candidates.len() > 1 {
            if let Some(diagnostics) = diagnostics {
                diagnostics.push(animator_mesh_ref_diagnostic(
                    relation,
                    &primary_bundle_path,
                    format!(
                        "Ambiguous Mesh path_id in AssetMap: {} candidate bundles",
                        mesh_candidates.len()
                    ),
                ));
            }
            return None;
        }
    }

    if relation.file_id > 0 {
        if let Some(fallback_bundle_path) =
            UnityExternalResolver::resolve_from_db(db, &relation.target_name)
        {
            if fallback_bundle_path != primary_bundle_path {
                if let Some(asset) = db
                    .find_by_bundle_and_path_id(&fallback_bundle_path, relation.target_path_id)
                    .ok()
                    .flatten()
                    .filter(|asset| asset.class_name == "Mesh")
                {
                    return Some((
                        fallback_bundle_path,
                        relation.target_path_id,
                        mesh_asset_name(&asset.asset_name, relation.target_path_id),
                    ));
                }
            }
        }
    }

    if let Some(diagnostics) = diagnostics {
        diagnostics.push(animator_mesh_ref_diagnostic(
            relation,
            &primary_bundle_path,
            "Resolved target bundle does not contain a Mesh with this path_id".to_string(),
        ));
    }
    None
}

fn mesh_asset_name(asset_name: &str, path_id: i64) -> String {
    let trimmed = asset_name.trim();
    if trimmed.is_empty() {
        format!("mesh_{}", path_id)
    } else {
        trimmed.to_string()
    }
}

fn animator_mesh_ref_diagnostic(
    relation: &RelationRow,
    resolved_bundle_path: &str,
    reason: String,
) -> AnimatorMeshRefDiagnostics {
    AnimatorMeshRefDiagnostics {
        relation_type: relation.relation_type.clone(),
        renderer_bundle_path: relation.bundle_path.clone(),
        renderer_path_id: relation.source_path_id,
        target_path_id: relation.target_path_id,
        file_id: relation.file_id,
        target_name: relation.target_name.clone(),
        target_bundle_path: relation.target_bundle_path.clone(),
        reason: format!("{} (resolved_bundle={})", reason, resolved_bundle_path),
    }
}

fn emit_animator_mesh_ref_diagnostics(
    diagnostics: &[AnimatorMeshRefDiagnostics],
    progress: Option<&tauri::ipc::Channel<ProgressPayload>>,
    task_id: Option<&str>,
) {
    for diagnostic in diagnostics.iter().take(24) {
        let message = format!(
            "Mesh relation skipped: type={}, renderer={}:{}, target_path_id={}, file_id={}, target_bundle='{}', target_name='{}', reason={}",
            diagnostic.relation_type,
            diagnostic.renderer_bundle_path,
            diagnostic.renderer_path_id,
            diagnostic.target_path_id,
            diagnostic.file_id,
            diagnostic.target_bundle_path,
            diagnostic.target_name,
            diagnostic.reason
        );
        if let Some(progress) = progress {
            progress
                .send(ProgressPayload {
                    step: "mesh_ref".into(),
                    message: message.clone(),
                })
                .ok();
        }
        if let Some(task_id) = task_id {
            TaskLogger::warn(task_id, "Animator Preview", &message);
        }
    }
    if diagnostics.len() > 24 {
        let message = format!(
            "Mesh relation diagnostics truncated: {} more skipped relation(s)",
            diagnostics.len() - 24
        );
        if let Some(progress) = progress {
            progress
                .send(ProgressPayload {
                    step: "mesh_ref".into(),
                    message: message.clone(),
                })
                .ok();
        }
        if let Some(task_id) = task_id {
            TaskLogger::warn(task_id, "Animator Preview", &message);
        }
    }
}

fn animator_transform_chain(
    db: &AssetDatabase,
    mesh_ref: &AnimatorMeshRef,
) -> Result<Vec<LocalTransform>, String> {
    if mesh_ref.transform_path_id == 0 {
        return Ok(Vec::new());
    }

    let mut current_bundle = mesh_ref.transform_bundle_path.clone();
    let mut current_path_id = mesh_ref.transform_path_id;
    let mut visited = HashSet::new();
    let mut leaf_to_root = Vec::new();

    while current_path_id != 0 {
        if !visited.insert((current_bundle.clone(), current_path_id)) {
            break;
        }
        let transform = load_local_transform(&current_bundle, current_path_id)?;
        leaf_to_root.push(transform);

        let parent = db
            .find_relations_by_bundle_and_source(
                "transform_parent",
                &current_bundle,
                current_path_id,
            )
            .unwrap_or_default()
            .into_iter()
            .next();

        let Some(parent_relation) = parent else {
            break;
        };
        if parent_relation.target_path_id == 0 {
            break;
        }
        current_bundle = relation_target_bundle(db, &parent_relation);
        current_path_id = parent_relation.target_path_id;
    }

    Ok(leaf_to_root)
}

fn load_local_transform(
    bundle_path: &str,
    transform_path_id: i64,
) -> Result<LocalTransform, String> {
    let bundle = AssetBundleLoader::load_bundle_serialized_only(Path::new(bundle_path))
        .map_err(|e| format!("Load Transform bundle failed: {}", e))?;
    let (sf, obj) = bundle
        .assets
        .iter()
        .find_map(|sf| {
            sf.objects
                .iter()
                .find(|obj| obj.path_id == transform_path_id && obj.class_id == 4)
                .map(|obj| (sf, obj))
        })
        .ok_or_else(|| format!("Transform path_id={} not found", transform_path_id))?;
    let transform = UnityClassParser::parse_transform(&sf.inner, &obj.inner)?;
    Ok(LocalTransform {
        rotation: transform.local_rotation,
        position: transform.local_position,
        scale: transform.local_scale,
    })
}

fn apply_transform_chain(geometry: &mut MeshGeometry, chain: &[LocalTransform]) {
    if chain.is_empty() {
        return;
    }

    for vertex in geometry.vertices.chunks_exact_mut(3) {
        let mut point = [vertex[0], vertex[1], vertex[2]];
        for transform in chain {
            point = transform_point(point, transform);
        }
        vertex.copy_from_slice(&point);
    }

    if geometry.normals.len() >= geometry.vertex_count * 3 {
        for normal in geometry.normals.chunks_exact_mut(3) {
            let mut value = [normal[0], normal[1], normal[2]];
            for transform in chain {
                value = rotate_vector(value, normalize_quaternion(transform.rotation));
            }
            value = normalize_vec3(value);
            normal.copy_from_slice(&value);
        }
    }
}

fn transform_point(point: [f32; 3], transform: &LocalTransform) -> [f32; 3] {
    let scaled = [
        point[0] * transform.scale[0],
        point[1] * transform.scale[1],
        point[2] * transform.scale[2],
    ];
    let rotated = rotate_vector(scaled, normalize_quaternion(transform.rotation));
    [
        rotated[0] + transform.position[0],
        rotated[1] + transform.position[1],
        rotated[2] + transform.position[2],
    ]
}

fn normalize_quaternion(q: [f32; 4]) -> [f32; 4] {
    let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if len <= f32::EPSILON {
        return [0.0, 0.0, 0.0, 1.0];
    }
    [q[0] / len, q[1] / len, q[2] / len, q[3] / len]
}

fn rotate_vector(v: [f32; 3], q: [f32; 4]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let s = q[3];
    let uv = cross(u, v);
    let uuv = cross(u, uv);
    [
        v[0] + 2.0 * (s * uv[0] + uuv[0]),
        v[1] + 2.0 * (s * uv[1] + uuv[1]),
        v[2] + 2.0 * (s * uv[2] + uuv[2]),
    ]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize_vec3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len <= f32::EPSILON {
        return v;
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::asset_map::asset_index::{
        AssetWriteRow, MapBundleWriteRows, RelationWriteRow,
    };
    use crate::common::mesh::mesh_types::MeshSubMeshInfo;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    fn temp_workspace(name: &str) -> std::path::PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "assetfinder_animator_hierarchy_{}_{}",
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

    fn relation(
        bundle_path: &str,
        relation_type: &str,
        source_path_id: i64,
        target_path_id: i64,
        file_id: i32,
    ) -> RelationWriteRow {
        RelationWriteRow {
            bundle_path: bundle_path.to_string(),
            relation_type: relation_type.to_string(),
            source_path_id,
            target_path_id,
            source_name: String::new(),
            target_name: String::new(),
            file_id,
            field_path: String::new(),
            target_bundle_path: String::new(),
        }
    }

    #[test]
    fn apply_transform_chain_bakes_vertices_from_leaf_to_root() {
        let mut source = geometry(vec![1.0, 0.0, 0.0], vec![0]);
        source.normals = vec![1.0, 0.0, 0.0];
        let chain = vec![
            LocalTransform {
                rotation: [0.0, 0.0, 0.0, 1.0],
                position: [0.0, 5.0, 0.0],
                scale: [2.0, 1.0, 1.0],
            },
            LocalTransform {
                rotation: [0.0, 0.0, 0.0, 1.0],
                position: [10.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
            },
        ];

        apply_transform_chain(&mut source, &chain);

        assert_eq!(source.vertices, vec![12.0, 5.0, 0.0]);
        assert_eq!(source.normals, vec![1.0, 0.0, 0.0]);
    }

    #[test]
    fn animator_mesh_refs_preserve_renderer_context_for_cross_bundle_meshes() {
        let workspace = temp_workspace("cross-bundle-renderer");
        std::fs::create_dir_all(&workspace).unwrap();
        let db = AssetDatabase::open(&workspace).unwrap();
        let renderer_bundle = workspace
            .join("animator_bundle")
            .to_string_lossy()
            .to_string();
        let mesh_bundle = workspace.join("mesh_bundle").to_string_lossy().to_string();

        let mut renderer_rows = map_bundle(&renderer_bundle);
        renderer_rows.assets = vec![
            asset(&renderer_bundle, 100, 1, "GameObject"),
            asset(&renderer_bundle, 300, 137, "SkinnedMeshRenderer"),
        ];
        renderer_rows.relations = vec![
            relation(&renderer_bundle, "transform_gameobject", 200, 100, 0),
            relation(&renderer_bundle, "gameobject_component", 100, 300, 0),
            relation(
                &renderer_bundle,
                UnityRelationKind::RENDERER_MESH,
                300,
                400,
                6,
            ),
        ];

        let mut mesh_rows = map_bundle(&mesh_bundle);
        mesh_rows.assets = vec![asset(&mesh_bundle, 400, 43, "Mesh")];

        db.insert_many_map_bundle_write_rows(&[renderer_rows, mesh_rows])
            .unwrap();

        let (mesh_refs, diagnostics, has_particles) =
            collect_animator_mesh_refs(&db, &renderer_bundle, 100);
        assert!(!has_particles);

        assert_eq!(mesh_refs.len(), 1);
        assert_eq!(mesh_refs[0].mesh_bundle_path, mesh_bundle);
        assert_eq!(mesh_refs[0].mesh_path_id, 400);
        assert_eq!(mesh_refs[0].renderer_bundle_path, renderer_bundle);
        assert_eq!(mesh_refs[0].renderer_path_id, 300);
        assert_eq!(mesh_refs[0].transform_path_id, 200);
        assert!(diagnostics.iter().any(|item| item
            .reason
            .contains("Corrected Mesh bundle via global path_id lookup")));

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn animator_mesh_refs_flag_particle_system_hierarchies() {
        let workspace = temp_workspace("particle-hierarchy");
        std::fs::create_dir_all(&workspace).unwrap();
        let db = AssetDatabase::open(&workspace).unwrap();
        let bundle = workspace
            .join("particle_bundle")
            .to_string_lossy()
            .to_string();

        let mut rows = map_bundle(&bundle);
        rows.assets = vec![
            asset(&bundle, 10, 1, "GameObject"),
            asset(&bundle, 20, 198, "ParticleSystem"),
            asset(&bundle, 30, 199, "ParticleSystemRenderer"),
        ];
        rows.relations = vec![
            relation(&bundle, "transform_gameobject", 100, 10, 0),
            relation(&bundle, "gameobject_component", 10, 20, 0),
            relation(&bundle, "gameobject_component", 10, 30, 0),
        ];
        db.insert_many_map_bundle_write_rows(&[rows]).unwrap();

        let (mesh_refs, diagnostics, has_particles) = collect_animator_mesh_refs(&db, &bundle, 10);

        assert!(mesh_refs.is_empty());
        assert!(diagnostics.is_empty());
        assert!(has_particles);

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn animator_mesh_refs_fall_back_to_hierarchy_root_for_leaf_selection() {
        let workspace = temp_workspace("leaf-root-fallback");
        std::fs::create_dir_all(&workspace).unwrap();
        let db = AssetDatabase::open(&workspace).unwrap();
        let bundle = workspace.join("model_bundle").to_string_lossy().to_string();

        // Hierarchy: GO 10 (transform 100) is the prefab root with two children:
        // GO 20 (transform 200) carries a MeshFilter + Mesh, GO 60 (transform 300)
        // is an empty leaf. Selecting the empty leaf must fall back to the root.
        let mut rows = map_bundle(&bundle);
        rows.assets = vec![
            asset(&bundle, 10, 1, "GameObject"),
            asset(&bundle, 20, 1, "GameObject"),
            asset(&bundle, 60, 1, "GameObject"),
            asset(&bundle, 30, 33, "MeshFilter"),
            asset(&bundle, 40, 23, "MeshRenderer"),
            asset(&bundle, 50, 43, "Mesh"),
        ];
        rows.relations = vec![
            relation(&bundle, "transform_gameobject", 100, 10, 0),
            relation(&bundle, "transform_gameobject", 200, 20, 0),
            relation(&bundle, "transform_gameobject", 300, 60, 0),
            RelationWriteRow {
                field_path: "m_Father".to_string(),
                ..relation(&bundle, "transform_parent", 200, 100, 0)
            },
            RelationWriteRow {
                field_path: "m_Father".to_string(),
                ..relation(&bundle, "transform_parent", 300, 100, 0)
            },
            relation(&bundle, "gameobject_component", 20, 30, 0),
            relation(&bundle, "gameobject_component", 20, 40, 0),
            relation(&bundle, UnityRelationKind::MESH_FILTER_MESH, 30, 50, 0),
        ];
        db.insert_many_map_bundle_write_rows(&[rows]).unwrap();

        let (direct_refs, _, _) = collect_animator_mesh_refs(&db, &bundle, 60);
        assert!(direct_refs.is_empty());

        let (refs, _, has_particles) =
            AnimatorPreviewHierarchy::collect_mesh_refs_with_root_fallback(&db, &bundle, 60);
        assert!(!has_particles);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].mesh_path_id, 50);
        assert_eq!(refs[0].transform_path_id, 200);

        // Selecting the root directly must keep working without the fallback.
        let (root_refs, _, _) =
            AnimatorPreviewHierarchy::collect_mesh_refs_with_root_fallback(&db, &bundle, 10);
        assert_eq!(root_refs.len(), 1);

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    #[ignore = "diagnostic against local real Windows asset map"]
    fn debug_windows_workspace_transform_hierarchy() {
        let workspace = std::path::PathBuf::from(
            r"D:\AssetDaemonCacheFolder\Windows-a444e50fa47a1f9081013a7b56663e68",
        );
        let db = AssetDatabase::open(&workspace).expect("open real asset map");
        let cases: [(&str, i64); 3] = [
            ("bundle158", -6543792190595572860),
            ("bundle163", -8180118559633401998),
            ("bundle166", -8589011025357178204),
        ];
        let bundles = db.get_bundle_infos().expect("bundle infos");
        for (bundle_name, root_go) in cases {
            let Some(info) = bundles
                .iter()
                .find(|info| info.path.ends_with(bundle_name))
            else {
                println!("[{bundle_name}] bundle not found in asset map");
                continue;
            };
            let bundle_path = &info.path;
            println!(
                "[{bundle_name}] path={} unity={} assets={}",
                bundle_path, info.unity_version, info.asset_count
            );

            let tx_go = db
                .get_relations_by_type(bundle_path, "transform_gameobject")
                .unwrap_or_default();
            let tx_parent = db
                .get_relations_by_type(bundle_path, "transform_parent")
                .unwrap_or_default();
            let go_component = db
                .get_relations_by_type(bundle_path, "gameobject_component")
                .unwrap_or_default();
            println!(
                "[{bundle_name}] transform_gameobject={} transform_parent={} gameobject_component={}",
                tx_go.len(),
                tx_parent.len(),
                go_component.len()
            );

            // Root transform lookup for the clicked GameObject.
            let roots: Vec<i64> = tx_go
                .iter()
                .filter(|rel| rel.target_path_id == root_go)
                .map(|rel| rel.source_path_id)
                .collect();
            println!("[{bundle_name}] root_go={root_go} root_transforms={roots:?}");

            // Parent -> children map exactly as collect_animator_mesh_refs builds it.
            let mut parent_to_children: HashMap<i64, Vec<i64>> = HashMap::new();
            for rel in &tx_parent {
                parent_to_children
                    .entry(rel.target_path_id)
                    .or_default()
                    .push(rel.source_path_id);
            }
            for root in &roots {
                println!(
                    "[{bundle_name}] root_transform={root} children_of_root={:?}",
                    parent_to_children.get(root)
                );
            }

            // Detect inverted rows: rows where the SOURCE is a parent of the TARGET.
            // With correct rows (child -> parent), the root transform should appear as
            // a target for each of its children.
            let root_as_source: Vec<(i64, i64)> = tx_parent
                .iter()
                .filter(|rel| roots.contains(&rel.source_path_id))
                .map(|rel| (rel.source_path_id, rel.target_path_id))
                .collect();
            println!(
                "[{bundle_name}] root_as_source_rows(source,target)={root_as_source:?}"
            );

            // BFS like the preview does.
            let mut seen = HashSet::new();
            let mut queue: VecDeque<i64> = roots.iter().copied().collect();
            while let Some(id) = queue.pop_front() {
                if !seen.insert(id) {
                    continue;
                }
                if let Some(children) = parent_to_children.get(&id) {
                    queue.extend(children.iter().copied());
                }
            }
            println!(
                "[{bundle_name}] bfs_transforms={} (expect close to transform_gameobject count)",
                seen.len()
            );

            // Print a small sample of parent rows.
            for rel in tx_parent.iter().take(6) {
                println!(
                    "[{bundle_name}] sample transform_parent field={} source={} target={}",
                    rel.field_path, rel.source_path_id, rel.target_path_id
                );
            }

            // RectTransform presence check.
            let rect_transforms = db
                .find_by_bundle(bundle_path)
                .unwrap_or_default()
                .into_iter()
                .filter(|asset| asset.class_id == 224)
                .count();
            println!("[{bundle_name}] rect_transform_assets={rect_transforms}");

            // Component resolution for the clicked GameObject.
            let go_rows: Vec<_> = go_component
                .iter()
                .filter(|rel| rel.source_path_id == root_go)
                .collect();
            println!(
                "[{bundle_name}] gameobject_component rows for root_go={}",
                go_rows.len()
            );
            for rel in go_rows.iter().take(8) {
                let asset = db
                    .find_by_bundle_and_path_id(bundle_path, rel.target_path_id)
                    .ok()
                    .flatten();
                println!(
                    "[{bundle_name}]   component target={} asset={:?}",
                    rel.target_path_id,
                    asset.map(|a| (a.class_id, a.class_name))
                );
            }

            // Sanity: do gameobject_component targets resolve to assets at all?
            let mut resolved = 0usize;
            let mut missing = 0usize;
            for rel in go_component.iter().take(40) {
                match db
                    .find_by_bundle_and_path_id(bundle_path, rel.target_path_id)
                    .ok()
                    .flatten()
                {
                    Some(_) => resolved += 1,
                    None => missing += 1,
                }
            }
            println!(
                "[{bundle_name}] component target sample: resolved={} missing={}",
                resolved, missing
            );

            // Walk m_Father links up to the true hierarchy root transform.
            let child_to_parent: HashMap<i64, i64> = tx_parent
                .iter()
                .map(|rel| (rel.source_path_id, rel.target_path_id))
                .collect();
            let mut top = roots.first().copied().unwrap_or(0);
            let mut guard = 0usize;
            while let Some(&parent) = child_to_parent.get(&top) {
                if parent == 0 || guard > 1000 {
                    break;
                }
                top = parent;
                guard += 1;
            }
            let top_go: Vec<i64> = tx_go
                .iter()
                .filter(|rel| rel.source_path_id == top)
                .map(|rel| rel.target_path_id)
                .collect();
            println!(
                "[{bundle_name}] walked_up_root_transform={} its_gameobjects={:?} steps={}",
                top,
                top_go,
                guard
            );
            for true_root_go in &top_go {
                let (refs, _, _) = collect_animator_mesh_refs(&db, bundle_path, *true_root_go);
                println!(
                    "[{bundle_name}] collect_from_true_root go={} mesh_refs={}",
                    true_root_go,
                    refs.len()
                );
            }

            let (mesh_refs, diagnostics, has_particles) =
                collect_animator_mesh_refs(&db, bundle_path, root_go);
            println!(
                "[{bundle_name}] mesh_refs={} diagnostics={} particles={}",
                mesh_refs.len(),
                diagnostics.len(),
                has_particles
            );
            let (fallback_refs, _, _) = AnimatorPreviewHierarchy::collect_mesh_refs_with_root_fallback(
                &db,
                bundle_path,
                root_go,
            );
            println!(
                "[{bundle_name}] with_root_fallback mesh_refs={}",
                fallback_refs.len()
            );
        }
    }

    #[test]
    #[ignore = "diagnostic against local bundle162 particle prefab"]
    fn debug_bundle162_particle_renderer_meshes() {
        use crate::unity::type_tree::unity_value::UnityValue;

        fn find_int_field(value: &UnityValue, field: &str) -> Option<i64> {
            match value {
                UnityValue::Object(map) => {
                    if let Some(UnityValue::Integer(v)) = map.get(field) {
                        return Some(*v);
                    }
                    map.values().find_map(|child| find_int_field(child, field))
                }
                UnityValue::Array(items) => {
                    items.iter().find_map(|child| find_int_field(child, field))
                }
                _ => None,
            }
        }

        fn find_ptr_field_local(value: &UnityValue, field: &str) -> Option<(i32, i64)> {
            match value {
                UnityValue::Object(map) => {
                    if let Some(UnityValue::Ptr { file_id, path_id }) = map.get(field) {
                        return Some((*file_id, *path_id));
                    }
                    map.values()
                        .find_map(|child| find_ptr_field_local(child, field))
                }
                UnityValue::Array(items) => items
                    .iter()
                    .find_map(|child| find_ptr_field_local(child, field)),
                _ => None,
            }
        }

        let bundle_path = r"C:\Users\Administrator\Desktop\Windows\bundle162";
        let bundle =
            AssetBundleLoader::load_bundle_serialized_only(std::path::Path::new(bundle_path))
                .expect("load bundle162");
        for sf in &bundle.assets {
            let inner = &sf.inner;
            println!(
                "[bundle162] serialized_file={} unity_version={} objects={} externals={}",
                sf.name,
                inner.unity_version,
                inner.m_objects.len(),
                inner.m_externals.len()
            );
            for (index, ext) in inner.m_externals.iter().enumerate() {
                println!(
                    "[bundle162] external[{}] path_name='{}' file_name='{}'",
                    index + 1,
                    ext.path_name,
                    ext.file_name
                );
            }
            let mut counts: std::collections::BTreeMap<u16, usize> = Default::default();
            for obj in &inner.m_objects {
                *counts.entry(obj.class_id).or_default() += 1;
            }
            for (class_id, count) in &counts {
                println!("[bundle162] class {} count {}", class_id, count);
            }
            for obj in &inner.m_objects {
                if obj.class_id != 1 {
                    continue;
                }
                if let Ok(go) = UnityClassParser::parse_game_object(inner, obj) {
                    println!(
                        "[bundle162] GameObject path_id={} name='{}' components={}",
                        obj.path_id,
                        go.name,
                        go.components.len()
                    );
                }
            }
            for obj in &inner.m_objects {
                if obj.class_id != 199 {
                    continue;
                }
                println!(
                    "[bundle162] ParticleSystemRenderer path_id={} byte_size={}",
                    obj.path_id, obj.byte_size
                );
                match UnityClassParser::read_typetree_value_public(inner, obj) {
                    Some(value) => {
                        let render_mode = find_int_field(&value, "m_RenderMode");
                        let mesh = find_ptr_field_local(&value, "m_Mesh");
                        let mesh1 = find_ptr_field_local(&value, "m_Mesh1");
                        let mesh2 = find_ptr_field_local(&value, "m_Mesh2");
                        let mesh3 = find_ptr_field_local(&value, "m_Mesh3");
                        println!(
                            "[bundle162]   m_RenderMode={:?} m_Mesh={:?} m_Mesh1={:?} m_Mesh2={:?} m_Mesh3={:?}",
                            render_mode, mesh, mesh1, mesh2, mesh3
                        );
                        println!("[bundle162]   full_value={:?}", value);
                    }
                    None => {
                        println!("[bundle162]   no typetree");
                    }
                }
                let raw = inner.object_bytes(obj).unwrap_or(&[]);
                let tail_start = raw.len().saturating_sub(96);
                let hex: Vec<String> = raw[tail_start..]
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect();
                println!(
                    "[bundle162]   raw_len={} tail_hex={}",
                    raw.len(),
                    hex.join(" ")
                );
                for candidate in [84148993i64, 432345564311716609i64] {
                    let found = inner
                        .m_objects
                        .iter()
                        .find(|o| o.path_id == candidate)
                        .map(|o| o.class_id);
                    println!(
                        "[bundle162]   local lookup path_id={} -> class_id={:?}",
                        candidate, found
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "diagnostic against local bundle162 particle prefab asset map"]
    fn debug_bundle162_external_mesh_lookup() {
        let workspace =
            std::path::PathBuf::from(r"D:\AssetDaemonCacheFolder\Windows-a444e50fa47a1f9081013a7b56663e68");
        let db = AssetDatabase::open(&workspace).expect("open real asset map");
        let mesh_path_id: i64 = 0x19C814BFDC10E9E7;
        let resolved = UnityExternalResolver::resolve_from_db(&db, "cab-7ac7c1ef9288043d14d1aa1b05adf618");
        println!("[bundle162] resolve cab-7ac7c1ef -> {:?}", resolved);
        if let Some(bundle_path) = &resolved {
            let asset = db
                .find_by_bundle_and_path_id(bundle_path, mesh_path_id)
                .ok()
                .flatten();
            println!(
                "[bundle162] mesh lookup in resolved bundle: {:?}",
                asset.as_ref().map(|a| (a.class_id, a.class_name.clone(), a.asset_name.clone()))
            );
        }
        let global = db.find_by_path_id(mesh_path_id).unwrap_or_default();
        for asset in &global {
            println!(
                "[bundle162] global path_id hit: bundle={} class={} name={}",
                asset.bundle_path, asset.class_name, asset.asset_name
            );
        }
        let bundles = db.get_bundle_infos().unwrap_or_default();
        for info in &bundles {
            if info.path.ends_with("bundle162") {
                println!(
                    "[bundle162] bundle162 info: unity={} assets={}",
                    info.unity_version, info.asset_count
                );
                let bundle_path = info.path.clone();
                for (label, go) in [
                    ("collision", -8679599894742462877i64),
                    ("PFX_WaterGunSpray", -5557925726780121410i64),
                    ("collision_fill", -1317133557772563333i64),
                    ("fire_extinguisher (2)", -940462217155042052i64),
                    ("collision_smokespread", 7614331831799663186i64),
                ] {
                    let (refs, _, has_particles) =
                        AnimatorPreviewHierarchy::collect_mesh_refs_with_root_fallback(
                            &db,
                            &bundle_path,
                            go,
                        );
                    println!(
                        "[bundle162] preview refs for '{}': refs={} particles={} {:?}",
                        label,
                        refs.len(),
                        has_particles,
                        refs.iter()
                            .map(|r| (r.mesh_name.clone(), r.mesh_bundle_path.clone()))
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
    }

    #[test]
    fn animator_mesh_refs_traverse_parent_links_created_from_transform_children() {
        let workspace = temp_workspace("children-parent-links");
        std::fs::create_dir_all(&workspace).unwrap();
        let db = AssetDatabase::open(&workspace).unwrap();
        let bundle = workspace.join("model_bundle").to_string_lossy().to_string();

        let mut rows = map_bundle(&bundle);
        rows.assets = vec![
            asset(&bundle, 10, 1, "GameObject"),
            asset(&bundle, 20, 1, "GameObject"),
            asset(&bundle, 30, 33, "MeshFilter"),
            asset(&bundle, 40, 23, "MeshRenderer"),
            asset(&bundle, 50, 43, "Mesh"),
        ];
        rows.relations = vec![
            relation(&bundle, "transform_gameobject", 100, 10, 0),
            relation(&bundle, "transform_gameobject", 200, 20, 0),
            RelationWriteRow {
                field_path: "m_Children".to_string(),
                ..relation(&bundle, "transform_parent", 200, 100, 0)
            },
            relation(&bundle, "gameobject_component", 20, 30, 0),
            relation(&bundle, "gameobject_component", 20, 40, 0),
            relation(&bundle, UnityRelationKind::MESH_FILTER_MESH, 30, 50, 0),
        ];
        db.insert_many_map_bundle_write_rows(&[rows]).unwrap();

        let (mesh_refs, diagnostics, has_particles) =
            collect_animator_mesh_refs(&db, &bundle, 10);
        assert!(!has_particles);

        assert!(diagnostics.is_empty());
        assert_eq!(mesh_refs.len(), 1);
        assert_eq!(mesh_refs[0].mesh_path_id, 50);
        assert_eq!(mesh_refs[0].transform_path_id, 200);

        drop(db);
        let _ = std::fs::remove_dir_all(workspace);
    }
}
