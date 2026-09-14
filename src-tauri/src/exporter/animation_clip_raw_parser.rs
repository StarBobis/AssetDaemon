use std::collections::HashMap;

use crate::common::bundle_file::asset_bundle::{ObjectInfo, SerializedFile};
use crate::common::serialized_file::object_reader::ObjectReader;
use crate::unity::type_tree::unity_value::UnityValue;

/// AnimationClipRawParser 专门负责读取 Unity AnimationClip 的原始二进制布局。
///
/// 这里不依赖 TypeTree，是因为部分 Unity 2019 bundle 的 blob TypeTree
/// common string 会错位，通用 TypeTree 读取器会把 AnimationClip 读成整数。
/// 读取顺序参考项目根目录 `assetstudio/AssetStudio/Classes/AnimationClip.cs`。
pub struct AnimationClipRawParser;

impl AnimationClipRawParser {
    /// 尝试把 class_id=74 的 AnimationClip 原始数据转换成项目已有的 UnityValue 结构。
    ///
    /// 返回 UnityValue 的原因是导出器已经有一套从 UnityValue 生成 GLB 动画通道的逻辑；
    /// raw parser 只补齐“读取 Unity 原始结构”这一层，避免重复写通道转换逻辑。
    pub fn parse(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<UnityValue, String> {
        let raw_data = serialized_file.object_bytes(object_info)?;
        let candidate_offsets = Self::candidate_named_object_offsets(&serialized_file.inner);
        let mut last_error = String::from("AnimationClip raw parse did not run");

        for offset in candidate_offsets {
            let mut reader = ObjectReader::new(raw_data, &serialized_file.inner);
            reader.pos = offset.min(raw_data.len());
            match Self::parse_from_current_position(&mut reader) {
                Ok(value) if Self::contains_runtime_clip_data(&value) => return Ok(value),
                Ok(_) => {
                    last_error =
                        format!("AnimationClip raw parse at offset {} had no curves", offset);
                }
                Err(error) => {
                    last_error = format!(
                        "AnimationClip raw parse at offset {} failed: {}",
                        offset, error
                    );
                }
            }
        }

        Err(last_error)
    }

    /// 计算 NamedObject.m_Name 可能出现的位置。
    ///
    /// AssetStudio 在 NoTarget 平台会先读取 Object/EditorExtension 基类字段；
    /// 普通运行时平台通常直接从 m_Name 开始。这里同时尝试两种位置，避免平台字段判断
    /// 或历史文件布局差异导致整个 AnimationClip 解析失败。
    fn candidate_named_object_offsets(
        serialized_file: &crate::common::serialized_file::serialized_file::SerializedFile,
    ) -> Vec<usize> {
        let mut offsets = Vec::new();
        if serialized_file.target_platform == -2 {
            offsets.push(
                4usize.saturating_add(
                    serialized_file
                        .header
                        .version
                        .ge(&14)
                        .then_some(24)
                        .unwrap_or(16),
                ),
            );
        }
        offsets.push(0);
        offsets.sort_unstable();
        offsets.dedup();
        offsets
    }

    /// 按 AssetStudio 的 AnimationClip 构造函数顺序读取字段。
    ///
    /// 当前导出 GLB 需要的是 m_MuscleClip.m_Clip 和 m_ClipBindingConstant；
    /// 传统曲线字段仍要正确跳过，否则后续读取位置会错位。
    fn parse_from_current_position(reader: &mut ObjectReader) -> Result<UnityValue, String> {
        let name = reader.read_aligned_string();
        Self::read_animation_flags(reader);
        Self::skip_quaternion_curve_array(reader, "m_RotationCurves")?;
        Self::skip_compressed_rotation_curve_array(reader, "m_CompressedRotationCurves")?;

        if Self::version_at_least(reader, 5, 3) {
            Self::skip_vector3_curve_array(reader, "m_EulerCurves")?;
        }

        Self::skip_vector3_curve_array(reader, "m_PositionCurves")?;
        Self::skip_vector3_curve_array(reader, "m_ScaleCurves")?;
        Self::skip_float_curve_array(reader, "m_FloatCurves")?;

        if Self::version_at_least(reader, 4, 3) {
            Self::skip_pptr_curve_array(reader, "m_PPtrCurves")?;
        }

        let _sample_rate = reader.read_f32();
        let _wrap_mode = reader.read_i32();
        if Self::version_at_least(reader, 3, 4) {
            Self::skip_vector3(reader);
            Self::skip_vector3(reader);
        }

        let mut clip_map = HashMap::new();
        clip_map.insert("m_Name".to_string(), UnityValue::String(name));

        if reader.version[0] >= 4 {
            let _muscle_clip_size = reader.read_u32();
            let muscle_clip = Self::read_clip_muscle_constant(reader)?;
            clip_map.insert("m_MuscleClip".to_string(), UnityValue::Object(muscle_clip));
        }

        if Self::version_at_least(reader, 4, 3) {
            let binding_constant = Self::read_animation_clip_binding_constant(reader)?;
            clip_map.insert(
                "m_ClipBindingConstant".to_string(),
                UnityValue::Object(binding_constant),
            );
        }

        if Self::version_at_least(reader, 2018, 3) {
            let _has_generic_root_transform = reader.read_bool();
            let _has_motion_float_curves = reader.read_bool();
            reader.align();
        }

        Self::skip_animation_events(reader)?;
        if reader.version[0] >= 2017 {
            reader.align();
        }

        Ok(UnityValue::Object(clip_map))
    }

    /// 检查 raw parser 读出的结构是否包含现代运行时动画数据。
    ///
    /// 这个校验用于过滤错误起始偏移：错位读取有时不会立刻崩溃，但不会形成合理的
    /// m_StreamedClip/m_DenseClip/m_ConstantClip 数据。
    fn contains_runtime_clip_data(value: &UnityValue) -> bool {
        let UnityValue::Object(clip_map) = value else {
            return false;
        };
        let Some(UnityValue::Object(muscle_clip)) = clip_map.get("m_MuscleClip") else {
            return false;
        };
        let Some(UnityValue::Object(runtime_clip)) = muscle_clip.get("m_Clip") else {
            return false;
        };
        Self::array_len_in_object(runtime_clip, "m_StreamedClip", "data") > 0
            || Self::array_len_in_object(runtime_clip, "m_DenseClip", "m_SampleArray") > 0
            || Self::array_len_in_object(runtime_clip, "m_ConstantClip", "data") > 0
    }

    /// 读取 Unity 5+ 的 AnimationClip 标志位并处理 4 字节对齐。
    fn read_animation_flags(reader: &mut ObjectReader) {
        if reader.version[0] >= 5 {
            let _legacy = reader.read_bool();
        } else if reader.version[0] >= 4 {
            let _animation_type = reader.read_i32();
        }
        let _compressed = reader.read_bool();
        if Self::version_at_least(reader, 4, 3) {
            let _use_high_quality_curve = reader.read_bool();
        }
        reader.align();
    }

    /// 读取 ClipMuscleConstant，并只保留 GLB 导出需要的 Runtime Clip 和 stop time。
    fn read_clip_muscle_constant(
        reader: &mut ObjectReader,
    ) -> Result<HashMap<String, UnityValue>, String> {
        Self::skip_human_pose(reader)?;
        Self::skip_xform(reader);
        if Self::version_at_least(reader, 5, 5) {
            Self::skip_xform(reader);
        }
        Self::skip_xform(reader);
        Self::skip_xform(reader);
        if reader.version[0] < 5 {
            Self::skip_xform(reader);
            Self::skip_xform(reader);
        }
        Self::skip_versioned_vector3_or_vector4(reader);

        let runtime_clip = Self::read_runtime_clip(reader)?;
        let _start_time = reader.read_f32();
        let stop_time = reader.read_f32();
        let _orientation_offset_y = reader.read_f32();
        let _level = reader.read_f32();
        let _cycle_offset = reader.read_f32();
        let _average_angular_speed = reader.read_f32();

        Self::skip_i32_array(reader, "m_IndexArray")?;
        if !Self::version_at_least(reader, 4, 3) {
            Self::skip_i32_array(reader, "m_AdditionalCurveIndexArray")?;
        }

        let value_delta_count = Self::read_sane_count(reader, 8, "m_ValueArrayDelta")?;
        reader.skip(value_delta_count.saturating_mul(8));

        if Self::version_at_least(reader, 5, 3) {
            Self::skip_f32_array(reader, "m_ValueArrayReferencePose")?;
        }

        let _mirror = reader.read_bool();
        if Self::version_at_least(reader, 4, 3) {
            let _loop_time = reader.read_bool();
        }
        let _loop_blend = reader.read_bool();
        let _loop_blend_orientation = reader.read_bool();
        let _loop_blend_position_y = reader.read_bool();
        let _loop_blend_position_xz = reader.read_bool();
        if Self::version_at_least(reader, 5, 5) {
            let _start_at_origin = reader.read_bool();
        }
        let _keep_original_orientation = reader.read_bool();
        let _keep_original_position_y = reader.read_bool();
        let _keep_original_position_xz = reader.read_bool();
        let _height_from_feet = reader.read_bool();
        reader.align();

        let mut muscle_clip = HashMap::new();
        muscle_clip.insert("m_Clip".to_string(), UnityValue::Object(runtime_clip));
        muscle_clip.insert(
            "m_StopTime".to_string(),
            UnityValue::Float(stop_time as f64),
        );
        Ok(muscle_clip)
    }

    /// 读取现代 Clip 容器：Streamed/Dense/Constant 三类曲线数据。
    fn read_runtime_clip(reader: &mut ObjectReader) -> Result<HashMap<String, UnityValue>, String> {
        let streamed_clip = Self::read_streamed_clip(reader)?;
        let dense_clip = Self::read_dense_clip(reader)?;
        let constant_clip = if Self::version_at_least(reader, 4, 3) {
            Self::read_constant_clip(reader)?
        } else {
            HashMap::new()
        };

        let mut runtime_clip = HashMap::new();
        runtime_clip.insert(
            "m_StreamedClip".to_string(),
            UnityValue::Object(streamed_clip),
        );
        runtime_clip.insert("m_DenseClip".to_string(), UnityValue::Object(dense_clip));
        runtime_clip.insert(
            "m_ConstantClip".to_string(),
            UnityValue::Object(constant_clip),
        );

        if !Self::version_at_least(reader, 2018, 3) {
            let binding = Self::read_value_array_constant(reader)?;
            runtime_clip.insert("m_Binding".to_string(), UnityValue::Object(binding));
        }

        Ok(runtime_clip)
    }

    /// StreamedClip 的 data 是 uint32 数组，后续导出器会按 AssetStudio 的 ReadData 方式重放。
    fn read_streamed_clip(
        reader: &mut ObjectReader,
    ) -> Result<HashMap<String, UnityValue>, String> {
        let data = Self::read_u32_array(reader, "m_StreamedClip.data")?;
        let curve_count = reader.read_u32();
        let mut streamed_clip = HashMap::new();
        streamed_clip.insert("data".to_string(), Self::u32_values(data));
        streamed_clip.insert(
            "curveCount".to_string(),
            UnityValue::Integer(curve_count as i64),
        );
        Ok(streamed_clip)
    }

    /// DenseClip 保存等间隔采样曲线，是很多 Unity 2019 动画的主要数据来源。
    fn read_dense_clip(reader: &mut ObjectReader) -> Result<HashMap<String, UnityValue>, String> {
        let frame_count = reader.read_i32();
        let curve_count = reader.read_u32();
        let sample_rate = reader.read_f32();
        let begin_time = reader.read_f32();
        let sample_array = Self::read_f32_array(reader, "m_DenseClip.m_SampleArray")?;

        let mut dense_clip = HashMap::new();
        dense_clip.insert(
            "m_FrameCount".to_string(),
            UnityValue::Integer(frame_count as i64),
        );
        dense_clip.insert(
            "m_CurveCount".to_string(),
            UnityValue::Integer(curve_count as i64),
        );
        dense_clip.insert(
            "m_SampleRate".to_string(),
            UnityValue::Float(sample_rate as f64),
        );
        dense_clip.insert(
            "m_BeginTime".to_string(),
            UnityValue::Float(begin_time as f64),
        );
        dense_clip.insert("m_SampleArray".to_string(), Self::f32_values(sample_array));
        Ok(dense_clip)
    }

    /// ConstantClip 保存整段动画不变的曲线，导出时会写成首尾两个关键帧。
    fn read_constant_clip(
        reader: &mut ObjectReader,
    ) -> Result<HashMap<String, UnityValue>, String> {
        let data = Self::read_f32_array(reader, "m_ConstantClip.data")?;
        let mut constant_clip = HashMap::new();
        constant_clip.insert("data".to_string(), Self::f32_values(data));
        Ok(constant_clip)
    }

    /// 读取 Unity 2018.3+ 的 AnimationClipBindingConstant。
    ///
    /// binding 决定每一段曲线数据对应哪个 Transform 或 BlendShape。
    fn read_animation_clip_binding_constant(
        reader: &mut ObjectReader,
    ) -> Result<HashMap<String, UnityValue>, String> {
        let binding_count = Self::read_sane_count(reader, 16, "genericBindings")?;
        let mut bindings = Vec::with_capacity(binding_count);
        for _ in 0..binding_count {
            bindings.push(UnityValue::Object(Self::read_generic_binding(reader)));
        }

        let mapping_count = Self::read_sane_count(reader, reader.ppt_size(), "pptrCurveMapping")?;
        for _ in 0..mapping_count {
            Self::skip_pptr(reader);
        }

        let mut binding_constant = HashMap::new();
        binding_constant.insert("genericBindings".to_string(), UnityValue::Array(bindings));
        Ok(binding_constant)
    }

    /// GenericBinding 的 path/attribute/typeID 是导出动画通道必须用到的核心字段。
    fn read_generic_binding(reader: &mut ObjectReader) -> HashMap<String, UnityValue> {
        let path = reader.read_u32();
        let attribute = reader.read_u32();
        Self::skip_pptr(reader);
        let type_id = if Self::version_at_least(reader, 5, 6) {
            reader.read_i32()
        } else {
            reader.read_u16() as i32
        };
        let _custom_type = reader.read_u8();
        let _is_pptr_curve = reader.read_u8();
        if Self::version_at_least(reader, 2022, 1) {
            let _is_int_curve = reader.read_u8();
        }
        reader.align();

        let mut binding = HashMap::new();
        binding.insert("path".to_string(), UnityValue::Integer(path as i64));
        binding.insert(
            "attribute".to_string(),
            UnityValue::Integer(attribute as i64),
        );
        binding.insert("typeID".to_string(), UnityValue::Integer(type_id as i64));
        binding
    }

    /// 读取旧版本 Clip.m_Binding，保留给 2018.3 以下资源作为兼容路径。
    fn read_value_array_constant(
        reader: &mut ObjectReader,
    ) -> Result<HashMap<String, UnityValue>, String> {
        let value_count = Self::read_sane_count(reader, 12, "m_ValueArray")?;
        let mut values = Vec::with_capacity(value_count);
        for _ in 0..value_count {
            let mut value = HashMap::new();
            value.insert(
                "m_ID".to_string(),
                UnityValue::Integer(reader.read_u32() as i64),
            );
            if !Self::version_at_least(reader, 5, 5) {
                value.insert(
                    "m_TypeID".to_string(),
                    UnityValue::Integer(reader.read_u32() as i64),
                );
            }
            value.insert(
                "m_Type".to_string(),
                UnityValue::Integer(reader.read_u32() as i64),
            );
            value.insert(
                "m_Index".to_string(),
                UnityValue::Integer(reader.read_u32() as i64),
            );
            values.push(UnityValue::Object(value));
        }

        let mut binding = HashMap::new();
        binding.insert("m_ValueArray".to_string(), UnityValue::Array(values));
        Ok(binding)
    }

    /// 跳过 QuaternionCurve[]，用于保持 reader 位置与 AssetStudio 一致。
    fn skip_quaternion_curve_array(
        reader: &mut ObjectReader,
        field_name: &str,
    ) -> Result<(), String> {
        let count = Self::read_sane_count(reader, 4, field_name)?;
        for _ in 0..count {
            Self::skip_animation_curve(reader, 16, field_name)?;
            let _path = reader.read_aligned_string();
        }
        Ok(())
    }

    /// 跳过 CompressedAnimationCurve[]，压缩旋转曲线当前不直接导出。
    fn skip_compressed_rotation_curve_array(
        reader: &mut ObjectReader,
        field_name: &str,
    ) -> Result<(), String> {
        let count = Self::read_sane_count(reader, 4, field_name)?;
        for _ in 0..count {
            let _path = reader.read_aligned_string();
            Self::skip_packed_int_vector(reader)?;
            Self::skip_packed_quat_vector(reader)?;
            Self::skip_packed_float_vector(reader)?;
            reader.skip(8);
        }
        Ok(())
    }

    /// 跳过 Vector3Curve[]，TypeTree 可用时原逻辑仍会处理这些传统曲线。
    fn skip_vector3_curve_array(reader: &mut ObjectReader, field_name: &str) -> Result<(), String> {
        let count = Self::read_sane_count(reader, 4, field_name)?;
        for _ in 0..count {
            Self::skip_animation_curve(reader, 12, field_name)?;
            let _path = reader.read_aligned_string();
        }
        Ok(())
    }

    /// 跳过 FloatCurve[]，其中可能包含 BlendShape；当前 raw 路径优先修复现代 muscle clip。
    fn skip_float_curve_array(reader: &mut ObjectReader, field_name: &str) -> Result<(), String> {
        let count = Self::read_sane_count(reader, 4, field_name)?;
        for _ in 0..count {
            Self::skip_animation_curve(reader, 4, field_name)?;
            let _attribute = reader.read_aligned_string();
            let _path = reader.read_aligned_string();
            let _class_id = reader.read_i32();
            Self::skip_pptr(reader);
        }
        Ok(())
    }

    /// 跳过 PPtrCurve[]，GLB 骨骼动画不需要对象引用曲线。
    fn skip_pptr_curve_array(reader: &mut ObjectReader, field_name: &str) -> Result<(), String> {
        let count = Self::read_sane_count(reader, 4, field_name)?;
        for _ in 0..count {
            let key_count = Self::read_sane_count(reader, reader.ppt_size() + 4, field_name)?;
            for _ in 0..key_count {
                let _time = reader.read_f32();
                Self::skip_pptr(reader);
            }
            let _attribute = reader.read_aligned_string();
            let _path = reader.read_aligned_string();
            let _class_id = reader.read_i32();
            Self::skip_pptr(reader);
        }
        Ok(())
    }

    /// 跳过 AnimationCurve<T>，value_size 是 T 在二进制中的字节数。
    fn skip_animation_curve(
        reader: &mut ObjectReader,
        value_size: usize,
        field_name: &str,
    ) -> Result<(), String> {
        let key_size = if reader.version[0] >= 2018 {
            8usize.saturating_add(value_size.saturating_mul(5))
        } else {
            4usize.saturating_add(value_size.saturating_mul(3))
        };
        let key_count = Self::read_sane_count(reader, key_size, field_name)?;
        reader.skip(key_count.saturating_mul(key_size));
        reader.skip(8);
        if Self::version_at_least(reader, 5, 3) {
            reader.skip(4);
        }
        Ok(())
    }

    /// 跳过 HumanPose，AnimationClip 的 modern clip 位于这个复杂结构之后。
    fn skip_human_pose(reader: &mut ObjectReader) -> Result<(), String> {
        Self::skip_xform(reader);
        Self::skip_versioned_vector3_or_vector4(reader);
        reader.skip(16);

        let goal_count = Self::read_sane_count(reader, 32, "m_GoalArray")?;
        for _ in 0..goal_count {
            Self::skip_xform(reader);
            reader.skip(8);
            if reader.version[0] >= 5 {
                Self::skip_versioned_vector3_or_vector4(reader);
                reader.skip(4);
            }
        }

        Self::skip_hand_pose(reader)?;
        Self::skip_hand_pose(reader)?;
        Self::skip_f32_array(reader, "m_DoFArray")?;

        if Self::version_at_least(reader, 5, 2) {
            let tdof_count = Self::read_sane_count(reader, 12, "m_TDoFArray")?;
            for _ in 0..tdof_count {
                Self::skip_versioned_vector3_or_vector4(reader);
            }
        }
        Ok(())
    }

    /// 跳过 HandPose，内部包含 xform、自由度数组和四个控制浮点。
    fn skip_hand_pose(reader: &mut ObjectReader) -> Result<(), String> {
        Self::skip_xform(reader);
        Self::skip_f32_array(reader, "HandPose.m_DoFArray")?;
        reader.skip(16);
        Ok(())
    }

    /// 跳过 Unity xform：位移、旋转、缩放。
    fn skip_xform(reader: &mut ObjectReader) {
        Self::skip_versioned_vector3_or_vector4(reader);
        reader.skip(16);
        Self::skip_versioned_vector3_or_vector4(reader);
    }

    /// Unity 5.4 以后这些字段是 Vector3；更早版本是 Vector4。
    fn skip_versioned_vector3_or_vector4(reader: &mut ObjectReader) {
        if Self::version_at_least(reader, 5, 4) {
            Self::skip_vector3(reader);
        } else {
            reader.skip(16);
        }
    }

    /// 跳过标准 Vector3。
    fn skip_vector3(reader: &mut ObjectReader) {
        reader.skip(12);
    }

    /// 跳过 PackedIntVector。
    fn skip_packed_int_vector(reader: &mut ObjectReader) -> Result<(), String> {
        let _item_count = reader.read_u32();
        Self::skip_byte_array(reader, "PackedIntVector.m_Data")?;
        let _bit_size = reader.read_u8();
        reader.align();
        Ok(())
    }

    /// 跳过 PackedQuatVector。
    fn skip_packed_quat_vector(reader: &mut ObjectReader) -> Result<(), String> {
        let _item_count = reader.read_u32();
        Self::skip_byte_array(reader, "PackedQuatVector.m_Data")?;
        Ok(())
    }

    /// 跳过 PackedFloatVector。
    fn skip_packed_float_vector(reader: &mut ObjectReader) -> Result<(), String> {
        let _item_count = reader.read_u32();
        let _range = reader.read_f32();
        let _start = reader.read_f32();
        Self::skip_byte_array(reader, "PackedFloatVector.m_Data")?;
        let _bit_size = reader.read_u8();
        reader.align();
        Ok(())
    }

    /// 跳过 AnimationEvent[]，事件不参与 GLB 动画采样导出。
    fn skip_animation_events(reader: &mut ObjectReader) -> Result<(), String> {
        let event_count = Self::read_sane_count(reader, 24, "m_Events")?;
        for _ in 0..event_count {
            let _time = reader.read_f32();
            let _function_name = reader.read_aligned_string();
            let _data = reader.read_aligned_string();
            Self::skip_pptr(reader);
            let _float_parameter = reader.read_f32();
            if reader.version[0] >= 3 {
                let _int_parameter = reader.read_i32();
            }
            let _message_options = reader.read_i32();
        }
        Ok(())
    }

    /// 跳过 PPtr<Object>。
    fn skip_pptr(reader: &mut ObjectReader) {
        let _file_id = reader.read_i32();
        let _path_id = reader.read_path_id();
    }

    /// 读取 Unity length-prefixed float 数组。
    fn read_f32_array(reader: &mut ObjectReader, field_name: &str) -> Result<Vec<f32>, String> {
        let count = Self::read_sane_count(reader, 4, field_name)?;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(reader.read_f32());
        }
        Ok(values)
    }

