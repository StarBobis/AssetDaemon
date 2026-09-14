/*
 * Bundle Extractor class -- Extracts all nodes from a Bundle to a disk cache directory.
 *
 * Follows Soul.md conventions: struct + impl organization, no free functions.
 *
 * Features:
 *   1. Extract all nodes (CAB files, .resS files, etc.) from a bundle to a specified cache directory
 *   2. Record nodes info to a JSON metadata file after extraction for later lookup
 *   3. Provide the ability to find extracted .resS files by StreamingInfo path
 *
 * Why extract to disk?
 *   Unity Mesh objects may store vertex data in .resS resource files,
 *   and .resS files are embedded inside the bundle as separate DirectoryNodes.
 *   The existing try_read_stream_data function only looks for .resS files in the filesystem,
 *   so we need to extract .resS nodes from the bundle to disk first, making them accessible as regular files.
 */

use crate::common::bundle_file::asset_bundle::AssetBundle;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::bundle_types::ExtractedNode;

/// Bundle extraction result
#[derive(Debug, Clone, Serialize)]
pub struct BundleExtractionResult {
    /// Summary of all extracted nodes
    pub nodes: Vec<ExtractedNode>,
    /// Node name to disk path mapping (for quick .resS lookup)
    pub node_map: HashMap<String, String>,
}

/**
 * Bundle node extractor class.
 *
 * Responsible for extracting all directory nodes from a bundle to a cache directory,
 * and recording metadata for later lookup by name.
 */
pub struct BundleExtractor;

impl BundleExtractor {
    /**
     * Extract all nodes from bundle to cache directory.
     *
     * @param bundle      Parsed AssetBundle
     * @param bundle_path  Original bundle file path (used to generate unique cache directory name)
     * @param cache_dir    Cache root directory
     * @return             Extraction result (node list + name-to-path mapping)
     */
    pub fn extract_all_nodes(
        bundle: &AssetBundle,
        bundle_path: &Path,
        cache_dir: &Path,
    ) -> Result<BundleExtractionResult, String> {
        // Generate a unique subdirectory name based on bundle file name
        let bundle_stem = bundle_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        // Use filename + timestamp hash to avoid collisions between same-named bundles from different paths
        let bundle_hash = Self::simple_hash(bundle_path.to_string_lossy().as_ref());
        let extract_subdir = format!("{}_{}", bundle_stem, bundle_hash);

        let extract_dir = cache_dir.join("bundle_nodes").join(&extract_subdir);
        fs::create_dir_all(&extract_dir).map_err(|e| {
            format!(
                "Failed to create extraction directory '{}': {}",
                extract_dir.display(),
                e
            )
        })?;

        let mut extracted_nodes = Vec::new();
        let mut node_map = HashMap::new();

        for node in &bundle.nodes {
            if !node.is_file() {
                continue;
            }

            // Extract node data
            let data = bundle
                .extract_node_data(node)
                .map_err(|e| format!("Failed to extract node '{}': {:?}", node.name, e))?;

            // Write to disk file
            let output_path = extract_dir.join(&node.name);
            fs::write(&output_path, &data).map_err(|e| {
                format!(
                    "Failed to write node file '{}': {}",
                    output_path.display(),
                    e
                )
            })?;

            let written = data.len() as u64;
            extracted_nodes.push(ExtractedNode {
                file_path: output_path.to_string_lossy().to_string(),
                size: written,
            });

            // Record name-to-path mapping (case-insensitive, Unity resource paths may have inconsistent case)
            node_map.insert(
                node.name.to_lowercase(),
                output_path.to_string_lossy().to_string(),
            );
        }

        // Write metadata JSON for quick lookup without re-extraction
        let meta_path = extract_dir.join("_metadata.json");
        let meta_json = serde_json::to_string_pretty(&node_map)
            .map_err(|e| format!("Failed to serialize metadata: {}", e))?;
        fs::write(&meta_path, &meta_json)
            .map_err(|e| format!("Failed to write metadata file: {}", e))?;

        Ok(BundleExtractionResult {
            nodes: extracted_nodes,
            node_map,
        })
    }

    /**
     * Get the root path of extracted nodes in the cache directory for a given bundle path.
     *
     * @param bundle_path Original bundle file path
     * @param cache_dir   Cache root directory
     * @return            Extraction directory path (may not exist)
     */
    pub fn get_extract_dir(bundle_path: &Path, cache_dir: &Path) -> PathBuf {
        let bundle_stem = bundle_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let bundle_hash = Self::simple_hash(bundle_path.to_string_lossy().as_ref());
        let extract_subdir = format!("{}_{}", bundle_stem, bundle_hash);
        cache_dir.join("bundle_nodes").join(&extract_subdir)
    }

    /**
     * Simple hash function for generating unique subdirectory names.
     * Uses the djb2 hash algorithm, producing an 8-character hex representation of a 32-bit hash.
     */
    fn simple_hash(input: &str) -> String {
        let mut hash: u32 = 5381;
        for b in input.bytes() {
            hash = hash.wrapping_mul(33).wrapping_add(b as u32);
        }
        format!("{:08x}", hash)
    }
}
