use crate::common::serialized_file::object_reader::ObjectReader;
use crate::common::serialized_file::serialized_file::{ObjectInfo, SerializedFile};
use crate::unity::classes::object::{PPtr, UnityObjectHeader};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MaterialObject {
    pub header: UnityObjectHeader,
    pub name: String,
    pub texture_refs: Vec<PPtr>,
    pub texture_slots: Vec<MaterialTextureSlot>,
    pub diffuse_color: [f32; 4],
    pub specular_color: [f32; 4],
    pub emissive_color: [f32; 4],
    pub shininess: f32,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MaterialTextureSlot {
    pub slot_name: String,
    pub texture: PPtr,
}

pub struct Material;

impl Material {
    pub fn read(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
        header: UnityObjectHeader,
    ) -> Result<MaterialObject, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        let mut reader = ObjectReader::new(raw, serialized_file);
        let parsed_material = Self::read_assetstudio_layout(&mut reader)?;
        Ok(MaterialObject {
            header,
            name: parsed_material.name,
            texture_refs: parsed_material
                .texture_slots
                .iter()
                .map(|slot| slot.texture)
                .collect(),
            texture_slots: parsed_material.texture_slots,
            diffuse_color: parsed_material.diffuse_color,
            specular_color: parsed_material.specular_color,
            emissive_color: parsed_material.emissive_color,
            shininess: parsed_material.shininess,
        })
    }

    /// 按 AssetStudio 的 Material.cs 顺序读取 Material。
    ///
    /// 这个路径不依赖 TypeTree，避免 blob TypeTree 字符串错位时丢失贴图引用。
    fn read_assetstudio_layout(reader: &mut ObjectReader) -> Result<ParsedMaterial, String> {
        let unity_version = reader.version;
        let is_notarget = reader.target_platform == -2;

        if is_notarget {
            let _object_hide_flags = reader.read_u32();
            Self::skip_pptr(reader);
            Self::skip_pptr(reader);
        }

        let name = reader.read_aligned_string();

        Self::skip_pptr(reader);
        if unity_version[0] == 4 && unity_version[1] >= 1 {
            Self::skip_string_array(reader, "m_ShaderKeywords")?;
        }
        if unity_version[0] > 2021 || (unity_version[0] == 2021 && unity_version[1] >= 3) {
            Self::skip_string_array(reader, "m_ValidKeywords")?;
            Self::skip_string_array(reader, "m_InvalidKeywords")?;
        } else if unity_version[0] >= 5 {
            let _shader_keywords = reader.read_aligned_string();
        }
        if unity_version[0] >= 5 {
            let _lightmap_flags = reader.read_u32();
        }
        if unity_version[0] > 5 || (unity_version[0] == 5 && unity_version[1] >= 6) {
            let _enable_instancing_variants = reader.read_bool();
            if unity_version[0] >= 2017 {
                let _double_sided_gi = reader.read_bool();
            }
            reader.align();
        }
        if unity_version[0] > 4 || (unity_version[0] == 4 && unity_version[1] >= 3) {
            let _custom_render_queue = reader.read_i32();
        }
        if unity_version[0] > 5 || (unity_version[0] == 5 && unity_version[1] >= 1) {
            let tag_count = Self::read_sane_count(reader, "stringTagMap")?;
            for _ in 0..tag_count {
                let _tag_name = reader.read_aligned_string();
                let _tag_value = reader.read_aligned_string();
            }
        }
        if unity_version[0] > 5 || (unity_version[0] == 5 && unity_version[1] >= 6) {
            Self::skip_string_array(reader, "disabledShaderPasses")?;
        }

        let mut parsed_material = Self::read_saved_properties(reader)?;
        parsed_material.name = name;
        Ok(parsed_material)
    }

    /// 读取 UnityPropertySheet，保留导出材质需要的贴图、颜色和高光参数。
    fn read_saved_properties(reader: &mut ObjectReader) -> Result<ParsedMaterial, String> {
        let mut parsed_material = ParsedMaterial::default();

        let texture_count = Self::read_sane_count(reader, "m_TexEnvs")?;
        for _ in 0..texture_count {
            let slot_name = reader.read_aligned_string();
            let texture = Self::read_pptr(reader);
            let _scale_x = reader.read_f32();
            let _scale_y = reader.read_f32();
            let _offset_x = reader.read_f32();
            let _offset_y = reader.read_f32();
            if !texture.is_null() {
                parsed_material
                    .texture_slots
                    .push(MaterialTextureSlot { slot_name, texture });
            }
        }

        if reader.version[0] >= 2021 {
            let int_count = Self::read_sane_count(reader, "m_Ints")?;
            for _ in 0..int_count {
                let _key = reader.read_aligned_string();
                let _value = reader.read_i32();
            }
        }

        let float_count = Self::read_sane_count(reader, "m_Floats")?;
        for _ in 0..float_count {
            let key = reader.read_aligned_string();
            let value = reader.read_f32();
            if key == "_Shininess" || key == "_Glossiness" {
                parsed_material.shininess = value;
            }
        }

        let color_count = Self::read_sane_count(reader, "m_Colors")?;
        for _ in 0..color_count {
            let key = reader.read_aligned_string();
            let color = [
                reader.read_f32(),
                reader.read_f32(),
                reader.read_f32(),
                reader.read_f32(),
            ];
            match key.as_str() {
                "_Color" => parsed_material.diffuse_color = color,
                "_SpecColor" => parsed_material.specular_color = color,
                "_EmissionColor" => parsed_material.emissive_color = color,
                _ => {}
            }
        }

        Ok(parsed_material)
    }

    /// 读取 PPtr 并返回 file_id/path_id。
    fn read_pptr(reader: &mut ObjectReader) -> PPtr {
        PPtr {
            file_id: reader.read_i32(),
            path_id: reader.read_path_id(),
        }
    }

    /// 跳过 PPtr 字段。
    fn skip_pptr(reader: &mut ObjectReader) {
        let _pptr = Self::read_pptr(reader);
    }

    /// 跳过 Unity 字符串数组。
    fn skip_string_array(reader: &mut ObjectReader, field_name: &str) -> Result<(), String> {
        let count = Self::read_sane_count(reader, field_name)?;
        for _ in 0..count {
            let _value = reader.read_aligned_string();
        }
        Ok(())
    }

    /// 读取数组长度并限制上限，防止错位读取后分配异常内存。
    fn read_sane_count(reader: &mut ObjectReader, field_name: &str) -> Result<usize, String> {
        let count = reader.read_i32();
        if !(0..=10_000).contains(&count) {
            return Err(format!("Invalid Material {} count: {}", field_name, count));
        }
        Ok(count as usize)
    }
}

