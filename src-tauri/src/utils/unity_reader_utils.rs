/*
 * Unity Binary Reader Utility Class - Modeled After AssetStudio's ObjectReader.
 *
 * Reads fields from raw byte streams in Unity serialization format (little-endian).
 * No business logic involved, purely a utility method.
 *
 * Follows Soul.md convention: organized as struct + impl, no free functions.
 */

/**
 * Unity Binary Reader Utility Class.
 *
 * Binary reader modeled after AssetStudio's ObjectReader, used to
 * read various primitive types and array fields from raw byte streams
 * in Unity serialization format (little-endian).
 * Includes version number parsing and boundary protection.
 */
pub struct UnityReader<'a> {
    data: &'a [u8],
    pos: usize,
    version: [i32; 4], // [major, minor, build, type]
}

impl<'a> UnityReader<'a> {
    /**
     * Creates a new UnityReader instance.
     *
     * Parses the version string into a quadruple [major, minor, build, type],
     * same logic as AssetStudio's SetVersion().
     *
     * @param data        Raw byte data
     * @param version_str Unity version string (e.g., "2019.4.31f1")
     */
    pub fn new(data: &'a [u8], version_str: &str) -> Self {
        // Parse version number: same logic as AssetStudio's SetVersion()
        let digits: Vec<i32> = version_str
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '.')
            .collect::<String>()
            .split('.')
            .filter_map(|s| s.parse::<i32>().ok())
            .collect();

        let mut version = [0i32; 4];
        for (i, d) in digits.iter().enumerate().take(4) {
            version[i] = *d;
        }

        UnityReader {
            data,
            pos: 0,
            version,
        }
    }

    /**
     * Returns the version number quadruple of the current reader.
     */
    pub fn get_version(&self) -> [i32; 4] {
        self.version
    }

    /**
     * Returns the current cursor position (for debugging cursor offset).
     */
    #[allow(dead_code)]
    pub fn pos(&self) -> usize {
        self.pos
    }

    /**
     * Returns the number of remaining readable bytes.
     */
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    // ---- Primitive type reads (little-endian), with boundary protection ----

    /**
     * Reads a 1-byte unsigned integer.
     */
    pub fn read_u8(&mut self) -> u8 {
        let v = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos = self.pos.saturating_add(1).min(self.data.len());
        v
    }

    /**
     * Reads a 2-byte little-endian unsigned integer.
     */
    pub fn read_u16(&mut self) -> u16 {
        let end = self.pos.saturating_add(2).min(self.data.len());
        let len = end - self.pos;
        let mut bytes = [0u8; 2];
        bytes[..len].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        u16::from_le_bytes(bytes)
    }

    /**
     * Reads a 4-byte little-endian signed integer.
     */
    pub fn read_i32(&mut self) -> i32 {
        let end = self.pos.saturating_add(4).min(self.data.len());
        let len = end - self.pos;
        let mut bytes = [0u8; 4];
        bytes[..len].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        i32::from_le_bytes(bytes)
    }

    /**
     * Reads a 4-byte little-endian unsigned integer.
     * Reads directly as u32 byte order, not via i32 conversion.
     */
    pub fn read_u32(&mut self) -> u32 {
        let end = self.pos.saturating_add(4).min(self.data.len());
        let len = end - self.pos;
        let mut bytes = [0u8; 4];
        bytes[..len].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        u32::from_le_bytes(bytes)
    }

    /**
     * Reads an 8-byte little-endian signed integer.
     */
    pub fn read_i64(&mut self) -> i64 {
        let end = self.pos.saturating_add(8).min(self.data.len());
        let len = end - self.pos;
        let mut bytes = [0u8; 8];
        bytes[..len].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        i64::from_le_bytes(bytes)
    }

    /**
     * Reads a 4-byte little-endian float.
     */
    pub fn read_f32(&mut self) -> f32 {
        let end = self.pos.saturating_add(4).min(self.data.len());
        let len = end - self.pos;
        let mut bytes = [0u8; 4];
        bytes[..len].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        f32::from_le_bytes(bytes)
    }

    /**
     * Reads a boolean value (1 byte, non-zero is true).
     */
    pub fn read_bool(&mut self) -> bool {
        self.read_u8() != 0
    }

    /**
     * Reads 1 byte (same as read_u8).
     */
    pub fn read_byte(&mut self) -> u8 {
        self.read_u8()
    }

    /**
     * Reads an array of the specified number of u32.
     *
     * @param count Expected number of elements to read (limited by remaining bytes)
     */
    pub fn read_u32_array(&mut self, count: usize) -> Vec<u32> {
        let max_read = self.remaining() / 4;
        let actual = count.min(max_read);
        let mut result = Vec::with_capacity(actual);
        for _ in 0..actual {
            result.push(self.read_u32());
        }
        result
    }

    /**
     * Reads an array of the specified number of f32.
     *
     * @param count Expected number of elements to read (limited by remaining bytes)
     */
    pub fn read_f32_array(&mut self, count: usize) -> Vec<f32> {
        let max_read = self.remaining() / 4;
        let actual = count.min(max_read);
        let mut result = Vec::with_capacity(actual);
        for _ in 0..actual {
            result.push(self.read_f32());
        }
        result
    }

    /**
     * Reads an aligned string (modeled after AssetStudio's ReadAlignedString).
     *
     * First reads a 4-byte length, then reads the UTF-8 string of that length,
     * then aligns to a 4-byte boundary.
     */
    pub fn read_aligned_string(&mut self) -> String {
        let len = self.read_i32();
        if len <= 0 || len > 1024 * 1024 {
            return String::new();
        }
        let len_usize = len as usize;
        let end = self.pos.saturating_add(len_usize).min(self.data.len());
        let s = String::from_utf8_lossy(&self.data[self.pos..end]).to_string();
        self.pos = end;
        // Align to 4 bytes
        self.pos = self.pos.saturating_add(3) & !3;
        self.pos = self.pos.min(self.data.len());
        s
    }

    /**
     * Reads a uint8 array (modeled after AssetStudio's ReadUInt8Array).
     *
     * First reads a 4-byte length, then reads the raw bytes of that length.
     */
    pub fn read_u8_array(&mut self) -> Vec<u8> {
        let len = self.read_i32();
        if len <= 0 || len > 100_000_000 {
            return Vec::new();
        }
        let len_usize = len as usize;
        let end = self.pos.saturating_add(len_usize).min(self.data.len());
        let bytes = self.data[self.pos..end].to_vec();
        self.pos = end;
        bytes
    }

    /**
     * Aligns to a 4-byte boundary (modeled after AssetStudio's AlignStream).
     */
    pub fn align(&mut self) {
        self.pos = self.pos.saturating_add(3) & !3;
        self.pos = self.pos.min(self.data.len());
    }

    /**
     * Skips the specified number of bytes.
     *
     * @param n Number of bytes to skip
     */
    pub fn skip(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n).min(self.data.len());
    }
}
