//! Layer styles drawn: what a layer's styles make of its pixels (after its masks and effects,
//! before it is placed).
//!
//! - Under it, each in its own blend mode with what is under the layer: Drop Shadow, then Outer
//!   Glow.
//! - The layer itself (its fill at Fill Opacity) with, inside its shape and each by its own mode:
//!   Gradient Overlay, Color Overlay, Satin, Inner Glow and Inner Shadow; then Stroke, then Bevel
//!   and Emboss.
//!
//! Shapes grown or shrunk, strokes, precise glows and bevels come from the exact distance to the
//! layer's edge; soft shadows and glows from Gaussian blurs of its shape. Sizes are in the
//! layer's pixels, so a smaller render draws them smaller.

use std::cell::OnceCell;

use veymelo_color::{BlendMode, blend};
use veymelo_effects::Buf;
use veymelo_keyframe::{Gradient, Value};
use veymelo_project::styles::{self, mode_of};
use veymelo_project::{Group, Layer, Property};
use veymelo_raster::filter::{self, Edge};
use veymelo_raster::{Image, Px, noise};

/// The farthest (buffer pixels) styles may reach past the layer on each side.
const MOST_PAD: f64 = 2048.0;

/// A styled layer: what goes under it (bottom first, each with its mode), and itself.
pub struct Styled {
    pub under: Vec<(Buf, BlendMode)>,
    pub body: Buf,
}

impl Styled {
    /// All of it in one picture (for a track matte, or a layer drawn alone first).
    pub fn flat(&self) -> Buf {
        if self.under.is_empty() {
            return self.body.clone();
        }
        let mut img = Image::new(self.body.img.width, self.body.img.height);
        for (b, m) in &self.under {
            veymelo_raster::blend_onto(&mut img, &b.img, *m, 1.0, 0);
        }
        veymelo_raster::blend_onto(&mut img, &self.body.img, BlendMode::Normal, 1.0, 0);
        Buf { img, origin: self.body.origin, scale: self.body.scale }
    }
}

/// How the layer itself is drawn (Advanced Blending).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blending {
    pub fill: f32,
    /// Whether it draws into red, green and blue.
    pub channels: [bool; 3],
    /// 0 none, 1 shallow, 2 deep.
    pub knockout: u32,
    pub interior_group: bool,
}

impl Default for Blending {
    fn default() -> Blending {
        Blending { fill: 1.0, channels: [true; 3], knockout: 0, interior_group: false }
    }
}

/// A style's settings, read at the time.
struct Reader<'a> {
    group: &'a Group,
    read: &'a dyn Fn(&Property) -> Value,
}

impl Reader<'_> {
    fn value(&self, id: &str) -> Option<Value> {
        self.group.prop(id).ok().map(self.read)
    }
    fn num(&self, id: &str, d: f64) -> f64 {
        self.value(id).map_or(d, |v| v.as_f64())
    }
    /// A percentage as 0 to 1.
    fn part(&self, id: &str, d: f64) -> f64 {
        (self.num(id, d) / 100.0).clamp(0.0, 1.0)
    }
    fn flag(&self, id: &str) -> bool {
        self.value(id).is_some_and(|v| v.as_bool())
    }
    fn choice(&self, id: &str) -> u32 {
        self.value(id).map_or(0, |v| v.as_choice())
    }
    fn mode(&self, id: &str) -> BlendMode {
        mode_of(self.choice(id))
    }
    /// A colour in the working space.
    fn color(&self, id: &str, linear: bool) -> [f32; 3] {
        let c = self.value(id).map_or(veymelo_color::Rgba::BLACK, |v| v.as_color());
        let ch = |v: f64| if linear { veymelo_color::srgb_to_linear(v) as f32 } else { v as f32 };
        [ch(c.r), ch(c.g), ch(c.b)]
    }
    fn gradient(&self) -> Option<Gradient> {
        self.value("colors").and_then(|v| v.as_gradient().cloned()).filter(|g| !g.colors.is_empty())
    }
}

