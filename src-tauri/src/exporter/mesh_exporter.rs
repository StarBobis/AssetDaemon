use crate::utils::format_utils::FormatUtils;
use std::{fs, path::Path};

const GLB_MAGIC: u32 = 0x46546C67;
const GLB_VERSION: u32 = 2;
const CHUNK_TYPE_JSON: u32 = 0x4E4F534A;
const CHUNK_TYPE_BIN: u32 = 0x004E4942;

pub struct MeshAttributes<'a> {
    pub vertices: &'a [f32],
    pub indices: &'a [u32],
    pub normals: Option<&'a [f32]>,
    pub uvs: Option<&'a [f32]>,
    pub tangents: Option<&'a [f32]>,
    pub colors: Option<&'a [f32]>,
    pub bone_weights: Option<&'a [f32]>,
    pub bone_indices: Option<&'a [u16]>,
    pub bind_poses: Option<&'a [f32]>,
    pub bone_name_hashes: Option<&'a [u32]>,
    pub root_bone_name_hash: Option<u32>,
    pub sub_meshes: Option<&'a [MeshSubMesh]>,
    pub blend_shapes: Option<&'a [MeshBlendShape]>,
}

#[derive(Debug, Clone, Copy)]
pub struct MeshSubMesh {
    pub index_start: usize,
    pub index_count: usize,
    pub topology: i32,
}

#[derive(Debug, Clone)]
pub struct MeshBlendShape {
    pub name: String,
    pub delta_vertices: Vec<f32>,
    pub delta_normals: Vec<f32>,
    pub delta_tangents: Vec<f32>,
}

pub struct MeshExporter;

// ---- helpers ----
#[derive(Default, Clone)]
struct J(serde_json::Value);

fn minmax(d: &[f32], stride: usize, is_min: bool) -> serde_json::Value {
    let mut v = vec![if is_min { f32::MAX } else { f32::MIN }; stride];
    for (i, &val) in d.iter().enumerate() {
        let c = i % stride;
        if is_min && val < v[c] {
            v[c] = val;
        }
        if !is_min && val > v[c] {
            v[c] = val;
        }
    }
    serde_json::json!(v)
}

impl MeshExporter {
    pub fn mesh_data_to_glb(raw: &[u8], output_path: &Path) -> Result<u64, String> {
        use crate::unity::mesh_heuristic_parser::UnityMeshParser;
        let p = UnityMeshParser::parse_unity_mesh(raw)?;
        if p.vertices.is_empty() {
            return Err("No vertices".to_string());
        }
        let a = MeshAttributes {
            vertices: &p.vertices,
            indices: &p.indices,
            normals: if p.normals.len() >= p.vertices.len() {
                Some(&p.normals)
            } else {
                None
            },
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: None,
            bone_indices: None,
            bind_poses: None,
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        };
        let data = Self::build(&a)?;
        fs::write(output_path, &data).map_err(|e| format!("Write: {}", e))?;
        Ok(data.len() as u64)
    }

