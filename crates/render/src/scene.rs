//! The 3D scene at a moment: the camera a composition is seen through, its lights, and how each
//! 3D layer takes light (its material).
//!
//! World space is the composition's: x right, y down, z away from the viewer. A camera looks
//! along its +z with +y down; it shows a point at depth z (in front of it) at
//! `zoom · x / z + width / 2`.

use veymelo_expr::{Eval, Place};
use veymelo_geom::{Mat4, Vec3, vec3};
use veymelo_keyframe::Value;
use veymelo_project::{Comp, Layer, LightKind, Property, Source};
use veymelo_time::Tick;

/// The nearest a camera sees (pixels in front of it).
pub const NEAR: f64 = 1.0;

/// What a camera blurs out of focus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Focus {
    /// How far from the eye is sharp.
    pub distance: f64,
    /// How wide the lens opens (pixels).
    pub aperture: f64,
    /// 1 for 100 %.
    pub level: f64,
    /// The iris: its sides (0 for a fast rectangle), turn (degrees), roundness and aspect.
    pub sides: u32,
    pub rotation: f64,
    pub roundness: f64,
    pub aspect: f64,
    /// Highlights out of focus: gain, threshold (0 to 1), saturation.
    pub gain: f64,
    pub threshold: f64,
    pub saturation: f64,
}

impl Focus {
    /// How wide (composition pixels, across) a point at depth `z` spreads.
    pub fn spread(&self, z: f64) -> f64 {
        if z <= NEAR || self.distance <= 0.0 {
            return 0.0;
        }
        self.aperture * (z - self.distance).abs() / z * self.level
    }
}

/// A camera at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// From the world to the camera's own space.
    pub view: Mat4,
    pub eye: Vec3,
    pub zoom: f64,
    pub focus: Option<Focus>,
}

/// The camera a composition is seen through at `t`: its top camera layer that is on and there,
/// or the standard one.
pub fn camera(eval: &Eval, comp: &Comp, t: Tick) -> Camera {
    let w = comp.width as f64;
    let (view, eye, zoom, layer) = eval.camera(comp, t);
    let Some(layer) = layer else { return Camera { view, eye, zoom, focus: None } };
    let at = eval.place(comp, layer);
    let opts = layer.props.group("cameraOptions").ok();
    let read = |id: &str, d: f64| opts.and_then(|g| g.prop(id).ok()).map_or(d, |p| eval.value(p, at, t).as_f64());
    let flag = |id: &str| opts.and_then(|g| g.prop(id).ok()).is_some_and(|p| eval.value(p, at, t).as_bool());
    let focus = flag("depthOfField").then(|| Focus {
        distance: read("focusDistance", zoom),
        aperture: read("aperture", veymelo_project::scene::aperture_for(w)).max(0.0),
        level: read("blurLevel", 100.0).max(0.0) / 100.0,
        sides: match read("irisShape", 0.0) as u32 {
            0 => 0,
            k => k + 2,
        },
        rotation: read("irisRotation", 0.0),
        roundness: (read("irisRoundness", 0.0) / 100.0).clamp(0.0, 1.0),
        aspect: read("irisAspectRatio", 1.0).max(0.01),
        gain: (read("highlightGain", 0.0) / 100.0).clamp(0.0, 1.0),
        threshold: (read("highlightThreshold", 255.0) / 255.0).clamp(0.0, 1.0),
        saturation: (read("highlightSaturation", 0.0) / 100.0).clamp(0.0, 1.0),
    });
    Camera { view, eye, zoom, focus }
}

/// A light at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    pub at: Vec3,
    /// Which way it shines (a parallel or spot light).
    pub dir: Vec3,
    /// Its colour times its intensity (in the working space).
    pub color: [f32; 3],
    /// Half its cone (radians), and how much of it is feathered.
    pub cone: f64,
    pub feather: f64,
    /// 0 none, 1 smooth, 2 inverse square clamped.
    pub falloff: u32,
    pub radius: f64,
    pub falloff_distance: f64,
    pub shadows: bool,
    pub darkness: f64,
    pub diffusion: f64,
    /// An ambient light kept out of nooks (0 to 1), looked for this far round a point.
    pub occlusion: f32,
    pub occlusion_distance: f64,
}

