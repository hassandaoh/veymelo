//! Water: a bottom seen through a rippled surface, bent by it and lit by the light it gathers
//! into bright lines, the surface itself lit and mirroring a sky (Caustics); and waves spreading
//! over water from ringing or swinging producers, slowed where it is shallow and stopped where
//! it is dry (Wave World), shown as a height map (to drive Caustics or a displacement) or as a
//! wireframe.

use std::f32::consts::TAU;

use veymelo_raster::filter::{self, Edge};
use veymelo_raster::{Image, Px, Sampling};
use veymelo_time::Tick;

use crate::defs::{angle, choice, color, effect, flag, layer, point, ranged};
use crate::stepper::{RATE, Stepped, key_of, step_at};
use crate::util::{luma, map_px, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "simulation";

/// A grey picture.
struct Field {
    w: usize,
    h: usize,
    v: Vec<f32>,
}

impl Field {
    fn get(&self, x: i64, y: i64) -> f32 {
        self.v[(y.clamp(0, self.h as i64 - 1) as usize) * self.w + x.clamp(0, self.w as i64 - 1) as usize]
    }

    /// Between pixels (their middles at whole numbers).
    fn at(&self, x: f32, y: f32) -> f32 {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let top = self.get(x0, y0) + (self.get(x0 + 1, y0) - self.get(x0, y0)) * fx;
        let bottom = self.get(x0, y0 + 1) + (self.get(x0 + 1, y0 + 1) - self.get(x0, y0 + 1)) * fx;
        top + (bottom - top) * fy
    }

    /// Its slope across and down, in its values a pixel.
    fn slope(&self, x: usize, y: usize) -> (f32, f32) {
        let (x, y) = (x as i64, y as i64);
        ((self.get(x + 1, y) - self.get(x - 1, y)) * 0.5, (self.get(x, y + 1) - self.get(x, y - 1)) * 0.5)
    }

    /// Smoothed by a gaussian `sigma` pixels wide.
    fn smooth(self, sigma: f64) -> Field {
        if sigma < 0.05 {
            return self;
        }
        let mut img = Image::new(self.w, self.h);
        for (p, &g) in img.px.iter_mut().zip(&self.v) {
            *p = [g, g, g, 1.0];
        }
        let soft = filter::gaussian(&img, sigma, sigma, Edge::Repeat);
        Field { w: self.w, h: self.h, v: soft.px.iter().map(|p| p[0]).collect() }
    }
}

/// A picture sampled once (clear past it), tiled, or reflected at its edges.
fn repeat(img: &Image, x: f64, y: f64, how: u32) -> Px {
    let (w, h) = (img.width as f64, img.height as f64);
    let fold = |v: f64, n: f64| {
        let r = v.rem_euclid(2.0 * n);
        if r > n { 2.0 * n - r } else { r }
    };
    match how {
        1 => img.sample(Sampling::Bilinear, x.rem_euclid(w).clamp(0.5, w - 0.5), y.rem_euclid(h).clamp(0.5, h - 0.5)),
        2 => img.sample(Sampling::Bilinear, fold(x, w).clamp(0.5, w - 0.5), fold(y, h).clamp(0.5, h - 0.5)),
        _ => img.sample(Sampling::Bilinear, x, y),
    }
}

effect!(
    CAUSTICS_DEF,
    "caustics",
    "Caustics",
    "a bottom seen through rippling water: bent by the surface, lit by the bright lines it gathers, the surface lit and mirroring a sky",
    [
        layer("bottom", "Bottom").doc("none: the layer"),
        ranged("bottomScaling", "Bottom Scaling", 1.0, 0.01, 10.0),
        choice("bottomRepeatMode", "Bottom Repeat Mode", &["once", "tiled", "reflected"], 2),
        choice("bottomSizeDiffers", "If Bottom Size Differs", &["center", "stretchToFit"], 1),
        ranged("bottomBlur", "Bottom Blur", 0.0, 0.0, 100.0),
        layer("waterSurface", "Water Surface").doc("its brightness is the surface's height; none: still water"),
        ranged("waveHeight", "Wave Height", 0.2, -1.0, 1.0),
        ranged("smoothing", "Smoothing", 5.0, 0.0, 100.0),
        ranged("waterDepth", "Water Depth", 0.1, 0.0, 1.0),
        ranged("refractiveIndex", "Refractive Index", 1.2, 1.0, 3.0),
        color("surfaceColor", "Surface Color", 1.0, 1.0, 1.0),
        ranged("surfaceOpacity", "Surface Opacity", 0.3, 0.0, 1.0),
        ranged("causticsStrength", "Caustics Strength", 0.0, 0.0, 1.0),
        layer("sky", "Sky"),
        ranged("skyScaling", "Sky Scaling", 1.0, 0.01, 10.0),
        choice("skyRepeatMode", "Sky Repeat Mode", &["once", "tiled", "reflected"], 2),
        choice("skySizeDiffers", "If Sky Size Differs", &["center", "stretchToFit"], 1),
        ranged("skyIntensity", "Sky Intensity", 0.3, 0.0, 1.0),
        ranged("skyConvergence", "Sky Convergence", 0.5, 0.0, 1.0),
        choice("lightType", "Light Type", &["distantSource", "pointSource", "firstCompLight"], 0),
        ranged("lightIntensity", "Light Intensity", 1.0, 0.0, 4.0),
        color("lightColor", "Light Color", 1.0, 1.0, 1.0),
        point("lightPosition", "Light Position"),
        ranged("lightHeight", "Light Height", 1.0, 0.0, 10.0),
        ranged("ambientLight", "Ambient Light", 0.35, 0.0, 2.0),
        ranged("diffuseReflection", "Diffuse Reflection", 0.75, 0.0, 2.0),
        ranged("specularReflection", "Specular Reflection", 0.2, 0.0, 2.0),
        ranged("highlightSharpness", "Highlight Sharpness", 15.0, 1.0, 100.0),
    ]
);
pub const CAUSTICS: Spec = Spec::new(&CAUSTICS_DEF, CATEGORY, caustics).points(&[("lightPosition", [0.0, 0.0])]);

fn caustics(ctx: &Ctx, mut b: Buf) -> Buf {
    let s = b.scale;
    let (w, h) = (b.img.width, b.img.height);
    let lw = ctx.bounds.width() * s;
    let mid = b.pixel(ctx.bounds.center());
    let mut bottom = ctx.layer_over("bottom", &b, Take::All, ctx.choice("bottomSizeDiffers") == 1).unwrap_or_else(|| b.img.clone());
    let blur = ctx.num("bottomBlur", 0.0) * s;
    if blur > 0.1 {
        bottom = filter::gaussian(&bottom, blur * 0.5, blur * 0.5, Edge::Repeat);
    }
    // the surface's height: the water layer's brightness, smoothed (none: still water)
    let surface_layer = ctx.layer_over("waterSurface", &b, Take::All, true);
    let height = surface_layer.map(|img| Field { w, h, v: img.px.iter().map(|p| luma(unpremul(*p).0) * p[3]).collect() }.smooth(ctx.num("smoothing", 5.0) * s * 0.5));
    let slope = |x: usize, y: usize| height.as_ref().map_or((0.0, 0.0), |f| f.slope(x, y));
    let (wave, depth, index) = (ctx.num("waveHeight", 0.2) as f32, ctx.num("waterDepth", 0.1), ctx.num("refractiveIndex", 1.2).max(1.0));
    // the surface's tilt: its height a quarter of the layer's width (times Wave Height) from
    // black to white, so a bigger layer with the same water looks the same
    let lift = wave * 0.25 * lw as f32;
    let tilt = |x: usize, y: usize| {
        let (gx, gy) = slope(x, y);
        (gx * lift, gy * lift)
    };
    // a ray straight down (the light's, or the eye's) bent as it enters the water, then going
    // down to the bottom: toward the surface's higher side, so a crest gathers the light like a
    // lens, and shows the bottom under it larger
    let deep = (depth * lw) as f32;
    let shift: Vec<(f32, f32)> = (0..w * h)
        .map(|i| {
            let (tx, ty) = tilt(i % w, i / w);
            let t = (tx * tx + ty * ty).sqrt();
            if t < 1e-9 {
                return (0.0, 0.0);
            }
            let into = t.atan();
            let bent = (into.sin() / index as f32).asin();
            let away = deep * (into - bent).tan() / t;
            (tx * away, ty * away)
        })
        .collect();
    // the light on the bottom: each point of the surface sends its share down to where its ray
    // lands, so the light pools where the surface gathers it and thins where it spreads it
    let strength = ctx.num("causticsStrength", 0.0) as f32;
    let pooled: Option<Field> = (strength > 0.0 && height.is_some()).then(|| {
        let mut g = vec![0f32; w * h];
        for (i, &(dx, dy)) in shift.iter().enumerate() {
            let (x, y) = ((i % w) as f32 + dx, (i / w) as f32 + dy);
            let (x0, y0) = (x.floor(), y.floor());
            let (fx, fy) = (x - x0, y - y0);
            for (ox, oy, k) in [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)] {
                let (px, py) = (x0 as i64 + ox, y0 as i64 + oy);
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    g[py as usize * w + px as usize] += k;
                }
            }
        }
        // softened a little, as the light is not a point
        Field { w, h, v: g }.smooth(0.7 * s.max(0.5))
    });
    let (scaling, how) = (ctx.num("bottomScaling", 1.0).max(0.01), ctx.choice("bottomRepeatMode"));
    let sky = ctx.layer_over("sky", &b, Take::All, ctx.choice("skySizeDiffers") == 1);
    let (sky_scale, sky_how, mirror, converge) = (ctx.num("skyScaling", 1.0).max(0.01), ctx.choice("skyRepeatMode"), ctx.num("skyIntensity", 0.3).clamp(0.0, 1.0) as f32, ctx.num("skyConvergence", 0.5).clamp(0.0, 1.0));
    let sc = ctx.color("surfaceColor");
    let surface = [sc.r as f32, sc.g as f32, sc.b as f32];
    let opacity = ctx.num("surfaceOpacity", 0.3).clamp(0.0, 1.0) as f32;
    let lc = ctx.color("lightColor");
    let mut tint = [lc.r as f32, lc.g as f32, lc.b as f32];
    let mut li = ctx.num("lightIntensity", 1.0) as f32;
    let mut lp = b.pixel(ctx.point("lightPosition"));
    let (mut lift, mut point) = (ctx.num("lightHeight", 1.0).max(0.01), ctx.choice("lightType") == 1);
    if ctx.choice("lightType") == 2 {
        match ctx.host.and_then(|h| h.scene()).and_then(|sc| sc.light) {
            // the composition's first light: its height above the water (toward the viewer)
            // in layer widths
            Some(l) if l.kind != 3 => {
                (tint, li) = (l.color, 1.0);
                if l.kind == 0 {
                    // a parallel light: from far along the way back up its beam
                    let far = lw.max(1.0) * 1000.0;
                    lp = veymelo_geom::vec2(mid.x - l.dir[0] * far, mid.y - l.dir[1] * far);
                    (lift, point) = ((l.dir[2] * far / lw.max(1.0)).max(0.01), false);
                } else {
                    let (s, o) = (b.scale, b.origin);
                    lp = veymelo_geom::vec2((l.at[0] - o.x) * s, (l.at[1] - o.y) * s);
                    (lift, point) = ((-l.at[2] * s / lw.max(1.0)).max(0.01), true);
                }
            }
            Some(l) => (tint, li) = (l.color, 0.0),
            None => {
                ctx.note("the composition has no light: ambient light only".into());
                li = 0.0;
            }
        }
    }
    let (ambient, diffuse, specular, sharp) = (ctx.num("ambientLight", 0.35) as f32, ctx.num("diffuseReflection", 0.75) as f32, ctx.num("specularReflection", 0.2) as f32, ctx.num("highlightSharpness", 15.0).max(1.0) as f32);
    let unit = |v: [f64; 3]| {
        let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
        [(v[0] / n) as f32, (v[1] / n) as f32, (v[2] / n) as f32]
    };
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    // toward the light: from the point (a point light) or from the light over the middle
    let toward = |x: f64, y: f64| {
        let (fx, fy) = if point { (x, y) } else { (mid.x, mid.y) };
        unit([(lp.x - fx) / lw.max(1.0), (lp.y - fy) / lw.max(1.0), lift])
    };
    // the bottom's point at (x, y) on screen, scaled about the middle
    let scaled = |x: f32, y: f32, k: f64| (mid.x + (x as f64 - mid.x) / k, mid.y + (y as f64 - mid.y) / k);
    map_px(&mut b.img, |x, y, _| {
        let (dx, dy) = shift[y * w + x];
        // where the bent ray meets the bottom
        let (bx, by) = (x as f32 + 0.5 + dx, y as f32 + 0.5 + dy);
        let (sx, sy) = scaled(bx, by, scaling);
        let (c, ba) = unpremul(repeat(&bottom, sx, sy, how));
        // the surface's tilt there, and the light it lets through and shines back
        let (tx, ty) = tilt(x, y);
        let n = unit([-tx as f64, -ty as f64, 1.0]);
        let l = toward(x as f64 + 0.5, y as f64 + 0.5);
        let lit = dot(n, l).max(0.0);
        let shine = specular * li * dot(n, unit([l[0] as f64, l[1] as f64, l[2] as f64 + 1.0])).max(0.0).powf(sharp);
        let gathered = pooled.as_ref().map_or(0.0, |p| (p.at(bx - 0.5, by - 0.5) - 1.0).clamp(-1.0, 4.0) * strength * li);
        let light = ambient + diffuse * li * lit + gathered;
        let top = [0, 1, 2].map(|k| surface[k] * (ambient + diffuse * li * lit) * tint[k]);
        let mut o = [0, 1, 2].map(|k| c[k] * light * tint[k] * (1.0 - opacity) + top[k] * opacity);
        // the sky mirrored, along the surface's tilt (Convergence draws it to the point itself)
        let mut a = ba + opacity * (1.0 - ba);
        if let Some(sk) = &sky {
            let reach = (1.0 - converge) as f32 * lw as f32 * 0.5;
            let (kx, ky) = scaled(x as f32 + 0.5 + n[0] * reach, y as f32 + 0.5 + n[1] * reach, sky_scale);
            let (sky_c, sky_a) = unpremul(repeat(sk, kx, ky, sky_how));
            let k = mirror * sky_a;
            o = [0, 1, 2].map(|q| o[q] * (1.0 - k) + sky_c[q] * surface[q] * k);
            a += k * (1.0 - a);
        }
        let o = [0, 1, 2].map(|k| o[k] + shine * tint[k]);
        [o[0] * a, o[1] * a, o[2] * a, a]
    });
    b
}

