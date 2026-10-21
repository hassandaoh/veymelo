//! Effects: what changes a layer's pixels after they are drawn and cut by its masks, before
//! they are placed in the composition. Each effect is a type of instance in a layer's `effects`
//! list, its settings ordinary properties (animated, given expressions, found by path like any
//! other), and a function that takes the layer's pixels and gives new ones.
//!
//! Effects work in the layer's own space: [`Buf`] holds pixels drawn at some number of pixels to
//! a layer unit, from some origin. Distances an effect takes are in layer units, so a picture
//! drawn small for a preview changes the same way as the full one.

pub mod audio;
pub mod bends;
pub mod cards;
pub mod channel;
pub mod blur;
pub mod breakup;
pub mod color;
pub mod colorfile;
pub mod controls;
pub mod crafts;
mod defs;
pub mod distort;
pub mod draw;
pub mod film;
pub mod foam;
pub mod forms;
pub mod generate;
pub mod grade;
pub mod grain;
pub mod hair;
pub mod keys;
pub mod liquid;
pub mod look;
pub mod lut;
pub mod matte;
pub mod noise;
pub mod obsolete;
pub mod ocio;
pub mod paint;
pub mod particles;
pub mod passes;
pub mod perspective;
pub mod playground;
pub mod puppet;
pub mod scene3d;
pub mod screen;
pub mod shatter;
pub mod sprites;
pub mod strokes;
mod stepper;
pub mod sphere;
pub mod steady;
pub mod stylize;
pub mod time;
pub mod transition;
#[cfg(test)]
mod goldens;
#[cfg(test)]
mod showcase;
#[cfg(test)]
mod testing;
mod util;
pub mod utility;
pub mod warp;
pub mod water;
pub mod weather;
pub mod words;

use std::cell::RefCell;

use veymelo_geom::{Mat3, Rect, Vec2, vec2};
use veymelo_keyframe::Value;
use veymelo_project::{Catalog, Group, GroupDef, Property};
use veymelo_raster::Image;
use veymelo_time::{FrameRate, Tick};

/// A layer's pixels while its effects run: pixel (0, 0) starts at `origin` (in layer units) and
/// there are `scale` pixels to a unit.
#[derive(Clone, Debug)]
pub struct Buf {
    pub img: Image,
    pub origin: Vec2,
    pub scale: f64,
}

impl Buf {
    /// From the buffer's pixels to the layer's units.
    pub fn to_layer(&self) -> Mat3 {
        Mat3::translate(self.origin) * Mat3::scale(vec2(1.0 / self.scale, 1.0 / self.scale))
    }

    /// Where a point of the layer is in the buffer's pixels.
    pub fn pixel(&self, p: Vec2) -> Vec2 {
        (p - self.origin) * self.scale
    }

    /// The part of the layer the buffer holds, in layer units.
    pub fn area(&self) -> Rect {
        Rect::new(self.origin.x, self.origin.y, self.origin.x + self.img.width as f64 / self.scale, self.origin.y + self.img.height as f64 / self.scale)
    }

    /// Grown with clear pixels (never shrunk) to hold all of `r` (in layer units): what an
    /// effect that draws over the whole layer needs.
    pub fn cover(&mut self, r: Rect) {
        let s = self.scale;
        let (w, h) = (self.img.width as f64, self.img.height as f64);
        // whole pixels to add on each side, so what is there keeps its place
        let left = ((self.origin.x - r.x0) * s).ceil().max(0.0);
        let top = ((self.origin.y - r.y0) * s).ceil().max(0.0);
        let right = ((r.x1 - self.origin.x) * s - w).ceil().max(0.0);
        let bottom = ((r.y1 - self.origin.y) * s - h).ceil().max(0.0);
        if left + right + w > 16384.0 || top + bottom + h > 16384.0 || left + top + right + bottom == 0.0 {
            return;
        }
        let (l, t, nw, nh) = (left as usize, top as usize, (left + w + right) as usize, (top + h + bottom) as usize);
        let mut out = Image::new(nw, nh);
        let ow = self.img.width;
        for y in 0..self.img.height {
            out.px[(y + t) * nw + l..(y + t) * nw + l + ow].copy_from_slice(&self.img.px[y * ow..(y + 1) * ow]);
        }
        self.img = out;
        self.origin -= vec2(left, top) / s;
    }