impl Light {
    /// Toward the light from `p`, and how far (endless for a parallel light).
    pub fn toward(&self, p: Vec3) -> (Vec3, f64) {
        match self.kind {
            LightKind::Parallel => (-self.dir, f64::INFINITY),
            _ => {
                let d = self.at - p;
                let len = d.length();
                if len < 1e-9 { (vec3(0.0, 0.0, -1.0), 0.0) } else { (d / len, len) }
            }
        }
    }

    /// How much of it reaches `p` (0 to 1): by distance, and inside a spot light's cone.
    pub fn reach(&self, p: Vec3) -> f64 {
        let (l, dist) = self.toward(p);
        let smooth = |t: f64| t * t * (3.0 - 2.0 * t);
        let mut k = match self.falloff {
            1 if dist.is_finite() => {
                if dist <= self.radius {
                    1.0
                } else if self.falloff_distance <= 0.0 {
                    0.0
                } else {
                    1.0 - smooth(((dist - self.radius) / self.falloff_distance).clamp(0.0, 1.0))
                }
            }
            2 if dist.is_finite() => {
                let r = self.radius.max(1.0);
                if dist <= r { 1.0 } else { (r / dist).powi(2) }
            }
            _ => 1.0,
        };
        if self.kind == LightKind::Spot {
            let a = (-l).dot(self.dir).clamp(-1.0, 1.0).acos();
            let inner = self.cone * (1.0 - self.feather);
            k *= if a <= inner {
                1.0
            } else if a >= self.cone {
                0.0
            } else {
                1.0 - smooth((a - inner) / (self.cone - inner).max(1e-9))
            };
        }
        k
    }
}

/// The lights of a composition that are on and there at `t`, their colours in the working space
/// (`linear`).
pub fn lights(eval: &Eval, comp: &Comp, t: Tick, linear: bool) -> Vec<Light> {
    comp.layers
        .iter()
        .filter(|l| l.switches.video && l.is_active_at(t))
        .filter_map(|layer| {
            let Source::Light(kind) = layer.source else { return None };
            if kind == LightKind::Environment {
                return None;
            }
            let at = eval.place(comp, layer);
            let world = eval.world(at, t, None);
            let g = layer.props.group("lightOptions").ok()?;
            let read = |id: &str, d: f64| g.prop(id).ok().map_or(d, |p| eval.value(p, at, t).as_f64());
            let flag = |id: &str| g.prop(id).ok().is_some_and(|p| eval.value(p, at, t).as_bool());
            let intensity = read("intensity", 100.0) / 100.0;
            let c = g.prop("color").ok().map_or(veymelo_color::Rgba::WHITE, |p| eval.value(p, at, t).as_color());
            let ch = |v: f64| (if linear { veymelo_color::srgb_to_linear(v) } else { v } * intensity) as f32;
            Some(Light {
                kind,
                at: world.apply(vec3(0.0, 0.0, 0.0)),
                dir: world.apply_vec(vec3(0.0, 0.0, 1.0)).normalize(),
                color: [ch(c.r), ch(c.g), ch(c.b)],
                cone: (read("coneAngle", 90.0).clamp(0.0, 180.0) / 2.0).to_radians(),
                feather: (read("coneFeather", 50.0) / 100.0).clamp(0.0, 1.0),
                falloff: g.prop("falloff").ok().map_or(0, |p| eval.value(p, at, t).as_choice()),
                radius: read("radius", 500.0).max(0.0),
                falloff_distance: read("falloffDistance", 500.0).max(0.0),
                shadows: flag("castsShadows") && kind != LightKind::Ambient,
                darkness: (read("shadowDarkness", 100.0) / 100.0).clamp(0.0, 1.0),
                diffusion: read("shadowDiffusion", 0.0).max(0.0),
                occlusion: if kind == LightKind::Ambient { (read("ambientOcclusion", 0.0) / 100.0).clamp(0.0, 1.0) as f32 } else { 0.0 },
                occlusion_distance: read("occlusionDistance", 60.0).max(0.0),
            })
        })
        .collect()
}

/// How a 3D layer takes light and shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// 0 off, 1 on, 2 only.
    pub casts: u32,
    pub transmission: f32,
    pub accepts: u32,
    pub lit: bool,
    pub ambient: f32,
    pub diffuse: f32,
    pub specular: f32,
    /// 0 to 1.
    pub shininess: f32,
    pub metal: f32,
    /// How much it mirrors (0 to 1), how sharply (1: a mirror), and how much less face on.
    pub reflection: f32,
    pub sharpness: f32,
    pub rolloff: f32,
    /// How much a model is seen through (0 to 1), how much less edge on, and how much light
    /// bends going into it.
    pub transparency: f32,
    pub clear_rolloff: f32,
    pub ior: f32,
}