/// The layer's styles list, if it has any style switched on or blending that is not plain.
pub fn list(layer: &Layer) -> Option<&Group> {
    if !styles::can_have(layer) {
        return None;
    }
    let list = layer.props.group(styles::LIST).ok()?;
    list.children.iter().filter_map(|c| c.as_group()).any(|g| g.enabled).then_some(list)
}

/// The layer's Advanced Blending at the time.
pub fn blending(list: &Group, read: &dyn Fn(&Property) -> Value) -> Blending {
    let Some(adv) = list.children.iter().filter_map(|c| c.as_group()).find(|g| g.def.id == styles::BLENDING && g.enabled).and_then(|g| g.group("advancedBlending").ok()) else {
        return Blending::default();
    };
    let r = Reader { group: adv, read };
    Blending { fill: r.part("fillOpacity", 100.0) as f32, channels: [r.flag("red"), r.flag("green"), r.flag("blue")], knockout: r.choice("knockout"), interior_group: r.flag("blendInteriorAsGroup") }
}

/// The light's angle and altitude (degrees).
fn light(list: &Group, read: &dyn Fn(&Property) -> Value) -> (f64, f64) {
    match list.children.iter().filter_map(|c| c.as_group()).find(|g| g.def.id == styles::BLENDING) {
        Some(bo) => {
            let r = Reader { group: bo, read };
            (r.num("globalLightAngle", styles::ANGLE), r.num("globalLightAltitude", styles::ALTITUDE))
        }
        None => (styles::ANGLE, styles::ALTITUDE),
    }
}

/// How far away from the light a thing at `distance` falls (the light at `angle`, counter-
/// clockwise from the right).
fn away(angle: f64, distance: f64) -> [f64; 2] {
    let a = angle.to_radians();
    [-distance * a.cos(), distance * a.sin()]
}

/// A one-channel picture.
#[derive(Clone)]
struct Plane {
    w: usize,
    h: usize,
    v: Vec<f32>,
}

impl Plane {
    fn map(&self, f: impl Fn(f32) -> f32) -> Plane {
        Plane { w: self.w, h: self.h, v: self.v.iter().map(|&x| f(x)).collect() }
    }

    /// Blurred as a Gaussian of deviation `sigma` (pixels), nothing past its edges.
    fn blur(&self, sigma: f64) -> Plane {
        if sigma < 0.05 {
            return self.clone();
        }
        let img = Image { width: self.w, height: self.h, px: self.v.iter().map(|&a| [a, a, a, a]).collect() };
        let out = filter::gaussian(&img, sigma, sigma, Edge::Clear);
        Plane { w: self.w, h: self.h, v: out.px.iter().map(|p| p[3]).collect() }
    }

    /// At point (x, y) between pixels, nothing past the edges.
    fn at(&self, x: f64, y: f64) -> f32 {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = ((fx - x0) as f32, (fy - y0) as f32);
        let get = |x: i64, y: i64| if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 { 0.0 } else { self.v[y as usize * self.w + x as usize] };
        let (x0, y0) = (x0 as i64, y0 as i64);
        let top = get(x0, y0) + (get(x0 + 1, y0) - get(x0, y0)) * tx;
        let bottom = get(x0, y0 + 1) + (get(x0 + 1, y0 + 1) - get(x0, y0 + 1)) * tx;
        top + (bottom - top) * ty
    }

    /// At pixel (x, y) moved by `by` (the plane moves, so it is read from `by` back).
    fn moved(&self, x: usize, y: usize, by: [f64; 2]) -> f32 {
        self.at(x as f64 + 0.5 - by[0], y as f64 + 0.5 - by[1])
    }

    fn get(&self, x: i64, y: i64) -> f32 {
        self.v[(y.clamp(0, self.h as i64 - 1) as usize) * self.w + x.clamp(0, self.w as i64 - 1) as usize]
    }
}

