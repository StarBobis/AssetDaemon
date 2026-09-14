/*
 * texture_format.rs - Unity TextureFormat constants
 *
 * Consolidates duplicate format number constants from texture_exporter.rs and dds_exporter.rs,
 * stored together under the common module for all exporters to reference.
 *
 * Values correspond to the Unity official TextureFormat enum:
 *   https://docs.unity3d.com/ScriptReference/TextureFormat.html
 *
 * Follows Soul.md conventions: struct + impl organization, no free functions.
 */

/**
 * Unity TextureFormat enum constants.
 *
 * All methods are static, hold no state; a pure collection of enum constants.
 */
pub struct TextureFormatConst;

#[allow(dead_code)]
impl TextureFormatConst {
    // Uncompressed formats
    pub const ALPHA8: i32 = 1;
    pub const ARGB4444: i32 = 2;
    pub const RGB24: i32 = 3;
    pub const RGBA32: i32 = 4;
    pub const ARGB32: i32 = 5;
    pub const RGB565: i32 = 7;
    pub const R16: i32 = 9;
    pub const RGBA4444: i32 = 13;
    pub const BGRA32: i32 = 14;
    pub const RHALF: i32 = 15;
    pub const RGHALF: i32 = 16;
    pub const RGBAHALF: i32 = 17;
    pub const RFLOAT: i32 = 18;
    pub const RGFLOAT: i32 = 19;
    pub const RGBAFLOAT: i32 = 20;

    // Legacy BC compression
    pub const DXT1: i32 = 10;
    pub const DXT3: i32 = 11;
    pub const DXT5: i32 = 12;

    // Modern BC compression (DX11+)
    pub const BC4: i32 = 26;
    pub const BC5: i32 = 27;
    pub const BC6H: i32 = 24;
    pub const BC7: i32 = 25;

    // Crunched BC
    pub const DXT1_CRUNCHED: i32 = 28;
    pub const DXT5_CRUNCHED: i32 = 29;

    // Other platform formats
    pub const PVRTC_RGB2: i32 = 30;
    pub const PVRTC_RGBA2: i32 = 31;
    pub const PVRTC_RGB4: i32 = 32;
    pub const PVRTC_RGBA4: i32 = 33;
    pub const ETC_RGB4: i32 = 34;
    pub const ETC2_RGBA8: i32 = 47;
    pub const ASTC_RGB_4X4: i32 = 48;
    pub const ETC_RGB4_CRUNCHED: i32 = 62;
    pub const ETC2_RGBA8_CRUNCHED: i32 = 63;

    // Special formats
    pub const YUY2: i32 = 21;
    pub const RGB9E5_FLOAT: i32 = 22;
    pub const RGBFLOAT: i32 = 23;

    // ================================================================
    // Format name lookup
    // ================================================================

    /// Texture format number -> readable name (consistent with AssetStudio's TextureFormat enum)
    pub fn format_name(fmt: i32) -> &'static str {
        match fmt {
            1 => "Alpha8",
            2 => "ARGB4444",
            3 => "RGB24",
            4 => "RGBA32",
            5 => "ARGB32",
            7 => "RGB565",
            9 => "R16",
            10 => "DXT1",
            11 => "DXT3",
            12 => "DXT5",
            13 => "RGBA4444",
            14 => "BGRA32",
            15 => "RHalf",
            16 => "RGHalf",
            17 => "RGBAHalf",
            18 => "RFloat",
            19 => "RGFloat",
            20 => "RGBAFloat",
            21 => "YUY2",
            22 => "RGB9e5Float",
            23 => "RGBFloat",
            24 => "BC6H",
            25 => "BC7",
            26 => "BC4",
            27 => "BC5",
            28 => "DXT1Crunched",
            29 => "DXT5Crunched",
            30 => "PVRTC_RGB2",
            31 => "PVRTC_RGBA2",
            32 => "PVRTC_RGB4",
            33 => "PVRTC_RGBA4",
            34 => "ETC_RGB4",
            47 => "ETC2_RGBA8",
            48 => "ASTC_RGB_4x4",
            62 => "ETC_RGB4Crunched",
            63 => "ETC2_RGBA8Crunched",
            _ => "Unknown",
        }
    }

    /// Texture dimension number -> readable name
    pub fn dimension_name(d: i32) -> &'static str {
        match d {
            1 => "2D",
            2 => "3D",
            3 => "Cube",
            4 => "2DArray",
            5 => "3DArray",
            6 => "CubeArray",
            7 => "2DMS",
            8 => "2DMSArray",
            _ => "Unknown",
        }
    }

    /// ColorSpace -> readable name
    pub fn color_space_name(cs: i32) -> &'static str {
        match cs {
            0 => "Gamma",
            1 => "Linear",
            _ => "Unknown",
        }
    }

    /// FilterMode -> readable name
    pub fn filter_mode_name(fm: i32) -> &'static str {
        match fm {
            0 => "Point",
            1 => "Bilinear",
            2 => "Trilinear",
            _ => "Unknown",
        }
    }

    /// WrapMode -> readable name
    pub fn wrap_mode_name(wm: i32) -> &'static str {
        match wm {
            0 => "Repeat",
            1 => "Clamp",
            2 => "Mirror",
            3 => "MirrorOnce",
            _ => "Unknown",
        }
    }

    /// Check whether a texture format contains an Alpha channel
    pub fn has_alpha(fmt: i32) -> bool {
        matches!(
            fmt,
            1 | 2 | 4 | 5 | 11 | 12 | 13 | 14 | 17 | 20 | 25 | 28 | 29 | 31 | 33 | 35 | 47 | 63
        )
    }
}