impl Material {
    pub fn of(eval: &Eval, at: Place, t: Tick) -> Material {
        let g = at.layer.props.group("materialOptions").ok();
        let value = |id: &str| g.and_then(|g| g.prop(id).ok()).map(|p: &Property| eval.value(p, at, t));
        let part = |id: &str, d: f64| (value(id).map_or(d, |v: Value| v.as_f64()) / 100.0).clamp(0.0, 10.0) as f32;
        Material {
            casts: value("castsShadows").map_or(0, |v| v.as_choice()),
            transmission: part("lightTransmission", 0.0).min(1.0),
            accepts: value("acceptsShadows").map_or(1, |v| v.as_choice()),
            lit: value("acceptsLights").is_none_or(|v| v.as_bool()),
            ambient: part("ambient", 100.0),
            diffuse: part("diffuse", 50.0),
            specular: part("specularIntensity", 50.0),
            shininess: part("specularShininess", 5.0).min(1.0),
            metal: part("metal", 100.0).min(1.0),
            reflection: part("reflectionIntensity", 0.0).min(1.0),
            sharpness: part("reflectionSharpness", 100.0).min(1.0),
            rolloff: part("reflectionRolloff", 0.0).min(1.0),
            transparency: part("transparency", 0.0).min(1.0),
            clear_rolloff: part("transparencyRolloff", 0.0).min(1.0),
            ior: value("indexOfRefraction").map_or(1.0, |v| v.as_f64()).clamp(1.0, 3.0) as f32,
        }
    }

    /// How much a model is seen through seen from `viewer` on a surface facing `n` (toward the
    /// viewer).
    pub fn clear(&self, n: Vec3, viewer: Vec3) -> f32 {
        self.transparency * (1.0 - self.clear_rolloff * (1.0 - n.dot(viewer).clamp(0.0, 1.0) as f32))
    }

    /// How much it mirrors seen from `viewer` on a surface facing `n` (toward the viewer).
    pub fn mirrors(&self, n: Vec3, viewer: Vec3) -> f32 {
        self.reflection * (1.0 - self.rolloff * n.dot(viewer).clamp(0.0, 1.0) as f32)
    }

    /// The shine's sharpness: 1 at 0 %, 256 at 100 %.
    pub fn exponent(&self) -> f64 {
        256f64.powf(self.shininess as f64).max(1.0)
    }
}

/// A colour (straight) at world point `p` on a surface facing `n`, seen from `viewer` (a unit
/// direction toward the eye), lit: the ambient lights (each kept out as much as it occludes
/// where only `open` of the sky round the point is open) and the light falling on it (through
/// it from behind by its transmission), each as much as `unshadowed(light, p)` lets through,
/// with a shine coloured by the light or, by its metal, by the surface. Unlit without lights.
/// Also how much shadow falls there (0 to 1).
#[allow(clippy::too_many_arguments)]
pub fn shade(rgb: [f32; 3], p: Vec3, n: Vec3, viewer: Vec3, m: &Material, lights: &[Light], unshadowed: &dyn Fn(usize, Vec3) -> [f32; 3], open: f32) -> ([f32; 3], f32) {
    if lights.is_empty() || !m.lit {
        return (rgb, 0.0);
    }
    // lit on the side it is seen from
    let n = if n.dot(viewer) < 0.0 { -n } else { n };
    let (mut ambient, mut diffuse, mut shine, mut shadow) = ([0f32; 3], [0f32; 3], [0f32; 3], 0f32);
    let exp = m.exponent();
    for (i, l) in lights.iter().enumerate() {
        if l.kind == LightKind::Ambient {
            let k = 1.0 - l.occlusion * (1.0 - open);
            (0..3).for_each(|c| ambient[c] += l.color[c] * k);
            continue;
        }
        let k = l.reach(p);
        if k <= 0.0 {
            continue;
        }
        let (to, _) = l.toward(p);
        let facing = n.dot(to);
        let lambert = if facing > 0.0 { facing } else { -facing * m.transmission as f64 };
        if lambert <= 0.0 {
            continue;
        }
        let through = if l.shadows && m.accepts != 0 { unshadowed(i, p) } else { [1.0; 3] };
        shadow = shadow.max(1.0 - (through[0] + through[1] + through[2]) / 3.0);
        let kd = (lambert * k) as f32;
        (0..3).for_each(|c| diffuse[c] += l.color[c] * kd * through[c]);
        if facing > 0.0 && m.specular > 0.0 {
            let half = (to + viewer).normalize();
            let s = (n.dot(half).max(0.0).powf(exp) * k) as f32;
            for c in 0..3 {
                let tint = 1.0 + (rgb[c] - 1.0) * m.metal;
                shine[c] += l.color[c] * s * through[c] * tint;
            }
        }
    }
    (std::array::from_fn(|c| rgb[c] * (m.ambient * ambient[c] + m.diffuse * diffuse[c]) + m.specular * shine[c]), shadow)
}

