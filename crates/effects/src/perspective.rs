//! Perspective: shadows (Drop Shadow, Radial Shadow), raised edges (Bevel Alpha, Bevel Edges),
//! the layer wrapped round a cylinder or a ball (Cylinder, Sphere), lit by a spot (Spotlight),
//! mirroring a surrounding picture (Environment), and stereo pairs (3D Glasses).
//!
//! Angles go clockwise from up: a light at angle `a` is toward (sin a, −cos a), y growing down.
//! In 3D, x is right, y down and z toward the viewer.

use std::f64::consts::{PI, TAU};

use veymelo_raster::filter::{self, Edge};
use veymelo_raster::{Image, Px, Sampling};

use crate::color::rgb3;
use crate::defs::{angle, choice, color, effect, flag, layer, pct, point, ranged};
use crate::distort::rect_px;
use crate::util::{luma, map_px, premul, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "perspective";

fn unit3(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    [v[0] / l, v[1] / l, v[2] / l]
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Toward a light at `angle` (clockwise from up) and `up` degrees above the layer.
fn toward(angle: f64, up: f64) -> [f64; 3] {
    let (a, e) = (angle.to_radians(), up.to_radians());
    unit3([a.sin() * e.cos(), -a.cos() * e.cos(), e.sin()])
}

fn over(top: Px, under: Px) -> Px {
    let k = 1.0 - top[3];
    [top[0] + under[0] * k, top[1] + under[1] * k, top[2] + under[2] * k, top[3] + under[3] * k]
}

/// The shadow put under the layer (or alone).
fn under_layer(shadow: &mut Image, layer: &Image, alone: bool) {
    if alone {
        return;
    }
    for (s, o) in shadow.px.iter_mut().zip(&layer.px) {
        *s = over(*o, *s);
    }
}

effect!(
    DROP_SHADOW_DEF,
    "dropShadow",
    "Drop Shadow",
    "a soft shadow of the layer's shape behind it",
    [
        color("shadowColor", "Shadow Color", 0.0, 0.0, 0.0),
        pct("opacity", "Opacity", 50.0, 0.0, 100.0),
        angle("direction", "Direction", 135.0).doc("where the shadow falls (clockwise from up)"),
        ranged("distance", "Distance", 5.0, 0.0, 32_000.0),
        ranged("softness", "Softness", 0.0, 0.0, 1000.0),
        flag("shadowOnly", "Shadow Only", false),
    ]
);
pub const DROP_SHADOW: Spec = Spec::new(&DROP_SHADOW_DEF, CATEGORY, drop_shadow);

fn drop_shadow(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = ctx.color("shadowColor");
    let opacity = (ctx.num("opacity", 50.0) / 100.0) as f32 * c.a as f32;
    let dir = ctx.num("direction", 135.0).to_radians();
    let dist = ctx.num("distance", 5.0) * b.scale;
    let soft = ctx.num("softness", 0.0) * b.scale;
    if !ctx.adjustment {
        b.pad((dist + soft * 1.5).ceil().min(4096.0) as usize + 2);
    }
    let (dx, dy) = (dir.sin() * dist, -dir.cos() * dist);
    let col = rgb3(c);
    let mut shadow = Image::new(b.img.width, b.img.height);
    map_px(&mut shadow, |x, y, _| {
        let a = b.img.sample(Sampling::Bilinear, x as f64 + 0.5 - dx, y as f64 + 0.5 - dy)[3] * opacity;
        [col[0] * a, col[1] * a, col[2] * a, a]
    });
    if soft > 0.0 {
        shadow = filter::gaussian(&shadow, soft * 0.5, soft * 0.5, Edge::Clear);
    }
    under_layer(&mut shadow, &b.img, ctx.flag("shadowOnly"));
    b.img = shadow;
    b
}

effect!(
    RADIAL_SHADOW_DEF,
    "radialShadow",
    "Radial Shadow",
    "the layer's shadow cast from a point light onto what is behind it, bigger the farther behind",
    [
        color("shadowColor", "Shadow Color", 0.0, 0.0, 0.0),
        pct("opacity", "Opacity", 50.0, 0.0, 100.0),
        point("lightSource", "Light Source"),
        ranged("projectionDistance", "Projection Distance", 10.0, 0.0, 1000.0),
        ranged("softness", "Softness", 0.0, 0.0, 1000.0),
        choice("render", "Render", &["regular", "glassEdge"], 0).doc("glass edge: only the part see-through parts cast, coloured by them"),
        pct("colorInfluence", "Color Influence", 0.0, 0.0, 100.0),
        flag("shadowOnly", "Shadow Only", false),
        flag("resizeLayer", "Resize Layer", false),
    ]
);
pub const RADIAL_SHADOW: Spec = Spec::new(&RADIAL_SHADOW_DEF, CATEGORY, radial_shadow).points(&[("lightSource", [0.3, 0.2])]);

fn radial_shadow(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = ctx.color("shadowColor");
    let col = rgb3(c);
    let opacity = (ctx.num("opacity", 50.0) / 100.0) as f32 * c.a as f32;
    let k = 1.0 + ctx.num("projectionDistance", 10.0).max(0.0) / 100.0;
    let soft = ctx.num("softness", 0.0).max(0.0) * b.scale;
    let glass = ctx.choice("render") == 1;
    let influence = (ctx.num("colorInfluence", 0.0) / 100.0) as f32;
    let light = ctx.point("lightSource");
    if ctx.flag("resizeLayer") && !ctx.adjustment {
        let l = b.pixel(light);
        let (bw, bh) = (b.img.width as f64, b.img.height as f64);
        let grow = [(0.0, 0.0), (bw, 0.0), (0.0, bh), (bw, bh)].iter().map(|(cx, cy)| (cx - l.x).hypot(cy - l.y) * (k - 1.0)).fold(0.0, f64::max);
        b.pad((grow + soft * 1.5).ceil().min(4096.0) as usize + 1);
    }
    let l = b.pixel(light);
    let mut shadow = Image::new(b.img.width, b.img.height);
    map_px(&mut shadow, |x, y, _| {
        // the layer, k times bigger about the light
        let s = b.img.sample(Sampling::Bilinear, l.x + (x as f64 + 0.5 - l.x) / k, l.y + (y as f64 + 0.5 - l.y) / k);
        let (sc, sa) = unpremul(s);
        let a = if glass { (sa * (1.0 - sa) * 4.0).clamp(0.0, 1.0) } else { sa } * opacity;
        premul([0, 1, 2].map(|i| col[i] + (sc[i] - col[i]) * influence), a)
    });
    if soft > 0.0 {
        shadow = filter::gaussian(&shadow, soft * 0.5, soft * 0.5, Edge::Clear);
    }
    under_layer(&mut shadow, &b.img, ctx.flag("shadowOnly"));
    b.img = shadow;
    b
}

/// A pixel lit (`s` above 0: the light's colour added) or shaded (below 0: darker).
fn lit(p: Px, s: f32, light: [f32; 3]) -> Px {
    if s > 0.0 {
        [p[0] + light[0] * s * p[3], p[1] + light[1] * s * p[3], p[2] + light[2] * s * p[3], p[3]]
    } else if s < 0.0 {
        let k = (1.0 + s).max(0.0);
        [p[0] * k, p[1] * k, p[2] * k, p[3]]
    } else {
        p
    }
}

effect!(
    BEVEL_ALPHA_DEF,
    "bevelAlpha",
    "Bevel Alpha",
    "the layer's shape raised along its edges, lit from one side",
    [
        ranged("edgeThickness", "Edge Thickness", 2.0, 0.0, 200.0),
        angle("lightAngle", "Light Angle", -60.0),
        color("lightColor", "Light Color", 1.0, 1.0, 1.0),
        ranged("lightIntensity", "Light Intensity", 0.4, 0.0, 1.0),
    ]
);
pub const BEVEL_ALPHA: Spec = Spec::new(&BEVEL_ALPHA_DEF, CATEGORY, bevel_alpha);

fn bevel_alpha(ctx: &Ctx, mut b: Buf) -> Buf {
    let t = ctx.num("edgeThickness", 2.0) * b.scale;
    let strength = ctx.num("lightIntensity", 0.4) as f32;
    if t <= 0.0 || strength == 0.0 {
        return b;
    }
    let light = rgb3(ctx.color("lightColor"));
    let l = toward(ctx.num("lightAngle", -60.0), 45.0);
    let (w, h) = (b.img.width, b.img.height);
    let a: Vec<f32> = b.img.px.iter().map(|p| p[3]).collect();
    let hgt = crate::matte::soften(&a, w, h, t * 0.5);
    let at = |x: i64, y: i64| hgt[(y.clamp(0, h as i64 - 1) as usize) * w + x.clamp(0, w as i64 - 1) as usize] as f64;
    let k = t * 1.25;
    map_px(&mut b.img, |x, y, p| {
        if p[3] <= 0.0 {
            return p;
        }
        let (xi, yi) = (x as i64, y as i64);
        let (dx, dy) = ((at(xi + 1, yi) - at(xi - 1, yi)) * 0.5, (at(xi, yi + 1) - at(xi, yi - 1)) * 0.5);
        let n = unit3([-dx * k, -dy * k, 1.0]);
        lit(p, (dot3(n, l) - l[2]) as f32 * strength * 2.0, light)
    });
    b
}

effect!(
    BEVEL_EDGES_DEF,
    "bevelEdges",
    "Bevel Edges",
    "the layer's rectangle given chiselled edges, lit from one side",
    [
        ranged("edgeThickness", "Edge Thickness", 0.1, 0.0, 0.5).doc("of the layer's shorter side"),
        angle("lightAngle", "Light Angle", -60.0),
        color("lightColor", "Light Color", 1.0, 1.0, 1.0),
        ranged("lightIntensity", "Light Intensity", 0.25, 0.0, 1.0),
    ]
);
pub const BEVEL_EDGES: Spec = Spec::new(&BEVEL_EDGES_DEF, CATEGORY, bevel_edges);

fn bevel_edges(ctx: &Ctx, mut b: Buf) -> Buf {
    let (x0, y0, w, h) = rect_px(ctx, &b);
    let t = ctx.num("edgeThickness", 0.1).clamp(0.0, 0.5) * w.min(h);
    let strength = ctx.num("lightIntensity", 0.25) as f32;
    if t <= 0.0 || strength == 0.0 {
        return b;
    }
    let light = rgb3(ctx.color("lightColor"));
    let l = toward(ctx.num("lightAngle", -60.0), 45.0);
    // a 45° slope facing the nearest edge
    let tilt = std::f64::consts::FRAC_1_SQRT_2;
    map_px(&mut b.img, |x, y, p| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let d = [px - x0, x0 + w - px, py - y0, y0 + h - py];
        if d.iter().any(|v| *v < 0.0) {
            return p;
        }
        let (side, near) = d.iter().enumerate().fold((0, f64::MAX), |(bi, bv), (i, v)| if *v < bv { (i, *v) } else { (bi, bv) });
        if near >= t {
            return p;
        }
        let n2 = [[-1.0, 0.0], [1.0, 0.0], [0.0, -1.0], [0.0, 1.0]][side];
        lit(p, (dot3([n2[0] * tilt, n2[1] * tilt, tilt], l) - l[2]) as f32 * strength * 2.0, light)
    });
    b
}

/// The light and surface of Cylinder and Sphere.
struct Shading {
    to_light: [f64; 3],
    colour: [f32; 3],
    strength: f32,
    ambient: f32,
    diffuse: f32,
    specular: f32,
    shine: f32,
}

impl Shading {
    fn of(ctx: &Ctx) -> Shading {
        Shading {
            to_light: toward(ctx.num("lightDirection", -45.0), ctx.num("lightHeight", 50.0)),
            colour: rgb3(ctx.color("lightColor")),
            strength: (ctx.num("lightIntensity", 100.0) / 100.0) as f32,
            ambient: (ctx.num("ambient", 20.0) / 100.0) as f32,
            diffuse: (ctx.num("diffuse", 80.0) / 100.0) as f32,
            specular: (ctx.num("specular", 20.0) / 100.0) as f32,
            shine: 1.0 / (ctx.num("roughness", 0.05) as f32).max(0.001),
        }
    }

    fn shade(&self, p: Px, n: [f64; 3]) -> Px {
        let (c, a) = unpremul(p);
        if a <= 0.0 {
            return [0.0; 4];
        }
        let nl = dot3(n, self.to_light).max(0.0) as f32;
        let half = unit3([self.to_light[0], self.to_light[1], self.to_light[2] + 1.0]);
        let gloss = if nl > 0.0 { (dot3(n, half).max(0.0) as f32).powf(self.shine) * self.specular } else { 0.0 };
        premul([0, 1, 2].map(|i| c[i] * (self.ambient + self.diffuse * nl * self.colour[i] * self.strength) + gloss * self.colour[i] * self.strength), a)
    }
}

macro_rules! round_effect {
    ($def:ident, $id:literal, $name:literal, $doc:literal, [$($own:expr),*]) => {
        effect!(
            $def,
            $id,
            $name,
            $doc,
            [
                $($own,)*
                choice("render", "Render", &["full", "outside", "inside"], 0),
                ranged("lightIntensity", "Light Intensity", 100.0, 0.0, 400.0),
                color("lightColor", "Light Color", 1.0, 1.0, 1.0),
                ranged("lightHeight", "Light Height", 50.0, -90.0, 90.0),
                angle("lightDirection", "Light Direction", -45.0),
                ranged("ambient", "Ambient", 20.0, 0.0, 200.0),
                ranged("diffuse", "Diffuse", 80.0, 0.0, 200.0),
                ranged("specular", "Specular", 20.0, 0.0, 200.0),
                ranged("roughness", "Roughness", 0.05, 0.001, 0.5),
            ]
        );
    };
}

round_effect!(
    CYLINDER_DEF,
    "cylinder",
    "Cylinder",
    "the layer wrapped round an upright cylinder, which can be tipped and turned",
    [
        pct("radius", "Radius", 100.0, 0.0, 400.0).doc("100: the layer's width goes once round"),
        point("position", "Position"),
        angle("rotation", "Rotation", 0.0).doc("the picture turned round the cylinder"),
        angle("rotationX", "Rotation X", 0.0),
        angle("rotationY", "Rotation Y", 0.0),
        angle("rotationZ", "Rotation Z", 0.0)
    ]
);
pub const CYLINDER: Spec = Spec::new(&CYLINDER_DEF, CATEGORY, cylinder).points(&[("position", [0.5, 0.5])]);

/// Turned by z after y after x (degrees).
fn turned(rx: f64, ry: f64, rz: f64) -> [[f64; 3]; 3] {
    let (sx, cx) = rx.to_radians().sin_cos();
    let (sy, cy) = ry.to_radians().sin_cos();
    let (sz, cz) = rz.to_radians().sin_cos();
    let mx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let my = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let mz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    let mul = |a: [[f64; 3]; 3], b: [[f64; 3]; 3]| -> [[f64; 3]; 3] { std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum())) };
    mul(mz, mul(my, mx))
}