macro_rules! wave_world_def {
    (producers: [$(($n:literal, $amp:literal)),*]) => {
        pub static WAVE_WORLD_DEF: veymelo_project::GroupDef = veymelo_project::GroupDef {
            id: "waveWorld",
            name: "Wave World",
            doc: "waves spreading over water from producers, slowed where the ground is shallow and stopped where it is dry, as a height map or a wireframe",
            body: veymelo_project::Body::Fixed(&[
                veymelo_project::DefNode::Prop(choice("view", "View", &["wireframePreview", "heightMap"], 0)),
                veymelo_project::DefNode::Prop(angle("horizontalRotation", "Horizontal Rotation", 0.0)),
                veymelo_project::DefNode::Prop(ranged("verticalRotation", "Vertical Rotation", 53.0, 0.0, 90.0)),
                veymelo_project::DefNode::Prop(ranged("verticalScale", "Vertical Scale", 0.5, 0.0, 10.0)),
                veymelo_project::DefNode::Prop(ranged("brightness", "Brightness", 0.5, -1.0, 2.0)),
                veymelo_project::DefNode::Prop(ranged("contrast", "Contrast", 0.75, 0.0, 4.0)),
                veymelo_project::DefNode::Prop(ranged("gammaAdjustment", "Gamma Adjustment", 1.0, 0.01, 10.0)),
                veymelo_project::DefNode::Prop(choice("renderDryAreasAs", "Render Dry Areas As", &["solid", "transparent"], 0)),
                veymelo_project::DefNode::Prop(ranged("transparency", "Transparency", 0.0, 0.0, 1.0)),
                veymelo_project::DefNode::Prop(ranged("gridResolution", "Grid Resolution", 60.0, 1.0, 400.0)),
                veymelo_project::DefNode::Prop(flag("gridResDownsamples", "Grid Res Downsamples", false).doc("a coarser grid in smaller previews")),
                veymelo_project::DefNode::Prop(ranged("waveSpeed", "Wave Speed", 0.2, 0.0, 5.0).doc("layer widths a second")),
                veymelo_project::DefNode::Prop(ranged("damping", "Damping", 0.05, 0.0, 5.0)),
                veymelo_project::DefNode::Prop(choice("reflectEdges", "Reflect Edges", &["none", "left", "top", "right", "bottom", "all"], 0)),
                veymelo_project::DefNode::Prop(ranged("preRoll", "Pre-roll (seconds)", 0.0, 0.0, 30.0)),
                veymelo_project::DefNode::Prop(layer("ground", "Ground").doc("its brightness is how high the bottom is")),
                veymelo_project::DefNode::Prop(ranged("steepness", "Steepness", 0.25, 0.0, 2.0)),
                veymelo_project::DefNode::Prop(ranged("groundHeight", "Height", 0.25, 0.0, 2.0)),
                veymelo_project::DefNode::Prop(ranged("waveStrength", "Wave Strength", 0.0, 0.0, 1.0).doc("how much the ground's slopes push the water")),
                $(
                    veymelo_project::DefNode::Prop(choice(concat!("producer", $n, "Type"), concat!("Producer ", $n, " Type"), &["ring", "line"], 0)),
                    veymelo_project::DefNode::Prop(point(concat!("producer", $n, "Position"), concat!("Producer ", $n, " Position"))),
                    veymelo_project::DefNode::Prop(ranged(concat!("producer", $n, "Height"), concat!("Producer ", $n, " Height/Length"), 0.1, 0.0, 2.0)),
                    veymelo_project::DefNode::Prop(ranged(concat!("producer", $n, "Width"), concat!("Producer ", $n, " Width"), 0.1, 0.0, 2.0)),
                    veymelo_project::DefNode::Prop(angle(concat!("producer", $n, "Angle"), concat!("Producer ", $n, " Angle"), 0.0)),
                    veymelo_project::DefNode::Prop(ranged(concat!("producer", $n, "Amplitude"), concat!("Producer ", $n, " Amplitude"), $amp, -2.0, 2.0)),
                    veymelo_project::DefNode::Prop(ranged(concat!("producer", $n, "Frequency"), concat!("Producer ", $n, " Frequency"), 1.0, 0.0, 30.0)),
                    veymelo_project::DefNode::Prop(angle(concat!("producer", $n, "Phase"), concat!("Producer ", $n, " Phase"), 0.0)),
                )*
            ]),
        };
    };
}

