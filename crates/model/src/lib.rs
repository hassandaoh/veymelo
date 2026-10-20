//! 3D models read from files: glTF 2.0 (`.gltf` with its buffers beside it or inside it, and
//! `.glb`), Wavefront OBJ with its MTL materials and STL — written from the Khronos glTF 2.0
//! specification and the OBJ/MTL and STL file descriptions; and solids made from a few numbers
//! ([`shapes`]).
//!
//! A model is meshes (triangles with one material each), metallic-roughness materials and the
//! pictures they use, a tree of nodes placing the meshes (and cameras and lights), skins that
//! bend meshes by joint nodes, morph targets, and animations moving the nodes. [`Model::pose`]
//! works out where everything is at a moment of an animation.
//!
//! Models are in their own units with +Y up and +Z toward the viewer, as glTF has them.

mod base64;
pub mod gltf;
pub mod obj;
pub mod shapes;
pub mod stl;

use std::path::Path;

pub use veymelo_geom::{Mat4, Quat, Vec3, vec3};

/// How a picture repeats past its edges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Wrap {
    #[default]
    Repeat,
    Clamp,
    Mirror,
}

/// A picture a material uses: straight RGBA from 0 to 1, as the file keeps it (a colour picture
/// in sRGB), the top row first.
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    pub width: usize,
    pub height: usize,
    pub px: Vec<[f32; 4]>,
    pub wrap: [Wrap; 2],
    /// Read between its pixels (else the nearest).
    pub smooth: bool,
}

impl Texture {
    pub fn solid(c: [f32; 4]) -> Texture {
        Texture { width: 1, height: 1, px: vec![c], wrap: [Wrap::Repeat; 2], smooth: true }
    }

    /// From a picture the system read (premultiplied).
    pub fn from_picture(p: veymelo_os::Picture) -> Texture {
        let px = p.px.into_iter().map(|q| if q[3] > 1e-6 { [q[0] / q[3], q[1] / q[3], q[2] / q[3], q[3]] } else { [0.0; 4] }).collect();
        Texture { width: p.width, height: p.height, px, wrap: [Wrap::Repeat; 2], smooth: true }
    }

    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        let fold = |v: i64, n: usize, wrap: Wrap| -> usize {
            let n = n as i64;
            (match wrap {
                Wrap::Repeat => v.rem_euclid(n),
                Wrap::Clamp => v.clamp(0, n - 1),
                Wrap::Mirror => {
                    let m = v.rem_euclid(2 * n);
                    if m < n { m } else { 2 * n - 1 - m }
                }
            }) as usize
        };
        self.px[fold(y, self.height, self.wrap[1]) * self.width + fold(x, self.width, self.wrap[0])]
    }

    /// The picture at texture point (u, v): (0, 0) its top left, (1, 1) its bottom right.
    pub fn at(&self, u: f64, v: f64) -> [f32; 4] {
        if self.px.is_empty() {
            return [1.0; 4];
        }
        let (x, y) = (u * self.width as f64 - 0.5, v * self.height as f64 - 0.5);
        if !self.smooth {
            return self.texel(x.round() as i64, y.round() as i64);
        }
        let (x0, y0) = (x.floor(), y.floor());
        let (tx, ty) = ((x - x0) as f32, (y - y0) as f32);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let (a, b, c, d) = (self.texel(x0, y0), self.texel(x0 + 1, y0), self.texel(x0, y0 + 1), self.texel(x0 + 1, y0 + 1));
        let mut out = [0.0; 4];
        for k in 0..4 {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bottom = c[k] + (d[k] - c[k]) * tx;
            out[k] = top + (bottom - top) * ty;
        }
        out
    }
}

/// A material's use of a picture: which, and which of a vertex's texture points it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Map {
    pub texture: usize,
    pub set: u32,
}

/// How a material's alpha is taken.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum AlphaMode {
    /// Not at all.
    #[default]
    Opaque,
    /// Shown where at least this, else not.
    Mask(f32),
    /// See-through.
    Blend,
}

