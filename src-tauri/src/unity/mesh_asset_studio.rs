/*
 * mesh_asset_studio.rs -- AssetStudio Mesh Parser Port
 *
 * Reads fields directly from the raw byte stream of a Mesh object, using exactly
 * the same version-branching logic as AssetStudio, without relying on TypeTree
 * field information.
 *
 * Follows Soul.md convention: struct + impl organization.
 */

use crate::unity::mesh_helper::MeshHelper;
use crate::unity::mesh_helper::VertexFormat;
use crate::utils::unity_reader_utils::UnityReader;

// ============================================================
// Parsed Result
// ============================================================

/// Mesh data extracted from AssetStudio-style parsing
///
/// Covers two scenarios:
/// 1. **No streaming data** (no .resS): `vertices`/`normals`/`uvs` directly available
/// 2. **With streaming data** (requires .resS): `has_streaming=true`, vertices in external resource,
///    caller must load .resS then call `process_stream_vertex_data()` to complete parsing
pub struct AssetStudioMesh {
    /// Final vertex positions
    pub vertices: Vec<f32>,
    /// Normals
    pub normals: Vec<f32>,
    /// Tangents (float4)
    pub tangents: Vec<f32>,
    /// Vertex colors (float4)
    pub colors: Vec<f32>,
    /// UV coordinates (float2)
    pub uvs: Vec<f32>,
    /// Skin weights, four weights per vertex
    pub bone_weights: Vec<f32>,
    /// Skin joint indices, four indices per vertex
    pub bone_indices: Vec<u16>,
    /// Bind pose matrices, 16 floats per bone
    pub bind_poses: Vec<f32>,
    /// Unity bone name hashes, one per bind pose when present
    pub bone_name_hashes: Vec<u32>,
    /// Unity root bone name hash when present
    pub root_bone_name_hash: Option<u32>,
    /// SubMesh ranges in the extracted triangle index list
    pub sub_meshes: Vec<ParsedSubMesh>,
    /// BlendShape frames converted to glTF-style sparse/full morph target deltas
    pub blend_shapes: Vec<ParsedBlendShape>,
    /// Triangle indices
    pub indices: Vec<u32>,

    // ============================================================
    // Streaming data metadata (valid when has_streaming=true)
    // ============================================================
    /// Whether vertex data is stored in an external .resS resource
    pub has_streaming: bool,
    /// StreamingInfo.path -- .resS file path/node name
    pub stream_path: String,
    /// StreamingInfo.offset -- offset within .resS
    pub stream_offset: u64,
    /// StreamingInfo.size -- data size
    pub stream_size: u32,

    /// Total Mesh vertex count (for buffer allocation)
    pub vertex_count: usize,
    /// Inline m_DataSize bytes (.resS data should be appended after this)
    pub data_size_bytes: Vec<u8>,
    /// Channel description array (serialization format, used for ReadVertexData)
    pub channels: Vec<ChannelInfo>,
    /// Stream description array
    pub streams: Vec<StreamInfo>,
}

#[derive(Debug, Clone, Copy)]
pub struct MeshParseOptions {
    pub include_auxiliary_data: bool,
}