wave_world_def!(producers: [("1", 0.5), ("2", 0.0)]);

pub const WAVE_WORLD: Spec = Spec::new(&WAVE_WORLD_DEF, CATEGORY, wave_world).points(&[("producer1Position", [0.5, 0.5]), ("producer2Position", [0.25, 0.25])]);

/// The water's height on the grid now and a step before.
#[derive(Clone, Debug, Default)]
pub struct Waves {
    now: Vec<f32>,
    before: Vec<f32>,
}

/// A wave producer at a moment, in grid cells.
struct Producer {
    line: bool,
    at: [f32; 2],
    /// Half its size along its own x and y (a ring: its width and height; a line: its length
    /// and width).
    half: [f32; 2],
    turn: f32,
    amplitude: f32,
    frequency: f32,
    phase: f32,
}

impl Producer {
    /// How much of grid point (x, y) it holds: wholly at its middle, nothing past its edge.
    fn hold(&self, x: f32, y: f32) -> f32 {
        let (dx, dy) = (x - self.at[0], y - self.at[1]);
        let (sn, cs) = self.turn.sin_cos();
        let (u, v) = (dx * cs + dy * sn, -dx * sn + dy * cs);
        let (a, w) = (self.half[0].max(0.5), self.half[1].max(0.5));
        let d = if self.line { (u.abs() / a).max(v.abs() / w) } else { ((u / a).powi(2) + (v / w).powi(2)).sqrt() };
        (1.0 - d).clamp(0.0, 1.0)
    }
}

