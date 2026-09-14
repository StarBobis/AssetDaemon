/*
 * unity/mesh_helper.rs - Shared mesh parsing utility methods
 *
 * References AssetStudio's MeshHelper class, providing:
 * - VertexFormat enum definition and size calculation
 * - Byte array to float/integer array conversion
 * - TypeTree value deep extraction methods
 *
 * Follows Soul.md convention: struct + impl organization.
 */

/// Unity vertex format enum (corresponding to AssetStudio's VertexFormat)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VertexFormat {
    Float,
    Float16,
    UNorm8,
    SNorm8,
    UNorm16,
    SNorm16,
    UInt8,
    SInt8,
    UInt16,
    SInt16,
    UInt32,
    SInt32,
}

/**
 * Mesh helper utility class.
 *
 * Provides Unity-version-independent general utility methods:
 * - Vertex format size calculation
 * - Byte data conversion (float/int)
 * - TypeTree value deep extraction
 */
pub struct MeshHelper;

impl MeshHelper {
    /**
     * Returns the byte size for the given format enum value.
     *
     * References AssetStudio's GetFormatSize() method:
     * Float=4, Float16=2, UNorm8=1, SNorm8=1,
     * UNorm16=2, SNorm16=2, UInt8=1, SInt8=1,
     * UInt16=2, SInt16=2, UInt32=4, SInt32=4
     */
    pub fn get_format_size(format: VertexFormat) -> u32 {
        match format {
            VertexFormat::Float => 4,
            VertexFormat::Float16 => 2,
            VertexFormat::UNorm8
            | VertexFormat::SNorm8
            | VertexFormat::UInt8
            | VertexFormat::SInt8 => 1,
            VertexFormat::UNorm16
            | VertexFormat::SNorm16
            | VertexFormat::UInt16
            | VertexFormat::SInt16 => 2,
            VertexFormat::UInt32 | VertexFormat::SInt32 => 4,
        }
    }

    /**
     * Checks whether the vertex format is an integer type (used to distinguish float and int channels).
     */
    pub fn is_int_format(format: VertexFormat) -> bool {
        matches!(
            format,
            VertexFormat::UInt8
                | VertexFormat::SInt8
                | VertexFormat::UInt16
                | VertexFormat::SInt16
                | VertexFormat::UInt32
                | VertexFormat::SInt32
        )
    }

    /**
     * Converts raw byte array to float array according to the specified vertex format.
     *
     * References AssetStudio's BytesToFloatArray() method.
     * Note: Only handles Float/Float16/UNorm8/SNorm8/UNorm16/SNorm16 formats.
     * Integer formats (UInt8/SInt8/UInt16/SInt16/UInt32/SInt32) are
     * handled by bytes_to_int_array(); callers should check is_int_format() first.
     *
     * @param input_bytes  Raw byte data
     * @param format       Vertex format
     * @return             Converted float array
     */
    pub fn bytes_to_float_array(input_bytes: &[u8], format: VertexFormat) -> Vec<f32> {
        let size = Self::get_format_size(format) as usize;
        if size == 0 || input_bytes.is_empty() {
            return Vec::new();
        }
        let len = input_bytes.len() / size;
        let mut result = vec![0.0f32; len];

        for i in 0..len {
            let off = i * size;
            if off + size > input_bytes.len() {
                break;
            }
            let value = match format {
                VertexFormat::Float => {
                    let bytes: [u8; 4] = input_bytes[off..off + 4].try_into().unwrap_or([0; 4]);
                    f32::from_le_bytes(bytes)
                }
                VertexFormat::Float16 => {
                    // Use FormatUtils::half_to_f32 for IEEE 754-2008 binary16 to f32 conversion
                    let bytes: [u8; 2] = input_bytes[off..off + 2].try_into().unwrap_or([0; 2]);
                    crate::utils::format_utils::FormatUtils::half_to_f32(u16::from_le_bytes(bytes))
                }
                VertexFormat::UNorm8 => input_bytes[off] as f32 / 255.0f32,
                VertexFormat::SNorm8 => {
                    let val = input_bytes[off] as i8;
                    (val as f32 / 127.0f32).max(-1.0)
                }
                VertexFormat::UNorm16 => {
                    let bytes: [u8; 2] = input_bytes[off..off + 2].try_into().unwrap_or([0; 2]);
                    u16::from_le_bytes(bytes) as f32 / 65535.0f32
                }
                VertexFormat::SNorm16 => {
                    let bytes: [u8; 2] = input_bytes[off..off + 2].try_into().unwrap_or([0; 2]);
                    let val = i16::from_le_bytes(bytes);
                    (val as f32 / 32767.0f32).max(-1.0)
                }
                // Integer formats not handled here - fully aligned with AssetStudio BytesToFloatArray
                _ => 0.0f32,
            };
            result[i] = value;
        }

        result
    }

