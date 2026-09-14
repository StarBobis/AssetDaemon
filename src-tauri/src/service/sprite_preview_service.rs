use crate::common::bundle_file::asset_bundle::AssetBundle;
use crate::common::command_types::TexturePreviewResult;
use crate::common::task::task_context::TaskContext;
use crate::exporter::texture_exporter::TextureExporter;
use crate::unity::classes::object::PPtr;
use crate::unity::classes::registry::UnityClassObject;
use crate::unity::classes::sprite::{RectF, SpriteObject, SpriteRenderData, SpriteSettings};
use crate::unity::classes::texture2d::Texture2DInfo;
use crate::unity::classes::texture_format::TextureFormatConst;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct SpritePreviewService;

impl SpritePreviewService {
    pub fn export_sprite_preview(
        ctx: &TaskContext,
        bundle: &AssetBundle,
        path_id: i64,
        max_preview_edge: Option<u32>,
    ) -> Result<TexturePreviewResult, String> {
        let cancel_token = ctx.cancel_token();
        ctx.progress("Parse Sprite", 2, 5, "Parsing Sprite data...");
        let sprite = Self::parse_sprite(bundle, path_id)?;
        let render_data = Self::resolve_sprite_render_data(bundle, &sprite)?;
        Self::export_render_data(
            ctx,
            Some(&cancel_token),
            bundle,
            path_id,
            "sprite",
            &render_data,
            max_preview_edge,
        )
    }

    pub fn export_sprite_mask_preview(
        ctx: &TaskContext,
        bundle: &AssetBundle,
        path_id: i64,
        max_preview_edge: Option<u32>,
    ) -> Result<TexturePreviewResult, String> {
        let cancel_token = ctx.cancel_token();
        ctx.progress("Parse SpriteMask", 2, 5, "Parsing SpriteMask data...");
        let mask = match bundle.parse_object_by_path_id(path_id)? {
            UnityClassObject::SpriteMask(mask) => mask,
            other => {
                return Err(format!(
                    "Object path_id={} is {}, not SpriteMask",
                    path_id,
                    other.class_name()
                ))
            }
        };
        let sprite = mask
            .sprite
            .ok_or_else(|| format!("SpriteMask path_id={} has no m_Sprite reference", path_id))?;
        if sprite.file_id != 0 {
            return Err(format!(
                "SpriteMask m_Sprite uses external file_id={}, external Sprite preview is not supported yet",
                sprite.file_id
            ));
        }
        let sprite = Self::parse_sprite(bundle, sprite.path_id)?;
        let render_data = Self::resolve_sprite_render_data(bundle, &sprite)?;
        Self::export_render_data(
            ctx,
            Some(&cancel_token),
            bundle,
            path_id,
            "sprite_mask",
            &render_data,
            max_preview_edge,
        )
    }

