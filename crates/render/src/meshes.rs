//! 3D model layers drawn in the scene: the model posed at the layer's time (its animation,
//! skins and morph targets), its triangles placed in the world, seen through the camera, cut
//! where they come nearer than the camera can see, and filled with four samples a pixel (so
//! their edges are smooth), the nearest kept at each; each sample's colour worked out from its
//! material (colour, pictures, normal map, occlusion, light given off, metal and roughness) and
//! lit by the scene's lights, the flat layers' shadows falling on it.
//!
//! A model unit is `unitScale` pixels, the model's +Y up and +Z toward the viewer turned to the
//! composition's y down and z away, its middle (the box round it as it stands) at the layer's
//! (0, 0, 0).

use std::sync::Arc;

use veymelo_color::{linear_to_srgb, srgb_to_linear};
use veymelo_geom::{Mat3, Mat4, Vec3, vec3};
use veymelo_model::{AlphaMode, Map, Model, Pose};
use veymelo_raster::{Px, par};

use crate::scene::{Camera, Light, Material, NEAR};

/// How far a model is drawn: the canvas pixels from (`x0`, `y0`), `w` × `h` of them, each with
/// the depth of the nearest of it there (infinite: none) and its colour (premultiplied, lit).
pub(crate) struct Shot {
    pub x0: usize,
    pub y0: usize,
    pub w: usize,
    pub h: usize,
    pub px: Vec<(f64, Px)>,
    /// What the surface nearest at each pixel is, for the render passes.
    pub facts: Vec<Facts>,
}

/// What a model's surface is at a pixel: its normal in the world, its texture point, and its
/// material (none: `u32::MAX`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Facts {
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub material: u32,
}

impl Default for Facts {
    fn default() -> Facts {
        Facts { normal: [0.0, 0.0, -1.0], uv: [0.0; 2], material: u32::MAX }
    }
}

impl Shot {
    /// What the surface is at canvas pixel (x, y).
    pub fn facts_at(&self, x: usize, y: usize) -> Facts {
        if x < self.x0 || y < self.y0 || x >= self.x0 + self.w || y >= self.y0 + self.h {
            return Facts::default();
        }
        self.facts.get((y - self.y0) * self.w + x - self.x0).copied().unwrap_or_default()
    }

    /// What of it is at canvas pixel (x, y): its depth and colour.
    pub fn at(&self, x: usize, y: usize) -> Option<(f64, Px)> {
        if x < self.x0 || y < self.y0 || x >= self.x0 + self.w || y >= self.y0 + self.h {
            return None;
        }
        let p = self.px[(y - self.y0) * self.w + x - self.x0];
        (p.0.is_finite() && p.1[3] > 1e-5).then_some(p)
    }

    pub fn reach(&self) -> [i64; 4] {
        [self.x0 as i64, self.y0 as i64, (self.x0 + self.w) as i64, (self.y0 + self.h) as i64]
    }
}

/// A vertex as placed: in the camera's space, in the world, its normal and tangent in the
/// world, its texture points and colour.
#[derive(Clone, Copy, Debug)]
struct Vertex {
    cam: Vec3,
    world: Vec3,
    normal: Vec3,
    tangent: [f64; 4],
    uv: [[f64; 2]; 2],
    color: [f32; 4],
}

impl Vertex {
    fn mix(a: &Vertex, b: &Vertex, t: f64) -> Vertex {
        let l = |x: f64, y: f64| x + (y - x) * t;
        let v = |p: Vec3, q: Vec3| p + (q - p) * t;
        Vertex {
            cam: v(a.cam, b.cam),
            world: v(a.world, b.world),
            normal: v(a.normal, b.normal),
            tangent: std::array::from_fn(|i| l(a.tangent[i], b.tangent[i])),
            uv: std::array::from_fn(|s| [l(a.uv[s][0], b.uv[s][0]), l(a.uv[s][1], b.uv[s][1])]),
            color: std::array::from_fn(|i| a.color[i] + (b.color[i] - a.color[i]) * t as f32),
        }
    }
}

/// A triangle to fill: its corners (on the canvas, and 1 / depth), and what to colour it by.
struct Tri {
    v: [Vertex; 3],
    screen: [[f64; 2]; 3],
    inv_z: [f64; 3],
    material: Option<usize>,
}

/// What a model is drawn with.
pub(crate) struct Look<'a> {
    pub model: &'a Model,
    pub pose: Pose,
    /// From the model's units to the world.
    pub place: Mat4,
    pub cam: &'a Camera,
    /// From the camera's space to the canvas: the lens, then the view.
    pub onto: Mat3,
    pub width: usize,
    pub height: usize,
    /// The layer's own material options.
    pub material: Material,
    pub lights: &'a [Light],
    /// How much of light `l` reaches world point `q` past the flat layers.
    pub unshadowed: &'a (dyn Fn(usize, Vec3) -> [f32; 3] + Sync),
    /// Colours worked in linear light (else as they are seen).
    pub linear: bool,
    /// The environment lighting the scene, if one does.
    pub env: Option<&'a crate::scene::Environment>,
}

