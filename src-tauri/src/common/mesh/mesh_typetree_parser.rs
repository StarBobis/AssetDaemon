/*
 * mesh_typetree_parser.rs -- Mesh TypeTree fallback parsing utility
 *
 * Extracted from mesh_service.rs, handles vertex/index parsing for the Mesh TypeTree fallback path.
 * Supports VertexData, CompressedMesh, m_Vertices and other storage formats.
 *
 * Follows Soul.md convention: struct + impl organization, no free functions.
 */

use crate::unity::type_tree::unity_value::UnityValue;
use crate::utils::format_utils::FormatUtils;

/**
 * TypeTree Mesh parser.
 *
 * When AssetStudio-style byte stream parsing fails, fall back to TypeTree-based Mesh parsing.
 * Supports VertexData, CompressedMesh, m_Vertices and other Unity Mesh storage formats.
 */
pub struct TypeTreeMeshParser;

impl TypeTreeMeshParser {
    // ============================================================
    // VertexData Parsing
    // ============================================================

    pub fn parse_vertex_data(val: &UnityValue) -> Option<Vec<f32>> {
        Self::parse_vertex_data_inner(val, None)
    }

    /// Parse VertexData using external byte data (load from .resS when m_DataSize is empty)
    pub fn parse_vertex_data_from_stream(val: &UnityValue, stream_data: &[u8]) -> Option<Vec<f32>> {
        Self::parse_vertex_data_inner(val, Some(stream_data))
    }

    fn parse_vertex_data_inner(val: &UnityValue, external_data: Option<&[u8]>) -> Option<Vec<f32>> {
        let vd = match val {
            UnityValue::Object(o) => o,
            _ => return None,
        };
        let vc = vd
            .get("m_VertexCount")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as usize;
        if vc < 3 || vc > 100_000 {
            return None;
        }

        // Get vertex buffer byte data (prefer externally injected .resS data)
        let data: Vec<u8> = if let Some(ext) = external_data {
            ext.to_vec()
        } else {
            let m_data = vd.get("m_DataSize")?;
            if let Some(s) = UnityValue::to_u8_slice(m_data) {
                s.to_vec()
            } else {
                let v = UnityValue::to_u8_vec(m_data);
                if v.is_empty() {
                    return None;
                } else {
                    v
                }
            }
        };
        if data.is_empty() {
            return None;
        }
        let channels = match vd.get("m_Channels") {
            Some(UnityValue::Array(a)) => a,
            _ => return None,
        };
        if channels.is_empty() {
            return None;
        }
        let streams: Vec<(usize, usize)> = vd
            .get("m_Streams")
            .and_then(|v| match v {
                UnityValue::Array(a) => Some(a),
                _ => None,
            })
            .map(|a| {
                a.iter()
                    .map(|s| {
                        let o = match s {
                            UnityValue::Object(o) => o,
                            _ => return (0usize, 0usize),
                        };
                        (
                            o.get("offset").and_then(|v| v.as_i64()).unwrap_or(0) as usize,
                            o.get("stride").and_then(|v| v.as_i64()).unwrap_or(0) as usize,
                        )
                    })
                    .collect()
            })
            .unwrap_or_else(|| Self::compute_streams_from_channels(channels, vc));
        if streams.is_empty() {
            return None;
        }
        let (si, ch_off, fmt, _pd) = (|| {
            for (i, ch) in channels.iter().enumerate() {
                let o = match ch {
                    UnityValue::Object(o) => o,
                    _ => continue,
                };
                let s = o.get("stream").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                let off = o.get("offset").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                let f = o.get("format").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                let d = o.get("dimension").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                if d >= 3 && i == 0 {
                    return Some((s, off, f, 3));
                }
            }
            None
        })()?;
        if si >= streams.len() {
            return None;
        }
        let (so, stride) = streams[si];
        if stride == 0 {
            return None;
        }
        let comp_size = match fmt {
            0 => 4,
            1 => 2,
            2 | 3 | 6 | 7 => 1,
            4 | 5 | 8 | 9 => 2,
            10 | 11 => 4,
            _ => 4,
        };
        let mut verts = Vec::with_capacity(vc * 3);
        for v in 0..vc {
            let base = so + ch_off + stride * v;
            for d in 0..3 {
                let bo = base + comp_size * d;
                if bo + comp_size > data.len() {
                    return None;
                }
                let val = match fmt {
                    0 | 10 | 11 => f32::from_le_bytes(data[bo..bo + 4].try_into().ok()?),
                    1 => FormatUtils::half_to_f32(u16::from_le_bytes(
                        data[bo..bo + 2].try_into().ok()?,
                    )),
                    2 => data[bo] as f32 / 255.0,
                    3 => (data[bo] as i8 as f32 / 127.0).max(-1.0),
                    4 => u16::from_le_bytes(data[bo..bo + 2].try_into().ok()?) as f32 / 65535.0,
                    5 => (i16::from_le_bytes(data[bo..bo + 2].try_into().ok()?) as f32 / 32767.0)
                        .max(-1.0),
                    _ => f32::from_le_bytes(data[bo..bo + 4].try_into().ok()?),
                };
                if !val.is_finite() {
                    return None;
                }
                verts.push(val);
            }
        }
        if verts.len() >= 9 {
            Some(verts)
        } else {
            None
        }
    }