    /// 读取 Unity length-prefixed uint 数组。
    fn read_u32_array(reader: &mut ObjectReader, field_name: &str) -> Result<Vec<u32>, String> {
        let count = Self::read_sane_count(reader, 4, field_name)?;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(reader.read_u32());
        }
        Ok(values)
    }

    /// 跳过 Unity length-prefixed int 数组。
    fn skip_i32_array(reader: &mut ObjectReader, field_name: &str) -> Result<(), String> {
        let count = Self::read_sane_count(reader, 4, field_name)?;
        reader.skip(count.saturating_mul(4));
        Ok(())
    }

    /// 跳过 Unity length-prefixed float 数组。
    fn skip_f32_array(reader: &mut ObjectReader, field_name: &str) -> Result<(), String> {
        let count = Self::read_sane_count(reader, 4, field_name)?;
        reader.skip(count.saturating_mul(4));
        Ok(())
    }

    /// 跳过 Unity length-prefixed byte 数组并处理 4 字节对齐。
    fn skip_byte_array(reader: &mut ObjectReader, field_name: &str) -> Result<(), String> {
        let count = Self::read_sane_count(reader, 1, field_name)?;
        reader.skip(count);
        reader.align();
        Ok(())
    }

    /// 读取数组长度并用剩余字节做基本校验，避免错位后分配巨量内存。
    fn read_sane_count(
        reader: &mut ObjectReader,
        minimum_item_size: usize,
        field_name: &str,
    ) -> Result<usize, String> {
        let count = reader.read_i32();
        if count < 0 {
            return Err(format!("{} has negative count {}", field_name, count));
        }
        let count = count as usize;
        let minimum_size = count.saturating_mul(minimum_item_size);
        if minimum_size > reader.remaining() {
            return Err(format!(
                "{} count {} exceeds remaining {} bytes",
                field_name,
                count,
                reader.remaining()
            ));
        }
        Ok(count)
    }

    /// 判断 Unity 主版本/次版本是否达到指定版本。
    fn version_at_least(reader: &ObjectReader, major: i32, minor: i32) -> bool {
        reader.version[0] > major || (reader.version[0] == major && reader.version[1] >= minor)
    }

    /// 获取嵌套数组长度，用于 parse 后的结构合理性校验。
    fn array_len_in_object(
        root: &HashMap<String, UnityValue>,
        object_key: &str,
        array_key: &str,
    ) -> usize {
        let Some(UnityValue::Object(object)) = root.get(object_key) else {
            return 0;
        };
        let Some(UnityValue::Array(items)) = object.get(array_key) else {
            return 0;
        };
        items.len()
    }

    /// 把 Vec<f32> 包装成 UnityValue::Array，复用导出器原有读取逻辑。
    fn f32_values(values: Vec<f32>) -> UnityValue {
        UnityValue::Array(
            values
                .into_iter()
                .map(|value| UnityValue::Float(value as f64))
                .collect(),
        )
    }

    /// 把 Vec<u32> 包装成 UnityValue::Array，保留 Unity 原始 uint 数据。
    fn u32_values(values: Vec<u32>) -> UnityValue {
        UnityValue::Array(
            values
                .into_iter()
                .map(|value| UnityValue::Integer(value as i64))
                .collect(),
        )
    }
}
