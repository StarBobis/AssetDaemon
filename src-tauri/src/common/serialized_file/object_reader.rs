/*
 * object_reader.rs - Unity SerializedFile Object Data Reader.
 *
 * Split from serialized_file.rs, modeled after AssetStudio ObjectReader,
 * responsible for reading serialized fields from raw object bytes.
 *
 * Follows Soul.md convention: organized as struct + impl, no free functions.
 */

use crate::common::serialized_file::serialized_file::SerializedFile;

/// ObjectReader is used to read serialized data of a single object
#[allow(dead_code)]
pub struct ObjectReader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
    pub version: [i32; 4],
    pub endian: u8,
    /// SerializedFile header.version (used for PPtr pathID size detection, etc.)
    pub header_version: u32,
    /// Target platform (BuildTarget, default -2 = NoTarget)
    pub target_platform: i32,
}

#[cfg(test)]
mod tests {
    use super::ObjectReader;

    fn reader_for(data: &[u8]) -> ObjectReader<'_> {
        ObjectReader::for_test(data)
    }

    #[test]
    fn strict_integer_read_reports_truncated_data() {
        let mut reader = reader_for(&[1, 2]);

        assert!(reader.try_read_i32().is_err());
    }

    #[test]
    fn legacy_integer_read_keeps_compatibility() {
        let mut reader = reader_for(&[1, 2]);

        assert_eq!(reader.read_i32(), 0x0201);
    }
}

#[allow(dead_code)]
impl<'a> ObjectReader<'a> {
    pub fn new(data: &'a [u8], serialized_file: &SerializedFile) -> Self {
        ObjectReader {
            data,
            pos: 0,
            version: serialized_file.version,
            endian: serialized_file.header.endianess,
            header_version: serialized_file.header.version,
            target_platform: serialized_file.target_platform,
        }
    }

    #[cfg(test)]
    fn for_test(data: &'a [u8]) -> Self {
        ObjectReader {
            data,
            pos: 0,
            version: [2020, 3, 0, 1],
            endian: 0,
            header_version: 22,
            target_platform: -2,
        }
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn read_u8(&mut self) -> u8 {
        let v = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos = self.pos.saturating_add(1).min(self.data.len());
        v
    }

    fn is_le(&self) -> bool {
        self.endian == 0
    }

    fn read_exact<const N: usize>(&mut self, label: &str) -> Result<[u8; N], String> {
        let end = self
            .pos
            .checked_add(N)
            .ok_or_else(|| format!("{} read offset overflowed", label))?;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| {
                format!(
                    "Unexpected end of object data while reading {} at byte {}: need {} bytes, {} remaining",
                    label,
                    self.pos,
                    N,
                    self.remaining()
                )
            })?;
        let mut out = [0u8; N];
        out.copy_from_slice(bytes);
        self.pos = end;
        Ok(out)
    }

    pub fn read_i16(&mut self) -> i16 {
        let end = self.pos.saturating_add(2).min(self.data.len());
        let len = end - self.pos;
        let mut bytes = [0u8; 2];
        bytes[..len].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        if self.is_le() {
            i16::from_le_bytes(bytes)
        } else {
            i16::from_be_bytes(bytes)
        }
    }

    pub fn read_u16(&mut self) -> u16 {
        let end = self.pos.saturating_add(2).min(self.data.len());
        let len = end - self.pos;
        let mut bytes = [0u8; 2];
        bytes[..len].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        if self.is_le() {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        }
    }

    pub fn read_i32(&mut self) -> i32 {
        let end = self.pos.saturating_add(4).min(self.data.len());
        let len = end - self.pos;
        let mut bytes = [0u8; 4];
        bytes[..len].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        if self.is_le() {
            i32::from_le_bytes(bytes)
        } else {
            i32::from_be_bytes(bytes)
        }
    }

    pub fn try_read_i32(&mut self) -> Result<i32, String> {
        let bytes = self.read_exact::<4>("i32")?;
        Ok(if self.is_le() {
            i32::from_le_bytes(bytes)
        } else {
            i32::from_be_bytes(bytes)
        })
    }

    pub fn read_u32(&mut self) -> u32 {
        let end = self.pos.saturating_add(4).min(self.data.len());
        let len = end - self.pos;
        let mut bytes = [0u8; 4];
        bytes[..len].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        if self.is_le() {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        }
    }