/// A 3D model layer's model posed at composition time `when` (its animation by its settings),
/// and the map from the model's units to the layer's own space.
pub(crate) fn posed(eval: &veymelo_expr::Eval, at: veymelo_expr::Place, model: &Model, when: veymelo_time::Tick) -> (Pose, Mat4) {
    let layer = at.layer;
    let read = |path: &str, def: f64| layer.props.prop(path).map_or(def, |p| eval.value(p, at, when).as_f64());
    let anim = (read("model/animation", 0.0).round() as usize).checked_sub(1).filter(|i| *i < model.animations.len());
    let mut secs = layer.layer_time(when).seconds() * read("model/speed", 100.0) / 100.0 + read("model/animationOffset", 0.0);
    if let Some(a) = anim {
        let length = model.animations[a].length();
        let looping = layer.props.prop("model/loop").is_ok_and(|p| eval.value(p, at, when).as_bool());
        if looping && length > 0.0 {
            secs = secs.rem_euclid(length);
        }
    }
    let centre = model.bounds(&model.pose(None, 0.0)).map_or(vec3(0.0, 0.0, 0.0), |(lo, hi)| (lo + hi) * 0.5);
    let k = read("model/unitScale", 100.0);
    // its units to the layer's: y and z turned round, its middle at the origin
    (model.pose(anim, secs), Mat4::scale(vec3(k, -k, -k)) * Mat4::translate(-centre))
}

/// A 3D primitive layer's solid at composition time `when`: its sizes and its surface from its
/// properties, in the layer's own space (y down, z away), its middle at the layer's (0, 0, 0).
/// Kept, as made, for the next frame that asks for the same.
pub(crate) fn primitive(eval: &veymelo_expr::Eval, at: veymelo_expr::Place, when: veymelo_time::Tick, kind: veymelo_project::Primitive) -> Arc<Model> {
    use veymelo_model::shapes::{Shape, Sizes};
    static KEPT: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Arc<Model>>>> = std::sync::OnceLock::new();
    let layer = at.layer;
    let num = |path: &str, d: f64| layer.props.prop(path).map_or(d, |p| eval.value(p, at, when).as_f64()) as f32;
    let color = |path: &str| layer.props.prop(path).ok().map(|p| eval.value(p, at, when).as_color());
    let shape = match kind {
        veymelo_project::Primitive::Cube => Shape::Cube,
        veymelo_project::Primitive::Sphere => Shape::Sphere,
        veymelo_project::Primitive::Plane => Shape::Plane,
        veymelo_project::Primitive::Torus => Shape::Torus,
        veymelo_project::Primitive::Cone => Shape::Cone,
        veymelo_project::Primitive::Cylinder => Shape::Cylinder,
    };
    let height = num("geometryOptions/height", 300.0);
    let sizes = Sizes {
        size: [num("geometryOptions/width", 300.0), height, num("geometryOptions/depth", 300.0)],
        radius: num("geometryOptions/radius", 150.0),
        tube: num("geometryOptions/tubeRadius", 50.0),
        height,
        segments: num("geometryOptions/segments", 48.0).round().max(3.0) as u32,
        rings: num("geometryOptions/rings", 24.0).round().max(2.0) as u32,
    };
    // its colours as they are seen, in linear light as a model's are
    let lin = |c: veymelo_color::Rgba| [srgb_to_linear(c.r) as f32, srgb_to_linear(c.g) as f32, srgb_to_linear(c.b) as f32, c.a as f32];
    let base = color("materialOptions/baseColor").map_or([0.6, 0.6, 0.6, 1.0], lin);
    let glow = color("materialOptions/emissive").map_or([0.0; 4], lin);
    let (metallic, roughness) = (num("materialOptions/metallic", 0.0) / 100.0, num("materialOptions/roughness", 50.0) / 100.0);
    let key = format!("{shape:?}|{sizes:?}|{base:?}|{glow:?}|{metallic}|{roughness}");
    let kept = KEPT.get_or_init(Default::default);
    if let Some(m) = kept.lock().ok().and_then(|k| k.get(&key).cloned()) {
        return m;
    }
    let material = veymelo_model::Material { name: "Primitive".into(), base, metallic: metallic.clamp(0.0, 1.0), roughness: roughness.clamp(0.0, 1.0), emissive: [glow[0], glow[1], glow[2]], ..veymelo_model::Material::default() };
    let mut model = veymelo_model::shapes::model(shape, &sizes, material);
    // +Y up and +Z toward the viewer turned to the layer's y down and z away (a half turn about
    // x, so its triangles keep their way round)
    for p in model.meshes.iter_mut().flat_map(|m| &mut m.primitives) {
        for v in p.positions.iter_mut().chain(p.normals.iter_mut()) {
            *v = [v[0], -v[1], -v[2]];
        }
        for t in &mut p.tangents {
            *t = [t[0], -t[1], -t[2], t[3]];
        }
    }
    let model = Arc::new(model);
    if let Ok(mut k) = kept.lock() {
        if k.len() > 256 {
            k.clear();
        }
        k.insert(key, model.clone());
    }
    model
}

