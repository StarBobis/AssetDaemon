/*
 * Unity Mesh parser class.
 *
 * Uses a heuristic approach to extract vertices and indices from raw Unity Mesh binary data.
 * Follows Soul.md convention: struct + impl organization, no free functions.
 */

use crate::utils::byte_utils::ByteUtils;

/// Parsed mesh data
pub struct ParsedMesh {
    /// Vertex positions (float3 array, flat)
    pub vertices: Vec<f32>,
    /// Normals (float3 array, flat, may be empty)
    #[allow(dead_code)]
    pub normals: Vec<f32>,
    /// UV coordinates (float2 array, flat, may be empty)
    #[allow(dead_code)]
    pub uvs: Vec<f32>,
    /// Triangle indices (uint32 array)
    pub indices: Vec<u32>,
}

/**
 * Unity Mesh parser class.
 *
 * Uses a heuristic approach to extract vertices and indices from raw Unity Mesh binary data.
 * All methods are static, no state is held.
 */
pub struct UnityMeshParser;

impl UnityMeshParser {
    /**
     * Heuristically parse Unity Mesh binary data.
     *
     * Unity serializes Mesh data as a contiguous byte block containing:
     * - VertexData: vertex count + channel description + vertex data stream
     * - IndexBuffer: index data (uint16 or uint32)
     *
     * Since version-specific formats cannot be relied upon, pattern matching is used:
     * 1. Look for signatures around the vertex count field
     * 2. Look for float3 patterns in vertex positions
     * 3. Look for contiguous index values
     *
     * @param data  Raw bytes of the Mesh object
     * @return     ParsedMesh or error
     */
    pub fn parse_unity_mesh(data: &[u8]) -> Result<ParsedMesh, String> {
        if data.len() < 64 {
            return Err("Mesh data too short".to_string());
        }

        let mut best_vertices: Vec<f32> = Vec::new();
        let mut best_normals: Vec<f32> = Vec::new();
        let best_uvs: Vec<f32> = Vec::new();
        let mut best_indices: Vec<u32> = Vec::new();

        // Try multiple offsets to find the vertex count
        for offset in (24..=64).step_by(4) {
            if offset + 4 > data.len() {
                break;
            }

            let vertex_count = ByteUtils::read_u32_le(data, offset) as usize;

            if vertex_count < 3 || vertex_count > 100_000 {
                continue;
            }

            let data_portion = &data[offset.min(data.len() - 4)..];

            if let Some(pos) = Self::extract_float3_array(data_portion, vertex_count) {
                best_vertices = pos;
            }

            if !best_vertices.is_empty() {
                let vert_byte_count = vertex_count * 12;
                if vert_byte_count + 12 <= data_portion.len() {
                    if let Some(normals) =
                        Self::extract_float3_array(&data_portion[vert_byte_count..], vertex_count)
                    {
                        if normals.len() >= vertex_count * 3 {
                            best_normals = normals;
                        }
                    }
                }
            }

            let search_start =
                (offset + 8 + vertex_count * 12 + 8).min(data.len().saturating_sub(8));
            if search_start < data.len() {
                if let Some(idx) = Self::find_index_buffer_u16(data, search_start, vertex_count) {
                    best_indices = idx;
                }
                if best_indices.is_empty() {
                    if let Some(idx) = Self::find_index_buffer_u32(data, search_start, vertex_count)
                    {
                        best_indices = idx;
                    }
                }
            }

            if !best_vertices.is_empty() && !best_indices.is_empty() {
                break;
            }
        }

        if best_vertices.is_empty() {
            if let Some(pos) = Self::find_float3_trail(data) {
                best_vertices = pos;
            }
        }

        if best_vertices.is_empty() {
            return Err("Unable to extract vertex information from Mesh data.".to_string());
        }

        Ok(ParsedMesh {
            vertices: best_vertices,
            normals: best_normals,
            uvs: best_uvs,
            indices: best_indices,
        })
    }