/// The layer as a texture at (u, v) from 0 to 1: round and round across, nothing past the top
/// and bottom.
fn texel(src: &Image, rect: (f64, f64, f64, f64), u: f64, v: f64) -> Px {
    let (x0, y0, w, h) = rect;
    if !(0.0..=1.0).contains(&v) {
        return [0.0; 4];
    }
    src.sample(Sampling::Bilinear, (x0 + u.rem_euclid(1.0) * w).clamp(x0 + 0.5, x0 + w - 0.5), (y0 + v * h).clamp(y0 + 0.5, y0 + h - 0.5))
}

fn cylinder(ctx: &Ctx, mut b: Buf) -> Buf {
    let rect = rect_px(ctx, &b);
    let (_, _, w, h) = rect;
    let r = (ctx.num("radius", 100.0) / 100.0 * w / TAU).max(0.5);
    let c = b.pixel(ctx.point("position"));
    let spin = ctx.num("rotation", 0.0).to_radians();
    let m = turned(ctx.num("rotationX", 0.0), ctx.num("rotationY", 0.0), ctx.num("rotationZ", 0.0));
    // the cylinder's own up, as the viewer sees it
    let axis = [m[0][1], m[1][1], m[2][1]];
    let a = 1.0 - axis[2] * axis[2];
    let mode = ctx.choice("render");
    let sh = Shading::of(ctx);
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let (dx, dy) = (x as f64 + 0.5 - c.x, y as f64 + 0.5 - c.y);
        // where the line of sight meets the cylinder's wall
        let da = dx * axis[0] + dy * axis[1];
        let bq = -da * axis[2];
        let disc = bq * bq - a * (dx * dx + dy * dy - da * da - r * r);
        if a <= 1e-10 || !disc.is_finite() || disc <= 0.0 {
            return [0.0; 4];
        }
        // about how far inside the outline (in pixels), for a smooth edge
        let edge = (disc / (2.0 * r * a.sqrt()) + 0.5).clamp(0.0, 1.0) as f32;
        let root = disc.sqrt();
        let face = |z: f64, back: bool| {
            let local = [m[0][0] * dx + m[1][0] * dy + m[2][0] * z, m[0][1] * dx + m[1][1] * dy + m[2][1] * z, m[0][2] * dx + m[1][2] * dy + m[2][2] * z];
            let u = (local[0].atan2(local[2]) + spin) / TAU + 0.5;
            let v = local[1] / h + 0.5;
            let s = if back { -1.0 } else { 1.0 };
            let (nx, nz) = (local[0] / r * s, local[2] / r * s);
            let n = [m[0][0] * nx + m[0][2] * nz, m[1][0] * nx + m[1][2] * nz, m[2][0] * nx + m[2][2] * nz];
            sh.shade(texel(&src, rect, u, v), n)
        };
        let front = || face((-bq + root) / a, false);
        let back = || face((-bq - root) / a, true);
        let p = match mode {
            1 => front(),
            2 => back(),
            _ => over(front(), back()),
        };
        p.map(|v| v * edge)
    });
    b
}