    /// `n` clear pixels more on every side (room for a blur to spread into).
    pub fn pad(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        let (w, h) = (self.img.width, self.img.height);
        let mut out = Image::new(w + 2 * n, h + 2 * n);
        for y in 0..h {
            out.px[(y + n) * (w + 2 * n) + n..(y + n) * (w + 2 * n) + n + w].copy_from_slice(&self.img.px[y * w..(y + 1) * w]);
        }
        self.img = out;
        self.origin -= vec2(n as f64, n as f64) / self.scale;
    }
}

/// One of the layer's masks, as an outline in layer units.
#[derive(Clone, Debug, PartialEq)]
pub struct MaskShape {
    pub name: String,
    pub points: Vec<[f64; 2]>,
    pub closed: bool,
    pub inverted: bool,
}

/// A mask's path as an outline: each curved side cut into short straight ones.
pub fn outline(path: &veymelo_keyframe::ShapePath) -> Vec<[f64; 2]> {
    let n = path.points.len();
    let mut out = Vec::new();
    if n == 0 {
        return out;
    }
    let sides = if path.closed { n } else { n - 1 };
    out.push(path.points[0]);
    for i in 0..sides {
        let (a, b) = (path.points[i], path.points[(i + 1) % n]);
        let c1 = [a[0] + path.out_tangents.get(i).map_or(0.0, |t| t[0]), a[1] + path.out_tangents.get(i).map_or(0.0, |t| t[1])];
        let j = (i + 1) % n;
        let c2 = [b[0] + path.in_tangents.get(j).map_or(0.0, |t| t[0]), b[1] + path.in_tangents.get(j).map_or(0.0, |t| t[1])];
        let straight = c1 == a && c2 == b;
        // about one piece for every two units of the side's control polygon
        let reach = (a[0] - c1[0]).hypot(a[1] - c1[1]) + (c1[0] - c2[0]).hypot(c1[1] - c2[1]) + (c2[0] - b[0]).hypot(c2[1] - b[1]);
        let k = if straight { 1 } else { ((reach / 2.0).ceil() as usize).clamp(2, 256) };
        for s in 1..=k {
            let t = s as f64 / k as f64;
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            out.push([w[0] * a[0] + w[1] * c1[0] + w[2] * c2[0] + w[3] * b[0], w[0] * a[1] + w[1] * c1[1] + w[2] * c2[1] + w[3] * b[1]]);
        }
    }
    if path.closed {
        out.pop();
    }
    out
}

/// How much of another layer an effect takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Take {
    /// What it shows, before its masks and effects.
    Source,
    /// Cut by its masks.
    Masks,
    /// Cut by its masks and changed by its effects.
    All,
}

/// How another layer is laid over this one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// At its own size, middle on middle.
    Centre,
    /// Stretched from its rectangle to this layer's.
    Stretch,
    /// At its own size, middle on middle, and repeated all round.
    Tile,
}

/// What the renderer can give an effect beyond its own pixels.
pub trait Host {
    /// Layer `id` of the same composition now, in its own space at the same pixels to a unit as
    /// the effect's: its pixels (as much of it as `take` says) and its rectangle.
    fn layer(&self, id: u64, take: Take) -> Option<(Buf, Rect)>;

    /// The effect's own layer at its own time `time`, cut by its masks and changed by the first
    /// `upto` of its effects (time effects and smoothing over time read neighbouring frames).
    fn this_at(&self, _time: Tick, _upto: usize) -> Option<Buf> {
        None
    }

    /// Layer `id` as [`Host::layer`] gives it, but `shift` later (or earlier) in the
    /// composition's time.
    fn layer_at(&self, _id: u64, _shift: Tick, _take: Take) -> Option<(Buf, Rect)> {
        None
    }

    /// The render passes (depth, object numbers, …) that came with what the effect's own layer
    /// shows, now; none when it has none (the 3D Channel effects then leave the layer as it is).
    fn passes(&self) -> Option<std::sync::Arc<veymelo_raster::passes::Passes>> {
        None
    }

    /// The composition's camera and first light as the effect's layer sees them.
    fn scene(&self) -> Option<CompScene> {
        None
    }
}

/// The composition's camera and first light, in the effect's layer's own space: x and y its
/// units, z out of its plane (away from the viewer while it is not turned).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompScene {
    /// From a point of the layer's space to where it is seen, in the layer's units: (x, y, w),
    /// divide by w.
    pub camera: [[f64; 4]; 3],
    /// Where the eye is.
    pub eye: [f64; 3],
    pub light: Option<CompLight>,
}

