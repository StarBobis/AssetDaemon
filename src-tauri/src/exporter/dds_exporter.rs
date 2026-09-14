/*
 * dds_exporter.rs -DDS (DirectDraw Surface) lossless export utility class.
 *
 * Unity internally stores textures as GPU native compressed formats (DXT1-5, BC4-7),
 * which are essentially headerless DDS data. This utility class writes the standard DDS header,
 * achieving lossless, zero-decode/re-encode texture export.
 *
 * DDS format specification: https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dx-graphics-dds-pguide
 *
 * Follows Soul.md convention: struct + impl organization, no free functions.
 */

use crate::unity::classes::texture_format::TextureFormatConst;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ================================================================
// DDS Constants
// ================================================================
const DDS_MAGIC: u32 = 0x20534444;
const DDS_HEADER_SIZE: u32 = 124;
const DDSD_CAPS: u32 = 0x1;
const DDSD_HEIGHT: u32 = 0x2;
const DDSD_WIDTH: u32 = 0x4;
const DDSD_PIXELFORMAT: u32 = 0x1000;
const DDSD_MIPMAPCOUNT: u32 = 0x20000;
const DDSD_LINEARSIZE: u32 = 0x80000;
const DDSD_PITCH: u32 = 0x8;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGBA: u32 = 0x41;
const DDSCAPS_TEXTURE: u32 = 0x1000;
const DDSCAPS_MIPMAP: u32 = 0x400008;
const DXGI_BC4_UNORM: u32 = 80;
const DXGI_BC5_UNORM: u32 = 83;
const DXGI_BC6H_UF16: u32 = 95;
const DXGI_BC7_UNORM: u32 = 98;

pub struct DDSExporter;

impl DDSExporter {
    /// Writes Unity compressed texture data directly to a DDS file.
    ///
    /// - DXT1/DXT3/DXT5 ->legacy FourCC (compatible with all DDS tools)
    /// - BC4/BC5/BC6H/BC7 ->"DX10" FourCC + DDS_HEADER_DXT10 extension
    /// - Uncompressed (RGBA32 etc.) ->write DDPF_RGBA pixel format directly
    #[allow(dead_code)]
    pub fn export(
        data: &[u8],
        format: i32,
        width: u32,
        height: u32,
        mip_count: i32,
        out: &Path,
    ) -> Result<u64, String> {
        Self::export_cancelable(data, format, width, height, mip_count, out, None)
    }

