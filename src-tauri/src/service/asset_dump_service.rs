use std::path::Path;

use crate::common::bundle_file::asset_bundle::AssetBundleLoader;
use crate::common::command_types::DumpResult;
use crate::common::serialized_file::object_reader::ObjectReader;
use crate::common::task::task_context::TaskContext;
use crate::utils::debug_utils::DebugUtils;
use crate::utils::dump_utils::{DumpUtils, FormatParams, FormatState};

const HEX_DUMP_MAX_BYTES: usize = 4096;

pub struct AssetDumpService;

impl AssetDumpService {
    pub fn get_dump(
        ctx: &TaskContext,
        bundle_path: &str,
        path_id: &str,
    ) -> Result<DumpResult, String> {
        let path_id_i64: i64 = path_id
            .parse()
            .map_err(|e| format!("invalid path_id: {}", e))?;

        let bundle_path = Path::new(bundle_path);
        if !bundle_path.exists() {
            return Err(format!("file not found: {}", bundle_path.display()));
        }

        ctx.progress("Load Bundle", 1, 3, "Loading Bundle for asset dump");
        let bundle = AssetBundleLoader::load_bundle(bundle_path)
            .map_err(|e| format!("failed to load Bundle: {}", e))?;
        ctx.check_cancelled()?;

        ctx.progress("Find Asset", 2, 3, "Finding asset object");
        let (file, obj_info) = bundle
            .assets
            .iter()
            .filter_map(|sf| {
                sf.objects
                    .iter()
                    .find(|o| o.path_id == path_id_i64)
                    .map(|o| (sf, o))
            })
            .next()
            .ok_or_else(|| format!("asset with path_id={} not found", path_id))?;

        let class_name = AssetBundleLoader::get_class_name(obj_info.class_id).unwrap_or("Unknown");
        let raw = file
            .object_bytes(obj_info)
            .map_err(|e| format!("failed to read object data: {}", e))?;
        ctx.check_cancelled()?;

        ctx.progress("Format Dump", 3, 3, "Formatting asset dump");
        let type_tree = file
            .inner
            .object_serialized_type(&obj_info.inner)
            .map(|st| &st.type_tree)
            .filter(|tt| !tt.nodes.is_empty());

        if let Some(tt) = type_tree {
            let mut reader = ObjectReader::new(raw, &file.inner);
            let params = FormatParams::default();
            let mut state = FormatState::default();

            let body = DumpUtils::format_typetree_dump(&mut reader, tt, &params, &mut state)?;
            let header = format!("// {} (path_id={})\n\n", class_name, path_id);

            Ok(DumpResult {
                dump_text: header + &body,
                truncated: state.truncated,
                truncated_items: state.truncated_items,
                has_typetree: true,
            })
        } else {
            Ok(Self::hex_dump_result(class_name, path_id, raw))
        }
    }

    fn hex_dump_result(class_name: &str, path_id: &str, raw: &[u8]) -> DumpResult {
        let display_len = raw.len().min(HEX_DUMP_MAX_BYTES);
        let hex = DebugUtils::hex_dump(&raw[..display_len], 0);

        let header = format!(
            "// {} (path_id={})\n// No TypeTree, showing hex dump (first {} bytes of {})\n\n",
            class_name,
            path_id,
            display_len,
            raw.len()
        );

        let mut dump_text = header + &hex;
        if raw.len() > HEX_DUMP_MAX_BYTES {
            dump_text.push_str(&format!(
                "\n... remaining {} bytes not displayed",
                raw.len() - HEX_DUMP_MAX_BYTES
            ));
        }

        DumpResult {
            dump_text,
            truncated: raw.len() > HEX_DUMP_MAX_BYTES,
            truncated_items: raw.len().saturating_sub(HEX_DUMP_MAX_BYTES),
            has_typetree: false,
        }
    }
}
