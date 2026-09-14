use std::collections::{HashMap, HashSet};

use crate::exporter::animation_clip_raw_parser::AnimationClipRawParser;
use crate::exporter::model_context::GlbSkeleton;
use crate::unity::type_tree::unity_value::UnityValue;

#[derive(Debug, Clone)]
pub struct GlbAnimation {
    pub name: String,
    pub channels: Vec<GlbAnimationChannel>,
}

#[derive(Debug, Clone)]
pub struct GlbAnimationChannel {
    pub node: usize,
    pub path: AnimationPath,
    pub times: Vec<f32>,
    pub values: Vec<f32>,
}

#[derive(Debug, Clone, Copy)]
pub enum AnimationPath {
    Translation,
    Rotation,
    Scale,
    Weights,
}

pub struct AnimationClipExporter;

impl AnimationClipExporter {
    pub fn binding_path_hashes_from_clip_object(
        sf: &crate::common::bundle_file::asset_bundle::SerializedFile,
        obj: &crate::common::bundle_file::asset_bundle::ObjectInfo,
    ) -> HashSet<u32> {
        if let Ok(value) = AnimationClipRawParser::parse(sf, obj) {
            let hashes = Self::binding_path_hashes_from_value(&value);
            if !hashes.is_empty() {
                return hashes;
            }
        }
        let handle = crate::common::bundle_file::asset_bundle::ObjectHandle::new(sf, obj);
        if let Ok(value) = handle.read() {
            return Self::binding_path_hashes_from_value(&value);
        }
        HashSet::new()
    }

    pub fn has_playable_transform_curves_for_hashes(
        sf: &crate::common::bundle_file::asset_bundle::SerializedFile,
        obj: &crate::common::bundle_file::asset_bundle::ObjectInfo,
        target_hashes: &HashSet<u32>,
    ) -> bool {
        if target_hashes.is_empty() {
            return false;
        }
        let hash_to_joint = target_hashes
            .iter()
            .enumerate()
            .map(|(index, hash)| (*hash, index))
            .collect::<HashMap<_, _>>();
        let empty_names = HashMap::new();
        let morph_target_names = Vec::new();
        if let Ok(value) = AnimationClipRawParser::parse(sf, obj) {
            if Self::extract_from_value(&value, &empty_names, &hash_to_joint, &morph_target_names)
                .is_some()
            {
                return true;
            }
        }
        let handle = crate::common::bundle_file::asset_bundle::ObjectHandle::new(sf, obj);
        let Ok(value) = handle.read() else {
            return false;
        };
        Self::extract_from_value(&value, &empty_names, &hash_to_joint, &morph_target_names)
            .is_some()
    }

    pub fn binding_path_hashes_from_value(value: &UnityValue) -> HashSet<u32> {
        let UnityValue::Object(clip_map) = value else {
            return HashSet::new();
        };
        let mut result = HashSet::new();
        if let Some(binding_constant) = Self::object_map(clip_map.get("m_ClipBindingConstant")) {
            for binding in Self::generic_bindings(binding_constant.get("genericBindings")) {
                if binding.type_id == 4 && binding.path != 0 {
                    result.insert(binding.path);
                }
            }
        }
        if result.is_empty() {
            if let Some(muscle_clip) = Self::object_map(clip_map.get("m_MuscleClip")) {
                if let Some(runtime_clip) = Self::object_map(muscle_clip.get("m_Clip")) {
                    if let Some(bindings) = Self::clip_bindings(clip_map, runtime_clip) {
                        for binding in bindings {
                            if binding.type_id == 4 && binding.path != 0 {
                                result.insert(binding.path);
                            }
                        }
                    }
                }
            }
        }
        result
    }

    pub fn extract_from_bundle(
        bundle: &crate::common::bundle_file::asset_bundle::AssetBundle,
        skeleton: &GlbSkeleton,
        morph_target_names: &[String],
    ) -> Vec<GlbAnimation> {
        Self::extract_from_bundle_matching(bundle, skeleton, morph_target_names, None, |_| true)
    }

    pub fn extract_from_bundle_path_ids(
        bundle: &crate::common::bundle_file::asset_bundle::AssetBundle,
        path_ids: &HashSet<i64>,
        skeleton: &GlbSkeleton,
        morph_target_names: &[String],
    ) -> Vec<GlbAnimation> {
        Self::extract_from_bundle_path_ids_with_hash_aliases(
            bundle,
            path_ids,
            skeleton,
            morph_target_names,
            None,
        )
    }

    pub fn extract_from_bundle_path_ids_with_hash_aliases(
        bundle: &crate::common::bundle_file::asset_bundle::AssetBundle,
        path_ids: &HashSet<i64>,
        skeleton: &GlbSkeleton,
        morph_target_names: &[String],
        hash_aliases: Option<&HashMap<u32, usize>>,
    ) -> Vec<GlbAnimation> {
        if path_ids.is_empty() {
            return Vec::new();
        }
        Self::extract_from_bundle_matching(
            bundle,
            skeleton,
            morph_target_names,
            hash_aliases,
            |path_id| path_ids.contains(&path_id),
        )
    }