    pub fn export_cancelable(
        data: &[u8],
        format: i32,
        width: u32,
        height: u32,
        mip_count: i32,
        out: &Path,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<u64, String> {
        Self::check_cancelled(cancel_token)?;
        let mut f = std::fs::File::create(out).map_err(|e| format!("create DDS: {}", e))?;

        let has_mips = mip_count > 1;
        let mips = if has_mips { mip_count as u32 } else { 1u32 };

        let (is_compressed, is_dx10, block_size, dxgi_format, legacy_fourcc): (
            bool,
            bool,
            u32,
            u32,
            [u8; 4],
        ) = match format {
            TextureFormatConst::DXT1 => (true, false, 8, 0, *b"DXT1"),
            TextureFormatConst::DXT3 => (true, false, 16, 0, *b"DXT3"),
            TextureFormatConst::DXT5 => (true, false, 16, 0, *b"DXT5"),
            TextureFormatConst::BC4 => (true, true, 8, DXGI_BC4_UNORM, [0u8; 4]),
            TextureFormatConst::BC5 => (true, true, 16, DXGI_BC5_UNORM, [0u8; 4]),
            TextureFormatConst::BC6H => (true, true, 16, DXGI_BC6H_UF16, [0u8; 4]),
            TextureFormatConst::BC7 => (true, true, 16, DXGI_BC7_UNORM, [0u8; 4]),
            _ => (false, false, 0, 0, [0u8; 4]),
        };

        let pitch_or_linear_size: u32 = if is_compressed {
            let bw = (width.max(1) + 3) / 4;
            let bh = (height.max(1) + 3) / 4;
            bw * bh * block_size
        } else {
            let row_bytes = width * 4;
            (row_bytes + 3) & !3
        };

        let flags = DDSD_CAPS
            | DDSD_HEIGHT
            | DDSD_WIDTH
            | DDSD_PIXELFORMAT
            | DDSD_LINEARSIZE
            | if has_mips { DDSD_MIPMAPCOUNT } else { 0 };

        let caps1 = DDSCAPS_TEXTURE | if has_mips { DDSCAPS_MIPMAP } else { 0 };

        // ---- DDS Magic + Header (128 bytes) ----
        f.write_all(&DDS_MAGIC.to_le_bytes())
            .map_err(|e| format!("magic: {}", e))?;
        f.write_all(&DDS_HEADER_SIZE.to_le_bytes())
            .map_err(|e| format!("size: {}", e))?;
        f.write_all(&flags.to_le_bytes())
            .map_err(|e| format!("flags: {}", e))?;
        f.write_all(&height.to_le_bytes())
            .map_err(|e| format!("h: {}", e))?;
        f.write_all(&width.to_le_bytes())
            .map_err(|e| format!("w: {}", e))?;
        f.write_all(&pitch_or_linear_size.to_le_bytes())
            .map_err(|e| format!("pitch: {}", e))?;
        f.write_all(&1u32.to_le_bytes())
            .map_err(|e| format!("depth: {}", e))?;
        f.write_all(&mips.to_le_bytes())
            .map_err(|e| format!("mips: {}", e))?;
        f.write_all(&[0u8; 44])
            .map_err(|e| format!("reserved: {}", e))?;

        // ---- PixelFormat (32 bytes) ----
        let pf_fourcc: [u8; 4] = if is_dx10 {
            *b"DX10"
        } else if is_compressed {
            legacy_fourcc
        } else {
            [0u8; 4]
        };
        let pf_flags = if is_dx10 || is_compressed {
            DDPF_FOURCC
        } else {
            DDPF_RGBA
        };
        let pf_rgb_bitcount = if pf_flags == DDPF_RGBA { 32u32 } else { 0u32 };

        f.write_all(&32u32.to_le_bytes())
            .map_err(|e| format!("pfSize: {}", e))?;
        f.write_all(&pf_flags.to_le_bytes())
            .map_err(|e| format!("pfFlags: {}", e))?;
        f.write_all(&pf_fourcc)
            .map_err(|e| format!("fourCC: {}", e))?;
        f.write_all(&pf_rgb_bitcount.to_le_bytes())
            .map_err(|e| format!("rgbBC: {}", e))?;
        f.write_all(&0u32.to_le_bytes())
            .map_err(|e| format!("rMask: {}", e))?;
        f.write_all(&0u32.to_le_bytes())
            .map_err(|e| format!("gMask: {}", e))?;
        f.write_all(&0u32.to_le_bytes())
            .map_err(|e| format!("bMask: {}", e))?;
        f.write_all(&0u32.to_le_bytes())
            .map_err(|e| format!("aMask: {}", e))?;

        // ---- Caps ----
        f.write_all(&caps1.to_le_bytes())
            .map_err(|e| format!("caps1: {}", e))?;
        f.write_all(&0u32.to_le_bytes())
            .map_err(|e| format!("caps2: {}", e))?;
        f.write_all(&0u32.to_le_bytes())
            .map_err(|e| format!("caps3: {}", e))?;
        f.write_all(&0u32.to_le_bytes())
            .map_err(|e| format!("caps4: {}", e))?;
        f.write_all(&0u32.to_le_bytes())
            .map_err(|e| format!("reserved2: {}", e))?;

        // ---- DX10 extension header (BC4-BC7) ----
        if is_dx10 {
            f.write_all(&dxgi_format.to_le_bytes())
                .map_err(|e| format!("dxgiFmt: {}", e))?;
            f.write_all(&3u32.to_le_bytes())
                .map_err(|e| format!("dim: {}", e))?;
            f.write_all(&0u32.to_le_bytes())
                .map_err(|e| format!("misc: {}", e))?;
            f.write_all(&1u32.to_le_bytes())
                .map_err(|e| format!("arrSize: {}", e))?;
            f.write_all(&0u32.to_le_bytes())
                .map_err(|e| format!("misc2: {}", e))?;
        }

        Self::check_cancelled(cancel_token)?;

        // ---- Data ----
        Self::write_all_cancelable(&mut f, data, cancel_token)?;
        let size = std::fs::metadata(out)
            .map_err(|e| format!("stat: {}", e))?
            .len();
        Self::check_cancelled(cancel_token)?;
        Ok(size)
    }

    pub fn export_rgba8_cancelable(
        rgba: &[u8],
        width: u32,
        height: u32,
        out: &Path,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<u64, String> {
        Self::check_cancelled(cancel_token)?;
        let required = width as usize * height as usize * 4;
        if width == 0 || height == 0 || rgba.len() < required {
            return Err(format!(
                "Invalid RGBA DDS data: {}x{}, need {} bytes, got {}",
                width,
                height,
                required,
                rgba.len()
            ));
        }

        let mut f = std::fs::File::create(out).map_err(|e| format!("create DDS: {}", e))?;
        let pitch = width * 4;

        f.write_all(&DDS_MAGIC.to_le_bytes())
            .map_err(|e| format!("magic: {}", e))?;
        f.write_all(&DDS_HEADER_SIZE.to_le_bytes())
            .map_err(|e| format!("size: {}", e))?;
        f.write_all(
            &(DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT | DDSD_PITCH).to_le_bytes(),
        )
        .map_err(|e| format!("flags: {}", e))?;
        f.write_all(&height.to_le_bytes())
            .map_err(|e| format!("h: {}", e))?;
        f.write_all(&width.to_le_bytes())
            .map_err(|e| format!("w: {}", e))?;
        f.write_all(&pitch.to_le_bytes())
            .map_err(|e| format!("pitch: {}", e))?;
        f.write_all(&0u32.to_le_bytes())
            .map_err(|e| format!("depth: {}", e))?;
        f.write_all(&0u32.to_le_bytes())
            .map_err(|e| format!("mips: {}", e))?;
        f.write_all(&[0u8; 44])
            .map_err(|e| format!("reserved: {}", e))?;

        f.write_all(&32u32.to_le_bytes())
            .map_err(|e| format!("pfSize: {}", e))?;
        f.write_all(&DDPF_RGBA.to_le_bytes())
            .map_err(|e| format!("pfFlags: {}", e))?;
        f.write_all(&[0u8; 4])
            .map_err(|e| format!("fourCC: {}", e))?;
        f.write_all(&32u32.to_le_bytes())
            .map_err(|e| format!("rgbBC: {}", e))?;
        f.write_all(&0x0000_00FFu32.to_le_bytes())
            .map_err(|e| format!("rMask: {}", e))?;
        f.write_all(&0x0000_FF00u32.to_le_bytes())
            .map_err(|e| format!("gMask: {}", e))?;
        f.write_all(&0x00FF_0000u32.to_le_bytes())
            .map_err(|e| format!("bMask: {}", e))?;
        f.write_all(&0xFF00_0000u32.to_le_bytes())
            .map_err(|e| format!("aMask: {}", e))?;

        f.write_all(&DDSCAPS_TEXTURE.to_le_bytes())
            .map_err(|e| format!("caps1: {}", e))?;
        f.write_all(&0u32.to_le_bytes())
            .map_err(|e| format!("caps2: {}", e))?;
        f.write_all(&0u32.to_le_bytes())
            .map_err(|e| format!("caps3: {}", e))?;
        f.write_all(&0u32.to_le_bytes())
            .map_err(|e| format!("caps4: {}", e))?;
        f.write_all(&0u32.to_le_bytes())
            .map_err(|e| format!("reserved2: {}", e))?;

        Self::write_all_cancelable(&mut f, &rgba[..required], cancel_token)?;
        let size = std::fs::metadata(out)
            .map_err(|e| format!("stat: {}", e))?
            .len();
        Self::check_cancelled(cancel_token)?;
        Ok(size)
    }

    fn check_cancelled(cancel_token: Option<&Arc<AtomicBool>>) -> Result<(), String> {
        if cancel_token.is_some_and(|token| token.load(Ordering::SeqCst)) {
            Err("Task cancelled".to_string())
        } else {
            Ok(())
        }
    }

    fn write_all_cancelable(
        file: &mut std::fs::File,
        data: &[u8],
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<(), String> {
        const CHUNK_SIZE: usize = 1024 * 1024;
        for chunk in data.chunks(CHUNK_SIZE) {
            Self::check_cancelled(cancel_token)?;
            file.write_all(chunk).map_err(|e| format!("data: {}", e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::DDSExporter;

    #[test]
    fn rgba8_dds_writes_display_oriented_rows() {
        let path = std::env::temp_dir().join(format!(
            "assetfinder_texture_display_{}.dds",
            std::process::id()
        ));
        let rgba = vec![
            30, 0, 0, 255, 40, 0, 0, 255, //
            10, 0, 0, 255, 20, 0, 0, 255,
        ];

        DDSExporter::export_rgba8_cancelable(&rgba, 2, 2, &path, None).expect("dds");

        let bytes = std::fs::read(&path).expect("read dds");
        assert_eq!(&bytes[..4], b"DDS ");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 124);
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            0x0000_100F
        );
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(bytes[76..80].try_into().unwrap()), 32);
        assert_eq!(u32::from_le_bytes(bytes[80..84].try_into().unwrap()), 0x41);
        assert_eq!(u32::from_le_bytes(bytes[84..88].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(bytes[88..92].try_into().unwrap()), 32);
        assert_eq!(
            u32::from_le_bytes(bytes[92..96].try_into().unwrap()),
            0x0000_00FF
        );
        assert_eq!(
            u32::from_le_bytes(bytes[96..100].try_into().unwrap()),
            0x0000_FF00
        );
        assert_eq!(
            u32::from_le_bytes(bytes[100..104].try_into().unwrap()),
            0x00FF_0000
        );
        assert_eq!(
            u32::from_le_bytes(bytes[104..108].try_into().unwrap()),
            0xFF00_0000
        );
        assert_eq!(&bytes[128..136], &[30, 0, 0, 255, 40, 0, 0, 255]);
        assert_eq!(&bytes[136..144], &[10, 0, 0, 255, 20, 0, 0, 255]);

        let _ = std::fs::remove_file(path);
    }
}
