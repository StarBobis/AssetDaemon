/*
 * material_info.rs -- shared material dependency model.
 *
 * These structs describe Unity material data after dependency resolution.
 * Exporters can map them to their own target format.
 */

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct MaterialInfo {
    pub name: String,
    pub diffuse_color: [f32; 4],
    pub specular_color: [f32; 4],
    pub emissive_color: [f32; 4],
    pub shininess: f32,
    pub textures: Vec<TextureSlot>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TextureSlot {
    pub slot_name: String,
    pub usage: String,
    pub file_name: String,
    pub relative_path: String,
}