    pub fn try_read_u32(&mut self) -> Result<u32, String> {
        let bytes = self.read_exact::<4>("u32")?;
        Ok(if self.is_le() {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    }

    pub fn read_i64(&mut self) -> i64 {
        let end = self.pos.saturating_add(8).min(self.data.len());
        let len = end - self.pos;
        let mut bytes = [0u8; 8];
        bytes[..len].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        if self.is_le() {
            i64::from_le_bytes(bytes)
        } else {
            i64::from_be_bytes(bytes)
        }
    }

    pub fn try_read_i64(&mut self) -> Result<i64, String> {
        let bytes = self.read_exact::<8>("i64")?;
        Ok(if self.is_le() {
            i64::from_le_bytes(bytes)
        } else {
            i64::from_be_bytes(bytes)
        })
    }

    pub fn read_u64(&mut self) -> u64 {
        let end = self.pos.saturating_add(8).min(self.data.len());
        let len = end - self.pos;
        let mut bytes = [0u8; 8];
        bytes[..len].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        if self.is_le() {
            u64::from_le_bytes(bytes)
        } else {
            u64::from_be_bytes(bytes)
        }
    }

    pub fn read_f32(&mut self) -> f32 {
        let end = self.pos.saturating_add(4).min(self.data.len());
        let len = end - self.pos;
        let mut bytes = [0u8; 4];
        bytes[..len].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        if self.is_le() {
            f32::from_le_bytes(bytes)
        } else {
            f32::from_be_bytes(bytes)
        }
    }

    pub fn try_read_f32(&mut self) -> Result<f32, String> {
        let bytes = self.read_exact::<4>("f32")?;
        Ok(if self.is_le() {
            f32::from_le_bytes(bytes)
        } else {
            f32::from_be_bytes(bytes)
        })
    }

    pub fn read_f64(&mut self) -> f64 {
        let end = self.pos.saturating_add(8).min(self.data.len());
        let len = end - self.pos;
        let mut bytes = [0u8; 8];
        bytes[..len].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        if self.is_le() {
            f64::from_le_bytes(bytes)
        } else {
            f64::from_be_bytes(bytes)
        }
    }

    pub fn read_bool(&mut self) -> bool {
        self.read_u8() != 0
    }

    pub fn try_read_bool(&mut self) -> Result<bool, String> {
        Ok(self.read_exact::<1>("bool")?[0] != 0)
    }

    pub fn read_aligned_string(&mut self) -> String {
        let len = self.read_i32();
        if len <= 0 || len > 1024 * 1024 {
            self.align();
            return String::new();
        }
        let len_usize = len as usize;
        let end = self.pos.saturating_add(len_usize).min(self.data.len());
        let s = String::from_utf8_lossy(&self.data[self.pos..end]).to_string();
        self.pos = end;
        self.align();
        s
    }

    pub fn try_read_aligned_string(&mut self) -> Result<String, String> {
        let len = self.try_read_i32()?;
        if len < 0 {
            return Err(format!("Invalid negative string length {}", len));
        }
        if len == 0 {
            self.align();
            return Ok(String::new());
        }
        if len > 1024 * 1024 {
            return Err(format!("String length {} exceeds limit", len));
        }
        let len_usize = len as usize;
        let end = self
            .pos
            .checked_add(len_usize)
            .ok_or_else(|| "String read offset overflowed".to_string())?;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| {
                format!(
                    "Unexpected end of object data while reading string at byte {}: need {} bytes, {} remaining",
                    self.pos,
                    len_usize,
                    self.remaining()
                )
            })?;
        let s = String::from_utf8_lossy(bytes).to_string();
        self.pos = end;
        self.align();
        Ok(s)
    }

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

    pub fn try_read_u8_array(&mut self) -> Result<Vec<u8>, String> {
        let len = self.try_read_i32()?;
        if len < 0 {
            return Err(format!("Invalid negative byte array length {}", len));
        }
        if len == 0 {
            return Ok(Vec::new());
        }
        if len > 100_000_000 {
            return Err(format!("Byte array length {} exceeds limit", len));
        }
        let len_usize = len as usize;
        let end = self
            .pos
            .checked_add(len_usize)
            .ok_or_else(|| "Byte array read offset overflowed".to_string())?;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| {
                format!(
                    "Unexpected end of object data while reading byte array at byte {}: need {} bytes, {} remaining",
                    self.pos,
                    len_usize,
                    self.remaining()
                )
            })?
            .to_vec();
        self.pos = end;
        Ok(bytes)
    }

    pub fn align(&mut self) {
        self.pos = self.pos.saturating_add(3) & !3;
        self.pos = self.pos.min(self.data.len());
    }

    #[allow(dead_code)]
    pub fn read_f32_array(&mut self, count: usize) -> Vec<f32> {
        let max_read = self.remaining() / 4;
        let actual = count.min(max_read);
        let mut result = Vec::with_capacity(actual);
        for _ in 0..actual {
            result.push(self.read_f32());
        }
        result
    }

    #[allow(dead_code)]
    pub fn read_u32_array(&mut self, count: usize) -> Vec<u32> {
        let max_read = self.remaining() / 4;
        let actual = count.min(max_read);
        let mut result = Vec::with_capacity(actual);
        for _ in 0..actual {
            result.push(self.read_u32());
        }
        result
    }

    pub fn read_string_to_null(&mut self) -> String {
        let mut bytes = Vec::new();
        while self.pos < self.data.len() {
            let b = self.data[self.pos];
            self.pos += 1;
            if b == 0 {
                break;
            }
            bytes.push(b);
        }
        String::from_utf8_lossy(&bytes).to_string()
    }

    pub fn skip(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n).min(self.data.len());
    }

    /// Reads the PPtr pathID field, size adapts based on Unity version.
    pub fn read_path_id(&mut self) -> i64 {
        if self.header_version < 14 {
            self.read_i32() as i64
        } else {
            self.read_i64()
        }
    }

    /// Returns the byte size of PPtr in serialized data (fileID + pathID).
    pub fn ppt_size(&self) -> usize {
        if self.header_version < 14 {
            8
        } else {
            12
        }
    }

    #[allow(dead_code)]
    pub fn read_i32_array(&mut self, count: usize) -> Vec<i32> {
        let max_read = self.remaining() / 4;
        let actual = count.min(max_read);
        let mut result = Vec::with_capacity(actual);
        for _ in 0..actual {
            result.push(self.read_i32());
        }
        result
    }
}