    pub fn export_sprite_png(
        bundle: &AssetBundle,
        path_id: i64,
        output_path: &std::path::Path,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<u64, String> {
        Self::check_cancelled(cancel_token)?;
        let sprite = Self::parse_sprite(bundle, path_id)?;
        let render_data = Self::resolve_sprite_render_data(bundle, &sprite)?;
        Self::export_render_data_to_png(bundle, &render_data, output_path, cancel_token)
    }

    pub fn export_sprite_mask_png(
        bundle: &AssetBundle,
        path_id: i64,
        output_path: &std::path::Path,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<u64, String> {
        Self::check_cancelled(cancel_token)?;
        let mask = match bundle.parse_object_by_path_id(path_id)? {
            UnityClassObject::SpriteMask(mask) => mask,
            other => {
                return Err(format!(
                    "Object path_id={} is {}, not SpriteMask",
                    path_id,
                    other.class_name()
                ))
            }
        };
        let sprite = mask
            .sprite
            .ok_or_else(|| format!("SpriteMask path_id={} has no m_Sprite reference", path_id))?;
        if sprite.file_id != 0 {
            return Err(format!(
                "SpriteMask m_Sprite uses external file_id={}, external Sprite export is not supported yet",
                sprite.file_id
            ));
        }
        let sprite = Self::parse_sprite(bundle, sprite.path_id)?;
        let render_data = Self::resolve_sprite_render_data(bundle, &sprite)?;
        Self::export_render_data_to_png(bundle, &render_data, output_path, cancel_token)
    }

    fn parse_sprite(bundle: &AssetBundle, path_id: i64) -> Result<SpriteObject, String> {
        match bundle.parse_object_by_path_id(path_id)? {
            UnityClassObject::Sprite(sprite) => Ok(sprite),
            other => Err(format!(
                "Object path_id={} is {}, not Sprite",
                path_id,
                other.class_name()
            )),
        }
    }

    fn resolve_sprite_render_data(
        bundle: &AssetBundle,
        sprite: &SpriteObject,
    ) -> Result<SpriteRenderData, String> {
        if let (Some(atlas_ref), Some(key)) = (sprite.sprite_atlas, sprite.render_data_key.as_ref())
        {
            if !atlas_ref.is_null() && atlas_ref.file_id == 0 {
                if let Ok(UnityClassObject::SpriteAtlas(atlas)) =
                    bundle.parse_object_by_path_id(atlas_ref.path_id)
                {
                    if let Some(entry) =
                        atlas.render_data_map.iter().find(|entry| &entry.key == key)
                    {
                        return Ok(entry.data.clone());
                    }
                }
            }
        }

        sprite
            .render_data
            .clone()
            .ok_or_else(|| "Sprite render data is unavailable".to_string())
    }

    fn export_render_data(
        ctx: &TaskContext,
        cancel_token: Option<&Arc<AtomicBool>>,
        bundle: &AssetBundle,
        source_path_id: i64,
        label: &str,
        render_data: &SpriteRenderData,
        max_preview_edge: Option<u32>,
    ) -> Result<TexturePreviewResult, String> {
        Self::check_cancelled(cancel_token)?;
        if render_data.texture.is_null() {
            return Err("Sprite render data has no Texture2D reference".to_string());
        }
        if render_data.texture.file_id != 0 {
            return Err(format!(
                "Sprite texture uses external file_id={}, external texture preview is not supported yet",
                render_data.texture.file_id
            ));
        }

        ctx.progress("Parse Texture", 3, 5, "Parsing Sprite Texture2D data...");
        let texture = Self::parse_texture(bundle, render_data.texture)?;

        Self::check_cancelled(cancel_token)?;
        ctx.progress("Decode Texture", 4, 5, "Decoding Sprite texture pixels...");
        let rgba = TextureExporter::decode_pixels_to_rgba_cancelable(
            &texture.pixel_data,
            texture.texture_format,
            texture.width,
            texture.height,
            cancel_token,
        )?;
        let sprite_rgba = Self::crop_sprite_rgba(
            &rgba,
            texture.width,
            texture.height,
            render_data.texture_rect,
            render_data.downscale_multiplier,
            render_data.settings,
            cancel_token,
        )?;

        Self::check_cancelled(cancel_token)?;
        ctx.progress("Encode Preview", 5, 5, "Encoding Sprite PNG preview...");
        let max_edge = max_preview_edge.unwrap_or(2048).clamp(32, 4096);
        let (preview_rgba, preview_width, preview_height) =
            TextureExporter::downscale_rgba_nearest_cancelable(
                &sprite_rgba.pixels,
                sprite_rgba.width,
                sprite_rgba.height,
                max_edge,
                cancel_token,
            )?;
        let png_data_url = TextureExporter::rgba_to_png_data_url_cancelable(
            &preview_rgba,
            preview_width,
            preview_height,
            cancel_token,
        )?;

        Ok(TexturePreviewResult {
            png_path: format!("memory://{}_{}.png", label, source_path_id),
            png_data_url,
            preview_width,
            preview_height,
            width: sprite_rgba.width,
            height: sprite_rgba.height,
            texture_format: texture.texture_format,
            texture_format_name: TextureFormatConst::format_name(texture.texture_format)
                .to_string(),
            complete_image_size: texture.complete_image_size,
            mip_count: texture.mip_count,
            image_count: texture.image_count,
            texture_dimension: texture.texture_dimension,
            texture_dimension_name: TextureFormatConst::dimension_name(texture.texture_dimension)
                .to_string(),
            byte_size: sprite_rgba.pixels.len() as u32,
            has_alpha: TextureFormatConst::has_alpha(texture.texture_format),
            color_space: texture.color_space,
            color_space_name: TextureFormatConst::color_space_name(texture.color_space).to_string(),
            filter_mode: texture.filter_mode,
            filter_mode_name: TextureFormatConst::filter_mode_name(texture.filter_mode).to_string(),
            aniso: texture.aniso,
            wrap_u: texture.wrap_u,
            wrap_v: texture.wrap_v,
            wrap_mode_name: TextureFormatConst::wrap_mode_name(texture.wrap_u).to_string(),
            is_readable: texture.is_readable,
            streaming_mipmaps: texture.streaming_mipmaps,
        })
    }

    fn parse_texture(bundle: &AssetBundle, texture_ref: PPtr) -> Result<Texture2DInfo, String> {
        match bundle.parse_object_by_path_id(texture_ref.path_id)? {
            UnityClassObject::Texture2D(texture) => Ok(texture.data),
            other => Err(format!(
                "Sprite texture path_id={} is {}, not Texture2D",
                texture_ref.path_id,
                other.class_name()
            )),
        }
    }

    fn export_render_data_to_png(
        bundle: &AssetBundle,
        render_data: &SpriteRenderData,
        output_path: &std::path::Path,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<u64, String> {
        Self::check_cancelled(cancel_token)?;
        if render_data.texture.is_null() {
            return Err("Sprite render data has no Texture2D reference".to_string());
        }
        if render_data.texture.file_id != 0 {
            return Err(format!(
                "Sprite texture uses external file_id={}, external texture export is not supported yet",
                render_data.texture.file_id
            ));
        }
        let texture = Self::parse_texture(bundle, render_data.texture)?;
        let rgba = TextureExporter::decode_pixels_to_rgba_cancelable(
            &texture.pixel_data,
            texture.texture_format,
            texture.width,
            texture.height,
            cancel_token,
        )?;
        let sprite_rgba = Self::crop_sprite_rgba(
            &rgba,
            texture.width,
            texture.height,
            render_data.texture_rect,
            render_data.downscale_multiplier,
            render_data.settings,
            cancel_token,
        )?;
        TextureExporter::save_png_cancelable(
            &sprite_rgba.pixels,
            sprite_rgba.width,
            sprite_rgba.height,
            output_path,
            cancel_token,
        )?;
        Self::check_cancelled(cancel_token)?;
        Ok(std::fs::metadata(output_path)
            .map_err(|e| format!("stat: {}", e))?
            .len())
    }

    fn crop_sprite_rgba(
        rgba: &[u8],
        texture_width: u32,
        texture_height: u32,
        rect: RectF,
        downscale_multiplier: f32,
        settings: SpriteSettings,
        cancel_token: Option<&Arc<AtomicBool>>,
    ) -> Result<SpriteRgba, String> {
        let scale = if downscale_multiplier.is_finite() && downscale_multiplier > 0.0 {
            downscale_multiplier
        } else {
            1.0
        };
        let scaled_texture_width = ((texture_width as f32 / scale) as u32).max(1);
        let scaled_texture_height = ((texture_height as f32 / scale) as u32).max(1);
        let rect_x = rect.x.floor().max(0.0) as u32;
        let rect_y = rect.y.floor().max(0.0) as u32;
        let rect_right = (rect.x + rect.width).ceil().max(0.0) as u32;
        let rect_bottom = (rect.y + rect.height).ceil().max(0.0) as u32;

        let mut left = rect_x.min(scaled_texture_width);
        let mut top = rect_y.min(scaled_texture_height);
        let mut right = rect_right.min(scaled_texture_width);
        let mut bottom = rect_bottom.min(scaled_texture_height);
        if right <= left || bottom <= top {
            if rect.width <= 0.0 || rect.height <= 0.0 {
                // Some bundles store a zero-area sprite textureRect (e.g. full-texture
                // mesh sprites). Fall back to the whole texture so the preview still renders.
                left = 0;
                top = 0;
                right = scaled_texture_width;
                bottom = scaled_texture_height;
            } else {
                return Err(format!(
                    "Sprite textureRect is outside texture bounds: rect=({}, {}, {}, {}), texture={}x{}",
                    rect.x, rect.y, rect.width, rect.height, texture_width, texture_height
                ));
            }
        }

        let width = right - left;
        let height = bottom - top;
        let mut out = vec![0u8; width as usize * height as usize * 4];
        for y in 0..height {
            if y % 32 == 0 {
                Self::check_cancelled(cancel_token)?;
            }
            let src_y = (((top + y) as f32 * scale).floor() as u32).min(texture_height - 1);
            for x in 0..width {
                let src_x = (((left + x) as f32 * scale).floor() as u32).min(texture_width - 1);
                let src = ((src_y * texture_width + src_x) * 4) as usize;
                let dst = ((y * width + x) * 4) as usize;
                out[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
            }
        }

        let cropped = SpriteRgba {
            pixels: out,
            width,
            height,
        };
        let rotated = if settings.packed {
            Self::apply_packing_rotation(cropped, settings.packing_rotation)
        } else {
            cropped
        };
        Self::check_cancelled(cancel_token)?;
        Ok(Self::flip_vertical(rotated))
    }

    fn apply_packing_rotation(image: SpriteRgba, rotation: u32) -> SpriteRgba {
        match rotation {
            1 => Self::flip_horizontal(image),
            2 => Self::flip_vertical(image),
            3 => Self::rotate_180(image),
            4 => Self::rotate_270(image),
            _ => image,
        }
    }

    fn flip_horizontal(image: SpriteRgba) -> SpriteRgba {
        let mut out = vec![0u8; image.pixels.len()];
        for y in 0..image.height {
            for x in 0..image.width {
                copy_pixel(
                    &image.pixels,
                    &mut out,
                    image.width,
                    x,
                    y,
                    image.width - 1 - x,
                    y,
                );
            }
        }
        SpriteRgba {
            pixels: out,
            ..image
        }
    }

    fn flip_vertical(image: SpriteRgba) -> SpriteRgba {
        let mut out = vec![0u8; image.pixels.len()];
        for y in 0..image.height {
            for x in 0..image.width {
                copy_pixel(
                    &image.pixels,
                    &mut out,
                    image.width,
                    x,
                    y,
                    x,
                    image.height - 1 - y,
                );
            }
        }
        SpriteRgba {
            pixels: out,
            ..image
        }
    }

    fn rotate_180(image: SpriteRgba) -> SpriteRgba {
        Self::flip_vertical(Self::flip_horizontal(image))
    }

    fn rotate_270(image: SpriteRgba) -> SpriteRgba {
        let mut out = vec![0u8; image.pixels.len()];
        let new_width = image.height;
        let new_height = image.width;
        for y in 0..image.height {
            for x in 0..image.width {
                let dst_x = y;
                let dst_y = image.width - 1 - x;
                copy_pixel_with_widths(
                    &image.pixels,
                    &mut out,
                    image.width,
                    new_width,
                    x,
                    y,
                    dst_x,
                    dst_y,
                );
            }
        }
        SpriteRgba {
            pixels: out,
            width: new_width,
            height: new_height,
        }
    }

    fn check_cancelled(cancel_token: Option<&Arc<AtomicBool>>) -> Result<(), String> {
        if cancel_token.is_some_and(|token| token.load(Ordering::SeqCst)) {
            Err("Task cancelled".to_string())
        } else {
            Ok(())
        }
    }
}

struct SpriteRgba {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

fn copy_pixel(src: &[u8], dst: &mut [u8], width: u32, sx: u32, sy: u32, dx: u32, dy: u32) {
    copy_pixel_with_widths(src, dst, width, width, sx, sy, dx, dy)
}

fn copy_pixel_with_widths(
    src: &[u8],
    dst: &mut [u8],
    src_width: u32,
    dst_width: u32,
    sx: u32,
    sy: u32,
    dx: u32,
    dy: u32,
) {
    let src_index = ((sy * src_width + sx) * 4) as usize;
    let dst_index = ((dy * dst_width + dx) * 4) as usize;
    dst[dst_index..dst_index + 4].copy_from_slice(&src[src_index..src_index + 4]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unity::classes::sprite::SpriteSettings;

    #[test]
    fn sprite_crop_applies_unity_vertical_flip() {
        let rgba = vec![
            10, 0, 0, 255, 20, 0, 0, 255, //
            30, 0, 0, 255, 40, 0, 0, 255,
        ];

        let cropped = SpritePreviewService::crop_sprite_rgba(
            &rgba,
            2,
            2,
            RectF {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 2.0,
            },
            1.0,
            SpriteSettings::from_raw(0),
            None,
        )
        .expect("crop");

        assert_eq!(cropped.width, 2);
        assert_eq!(cropped.height, 2);
        assert_eq!(cropped.pixels[0], 30);
        assert_eq!(cropped.pixels[4], 40);
        assert_eq!(cropped.pixels[8], 10);
        assert_eq!(cropped.pixels[12], 20);
    }

    #[test]
    fn sprite_crop_rejects_rect_outside_texture() {
        let result = SpritePreviewService::crop_sprite_rgba(
            &[0; 16],
            2,
            2,
            RectF {
                x: 4.0,
                y: 4.0,
                width: 1.0,
                height: 1.0,
            },
            1.0,
            SpriteSettings::from_raw(0),
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn sprite_crop_falls_back_to_full_texture_for_degenerate_rect() {
        // Some bundles (e.g. bundle156 control_wide_bg) store a zero-area sprite
        // textureRect (0.19, 0, 0, 0). The preview must fall back to the whole texture.
        let rgba = vec![255u8; 1100 * 66 * 4];

        let cropped = SpritePreviewService::crop_sprite_rgba(
            &rgba,
            1100,
            66,
            RectF {
                x: 0.19,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
            1.0,
            SpriteSettings::from_raw(0),
            None,
        )
        .expect("degenerate textureRect should fall back to full texture");

        assert_eq!(cropped.width, 1100);
        assert_eq!(cropped.height, 66);
        assert_eq!(cropped.pixels.len(), 1100 * 66 * 4);
    }

    #[test]
    fn sprite_crop_samples_scaled_texture_space() {
        let rgba = vec![
            10, 0, 0, 255, 20, 0, 0, 255, 30, 0, 0, 255, 40, 0, 0, 255, 50, 0, 0, 255, 60, 0, 0,
            255, 70, 0, 0, 255, 80, 0, 0, 255, 90, 0, 0, 255, 100, 0, 0, 255, 110, 0, 0, 255, 120,
            0, 0, 255, 130, 0, 0, 255, 140, 0, 0, 255, 150, 0, 0, 255, 160, 0, 0, 255,
        ];

        let cropped = SpritePreviewService::crop_sprite_rgba(
            &rgba,
            4,
            4,
            RectF {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 2.0,
            },
            2.0,
            SpriteSettings::from_raw(0),
            None,
        )
        .expect("crop");

        assert_eq!(cropped.width, 2);
        assert_eq!(cropped.height, 2);
        assert_eq!(cropped.pixels[0], 90);
        assert_eq!(cropped.pixels[4], 110);
        assert_eq!(cropped.pixels[8], 10);
        assert_eq!(cropped.pixels[12], 30);
    }
}
