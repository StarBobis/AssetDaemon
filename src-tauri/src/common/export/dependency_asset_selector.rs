use std::collections::{HashSet, VecDeque};
use std::path::Path;

use tauri::ipc::Channel;

use crate::common::asset_map::asset_index::{AssetDatabase, AssetRow, RelationRow};
use crate::common::asset_map::live_dependency_resolver::LiveDependencyResolver;
use crate::common::asset_map::repository::AssetMapRepository;
use crate::common::command_types::DependencyExportAssetRef;
use crate::common::scan::scan_types::ProgressPayload;
use crate::common::task::task_context::TaskContext;
use crate::unity::relations::UnityRelationKind;

const DEPENDENCY_EXPORT_CLASSES: &[&str] = &[
    "Texture2D",
    "Sprite",
    "Mesh",
    "AudioClip",
    "TextAsset",
    "Font",
    "Shader",
    "MonoBehaviour",
    "AnimationClip",
    "Animator",
    "Avatar",
    "RuntimeAnimatorController",
    "AnimatorController",
    "AnimatorOverrideController",
    "VideoClip",
    "MovieTexture",
    "Material",
];

pub struct DependencyAssetSelector;

impl DependencyAssetSelector {
    #[allow(dead_code)]
    pub fn select_for_workspace(
        workspace: &Path,
        cache_root: Option<&Path>,
        bundle_path: &str,
        path_id: i64,
        class_name: &str,
    ) -> Result<Vec<DependencyExportAssetRef>, String> {
        let db = AssetMapRepository::open(workspace, cache_root)?;
        Ok(select_from_db(&db, bundle_path, path_id, class_name))
    }

    pub fn select_for_workspace_with_task(
        workspace: &Path,
        cache_root: Option<&Path>,
        bundle_path: &str,
        path_id: i64,
        class_name: &str,
        ctx: &TaskContext,
    ) -> Result<Vec<DependencyExportAssetRef>, String> {
        ctx.progress(
            "asset_map",
            1,
            4,
            "Opening AssetMap for related asset lookup",
        );
        let db = AssetMapRepository::open(workspace, cache_root)?;
        ctx.check_cancelled()?;
        ctx.progress(
            "resolve",
            2,
            4,
            format!("Resolving related assets for {}", class_name),
        );
        let assets = select_from_db_with_task(&db, bundle_path, path_id, class_name, ctx)?;
        ctx.progress(
            "done",
            4,
            4,
            format!("Resolved {} related exportable asset(s)", assets.len()),
        );
        Ok(assets)
    }
}

#[allow(dead_code)]
fn select_from_db(
    db: &AssetDatabase,
    bundle_path: &str,
    path_id: i64,
    class_name: &str,
) -> Vec<DependencyExportAssetRef> {
    select_rows_from_db(db, bundle_path, path_id, class_name, None)
        .unwrap_or_default()
        .into_iter()
        .map(row_to_dependency_ref)
        .collect()
}

fn select_from_db_with_task(
    db: &AssetDatabase,
    bundle_path: &str,
    path_id: i64,
    class_name: &str,
    ctx: &TaskContext,
) -> Result<Vec<DependencyExportAssetRef>, String> {
    Ok(
        select_rows_from_db(db, bundle_path, path_id, class_name, Some(ctx))?
            .into_iter()
            .map(row_to_dependency_ref)
            .collect(),
    )
}

fn select_rows_from_db(
    db: &AssetDatabase,
    bundle_path: &str,
    path_id: i64,
    class_name: &str,
    ctx: Option<&TaskContext>,
) -> Result<Vec<AssetRow>, String> {
    let mut rows = Vec::<AssetRow>::new();
    let mut seen = HashSet::<(String, i64)>::new();

    push_asset(db, &mut rows, &mut seen, bundle_path, path_id);
    check_cancelled(ctx)?;

    match class_name {
        "GameObject" => {}
        "Mesh" => collect_mesh_related_assets_with_task(
            db,
            &mut rows,
            &mut seen,
            bundle_path,
            path_id,
            ctx,
        )?,
        "Material" => collect_material_related_assets_with_task(
            db,
            &mut rows,
            &mut seen,
            bundle_path,
            path_id,
            ctx,
        )?,
        "Animator" | "Animation" => {
            collect_animation_related_assets(db, &mut rows, &mut seen, bundle_path, path_id)
        }
        "AnimatorController" | "RuntimeAnimatorController" | "AnimatorOverrideController" => {
            collect_controller_related_assets(db, &mut rows, &mut seen, bundle_path, path_id)
        }
        _ => {}
    }

    rows.retain(|row| DEPENDENCY_EXPORT_CLASSES.contains(&row.class_name.as_str()));
    rows.sort_by(|left, right| {
        export_class_order(&left.class_name)
            .cmp(&export_class_order(&right.class_name))
            .then_with(|| left.asset_name.cmp(&right.asset_name))
            .then_with(|| left.bundle_path.cmp(&right.bundle_path))
            .then_with(|| left.path_id.cmp(&right.path_id))
    });
    Ok(rows)
}

