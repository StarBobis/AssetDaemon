use crate::unity::classes::texture_format::TextureFormatConst;
use base64::Engine;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct TextureExporter;

impl TextureExporter {
    /// Public decode interface: decode pixel data according to texture format and save as PNG (for preview)
    #[allow(dead_code)]
    pub fn decode_pixels(
        data: &[u8],
        format: i32,
        width: u32,
        height: u32,
        out: &Path,
    ) -> Result<(), String> {
        let rgba = Self::decode_pixels_to_display_rgba(data, format, width, height)?;
        Self::save_png(&rgba, width, height, out)
    }

    /// Decode pixels to RGBA8 (for preview)
    #[allow(dead_code)]
    pub fn decode_pixels_to_rgba(
        data: &[u8],
        format: i32,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, String> {
        Self::decode_pixels_to_rgba_cancelable(data, format, width, height, None)
    }

    /// Decode Unity texture pixels to display-oriented RGBA8.
    ///
    /// Unity stores Texture2D pixel rows bottom-to-top for the formats decoded here, while PNG and
    /// browser image display expect the first row to be the visual top row.
    #[allow(dead_code)]
    pub fn decode_pixels_to_display_rgba(
        data: &[u8],
        format: i32,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, String> {
        Self::decode_pixels_to_display_rgba_cancelable(data, format, width, height, None)
    }

    /// Decode Unity texture pixels to display-oriented RGBA8 with cancellation support.
    pub fn decode_pixels_to_display_rgba_cancelable(
        data: &[u8],
        format: i32,
        width: u32,
        height: u32,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<Vec<u8>, String> {
        let rgba =
            Self::decode_pixels_to_rgba_cancelable(data, format, width, height, cancel_token)?;
        Self::flip_rgba_vertical_cancelable(&rgba, width, height, cancel_token)
    }

    /// Decode pixels to RGBA8, checking the task cancel token during long compressed decodes.
    pub fn decode_pixels_to_rgba_cancelable(
        data: &[u8],
        format: i32,
        width: u32,
        height: u32,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<Vec<u8>, String> {
        if width == 0 || height == 0 {
            return Err("Texture has invalid dimensions".to_string());
        }
        Self::check_cancelled(cancel_token)?;
        match format {
            TextureFormatConst::RGB24 => {
                Self::raw_rgb24_cancelable(data, width, height, cancel_token)
            }
            TextureFormatConst::RGBA32 => {
                let required = Self::image_byte_len(width, height, 4)?;
                Self::require_data_len(data, required, "RGBA32")?;
                Self::copy_bytes_cancelable(&data[..required], cancel_token)
            }
            TextureFormatConst::ARGB32 => {
                Self::raw_argb32_cancelable(data, width, height, cancel_token)
            }
            TextureFormatConst::BGRA32 => {
                Self::raw_bgra32_cancelable(data, width, height, cancel_token)
            }
            TextureFormatConst::DXT5 => {
                Self::bc_to_rgba(data, width, height, 16, cancel_token, |blk, obuf, pitch| {
                    bcdec_rs::bc3(blk, obuf, pitch)
                })
            }
            TextureFormatConst::DXT3 => {
                Self::bc_to_rgba(data, width, height, 16, cancel_token, |blk, obuf, pitch| {
                    bcdec_rs::bc2(blk, obuf, pitch)
                })
            }
            TextureFormatConst::DXT1 => {
                Self::bc_to_rgba(data, width, height, 8, cancel_token, |blk, obuf, pitch| {
                    bcdec_rs::bc1(blk, obuf, pitch)
                })
            }
            TextureFormatConst::BC7 => {
                Self::bc_to_rgba(data, width, height, 16, cancel_token, |blk, obuf, pitch| {
                    bcdec_rs::bc7(blk, obuf, pitch)
                })
            }
            TextureFormatConst::BC4 => Self::bc4_to_rgba(data, width, height, cancel_token),
            TextureFormatConst::BC5 => Self::bc5_to_rgba(data, width, height, cancel_token),
            TextureFormatConst::BC6H => Self::bc6h_to_rgba(data, width, height, cancel_token),
            _ => Err(format!(
                "Texture format {} preview decoding not yet supported",
                format
            )),
        }
    }

    /// Save as PNG (for preview)
    #[allow(dead_code)]
    pub fn save_png(rgba: &[u8], width: u32, height: u32, out: &Path) -> Result<(), String> {
        Self::save_png_cancelable(rgba, width, height, out, None)
    }

    /// Save as PNG (for preview), with cancellation checks before and after encoding.
    pub fn save_png_cancelable(
        rgba: &[u8],
        width: u32,
        height: u32,
        out: &Path,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<(), String> {
        use image::{ImageBuffer, Rgba};
        Self::check_cancelled(cancel_token)?;
        let buf = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba.to_vec())
            .ok_or_else(|| "Failed to create image".to_string())?;
        Self::check_cancelled(cancel_token)?;
        buf.save(out).map_err(|e| format!("Save PNG: {}", e))?;
        Self::check_cancelled(cancel_token)?;
        Ok(())
    }

    pub fn save_tga_cancelable(
        rgba: &[u8],
        width: u32,
        height: u32,
        out: &Path,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<(), String> {
        use image::codecs::tga::TgaEncoder;
        use image::{ColorType, ImageEncoder};
        Self::check_cancelled(cancel_token)?;
        let required = Self::image_byte_len(width, height, 4)?;
        Self::require_data_len(rgba, required, "RGBA")?;
        let file = std::fs::File::create(out).map_err(|e| format!("Create TGA: {}", e))?;
        TgaEncoder::new(file)
            .write_image(&rgba[..required], width, height, ColorType::Rgba8.into())
            .map_err(|e| format!("Save TGA: {}", e))?;
        Self::check_cancelled(cancel_token)?;
        Ok(())
    }

    /// Encode an RGBA buffer as an in-memory PNG data URL.
    #[allow(dead_code)]
    pub fn rgba_to_png_data_url(rgba: &[u8], width: u32, height: u32) -> Result<String, String> {
        Self::rgba_to_png_data_url_cancelable(rgba, width, height, None)
    }

    /// Encode an RGBA buffer as an in-memory PNG data URL, with cancellation checks around encoding.
    pub fn rgba_to_png_data_url_cancelable(
        rgba: &[u8],
        width: u32,
        height: u32,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<String, String> {
        let png_bytes = Self::rgba_to_png_bytes_cancelable(rgba, width, height, cancel_token)?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(png_bytes);
        Ok(format!("data:image/png;base64,{}", encoded))
    }

    #[allow(dead_code)]
    fn rgba_to_png_bytes(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
        Self::rgba_to_png_bytes_cancelable(rgba, width, height, None)
    }

    fn rgba_to_png_bytes_cancelable(
        rgba: &[u8],
        width: u32,
        height: u32,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<Vec<u8>, String> {
        use image::codecs::png::PngEncoder;
        use image::{ColorType, ImageEncoder};
        Self::check_cancelled(cancel_token)?;
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(rgba, width, height, ColorType::Rgba8.into())
            .map_err(|e| format!("Encode PNG: {}", e))?;
        Self::check_cancelled(cancel_token)?;
        Ok(bytes)
    }

    fn image_byte_len(width: u32, height: u32, bytes_per_pixel: usize) -> Result<usize, String> {
        (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
            .ok_or_else(|| "Texture dimensions overflowed byte length".to_string())
    }

    fn require_data_len(data: &[u8], required: usize, label: &str) -> Result<(), String> {
        if data.len() < required {
            Err(format!(
                "Insufficient {} texture data: need {} bytes, got {}",
                label,
                required,
                data.len()
            ))
        } else {
            Ok(())
        }
    }

    /// Downscale RGBA with nearest-neighbor sampling for fast preview thumbnails.
    #[allow(dead_code)]
    pub fn downscale_rgba_nearest(
        rgba: &[u8],
        width: u32,
        height: u32,
        max_edge: u32,
    ) -> Result<(Vec<u8>, u32, u32), String> {
        Self::downscale_rgba_nearest_cancelable(rgba, width, height, max_edge, None)
    }

    /// Downscale RGBA with nearest-neighbor sampling for fast preview thumbnails.
    pub fn downscale_rgba_nearest_cancelable(
        rgba: &[u8],
        width: u32,
        height: u32,
        max_edge: u32,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<(Vec<u8>, u32, u32), String> {
        if width == 0 || height == 0 {
            return Err("Texture has invalid dimensions".to_string());
        }
        Self::check_cancelled(cancel_token)?;
        if max_edge == 0 || width <= max_edge && height <= max_edge {
            return Ok((
                Self::copy_bytes_cancelable(rgba, cancel_token)?,
                width,
                height,
            ));
        }

        let scale = max_edge as f32 / width.max(height) as f32;
        let target_width = ((width as f32 * scale).round() as u32).max(1);
        let target_height = ((height as f32 * scale).round() as u32).max(1);
        let mut out = vec![0u8; target_width as usize * target_height as usize * 4];

        for y in 0..target_height {
            if y % 32 == 0 {
                Self::check_cancelled(cancel_token)?;
            }
            let src_y = ((y as u64 * height as u64) / target_height as u64) as u32;
            for x in 0..target_width {
                let src_x = ((x as u64 * width as u64) / target_width as u64) as u32;
                let src = ((src_y * width + src_x) * 4) as usize;
                let dst = ((y * target_width + x) * 4) as usize;
                out[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
            }
        }

        Ok((out, target_width, target_height))
    }

    /// Flip an RGBA buffer vertically, preserving row order within each row.
    #[allow(dead_code)]
    pub fn flip_rgba_vertical(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
        Self::flip_rgba_vertical_cancelable(rgba, width, height, None)
    }

    /// Flip an RGBA buffer vertically with cancellation support.
    pub fn flip_rgba_vertical_cancelable(
        rgba: &[u8],
        width: u32,
        height: u32,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<Vec<u8>, String> {
        if width == 0 || height == 0 {
            return Err("Texture has invalid dimensions".to_string());
        }
        let required = Self::image_byte_len(width, height, 4)?;
        Self::require_data_len(rgba, required, "RGBA")?;
        Self::check_cancelled(cancel_token)?;

        let row_stride = width as usize * 4;
        let mut out = vec![0u8; required];
        for y in 0..height as usize {
            if y % 32 == 0 {
                Self::check_cancelled(cancel_token)?;
            }
            let src = y * row_stride;
            let dst = (height as usize - 1 - y) * row_stride;
            out[dst..dst + row_stride].copy_from_slice(&rgba[src..src + row_stride]);
        }
        Self::check_cancelled(cancel_token)?;
        Ok(out)
    }

    // ================================================================
    // Internal: raw format ->RGBA (for PNG preview)
    // ================================================================

    #[allow(dead_code)]
    fn raw_rgb24(data: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
        Self::raw_rgb24_cancelable(data, w, h, None)
    }

    fn raw_rgb24_cancelable(
        data: &[u8],
        w: u32,
        h: u32,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<Vec<u8>, String> {
        let required = Self::image_byte_len(w, h, 3)?;
        Self::require_data_len(data, required, "RGB24")?;
        let n = (w * h) as usize;
        let mut out = Vec::with_capacity(n * 4);
        for (index, c) in data[..required].chunks(3).take(n).enumerate() {
            if index % 65536 == 0 {
                Self::check_cancelled(cancel_token)?;
            }
            out.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
        Ok(out)
    }

    #[allow(dead_code)]
    fn raw_argb32(data: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
        Self::raw_argb32_cancelable(data, w, h, None)
    }

    fn raw_argb32_cancelable(
        data: &[u8],
        w: u32,
        h: u32,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<Vec<u8>, String> {
        let required = Self::image_byte_len(w, h, 4)?;
        Self::require_data_len(data, required, "ARGB32")?;
        let mut out = Vec::with_capacity(required);
        for (index, c) in data[..required].chunks(4).enumerate() {
            if index % 65536 == 0 {
                Self::check_cancelled(cancel_token)?;
            }
            out.extend_from_slice(&[c[1], c[2], c[3], c[0]]);
        }
        Ok(out)
    }

    #[allow(dead_code)]
    fn raw_bgra32(data: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
        Self::raw_bgra32_cancelable(data, w, h, None)
    }

    fn raw_bgra32_cancelable(
        data: &[u8],
        w: u32,
        h: u32,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<Vec<u8>, String> {
        let required = Self::image_byte_len(w, h, 4)?;
        Self::require_data_len(data, required, "BGRA32")?;
        let mut out = Vec::with_capacity(required);
        for (index, c) in data[..required].chunks(4).enumerate() {
            if index % 65536 == 0 {
                Self::check_cancelled(cancel_token)?;
            }
            out.extend_from_slice(&[c[2], c[1], c[0], c[3]]);
        }
        Ok(out)
    }

    // ================================================================
    // Internal: BC compressed ->RGBA
    // ================================================================

    fn bc_to_rgba<F>(
        data: &[u8],
        w: u32,
        h: u32,
        block_size: usize,
        cancel_token: Option<&Arc<AtomicBool>>,
        decode_fn: F,
    ) -> Result<Vec<u8>, String>
    where
        F: Fn(&[u8], &mut [u8], usize),
    {
        let width = w as usize;
        let height = h as usize;
        let len = Self::image_byte_len(w, h, 4)?;
        let mut rgba = vec![0u8; len];
        let bwx = (width + 3) / 4;
        let bwy = (height + 3) / 4;
        let stride = width * 4;
        for by in 0..bwy {
            Self::check_cancelled(cancel_token)?;
            for bx in 0..bwx {
                let idx = (by * bwx + bx) * block_size;
                if idx + block_size > data.len() {
                    return Err("Insufficient texture data".to_string());
                }
                let mut block = [0u8; 64];
                decode_fn(&data[idx..idx + block_size], &mut block, 16);
                Self::copy_bc_block(&mut rgba, &block, width, height, bx, by, stride);
            }
        }
        Ok(rgba)
    }

    fn copy_bc_block(
        rgba: &mut [u8],
        block: &[u8],
        width: usize,
        height: usize,
        bx: usize,
        by: usize,
        stride: usize,
    ) {
        let max_rows = (height - by * 4).min(4);
        let max_cols = (width - bx * 4).min(4);
        for row in 0..max_rows {
            let dst = (by * 4 + row) * stride + bx * 4 * 4;
            let src = row * 16;
            rgba[dst..dst + max_cols * 4].copy_from_slice(&block[src..src + max_cols * 4]);
        }
    }

    fn bc4_to_rgba(
        data: &[u8],
        w: u32,
        h: u32,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<Vec<u8>, String> {
        let width = w as usize;
        let height = h as usize;
        let len = Self::image_byte_len(w, h, 4)?;
        let mut rgba = vec![0u8; len];
        let bwx = (width + 3) / 4;
        let bwy = (height + 3) / 4;
        let stride = width * 4;
        for by in 0..bwy {
            Self::check_cancelled(cancel_token)?;
            for bx in 0..bwx {
                let idx = (by * bwx + bx) * 8;
                if idx + 8 > data.len() {
                    return Err("Insufficient BC4 data".to_string());
                }
                let mut temp = [0u8; 16];
                bcdec_rs::bc4(&data[idx..idx + 8], &mut temp, 4, false);
                let mut block = [0u8; 64];
                for row in 0..4 {
                    for col in 0..4 {
                        let r = temp[row * 4 + col];
                        let di = row * 16 + col * 4;
                        block[di] = r;
                        block[di + 1] = r;
                        block[di + 2] = r;
                        block[di + 3] = 255;
                    }
                }
                Self::copy_bc_block(&mut rgba, &block, width, height, bx, by, stride);
            }
        }
        Ok(rgba)
    }

    fn bc5_to_rgba(
        data: &[u8],
        w: u32,
        h: u32,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<Vec<u8>, String> {
        let width = w as usize;
        let height = h as usize;
        let len = Self::image_byte_len(w, h, 4)?;
        let mut rgba = vec![0u8; len];
        let bwx = (width + 3) / 4;
        let bwy = (height + 3) / 4;
        let stride = width * 4;
        for by in 0..bwy {
            Self::check_cancelled(cancel_token)?;
            for bx in 0..bwx {
                let idx = (by * bwx + bx) * 16;
                if idx + 16 > data.len() {
                    return Err("Insufficient BC5 data".to_string());
                }
                let mut temp = [0u8; 32];
                bcdec_rs::bc5(&data[idx..idx + 16], &mut temp, 8, false);
                let mut block = [0u8; 64];
                for row in 0..4 {
                    for col in 0..4 {
                        let si = row * 8 + col * 2;
                        let di = row * 16 + col * 4;
                        block[di] = temp[si];
                        block[di + 1] = temp[si + 1];
                        block[di + 2] = 0;
                        block[di + 3] = 255;
                    }
                }
                Self::copy_bc_block(&mut rgba, &block, width, height, bx, by, stride);
            }
        }
        Ok(rgba)
    }

    fn bc6h_to_rgba(
        data: &[u8],
        w: u32,
        h: u32,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<Vec<u8>, String> {
        let width = w as usize;
        let height = h as usize;
        let len = Self::image_byte_len(w, h, 4)?;
        let mut rgba = vec![0u8; len];
        let bwx = (width + 3) / 4;
        let bwy = (height + 3) / 4;
        let stride = width * 4;
        for by in 0..bwy {
            Self::check_cancelled(cancel_token)?;
            for bx in 0..bwx {
                let idx = (by * bwx + bx) * 16;
                if idx + 16 > data.len() {
                    return Err("Insufficient BC6H data".to_string());
                }
                let mut temp = [0.0f32; 48];
                bcdec_rs::bc6h_float(&data[idx..idx + 16], &mut temp, 12, false);
                let mut block = [0u8; 64];
                for row in 0..4 {
                    for col in 0..4 {
                        let si = row * 12 + col * 3;
                        let di = row * 16 + col * 4;
                        block[di] = (temp[si] * 255.0).clamp(0.0, 255.0) as u8;
                        block[di + 1] = (temp[si + 1] * 255.0).clamp(0.0, 255.0) as u8;
                        block[di + 2] = (temp[si + 2] * 255.0).clamp(0.0, 255.0) as u8;
                        block[di + 3] = 255;
                    }
                }
                Self::copy_bc_block(&mut rgba, &block, width, height, bx, by, stride);
            }
        }
        Ok(rgba)
    }

    fn check_cancelled(cancel_token: Option<&Arc<AtomicBool>>) -> Result<(), String> {
        if cancel_token.is_some_and(|token| token.load(Ordering::SeqCst)) {
            Err("Task cancelled".to_string())
        } else {
            Ok(())
        }
    }

    fn copy_bytes_cancelable(
        data: &[u8],
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<Vec<u8>, String> {
        const CHUNK_SIZE: usize = 1024 * 1024;
        let mut out = Vec::with_capacity(data.len());
        for chunk in data.chunks(CHUNK_SIZE) {
            Self::check_cancelled(cancel_token)?;
            out.extend_from_slice(chunk);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::TextureExporter;
    use crate::unity::classes::texture_format::TextureFormatConst;

    #[test]
    fn rgb24_decode_rejects_truncated_data_instead_of_panicking() {
        let result =
            TextureExporter::decode_pixels_to_rgba(&[255, 0], TextureFormatConst::RGB24, 1, 1);

        assert!(result.is_err());
    }

    #[test]
    fn bc_decoder_handles_non_block_aligned_edges_without_panicking() {
        let result =
            TextureExporter::decode_pixels_to_rgba(&[0; 8], TextureFormatConst::DXT1, 1, 1)
                .expect("1x1 DXT1 should decode from one block");

        assert_eq!(result.len(), 4);
    }

    #[test]
    fn display_decode_flips_unity_texture_rows_vertically() {
        let data = vec![
            10, 0, 0, 255, 20, 0, 0, 255, //
            30, 0, 0, 255, 40, 0, 0, 255,
        ];

        let raw = TextureExporter::decode_pixels_to_rgba(&data, TextureFormatConst::RGBA32, 2, 2)
            .expect("raw decode");
        let result =
            TextureExporter::decode_pixels_to_display_rgba(&data, TextureFormatConst::RGBA32, 2, 2)
                .expect("display decode");

        assert_eq!(raw[0], 10);
        assert_eq!(raw[4], 20);
        assert_eq!(raw[8], 30);
        assert_eq!(raw[12], 40);
        assert_eq!(result[0], 30);
        assert_eq!(result[4], 40);
        assert_eq!(result[8], 10);
        assert_eq!(result[12], 20);
    }

    #[test]
    fn save_tga_preserves_display_oriented_rows() {
        let path = std::env::temp_dir().join(format!(
            "assetfinder_texture_display_{}.tga",
            std::process::id()
        ));
        let rgba = vec![
            30, 0, 0, 255, 40, 0, 0, 255, //
            10, 0, 0, 255, 20, 0, 0, 255,
        ];

        TextureExporter::save_tga_cancelable(&rgba, 2, 2, &path, None).expect("save tga");

        let decoded = image::open(&path).expect("open tga").to_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0[0], 30);
        assert_eq!(decoded.get_pixel(1, 0).0[0], 40);
        assert_eq!(decoded.get_pixel(0, 1).0[0], 10);
        assert_eq!(decoded.get_pixel(1, 1).0[0], 20);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn vertical_flip_rejects_truncated_rgba() {
        let result = TextureExporter::flip_rgba_vertical(&[0; 12], 2, 2);

        assert!(result.is_err());
    }
}