    /**
     * Read a little-endian u32 from the byte array at the given offset.
     */
    /**
     * Attempt to extract a specified number of float3 vertices from the data.
     */
    fn extract_float3_array(data: &[u8], count: usize) -> Option<Vec<f32>> {
        let needed = count * 3 * 4;
        if needed > data.len() || needed == 0 {
            return None;
        }

        let mut result = Vec::with_capacity(count * 3);
        for i in 0..count {
            let off = i * 12;
            if off + 12 > data.len() {
                return None;
            }
            let x = ByteUtils::read_f32_le(data, off);
            let y = ByteUtils::read_f32_le(data, off + 4);
            let z = ByteUtils::read_f32_le(data, off + 8);

            if !x.is_finite() || !y.is_finite() || !z.is_finite() {
                return None;
            }

            result.push(x);
            result.push(y);
            result.push(z);
        }

        Some(result)
    }

    /**
     * Find a uint16 type index buffer.
     *
     * Searches the data for sequences of uint16 values that can form valid triangles.
     * Valid triangle indices satisfy: all indices < vertex_count.
     */
    fn find_index_buffer_u16(data: &[u8], start: usize, vertex_count: usize) -> Option<Vec<u32>> {
        let data_len = data.len();
        if data_len < 6 {
            return None;
        }
        let max_search = data_len.saturating_sub(6);
        let start = start.min(max_search);
        let end = (start + 60000).min(max_search);

        let mut best_run: Vec<u32> = Vec::new();

        let mut i = start;
        while i + 5 < end {
            let idx0 = u16::from_le_bytes([data[i], data[i + 1]]) as u32;
            let idx1 = u16::from_le_bytes([data[i + 2], data[i + 3]]) as u32;
            let idx2 = u16::from_le_bytes([data[i + 4], data[i + 5]]) as u32;

            if idx0 < vertex_count as u32
                && idx1 < vertex_count as u32
                && idx2 < vertex_count as u32
                && !(idx0 == idx1 || idx1 == idx2 || idx0 == idx2)
            {
                best_run.push(idx0);
                best_run.push(idx1);
                best_run.push(idx2);
                i += 6;
            } else {
                if best_run.len() >= 6 {
                    break;
                }
                best_run.clear();
                i += 2;
            }
        }

        if best_run.len() >= 3 {
            Some(best_run)
        } else {
            None
        }
    }

    /**
     * Find a uint32 type index buffer.
     */
    fn find_index_buffer_u32(data: &[u8], start: usize, vertex_count: usize) -> Option<Vec<u32>> {
        let data_len = data.len();
        if data_len < 12 {
            return None;
        }
        let max_search = data_len.saturating_sub(12);
        let start = start.min(max_search);
        let end = (start + 80000).min(max_search);

        let mut best_run: Vec<u32> = Vec::new();

        let mut i = start;
        while i + 8 < end {
            let idx0 = ByteUtils::read_u32_le(data, i);
            let idx1 = ByteUtils::read_u32_le(data, i + 4);
            let idx2 = ByteUtils::read_u32_le(data, i + 8);

            if idx0 < vertex_count as u32
                && idx1 < vertex_count as u32
                && idx2 < vertex_count as u32
                && !(idx0 == idx1 || idx1 == idx2 || idx0 == idx2)
            {
                best_run.push(idx0);
                best_run.push(idx1);
                best_run.push(idx2);
                i += 12;
            } else {
                if best_run.len() >= 6 {
                    break;
                }
                best_run.clear();
                i += 4;
            }
        }

        if best_run.len() >= 3 {
            Some(best_run)
        } else {
            None
        }
    }

    /**
     * Fallback: look for float3 patterns near the end of the data.
     */
    fn find_float3_trail(data: &[u8]) -> Option<Vec<f32>> {
        let len = data.len();
        if len < 48 {
            return None;
        }

        let start = len / 4;
        let end = (len * 3) / 4;

        let mut candidates: Vec<f32> = Vec::new();
        let mut i = start;

        while i + 12 <= end {
            let x = ByteUtils::read_f32_le(data, i);
            let y = ByteUtils::read_f32_le(data, i + 4);
            let z = ByteUtils::read_f32_le(data, i + 8);

            if x.is_finite()
                && y.is_finite()
                && z.is_finite()
                && (x.abs() < 1000.0)
                && (y.abs() < 1000.0)
                && (z.abs() < 1000.0)
            {
                candidates.push(x);
                candidates.push(y);
                candidates.push(z);
                i += 12;
            } else {
                if candidates.len() >= 12 {
                    break;
                }
                candidates.clear();
                i += 4;
            }
        }

        if candidates.len() >= 12 {
            Some(candidates)
        } else {
            None
        }
    }
}