round_effect!(
    SPHERE_DEF,
    "sphere",
    "Sphere",
    "the layer wrapped round a ball, which can be turned",
    [ranged("radius", "Radius", 120.0, 0.0, 4000.0), point("offset", "Offset"), angle("rotationX", "Rotation X", 0.0), angle("rotationY", "Rotation Y", 0.0), angle("rotationZ", "Rotation Z", 0.0)]
);
pub const SPHERE: Spec = Spec::new(&SPHERE_DEF, CATEGORY, sphere).points(&[("offset", [0.5, 0.5])]);

fn sphere(ctx: &Ctx, mut b: Buf) -> Buf {
    let rect = rect_px(ctx, &b);
    let r = (ctx.num("radius", 120.0) * b.scale).max(0.5);
    let c = b.pixel(ctx.point("offset"));
    let m = turned(ctx.num("rotationX", 0.0), ctx.num("rotationY", 0.0), ctx.num("rotationZ", 0.0));
    let mode = ctx.choice("render");
    let sh = Shading::of(ctx);
    let src = b.img.clone();
    // a direction from the ball's middle, turned back, as longitude and latitude on the layer
    let tex = |p: [f64; 3]| {
        let t = [m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2], m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2], m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2]];
        texel(&src, rect, t[0].atan2(t[2]) / TAU + 0.5, t[1].clamp(-1.0, 1.0).asin() / PI + 0.5)
    };
    map_px(&mut b.img, |x, y, _| {
        let (sx, sy) = ((x as f64 + 0.5 - c.x) / r, (y as f64 + 0.5 - c.y) / r);
        let d = sx.hypot(sy);
        let edge = ((1.0 - d) * r + 0.5).clamp(0.0, 1.0) as f32;
        if edge <= 0.0 {
            return [0.0; 4];
        }
        let (sx, sy) = if d > 1.0 { (sx / d * 0.999, sy / d * 0.999) } else { (sx, sy) };
        let z = (1.0 - sx * sx - sy * sy).max(0.0).sqrt();
        let front = || sh.shade(tex([sx, sy, z]), [sx, sy, z]);
        let back = || sh.shade(tex([sx, sy, -z]), [-sx, -sy, z]);
        let p = match mode {
            1 => front(),
            2 => back(),
            _ => over(front(), back()),
        };
        p.map(|v| v * edge)
    });
    b
}