fn collect_mesh_related_assets_with_task(
    db: &AssetDatabase,
    rows: &mut Vec<AssetRow>,
    seen: &mut HashSet<(String, i64)>,
    mesh_bundle_path: &str,
    mesh_path_id: i64,
    ctx: Option<&TaskContext>,
) -> Result<(), String> {
    push_asset(db, rows, seen, mesh_bundle_path, mesh_path_id);
    let mut material_ids = HashSet::<(String, i64)>::new();

    for renderer_mesh in lookup_relations_by_target(
        db,
        UnityRelationKind::RENDERER_MESH,
        mesh_bundle_path,
        mesh_path_id,
    ) {
        check_cancelled(ctx)?;
        let renderer_bundle = renderer_mesh.bundle_path.clone();
        collect_renderer_assets(
            db,
            rows,
            seen,
            &renderer_bundle,
            renderer_mesh.source_path_id,
            &mut HashSet::new(),
            &mut material_ids,
        );
    }

    for mesh_filter in lookup_relations_by_target(
        db,
        UnityRelationKind::MESH_FILTER_MESH,
        mesh_bundle_path,
        mesh_path_id,
    ) {
        check_cancelled(ctx)?;
        for mesh_filter_go in lookup_relations_by_source(
            db,
            UnityRelationKind::MESH_FILTER_GAMEOBJECT,
            &mesh_filter.bundle_path,
            mesh_filter.source_path_id,
        ) {
            for renderer_go in lookup_relations_by_target(
                db,
                UnityRelationKind::RENDERER_GAMEOBJECT,
                &mesh_filter_go.bundle_path,
                mesh_filter_go.target_path_id,
            ) {
                collect_renderer_assets(
                    db,
                    rows,
                    seen,
                    &renderer_go.bundle_path,
                    renderer_go.source_path_id,
                    &mut HashSet::new(),
                    &mut material_ids,
                );
            }
        }
    }

    for (material_bundle, material_path_id) in material_ids {
        check_cancelled(ctx)?;
        collect_material_related_assets_with_task(
            db,
            rows,
            seen,
            &material_bundle,
            material_path_id,
            ctx,
        )?;
    }
    Ok(())
}

fn collect_renderer_assets(
    db: &AssetDatabase,
    rows: &mut Vec<AssetRow>,
    seen: &mut HashSet<(String, i64)>,
    renderer_bundle_path: &str,
    renderer_path_id: i64,
    mesh_ids: &mut HashSet<(String, i64)>,
    material_ids: &mut HashSet<(String, i64)>,
) {
    push_asset(db, rows, seen, renderer_bundle_path, renderer_path_id);
    for mesh in lookup_relations_by_source(
        db,
        UnityRelationKind::RENDERER_MESH,
        renderer_bundle_path,
        renderer_path_id,
    ) {
        let mesh_bundle = relation_target_bundle(&mesh);
        mesh_ids.insert((mesh_bundle.clone(), mesh.target_path_id));
        push_asset(db, rows, seen, &mesh_bundle, mesh.target_path_id);
    }
    for material in lookup_relations_by_source(
        db,
        UnityRelationKind::RENDERER_MATERIAL,
        renderer_bundle_path,
        renderer_path_id,
    ) {
        let material_bundle = relation_target_bundle(&material);
        material_ids.insert((material_bundle.clone(), material.target_path_id));
        push_asset(db, rows, seen, &material_bundle, material.target_path_id);
    }
}