/// Direction `d` bent going through a surface facing `n` (against `d`) from a stuff into one `eta`
/// times as slow: none where it cannot go through (reflected whole inside).
pub fn refract(d: Vec3, n: Vec3, eta: f64) -> Option<Vec3> {
    let c = n.dot(d);
    let k = 1.0 - eta * eta * (1.0 - c * c);
    (k >= 0.0).then(|| (d * eta - n * (eta * c + k.sqrt())).normalize())
}

/// The directions a surface facing `n` mirrors along, seen from `viewer`, as sharply as
/// `sharpness` (1: a mirror, one way; less: many ways in a cone round it, wider the less, the
/// pattern turned a little from point `q` to point).
pub fn mirror_rays(q: Vec3, n: Vec3, viewer: Vec3, sharpness: f32) -> Vec<Vec3> {
    let r = n * (2.0 * n.dot(viewer)) - viewer;
    if sharpness >= 1.0 {
        return vec![r];
    }
    let side = if r.x.abs() < 0.9 { vec3(1.0, 0.0, 0.0) } else { vec3(0.0, 1.0, 0.0) };
    let a = r.cross(side).normalize();
    let b = r.cross(a);
    let wide = ((1.0 - sharpness as f64) * MIRROR_CONE).tan();
    let turn = veymelo_raster::noise((q.x * 7.0) as usize, (q.y * 7.0) as usize, (q.z as i64 as u32) ^ MIRROR_SEED) as f64 * std::f64::consts::TAU;
    (0..MIRROR_RAYS)
        .map(|k| {
            let s = wide * ((k as f64 + 0.5) / MIRROR_RAYS as f64).sqrt();
            let ang = k as f64 * 2.399_963 + turn;
            (r + a * (s * ang.cos()) + b * (s * ang.sin())).normalize()
        })
        .collect()
}

/// How many ways a surface less sharp than a mirror mirrors along, and the widest cone (half
/// across, radians) of them.
pub const MIRROR_RAYS: usize = 16;
pub const MIRROR_CONE: f64 = 0.5;

/// What the mirrors' turns are made from (apart from the shadows' and the occlusion's).
pub const MIRROR_SEED: u32 = 0x85EB_CA6B;

/// A surface a reflection meets, lit simply (as it is seen in a mirror): colour `rgb` (straight)
/// facing `n` (toward where it is seen from) at `p`, taking `ambient` of the ambient lights and
/// `diffuse` of the light falling on it (no shadows, no shine), of the light the environment
/// lets fall on it and of the light bouncing onto it (`bounce`, as much as is taken); as it is
/// when it takes no light.
#[allow(clippy::too_many_arguments)]
pub fn seen_in_mirror(rgb: [f32; 3], p: Vec3, n: Vec3, ambient: f32, diffuse: f32, lit: bool, lights: &[Light], env: Option<&Environment>, bounce: [f32; 3]) -> [f32; 3] {
    if !lit || (lights.is_empty() && env.is_none()) {
        return rgb;
    }
    let (mut amb, mut dif) = ([0f32; 3], [0f32; 3]);
    let fell = bounce;
    for l in lights {
        if l.kind == LightKind::Ambient {
            (0..3).for_each(|c| amb[c] += l.color[c]);
            continue;
        }
        let k = l.reach(p);
        let (to, _) = l.toward(p);
        let facing = n.dot(to);
        if k <= 0.0 || facing <= 0.0 {
            continue;
        }
        (0..3).for_each(|c| dif[c] += l.color[c] * (facing * k) as f32);
    }
    let fall = env.map_or([0.0; 3], |e| e.falling(n));
    std::array::from_fn(|c| rgb[c] * (ambient * amb[c] + diffuse * dif[c] + diffuse * 2.0 * (fall[c] + fell[c])))
}