/// How far each pixel is from the shape's edge (below zero inside), the half-covered pixels
/// placed by how much they are covered.
fn signed_distance(alpha: &Plane) -> Plane {
    let (w, h) = (alpha.w, alpha.h);
    let inside: Vec<bool> = alpha.v.iter().map(|&a| a >= 0.5).collect();
    let outside: Vec<bool> = inside.iter().map(|b| !b).collect();
    let to_in = veymelo_effects::puppet::mesh::distance_sq(w, h, &inside);
    let to_out = veymelo_effects::puppet::mesh::distance_sq(w, h, &outside);
    Plane {
        w,
        h,
        v: (0..w * h)
            .map(|i| {
                let a = alpha.v[i];
                if a > 0.0 && a < 1.0 {
                    0.5 - a
                } else if a >= 0.5 {
                    -(to_out[i].sqrt() as f32 - 0.5)
                } else {
                    to_in[i].sqrt() as f32 - 0.5
                }
            })
            .collect(),
    }
}

/// How much of a pixel the shape grown by `r` covers (below zero: shrunk).
fn grown(distance: f32, r: f32) -> f32 {
    (r + 0.5 - distance).clamp(0.0, 1.0)
}

/// The shape (or, `outside`, what is round it) grown by `spread` of `size` and softened by the
/// rest.
fn soft(alpha: &Plane, distance: &dyn Fn() -> Plane, size: f64, spread: f64, outside: bool) -> Plane {
    let hard = size * spread;
    let base = if hard > 0.01 {
        let r = hard as f32;
        distance().map(|v| if outside { grown(-v, r) } else { grown(v, r) })
    } else if outside {
        alpha.map(|a| 1.0 - a)
    } else {
        alpha.clone()
    };
    base.blur((size - hard).max(0.0) / 2.0)
}

/// A colour (straight) at `a` of it blended onto `dst` by `mode`.
fn mix(mode: BlendMode, dst: Px, c: [f32; 3], a: f32, grain: f32) -> Px {
    blend(mode, dst, [c[0] * a, c[1] * a, c[2] * a, a], grain)
}

/// An inner style blended onto the layer inside its shape: within an edge pixel, onto the part
/// the shape covers (the layer keeps its alpha).
fn inside(img: &mut Image, alpha: &Plane, mode: BlendMode, seed: u32, f: impl Fn(usize, usize) -> ([f32; 3], f32)) {
    let w = img.width;
    for (i, d) in img.px.iter_mut().enumerate() {
        let k = alpha.v[i];
        if k <= 0.0 {
            continue;
        }
        let (x, y) = (i % w, i / w);
        let (c, a) = f(x, y);
        if a > 0.0 {
            let grain = noise(x, y, seed);
            *d = mix(mode, d.map(|v| v / k), c, a, grain).map(|v| v * k);
        }
    }
}

/// A picture of one colour at `f(x, y)` of it.
fn sheet(w: usize, h: usize, f: impl Fn(usize, usize) -> ([f32; 3], f32)) -> Image {
    let mut img = Image::new(w, h);
    for (i, d) in img.px.iter_mut().enumerate() {
        let (c, a) = f(i % w, i / w);
        let a = a.clamp(0.0, 1.0);
        *d = [c[0] * a, c[1] * a, c[2] * a, a];
    }
    img
}

/// Less, here and there (a style's noise).
fn grainy(a: f32, amount: f32, x: usize, y: usize, seed: u32) -> f32 {
    if amount <= 0.0 { a } else { a * (1.0 - amount * noise(x, y, seed)) }
}

/// A glow: how strong (0 to 1) and in what colour.
struct Glow {
    mode: BlendMode,
    opacity: f32,
    noise: f32,
    gradient: Option<Gradient>,
    color: [f32; 3],
    precise: bool,
    from_middle: bool,
    spread: f64,
    size: f64,
    range: f64,
    jitter: f32,
}

impl Glow {
    fn read(r: &Reader, inner: bool, linear: bool) -> Glow {
        Glow {
            mode: r.mode("blendMode"),
            opacity: r.part("opacity", 75.0) as f32,
            noise: r.part("noise", 0.0) as f32,
            gradient: (r.choice("colorType") == 1).then(|| r.gradient()).flatten(),
            color: r.color("color", linear),
            precise: r.choice("technique") == 1,
            from_middle: inner && r.choice("source") == 0,
            spread: r.part(if inner { "choke" } else { "spread" }, 0.0),
            size: r.num("size", 5.0).clamp(0.0, 250.0),
            range: r.part("range", 50.0).max(0.01),
            jitter: r.part("jitter", 0.0) as f32,
        }
    }

