/*
 * Byte reader utility class -- provides big-endian binary reading capabilities.
 *
 * Follows Soul.md convention: organized as struct + impl, no free functions.
 */

use std::io::Read;

/**
 * Byte reader utility class (big-endian).
 *
 * Contains general methods for reading various integer types and strings from `&mut impl Read`.
 * All methods are static methods and do not hold state.
 * Does not involve any business logic; purely utility methods.
 */
pub struct BundleReaderUtils;

impl BundleReaderUtils {
    /**
     * Read a 1-byte unsigned integer.
     */
    pub fn read_u8<R: Read>(reader: &mut R) -> Result<u8, String> {
        let mut buf = [0u8; 1];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(buf[0])
    }

    /**
     * Read a 2-byte big-endian unsigned integer.
     */
    pub fn read_u16<R: Read>(reader: &mut R) -> Result<u16, String> {
        let mut buf = [0u8; 2];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(u16::from_be_bytes(buf))
    }

    /**
     * Read a 4-byte big-endian signed integer.
     */
    pub fn read_i32<R: Read>(reader: &mut R) -> Result<i32, String> {
        let mut buf = [0u8; 4];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(i32::from_be_bytes(buf))
    }

    /**
     * Read a 4-byte big-endian unsigned integer.
     */
    pub fn read_u32<R: Read>(reader: &mut R) -> Result<u32, String> {
        let mut buf = [0u8; 4];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(u32::from_be_bytes(buf))
    }

    /**
     * Read an 8-byte big-endian signed integer.
     */
    pub fn read_i64<R: Read>(reader: &mut R) -> Result<i64, String> {
        let mut buf = [0u8; 8];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(i64::from_be_bytes(buf))
    }

    /**
     * Read raw data of a specified number of bytes.
     *
     * First attempts exact reading; if that fails, falls back to reading as many bytes as possible.
     */
    pub fn read_bytes<R: Read>(reader: &mut R, count: usize) -> Result<Vec<u8>, String> {
        let mut buf = vec![0u8; count];
        match reader.read_exact(&mut buf) {
            Ok(()) => Ok(buf),
            Err(e) => {
                // Fallback: try reading as many bytes as possible
                let available = match reader.read(&mut buf) {
                    Ok(n) => n,
                    Err(_) => 0,
                };
                if available == 0 {
                    return Err(format!(
                        "Read error: {} (requested {} bytes, 0 available)",
                        e, count
                    ));
                }
                buf.truncate(available);
                Ok(buf)
            }
        }
    }

    /**
     * Read a null-terminated string (max 1024 bytes).
     */
    pub fn read_string_to_null<R: Read>(reader: &mut R) -> Result<String, String> {
        Self::read_string_to_null_limited(reader, 1024)
    }

    /**
     * Read a null-terminated string (with max length limit).
     */
    pub fn read_string_to_null_limited<R: Read>(
        reader: &mut R,
        max_len: usize,
    ) -> Result<String, String> {
        let mut bytes = Vec::new();
        for _ in 0..max_len {
            let b = Self::read_u8(reader)?;
            if b == 0 {
                return String::from_utf8(bytes).map_err(|e| format!("UTF-8 parse failure: {}", e));
            }
            bytes.push(b);
        }
        // Reached max length without encountering null -- possibly encrypted data or format error
        Err(format!(
            "String exceeded {} bytes without encountering null terminator (read: {:?}...)",
            max_len,
            String::from_utf8_lossy(&bytes[..bytes.len().min(32)])
        ))
    }
}