/// The normal matrix of `m`: how it turns surface directions.
fn normals_of(m: &Mat4) -> Mat4 {
    let mut lin = *m;
    for row in lin.0.iter_mut().take(3) {
        row[3] = 0.0;
    }
    lin.inverse().map_or(lin, |i| i.transpose())
}

/// A primitive's vertices posed and placed (in the world, and in the camera's space by `view`),
/// its triangles' corners (turned the right way round), its material and whether it shows from
/// behind.
struct Part {
    verts: Vec<Vertex>,
    corners: Vec<[u32; 3]>,
    material: Option<usize>,
    both: bool,
}

/// The model's primitives posed (`pose`) and placed by `place` (its units to the world).
fn parts(model: &Model, pose: &Pose, place: &Mat4, view: &Mat4) -> Vec<Part> {
    let mut out = Vec::new();
    for (node, mesh) in model.shown(pose) {
        let weights = &pose.weights[node];
        for prim in &model.meshes[mesh].primitives {
            // skinned: the joints place it; else its node does
            let joints = model.nodes[node].skin.filter(|_| !prim.joints.is_empty()).map(|s| model.joints(s, pose));
            let node_place = *place * pose.world[node];
            let m = &node_place.0;
            let flip = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]) < 0.0;
            let place_n = normals_of(&node_place);
            let verts: Vec<Vertex> = (0..prim.positions.len())
                .map(|i| {
                    let mut p = prim.positions[i].map(|v| v as f64);
                    let mut n = prim.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]).map(|v| v as f64);
                    // morph targets
                    for (k, t) in prim.targets.iter().enumerate() {
                        let w = weights.get(k).copied().unwrap_or(0.0) as f64;
                        if w == 0.0 {
                            continue;
                        }
                        if let Some(d) = t.positions.get(i) {
                            (0..3).for_each(|c| p[c] += w * d[c] as f64);
                        }
                        if let Some(d) = t.normals.get(i) {
                            (0..3).for_each(|c| n[c] += w * d[c] as f64);
                        }
                    }
                    let (pv, nv) = (vec3(p[0], p[1], p[2]), vec3(n[0], n[1], n[2]));
                    let tan = prim.tangents.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 1.0]);
                    let tv = vec3(tan[0] as f64, tan[1] as f64, tan[2] as f64);
                    let (world, normal, tangent) = match &joints {
                        Some(js) => {
                            // bent by its joints, each as much as its weight
                            let (jn, jw) = (prim.joints[i], prim.weights[i]);
                            let mut m = [[0.0; 4]; 4];
                            for k in 0..4 {
                                if let Some(j) = js.get(jn[k] as usize) {
                                    for (r, row) in m.iter_mut().enumerate() {
                                        for (c, v) in row.iter_mut().enumerate() {
                                            *v += jw[k] as f64 * j.0[r][c];
                                        }
                                    }
                                }
                            }
                            let skin = *place * Mat4(m);
                            let sn = normals_of(&skin);
                            (skin.apply(pv), sn.apply_vec(nv), skin.apply_vec(tv))
                        }
                        None => (node_place.apply(pv), place_n.apply_vec(nv), node_place.apply_vec(tv)),
                    };
                    let normal = if normal.length() > 1e-12 { normal.normalize() } else { vec3(0.0, 0.0, -1.0) };
                    let tangent = if tangent.length() > 1e-12 { tangent.normalize() } else { vec3(1.0, 0.0, 0.0) };
                    Vertex {
                        cam: view.apply(world),
                        world,
                        normal,
                        tangent: [tangent.x, tangent.y, tangent.z, tan[3] as f64 * if flip { -1.0 } else { 1.0 }],
                        uv: std::array::from_fn(|s| prim.uvs[s].get(i).map_or([0.0; 2], |u| [u[0] as f64, u[1] as f64])),
                        color: prim.colors.get(i).copied().unwrap_or([1.0; 4]),
                    }
                })
                .collect();
            let corners = prim.indices.as_chunks::<3>().0.iter().map(|t| if flip { [t[0], t[2], t[1]] } else { *t }).collect();
            let both = prim.material.and_then(|m| model.materials.get(m)).is_some_and(|m| m.two_sided);
            out.push(Part { verts, corners, material: prim.material, both });
        }
    }
    out
}

