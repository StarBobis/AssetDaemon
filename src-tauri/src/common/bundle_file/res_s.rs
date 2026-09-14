/*
 * res_s.rs - Unity .resS Resource File Reader
 *
 * .resS file is Unity's streaming resource file,
 * used to store large binary blobs such as mesh vertex buffer data
 * and texture pixel data.
 *
 * These files have no specific header format; they are essentially pure binary data blobs.
 * Mesh/Texture2D objects reference data blocks at specific offsets via
 * StreamingInfo (path, offset, size).
 *
 * Based on AssetStudio's ResourceReader.cs:
 *   - GetData() reads size bytes at the specified offset
 *   - Supports finding .resS files in the Bundle's node/file system
 *
 * Follows Soul.md convention: organized as struct + impl, no free functions.
 */

/**
 * .resS resource file reader class.
 *
 * Holds the complete data of a .resS file, supporting sub-block reads by offset and size.
 */
#[allow(dead_code)]
pub struct ResS {
    /// File name (e.g., "CAB-xxx.resS")
    pub file_name: String,
    /// Complete file data
    data: Vec<u8>,
}

impl ResS {
    /**
     * Creates a new ResS reader.
     *
     * @param file_name  File name (for identification and logging)
     * @param data       Complete .resS file data
     */
    pub fn new(file_name: String, data: Vec<u8>) -> Self {
        ResS { file_name, data }
    }

    /**
     * Returns the total file size.
     */
    #[allow(dead_code)]
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /**
     * Reads a data block of the specified size at the specified offset.
     *
     * Corresponds to AssetStudio ResourceReader.GetData():
     *   binaryReader.BaseStream.Position = offset;
     *   return binaryReader.ReadBytes((int)size);
     *
     * @param offset  Starting offset of the data in the .resS file
     * @param size    Number of bytes to read
     * @return        The byte slice read, returns an error if out of bounds
     */
    pub fn read(&self, offset: usize, size: usize) -> Result<&[u8], String> {
        let end = offset.saturating_add(size);
        if offset >= self.data.len() {
            return Err(format!(
                "ResS '{}': offset {} exceeds file size {}",
                self.file_name,
                offset,
                self.data.len()
            ));
        }
        if end > self.data.len() {
            return Err(format!(
                "ResS '{}': read range [{}, {}) exceeds file size {}",
                self.file_name,
                offset,
                end,
                self.data.len()
            ));
        }
        Ok(&self.data[offset..end])
    }
}