    /// How strong it is at each pixel (from the shape, or for an inner glow from round it).
    fn strength(&self, alpha: &Plane, distance: &dyn Fn() -> Plane, s: f64, inner: bool) -> Plane {
        let size = self.size * s;
        if self.precise {
            let hard = (size * self.spread) as f32;
            let reach = (size as f32 - hard).max(0.5);
            return distance().map(|v| {
                let v = if inner { -v } else { v };
                (1.0 - (v - hard).max(0.0) / reach).clamp(0.0, 1.0)
            });
        }
        soft(alpha, distance, size, self.spread, inner).map(|v| (v * 2.0).min(1.0))
    }

    /// Its colour and alpha where it is `v` strong.
    fn color_at(&self, v: f32, x: usize, y: usize) -> ([f32; 3], f32) {
        let v = v.clamp(0.0, 1.0).powf((0.5 / self.range) as f32);
        match &self.gradient {
            None => (self.color, v),
            Some(g) => {
                let j = if self.jitter > 0.0 { (noise(x, y, 77) - 0.5) * self.jitter } else { 0.0 };
                let c = g.at((1.0 - v + j).clamp(0.0, 1.0) as f64);
                ([c[0] as f32, c[1] as f32, c[2] as f32], c[3] as f32 * (v * 8.0).min(1.0))
            }
        }
    }
}

