//! glTF 2.0, as the Khronos specification lays it out: a JSON document (alone in a `.gltf`, its
//! buffers and pictures in files beside it or in data URIs; or the first chunk of a binary
//! `.glb`, its buffer the second) of buffers, views into them and accessors reading typed
//! numbers out of those; meshes of primitives; materials; textures, their pictures and
//! samplers; nodes; scenes; cameras; skins; animations; and lights (KHR_lights_punctual), unlit
//! materials (KHR_materials_unlit) and emissive strength (KHR_materials_emissive_strength).

use std::path::Path;

use veymelo_json::Value as Json;

use crate::{AlphaMode, Animation, Camera, CameraKind, Channel, Interp, Light, LightKind, Map, Mat4, Material, Mesh, Model, Moves, Node, Primitive, Quat, Skin, Target, Texture, Wrap};

/// A glTF document's numbers and lists, read as it goes.
struct Doc<'a> {
    json: Json,
    buffers: Vec<Vec<u8>>,
    dir: &'a Path,
}

fn num(j: Option<&Json>, d: f64) -> f64 {
    j.and_then(Json::as_f64).unwrap_or(d)
}

fn index(j: Option<&Json>) -> Option<usize> {
    j.and_then(Json::as_u64).map(|v| v as usize)
}

fn list<'j>(j: &'j Json, key: &str) -> &'j [Json] {
    j.get(key).and_then(Json::as_array).map_or(&[], Vec::as_slice)
}

fn floats(j: Option<&Json>) -> Vec<f64> {
    j.and_then(Json::as_array).map(|a| a.iter().filter_map(Json::as_f64).collect()).unwrap_or_default()
}

/// A URI's `%xx` turned back into its bytes.
fn unescape(uri: &str) -> String {
    let b = uri.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&uri[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The bytes a URI names: a data URI's, or a file's beside the document.
fn load(uri: &str, dir: &Path) -> Result<Vec<u8>, String> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let (_, data) = rest.split_once(',').ok_or("a data URI with no data")?;
        return if rest.split(',').next().is_some_and(|h| h.ends_with(";base64")) { crate::base64::decode(data) } else { Ok(unescape(data).into_bytes()) };
    }
    let path = dir.join(unescape(uri));
    std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

impl Doc<'_> {
    /// The bytes of buffer view `v`, and its stride.
    fn view(&self, v: usize) -> Result<(&[u8], Option<usize>), String> {
        let view = list(&self.json, "bufferViews").get(v).ok_or_else(|| format!("there is no buffer view {v}"))?;
        let buffer = self.buffers.get(index(view.get("buffer")).unwrap_or(usize::MAX)).ok_or_else(|| format!("buffer view {v} names no buffer"))?;
        let (offset, length) = (num(view.get("byteOffset"), 0.0) as usize, num(view.get("byteLength"), 0.0) as usize);
        let bytes = buffer.get(offset..offset + length).ok_or_else(|| format!("buffer view {v} runs past its buffer"))?;
        Ok((bytes, index(view.get("byteStride")).filter(|s| *s > 0)))
    }

    /// Accessor `a` as numbers, `width` a value (and that width).
    fn numbers(&self, a: usize) -> Result<(Vec<f32>, usize), String> {
        let acc = list(&self.json, "accessors").get(a).ok_or_else(|| format!("there is no accessor {a}"))?;
        let width = match acc.get("type").and_then(Json::as_str) {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") | Some("MAT2") => 4,
            Some("MAT3") => 9,
            Some("MAT4") => 16,
            other => return Err(format!("accessor {a} has an unknown type {other:?}")),
        };
        let kind = num(acc.get("componentType"), 5126.0) as u32;
        let normalized = acc.get("normalized").and_then(Json::as_bool).unwrap_or(false);
        let count = num(acc.get("count"), 0.0) as usize;
        let mut out = vec![0.0f32; count * width];
        if let Some(v) = index(acc.get("bufferView")) {
            let (bytes, stride) = self.view(v)?;
            read_into(&mut out, bytes, num(acc.get("byteOffset"), 0.0) as usize, stride, kind, width, count, normalized).map_err(|e| format!("accessor {a}: {e}"))?;
        }
        // a few values given apart from the rest
        if let Some(sparse) = acc.get("sparse") {
            let n = num(sparse.get("count"), 0.0) as usize;
            let (ix, vals) = (sparse.get("indices").ok_or("a sparse accessor with no indices")?, sparse.get("values").ok_or("a sparse accessor with no values")?);
            let mut at = vec![0.0f32; n];
            let (b, _) = self.view(index(ix.get("bufferView")).unwrap_or(usize::MAX))?;
            read_into(&mut at, b, num(ix.get("byteOffset"), 0.0) as usize, None, num(ix.get("componentType"), 5125.0) as u32, 1, n, false)?;
            let mut values = vec![0.0f32; n * width];
            let (b, _) = self.view(index(vals.get("bufferView")).unwrap_or(usize::MAX))?;
            read_into(&mut values, b, num(vals.get("byteOffset"), 0.0) as usize, None, kind, width, n, normalized)?;
            for (k, i) in at.iter().enumerate() {
                let i = *i as usize;
                if i < count {
                    out[i * width..(i + 1) * width].copy_from_slice(&values[k * width..(k + 1) * width]);
                }
            }
        }
        Ok((out, width))
    }

    fn vec<const N: usize>(&self, a: usize) -> Result<Vec<[f32; N]>, String> {
        let (v, w) = self.numbers(a)?;
        if w < N {
            return Err(format!("accessor {a} has {w} numbers a value, {N} are wanted"));
        }
        Ok(v.chunks_exact(w).map(|c| std::array::from_fn(|i| c[i])).collect())
    }
}