/// A metallic-roughness material, as glTF has them.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    /// Linear RGBA.
    pub base: [f32; 4],
    pub base_map: Option<Map>,
    pub metallic: f32,
    pub roughness: f32,
    /// Roughness in its green, metalness in its blue.
    pub metal_rough_map: Option<Map>,
    pub normal_map: Option<Map>,
    pub normal_scale: f32,
    /// How much light reaches, in its red.
    pub occlusion_map: Option<Map>,
    pub occlusion: f32,
    /// Linear light it gives off.
    pub emissive: [f32; 3],
    pub emissive_map: Option<Map>,
    pub alpha: AlphaMode,
    pub two_sided: bool,
    /// Shown as its colour, not lit.
    pub unlit: bool,
}

impl Default for Material {
    fn default() -> Material {
        Material {
            name: String::new(),
            base: [1.0; 4],
            base_map: None,
            metallic: 1.0,
            roughness: 1.0,
            metal_rough_map: None,
            normal_map: None,
            normal_scale: 1.0,
            occlusion_map: None,
            occlusion: 1.0,
            emissive: [0.0; 3],
            emissive_map: None,
            alpha: AlphaMode::Opaque,
            two_sided: false,
            unlit: false,
        }
    }
}

/// How a morph target moves a primitive's vertices.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Target {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
}

/// Triangles of one material.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Primitive {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Texture points, the first set and the second (each empty when there are none).
    pub uvs: [Vec<[f32; 2]>; 2],
    /// Linear RGBA (empty: white).
    pub colors: Vec<[f32; 4]>,
    /// Each vertex's tangent and, in w, which way its bitangent goes (for normal maps).
    pub tangents: Vec<[f32; 4]>,
    /// The joints that bend each vertex, and how much each does (skinned meshes).
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub targets: Vec<Target>,
    /// Three a triangle, counter-clockwise from the front.
    pub indices: Vec<u32>,
    pub material: Option<usize>,
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(a: [f32; 3]) -> Option<[f32; 3]> {
    let l = dot(a, a).sqrt();
    (l > 1e-20 && l.is_finite()).then(|| a.map(|v| v / l))
}

impl Primitive {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    /// Each triangle its own three vertices, facing the way it faces (for a mesh that gives no
    /// normals: flat faces, as glTF says).
    pub fn flat(&mut self) {
        let old = std::mem::take(self);
        self.material = old.material;
        for t in old.indices.as_chunks::<3>().0 {
            let (a, b, c) = (old.positions[t[0] as usize], old.positions[t[1] as usize], old.positions[t[2] as usize]);
            let n = unit(cross(sub(b, a), sub(c, a))).unwrap_or([0.0, 0.0, 1.0]);
            for &i in t {
                let i = i as usize;
                self.indices.push(self.positions.len() as u32);
                self.positions.push(old.positions[i]);
                self.normals.push(n);
                for s in 0..2 {
                    if let Some(uv) = old.uvs[s].get(i) {
                        self.uvs[s].push(*uv);
                    }
                }
                if let Some(c) = old.colors.get(i) {
                    self.colors.push(*c);
                }
                if let (Some(j), Some(w)) = (old.joints.get(i), old.weights.get(i)) {
                    self.joints.push(*j);
                    self.weights.push(*w);
                }
            }
        }
        // morph targets move each copy of a vertex as they moved it
        self.targets = old
            .targets
            .iter()
            .map(|g| {
                let pick = |v: &Vec<[f32; 3]>| if v.is_empty() { Vec::new() } else { old.indices.iter().map(|&i| v.get(i as usize).copied().unwrap_or([0.0; 3])).collect() };
                Target { positions: pick(&g.positions), normals: Vec::new() }
            })
            .collect();
    }

