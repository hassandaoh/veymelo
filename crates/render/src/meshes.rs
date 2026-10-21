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
/// world, its texture points and colour, and the light bouncing onto it.
#[derive(Clone, Copy, Debug)]
struct Vertex {
    cam: Vec3,
    world: Vec3,
    normal: Vec3,
    tangent: [f64; 4],
    uv: [[f64; 2]; 2],
    color: [f32; 4],
    bounce: [f32; 3],
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
            bounce: std::array::from_fn(|i| a.bounce[i] + (b.bounce[i] - a.bounce[i]) * t as f32),
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

/// A model's triangles to fill: the corners placed (each on the canvas too, and 1 / its depth),
/// and each triangle's three, and its material.
struct Placed {
    verts: Vec<Vertex>,
    screen: Vec<[f64; 2]>,
    inv_z: Vec<f64>,
    tris: Vec<([u32; 3], Option<usize>)>,
}

impl Placed {
    fn tri(&self, i: usize) -> Tri {
        let (c, material) = self.tris[i];
        let c = c.map(|k| k as usize);
        Tri { v: c.map(|k| self.verts[k]), screen: c.map(|k| self.screen[k]), inv_z: c.map(|k| self.inv_z[k]), material }
    }
}

/// What a surface at world point `q` facing `n` mirrors seen from `viewer`, as sharply as it
/// says (`Look::mirror`).
pub(crate) type Mirror<'a> = &'a (dyn Fn(Vec3, Vec3, Vec3, f32, [f32; 3]) -> [f32; 3] + Sync);

/// What is seen through glass at world point `q` on a surface facing `n` from `viewer`
/// (`Look::through`).
pub(crate) type Through<'a> = &'a (dyn Fn(Vec3, Vec3, Vec3, f32) -> Option<[f32; 3]> + Sync);

/// What a model is drawn with.
pub(crate) struct Look<'a> {
    pub model: &'a Arc<Model>,
    pub pose: &'a Pose,
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
    /// Whether a shadow can fall on it (else `unshadowed` lets every light through).
    pub shadowed: bool,
    /// What casts shadows, as the GPU reads it.
    pub shadows: Option<&'a crate::gpu::Shadows>,
    /// How much of the sky round world point `q` (on a surface facing `n`) is open, as far as
    /// the ambient occlusion looks (none: no light occludes), and how far that is.
    pub open: Option<&'a (dyn Fn(Vec3, Vec3) -> f32 + Sync)>,
    pub reach: Option<f64>,
    /// What a surface at world point `q` facing `n` mirrors seen from `viewer`, as sharply as
    /// it says, `none` where it meets nothing (none: nothing mirrors).
    pub mirror: Option<Mirror<'a>>,
    /// The light bouncing onto each of its vertices (in `probes`' order), and how much of it
    /// is taken (none: light does not bounce).
    pub bounce: Option<(&'a [[f32; 3]], f32)>,
    /// The light bouncing onto every point it was gathered at in the scene, and how much of it
    /// surfaces take (for what is seen in mirrors and through glass).
    pub scene_light: (&'a [[f32; 3]], f32),
    /// What is seen through it at world point `q` on a surface facing `n` from `viewer`, light
    /// bending by index of refraction `ior` going into it and out again: none where nothing is
    /// met past it (and there is no environment), or nothing can be seen through.
    pub through: Option<Through<'a>>,
    /// Drawn on the GPU where it can be.
    pub gpu: bool,
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
                        bounce: [0.0; 3],
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
fn triangles(look: &Look) -> Placed {
    let lens = &look.onto.0;
    let project = |c: Vec3| -> [f64; 2] {
        let x = lens[0][0] * c.x + lens[0][1] * c.y + lens[0][2] * c.z;
        let y = lens[1][0] * c.x + lens[1][1] * c.y + lens[1][2] * c.z;
        let w = lens[2][0] * c.x + lens[2][1] * c.y + lens[2][2] * c.z;
        [x / w, y / w]
    };
    let mut out = Placed { verts: Vec::new(), screen: Vec::new(), inv_z: Vec::new(), tris: Vec::new() };
    let add = |out: &mut Placed, v: Vertex| -> u32 {
        out.screen.push(project(v.cam));
        out.inv_z.push(1.0 / v.cam.z);
        out.verts.push(v);
        (out.verts.len() - 1) as u32
    };
    let mut probe = 0;
    for part in parts(look.model, look.pose, &look.place, &look.cam.view) {
        let first = out.verts.len() as u32;
        for v in &part.verts {
            let bounce = look.bounce.and_then(|(light, _)| light.get(probe).copied()).unwrap_or([0.0; 3]);
            add(&mut out, Vertex { bounce, ..*v });
            probe += 1;
        }
        for t in &part.corners {
            let t = t.map(|k| first + k);
            let corners = t.map(|k| out.verts[k as usize]);
            // facing away (seen from behind): left out unless shown from both sides
            let (a, b, c) = (corners[0].cam, corners[1].cam, corners[2].cam);
            if !part.both && (b - a).cross(c - a).dot(a) >= 0.0 {
                continue;
            }
            // cut where it comes nearer than the camera sees
            let mut poly: Vec<u32> = Vec::with_capacity(4);
            for k in 0..3 {
                let (p, q) = (&corners[k], &corners[(k + 1) % 3]);
                let (pin, qin) = (p.cam.z >= NEAR, q.cam.z >= NEAR);
                if pin {
                    poly.push(t[k]);
                }
                if pin != qin {
                    let cut = add(&mut out, Vertex::mix(p, q, (NEAR - p.cam.z) / (q.cam.z - p.cam.z)));
                    poly.push(cut);
                }
            }
            for k in 1..poly.len().saturating_sub(1) {
                out.tris.push(([poly[0], poly[k], poly[k + 1]], part.material));
            }
        }
    }
    out
}

/// A model's triangles in the world, arranged in a tree of boxes so a ray finds those it may
/// meet quickly: what a model's shadow is found from.
pub(crate) struct Caster {
    /// Each triangle, and which it is in the model's order (`corners`, `faces`).
    tris: Vec<([Vec3; 3], u32)>,
    /// Boxes: lowest and highest corners, and either the two below (`count` 0) or the run of
    /// triangles from `first`.
    nodes: Vec<(Vec3, Vec3, u32, u32, u32)>,
}

/// The box round triangles `ts`.
fn boxed(ts: &[([Vec3; 3], u32)]) -> (Vec3, Vec3) {
    let mut lo = vec3(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut hi = vec3(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (t, _) in ts {
        for p in t {
            lo = vec3(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = vec3(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
    }
    (lo, hi)
}

/// The model posed (`pose`) and placed by `place`: its triangles in the world, three corners
/// each (both sides of every triangle cast, as a `Caster`'s).
pub(crate) fn corners(model: &Model, pose: &Pose, place: &Mat4) -> Vec<[f32; 3]> {
    let f = |v: Vec3| [v.x as f32, v.y as f32, v.z as f32];
    parts(model, pose, place, &Mat4::IDENTITY).iter().flat_map(|p| p.corners.iter().flat_map(|t| t.map(|k| f(p.verts[k as usize].world)))).collect()
}

/// A triangle of a model as a reflection sees it: its corners' normals (in the world; and the
/// layer's ambient, diffuse and whether it takes light, in their w) and colours (in the working
/// space, its picture apart; and how much its layer mirrors, how much less face on, and how
/// see-through it is, in their w), the light it gives off (and how much less clear its glass is
/// face on, in its w), the points the light bouncing onto its corners was gathered at and its
/// picture among the scene's (`u32::MAX`: none), and its corners' points on that picture (u, v
/// each; then whether colours are worked in linear light).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Face {
    pub normals: [[f32; 4]; 3],
    pub colours: [[f32; 4]; 3],
    pub glow: [f32; 4],
    pub probes: [u32; 4],
    pub uvs: [[f32; 4]; 2],
}

impl Face {
    /// Its picture's colour at its corners' weights `w` (in the working space), if it has one
    /// (`pics`: the scene's pictures, as `faces` numbers them).
    pub fn picture(&self, w: [f64; 3], pics: &[&veymelo_model::Texture]) -> Option<[f32; 3]> {
        let t = pics.get(self.probes[3] as usize)?;
        let u = self.uvs[0][0] as f64 * w[0] + self.uvs[0][2] as f64 * w[1] + self.uvs[1][0] as f64 * w[2];
        let v = self.uvs[0][1] as f64 * w[0] + self.uvs[0][3] as f64 * w[1] + self.uvs[1][1] as f64 * w[2];
        let c = t.at(u, v);
        let linear = self.uvs[1][2] > 0.5;
        Some(std::array::from_fn(|k| if linear { srgb_to_linear(c[k].max(0.0) as f64) as f32 } else { c[k] }))
    }
}

/// The pictures a scene's model faces read (each model's colour pictures, model after model),
/// and for each model, its pictures' numbers among them (`u32::MAX`: not one of them).
pub(crate) fn face_pictures<'m>(models: &[&'m Model]) -> (Vec<&'m veymelo_model::Texture>, Vec<Vec<u32>>) {
    let mut list = Vec::new();
    let numbers = models
        .iter()
        .map(|model| {
            let mut of = vec![u32::MAX; model.textures.len()];
            for m in &model.materials {
                if let Some(map) = m.base_map
                    && map.texture < of.len()
                    && of[map.texture] == u32::MAX
                {
                    of[map.texture] = list.len() as u32;
                    list.push(&model.textures[map.texture]);
                }
            }
            of
        })
        .collect();
    (list, numbers)
}

/// The model posed (`pose`) and placed by `place`: each of its vertices (in `parts`' order, as
/// `triangles` takes them), where it is and which way it faces in the world.
pub(crate) fn probes(model: &Model, pose: &Pose, place: &Mat4) -> Vec<(Vec3, Vec3)> {
    parts(model, pose, place, &Mat4::IDENTITY).iter().flat_map(|p| p.verts.iter().map(|v| (v.world, v.normal)).collect::<Vec<_>>()).collect()
}

/// The model posed (`pose`) and placed by `place`, as a reflection sees each of its triangles (in
/// `corners`' order), its layer taking light as `m` says, colours worked in linear light (else
/// as they are seen); its vertices' light gathered at the points from `first_probe` (`probes`'
/// order), if it was.
pub(crate) fn faces(model: &Model, pose: &Pose, place: &Mat4, m: &Material, linear: bool, first_probe: Option<usize>, pictures: &[u32]) -> Vec<Face> {
    let to_work = |c: f32| if linear { c } else { linear_to_srgb(c.max(0.0) as f64) as f32 };
    let fallback = veymelo_model::Material::default();
    let mut out = Vec::new();
    let mut part_first = 0;
    for p in parts(model, pose, place, &Mat4::IDENTITY) {
        let mat = p.material.and_then(|i| model.materials.get(i)).unwrap_or(&fallback);
        for t in &p.corners {
            let corner = |k: u32| p.verts[k as usize];
            let normals = [0, 1, 2].map(|i| {
                let n = corner(t[i]).normal;
                [n.x as f32, n.y as f32, n.z as f32, [m.ambient, m.diffuse, m.lit as u32 as f32][i]]
            });
            let colours = [0, 1, 2].map(|i| {
                let v = corner(t[i]);
                let c: [f32; 3] = std::array::from_fn(|k| to_work(mat.base[k] * v.color[k]));
                [c[0], c[1], c[2], [m.reflection, m.rolloff, m.transparency][i]]
            });
            // its colour picture, and its corners' points on it
            let picture = mat.base_map.filter(|b| b.texture < model.textures.len()).map_or(u32::MAX, |b| pictures.get(b.texture).copied().unwrap_or(u32::MAX));
            let set = mat.base_map.map_or(0, |b| (b.set as usize).min(1));
            let uv = |i: usize| corner(t[i]).uv[set].map(|c| c as f32);
            let uvs = [[uv(0)[0], uv(0)[1], uv(1)[0], uv(1)[1]], [uv(2)[0], uv(2)[1], linear as u32 as f32, 0.0]];
            let probes = [0, 1, 2, 3].map(|i| match first_probe {
                Some(f) if i < 3 => (f + part_first + t[i] as usize) as u32,
                _ if i == 3 => picture,
                _ => u32::MAX,
            });
            out.push(Face { normals, colours, glow: [to_work(mat.emissive[0]), to_work(mat.emissive[1]), to_work(mat.emissive[2]), m.clear_rolloff], probes, uvs });
        }
        part_first += p.verts.len();
    }
    out
}

impl Caster {
    /// The model posed and placed so (both sides of every triangle cast).
    pub fn new(model: &Model, pose: &Pose, place: &Mat4) -> Caster {
        let mut tris: Vec<([Vec3; 3], u32)> = parts(model, pose, place, &Mat4::IDENTITY).iter().flat_map(|p| p.corners.iter().map(|t| [p.verts[t[0] as usize].world, p.verts[t[1] as usize].world, p.verts[t[2] as usize].world]).collect::<Vec<_>>()).enumerate().map(|(i, t)| (t, i as u32)).collect();
        let mut nodes = Vec::new();
        let n = tris.len();
        Caster::split(&mut tris, 0, n, &mut nodes);
        Caster { tris, nodes }
    }

    /// Boxes for triangles `first..first + count` (sorted in place), returning the top one's index.
    fn split(tris: &mut [([Vec3; 3], u32)], first: usize, count: usize, nodes: &mut Vec<(Vec3, Vec3, u32, u32, u32)>) -> u32 {
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
        let key = |(t, _): &([Vec3; 3], u32)| {
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

    /// The box round all its triangles: its lowest and highest corners (none: it has none).
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        self.nodes.first().map(|n| (n.0, n.1))
    }

    /// Whether the ray from `from` along unit `dir` meets a triangle between `near` and `far`.
    pub fn blocks(&self, from: Vec3, dir: Vec3, near: f64, far: f64) -> bool {
        self.walk(from, dir, near, far, true).is_some()
    }

    /// The nearest triangle the ray from `from` along unit `dir` meets between `near` and
    /// `far`: how far, which (in the model's order), and where on it (its second and third
    /// corners' weights).
    pub fn nearest(&self, from: Vec3, dir: Vec3, near: f64, far: f64) -> Option<(f64, u32, f64, f64)> {
        self.walk(from, dir, near, far, false)
    }

    /// The triangles the ray meets: the first found (`any`), else the nearest.
    fn walk(&self, from: Vec3, dir: Vec3, near: f64, mut far: f64, any: bool) -> Option<(f64, u32, f64, f64)> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut best = None;
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
            for (t, which) in &self.tris[b as usize..(b + count) as usize] {
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
                    best = Some((d, *which, u, v));
                    if any {
                        return best;
                    }
                    far = d;
                }
            }
        }
        best
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
    // how open the sky round it is (on the side it is seen from)
    let open = match look.open {
        Some(f) if look.material.lit && look.material.accepts != 0 => f(world, if n.dot(viewer) < 0.0 { -n } else { n }),
        _ => 1.0,
    };
    // (lit by an environment alone: none but its light; one that takes no light is as it is)
    let (mut lit, _) = if look.lights.is_empty() && look.env.is_some() && look.material.lit { ([0.0; 3], 0.0) } else { crate::scene::shade(rgb, world, n, viewer, &mat, look.lights, look.unshadowed, open) };
    // the environment's light, as its metal and roughness take it
    if let Some(env) = look.env.filter(|_| look.material.lit) {
        let kept = 1.0 - env.occlusion * (1.0 - open);
        let add = env.light(rgb, n, viewer, metal, rough, look.material.diffuse * 2.0, look.material.specular * 2.0, occlusion * kept);
        (0..3).for_each(|k| lit[k] += add[k]);
    }
    // the light bouncing onto it from the surfaces round it
    if let Some((_, amount)) = look.bounce.filter(|_| look.material.lit) {
        let e: [f32; 3] = std::array::from_fn(|c| tri.v.iter().zip(b).map(|(v, w)| v.bounce[c] * w as f32).sum());
        let k = mat.diffuse * 2.0 * amount;
        (0..3).for_each(|c| lit[c] += rgb[c] * k * e[c]);
    }
    let mut c = [lit[0] + emissive[0], lit[1] + emissive[1], lit[2] + emissive[2]];
    // what it mirrors, mixed in
    if let Some(mirror) = look.mirror.filter(|_| look.material.reflection > 0.0) {
        let facing = if n.dot(viewer) < 0.0 { -n } else { n };
        let r = look.material.mirrors(facing, viewer);
        if r > 0.0 {
            let seen = mirror(world, facing, viewer, look.material.sharpness, c);
            c = std::array::from_fn(|k| c[k] * (1.0 - r) + seen[k] * r);
        }
    }
    // seen through, as glass: what is behind it bent and tinted by it (nothing met past it: what
    // is under it shows through)
    let mut alpha = alpha;
    if let Some(through) = look.through.filter(|_| look.material.transparency > 0.0) {
        let facing = if n.dot(viewer) < 0.0 { -n } else { n };
        let t = look.material.clear(facing, viewer);
        if t > 0.0 {
            match through(world, facing, viewer, look.material.ior) {
                Some(seen) => c = std::array::from_fn(|k| c[k] * (1.0 - t) + seen[k] * rgb[k] * t),
                None => alpha *= 1.0 - t,
            }
        }
    }
    [c[0] * alpha, c[1] * alpha, c[2] * alpha, alpha]
}

/// Rows of the canvas filled together.
const BAND: usize = 32;

/// The model drawn as `look` says.
pub(crate) fn shoot(look: &Look) -> Option<Shot> {
    let placed = triangles(look);
    if placed.tris.is_empty() {
        return None;
    }
    // the canvas pixels the triangles reach
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (c, _) in &placed.tris {
        for &k in c {
            let p = placed.screen[k as usize];
            (x0, y0, x1, y1) = (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]));
        }
    }
    let (x0, y0) = (x0.floor().max(0.0) as usize, y0.floor().max(0.0) as usize);
    let (x1, y1) = ((x1.ceil().max(0.0) as usize + 1).min(look.width), (y1.ceil().max(0.0) as usize + 1).min(look.height));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    // on the GPU when it can (when it can find the shadows falling on it)
    if look.gpu && ((!look.shadowed && look.reach.is_none() && look.material.reflection <= 0.0 && look.bounce.is_none() && look.material.transparency <= 0.0) || look.shadows.is_some()) && !crate::gpu::given_up() {
        match fill_gpu(look, &placed, [x0, y0, x1, y1]) {
            Ok(Some(shot)) => return Some(shot),
            Ok(None) => {}
            Err(e) => crate::gpu::failed(&e),
        }
    }
    let tris: Vec<Tri> = (0..placed.tris.len()).map(|i| placed.tri(i)).collect();
    fill(look, tris, [x0, y0, x1, y1])
}

/// Triangles `tris` filled on the processor, on canvas pixels `area` (x0, y0, x1, y1).
fn fill(look: &Look, tris: Vec<Tri>, area: [usize; 4]) -> Option<Shot> {
    let [x0, y0, x1, y1] = area;
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

/// A model's pictures as the GPU reads them: their texels one after another, and where each is.
struct Pictures {
    texels: veymelo_os::gpu::Buffer,
    pics: Vec<crate::gpu::Pic>,
}

/// The models whose pictures are on the GPU (each put there once), the most recently used last.
static PICTURES: std::sync::Mutex<Vec<(Arc<Model>, Arc<Pictures>)>> = std::sync::Mutex::new(Vec::new());

/// The most bytes of pictures, and the most models' pictures, kept on the GPU.
const MOST_PICTURES: (usize, usize) = (1 << 30, 64);

/// Model `model`'s pictures on the GPU: none when they are too many to put there.
fn pictures(model: &Arc<Model>) -> Result<Option<Arc<Pictures>>, String> {
    let total: usize = model.textures.iter().map(|t| t.px.len()).sum();
    if total * 16 > 1 << 31 {
        return Ok(None);
    }
    let mut kept = PICTURES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = kept.iter().position(|(m, _)| Arc::ptr_eq(m, model)) {
        let entry = kept.remove(i);
        let p = entry.1.clone();
        kept.push(entry);
        return Ok(Some(p));
    }
    let (texels, pics) = packed(model.textures.iter(), total)?;
    let p = Arc::new(Pictures { texels, pics });
    // a model without pictures has nothing worth keeping
    if total == 0 {
        return Ok(Some(p));
    }
    kept.push((model.clone(), p.clone()));
    // the least recently used let go, past the most
    let mut bytes: usize = kept.iter().map(|(_, p)| p.texels.len()).sum();
    while kept.len() > 1 && (bytes > MOST_PICTURES.0 || kept.len() > MOST_PICTURES.1) {
        bytes -= kept[0].1.texels.len();
        kept.remove(0);
    }
    Ok(Some(p))
}

/// A scene's face pictures (`face_pictures`) as the GPU reads them: where each is, and their
/// texels one after another.
pub(crate) struct FacePictures {
    pub pics: veymelo_os::gpu::Buffer,
    pub texels: veymelo_os::gpu::Buffer,
}

/// Face pictures on the GPU, and the models they are of.
type KeptPictures = (Vec<Arc<Model>>, Arc<FacePictures>);

/// The face pictures last put on the GPU (kept while the same models are drawn).
static FACE_PICTURES: std::sync::Mutex<Option<KeptPictures>> = std::sync::Mutex::new(None);

/// Models `models`' face pictures (`list`, as `face_pictures` makes them of the models) on the
/// GPU: none when they are too many to put there.
pub(crate) fn face_pictures_on_gpu(models: &[Arc<Model>], list: &[&veymelo_model::Texture]) -> Result<Option<Arc<FacePictures>>, String> {
    let total: usize = list.iter().map(|t| t.px.len()).sum();
    if total * 16 > 1 << 31 {
        return Ok(None);
    }
    let mut kept = FACE_PICTURES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, p)) = kept.as_ref()
        && k.len() == models.len()
        && k.iter().zip(models).all(|(a, b)| Arc::ptr_eq(a, b))
    {
        return Ok(Some(p.clone()));
    }
    let (texels, pics) = packed(list.iter().copied(), total)?;
    let p = Arc::new(FacePictures { pics: veymelo_os::gpu::Buffer::of(&pics)?, texels });
    *kept = Some((models.to_vec(), p.clone()));
    Ok(Some(p))
}

/// Pictures one after another as the GPU reads them (`total` texels in all), and where each is.
fn packed<'t>(list: impl Iterator<Item = &'t veymelo_model::Texture>, total: usize) -> Result<(veymelo_os::gpu::Buffer, Vec<crate::gpu::Pic>), String> {
    let mut texels = veymelo_os::gpu::Buffer::new((total * 16).max(16))?;
    let mut pics = Vec::new();
    let out = texels.slice_mut::<[f32; 4]>();
    let wrap = |w: veymelo_model::Wrap| match w {
        veymelo_model::Wrap::Repeat => 0u32,
        veymelo_model::Wrap::Clamp => 1,
        veymelo_model::Wrap::Mirror => 2,
    };
    let mut at = 0;
    for t in list {
        let n = t.px.len();
        out[at..at + n].copy_from_slice(&t.px);
        // a picture with no pixels (or not as many as it says) reads as white
        let (w, h) = if n > 0 && n == t.width * t.height { (t.width, t.height) } else { (0, 0) };
        pics.push(crate::gpu::Pic { a: [at as u32, w as u32, h as u32, t.smooth as u32 | wrap(t.wrap[0]) << 1 | wrap(t.wrap[1]) << 3] });
        at += n;
    }
    Ok((texels, pics))
}