/// The model's vertices posed and placed, as triangles to fill on the canvas.
fn triangles(look: &Look) -> Vec<Tri> {
    let lens = &look.onto.0;
    let project = |c: Vec3| -> [f64; 2] {
        let x = lens[0][0] * c.x + lens[0][1] * c.y + lens[0][2] * c.z;
        let y = lens[1][0] * c.x + lens[1][1] * c.y + lens[1][2] * c.z;
        let w = lens[2][0] * c.x + lens[2][1] * c.y + lens[2][2] * c.z;
        [x / w, y / w]
    };
    let mut out = Vec::new();
    for part in parts(look.model, &look.pose, &look.place, &look.cam.view) {
        for t in &part.corners {
            let corners = [part.verts[t[0] as usize], part.verts[t[1] as usize], part.verts[t[2] as usize]];
            // facing away (seen from behind): left out unless shown from both sides
            let (a, b, c) = (corners[0].cam, corners[1].cam, corners[2].cam);
            if !part.both && (b - a).cross(c - a).dot(a) >= 0.0 {
                continue;
            }
            // cut where it comes nearer than the camera sees
            let mut poly: Vec<Vertex> = Vec::with_capacity(4);
            for k in 0..3 {
                let (p, q) = (&corners[k], &corners[(k + 1) % 3]);
                let (pin, qin) = (p.cam.z >= NEAR, q.cam.z >= NEAR);
                if pin {
                    poly.push(*p);
                }
                if pin != qin {
                    poly.push(Vertex::mix(p, q, (NEAR - p.cam.z) / (q.cam.z - p.cam.z)));
                }
            }
            for k in 1..poly.len().saturating_sub(1) {
                let v = [poly[0], poly[k], poly[k + 1]];
                out.push(Tri { screen: v.map(|x| project(x.cam)), inv_z: v.map(|x| 1.0 / x.cam.z), v, material: part.material });
            }
        }
    }
    out
}

/// A model's triangles in the world, arranged in a tree of boxes so a ray finds those it may
/// meet quickly: what a model's shadow is found from.
pub(crate) struct Caster {
    tris: Vec<[Vec3; 3]>,
    /// Boxes: lowest and highest corners, and either the two below (`count` 0) or the run of
    /// triangles from `first`.
    nodes: Vec<(Vec3, Vec3, u32, u32, u32)>,
}