/// The layer's styles round and over its pixels `content`, read at the time; `rect` its
/// rectangle (layer pixels), for a gradient not aligned with its shape.
pub fn draw(list: &Group, read: &dyn Fn(&Property) -> Value, content: &Buf, rect: veymelo_geom::Rect, linear: bool, seed: u32) -> Styled {
    let (light_angle, light_altitude) = light(list, read);
    let blending = blending(list, read);
    let on: Vec<&Group> = list.children.iter().filter_map(|c| c.as_group()).filter(|g| g.enabled && g.def.id != styles::BLENDING).collect();
    let style = |id: &str| on.iter().find(|g| g.def.id == id).map(|g| Reader { group: g, read });
    let angle_of = |r: &Reader| if r.flag("useGlobalLight") { light_angle } else { r.num("angle", styles::ANGLE) };
    let s = content.scale.max(1e-6);
    // how far past the layer they reach
    let mut reach: f64 = 0.0;
    for r in on.iter().map(|g| Reader { group: g, read }) {
        let size = r.num("size", 5.0).clamp(0.0, 250.0);
        let far = match r.group.def.id {
            "dropShadow" | "innerShadow" => r.num("distance", 5.0).max(0.0) + size * 1.6,
            "outerGlow" | "innerGlow" => size * 1.6,
            "bevelEmboss" => size + r.num("soften", 0.0) * 2.0,
            "satin" => r.num("distance", 11.0) + size * 1.6,
            "stroke" => size,
            _ => 0.0,
        };
        reach = reach.max(far);
    }
    let mut buf = content.clone();
    if buf.img.px.is_empty() {
        return Styled { under: Vec::new(), body: buf };
    }
    buf.pad(((reach * s).ceil() + 2.0).min(MOST_PAD) as usize);
    let (w, h) = (buf.img.width, buf.img.height);
    let alpha = Plane { w, h, v: buf.img.px.iter().map(|p| p[3].clamp(0.0, 1.0)).collect() };
    let cell = OnceCell::new();
    let distance = || cell.get_or_init(|| signed_distance(&alpha)).clone();
    let distance: &dyn Fn() -> Plane = &distance;
    let mut under = Vec::new();
    let place = |img: Image| Buf { img, origin: buf.origin, scale: buf.scale };
    // ---- under the layer
    if let Some(r) = style("dropShadow") {
        let (color, opacity, size, spread, grain) = (r.color("color", linear), r.part("opacity", 75.0) as f32, r.num("size", 5.0).clamp(0.0, 250.0) * s, r.part("spread", 0.0), r.part("noise", 0.0) as f32);
        let by = away(angle_of(&r), r.num("distance", 5.0).max(0.0) * s);
        let shape = soft(&alpha, distance, size, spread, false);
        let knocks = r.flag("knocksOut");
        let img = sheet(w, h, |x, y| {
            let mut a = shape.moved(x, y, by) * opacity;
            if knocks {
                a *= 1.0 - alpha.v[y * w + x];
            }
            (color, grainy(a, grain, x, y, seed))
        });
        under.push((place(img), r.mode("blendMode")));
    }
    if let Some(r) = style("outerGlow") {
        let g = Glow::read(&r, false, linear);
        let v = g.strength(&alpha, distance, s, false);
        let img = sheet(w, h, |x, y| {
            let i = y * w + x;
            let (c, a) = g.color_at(v.v[i], x, y);
            // round the layer, not under it
            (c, grainy(a * g.opacity * (1.0 - alpha.v[i]), g.noise, x, y, seed ^ 0x5a5a))
        });
        under.push((place(img), g.mode));
    }
    // ---- the layer itself
    let mut body = buf.img.clone();
    let fade = |img: &mut Image, k: f32| img.px.iter_mut().for_each(|p| *p = p.map(|v| v * k));
    if !blending.interior_group && blending.fill < 1.0 {
        fade(&mut body, blending.fill);
    }
    if let Some(r) = style("gradientOverlay") {
        let gradient = r.gradient().unwrap_or(Gradient { colors: vec![(0.0, [0.0; 3]), (1.0, [1.0; 3])], opacities: vec![(0.0, 1.0), (1.0, 1.0)] });
        let (opacity, kind, reverse, scale) = (r.part("opacity", 100.0) as f32, r.choice("style"), r.flag("reverse"), r.num("scale", 100.0).clamp(10.0, 150.0) / 100.0);
        let a = r.num("angle", 90.0).to_radians();
        let off = r.value("offset").map_or([0.0; 2], |v| v.as_vec2());
        // the frame it spreads over: the layer's shape, or its whole rectangle
        let [x0, y0, x1, y1] = if r.flag("alignWithLayer") {
            let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
            for (i, &k) in alpha.v.iter().enumerate() {
                if k > 0.0 {
                    let (x, y) = ((i % w) as f64, (i / w) as f64);
                    b = [b[0].min(x), b[1].min(y), b[2].max(x + 1.0), b[3].max(y + 1.0)];
                }
            }
            if b[0] > b[2] { [0.0, 0.0, 1.0, 1.0] } else { b }
        } else {
            let (p, q) = (buf.pixel(veymelo_geom::vec2(rect.x0, rect.y0)), buf.pixel(veymelo_geom::vec2(rect.x1, rect.y1)));
            [p.x, p.y, q.x, q.y]
        };
        let (cx, cy) = ((x0 + x1) / 2.0 + off[0] * s, (y0 + y1) / 2.0 + off[1] * s);
        let (hw, hh) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
        let (ux, uy) = (a.cos(), -a.sin());
        let along_most = ((hw * ux).abs() + (hh * uy).abs()).max(1e-3) * scale;
        let radius = hw.max(hh).max(1e-3) * scale;
        inside(&mut body, &alpha, r.mode("blendMode"), seed, |x, y| {
            let (px, py) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
            let (along, across) = (px * ux + py * uy, -px * uy + py * ux);
            let t = match kind {
                1 => (px * px + py * py).sqrt() / radius,
                2 => (((-py).atan2(px) - a) / std::f64::consts::TAU).rem_euclid(1.0),
                3 => along.abs() / along_most,
                4 => (along.abs() + across.abs()) / along_most,
                _ => 0.5 + along / (2.0 * along_most),
            };
            let t = if reverse { 1.0 - t } else { t };
            let c = gradient.at(t.clamp(0.0, 1.0));
            ([c[0] as f32, c[1] as f32, c[2] as f32], c[3] as f32 * opacity)
        });
    }
    if let Some(r) = style("colorOverlay") {
        let (c, o) = (r.color("color", linear), r.part("opacity", 100.0) as f32);
        inside(&mut body, &alpha, r.mode("blendMode"), seed, |_, _| (c, o));
    }
    if let Some(r) = style("satin") {
        let (color, opacity, invert) = (r.color("color", linear), r.part("opacity", 50.0) as f32, r.flag("invert"));
        let by = away(r.num("angle", 19.0), r.num("distance", 11.0).clamp(0.0, 250.0) * s);
        let soft_shape = alpha.blur(r.num("size", 14.0).clamp(0.0, 250.0) * s / 2.0);
        inside(&mut body, &alpha, r.mode("blendMode"), seed, |x, y| {
            // the shape laid over itself, one copy each way
            let v = (soft_shape.moved(x, y, by) - soft_shape.moved(x, y, [-by[0], -by[1]])).abs().min(1.0);
            (color, if invert { 1.0 - v } else { v } * opacity)
        });
    }
    if let Some(r) = style("innerGlow") {
        let g = Glow::read(&r, true, linear);
        let v = g.strength(&alpha, distance, s, true);
        inside(&mut body, &alpha, g.mode, seed, |x, y| {
            let i = y * w + x;
            let e = if g.from_middle { 1.0 - v.v[i] } else { v.v[i] };
            let (c, a) = g.color_at(e, x, y);
            (c, grainy(a * g.opacity, g.noise, x, y, seed ^ 0x3c3c))
        });
    }
    if let Some(r) = style("innerShadow") {
        let (color, opacity, grain) = (r.color("color", linear), r.part("opacity", 75.0) as f32, r.part("noise", 0.0) as f32);
        let by = away(angle_of(&r), r.num("distance", 5.0).max(0.0) * s);
        let shape = soft(&alpha, distance, r.num("size", 5.0).clamp(0.0, 250.0) * s, r.part("choke", 0.0), true);
        inside(&mut body, &alpha, r.mode("blendMode"), seed, |x, y| (color, grainy(shape.moved(x, y, by).min(1.0) * opacity, grain, x, y, seed ^ 0x1234)));
    }
    if blending.interior_group && blending.fill < 1.0 {
        fade(&mut body, blending.fill);
    }
    if let Some(r) = style("stroke") {
        let d = distance();
        let (color, opacity, mode) = (r.color("color", linear), r.part("opacity", 100.0) as f32, r.mode("blendMode"));
        let size = (r.num("size", 3.0).clamp(0.0, 250.0) * s) as f32;
        let (out, inn) = match r.choice("position") {
            1 => (0.0, size),
            2 => (size / 2.0, size / 2.0),
            _ => (size, 0.0),
        };
        if inn > 0.0 {
            // inside the edge: over the layer, on the part of each pixel the ring takes
            inside(&mut body, &alpha, mode, seed, |x, y| {
                let i = y * w + x;
                let a = alpha.v[i];
                (color, ((a - grown(d.v[i], -inn)) / a.max(1e-6)).clamp(0.0, 1.0) * opacity)
            });
        }
        if out > 0.0 {
            // outside it: where the layer leaves the pixel uncovered
            for (i, p) in body.px.iter_mut().enumerate() {
                let ring = (grown(d.v[i], out) - alpha.v[i]).max(0.0) * opacity;
                if ring > 0.0 {
                    let k = 1.0 - ring;
                    *p = [color[0] * ring + p[0] * k, color[1] * ring + p[1] * k, color[2] * ring + p[2] * k, ring + p[3] * k];
                }
            }
        }
    }
    if let Some(r) = style("bevelEmboss") {
        let gl = r.flag("useGlobalLight");
        bevel(&mut body, &alpha, &distance(), &r, s, if gl { light_angle } else { r.num("angle", styles::ANGLE) }, if gl { light_altitude } else { r.num("altitude", styles::ALTITUDE) }, linear, seed);
    }
    Styled { under, body: place(body) }
}