/// `count` values of `width` numbers of component type `kind` from `bytes` at `offset` (each
/// `stride` bytes on when given), into `out`.
#[allow(clippy::too_many_arguments)]
fn read_into(out: &mut [f32], bytes: &[u8], offset: usize, stride: Option<usize>, kind: u32, width: usize, count: usize, normalized: bool) -> Result<(), String> {
    let size = match kind {
        5120 | 5121 => 1,
        5122 | 5123 => 2,
        5125 | 5126 => 4,
        _ => return Err(format!("component type {kind} is not one glTF has")),
    };
    let step = stride.unwrap_or(size * width);
    for i in 0..count {
        for c in 0..width {
            let at = offset + i * step + c * size;
            let b = bytes.get(at..at + size).ok_or("it runs past its buffer view")?;
            out[i * width + c] = match kind {
                5120 => {
                    let v = b[0] as i8 as f32;
                    if normalized { (v / 127.0).max(-1.0) } else { v }
                }
                5121 => {
                    let v = b[0] as f32;
                    if normalized { v / 255.0 } else { v }
                }
                5122 => {
                    let v = i16::from_le_bytes([b[0], b[1]]) as f32;
                    if normalized { (v / 32767.0).max(-1.0) } else { v }
                }
                5123 => {
                    let v = u16::from_le_bytes([b[0], b[1]]) as f32;
                    if normalized { v / 65535.0 } else { v }
                }
                5125 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
                _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            };
        }
    }
    Ok(())
}

/// A 4 × 4 matrix from sixteen numbers down its columns.
fn matrix(v: &[f32]) -> Mat4 {
    Mat4(std::array::from_fn(|r| std::array::from_fn(|c| v.get(c * 4 + r).copied().unwrap_or(if r == c { 1.0 } else { 0.0 }) as f64)))
}

