/*
 * Shared skin remapping helpers for preview and GLB export workflows.
 */

use crate::common::mesh::mesh_types::MeshGeometry;
use crate::exporter::model_context::{GlbJoint, GlbSkeleton, ModelContextResolver};
use std::collections::HashMap;

pub fn bind_static_geometry_to_first_joint(geometry: &mut MeshGeometry) {
    geometry.bone_weights = Vec::with_capacity(geometry.vertex_count * 4);
    geometry.bone_indices = Vec::with_capacity(geometry.vertex_count * 4);
    for _ in 0..geometry.vertex_count {
        geometry
            .bone_weights
            .extend_from_slice(&[1.0, 0.0, 0.0, 0.0]);
        geometry.bone_indices.extend_from_slice(&[0, 0, 0, 0]);
    }
}

pub fn remap_geometry_skin_to_skeleton(
    geometry: &mut MeshGeometry,
    skeleton: &GlbSkeleton,
) -> Result<(), String> {
    if geometry.bone_name_hashes.is_empty() {
        return Err("Mesh has no bone name hashes for skeleton remap".to_string());
    }
    let skeleton_hash_to_skin_index = skeleton_hash_to_skin_index(skeleton);
    if skeleton_hash_to_skin_index.is_empty() {
        return Err("Resolved skeleton has no hashable skin joints".to_string());
    }

    let mut local_to_global = Vec::<u16>::with_capacity(geometry.bone_name_hashes.len());
    for hash in &geometry.bone_name_hashes {
        let Some(global_index) = skeleton_hash_to_skin_index.get(hash).copied() else {
            return Err(format!(
                "bone hash {} is not present in resolved skeleton",
                hash
            ));
        };
        local_to_global.push(global_index as u16);
    }

    for joint in geometry
        .bone_indices
        .iter_mut()
        .take(geometry.vertex_count * 4)
    {
        let Some(global_index) = local_to_global.get(*joint as usize).copied() else {
            *joint = 0;
            continue;
        };
        *joint = global_index;
    }
    align_geometry_skin_metadata_to_skeleton(geometry, skeleton);
    Ok(())
}

pub fn align_geometry_skin_metadata_to_skeleton(
    geometry: &mut MeshGeometry,
    skeleton: &GlbSkeleton,
) {
    geometry.bind_poses = identity_bind_poses(skeleton.skin_joints.len());
    geometry.bone_name_hashes = skeleton_skin_hashes(skeleton);
    geometry.root_bone_name_hash = None;
}

pub fn identity_bind_poses(count: usize) -> Vec<f32> {
    let mut values = Vec::with_capacity(count * 16);
    for _ in 0..count {
        values.extend_from_slice(&[
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]);
    }
    values
}

pub fn merge_skeleton_into(target: &mut GlbSkeleton, source: &GlbSkeleton) {
    let mut joint_map = HashMap::<String, usize>::new();
    for (index, joint) in target.joints.iter().enumerate() {
        joint_map.insert(skeleton_joint_key(joint), index);
    }

    let mut source_to_target = Vec::<usize>::with_capacity(source.joints.len());
    for joint in &source.joints {
        let key = skeleton_joint_key(joint);
        if let Some(index) = joint_map.get(&key).copied() {
            source_to_target.push(index);
            continue;
        }
        let new_index = target.joints.len();
        let mut cloned = joint.clone();
        cloned.parent = None;
        cloned.children.clear();
        target.joints.push(cloned);
        joint_map.insert(key, new_index);
        source_to_target.push(new_index);
    }

    for (source_index, source_joint) in source.joints.iter().enumerate() {
        let target_index = source_to_target[source_index];
        if let Some(source_parent) = source_joint.parent {
            if let Some(target_parent) = source_to_target.get(source_parent).copied() {
                target.joints[target_index].parent = Some(target_parent);
                if !target.joints[target_parent]
                    .children
                    .contains(&target_index)
                {
                    target.joints[target_parent].children.push(target_index);
                }
            }
        }
        for source_child in &source_joint.children {
            if let Some(target_child) = source_to_target.get(*source_child).copied() {
                if !target.joints[target_index].children.contains(&target_child) {
                    target.joints[target_index].children.push(target_child);
                }
            }
        }
    }

    for source_skin_joint in &source.skin_joints {
        if let Some(target_skin_joint) = source_to_target.get(*source_skin_joint).copied() {
            if !target.skin_joints.contains(&target_skin_joint) {
                target.skin_joints.push(target_skin_joint);
            }
        }
    }
    for source_root in &source.roots {
        if let Some(target_root) = source_to_target.get(*source_root).copied() {
            if !target.roots.contains(&target_root) {
                target.roots.push(target_root);
            }
        }
    }
    if target.skeleton_root.is_none() {
        target.skeleton_root = source
            .skeleton_root
            .and_then(|root| source_to_target.get(root).copied());
    }
}