fn collect_material_related_assets_with_task(
    db: &AssetDatabase,
    rows: &mut Vec<AssetRow>,
    seen: &mut HashSet<(String, i64)>,
    material_bundle_path: &str,
    material_path_id: i64,
    ctx: Option<&TaskContext>,
) -> Result<(), String> {
    push_asset(db, rows, seen, material_bundle_path, material_path_id);
    check_cancelled(ctx)?;
    if let Some(texture_refs) =
        collect_material_texture_assets_live(db, material_bundle_path, material_path_id, ctx)?
    {
        for (target_bundle, target_path_id) in texture_refs {
            check_cancelled(ctx)?;
            if let Some(row) = lookup_asset(db, &target_bundle, target_path_id)
                .filter(|row| row.class_name == "Texture2D" || row.class_name == "Sprite")
            {
                push_row(rows, seen, row);
            }
        }
        return Ok(());
    }

    for relation in lookup_relations_by_source(db, "pptr", material_bundle_path, material_path_id) {
        check_cancelled(ctx)?;
        let target_bundle = relation_target_bundle(&relation);
        if let Some(row) = lookup_asset(db, &target_bundle, relation.target_path_id)
            .filter(|row| row.class_name == "Texture2D" || row.class_name == "Sprite")
        {
            push_row(rows, seen, row);
        }
    }
    Ok(())
}

fn collect_material_texture_assets_live(
    db: &AssetDatabase,
    material_bundle_path: &str,
    material_path_id: i64,
    ctx: Option<&TaskContext>,
) -> Result<Option<Vec<(String, i64)>>, String> {
    check_cancelled(ctx)?;
    let mut resolver = LiveDependencyResolver::new();
    let progress = if let Some(ctx) = ctx {
        ctx.progress(
            "parse_material",
            3,
            4,
            format!(
                "Parsing Material texture references: path_id={}",
                material_path_id
            ),
        );
        Channel::<ProgressPayload>::new(|_| Ok(()))
    } else {
        Channel::<ProgressPayload>::new(|_| Ok(()))
    };
    let Some((_material_info, texture_refs)) =
        resolver.resolve_material_textures(db, material_bundle_path, material_path_id, &progress)
    else {
        if let Some(ctx) = ctx {
            ctx.warn(format!(
                "Material texture parser found no Texture2D references: path_id={}",
                material_path_id
            ));
        }
        return Ok(None);
    };
    check_cancelled(ctx)?;
    if let Some(ctx) = ctx {
        ctx.progress(
            "parse_material",
            4,
            4,
            format!(
                "Material texture parser resolved {} Texture2D reference(s)",
                texture_refs.len()
            ),
        );
    }
    Ok(Some(
        texture_refs
            .into_iter()
            .map(|(path_id, bundle_path, _name)| (bundle_path, path_id))
            .collect(),
    ))
}

fn check_cancelled(ctx: Option<&TaskContext>) -> Result<(), String> {
    if let Some(ctx) = ctx {
        ctx.check_cancelled()?;
    }
    Ok(())
}

fn collect_animation_related_assets(
    db: &AssetDatabase,
    rows: &mut Vec<AssetRow>,
    seen: &mut HashSet<(String, i64)>,
    component_bundle_path: &str,
    component_path_id: i64,
) {
    let mut controller_queue = VecDeque::<(String, i64)>::new();
    for relation_type in [
        UnityRelationKind::ANIMATOR_GAMEOBJECT,
        UnityRelationKind::ANIMATOR_AVATAR,
        UnityRelationKind::ANIMATOR_CONTROLLER,
        UnityRelationKind::ANIMATION_GAMEOBJECT,
        UnityRelationKind::ANIMATION_DEFAULT_CLIP,
        UnityRelationKind::ANIMATION_CLIPS,
    ] {
        for relation in
            lookup_relations_by_source(db, relation_type, component_bundle_path, component_path_id)
        {
            let target_bundle = relation_target_bundle(&relation);
            if let Some(row) = lookup_asset(db, &target_bundle, relation.target_path_id) {
                if is_controller_class(&row.class_name) {
                    controller_queue.push_back((row.bundle_path.clone(), row.path_id));
                }
                push_row(rows, seen, row);
            }
        }
    }

    collect_controller_queue_assets(db, rows, seen, controller_queue);
}