/// How far round a point the ambient occlusion of `lights` and `env` looks: none when none of
/// them occludes.
pub fn occlusion_reach(lights: &[Light], env: Option<&Environment>) -> Option<f64> {
    let reach = lights.iter().filter(|l| l.occlusion > 0.0).map(|l| l.occlusion_distance).chain(env.filter(|e| e.occlusion > 0.0).map(|e| e.occlusion_distance)).fold(0.0, f64::max);
    (reach > 0.0).then_some(reach)
}

/// How many rays the ambient occlusion looks along from a point.
pub const OPEN_RAYS: usize = 32;

/// The directions the ambient occlusion looks along from world point `q` on a surface facing
/// `n`: spread evenly over the half sphere it faces, more toward its top (as light falls, by the
/// cosine: a sunflower on the disc beneath, lifted), the pattern turned a little from point to
/// point.
pub fn open_rays(q: Vec3, n: Vec3) -> impl Iterator<Item = Vec3> {
    let side = if n.x.abs() < 0.9 { vec3(1.0, 0.0, 0.0) } else { vec3(0.0, 1.0, 0.0) };
    let a = n.cross(side).normalize();
    let b = n.cross(a);
    let turn = veymelo_raster::noise((q.x * 7.0) as usize, (q.y * 7.0) as usize, (q.z as i64 as u32) ^ OPEN_SEED) as f64 * std::f64::consts::TAU;
    (0..OPEN_RAYS).map(move |k| {
        let r = ((k as f64 + 0.5) / OPEN_RAYS as f64).sqrt();
        let ang = k as f64 * 2.399_963 + turn;
        a * (r * ang.cos()) + b * (r * ang.sin()) + n * (1.0 - r * r).max(0.0).sqrt()
    })
}

/// What the ambient occlusion's turns are made from (apart from the shadows').
pub const OPEN_SEED: u32 = 0x9E37_79B9;

/// Whether a layer draws in 3D (a 3D layer with pixels of its own, not an adjustment layer).
pub fn in_3d(layer: &Layer) -> bool {
    layer.is_3d() && !layer.switches.adjustment && !matches!(layer.source, Source::Camera | Source::Light(_) | Source::Null)
}

/// A panorama's pixels (straight colours), `w` × `h`.
#[derive(Clone, Debug)]
struct Pano {
    w: usize,
    h: usize,
    px: Vec<[f32; 3]>,
}

impl Pano {
    /// The colour at (`u`, `v`) (0 to 1 across and down), between its pixels, round past its
    /// left and right edges.
    fn at(&self, u: f64, v: f64) -> [f32; 3] {
        let (x, y) = (u * self.w as f64 - 0.5, (v * self.h as f64 - 0.5).clamp(0.0, (self.h - 1) as f64));
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
        let get = |i: f64, j: f64| self.px[(j as usize).min(self.h - 1) * self.w + (i as i64).rem_euclid(self.w as i64) as usize];
        let (a, b, c, d) = (get(x0, y0), get(x0 + 1.0, y0), get(x0, y0 + 1.0), get(x0 + 1.0, y0 + 1.0));
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * fx;
            top + (c[k] + (d[k] - c[k]) * fx - top) * fy
        })
    }

    /// Half as wide and high (odd sizes rounded up), each pixel the mean of those it covers.
    fn half(&self) -> Pano {
        let (w, h) = (self.w.div_ceil(2).max(1), self.h.div_ceil(2).max(1));
        let mut px = vec![[0.0f32; 3]; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0.0f32; 3];
                let mut n = 0.0;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = (2 * x + dx, 2 * y + dy);
                    if sx < self.w && sy < self.h {
                        let p = self.px[sy * self.w + sx];
                        (0..3).for_each(|k| sum[k] += p[k]);
                        n += 1.0;
                    }
                }
                px[y * w + x] = sum.map(|v| v / n);
            }
        }
        Pano { w, h, px }
    }
}

/// Where world direction `d` falls on a panorama turned `rot` radians about the vertical: (u, v)
/// from 0 to 1 across and down (up, −y in the world, at its top).
pub fn panorama_at(d: Vec3, rot: f64) -> (f64, f64) {
    let u = 0.5 + (d.x.atan2(d.z) + rot) / std::f64::consts::TAU;
    (u - u.floor(), (-d.y).clamp(-1.0, 1.0).acos() / std::f64::consts::PI)
}