    fn extract_from_bundle_matching(
        bundle: &crate::common::bundle_file::asset_bundle::AssetBundle,
        skeleton: &GlbSkeleton,
        morph_target_names: &[String],
        hash_aliases: Option<&HashMap<u32, usize>>,
        mut matches_path_id: impl FnMut(i64) -> bool,
    ) -> Vec<GlbAnimation> {
        let name_to_joint =
            crate::exporter::model_context::ModelContextResolver::transform_path_by_name(skeleton);
        let mut hash_to_joint =
            crate::exporter::model_context::ModelContextResolver::transform_path_by_unity_hash(
                skeleton,
            );
        if let Some(hash_aliases) = hash_aliases {
            for (hash, joint_index) in hash_aliases {
                hash_to_joint.entry(*hash).or_insert(*joint_index);
            }
        }
        let mut animations = Vec::new();
        for sf in &bundle.assets {
            for obj in &sf.objects {
                if obj.class_id != 74 {
                    continue;
                }
                if !matches_path_id(obj.path_id) {
                    continue;
                }
                let handle = crate::common::bundle_file::asset_bundle::ObjectHandle::new(sf, obj);
                if let Ok(value) = AnimationClipRawParser::parse(sf, obj) {
                    if let Some(animation) = Self::extract_from_value(
                        &value,
                        &name_to_joint,
                        &hash_to_joint,
                        morph_target_names,
                    ) {
                        animations.push(animation);
                        continue;
                    }
                }
                let Ok(value) = handle.read() else {
                    continue;
                };
                if let Some(animation) = Self::extract_from_value(
                    &value,
                    &name_to_joint,
                    &hash_to_joint,
                    morph_target_names,
                ) {
                    animations.push(animation);
                }
            }
        }
        animations
    }

    fn extract_from_value(
        value: &UnityValue,
        name_to_joint: &HashMap<String, usize>,
        hash_to_joint: &HashMap<u32, usize>,
        morph_target_names: &[String],
    ) -> Option<GlbAnimation> {
        let UnityValue::Object(map) = value else {
            return None;
        };
        let name = map
            .get("m_Name")
            .and_then(|v| v.as_str())
            .unwrap_or("AnimationClip")
            .to_string();
        let mut channels = Vec::new();
        Self::collect_vector3_curves(
            map.get("m_PositionCurves"),
            AnimationPath::Translation,
            name_to_joint,
            &mut channels,
        );
        Self::collect_vector3_curves(
            map.get("m_ScaleCurves"),
            AnimationPath::Scale,
            name_to_joint,
            &mut channels,
        );
        Self::collect_quat_curves(map.get("m_RotationCurves"), name_to_joint, &mut channels);
        Self::collect_blend_shape_curves(
            map.get("m_FloatCurves"),
            morph_target_names,
            &mut channels,
        );
        Self::collect_muscle_clip_curves(map, hash_to_joint, morph_target_names, &mut channels);
        if channels.is_empty() {
            None
        } else {
            Some(GlbAnimation { name, channels })
        }
    }

    fn collect_muscle_clip_curves(
        clip_map: &HashMap<String, UnityValue>,
        hash_to_joint: &HashMap<u32, usize>,
        morph_target_names: &[String],
        out: &mut Vec<GlbAnimationChannel>,
    ) {
        let Some(muscle_clip) = Self::object_map(clip_map.get("m_MuscleClip")) else {
            return;
        };
        let Some(runtime_clip) = Self::object_map(muscle_clip.get("m_Clip")) else {
            return;
        };
        let Some(bindings) = Self::clip_bindings(clip_map, runtime_clip) else {
            return;
        };
        let stop_time = Self::number(muscle_clip.get("m_StopTime")).unwrap_or(0.0);
        let mut channel_builder = ModernChannelBuilder::new(morph_target_names);
        Self::collect_streamed_clip_curves(
            runtime_clip,
            &bindings,
            hash_to_joint,
            &mut channel_builder,
        );
        Self::collect_dense_clip_curves(
            runtime_clip,
            &bindings,
            hash_to_joint,
            &mut channel_builder,
        );
        Self::collect_constant_clip_curves(
            runtime_clip,
            &bindings,
            hash_to_joint,
            stop_time,
            &mut channel_builder,
        );
        out.extend(channel_builder.finish());
    }

    fn collect_streamed_clip_curves(
        runtime_clip: &HashMap<String, UnityValue>,
        bindings: &[GenericBinding],
        hash_to_joint: &HashMap<u32, usize>,
        channel_builder: &mut ModernChannelBuilder,
    ) {
        let Some(streamed_clip) = Self::object_map(runtime_clip.get("m_StreamedClip")) else {
            return;
        };
        let Some(words) = Self::u32_array(streamed_clip.get("data")) else {
            return;
        };
        let bytes = words
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect::<Vec<_>>();
        let mut offset = 0usize;
        while offset + 8 <= bytes.len() {
            let Some(time) = Self::read_f32(&bytes, &mut offset) else {
                break;
            };
            let Some(key_count) = Self::read_i32(&bytes, &mut offset) else {
                break;
            };
            if key_count < 0 {
                break;
            }
            let mut values_by_index = HashMap::<usize, f32>::new();
            for _ in 0..key_count {
                let Some(index) = Self::read_i32(&bytes, &mut offset) else {
                    return;
                };
                let coeff_start = offset;
                offset = offset.saturating_add(16);
                if coeff_start + 16 > bytes.len() {
                    return;
                }
                let value = f32::from_le_bytes(
                    bytes[coeff_start + 12..coeff_start + 16]
                        .try_into()
                        .ok()
                        .unwrap_or([0; 4]),
                );
                if index >= 0 {
                    values_by_index.insert(index as usize, value);
                }
            }
            let mut curve_index = 0usize;
            while curve_index < values_by_index.len().saturating_add(1_000) {
                let data_index = match values_by_index
                    .keys()
                    .copied()
                    .filter(|index| *index >= curve_index)
                    .min()
                {
                    Some(index) => index,
                    None => break,
                };
                let Some((binding_start, binding)) =
                    Self::find_binding_with_start(bindings, data_index)
                else {
                    curve_index = data_index + 1;
                    continue;
                };
                let stride = Self::binding_stride(binding);
                let mut values = Vec::with_capacity(stride);
                for component_index in 0..stride {
                    if let Some(value) = values_by_index.get(&(binding_start + component_index)) {
                        values.push(*value);
                    }
                }
                if values.len() == stride {
                    channel_builder.push_binding_frame(binding, time, &values, hash_to_joint);
                }
                curve_index = binding_start + stride;
            }
        }
    }