    /**
     * Converts raw byte array to integer array according to the specified vertex format.
     *
     * References AssetStudio's BytesToIntArray() method.
     */
    pub fn bytes_to_int_array(input_bytes: &[u8], format: VertexFormat) -> Vec<i32> {
        let size = Self::get_format_size(format) as usize;
        if size == 0 || input_bytes.is_empty() {
            return Vec::new();
        }
        let len = input_bytes.len() / size;
        let mut result = vec![0i32; len];

        for i in 0..len {
            let off = i * size;
            if off + size > input_bytes.len() {
                break;
            }
            let value = match format {
                // AssetStudio uses ToInt16 (signed) for both UInt16/SInt16
                VertexFormat::UInt8 | VertexFormat::SInt8 => input_bytes[off] as i32,
                VertexFormat::UInt16 | VertexFormat::SInt16 => {
                    let bytes: [u8; 2] = input_bytes[off..off + 2].try_into().unwrap_or([0; 2]);
                    i16::from_le_bytes(bytes) as i32
                }
                VertexFormat::UInt32 => {
                    let bytes: [u8; 4] = input_bytes[off..off + 4].try_into().unwrap_or([0; 4]);
                    u32::from_le_bytes(bytes) as i32
                }
                VertexFormat::SInt32 => {
                    let bytes: [u8; 4] = input_bytes[off..off + 4].try_into().unwrap_or([0; 4]);
                    i32::from_le_bytes(bytes)
                }
                _ => 0,
            };
            result[i] = value;
        }

        result
    }

    /**
     * Converts VertexFormat2017 enum (0-12) to standard VertexFormat.
     *
     * Unity 2017.x introduced new format enum values incompatible with older versions.
     * References AssetStudio's ToVertexFormat() method.
     */
    pub fn from_vertex_format_2017(format_2017: u32) -> VertexFormat {
        // References AssetStudio MeshHelper.ToVertexFormat() -> version[0] < 2019 branch
        //
        // AssetStudio VertexFormat2017 enum:
        //   Float(0), Float16(1), Color(2), UNorm8(3), SNorm8(4),
        //   UNorm16(5), SNorm16(6), UInt8(7), SInt8(8),
        //   UInt16(9), SInt16(10), UInt32(11), SInt32(12)
        // Color(2) maps to UNorm8, so both 2 and 3 -> UNorm8
        match format_2017 {
            0 => VertexFormat::Float,
            1 => VertexFormat::Float16,
            2 => VertexFormat::UNorm8, // AssetStudio: Color -> UNorm8
            3 => VertexFormat::UNorm8, // AssetStudio: UNorm8
            4 => VertexFormat::SNorm8,
            5 => VertexFormat::UNorm16,
            6 => VertexFormat::SNorm16,
            7 => VertexFormat::UInt8,
            8 => VertexFormat::SInt8,
            9 => VertexFormat::UInt16,
            10 => VertexFormat::SInt16,
            11 => VertexFormat::UInt32,
            12 => VertexFormat::SInt32,
            _ => VertexFormat::Float,
        }
    }

    /**
     * Converts legacy VertexChannelFormat enum (0-4) to standard VertexFormat.
     *
     * References AssetStudio's ToVertexFormat() branch for version[0] < 2017.
     */
    pub fn from_vertex_channel_format(channel_format: u32) -> VertexFormat {
        match channel_format {
            0 => VertexFormat::Float,   // kChannelFormatFloat
            1 => VertexFormat::Float16, // kChannelFormatFloat16
            2 => VertexFormat::UNorm8,  // kChannelFormatColor
            3 => VertexFormat::UInt8,   // kChannelFormatByte
            4 => VertexFormat::UInt32,  // kChannelFormatUInt32
            _ => VertexFormat::Float,
        }
    }
}