/// The world direction a panorama's (`u`, `v`) looks along.
fn panorama_dir(u: f64, v: f64, rot: f64) -> Vec3 {
    let (phi, theta) = ((u - 0.5) * std::f64::consts::TAU - rot, v * std::f64::consts::PI);
    vec3(theta.sin() * phi.sin(), -theta.cos(), theta.sin() * phi.cos())
}

/// The light of an environment: its panorama (in the working space, times the light's colour
/// and intensity) halved again and again (what a rougher surface shines with), the light falling
/// from it on each way a surface may face, and the turn of it about the vertical.
#[derive(Clone, Debug)]
pub struct Environment {
    levels: Vec<Pano>,
    falling: Pano,
    rotation: f64,
    /// Kept out of nooks (0 to 1), looked for this far round a point.
    pub occlusion: f32,
    pub occlusion_distance: f64,
    /// As the GPU reads it, once asked for.
    packed: std::sync::OnceLock<Packed>,
}

/// An environment's panoramas as the GPU reads them: their pixels one after another, and each
/// one's first pixel, width and height.
pub type Packed = (Vec<[f32; 4]>, Vec<[u32; 4]>);

impl Environment {
    /// From a panorama's `w` × `h` pixels, turned `rotation` radians.
    pub fn new(w: usize, h: usize, px: Vec<[f32; 3]>, rotation: f64) -> Option<Environment> {
        if w == 0 || h == 0 || px.len() != w * h {
            return None;
        }
        let mut level = Pano { w, h, px };
        while level.w > 1024 {
            level = level.half();
        }
        let mut levels = vec![level];
        loop {
            let last = levels.last().expect("one at least");
            if last.w <= 8 || last.h <= 4 {
                break;
            }
            let next = last.half();
            levels.push(next);
        }
        // the light falling on a surface facing each way: the panorama (a small copy) weighed by
        // the cosine and by how much of the sphere each of its pixels covers
        let small = levels.iter().find(|l| l.w <= 64).unwrap_or_else(|| levels.last().expect("one at least"));
        let from: Vec<(Vec3, f64, [f32; 3])> = (0..small.h)
            .flat_map(|y| (0..small.w).map(move |x| (x, y)))
            .map(|(x, y)| {
                let (u, v) = ((x as f64 + 0.5) / small.w as f64, (y as f64 + 0.5) / small.h as f64);
                (panorama_dir(u, v, 0.0), (v * std::f64::consts::PI).sin(), small.px[y * small.w + x])
            })
            .collect();
        let (fw, fh) = (32, 16);
        let falling = Pano {
            w: fw,
            h: fh,
            px: (0..fw * fh)
                .map(|i| {
                    let n = panorama_dir(((i % fw) as f64 + 0.5) / fw as f64, ((i / fw) as f64 + 0.5) / fh as f64, 0.0);
                    let (mut sum, mut weight) = ([0.0f64; 3], 0.0);
                    for (d, area, c) in &from {
                        let k = n.dot(*d).max(0.0) * area;
                        if k > 0.0 {
                            (0..3).for_each(|j| sum[j] += c[j] as f64 * k);
                            weight += k;
                        }
                    }
                    sum.map(|v| if weight > 0.0 { (v / weight) as f32 } else { 0.0 })
                })
                .collect(),
        };
        Some(Environment { levels, falling, rotation, occlusion: 0.0, occlusion_distance: 0.0, packed: std::sync::OnceLock::new() })
    }

    /// Its panoramas as the GPU reads them (light.metal's `Env`): their pixels one after
    /// another, the falling light's first, and each one's first pixel, width and height.
    pub fn packed(&self) -> &Packed {
        self.packed.get_or_init(|| {
            let (mut px, mut levels) = (Vec::new(), Vec::new());
            for p in std::iter::once(&self.falling).chain(&self.levels) {
                levels.push([px.len() as u32, p.w as u32, p.h as u32, 0]);
                px.extend(p.px.iter().map(|c| [c[0], c[1], c[2], 0.0]));
            }
            (px, levels)
        })
    }

    /// Its turn about the vertical (radians).
    pub fn rotation(&self) -> f64 {
        self.rotation
    }

    /// How many panoramas it shines with (the GPU's `Env::count`).
    pub fn shining(&self) -> usize {
        self.levels.len()
    }