/// A model from a `.gltf` or `.glb` file's bytes (`dir`: where the files it names are).
pub fn read(bytes: &[u8], dir: &Path) -> Result<Model, String> {
    let (text, bin) = if bytes.starts_with(b"glTF") {
        // a binary file: a header, then chunks of a length, a kind and their bytes
        let mut at = 12;
        let (mut text, mut bin) = (None, None);
        while at + 8 <= bytes.len() {
            let len = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
            let kind = &bytes[at + 4..at + 8];
            let body = bytes.get(at + 8..at + 8 + len).ok_or("a .glb chunk runs past the end of the file")?;
            match kind {
                b"JSON" => text = Some(String::from_utf8_lossy(body).into_owned()),
                b"BIN\0" => bin = Some(body.to_vec()),
                _ => {}
            }
            at += 8 + len.div_ceil(4) * 4;
        }
        (text.ok_or("the .glb has no JSON chunk")?, bin)
    } else {
        (String::from_utf8_lossy(bytes).into_owned(), None)
    };
    let json = Json::parse(&text).map_err(|e| format!("the glTF is not JSON: {e}"))?;
    let version = json.get("asset").and_then(|a| a.get("version")).and_then(Json::as_str).unwrap_or("");
    if !version.starts_with('2') {
        return Err(format!("this reads glTF 2.0, not version \"{version}\""));
    }
    let mut notes: Vec<String> = Vec::new();
    for e in list(&json, "extensionsRequired").iter().filter_map(Json::as_str) {
        if !matches!(e, "KHR_lights_punctual" | "KHR_materials_unlit" | "KHR_materials_emissive_strength") {
            return Err(format!("the model needs the glTF extension {e}, which this does not read"));
        }
    }
    let mut buffers = Vec::new();
    for (i, b) in list(&json, "buffers").iter().enumerate() {
        buffers.push(match b.get("uri").and_then(Json::as_str) {
            Some(uri) => load(uri, dir)?,
            None if i == 0 => bin.clone().ok_or("buffer 0 has no URI and there is no binary chunk")?,
            None => return Err(format!("buffer {i} has no URI")),
        });
    }
    let doc = Doc { json, buffers, dir };
    let j = &doc.json;
    // pictures and how they are read
    let images: Vec<Option<Texture>> = list(j, "images")
        .iter()
        .enumerate()
        .map(|(i, img)| {
            let bytes = match (img.get("uri").and_then(Json::as_str), index(img.get("bufferView"))) {
                (Some(uri), _) => load(uri, doc.dir),
                (None, Some(v)) => doc.view(v).map(|(b, _)| b.to_vec()),
                _ => Err(format!("image {i} has no URI or buffer view")),
            };
            match bytes.and_then(|b| veymelo_os::read_picture_bytes(&b)) {
                Ok(p) => Some(Texture::from_picture(p)),
                Err(e) => {
                    notes.push(format!("picture {i} cannot be read ({e}): white stands in"));
                    None
                }
            }
        })
        .collect();
    let wrap = |v: f64| match v as u32 {
        33071 => Wrap::Clamp,
        33648 => Wrap::Mirror,
        _ => Wrap::Repeat,
    };
    let textures: Vec<Texture> = list(j, "textures")
        .iter()
        .map(|t| {
            let mut tex = index(t.get("source")).and_then(|s| images.get(s).cloned().flatten()).unwrap_or_else(|| Texture::solid([1.0; 4]));
            if let Some(s) = index(t.get("sampler")).and_then(|s| list(j, "samplers").get(s)) {
                tex.wrap = [wrap(num(s.get("wrapS"), 10497.0)), wrap(num(s.get("wrapT"), 10497.0))];
                tex.smooth = num(s.get("magFilter"), 9729.0) as u32 != 9728;
            }
            tex
        })
        .collect();
    let map = |m: Option<&Json>| m.and_then(|m| Some(Map { texture: index(m.get("index")).filter(|i| *i < textures.len())?, set: num(m.get("texCoord"), 0.0) as u32 }));
    let materials: Vec<Material> = list(j, "materials")
        .iter()
        .map(|m| {
            let pbr = m.get("pbrMetallicRoughness");
            let ext = m.get("extensions");
            let base = floats(pbr.and_then(|p| p.get("baseColorFactor")));
            let emissive = floats(m.get("emissiveFactor"));
            let strength = num(ext.and_then(|e| e.get("KHR_materials_emissive_strength")).and_then(|e| e.get("emissiveStrength")), 1.0) as f32;
            Material {
                name: m.get("name").and_then(Json::as_str).unwrap_or("").to_string(),
                base: if base.len() == 4 { std::array::from_fn(|i| base[i] as f32) } else { [1.0; 4] },
                base_map: map(pbr.and_then(|p| p.get("baseColorTexture"))),
                metallic: num(pbr.and_then(|p| p.get("metallicFactor")), 1.0) as f32,
                roughness: num(pbr.and_then(|p| p.get("roughnessFactor")), 1.0) as f32,
                metal_rough_map: map(pbr.and_then(|p| p.get("metallicRoughnessTexture"))),
                normal_map: map(m.get("normalTexture")),
                normal_scale: num(m.get("normalTexture").and_then(|n| n.get("scale")), 1.0) as f32,
                occlusion_map: map(m.get("occlusionTexture")),
                occlusion: num(m.get("occlusionTexture").and_then(|n| n.get("strength")), 1.0) as f32,
                emissive: if emissive.len() == 3 { std::array::from_fn(|i| emissive[i] as f32 * strength) } else { [0.0; 3] },
                emissive_map: map(m.get("emissiveTexture")),
                alpha: match m.get("alphaMode").and_then(Json::as_str) {
                    Some("MASK") => AlphaMode::Mask(num(m.get("alphaCutoff"), 0.5) as f32),
                    Some("BLEND") => AlphaMode::Blend,
                    _ => AlphaMode::Opaque,
                },
                two_sided: m.get("doubleSided").and_then(Json::as_bool).unwrap_or(false),
                unlit: ext.is_some_and(|e| e.get("KHR_materials_unlit").is_some()),
            }
        })
        .collect();
    // meshes
    let mut meshes = Vec::new();
    for (mi, m) in list(j, "meshes").iter().enumerate() {
        let mut prims = Vec::new();
        for (pi, p) in list(m, "primitives").iter().enumerate() {
            let mode = num(p.get("mode"), 4.0) as u32;
            if mode < 4 {
                notes.push(format!("mesh {mi} primitive {pi} is points or lines: left out"));
                continue;
            }
            let attrs = p.get("attributes").ok_or_else(|| format!("mesh {mi} primitive {pi} has no attributes"))?;
            let attr = |k: &str| index(attrs.get(k));
            let Some(pos) = attr("POSITION") else {
                notes.push(format!("mesh {mi} primitive {pi} has no positions: left out"));
                continue;
            };
            let mut prim = Primitive { positions: doc.vec::<3>(pos)?, material: index(p.get("material")).filter(|m| *m < materials.len()), ..Default::default() };
            let n = prim.positions.len();
            if let Some(a) = attr("NORMAL") {
                prim.normals = doc.vec::<3>(a)?;
            }
            for s in 0..2 {
                if let Some(a) = attr(&format!("TEXCOORD_{s}")) {
                    prim.uvs[s] = doc.vec::<2>(a)?;
                }
            }
            if let Some(a) = attr("COLOR_0") {
                let (v, w) = doc.numbers(a)?;
                prim.colors = v.chunks_exact(w).map(|c| [c[0], c[1], c[2], if w > 3 { c[3] } else { 1.0 }]).collect();
            }
            if let Some(a) = attr("TANGENT") {
                prim.tangents = doc.vec::<4>(a)?;
            }
            if let (Some(jn), Some(wt)) = (attr("JOINTS_0"), attr("WEIGHTS_0")) {
                prim.joints = doc.vec::<4>(jn)?.into_iter().map(|v| v.map(|x| x as u16)).collect();
                prim.weights = doc.vec::<4>(wt)?;
            }
            for t in list(p, "targets") {
                prim.targets.push(Target {
                    positions: index(t.get("POSITION")).map(|a| doc.vec::<3>(a)).transpose()?.unwrap_or_default(),
                    normals: index(t.get("NORMAL")).map(|a| doc.vec::<3>(a)).transpose()?.unwrap_or_default(),
                });
            }
            let order: Vec<u32> = match index(p.get("indices")) {
                Some(a) => doc.numbers(a)?.0.into_iter().map(|v| v as u32).collect(),
                None => (0..n as u32).collect(),
            };
            prim.indices = match mode {
                5 => (2..order.len()).flat_map(|k| if k % 2 == 0 { [order[k - 2], order[k - 1], order[k]] } else { [order[k - 1], order[k - 2], order[k]] }).collect(),
                6 => (2..order.len()).flat_map(|k| [order[0], order[k - 1], order[k]]).collect(),
                _ => order.as_chunks::<3>().0.iter().flatten().copied().collect(),
            };
            if prim.indices.iter().any(|&i| i as usize >= n) {
                return Err(format!("mesh {mi} primitive {pi} names a vertex it does not have"));
            }
            if prim.normals.len() != n {
                prim.flat();
            }
            if prim.material.and_then(|m| materials.get(m)).is_some_and(|m| m.normal_map.is_some()) && prim.tangents.len() != prim.positions.len() {
                prim.make_tangents();
            }
            prims.push(prim);
        }
        meshes.push(Mesh { name: m.get("name").and_then(Json::as_str).unwrap_or("").to_string(), primitives: prims, weights: floats(m.get("weights")).into_iter().map(|v| v as f32).collect() });
    }
    let nodes: Vec<Node> = list(j, "nodes")
        .iter()
        .map(|n| {
            let (t, r, s, m) = (floats(n.get("translation")), floats(n.get("rotation")), floats(n.get("scale")), floats(n.get("matrix")));
            Node {
                name: n.get("name").and_then(Json::as_str).unwrap_or("").to_string(),
                translation: if t.len() == 3 { [t[0], t[1], t[2]] } else { [0.0; 3] },
                rotation: if r.len() == 4 { Quat { x: r[0], y: r[1], z: r[2], w: r[3] }.normalize() } else { Quat::IDENTITY },
                scale: if s.len() == 3 { [s[0], s[1], s[2]] } else { [1.0; 3] },
                matrix: (m.len() == 16).then(|| matrix(&m.iter().map(|v| *v as f32).collect::<Vec<_>>())),
                mesh: index(n.get("mesh")),
                skin: index(n.get("skin")),
                camera: index(n.get("camera")),
                light: index(n.get("extensions").and_then(|e| e.get("KHR_lights_punctual")).and_then(|l| l.get("light"))),
                weights: n.get("weights").map(|w| floats(Some(w)).into_iter().map(|v| v as f32).collect()),
                children: list(n, "children").iter().filter_map(Json::as_u64).map(|c| c as usize).collect(),
            }
        })
        .collect();
    let roots: Vec<usize> = match list(j, "scenes").get(index(j.get("scene")).unwrap_or(0)) {
        Some(scene) => list(scene, "nodes").iter().filter_map(Json::as_u64).map(|v| v as usize).collect(),
        // no scene: every node no other holds
        None => (0..nodes.len()).filter(|i| !nodes.iter().any(|n| n.children.contains(i))).collect(),
    };
    let cameras = list(j, "cameras")
        .iter()
        .map(|c| Camera {
            name: c.get("name").and_then(Json::as_str).unwrap_or("").to_string(),
            kind: match c.get("type").and_then(Json::as_str) {
                Some("orthographic") => {
                    let o = c.get("orthographic");
                    CameraKind::Orthographic { xmag: num(o.and_then(|o| o.get("xmag")), 1.0), ymag: num(o.and_then(|o| o.get("ymag")), 1.0) }
                }
                _ => CameraKind::Perspective { yfov: num(c.get("perspective").and_then(|p| p.get("yfov")), 0.8) },
            },
        })
        .collect();
    let lights = j
        .get("extensions")
        .and_then(|e| e.get("KHR_lights_punctual"))
        .map(|l| list(l, "lights"))
        .unwrap_or(&[])
        .iter()
        .map(|l| {
            let c = floats(l.get("color"));
            Light {
                name: l.get("name").and_then(Json::as_str).unwrap_or("").to_string(),
                kind: match l.get("type").and_then(Json::as_str) {
                    Some("directional") => LightKind::Directional,
                    Some("spot") => {
                        let s = l.get("spot");
                        LightKind::Spot { inner: num(s.and_then(|s| s.get("innerConeAngle")), 0.0), outer: num(s.and_then(|s| s.get("outerConeAngle")), std::f64::consts::FRAC_PI_4) }
                    }
                    _ => LightKind::Point,
                },
                color: if c.len() == 3 { [c[0] as f32, c[1] as f32, c[2] as f32] } else { [1.0; 3] },
                intensity: num(l.get("intensity"), 1.0) as f32,
                range: l.get("range").and_then(Json::as_f64).map(|r| r as f32),
            }
        })
        .collect();
    let mut skins = Vec::new();
    for s in list(j, "skins") {
        let joints: Vec<usize> = list(s, "joints").iter().filter_map(Json::as_u64).map(|v| v as usize).collect();
        let inverse_bind = match index(s.get("inverseBindMatrices")) {
            Some(a) => doc.numbers(a)?.0.as_chunks::<16>().0.iter().map(|m| matrix(m)).collect(),
            None => vec![Mat4::IDENTITY; joints.len()],
        };
        skins.push(Skin { joints, inverse_bind });
    }
    let mut animations = Vec::new();
    for (ai, a) in list(j, "animations").iter().enumerate() {
        let samplers = list(a, "samplers");
        let mut channels = Vec::new();
        for c in list(a, "channels") {
            let target = c.get("target");
            let (Some(node), Some(path)) = (index(target.and_then(|t| t.get("node"))), target.and_then(|t| t.get("path")).and_then(Json::as_str)) else { continue };
            let moves = match path {
                "translation" => Moves::Translation,
                "rotation" => Moves::Rotation,
                "scale" => Moves::Scale,
                "weights" => Moves::Weights,
                other => {
                    notes.push(format!("animation {ai} moves a node's \"{other}\", which this does not read"));
                    continue;
                }
            };
            let Some(s) = index(c.get("sampler")).and_then(|i| samplers.get(i)) else { continue };
            let interp = match s.get("interpolation").and_then(Json::as_str) {
                Some("STEP") => Interp::Step,
                Some("CUBICSPLINE") => Interp::Cubic,
                _ => Interp::Linear,
            };
            let times = doc.numbers(index(s.get("input")).ok_or("an animation sampler with no input")?)?.0;
            let values = doc.numbers(index(s.get("output")).ok_or("an animation sampler with no output")?)?.0;
            let per_key = if interp == Interp::Cubic { 3 } else { 1 };
            let width = match moves {
                Moves::Translation | Moves::Scale => 3,
                Moves::Rotation => 4,
                Moves::Weights => values.len() / (times.len() * per_key).max(1),
            };
            channels.push(Channel { node, moves, interp, times, values, width });
        }
        animations.push(Animation { name: a.get("name").and_then(Json::as_str).map_or_else(|| format!("Animation {}", ai + 1), str::to_string), channels });
    }
    Ok(Model { meshes, nodes, roots, materials, textures, cameras, lights, skins, animations, notes })
}