/// The triangles `placed` filled on the GPU, as `fill` fills them, on canvas pixels `area`: none
/// when it cannot (then the processor does).
fn fill_gpu(look: &Look, placed: &Placed, area: [usize; 4]) -> Result<Option<Shot>, String> {
    use crate::gpu::{self, Mat, MeshParams, NONE, TILE};
    use veymelo_os::gpu::{Arg, bytes_of, run};
    let [x0, y0, x1, y1] = area;
    let (w, h) = (x1 - x0, y1 - y0);
    let model = look.model;
    if placed.verts.len() >= u32::MAX as usize || placed.verts.len() * 96 > 1 << 31 {
        return Ok(None);
    }
    let Some(pictures) = pictures(model)? else { return Ok(None) };
    let f = |v: f64| v as f32;
    let (fx, fy) = (x0 as f64, y0 as f64);
    // the corners, on the canvas from the shot's corner
    let verts: Vec<gpu::Vert> = placed
        .verts
        .iter()
        .zip(&placed.screen)
        .zip(&placed.inv_z)
        .map(|((v, s), iz)| gpu::Vert {
            screen: [f(s[0] - fx), f(s[1] - fy), f(*iz), 0.0],
            world: [f(v.world.x), f(v.world.y), f(v.world.z), 0.0],
            normal: [f(v.normal.x), f(v.normal.y), f(v.normal.z), 0.0],
            tangent: v.tangent.map(f),
            uv: [f(v.uv[0][0]), f(v.uv[0][1]), f(v.uv[1][0]), f(v.uv[1][1])],
            color: v.color,
            bounce: [v.bounce[0], v.bounce[1], v.bounce[2], 0.0],
        })
        .collect();
    // the materials, and the standard one after them (for triangles with none)
    let pictured = |m: Option<Map>| m.filter(|m| m.texture < model.textures.len()).map_or(NONE, |m| m.texture as u32);
    let set = |m: Option<Map>| m.map_or(0, |m| m.set.min(1));
    let fallback = veymelo_model::Material::default();
    let mats: Vec<Mat> = model
        .materials
        .iter()
        .chain([&fallback])
        .map(|m| Mat {
            base: m.base,
            emissive: [m.emissive[0], m.emissive[1], m.emissive[2], m.normal_scale],
            pbr: [m.metallic, m.roughness, m.occlusion, if let AlphaMode::Mask(c) = m.alpha { c } else { 0.0 }],
            maps: [pictured(m.base_map), pictured(m.metal_rough_map), pictured(m.normal_map), pictured(m.occlusion_map)],
            more: [
                pictured(m.emissive_map),
                set(m.base_map) | set(m.metal_rough_map) << 1 | set(m.normal_map) << 2 | set(m.occlusion_map) << 3 | set(m.emissive_map) << 4,
                match m.alpha {
                    AlphaMode::Opaque => 0,
                    AlphaMode::Mask(_) => 1,
                    AlphaMode::Blend => 2,
                },
                m.unlit as u32,
            ],
        })
        .collect();
    let standard = model.materials.len() as u32;
    // each triangle, and the tiles of pixels it may cover (its samples are a quarter of a pixel in)
    let tiles = [w.div_ceil(TILE[0]), h.div_ceil(TILE[1])];
    let mut tris = Vec::with_capacity(placed.tris.len());
    let mut spans: Vec<Option<[usize; 4]>> = Vec::with_capacity(placed.tris.len());
    let mut starts = vec![0u32; tiles[0] * tiles[1] + 1];
    for (c, material) in &placed.tris {
        let s = c.map(|k| placed.screen[k as usize]);
        let (lo_x, hi_x) = (s[0][0].min(s[1][0]).min(s[2][0]) - fx, s[0][0].max(s[1][0]).max(s[2][0]) - fx);
        let (lo_y, hi_y) = (s[0][1].min(s[1][1]).min(s[2][1]) - fy, s[0][1].max(s[1][1]).max(s[2][1]) - fy);
        tris.push(gpu::Tri { v: [c[0], c[1], c[2], material.filter(|&m| m < model.materials.len()).map_or(standard, |m| m as u32)], reach: [f(lo_x), f(lo_y), f(hi_x), f(hi_y)] });
        let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
        let span = (area.abs() >= 1e-12 && hi_x >= -1.0 && hi_y >= -1.0 && lo_x <= w as f64 && lo_y <= h as f64).then(|| {
            let tile = |v: f64, n: usize, size: usize| ((v.max(0.0) as usize) / size).min(n - 1);
            [tile(lo_x - 1.0, tiles[0], TILE[0]), tile(lo_y - 1.0, tiles[1], TILE[1]), tile(hi_x + 1.0, tiles[0], TILE[0]), tile(hi_y + 1.0, tiles[1], TILE[1])]
        });
        if let Some([a, b, c, d]) = span {
            for ty in b..=d {
                for tx in a..=c {
                    starts[ty * tiles[0] + tx + 1] += 1;
                }
            }
        }
        spans.push(span);
    }
    for i in 1..starts.len() {
        starts[i] += starts[i - 1];
    }
    let listed_len = *starts.last().unwrap_or(&0) as usize;
    if listed_len > (1 << 28) {
        return Ok(None);
    }
    // each tile's triangles, in the model's order
    let mut listed = vec![0u32; listed_len.max(1)];
    let mut next: Vec<u32> = starts[..starts.len() - 1].to_vec();
    for (i, span) in spans.iter().enumerate() {
        if let Some([a, b, c, d]) = *span {
            for ty in b..=d {
                for tx in a..=c {
                    let k = &mut next[ty * tiles[0] + tx];
                    listed[*k as usize] = i as u32;
                    *k += 1;
                }
            }
        }
    }
    let lights: Vec<gpu::Light> = look.lights.iter().map(gpu::Light::of).collect();
    let (env, env_levels, turn, shining) = gpu::environment(look.env)?;
    let scene_light = veymelo_os::gpu::Buffer::of(&look.scene_light.0.iter().map(|c| [c[0], c[1], c[2], 0.0]).collect::<Vec<[f32; 4]>>())?;
    // none falling on it and no occlusion: nothing rays meet
    let none;
    let shadows = match look.shadows {
        Some(s) if look.shadowed || look.reach.is_some() || look.material.reflection > 0.0 || look.bounce.is_some() || look.material.transparency > 0.0 => s,
        _ => {
            none = gpu::Shadows::none()?;
            &none
        }
    };
    let m = &look.material;
    let params = MeshParams {
        a: [w as u32, h as u32, tiles[0] as u32, lights.len() as u32],
        b: [look.linear as u32, m.lit as u32, shining, (m.accepts != 0) as u32],
        eye: [f(look.cam.eye.x), f(look.cam.eye.y), f(look.cam.eye.z), turn],
        layer: [m.ambient, m.diffuse, m.specular, m.transmission],
        c: [shadows.count, 0, 0, 0],
        d: [look.env.map_or(0.0, |e| e.occlusion), look.reach.map_or(0.0, |r| r as f32), look.bounce.map_or(0.0, |b| b.1), look.scene_light.1],
        e: [if look.mirror.is_some() { m.reflection } else { 0.0 }, m.sharpness, m.rolloff, 0.0],
        f: [if look.through.is_some() { m.transparency } else { 0.0 }, m.clear_rolloff, m.ior, 0.0],
    };
    let (verts, tris, starts, listed) = (gpu::take_with(gpu::VERTS, &verts)?, gpu::take_with(gpu::TRIS, &tris)?, gpu::take_with(gpu::STARTS, &starts)?, gpu::take_with(gpu::LISTED, &listed)?);
    let (mats, pics, lights) = (gpu::take_with(gpu::MATS, &mats)?, gpu::take_with(gpu::PICS, &pictures.pics)?, gpu::take_with(gpu::LIGHTS, &lights)?);
    let (mut color, mut nearest, mut surface) = (gpu::take(gpu::COLOR, w * h * 16)?, gpu::take(gpu::NEAREST, w * h * 16)?, gpu::take(gpu::SURFACE, w * h * 16)?);
    let mut args = vec![
        Arg::Write(&mut color),
        Arg::Write(&mut nearest),
        Arg::Write(&mut surface),
        Arg::Read(&verts),
        Arg::Read(&tris),
        Arg::Read(&starts),
        Arg::Read(&listed),
        Arg::Read(&mats),
        Arg::Read(&pics),
        Arg::Read(&pictures.texels),
        Arg::Read(&lights),
        Arg::Bytes(bytes_of(&params)),
        Arg::Read(&env),
        Arg::Read(&env_levels),
        Arg::Read(&shadows.casters),
        Arg::Read(&shadows.texels),
        Arg::Read(&shadows.faces),
        Arg::Read(&scene_light),
    ];
    let (kernel, traced) = shadows.kernel(gpu::MESH, gpu::MESH_RAYS)?;
    args.extend(traced);
    run(&kernel, &args, [w, h])?;
    drop(args);
    let (c, n, s) = (&color.slice::<[f32; 4]>()[..w * h], &nearest.slice::<[f32; 4]>()[..w * h], &surface.slice::<[f32; 4]>()[..w * h]);
    let px = c.iter().zip(n).map(|(c, n)| (n[0] as f64, *c)).collect();
    let facts = n
        .iter()
        .zip(s)
        .map(|(n, s)| {
            let material = s[2].to_bits();
            Facts { normal: [n[1], n[2], n[3]], uv: [s[0], s[1]], material: if material >= standard { u32::MAX } else { material } }
        })
        .collect();
    for (slot, b) in [(gpu::VERTS, verts), (gpu::TRIS, tris), (gpu::STARTS, starts), (gpu::LISTED, listed), (gpu::MATS, mats), (gpu::PICS, pics), (gpu::LIGHTS, lights), (gpu::COLOR, color), (gpu::NEAREST, nearest), (gpu::SURFACE, surface), (gpu::ENV, env), (gpu::ENV_LEVELS, env_levels)] {
        gpu::keep(slot, b);
    }
    Ok(Some(Shot { x0, y0, w, h, px, facts }))
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
        occlusion: 0.0,
        occlusion_distance: 0.0,
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

#[cfg(test)]
mod tests {
    use super::*;
    use veymelo_model::shapes::{Shape, Sizes};
    use veymelo_model::{Texture, Wrap};

    /// A picture `w` × `h` of stripes over a ramp, its alpha falling across it.
    fn picture(w: usize, h: usize, wrap: [Wrap; 2], smooth: bool) -> Texture {
        let px = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                let stripe = if (i % w) / 3 % 2 == 0 { 0.9 } else { 0.2 };
                [stripe, x, y, 1.0 - 0.8 * x]
            })
            .collect();
        Texture { width: w, height: h, px, wrap, smooth }
    }

    /// A sphere with every kind of picture its material can take, both sets of texture points
    /// (the second past the edges, so the pictures repeat, clamp and mirror) and vertex colours.
    fn textured(alpha: AlphaMode) -> Arc<Model> {
        let map = |texture, set| Some(Map { texture, set });
        let material = veymelo_model::Material {
            base: [0.9, 0.8, 0.7, 1.0],
            base_map: map(0, 0),
            metal_rough_map: map(1, 1),
            normal_map: map(2, 0),
            normal_scale: 0.8,
            occlusion_map: map(1, 0),
            occlusion: 0.7,
            emissive: [0.1, 0.0, 0.05],
            emissive_map: map(0, 1),
            alpha,
            metallic: 0.7,
            roughness: 0.6,
            ..veymelo_model::Material::default()
        };
        let mut m = veymelo_model::shapes::model(Shape::Sphere, &Sizes { radius: 1.0, segments: 40, rings: 20, ..Sizes::default() }, material);
        let p = &mut m.meshes[0].primitives[0];
        p.uvs[1] = p.uvs[0].iter().map(|u| [u[0] * 3.0 - 1.0, u[1] * 2.5 - 0.7]).collect();
        p.colors = p.positions.iter().map(|q| [0.6 + 0.4 * q[0], 0.8, 0.7 + 0.3 * q[1], 1.0]).collect();
        m.textures = vec![picture(37, 23, [Wrap::Repeat, Wrap::Mirror], true), picture(16, 9, [Wrap::Mirror, Wrap::Clamp], false), picture(20, 20, [Wrap::Clamp, Wrap::Repeat], true)];
        Arc::new(m)
    }

    fn light(kind: veymelo_project::LightKind, at: Vec3, dir: Vec3, color: f32) -> Light {
        Light { kind, at, dir, color: [color, color * 0.9, color * 0.8], cone: 0.6, feather: 0.4, falloff: 2, radius: 300.0, falloff_distance: 400.0, shadows: false, darkness: 1.0, diffusion: 0.0, occlusion: 0.0, occlusion_distance: 0.0 }
    }

    /// A sky for an environment light: blue above, warm below, a bright sun.
    fn sky() -> crate::scene::Environment {
        let (w, h) = (64, 32);
        let px = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let sun = if (x as i32 - 20).abs() < 3 && (y as i32 - 8).abs() < 3 { 6.0 } else { 0.0 };
                let v = y as f32 / h as f32;
                [0.3 + 0.6 * v + sun, 0.5 + 0.2 * v + sun, 0.9 - 0.5 * v + sun]
            })
            .collect();
        crate::scene::Environment::new(w, h, px, 0.4).expect("a sky")
    }

    /// The sphere drawn on the GPU and by the processor (lit by `lights` lights of three, and by
    /// a sky with `env`; taking light, with `lit`): how far apart their colours are at most, and
    /// how many pixels differ by more than 2/255 (of how many either covers).
    fn both(alpha: AlphaMode, linear: bool, lights_on: usize, env: bool, lit: bool) -> Option<(f32, usize, usize)> {
        let model = textured(alpha);
        let sky = sky();
        let cam = Camera { view: Mat4::translate(vec3(0.0, 0.0, 600.0)), eye: vec3(0.0, 0.0, -600.0), zoom: 700.0, focus: None };
        let lights = [
            light(veymelo_project::LightKind::Point, vec3(-300.0, -250.0, -300.0), vec3(0.0, 0.0, 1.0), 0.9),
            light(veymelo_project::LightKind::Spot, vec3(250.0, -100.0, -400.0), vec3(-0.4, 0.15, 1.0).normalize(), 0.7),
            light(veymelo_project::LightKind::Ambient, Vec3::default(), vec3(0.0, 0.0, 1.0), 0.3),
        ];
        let material = Material { casts: 0, transmission: 0.2, accepts: 1, lit, ambient: 1.0, diffuse: 0.6, specular: 0.5, shininess: 0.5, metal: 1.0, reflection: 0.0, sharpness: 1.0, rolloff: 0.0, transparency: 0.0, clear_rolloff: 0.0, ior: 1.0 };
        let unshadowed = |_: usize, _: Vec3| [1.0; 3];
        let pose = model.pose(None, 0.0);
        let (w, h) = (320, 240);
        let look = |gpu| Look {
            model: &model,
            pose: &pose,
            place: Mat4::rotate_y(0.6) * Mat4::rotate_x(-0.3) * Mat4::scale(vec3(90.0, -90.0, -90.0)),
            cam: &cam,
            onto: Mat3([[cam.zoom, 0.0, w as f64 / 2.0], [0.0, cam.zoom, h as f64 / 2.0], [0.0, 0.0, 1.0]]),
            width: w,
            height: h,
            material,
            lights: &lights[..lights_on],
            unshadowed: &unshadowed,
            linear,
            env: env.then_some(&sky),
            shadowed: false,
            shadows: None,
            open: None,
            reach: None,
            mirror: None,
            bounce: None,
            through: None,
            scene_light: (&[], 0.0),
            gpu,
        };
        let cpu = shoot(&look(false)).expect("the processor draws it");
        let gpu = shoot(&look(true))?;
        assert_eq!((cpu.x0, cpu.y0, cpu.w, cpu.h), (gpu.x0, gpu.y0, gpu.w, gpu.h));
        let (mut most, mut many, mut covered) = (0f32, 0, 0);
        for (i, (a, b)) in cpu.px.iter().zip(&gpu.px).enumerate() {
            if a.0.is_finite() || b.0.is_finite() {
                covered += 1;
            }
            if a.0.is_finite() && b.0.is_finite() {
                assert!((a.0 - b.0).abs() < a.0 * 1e-4, "depth at {i}: {} and {}", a.0, b.0);
                assert_eq!(cpu.facts[i].material, gpu.facts[i].material);
            }
            let d = (0..4).map(|c| (a.1[c] - b.1[c]).abs()).fold(0.0, f32::max);
            most = most.max(d);
            if d > 2.0 / 255.0 {
                many += 1;
            }
        }
        Some((most, many, covered))
    }

    #[test]
    fn a_model_is_drawn_alike_on_the_gpu_and_by_the_processor() {
        if !veymelo_os::gpu::available() {
            return;
        }
        let ways = [AlphaMode::Opaque, AlphaMode::Mask(0.5), AlphaMode::Blend].into_iter().flat_map(|a| [(a, false, 3, false, true), (a, true, 3, false, true)]);
        // lit by an environment too, by it alone, and taking no light
        let more = [(AlphaMode::Opaque, true, 3, true, true), (AlphaMode::Opaque, false, 0, true, true), (AlphaMode::Opaque, true, 0, true, false), (AlphaMode::Blend, true, 3, true, false)];
        for (alpha, linear, lights, env, lit) in ways.chain(more) {
            {
                let (most, many, covered) = both(alpha, linear, lights, env, lit).expect("the GPU draws it");
                assert!(covered > 10_000, "{covered}");
                // the same, but for a few pixels on the edge of a mask (where single precision
                // tips a sample the other way), and where the sun's sharp edge shines
                let edge = if matches!(alpha, AlphaMode::Mask(_)) { covered / 200 } else if env { covered / 2000 } else { 0 };
                assert!(many <= edge, "{alpha:?} linear {linear} lights {lights} env {env} lit {lit}: {many} of {covered} pixels differ (most {most})");
            }
        }
    }
}