    fn compute_streams_from_channels(channels: &[UnityValue], vc: usize) -> Vec<(usize, usize)> {
        let max_s = channels
            .iter()
            .filter_map(|c| match c {
                UnityValue::Object(o) => o.get("stream").and_then(|v| v.as_i64()),
                _ => None,
            })
            .max()
            .unwrap_or(0) as usize;
        let mut res: Vec<(usize, usize)> = Vec::new();
        let mut cur_off = 0usize;
        for s in 0..=max_s {
            let mut stride = 0usize;
            for ch in channels {
                let o = match ch {
                    UnityValue::Object(o) => o,
                    _ => continue,
                };
                if o.get("stream").and_then(|v| v.as_i64()).unwrap_or(0) as usize != s {
                    continue;
                }
                let d = o.get("dimension").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                if d == 0 {
                    continue;
                }
                let f = o.get("format").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                let fs = match f {
                    0 => 4,
                    1 => 2,
                    2 | 3 | 6 | 7 => 1,
                    4 | 5 | 8 | 9 => 2,
                    10 | 11 => 4,
                    _ => 4,
                };
                stride += d * fs;
            }
            if stride == 0 {
                stride = 12;
            }
            res.push((cur_off, stride));
            cur_off += vc * stride;
            cur_off = (cur_off + 15) & !15;
        }
        res
    }

    // ============================================================
    // CompressedMesh Parsing
    // ============================================================

    pub fn parse_compressed_mesh_vertices(val: &UnityValue) -> Option<Vec<f32>> {
        let cm = match val {
            UnityValue::Object(o) => o,
            _ => return None,
        };
        Self::unpack_packed_floats(cm.get("m_Vertices")?, 3)
    }

    pub fn parse_compressed_mesh_indices(val: &UnityValue) -> Option<Vec<u32>> {
        let cm = match val {
            UnityValue::Object(o) => o,
            _ => return None,
        };
        Self::unpack_packed_ints(cm.get("m_Triangles")?)
    }