    /// Tangents from how its texture points lie across its triangles, for normal maps.
    pub fn make_tangents(&mut self) {
        let n = self.positions.len();
        if self.uvs[0].len() != n || self.normals.len() != n {
            return;
        }
        let (mut tan, mut bit) = (vec![[0.0f32; 3]; n], vec![[0.0f32; 3]; n]);
        for t in self.indices.as_chunks::<3>().0 {
            let (i, j, k) = (t[0] as usize, t[1] as usize, t[2] as usize);
            let (e1, e2) = (sub(self.positions[j], self.positions[i]), sub(self.positions[k], self.positions[i]));
            let (w0, w1, w2) = (self.uvs[0][i], self.uvs[0][j], self.uvs[0][k]);
            let (du1, dv1, du2, dv2) = (w1[0] - w0[0], w1[1] - w0[1], w2[0] - w0[0], w2[1] - w0[1]);
            let det = du1 * dv2 - du2 * dv1;
            if det.abs() < 1e-12 {
                continue;
            }
            let r = 1.0 / det;
            let s = [0, 1, 2].map(|c| (e1[c] * dv2 - e2[c] * dv1) * r);
            let q = [0, 1, 2].map(|c| (e2[c] * du1 - e1[c] * du2) * r);
            for v in [i, j, k] {
                for c in 0..3 {
                    tan[v][c] += s[c];
                    bit[v][c] += q[c];
                }
            }
        }
        self.tangents = (0..n)
            .map(|v| {
                let nn = self.normals[v];
                let d = dot(nn, tan[v]);
                let o = unit(sub(tan[v], nn.map(|c| c * d))).unwrap_or_else(|| unit(cross(nn, if nn[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] })).unwrap_or([1.0, 0.0, 0.0]));
                let w = if dot(cross(nn, o), bit[v]) < 0.0 { -1.0 } else { 1.0 };
                [o[0], o[1], o[2], w]
            })
            .collect();
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub name: String,
    pub primitives: Vec<Primitive>,
    /// Its morph targets' weights when nothing animates them.
    pub weights: Vec<f32>,
}

/// A node of the tree: where it is from its parent (moved, turned and scaled, or a matrix), and
/// what it holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub name: String,
    pub translation: [f64; 3],
    pub rotation: Quat,
    pub scale: [f64; 3],
    /// A matrix in place of the three (never animated).
    pub matrix: Option<Mat4>,
    pub mesh: Option<usize>,
    pub skin: Option<usize>,
    /// A camera looking down its −Z, +Y up.
    pub camera: Option<usize>,
    /// A light shining down its −Z.
    pub light: Option<usize>,
    pub weights: Option<Vec<f32>>,
    pub children: Vec<usize>,
}

impl Default for Node {
    fn default() -> Node {
        Node { name: String::new(), translation: [0.0; 3], rotation: Quat::IDENTITY, scale: [1.0; 3], matrix: None, mesh: None, skin: None, camera: None, light: None, weights: None, children: Vec::new() }
    }
}

/// Moved by `t`, after turning by `r`, after scaling by `s`.
pub fn trs(t: [f64; 3], r: Quat, s: [f64; 3]) -> Mat4 {
    let mut m = r.to_mat4();
    for (row, ti) in m.0.iter_mut().zip(t) {
        for (v, sj) in row.iter_mut().zip(s) {
            *v *= sj;
        }
        row[3] = ti;
    }
    m
}

