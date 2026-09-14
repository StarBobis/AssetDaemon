/*
 * Format Utilities -- Provides binary alignment, FourCC encoding, half-float conversion, and other capabilities.
 *
 * Follows Soul.md conventions: organized as struct + impl, no free functions.
 */

/**
 * Binary format processing utility class.
 *
 * Contains general methods for GLB alignment, DDS FourCC encoding, IEEE half-float conversion, etc.
 * Does not involve any business logic; it is a pure utility method.
 */
pub struct FormatUtils;

impl FormatUtils {
    /** Aligns to 4 bytes */
    pub fn align4(n: usize) -> usize {
        (n + 3) & !3
    }

    /**
     * Pads JSON data to 4-byte alignment.
     *
     * glTF 2.0 spec requires JSON chunks to be padded with spaces (0x20) to 4-byte alignment.
     * Null terminator not allowed -- \0 is not valid whitespace in the JSON spec.
     */
    pub fn align4_json(data: &[u8]) -> Vec<u8> {
        let mut result = Vec::with_capacity(data.len() + 3);
        result.extend_from_slice(data);
        while result.len() % 4 != 0 {
            result.push(b' '); // Space padding (JSON valid whitespace)
        }
        result
    }

    /**
     * Builds a FourCC code (little-endian u32), reserved for DDS/GLB extension use.
     */
    #[allow(dead_code)]
    pub fn make_fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
        a as u32 | (b as u32) << 8 | (c as u32) << 16 | (d as u32) << 24
    }

    /**
     * Converts half-float (16-bit IEEE 754-2008 binary16) to f32.
     */
    pub fn half_to_f32(h: u16) -> f32 {
        let sign = ((h >> 15) & 1) as i32;
        let exp = ((h >> 10) & 0x1F) as i32;
        let mant = (h & 0x3FF) as i32;
        if exp == 0 {
            let val = (mant as f32) * 2.0f32.powi(-24);
            if sign == 1 {
                -val
            } else {
                val
            }
        } else if exp == 31 {
            if mant == 0 {
                if sign == 1 {
                    f32::NEG_INFINITY
                } else {
                    f32::INFINITY
                }
            } else {
                f32::NAN
            }
        } else {
            let val = (1.0 + (mant as f32) / 1024.0) * 2.0f32.powi(exp - 15);
            if sign == 1 {
                -val
            } else {
                val
            }
        }
    }
}