    fn collect_dense_clip_curves(
        runtime_clip: &HashMap<String, UnityValue>,
        bindings: &[GenericBinding],
        hash_to_joint: &HashMap<u32, usize>,
        channel_builder: &mut ModernChannelBuilder,
    ) {
        let Some(dense_clip) = Self::object_map(runtime_clip.get("m_DenseClip")) else {
            return;
        };
        let frame_count = Self::number(dense_clip.get("m_FrameCount")).unwrap_or(0.0) as usize;
        let curve_count = Self::number(dense_clip.get("m_CurveCount")).unwrap_or(0.0) as usize;
        let sample_rate = Self::number(dense_clip.get("m_SampleRate")).unwrap_or(0.0);
        let begin_time = Self::number(dense_clip.get("m_BeginTime")).unwrap_or(0.0);
        let Some(samples) = Self::number_array(dense_clip.get("m_SampleArray")) else {
            return;
        };
        let stream_count = Self::object_map(runtime_clip.get("m_StreamedClip"))
            .and_then(|streamed_clip| Self::number(streamed_clip.get("curveCount")))
            .unwrap_or(0.0) as usize;
        if frame_count == 0 || curve_count == 0 || sample_rate <= 0.0 {
            return;
        }
        for frame_index in 0..frame_count {
            let time = begin_time + frame_index as f32 / sample_rate;
            let frame_offset = frame_index * curve_count;
            if frame_offset >= samples.len() {
                break;
            }
            let mut curve_index = 0usize;
            while curve_index < curve_count {
                let binding_index = stream_count + curve_index;
                let Some(binding) = Self::find_binding(bindings, binding_index) else {
                    curve_index += 1;
                    continue;
                };
                let stride = Self::binding_stride(binding);
                let sample_start = frame_offset + curve_index;
                let sample_end = sample_start + stride;
                if sample_end <= samples.len() {
                    channel_builder.push_binding_frame(
                        binding,
                        time,
                        &samples[sample_start..sample_end],
                        hash_to_joint,
                    );
                }
                curve_index += stride.max(1);
            }
        }
    }

    fn collect_constant_clip_curves(
        runtime_clip: &HashMap<String, UnityValue>,
        bindings: &[GenericBinding],
        hash_to_joint: &HashMap<u32, usize>,
        stop_time: f32,
        channel_builder: &mut ModernChannelBuilder,
    ) {
        let Some(constant_clip) = Self::object_map(runtime_clip.get("m_ConstantClip")) else {
            return;
        };
        let Some(samples) = Self::number_array(constant_clip.get("data")) else {
            return;
        };
        if samples.is_empty() {
            return;
        }
        let stream_count = Self::object_map(runtime_clip.get("m_StreamedClip"))
            .and_then(|streamed_clip| Self::number(streamed_clip.get("curveCount")))
            .unwrap_or(0.0) as usize;
        let dense_count = Self::object_map(runtime_clip.get("m_DenseClip"))
            .and_then(|dense_clip| Self::number(dense_clip.get("m_CurveCount")))
            .unwrap_or(0.0) as usize;
        for time in [0.0, stop_time.max(0.0)] {
            let mut curve_index = 0usize;
            while curve_index < samples.len() {
                let binding_index = stream_count + dense_count + curve_index;
                let Some(binding) = Self::find_binding(bindings, binding_index) else {
                    curve_index += 1;
                    continue;
                };
                let stride = Self::binding_stride(binding);
                let sample_end = curve_index + stride;
                if sample_end <= samples.len() {
                    channel_builder.push_binding_frame(
                        binding,
                        time,
                        &samples[curve_index..sample_end],
                        hash_to_joint,
                    );
                }
                curve_index += stride.max(1);
            }
        }
    }

    fn clip_bindings(
        clip_map: &HashMap<String, UnityValue>,
        runtime_clip: &HashMap<String, UnityValue>,
    ) -> Option<Vec<GenericBinding>> {
        if let Some(binding_constant) = Self::object_map(clip_map.get("m_ClipBindingConstant")) {
            let bindings = Self::generic_bindings(binding_constant.get("genericBindings"));
            if !bindings.is_empty() {
                return Some(bindings);
            }
        }
        let binding = Self::object_map(runtime_clip.get("m_Binding"))?;
        let values = Self::array_items(binding.get("m_ValueArray"))?;
        let mut result = Vec::new();
        let mut index = 0usize;
        while index < values.len() {
            let UnityValue::Object(value_map) = &values[index] else {
                index += 1;
                continue;
            };
            let curve_id = Self::u32_number(value_map.get("m_ID")).unwrap_or(0);
            let curve_type_id = Self::u32_number(value_map.get("m_TypeID")).unwrap_or(0);
            match curve_type_id {
                4_174_552_735 => {
                    result.push(GenericBinding::transform(curve_id, 1));
                    index += 3;
                }
                2_211_994_246 => {
                    result.push(GenericBinding::transform(curve_id, 2));
                    index += 4;
                }
                1_512_518_241 => {
                    result.push(GenericBinding::transform(curve_id, 3));
                    index += 3;
                }
                _ => {
                    result.push(GenericBinding {
                        path: 0,
                        attribute: curve_id,
                        type_id: 95,
                    });
                    index += 1;
                }
            }
        }
        (!result.is_empty()).then_some(result)
    }