effect!(
    SPOTLIGHT_DEF,
    "spotlight",
    "Spotlight",
    "a spotlight shining onto the layer from above, the rest in the dark",
    [
        point("from", "From"),
        point("to", "To"),
        pct("height", "Height", 50.0, 0.0, 1000.0).doc("how high the light is, of the layer's longer side"),
        ranged("coneAngle", "Cone Angle", 20.0, 0.1, 89.9),
        pct("edgeSoftness", "Edge Softness", 25.0, 0.0, 100.0),
        color("color", "Color", 1.0, 1.0, 1.0),
        ranged("intensity", "Intensity", 100.0, 0.0, 400.0),
        choice("render", "Render", &["light", "lightOnly"], 0),
    ]
);
pub const SPOTLIGHT: Spec = Spec::new(&SPOTLIGHT_DEF, CATEGORY, spotlight).points(&[("from", [0.3, 0.2]), ("to", [0.5, 0.5])]);

fn spotlight(ctx: &Ctx, mut b: Buf) -> Buf {
    let (_, _, w, h) = rect_px(ctx, &b);
    let (from, to) = (b.pixel(ctx.point("from")), b.pixel(ctx.point("to")));
    let height = (ctx.num("height", 50.0) / 100.0 * w.max(h)).max(1e-3);
    let cone = ctx.num("coneAngle", 20.0).clamp(0.1, 89.9).to_radians();
    let soft = (ctx.num("edgeSoftness", 25.0) / 100.0).clamp(0.0, 1.0);
    let col = rgb3(ctx.color("color"));
    let k = (ctx.num("intensity", 100.0) / 100.0) as f32;
    let only = ctx.choice("render") == 1;
    let axis = unit3([to.x - from.x, to.y - from.y, -height]);
    let inner = cone * (1.0 - soft);
    map_px(&mut b.img, |x, y, p| {
        let v = unit3([x as f64 + 0.5 - from.x, y as f64 + 0.5 - from.y, -height]);
        let off = dot3(v, axis).clamp(-1.0, 1.0).acos();
        let t = ((off - inner) / (cone - inner).max(1e-9)).clamp(0.0, 1.0) as f32;
        let m = (1.0 - t * t * (3.0 - 2.0 * t)) * k;
        if only {
            let a = m.clamp(0.0, 1.0);
            [col[0] * a, col[1] * a, col[2] * a, a]
        } else {
            [p[0] * m * col[0], p[1] * m * col[1], p[2] * m * col[2], p[3]]
        }
    });
    b
}