    fn unpack_packed_floats(packed: &UnityValue, elem_size: usize) -> Option<Vec<f32>> {
        let obj = match packed {
            UnityValue::Object(o) => o,
            _ => return None,
        };
        let n = obj.get("m_NumItems").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
        if n == 0 || n % elem_size != 0 {
            return None;
        }
        let range = obj
            .get("m_Range")
            .and_then(|v| match v {
                UnityValue::Float(f) => Some(*f as f32),
                _ => None,
            })
            .unwrap_or(1.0);
        let start = obj
            .get("m_Start")
            .and_then(|v| match v {
                UnityValue::Float(f) => Some(*f as f32),
                _ => None,
            })
            .unwrap_or(0.0);
        let bit = obj.get("m_BitSize").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
        let data = UnityValue::to_u8_slice(obj.get("m_Data")?)
            .map(|s| s.to_owned())
            .or_else(|| {
                let v = UnityValue::to_u8_vec(obj.get("m_Data")?);
                if v.is_empty() {
                    None
                } else {
                    Some(v)
                }
            })?;
        if bit == 0 || bit > 32 {
            return None;
        }
        let ints = Self::unpack_bits(&data, n, bit)?;
        let max = ((1u64 << bit) - 1) as f32;
        let scale = if max > 0.0 { range / max } else { 0.0 };
        Some(ints.iter().map(|i| *i as f32 * scale + start).collect())
    }

    fn unpack_packed_ints(packed: &UnityValue) -> Option<Vec<u32>> {
        let obj = match packed {
            UnityValue::Object(o) => o,
            _ => return None,
        };
        let n = obj.get("m_NumItems").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
        if n == 0 {
            return None;
        }
        let bit = obj.get("m_BitSize").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
        let data = UnityValue::to_u8_slice(obj.get("m_Data")?)
            .map(|s| s.to_owned())
            .or_else(|| {
                let v = UnityValue::to_u8_vec(obj.get("m_Data")?);
                if v.is_empty() {
                    None
                } else {
                    Some(v)
                }
            })?;
        if bit == 0 || bit > 32 {
            return None;
        }
        Self::unpack_bits(&data, n, bit)
    }

    fn unpack_bits(data: &[u8], count: usize, bit_size: usize) -> Option<Vec<u32>> {
        let total = count * bit_size;
        if data.len() < (total + 7) / 8 {
            return None;
        }
        let mut res = Vec::with_capacity(count);
        let mut bp = 0usize;
        for _ in 0..count {
            let byte_pos = bp / 8;
            let bit_off = bp % 8;
            let mut val: u64 = 0;
            let mut read = 0usize;
            while read < bit_size {
                let idx = byte_pos + (bit_off + read) / 8;
                if idx >= data.len() {
                    return None;
                }
                let byte = data[idx] as u64;
                let shift = (bit_off + read) % 8;
                val |= byte << shift;
                read += 8 - shift.min(8);
            }
            val &= (1u64 << bit_size) - 1;
            res.push(val as u32);
            bp += bit_size;
        }
        Some(res)
    }

    // ============================================================
    // UnityValue Deep Extraction
    // ============================================================

    pub fn deep_extract_floats(val: &UnityValue) -> Vec<f32> {
        match val {
            UnityValue::Array(a) => {
                let f: Vec<_> = a
                    .iter()
                    .filter_map(|v| match v {
                        UnityValue::Float(f) => Some(*f as f32),
                        UnityValue::Integer(i) => Some(*i as f32),
                        _ => None,
                    })
                    .collect();
                if f.len() >= 6 {
                    return f;
                }
                for item in a {
                    let r = Self::deep_extract_floats(item);
                    if !r.is_empty() {
                        return r;
                    }
                }
                vec![]
            }
            UnityValue::Object(o) => {
                for k in &["m_Data", "m_DataSize", "data", "m_VertexData", "m_Channels"] {
                    if let Some(sub) = o.get(*k) {
                        let r = Self::deep_extract_floats(sub);
                        if !r.is_empty() {
                            return r;
                        }
                    }
                }
                for (_, v) in o.iter() {
                    let r = Self::deep_extract_floats(v);
                    if !r.is_empty() {
                        return r;
                    }
                }
                vec![]
            }
            UnityValue::Bytes(b) => {
                if b.len() >= 12 && b.len() % 4 == 0 {
                    let f: Vec<_> = b
                        .chunks(4)
                        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                        .filter(|f| f.is_finite())
                        .collect();
                    if f.len() >= 3 {
                        return f;
                    }
                }
                vec![]
            }
            _ => vec![],
        }
    }
}
