/*
 * unity/mod.rs -- Unity asset parsing module
 *
 * Dispatches to different parser versions based on the Unity version number.
 * Follows Soul.md conventions: this file contains only module declarations.
 *
 * Version differences (see AssetStudio):
 * - 4.x and below: VertexData uses the old channel-mask approach (GetChannels)
 * - 5.x ~ 2017.x: VertexData uses ChannelInfo/StreamInfo format
 * - 2018.x+: channel map adds kShaderChannelBlendWeight/BlendIndices
 * - 2017.x specific: VertexFormat2017 enum is incompatible with older versions
 */

pub mod classes;
pub mod mesh_asset_studio;
pub mod mesh_helper;
pub mod mesh_heuristic_parser;
pub mod relations;
pub mod type_tree;
