/*
 * workspace/hierarchy_service.rs -- Scene Hierarchy building service
 *
 * Note: In serialized_file::ObjectInfo, class_id is always the real Unity class ID,
 * while type_id is the class_id itself in older versions and a type table index in 2019+.
 * All type filtering must use class_id, not type_id.
 */

use crate::common::bundle_file::asset_bundle::{
    AssetBundleLoader, SerializedFile as BundleSerializedFile,
};
use crate::common::scene::scene_types::*;
use crate::common::serialized_file::serialized_file::SerializedFile;
use crate::unity::classes::registry::UnityClassParser;
use std::collections::HashMap;

pub struct HierarchyBuilder;

impl HierarchyBuilder {
    /**
     * Build scene hierarchy tree, returning tree structure + diagnostic logs.
     */
    pub fn build_hierarchy(
        assets: &[BundleSerializedFile],
        logs: &mut Vec<String>,
    ) -> SceneHierarchyResult {
        let mut roots: Vec<SceneNode> = Vec::new();
        let mut diag: Vec<String> = Vec::new();

        diag.push(format!(
            "build_hierarchy: {} SerializedFile(s)",
            assets.len()
        ));

        for (file_idx, sf) in assets.iter().enumerate() {
            let inner = &sf.inner;
            if inner.m_objects.is_empty() {
                diag.push(format!(
                    "  file[{}]: m_objects is empty, skipping",
                    file_idx
                ));
                continue;
            }

            diag.push(format!(
                "  file[{}]: {} object(s), version={:?}",
                file_idx,
                inner.m_objects.len(),
                inner.version,
            ));

            // Count distribution of all class_ids
            let mut class_counts: HashMap<u16, usize> = HashMap::new();
            for obj in &inner.m_objects {
                *class_counts.entry(obj.class_id).or_insert(0) += 1;
            }
            let mut sorted_classes: Vec<_> = class_counts.into_iter().collect();
            sorted_classes.sort();
            for (cid, count) in &sorted_classes {
                let name = AssetBundleLoader::get_class_name(*cid as i32).unwrap_or("?");
                diag.push(format!(
                    "    class_id {} ({}): {} object(s)",
                    cid, name, count
                ));
            }

            let transforms = Self::collect_by_class(&inner.m_objects, 4);
            let game_objects = Self::collect_by_class(&inner.m_objects, 1);

            diag.push(format!(
                "    Filter results: GameObject(class_id=1)={}, Transform(class_id=4)={}",
                game_objects.len(),
                transforms.len(),
            ));

            if game_objects.is_empty() {
                diag.push("    -> No GameObject, skipping this file".to_string());
                continue;
            }

            // Transform path_id -> father Transform path_id
            let mut child_to_parent: HashMap<i64, i64> = HashMap::new();
            for (path_id, _) in &transforms {
                if let Some(fid) = Self::read_transform_father(inner, *path_id) {
                    child_to_parent.insert(*path_id, fid);
                    diag.push(format!(
                        "    Transform(path_id={}) -> father Transform(path_id={})",
                        path_id, fid,
                    ));
                } else {
                    diag.push(format!(
                        "    Transform(path_id={}) -> no father (root)",
                        path_id,
                    ));
                }
            }

            // GameObject path_id -> (name, components)
            let mut go_names: HashMap<i64, String> = HashMap::new();
            let mut go_components: HashMap<i64, Vec<ComponentRef>> = HashMap::new();
            for (path_id, _) in &game_objects {
                let (n, c) = Self::read_game_object(inner, *path_id);
                diag.push(format!(
                    "    GameObject(path_id={}) name='{}' ({} component(s))",
                    path_id,
                    if n.is_empty() { "(empty)" } else { &n },
                    c.len(),
                ));
                go_names.insert(*path_id, n);
                go_components.insert(*path_id, c);
            }

            // Transform path_id -> GameObject path_id (from Component base class)
            let mut transform_to_go: HashMap<i64, i64> = HashMap::new();
            for (path_id, _) in &transforms {
                if let Some(gid) = Self::read_component_gameobject(inner, *path_id) {
                    transform_to_go.insert(*path_id, gid);
                    diag.push(format!(
                        "    Transform(path_id={}) -> GameObject(path_id={})",
                        path_id, gid,
                    ));
                } else {
                    diag.push(format!(
                        "    Transform(path_id={}) -> m_GameObject NOT FOUND",
                        path_id,
                    ));
                }
            }

            // parent_go_path_id -> children_go_path_ids
            let mut parent_to_children: HashMap<i64, Vec<i64>> = HashMap::new();
            let mut is_child: HashMap<i64, bool> = HashMap::new();
            let mut orphan_transforms = 0;

            for (tid, _) in &transforms {
                let go_id = match transform_to_go.get(tid) {
                    Some(v) => *v,
                    None => {
                        orphan_transforms += 1;
                        continue;
                    }
                };
                if !game_objects.contains_key(&go_id) {
                    orphan_transforms += 1;
                    continue;
                }
                let ftid = match child_to_parent.get(tid) {
                    Some(v) => *v,
                    None => continue,
                };
                let fgo_id = match transform_to_go.get(&ftid) {
                    Some(v) => *v,
                    None => continue,
                };
                if !game_objects.contains_key(&fgo_id) {
                    continue;
                }
                parent_to_children.entry(fgo_id).or_default().push(go_id);
                is_child.insert(go_id, true);
            }

            if orphan_transforms > 0 {
                diag.push(format!(
                    "    {} Transform(s) without corresponding GameObject",
                    orphan_transforms
                ));
            }

            // Build all SceneNode entries
            let mut all_nodes: HashMap<i64, SceneNode> = HashMap::new();
            for (pid, _) in &game_objects {
                all_nodes.insert(
                    *pid,
                    SceneNode {
                        name: go_names.get(pid).cloned().unwrap_or_default(),
                        path_id: *pid,
                        class_id: 1,
                        children: Vec::new(),
                        components: go_components.get(pid).cloned().unwrap_or_default(),
                    },
                );
            }

            // Roots: GameObjects that are nobody's child
            let mut root_ids: Vec<i64> = game_objects
                .keys()
                .filter(|id| !is_child.contains_key(id))
                .copied()
                .collect();
            root_ids.sort();

            diag.push(format!(
                "    Root nodes: {}, Child nodes: {}",
                root_ids.len(),
                is_child.len(),
            ));

            // Collect all root nodes + orphan nodes for this file
            let mut file_roots: Vec<SceneNode> = Vec::new();
            for rid in &root_ids {
                if let Some(n) = all_nodes.remove(rid) {
                    file_roots.push(Self::build_tree(n, &mut all_nodes, &parent_to_children));
                }
            }
            let mut orphan_ids: Vec<i64> = all_nodes.keys().copied().collect();
            orphan_ids.sort();
            if !orphan_ids.is_empty() {
                diag.push(format!("    Orphan nodes: {}", orphan_ids.len()));
            }
            for oid in orphan_ids {
                if let Some(n) = all_nodes.remove(&oid) {
                    file_roots.push(Self::build_tree(n, &mut all_nodes, &parent_to_children));
                }
            }

            // Wrap with CAB name (AssetStudio style)
            if !file_roots.is_empty() {
                let cab_name = sf.name.rsplit('/').next().unwrap_or(&sf.name).to_string();
                if file_roots.len() == 1 {
                    roots.push(file_roots.swap_remove(0));
                } else {
                    roots.push(SceneNode {
                        name: cab_name,
                        path_id: -(file_idx as i64 + 1),
                        class_id: 0,
                        children: file_roots,
                        components: Vec::new(),
                    });
                }
            }
        }

        diag.push(format!(
            "build_hierarchy complete: {} root node(s) total",
            roots.len()
        ));
        logs.extend(diag.clone());

        SceneHierarchyResult {
            roots,
            diagnostics: diag,
        }
    }