/// The producers that move the water at layer time `t`: `cells` grid cells to the layer's
/// width.
fn producers(ctx: &Ctx, t: f64, cells: f32) -> Vec<Producer> {
    let r = ctx.bounds;
    let tt = Tick::from_seconds(t);
    let num = |id: &str, d: f64| ctx.value_at(id, tt).map_or(d, |v| v.as_f64()) as f32;
    let per = cells / r.width().max(1e-9) as f32;
    (1..=2)
        .filter_map(|n| {
            let id = |what: &str| format!("producer{n}{what}");
            let amplitude = num(&id("Amplitude"), 0.0);
            if amplitude == 0.0 {
                return None;
            }
            let p = ctx.value_at(&id("Position"), tt).map_or([0.0; 2], |v| v.as_vec2());
            let line = ctx.choice(&id("Type")) == 1;
            let (along, across) = (num(&id("Height"), 0.1) * cells * 0.5, num(&id("Width"), 0.1) * cells * 0.5);
            Some(Producer {
                line,
                at: [(p[0] - r.x0) as f32 * per, (p[1] - r.y0) as f32 * per],
                half: if line { [along, across] } else { [across, along] },
                turn: num(&id("Angle"), 0.0).to_radians(),
                amplitude,
                frequency: num(&id("Frequency"), 1.0),
                phase: num(&id("Phase"), 0.0).to_radians(),
            })
        })
        .collect()
}