#[derive(Debug, Clone)]
struct ParsedMaterial {
    name: String,
    texture_slots: Vec<MaterialTextureSlot>,
    diffuse_color: [f32; 4],
    specular_color: [f32; 4],
    emissive_color: [f32; 4],
    shininess: f32,
}

impl Default for ParsedMaterial {
    fn default() -> Self {
        Self {
            name: String::new(),
            texture_slots: Vec::new(),
            diffuse_color: [0.0, 0.0, 0.0, 0.0],
            specular_color: [0.0, 0.0, 0.0, 0.0],
            emissive_color: [0.0, 0.0, 0.0, 0.0],
            shininess: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assetstudio_layout_reads_notarget_material_inheritance_before_tex_envs() {
        let data = material_bytes_for_test();
        let mut reader = ObjectReader {
            data: &data,
            pos: 0,
            version: [2019, 4, 41, 2],
            endian: 0,
            header_version: 22,
            target_platform: -2,
        };

        let material = Material::read_assetstudio_layout(&mut reader).expect("material");

        assert_eq!(material.name, "female_Body");
        assert_eq!(material.texture_slots.len(), 2);
        assert_eq!(material.texture_slots[0].slot_name, "_BumpMap");
        assert_eq!(material.texture_slots[0].texture.file_id, 7);
        assert_eq!(material.texture_slots[0].texture.path_id, 101);
        assert_eq!(material.texture_slots[1].slot_name, "_MainTex");
        assert_eq!(material.texture_slots[1].texture.file_id, 8);
        assert_eq!(material.texture_slots[1].texture.path_id, 202);
    }

    #[test]
    fn naraka_material_texture_pptrs_stay_within_external_table_when_fixture_exists() {
        let fixture = std::path::Path::new(r"D:\NarakaAssets\0\0\000fe3c41993e1d3");
        if !fixture.exists() {
            return;
        }

        let bundle =
            crate::common::bundle_file::asset_bundle::AssetBundleLoader::load_bundle(fixture)
                .expect("bundle");
        let material_path_id = -148122273519210158i64;
        let (sf, obj) = bundle
            .assets
            .iter()
            .find_map(|sf| {
                sf.objects
                    .iter()
                    .find(|obj| obj.path_id == material_path_id && obj.class_id == 21)
                    .map(|obj| (sf, obj))
            })
            .expect("material object");
        let material = Material::read(
            &sf.inner,
            &obj.inner,
            UnityObjectHeader {
                path_id: obj.path_id,
                class_id: obj.class_id,
                class_name: "Material".to_string(),
                byte_size: obj.byte_size,
                unity_version: sf.unity_version.clone(),
            },
        )
        .expect("material parse");

        let external_count = sf.inner.m_externals.len();
        assert!(
            !material.texture_slots.is_empty(),
            "fixture material should expose at least one texture slot"
        );
        assert!(
            material
                .texture_slots
                .iter()
                .all(|slot| slot.texture.file_id <= 0
                    || slot.texture.file_id as usize <= external_count),
            "material texture refs {:?} exceed {} externals",
            material.texture_slots,
            external_count
        );
    }

    fn material_bytes_for_test() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u32.to_le_bytes()); // m_ObjectHideFlags
        push_pptr(&mut bytes, 0, 0); // m_PrefabParentObject
        push_pptr(&mut bytes, 0, 0); // m_PrefabInternal
        push_aligned_string(&mut bytes, "female_Body"); // m_Name
        push_pptr(&mut bytes, 0, 0); // m_Shader
        push_aligned_string(&mut bytes, ""); // m_ShaderKeywords
        bytes.extend_from_slice(&0u32.to_le_bytes()); // m_LightmapFlags
        bytes.push(0); // m_EnableInstancingVariants
        bytes.push(0); // m_DoubleSidedGI
        align(&mut bytes);
        bytes.extend_from_slice(&(-1i32).to_le_bytes()); // m_CustomRenderQueue
        bytes.extend_from_slice(&0i32.to_le_bytes()); // stringTagMap size
        bytes.extend_from_slice(&0i32.to_le_bytes()); // disabledShaderPasses size

        bytes.extend_from_slice(&2i32.to_le_bytes()); // m_TexEnvs size
        push_tex_env(&mut bytes, "_BumpMap", 7, 101);
        push_tex_env(&mut bytes, "_MainTex", 8, 202);
        bytes.extend_from_slice(&0i32.to_le_bytes()); // m_Floats size
        bytes.extend_from_slice(&0i32.to_le_bytes()); // m_Colors size
        bytes
    }

    fn push_tex_env(bytes: &mut Vec<u8>, slot_name: &str, file_id: i32, path_id: i64) {
        push_aligned_string(bytes, slot_name);
        push_pptr(bytes, file_id, path_id);
        bytes.extend_from_slice(&1.0f32.to_le_bytes()); // scale x
        bytes.extend_from_slice(&1.0f32.to_le_bytes()); // scale y
        bytes.extend_from_slice(&0.0f32.to_le_bytes()); // offset x
        bytes.extend_from_slice(&0.0f32.to_le_bytes()); // offset y
    }

    fn push_pptr(bytes: &mut Vec<u8>, file_id: i32, path_id: i64) {
        bytes.extend_from_slice(&file_id.to_le_bytes());
        bytes.extend_from_slice(&path_id.to_le_bytes());
    }

    fn push_aligned_string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as i32).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
        align(bytes);
    }

    fn align(bytes: &mut Vec<u8>) {
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
    }
}
