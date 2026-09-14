/*
 * registry.rs -- Strongly typed Unity class object registry.
 *
 * This module is the object-oriented seam above SerializedFile/ObjectReader:
 * callers ask for a Unity class object and receive a typed struct instead of
 * re-parsing raw bytes at each call site.
 */

use std::collections::HashMap;

use crate::common::bundle_file::res_s::ResS;
use crate::common::serialized_file::object_reader::ObjectReader;
use crate::common::serialized_file::serialized_file::{ObjectInfo, SerializedFile};
use crate::unity::classes::material::{Material, MaterialObject};
use crate::unity::classes::mesh::{Mesh, MeshObject};
use crate::unity::classes::object::{PPtr, RawUnityObject, UnityObjectHeader};
use crate::unity::classes::sprite::{SpriteAtlasObject, SpriteObject};
use crate::unity::classes::sprite_mask::SpriteMaskObject;
use crate::unity::classes::texture2d::{Texture2DObject, TextureReader};
use crate::unity::type_tree::type_tree_reader_utils::TypeTreeReaderUtils;
use crate::unity::type_tree::unity_value::UnityValue;
use crate::utils::class_name_utils::ClassNameUtils;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct GameObjectObject {
    pub header: UnityObjectHeader,
    pub name: String,
    pub components: Vec<PPtr>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ComponentObject {
    pub header: UnityObjectHeader,
    pub game_object: PPtr,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MonoBehaviourObject {
    pub component: ComponentObject,
    pub script: PPtr,
    pub name: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MeshFilterObject {
    pub component: ComponentObject,
    pub mesh: Option<PPtr>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TransformObject {
    pub header: UnityObjectHeader,
    pub game_object: PPtr,
    pub local_rotation: [f32; 4],
    pub local_position: [f32; 3],
    pub local_scale: [f32; 3],
    pub children: Vec<PPtr>,
    pub father: PPtr,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RendererObject {
    pub header: UnityObjectHeader,
    pub game_object: PPtr,
    pub mesh: Option<PPtr>,
    pub materials: Vec<PPtr>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SkinnedMeshRendererObject {
    pub renderer: RendererObject,
    pub bones: Vec<PPtr>,
    pub root_bone: Option<PPtr>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ParticleSystemRendererObject {
    pub renderer: RendererObject,
    /// ParticleSystemRenderMode: 0=Billboard, 1=Stretch, 2=HorizontalBillboard,
    /// 3=VerticalBillboard, 4=Mesh, 5=None. Only Mesh mode renders m_Mesh.
    pub render_mode: Option<i64>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AssetBundleContainerEntry {
    pub asset_path: String,
    pub preload_index: i32,
    pub preload_size: i32,
    pub asset: PPtr,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AssetBundleObject {
    pub header: UnityObjectHeader,
    pub name: String,
    pub preload_table: Vec<PPtr>,
    pub container: Vec<AssetBundleContainerEntry>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AnimatorObject {
    pub component: ComponentObject,
    pub avatar: PPtr,
    pub controller: PPtr,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AnimationObject {
    pub component: ComponentObject,
    pub default_animation: PPtr,
    pub animations: Vec<PPtr>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AnimatorOverrideControllerObject {
    pub header: UnityObjectHeader,
    pub name: String,
    pub controller: PPtr,
    pub clips: Vec<AnimatorOverrideClip>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AnimatorControllerObject {
    pub header: UnityObjectHeader,
    pub name: String,
    pub animation_clips: Vec<PPtr>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AnimatorOverrideClip {
    pub original_clip: PPtr,
    pub override_clip: PPtr,
}

pub enum UnityClassObject {
    GameObject(GameObjectObject),
    Component(ComponentObject),
    MeshFilter(MeshFilterObject),
    Transform(TransformObject),
    Renderer(RendererObject),
    SkinnedMeshRenderer(SkinnedMeshRendererObject),
    ParticleSystemRenderer(ParticleSystemRendererObject),
    Material(MaterialObject),
    AssetBundle(AssetBundleObject),
    Texture2D(Texture2DObject),
    Mesh(MeshObject),
    Animator(AnimatorObject),
    Animation(AnimationObject),
    AnimatorController(AnimatorControllerObject),
    AnimatorOverrideController(AnimatorOverrideControllerObject),
    Sprite(SpriteObject),
    SpriteAtlas(SpriteAtlasObject),
    SpriteMask(SpriteMaskObject),
    Raw(RawUnityObject),
}

impl UnityClassObject {
    pub fn class_name(&self) -> &str {
        match self {
            UnityClassObject::GameObject(object) => &object.header.class_name,
            UnityClassObject::Component(object) => &object.header.class_name,
            UnityClassObject::MeshFilter(object) => &object.component.header.class_name,
            UnityClassObject::Transform(object) => &object.header.class_name,
            UnityClassObject::Renderer(object) => &object.header.class_name,
            UnityClassObject::SkinnedMeshRenderer(object) => &object.renderer.header.class_name,
            UnityClassObject::ParticleSystemRenderer(object) => {
                &object.renderer.header.class_name
            }
            UnityClassObject::Material(object) => &object.header.class_name,
            UnityClassObject::AssetBundle(object) => &object.header.class_name,
            UnityClassObject::Texture2D(object) => &object.header.class_name,
            UnityClassObject::Mesh(object) => &object.header.class_name,
            UnityClassObject::Animator(object) => &object.component.header.class_name,
            UnityClassObject::Animation(object) => &object.component.header.class_name,
            UnityClassObject::AnimatorController(object) => &object.header.class_name,
            UnityClassObject::AnimatorOverrideController(object) => &object.header.class_name,
            UnityClassObject::Sprite(object) => &object.header.class_name,
            UnityClassObject::SpriteAtlas(object) => &object.header.class_name,
            UnityClassObject::SpriteMask(object) => &object.header.class_name,
            UnityClassObject::Raw(object) => &object.header.class_name,
        }
    }
}

pub struct UnityClassParser;

impl UnityClassParser {
    pub fn parse_object(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
        resources: &HashMap<String, ResS>,
    ) -> Result<UnityClassObject, String> {
        match object_info.class_id as i32 {
            1 => Self::parse_game_object(serialized_file, object_info)
                .map(UnityClassObject::GameObject),
            4 => {
                Self::parse_transform(serialized_file, object_info).map(UnityClassObject::Transform)
            }
            21 => {
                Self::parse_material(serialized_file, object_info).map(UnityClassObject::Material)
            }
            33 => Self::parse_mesh_filter(serialized_file, object_info)
                .map(UnityClassObject::MeshFilter),
            23 | 119 => {
                Self::parse_renderer(serialized_file, object_info).map(UnityClassObject::Renderer)
            }
            28 | 271 => Self::parse_texture2d(serialized_file, object_info, resources)
                .map(UnityClassObject::Texture2D),
            34 | 43 => Self::parse_mesh(serialized_file, object_info).map(UnityClassObject::Mesh),
            137 => Self::parse_skinned_mesh_renderer(serialized_file, object_info)
                .map(UnityClassObject::SkinnedMeshRenderer),
            199 => Self::parse_particle_system_renderer(serialized_file, object_info)
                .map(UnityClassObject::ParticleSystemRenderer),
            142 => Self::parse_asset_bundle(serialized_file, object_info)
                .map(UnityClassObject::AssetBundle),
            91 => Self::parse_animator_controller(serialized_file, object_info)
                .map(UnityClassObject::AnimatorController),
            95 => {
                Self::parse_animator(serialized_file, object_info).map(UnityClassObject::Animator)
            }
            114 => {
                Self::parse_component(serialized_file, object_info).map(UnityClassObject::Component)
            }
            111 => {
                Self::parse_animation(serialized_file, object_info).map(UnityClassObject::Animation)
            }
            221 => Self::parse_animator_override_controller(serialized_file, object_info)
                .map(UnityClassObject::AnimatorOverrideController),
            213 | 264 => {
                Self::parse_sprite(serialized_file, object_info).map(UnityClassObject::Sprite)
            }
            265 | 687078895 => Self::parse_sprite_atlas(serialized_file, object_info)
                .map(UnityClassObject::SpriteAtlas),
            331 => Self::parse_sprite_mask(serialized_file, object_info)
                .map(UnityClassObject::SpriteMask),
            id if Self::is_component_class(id) => {
                Self::parse_component(serialized_file, object_info).map(UnityClassObject::Component)
            }
            _ => Self::parse_raw(serialized_file, object_info).map(UnityClassObject::Raw),
        }
    }

    pub fn parse_game_object(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<GameObjectObject, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        Self::read_editor_extension_base(&mut reader);
        let component_count = reader.read_i32();
        if component_count < 0 || component_count > 1_000_000 {
            return Err(format!(
                "Invalid GameObject component count: {}",
                component_count
            ));
        }
        let mut components = Vec::with_capacity(component_count as usize);
        for _ in 0..component_count {
            if (reader.version[0] == 5 && reader.version[1] < 5) || reader.version[0] < 5 {
                reader.read_i32();
            }
            let component = Self::read_pptr(&mut reader);
            if !component.is_null() {
                components.push(component);
            }
        }
        reader.read_i32();
        let name = reader.read_aligned_string();
        Ok(GameObjectObject {
            header: Self::header(serialized_file, object_info),
            name,
            components,
        })
    }

    pub fn parse_component(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<ComponentObject, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        Ok(ComponentObject {
            header: Self::header(serialized_file, object_info),
            game_object: Self::read_component_game_object(&mut reader),
        })
    }

    #[allow(dead_code)]
    pub fn parse_mono_behaviour(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<MonoBehaviourObject, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        let component =
            Self::read_behaviour_component(&mut reader, Self::header(serialized_file, object_info));
        let script = Self::read_pptr(&mut reader);
        let name = reader.read_aligned_string();
        Ok(MonoBehaviourObject {
            component,
            script,
            name,
        })
    }

    pub fn parse_mesh_filter(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<MeshFilterObject, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        Self::read_editor_extension_base(&mut reader);
        let (game_object, mesh) = Self::read_mesh_filter_body(&mut reader);
        Ok(MeshFilterObject {
            component: ComponentObject {
                header: Self::header(serialized_file, object_info),
                game_object,
            },
            mesh,
        })
    }

    /// 读取 MeshFilter 的核心字段。
    ///
    /// 布局参考 AssetStudio `MeshFilter.cs`：Component 基类先读 `m_GameObject`，
    /// MeshFilter 自身随后只包含一个 `m_Mesh` 引用。
    fn read_mesh_filter_body(reader: &mut ObjectReader) -> (PPtr, Option<PPtr>) {
        let game_object = Self::read_pptr(reader);
        let mesh = Self::read_pptr(reader);
        (game_object, (!mesh.is_null()).then_some(mesh))
    }

    pub fn parse_transform(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<TransformObject, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        let game_object = Self::read_component_game_object(&mut reader);
        let local_rotation = [
            reader.read_f32(),
            reader.read_f32(),
            reader.read_f32(),
            reader.read_f32(),
        ];
        let local_position = [reader.read_f32(), reader.read_f32(), reader.read_f32()];
        let local_scale = [reader.read_f32(), reader.read_f32(), reader.read_f32()];
        let child_count = reader.read_i32();
        if child_count < 0 || child_count > 1_000_000 {
            return Err(format!("Invalid Transform child count: {}", child_count));
        }
        let mut children = Vec::with_capacity(child_count as usize);
        for _ in 0..child_count {
            let child = Self::read_pptr(&mut reader);
            if !child.is_null() {
                children.push(child);
            }
        }
        let father = Self::read_pptr(&mut reader);
        Ok(TransformObject {
            header: Self::header(serialized_file, object_info),
            game_object,
            local_rotation,
            local_position,
            local_scale,
            children,
            father,
        })
    }

    pub fn parse_renderer(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<RendererObject, String> {
        let header = Self::header(serialized_file, object_info);
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        let game_object = Self::read_component_game_object(&mut reader);
        if let Ok(materials) = Self::read_renderer_materials(&mut reader) {
            if !materials.is_empty() {
                return Ok(RendererObject {
                    header,
                    game_object,
                    mesh: None,
                    materials,
                });
            }

            let value = Self::read_typetree_value(serialized_file, object_info);
            if let Some(value) = value.as_ref() {
                let typetree_materials = Self::find_ptr_array_field(value, "m_Materials");
                if !typetree_materials.is_empty() {
                    return Ok(RendererObject {
                        header,
                        game_object,
                        mesh: Self::find_ptr_field(value, "m_Mesh"),
                        materials: typetree_materials,
                    });
                }
            }

            return Ok(RendererObject {
                header,
                game_object,
                mesh: None,
                materials,
            });
        }

        let value = Self::read_typetree_value(serialized_file, object_info);
        let mesh = value
            .as_ref()
            .and_then(|value| Self::find_ptr_field(value, "m_Mesh"));
        let materials = value
            .as_ref()
            .map(|value| Self::find_ptr_array_field(value, "m_Materials"))
            .unwrap_or_default();
        Ok(RendererObject {
            header,
            game_object,
            mesh,
            materials,
        })
    }

    pub fn parse_skinned_mesh_renderer(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<SkinnedMeshRendererObject, String> {
        let header = Self::header(serialized_file, object_info);
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        let game_object = Self::read_component_game_object(&mut reader);
        if let Ok(materials) = Self::read_renderer_materials(&mut reader) {
            if let Ok((mesh, bones)) = Self::read_skinned_mesh_renderer_tail(&mut reader) {
                return Ok(SkinnedMeshRendererObject {
                    renderer: RendererObject {
                        header,
                        game_object,
                        mesh,
                        materials,
                    },
                    bones,
                    root_bone: None,
                });
            }
        }

        let renderer = Self::parse_renderer(serialized_file, object_info)?;
        let value = Self::read_typetree_value(serialized_file, object_info);
        let bones = value
            .as_ref()
            .map(|value| Self::find_ptr_array_field(value, "m_Bones"))
            .unwrap_or_default();
        let root_bone = value
            .as_ref()
            .and_then(|value| Self::find_ptr_field(value, "m_RootBone"));
        Ok(SkinnedMeshRendererObject {
            renderer,
            bones,
            root_bone,
        })
    }

    /// Parse a ParticleSystemRenderer (class 199).
    ///
    /// The Renderer base layout is shared with MeshRenderer, so the manual
    /// reader handles m_GameObject/m_Materials; m_Mesh and m_RenderMode only
    /// exist on the particle tail and must come from the TypeTree.
    pub fn parse_particle_system_renderer(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<ParticleSystemRendererObject, String> {
        let mut renderer = Self::parse_renderer(serialized_file, object_info)?;
        let value = Self::read_typetree_value(serialized_file, object_info);
        let mut render_mode = None;
        if let Some(value) = value.as_ref() {
            if renderer.mesh.is_none() {
                renderer.mesh = Self::find_ptr_field(value, "m_Mesh");
            }
            render_mode = Self::find_int_field(value, "m_RenderMode");
        }
        Ok(ParticleSystemRendererObject {
            renderer,
            render_mode,
        })
    }

    /// 读取 Renderer 基类里导出依赖需要的 Material PPtr 数组。
    ///
    /// 读取顺序参考 AssetStudio `Renderer.cs`；如果版本分支或字段错位导致数量异常，
    /// 调用方会回退到 TypeTree 路径。
    fn read_renderer_materials(reader: &mut ObjectReader) -> Result<Vec<PPtr>, String> {
        let unity_version = reader.version;
        if unity_version[0] < 5 {
            reader.skip(4);
        } else if unity_version[0] > 5 || (unity_version[0] == 5 && unity_version[1] >= 4) {
            let renderer_flag_count = Self::modern_renderer_flag_count(reader);
            reader.skip(renderer_flag_count);
            reader.align();
            if unity_version[0] >= 2018 {
                reader.skip(4);
            }
            if unity_version[0] > 2018 || (unity_version[0] == 2018 && unity_version[1] >= 3) {
                reader.skip(4);
            }
            reader.skip(4);
        } else {
            let _enabled = reader.read_bool();
            reader.align();
            let _cast_shadows = reader.read_u8();
            let _receive_shadows = reader.read_bool();
            reader.align();
            reader.skip(4);
        }

        if unity_version[0] >= 3 {
            reader.skip(16);
        }
        if unity_version[0] >= 5 {
            reader.skip(16);
        }

        let material_count = Self::read_sane_count(reader, "m_Materials")?;
        let mut materials = Vec::with_capacity(material_count);
        for _ in 0..material_count {
            let material = Self::read_pptr(reader);
            if !material.is_null() {
                materials.push(material);
            }
        }

        if unity_version[0] < 3 {
            reader.skip(16);
        } else {
            if unity_version[0] > 5 || (unity_version[0] == 5 && unity_version[1] >= 5) {
                reader.skip(4);
            } else {
                let subset_count = Self::read_sane_count(reader, "m_SubsetIndices")?;
                reader.skip(subset_count.saturating_mul(4));
            }
            let _static_batch_root = Self::read_pptr(reader);
        }

        if unity_version[0] > 5 || (unity_version[0] == 5 && unity_version[1] >= 4) {
            let _probe_anchor = Self::read_pptr(reader);
            let _light_probe_volume_override = Self::read_pptr(reader);
        } else if unity_version[0] > 3 || (unity_version[0] == 3 && unity_version[1] >= 5) {
            let _use_light_probes = reader.read_bool();
            reader.align();
            if unity_version[0] >= 5 {
                reader.skip(4);
            }
            let _light_probe_anchor = Self::read_pptr(reader);
        }

        if unity_version[0] > 4 || (unity_version[0] == 4 && unity_version[1] >= 3) {
            if unity_version[0] == 4 && unity_version[1] == 3 {
                reader.skip(2);
            } else {
                reader.skip(4);
            }
            reader.skip(2);
            reader.align();
        }

        Ok(materials)
    }

    /// 读取 SkinnedMeshRenderer 自有字段里导出依赖需要的 Mesh/Bones。
    fn read_skinned_mesh_renderer_tail(
        reader: &mut ObjectReader,
    ) -> Result<(Option<PPtr>, Vec<PPtr>), String> {
        reader.skip(4);
        let _update_when_offscreen = reader.read_bool();
        let _skin_normals = reader.read_bool();
        reader.align();

        if reader.version[0] == 2 && reader.version[1] < 6 {
            let _disable_animation_when_offscreen = Self::read_pptr(reader);
        }

        let mesh = Self::read_pptr(reader);
        let bone_count = Self::read_sane_count(reader, "m_Bones")?;
        let mut bones = Vec::with_capacity(bone_count);
        for _ in 0..bone_count {
            let bone = Self::read_pptr(reader);
            if !bone.is_null() {
                bones.push(bone);
            }
        }

        if reader.version[0] > 4 || (reader.version[0] == 4 && reader.version[1] >= 3) {
            let blend_shape_weight_count = Self::read_sane_count(reader, "m_BlendShapeWeights")?;
            reader.skip(blend_shape_weight_count.saturating_mul(4));
        }

        Ok(((!mesh.is_null()).then_some(mesh), bones))
    }

    /// 计算 Unity 5.4+ Renderer flag 字节数。
    fn modern_renderer_flag_count(reader: &ObjectReader) -> usize {
        let unity_version = reader.version;
        let mut count = 6usize;
        if unity_version[0] > 2017 || (unity_version[0] == 2017 && unity_version[1] >= 2) {
            count += 1;
        }
        if unity_version[0] >= 2021 {
            count += 1;
        }
        if unity_version[0] > 2019 || (unity_version[0] == 2019 && unity_version[1] >= 3) {
            count += 1;
        }
        if unity_version[0] >= 2020 {
            count += 1;
        }
        count
    }

    pub fn parse_material(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<MaterialObject, String> {
        Material::read(
            serialized_file,
            object_info,
            Self::header(serialized_file, object_info),
        )
    }

    pub fn parse_animator(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<AnimatorObject, String> {
        if let Ok(animator) = Self::parse_animator_raw(serialized_file, object_info) {
            return Ok(animator);
        }

        Self::parse_animator_typetree(serialized_file, object_info)
            .ok_or_else(|| "Animator parser failed and TypeTree fallback is unavailable".to_string())
    }

    fn parse_animator_raw(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<AnimatorObject, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        let component = Self::try_read_behaviour_component(
            &mut reader,
            Self::header(serialized_file, object_info),
        )?;
        let avatar = Self::try_read_pptr(&mut reader)?;
        let controller = Self::try_read_pptr(&mut reader)?;
        Self::read_animator_tail(&mut reader)?;
        Ok(AnimatorObject {
            component,
            avatar,
            controller,
        })
    }

    fn parse_animator_typetree(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Option<AnimatorObject> {
        let value = Self::read_typetree_value(serialized_file, object_info)?;
        Some(AnimatorObject {
            component: ComponentObject {
                header: Self::header(serialized_file, object_info),
                game_object: Self::find_ptr_field(&value, "m_GameObject")?,
            },
            avatar: Self::find_ptr_field(&value, "m_Avatar").unwrap_or(PPtr {
                file_id: 0,
                path_id: 0,
            }),
            controller: Self::find_ptr_field(&value, "m_Controller").unwrap_or(PPtr {
                file_id: 0,
                path_id: 0,
            }),
        })
    }

    fn read_animator_tail(reader: &mut ObjectReader) -> Result<(), String> {
        let version = reader.version;
        let _culling_mode = reader.try_read_i32()?;

        if version[0] > 4 || (version[0] == 4 && version[1] >= 5) {
            let _update_mode = reader.try_read_i32()?;
        }

        let _apply_root_motion = reader.try_read_bool()?;
        if version[0] == 4 && version[1] >= 5 {
            reader.align();
        }

        if version[0] >= 5 {
            let _linear_velocity_blending = reader.try_read_bool()?;
            if version[0] > 2021 || (version[0] == 2021 && version[1] >= 2) {
                let _stabilize_feet = reader.try_read_bool()?;
            }
            reader.align();
        }

        if version[0] < 4 || (version[0] == 4 && version[1] < 5) {
            let _animate_physics = reader.try_read_bool()?;
        }

        if version[0] > 4 || (version[0] == 4 && version[1] >= 3) {
            let _has_transform_hierarchy = reader.try_read_bool()?;
        }

        if version[0] > 4 || (version[0] == 4 && version[1] >= 5) {
            let _allow_constant_clip_sampling_optimization = reader.try_read_bool()?;
        }

        if version[0] >= 5 && version[0] < 2018 {
            reader.align();
        }

        if version[0] >= 2018 {
            let _keep_animator_controller_state_on_disable = reader.try_read_bool()?;
            reader.align();
        }

        Ok(())
    }

    pub fn parse_animation(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<AnimationObject, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        let component =
            Self::read_behaviour_component(&mut reader, Self::header(serialized_file, object_info));
        let default_animation = Self::read_pptr(&mut reader);
        let animation_count = Self::read_sane_count(&mut reader, "m_Animations")?;
        let mut animations = Vec::with_capacity(animation_count);
        for _ in 0..animation_count {
            let animation = Self::read_pptr(&mut reader);
            if !animation.is_null() {
                animations.push(animation);
            }
        }
        Ok(AnimationObject {
            component,
            default_animation,
            animations,
        })
    }

    pub fn parse_animator_override_controller(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<AnimatorOverrideControllerObject, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        let name = Self::read_named_object_name(&mut reader);
        let controller = Self::read_pptr(&mut reader);
        let clip_count = Self::read_sane_count(&mut reader, "m_Clips")?;
        let mut clips = Vec::with_capacity(clip_count);
        for _ in 0..clip_count {
            clips.push(AnimatorOverrideClip {
                original_clip: Self::read_pptr(&mut reader),
                override_clip: Self::read_pptr(&mut reader),
            });
        }
        Ok(AnimatorOverrideControllerObject {
            header: Self::header(serialized_file, object_info),
            name,
            controller,
            clips,
        })
    }

    pub fn parse_animator_controller(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<AnimatorControllerObject, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        let name = Self::read_named_object_name(&mut reader);
        let animation_clips = Self::read_animator_controller_tail(&mut reader)?;
        Ok(AnimatorControllerObject {
            header: Self::header(serialized_file, object_info),
            name,
            animation_clips,
        })
    }

    fn read_animator_controller_tail(reader: &mut ObjectReader) -> Result<Vec<PPtr>, String> {
        let controller_size = reader.read_u32() as usize;
        if controller_size > reader.remaining() {
            return Err(format!(
                "AnimatorController controller blob size {} exceeds remaining {} bytes",
                controller_size,
                reader.remaining()
            ));
        }
        reader.skip(controller_size);

        let tos_count = Self::read_sane_count(reader, "m_TOS")?;
        for _ in 0..tos_count {
            let _key = reader.read_u32();
            let _value = reader.read_aligned_string();
        }

        let clip_count = Self::read_sane_count(reader, "m_AnimationClips")?;
        let mut animation_clips = Vec::with_capacity(clip_count);
        for _ in 0..clip_count {
            let animation_clip = Self::read_pptr(reader);
            if !animation_clip.is_null() {
                animation_clips.push(animation_clip);
            }
        }
        Ok(animation_clips)
    }

    pub fn parse_asset_bundle(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<AssetBundleObject, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        let name = Self::read_named_object_name(&mut reader);

        let preload_count = reader.read_i32();
        if preload_count < 0 || preload_count > 1_000_000 {
            return Err(format!(
                "Invalid AssetBundle preload count: {}",
                preload_count
            ));
        }
        let mut preload_table = Vec::with_capacity(preload_count as usize);
        for _ in 0..preload_count {
            preload_table.push(Self::read_pptr(&mut reader));
        }

        let container_count = reader.read_i32();
        if container_count < 0 || container_count > 1_000_000 {
            return Err(format!(
                "Invalid AssetBundle container count: {}",
                container_count
            ));
        }
        let mut container = Vec::with_capacity(container_count as usize);
        for _ in 0..container_count {
            let asset_path = reader.read_aligned_string();
            let preload_index = reader.read_i32();
            let preload_size = reader.read_i32();
            let asset = Self::read_pptr(&mut reader);
            container.push(AssetBundleContainerEntry {
                asset_path,
                preload_index,
                preload_size,
                asset,
            });
        }

        Ok(AssetBundleObject {
            header: Self::header(serialized_file, object_info),
            name,
            preload_table,
            container,
        })
    }

    pub fn parse_texture2d(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
        resources: &HashMap<String, ResS>,
    ) -> Result<Texture2DObject, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        let data = TextureReader::read_texture(&mut reader, resources)?;
        Ok(Texture2DObject {
            header: Self::header(serialized_file, object_info),
            data,
        })
    }

    pub fn parse_mesh(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<MeshObject, String> {
        Mesh::read(
            serialized_file,
            object_info,
            Self::header(serialized_file, object_info),
        )
    }

    pub fn parse_sprite_mask(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<SpriteMaskObject, String> {
        SpriteMaskObject::read(
            serialized_file,
            object_info,
            Self::header(serialized_file, object_info),
        )
    }

    pub fn parse_sprite(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<SpriteObject, String> {
        SpriteObject::read(
            serialized_file,
            object_info,
            Self::header(serialized_file, object_info),
        )
    }

    pub fn parse_sprite_atlas(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<SpriteAtlasObject, String> {
        SpriteAtlasObject::read(
            serialized_file,
            object_info,
            Self::header(serialized_file, object_info),
        )
    }

    pub fn parse_raw(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<RawUnityObject, String> {
        Ok(RawUnityObject {
            header: Self::header(serialized_file, object_info),
            bytes: serialized_file.object_bytes(object_info)?.to_vec(),
        })
    }

    fn header(serialized_file: &SerializedFile, object_info: &ObjectInfo) -> UnityObjectHeader {
        let class_id = object_info.class_id as i32;
        UnityObjectHeader {
            path_id: object_info.path_id,
            class_id,
            class_name: ClassNameUtils::get_class_name(class_id)
                .unwrap_or("Unknown")
                .to_string(),
            byte_size: object_info.byte_size,
            unity_version: serialized_file.unity_version.clone(),
        }
    }

    fn read_pptr(reader: &mut ObjectReader) -> PPtr {
        PPtr {
            file_id: reader.read_i32(),
            path_id: reader.read_path_id(),
        }
    }

    fn try_read_pptr(reader: &mut ObjectReader) -> Result<PPtr, String> {
        Ok(PPtr {
            file_id: reader.try_read_i32()?,
            path_id: if reader.header_version < 14 {
                reader.try_read_i32()? as i64
            } else {
                reader.try_read_i64()?
            },
        })
    }

    fn read_editor_extension_base(reader: &mut ObjectReader) {
        if reader.target_platform != -2 {
            return;
        }

        let _object_hide_flags = reader.read_u32();
        let _prefab_parent = Self::read_pptr(reader);
        let _prefab_internal = Self::read_pptr(reader);
    }

    fn read_named_object_name(reader: &mut ObjectReader) -> String {
        Self::read_editor_extension_base(reader);
        reader.read_aligned_string()
    }

    fn read_component_game_object(reader: &mut ObjectReader) -> PPtr {
        Self::read_editor_extension_base(reader);
        Self::read_pptr(reader)
    }

    fn read_behaviour_component(
        reader: &mut ObjectReader,
        header: UnityObjectHeader,
    ) -> ComponentObject {
        let game_object = Self::read_component_game_object(reader);
        let _enabled = reader.read_u8();
        reader.align();
        ComponentObject {
            header,
            game_object,
        }
    }

    fn try_read_behaviour_component(
        reader: &mut ObjectReader,
        header: UnityObjectHeader,
    ) -> Result<ComponentObject, String> {
        Self::try_read_editor_extension_base(reader)?;
        let game_object = Self::try_read_pptr(reader)?;
        let _enabled = reader.try_read_bool()?;
        reader.align();
        Ok(ComponentObject {
            header,
            game_object,
        })
    }

    fn try_read_editor_extension_base(reader: &mut ObjectReader) -> Result<(), String> {
        if reader.target_platform != -2 {
            return Ok(());
        }

        let _object_hide_flags = reader.try_read_u32()?;
        let _prefab_parent = Self::try_read_pptr(reader)?;
        let _prefab_internal = Self::try_read_pptr(reader)?;
        Ok(())
    }

    fn read_sane_count(reader: &mut ObjectReader, field_name: &str) -> Result<usize, String> {
        let count = reader.read_i32();
        if !(0..=1_000_000).contains(&count) {
            return Err(format!("Invalid {} count: {}", field_name, count));
        }
        Ok(count as usize)
    }

    fn read_typetree_value(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Option<UnityValue> {
        let type_tree = serialized_file
            .object_serialized_type(object_info)?
            .type_tree
            .clone();
        if type_tree.nodes.is_empty() {
            return None;
        }
        let raw = serialized_file.object_bytes(object_info).ok()?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        TypeTreeReaderUtils::read_typetree_value(&mut reader, &type_tree, serialized_file).ok()
    }

    pub fn read_typetree_value_public(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Option<UnityValue> {
        Self::read_typetree_value(serialized_file, object_info)
    }

    fn find_int_field(value: &UnityValue, field: &str) -> Option<i64> {
        match value {
            UnityValue::Object(map) => {
                if let Some(UnityValue::Integer(v)) = map.get(field) {
                    return Some(*v);
                }
                map.values()
                    .find_map(|child| Self::find_int_field(child, field))
            }
            UnityValue::Array(items) => items
                .iter()
                .find_map(|child| Self::find_int_field(child, field)),
            _ => None,
        }
    }

    fn find_ptr_field(value: &UnityValue, field: &str) -> Option<PPtr> {        match value {
            UnityValue::Object(map) => {
                if let Some(UnityValue::Ptr { file_id, path_id }) = map.get(field) {
                    if *path_id != 0 {
                        return Some(PPtr {
                            file_id: *file_id,
                            path_id: *path_id,
                        });
                    }
                }
                map.values()
                    .find_map(|child| Self::find_ptr_field(child, field))
            }
            UnityValue::Array(items) => items
                .iter()
                .find_map(|child| Self::find_ptr_field(child, field)),
            _ => None,
        }
    }

    fn find_ptr_array_field(value: &UnityValue, field: &str) -> Vec<PPtr> {
        match value {
            UnityValue::Object(map) => {
                if let Some(found) = map.get(field) {
                    let refs = Self::ptrs_from_value(found);
                    if !refs.is_empty() {
                        return refs;
                    }
                }
                for child in map.values() {
                    let refs = Self::find_ptr_array_field(child, field);
                    if !refs.is_empty() {
                        return refs;
                    }
                }
                Vec::new()
            }
            UnityValue::Array(items) => {
                for child in items {
                    let refs = Self::find_ptr_array_field(child, field);
                    if !refs.is_empty() {
                        return refs;
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn ptrs_from_value(value: &UnityValue) -> Vec<PPtr> {
        match value {
            UnityValue::Ptr { file_id, path_id } => {
                if *path_id != 0 {
                    vec![PPtr {
                        file_id: *file_id,
                        path_id: *path_id,
                    }]
                } else {
                    Vec::new()
                }
            }
            UnityValue::Array(items) => items.iter().flat_map(Self::ptrs_from_value).collect(),
            UnityValue::Object(map) => {
                if let Some(UnityValue::Array(items)) = map.get("Array") {
                    items.iter().flat_map(Self::ptrs_from_value).collect()
                } else {
                    map.values().flat_map(Self::ptrs_from_value).collect()
                }
            }
            _ => Vec::new(),
        }
    }

    fn is_component_class(class_id: i32) -> bool {
        matches!(
            class_id,
            2 | 8
                | 20
                | 23
                | 25
                | 26
                | 31
                | 33
                | 41
                | 45
                | 53
                | 56
                | 64
                | 65
                | 81
                | 95
                | 114
                | 119
                | 120
                | 137
                | 141
                | 143
                | 146
                | 180
                | 198
                | 199
                | 212
                | 222
                | 223
                | 224
                | 233
                | 234
                | 246
                | 255
                | 108
                | 259
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::common::serialized_file::object_reader::ObjectReader;
    use crate::unity::classes::registry::UnityClassParser;

    #[test]
    fn animator_controller_parser_reads_animation_clip_references_after_controller_blob() {
        let data = animator_controller_bytes_for_test();
        let mut reader = ObjectReader {
            data: &data,
            pos: 0,
            version: [2019, 4, 29, 1],
            endian: 0,
            header_version: 22,
            target_platform: 0,
        };

        let name = UnityClassParser::read_named_object_name(&mut reader);
        let animation_clips = UnityClassParser::read_animator_controller_tail(&mut reader)
            .expect("animator controller");

        assert_eq!(name, "controller");
        assert_eq!(animation_clips.len(), 2);
        assert_eq!(animation_clips[0].file_id, 0);
        assert_eq!(animation_clips[0].path_id, 101);
        assert_eq!(animation_clips[1].file_id, 2);
        assert_eq!(animation_clips[1].path_id, 202);
    }

    #[test]
    fn notarget_component_base_skips_object_and_editor_extension_fields() {
        let data = notarget_component_bytes_for_test(700);
        let mut reader = ObjectReader {
            data: &data,
            pos: 0,
            version: [2019, 4, 29, 1],
            endian: 0,
            header_version: 22,
            target_platform: -2,
        };

        let game_object = UnityClassParser::read_component_game_object(&mut reader);

        assert_eq!(game_object.file_id, 0);
        assert_eq!(game_object.path_id, 700);
        assert_eq!(reader.pos, 40);
    }

    #[test]
    fn notarget_animator_body_reads_game_object_after_base_and_behaviour() {
        let mut data = notarget_component_bytes_for_test(700);
        data.push(1);
        while data.len() % 4 != 0 {
            data.push(0);
        }
        data.extend_from_slice(&0i32.to_le_bytes());
        data.extend_from_slice(&800i64.to_le_bytes());
        data.extend_from_slice(&0i32.to_le_bytes());
        data.extend_from_slice(&900i64.to_le_bytes());
        let mut reader = ObjectReader {
            data: &data,
            pos: 0,
            version: [2019, 4, 29, 1],
            endian: 0,
            header_version: 22,
            target_platform: -2,
        };

        let component = UnityClassParser::read_behaviour_component(
            &mut reader,
            crate::unity::classes::object::UnityObjectHeader {
                path_id: 95,
                class_id: 95,
                class_name: "Animator".to_string(),
                byte_size: data.len() as u32,
                unity_version: "2019.4.29f1".to_string(),
            },
        );
        let avatar = UnityClassParser::read_pptr(&mut reader);
        let controller = UnityClassParser::read_pptr(&mut reader);

        assert_eq!(component.game_object.path_id, 700);
        assert_eq!(avatar.path_id, 800);
        assert_eq!(controller.path_id, 900);
    }

    #[test]
    fn animator_parser_consumes_post_controller_flags_for_modern_versions() {
        let mut data = notarget_component_bytes_for_test(700);
        data.push(1);
        while data.len() % 4 != 0 {
            data.push(0);
        }
        data.extend_from_slice(&0i32.to_le_bytes());
        data.extend_from_slice(&800i64.to_le_bytes());
        data.extend_from_slice(&0i32.to_le_bytes());
        data.extend_from_slice(&900i64.to_le_bytes());
        data.extend_from_slice(&2i32.to_le_bytes());
        data.extend_from_slice(&3i32.to_le_bytes());
        data.push(1);
        data.push(1);
        data.push(1);
        data.push(1);
        data.push(1);
        data.push(1);
        data.push(1);
        data.push(1);

        let mut reader = ObjectReader {
            data: &data,
            pos: 0,
            version: [2019, 4, 29, 1],
            endian: 0,
            header_version: 22,
            target_platform: -2,
        };
        let component = UnityClassParser::try_read_behaviour_component(
            &mut reader,
            crate::unity::classes::object::UnityObjectHeader {
                path_id: 95,
                class_id: 95,
                class_name: "Animator".to_string(),
                byte_size: data.len() as u32,
                unity_version: "2019.4.29f1".to_string(),
            },
        );
        assert!(component.is_ok());
        let avatar = UnityClassParser::try_read_pptr(&mut reader).unwrap();
        let controller = UnityClassParser::try_read_pptr(&mut reader).unwrap();
        UnityClassParser::read_animator_tail(&mut reader).unwrap();

        assert_eq!(component.unwrap().game_object.path_id, 700);
        assert_eq!(avatar.path_id, 800);
        assert_eq!(controller.path_id, 900);
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn notarget_mono_behaviour_body_reads_script_after_behaviour_base() {
        let mut data = notarget_component_bytes_for_test(700);
        data.push(1);
        while data.len() % 4 != 0 {
            data.push(0);
        }
        data.extend_from_slice(&0i32.to_le_bytes());
        data.extend_from_slice(&900i64.to_le_bytes());
        push_aligned_string(&mut data, "driver");
        let mut reader = ObjectReader {
            data: &data,
            pos: 0,
            version: [2019, 4, 29, 1],
            endian: 0,
            header_version: 22,
            target_platform: -2,
        };

        let component = UnityClassParser::read_behaviour_component(
            &mut reader,
            crate::unity::classes::object::UnityObjectHeader {
                path_id: 114,
                class_id: 114,
                class_name: "MonoBehaviour".to_string(),
                byte_size: data.len() as u32,
                unity_version: "2019.4.29f1".to_string(),
            },
        );
        let script = UnityClassParser::read_pptr(&mut reader);
        let name = reader.read_aligned_string();

        assert_eq!(component.game_object.path_id, 700);
        assert_eq!(script.file_id, 0);
        assert_eq!(script.path_id, 900);
        assert_eq!(name, "driver");
    }

    #[test]
    fn renderer_parser_reads_materials_after_unity_2019_flags() {
        let data = renderer_body_bytes_for_test();
        let mut reader = ObjectReader {
            data: &data,
            pos: 12,
            version: [2019, 4, 29, 1],
            endian: 0,
            header_version: 22,
            target_platform: 0,
        };

        let materials =
            UnityClassParser::read_renderer_materials(&mut reader).expect("renderer materials");

        assert_eq!(materials.len(), 2);
        assert_eq!(materials[0].file_id, 0);
        assert_eq!(materials[0].path_id, 11);
        assert_eq!(materials[1].file_id, 3);
        assert_eq!(materials[1].path_id, 22);
    }

    fn renderer_body_bytes_for_test() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&777i64.to_le_bytes());
        bytes.extend_from_slice(&[1, 1, 1, 1, 1, 1, 1, 1]);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 16]);
        bytes.extend_from_slice(&[0u8; 16]);
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&11i64.to_le_bytes());
        bytes.extend_from_slice(&3i32.to_le_bytes());
        bytes.extend_from_slice(&22i64.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i64.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i64.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i64.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0i16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes
    }

    #[test]
    fn mesh_filter_parser_reads_mesh_after_component_game_object() {
        let data = mesh_filter_body_bytes_for_test();
        let mut reader = ObjectReader {
            data: &data,
            pos: 0,
            version: [2019, 4, 29, 1],
            endian: 0,
            header_version: 22,
            target_platform: 0,
        };

        let (game_object, mesh) = UnityClassParser::read_mesh_filter_body(&mut reader);
        let mesh = mesh.expect("mesh reference");

        assert_eq!(game_object.file_id, 0);
        assert_eq!(game_object.path_id, 700);
        assert_eq!(mesh.file_id, 0);
        assert_eq!(mesh.path_id, 900);
    }

    fn mesh_filter_body_bytes_for_test() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&700i64.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&900i64.to_le_bytes());
        bytes
    }

    fn notarget_component_bytes_for_test(game_object_path_id: i64) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        push_pptr(&mut bytes, 0, 0);
        push_pptr(&mut bytes, 0, 0);
        push_pptr(&mut bytes, 0, game_object_path_id);
        bytes
    }

    fn animator_controller_bytes_for_test() -> Vec<u8> {
        let mut bytes = Vec::new();
        push_aligned_string(&mut bytes, "controller");
        bytes.extend_from_slice(&8u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&123u32.to_le_bytes());
        push_aligned_string(&mut bytes, "Idle");
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&101i64.to_le_bytes());
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes.extend_from_slice(&202i64.to_le_bytes());
        bytes
    }

    fn push_aligned_string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as i32).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
    }

    fn push_pptr(bytes: &mut Vec<u8>, file_id: i32, path_id: i64) {
        bytes.extend_from_slice(&file_id.to_le_bytes());
        bytes.extend_from_slice(&path_id.to_le_bytes());
    }
}