impl Node {
    pub fn local(&self) -> Mat4 {
        self.matrix.unwrap_or_else(|| trs(self.translation, self.rotation, self.scale))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CameraKind {
    /// How wide it sees up and down (radians).
    Perspective { yfov: f64 },
    /// How far it sees either side, across and up.
    Orthographic { xmag: f64, ymag: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub name: String,
    pub kind: CameraKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightKind {
    Directional,
    Point,
    /// Its cone: full inside `inner`, nothing past `outer` (radians from its middle).
    Spot { inner: f64, outer: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub name: String,
    pub kind: LightKind,
    /// Linear RGB.
    pub color: [f32; 3],
    /// Lux for a directional light, candela for the others.
    pub intensity: f32,
    pub range: Option<f32>,
}

/// The joint nodes that bend a mesh, and where each was when the mesh was bound to it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skin {
    pub joints: Vec<usize>,
    pub inverse_bind: Vec<Mat4>,
}

/// What an animation channel moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moves {
    Translation,
    Rotation,
    Scale,
    /// A mesh's morph weights.
    Weights,
}

/// How an animation goes between its keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interp {
    Step,
    Linear,
    /// Cubic, with tangents in and out at each key.
    Cubic,
}

/// One thing a node does over an animation: keys at times, `width` numbers a key (for cubic, a
/// tangent in, the value and a tangent out a key).
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub node: usize,
    pub moves: Moves,
    pub interp: Interp,
    pub times: Vec<f32>,
    pub values: Vec<f32>,
    pub width: usize,
}

impl Channel {
    /// Its value at `t` seconds (held before its first key and after its last).
    pub fn at(&self, t: f64) -> Vec<f64> {
        let w = self.width;
        let n = self.times.len();
        let per = if self.interp == Interp::Cubic { 3 * w } else { w };
        let value = |k: usize| -> Vec<f64> {
            let at = k * per + if self.interp == Interp::Cubic { w } else { 0 };
            (0..w).map(|i| self.values.get(at + i).copied().unwrap_or(0.0) as f64).collect()
        };
        if n == 0 || w == 0 {
            return vec![0.0; w];
        }
        if t <= self.times[0] as f64 {
            return value(0);
        }
        if t >= self.times[n - 1] as f64 {
            return value(n - 1);
        }
        let k = self.times.partition_point(|x| (*x as f64) <= t).clamp(1, n - 1) - 1;
        let (t0, t1) = (self.times[k] as f64, self.times[k + 1] as f64);
        let d = (t1 - t0).max(1e-9);
        let s = ((t - t0) / d).clamp(0.0, 1.0);
        let (a, b) = (value(k), value(k + 1));
        match self.interp {
            Interp::Step => a,
            Interp::Linear if self.moves == Moves::Rotation => {
                let q = |v: &[f64]| Quat { x: v[0], y: v[1], z: v[2], w: v[3] };
                let r = q(&a).slerp(q(&b), s);
                vec![r.x, r.y, r.z, r.w]
            }
            Interp::Linear => a.iter().zip(&b).map(|(x, y)| x + (y - x) * s).collect(),
            Interp::Cubic => {
                // Hermite between the two values, with the out tangent of the first key and the
                // in tangent of the second (scaled by the time between)
                let out_tan = |k: usize| (0..w).map(|i| self.values.get(k * per + 2 * w + i).copied().unwrap_or(0.0) as f64 * d).collect::<Vec<_>>();
                let in_tan = |k: usize| (0..w).map(|i| self.values.get(k * per + i).copied().unwrap_or(0.0) as f64 * d).collect::<Vec<_>>();
                let (m0, m1) = (out_tan(k), in_tan(k + 1));
                let (s2, s3) = (s * s, s * s * s);
                let (h00, h10, h01, h11) = (2.0 * s3 - 3.0 * s2 + 1.0, s3 - 2.0 * s2 + s, -2.0 * s3 + 3.0 * s2, s3 - s2);
                let v: Vec<f64> = (0..w).map(|i| h00 * a[i] + h10 * m0[i] + h01 * b[i] + h11 * m1[i]).collect();
                if self.moves == Moves::Rotation {
                    let l = v.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
                    v.into_iter().map(|x| x / l).collect()
                } else {
                    v
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Animation {
    pub name: String,
    pub channels: Vec<Channel>,
}

impl Animation {
    /// How long it lasts, in seconds.
    pub fn length(&self) -> f64 {
        self.channels.iter().filter_map(|c| c.times.last()).fold(0.0f64, |a, t| a.max(*t as f64))
    }
}

/// A model read from a file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub meshes: Vec<Mesh>,
    pub nodes: Vec<Node>,
    /// The nodes at the top of the tree that is shown.
    pub roots: Vec<usize>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub cameras: Vec<Camera>,
    pub lights: Vec<Light>,
    pub skins: Vec<Skin>,
    pub animations: Vec<Animation>,
    /// What could not be read or is not used, said once each.
    pub notes: Vec<String>,
}

/// A node's place from its parent: a matrix, or moved, turned and scaled.
type Local = (Option<Mat4>, [f64; 3], Quat, [f64; 3]);

/// Where everything is at a moment: each node's place in the model, and each node's morph
/// weights.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub world: Vec<Mat4>,
    pub weights: Vec<Vec<f32>>,
}

impl Model {
    /// The model in the file at `path` (a `.gltf`, `.glb`, `.obj` or `.stl`).
    pub fn open(path: &str) -> Result<Model, String> {
        let p = Path::new(path);
        let bytes = std::fs::read(p).map_err(|e| format!("cannot read {path}: {e}"))?;
        let dir = p.parent().unwrap_or(Path::new("."));
        match p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("obj") => obj::read(&String::from_utf8_lossy(&bytes), dir),
            Some("gltf") | Some("glb") => gltf::read(&bytes, dir),
            Some("stl") => stl::read(&bytes),
            _ => Err(format!("{path} is not a model this reads (.gltf, .glb, .obj or .stl)")),
        }
    }

    /// Whether a file's name says it is a model.
    pub fn is_model(path: &str) -> bool {
        matches!(Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("gltf" | "glb" | "obj" | "stl"))
    }

    pub fn triangles(&self) -> usize {
        self.meshes.iter().flat_map(|m| &m.primitives).map(Primitive::triangles).sum()
    }

    /// The longest animation's length, in seconds (0: none).
    pub fn length(&self) -> f64 {
        self.animations.iter().map(Animation::length).fold(0.0, f64::max)
    }

    /// Where everything is `t` seconds into animation `animation` (none: as the file has it).
    pub fn pose(&self, animation: Option<usize>, t: f64) -> Pose {
        let n = self.nodes.len();
        let mut local: Vec<Local> = self.nodes.iter().map(|d| (d.matrix, d.translation, d.rotation, d.scale)).collect();
        let mut weights: Vec<Vec<f32>> = self.nodes.iter().map(|d| d.weights.clone().or_else(|| d.mesh.and_then(|m| self.meshes.get(m)).map(|m| m.weights.clone())).unwrap_or_default()).collect();
        if let Some(a) = animation.and_then(|i| self.animations.get(i)) {
            for c in &a.channels {
                let Some(l) = local.get_mut(c.node) else { continue };
                let v = c.at(t);
                match c.moves {
                    Moves::Translation if v.len() == 3 => l.1 = [v[0], v[1], v[2]],
                    Moves::Rotation if v.len() == 4 => l.2 = Quat { x: v[0], y: v[1], z: v[2], w: v[3] }.normalize(),
                    Moves::Scale if v.len() == 3 => l.3 = [v[0], v[1], v[2]],
                    Moves::Weights => weights[c.node] = v.into_iter().map(|x| x as f32).collect(),
                    _ => {}
                }
                // an animated node is moved, turned and scaled, not by a matrix
                if c.moves != Moves::Weights {
                    l.0 = None;
                }
            }
        }
        let mut world = vec![Mat4::IDENTITY; n];
        let mut seen = vec![false; n];
        let mut stack: Vec<(usize, Mat4)> = self.roots.iter().map(|&r| (r, Mat4::IDENTITY)).collect();
        while let Some((i, parent)) = stack.pop() {
            if i >= n || seen[i] {
                continue;
            }
            seen[i] = true;
            let (m, t, r, s) = local[i];
            world[i] = parent * m.unwrap_or_else(|| trs(t, r, s));
            for &c in &self.nodes[i].children {
                stack.push((c, world[i]));
            }
        }
        Pose { world, weights }
    }

    /// Each joint's bending matrix for `skin` in `pose`.
    pub fn joints(&self, skin: usize, pose: &Pose) -> Vec<Mat4> {
        let Some(s) = self.skins.get(skin) else { return Vec::new() };
        s.joints.iter().enumerate().map(|(k, &j)| pose.world.get(j).copied().unwrap_or(Mat4::IDENTITY) * s.inverse_bind.get(k).copied().unwrap_or(Mat4::IDENTITY)).collect()
    }

    /// The nodes shown that hold meshes, in `pose`: (node, mesh).
    pub fn shown(&self, _pose: &Pose) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut stack: Vec<usize> = self.roots.clone();
        let mut seen = vec![false; self.nodes.len()];
        while let Some(i) = stack.pop() {
            if i >= self.nodes.len() || seen[i] {
                continue;
            }
            seen[i] = true;
            if let Some(m) = self.nodes[i].mesh.filter(|m| *m < self.meshes.len()) {
                out.push((i, m));
            }
            stack.extend(self.nodes[i].children.iter().copied());
        }
        out.sort_unstable();
        out
    }

    /// The box round everything shown in `pose` (unbent by skins): its lowest and highest
    /// corners.
    pub fn bounds(&self, pose: &Pose) -> Option<(Vec3, Vec3)> {
        let mut lo = vec3(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut hi = vec3(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for (node, mesh) in self.shown(pose) {
            let m = pose.world[node];
            for p in self.meshes[mesh].primitives.iter().flat_map(|p| &p.positions) {
                let q = m.apply(vec3(p[0] as f64, p[1] as f64, p[2] as f64));
                lo = vec3(lo.x.min(q.x), lo.y.min(q.y), lo.z.min(q.z));
                hi = vec3(hi.x.max(q.x), hi.y.max(q.y), hi.z.max(q.z));
            }
        }
        lo.x.is_finite().then_some((lo, hi))
    }
}

#[cfg(test)]
mod tests;