effect!(
    ENVIRONMENT_DEF,
    "environment",
    "Environment",
    "a picture of the surroundings reflected in the layer, as if its brightness were bumps on a mirror",
    [
        layer("environment", "Environment").doc("none: the layer itself"),
        choice("mapping", "Mapping", &["spherical", "probe", "verticalCross"], 0),
        flag("filterEnvironment", "Filter Environment", true),
        pct("height", "Height", 10.0, 0.0, 100.0),
    ]
);
pub const ENVIRONMENT: Spec = Spec::new(&ENVIRONMENT_DEF, CATEGORY, environment);

/// Where a direction (x right, y down, z toward the viewer) is on the surroundings' picture,
/// from 0 to 1 each way.
fn env_at(mapping: u32, r: [f64; 3]) -> (f64, f64) {
    match mapping {
        // a mirror ball
        1 => {
            let m = 2.0 * (r[0] * r[0] + r[1] * r[1] + (r[2] + 1.0).powi(2)).sqrt().max(1e-9);
            (0.5 + r[0] / m, 0.5 + r[1] / m)
        }
        // a cube's faces laid out as a cross, three across and four down
        2 => {
            let (ax, ay, az) = (r[0].abs(), r[1].abs(), r[2].abs());
            let (col, row, u, v) = if ax >= ay && ax >= az {
                if r[0] > 0.0 { (2.0, 1.0, -r[2] / ax, r[1] / ax) } else { (0.0, 1.0, r[2] / ax, r[1] / ax) }
            } else if ay >= az {
                if r[1] < 0.0 { (1.0, 0.0, r[0] / ay, -r[2] / ay) } else { (1.0, 2.0, r[0] / ay, r[2] / ay) }
            } else if r[2] > 0.0 {
                (1.0, 1.0, r[0] / az, r[1] / az)
            } else {
                (1.0, 3.0, r[0] / az, -r[1] / az)
            };
            ((col + (u + 1.0) * 0.5) / 3.0, (row + (v + 1.0) * 0.5) / 4.0)
        }
        // longitude and latitude
        _ => (0.5 + r[0].atan2(r[2]) / TAU, 0.5 + r[1].clamp(-1.0, 1.0).asin() / PI),
    }
}