/// The box round triangles `ts`.
fn boxed(ts: &[[Vec3; 3]]) -> (Vec3, Vec3) {
    let mut lo = vec3(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut hi = vec3(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for t in ts {
        for p in t {
            lo = vec3(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = vec3(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
    }
    (lo, hi)
}

impl Caster {
    /// The model posed and placed so (both sides of every triangle cast).
    pub fn new(model: &Model, pose: &Pose, place: &Mat4) -> Caster {
        let mut tris: Vec<[Vec3; 3]> = parts(model, pose, place, &Mat4::IDENTITY).iter().flat_map(|p| p.corners.iter().map(|t| [p.verts[t[0] as usize].world, p.verts[t[1] as usize].world, p.verts[t[2] as usize].world]).collect::<Vec<_>>()).collect();
        let mut nodes = Vec::new();
        let n = tris.len();
        Caster::split(&mut tris, 0, n, &mut nodes);
        Caster { tris, nodes }
    }

    /// Boxes for triangles `first..first + count` (sorted in place), returning the top one's index.
    fn split(tris: &mut [[Vec3; 3]], first: usize, count: usize, nodes: &mut Vec<(Vec3, Vec3, u32, u32, u32)>) -> u32 {
        let (lo, hi) = boxed(&tris[first..first + count]);
        let at = nodes.len() as u32;
        if count <= 4 {
            nodes.push((lo, hi, 0, first as u32, count as u32));
            return at;
        }
        nodes.push((lo, hi, 0, 0, 0));
        // halved along the box's longest side, by the triangles' middles
        let size = hi - lo;
        let axis = if size.x >= size.y && size.x >= size.z { 0 } else if size.y >= size.z { 1 } else { 2 };
        let key = |t: &[Vec3; 3]| {
            let c = t[0] + t[1] + t[2];
            [c.x, c.y, c.z][axis]
        };
        tris[first..first + count].sort_by(|a, b| key(a).total_cmp(&key(b)));
        let half = count / 2;
        let left = Caster::split(tris, first, half, nodes);
        let right = Caster::split(tris, first + half, count - half, nodes);
        nodes[at as usize] = (lo, hi, left, right, 0);
        at
    }

    /// Whether the ray from `from` along unit `dir` meets a triangle between `near` and `far`.
    pub fn blocks(&self, from: Vec3, dir: Vec3, near: f64, far: f64) -> bool {
        if self.nodes.is_empty() {
            return false;
        }
        let inv = vec3(1.0 / dir.x, 1.0 / dir.y, 1.0 / dir.z);
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            let (lo, hi, a, b, count) = self.nodes[i as usize];
            // the slabs of the box
            let (mut t0, mut t1) = (near, far);
            for (o, d, l, h) in [(from.x, inv.x, lo.x, hi.x), (from.y, inv.y, lo.y, hi.y), (from.z, inv.z, lo.z, hi.z)] {
                let (mut ta, mut tb) = ((l - o) * d, (h - o) * d);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
            }
            if t0 > t1 {
                continue;
            }
            if count == 0 {
                stack.extend([a, b]);
                continue;
            }
            for t in &self.tris[b as usize..(b + count) as usize] {
                // Möller–Trumbore
                let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
                let p = dir.cross(e2);
                let det = e1.dot(p);
                if det.abs() < 1e-12 {
                    continue;
                }
                let s = from - t[0];
                let u = s.dot(p) / det;
                if !(0.0..=1.0).contains(&u) {
                    continue;
                }
                let q = s.cross(e1);
                let v = dir.dot(q) / det;
                if v < 0.0 || u + v > 1.0 {
                    continue;
                }
                let d = e2.dot(q) / det;
                if d > near && d < far {
                    return true;
                }
            }
        }
        false
    }
}

/// A picture of the model at texture point set `map.set` of `uv`.
fn sample(model: &Model, map: Option<Map>, uv: &[[f64; 2]; 2]) -> Option<[f32; 4]> {
    let m = map?;
    let t = model.textures.get(m.texture)?;
    let p = uv[(m.set as usize).min(1)];
    Some(t.at(p[0], p[1]))
}

/// What a point of a triangle (at weights `b` of its corners) looks like, lit: its colour,
/// premultiplied.
fn shade(look: &Look, tri: &Tri, b: [f64; 3]) -> Px {
    let mix3 = |f: &dyn Fn(&Vertex) -> Vec3| f(&tri.v[0]) * b[0] + f(&tri.v[1]) * b[1] + f(&tri.v[2]) * b[2];
    let world = mix3(&|v| v.world);
    let mut n = mix3(&|v| v.normal);
    n = if n.length() > 1e-12 { n.normalize() } else { vec3(0.0, 0.0, -1.0) };
    let uv: [[f64; 2]; 2] = std::array::from_fn(|s| [0, 1].map(|c| tri.v.iter().zip(b).map(|(v, w)| v.uv[s][c] * w).sum::<f64>()));
    let vcolor: [f32; 4] = std::array::from_fn(|c| tri.v.iter().zip(b).map(|(v, w)| v.color[c] * w as f32).sum());
    let model = look.model;
    let fallback = veymelo_model::Material::default();
    let m = tri.material.and_then(|i| model.materials.get(i)).unwrap_or(&fallback);
    // the colour in the working space: linear factors and vertex colours, an sRGB picture
    let to_work = |linear: [f32; 3]| if look.linear { linear } else { linear.map(|v| linear_to_srgb(v.max(0.0) as f64) as f32) };
    let pic_to_work = |srgb: [f32; 3]| if look.linear { srgb.map(|v| srgb_to_linear(v.max(0.0) as f64) as f32) } else { srgb };
    let factor = to_work([m.base[0] * vcolor[0], m.base[1] * vcolor[1], m.base[2] * vcolor[2]]);
    let tex = sample(model, m.base_map, &uv).unwrap_or([1.0; 4]);
    let pic = pic_to_work([tex[0], tex[1], tex[2]]);
    let rgb = [factor[0] * pic[0], factor[1] * pic[1], factor[2] * pic[2]];
    let alpha = match m.alpha {
        AlphaMode::Opaque => 1.0,
        _ => (m.base[3] * vcolor[3] * tex[3]).clamp(0.0, 1.0),
    };
    let emissive = {
        let e = sample(model, m.emissive_map, &uv).map_or([1.0; 3], |t| pic_to_work([t[0], t[1], t[2]]));
        let k = to_work(m.emissive);
        [k[0] * e[0], k[1] * e[1], k[2] * e[2]]
    };
    if m.unlit {
        return [rgb[0] * alpha, rgb[1] * alpha, rgb[2] * alpha, alpha];
    }
    // the normal map turns the normal within the surface
    if let Some(t) = sample(model, m.normal_map, &uv) {
        let tan = [0, 1, 2, 3].map(|c| tri.v.iter().zip(b).map(|(v, w)| v.tangent[c] * w).sum::<f64>());
        let tv = vec3(tan[0], tan[1], tan[2]);
        let tv = (tv - n * tv.dot(n)).normalize();
        let bv = n.cross(tv) * if tan[3] < 0.0 { -1.0 } else { 1.0 };
        let s = m.normal_scale as f64;
        let (x, y, z) = ((t[0] as f64 * 2.0 - 1.0) * s, (t[1] as f64 * 2.0 - 1.0) * s, t[2] as f64 * 2.0 - 1.0);
        let turned = tv * x + bv * y + n * z;
        if turned.length() > 1e-9 {
            n = turned.normalize();
        }
    }
    let mr = sample(model, m.metal_rough_map, &uv).unwrap_or([1.0; 4]);
    let (metal, rough) = ((m.metallic * mr[2]).clamp(0.0, 1.0), (m.roughness * mr[1]).clamp(0.02, 1.0));
    let occlusion = sample(model, m.occlusion_map, &uv).map_or(1.0, |o| 1.0 + m.occlusion * (o[0] - 1.0));
    // the metal-roughness material as the layer's light-taking one
    let mut mat = look.material;
    mat.diffuse *= 1.0 - 0.9 * metal;
    mat.ambient *= occlusion;
    mat.specular *= (1.0 - rough) * (1.0 - rough) * 2.0;
    mat.shininess = (1.0 - rough).clamp(0.0, 1.0);
    mat.metal = metal;
    let viewer = (look.cam.eye - world).normalize();
    let (mut lit, _) = if look.lights.is_empty() && look.env.is_some() { ([0.0; 3], 0.0) } else { crate::scene::shade(rgb, world, n, viewer, &mat, look.lights, look.unshadowed) };
    // the environment's light, as its metal and roughness take it
    if let Some(env) = look.env.filter(|_| look.material.lit) {
        let add = env.light(rgb, n, viewer, metal, rough, look.material.diffuse * 2.0, look.material.specular * 2.0, occlusion);
        (0..3).for_each(|k| lit[k] += add[k]);
    }
    let c = [lit[0] + emissive[0], lit[1] + emissive[1], lit[2] + emissive[2]];
    [c[0] * alpha, c[1] * alpha, c[2] * alpha, alpha]
}

/// Rows of the canvas filled together.
const BAND: usize = 32;

/// The model drawn as `look` says.
pub(crate) fn shoot(look: &Look) -> Option<Shot> {
    let tris = triangles(look);
    if tris.is_empty() {
        return None;
    }
    // the canvas pixels the triangles reach
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for t in &tris {
        for p in &t.screen {
            (x0, y0, x1, y1) = (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]));
        }
    }
    let (x0, y0) = (x0.floor().max(0.0) as usize, y0.floor().max(0.0) as usize);
    let (x1, y1) = ((x1.ceil().max(0.0) as usize + 1).min(look.width), (y1.ceil().max(0.0) as usize + 1).min(look.height));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (w, h) = (x1 - x0, y1 - y0);
    // each band of rows: the triangles that reach it
    let bands = h.div_ceil(BAND);
    let mut binned: Vec<Vec<usize>> = vec![Vec::new(); bands];
    for (i, t) in tris.iter().enumerate() {
        let (lo, hi) = t.screen.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p[1]), b.max(p[1])));
        let (b0, b1) = (((lo.floor().max(y0 as f64) as usize).saturating_sub(y0)) / BAND, (((hi.ceil().max(0.0) as usize).min(y1 - 1)).saturating_sub(y0)) / BAND);
        for list in binned.iter_mut().take((b1 + 1).min(bands)).skip(b0) {
            list.push(i);
        }
    }
    let masked: Vec<Option<f32>> = (0..look.model.materials.len().max(1)).map(|i| look.model.materials.get(i).and_then(|m| if let AlphaMode::Mask(c) = m.alpha { Some(c) } else { None })).collect();
    let mut px = vec![(f64::INFINITY, [0.0f32; 4], Facts::default()); w * h];
    let tris = Arc::new(tris);
    par::chunks(&mut px, BAND * w, |band, out| {
        let rows = out.len() / w;
        let top = y0 + band * BAND;
        // two samples across and down a pixel: the nearest triangle at each and its depth
        let (sw, sh) = (2 * w, 2 * rows);
        let mut depth = vec![f64::INFINITY; sw * sh];
        let mut who = vec![u32::MAX; sw * sh];
        for &i in &binned[band] {
            let t = &tris[i];
            let s = &t.screen;
            let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
            if area.abs() < 1e-12 {
                continue;
            }
            let (lo_x, hi_x) = (s.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min), s.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max));
            let (lo_y, hi_y) = (s.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min), s.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max));
            // the samples it may cover (sample (i, j) of the band at canvas (x0 + (i + 0.5) / 2, top + (j + 0.5) / 2))
            let si0 = (((lo_x - x0 as f64) * 2.0 - 0.5).floor().max(0.0) as usize).min(sw);
            let si1 = ((((hi_x - x0 as f64) * 2.0 - 0.5).ceil() + 1.0).max(0.0) as usize).min(sw);
            let sj0 = (((lo_y - top as f64) * 2.0 - 0.5).floor().max(0.0) as usize).min(sh);
            let sj1 = ((((hi_y - top as f64) * 2.0 - 0.5).ceil() + 1.0).max(0.0) as usize).min(sh);
            let cutoff = t.material.and_then(|m| masked.get(m).copied().flatten());
            for j in sj0..sj1 {
                let py = top as f64 + (j as f64 + 0.5) / 2.0;
                for k in si0..si1 {
                    let pxx = x0 as f64 + (k as f64 + 0.5) / 2.0;
                    let e = |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (pxx - a[0]);
                    let (w0, w1, w2) = (e(s[1], s[2]) / area, e(s[2], s[0]) / area, e(s[0], s[1]) / area);
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let iz = w0 * t.inv_z[0] + w1 * t.inv_z[1] + w2 * t.inv_z[2];
                    if iz <= 0.0 {
                        continue;
                    }
                    let z = 1.0 / iz;
                    let at = j * sw + k;
                    if z >= depth[at] {
                        continue;
                    }
                    if let Some(c) = cutoff {
                        let b = [w0 * t.inv_z[0] / iz, w1 * t.inv_z[1] / iz, w2 * t.inv_z[2] / iz];
                        let m = &look.model.materials[t.material.unwrap_or(0)];
                        let uv: [[f64; 2]; 2] = std::array::from_fn(|s| [0, 1].map(|c| t.v.iter().zip(b).map(|(v, w)| v.uv[s][c] * w).sum::<f64>()));
                        let a = m.base[3] * sample(look.model, m.base_map, &uv).map_or(1.0, |p| p[3]);
                        if a < c {
                            continue;
                        }
                    }
                    depth[at] = z;
                    who[at] = i as u32;
                }
            }
        }
        // each pixel: its four samples, each triangle's colour worked out once
        for r in 0..rows {
            for x in 0..w {
                let mut sum = [0f32; 4];
                let mut near = f64::INFINITY;
                let mut nearest = u32::MAX;
                let mut done: [(u32, Px); 4] = [(u32::MAX, [0.0; 4]); 4];
                for (n, (dj, di)) in [(0, 0), (0, 1), (1, 0), (1, 1)].into_iter().enumerate() {
                    let at = (2 * r + dj) * sw + 2 * x + di;
                    let id = who[at];
                    if id == u32::MAX {
                        continue;
                    }
                    if depth[at] < near {
                        (near, nearest) = (depth[at], id);
                    }
                    let color = match done.iter().find(|d| d.0 == id) {
                        Some(d) => d.1,
                        None => {
                            let t = &tris[id as usize];
                            let c = shade(look, t, inside(t, x0 as f64 + x as f64 + 0.5, top as f64 + r as f64 + 0.5));
                            done[n] = (id, c);
                            c
                        }
                    };
                    (0..4).for_each(|c| sum[c] += color[c] * 0.25);
                }
                let facts = match tris.get(nearest as usize) {
                    Some(t) => surface(t, inside(t, x0 as f64 + x as f64 + 0.5, top as f64 + r as f64 + 0.5)),
                    None => Facts::default(),
                };
                out[r * w + x] = (near, sum, facts);
            }
        }
    });
    let facts = px.iter().map(|p| p.2).collect();
    let px = px.into_iter().map(|p| (p.0, p.1)).collect();
    Some(Shot { x0, y0, w, h, px, facts })
}