fn collect_controller_related_assets(
    db: &AssetDatabase,
    rows: &mut Vec<AssetRow>,
    seen: &mut HashSet<(String, i64)>,
    bundle_path: &str,
    path_id: i64,
) {
    collect_controller_queue_assets(
        db,
        rows,
        seen,
        VecDeque::from([(bundle_path.to_string(), path_id)]),
    );
}

fn collect_controller_queue_assets(
    db: &AssetDatabase,
    rows: &mut Vec<AssetRow>,
    seen: &mut HashSet<(String, i64)>,
    mut controller_queue: VecDeque<(String, i64)>,
) {
    let mut seen_controllers = HashSet::<(String, i64)>::new();
    while let Some((controller_bundle, controller_path_id)) = controller_queue.pop_front() {
        if !seen_controllers.insert((controller_bundle.clone(), controller_path_id)) {
            continue;
        }
        push_asset(db, rows, seen, &controller_bundle, controller_path_id);
        for relation_type in [
            UnityRelationKind::CONTROLLER_ANIMATION_CLIP,
            UnityRelationKind::OVERRIDE_BASE_CONTROLLER,
            UnityRelationKind::OVERRIDE_CLIP_ORIGINAL,
            UnityRelationKind::OVERRIDE_CLIP_OVERRIDE,
        ] {
            for relation in lookup_relations_by_source(
                db,
                relation_type,
                &controller_bundle,
                controller_path_id,
            ) {
                let target_bundle = relation_target_bundle(&relation);
                if let Some(row) = lookup_asset(db, &target_bundle, relation.target_path_id) {
                    if is_controller_class(&row.class_name) {
                        controller_queue.push_back((row.bundle_path.clone(), row.path_id));
                    }
                    if row.class_name == "AnimationClip" || is_controller_class(&row.class_name) {
                        push_row(rows, seen, row);
                    }
                }
            }
        }
    }
}

fn push_asset(
    db: &AssetDatabase,
    rows: &mut Vec<AssetRow>,
    seen: &mut HashSet<(String, i64)>,
    bundle_path: &str,
    path_id: i64,
) {
    if path_id == 0 {
        return;
    }
    if let Some(row) = lookup_asset(db, bundle_path, path_id) {
        push_row(rows, seen, row);
    }
}

fn lookup_asset(db: &AssetDatabase, bundle_path: &str, path_id: i64) -> Option<AssetRow> {
    db.find_by_bundle_and_path_id(bundle_path, path_id)
        .ok()
        .flatten()
}

fn lookup_relations_by_source(
    db: &AssetDatabase,
    relation_type: &str,
    bundle_path: &str,
    source_path_id: i64,
) -> Vec<RelationRow> {
    db.find_relations_by_bundle_and_source(relation_type, bundle_path, source_path_id)
        .unwrap_or_default()
}

fn lookup_relations_by_target(
    db: &AssetDatabase,
    relation_type: &str,
    bundle_path: &str,
    target_path_id: i64,
) -> Vec<RelationRow> {
    db.find_relations_by_bundle_and_target(relation_type, bundle_path, target_path_id)
        .unwrap_or_default()
}

fn push_row(rows: &mut Vec<AssetRow>, seen: &mut HashSet<(String, i64)>, row: AssetRow) {
    if seen.insert((row.bundle_path.clone(), row.path_id)) {
        rows.push(row);
    }
}

fn relation_target_bundle(relation: &RelationRow) -> String {
    if !relation.target_bundle_path.is_empty() {
        return relation.target_bundle_path.clone();
    }
    if relation.file_id == 0 {
        return relation.bundle_path.clone();
    }
    relation.bundle_path.clone()
}

fn row_to_dependency_ref(row: AssetRow) -> DependencyExportAssetRef {
    DependencyExportAssetRef {
        bundle_path: row.bundle_path,
        path_id: row.path_id.to_string(),
        class_name: row.class_name,
        asset_name: row.asset_name,
        byte_size: row.byte_size,
    }
}

fn export_class_order(class_name: &str) -> usize {
    DEPENDENCY_EXPORT_CLASSES
        .iter()
        .position(|name| *name == class_name)
        .unwrap_or(usize::MAX)
}