static KEPT: Stepped<Waves> = Stepped::new();

/// Water shallower than this (of the ground's Height) is none.
const DRY: f32 = 1e-4;

fn wave_world(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = ctx.bounds;
    let mut res = ctx.num("gridResolution", 60.0).clamp(1.0, 400.0);
    if ctx.flag("gridResDownsamples") {
        res = (res * b.scale).round().max(1.0);
    }
    let cols = res.round() as usize + 1;
    let rows = ((res * r.height() / r.width().max(1e-9)).round() as usize).max(1) + 1;
    let (n, cells) = (cols * rows, (cols - 1) as f32);
    // the ground as it is at the layer's start (the water does not follow it later): its
    // brightness raises the bottom; where it comes up through the water it is dry
    let level = ctx.num("groundHeight", 0.25) as f32;
    let ground = ctx.value_in(ctx.effect, "ground").and_then(|v| v.as_layer()).and_then(|id| ctx.host?.layer_at(id, -ctx.time, Take::All));
    let depth: Option<Vec<f32>> = ground.map(|(g, gr)| {
        let steep = ctx.num("steepness", 0.25) as f32;
        (0..n)
            .map(|i| {
                let (fx, fy) = ((i % cols) as f64 / (cols - 1) as f64, (i / cols) as f64 / (rows - 1) as f64);
                let q = g.pixel(veymelo_geom::vec2(gr.x0 + fx * gr.width(), gr.y0 + fy * gr.height()));
                // the edge points from the ground's edge pixels (not half from past it)
                let (qx, qy) = (q.x.clamp(0.5, (g.img.width as f64 - 0.5).max(0.5)), q.y.clamp(0.5, (g.img.height as f64 - 0.5).max(0.5)));
                let (c, a) = unpremul(g.img.sample(Sampling::Bilinear, qx, qy));
                level - steep * luma(c) * a
            })
            .collect()
    });
    // how quick the waves are in each cell, of their speed in deep water: none where it is dry,
    // slower where it is shallow (by Wave Strength)
    let strength = ctx.num("waveStrength", 0.0).clamp(0.0, 1.0) as f32;
    let quick: Vec<f32> = match &depth {
        Some(d) => d.iter().map(|&v| if v <= DRY { 0.0 } else { 1.0 - strength * (1.0 - (v / level.max(1e-3)).clamp(0.0, 1.0).sqrt()) }).collect(),
        None => vec![1.0; n],
    };
    let reflect = ctx.choice("reflectEdges");
    // left, top, right, bottom: does the side let the waves out?
    let open = [1, 2, 3, 4].map(|side| reflect != 5 && reflect != side);
    let preroll = ctx.num("preRoll", 0.0).max(0.0);
    let mut key = vec![key_of(ctx, 6), cols as u64, rows as u64, r.x0.to_bits(), r.y0.to_bits(), r.width().to_bits(), r.height().to_bits()];
    key.extend(quick.iter().map(|v| v.to_bits() as u64));
    let state = KEPT.run(crate::noise::mix(&key), step_at(ctx.time.seconds() + preroll), || Waves { now: vec![0.0; n], before: vec![0.0; n] }, |w, k| {
        let when = k as f64 / RATE - preroll;
        let tt = Tick::from_seconds(when);
        // in cells a second
        let speed = ctx.value_at("waveSpeed", tt).map_or(0.2, |v| v.as_f64()).max(0.0) as f32 * cells;
        let damp = ctx.value_at("damping", tt).map_or(0.05, |v| v.as_f64()).max(0.0) as f32 * 4.0;
        let dt = (1.0 / RATE) as f32;
        // small enough steps that no wave outruns half a cell in one
        let subs = ((speed * dt / 0.5).ceil() as usize).clamp(1, 64);
        let h = dt / subs as f32;
        let c2: Vec<f32> = quick.iter().map(|q| (q * speed) * (q * speed)).collect();
        let drivers = producers(ctx, when, cells);
        let holds: Vec<Vec<(usize, f32)>> = drivers
            .iter()
            .map(|p| {
                let reach = p.half[0].max(p.half[1]).max(0.5) + 1.0;
                let (x0, x1) = ((p.at[0] - reach).floor().max(0.0) as usize, ((p.at[0] + reach).ceil().max(0.0) as usize).min(cols - 1));
                let (y0, y1) = ((p.at[1] - reach).floor().max(0.0) as usize, ((p.at[1] + reach).ceil().max(0.0) as usize).min(rows - 1));
                (y0..=y1).flat_map(|y| (x0..=x1).map(move |x| (x, y))).filter_map(|(x, y)| {
                    let k = p.hold(x as f32, y as f32);
                    (k > 0.0 && c2[y * cols + x] > 0.0).then_some((y * cols + x, k))
                }).collect()
            })
            .collect();
        let d = damp * h * 0.5;
        for sub in 0..subs {
            let time = when as f32 + sub as f32 * h;
            let mut next = vec![0f32; n];
            for y in 0..rows {
                for x in 0..cols {
                    let i = y * cols + x;
                    let out = (x == 0 && open[0]) || (y == 0 && open[1]) || (x == cols - 1 && open[2]) || (y == rows - 1 && open[3]);
                    if c2[i] <= 0.0 || out {
                        continue;
                    }
                    // what flows in from each wet neighbour (none across a closed edge or from
                    // dry ground, so the waves come back from them)
                    let u = w.now[i];
                    let mut flow = 0f32;
                    for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
                        if nx < cols && ny < rows {
                            let j = ny * cols + nx;
                            if c2[j] > 0.0 {
                                flow += 0.5 * (c2[i] + c2[j]) * (w.now[j] - u);
                            }
                        }
                    }
                    next[i] = (2.0 * u - w.before[i] * (1.0 - d) + h * h * flow) / (1.0 + d);
                }
            }
            // the open edges carry the waves on out of the layer, from the cell inside them;
            // the sides first, then the top and bottom (whose ends meet the sides)
            let carry = |i: usize, inner: usize, next: &mut Vec<f32>| {
                if c2[i] > 0.0 {
                    let c = c2[i].sqrt() * h;
                    next[i] = w.now[inner] + (c - 1.0) / (c + 1.0) * (next[inner] - w.now[i]);
                }
            };
            for y in 0..rows {
                if open[0] {
                    carry(y * cols, y * cols + 1, &mut next);
                }
                if open[2] {
                    carry(y * cols + cols - 1, y * cols + cols - 2, &mut next);
                }
            }
            for x in 0..cols {
                if open[1] {
                    carry(x, cols + x, &mut next);
                }
                if open[3] {
                    carry((rows - 1) * cols + x, (rows - 2) * cols + x, &mut next);
                }
            }
            // the producers hold the water to their swing
            for (p, hold) in drivers.iter().zip(&holds) {
                let swing = p.amplitude * (TAU * p.frequency * time + p.phase).sin();
                for &(i, k) in hold {
                    next[i] += (swing - next[i]) * k;
                }
            }
            w.before = std::mem::replace(&mut w.now, next);
        }
    });
    let s = b.scale;
    let origin = b.origin;
    // the grid point under layer point (px, py)
    let grid_at = |px: f64, py: f64| ((px - r.x0) / r.width().max(1e-9) * (cols - 1) as f64, (py - r.y0) / r.height().max(1e-9) * (rows - 1) as f64);
    // between grid points smoothly (a cubic through four on each side), so the surface's slope
    // does not jump from cell to cell (lenses and displacements driven by it would show the grid)
    let height = |px: f64, py: f64| -> f32 {
        let (fx, fy) = grid_at(px, py);
        let (fx, fy) = (fx.clamp(0.0, (cols - 1) as f64), fy.clamp(0.0, (rows - 1) as f64));
        let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let g = |x: i64, y: i64| state.now[(y.clamp(0, rows as i64 - 1) as usize) * cols + x.clamp(0, cols as i64 - 1) as usize];
        let cubic = |a: f32, b: f32, c: f32, d: f32, t: f32| b + 0.5 * t * (c - a + t * (2.0 * a - 5.0 * b + 4.0 * c - d + t * (3.0 * (b - c) + d - a)));
        let row = |y: i64| cubic(g(x0 - 1, y), g(x0, y), g(x0 + 1, y), g(x0 + 2, y), tx);
        cubic(row(y0 - 1), row(y0), row(y0 + 1), row(y0 + 2), ty)
    };
    let dry_at = |px: f64, py: f64| {
        let (fx, fy) = grid_at(px, py);
        let (x, y) = (fx.round().clamp(0.0, (cols - 1) as f64) as usize, fy.round().clamp(0.0, (rows - 1) as f64) as usize);
        quick[y * cols + x] <= 0.0
    };
    if ctx.choice("view") == 1 {
        let (bright, contrast, gamma) = (ctx.num("brightness", 0.5) as f32, ctx.num("contrast", 0.75) as f32, ctx.num("gammaAdjustment", 1.0).max(0.01) as f32);
        let (dry_clear, solid) = (ctx.choice("renderDryAreasAs") == 1, 1.0 - ctx.num("transparency", 0.0).clamp(0.0, 1.0) as f32);
        map_px(&mut b.img, |x, y, _| {
            let (px, py) = (origin.x + (x as f64 + 0.5) / s, origin.y + (y as f64 + 0.5) / s);
            let v = (bright + height(px, py) * contrast).clamp(0.0, 1.0).powf(1.0 / gamma);
            let a = if dry_clear && dry_at(px, py) { 0.0 } else { solid };
            [v * a, v * a, v * a, a]
        });
        return b;
    }
    // the wireframe: the grid turned about the upright, tipped toward the eye, its heights
    // raised, seen a little in perspective
    let (turn, tip, lift) = (ctx.num("horizontalRotation", 0.0).to_radians(), ctx.num("verticalRotation", 53.0).to_radians(), ctx.num("verticalScale", 0.5));
    let mid = r.center();
    let size = r.width().max(r.height());
    let (sn, cs) = turn.sin_cos();
    let (st, ct) = tip.sin_cos();
    let see = |gx: usize, gy: usize| -> Option<[f32; 2]> {
        let (x, y) = ((gx as f64 / (cols - 1) as f64 - 0.5) * r.width(), (gy as f64 / (rows - 1) as f64 - 0.5) * r.height());
        let z = -(state.now[gy * cols + gx] as f64) * lift * size * 0.25;
        let (x, y) = (x * cs - y * sn, x * sn + y * cs);
        let (y, z) = (y * ct - z * st, y * st + z * ct);
        let far = size * 2.0;
        (far + z > far * 0.05).then(|| {
            let k = far / (far + z);
            let p = b.pixel(veymelo_geom::vec2(mid.x + x * k, mid.y + y * k));
            [p.x as f32, p.y as f32]
        })
    };
    let points: Vec<Option<[f32; 2]>> = (0..n).map(|i| see(i % cols, i / cols)).collect();
    let mut lines = Vec::new();
    for i in 0..n {
        let (gx, gy) = (i % cols, i / cols);
        let Some(a) = points[i] else { continue };
        for j in [(gx + 1 < cols).then_some(i + 1), (gy + 1 < rows).then_some(i + cols)].into_iter().flatten() {
            if let Some(z) = points[j] {
                let wet = quick[i] > 0.0 && quick[j] > 0.0;
                let col = if wet { [0.45, 0.9, 0.5, 1.0] } else { [0.6, 0.45, 0.3, 1.0] };
                lines.push(crate::sprites::Sprite::new(a[0], a[1], 0.5, col, crate::sprites::Shape::Line { dx: a[0] - z[0], dy: a[1] - z[1] }));
            }
        }
    }
    let mut out = Image::filled(b.img.width, b.img.height, [0.0, 0.0, 0.0, 1.0]);
    crate::sprites::draw(&mut out, &lines, false);
    b.img = out;
    b
}

pub const ALL: [Spec; 2] = [CAUSTICS, WAVE_WORLD];

#[cfg(test)]
mod tests;