/// Bevel and Emboss: a height from the distance to the layer's edge (shaped by the style),
/// lit from the light, its highlights and shadows each blended by their own mode.
#[allow(clippy::too_many_arguments)]
fn bevel(body: &mut Image, alpha: &Plane, d: &Plane, r: &Reader, s: f64, angle: f64, altitude: f64, linear: bool, seed: u32) {
    let size = (r.num("size", 5.0).clamp(0.0, 250.0) * s).max(0.5) as f32;
    let style = r.choice("style");
    let w = alpha.w;
    let mut height = d.map(|v| match style {
        // outer: rising up to the edge from outside
        0 => (1.0 - v.max(0.0) / size).clamp(0.0, 1.0),
        // emboss and stroke emboss: half out, half in
        2 | 4 => (0.5 - v / (2.0 * size)).clamp(0.0, 1.0),
        // pillow: sunk at the edge, raised either side
        3 => (v.abs() / size).clamp(0.0, 1.0),
        // inner: rising from the edge inward
        _ => (-v / size + 0.5 / size).clamp(0.0, 1.0),
    });
    // the distance is exact only at pixel middles: smoothed, so diagonal edges do not show steps
    // once lit (smooth: rounded over its size; chisel: crisp, a little softer for soft)
    let sigma = match r.choice("technique") {
        0 => (size as f64 / 4.0).max(1.0),
        2 => 1.5,
        _ => 0.75,
    };
    height = height.blur(sigma);
    let soften = r.num("soften", 0.0).clamp(0.0, 16.0) * s / 2.0;
    if soften > 0.05 {
        height = height.blur(soften);
    }
    let k = (size as f64 * r.num("depth", 100.0).clamp(1.0, 1000.0) / 100.0) as f32 * if r.choice("direction") == 1 { -1.0 } else { 1.0 };
    let (th, al) = (angle.to_radians(), altitude.clamp(0.0, 90.0).to_radians());
    let light = [(al.cos() * th.cos()) as f32, (-al.cos() * th.sin()) as f32, al.sin() as f32];
    let flat = light[2];
    // where it paints: outside for an outer bevel, inside for an inner one, both for the rest
    let region = |i: usize| -> f32 {
        let a = alpha.v[i];
        match style {
            0 => (grown(d.v[i], size) - a).max(0.0),
            1 => a,
            _ => grown(d.v[i], size),
        }
    };
    let lit = |x: usize, y: usize| -> (f32, f32) {
        let (xi, yi) = (x as i64, y as i64);
        let gx = (height.get(xi + 1, yi) - height.get(xi - 1, yi)) * 0.5 * k;
        let gy = (height.get(xi, yi + 1) - height.get(xi, yi - 1)) * 0.5 * k;
        let n = [-gx, -gy, 1.0];
        let dot = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]) / (n[0] * n[0] + n[1] * n[1] + 1.0).sqrt();
        (((dot - flat) / (1.0 - flat).max(1e-4)).clamp(0.0, 1.0), ((flat - dot) / flat.max(1e-4)).clamp(0.0, 1.0))
    };
    let (sh_mode, sh_color, sh_op) = (r.mode("shadowMode"), r.color("shadowColor", linear), r.part("shadowOpacity", 75.0) as f32);
    let (hl_mode, hl_color, hl_op) = (r.mode("highlightMode"), r.color("highlightColor", linear), r.part("highlightOpacity", 75.0) as f32);
    for (i, p) in body.px.iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        let area = region(i);
        if area <= 0.0 {
            continue;
        }
        let (hl, shade) = lit(x, y);
        let grain = noise(x, y, seed);
        if shade > 0.0 {
            *p = mix(sh_mode, *p, sh_color, shade * sh_op * area, grain);
        }
        if hl > 0.0 {
            *p = mix(hl_mode, *p, hl_color, hl * hl_op * area, grain);
        }
    }
}

/// Of what was drawn onto `dst`, only the channels switched on kept (the rest as `before`).
pub fn into_channels(dst: &mut Image, before: &Image, channels: [bool; 3]) {
    for (p, q) in dst.px.iter_mut().zip(&before.px) {
        for (c, on) in channels.iter().enumerate() {
            if !on {
                p[c] = q[c];
            }
        }
    }
}