fn is_controller_class(class_name: &str) -> bool {
    matches!(
        class_name,
        "AnimatorController" | "RuntimeAnimatorController" | "AnimatorOverrideController"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDirs {
        root: PathBuf,
        workspace: PathBuf,
        cache_root: PathBuf,
    }

    impl TestDirs {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "assetfinder-export-deps-{}-{}-{}",
                name,
                std::process::id(),
                unique
            ));
            let workspace = root.join("workspace");
            let cache_root = root.join("cache");
            fs::create_dir_all(&workspace).unwrap();
            fs::create_dir_all(&cache_root).unwrap();
            Self {
                root,
                workspace,
                cache_root,
            }
        }

        fn bundle_path(&self, file_name: &str) -> String {
            self.workspace.join(file_name).to_string_lossy().to_string()
        }
    }

    impl Drop for TestDirs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn open_cache_db(dirs: &TestDirs) -> AssetDatabase {
        AssetDatabase::open_with_cache_root(&dirs.workspace, Some(&dirs.cache_root)).unwrap()
    }

    #[test]
    fn dependency_export_asset_lookup_uses_cache_root() {
        let dirs = TestDirs::new("asset-lookup");
        let db = open_cache_db(&dirs);
        let bundle_path = dirs.bundle_path("bundle-a");

        db.replace_bundle_entry_no_transaction(
            &bundle_path,
            "md5",
            1,
            1,
            "2022.3",
            &[("", 11, 28, "Texture2D", "hero_diffuse", 256)],
            &[],
            &[],
            &[],
        )
        .unwrap();

        assert!(!Path::new(&dirs.workspace).join("asset_index").exists());

        let assets = select_from_db(&db, &bundle_path, 11, "Texture2D");
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].class_name, "Texture2D");
        assert_eq!(assets[0].asset_name, "hero_diffuse");
    }

    #[test]
    fn dependency_export_relation_target_lookup_uses_cache_root() {
        let dirs = TestDirs::new("relation-target");
        let db = open_cache_db(&dirs);
        let bundle_path = dirs.bundle_path("bundle-a");

        db.replace_bundle_entry_no_transaction(
            &bundle_path,
            "md5",
            1,
            1,
            "2022.3",
            &[
                ("", 10, 1, "GameObject", "hero_root", 64),
                ("", 20, 4, "Transform", "hero_root_transform", 64),
            ],
            &[],
            &[],
            &[(
                "",
                "transform_gameobject",
                20,
                10,
                "hero_root_transform",
                "hero_root",
                0,
                "m_GameObject",
                Cow::Borrowed(""),
            )],
        )
        .unwrap();

        let relations = lookup_relations_by_target(&db, "transform_gameobject", &bundle_path, 10);

        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].source_path_id, 20);
        assert_eq!(relations[0].target_path_id, 10);
    }

    #[test]
    fn dependency_export_asset_lookup_does_not_guess_game_object_preview_assets() {
        let dirs = TestDirs::new("gameobject-deps");
        let db = open_cache_db(&dirs);
        let bundle_path = dirs.bundle_path("bundle-a");

        db.replace_bundle_entry_no_transaction(
            &bundle_path,
            "md5",
            1,
            1,
            "2022.3",
            &[
                ("", 10, 1, "GameObject", "hero_root", 64),
                ("", 30, 23, "MeshRenderer", "hero_renderer", 64),
                ("", 40, 43, "Mesh", "hero_mesh", 256),
                ("", 50, 21, "Material", "hero_mat", 128),
            ],
            &[],
            &[],
            &[
                (
                    "",
                    "gameobject_component",
                    10,
                    30,
                    "hero_root",
                    "hero_renderer",
                    0,
                    "m_Component",
                    Cow::Borrowed(""),
                ),
                (
                    "",
                    UnityRelationKind::RENDERER_MESH,
                    30,
                    40,
                    "hero_renderer",
                    "hero_mesh",
                    0,
                    "m_Mesh",
                    Cow::Borrowed(""),
                ),
                (
                    "",
                    UnityRelationKind::RENDERER_MATERIAL,
                    30,
                    50,
                    "hero_renderer",
                    "hero_mat",
                    0,
                    "m_Materials[0]",
                    Cow::Borrowed(""),
                ),
            ],
        )
        .unwrap();

        let assets = select_from_db(&db, &bundle_path, 10, "GameObject");

        assert!(assets.is_empty());
    }
}
