/*
 * bundle_file.rs - UnityFS AssetBundle file parser
 *
 * Fully mimics AssetStudio's BundleFile.cs implementation.
 * Supports UnityFS format (signature "UnityFS"),
 * handles LZ4/LZMA compressed blocks, parses DirectoryInfo nodes.
 */

use std::fs;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

use crate::utils::byte_reader_utils::BundleReaderUtils;
use crate::utils::compression_utils::CompressionUtils;
use crate::utils::debug_utils::DebugUtils;

/// Bundle file header
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct BundleHeader {
    pub signature: String,
    pub version: u32,
    pub unity_version: String,
    pub unity_revision: String,
    pub size: i64,
    pub compressed_blocks_info_size: u32,
    pub uncompressed_blocks_info_size: u32,
    pub flags: u32,
}

/// Storage block information
#[derive(Debug, Clone)]
pub struct StorageBlock {
    pub compressed_size: u32,
    pub uncompressed_size: u32,
    pub flags: u16,
}

/// Directory node (files inside Bundle)
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DirectoryNode {
    pub offset: i64,
    pub size: i64,
    pub flags: u32,
    pub path: String,
}

/// File data extracted from inside Bundle
#[derive(Debug, Clone)]
pub struct StreamFile {
    pub path: String,
    pub file_name: String,
    pub data: Vec<u8>,
}

// ================================================================
// Archive flags (modeled after AssetStudio)
// ================================================================

const COMPRESSION_TYPE_MASK: u32 = 0x3f;
const BLOCKS_INFO_AT_THE_END: u32 = 0x80;
const BLOCK_INFO_NEED_PADDING_AT_START: u32 = 0x200;

// Storage block flags
const STORAGE_COMPRESSION_TYPE_MASK: u16 = 0x3f;

// ================================================================
// BundleFile struct
// ================================================================

#[allow(dead_code)]
pub struct BundleFile {
    pub header: BundleHeader,
    pub blocks_info: Vec<StorageBlock>,
    pub directory_info: Vec<DirectoryNode>,
    pub file_list: Vec<StreamFile>,
}

impl BundleFile {
    /// Load Bundle from file path
    pub fn load(path: &Path) -> Result<Self, String> {
        let file_data = fs::read(path)
            .map_err(|e| format!("Failed to read file '{}': {}", path.display(), e))?;
        Self::parse(&file_data)
    }

    /// Load Bundle from a file path while extracting only files accepted by `include_file`.
    ///
    /// Unlike `parse_filtered`, this path does not read the whole UnityFS file into memory before
    /// decompression. It seeks to BlocksInfo and the compressed data blocks directly.
    pub fn load_filtered<F>(path: &Path, include_file: F) -> Result<Self, String>
    where
        F: Fn(&str) -> bool,
    {
        let mut file = fs::File::open(path)
            .map_err(|e| format!("Failed to open '{}': {}", path.display(), e))?;
        let total_len = file
            .metadata()
            .map_err(|e| format!("Failed to stat '{}': {}", path.display(), e))?
            .len()
            .try_into()
            .map_err(|_| {
                format!(
                    "File too large to index on this platform: {}",
                    path.display()
                )
            })?;
        Self::parse_filtered_reader_with_logs(
            &mut file,
            total_len,
            &mut vec![],
            include_file,
            |_| None,
        )
    }

    /// Load Bundle from file path, with diagnostic logs
    pub fn load_with_logs(path: &Path, logs: &mut Vec<String>) -> Result<Self, String> {
        logs.push(format!(
            "BundleFile::load_with_logs: reading file '{}'",
            path.display()
        ));
        let file_data = fs::read(path)
            .map_err(|e| format!("Failed to read file '{}': {}", path.display(), e))?;
        logs.push(format!("  File size: {} bytes", file_data.len()));
        Self::parse_with_logs(&file_data, logs)
    }