fn environment(ctx: &Ctx, mut b: Buf) -> Buf {
    let env = ctx.layer_over("environment", &b, Take::All, true).unwrap_or_else(|| b.img.clone());
    let env = if ctx.flag("filterEnvironment") { filter::gaussian(&env, 2.0 * b.scale, 2.0 * b.scale, Edge::Repeat) } else { env };
    let mapping = ctx.choice("mapping");
    let height = ctx.num("height", 10.0) / 100.0 * 20.0;
    let (w, h) = (b.img.width, b.img.height);
    // bumps: the brightness, rounded off at the alpha's edge
    let relief: Vec<f32> = b.img.px.iter().map(|p| (luma(unpremul(*p).0) * 0.5 + 0.5) * p[3]).collect();
    let bumps = crate::matte::soften(&relief, w, h, 1.5 * b.scale.max(0.25));
    let at = |x: i64, y: i64| bumps[(y.clamp(0, h as i64 - 1) as usize) * w + x.clamp(0, w as i64 - 1) as usize] as f64;
    let (lx, ly, lw, lh) = rect_px(ctx, &b);
    let s = b.scale.max(1e-6);
    map_px(&mut b.img, |x, y, p| {
        let a = p[3];
        if a <= 0.0 {
            return [0.0; 4];
        }
        let (xi, yi) = (x as i64, y as i64);
        let (gx, gy) = ((at(xi + 1, yi) - at(xi - 1, yi)) * 0.5 / s, (at(xi, yi + 1) - at(xi, yi - 1)) * 0.5 / s);
        let n = unit3([-gx * height, -gy * height, 1.0]);
        // the line of sight (into the screen) bounced off the surface
        let d = -n[2];
        let r = [-2.0 * d * n[0], -2.0 * d * n[1], 1.0 + 2.0 * d * n[2]];
        let (u, v) = env_at(mapping, r);
        let e = env.sample(Sampling::Bilinear, (lx + u.rem_euclid(1.0) * lw).clamp(0.5, w as f64 - 0.5), (ly + v.clamp(0.0, 1.0) * lh).clamp(0.5, h as f64 - 0.5));
        // straight colour, fading to black as the surroundings grow see-through
        let k = 1.0 / e[3].max(1e-3);
        premul([e[0] * k, e[1] * k, e[2] * k], a)
    });
    b
}

