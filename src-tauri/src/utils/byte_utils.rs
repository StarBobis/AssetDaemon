/*
 * Byte utility class -- provides little-endian reading, byte conversion, and other general capabilities.
 *
 * Follows Soul.md convention: organized as struct + impl, no free functions.
 */

/**
 * Byte processing utility class.
 *
 * Contains general methods for reading little-endian integers and floats from byte arrays.
 * Does not involve any business logic; purely utility methods.
 */
pub struct ByteUtils;

impl ByteUtils {
    /**
     * Read a little-endian u32 from the specified offset in a byte array.
     */
    pub fn read_u32_le(data: &[u8], offset: usize) -> u32 {
        // offset + 4 must not overflow usize before the bounds comparison.
        let Some(end) = offset.checked_add(4) else {
            return 0;
        };
        if end > data.len() {
            return 0;
        }
        u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ])
    }

    /**
     * Read a little-endian f32 from the specified offset in a byte array.
     */
    pub fn read_f32_le(data: &[u8], offset: usize) -> f32 {
        let Some(end) = offset.checked_add(4) else {
            return 0.0;
        };
        if end > data.len() {
            return 0.0;
        }
        f32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ])
    }

    /**
     * Read a little-endian i32 from the specified offset in a byte array (reserved utility method for future parsing modules).
     */
    #[allow(dead_code)]
    pub fn read_i32_le(data: &[u8], offset: usize) -> i32 {
        let Some(end) = offset.checked_add(4) else {
            return 0;
        };
        if end > data.len() {
            return 0;
        }
        i32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ])
    }
}