    fn build(attrs: &MeshAttributes) -> Result<Vec<u8>, String> {
        let v = attrs.vertices;
        let idx = attrs.indices;
        let vc = v.len() / 3;
        if v.len() < 3 || v.len() % 3 != 0 {
            return Err(format!("Vertices: {} floats", v.len()));
        }

        let has_n = attrs.normals.map_or(false, |n| n.len() >= v.len());
        let has_u = attrs.uvs.map_or(false, |u| u.len() >= 2 * vc);
        let has_c = attrs.colors.map_or(false, |c| c.len() >= 4 * vc);
        let has_t = attrs.tangents.map_or(false, |t| t.len() >= 4 * vc);
        let _has_bone_name_hashes = attrs
            .bone_name_hashes
            .map_or(false, |bone_name_hashes| !bone_name_hashes.is_empty());

        let cnv = |d: &[f32], s: usize| -> Vec<f32> {
            d.iter()
                .enumerate()
                .map(|(i, val)| if i % s == 0 { -val } else { *val })
                .collect()
        };
        let vtx = cnv(v, 3);
        let nrm = attrs.normals.map(|n| cnv(n, 3));
        let tng = attrs.tangents.map(|t| cnv(t, 4));
        let col = attrs.colors.map(|c| c.to_vec());
        let uv = attrs.uvs.map(|u| {
            u.iter()
                .enumerate()
                .map(|(i, val)| if i % 2 == 1 { 1.0 - val } else { *val })
                .collect::<Vec<_>>()
        });

        let ci: Vec<u32> = if idx.len() >= 3 {
            idx.chunks(3).flat_map(|t| [t[0], t[2], t[1]]).collect()
        } else {
            idx.to_vec()
        };

        let mut buf: Vec<u8> = Vec::new();
        let mut acc: Vec<J> = vec![];
        let mut bvs: Vec<J> = vec![];
        let mut off: usize = 0;
        let mut push_bv = |buf_id: u32, offset: usize, len: usize, target: u32| -> usize {
            let i = bvs.len();
            bvs.push(J(serde_json::json!({"buffer":buf_id,"byteOffset":offset,"byteLength":len,"target":target})));
            i
        };
        let mut push_acc = |v: serde_json::Value| -> usize {
            let i = acc.len();
            acc.push(J(v));
            i
        };

        // Index byte length (index data itself is appended to the buffer at the end)
        let ib: Vec<u8> = ci.iter().flat_map(|i| i.to_le_bytes()).collect();
        let il = ib.len();
        let idx_bi = push_bv(0, off, il, 34963);
        push_acc(
            serde_json::json!({"bufferView":idx_bi,"componentType":5125,"count":ci.len(),"type":"SCALAR"}),
        );

        // Position (prepend to buffer)
        let pb: Vec<u8> = vtx.iter().flat_map(|f| f.to_le_bytes()).collect();
        let pl = pb.len();
        buf = [&pb[..], &buf[..]].concat();
        off = pl;
        let p_bi = push_bv(0, 0, pl, 34962);
        let p_ai = push_acc(
            serde_json::json!({"bufferView":p_bi,"componentType":5126,"count":vc,"type":"VEC3","min":minmax(&vtx,3,true),"max":minmax(&vtx,3,false)}),
        );

        // Normal
        let mut n_ai = 0usize;
        if let Some(ref n) = nrm {
            let nb: Vec<u8> = n.iter().flat_map(|f| f.to_le_bytes()).collect();
            let nl = nb.len();
            buf.extend_from_slice(&nb);
            let bi = push_bv(0, off, nl, 34962);
            n_ai = push_acc(
                serde_json::json!({"bufferView":bi,"componentType":5126,"count":vc,"type":"VEC3","min":minmax(n,3,true),"max":minmax(n,3,false)}),
            );
            off += nl;
        }

        // UV
        let mut u_ai = 0usize;
        if let Some(ref u) = uv {
            let ul = u.len() * 4;
            let up = FormatUtils::align4(ul);
            let mut ub: Vec<u8> = u.iter().flat_map(|f| f.to_le_bytes()).collect();
            ub.resize(up, 0);
            buf.extend_from_slice(&ub);
            let bi = push_bv(0, off, ul, 34962);
            u_ai = push_acc(
                serde_json::json!({"bufferView":bi,"componentType":5126,"count":vc,"type":"VEC2","min":minmax(u,2,true),"max":minmax(u,2,false)}),
            );
            off += up;
        }

        // Color
        let mut c_ai = 0usize;
        if let Some(ref c) = col {
            let cl = c.len() * 4;
            let cp = FormatUtils::align4(cl);
            let mut cb: Vec<u8> = c.iter().flat_map(|f| f.to_le_bytes()).collect();
            cb.resize(cp, 0);
            buf.extend_from_slice(&cb);
            let bi = push_bv(0, off, cl, 34962);
            c_ai = push_acc(
                serde_json::json!({"bufferView":bi,"componentType":5126,"count":vc,"type":"VEC4","min":minmax(c,4,true),"max":minmax(c,4,false)}),
            );
            off += cp;
        }

        // Tangent
        let mut t_ai = 0usize;
        if let Some(ref t) = tng {
            let tl = t.len() * 4;
            let tp = FormatUtils::align4(tl);
            let mut tb: Vec<u8> = t.iter().flat_map(|f| f.to_le_bytes()).collect();
            tb.resize(tp, 0);
            buf.extend_from_slice(&tb);
            let bi = push_bv(0, off, tl, 34962);
            t_ai = push_acc(
                serde_json::json!({"bufferView":bi,"componentType":5126,"count":vc,"type":"VEC4","min":minmax(t,4,true),"max":minmax(t,4,false)}),
            );
            off += tp;
        }

        // Fix index bufferView offset
        bvs[idx_bi].0 =
            serde_json::json!({"buffer":0,"byteOffset":off,"byteLength":il,"target":34963});
        buf.extend_from_slice(&ib);

        // Attributes
        let mut attrs_obj = serde_json::Map::new();
        attrs_obj.insert("POSITION".into(), serde_json::json!(p_ai));
        if has_n {
            attrs_obj.insert("NORMAL".into(), serde_json::json!(n_ai));
        }
        if has_u {
            attrs_obj.insert("TEXCOORD_0".into(), serde_json::json!(u_ai));
        }
        if has_c {
            attrs_obj.insert("COLOR_0".into(), serde_json::json!(c_ai));
        }
        if has_t {
            attrs_obj.insert("TANGENT".into(), serde_json::json!(t_ai));
        }

        let pad = (4 - buf.len() % 4) % 4;
        buf.extend(std::iter::repeat(0u8).take(pad));
        // BIN chunk unpadded data length (without padding, conforms to glTF 2.0 specification)
        let bin_data_len = buf.len() - pad;

        let json_val = serde_json::json!({
            "asset":{"version":"2.0","generator":"AssetDaemon"},
            "scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
            "meshes":[{"primitives":[{"attributes":attrs_obj,"indices":0,"mode":4}]}],
            "accessors":acc.iter().map(|a| a.0.clone()).collect::<Vec<_>>(),
            "bufferViews":bvs.iter().map(|a| a.0.clone()).collect::<Vec<_>>(),
            // buffer.byteLength should also be the unpadded data length
            "buffers":[{"byteLength":bin_data_len}]
        });

        let js = serde_json::to_string(&json_val).map_err(|e| format!("JSON: {}", e))?;
        let jp = FormatUtils::align4_json(js.as_bytes());
        // total = header(12) + json_chunk_header(8) + json_data + bin_chunk_header(8) + bin_data_with_padding
        let total = 12 + 8 + jp.len() + 8 + buf.len();
        let mut glb = Vec::with_capacity(total);
        glb.extend_from_slice(&GLB_MAGIC.to_le_bytes());
        glb.extend_from_slice(&GLB_VERSION.to_le_bytes());
        glb.extend_from_slice(&(total as u32).to_le_bytes());
        glb.extend_from_slice(&(jp.len() as u32).to_le_bytes());
        glb.extend_from_slice(&CHUNK_TYPE_JSON.to_le_bytes());
        glb.extend_from_slice(&jp);
        // BIN chunkLength uses the unpadded data length (glTF spec: chunkLength does not include padding)
        glb.extend_from_slice(&(bin_data_len as u32).to_le_bytes());
        glb.extend_from_slice(&CHUNK_TYPE_BIN.to_le_bytes());
        glb.extend_from_slice(&buf);
        Ok(glb)
    }