/// How 3D Glasses puts its two views together.
pub const VIEWS_3D: &[&str] = &["stereoPairSideBySide", "overUnder", "interlaceUpperLLowerR", "redGreenLR", "redBlueLR", "balancedColoredRedBlue", "balancedRedGreenLR", "balancedRedBlueLR", "difference"];

effect!(
    GLASSES_DEF,
    "glasses3d",
    "3D Glasses",
    "a left and a right view put together for 3D glasses: side by side, interlaced or as an anaglyph",
    [
        layer("leftView", "Left View").doc("none: this layer"),
        layer("rightView", "Right View").doc("none: this layer"),
        ranged("sceneConvergence", "Scene Convergence", 0.0, -1000.0, 1000.0),
        ranged("verticalAlignment", "Vertical Alignment", 0.0, -1000.0, 1000.0),
        choice("units", "Units", &["pixels", "percentOfSource"], 0),
        flag("swapLeftRight", "Swap Left-Right", false),
        choice("view3d", "3D View", VIEWS_3D, 5),
        ranged("balance", "Balance", 7.0, 0.0, 20.0),
    ]
);
pub const GLASSES: Spec = Spec::new(&GLASSES_DEF, CATEGORY, glasses);

fn glasses(ctx: &Ctx, mut b: Buf) -> Buf {
    let view_of = |id: &str| ctx.layer_over(id, &b, Take::All, false).unwrap_or_else(|| b.img.clone());
    let (mut l, mut r) = (view_of("leftView"), view_of("rightView"));
    if ctx.flag("swapLeftRight") {
        std::mem::swap(&mut l, &mut r);
    }
    let unit = if ctx.choice("units") == 1 { ctx.bounds.width() / 100.0 } else { 1.0 } * b.scale;
    let conv = ctx.num("sceneConvergence", 0.0) * unit;
    let va = ctx.num("verticalAlignment", 0.0) * unit;
    let shifted = |img: &Image, dx: f64, dy: f64| -> Image {
        if dx == 0.0 && dy == 0.0 {
            return img.clone();
        }
        let mut out = Image::new(img.width, img.height);
        map_px(&mut out, |x, y, _| img.sample(Sampling::Bilinear, x as f64 + 0.5 - dx, y as f64 + 0.5 - dy));
        out
    };
    let (l, r) = (shifted(&l, conv * 0.5, 0.0), shifted(&r, -conv * 0.5, va));
    let view = ctx.choice("view3d");
    let k = (ctx.num("balance", 7.0) / 20.0).clamp(0.0, 1.0) as f32;
    let (w, h) = (b.img.width as f64, b.img.height as f64);
    let lum = |c: [f32; 3]| luma(c);
    map_px(&mut b.img, |x, y, _| {
        let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
        match view {
            0 => {
                if fx < w * 0.5 {
                    l.sample(Sampling::Bilinear, fx * 2.0, fy)
                } else {
                    r.sample(Sampling::Bilinear, (fx - w * 0.5) * 2.0, fy)
                }
            }
            1 => {
                if fy < h * 0.5 {
                    l.sample(Sampling::Bilinear, fx, fy * 2.0)
                } else {
                    r.sample(Sampling::Bilinear, fx, (fy - h * 0.5) * 2.0)
                }
            }
            2 => {
                if y % 2 == 0 {
                    l.px[y * l.width + x]
                } else {
                    r.px[y * r.width + x]
                }
            }
            _ => {
                let ((lc, la), (rc, ra)) = (unpremul(l.px[y * l.width + x]), unpremul(r.px[y * r.width + x]));
                let (ll, rl) = (lum(lc), lum(rc));
                let c = match view {
                    3 => [ll, rl, 0.0],
                    4 => [ll, 0.0, rl],
                    5 => [lc[0] + (ll - lc[0]) * k, rc[1], rc[2]],
                    6 => [ll * (1.0 - 0.5 * k) + lc[0] * 0.5 * k, rl, 0.0],
                    7 => [ll * (1.0 - 0.5 * k) + lc[0] * 0.5 * k, 0.0, rl],
                    _ => [(lc[0] - rc[0]).abs(), (lc[1] - rc[1]).abs(), (lc[2] - rc[2]).abs()],
                };
                premul(c, la.max(ra))
            }
        }
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 9] = [DROP_SHADOW, RADIAL_SHADOW, BEVEL_ALPHA, BEVEL_EDGES, CYLINDER, SPHERE, SPOTLIGHT, ENVIRONMENT, GLASSES];

#[cfg(test)]
mod tests;