/// The composition's first light, in a layer's space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompLight {
    /// 0 parallel, 1 spot, 2 point, 3 ambient.
    pub kind: u8,
    pub at: [f64; 3],
    /// Which way it shines (parallel and spot).
    pub dir: [f64; 3],
    /// Its colour times its intensity.
    pub color: [f32; 3],
}

/// A property's value at a layer time, its expression applied.
pub type ReadAt<'r> = &'r dyn Fn(&Property, Tick) -> Value;

/// What an effect draws with: its own settings (read at the time, expressions applied), the
/// layer's time and size, and a seed of its own.
pub struct Ctx<'r> {
    /// The effect's instance in the layer's list.
    pub effect: &'r Group,
    /// A property's value now, its expression applied.
    pub read: &'r dyn Fn(&Property) -> Value,
    /// The layer's own time.
    pub time: Tick,
    /// The layer's rectangle in its own units (for a layer without a size of its own, the
    /// composition's size about the layer's origin).
    pub bounds: Rect,
    /// The same for this instance on this layer every frame.
    pub seed: u64,
    /// The pixels are what is under an adjustment layer (the whole composition): nothing lies
    /// past their edges but more of the same.
    pub adjustment: bool,
    /// The project keeps colours as floats (32-bit): they may go past white and below black.
    pub float: bool,
    /// The effect's place in its layer's list, from 0.
    pub index: usize,
    /// How long a frame of the composition lasts.
    pub frame: Tick,
    /// The layer's masks (switched on), top first.
    pub masks: &'r [MaskShape],
    /// A property's value at another layer time, its expression applied.
    pub read_at: Option<ReadAt<'r>>,
    pub host: Option<&'r dyn Host>,
    /// The composition's shutter when the layer is motion blurred: open `.0` degrees of a
    /// frame from `.1` degrees, in at least `.2` pictures.
    pub shutter: Option<(f64, f64, u32)>,
    /// The composition's time as it is shown (from its start).
    pub comp_time: Tick,
    /// The composition's frame rate.
    pub rate: FrameRate,
    /// Draw what an effect's own tool shows over its result (a puppet's meshes and pins), for a
    /// preview.
    pub marks: bool,
    notes: RefCell<Vec<String>>,
}