/// A triangle's weights at canvas point (x, y), held inside it, as perspective has them.
fn inside(t: &Tri, x: f64, y: f64) -> [f64; 3] {
    let s = &t.screen;
    let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
    let e = |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
    let raw = [e(s[1], s[2]) / area, e(s[2], s[0]) / area, e(s[0], s[1]) / area].map(|v| v.max(0.0));
    let total = (raw[0] + raw[1] + raw[2]).max(1e-12);
    let sb = raw.map(|v| v / total);
    let iz = sb[0] * t.inv_z[0] + sb[1] * t.inv_z[1] + sb[2] * t.inv_z[2];
    [sb[0] * t.inv_z[0] / iz, sb[1] * t.inv_z[1] / iz, sb[2] * t.inv_z[2] / iz]
}

/// A triangle's surface at weights `b`: its normal, texture point and material.
fn surface(t: &Tri, b: [f64; 3]) -> Facts {
    let n = t.v[0].normal * b[0] + t.v[1].normal * b[1] + t.v[2].normal * b[2];
    let n = if n.length() > 1e-12 { n.normalize() } else { vec3(0.0, 0.0, -1.0) };
    let uv = [0, 1].map(|c| t.v.iter().zip(b).map(|(v, w)| v.uv[0][c] * w).sum::<f64>() as f32);
    Facts { normal: [n.x as f32, n.y as f32, n.z as f32], uv, material: t.material.map_or(u32::MAX, |m| m as u32) }
}

