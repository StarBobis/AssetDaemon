/*
 * Exporter module -- collection of asset type export utility classes.
 *
 * Each sub-module handles exporting one specific data type and is indivisible.
 * Naming convention: xxx_exporter.rs
 *
 * Following Soul.md conventions: organized as struct + impl, no free functions.
 */

#[allow(dead_code)]
pub mod animation_clip_exporter;
pub mod animation_clip_raw_parser;
pub mod audio_exporter;
pub mod dds_exporter;
pub mod font_exporter;
pub mod glb_exporter;
pub mod material_info;
pub mod mesh_exporter;
pub mod model_context;
pub mod mono_behaviour_exporter;
pub mod raw_exporter;
pub mod shader_exporter;
#[allow(dead_code)]
pub mod text_asset_exporter;
pub mod texture_exporter;
pub mod video_exporter;