    /// Parse Bundle from byte array
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        Self::parse_with_logs(data, &mut vec![])
    }

    /// Parse a Bundle while extracting only files accepted by `include_file`.
    pub fn parse_filtered<F>(data: &[u8], include_file: F) -> Result<Self, String>
    where
        F: Fn(&str) -> bool,
    {
        Self::parse_filtered_range_with_logs(data, &mut vec![], include_file, |_| None)
    }

    /// Parse Bundle from byte array, with diagnostic logs
    pub fn parse_with_logs(data: &[u8], logs: &mut Vec<String>) -> Result<Self, String> {
        Self::parse_filtered_with_logs(data, logs, |_| true)
    }

    fn parse_filtered_with_logs<F>(
        data: &[u8],
        logs: &mut Vec<String>,
        include_file: F,
    ) -> Result<Self, String>
    where
        F: Fn(&str) -> bool,
    {
        Self::parse_filtered_range_with_logs(data, logs, include_file, |_| None)
    }

    fn parse_filtered_range_with_logs<F, R>(
        data: &[u8],
        logs: &mut Vec<String>,
        include_file: F,
        file_range: R,
    ) -> Result<Self, String>
    where
        F: Fn(&str) -> bool,
        R: Fn(&DirectoryNode) -> Option<(usize, usize)>,
    {
        let mut reader = Cursor::new(data);
        let total_len = data.len();
        logs.push(format!(
            "BundleFile::parse_with_logs: total bytes={}",
            total_len
        ));

        // ============================================================
        // Read signature
        // ============================================================
        let signature = BundleReaderUtils::read_string_to_null_limited(&mut reader, 32)
            .map_err(|e| format!("Failed to read signature: {}", e))?;
        logs.push(format!("  Signature: '{}'", signature));

        if signature != "UnityFS" {
            let err = format!(
                "Unsupported Bundle format: '{}', only UnityFS is supported",
                signature
            );
            logs.push(format!("  Error: {}", err));
            return Err(err);
        }

        let version = BundleReaderUtils::read_u32(&mut reader)
            .map_err(|e| format!("Failed to read version: {}", e))?;
        let unity_version = BundleReaderUtils::read_string_to_null(&mut reader)
            .map_err(|e| format!("Failed to read unity_version: {}", e))?;
        let unity_revision = BundleReaderUtils::read_string_to_null(&mut reader)
            .map_err(|e| format!("Failed to read unity_revision: {}", e))?;
        logs.push(format!(
            "  Version: {}, unity_version='{}', unity_revision='{}'",
            version, unity_version, unity_revision
        ));

        // ============================================================
        // Read Header (UnityFS format)
        // ============================================================
        let size = BundleReaderUtils::read_i64(&mut reader)
            .map_err(|e| format!("Failed to read size: {}", e))?;
        let compressed_blocks_info_size = BundleReaderUtils::read_u32(&mut reader)
            .map_err(|e| format!("Failed to read compressed_blocks_info_size: {}", e))?;
        let uncompressed_blocks_info_size = BundleReaderUtils::read_u32(&mut reader)
            .map_err(|e| format!("Failed to read uncompressed_blocks_info_size: {}", e))?;
        let flags = BundleReaderUtils::read_u32(&mut reader)
            .map_err(|e| format!("Failed to read flags: {}", e))?;
        logs.push(format!(
            "  Header: size={}, compressed_blocks_info_size={}, uncompressed_blocks_info_size={}, flags=0x{:X}",
            size, compressed_blocks_info_size, uncompressed_blocks_info_size, flags
        ));

        // Safety check: prevent OOM from malicious values
        // Also check whether it is encrypted data (encrypted UnityFS files usually produce huge values)
        if compressed_blocks_info_size > 100_000_000 || uncompressed_blocks_info_size > 500_000_000
        {
            // reader.position() may point past the end of the buffer after
            // reading a malformed header, so clamp both sides before slicing.
            let ctx_start = reader
                .position()
                .saturating_sub(64)
                .min(total_len as u64) as usize;
            let ctx_end = (reader.position() as usize)
                .saturating_add(64)
                .min(total_len)
                .max(ctx_start);
            let ctx_hex = DebugUtils::hex_dump(&data[ctx_start..ctx_end], ctx_start);
            let err =
                format!(
                "UnityFS header parse exception - file may be undecrypted or format unsupported\n\
                 compressed_blocks_info_size={} (0x{:08X})\n\
                 uncompressed_blocks_info_size={} (0x{:08X})\n\
                 signature='{}', version={}, unity_version='{}', unity_revision='{}', size={}\n\
                 reader_pos={}, total_len={}\n\
                 header surrounding bytes (offset {}..{}):\n{}",
                compressed_blocks_info_size, compressed_blocks_info_size,
                uncompressed_blocks_info_size, uncompressed_blocks_info_size,
                signature, version, unity_version, unity_revision, size,
                reader.position(), total_len,
                ctx_start, ctx_end, ctx_hex
            );
            logs.push(format!("  Safety check triggered: {}", err));
            return Err(err);
        }

        let header = BundleHeader {
            signature: signature.clone(),
            version,
            unity_version: unity_version.clone(),
            unity_revision,
            size,
            compressed_blocks_info_size,
            uncompressed_blocks_info_size,
            flags,
        };

        // ============================================================
        // Read BlocksInfo + DirectoryInfo
        // ============================================================
        let header_end_pos = reader.position();
        let blocks_info_at_end = (flags & BLOCKS_INFO_AT_THE_END) != 0;

        let blocks_info_start_pos = if header.version >= 7 {
            ((header_end_pos as usize + 15) & !15) as u64
        } else {
            header_end_pos
        };

        let blocks_info_bytes = if blocks_info_at_end {
            // Blocks info at end of file
            let start = total_len.saturating_sub(compressed_blocks_info_size as usize);
            if start >= total_len {
                return Err(format!(
                    "blocks_info_at_end out of bounds: total_len={}, compressed_blocks_info_size={}",
                    total_len, compressed_blocks_info_size
                ));
            }
            let available = total_len - start;
            let to_read = (compressed_blocks_info_size as usize).min(available);
            data[start..start + to_read].to_vec()
        } else {
            // Blocks info immediately after header
            reader
                .seek(SeekFrom::Start(blocks_info_start_pos))
                .map_err(|e| format!("Seek align failed: {}", e))?;
            let pos = reader.position() as usize;
            if pos > total_len {
                return Err(format!(
                    "blocks_info start out of bounds: pos={}, total_len={}",
                    pos, total_len
                ));
            }
            let remaining = total_len - pos;
            let to_read = (compressed_blocks_info_size as usize).min(remaining);
            let bytes = data[pos..pos + to_read].to_vec();
            reader
                .seek(SeekFrom::Start((pos + to_read) as u64))
                .map_err(|e| format!("Seek past blocks info failed: {}", e))?;
            bytes
        };

        if blocks_info_bytes.is_empty() {
            return Err(format!(
                "blocks_info_bytes is empty (compressed_size={}, total_len={}, blocks_at_end={:?})",
                compressed_blocks_info_size, total_len, blocks_info_at_end
            ));
        }

        // Decompress BlocksInfo
        let compression_type = flags & COMPRESSION_TYPE_MASK;
        logs.push(format!(
            "  Compression type: {}, blocks_at_end: {}",
            compression_type, blocks_info_at_end
        ));
        let uncompressed = CompressionUtils::decompress_data(
            &blocks_info_bytes,
            uncompressed_blocks_info_size as usize,
            compression_type,
        )
        .map_err(|e| format!("Failed to decompress BlocksInfo: {}", e))?;
        logs.push(format!(
            "  BlocksInfo decompressed size: {} bytes",
            uncompressed.len()
        ));

        let (blocks_info, directory_info) = Self::read_blocks_info_and_directory(&uncompressed)?;
        logs.push(format!(
            "  Blocks count: {}, Directory nodes: {}",
            blocks_info.len(),
            directory_info.len()
        ));
        for (i, node) in directory_info.iter().enumerate().take(10) {
            logs.push(format!(
                "    Node[{}]: offset={}, size={}, path='{}'",
                i, node.offset, node.size, node.path
            ));
        }
        if directory_info.len() > 10 {
            logs.push(format!("    ... {} more nodes", directory_info.len() - 10));
        }

        // ============================================================
        // Decompress data blocks and extract files
        // ============================================================
        let mut data_start = if blocks_info_at_end {
            blocks_info_start_pos
        } else {
            blocks_info_start_pos + compressed_blocks_info_size as u64
        };
        if (flags & BLOCK_INFO_NEED_PADDING_AT_START) != 0 {
            data_start = (data_start + 15) & !15;
        }

        reader
            .seek(SeekFrom::Start(data_start))
            .map_err(|e| format!("Seek to data blocks failed (pos={}): {}", data_start, e))?;
        logs.push(format!("  Data blocks start offset: {}", data_start));

        let file_list = Self::read_blocks_and_files_robust(
            &mut reader,
            &blocks_info,
            &directory_info,
            total_len,
            logs,
            include_file,
            file_range,
        )?;
        logs.push(format!(
            "  File list extraction complete: {} files",
            file_list.len()
        ));
        for (i, f) in file_list.iter().enumerate().take(10) {
            logs.push(format!(
                "    File[{}]: path='{}', size={}B",
                i,
                f.path,
                f.data.len()
            ));
        }
        if file_list.len() > 10 {
            logs.push(format!("    ... {} more files", file_list.len() - 10));
        }

        logs.push(format!(
            "BundleFile parse complete: {} blocks, {} nodes, {} files",
            blocks_info.len(),
            directory_info.len(),
            file_list.len()
        ));

        Ok(BundleFile {
            header,
            blocks_info,
            directory_info,
            file_list,
        })
    }

    fn parse_filtered_reader_with_logs<Rd, F, FR>(
        reader: &mut Rd,
        total_len: usize,
        logs: &mut Vec<String>,
        include_file: F,
        file_range: FR,
    ) -> Result<Self, String>
    where
        Rd: Read + Seek,
        F: Fn(&str) -> bool,
        FR: Fn(&DirectoryNode) -> Option<(usize, usize)>,
    {
        reader
            .seek(SeekFrom::Start(0))
            .map_err(|e| format!("Seek to start failed: {}", e))?;
        logs.push(format!(
            "BundleFile::parse_reader_with_logs: total bytes={}",
            total_len
        ));

        let signature = BundleReaderUtils::read_string_to_null_limited(reader, 32)
            .map_err(|e| format!("Failed to read signature: {}", e))?;
        logs.push(format!("  Signature: '{}'", signature));

        if signature != "UnityFS" {
            let err = format!(
                "Unsupported Bundle format: '{}', only UnityFS is supported",
                signature
            );
            logs.push(format!("  Error: {}", err));
            return Err(err);
        }

        let version = BundleReaderUtils::read_u32(reader)
            .map_err(|e| format!("Failed to read version: {}", e))?;
        let unity_version = BundleReaderUtils::read_string_to_null(reader)
            .map_err(|e| format!("Failed to read unity_version: {}", e))?;
        let unity_revision = BundleReaderUtils::read_string_to_null(reader)
            .map_err(|e| format!("Failed to read unity_revision: {}", e))?;

        let size = BundleReaderUtils::read_i64(reader)
            .map_err(|e| format!("Failed to read size: {}", e))?;
        let compressed_blocks_info_size = BundleReaderUtils::read_u32(reader)
            .map_err(|e| format!("Failed to read compressed_blocks_info_size: {}", e))?;
        let uncompressed_blocks_info_size = BundleReaderUtils::read_u32(reader)
            .map_err(|e| format!("Failed to read uncompressed_blocks_info_size: {}", e))?;
        let flags = BundleReaderUtils::read_u32(reader)
            .map_err(|e| format!("Failed to read flags: {}", e))?;

        if compressed_blocks_info_size > 100_000_000 || uncompressed_blocks_info_size > 500_000_000
        {
            return Err(format!(
                "UnityFS header parse exception - file may be undecrypted or format unsupported; compressed_blocks_info_size={} uncompressed_blocks_info_size={} signature='{}' version={} unity_version='{}' unity_revision='{}' size={} total_len={}",
                compressed_blocks_info_size,
                uncompressed_blocks_info_size,
                signature,
                version,
                unity_version,
                unity_revision,
                size,
                total_len
            ));
        }

        let header = BundleHeader {
            signature,
            version,
            unity_version,
            unity_revision,
            size,
            compressed_blocks_info_size,
            uncompressed_blocks_info_size,
            flags,
        };

        let header_end_pos = reader
            .stream_position()
            .map_err(|e| format!("Read header position failed: {}", e))?;
        let blocks_info_at_end = (flags & BLOCKS_INFO_AT_THE_END) != 0;
        let blocks_info_start_pos = if header.version >= 7 {
            ((header_end_pos as usize + 15) & !15) as u64
        } else {
            header_end_pos
        };

        let blocks_info_bytes = if blocks_info_at_end {
            let start = total_len.saturating_sub(compressed_blocks_info_size as usize);
            if start >= total_len {
                return Err(format!(
                    "blocks_info_at_end out of bounds: total_len={}, compressed_blocks_info_size={}",
                    total_len, compressed_blocks_info_size
                ));
            }
            reader
                .seek(SeekFrom::Start(start as u64))
                .map_err(|e| format!("Seek to end BlocksInfo failed: {}", e))?;
            BundleReaderUtils::read_bytes(reader, compressed_blocks_info_size as usize)
                .map_err(|e| format!("Failed to read end BlocksInfo: {}", e))?
        } else {
            reader
                .seek(SeekFrom::Start(blocks_info_start_pos))
                .map_err(|e| format!("Seek align failed: {}", e))?;
            let remaining = total_len.saturating_sub(blocks_info_start_pos as usize);
            let to_read = (compressed_blocks_info_size as usize).min(remaining);
            BundleReaderUtils::read_bytes(reader, to_read)
                .map_err(|e| format!("Failed to read BlocksInfo: {}", e))?
        };

        if blocks_info_bytes.is_empty() {
            return Err(format!(
                "blocks_info_bytes is empty (compressed_size={}, total_len={}, blocks_at_end={:?})",
                compressed_blocks_info_size, total_len, blocks_info_at_end
            ));
        }

        let compression_type = flags & COMPRESSION_TYPE_MASK;
        let uncompressed = CompressionUtils::decompress_data(
            &blocks_info_bytes,
            uncompressed_blocks_info_size as usize,
            compression_type,
        )
        .map_err(|e| format!("Failed to decompress BlocksInfo: {}", e))?;

        let (blocks_info, directory_info) = Self::read_blocks_info_and_directory(&uncompressed)?;

        let mut data_start = if blocks_info_at_end {
            blocks_info_start_pos
        } else {
            blocks_info_start_pos + compressed_blocks_info_size as u64
        };
        if (flags & BLOCK_INFO_NEED_PADDING_AT_START) != 0 {
            data_start = (data_start + 15) & !15;
        }

        reader
            .seek(SeekFrom::Start(data_start))
            .map_err(|e| format!("Seek to data blocks failed (pos={}): {}", data_start, e))?;

        let file_list = Self::read_blocks_and_files_robust(
            reader,
            &blocks_info,
            &directory_info,
            total_len,
            logs,
            include_file,
            file_range,
        )?;

        Ok(BundleFile {
            header,
            blocks_info,
            directory_info,
            file_list,
        })
    }

    /// Parse BlocksInfo + DirectoryInfo (decompressed data)
    fn read_blocks_info_and_directory(
        data: &[u8],
    ) -> Result<(Vec<StorageBlock>, Vec<DirectoryNode>), String> {
        let mut reader = Cursor::new(data);

        // Skip 16 byte hash
        let _hash = BundleReaderUtils::read_bytes(&mut reader, 16)?;

        let blocks_info_count = BundleReaderUtils::read_i32(&mut reader)?;
        if blocks_info_count < 0 || blocks_info_count > 200000 {
            return Err(format!("Abnormal block count: {}", blocks_info_count));
        }

        let mut blocks_info = Vec::with_capacity(blocks_info_count as usize);
        for _ in 0..blocks_info_count {
            let uncompressed_size = BundleReaderUtils::read_u32(&mut reader)?;
            let compressed_size = BundleReaderUtils::read_u32(&mut reader)?;
            let flags = BundleReaderUtils::read_u16(&mut reader)?;
            blocks_info.push(StorageBlock {
                uncompressed_size,
                compressed_size,
                flags,
            });
        }

        let nodes_count = BundleReaderUtils::read_i32(&mut reader)?;
        if nodes_count < 0 || nodes_count > 100000 {
            return Err(format!("Abnormal node count: {}", nodes_count));
        }

        let mut directory_info = Vec::with_capacity(nodes_count as usize);
        for _ in 0..nodes_count {
            let offset = BundleReaderUtils::read_i64(&mut reader)?;
            let size = BundleReaderUtils::read_i64(&mut reader)?;
            let flags = BundleReaderUtils::read_u32(&mut reader)?;
            let path = BundleReaderUtils::read_string_to_null(&mut reader)?;
            directory_info.push(DirectoryNode {
                offset,
                size,
                flags,
                path,
            });
        }

        Ok((blocks_info, directory_info))
    }

    /// Read compressed blocks and extract file data (robust version: tolerant of partial data)
    fn read_blocks_and_files_robust<R: Read + Seek, F, FR>(
        reader: &mut R,
        blocks_info: &[StorageBlock],
        directory_info: &[DirectoryNode],
        _total_file_len: usize,
        logs: &mut Vec<String>,
        include_file: F,
        file_range: FR,
    ) -> Result<Vec<StreamFile>, String>
    where
        F: Fn(&str) -> bool,
        FR: Fn(&DirectoryNode) -> Option<(usize, usize)>,
    {
        let included_nodes: Vec<&DirectoryNode> = directory_info
            .iter()
            .filter(|node| include_file(&node.path))
            .collect();
        if included_nodes.is_empty() {
            return Ok(Vec::new());
        }

        let mut block_ranges = Vec::with_capacity(blocks_info.len());
        let mut block_offset = 0usize;
        for block in blocks_info {
            let start = block_offset;
            block_offset = block_offset.saturating_add(block.uncompressed_size as usize);
            block_ranges.push((start, block_offset));
        }

        let mut needed_blocks = vec![false; blocks_info.len()];
        for node in &included_nodes {
            let node_base = node.offset.max(0) as usize;
            let node_size = node.size.max(0) as usize;
            let (relative_offset, read_size) = file_range(node).unwrap_or((0, node_size));
            let relative_offset = relative_offset.min(node_size);
            let read_size = read_size.min(node_size.saturating_sub(relative_offset));
            let node_start = node_base.saturating_add(relative_offset);
            let node_end = node_start.saturating_add(read_size);
            for (idx, (block_start, block_end)) in block_ranges.iter().enumerate() {
                if *block_start < node_end && *block_end > node_start {
                    needed_blocks[idx] = true;
                }
            }
        }

        let needed_count = needed_blocks.iter().filter(|needed| **needed).count();
        logs.push(format!(
            "    Filtered extraction: {} included nodes, {} / {} blocks needed",
            included_nodes.len(),
            needed_count,
            blocks_info.len()
        ));

        let total_blocks = blocks_info.len();
        let mut compressed_blocks: Vec<(usize, usize, u32, Vec<u8>)> =
            Vec::with_capacity(needed_count);

        for (i, block) in blocks_info.iter().enumerate() {
            let compression_type = (block.flags & STORAGE_COMPRESSION_TYPE_MASK) as u32;
            let compressed_size = block.compressed_size as usize;
            let uncompressed_size = block.uncompressed_size as usize;

            if compressed_size == 0 {
                continue;
            }

            if needed_blocks.get(i).copied().unwrap_or(false) {
                logs.push(format!(
                    "    Decompressing block[{}/{}]: compressed={}, uncompressed={}, type={}",
                    i + 1,
                    total_blocks,
                    compressed_size,
                    uncompressed_size,
                    compression_type
                ));

                let compressed = match BundleReaderUtils::read_bytes(reader, compressed_size) {
                    Ok(data) => data,
                    Err(e) => {
                        return Err(format!(
                            "Failed to read compressed data for block {} (compressed_size={}): {}",
                            i, compressed_size, e
                        ));
                    }
                };

                compressed_blocks.push((i, uncompressed_size, compression_type, compressed));
            } else {
                reader
                    .seek(SeekFrom::Current(compressed_size as i64))
                    .map_err(|e| {
                        format!(
                            "Failed to skip compressed data for block {} (compressed_size={}): {}",
                            i, compressed_size, e
                        )
                    })?;
            }
        }

        let decompressed_blocks = Self::decompress_selected_blocks(compressed_blocks)?;
        let decompressed_total: usize = decompressed_blocks
            .iter()
            .filter_map(|block| block.as_ref().map(Vec::len))
            .sum();
        logs.push(format!(
            "    Needed blocks decompressed: total data {} bytes",
            decompressed_total
        ));

        let mut file_list = Vec::with_capacity(included_nodes.len());
        for node in included_nodes {
            let node_base = node.offset.max(0) as usize;
            let node_size = node.size.max(0) as usize;
            let (relative_offset, read_size) = file_range(node).unwrap_or((0, node_size));
            let relative_offset = relative_offset.min(node_size);
            let read_size = read_size.min(node_size.saturating_sub(relative_offset));
            let offset = node_base.saturating_add(relative_offset);
            let data = Self::collect_node_data_from_blocks(
                offset,
                read_size,
                &block_ranges,
                &decompressed_blocks,
            );

            let file_name = Path::new(&node.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| node.path.clone());

            file_list.push(StreamFile {
                path: node.path.clone(),
                file_name,
                data,
            });
        }

        Ok(file_list)
    }

    fn decompress_selected_blocks(
        compressed_blocks: Vec<(usize, usize, u32, Vec<u8>)>,
    ) -> Result<Vec<Option<Vec<u8>>>, String> {
        if compressed_blocks.is_empty() {
            return Ok(Vec::new());
        }

        let total_uncompressed: usize = compressed_blocks
            .iter()
            .map(|(_, uncompressed_size, _, _)| *uncompressed_size)
            .sum();
        if compressed_blocks.len() > 1 && total_uncompressed >= 16 * 1024 * 1024 {
            return Self::decompress_selected_blocks_parallel(compressed_blocks);
        }

        let max_index = compressed_blocks
            .iter()
            .map(|(idx, _, _, _)| *idx)
            .max()
            .unwrap_or(0);
        let mut blocks = Vec::with_capacity(max_index + 1);
        blocks.resize_with(max_index + 1, || None);

        for (i, uncompressed_size, compression_type, compressed) in compressed_blocks {
            let uncompressed =
                CompressionUtils::decompress_data(&compressed, uncompressed_size, compression_type)
                    .map_err(|e| {
                        format!(
                            "Failed to decompress block {} (uncompressed={}, type={}): {}",
                            i, uncompressed_size, compression_type, e
                        )
                    })?;
            blocks[i] = Some(uncompressed);
        }

        Ok(blocks)
    }

    fn decompress_selected_blocks_parallel(
        compressed_blocks: Vec<(usize, usize, u32, Vec<u8>)>,
    ) -> Result<Vec<Option<Vec<u8>>>, String> {
        let max_index = compressed_blocks
            .iter()
            .map(|(idx, _, _, _)| *idx)
            .max()
            .unwrap_or(0);
        let block_count = compressed_blocks.len();
        let cpu_count = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let worker_count = cpu_count.min(32).min(block_count).max(1);
        let next_block = AtomicUsize::new(0);
        let failed = AtomicBool::new(false);
        let mut blocks = Vec::with_capacity(max_index + 1);
        blocks.resize_with(max_index + 1, || None);

        thread::scope(|scope| {
            let (block_tx, block_rx) = mpsc::sync_channel(worker_count * 2);
            let mut handles = Vec::with_capacity(worker_count);
            for _worker_id in 0..worker_count {
                let compressed_blocks = &compressed_blocks;
                let next_block = &next_block;
                let failed = &failed;
                let block_tx = block_tx.clone();
                handles.push(scope.spawn(move || loop {
                    if failed.load(Ordering::Relaxed) {
                        break;
                    }
                    let idx = next_block.fetch_add(1, Ordering::Relaxed);
                    if idx >= compressed_blocks.len() {
                        break;
                    }

                    let (block_index, uncompressed_size, compression_type, compressed) =
                        &compressed_blocks[idx];
                    let message = CompressionUtils::decompress_data(
                        compressed,
                        *uncompressed_size,
                        *compression_type,
                    )
                    .map(|data| (*block_index, *uncompressed_size, data))
                    .map_err(|e| {
                        format!(
                            "Failed to decompress block {} (uncompressed={}, type={}): {}",
                            block_index, uncompressed_size, compression_type, e
                        )
                    })
                    .and_then(|(block_index, uncompressed_size, data)| {
                        if data.len() == uncompressed_size {
                            Ok((block_index, data))
                        } else {
                            Err(format!(
                                "Failed to decompress block {}: wrote {} bytes but expected {} bytes",
                                block_index,
                                data.len(),
                                uncompressed_size
                            ))
                        }
                    });

                    let is_err = message.is_err();
                    if is_err {
                        failed.store(true, Ordering::Relaxed);
                    }
                    if block_tx.send(message).is_err() || is_err {
                        break;
                    }
                }));
            }
            drop(block_tx);

            let mut received = 0usize;
            while received < block_count {
                match block_rx.recv() {
                    Ok(Ok((block_index, data))) => {
                        if let Some(slot) = blocks.get_mut(block_index) {
                            *slot = Some(data);
                        }
                        received += 1;
                    }
                    Ok(Err(e)) => return Err(e),
                    Err(_) => return Err("Block decompression worker stopped early".to_string()),
                }
            }

            for handle in handles {
                handle
                    .join()
                    .map_err(|_| "Block decompression worker panicked".to_string())?;
            }
            Ok::<(), String>(())
        })?;

        Ok(blocks)
    }

    fn collect_node_data_from_blocks(
        offset: usize,
        size: usize,
        block_ranges: &[(usize, usize)],
        decompressed_blocks: &[Option<Vec<u8>>],
    ) -> Vec<u8> {
        if size == 0 {
            return Vec::new();
        }

        let node_end = offset.saturating_add(size);
        let mut data = Vec::with_capacity(size);
        for (idx, (block_start, block_end)) in block_ranges.iter().enumerate() {
            if *block_start >= node_end || *block_end <= offset {
                continue;
            }
            let Some(Some(block_data)) = decompressed_blocks.get(idx) else {
                continue;
            };
            let read_start = offset.max(*block_start) - *block_start;
            let read_end = node_end.min(*block_end) - *block_start;
            if read_end > read_start && read_end <= block_data.len() {
                data.extend_from_slice(&block_data[read_start..read_end]);
            }
        }
        data
    }

    /// Check if Bundle is compressed (has blocks with non-zero compression type)
    pub fn is_compressed(&self) -> bool {
        self.blocks_info
            .iter()
            .any(|b| (b.flags & STORAGE_COMPRESSION_TYPE_MASK) != 0)
    }

    /// Get total file size (sum of all decompressed blocks)
    pub fn size(&self) -> u64 {
        if self.blocks_info.is_empty() {
            return self.header.size.max(0) as u64;
        }
        self.blocks_info
            .iter()
            .map(|b| b.uncompressed_size as u64)
            .sum()
    }
}

// All binary reading methods have been migrated to BundleReaderUtils struct in src/utils/byte_reader_utils.rs