    fn collect_by_class<'a>(
        objects: &'a [crate::common::serialized_file::serialized_file::ObjectInfo],
        class_id: u16,
    ) -> HashMap<i64, &'a crate::common::serialized_file::serialized_file::ObjectInfo> {
        objects
            .iter()
            .filter(|o| o.class_id == class_id)
            .map(|o| (o.path_id, o))
            .collect()
    }

    fn read_transform_father(sf: &SerializedFile, path_id: i64) -> Option<i64> {
        let obj = sf
            .m_objects
            .iter()
            .find(|o| o.path_id == path_id && o.class_id == 4)?;
        let transform = UnityClassParser::parse_transform(sf, obj).ok()?;
        if transform.father.is_null() {
            None
        } else {
            Some(transform.father.path_id)
        }
    }

    fn read_component_gameobject(sf: &SerializedFile, path_id: i64) -> Option<i64> {
        let obj = sf
            .m_objects
            .iter()
            .find(|o| o.path_id == path_id && o.class_id == 4)?;
        let transform = UnityClassParser::parse_transform(sf, obj).ok()?;
        if transform.game_object.is_null() {
            None
        } else {
            Some(transform.game_object.path_id)
        }
    }

    fn read_game_object(sf: &SerializedFile, path_id: i64) -> (String, Vec<ComponentRef>) {
        let obj = match sf
            .m_objects
            .iter()
            .find(|o| o.path_id == path_id && o.class_id == 1)
        {
            Some(o) => o,
            None => return (String::new(), Vec::new()),
        };
        let game_object = match UnityClassParser::parse_game_object(sf, obj) {
            Ok(go) => go,
            Err(_) => return (String::new(), Vec::new()),
        };
        let mut comps = Vec::new();
        for component in game_object.components {
            let cid = component.path_id;
            let cn = sf
                .m_objects
                .iter()
                .find(|o| o.path_id == cid)
                .map(|o| {
                    AssetBundleLoader::get_class_name(o.class_id as i32)
                        .unwrap_or("Unknown")
                        .to_string()
                })
                .unwrap_or_default();
            let ci = sf
                .m_objects
                .iter()
                .find(|o| o.path_id == cid)
                .map(|o| o.class_id)
                .unwrap_or(0);
            comps.push(ComponentRef {
                path_id: cid,
                class_name: cn,
                class_id: ci as i32,
            });
        }
        (game_object.name, comps)
    }

    fn build_tree(
        node: SceneNode,
        all_nodes: &mut HashMap<i64, SceneNode>,
        parent_to_children: &HashMap<i64, Vec<i64>>,
    ) -> SceneNode {
        let mut r = node;
        if let Some(ids) = parent_to_children.get(&r.path_id) {
            let mut s = ids.clone();
            s.sort();
            for cid in s {
                if let Some(cn) = all_nodes.remove(&cid) {
                    r.children
                        .push(Self::build_tree(cn, all_nodes, parent_to_children));
                }
            }
        }
        r
    }
}