fn skeleton_hash_to_skin_index(skeleton: &GlbSkeleton) -> HashMap<u32, usize> {
    let joint_hashes = ModelContextResolver::transform_path_by_unity_hash(skeleton);
    let mut result = HashMap::new();
    for (skin_index, joint_index) in skeleton.skin_joints.iter().copied().enumerate() {
        for (hash, hashed_joint_index) in &joint_hashes {
            if *hashed_joint_index == joint_index {
                result.entry(*hash).or_insert(skin_index);
            }
        }
    }
    result
}

fn skeleton_skin_hashes(skeleton: &GlbSkeleton) -> Vec<u32> {
    let joint_hashes = ModelContextResolver::transform_path_by_unity_hash(skeleton);
    skeleton
        .skin_joints
        .iter()
        .map(|joint_index| {
            joint_hashes
                .iter()
                .find_map(|(hash, hashed_joint_index)| {
                    (*hashed_joint_index == *joint_index).then_some(*hash)
                })
                .unwrap_or(0)
        })
        .collect()
}

fn skeleton_joint_key(joint: &GlbJoint) -> String {
    if joint.transform_path_id != 0 {
        format!("id:{}", joint.transform_path_id)
    } else {
        format!("path:{}", joint.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exporter::model_context::{GlbJoint, GlbSkeleton};

    #[test]
    fn remap_geometry_skin_to_skeleton_maps_local_bone_indices_to_shared_skin() {
        let skeleton = GlbSkeleton {
            joints: vec![
                joint("root", "root", 1),
                joint("body", "root/body", 2),
                joint("hair", "root/hair", 3),
            ],
            skin_joints: vec![1, 2],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        let mut geometry = MeshGeometry {
            vertices: vec![0.0, 0.0, 0.0],
            normals: Vec::new(),
            uvs: Vec::new(),
            tangents: Vec::new(),
            colors: Vec::new(),
            bone_weights: vec![1.0, 0.0, 0.0, 0.0],
            bone_indices: vec![0, 0, 0, 0],
            bind_poses: identity_bind_poses(1),
            bone_name_hashes: vec![crc("hair")],
            root_bone_name_hash: None,
            sub_meshes: Vec::new(),
            parts: Vec::new(),
            blend_shapes: Vec::new(),
            indices: vec![0],
            success: true,
            error: String::new(),
            vertex_count: 1,
            triangle_count: 0,
        };

        remap_geometry_skin_to_skeleton(&mut geometry, &skeleton).expect("remap");

        assert_eq!(geometry.bone_indices, vec![1, 1, 1, 1]);
        assert_eq!(geometry.bind_poses.len(), skeleton.skin_joints.len() * 16);
        assert_eq!(geometry.bone_name_hashes.len(), skeleton.skin_joints.len());
    }

    #[test]
    fn merge_skeleton_into_preserves_new_skin_joints_for_later_submeshes() {
        let mut target = GlbSkeleton {
            joints: vec![joint("root", "root", 1), joint("body", "root/body", 2)],
            skin_joints: vec![1],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        target.joints[1].parent = Some(0);
        target.joints[0].children = vec![1];
        let mut source = GlbSkeleton {
            joints: vec![joint("root", "root", 1), joint("hair", "root/hair", 3)],
            skin_joints: vec![1],
            skin_joint_hashes: Vec::new(),
            mesh_parent: None,
            skeleton_root: Some(0),
            roots: vec![0],
        };
        source.joints[1].parent = Some(0);
        source.joints[0].children = vec![1];

        merge_skeleton_into(&mut target, &source);

        assert_eq!(target.joints.len(), 3);
        assert_eq!(target.skin_joints, vec![1, 2]);
        assert_eq!(target.joints[2].name, "hair");
        assert_eq!(target.joints[2].parent, Some(0));
    }

    fn joint(name: &str, path: &str, transform_path_id: i64) -> GlbJoint {
        GlbJoint {
            name: name.to_string(),
            path: path.to_string(),
            transform_path_id,
            parent: None,
            children: Vec::new(),
            translation: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
        }
    }

    fn crc(value: &str) -> u32 {
        let mut result = 0xFFFF_FFFFu32;
        for byte in value.as_bytes() {
            let mut table_value = ((result as u8) ^ *byte) as u32;
            for _ in 0..8 {
                table_value = if table_value & 1 != 0 {
                    (table_value >> 1) ^ 0xEDB8_8320
                } else {
                    table_value >> 1
                };
            }
            result = table_value ^ (result >> 8);
        }
        result ^ 0xFFFF_FFFF
    }
}
