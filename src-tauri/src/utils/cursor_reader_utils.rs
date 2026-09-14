/*
 * Cursor reader utility class -- provides Cursor-based binary reading capabilities.
 *
 * Follows Soul.md convention: organized as struct + impl, no free functions.
 */

use std::io::{Cursor, Read};

/**
 * Cursor reader utility class (little-endian).
 *
 * Contains general methods for reading various integer types and aligned strings from `&mut Cursor<&[u8]>`.
 * All methods are static methods and do not hold state.
 * Does not involve any business logic; purely utility methods.
 */
pub struct CursorReaderUtils;

impl CursorReaderUtils {
    /**
     * Read a 4-byte little-endian signed integer.
     */
    pub fn read_i32(data: &mut Cursor<&[u8]>) -> Result<i32, String> {
        let mut buf = [0u8; 4];
        data.read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(i32::from_le_bytes(buf))
    }

    /**
     * Read an 8-byte little-endian signed integer.
     */
    pub fn read_i64(data: &mut Cursor<&[u8]>) -> Result<i64, String> {
        let mut buf = [0u8; 8];
        data.read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(i64::from_le_bytes(buf))
    }

    /**
     * Read an aligned string (4-byte alignment).
     *
     * Corresponds to Unity's AlignedString format: first read length (i32),
     * then read the string of that length, and finally align to 4 bytes.
     * Length <= 0 or > 1000000 is treated as an empty string.
     */
    pub fn read_aligned_string(data: &mut Cursor<&[u8]>) -> Result<String, String> {
        let len = Self::read_i32(data)?;
        if len <= 0 || len > 1000000 {
            // align
            let pos = data.position();
            data.set_position((pos + 3) & !3);
            return Ok(String::new());
        }
        let len_usize = len as usize;
        let mut buf = vec![0u8; len_usize];
        data.read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        let s = String::from_utf8_lossy(&buf).to_string();
        // align to 4
        let pos = data.position();
        data.set_position((pos + 3) & !3);
        Ok(s)
    }
}
