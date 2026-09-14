use std::path::Path;

use crate::common::bundle_file::asset_bundle::AssetBundleLoader;
use crate::common::command_types::TexturePreviewResult;
use crate::common::task::task_context::TaskContext;
use crate::exporter::texture_exporter::TextureExporter;
use crate::service::sprite_preview_service::SpritePreviewService;
use crate::unity::classes::registry::UnityClassObject;
use crate::unity::classes::texture_format::TextureFormatConst;

pub struct TexturePreviewService;

impl TexturePreviewService {
    pub fn export_preview(
        ctx: &TaskContext,
        bundle_path: &str,
        path_id: &str,
        cache_dir: &str,
        max_preview_edge: Option<u32>,
    ) -> Result<TexturePreviewResult, String> {
        let cancel_token = ctx.cancel_token();
        let path_id: i64 = path_id
            .parse()
            .map_err(|e| format!("invalid path_id: {}", e))?;
        let bundle_path = Path::new(bundle_path);
        let cache_dir = Path::new(cache_dir);
        if !bundle_path.exists() {
            return Err(format!("file not found: {}", bundle_path.display()));
        }

        ctx.progress("Load Bundle", 1, 6, "Loading texture Unity file...");
        let bundle = AssetBundleLoader::load_unity_file(bundle_path)
            .map_err(|e| format!("failed to parse Unity file: {:?}", e))?;

        ctx.check_cancelled()?;

        ctx.progress("Find Texture", 2, 6, "Finding preview source object...");
        let (class_id, node_name) = bundle
            .assets
            .iter()
            .enumerate()
            .find_map(|(i, file)| {
                file.objects
                    .iter()
                    .find(|obj| obj.path_id == path_id)
                    .map(|obj| (obj.class_id, bundle.asset_names.get(i).cloned().flatten()))
            })
            .ok_or_else(|| format!("Object with path_id={} not found", path_id))?;

        if matches!(class_id, 171 | 213 | 264) {
            return SpritePreviewService::export_sprite_preview(
                ctx,
                &bundle,
                path_id,
                max_preview_edge,
            );
        }

        if matches!(class_id, 331) {
            return SpritePreviewService::export_sprite_mask_preview(
                ctx,
                &bundle,
                path_id,
                max_preview_edge,
            );
        }

        if class_id != 28 && class_id != 271 {
            return Err(format!(
                "Object path_id={} has class_id={}, not Texture2D/Sprite/SpriteMask",
                path_id, class_id
            ));
        }

        let base_name = node_name.unwrap_or_else(|| format!("tex_{}", path_id));
        let png_name = format!("{}_{}.png", base_name, path_id);
        let png_path = cache_dir.join(&png_name);

        ctx.progress("Parse Texture", 3, 6, "Parsing Texture2D data...");
        let info = match bundle.parse_object_by_path_id(path_id).map_err(|e| e)? {
            UnityClassObject::Texture2D(texture) => texture.data,
            other => {
                return Err(format!(
                    "Object path_id={} is {}, not Texture2D",
                    path_id,
                    other.class_name()
                ));
            }
        };

        ctx.check_cancelled()?;

        ctx.progress(
            "Decode Texture",
            4,
            6,
            "Decoding texture pixels in memory...",
        );
        let rgba = TextureExporter::decode_pixels_to_display_rgba_cancelable(
            &info.pixel_data,
            info.texture_format,
            info.width,
            info.height,
            Some(&cancel_token),
        )
        .map_err(|e| e)?;

        ctx.check_cancelled()?;

        ctx.progress("Scale Preview", 5, 6, "Scaling texture preview...");
        let max_edge = max_preview_edge.unwrap_or(1024).clamp(32, 4096);
        let (preview_rgba, preview_width, preview_height) =
            TextureExporter::downscale_rgba_nearest_cancelable(
                &rgba,
                info.width,
                info.height,
                max_edge,
                Some(&cancel_token),
            )
            .map_err(|e| e)?;

        ctx.progress("Encode Preview", 6, 6, "Encoding texture PNG preview...");
        let png_data_url = TextureExporter::rgba_to_png_data_url_cancelable(
            &preview_rgba,
            preview_width,
            preview_height,
            Some(&cancel_token),
        )
        .map_err(|e| e)?;

        let result = TexturePreviewResult {
            png_path: png_path.to_string_lossy().to_string(),
            png_data_url,
            preview_width,
            preview_height,
            width: info.width,
            height: info.height,
            texture_format: info.texture_format,
            texture_format_name: TextureFormatConst::format_name(info.texture_format).to_string(),
            complete_image_size: info.complete_image_size,
            mip_count: info.mip_count,
            image_count: info.image_count,
            texture_dimension: info.texture_dimension,
            texture_dimension_name: TextureFormatConst::dimension_name(info.texture_dimension)
                .to_string(),
            byte_size: info.image_data_size,
            has_alpha: TextureFormatConst::has_alpha(info.texture_format),
            color_space: info.color_space,
            color_space_name: TextureFormatConst::color_space_name(info.color_space).to_string(),
            filter_mode: info.filter_mode,
            filter_mode_name: TextureFormatConst::filter_mode_name(info.filter_mode).to_string(),
            aniso: info.aniso,
            wrap_u: info.wrap_u,
            wrap_v: info.wrap_v,
            wrap_mode_name: TextureFormatConst::wrap_mode_name(info.wrap_u).to_string(),
            is_readable: info.is_readable,
            streaming_mipmaps: info.streaming_mipmaps,
        };
        Ok(result)
    }
}