    fn generic_bindings(value: Option<&UnityValue>) -> Vec<GenericBinding> {
        let Some(items) = Self::array_items(value) else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|item| {
                let UnityValue::Object(map) = item else {
                    return None;
                };
                Some(GenericBinding {
                    path: Self::u32_number(map.get("path"))?,
                    attribute: Self::u32_number(map.get("attribute"))?,
                    type_id: Self::i32_number(map.get("typeID").or_else(|| map.get("typeId")))?,
                })
            })
            .collect()
    }

    fn find_binding(bindings: &[GenericBinding], index: usize) -> Option<&GenericBinding> {
        Self::find_binding_with_start(bindings, index).map(|(_, binding)| binding)
    }

    fn find_binding_with_start(
        bindings: &[GenericBinding],
        index: usize,
    ) -> Option<(usize, &GenericBinding)> {
        let mut curves = 0usize;
        for binding in bindings {
            let start = curves;
            curves += Self::binding_stride(binding);
            if curves > index {
                return Some((start, binding));
            }
        }
        None
    }

    fn binding_stride(binding: &GenericBinding) -> usize {
        if binding.type_id == 4 {
            match binding.attribute {
                1 | 3 | 4 => 3,
                2 => 4,
                _ => 1,
            }
        } else {
            1
        }
    }

    fn collect_vector3_curves(
        value: Option<&UnityValue>,
        path: AnimationPath,
        name_to_joint: &HashMap<String, usize>,
        out: &mut Vec<GlbAnimationChannel>,
    ) {
        let Some(curves) = Self::array_items(value) else {
            return;
        };
        for curve in curves {
            let UnityValue::Object(curve_map) = curve else {
                continue;
            };
            let Some(node) = Self::curve_node_index(curve_map, name_to_joint) else {
                continue;
            };
            let Some(keys) =
                Self::array_items(curve_map.get("curve").or_else(|| curve_map.get("m_Curve")))
            else {
                continue;
            };
            let mut times = Vec::new();
            let mut values = Vec::new();
            for key in keys {
                let UnityValue::Object(key_map) = key else {
                    continue;
                };
                let Some(time) = Self::number(key_map.get("time")) else {
                    continue;
                };
                let Some(vec) = Self::vec3(key_map.get("value")) else {
                    continue;
                };
                times.push(time);
                let value = match path {
                    AnimationPath::Translation => [-vec[0], vec[1], vec[2]],
                    AnimationPath::Scale => vec,
                    AnimationPath::Rotation | AnimationPath::Weights => vec,
                };
                values.extend_from_slice(&value);
            }
            if !times.is_empty() {
                out.push(GlbAnimationChannel {
                    node,
                    path,
                    times,
                    values,
                });
            }
        }
    }

    fn collect_quat_curves(
        value: Option<&UnityValue>,
        name_to_joint: &HashMap<String, usize>,
        out: &mut Vec<GlbAnimationChannel>,
    ) {
        let Some(curves) = Self::array_items(value) else {
            return;
        };
        for curve in curves {
            let UnityValue::Object(curve_map) = curve else {
                continue;
            };
            let Some(node) = Self::curve_node_index(curve_map, name_to_joint) else {
                continue;
            };
            let Some(keys) =
                Self::array_items(curve_map.get("curve").or_else(|| curve_map.get("m_Curve")))
            else {
                continue;
            };
            let mut times = Vec::new();
            let mut values = Vec::new();
            for key in keys {
                let UnityValue::Object(key_map) = key else {
                    continue;
                };
                let Some(time) = Self::number(key_map.get("time")) else {
                    continue;
                };
                let Some(q) = Self::quat(key_map.get("value")) else {
                    continue;
                };
                times.push(time);
                values.extend_from_slice(&[q[0], -q[1], -q[2], q[3]]);
            }
            if !times.is_empty() {
                out.push(GlbAnimationChannel {
                    node,
                    path: AnimationPath::Rotation,
                    times,
                    values,
                });
            }
        }
    }

    fn collect_blend_shape_curves(
        value: Option<&UnityValue>,
        morph_target_names: &[String],
        out: &mut Vec<GlbAnimationChannel>,
    ) {
        if morph_target_names.is_empty() {
            return;
        }
        let Some(curves) = Self::array_items(value) else {
            return;
        };
        let mut by_time: HashMap<u32, Vec<f32>> = HashMap::new();
        for curve in curves {
            let UnityValue::Object(curve_map) = curve else {
                continue;
            };
            let Some(attribute) = curve_map
                .get("attribute")
                .or_else(|| curve_map.get("m_Attribute"))
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            let Some(shape_index) = Self::blend_shape_index(attribute, morph_target_names) else {
                continue;
            };
            let Some(keys) =
                Self::array_items(curve_map.get("curve").or_else(|| curve_map.get("m_Curve")))
            else {
                continue;
            };
            for key in keys {
                let UnityValue::Object(key_map) = key else {
                    continue;
                };
                let Some(time) = Self::number(key_map.get("time")) else {
                    continue;
                };
                let Some(value) = Self::number(key_map.get("value")) else {
                    continue;
                };
                let slot = by_time
                    .entry(time.to_bits())
                    .or_insert_with(|| vec![0.0; morph_target_names.len()]);
                slot[shape_index] = value / 100.0;
            }
        }
        if by_time.is_empty() {
            return;
        }
        let mut pairs: Vec<(f32, Vec<f32>)> = by_time
            .into_iter()
            .map(|(time_bits, values)| (f32::from_bits(time_bits), values))
            .collect();
        pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut times = Vec::with_capacity(pairs.len());
        let mut values = Vec::with_capacity(pairs.len() * morph_target_names.len());
        for (time, frame_values) in pairs {
            times.push(time);
            values.extend_from_slice(&frame_values);
        }
        out.push(GlbAnimationChannel {
            node: 0,
            path: AnimationPath::Weights,
            times,
            values,
        });
    }

    fn blend_shape_index(attribute: &str, morph_target_names: &[String]) -> Option<usize> {
        let normalized = attribute
            .strip_prefix("blendShape.")
            .unwrap_or(attribute)
            .trim();
        let normalized_lower = normalized.to_lowercase();
        morph_target_names
            .iter()
            .position(|name| name == normalized || name.to_lowercase() == normalized_lower)
    }

    fn curve_node_index(
        curve_map: &HashMap<String, UnityValue>,
        name_to_joint: &HashMap<String, usize>,
    ) -> Option<usize> {
        let path = curve_map
            .get("path")
            .or_else(|| curve_map.get("m_Path"))
            .and_then(|v| v.as_str())?;
        let last = path.rsplit('/').next().unwrap_or(path);
        name_to_joint.get(last).copied()
    }

    fn array_items(value: Option<&UnityValue>) -> Option<&[UnityValue]> {
        match value? {
            UnityValue::Array(items) => Some(items.as_slice()),
            UnityValue::Object(map) => match map.get("Array") {
                Some(UnityValue::Array(items)) => Some(items.as_slice()),
                _ => None,
            },
            _ => None,
        }
    }

    fn object_map(value: Option<&UnityValue>) -> Option<&HashMap<String, UnityValue>> {
        match value? {
            UnityValue::Object(map) => Some(map),
            _ => None,
        }
    }

    fn number_array(value: Option<&UnityValue>) -> Option<Vec<f32>> {
        match value? {
            UnityValue::Array(items) => Some(
                items
                    .iter()
                    .filter_map(|item| Self::number(Some(item)))
                    .collect(),
            ),
            UnityValue::Object(_) => Self::array_items(value).map(|items| {
                items
                    .iter()
                    .filter_map(|item| Self::number(Some(item)))
                    .collect()
            }),
            _ => None,
        }
    }

    fn u32_array(value: Option<&UnityValue>) -> Option<Vec<u32>> {
        match value? {
            UnityValue::Array(items) => Some(
                items
                    .iter()
                    .filter_map(|item| Self::u32_number(Some(item)))
                    .collect(),
            ),
            UnityValue::Object(_) => Self::array_items(value).map(|items| {
                items
                    .iter()
                    .filter_map(|item| Self::u32_number(Some(item)))
                    .collect()
            }),
            _ => None,
        }
    }

    fn read_i32(bytes: &[u8], offset: &mut usize) -> Option<i32> {
        let end = *offset + 4;
        let value = i32::from_le_bytes(bytes.get(*offset..end)?.try_into().ok()?);
        *offset = end;
        Some(value)
    }

    fn read_f32(bytes: &[u8], offset: &mut usize) -> Option<f32> {
        let end = *offset + 4;
        let value = f32::from_le_bytes(bytes.get(*offset..end)?.try_into().ok()?);
        *offset = end;
        Some(value)
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

    fn vec3(value: Option<&UnityValue>) -> Option<[f32; 3]> {
        let map = match value? {
            UnityValue::Object(map) => map,
            _ => return None,
        };
        Some([
            Self::number(map.get("x")).unwrap_or(0.0),
            Self::number(map.get("y")).unwrap_or(0.0),
            Self::number(map.get("z")).unwrap_or(0.0),
        ])
    }

    fn quat(value: Option<&UnityValue>) -> Option<[f32; 4]> {
        let map = match value? {
            UnityValue::Object(map) => map,
            _ => return None,
        };
        Some([
            Self::number(map.get("x")).unwrap_or(0.0),
            Self::number(map.get("y")).unwrap_or(0.0),
            Self::number(map.get("z")).unwrap_or(0.0),
            Self::number(map.get("w")).unwrap_or(1.0),
        ])
    }

    fn number(value: Option<&UnityValue>) -> Option<f32> {
        match value? {
            UnityValue::Float(v) => Some(*v as f32),
            UnityValue::Integer(v) => Some(*v as f32),
            _ => None,
        }
    }

    fn u32_number(value: Option<&UnityValue>) -> Option<u32> {
        match value? {
            UnityValue::Integer(v) => Some(*v as u32),
            UnityValue::Float(v) => Some(*v as u32),
            _ => None,
        }
    }

    fn i32_number(value: Option<&UnityValue>) -> Option<i32> {
        match value? {
            UnityValue::Integer(v) => Some(*v as i32),
            UnityValue::Float(v) => Some(*v as i32),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct GenericBinding {
    path: u32,
    attribute: u32,
    type_id: i32,
}

impl GenericBinding {
    fn transform(path: u32, attribute: u32) -> Self {
        Self {
            path,
            attribute,
            type_id: 4,
        }
    }
}

struct ModernChannelBuilder<'a> {
    morph_target_names: &'a [String],
    transform_channels: HashMap<(usize, u32), Vec<(f32, Vec<f32>)>>,
    weight_frames: HashMap<u32, Vec<f32>>,
}

impl<'a> ModernChannelBuilder<'a> {
    fn new(morph_target_names: &'a [String]) -> Self {
        Self {
            morph_target_names,
            transform_channels: HashMap::new(),
            weight_frames: HashMap::new(),
        }
    }

    fn push_binding_frame(
        &mut self,
        binding: &GenericBinding,
        time: f32,
        values: &[f32],
        hash_to_joint: &HashMap<u32, usize>,
    ) {
        if binding.type_id == 4 {
            let Some(node) = hash_to_joint.get(&binding.path).copied() else {
                return;
            };
            let converted = match binding.attribute {
                1 if values.len() >= 3 => vec![-values[0], values[1], values[2]],
                2 if values.len() >= 4 => vec![values[0], -values[1], -values[2], values[3]],
                3 if values.len() >= 3 => vec![values[0], values[1], values[2]],
                4 if values.len() >= 3 => Self::unity_euler_to_gltf_quaternion(values),
                _ => return,
            };
            self.transform_channels
                .entry((node, binding.attribute))
                .or_default()
                .push((time, converted));
        } else if binding.type_id == 137 && !self.morph_target_names.is_empty() {
            let Some(shape_index) = self.blend_shape_index(binding.attribute) else {
                return;
            };
            let slot = self
                .weight_frames
                .entry(time.to_bits())
                .or_insert_with(|| vec![0.0; self.morph_target_names.len()]);
            slot[shape_index] = values.first().copied().unwrap_or(0.0) / 100.0;
        }
    }

    fn finish(self) -> Vec<GlbAnimationChannel> {
        let mut channels = Vec::new();
        for ((node, attribute), mut frames) in self.transform_channels {
            frames.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let mut times = Vec::with_capacity(frames.len());
            let mut values = Vec::new();
            for (time, frame_values) in frames {
                times.push(time);
                values.extend_from_slice(&frame_values);
            }
            let path = match attribute {
                1 => AnimationPath::Translation,
                2 | 4 => AnimationPath::Rotation,
                3 => AnimationPath::Scale,
                _ => continue,
            };
            channels.push(GlbAnimationChannel {
                node,
                path,
                times,
                values,
            });
        }
        if !self.weight_frames.is_empty() {
            let mut frames = self
                .weight_frames
                .into_iter()
                .map(|(time_bits, values)| (f32::from_bits(time_bits), values))
                .collect::<Vec<_>>();
            frames.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let mut times = Vec::with_capacity(frames.len());
            let mut values = Vec::new();
            for (time, frame_values) in frames {
                times.push(time);
                values.extend_from_slice(&frame_values);
            }
            channels.push(GlbAnimationChannel {
                node: 0,
                path: AnimationPath::Weights,
                times,
                values,
            });
        }
        channels
    }

    fn blend_shape_index(&self, attribute_hash: u32) -> Option<usize> {
        self.morph_target_names
            .iter()
            .position(|name| AnimationClipExporter::unity_crc32(name.as_bytes()) == attribute_hash)
    }

    fn unity_euler_to_gltf_quaternion(values: &[f32]) -> Vec<f32> {
        let (x, y, z) = (
            values[0].to_radians(),
            values[1].to_radians(),
            values[2].to_radians(),
        );
        let qx = Self::axis_angle_quaternion(1.0, 0.0, 0.0, x);
        let qy = Self::axis_angle_quaternion(0.0, 1.0, 0.0, y);
        let qz = Self::axis_angle_quaternion(0.0, 0.0, 1.0, z);
        let unity = Self::multiply_quaternion(Self::multiply_quaternion(qy, qx), qz);
        vec![unity[0], -unity[1], -unity[2], unity[3]]
    }

    fn axis_angle_quaternion(x: f32, y: f32, z: f32, angle: f32) -> [f32; 4] {
        let half = angle * 0.5;
        let sin = half.sin();
        [x * sin, y * sin, z * sin, half.cos()]
    }

    fn multiply_quaternion(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
        [
            a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
            a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
            a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
            a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::AnimationClipExporter;
    use super::AnimationPath;
    use crate::unity::type_tree::unity_value::UnityValue;
    use std::collections::HashMap;

    #[test]
    fn modern_dense_clip_extracts_transform_translation_channel() {
        let path_hash = AnimationClipExporter::unity_crc32("root/hip".as_bytes());
        let mut hash_to_joint = HashMap::new();
        hash_to_joint.insert(path_hash, 1usize);
        let clip = animation_clip_value(
            "walk",
            vec![binding(path_hash, 1, 4)],
            dense_clip(2, 3, 30.0, 0.0, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]),
        );

        let animation =
            AnimationClipExporter::extract_from_value(&clip, &HashMap::new(), &hash_to_joint, &[])
                .expect("modern dense animation");

        assert_eq!(animation.name, "walk");
        assert_eq!(animation.channels.len(), 1);
        assert!(matches!(
            animation.channels[0].path,
            AnimationPath::Translation
        ));
        assert_eq!(animation.channels[0].node, 1);
        assert_eq!(animation.channels[0].times, vec![0.0, 1.0 / 30.0]);
        assert_eq!(
            animation.channels[0].values,
            vec![-1.0, 2.0, 3.0, -4.0, 5.0, 6.0]
        );
    }

    #[test]
    fn modern_streamed_clip_extracts_transform_scale_channel() {
        let path_hash = AnimationClipExporter::unity_crc32("root/hip".as_bytes());
        let mut hash_to_joint = HashMap::new();
        hash_to_joint.insert(path_hash, 2usize);
        let clip = animation_clip_value_with_stream(
            "idle",
            vec![binding(path_hash, 3, 4)],
            streamed_clip_with_frames(vec![
                (0.0, vec![(0, 1.0), (1, 1.0), (2, 1.0)]),
                (0.5, vec![(0, 2.0), (1, 3.0), (2, 4.0)]),
            ]),
        );

        let animation =
            AnimationClipExporter::extract_from_value(&clip, &HashMap::new(), &hash_to_joint, &[])
                .expect("modern streamed animation");

        assert_eq!(animation.name, "idle");
        assert_eq!(animation.channels.len(), 1);
        assert!(matches!(animation.channels[0].path, AnimationPath::Scale));
        assert_eq!(animation.channels[0].node, 2);
        assert_eq!(animation.channels[0].times, vec![0.0, 0.5]);
        assert_eq!(
            animation.channels[0].values,
            vec![1.0, 1.0, 1.0, 2.0, 3.0, 4.0]
        );
    }

    #[test]
    fn modern_dense_clip_extracts_transform_euler_as_rotation_channel() {
        let path_hash = AnimationClipExporter::unity_crc32("root/hip".as_bytes());
        let mut hash_to_joint = HashMap::new();
        hash_to_joint.insert(path_hash, 3usize);
        let clip = animation_clip_value(
            "turn",
            vec![binding(path_hash, 4, 4)],
            dense_clip(2, 3, 30.0, 0.0, vec![0.0, 0.0, 0.0, 0.0, 90.0, 0.0]),
        );

        let animation =
            AnimationClipExporter::extract_from_value(&clip, &HashMap::new(), &hash_to_joint, &[])
                .expect("modern euler animation");

        assert_eq!(animation.channels.len(), 1);
        assert!(matches!(
            animation.channels[0].path,
            AnimationPath::Rotation
        ));
        assert_eq!(animation.channels[0].node, 3);
        assert_eq!(animation.channels[0].times, vec![0.0, 1.0 / 30.0]);
        assert_close_slice(
            &animation.channels[0].values,
            &[0.0, -0.0, -0.0, 1.0, 0.0, -0.70710677, -0.0, 0.70710677],
        );
    }

    #[test]
    fn modern_streamed_clip_uses_binding_start_for_sparse_curve_keys() {
        let rotation_hash = AnimationClipExporter::unity_crc32("root/spine".as_bytes());
        let scale_hash = AnimationClipExporter::unity_crc32("root/hip".as_bytes());
        let mut hash_to_joint = HashMap::new();
        hash_to_joint.insert(rotation_hash, 4usize);
        hash_to_joint.insert(scale_hash, 5usize);
        let clip = animation_clip_value_with_stream(
            "sparse",
            vec![binding(rotation_hash, 2, 4), binding(scale_hash, 3, 4)],
            streamed_clip_with_frames(vec![
                (
                    0.0,
                    vec![
                        (0, 0.0),
                        (1, 0.0),
                        (2, 0.0),
                        (3, 1.0),
                        (4, 1.0),
                        (5, 1.0),
                        (6, 1.0),
                    ],
                ),
                (
                    0.5,
                    vec![(1, 0.25), (2, 0.5), (3, 0.75), (4, 2.0), (5, 3.0), (6, 4.0)],
                ),
            ]),
        );

        let animation =
            AnimationClipExporter::extract_from_value(&clip, &HashMap::new(), &hash_to_joint, &[])
                .expect("modern sparse streamed animation");

        assert_eq!(animation.channels.len(), 2);
        let rotation = animation
            .channels
            .iter()
            .find(|channel| matches!(channel.path, AnimationPath::Rotation))
            .expect("rotation channel");
        assert_eq!(rotation.node, 4);
        assert_eq!(rotation.times, vec![0.0]);

        let scale = animation
            .channels
            .iter()
            .find(|channel| matches!(channel.path, AnimationPath::Scale))
            .expect("scale channel");
        assert_eq!(scale.node, 5);
        assert_eq!(scale.times, vec![0.0, 0.5]);
        assert_eq!(scale.values, vec![1.0, 1.0, 1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn hash_aliases_make_avatar_tos_bindings_playable() {
        let avatar_tos_hash = AnimationClipExporter::unity_crc32("avatar/body_dummy".as_bytes());
        let clip = animation_clip_value(
            "avatar-bound",
            vec![binding(avatar_tos_hash, 1, 4)],
            dense_clip(1, 3, 30.0, 0.0, vec![1.0, 2.0, 3.0]),
        );

        assert!(
            AnimationClipExporter::extract_from_value(
                &clip,
                &HashMap::new(),
                &HashMap::new(),
                &[]
            )
            .is_none(),
            "without Avatar TOS aliases the binding hash should not hit the preview skeleton"
        );

        let mut hash_aliases = HashMap::new();
        hash_aliases.insert(avatar_tos_hash, 7usize);
        let animation =
            AnimationClipExporter::extract_from_value(&clip, &HashMap::new(), &hash_aliases, &[])
                .expect("alias-backed animation");

        assert_eq!(animation.channels.len(), 1);
        assert_eq!(animation.channels[0].node, 7);
        assert!(matches!(
            animation.channels[0].path,
            AnimationPath::Translation
        ));
    }

    fn animation_clip_value(
        name: &str,
        bindings: Vec<UnityValue>,
        dense_clip: UnityValue,
    ) -> UnityValue {
        animation_clip_value_with_parts(name, bindings, streamed_clip(), dense_clip)
    }

    fn animation_clip_value_with_stream(
        name: &str,
        bindings: Vec<UnityValue>,
        streamed_clip: UnityValue,
    ) -> UnityValue {
        animation_clip_value_with_parts(
            name,
            bindings,
            streamed_clip,
            dense_clip(0, 0, 0.0, 0.0, Vec::new()),
        )
    }

    fn animation_clip_value_with_parts(
        name: &str,
        bindings: Vec<UnityValue>,
        streamed_clip: UnityValue,
        dense_clip: UnityValue,
    ) -> UnityValue {
        let mut runtime_clip = HashMap::new();
        runtime_clip.insert("m_StreamedClip".to_string(), streamed_clip);
        runtime_clip.insert("m_DenseClip".to_string(), dense_clip);

        let mut muscle_clip = HashMap::new();
        muscle_clip.insert("m_Clip".to_string(), UnityValue::Object(runtime_clip));
        muscle_clip.insert("m_StopTime".to_string(), UnityValue::Float(1.0));

        let mut binding_constant = HashMap::new();
        binding_constant.insert("genericBindings".to_string(), UnityValue::Array(bindings));

        let mut clip = HashMap::new();
        clip.insert("m_Name".to_string(), UnityValue::String(name.to_string()));
        clip.insert("m_MuscleClip".to_string(), UnityValue::Object(muscle_clip));
        clip.insert(
            "m_ClipBindingConstant".to_string(),
            UnityValue::Object(binding_constant),
        );
        UnityValue::Object(clip)
    }

    fn binding(path: u32, attribute: u32, type_id: i32) -> UnityValue {
        let mut map = HashMap::new();
        map.insert("path".to_string(), UnityValue::Integer(path as i64));
        map.insert(
            "attribute".to_string(),
            UnityValue::Integer(attribute as i64),
        );
        map.insert("typeID".to_string(), UnityValue::Integer(type_id as i64));
        UnityValue::Object(map)
    }

    fn dense_clip(
        frame_count: i64,
        curve_count: i64,
        sample_rate: f64,
        begin_time: f64,
        samples: Vec<f32>,
    ) -> UnityValue {
        let mut map = HashMap::new();
        map.insert("m_FrameCount".to_string(), UnityValue::Integer(frame_count));
        map.insert("m_CurveCount".to_string(), UnityValue::Integer(curve_count));
        map.insert("m_SampleRate".to_string(), UnityValue::Float(sample_rate));
        map.insert("m_BeginTime".to_string(), UnityValue::Float(begin_time));
        map.insert(
            "m_SampleArray".to_string(),
            UnityValue::Array(
                samples
                    .into_iter()
                    .map(|value| UnityValue::Float(value as f64))
                    .collect(),
            ),
        );
        UnityValue::Object(map)
    }

    fn streamed_clip() -> UnityValue {
        let mut map = HashMap::new();
        map.insert("data".to_string(), UnityValue::Array(Vec::new()));
        map.insert("curveCount".to_string(), UnityValue::Integer(0));
        UnityValue::Object(map)
    }

    fn streamed_clip_with_frames(frames: Vec<(f32, Vec<(i32, f32)>)>) -> UnityValue {
        let mut bytes = Vec::new();
        let mut curve_count = 0usize;
        for (time, keys) in frames {
            bytes.extend_from_slice(&time.to_le_bytes());
            bytes.extend_from_slice(&(keys.len() as i32).to_le_bytes());
            for (index, value) in keys {
                curve_count = curve_count.max(index as usize + 1);
                bytes.extend_from_slice(&index.to_le_bytes());
                bytes.extend_from_slice(&0.0f32.to_le_bytes());
                bytes.extend_from_slice(&0.0f32.to_le_bytes());
                bytes.extend_from_slice(&0.0f32.to_le_bytes());
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        let words = bytes
            .chunks_exact(4)
            .map(|chunk| UnityValue::Integer(u32::from_le_bytes(chunk.try_into().unwrap()) as i64))
            .collect();
        let mut map = HashMap::new();
        map.insert("data".to_string(), UnityValue::Array(words));
        map.insert(
            "curveCount".to_string(),
            UnityValue::Integer(curve_count as i64),
        );
        UnityValue::Object(map)
    }

    fn assert_close_slice(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len());
        for (index, (actual, expected)) in actual.iter().zip(expected.iter()).enumerate() {
            assert!(
                (actual - expected).abs() < 0.00001,
                "value at index {}: expected {}, got {}",
                index,
                expected,
                actual
            );
        }
    }
}
