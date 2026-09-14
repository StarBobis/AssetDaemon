/*
 * Compression Utility Class - Provides LZ4, LZMA decompression capabilities.
 *
 * Follows Soul.md convention: organized as struct + impl, no free functions.
 */

use std::io::Cursor;

// ================================================================
// Compression type constants (consistent with bundle_file.rs)
// ================================================================

const COMPRESSION_NONE: u32 = 0;
const COMPRESSION_LZMA: u32 = 1;
const COMPRESSION_LZ4: u32 = 2;
const COMPRESSION_LZ4HC: u32 = 3;

/**
 * Compression/Decompression Utility Class.
 *
 * Wraps compression algorithms used by UnityFS Bundle such as LZ4, LZMA.
 * No business logic involved, purely a utility method.
 */
pub struct CompressionUtils;

impl CompressionUtils {
    /**
     * Decompresses data according to compression type.
     *
     * @param data             Compressed data
     * @param expected_size    Expected decompressed size
     * @param compression_type Compression type (0=None, 1=LZMA, 2=LZ4, 3=LZ4HC)
     * @return                 Decompressed data
     */
    pub fn decompress_data(
        data: &[u8],
        expected_size: usize,
        compression_type: u32,
    ) -> Result<Vec<u8>, String> {
        match compression_type {
            COMPRESSION_NONE => Ok(data.to_vec()),
            COMPRESSION_LZ4 | COMPRESSION_LZ4HC => Self::lz4_decompress(data, expected_size),
            COMPRESSION_LZMA => Self::lzma_decompress(data, expected_size),
            _ => Err(format!(
                "Unsupported compression type: {}",
                compression_type
            )),
        }
    }

    /**
     * LZ4 decompression.
     */
    fn lz4_decompress(data: &[u8], expected_size: usize) -> Result<Vec<u8>, String> {
        let mut output = vec![0u8; expected_size];
        let written = lz4_flex::decompress_into(data, &mut output)
            .map_err(|e| format!("LZ4 decompression failed: {}", e))?;
        if written != expected_size {
            return Err(format!(
                "LZ4 decompressed size mismatch: wrote {} bytes but expected {} bytes",
                written, expected_size
            ));
        }
        Ok(output)
    }

    /**
     * LZMA decompression.
     */
    fn lzma_decompress(data: &[u8], expected_size: usize) -> Result<Vec<u8>, String> {
        use lzma_rs::decompress::{Options, UnpackedSize};
        let mut output = Vec::with_capacity(expected_size);
        let options = Options {
            unpacked_size: UnpackedSize::UseProvided(Some(expected_size as u64)),
            memlimit: None,
            allow_incomplete: true,
        };
        let mut input = Cursor::new(data);
        lzma_rs::lzma_decompress_with_options(&mut input, &mut output, &options)
            .map_err(|e| format!("LZMA decompression failed: {}", e))?;
        if output.is_empty() {
            return Err("LZMA decompression result is empty".to_string());
        }
        Ok(output)
    }
}
