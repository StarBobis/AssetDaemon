use std::collections::{HashMap, HashSet};

use std::path::Path;

use crate::common::asset_map::asset_index::AssetDatabase;
use crate::common::bundle_file::asset_bundle::{AssetBundle, AssetBundleLoader};
use crate::common::unity_dependency::external_resolver::UnityExternalResolver;
use crate::unity::classes::object::PPtr;
use crate::unity::classes::registry::{
    SkinnedMeshRendererObject, UnityClassObject, UnityClassParser,
};
use crate::unity::relations::UnityRelationKind;

#[derive(Debug, Clone)]
pub struct GlbSkeleton {
    /// Exported Transform nodes. This may include non-skin ancestor nodes so
    /// that skin joints keep Unity's real hierarchy instead of becoming a
    /// flattened bone list.
    pub joints: Vec<GlbJoint>,
    /// Indices into `joints`, ordered exactly like Unity's SkinnedMeshRenderer
    /// m_Bones array or Mesh m_BoneNameHashes/m_BindPose order.
    pub skin_joints: Vec<usize>,
    /// Unity path/name hashes for `skin_joints`, when available from Mesh.m_BoneNameHashes.
    pub skin_joint_hashes: Vec<u32>,
    /// Transform node that owns the renderer mesh, when it is available.
    pub mesh_parent: Option<usize>,
    /// Root joint for the glTF skin. This may be below a scene root.
    pub skeleton_root: Option<usize>,
    pub roots: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct GlbJoint {
    pub name: String,
    pub path: String,
    pub transform_path_id: i64,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

pub struct ModelContextResolver;

impl ModelContextResolver {
    pub fn resolve_skeleton_for_renderer_with_hashes(
        bundle: &AssetBundle,
        renderer_path_id: i64,
        mesh_path_id: i64,
        bind_pose_count: usize,
        bone_name_hashes: &[u32],
        root_bone_name_hash: Option<u32>,
    ) -> Option<GlbSkeleton> {
        let (sf, obj) = bundle.assets.iter().find_map(|sf| {
            sf.objects
                .iter()
                .find(|obj| obj.path_id == renderer_path_id && obj.class_id == 137)
                .map(|obj| (sf, obj))
        })?;
        let Ok(UnityClassObject::SkinnedMeshRenderer(renderer)) =
            UnityClassParser::parse_object(&sf.inner, &obj.inner, &bundle.resources)
        else {
            return None;
        };
        Self::resolve_skeleton_from_renderer(
            bundle,
            &renderer,
            mesh_path_id,
            bind_pose_count,
            bone_name_hashes,
            root_bone_name_hash,
        )
    }

    pub fn resolve_skeleton_for_renderer_with_map(
        bundle: &AssetBundle,
        bundle_path: &str,
        renderer_path_id: i64,
        mesh_path_id: i64,
        mesh_bundle_path: &str,
        db: Option<&AssetDatabase>,
        bind_pose_count: usize,
        bone_name_hashes: &[u32],
        root_bone_name_hash: Option<u32>,
    ) -> Option<GlbSkeleton> {
        let (sf_index, sf, obj) = bundle.assets.iter().enumerate().find_map(|(index, sf)| {
            sf.objects
                .iter()
                .find(|obj| obj.path_id == renderer_path_id && obj.class_id == 137)
                .map(|obj| (index, sf, obj))
        })?;
        let Ok(UnityClassObject::SkinnedMeshRenderer(renderer)) =
            UnityClassParser::parse_object(&sf.inner, &obj.inner, &bundle.resources)
        else {
            return None;
        };
        Self::resolve_skeleton_from_renderer_with_context(
            bundle,
            bundle_path,
            sf_index,
            &renderer,
            mesh_path_id,
            mesh_bundle_path,
            db,
            bind_pose_count,
            bone_name_hashes,
            root_bone_name_hash,
        )
    }

    pub fn resolve_skeleton_for_mesh_with_hashes(
        bundle: &AssetBundle,
        mesh_path_id: i64,
        bind_pose_count: usize,
        bone_name_hashes: &[u32],
        root_bone_name_hash: Option<u32>,
    ) -> Option<GlbSkeleton> {
        for sf in &bundle.assets {
            for obj in &sf.objects {
                if obj.class_id != 137 {
                    continue;
                }
                let Ok(UnityClassObject::SkinnedMeshRenderer(renderer)) =
                    UnityClassParser::parse_object(&sf.inner, &obj.inner, &bundle.resources)
                else {
                    continue;
                };
                if renderer.renderer.mesh.map(|mesh| mesh.path_id) != Some(mesh_path_id) {
                    continue;
                }

                if let Some(skeleton) = Self::resolve_skeleton_from_renderer(
                    bundle,
                    &renderer,
                    mesh_path_id,
                    bind_pose_count,
                    bone_name_hashes,
                    root_bone_name_hash,
                ) {
                    return Some(skeleton);
                }
            }
        }

        if bind_pose_count > 0 && bone_name_hashes.len() == bind_pose_count {
            Self::build_skeleton_from_bone_hashes(
                bundle,
                bone_name_hashes,
                root_bone_name_hash,
                None,
                bind_pose_count,
            )
        } else {
            None
        }
    }

    fn resolve_skeleton_from_renderer(
        bundle: &AssetBundle,
        renderer: &SkinnedMeshRendererObject,
        mesh_path_id: i64,
        bind_pose_count: usize,
        bone_name_hashes: &[u32],
        root_bone_name_hash: Option<u32>,
    ) -> Option<GlbSkeleton> {
        Self::resolve_skeleton_from_renderer_with_context(
            bundle,
            "",
            0,
            renderer,
            mesh_path_id,
            "",
            None,
            bind_pose_count,
            bone_name_hashes,
            root_bone_name_hash,
        )
    }

    fn resolve_skeleton_from_renderer_with_context(
        bundle: &AssetBundle,
        bundle_path: &str,
        serialized_file_index: usize,
        renderer: &SkinnedMeshRendererObject,
        mesh_path_id: i64,
        mesh_bundle_path: &str,
        db: Option<&AssetDatabase>,
        bind_pose_count: usize,
        bone_name_hashes: &[u32],
        root_bone_name_hash: Option<u32>,
    ) -> Option<GlbSkeleton> {
        if !Self::pptr_matches_bundle_path(
            &renderer.renderer.mesh?,
            mesh_path_id,
            mesh_bundle_path,
            bundle_path,
            serialized_file_index,
            bundle,
            db,
        ) {
            return None;
        }

        let bone_refs: Vec<(String, i64)> = renderer
            .bones
            .iter()
            .filter_map(|bone| {
                Self::resolve_pptr_bundle_path(bone, bundle_path, serialized_file_index, bundle, db)
                    .map(|path| (path, bone.path_id))
            })
            .collect();
        let root_bone = renderer.root_bone.and_then(|bone| {
            Self::resolve_pptr_bundle_path(&bone, bundle_path, serialized_file_index, bundle, db)
                .map(|path| (path, bone.path_id))
        });
        let mesh_transform =
            Self::find_transform_by_game_object(bundle, renderer.renderer.game_object.path_id);

        if !bone_refs.is_empty() && bind_pose_count > 0 && bone_name_hashes.len() == bind_pose_count
        {
            if let Some(filtered_bone_refs) = Self::select_renderer_bones_by_hashes(
                bundle,
                bundle_path,
                &bone_refs,
                bone_name_hashes,
            ) {
                if let Some(skeleton) = Self::build_skeleton_with_external_bones(
                    bundle,
                    bundle_path,
                    &filtered_bone_refs,
                    root_bone.clone(),
                    mesh_transform,
                    bind_pose_count,
                    bone_name_hashes,
                ) {
                    return Some(skeleton);
                }
            }
        }

        // Fall back to direct m_Bones order when the Mesh has no complete bone-name hash table.
        if !bone_refs.is_empty() && bone_refs.len() == bind_pose_count {
            if let Some(skeleton) = Self::build_skeleton_with_external_bones(
                bundle,
                bundle_path,
                &bone_refs,
                root_bone.clone(),
                mesh_transform,
                bind_pose_count,
                &[],
            ) {
                return Some(skeleton);
            }
        }

        if bind_pose_count > 0 && bone_name_hashes.len() == bind_pose_count {
            return Self::build_skeleton_from_bone_hashes(
                bundle,
                bone_name_hashes,
                root_bone_name_hash,
                mesh_transform,
                bind_pose_count,
            );
        }

        None
    }

    fn select_renderer_bones_by_hashes(
        bundle: &AssetBundle,
        bundle_path: &str,
        bone_refs: &[(String, i64)],
        bone_name_hashes: &[u32],
    ) -> Option<Vec<(String, i64)>> {
        let mut external_bundles = HashMap::<String, AssetBundle>::new();
        for path in bone_refs.iter().map(|(path, _)| path) {
            if path == bundle_path || external_bundles.contains_key(path) {
                continue;
            }
            let loaded = AssetBundleLoader::load_bundle(Path::new(path)).ok()?;
            external_bundles.insert(path.clone(), loaded);
        }

        let mut transforms_by_bundle = HashMap::<String, HashMap<i64, TransformInfo>>::new();
        transforms_by_bundle.insert(
            bundle_path.to_string(),
            Self::read_all_transform_infos(bundle),
        );
        for (path, loaded) in &external_bundles {
            transforms_by_bundle.insert(path.clone(), Self::read_all_transform_infos(loaded));
        }

        Self::select_renderer_bones_by_hashes_from_transforms(
            bone_refs,
            bone_name_hashes,
            &transforms_by_bundle,
        )
    }

    fn select_renderer_bones_by_hashes_from_transforms(
        bone_refs: &[(String, i64)],
        bone_name_hashes: &[u32],
        transforms_by_bundle: &HashMap<String, HashMap<i64, TransformInfo>>,
    ) -> Option<Vec<(String, i64)>> {
        let mut bone_by_hash = HashMap::<u32, (String, i64)>::new();
        for (path, transform_id) in bone_refs {
            let transforms = transforms_by_bundle.get(path)?;
            let info = transforms.get(transform_id)?;
            let full_path = Self::build_transform_path(*transform_id, transforms);
            let mut suffix = full_path.as_str();
            loop {
                bone_by_hash
                    .entry(Self::unity_crc32(suffix.as_bytes()))
                    .or_insert_with(|| (path.clone(), *transform_id));
                let Some((_, next_suffix)) = suffix.split_once('/') else {
                    break;
                };
                suffix = next_suffix;
            }
            bone_by_hash
                .entry(Self::unity_crc32(info.name.as_bytes()))
                .or_insert_with(|| (path.clone(), *transform_id));
        }

        let mut selected = Vec::with_capacity(bone_name_hashes.len());
        for hash in bone_name_hashes {
            selected.push(bone_by_hash.get(hash)?.clone());
        }
        Some(selected)
    }

    pub fn resolve_skeleton_for_mesh_with_map(
        mesh_bundle: &AssetBundle,
        mesh_path_id: i64,
        mesh_bundle_path: &str,
        db: Option<&crate::common::asset_map::asset_index::AssetDatabase>,
        bind_pose_count: usize,
        bone_name_hashes: &[u32],
        root_bone_name_hash: Option<u32>,
    ) -> Option<(GlbSkeleton, Option<String>)> {
        if let Some(skeleton) = Self::resolve_skeleton_for_mesh_with_hashes(
            mesh_bundle,
            mesh_path_id,
            bind_pose_count,
            bone_name_hashes,
            root_bone_name_hash,
        ) {
            return Some((skeleton, None));
        }

        let mut candidate_bundles = Vec::<String>::new();
        if let Some(db) = db {
            for relation_type in [
                UnityRelationKind::RENDERER_MESH,
                UnityRelationKind::MESH_FILTER_MESH,
            ] {
                if let Ok(rows) = db.find_relations_by_bundle_and_target(
                    relation_type,
                    mesh_bundle_path,
                    mesh_path_id,
                ) {
                    for row in rows {
                        if !candidate_bundles.contains(&row.bundle_path) {
                            candidate_bundles.push(row.bundle_path);
                        }
                    }
                }
            }
        }
        if !candidate_bundles.contains(&mesh_bundle_path.to_string()) {
            candidate_bundles.push(mesh_bundle_path.to_string());
        }

        for bundle_path in candidate_bundles {
            if bundle_path == mesh_bundle_path {
                if let Some(skeleton) = Self::resolve_skeleton_for_mesh_with_hashes(
                    mesh_bundle,
                    mesh_path_id,
                    bind_pose_count,
                    bone_name_hashes,
                    root_bone_name_hash,
                ) {
                    return Some((skeleton, None));
                }
            } else {
                let path = std::path::Path::new(&bundle_path);
                if !path.exists() {
                    continue;
                }
                let candidate =
                    match crate::common::bundle_file::asset_bundle::AssetBundleLoader::load_bundle(
                        path,
                    ) {
                        Ok(bundle) => bundle,
                        Err(_) => continue,
                    };
                if let Some(skeleton) = Self::resolve_skeleton_for_mesh_with_hashes(
                    &candidate,
                    mesh_path_id,
                    bind_pose_count,
                    bone_name_hashes,
                    root_bone_name_hash,
                ) {
                    return Some((skeleton, Some(bundle_path)));
                }
            }
        }

        None
    }

    pub fn transform_path_by_name(skeleton: &GlbSkeleton) -> HashMap<String, usize> {
        let mut result = HashMap::new();
        for (i, joint) in skeleton.joints.iter().enumerate() {
            result.entry(joint.name.clone()).or_insert(i);
            result.entry(joint.name.to_lowercase()).or_insert(i);
            result.entry(joint.path.clone()).or_insert(i);
            result.entry(joint.path.to_lowercase()).or_insert(i);
            let mut suffix = joint.path.as_str();
            loop {
                result.entry(suffix.to_owned()).or_insert(i);
                result.entry(suffix.to_lowercase()).or_insert(i);
                let Some((_, next_suffix)) = suffix.split_once('/') else {
                    break;
                };
                suffix = next_suffix;
            }
        }
        result
    }

    pub fn transform_path_by_unity_hash(skeleton: &GlbSkeleton) -> HashMap<u32, usize> {
        let mut result = HashMap::new();
        for (skin_index, hash) in skeleton.skin_joint_hashes.iter().copied().enumerate() {
            if let Some(joint_index) = skeleton.skin_joints.get(skin_index).copied() {
                result.entry(hash).or_insert(joint_index);
            }
        }
        for (joint_index, joint) in skeleton.joints.iter().enumerate() {
            let mut suffix = joint.path.as_str();
            loop {
                result
                    .entry(Self::unity_crc32(suffix.as_bytes()))
                    .or_insert(joint_index);
                let Some((_, next_suffix)) = suffix.split_once('/') else {
                    break;
                };
                suffix = next_suffix;
            }
            result
                .entry(Self::unity_crc32(joint.name.as_bytes()))
                .or_insert(joint_index);
        }
        result
    }

    pub fn extend_skeleton_with_transform_paths(
        bundle: &AssetBundle,
        skeleton: &GlbSkeleton,
        target_paths: &HashMap<u32, String>,
    ) -> GlbSkeleton {
        if target_paths.is_empty() {
            return skeleton.clone();
        }
        let transforms = Self::read_all_transform_infos(bundle);

        // Pre-compute lookup maps: name → transform id, path → transform id
        let mut name_to_tid: HashMap<String, i64> = HashMap::new();
        let mut path_to_tid: HashMap<String, i64> = HashMap::new();
        for (&tid, info) in &transforms {
            name_to_tid.entry(info.name.to_lowercase()).or_insert(tid);
            let path = Self::build_transform_path(tid, &transforms);
            let lower = path.to_lowercase();
            path_to_tid.entry(lower.clone()).or_insert(tid);
            // Also store path suffixes
            let mut suffix = lower.as_str();
            loop {
                path_to_tid.entry(suffix.to_owned()).or_insert(tid);
                if let Some((_, rest)) = suffix.split_once('/') {
                    suffix = rest;
                } else {
                    break;
                }
            }
        }

        // Resolve each TOS path to a transform id
        let mut path_to_resolved: HashMap<String, Option<i64>> = HashMap::new();
        for path in target_paths.values() {
            if path_to_resolved.contains_key(path.as_str()) {
                continue;
            }
            let normalized = path.replace('\\', "/");
            let lower = normalized.to_lowercase();
            // Try full path match
            let resolved = path_to_tid.get(&lower).copied()
                // Try suffix matches
                .or_else(|| {
                    let mut suffix = lower.as_str();
                    loop {
                        if let Some(&tid) = path_to_tid.get(suffix) {
                            return Some(tid);
                        }
                        if let Some((_, rest)) = suffix.split_once('/') {
                            suffix = rest;
                        } else {
                            break;
                        }
                    }
                    None
                })
                // Try last component name match
                .or_else(|| {
                    lower.rsplit('/').next()
                        .and_then(|name| name_to_tid.get(name).copied())
                });
            path_to_resolved.insert(path.clone(), resolved);
        }

        let mut extended = skeleton.clone();
        let mut joint_by_transform: HashMap<i64, usize> = extended
            .joints
            .iter()
            .enumerate()
            .map(|(index, joint)| (joint.transform_path_id, index))
            .collect();
        let mut name_to_joint: HashMap<String, usize> = extended
            .joints
            .iter()
            .enumerate()
            .map(|(i, j)| (j.name.to_lowercase(), i))
            .collect();

        // Phase 1: add transforms found in bundle (with parent chain)
        let resolved_ids: HashSet<i64> = path_to_resolved.values().filter_map(|&v| v).collect();
        if !resolved_ids.is_empty() {
            let mut wanted = HashSet::<i64>::new();
            for &tid in &resolved_ids {
                let mut current = tid;
                while current != 0 && transforms.contains_key(&current) {
                    if joint_by_transform.contains_key(&current) {
                        break;
                    }
                    wanted.insert(current);
                    current = transforms.get(&current)
                        .map(|info| info.parent_transform)
                        .unwrap_or(0);
                }
            }
            if !wanted.is_empty() {
                let mut ordered: Vec<i64> = wanted.iter().copied().collect();
                ordered.sort_by_key(|id| Self::transform_depth(*id, &transforms));
                for tid in ordered {
                    if joint_by_transform.contains_key(&tid) { continue; }
                    let Some(info) = transforms.get(&tid) else { continue; };
                    let index = extended.joints.len();
                    let parent = joint_by_transform.get(&info.parent_transform).copied();
                    let path = Self::build_transform_path(tid, &transforms);
                    extended.joints.push(GlbJoint {
                        name: info.name.clone(),
                        path,
                        transform_path_id: tid,
                        parent,
                        children: Vec::new(),
                        translation: info.translation,
                        rotation: info.rotation,
                        scale: info.scale,
                    });
                    joint_by_transform.insert(tid, index);
                    name_to_joint.insert(info.name.to_lowercase(), index);
                    if let Some(p) = parent {
                        if !extended.joints[p].children.contains(&index) {
                            extended.joints[p].children.push(index);
                        }
                    }
                }
            }
        }

        // Phase 2: create synthetic joints for unresolved paths
        for (_hash, path) in target_paths {
            if path_to_resolved.get(path.as_str()).copied().flatten().is_some() {
                continue; // already resolved via transform
            }
            let normalized = path.replace('\\', "/");
            let components: Vec<&str> = normalized.split('/').collect();
            if components.is_empty() { continue; }
            let mut parent: Option<usize> = None;
            // Find deepest existing ancestor
            for end in (1..=components.len()).rev() {
                let ancestor = components[..end].join("/").to_lowercase();
                if let Some(&idx) = name_to_joint.get(&ancestor) {
                    parent = Some(idx);
                    break;
                }
            }
            // Add missing components
            for i in 0..components.len() {
                let comp_lower = components[i].to_lowercase();
                if let Some(&existing) = name_to_joint.get(&comp_lower) {
                    parent = Some(existing);
                    continue;
                }
                let index = extended.joints.len();
                let path_str = components[..=i].join("/");
                extended.joints.push(GlbJoint {
                    name: components[i].to_string(),
                    path: path_str.clone(),
                    transform_path_id: 0,
                    parent,
                    children: Vec::new(),
                    translation: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                });
                name_to_joint.insert(comp_lower, index);
                name_to_joint.insert(path_str.to_lowercase(), index);
                if let Some(p) = parent {
                    if !extended.joints[p].children.contains(&index) {
                        extended.joints[p].children.push(index);
                    }
                }
                parent = Some(index);
            }
        }

        extended.roots = extended
            .joints
            .iter()
            .enumerate()
            .filter_map(|(index, joint)| joint.parent.is_none().then_some(index))
            .collect();
        if extended.roots.is_empty() && !extended.joints.is_empty() {
            extended.roots.push(0);
        }
        extended
    }

    fn build_skeleton(
        bundle: &AssetBundle,
        bone_path_ids: &[i64],
        root_bone: Option<i64>,
        mesh_transform: Option<i64>,
        bind_pose_count: usize,
        skin_joint_hashes: &[u32],
    ) -> Option<GlbSkeleton> {
        let transforms = Self::read_all_transform_infos(bundle);
        if transforms.is_empty() {
            return None;
        }

        let valid_bones: Vec<i64> = bone_path_ids
            .iter()
            .copied()
            .filter(|id| transforms.contains_key(id))
            .collect();
        if valid_bones.is_empty() {
            return None;
        }

        let mut wanted: HashSet<i64> = HashSet::new();
        for transform_id in &valid_bones {
            let mut current = *transform_id;
            while current != 0 && wanted.insert(current) {
                let Some(info) = transforms.get(&current) else {
                    break;
                };
                current = info.parent_transform;
            }
        }
        if let Some(root_transform_id) = root_bone {
            let mut current = root_transform_id;
            while current != 0 && transforms.contains_key(&current) && wanted.insert(current) {
                current = transforms
                    .get(&current)
                    .map(|info| info.parent_transform)
                    .unwrap_or(0);
            }
        }
        if let Some(mesh_transform_id) = mesh_transform {
            let mut current = mesh_transform_id;
            while current != 0 && transforms.contains_key(&current) && wanted.insert(current) {
                current = transforms
                    .get(&current)
                    .map(|info| info.parent_transform)
                    .unwrap_or(0);
            }
        }

        let mut joint_by_transform: HashMap<i64, usize> = HashMap::new();
        let mut joints = Vec::with_capacity(wanted.len());
        let mut ordered = wanted.iter().copied().collect::<Vec<_>>();
        ordered.sort_by_key(|id| Self::transform_depth(*id, &transforms));

        for transform_id in ordered {
            let info = transforms.get(&transform_id)?;
            let index = joints.len();
            joint_by_transform.insert(transform_id, index);
            joints.push(GlbJoint {
                name: info.name.clone(),
                path: Self::build_transform_path(transform_id, &transforms),
                transform_path_id: transform_id,
                parent: None,
                children: Vec::new(),
                translation: info.translation,
                rotation: info.rotation,
                scale: info.scale,
            });
        }

        for transform_id in wanted {
            let Some(joint_index) = joint_by_transform.get(&transform_id).copied() else {
                continue;
            };
            let Some(info) = transforms.get(&transform_id) else {
                continue;
            };
            if let Some(parent_index) = joint_by_transform.get(&info.parent_transform).copied() {
                joints[joint_index].parent = Some(parent_index);
                joints[parent_index].children.push(joint_index);
            }
        }

        let skin_joints = valid_bones
            .iter()
            .filter_map(|id| joint_by_transform.get(id).copied())
            .collect::<Vec<_>>();
        if skin_joints.is_empty() {
            return None;
        }
        if bind_pose_count > 0 && skin_joints.len() != bind_pose_count {
            return None;
        }
        let mesh_parent = mesh_transform.and_then(|id| joint_by_transform.get(&id).copied());

        let mut roots: Vec<usize> = joints
            .iter()
            .enumerate()
            .filter_map(|(i, joint)| {
                if joint.parent.is_none() {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();
        let skeleton_root = root_bone
            .and_then(|root_transform_id| joint_by_transform.get(&root_transform_id).copied())
            .or_else(|| skin_joints.first().copied());
        if let Some(root_transform_id) = root_bone {
            if let Some(root_index) = joint_by_transform.get(&root_transform_id).copied() {
                if joints[root_index].parent.is_none() {
                    roots.retain(|idx| *idx != root_index);
                    roots.insert(0, root_index);
                }
            }
        }
        if roots.is_empty() && !joints.is_empty() {
            roots.push(0);
        }

        Some(GlbSkeleton {
            joints,
            skin_joints,
            skin_joint_hashes: (skin_joint_hashes.len() == valid_bones.len())
                .then(|| skin_joint_hashes.to_vec())
                .unwrap_or_default(),
            mesh_parent,
            skeleton_root,
            roots,
        })
    }

    fn build_skeleton_with_external_bones(
        bundle: &AssetBundle,
        bundle_path: &str,
        bone_refs: &[(String, i64)],
        root_bone: Option<(String, i64)>,
        mesh_transform: Option<i64>,
        bind_pose_count: usize,
        skin_joint_hashes: &[u32],
    ) -> Option<GlbSkeleton> {
        let local_only = bone_refs.iter().all(|(path, _)| path == bundle_path)
            && root_bone
                .as_ref()
                .map(|(path, _)| path == bundle_path)
                .unwrap_or(true);
        if local_only {
            let bone_path_ids = bone_refs
                .iter()
                .map(|(_, path_id)| *path_id)
                .collect::<Vec<_>>();
            return Self::build_skeleton(
                bundle,
                &bone_path_ids,
                root_bone.map(|(_, path_id)| path_id),
                mesh_transform,
                bind_pose_count,
                skin_joint_hashes,
            );
        }

        let mut external_bundles = HashMap::<String, AssetBundle>::new();
        for path in bone_refs
            .iter()
            .map(|(path, _)| path)
            .chain(root_bone.as_ref().map(|(path, _)| path))
        {
            if path == bundle_path || external_bundles.contains_key(path) {
                continue;
            }
            let loaded = AssetBundleLoader::load_bundle(Path::new(path)).ok()?;
            external_bundles.insert(path.clone(), loaded);
        }

        let mut transforms_by_bundle = HashMap::<String, HashMap<i64, TransformInfo>>::new();
        transforms_by_bundle.insert(
            bundle_path.to_string(),
            Self::read_all_transform_infos(bundle),
        );
        for (path, loaded) in &external_bundles {
            transforms_by_bundle.insert(path.clone(), Self::read_all_transform_infos(loaded));
        }

        let valid_bones = bone_refs
            .iter()
            .filter(|(path, path_id)| {
                transforms_by_bundle
                    .get(path)
                    .is_some_and(|transforms| transforms.contains_key(path_id))
            })
            .cloned()
            .collect::<Vec<_>>();
        if valid_bones.is_empty() {
            return None;
        }
        if bind_pose_count > 0 && valid_bones.len() != bind_pose_count {
            return None;
        }

        let mut wanted = HashSet::<(String, i64)>::new();
        for (bundle_path, transform_id) in &valid_bones {
            Self::insert_transform_ancestors(
                *transform_id,
                bundle_path,
                &transforms_by_bundle,
                &mut wanted,
            );
        }
        if let Some((root_bundle_path, root_transform_id)) = &root_bone {
            Self::insert_transform_ancestors(
                *root_transform_id,
                root_bundle_path,
                &transforms_by_bundle,
                &mut wanted,
            );
        }
        if let Some(mesh_transform_id) = mesh_transform {
            Self::insert_transform_ancestors(
                mesh_transform_id,
                bundle_path,
                &transforms_by_bundle,
                &mut wanted,
            );
        }

        let mut joint_by_transform = HashMap::<(String, i64), usize>::new();
        let mut joints = Vec::with_capacity(wanted.len());
        let mut ordered = wanted.iter().cloned().collect::<Vec<_>>();
        ordered.sort_by_key(|(path, id)| {
            transforms_by_bundle
                .get(path)
                .map(|transforms| Self::transform_depth(*id, transforms))
                .unwrap_or(usize::MAX)
        });

        for (path, transform_id) in ordered {
            let info = transforms_by_bundle.get(&path)?.get(&transform_id)?;
            let index = joints.len();
            joint_by_transform.insert((path.clone(), transform_id), index);
            joints.push(GlbJoint {
                name: info.name.clone(),
                path: Self::build_transform_path(transform_id, transforms_by_bundle.get(&path)?),
                transform_path_id: transform_id,
                parent: None,
                children: Vec::new(),
                translation: info.translation,
                rotation: info.rotation,
                scale: info.scale,
            });
        }

        for (path, transform_id) in wanted {
            let Some(joint_index) = joint_by_transform
                .get(&(path.clone(), transform_id))
                .copied()
            else {
                continue;
            };
            let Some(info) = transforms_by_bundle
                .get(&path)
                .and_then(|transforms| transforms.get(&transform_id))
            else {
                continue;
            };
            if let Some(parent_index) = joint_by_transform
                .get(&(path.clone(), info.parent_transform))
                .copied()
            {
                joints[joint_index].parent = Some(parent_index);
                joints[parent_index].children.push(joint_index);
            }
        }

        let skin_joints = valid_bones
            .iter()
            .filter_map(|(path, id)| joint_by_transform.get(&(path.clone(), *id)).copied())
            .collect::<Vec<_>>();
        if skin_joints.is_empty() {
            return None;
        }
        let mesh_parent = mesh_transform.and_then(|id| {
            joint_by_transform
                .get(&(bundle_path.to_string(), id))
                .copied()
        });
        let mut roots = joints
            .iter()
            .enumerate()
            .filter_map(|(i, joint)| joint.parent.is_none().then_some(i))
            .collect::<Vec<_>>();
        let skeleton_root = root_bone
            .and_then(|(path, id)| joint_by_transform.get(&(path, id)).copied())
            .or_else(|| skin_joints.first().copied());
        if roots.is_empty() && !joints.is_empty() {
            roots.push(0);
        }

        Some(GlbSkeleton {
            joints,
            skin_joints,
            skin_joint_hashes: (skin_joint_hashes.len() == valid_bones.len())
                .then(|| skin_joint_hashes.to_vec())
                .unwrap_or_default(),
            mesh_parent,
            skeleton_root,
            roots,
        })
    }

    fn insert_transform_ancestors(
        transform_id: i64,
        bundle_path: &str,
        transforms_by_bundle: &HashMap<String, HashMap<i64, TransformInfo>>,
        wanted: &mut HashSet<(String, i64)>,
    ) {
        let Some(transforms) = transforms_by_bundle.get(bundle_path) else {
            return;
        };
        let mut current = transform_id;
        while current != 0 && wanted.insert((bundle_path.to_string(), current)) {
            let Some(info) = transforms.get(&current) else {
                break;
            };
            current = info.parent_transform;
        }
    }

    fn pptr_matches_bundle_path(
        pptr: &PPtr,
        expected_path_id: i64,
        expected_bundle_path: &str,
        source_bundle_path: &str,
        serialized_file_index: usize,
        source_bundle: &AssetBundle,
        db: Option<&AssetDatabase>,
    ) -> bool {
        if pptr.path_id != expected_path_id {
            return false;
        }
        if expected_bundle_path.is_empty() || source_bundle_path.is_empty() {
            return true;
        }
        Self::resolve_pptr_bundle_path(
            pptr,
            source_bundle_path,
            serialized_file_index,
            source_bundle,
            db,
        )
        .as_deref()
            == Some(expected_bundle_path)
    }

    fn resolve_pptr_bundle_path(
        pptr: &PPtr,
        source_bundle_path: &str,
        serialized_file_index: usize,
        source_bundle: &AssetBundle,
        db: Option<&AssetDatabase>,
    ) -> Option<String> {
        if pptr.is_null() {
            return None;
        }
        if pptr.file_id == 0 || source_bundle_path.is_empty() {
            return Some(source_bundle_path.to_string());
        }
        let externals = &source_bundle
            .assets
            .get(serialized_file_index)?
            .inner
            .m_externals;
        let db = db?;
        UnityExternalResolver::resolve_pptr(db, source_bundle_path, externals, pptr)
    }

    fn build_skeleton_from_bone_hashes(
        bundle: &AssetBundle,
        bone_name_hashes: &[u32],
        root_bone_name_hash: Option<u32>,
        mesh_transform: Option<i64>,
        bind_pose_count: usize,
    ) -> Option<GlbSkeleton> {
        if bone_name_hashes.is_empty() {
            return None;
        }
        if bind_pose_count > 0 && bone_name_hashes.len() != bind_pose_count {
            return None;
        }
        let path_index = Self::build_transform_path_hash_index(bundle);
        if path_index.is_empty() {
            return None;
        }

        let mut bone_path_ids = Vec::with_capacity(bone_name_hashes.len());
        for hash in bone_name_hashes {
            let candidate = path_index.get(hash).or_else(|| {
                path_index.iter().find_map(|(_, info)| {
                    let name_hash = Self::unity_crc32(info.name.as_bytes());
                    (name_hash == *hash).then_some(info)
                })
            })?;
            bone_path_ids.push(candidate.transform_path_id);
        }

        let root_bone = root_bone_name_hash
            .and_then(|hash| path_index.get(&hash).map(|info| info.transform_path_id));
        Self::build_skeleton(
            bundle,
            &bone_path_ids,
            root_bone,
            mesh_transform,
            bind_pose_count,
            bone_name_hashes,
        )
    }

    fn build_transform_path_hash_index(bundle: &AssetBundle) -> HashMap<u32, TransformPathInfo> {
        let transforms = Self::read_all_transform_infos(bundle);
        let mut child_ids = HashSet::<i64>::new();
        for info in transforms.values() {
            for child in &info.children {
                child_ids.insert(*child);
            }
        }

        let roots = transforms
            .keys()
            .copied()
            .filter(|id| !child_ids.contains(id))
            .collect::<Vec<_>>();
        let mut result = HashMap::new();
        for root in roots {
            Self::collect_transform_path_hashes(root, String::new(), &transforms, &mut result);
        }
        result
    }

    fn read_all_transform_infos(bundle: &AssetBundle) -> HashMap<i64, TransformInfo> {
        let game_object_names = Self::read_game_object_names(bundle);
        let mut transforms = HashMap::<i64, TransformInfo>::new();
        for sf in &bundle.assets {
            for obj in &sf.objects {
                if obj.class_id != 4 {
                    continue;
                }
                let Ok(transform) = UnityClassParser::parse_transform(&sf.inner, &obj.inner) else {
                    continue;
                };
                let name = game_object_names
                    .get(&transform.game_object.path_id)
                    .cloned()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("bone_{}", obj.path_id));
                transforms.insert(
                    obj.path_id,
                    TransformInfo {
                        name,
                        game_object_path_id: transform.game_object.path_id,
                        parent_transform: transform.father.path_id,
                        children: transform
                            .children
                            .iter()
                            .filter_map(|child| {
                                if child.is_null() {
                                    None
                                } else {
                                    Some(child.path_id)
                                }
                            })
                            .collect(),
                        translation: transform.local_position,
                        rotation: transform.local_rotation,
                        scale: transform.local_scale,
                    },
                );
            }
        }
        transforms
    }

    fn transform_depth(transform_id: i64, transforms: &HashMap<i64, TransformInfo>) -> usize {
        let mut depth = 0usize;
        let mut current = transform_id;
        let mut seen = HashSet::<i64>::new();
        while let Some(info) = transforms.get(&current) {
            if info.parent_transform == 0 || !seen.insert(current) {
                break;
            }
            depth += 1;
            current = info.parent_transform;
        }
        depth
    }

    fn build_transform_path(transform_id: i64, transforms: &HashMap<i64, TransformInfo>) -> String {
        let mut names = Vec::new();
        let mut current = transform_id;
        let mut seen = HashSet::<i64>::new();
        while let Some(info) = transforms.get(&current) {
            if !seen.insert(current) {
                break;
            }
            names.push(info.name.clone());
            if info.parent_transform == 0 {
                break;
            }
            current = info.parent_transform;
        }
        names.reverse();
        names.join("/")
    }

    fn collect_transform_path_hashes(
        transform_path_id: i64,
        parent_path: String,
        transforms: &HashMap<i64, TransformInfo>,
        result: &mut HashMap<u32, TransformPathInfo>,
    ) {
        let Some(info) = transforms.get(&transform_path_id) else {
            return;
        };
        let path = if parent_path.is_empty() {
            info.name.clone()
        } else {
            format!("{}/{}", parent_path, info.name)
        };

        let mut suffix = path.as_str();
        loop {
            result
                .entry(Self::unity_crc32(suffix.as_bytes()))
                .or_insert_with(|| TransformPathInfo {
                    transform_path_id,
                    name: info.name.clone(),
                });
            let Some((_, next)) = suffix.split_once('/') else {
                break;
            };
            suffix = next;
        }

        let children = transforms
            .get(&transform_path_id)
            .map(|info| info.children.clone())
            .unwrap_or_default();
        for child in children {
            Self::collect_transform_path_hashes(child, path.clone(), transforms, result);
        }
    }

    fn unity_crc32(data: &[u8]) -> u32 {
        let mut value = 0xFFFF_FFFFu32;
        for byte in data {
            let mut table_value = ((value as u8) ^ *byte) as u32;
            for _ in 0..8 {
                table_value = if table_value & 1 != 0 {
                    (table_value >> 1) ^ 0xEDB8_8320
                } else {
                    table_value >> 1
                };
            }
            value = table_value ^ (value >> 8);
        }
        value ^ 0xFFFF_FFFF
    }

    fn find_transform_by_game_object(
        bundle: &AssetBundle,
        game_object_path_id: i64,
    ) -> Option<i64> {
        Self::read_all_transform_infos(bundle)
            .into_iter()
            .find_map(|(transform_id, info)| {
                (info.game_object_path_id == game_object_path_id).then_some(transform_id)
            })
    }

    fn read_game_object_names(bundle: &AssetBundle) -> HashMap<i64, String> {
        let mut names = HashMap::new();
        for sf in &bundle.assets {
            for obj in sf.objects.iter().filter(|obj| obj.class_id == 1) {
                let Ok(game_object) = UnityClassParser::parse_game_object(&sf.inner, &obj.inner)
                else {
                    continue;
                };
                names.insert(obj.path_id, game_object.name);
            }
        }
        names
    }
}

#[derive(Debug, Clone)]
struct TransformInfo {
    name: String,
    game_object_path_id: i64,
    parent_transform: i64,
    children: Vec<i64>,
    translation: [f32; 3],
    rotation: [f32; 4],
    scale: [f32; 3],
}

#[derive(Debug, Clone)]
struct TransformPathInfo {
    transform_path_id: i64,
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_bone_hash_selection_allows_full_renderer_skeleton_for_mesh_subset() {
        let bundle_path = "model.bundle".to_string();
        let bone_refs = vec![
            (bundle_path.clone(), 1),
            (bundle_path.clone(), 2),
            (bundle_path.clone(), 3),
        ];
        let transforms_by_bundle = HashMap::from([(
            bundle_path.clone(),
            HashMap::from([
                (1, transform("root", 0, vec![2, 3])),
                (2, transform("body", 1, Vec::new())),
                (3, transform("hair", 1, Vec::new())),
            ]),
        )]);
        let bone_name_hashes = vec![
            ModelContextResolver::unity_crc32("hair".as_bytes()),
            ModelContextResolver::unity_crc32("body".as_bytes()),
        ];

        let selected = ModelContextResolver::select_renderer_bones_by_hashes_from_transforms(
            &bone_refs,
            &bone_name_hashes,
            &transforms_by_bundle,
        )
        .expect("select subset");

        assert_eq!(selected, vec![(bundle_path.clone(), 3), (bundle_path, 2)]);
    }

    #[test]
    fn renderer_bone_hash_selection_reorders_renderer_bones_to_bind_pose_order() {
        let bundle_path = "model.bundle".to_string();
        let bone_refs = vec![(bundle_path.clone(), 2), (bundle_path.clone(), 3)];
        let transforms_by_bundle = HashMap::from([(
            bundle_path.clone(),
            HashMap::from([
                (1, transform("root", 0, vec![2, 3])),
                (2, transform("body", 1, Vec::new())),
                (3, transform("hat", 1, Vec::new())),
            ]),
        )]);
        let bone_name_hashes = vec![
            ModelContextResolver::unity_crc32("hat".as_bytes()),
            ModelContextResolver::unity_crc32("body".as_bytes()),
        ];

        let selected = ModelContextResolver::select_renderer_bones_by_hashes_from_transforms(
            &bone_refs,
            &bone_name_hashes,
            &transforms_by_bundle,
        )
        .expect("select reordered bones");

        assert_eq!(selected, vec![(bundle_path.clone(), 3), (bundle_path, 2)]);
    }

    fn transform(name: &str, parent_transform: i64, children: Vec<i64>) -> TransformInfo {
        TransformInfo {
            name: name.to_string(),
            game_object_path_id: 0,
            parent_transform,
            children,
            translation: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
        }
    }
}