    #[allow(dead_code)]
    pub fn build_obj_from_geometry(v: &[f32], idx: &[u32], name: &str) -> Result<String, String> {
        Self::build_obj(v, idx, None, None, name)
    }

    pub fn build_obj_with_attributes(
        v: &[f32],
        idx: &[u32],
        nrm: Option<&[f32]>,
        uv: Option<&[f32]>,
        name: &str,
    ) -> Result<String, String> {
        Self::build_obj(v, idx, nrm, uv, name)
    }

    fn build_obj(
        v: &[f32],
        idx: &[u32],
        nrm: Option<&[f32]>,
        uv: Option<&[f32]>,
        name: &str,
    ) -> Result<String, String> {
        if v.len() < 3 || v.len() % 3 != 0 {
            return Err(format!("Vertices: {} floats", v.len()));
        }
        let vc = v.len() / 3;
        let hn = nrm.map_or(false, |n| n.len() >= v.len());
        let hu = uv.map_or(false, |u| u.len() >= 2 * vc);
        let mut o = String::from("# OBJ by AssetDaemon\n");
        if !name.is_empty() {
            o.push_str(&format!("o {}\n", name));
        }
        for i in 0..vc {
            o.push_str(&format!(
                "v {:.6} {:.6} {:.6}\n",
                -v[i * 3],
                v[i * 3 + 1],
                v[i * 3 + 2]
            ));
        }
        if let Some(n) = nrm {
            for i in 0..vc {
                o.push_str(&format!(
                    "vn {:.6} {:.6} {:.6}\n",
                    -n[i * 3],
                    n[i * 3 + 1],
                    n[i * 3 + 2]
                ));
            }
        }
        if let Some(u) = uv {
            for i in 0..vc {
                o.push_str(&format!("vt {:.6} {:.6}\n", u[i * 2], 1.0 - u[i * 2 + 1]));
            }
        }
        if idx.len() >= 3 {
            for t in idx.chunks(3) {
                let (a, b, c) = (t[0] as usize + 1, t[2] as usize + 1, t[1] as usize + 1);
                o.push_str(&match (hn, hu) {
                    (true, true) => {
                        format!("f {}/{}/{} {}/{}/{} {}/{}/{}\n", a, a, a, b, b, b, c, c, c)
                    }
                    (true, false) => format!("f {}//{} {}//{} {}//{}\n", a, a, b, b, c, c),
                    (false, true) => format!("f {}/{} {}/{} {}/{}\n", a, a, b, b, c, c),
                    _ => format!("f {} {} {}\n", a, b, c),
                });
            }
        } else {
            for i in (0..vc).step_by(3) {
                o.push_str(&format!("f {} {} {}\n", i + 1, i + 3, i + 2));
            }
        }
        Ok(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_attributes() -> MeshAttributes<'static> {
        MeshAttributes {
            vertices: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            indices: &[0, 1, 2, 0, 2, 3],
            normals: Some(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]),
            uvs: None,
            tangents: None,
            colors: None,
            bone_weights: None,
            bone_indices: None,
            bind_poses: None,
            bone_name_hashes: None,
            root_bone_name_hash: None,
            sub_meshes: None,
            blend_shapes: None,
        }
    }