    /// The light falling on a surface facing `n`.
    pub fn falling(&self, n: Vec3) -> [f32; 3] {
        let (u, v) = panorama_at(n, self.rotation);
        self.falling.at(u, v)
    }

    /// What a surface `rough` (0 a mirror, 1 matte) shines with along direction `d`.
    pub fn shine(&self, d: Vec3, rough: f32) -> [f32; 3] {
        let (u, v) = panorama_at(d, self.rotation);
        let at = rough.clamp(0.0, 1.0) * (self.levels.len() - 1) as f32;
        let (l0, f) = (at.floor() as usize, at - at.floor());
        let l1 = (l0 + 1).min(self.levels.len() - 1);
        let (a, b) = (self.levels[l0].at(u, v), self.levels[l1].at(u, v));
        std::array::from_fn(|k| a[k] + (b[k] - a[k]) * f)
    }

    /// The light it gives a surface of colour `rgb` facing `n` (toward `viewer`, a unit direction
    /// to the eye), `metal` and `rough` (0 to 1), taking `diffuse` of the light falling on it and
    /// `specular` of its shine (1: as it is), its nooks `occlusion` dark.
    #[allow(clippy::too_many_arguments)]
    pub fn light(&self, rgb: [f32; 3], n: Vec3, viewer: Vec3, metal: f32, rough: f32, diffuse: f32, specular: f32, occlusion: f32) -> [f32; 3] {
        let n = if n.dot(viewer) < 0.0 { -n } else { n };
        let nv = n.dot(viewer).clamp(1e-4, 1.0);
        let reflected = n * (2.0 * nv) - viewer;
        let (fall, shine) = (self.falling(n), self.shine(reflected, rough));
        // the share of light a surface shines back, by angle and roughness (Karis's fit of the
        // split-sum)
        let r = rough as f64;
        let c = [1.0 - r, r * -0.0275 + 0.0425, r * -0.572 + 1.04, r * 0.022 - 0.04];
        let a004 = (c[0] * c[0]).min((-9.28 * nv).exp2()) * c[0] + c[1];
        let (scale, bias) = ((-1.04 * a004 + c[2]) as f32, (1.04 * a004 + c[3]) as f32);
        std::array::from_fn(|k| {
            let f0 = 0.04 + (rgb[k] - 0.04) * metal;
            let back = f0 * scale + bias;
            (fall[k] * rgb[k] * (1.0 - back) * (1.0 - metal) * diffuse + shine[k] * back * specular) * occlusion
        })
    }
}

/// A composition's environment light: the layer its picture is from (none: the composition's
/// environment layer), its colour times its intensity in the working space, its turn (radians),
/// and its ambient occlusion (0 to 1) and how far that looks.
#[derive(Clone, Copy, Debug)]
pub struct EnvLight {
    pub source: Option<u64>,
    pub tint: [f32; 3],
    pub rotation: f64,
    pub occlusion: f32,
    pub occlusion_distance: f64,
}

/// A composition's environment light at `t` (the first that is on), its colour in the working
/// space (`linear`).
pub fn environment_light(eval: &Eval, comp: &Comp, t: Tick, linear: bool) -> Option<EnvLight> {
    let layer = comp.layers.iter().find(|l| l.switches.video && l.is_active_at(t) && l.source == Source::Light(LightKind::Environment))?;
    let at = eval.place(comp, layer);
    let g = layer.props.group("lightOptions").ok()?;
    let value = |id: &str| g.prop(id).ok().map(|p| eval.value(p, at, t));
    let intensity = value("intensity").map_or(1.0, |v| v.as_f64() / 100.0);
    let c = value("color").map_or(veymelo_color::Rgba::WHITE, |v| v.as_color());
    let ch = |v: f64| (if linear { veymelo_color::srgb_to_linear(v) } else { v } * intensity) as f32;
    let source = value("source").and_then(|v| v.as_layer());
    let occlusion = (value("ambientOcclusion").map_or(0.0, |v| v.as_f64()) / 100.0).clamp(0.0, 1.0) as f32;
    let distance = value("occlusionDistance").map_or(60.0, |v| v.as_f64()).max(0.0);
    Some(EnvLight { source, tint: [ch(c.r), ch(c.g), ch(c.b)], rotation: value("rotation").map_or(0.0, |v| v.as_f64()).to_radians(), occlusion, occlusion_distance: distance })
}