impl<'r> Ctx<'r> {
    pub fn new(effect: &'r Group, read: &'r dyn Fn(&Property) -> Value, time: Tick, bounds: Rect, seed: u64) -> Ctx<'r> {
        Ctx { effect, read, time, bounds, seed, adjustment: false, float: false, index: 0, frame: Tick::from_seconds(1.0 / 30.0), masks: &[], read_at: None, host: None, shutter: None, comp_time: time, rate: FrameRate::default(), marks: false, notes: RefCell::new(Vec::new()) }
    }

    /// The value of the setting at `path` of the effect at layer time `t` (now, when the
    /// renderer cannot say).
    pub fn value_at(&self, path: &str, t: Tick) -> Option<Value> {
        let p = self.effect.prop(path).ok()?;
        Some(match self.read_at {
            Some(f) => f(p, t),
            None => (self.read)(p),
        })
    }

    /// The mask with number `n` (from 1), if there is one.
    pub fn mask(&self, n: usize) -> Option<&MaskShape> {
        self.masks.get(n.checked_sub(1)?)
    }

    /// The value of the setting at `path` (inside `g`, or the effect itself).
    pub fn value_in(&self, g: &Group, path: &str) -> Option<Value> {
        g.prop(path).ok().map(self.read)
    }

    pub fn num_in(&self, g: &Group, path: &str, d: f64) -> f64 {
        self.value_in(g, path).map_or(d, |v| v.as_f64())
    }

    pub fn v2_in(&self, g: &Group, path: &str, d: [f64; 2]) -> [f64; 2] {
        self.value_in(g, path).map_or(d, |v| v.as_vec2())
    }

    pub fn num(&self, path: &str, d: f64) -> f64 {
        self.num_in(self.effect, path, d)
    }

    /// A point setting, in layer units.
    pub fn point(&self, path: &str) -> Vec2 {
        let p = self.v2_in(self.effect, path, [0.0; 2]);
        vec2(p[0], p[1])
    }

    pub fn flag(&self, path: &str) -> bool {
        self.value_in(self.effect, path).is_some_and(|v| v.as_bool())
    }

    /// A menu setting, counted from 0.
    pub fn choice(&self, path: &str) -> u32 {
        self.value_in(self.effect, path).map_or(0, |v| v.as_choice())
    }

    pub fn color(&self, path: &str) -> veymelo_color::Rgba {
        self.value_in(self.effect, path).map_or(veymelo_color::Rgba::WHITE, |v| v.as_color())
    }

    /// The layer a layer setting names, laid over `buf`'s pixels: stretched from its rectangle to
    /// this layer's, or (not `stretch`) centred on it at its own size. None when the setting
    /// names no layer or it cannot be drawn.
    pub fn layer_over(&self, path: &str, buf: &Buf, take: Take, stretch: bool) -> Option<Image> {
        self.layer_placed(path, buf, take, if stretch { Fit::Stretch } else { Fit::Centre }).map(|(img, _)| img)
    }

    /// The layer a layer setting names, laid over `buf`'s pixels as `fit` says, and where its
    /// rectangle lands in those pixels (for tiles, the first one).
    pub fn layer_placed(&self, path: &str, buf: &Buf, take: Take, fit: Fit) -> Option<(Image, Rect)> {
        let id = self.value_in(self.effect, path).and_then(|v| v.as_layer())?;
        let (other, rect) = self.host?.layer(id, take)?;
        let b = self.bounds;
        // from this layer's units to the other's
        let to_other = |p: Vec2| -> Vec2 {
            match fit {
                Fit::Stretch => vec2(rect.x0 + (p.x - b.x0) / b.width().max(1e-9) * rect.width(), rect.y0 + (p.y - b.y0) / b.height().max(1e-9) * rect.height()),
                Fit::Centre => p - b.center() + rect.center(),
                Fit::Tile => {
                    let q = p - b.center() + rect.center();
                    vec2(rect.x0 + (q.x - rect.x0).rem_euclid(rect.width().max(1e-9)), rect.y0 + (q.y - rect.y0).rem_euclid(rect.height().max(1e-9)))
                }
            }
        };
        let mut out = Image::new(buf.img.width, buf.img.height);
        for y in 0..buf.img.height {
            for x in 0..buf.img.width {
                let p = buf.origin + vec2(x as f64 + 0.5, y as f64 + 0.5) / buf.scale;
                let q = other.pixel(to_other(p));
                out.px[y * buf.img.width + x] = other.img.sample(veymelo_raster::Sampling::Bilinear, q.x, q.y);
            }
        }
        let lands = match fit {
            Fit::Stretch => b,
            _ => {
                let c = b.center() - rect.center();
                Rect::new(rect.x0 + c.x, rect.y0 + c.y, rect.x1 + c.x, rect.y1 + c.y)
            }
        };
        let (a, z) = (buf.pixel(vec2(lands.x0, lands.y0)), buf.pixel(vec2(lands.x1, lands.y1)));
        Some((out, Rect::new(a.x, a.y, z.x, z.y)))
    }

    /// Something to tell whoever renders (an effect that cannot do what it is asked).
    pub fn note(&self, text: String) {
        let mut n = self.notes.borrow_mut();
        if !n.contains(&text) {
            n.push(text);
        }
    }

    pub fn notes(self) -> Vec<String> {
        self.notes.into_inner()
    }
}

/// An effect: its definition (id, name, settings), its kind, and how it draws.
pub struct Spec {
    pub def: &'static GroupDef,
    /// Its family: distort, blur, colour…
    pub category: &'static str,
    pub run: fn(&Ctx, Buf) -> Buf,
    /// Lists inside it and the types each holds (a puppet's meshes and pins).
    pub lists: &'static [(&'static str, &'static GroupDef)],
    /// Point settings that start at a place on the layer, as fractions of its rectangle
    /// (`(0.5, 0.5)` is its middle).
    pub points: &'static [(&'static str, [f64; 2])],
}

impl Spec {
    pub const fn new(def: &'static GroupDef, category: &'static str, run: fn(&Ctx, Buf) -> Buf) -> Spec {
        Spec { def, category, run, lists: &[], points: &[] }
    }

    pub const fn points(mut self, points: &'static [(&'static str, [f64; 2])]) -> Spec {
        self.points = points;
        self
    }

    /// Give a new instance on a layer with rectangle `bounds` its starting points.
    pub fn start(&self, g: &mut Group, bounds: Rect) {
        for (path, f) in self.points {
            if let Ok(p) = g.prop_mut(path) {
                let _ = p.set_value(Value::Vec2([bounds.x0 + f[0] * bounds.width(), bounds.y0 + f[1] * bounds.height()]));
            }
        }
    }
}

