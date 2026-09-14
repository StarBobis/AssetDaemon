/*
 * serialized_file_reader_utils.rs - Serialized file reader utility class.
 *
 * Split from serialized_file.rs, encapsulates all binary reading helper functions,
 * including various endian integer/float reading, TypeTree parsing, etc.
 *
 * Follows Soul.md convention: struct + impl organization, no free functions.
 */

use std::io::{Read, Seek, SeekFrom};

/**
 * Serialized file reader utility class.
 *
 * Encapsulates all binary reading helper functions used in SerializedFile parsing.
 * All methods are static methods, hold no state, and are pure utility methods.
 */
pub struct SerializedFileReaderUtils;

impl SerializedFileReaderUtils {
    // ----------------------------------------------------------
    // Basic Read Methods
    // ----------------------------------------------------------

    pub fn read_u8<R: Read>(reader: &mut R) -> Result<u8, String> {
        let mut buf = [0u8; 1];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(buf[0])
    }

    pub fn read_u16_be(reader: &mut dyn Read) -> Result<u16, String> {
        let mut buf = [0u8; 2];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(u16::from_be_bytes(buf))
    }

    pub fn read_u16_le(reader: &mut dyn Read) -> Result<u16, String> {
        let mut buf = [0u8; 2];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(u16::from_le_bytes(buf))
    }

    pub fn read_i16_be(reader: &mut dyn Read) -> Result<i16, String> {
        let mut buf = [0u8; 2];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(i16::from_be_bytes(buf))
    }

    pub fn read_i16_le(reader: &mut dyn Read) -> Result<i16, String> {
        let mut buf = [0u8; 2];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(i16::from_le_bytes(buf))
    }

    pub fn read_i32_be(reader: &mut dyn Read) -> Result<i32, String> {
        let mut buf = [0u8; 4];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(i32::from_be_bytes(buf))
    }

    pub fn read_i32_le(reader: &mut dyn Read) -> Result<i32, String> {
        let mut buf = [0u8; 4];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(i32::from_le_bytes(buf))
    }

    pub fn read_u32_be(reader: &mut dyn Read) -> Result<u32, String> {
        let mut buf = [0u8; 4];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(u32::from_be_bytes(buf))
    }

    pub fn read_u32_le(reader: &mut dyn Read) -> Result<u32, String> {
        let mut buf = [0u8; 4];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(u32::from_le_bytes(buf))
    }

    pub fn read_i64_be(reader: &mut dyn Read) -> Result<i64, String> {
        let mut buf = [0u8; 8];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(i64::from_be_bytes(buf))
    }

    pub fn read_i64_le(reader: &mut dyn Read) -> Result<i64, String> {
        let mut buf = [0u8; 8];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(i64::from_le_bytes(buf))
    }

    pub fn read_u64_be(reader: &mut dyn Read) -> Result<u64, String> {
        let mut buf = [0u8; 8];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(u64::from_be_bytes(buf))
    }

    pub fn read_u64_le(reader: &mut dyn Read) -> Result<u64, String> {
        let mut buf = [0u8; 8];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(u64::from_le_bytes(buf))
    }

    pub fn read_bool<R: Read>(reader: &mut R) -> Result<bool, String> {
        Ok(Self::read_u8(reader)? != 0)
    }

    pub fn read_bytes<R: Read>(reader: &mut R, count: usize) -> Result<Vec<u8>, String> {
        if count > 100_000_000 {
            return Err(format!("read_bytes request too large: {} bytes", count));
        }
        let mut buf = vec![0u8; count];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        Ok(buf)
    }

    pub fn read_string_to_null<R: Read>(reader: &mut R) -> Result<String, String> {
        let mut bytes = Vec::new();
        loop {
            let b = Self::read_u8(reader)?;
            if b == 0 {
                break;
            }
            bytes.push(b);
        }
        String::from_utf8(bytes).map_err(|e| format!("UTF-8 parse failed: {}", e))
    }

    // Backward compat aliases (default Big-Endian)
    #[allow(dead_code)]
    pub fn read_u16<R: Read>(reader: &mut R) -> Result<u16, String> {
        Self::read_u16_be(reader)
    }
    #[allow(dead_code)]
    pub fn read_i16<R: Read>(reader: &mut R) -> Result<i16, String> {
        Self::read_i16_be(reader)
    }
    #[allow(dead_code)]
    pub fn read_u32<R: Read>(reader: &mut R) -> Result<u32, String> {
        Self::read_u32_be(reader)
    }
    #[allow(dead_code)]
    pub fn read_i32<R: Read>(reader: &mut R) -> Result<i32, String> {
        Self::read_i32_be(reader)
    }
    #[allow(dead_code)]
    pub fn read_u64<R: Read>(reader: &mut R) -> Result<u64, String> {
        Self::read_u64_be(reader)
    }
    #[allow(dead_code)]
    pub fn read_i64<R: Read>(reader: &mut R) -> Result<i64, String> {
        Self::read_i64_be(reader)
    }

    pub fn read_i32_array<R: Read>(reader: &mut R, is_le: bool) -> Result<Vec<i32>, String> {
        let ri32: fn(&mut dyn Read) -> Result<i32, String> = if is_le {
            Self::read_i32_le
        } else {
            Self::read_i32_be
        };
        let count = ri32(reader)?;
        if count <= 0 || count > 100000 {
            return Ok(Vec::new());
        }
        let mut result = Vec::with_capacity(count as usize);
        for _ in 0..count {
            result.push(ri32(reader)?);
        }
        Ok(result)
    }

    pub fn align_stream<R: Seek>(reader: &mut R, alignment: u64) -> Result<(), String> {
        let pos = reader
            .stream_position()
            .map_err(|e| format!("Seek error: {}", e))?;
        let aligned = (pos + alignment - 1) & !(alignment - 1);
        reader
            .seek(SeekFrom::Start(aligned))
            .map_err(|e| format!("Seek error: {}", e))?;
        Ok(())
    }

    // ----------------------------------------------------------
    // Version String Parsing
    // ----------------------------------------------------------

    /**
     * Parse a Unity version string into a [major, minor, build, type] quadruple.
     *
     * Replace all non-numeric characters with '.', then split by '.' and parse.
     * E.g.: "2019.4.29f1" -> "2019.4.29.1" -> [2019, 4, 29, 1]
     */
    pub fn parse_version(version_str: &str) -> [i32; 4] {
        let processed: String = version_str
            .chars()
            .map(|c| if c.is_ascii_digit() { c } else { '.' })
            .collect();
        let digits: Vec<i32> = processed
            .split('.')
            .filter_map(|s| s.parse::<i32>().ok())
            .collect();
        let mut version = [0i32; 4];
        for (i, d) in digits.iter().enumerate().take(4) {
            version[i] = *d;
        }
        version
    }
}