/// The lights a model is lit by where the composition has none: a key light from above and
/// to the left of the camera, and a soft fill (else a model would show flat, its colours alone).
pub(crate) fn standard_lights(cam: &Camera) -> Vec<Light> {
    let back = cam.view.inverse().unwrap_or(Mat4::IDENTITY);
    let key = back.apply_vec(vec3(0.45, 0.6, 1.0)).normalize();
    // how strong (a scale on the colour, in whichever space colours are worked in)
    let light = |kind, dir: Vec3, c: f32| Light {
        kind,
        at: cam.eye,
        dir,
        color: [c; 3],
        cone: std::f64::consts::PI,
        feather: 0.0,
        falloff: 0,
        radius: 0.0,
        falloff_distance: 0.0,
        shadows: false,
        darkness: 0.0,
        diffusion: 0.0,
    };
    vec![light(veymelo_project::LightKind::Parallel, key, 1.0), light(veymelo_project::LightKind::Ambient, key, 0.45)]
}

/// A drawn model blurred as the camera sees it out of focus: each pixel by how far its depth is
/// from the focus distance (`k` canvas pixels to a composition pixel), the blur spreading past
/// the model's edges on a canvas `width` × `height`.
pub(crate) fn defocus(shot: Shot, focus: &crate::scene::Focus, k: f64, width: usize, height: usize) -> Shot {
    let spread = |z: f64| if z.is_finite() { focus.spread(z) * k } else { 0.0 };
    let most = shot.px.iter().map(|p| spread(p.0)).fold(0.0f64, f64::max).min(400.0);
    if most < 0.5 {
        return shot;
    }
    // room to spread into
    let pad = (most / 2.0).ceil() as usize + 2;
    let (x0, y0) = (shot.x0.saturating_sub(pad), shot.y0.saturating_sub(pad));
    let (x1, y1) = ((shot.x0 + shot.w + pad).min(width), (shot.y0 + shot.h + pad).min(height));
    let (w, h) = (x1 - x0, y1 - y0);
    let mut img = veymelo_raster::Image::new(w, h);
    let mut depth = vec![f64::INFINITY; w * h];
    let mut facts = vec![Facts::default(); w * h];
    for y in 0..shot.h {
        for x in 0..shot.w {
            let (p, i) = (shot.px[y * shot.w + x], (y + shot.y0 - y0) * w + x + shot.x0 - x0);
            img.px[i] = p.1;
            depth[i] = p.0;
            facts[i] = shot.facts[y * shot.w + x];
        }
    }
    // where nothing is, the nearest depth round it (so a blurred edge spreads by its own)
    let r = pad as isize;
    let near_across: Vec<f64> = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as isize, i / w);
            (-r..=r).filter_map(|d| usize::try_from(x + d).ok().filter(|&xx| xx < w).map(|xx| depth[y * w + xx])).fold(f64::INFINITY, f64::min)
        })
        .collect();
    let filled: Vec<f64> = (0..w * h)
        .map(|i| {
            if depth[i].is_finite() {
                return depth[i];
            }
            let (x, y) = (i % w, (i / w) as isize);
            (-r..=r).filter_map(|d| usize::try_from(y + d).ok().filter(|&yy| yy < h).map(|yy| near_across[yy * w + x])).fold(f64::INFINITY, f64::min)
        })
        .collect();
    let levels = ((most / 2.0).ceil() as usize + 1).clamp(2, 8);
    let blurs: Vec<veymelo_raster::Image> = (0..levels)
        .map(|n| {
            let d = most * n as f64 / (levels - 1) as f64;
            if d < 0.5 {
                img.clone()
            } else if focus.sides == 0 {
                veymelo_raster::filter::boxes(&img, d, d * focus.aspect.min(4.0), 1, veymelo_raster::filter::Edge::Clear)
            } else {
                veymelo_raster::filter::boxes(&img, d / 3f64.sqrt(), d / 3f64.sqrt() * focus.aspect.min(4.0), 3, veymelo_raster::filter::Edge::Clear)
            }
        })
        .collect();
    let px = (0..w * h)
        .map(|i| {
            let f = (spread(filled[i]).min(most) / most * (levels - 1) as f64).clamp(0.0, (levels - 1) as f64);
            let n = (f.floor() as usize).min(levels - 2);
            let t = (f - n as f64) as f32;
            let (a, b) = (blurs[n].px[i], blurs[n + 1].px[i]);
            (filled[i], std::array::from_fn(|c| a[c] + (b[c] - a[c]) * t))
        })
        .collect();
    Shot { x0, y0, w, h, px, facts }
}