impl Default for MeshParseOptions {
    fn default() -> Self {
        Self {
            include_auxiliary_data: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct VertexReadData {
    pub vertices: Vec<f32>,
    pub normals: Vec<f32>,
    pub uvs: Vec<f32>,
    pub tangents: Vec<f32>,
    pub colors: Vec<f32>,
    pub bone_weights: Vec<f32>,
    pub bone_indices: Vec<u16>,
}

// UnityReader has been migrated to src/utils/unity_reader_utils.rs

// ============================================================
// Sub-mesh information (mimics AssetStudio SubMesh)
// ============================================================

struct SubMeshInfo {
    first_byte: u32,
    index_count: u32,
    topology: i32,
    base_vertex: u32,
}

#[derive(Debug, Clone)]
pub struct ParsedSubMesh {
    pub index_start: usize,
    pub index_count: usize,
    pub topology: i32,
}

#[derive(Debug, Clone)]
pub struct ParsedBlendShape {
    pub name: String,
    pub weight: f32,
    pub delta_vertices: Vec<f32>,
    pub delta_normals: Vec<f32>,
    pub delta_tangents: Vec<f32>,
}

#[derive(Debug, Clone)]
struct BlendShapeVertex {
    vertex: [f32; 3],
    normal: [f32; 3],
    tangent: [f32; 3],
    index: u32,
}

#[derive(Debug, Clone)]
struct BlendShapeFrameInfo {
    first_vertex: u32,
    vertex_count: u32,
}

#[derive(Debug, Clone)]
struct BlendShapeChannelInfo {
    name: String,
    frame_index: i32,
    frame_count: i32,
}

// ============================================================
// Main Mesh parsing function -- port of AssetStudio Mesh(ObjectReader reader)
// ============================================================

/**
 * AssetStudio-style Mesh parser class.
 *
 * Fully ports the AssetStudio Mesh(ObjectReader reader) constructor logic,
 * reading byte by byte with precise field skipping based on Unity version.
 * All methods are static (no self), purely algorithmic encapsulation.
 *
 * Follows Soul.md convention: struct + impl organization, no free functions.
 */
pub struct AssetStudioMeshParser;

impl AssetStudioMeshParser {
    /**
     * Parse vertex and index data from raw Mesh object bytes.
     *
     * Fully ports the AssetStudio Mesh(ObjectReader reader) constructor logic,
     * reading byte by byte with precise field skipping based on Unity version.
     *
     * @param raw_data      Raw bytes of the Mesh object
     * @param unity_version Unity version string
     * @return              Parsed mesh data
     */
    /**
     * Parse the Mesh and return detailed per-field logs.
     *
     * Same as parse_mesh_from_raw, but collects per-field read logs.
     *
     * @param raw_data      Raw bytes of the Mesh object
     * @param unity_version Unity version string
     * @param field_logs    Output per-field log lines
     * @return              Parsed mesh data
     */
    pub fn parse_mesh_from_raw_with_logs(
        raw_data: &[u8],
        unity_version: &str,
        field_logs: &mut Vec<String>,
    ) -> Result<AssetStudioMesh, String> {
        Self::parse_mesh_from_raw_internal(
            raw_data,
            unity_version,
            field_logs,
            MeshParseOptions::default(),
        )
    }

    pub fn parse_mesh_preview_from_raw_with_logs(
        raw_data: &[u8],
        unity_version: &str,
        field_logs: &mut Vec<String>,
    ) -> Result<AssetStudioMesh, String> {
        Self::parse_mesh_from_raw_internal(
            raw_data,
            unity_version,
            field_logs,
            MeshParseOptions {
                include_auxiliary_data: false,
            },
        )
    }

    /// Simplified interface without logs
    #[allow(dead_code)]
    pub fn parse_mesh_from_raw(
        raw_data: &[u8],
        unity_version: &str,
    ) -> Result<AssetStudioMesh, String> {
        Self::parse_mesh_from_raw_internal(
            raw_data,
            unity_version,
            &mut vec![],
            MeshParseOptions::default(),
        )
    }

    fn parse_mesh_from_raw_internal(
        raw_data: &[u8],
        unity_version: &str,
        logs: &mut Vec<String>,
        options: MeshParseOptions,
    ) -> Result<AssetStudioMesh, String> {
        if raw_data.len() < 64 {
            return Err("Mesh raw data too short".to_string());
        }

        // Safety limits: prevent massive allocations when reading garbage values
        const MAX_VERTICES: usize = 1_000_000; // Max 1 million vertices
        const MAX_INDICES: usize = 10_000_000; // Max 10 million indices
        const MAX_SUBMESHES: i32 = 64; // Max 64 sub-meshes

        let mut r = UnityReader::new(raw_data, unity_version);
        let ver = r.get_version();

        logs.push(format!(
            "Creating UnityReader: version=[{}.{}.{}.{}], data_len={}",
            ver[0],
            ver[1],
            ver[2],
            ver[3],
            raw_data.len()
        ));

        // ---- Mimic Mesh(ObjectReader) : base(reader) read m_Name first ----
        // NamedObject base class reads m_Name
        let mesh_name = r.read_aligned_string();
        logs.push(format!(
            "Reading m_Name = \"{}\"",
            if mesh_name.is_empty() {
                "(empty)"
            } else {
                &mesh_name
            }
        ));

        // ---- BLOCK 1: Index format flag for very old versions (3.5 down) ----
        let mut use_16bit = true;
        if ver[0] < 3 || (ver[0] == 3 && ver[1] < 5) {
            use_16bit = r.read_i32() > 0;
            logs.push(format!("Reading m_Use16BitIndices = {}", use_16bit));
        } else {
            logs.push(format!("Index format: default 16-bit (version >= 3.5)"));
        }

        // ---- BLOCK 2: Index buffer for versions 2.5 and below ----
        if ver[0] == 2 && ver[1] <= 5 {
            let idx_buf_size = r.read_i32();
            logs.push(format!(
                "Reading legacy index buffer: size={}B",
                idx_buf_size
            ));
            if use_16bit {
                let count = idx_buf_size as usize / 2;
                for _ in 0..count {
                    r.read_u16();
                }
                r.align();
            } else {
                let count = idx_buf_size as usize / 4;
                for _ in 0..count {
                    r.read_u32();
                }
            }
        }

        // ---- BLOCK 3: SubMeshes (all versions) ----
        let sub_mesh_count = r.read_i32().clamp(0, MAX_SUBMESHES);
        logs.push(format!("Reading m_SubMeshes: count={}", sub_mesh_count));
        let mut sub_meshes: Vec<SubMeshInfo> = Vec::new();
        for sm_idx in 0..sub_mesh_count {
            let first_byte = r.read_u32();
            let index_count = r.read_u32();
            let topology = r.read_i32();
            let mut sm_log = format!(
                "  SubMesh[{}]: firstByte={}, indexCount={}, topology={}",
                sm_idx, first_byte, index_count, topology
            );
            // 4.0 down: triangleCount
            if ver[0] < 4 {
                let tri_count = r.read_u32();
                sm_log.push_str(&format!(", triangleCount={}", tri_count));
            }
            // 2017.3+: baseVertex
            let base_vertex = if ver[0] > 2017 || (ver[0] == 2017 && ver[1] >= 3) {
                let bv = r.read_u32();
                sm_log.push_str(&format!(", baseVertex={}", bv));
                bv
            } else {
                0
            };
            // 3.0+: firstVertex, vertexCount, localAABB
            if ver[0] >= 3 {
                let fv = r.read_u32();
                let vc = r.read_u32();
                let min_x = r.read_f32();
                let min_y = r.read_f32();
                let min_z = r.read_f32();
                let max_x = r.read_f32();
                let max_y = r.read_f32();
                let max_z = r.read_f32();
                sm_log.push_str(&format!(", firstVertex={}, vertexCount={}, aabb=({:.3},{:.3},{:.3})-({:.3},{:.3},{:.3})",
                fv, vc, min_x, min_y, min_z, max_x, max_y, max_z));
            }
            logs.push(sm_log);
            sub_meshes.push(SubMeshInfo {
                first_byte,
                index_count,
                topology,
                base_vertex,
            });
        }

        // ---- BLOCK 4: BlendShapes (4.1 and up) ----
        let has_blend_shapes = ver[0] > 4 || (ver[0] == 4 && ver[1] >= 1);
        logs.push(format!(
            "Reading m_Shapes (BlendShapes): present={}",
            has_blend_shapes
        ));
        let mut blend_shape_vertices: Vec<BlendShapeVertex> = Vec::new();
        let mut blend_shape_frames: Vec<BlendShapeFrameInfo> = Vec::new();
        let mut blend_shape_channels: Vec<BlendShapeChannelInfo> = Vec::new();
        let mut blend_shape_weights: Vec<f32> = Vec::new();
        if has_blend_shapes {
            if ver[0] > 4 || (ver[0] == 4 && ver[1] >= 3) {
                let num_verts = r.read_i32();
                logs.push(format!("  BlendShape 4.3+ path: vertices={}", num_verts));
                if options.include_auxiliary_data {
                    for _ in 0..num_verts {
                        let vertex = [r.read_f32(), r.read_f32(), r.read_f32()];
                        let normal = [r.read_f32(), r.read_f32(), r.read_f32()];
                        let tangent = [r.read_f32(), r.read_f32(), r.read_f32()];
                        let index = r.read_u32();
                        blend_shape_vertices.push(BlendShapeVertex {
                            vertex,
                            normal,
                            tangent,
                            index,
                        });
                    }
                } else if num_verts > 0 {
                    r.skip(num_verts as usize * 40);
                }
                let num_shapes = r.read_i32();
                logs.push(format!("  BlendShapes: {} shapes", num_shapes));
                for _ in 0..num_shapes {
                    let first_vertex = r.read_u32();
                    let vertex_count = r.read_u32();
                    let _ = r.read_bool(); // hasNormals
                    let _ = r.read_bool(); // hasTangents
                    r.align();
                    if options.include_auxiliary_data {
                        blend_shape_frames.push(BlendShapeFrameInfo {
                            first_vertex,
                            vertex_count,
                        });
                    }
                }
                let num_channels = r.read_i32();
                logs.push(format!("  BlendShapeChannels: {} channels", num_channels));
                for _ in 0..num_channels {
                    let ch_name = r.read_aligned_string();
                    let name_hash = r.read_u32();
                    let frame_index = r.read_i32();
                    let frame_count = r.read_i32();
                    logs.push(format!("    Channel: name=\"{}\"", ch_name));
                    let _ = name_hash;
                    if options.include_auxiliary_data {
                        blend_shape_channels.push(BlendShapeChannelInfo {
                            name: ch_name,
                            frame_index,
                            frame_count,
                        });
                    }
                }
                let full_weights_count = r.read_i32();
                logs.push(format!(
                    "  BlendShape fullWeights: {} items",
                    full_weights_count
                ));
                if options.include_auxiliary_data
                    && full_weights_count > 0
                    && full_weights_count as usize <= r.remaining() / 4
                {
                    blend_shape_weights = r.read_f32_array(full_weights_count as usize);
                } else if full_weights_count > 0 {
                    r.skip(full_weights_count as usize * 4);
                }
            } else {
                let shapes_size = r.read_i32();
                logs.push(format!("  BlendShape 4.1-4.2 path: {} shapes", shapes_size));
                for _ in 0..shapes_size {
                    if ver[0] == 4 && ver[1] < 3 {
                        let _name = r.read_aligned_string();
                    }
                    r.skip(8);
                    if ver[0] == 4 && ver[1] < 3 {
                        r.skip(24);
                    }
                    let _ = r.read_bool();
                    let _ = r.read_bool();
                }
                r.align();
                let shape_verts_size = r.read_i32();
                logs.push(format!("  BlendShape vertices: {} items", shape_verts_size));
                for _ in 0..shape_verts_size {
                    let _ = (r.read_f32(), r.read_f32(), r.read_f32());
                    let _ = (r.read_f32(), r.read_f32(), r.read_f32());
                    let _ = (r.read_f32(), r.read_f32(), r.read_f32());
                    let _ = r.read_u32();
                }
            }
        }

        let mut bone_weights: Vec<f32> = Vec::new();
        let mut bone_indices: Vec<u16> = Vec::new();
        let mut bind_poses: Vec<f32> = Vec::new();
        let mut bone_name_hashes: Vec<u32> = Vec::new();
        let mut root_bone_name_hash: Option<u32> = None;
        let parsed_blend_shapes: Vec<ParsedBlendShape>;

        // ---- BLOCK 5: BindPose + BoneNameHashes (4.3 and up) ----
        if ver[0] > 4 || (ver[0] == 4 && ver[1] >= 3) {
            let bind_pose_count = r.read_i32();
            logs.push(format!("Reading BindPose: {} matrices", bind_pose_count));
            if bind_pose_count > 0 && bind_pose_count < 1_000_000 {
                bind_poses = r.read_f32_array(bind_pose_count as usize * 16);
            }
            let bone_hash_count = r.read_i32();
            logs.push(format!(
                "Reading BoneNameHashes: {} hashes",
                bone_hash_count
            ));
            if bone_hash_count > 0 && bone_hash_count < 1_000_000 {
                bone_name_hashes = r.read_u32_array(bone_hash_count as usize);
            }
            let root_bone_hash = r.read_u32();
            logs.push(format!(
                "Reading m_RootBoneNameHash = 0x{:08X}",
                root_bone_hash
            ));
            root_bone_name_hash = Some(root_bone_hash);
        }

        // ---- BLOCK 6: 2.6.0 and up -- main data block ----
        let mut index_buffer: Vec<u32> = Vec::new();
        if ver[0] > 2 || (ver[0] == 2 && ver[1] >= 6) {
            // 6a: BonesAABB (2019 and up)
            if ver[0] >= 2019 {
                let bones_aabb_size = r.read_i32();
                logs.push(format!("Reading m_BonesAABB: {} groups", bones_aabb_size));
                for _ in 0..bones_aabb_size {
                    let _ = (r.read_f32(), r.read_f32(), r.read_f32());
                    let _ = (r.read_f32(), r.read_f32(), r.read_f32());
                }
                let var_bone_count_count = r.read_i32();
                logs.push(format!(
                    "Reading m_VariableBoneCountWeights: {} items",
                    var_bone_count_count
                ));
                let _var_bone_count = r.read_u32_array(var_bone_count_count.clamp(0, 256) as usize);
            }

            // 6b: MeshCompression
            let mesh_compression = r.read_byte();
            logs.push(format!("Reading m_MeshCompression = {}", mesh_compression));
            if ver[0] >= 4 {
                if ver[0] < 5 {
                    let _stream_compression = r.read_byte();
                }
                let is_readable = r.read_bool();
                let keep_vertices = r.read_bool();
                let keep_indices = r.read_bool();
                logs.push(format!(
                    "Reading m_IsReadable={}, m_KeepVertices={}, m_KeepIndices={}",
                    is_readable, keep_vertices, keep_indices
                ));
            }
            r.align();

            // 6c: IndexFormat (2017.3.1px+ fix)
            // AssetStudio has three branches: 2017.4+, 2017.3.1p1+ (with buildType.IsPatch), and 2017.3.x + no compression
            // Since buildType information is unavailable, 2017.3.x only keeps the mesh_compression==0 branch
            if (ver[0] > 2017 || (ver[0] == 2017 && ver[1] >= 4))
                || (ver[0] == 2017 && ver[1] == 3 && mesh_compression == 0)
            {
                let index_format = r.read_i32();
                use_16bit = index_format == 0;
                logs.push(format!(
                    "Reading m_IndexFormat = {} (16bit={})",
                    index_format, use_16bit
                ));
            }

            // 6d: IndexBuffer
            let idx_buf_size = r.read_i32().min(r.remaining() as i32);
            logs.push(format!(
                "Reading m_IndexBuffer: size={}B, format={}",
                idx_buf_size,
                if use_16bit { "u16" } else { "u32" }
            ));
            if use_16bit {
                let count = (idx_buf_size as usize / 2).min(MAX_INDICES);
                for _ in 0..count {
                    index_buffer.push(r.read_u16() as u32);
                }
                r.align();
            } else {
                let count = (idx_buf_size as usize / 4).min(MAX_INDICES);
                index_buffer = r.read_u32_array(count);
            }
            logs.push(format!(
                "  IndexBuffer actually read: {} indices",
                index_buffer.len()
            ));
        }

        // ---- BLOCK 7: OLD format vs NEW VertexData (version fork) ----
        #[allow(unused_assignments)]
        let mut vertex_count: usize = 0;
        let mut vertices: Vec<f32> = Vec::new();
        let mut normals: Vec<f32> = Vec::new();
        let mut tangents: Vec<f32> = Vec::new();
        let mut colors: Vec<f32> = Vec::new();
        let mut uvs: Vec<f32> = Vec::new();

        // Store NEW PATH VertexData results, deferring to ProcessData stage
        // Mimics AssetStudio: constructor only reads fields, ProcessData() calls ReadVertexData()
        let mut deferred_channels: Vec<ChannelInfo> = Vec::new();
        let mut deferred_streams: Vec<StreamInfo> = Vec::new();
        let mut deferred_data_size: Vec<u8> = Vec::new();
        let mut deferred_vertex_count: usize = 0;

        if ver[0] < 3 || (ver[0] == 3 && ver[1] < 5) {
            // === OLD PATH (3.4.2 and earlier): direct arrays ===
            vertex_count = (r.read_i32() as usize).min(MAX_VERTICES);
            vertices = r.read_f32_array(vertex_count * 3);
            parsed_blend_shapes = if options.include_auxiliary_data {
                Self::build_blend_shapes(
                    &blend_shape_vertices,
                    &blend_shape_frames,
                    &blend_shape_channels,
                    &blend_shape_weights,
                    vertex_count,
                )
            } else {
                Vec::new()
            };

            let skin_count = r.read_i32() as usize;
            logs.push(format!("Reading Skin (BoneWeights4): {} items", skin_count));
            for _ in 0..skin_count {
                for _ in 0..4 {
                    bone_weights.push(r.read_f32());
                }
                for _ in 0..4 {
                    bone_indices.push(r.read_u32().min(u16::MAX as u32) as u16);
                }
            }

            let bind_pose_count = r.read_i32();
            logs.push(format!(
                "Reading BindPose: {} matrices (old path)",
                bind_pose_count
            ));
            if bind_pose_count > 0 && bind_pose_count < 1_000_000 {
                bind_poses = r.read_f32_array(bind_pose_count as usize * 16);
            }
            let uv0_count = r.read_i32() as usize;
            logs.push(format!("Reading UVs: {} items", uv0_count));
            uvs = r.read_f32_array(uv0_count * 2);
            let uv1_count = r.read_i32();
            logs.push(format!("Reading UV1: {} items (skipping)", uv1_count));
            if uv1_count > 0 && uv1_count < 1_000_000 {
                r.skip(uv1_count as usize * 2 * 4);
            }

            if ver[0] == 2 && ver[1] <= 5 {
                let ts_size = r.read_i32() as usize;
                logs.push(format!(
                    "Reading TangentSpace (interleaved): {} items",
                    ts_size
                ));
                for _ in 0..ts_size {
                    let _nx = r.read_f32();
                    let _ny = r.read_f32();
                    let _nz = r.read_f32();
                    let _tx = r.read_f32();
                    let _ty = r.read_f32();
                    let _tz = r.read_f32();
                    let _tw = r.read_f32();
                }
            } else {
                let tangent_count = r.read_i32() as usize;
                tangents = r.read_f32_array(tangent_count * 4);
                logs.push(format!("Reading Tangents: {} items", tangent_count));
                let normal_count = r.read_i32() as usize;
                normals = r.read_f32_array(normal_count * 3);
                logs.push(format!("Reading Normals: {} items", normal_count));
            }
        } else {
            // === NEW PATH: VertexData structure ===
            if ver[0] < 2018 || (ver[0] == 2018 && ver[1] < 2) {
                let skin_count = r.read_i32() as usize;
                logs.push(format!("Reading Skin (BoneWeights4): {} items", skin_count));
                for _ in 0..skin_count {
                    for _ in 0..4 {
                        bone_weights.push(r.read_f32());
                    }
                    for _ in 0..4 {
                        bone_indices.push(r.read_u32().min(u16::MAX as u32) as u16);
                    }
                }
            }

            if ver[0] == 3 || (ver[0] == 4 && ver[1] <= 2) {
                let bp_count = r.read_i32() as usize;
                logs.push(format!("Reading BindPose (inline): {} matrices", bp_count));
                bind_poses = r.read_f32_array(bp_count * 16);
            }

            // === Core: read VertexData ===
            if ver[0] < 2018 {
                let current_channels = r.read_u32();
                logs.push(format!(
                    "Reading m_CurrentChannels = 0x{:X}",
                    current_channels
                ));
            }

            vertex_count = (r.read_u32() as usize).min(MAX_VERTICES);
            logs.push(format!(
                "Reading m_VertexData.m_VertexCount = {}",
                vertex_count
            ));
            parsed_blend_shapes = if options.include_auxiliary_data {
                Self::build_blend_shapes(
                    &blend_shape_vertices,
                    &blend_shape_frames,
                    &blend_shape_channels,
                    &blend_shape_weights,
                    vertex_count,
                )
            } else {
                Vec::new()
            };

            // m_Channels
            let mut channels: Vec<ChannelInfo> = Vec::new();
            if ver[0] >= 4 {
                let ch_count = r.read_i32();
                logs.push(format!(
                    "Reading m_VertexData.m_Channels: {} channels",
                    ch_count
                ));
                for ch_idx in 0..ch_count {
                    let stream = r.read_u8();
                    let offset = r.read_u8();
                    let format = r.read_u8();
                    let dimension = r.read_u8() & 0xF;
                    logs.push(format!(
                        "  Channel[{}]: stream={}, offset={}, format={}, dimension={}",
                        ch_idx, stream, offset, format, dimension
                    ));
                    channels.push(ChannelInfo {
                        stream,
                        offset,
                        format,
                        dimension,
                    });
                }
            }

            // m_Streams
            let mut streams: Vec<StreamInfo> = Vec::new();
            if ver[0] < 5 {
                if ver[0] < 4 {
                    logs.push(format!("Reading m_VertexData.m_Streams: fixed 4 (pre-4.0)"));
                    for st_idx in 0..4 {
                        let ch_mask = r.read_u32();
                        let offset = r.read_u32();
                        let stride = r.read_u32();
                        let _align = r.read_u32();
                        logs.push(format!(
                            "  Stream[{}]: channelMask=0x{:X}, offset={}, stride={}",
                            st_idx, ch_mask, offset, stride
                        ));
                        streams.push(StreamInfo {
                            channel_mask: ch_mask,
                            offset,
                            stride,
                        });
                    }
                } else {
                    let st_count = r.read_i32() as usize;
                    logs.push(format!(
                        "Reading m_VertexData.m_Streams: {} streams (4.x)",
                        st_count
                    ));
                    for st_idx in 0..st_count {
                        let ch_mask = r.read_u32();
                        let offset = r.read_u32();
                        let stride = r.read_u8() as u32;
                        let _divider = r.read_u8();
                        let _freq = r.read_u16();
                        logs.push(format!(
                            "  Stream[{}]: channelMask=0x{:X}, offset={}, stride={}",
                            st_idx, ch_mask, offset, stride
                        ));
                        streams.push(StreamInfo {
                            channel_mask: ch_mask,
                            offset,
                            stride,
                        });
                    }
                }
                if ver[0] < 4 {
                    logs.push(format!("  Deriving Channels (pre-4.0): from stream mask"));
                    Self::derive_channels_pre_v4(&mut channels, &streams, &ver);
                }
            } else {
                streams = Self::compute_streams_from_channels(&channels, vertex_count, &ver);
                logs.push(format!(
                    "Computing m_VertexData.m_Streams (5.0+): {} streams",
                    streams.len()
                ));
                for st_idx in 0..streams.len() {
                    logs.push(format!(
                        "  Stream[{}]: channelMask=0x{:X}, offset={}, stride={}",
                        st_idx,
                        streams[st_idx].channel_mask,
                        streams[st_idx].offset,
                        streams[st_idx].stride
                    ));
                }
            }

            // m_DataSize
            deferred_data_size = r.read_u8_array();
            logs.push(format!(
                "Reading m_VertexData.m_DataSize: {} bytes",
                deferred_data_size.len()
            ));
            r.align();

            deferred_channels = channels;
            deferred_streams = streams;
            deferred_vertex_count = vertex_count;
        }

        // ---- BLOCK 8: CompressedMesh ----
        let mut compressed_mesh: Option<CompressedMeshData> = None;
        logs.push(format!("Reading CompressedMesh:"));
        if ver[0] > 2 || (ver[0] == 2 && ver[1] >= 6) {
            let cm = Self::read_compressed_mesh_with_logs(&mut r, &ver, logs);
            logs.push(format!(
                "  CompressedMesh: verts={}, tris={}, norms={}",
                cm.vertices.num_items, cm.triangles.num_items, cm.normals.num_items
            ));
            compressed_mesh = Some(cm);
        } else {
            logs.push(format!("  Skipping (version < 2.6)"));
        }

        // ---- BLOCK 9: AABB ----
        let aabb_min_x = r.read_f32();
        let aabb_min_y = r.read_f32();
        let aabb_min_z = r.read_f32();
        let aabb_max_x = r.read_f32();
        let aabb_max_y = r.read_f32();
        let aabb_max_z = r.read_f32();
        logs.push(format!(
            "Reading m_LocalAABB: min=({:.4},{:.4},{:.4}) max=({:.4},{:.4},{:.4})",
            aabb_min_x, aabb_min_y, aabb_min_z, aabb_max_x, aabb_max_y, aabb_max_z
        ));

        // ---- BLOCK 10: Old colors (3.4.2 and earlier) ----
        if ver[0] < 3 || (ver[0] == 3 && ver[1] <= 4) {
            let colors_size = r.read_i32();
            logs.push(format!("Reading legacy Colors: {} items", colors_size));
            if colors_size > 0 && colors_size < 1_000_000 {
                r.skip(colors_size as usize * 4);
            }
            let collision_tri_size = r.read_i32();
            logs.push(format!(
                "Reading legacy CollisionTriangles: {} items (skipping)",
                collision_tri_size
            ));
            if collision_tri_size > 0 && collision_tri_size < 1_000_000 {
                r.skip(collision_tri_size as usize * 4);
            }
            let collision_vert_count = r.read_i32();
            logs.push(format!(
                "Reading legacy CollisionVertices: {}",
                collision_vert_count
            ));
        }

        // ---- BLOCK 11: MeshUsageFlags ----
        let mesh_usage_flags = r.read_i32();
        logs.push(format!("Reading m_MeshUsageFlags = {}", mesh_usage_flags));

        // ---- BLOCK 12: CookingOptions (2022.1+) ----
        if ver[0] > 2022 || (ver[0] == 2022 && ver[1] >= 1) {
            let cooking_options = r.read_i32();
            logs.push(format!("Reading CookingOptions = {}", cooking_options));
        }

        // ---- BLOCK 13: CollisionMesh (5.0+) ----
        if ver[0] >= 5 {
            let convex = r.read_u8_array();
            logs.push(format!(
                "Reading m_BakedConvexCollisionMesh: {} bytes",
                convex.len()
            ));
            r.align();
            let triangle_mesh = r.read_u8_array();
            logs.push(format!(
                "Reading m_BakedTriangleCollisionMesh: {} bytes",
                triangle_mesh.len()
            ));
            r.align();
        }

        // ---- BLOCK 14: MeshMetrics (2018.2+) ----
        if ver[0] > 2018 || (ver[0] == 2018 && ver[1] >= 2) {
            let mm0 = r.read_f32();
            let mm1 = r.read_f32();
            logs.push(format!("Reading m_MeshMetrics: [{:.6}, {:.6}]", mm0, mm1));
        }

        // ---- BLOCK 15: StreamingInfo (2018.3+) -- ports AssetStudio StreamingInfo(reader) ----
        // AssetStudio read order: offset -> size -> path
        let mut stream_path: Option<String> = None;
        let mut _stream_offset: u64 = 0;
        let mut _stream_size: u32 = 0;
        if ver[0] > 2018 || (ver[0] == 2018 && ver[1] >= 3) {
            r.align();
            // AssetStudio: offset -> size -> path
            _stream_offset = if ver[0] >= 2020 {
                r.read_i64() as u64
            } else {
                r.read_u32() as u64
            };
            _stream_size = r.read_u32();
            stream_path = Some(r.read_aligned_string());
            logs.push(format!(
                "Reading m_StreamData: path=\"{}\", offset={}, size={}",
                stream_path
                    .as_ref()
                    .map(|s| s.as_str())
                    .unwrap_or("(empty)"),
                _stream_offset,
                _stream_size
            ));
        } else {
            logs.push(format!("m_StreamData: skipping (version < 2018.3)"));
        }

        // ================================================================
        // ProcessData() -- ports AssetStudio Mesh.ProcessData()
        // ================================================================
        logs.push(format!(""));
        logs.push(format!("===== ProcessData Stage ====="));

        let has_streaming_data = stream_path.as_ref().map_or(false, |p| !p.is_empty());
        let data_is_empty = deferred_data_size.is_empty() || deferred_data_size.len() < 4;

        let data_is_partial = if has_streaming_data && !data_is_empty {
            true
        } else {
            false
        };
        logs.push(format!(
            "ProcessData: has_streaming={}, data_is_empty={}, data_is_partial={}",
            has_streaming_data, data_is_empty, data_is_partial
        ));

        if has_streaming_data && (data_is_empty || data_is_partial) {
            let (indices, parsed_sub_meshes) =
                Self::extract_triangles_with_ranges(&index_buffer, &sub_meshes, use_16bit);
            logs.push(format!(
                "Streaming data path: returning {} indices, awaiting external .resS loading",
                indices.len()
            ));
            return Ok(AssetStudioMesh {
                vertices: vec![],
                normals: vec![],
                tangents: vec![],
                colors: vec![],
                uvs: vec![],
                bone_weights,
                bone_indices,
                bind_poses,
                bone_name_hashes,
                root_bone_name_hash,
                sub_meshes: parsed_sub_meshes,
                blend_shapes: parsed_blend_shapes,
                indices,
                has_streaming: true,
                stream_path: stream_path.unwrap_or_default(),
                stream_offset: _stream_offset,
                stream_size: _stream_size,
                vertex_count: deferred_vertex_count,
                data_size_bytes: deferred_data_size,
                channels: deferred_channels,
                streams: deferred_streams,
            });
        }
        if !has_streaming_data
            && data_is_empty
            && !deferred_channels.is_empty()
            && deferred_channels.iter().any(|c| c.dimension > 0)
            && deferred_vertex_count > 0
        {
            let (indices, parsed_sub_meshes) =
                Self::extract_triangles_with_ranges(&index_buffer, &sub_meshes, use_16bit);
            logs.push(format!("Fallback path: has_streaming=false but has channels+empty data, returning awaiting .resS load, indices={}", indices.len()));
            return Ok(AssetStudioMesh {
                vertices: vec![],
                normals: vec![],
                tangents: vec![],
                colors: vec![],
                uvs: vec![],
                bone_weights,
                bone_indices,
                bind_poses,
                bone_name_hashes,
                root_bone_name_hash,
                sub_meshes: parsed_sub_meshes,
                blend_shapes: parsed_blend_shapes,
                indices,
                has_streaming: false,
                stream_path: String::new(),
                stream_offset: 0,
                stream_size: 0,
                vertex_count: deferred_vertex_count,
                data_size_bytes: vec![],
                channels: deferred_channels,
                streams: deferred_streams,
            });
        }

        // Step 2: ReadVertexData -- ports AssetStudio call in ProcessData
        logs.push(format!(
            "ReadVertexData: channels={}, data={}B, vertex_count={}",
            deferred_channels.len(),
            deferred_data_size.len(),
            deferred_vertex_count
        ));
        let mut diag_info: Vec<String> = Vec::new();
        if !deferred_channels.is_empty()
            && !deferred_data_size.is_empty()
            && deferred_vertex_count > 0
        {
            let ch_count = deferred_channels.iter().filter(|c| c.dimension > 0).count();
            let total_channels = deferred_channels.len();
            logs.push(format!(
                "ReadVertexData input: {} channels ({} active), {} streams, data={}B, verts={}",
                total_channels,
                ch_count,
                deferred_streams.len(),
                deferred_data_size.len(),
                deferred_vertex_count
            ));

            let vertex_data = Self::read_vertex_data(
                &deferred_channels,
                &deferred_streams,
                &deferred_data_size,
                deferred_vertex_count,
                &ver,
                false,
            );
            vertices = vertex_data.vertices;
            normals = vertex_data.normals;
            uvs = vertex_data.uvs;
            tangents = vertex_data.tangents;
            colors = vertex_data.colors;
            if !vertex_data.bone_weights.is_empty() {
                bone_weights = vertex_data.bone_weights;
            }
            if !vertex_data.bone_indices.is_empty() {
                bone_indices = vertex_data.bone_indices;
            }
            vertex_count = deferred_vertex_count;
            logs.push(format!("ReadVertexData result: vertices={}, normals={}, uvs={}, tangents={}, colors={}, skin_weights={}, skin_indices={}",
            vertices.len()/3, normals.len()/3, uvs.len()/2, tangents.len()/4, colors.len()/4,
            bone_weights.len()/4, bone_indices.len()/4));
            let _ = vertex_count;
        } else {
            if deferred_channels.is_empty() {
                diag_info.push("channels empty".to_string());
            }
            if deferred_data_size.is_empty() {
                diag_info.push("data_size empty".to_string());
            }
            if deferred_vertex_count == 0 {
                diag_info.push("vertex_count=0".to_string());
            }
            logs.push(format!("ReadVertexData skipped: {}", diag_info.join("; ")));
        }

        // Step 3: DecompressCompressedMesh -- ports AssetStudio ProcessData()
        // When VertexData is empty but CompressedMesh has data, decompress compressed vertices
        if vertices.is_empty() || vertex_count == 0 {
            if let Some(ref cm) = compressed_mesh {
                if cm.vertices.num_items > 0 {
                    let unpacked = cm.vertices.unpack_floats(3);
                    if !unpacked.is_empty() {
                        vertices = unpacked;
                        vertex_count = vertices.len() / 3;
                        logs.push(format!(
                            "DecompressCompressedMesh: decompressed {} vertices from m_Vertices",
                            vertex_count
                        ));
                    }
                }
                if vertices.is_empty() {
                    if let Some(ref cm) = compressed_mesh {
                        if cm.vertices.num_items > 0 {
                            // Fallback: try decompressing with itemCountInChunk=1
                            let unpacked = cm.vertices.unpack_floats(1);
                            if !unpacked.is_empty() {
                                vertices = unpacked;
                                vertex_count = vertices.len() / 3;
                                logs.push(format!("DecompressCompressedMesh(1): decompressed {} vertices from m_Vertices", vertex_count));
                            }
                        }
                    }
                }
                if uvs.is_empty() && cm.uv.num_items > 0 {
                    let unpacked = cm.uv.unpack_floats(2);
                    if !unpacked.is_empty() {
                        uvs = unpacked;
                        logs.push(format!(
                            "DecompressCompressedMesh: decompressed {} floats from m_UV",
                            uvs.len()
                        ));
                    }
                }
                if normals.is_empty() && cm.normals.num_items > 0 {
                    let unpacked = cm.normals.unpack_floats(3);
                    if !unpacked.is_empty() {
                        normals = unpacked;
                        logs.push(format!(
                            "DecompressCompressedMesh: decompressed {} floats from m_Normals",
                            normals.len()
                        ));
                    }
                }
            }
        }

        // Step 3b: If indices are still empty, decompress triangle indices from CompressedMesh
        if index_buffer.is_empty() {
            if let Some(ref cm) = compressed_mesh {
                if cm.triangles.num_items > 0 {
                    let packed_indices = cm.triangles.unpack_u32s();
                    if !packed_indices.is_empty() {
                        index_buffer = packed_indices;
                        logs.push(format!(
                            "DecompressCompressedMesh: decompressed {} indices from m_Triangles",
                            index_buffer.len()
                        ));
                    }
                }
            }
        }

        // Step 4: GetTriangles -- extract triangle indices
        let (indices, parsed_sub_meshes) =
            Self::extract_triangles_with_ranges(&index_buffer, &sub_meshes, use_16bit);
        logs.push(format!(
            "GetTriangles: {} indices ({} triangles)",
            indices.len(),
            indices.len() / 3
        ));

        if vertices.is_empty() && !has_streaming_data {
            let reasons: Vec<String> = diag_info.clone();
            let reason = if reasons.is_empty() {
                "Unknown reason".to_string()
            } else {
                reasons.join("; ")
            };
            logs.push(format!(
                "AssetStudio parsing failed: could not extract vertex data -- {}",
                reason
            ));
            return Err(format!(
                "AssetStudio parsing: could not extract vertex data -- {}",
                reason
            ));
        }

        logs.push(format!(
            "AssetStudio parsing complete: {} vertices, {} triangles (inline)",
            vertices.len() / 3,
            indices.len() / 3
        ));
        Ok(AssetStudioMesh {
            vertices,
            normals,
            tangents,
            colors,
            uvs,
            bone_weights,
            bone_indices,
            bind_poses,
            bone_name_hashes,
            root_bone_name_hash,
            sub_meshes: parsed_sub_meshes,
            blend_shapes: parsed_blend_shapes,
            indices,
            has_streaming: false,
            stream_path: String::new(),
            stream_offset: 0,
            stream_size: 0,
            vertex_count: 0,
            data_size_bytes: vec![],
            channels: vec![],
            streams: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AssetStudioMeshParser, SubMeshInfo};

    #[test]
    fn extract_triangles_applies_submesh_base_vertex() {
        let index_buffer = vec![0, 1, 2, 0, 1, 2];
        let sub_meshes = vec![
            SubMeshInfo {
                first_byte: 0,
                index_count: 3,
                topology: 0,
                base_vertex: 0,
            },
            SubMeshInfo {
                first_byte: 6,
                index_count: 3,
                topology: 0,
                base_vertex: 10,
            },
        ];

        let (indices, ranges) =
            AssetStudioMeshParser::extract_triangles_with_ranges(&index_buffer, &sub_meshes, true);

        assert_eq!(indices, vec![0, 1, 2, 10, 11, 12]);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].index_start, 0);
        assert_eq!(ranges[0].index_count, 3);
        assert_eq!(ranges[1].index_start, 3);
        assert_eq!(ranges[1].index_count, 3);
    }
}

// ============================================================
// PackedFloatVector -- ports AssetStudio PackedFloatVector
// ============================================================

/// Compressed floating-point vector, stored in bit-packed format.
/// Reference: AssetStudio PackedFloatVector.UnpackFloats()
#[derive(Clone, Default)]
struct PackedFloatVector {
    num_items: u32,
    range: f32,
    start: f32,
    bit_size: u32,
    data: Vec<u8>,
}

impl PackedFloatVector {
    fn unpack_floats(&self, item_count_in_chunk: usize) -> Vec<f32> {
        if self.bit_size == 0 || self.data.is_empty() {
            return vec![];
        }
        let bit_count = self.bit_size as usize;
        let unpacked_size = self.num_items as usize * item_count_in_chunk;
        let mut unpacked = vec![0.0f32; unpacked_size];
        let total_bits = self.data.len() * 8;
        let mut bit_index = 0;

        for i in 0..self.num_items as usize {
            if bit_index >= total_bits {
                break;
            }
            let mut value: u32 = 0;
            for b in 0..bit_count {
                if bit_index >= total_bits {
                    break;
                }
                let byte_idx = bit_index >> 3;
                let bit_off = bit_index & 7;
                if byte_idx < self.data.len() {
                    value |= (((self.data[byte_idx] >> bit_off) & 1) as u32) << b;
                }
                bit_index += 1;
            }
            let max_val = (1u32 << bit_count).saturating_sub(1);
            let f = if max_val > 0 {
                value as f32 / max_val as f32
            } else {
                0.0
            };
            unpacked[i] = f * self.range + self.start;
        }

        // De-interleave: PackedFloatVector stores values interleaved by component,
        // reorder by itemCountInChunk
        // AssetStudio: result[i] = unpacked[(i % itemCountInChunk) * m_NumItems + (i / itemCountInChunk)]
        if item_count_in_chunk > 1 {
            let mut result = vec![0.0f32; unpacked_size];
            for i in 0..unpacked_size {
                let src =
                    (i % item_count_in_chunk) * self.num_items as usize + (i / item_count_in_chunk);
                if src < unpacked_size {
                    result[i] = unpacked[src];
                }
            }
            return result;
        }
        unpacked
    }
}

/// Compressed integer vector, stored in bit-packed format.
/// Reference: AssetStudio PackedIntVector.UnpackInts()
#[derive(Clone, Default)]
struct PackedIntVector {
    num_items: u32,
    bit_size: u32,
    data: Vec<u8>,
}

impl PackedIntVector {
    fn unpack_u32s(&self) -> Vec<u32> {
        if self.bit_size == 0 || self.data.is_empty() {
            return vec![];
        }
        let bit_count = self.bit_size as usize;
        let unpacked_size = self.num_items as usize;
        let mut unpacked = vec![0u32; unpacked_size];
        let total_bits = self.data.len() * 8;
        let mut bit_index = 0;

        for i in 0..unpacked_size {
            if bit_index >= total_bits {
                break;
            }
            let mut value: u32 = 0;
            for b in 0..bit_count {
                if bit_index >= total_bits {
                    break;
                }
                let byte_idx = bit_index >> 3;
                let bit_off = bit_index & 7;
                if byte_idx < self.data.len() {
                    value |= (((self.data[byte_idx] >> bit_off) & 1) as u32) << b;
                }
                bit_index += 1;
            }
            unpacked[i] = value;
        }
        unpacked
    }
}

/// Compressed mesh data -- ports AssetStudio CompressedMesh
/// Some fields are not yet read by business logic, but are kept for complete decompression implementation.
#[derive(Clone, Default)]
#[allow(dead_code)]
struct CompressedMeshData {
    vertices: PackedFloatVector,
    uv: PackedFloatVector,
    bind_poses: PackedFloatVector,
    normals: PackedFloatVector,
    tangents: PackedFloatVector,
    weights: PackedIntVector,
    normal_signs: PackedIntVector,
    tangent_signs: PackedIntVector,
    float_colors: PackedFloatVector,
    bone_indices: PackedIntVector,
    triangles: PackedIntVector,
    colors: PackedIntVector,
    uv_info: u32,
}

// ============================================================
// Channel Info (ports AssetStudio ChannelInfo)
// ============================================================

#[derive(Clone)]
pub struct ChannelInfo {
    pub stream: u8,
    pub offset: u8,
    pub format: u8,
    pub dimension: u8,
}

// ============================================================
// Stream Info (ports AssetStudio StreamInfo)
// ============================================================

pub struct StreamInfo {
    pub channel_mask: u32,
    pub offset: u32,
    pub stride: u32,
}

// ============================================================
// Internal helper methods -- all are private methods of AssetStudioMeshParser
// ============================================================

impl AssetStudioMeshParser {
    /**
     * Derive channels from stream mask (ports AssetStudio VertexData.GetChannels).
     *
     * Used for versions below 4.0, reverse-derives the ChannelInfo array from the
     * channel_mask of each stream.
     */
    fn derive_channels_pre_v4(
        channels: &mut Vec<ChannelInfo>,
        streams: &[StreamInfo],
        _version: &[i32; 4],
    ) {
        // Ports AssetStudio VertexData.GetChannels(): uses a fixed 6-element array, array index = ShaderChannel enum value
        // Fixed 6 channels: Vertex(0), Normal(1), Color(2), TexCoord0(3), TexCoord1(4), Tangent(5)
        // Initialize 6 empty channels first (dimension=0 means inactive)
        let mut chn_buf: [Option<ChannelInfo>; 6] = Default::default();
        // [None; 6] requires Copy, use loop for initialization
        for s in 0..streams.len() {
            let mask = streams[s].channel_mask;
            let mut offset: u8 = 0;
            for chn in 0..6u32 {
                if (mask & (1 << chn)) != 0 {
                    let fmt: u8;
                    let dim: u8;
                    match chn {
                        0 | 1 => {
                            fmt = 0;
                            dim = 3;
                        } // Float x3
                        2 => {
                            fmt = 2;
                            dim = 4;
                        } // Color x4
                        3 | 4 => {
                            fmt = 0;
                            dim = 2;
                        } // Float x2
                        5 => {
                            fmt = 0;
                            dim = 4;
                        } // Float x4
                        _ => {
                            fmt = 0;
                            dim = 0;
                        }
                    }
                    chn_buf[chn as usize] = Some(ChannelInfo {
                        stream: s as u8,
                        offset,
                        format: fmt,
                        dimension: dim,
                    });
                    let vf = MeshHelper::from_vertex_channel_format(fmt as u32);
                    offset += dim * MeshHelper::get_format_size(vf) as u8;
                }
            }
        }
        // Convert the fixed array back to Vec, preserving ShaderChannel index order
        channels.clear();
        for chn in chn_buf.iter() {
            if let Some(info) = chn {
                channels.push(info.clone());
            } else {
                // Keep inactive channels as empty ChannelInfo to maintain array index alignment with ShaderChannel
                channels.push(ChannelInfo {
                    stream: 0,
                    offset: 0,
                    format: 0,
                    dimension: 0,
                });
            }
        }
    }

    /**
     * Compute streams from channels (ports AssetStudio VertexData.GetStreams).
     *
     * Used for version 5.0 and above, computes offset and stride for each stream
     * based on channel descriptions.
     */
    fn compute_streams_from_channels(
        channels: &[ChannelInfo],
        vertex_count: usize,
        version: &[i32; 4],
    ) -> Vec<StreamInfo> {
        let max_stream = channels
            .iter()
            .map(|c| c.stream as usize)
            .max()
            .unwrap_or(0);
        let stream_count = max_stream + 1;
        let mut streams: Vec<StreamInfo> = Vec::with_capacity(stream_count);
        let mut offset: u32 = 0;

        for s in 0..stream_count {
            let mut chn_mask: u32 = 0;
            let mut stride: u32 = 0;
            for (chn_idx, chn) in channels.iter().enumerate() {
                if chn.stream as usize == s && chn.dimension > 0 {
                    chn_mask |= 1 << chn_idx;
                    let vf = Self::to_vertex_format(chn.format as u32, version);
                    stride += chn.dimension as u32 * MeshHelper::get_format_size(vf);
                }
            }
            if stride == 0 {
                stride = 12;
            }
            streams.push(StreamInfo {
                channel_mask: chn_mask,
                offset,
                stride,
            });
            offset += vertex_count as u32 * stride;
            offset = (offset + 15) & !15; // 16-byte align
        }
        streams
    }

    /**
     * Format conversion (ports AssetStudio MeshHelper.ToVertexFormat).
     *
     * Selects the appropriate vertex format conversion based on Unity version.
     */
    fn to_vertex_format(format: u32, version: &[i32; 4]) -> VertexFormat {
        if version[0] < 2017 {
            MeshHelper::from_vertex_channel_format(format)
        } else if version[0] < 2019 {
            MeshHelper::from_vertex_format_2017(format)
        } else {
            // 2019+: direct mapping
            match format {
                0 => VertexFormat::Float,
                1 => VertexFormat::Float16,
                2 => VertexFormat::UNorm8,
                3 => VertexFormat::SNorm8,
                4 => VertexFormat::UNorm16,
                5 => VertexFormat::SNorm16,
                6 => VertexFormat::UInt8,
                7 => VertexFormat::SInt8,
                8 => VertexFormat::UInt16,
                9 => VertexFormat::SInt16,
                10 => VertexFormat::UInt32,
                11 => VertexFormat::SInt32,
                _ => VertexFormat::Float,
            }
        }
    }

    /**
     * Read vertex data (ports AssetStudio ReadVertexData).
     *
     * Extracts vertex positions, normals, and UV coordinates from the raw data buffer
     * based on ChannelInfo and StreamInfo arrays.
     *
     * @return (vertices, normals, uvs) -- each as Vec<f32>, every 3 values form a group
     */
    fn read_vertex_data(
        channels: &[ChannelInfo],
        streams: &[StreamInfo],
        data: &[u8],
        vertex_count: usize,
        version: &[i32; 4],
        is_big_endian: bool,
    ) -> VertexReadData {
        let mut out = VertexReadData::default();

        for (chn, ch) in channels.iter().enumerate() {
            if ch.dimension == 0 {
                continue;
            }
            if ch.stream as usize >= streams.len() {
                continue;
            }

            let stream = &streams[ch.stream as usize];
            let channel_mask = stream.channel_mask;
            if (channel_mask & (1 << chn)) == 0 {
                continue;
            }

            // Ports AssetStudio: pre-2018 Color + kChannelFormatColor -> dimension=4
            let mut dim = ch.dimension;
            if version[0] < 2018 && chn == 2 && ch.format == 2 {
                dim = 4;
            }

            let vf = Self::to_vertex_format(ch.format as u32, version);
            let fmt_size = MeshHelper::get_format_size(vf) as usize;
            let total_components = vertex_count * dim as usize;

            // Extract component bytes
            let mut component_bytes = vec![0u8; total_components * fmt_size];
            for v in 0..vertex_count {
                let vertex_offset = (stream.offset as usize)
                    .saturating_add(ch.offset as usize)
                    .saturating_add((stream.stride as usize).saturating_mul(v));
                for d in 0..dim as usize {
                    let src_offset = vertex_offset.saturating_add(fmt_size * d);
                    let dst_offset = fmt_size * (v * dim as usize + d);
                    if src_offset >= data.len() {
                        continue;
                    }
                    let end = (src_offset + fmt_size).min(data.len());
                    let len = end - src_offset;
                    if len > 0 && dst_offset + len <= component_bytes.len() {
                        component_bytes[dst_offset..dst_offset + len]
                            .copy_from_slice(&data[src_offset..end]);
                    }
                }
            }

            // BigEndian byte swap -- ports AssetStudio: reverse bytes per component
            // if (reader.Endian == EndianType.BigEndian && componentByteSize > 1)
            if is_big_endian && fmt_size > 1 {
                for i in 0..total_components {
                    let off = i * fmt_size;
                    if off + fmt_size <= component_bytes.len() {
                        component_bytes[off..off + fmt_size].reverse();
                    }
                }
            }

            // Ports AssetStudio: check IsIntFormat and route int formats to BytesToIntArray
            // float formats go through BytesToFloatArray
            let int_vals = if MeshHelper::is_int_format(vf) {
                MeshHelper::bytes_to_int_array(&component_bytes, vf)
            } else {
                Vec::new()
            };
            let float_vals = if int_vals.is_empty() {
                MeshHelper::bytes_to_float_array(&component_bytes, vf)
            } else {
                int_vals.iter().map(|&v| v as f32).collect()
            };

            // Map by channel index (ports AssetStudio's switch(chn))
            if version[0] >= 2018 {
                // 2018+ ShaderChannel order:
                //   0=Vertex, 1=Normal, 2=Tangent, 3=Color,
                //   4=TexCoord0, 5=TexCoord1, ...
                match chn {
                    0 => out.vertices = float_vals,
                    1 => out.normals = float_vals,
                    2 => out.tangents = float_vals,
                    3 => out.colors = float_vals,
                    4 if out.uvs.is_empty() => out.uvs = float_vals,
                    12 => {
                        out.bone_weights = Self::expand_vertex_groups_f32(
                            &float_vals,
                            vertex_count,
                            dim as usize,
                            4,
                        )
                    }
                    13 => {
                        let source = if int_vals.is_empty() {
                            float_vals
                                .iter()
                                .map(|value| *value as i32)
                                .collect::<Vec<_>>()
                        } else {
                            int_vals
                        };
                        out.bone_indices =
                            Self::expand_vertex_groups_u16(&source, vertex_count, dim as usize, 4);
                    }
                    _ => {}
                }
            } else {
                // Pre-2018 ShaderChannel order:
                //   0=Vertex, 1=Normal, 2=Color, 3=TexCoord0, 4=TexCoord1,
                //   5=TexCoord2(>=5) or Tangent(<5), 6=TexCoord3(>=5), 7=Tangent(>=5)
                match chn {
                    0 => out.vertices = float_vals,
                    1 => out.normals = float_vals,
                    2 => out.colors = float_vals,
                    3 if out.uvs.is_empty() => out.uvs = float_vals,
                    5 if version[0] < 5 => out.tangents = float_vals, // pre-5: ch5=Tangent
                    7 if version[0] >= 5 => {
                        // 5+: ch7=Tangent
                        if out.tangents.is_empty() {
                            out.tangents = float_vals;
                        }
                    }
                    _ => {}
                }
            }
        }

        out
    }

    fn expand_vertex_groups_f32(
        values: &[f32],
        vertex_count: usize,
        dim: usize,
        target_dim: usize,
    ) -> Vec<f32> {
        if values.is_empty() || vertex_count == 0 || dim == 0 {
            return Vec::new();
        }
        let mut out = vec![0.0f32; vertex_count * target_dim];
        for vertex in 0..vertex_count {
            for component in 0..dim.min(target_dim) {
                let src = vertex * dim + component;
                let dst = vertex * target_dim + component;
                if src < values.len() {
                    out[dst] = values[src];
                }
            }
        }
        out
    }

    fn expand_vertex_groups_u16(
        values: &[i32],
        vertex_count: usize,
        dim: usize,
        target_dim: usize,
    ) -> Vec<u16> {
        if values.is_empty() || vertex_count == 0 || dim == 0 {
            return Vec::new();
        }
        let mut out = vec![0u16; vertex_count * target_dim];
        for vertex in 0..vertex_count {
            for component in 0..dim.min(target_dim) {
                let src = vertex * dim + component;
                let dst = vertex * target_dim + component;
                if src < values.len() {
                    out[dst] = values[src].clamp(0, u16::MAX as i32) as u16;
                }
            }
        }
        out
    }

    /**
     * Extract triangle indices (ports AssetStudio GetTriangles).
     *
     * Extracts triangle indices from the index buffer based on sub-mesh info,
     * supporting three topologies:
     * - Triangles(0): every 3 indices form a triangle
     * - TriangleStrip(1): expand triangle strip into individual triangles
     * - Quads(2): split each quad into two triangles
     */
    fn extract_triangles_with_ranges(
        index_buffer: &[u32],
        sub_meshes: &[SubMeshInfo],
        use_16bit: bool,
    ) -> (Vec<u32>, Vec<ParsedSubMesh>) {
        // Ports AssetStudio GetTriangles(): supports Triangles(0), TriangleStrip(1), Quads(2) topologies
        let mut indices = Vec::new();
        let mut parsed_sub_meshes = Vec::new();
        for sub in sub_meshes {
            let out_start = indices.len();
            // Starting offset in the index buffer: firstByte is a byte offset, convert to element index
            let first_index = if use_16bit {
                (sub.first_byte / 2) as usize // 16-bit: 2 bytes per index
            } else {
                (sub.first_byte / 4) as usize // 32-bit: 4 bytes per index (i.e. firstByte/2/2)
            };
            let index_count = sub.index_count as usize;
            let topology = sub.topology;
            let with_base_vertex = |index: u32| -> u32 { index.saturating_add(sub.base_vertex) };

            if topology == 0 {
                // GfxPrimitiveType::Triangles -- every 3 indices form a triangle
                for i in (0..index_count).step_by(3) {
                    if first_index + i + 2 < index_buffer.len() {
                        indices.push(with_base_vertex(index_buffer[first_index + i]));
                        indices.push(with_base_vertex(index_buffer[first_index + i + 1]));
                        indices.push(with_base_vertex(index_buffer[first_index + i + 2]));
                    }
                }
            } else if topology == 1 {
                // GfxPrimitiveType::TriangleStrip -- de-stripify: expand strip into individual triangles
                // Each consecutive group of 3 vertices forms a triangle, each new vertex adds one triangle
                for i in 0..index_count.saturating_sub(2) {
                    if first_index + i + 2 >= index_buffer.len() {
                        break;
                    }
                    let a = with_base_vertex(index_buffer[first_index + i]);
                    let b = with_base_vertex(index_buffer[first_index + i + 1]);
                    let c = with_base_vertex(index_buffer[first_index + i + 2]);

                    // Skip degenerate triangles (any two vertices the same)
                    if a == b || a == c || b == c {
                        continue;
                    }

                    // Triangle strip winding order flips: odd-index triangles need a,b swap to keep orientation consistent
                    if (i & 1) == 1 {
                        indices.push(b);
                        indices.push(a);
                    } else {
                        indices.push(a);
                        indices.push(b);
                    }
                    indices.push(c);
                }
            } else if topology == 2 {
                // GfxPrimitiveType::Quads -- every 4 indices form a quad, split into 2 triangles
                for q in (0..index_count).step_by(4) {
                    if first_index + q + 3 >= index_buffer.len() {
                        break;
                    }
                    let i0 = with_base_vertex(index_buffer[first_index + q]);
                    let i1 = with_base_vertex(index_buffer[first_index + q + 1]);
                    let i2 = with_base_vertex(index_buffer[first_index + q + 2]);
                    let i3 = with_base_vertex(index_buffer[first_index + q + 3]);
                    // Quad (i0,i1,i2,i3) -> triangles (i0,i1,i2) + (i0,i2,i3)
                    indices.push(i0);
                    indices.push(i1);
                    indices.push(i2);
                    indices.push(i0);
                    indices.push(i2);
                    indices.push(i3);
                }
            }
            // topology 3 (Lines), 4 (LineStrip), 5 (Points) not handled (non-triangle topology)
            let out_count = indices.len() - out_start;
            if out_count > 0 {
                parsed_sub_meshes.push(ParsedSubMesh {
                    index_start: out_start,
                    index_count: out_count,
                    topology: sub.topology,
                });
            }
        }
        (indices, parsed_sub_meshes)
    }

    fn build_blend_shapes(
        vertices: &[BlendShapeVertex],
        frames: &[BlendShapeFrameInfo],
        channels: &[BlendShapeChannelInfo],
        weights: &[f32],
        vertex_count: usize,
    ) -> Vec<ParsedBlendShape> {
        if vertices.is_empty() || frames.is_empty() || channels.is_empty() || vertex_count == 0 {
            return Vec::new();
        }

        let mut shapes = Vec::new();
        for channel in channels {
            let frame_start = channel.frame_index.max(0) as usize;
            let frame_count = channel.frame_count.max(0) as usize;
            for frame_offset in 0..frame_count {
                let frame_index = frame_start + frame_offset;
                let Some(frame) = frames.get(frame_index) else {
                    continue;
                };
                let mut delta_vertices = vec![0.0f32; vertex_count * 3];
                let mut delta_normals = vec![0.0f32; vertex_count * 3];
                let mut delta_tangents = vec![0.0f32; vertex_count * 3];
                let start = frame.first_vertex as usize;
                let count = frame.vertex_count as usize;
                for item in vertices.iter().skip(start).take(count) {
                    let vertex_index = item.index as usize;
                    if vertex_index >= vertex_count {
                        continue;
                    }
                    let dst = vertex_index * 3;
                    delta_vertices[dst..dst + 3].copy_from_slice(&item.vertex);
                    delta_normals[dst..dst + 3].copy_from_slice(&item.normal);
                    delta_tangents[dst..dst + 3].copy_from_slice(&item.tangent);
                }
                let name = if frame_count > 1 {
                    format!("{}_{}", channel.name, frame_offset)
                } else {
                    channel.name.clone()
                };
                shapes.push(ParsedBlendShape {
                    name,
                    weight: weights.get(frame_index).copied().unwrap_or(0.0),
                    delta_vertices,
                    delta_normals,
                    delta_tangents,
                });
            }
        }
        shapes
    }

    /**
     * Read CompressedMesh and return structured data (ports AssetStudio CompressedMesh(reader)).
     *
     * Used for version 2.6.0+. Reads PackedFloatVector/PackedIntVector into memory,
     * for decompression of vertex data in the DecompressCompressedMesh stage.
     */
    fn read_compressed_mesh_with_logs(
        r: &mut UnityReader,
        version: &[i32; 4],
        logs: &mut Vec<String>,
    ) -> CompressedMeshData {
        fn read_packed_float(
            r: &mut UnityReader,
            name: &str,
            logs: &mut Vec<String>,
        ) -> PackedFloatVector {
            let num = r.read_u32();
            let range = r.read_f32();
            let start = r.read_f32();
            let bit_size = r.read_u32();
            let data = r.read_u8_array();
            logs.push(format!(
                "  {}: num={}, range={:.4}, start={:.4}, bitSize={}, data={}B",
                name,
                num,
                range,
                start,
                bit_size,
                data.len()
            ));
            PackedFloatVector {
                num_items: num,
                range,
                start,
                bit_size,
                data,
            }
        }

        fn read_packed_int(
            r: &mut UnityReader,
            name: &str,
            logs: &mut Vec<String>,
        ) -> PackedIntVector {
            let num = r.read_u32();
            let bit_size = r.read_u32();
            let data = r.read_u8_array();
            logs.push(format!(
                "  {}: num={}, bitSize={}, data={}B",
                name,
                num,
                bit_size,
                data.len()
            ));
            PackedIntVector {
                num_items: num,
                bit_size,
                data,
            }
        }

        let mut cm = CompressedMeshData {
            vertices: read_packed_float(r, "m_Vertices", logs),
            uv: read_packed_float(r, "m_UV", logs),
            bind_poses: if version[0] < 5 {
                read_packed_float(r, "m_BindPoses", logs)
            } else {
                PackedFloatVector::default()
            },
            normals: read_packed_float(r, "m_Normals", logs),
            tangents: read_packed_float(r, "m_Tangents", logs),
            weights: read_packed_int(r, "m_Weights", logs),
            normal_signs: read_packed_int(r, "m_NormalSigns", logs),
            tangent_signs: read_packed_int(r, "m_TangentSigns", logs),
            float_colors: if version[0] >= 5 {
                read_packed_float(r, "m_FloatColors", logs)
            } else {
                PackedFloatVector::default()
            },
            bone_indices: read_packed_int(r, "m_BoneIndices", logs),
            triangles: read_packed_int(r, "m_Triangles", logs),
            colors: PackedIntVector::default(),
            uv_info: 0,
        };
        if version[0] > 3 || (version[0] == 3 && version[1] >= 5) {
            if version[0] < 5 {
                cm.colors = read_packed_int(r, "m_Colors", logs);
            } else {
                cm.uv_info = r.read_u32();
                logs.push(format!("  m_UVInfo = 0x{:X}", cm.uv_info));
            }
        }
        cm
    }
}

// ============================================================
// process_stream_vertex_data -- ports AssetStudio ProcessData streaming merge logic
// ============================================================

impl AssetStudioMeshParser {
    /**
     * After loading the .resS external resource, call ReadVertexData to complete vertex parsing.
     *
     * AssetStudio flow:
     *   Mesh(ObjectReader)   <- reads all fields, including StreamingInfo
     *   ProcessData():
     *     1. If m_StreamData.size > 0:
     *        -> m_VertexData.m_Data = resourceReader.GetData()
     *     2. If m_VertexData.m_Data is empty and m_CompressedMesh has data:
     *        -> DecompressCompressedMesh() overrides
     *     3. ReadVertexData() -> parse channel mapping -> extract vertices/normals/UV
     *     4. GetTriangles() -> extract triangle indices (already done in constructor)
     *
     * Here we implement steps 1+3.
     *
     * Note: The caller (mesh_service::load_stream_data) has already sliced the .resS raw data
     * based on StreamingInfo offset/size, so the passed-in res_data is the equivalent of
     * ResourceReader.GetData() and can be used directly as m_DataSize.
     *
     * @param meta     Metadata portion of AssetStudioMesh (valid when has_streaming=true)
     * @param res_data Vertex buffer bytes sliced by stream_offset/size (= m_DataSize equivalent)
     * @param version  Version string
     * @return         Vertex buffer data, including skin channels when present.
     */
    pub fn process_stream_vertex_data(
        meta: &AssetStudioMesh,
        res_data: &[u8],
        version_str: &str,
    ) -> VertexReadData {
        if !meta.has_streaming {
            if !meta.vertices.is_empty() {
                // No streaming data but has inline vertices, return existing vertices directly
                return VertexReadData {
                    vertices: meta.vertices.clone(),
                    normals: meta.normals.clone(),
                    uvs: meta.uvs.clone(),
                    tangents: meta.tangents.clone(),
                    colors: meta.colors.clone(),
                    bone_weights: meta.bone_weights.clone(),
                    bone_indices: meta.bone_indices.clone(),
                };
            }
            // has_streaming=false but vertices is empty --
            // the AssetStudio parser did not detect the streaming flag, but the actual data is in .resS,
            // the caller (mesh_service) passed .resS data, so proceed to try ReadVertexData.
        }

        // === Step 1: Streaming data replaces m_DataSize ===
        // AssetStudio's ProcessData():
        //   resourceReader = new ResourceReader(m_StreamData.path, assetsFile, m_StreamData.offset, m_StreamData.size);
        //   m_VertexData.m_DataSize = resourceReader.GetData();
        //
        // ResourceReader.GetData() has already sliced by offset/size, returning data used directly as m_DataSize.
        // The caller (mesh_service::load_stream_data) has simulated this: reading bytes in the
        // [stream_offset..stream_offset+stream_size] range from the .resS file and passing them in.
        // Therefore res_data here is the already-sliced m_DataSize equivalent, used directly without additional slicing.
        //
        // Note: m_Streams[].offset is a buffer-internal offset (computed by compute_streams_from_channels),
        // completely different from the file-level stream_offset. ReadVertexData internally uses
        // the buffer-internal offset, not the file offset.
        let combined = res_data;

        if combined.is_empty() || meta.channels.is_empty() || meta.vertex_count == 0 {
            return VertexReadData::default();
        }

        // === Step 2: Version string parsing ===
        let digits: Vec<i32> = version_str
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '.')
            .collect::<String>()
            .split('.')
            .filter_map(|s| s.parse::<i32>().ok())
            .collect();
        let mut version = [0i32; 4];
        for (i, d) in digits.iter().enumerate().take(4) {
            version[i] = *d;
        }

        // === Step 3: ReadVertexData ===
        Self::read_vertex_data(
            &meta.channels,
            &meta.streams,
            &combined,
            meta.vertex_count,
            &version,
            false,
        )
    }
}