/// Every effect, by family.
static FAMILIES: &[&[Spec]] = &[&[puppet::SPEC], &blur::ALL, &color::ALL, &grade::ALL, &look::ALL, &generate::ALL, &forms::ALL, &paint::ALL, &distort::ALL, &warp::ALL, &bends::ALL, &liquid::ALL, &stylize::ALL, &crafts::ALL, &grain::ALL, &film::ALL, &channel::ALL, &matte::ALL, &keys::ALL, &screen::ALL, &perspective::ALL, &transition::ALL, &time::ALL, &words::ALL, &utility::ALL, &ocio::ALL, &weather::ALL, &particles::ALL, &[hair::HAIR, playground::PLAYGROUND, cards::DANCE, cards::WIPE, shatter::SHATTER, foam::FOAM], &breakup::ALL, &water::ALL, &sphere::ALL, &passes::ALL, &obsolete::ALL, &controls::ALL, &audio::ALL, &strokes::ALL, &steady::ALL];

/// Every effect.
pub fn all() -> impl Iterator<Item = &'static Spec> {
    FAMILIES.iter().flat_map(|f| f.iter())
}

/// The effect with type `id`.
pub fn find(id: &str) -> Option<&'static Spec> {
    all().find(|s| s.def.id == id)
}

/// Make the effects (and what goes in their lists) known to a catalog, so projects holding
/// them can be read.
pub fn register(c: &mut Catalog) {
    for s in all() {
        c.add("effects", s.def);
        for (list, def) in s.lists {
            c.add(list, def);
        }
    }
}

/// How a layer's effects run.
pub struct Run<'r> {
    pub read: &'r dyn Fn(&Property) -> Value,
    /// The layer's own time.
    pub time: Tick,
    pub bounds: Rect,
    /// The layer's own (each effect's seed comes from it and the effect's id).
    pub seed: u64,
    pub adjustment: bool,
    pub host: Option<&'r dyn Host>,
    /// Steps a channel is rounded to after each effect (an 8 or 16-bit project), if any.
    pub levels: Option<f32>,
    /// Only the first this many effects of the list run.
    pub upto: usize,
    /// How long a frame of the composition lasts.
    pub frame: Tick,
    pub masks: &'r [MaskShape],
    pub read_at: Option<ReadAt<'r>>,
    /// The composition's shutter when the layer is motion blurred (see [`Ctx::shutter`]).
    pub shutter: Option<(f64, f64, u32)>,
    /// The composition's time as it is shown, and its frame rate.
    pub comp_time: Tick,
    pub rate: FrameRate,
    /// Draw what the effects' own tools show (see [`Ctx::marks`]).
    pub marks: bool,
}

/// Run a layer's effects, top to bottom, on its pixels. What they said goes to `notes`.
pub fn run_all(effects: &Group, buf: Buf, how: &Run, notes: &mut Vec<String>) -> Buf {
    let mut buf = buf;
    // effects work on the whole layer (a layer's own pixels may cover only part of it); under
    // an adjustment layer the picture already is the whole composition
    if !how.adjustment {
        buf.cover(how.bounds);
    }
    for (index, node) in effects.children.iter().enumerate().take(how.upto) {
        let veymelo_project::Node::Group(g) = node else { continue };
        if !g.enabled {
            continue;
        }
        let Some(spec) = find(g.def.id) else { continue };
        let mut ctx = Ctx::new(g, how.read, how.time, how.bounds, how.seed ^ g.id.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        ctx.adjustment = how.adjustment;
        ctx.float = how.levels.is_none();
        ctx.index = index;
        ctx.frame = how.frame;
        ctx.masks = how.masks;
        ctx.read_at = how.read_at;
        ctx.host = how.host;
        ctx.shutter = how.shutter;
        ctx.comp_time = how.comp_time;
        ctx.rate = how.rate;
        ctx.marks = how.marks;
        buf = (spec.run)(&ctx, buf);
        if let Some(levels) = how.levels {
            buf.img.clamp();
            buf.img.quantize(levels);
        }
        for n in ctx.notes() {
            if !notes.contains(&n) {
                notes.push(n);
            }
        }
    }
    buf
}