    /// A structurally valid GLB: correct magic/version/length, both chunks, and every
    /// bufferView references in-bounds data (guards the index-not-duplicated layout).
    #[test]
    fn glb_buffer_layout_is_compact_and_in_bounds() {
        let glb = MeshExporter::build(&sample_attributes()).expect("build glb");
        assert_eq!(&glb[0..4], b"glTF");
        assert_eq!(u32::from_le_bytes(glb[4..8].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(glb[8..12].try_into().unwrap()), glb.len() as u32);

        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        assert_eq!(&glb[16..20], b"JSON");
        let json: serde_json::Value =
            serde_json::from_slice(&glb[20..20 + json_len]).expect("parse json");

        let bin_len = u32::from_le_bytes(glb[20 + json_len..24 + json_len].try_into().unwrap()) as usize;
        assert_eq!(&glb[24 + json_len..28 + json_len], b"BIN\x00");
        let bin_start = 28 + json_len;
        let bin = &glb[bin_start..bin_start + bin_len];

        // buffer.byteLength must equal the written BIN chunk length
        assert_eq!(
            json["buffers"][0]["byteLength"].as_u64().unwrap(),
            bin_len as u64
        );

        // every bufferView must stay within the BIN data (no orphan/out-of-range refs)
        let buffer_views = json["bufferViews"].as_array().unwrap();
        assert_eq!(buffer_views.len(), 3, "index + position + normal");
        for bv in buffer_views {
            let off = bv["byteOffset"].as_u64().unwrap() as usize;
            let len = bv["byteLength"].as_u64().unwrap() as usize;
            assert!(
                off + len <= bin.len(),
                "bufferView out of bounds: off={} len={} bin={}",
                off,
                len,
                bin.len()
            );
        }

        let prim = &json["meshes"][0]["primitives"][0];
        assert_eq!(
            json["accessors"][prim["indices"].as_u64().unwrap() as usize]["count"].as_u64().unwrap(),
            6
        );
        assert_eq!(
            json["accessors"][prim["attributes"]["POSITION"].as_u64().unwrap() as usize]["count"]
                .as_u64()
                .unwrap(),
            4
        );
    }
}
