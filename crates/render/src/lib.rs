//! Drawing a frame of a composition, all by our own code.
//!
//! Layers are drawn bottom to top. Each makes its pixels in its own space, drawn at the size it
//! will be seen (so edges stay sharp however it is scaled), and those pixels are placed into the
//! composition through its transform and its parents', blurred along its motion when motion blur
//! is on, cut by its track matte or by what is under it (preserve transparency), and mixed in
//! with its blend mode and opacity. A composition layer draws the composition it shows, or with
//! collapse on, draws that composition's layers straight in.
//!
//! The composition's background colour is not a layer: layers are drawn on clear and the
//! background goes under the finished picture, so blend modes and stencils never mix with it.
//!
//! Colours are kept as stored (sRGB) unless the project mixes in linear light. In 8 and 16-bit
//! projects every step is rounded to that many bits, as such a project is; in 32-bit ones colours
//! may go past white. What cannot be drawn yet is left out and said in the notes.

mod bounce;
mod colour_glyph;
mod extrude;
mod gpu;
mod masks;
mod meshes;
mod passes;
pub mod audio;
pub mod png;
mod scene;
mod shapes;
mod styles;
mod text;
mod three;

use std::cell::RefCell;
use std::sync::Arc;

use veymelo_color::{BlendMode, Rgba, linear_to_srgb, luminance, srgb_to_linear};
use veymelo_effects::{Buf, Take};
use veymelo_expr::{Eval, Place};
use veymelo_geom::{Mat3, Rect, Vec2, vec2};
use veymelo_keyframe::Value;
use veymelo_project::queue::ProxyUse;
use veymelo_project::{Comp, Footage, FootageKind, FrameBlend, ItemKind, Layer, MatteKind, Project, Property, Quality, Source};
use veymelo_raster::{Image, Px, Sampling, accumulate, blend_onto, place};
use veymelo_time::Tick;

/// Gives a property's value for the layer being drawn, at its time, with its expression.
pub(crate) type Read<'r> = &'r dyn Fn(&Property) -> Value;

/// How to draw.
#[derive(Clone, Debug)]
pub struct Options {
    /// The size to draw at, 1 for the composition's own (0.5 for half).
    pub scale: f64,
    /// Draw the composition's background colour (else it is clear).
    pub background: bool,
    /// Blur moving layers whose motion blur switch is on (when their composition allows it).
    pub motion_blur: bool,
    /// Draw guide layers too (they are left out of renders).
    pub guides: bool,
    /// Every layer drawn at this quality (none: as each layer's switch says).
    pub quality: Option<Quality>,
    /// Effects all on or all off (none: as each layer's switch says).
    pub effects: Option<bool>,
    /// Follow the solo switches (else every layer is drawn as if none were soloed).
    pub solo: bool,
    /// Bits a colour, 8, 16 or 32 (none: the project's).
    pub depth: Option<u8>,
    /// Which proxies are drawn in place of what they stand for (by default those switched on).
    pub proxies: ProxyUse,
    /// Draw on the GPU what it can draw (Metal on Apple silicon; by default when there is one),
    /// the same pictures as the processor's to within the last bits of a float.
    pub gpu: bool,
    /// A composition seen other than through its own camera (a preview looked round).
    pub orbit: Option<veymelo_expr::Orbit>,
    /// Draw puppet meshes and pins over the layers they bend (a preview).
    pub puppet_meshes: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options { scale: 1.0, background: true, motion_blur: true, guides: false, quality: None, effects: None, solo: true, depth: None, proxies: ProxyUse::Current, gpu: veymelo_os::gpu::available(), orbit: None, puppet_meshes: false }
    }
}

/// A frame, and what of it could not be drawn yet.
#[derive(Clone, Debug)]
pub struct Rendered {
    pub image: Image,
    pub notes: Vec<String>,
}

/// A frame as a PNG picture.
pub fn to_png(image: &Image) -> Vec<u8> {
    png::encode(image.width as u32, image.height as u32, &image.to_rgba8())
}

/// How deep compositions may go inside each other.
const MAX_DEPTH: usize = 32;

/// The most pixels a layer's picture is drawn with (a 3D layer far bigger than the frame, seen
/// close, would otherwise ask for many times the memory there is); past it, it is drawn less
/// finely.
pub const MOST_PIXELS: f64 = 24_000_000.0;

/// Detail `s` held so `area` (square units) drawn at it stays within [`MOST_PIXELS`].
fn held(s: f64, area: f64) -> f64 {
    if area * s * s > MOST_PIXELS { (MOST_PIXELS / area).sqrt().max(1.0 / 64.0) } else { s }
}
/// The most pixels across a layer is drawn at.
const MAX_SIDE: f64 = 8192.0;

pub use gpu::{let_go, scenes_drawn};
pub use veymelo_expr::Orbit;

/// Composition `comp` of `project` at time `t`.
pub fn render(project: &Project, comp: u64, t: Tick, options: &Options) -> Result<Rendered, String> {
    // its pictures filtered by the GPU too, or all of it drawn by the processor
    veymelo_raster::gpu::with_mode(!options.gpu, || render_on(project, comp, t, options))
}

fn render_on(project: &Project, comp: u64, t: Tick, options: &Options) -> Result<Rendered, String> {
    let c = project.comp(comp).ok_or_else(|| format!("there is no composition {comp}"))?;
    if !(options.scale > 0.0 && options.scale <= 4.0) {
        return Err(format!("the scale is more than 0 and at most 4, not {}", options.scale));
    }
    let (w, h) = (((c.width as f64 * options.scale).round() as usize).max(1), ((c.height as f64 * options.scale).round() as usize).max(1));
    let r = Renderer::new(project, options);
    let mut image = Image::new(w, h);
    let view = Mat3::scale(vec2(w as f64 / c.width as f64, h as f64 / c.height as f64));
    r.comp(c, t, view, &mut image, 1.0, 0);
    if options.background {
        let bg = r.px(c.background.with_alpha(1.0), 1.0);
        veymelo_raster::par::chunks(&mut image.px, 1 << 14, |_, part| {
            for p in part {
                let k = 1.0 - p[3];
                *p = [p[0] + bg[0] * k, p[1] + bg[1] * k, p[2] + bg[2] * k, p[3] + k];
            }
        });
        r.settle(&mut image);
    }
    if r.linear {
        image.map_colors(|v| linear_to_srgb(v as f64) as f32);
    }
    let mut notes = r.notes.into_inner();
    notes.extend(r.eval.notes());
    Ok(Rendered { image, notes })
}

/// The pixels of layer `layer` of composition `comp` at composition time `t`, in its own space
/// at one pixel to a unit, all of it, cut by its masks but before its effects (what a puppet
/// mesh is made over).
pub fn layer_pixels(project: &Project, comp: u64, layer: u64, t: Tick) -> Option<Buf> {
    let c = project.comp(comp)?;
    let l = c.layer(layer)?;
    let options = Options::default();
    let r = Renderer { fixed: Some(1.0), ..Renderer::new(project, &options) };
    let mut buf = r.pixels(c, l, t, Mat3::default(), &Image::new(1, 1), 0)?;
    let at = r.at(c, l);
    masks::apply(l, &masks::Reading { read: &|p, t| r.eval.value(p, at, t), t, shutter: None }, &mut buf.img, buf.origin, buf.scale);
    Some(buf)
}

/// A 3D model layer's model posed at composition time `t`, and the map from the model's units
/// to the composition's world (none when the layer is not a 3D model layer or its file cannot
/// be read).
pub fn model_in_world(project: &Project, comp: u64, layer: u64, t: Tick) -> Option<(std::sync::Arc<veymelo_model::Model>, veymelo_model::Pose, veymelo_geom::Mat4)> {
    let c = project.comp(comp)?;
    let l = c.layer(layer)?;
    let Source::Model(item) = l.source else { return None };
    let Some(veymelo_project::ItemKind::Footage(f)) = project.item(item).map(|i| &i.kind) else { return None };
    let model = veymelo_media::model(&f.path).ok()?;
    let eval = veymelo_expr::Eval::new(project);
    let at = eval.place(c, l);
    let cam = scene::camera(&eval, c, t);
    let (pose, to_layer) = meshes::posed(&eval, at, &model, t);
    let world = eval.world(at, t, Some(cam.eye)) * to_layer;
    Some((model, pose, world))
}

/// A layer's own pixels at composition time `t`, before its masks and effects (what a tracker
/// follows), in its own space at one pixel to a unit.
pub fn source_pixels(project: &Project, comp: u64, layer: u64, t: Tick) -> Option<Buf> {
    let c = project.comp(comp)?;
    let l = c.layer(layer)?;
    let options = Options::default();
    let r = Renderer { fixed: Some(1.0), ..Renderer::new(project, &options) };
    r.pixels(c, l, t, Mat3::default(), &Image::new(1, 1), 0)
}

/// `source_pixels`, drawn at `scale` pixels to the layer's unit (small: a picture of it), and
/// how big its own picture is when its source says (a solid's, a composition's, footage's).
pub fn source_pixels_at(project: &Project, comp: u64, layer: u64, t: Tick, scale: f64) -> Option<Buf> {
    let c = project.comp(comp)?;
    let l = c.layer(layer)?;
    let options = Options { gpu: false, ..Options::default() };
    let r = Renderer { fixed: Some(scale), ..Renderer::new(project, &options) };
    r.pixels(c, l, t, Mat3::default(), &Image::new(1, 1), 0)
}

pub fn source_extent(project: &Project, layer: &Layer) -> Option<(f64, f64)> {
    source_size(project, layer)
}

/// The pixels effect number `upto` (from 0) of a layer works on, at composition time `t`: the
/// layer's, cut by its masks and changed by the effects above it, in its own space at one pixel
/// to a unit; and whether they are in linear light (else as they are seen).
pub fn effect_input(project: &Project, comp: u64, layer: u64, upto: usize, t: Tick) -> Option<(Buf, bool)> {
    let c = project.comp(comp)?;
    let l = c.layer(layer)?;
    let options = Options::default();
    let r = Renderer { fixed: Some(1.0), ..Renderer::new(project, &options) };
    let mut buf = r.pixels(c, l, t, Mat3::default(), &Image::new(1, 1), 0)?;
    let at = r.at(c, l);
    masks::apply(l, &masks::Reading { read: &|p, t| r.eval.value(p, at, t), t, shutter: None }, &mut buf.img, buf.origin, buf.scale);
    let buf = r.effects(c, l, t, buf, 0, upto);
    Some((buf, r.linear))
}

/// What expressions ask of the pictures, drawn for them: a layer's box and its colours.
struct Seen;

thread_local! {
    /// How many pictures are being drawn for expressions now, one inside another (a layer's
    /// expression reading one whose own reads another…): past two, nothing more is drawn.
    static LOOKING: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Run `f` as one more picture drawn for expressions, if not too many are already.
fn looking<T>(f: impl FnOnce() -> Option<T>) -> Option<T> {
    if LOOKING.get() >= 2 {
        return None;
    }
    LOOKING.set(LOOKING.get() + 1);
    let out = f();
    LOOKING.set(LOOKING.get() - 1);
    out
}

impl veymelo_expr::Pictures for Seen {
    fn bounds(&self, project: &Project, comp: u64, layer: u64, t: Tick) -> Option<[f64; 4]> {
        looking(|| {
            let c = project.comp(comp)?;
            let l = c.layer(layer)?;
            let options = Options::default();
            let r = Renderer { fixed: Some(1.0), ..Renderer::new(project, &options) };
            let buf = r.pixels(c, l, t, Mat3::default(), &Image::new(1, 1), 0)?;
            let img = &buf.img;
            let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
            for y in 0..img.height {
                for x in 0..img.width {
                    if img.px[y * img.width + x][3] > 0.5 / 255.0 {
                        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                    }
                }
            }
            if x0 == usize::MAX {
                return Some([0.0; 4]);
            }
            let a = buf.origin + vec2(x0 as f64, y0 as f64) * (1.0 / buf.scale);
            Some([a.x, a.y, (x1 + 1 - x0) as f64 / buf.scale, (y1 + 1 - y0) as f64 / buf.scale])
        })
    }

    fn sample(&self, project: &Project, comp: u64, layer: u64, point: [f64; 2], radius: [f64; 2], after: bool, t: Tick) -> Option<[f64; 4]> {
        looking(|| {
            let (buf, linear) = effect_input(project, comp, layer, if after { usize::MAX } else { 0 }, t)?;
            let c = buf.pixel(vec2(point[0], point[1]));
            let (rx, ry) = ((radius[0] * buf.scale).max(0.5), (radius[1] * buf.scale).max(0.5));
            let mut sum = [0.0f64; 4];
            let mut n = 0usize;
            for y in (c.y - ry).floor() as i64..=(c.y + ry).ceil() as i64 {
                for x in (c.x - rx).floor() as i64..=(c.x + rx).ceil() as i64 {
                    let (dx, dy) = ((x as f64 + 0.5 - c.x) / rx, (y as f64 + 0.5 - c.y) / ry);
                    if dx * dx + dy * dy > 1.0 {
                        continue;
                    }
                    let p = buf.img.get(x, y);
                    (0..4).for_each(|k| sum[k] += p[k] as f64);
                    n += 1;
                }
            }
            if n == 0 {
                // smaller than a pixel: the one it is in
                let p = buf.img.get(c.x.floor() as i64, c.y.floor() as i64);
                (0..4).for_each(|k| sum[k] = p[k] as f64);
                n = 1;
            }
            let a = sum[3] / n as f64;
            if sum[3] <= 1e-9 {
                return Some([0.0; 4]);
            }
            let straight = |k: usize| {
                let v = (sum[k] / sum[3]).clamp(0.0, 1.0);
                if linear { veymelo_color::linear_to_srgb(v) } else { v }
            };
            Some([straight(0), straight(1), straight(2), a])
        })
    }
}

/// Let expressions read pictures (sourceRectAtTime, sampleImage) through this crate's drawing.
pub fn show_pictures() {
    veymelo_expr::show_pictures(&Seen);
}

struct Renderer<'a> {
    project: &'a Project,
    /// Every property's value, with its expression.
    eval: Eval<'a>,
    options: &'a Options,
    /// Draw layers whole at this many pixels to a unit (else at the detail they are seen with).
    fixed: Option<f64>,
    /// Colours are mixed in linear light.
    linear: bool,
    /// Steps a channel is rounded to after each layer (none in 32-bit projects).
    levels: Option<f32>,
    /// Layers whose effects are being worked out, outermost first (an effect that reads one of
    /// them gets it without its effects, so nothing leads back to itself).
    within: Vec<u64>,
    notes: RefCell<Vec<String>>,
}

/// The other layers of a composition, and the effect's own layer at other times, for effects
/// that read them (a blur map, a matte, neighbouring frames).
struct Others<'r, 'a> {
    r: &'r Renderer<'a>,
    comp: &'r Comp,
    /// The layer whose effects run (none for an adjustment layer's).
    this: Option<&'r Layer>,
    t: Tick,
    scale: f64,
    depth: usize,
    within: Vec<u64>,
}

impl Others<'_, '_> {
    /// A renderer drawing layers whole at the effect's pixels to a unit.
    fn helper(&self) -> Renderer<'_> {
        Renderer { fixed: Some(self.scale), within: self.within.clone(), ..Renderer::new(self.r.project, self.r.options) }
    }

    fn keep_notes(&self, r: Renderer) {
        for n in r.notes.into_inner().into_iter().chain(r.eval.notes()) {
            self.r.note(n);
        }
    }
}

impl veymelo_effects::Host for Others<'_, '_> {
    fn layer(&self, id: u64, take: Take) -> Option<(Buf, Rect)> {
        let l = self.comp.layer(id)?;
        if !l.is_active_at(self.t) || self.depth >= MAX_DEPTH {
            return None;
        }
        let r = self.helper();
        let buf = r.source(self.comp, l, self.t, Mat3::default(), &Image::new(1, 1), self.depth + 1, take);
        self.keep_notes(r);
        Some((buf?, self.r.bounds(self.comp, l)))
    }

    fn layer_at(&self, id: u64, shift: Tick, take: Take) -> Option<(Buf, Rect)> {
        let l = self.comp.layer(id)?;
        let t = self.t + shift;
        if !l.is_active_at(t) || self.depth >= MAX_DEPTH {
            return None;
        }
        let r = self.helper();
        let buf = r.source(self.comp, l, t, Mat3::default(), &Image::new(1, 1), self.depth + 1, take);
        self.keep_notes(r);
        Some((buf?, self.r.bounds(self.comp, l)))
    }

    fn scene(&self) -> Option<veymelo_effects::CompScene> {
        let (layer, comp, t, r) = (self.this?, self.comp, self.t, self.r);
        let cam = scene::camera(&r.eval, comp, t);
        let flat = r.matrix(comp, layer, t);
        let world = if layer.is_3d() { r.eval.world(r.at(comp, layer), t, Some(cam.eye)) } else { veymelo_geom::Mat4::from_mat3(&flat) };
        let (w, h) = (comp.width as f64, comp.height as f64);
        // seen through the camera, then back from the composition's pixels to the layer's units
        let lens = flat.inverse()? * Mat3([[cam.zoom, 0.0, w / 2.0], [0.0, cam.zoom, h / 2.0], [0.0, 0.0, 1.0]]);
        let seen = cam.view * world;
        let camera: [[f64; 4]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| lens.0[i][k] * seen.0[k][j]).sum()));
        let unworld = world.inverse()?;
        let eye = unworld.apply(cam.eye);
        let light = scene::lights(&r.eval, comp, t, r.linear).first().map(|l| veymelo_effects::CompLight {
            kind: match l.kind {
                veymelo_project::LightKind::Parallel => 0,
                veymelo_project::LightKind::Spot => 1,
                veymelo_project::LightKind::Point => 2,
                veymelo_project::LightKind::Ambient | veymelo_project::LightKind::Environment => 3,
            },
            at: unworld.apply(l.at).into(),
            dir: unworld.apply_vec(l.dir).normalize().into(),
            color: l.color,
        });
        Some(veymelo_effects::CompScene { camera, eye: eye.into(), light })
    }

    fn passes(&self) -> Option<Arc<veymelo_raster::passes::Passes>> {
        let (l, r) = (self.this?, self.r);
        // a composition's: from its layers as they are drawn, at the effect's detail
        if let Source::Comp(id) = &l.source {
            let inner = r.project.comp(*id)?;
            let nt = r.eval.source_time(r.at(self.comp, l), self.t);
            if self.depth >= MAX_DEPTH || nt < Tick::ZERO || nt >= inner.duration {
                return None;
            }
            let helper = self.helper();
            let p = helper.comp_passes(inner, nt, self.scale, self.depth + 1);
            self.keep_notes(helper);
            return Some(Arc::new(p));
        }
        let Source::Footage(id) = &l.source else { return None };
        let Some(ItemKind::Footage(f)) = r.project.item(*id).map(|i| &i.kind) else { return None };
        if !veymelo_media::is_exr(&f.path) {
            return None;
        }
        let st = r.eval.source_time(r.at(self.comp, l), self.t).seconds();
        let (n, _) = veymelo_media::frame_at(f, st)?;
        match veymelo_media::passes(&f.frame_path(n)) {
            // its pixels over the layer's (the footage's, made square)
            Ok(p) => {
                let wide = (f.width as f64 * f.pixel_aspect).round().max(1.0);
                if (p.width as f64 - wide).abs() < 0.5 {
                    return Some(p);
                }
                let mut q = (*p).clone();
                q.scale = p.width as f64 / wide;
                Some(Arc::new(q))
            }
            Err(e) => {
                r.note(format!("\"{}\": {e}", l.name));
                None
            }
        }
    }

    fn this_at(&self, time: Tick, upto: usize) -> Option<Buf> {
        let l = self.this?;
        if self.depth >= MAX_DEPTH {
            return None;
        }
        let t = l.comp_time(time);
        let r = self.helper();
        let buf = r.source(self.comp, l, t, Mat3::default(), &Image::new(1, 1), self.depth + 1, Take::Masks).map(|buf| r.effects(self.comp, l, t, buf, self.depth + 1, upto));
        self.keep_notes(r);
        buf
    }
}

/// The size of a layer's own space, if it has one.
/// How much a track matte pixel `q` lets through, by its kind.
pub(crate) fn matte_of(q: Px, kind: MatteKind) -> f32 {
    match kind {
        MatteKind::Alpha => q[3],
        MatteKind::AlphaInverted => 1.0 - q[3],
        MatteKind::Luma => luminance(q[0] as f64, q[1] as f64, q[2] as f64) as f32,
        MatteKind::LumaInverted => 1.0 - luminance(q[0] as f64, q[1] as f64, q[2] as f64) as f32,
    }
    .clamp(0.0, 1.0)
}

fn source_size(project: &Project, layer: &Layer) -> Option<(f64, f64)> {
    match &layer.source {
        Source::Solid { width, height, .. } => Some((*width as f64, *height as f64)),
        Source::Comp(id) => project.comp(*id).map(|c| (c.width as f64, c.height as f64)),
        Source::Footage(id) => match project.item(*id).map(|i| &i.kind) {
            // square pixels: a pixel `pixel_aspect` wide is that many across
            Some(ItemKind::Footage(f)) if f.kind != FootageKind::Audio && f.width > 0 => Some(((f.width as f64 * f.pixel_aspect).round().max(1.0), f.height as f64)),
            _ => None,
        },
        _ => None,
    }
}

impl<'a> Renderer<'a> {
    fn new(project: &'a Project, options: &'a Options) -> Renderer<'a> {
        show_pictures();
        let levels = match options.depth.unwrap_or(project.settings.bit_depth) {
            8 => Some(255.0),
            16 => Some(32768.0),
            _ => None,
        };
        let mut eval = Eval::new(project);
        eval.orbit = options.orbit;
        Renderer { project, eval, options, fixed: None, linear: project.settings.linear_blending, levels, within: Vec::new(), notes: RefCell::new(Vec::new()) }
    }

    /// A layer's rectangle in its own units; one without a size of its own has the
    /// composition's, about its origin.
    fn bounds(&self, comp: &Comp, layer: &Layer) -> Rect {
        match source_size(self.project, layer) {
            Some((w, h)) => Rect::from_size(w, h),
            None => Rect::new(-(comp.width as f64) / 2.0, -(comp.height as f64) / 2.0, comp.width as f64 / 2.0, comp.height as f64 / 2.0),
        }
    }

    fn at<'l>(&self, comp: &'l Comp, layer: &'l Layer) -> Place<'l>
    where
        'a: 'l,
    {
        self.eval.place(comp, layer)
    }

    /// Where a layer's own pixels go in its composition at `t`.
    fn matrix(&self, comp: &Comp, layer: &Layer, t: Tick) -> Mat3 {
        self.eval.matrix(self.at(comp, layer), t)
    }

    fn note(&self, text: String) {
        let mut n = self.notes.borrow_mut();
        if !n.contains(&text) {
            n.push(text);
        }
    }

    /// A colour as a pixel in the working space, times `alpha`.
    fn px(&self, c: Rgba, alpha: f64) -> Px {
        let a = (c.a * alpha).clamp(0.0, 1.0) as f32;
        let ch = |v: f64| if self.linear { srgb_to_linear(v) as f32 } else { v as f32 };
        [ch(c.r) * a, ch(c.g) * a, ch(c.b) * a, a]
    }

    /// Round the picture to the project's bits.
    fn settle(&self, img: &mut Image) {
        if let Some(levels) = self.levels {
            img.clamp();
            img.quantize(levels);
        }
    }

    /// `settle`, within columns and rows `area` (x0, y0, x1, y1) only: enough after drawing that
    /// changed nothing else, as a canvas is settled once each layer is drawn.
    fn settle_in(&self, img: &mut Image, area: Option<[usize; 4]>) {
        if let (Some(levels), Some(area)) = (self.levels, area) {
            img.settle_in(levels, area);
        }
    }

    /// Draw the layers of `comp` at its time `t` onto `canvas`, `view` taking its pixels to the
    /// canvas's, each layer's opacity times `fade`.
    #[allow(clippy::too_many_arguments)]
    fn comp(&self, comp: &Comp, t: Tick, view: Mat3, canvas: &mut Image, fade: f64, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let shown = self.shown(comp, t, depth);
        // 3D layers one after another make one scene (cameras, lights and nulls between them
        // draw nothing and do not part them); a 2D layer starts another
        let mut k = 0;
        while k < shown.len() {
            if scene::in_3d(shown[k]) {
                let mut run = Vec::new();
                while k < shown.len() && (scene::in_3d(shown[k]) || matches!(shown[k].source, Source::Camera | Source::Light(_) | Source::Null)) {
                    if scene::in_3d(shown[k]) {
                        self.members(comp, shown[k], t, &[], &mut run);
                    }
                    k += 1;
                }
                self.run_3d(comp, &run, t, view, canvas, fade, depth, None);
            } else {
                self.layer(comp, shown[k], t, view, canvas, fade, depth);
                k += 1;
            }
        }
    }

    /// The layers of `comp` drawn at `t`, bottom first.
    fn shown<'c>(&self, comp: &'c Comp, t: Tick, depth: usize) -> Vec<&'c Layer> {
        let guides = self.options.guides && depth == 0;
        let solo = self.options.solo && comp.layers.iter().any(|l| l.switches.solo && l.switches.video);
        comp.layers
            .iter()
            .rev()
            // an environment layer lights the scene and is not drawn itself
            .filter(|l| l.is_active_at(t) && l.switches.video && !l.environment && (!l.switches.guide || guides) && (!solo || l.switches.solo))
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn layer(&self, comp: &Comp, layer: &Layer, t: Tick, view: Mat3, canvas: &mut Image, fade: f64, depth: usize) {
        let name = &layer.name;
        let alpha = self.eval.opacity(self.at(comp, layer), t) * fade;
        // (a camera, light or null alone draws nothing)
        if layer.is_3d() && !matches!(layer.source, Source::Camera | Source::Light(_) | Source::Null) {
            self.note(format!("\"{name}\" is 3D; it is drawn flat for now"));
        }
        if layer.switches.adjustment {
            // an adjustment layer changes what is under it by its effects; with none, nothing
            if self.has_effects(layer) && alpha > 0.0 {
                self.adjust(comp, layer, t, view, canvas, alpha, depth);
            }
            return;
        }
        if alpha <= 0.0 && !layer.blend.is_stencil() {
            return;
        }
        // a collapsed composition draws its layers straight into this one, sharp at any size
        let has = |list: &str| layer.props.group(list).is_ok_and(|g| !g.children.is_empty());
        if let Source::Comp(id) = layer.source
            && layer.switches.collapse
            && self.proxy(id).is_none()
            && layer.matte.is_none()
            && !layer.preserve_transparency
            && !has("masks")
            && !has("effects")
            && styles::list(layer).is_none()
            && let Some(original) = self.project.comp(id)
        {
            let set = self.instance(comp, layer, id, t);
            let inner: &Comp = set.as_deref().unwrap_or(original);
            let at = self.at(comp, layer);
            if !self.blurs(comp, layer) {
                let nt = self.eval.source_time(at, t);
                if nt >= Tick::ZERO && nt < inner.duration {
                    self.comp(inner, nt, view * self.matrix(comp, layer, t), canvas, alpha, depth + 1);
                }
                return;
            }
            // moving: drawn at moments across the shutter, each over what is under it, averaged
            let mb = &comp.motion_blur;
            let frame = comp.frame_rate.frame_duration().seconds();
            let n = mb.samples.clamp(2, 64) as usize;
            let mut sum = vec![[0f32; 4]; canvas.px.len()];
            for i in 0..n {
                let when = t + Tick::from_seconds((mb.phase + mb.angle * i as f64 / (n - 1) as f64) / 360.0 * frame);
                let mut img = canvas.clone();
                let nt = self.eval.source_time(at, when);
                if nt >= Tick::ZERO && nt < inner.duration {
                    let a = self.eval.opacity(at, when) * fade;
                    self.comp(inner, nt, view * self.matrix(comp, layer, when), &mut img, a, depth + 1);
                }
                for (s, p) in sum.iter_mut().zip(&img.px) {
                    (0..4).for_each(|k| s[k] += p[k]);
                }
            }
            for (p, s) in canvas.px.iter_mut().zip(&sum) {
                *p = s.map(|v| v / n as f32);
            }
            self.settle(canvas);
            return;
        }
        if self.quality(layer) == Quality::Wireframe {
            if let Some((w, h)) = source_size(self.project, layer) {
                let m = view * self.matrix(comp, layer, t);
                outline(canvas, [vec2(0.0, 0.0), vec2(w, 0.0), vec2(w, h), vec2(0.0, h)].map(|p| m.apply(p)), layer.label.color());
                self.settle(canvas);
            }
            return;
        }
        let Some(buf) = self.source(comp, layer, t, view, canvas, depth, Take::All) else { return };
        let styled = self.styled(comp, layer, t, &buf);
        let mats = self.placements(comp, layer, t, view, &buf);
        let how = self.sampling(layer);
        let seed = self.seed(comp, layer, t);
        let matte = layer.matte.and_then(|m| comp.layer(m.layer).filter(|ml| ml.id != layer.id).map(|ml| (ml, m.kind)));
        // all of the canvas the layer's drawing can change (only that needs settling after it)
        let (cw, ch) = (canvas.width, canvas.height);
        let mut area: Option<[usize; 4]> = None;
        let mut reaches = |img: &Image, mats: &[Mat3], mode: BlendMode| {
            for m in mats {
                let r = if mode.is_stencil() { [0, 0, cw, ch] } else { veymelo_raster::reach(cw, ch, img, m) };
                area = Some(area.map_or(r, |a| [a[0].min(r[0]), a[1].min(r[1]), a[2].max(r[2]), a[3].max(r[3])]));
            }
        };
        if matte.is_some() || layer.preserve_transparency {
            // drawn alone first, cut, then mixed in
            let mut alone = Image::new(canvas.width, canvas.height);
            match &styled {
                Some((st, _)) => {
                    let flat = st.flat();
                    let mats = self.placements(comp, layer, t, view, &flat);
                    reaches(&flat.img, &mats, layer.blend);
                    self.put(&mut alone, &flat, &mats, how, BlendMode::Normal, 1.0, seed);
                }
                None => {
                    reaches(&buf.img, &mats, layer.blend);
                    self.put(&mut alone, &buf, &mats, how, BlendMode::Normal, 1.0, seed);
                }
            }
            if let Some((ml, kind)) = matte {
                let cut = self.alone(comp, ml, t, view, canvas.width, canvas.height, depth);
                for (p, q) in alone.px.iter_mut().zip(&cut.px) {
                    let k = matte_of(*q, kind);
                    *p = p.map(|v| v * k);
                }
            }
            if layer.preserve_transparency {
                for (p, q) in alone.px.iter_mut().zip(&canvas.px) {
                    *p = p.map(|v| v * q[3]);
                }
            }
            blend_onto(canvas, &alone, layer.blend, alpha as f32, seed);
        } else if let Some((st, blending)) = &styled {
            // what goes under it, each by its own mode
            for (under, mode) in &st.under {
                let mats = self.placements(comp, layer, t, view, under);
                reaches(&under.img, &mats, *mode);
                self.put(canvas, under, &mats, how, *mode, alpha as f32, seed);
            }
            let mats = self.placements(comp, layer, t, view, &st.body);
            if blending.knockout != 0 {
                // its shape cuts through what is under it
                let mut cover = Image::new(canvas.width, canvas.height);
                let own = self.placements(comp, layer, t, view, &buf);
                reaches(&buf.img, &own, BlendMode::Normal);
                self.put(&mut cover, &buf, &own, how, BlendMode::Normal, 1.0, seed);
                for (p, c) in canvas.px.iter_mut().zip(&cover.px) {
                    let k = 1.0 - c[3].clamp(0.0, 1.0) * alpha as f32;
                    *p = p.map(|v| v * k);
                }
            }
            let before = (blending.channels != [true; 3]).then(|| canvas.clone());
            reaches(&st.body.img, &mats, layer.blend);
            self.put(canvas, &st.body, &mats, how, layer.blend, alpha as f32, seed);
            if let Some(before) = before {
                styles::into_channels(canvas, &before, blending.channels);
            }
        } else {
            reaches(&buf.img, &mats, layer.blend);
            self.put(canvas, &buf, &mats, how, layer.blend, alpha as f32, seed);
        }
        self.settle_in(canvas, area);
    }

    /// The layer's styles drawn round its pixels, and how it is blended, when it has any on.
    fn styled(&self, comp: &Comp, layer: &Layer, t: Tick, buf: &Buf) -> Option<(styles::Styled, styles::Blending)> {
        let list = styles::list(layer)?;
        let at = self.at(comp, layer);
        let read = |p: &Property| self.eval.value(p, at, t);
        let blending = styles::blending(list, &read);
        Some((styles::draw(list, &read, buf, self.bounds(comp, layer), self.linear, layer.id as u32), blending))
    }

    /// An adjustment layer: its effects run on what is under it, and the result takes the place
    /// of what was there where the layer is (its own pixels: a solid's rectangle, its masks), as
    /// much as its opacity says, mixed by its blend mode.
    #[allow(clippy::too_many_arguments)]
    fn adjust(&self, comp: &Comp, layer: &Layer, t: Tick, view: Mat3, canvas: &mut Image, alpha: f64, depth: usize) {
        let Ok(effects) = layer.props.group("effects") else { return };
        let m = view * self.matrix(comp, layer, t);
        // what is under, as the layer's own pixels: exact while the layer is not turned or slanted
        let (ax, ay) = (m.apply_vec(vec2(1.0, 0.0)), m.apply_vec(vec2(0.0, 1.0)));
        let upright = ax.y.abs() < 1e-9 && ay.x.abs() < 1e-9 && ax.x > 0.0 && (ax.x - ay.y).abs() < 1e-9 * ax.x.max(1.0);
        let frame = if upright { m } else { view };
        if !upright {
            self.note(format!("\"{}\" is an adjustment layer turned or slanted; its effects work as if it were not", layer.name));
        }
        let Some(inv) = frame.inverse() else { return };
        let scale = frame.apply_vec(vec2(1.0, 0.0)).length();
        let below = Buf { img: canvas.clone(), origin: inv.apply(vec2(0.0, 0.0)), scale };
        let at = self.at(comp, layer);
        let read = |p: &Property| self.eval.value(p, at, t);
        let within: Vec<u64> = self.within.iter().copied().chain([layer.id]).collect();
        let host = Others { r: self, comp, this: None, t, scale, depth, within };
        let shapes = self.mask_shapes(comp, layer, t);
        let read_at = |p: &Property, lt: Tick| self.eval.value(p, at, layer.comp_time(lt));
        let how = veymelo_effects::Run {
            read: &read,
            time: layer.layer_time(t),
            bounds: self.bounds(comp, layer),
            seed: layer.id,
            adjustment: true,
            host: Some(&host),
            levels: self.levels,
            upto: usize::MAX,
            frame: comp.frame_rate.frame_duration(),
            masks: &shapes,
            read_at: Some(&read_at),
            shutter: self.blurs(comp, layer).then_some((comp.motion_blur.angle, comp.motion_blur.phase, comp.motion_blur.samples)),
            comp_time: t + comp.start,
            rate: comp.frame_rate,
            marks: self.options.puppet_meshes,
        };
        let mut notes = Vec::new();
        let done = veymelo_effects::run_all(effects, below.clone(), &how, &mut notes);
        for n in notes {
            self.note(format!("\"{}\": {n}", layer.name));
        }
        // the result back on the canvas's pixels
        let mut changed = Image::new(canvas.width, canvas.height);
        let back = Mat3::translate((done.origin - below.origin) * scale) * Mat3::scale(vec2(scale / done.scale, scale / done.scale));
        place(&mut changed, &done.img, &back, Sampling::Bilinear, BlendMode::Normal, 1.0, 0);
        // where the layer is
        let mut cover = Image::new(canvas.width, canvas.height);
        if let Some(own) = self.source(comp, layer, t, view, canvas, depth, Take::Masks) {
            let mats = self.placements(comp, layer, t, view, &own);
            self.put(&mut cover, &own, &mats, self.sampling(layer), BlendMode::Normal, 1.0, 0);
        }
        let mixed = if layer.blend == BlendMode::Normal {
            changed
        } else {
            let mut m = canvas.clone();
            blend_onto(&mut m, &changed, layer.blend, 1.0, self.seed(comp, layer, t));
            m
        };
        let k = alpha as f32;
        for ((p, q), c) in canvas.px.iter_mut().zip(&mixed.px).zip(&cover.px) {
            let f = c[3].clamp(0.0, 1.0) * k;
            if f > 0.0 {
                *p = [0, 1, 2, 3].map(|i| p[i] + (q[i] - p[i]) * f);
            }
        }
        self.settle(canvas);
    }

    /// `layer` drawn by itself on a clear picture (as a track matte sees it).
    #[allow(clippy::too_many_arguments)]
    fn alone(&self, comp: &Comp, layer: &Layer, t: Tick, view: Mat3, w: usize, h: usize, depth: usize) -> Image {
        let mut img = Image::new(w, h);
        if !layer.is_active_at(t) || depth > MAX_DEPTH {
            return img;
        }
        let mut copy = layer.clone();
        copy.blend = BlendMode::Normal;
        copy.preserve_transparency = false;
        copy.switches.adjustment = false;
        self.layer(comp, &copy, t, view, &mut img, 1.0, depth + 1);
        img
    }

    /// Put a layer's pixels onto `target` at each of `mats` (several: a motion blur).
    #[allow(clippy::too_many_arguments)]
    fn put(&self, target: &mut Image, buf: &Buf, mats: &[Mat3], how: Sampling, mode: BlendMode, alpha: f32, seed: u32) {
        if let [m] = mats {
            place(target, &buf.img, m, how, mode, alpha, seed);
            return;
        }
        let mut sum = Image::new(target.width, target.height);
        let k = 1.0 / mats.len() as f32;
        for m in mats {
            accumulate(&mut sum, &buf.img, m, how, k);
        }
        blend_onto(target, &sum, mode, alpha, seed);
    }

    /// How carefully a layer is drawn: as the render says, or as its switch says.
    fn quality(&self, layer: &Layer) -> Quality {
        self.options.quality.unwrap_or(layer.switches.quality)
    }

    fn sampling(&self, layer: &Layer) -> Sampling {
        match (self.quality(layer), layer.switches.sampling) {
            (Quality::Draft, _) => Sampling::Nearest,
            (_, veymelo_project::Sampling::Bicubic) => Sampling::Bicubic,
            _ => Sampling::Bilinear,
        }
    }

    /// The Dissolve pattern: the layer's own, and for Dancing Dissolve a new one every frame.
    fn seed(&self, comp: &Comp, layer: &Layer, t: Tick) -> u32 {
        let base = (layer.id as u32).wrapping_mul(0x9E37_79B9);
        if layer.blend == BlendMode::DancingDissolve { base ^ (comp.frame_rate.frame_at(t) as u32).wrapping_mul(0x85EB_CA6B) } else { base }
    }

    /// Whether a layer has effects to apply (it is then drawn whole, for them to work on).
    fn has_effects(&self, layer: &Layer) -> bool {
        self.options.effects.unwrap_or(layer.switches.effects) && layer.props.group("effects").is_ok_and(|g| g.children.iter().any(|n| matches!(n, veymelo_project::Node::Group(g) if g.enabled)))
    }

    /// Whether only the part of a layer that is seen need be drawn (not while it moves under
    /// motion blur, nor when its effects take its pixels from elsewhere).
    fn seen_only(&self, comp: &Comp, layer: &Layer) -> bool {
        self.fixed.is_none() && !self.blurs(comp, layer) && !self.has_effects(layer) && styles::list(layer).is_none() && !scene::in_3d(layer)
    }

    /// Whether a layer is blurred along its motion.
    fn blurs(&self, comp: &Comp, layer: &Layer) -> bool {
        self.options.motion_blur && comp.enable_motion_blur && layer.switches.motion_blur && self.quality(layer) == Quality::Best
    }

    /// From the buffer's pixels to the canvas's: one matrix, or one for each moment the
    /// shutter is open while the layer moves.
    fn placements(&self, comp: &Comp, layer: &Layer, t: Tick, view: Mat3, buf: &Buf) -> Vec<Mat3> {
        let at = |dt: f64| view * self.matrix(comp, layer, t + Tick::from_seconds(dt)) * buf.to_layer();
        if !self.blurs(comp, layer) {
            return vec![at(0.0)];
        }
        let mb = &comp.motion_blur;
        let frame = comp.frame_rate.frame_duration().seconds();
        let shutter = |i: usize, n: usize| {
            let f = if n > 1 { i as f64 / (n - 1) as f64 } else { 0.5 };
            (mb.phase + mb.angle * f) / 360.0 * frame
        };
        // how far the farthest corner travels while the shutter is open, in canvas pixels
        let (w, h) = (buf.img.width as f64, buf.img.height as f64);
        let corners = [vec2(0.0, 0.0), vec2(w, 0.0), vec2(0.0, h), vec2(w, h)];
        const PROBES: usize = 8;
        let mut travel = [0.0f64; 4];
        let mut last: Option<[Vec2; 4]> = None;
        for i in 0..PROBES {
            let m = at(shutter(i, PROBES));
            let pts = corners.map(|c| m.apply(c));
            if let Some(prev) = last {
                for k in 0..4 {
                    travel[k] += (pts[k] - prev[k]).length();
                }
            }
            last = Some(pts);
        }
        let travel = travel.into_iter().fold(0.0, f64::max);
        if travel < 0.5 {
            return vec![at(0.0)];
        }
        let least = mb.samples.max(2);
        let n = (travel.ceil() as u32).clamp(least, mb.adaptive_limit.max(least)) as usize;
        (0..n).map(|i| at(shutter(i, n))).collect()
    }

    /// How many canvas pixels one of a layer's own units is, at its biggest: the layer is drawn
    /// with that many pixels to a unit, so it is never enlarged afterwards.
    fn detail(&self, comp: &Comp, layer: &Layer, t: Tick, view: Mat3) -> f64 {
        if let Some(s) = self.fixed {
            return s;
        }
        if scene::in_3d(layer) {
            // drawn whole: never more pixels than a picture is let have
            let r = self.bounds(comp, layer);
            return held(self.detail_3d(comp, layer, t, view), r.width().max(0.0) * r.height().max(0.0));
        }
        let m = view * self.matrix(comp, layer, t);
        let (sx, sy) = (m.apply_vec(vec2(1.0, 0.0)).length(), m.apply_vec(vec2(0.0, 1.0)).length());
        sx.max(sy).clamp(1.0 / 64.0, 16.0)
    }

    /// Detail `s` for a layer whose only effects are blurs: no finer than leaves its softest
    /// blur some sixteen pixels wide (finer only takes memory and time; the blur would wash it
    /// out). Other layers keep `s`.
    fn softened(&self, comp: &Comp, layer: &Layer, t: Tick, s: f64) -> f64 {
        if self.fixed.is_some() || !self.has_effects(layer) || styles::list(layer).is_some() {
            return s;
        }
        let Ok(list) = layer.props.group("effects") else { return s };
        let at = self.at(comp, layer);
        let mut least = f64::INFINITY;
        for n in &list.children {
            let veymelo_project::Node::Group(g) = n else { continue };
            if !g.enabled {
                continue;
            }
            let read = |id: &str| g.prop(id).ok().map(|p| self.eval.value(p, at, t).as_f64());
            let r = match g.def.id {
                "gaussianBlur" | "gaussianBlurLegacy" => read("blurriness"),
                "fastBoxBlur" => read("radius"),
                _ => return s,
            };
            least = least.min(r.unwrap_or(0.0));
        }
        if least.is_finite() && least >= 4.0 { s.min((16.0 / least).max(1.0 / 16.0)) } else { s }
    }

    /// The part of a flat layer the canvas shows, in its own units (none: all of it).
    fn seen_2d(&self, comp: &Comp, layer: &Layer, t: Tick, view: Mat3, canvas: &Image) -> Option<Rect> {
        if !self.seen_only(comp, layer) {
            return None;
        }
        (view * self.matrix(comp, layer, t)).inverse().map(|inv| inv.map_rect(&Rect::from_size(canvas.width as f64, canvas.height as f64).inflate(2.0)))
    }

    /// For a 3D layer whose own units reach over `r`: the part of it the canvas shows (empty
    /// when none does) and the detail
    /// to draw it with, held so its picture stays within [`MOST_PIXELS`]; none when that
    /// cannot be told (it is drawn whole, as before).
    fn seen_part(&self, comp: &Comp, layer: &Layer, t: Tick, view: Mat3, canvas: &Image, r: Rect) -> Option<(Rect, f64)> {
        if !scene::in_3d(layer) {
            return None;
        }
        let (part, s) = self.seen_3d(comp, layer, t, view, (canvas.width, canvas.height), r)?;
        let area = part.width().max(0.0) * part.height().max(0.0);
        Some((part, held(s, area)))
    }

    /// The part of a layer of size `w` × `h` that can be seen on the canvas, in its own units
    /// (all of it while it moves under motion blur, or when the transform cannot be undone).
    #[allow(clippy::too_many_arguments)]
    fn seen(&self, comp: &Comp, layer: &Layer, t: Tick, view: Mat3, canvas: &Image, w: f64, h: f64) -> Rect {
        let whole = Rect::from_size(w, h);
        if !self.seen_only(comp, layer) {
            return whole;
        }
        let Some(inv) = (view * self.matrix(comp, layer, t)).inverse() else { return whole };
        let screen = Rect::from_size(canvas.width as f64, canvas.height as f64).inflate(2.0);
        inv.map_rect(&screen).intersect(&whole)
    }

    /// A layer's pixels in its own space, drawn at the detail it is seen with: as it shows, or
    /// cut by its masks, or also changed by its effects (`take`).
    #[allow(clippy::too_many_arguments)]
    fn source(&self, comp: &Comp, layer: &Layer, t: Tick, view: Mat3, canvas: &Image, depth: usize, take: Take) -> Option<Buf> {
        if depth > MAX_DEPTH {
            return None;
        }
        let mut buf = self.pixels(comp, layer, t, view, canvas, depth)?;
        if take == Take::Source {
            return Some(buf);
        }
        let at = self.at(comp, layer);
        // the moments the shutter is open, where motion blur is drawn at all
        let shutter = (self.options.motion_blur && comp.enable_motion_blur && self.quality(layer) == Quality::Best).then(|| {
            let mb = &comp.motion_blur;
            let (frame, n) = (comp.frame_rate.frame_duration().seconds(), mb.samples.clamp(2, 64) as usize);
            (0..n).map(|i| t + Tick::from_seconds((mb.phase + mb.angle * i as f64 / (n - 1) as f64) / 360.0 * frame)).collect()
        });
        masks::apply(layer, &masks::Reading { read: &|p, t| self.eval.value(p, at, t), t, shutter }, &mut buf.img, buf.origin, buf.scale);
        if take == Take::All {
            buf = self.effects(comp, layer, t, buf, depth, usize::MAX);
        }
        (!buf.img.is_empty()).then_some(buf)
    }

    /// A layer's masks (those switched on, with a mode) as outlines, for effects.
    fn mask_shapes(&self, comp: &Comp, layer: &Layer, t: Tick) -> Vec<veymelo_effects::MaskShape> {
        let Ok(list) = layer.props.group("masks") else { return Vec::new() };
        let at = self.at(comp, layer);
        list.children
            .iter()
            .filter_map(|n| if let veymelo_project::Node::Group(g) = n { Some(g) } else { None })
            .filter(|g| g.enabled)
            .filter_map(|g| {
                let v = |p: &str| g.prop(p).ok().map(|p| self.eval.value(p, at, t));
                let path = v("path")?;
                let mut path = path.as_path()?.clone();
                if v("rotoBezier").is_some_and(|v| v.as_bool()) {
                    path = path.roto_smoothed();
                }
                Some(veymelo_effects::MaskShape { name: g.name().to_string(), points: veymelo_effects::outline(&path), closed: path.closed, inverted: v("inverted").is_some_and(|v| v.as_bool()) })
            })
            .collect()
    }

    /// A layer's pixels changed by the first `upto` of its effects.
    fn effects(&self, comp: &Comp, layer: &Layer, t: Tick, buf: Buf, depth: usize, upto: usize) -> Buf {
        if !self.has_effects(layer) || self.within.contains(&layer.id) {
            return buf;
        }
        let Ok(effects) = layer.props.group("effects") else { return buf };
        let at = self.at(comp, layer);
        let read = |p: &Property| self.eval.value(p, at, t);
        let within: Vec<u64> = self.within.iter().copied().chain([layer.id]).collect();
        let host = Others { r: self, comp, this: Some(layer), t, scale: buf.scale, depth, within };
        let shapes = self.mask_shapes(comp, layer, t);
        let read_at = |p: &Property, lt: Tick| self.eval.value(p, at, layer.comp_time(lt));
        let how = veymelo_effects::Run {
            read: &read,
            time: layer.layer_time(t),
            bounds: self.bounds(comp, layer),
            seed: layer.id,
            adjustment: false,
            host: Some(&host),
            levels: self.levels,
            upto,
            frame: comp.frame_rate.frame_duration(),
            masks: &shapes,
            read_at: Some(&read_at),
            shutter: self.blurs(comp, layer).then_some((comp.motion_blur.angle, comp.motion_blur.phase, comp.motion_blur.samples)),
            comp_time: t + comp.start,
            rate: comp.frame_rate,
            marks: self.options.puppet_meshes,
        };
        let mut notes = Vec::new();
        let buf = veymelo_effects::run_all(effects, buf, &how, &mut notes);
        for n in notes {
            self.note(format!("\"{}\": {n}", layer.name));
        }
        buf
    }

    /// A layer's own pixels, before masks.
    #[allow(clippy::too_many_arguments)]
    fn pixels(&self, comp: &Comp, layer: &Layer, t: Tick, view: Mat3, canvas: &Image, depth: usize) -> Option<Buf> {
        let name = &layer.name;
        // a 3D layer nowhere in the picture is not drawn at all (shapes and words are measured
        // by what they draw, below)
        if !matches!(layer.source, Source::Shape | Source::Text) && self.seen_part(comp, layer, t, view, canvas, self.bounds(comp, layer)).is_some_and(|(p, _)| p.is_empty()) {
            return None;
        }
        if layer.source == Source::Shape {
            let contents = layer.props.group("contents").ok()?;
            let at = self.at(comp, layer);
            let drawing = shapes::Drawing::of(contents, layer.layer_time(t), &|p| self.eval.value(p, at, t));
            let whole = drawing.bounds;
            let (s, area) = match self.seen_part(comp, layer, t, view, canvas, whole) {
                Some((part, _)) if part.is_empty() => return None,
                Some((part, s)) => (s, part),
                None => {
                    let s = self.detail(comp, layer, t, view);
                    let s = if scene::in_3d(layer) && self.fixed.is_none() { held(s, whole.width().max(0.0) * whole.height().max(0.0)) } else { s };
                    let s = self.softened(comp, layer, t, s);
                    (s, self.seen_2d(comp, layer, t, view, canvas).map_or(whole, |r| whole.intersect(&r)))
                }
            };
            let (img, origin, s) = drawing.paint(area, s, &|c, a| self.px(c, a))?;
            return Some(Buf { img, origin, scale: s });
        }
        if layer.source == Source::Text {
            let (s, seen) = (self.detail(comp, layer, t, view), self.seen_2d(comp, layer, t, view, canvas));
            let at = self.at(comp, layer);
            let (drawn, notes) = text::draw(layer, layer.layer_time(t), &|p| self.eval.value(p, at, t), &|p, vars| self.eval.value_with(p, at, t, vars), s, seen, &|c, a| self.px(c, a));
            for n in notes {
                self.note(n);
            }
            let (img, origin, s) = drawn?;
            return Some(Buf { img, origin, scale: s });
        }
        if let Source::Footage(item) | Source::Comp(item) = layer.source
            && let Some(f) = self.proxy(item)
        {
            return self.frames(comp, layer, f, source_size(self.project, layer), t);
        }
        // a vector picture: drawn at the detail it is seen with, so it stays sharp at any size
        if let Source::Footage(item) = layer.source
            && let Some(ItemKind::Footage(f)) = self.project.item(item).map(|i| &i.kind)
            && veymelo_media::is_vector(&f.path)
        {
            return self.vector(comp, layer, &f.path, veymelo_media::page_of(f), t, view, canvas);
        }
        if let Source::Footage(item) = layer.source {
            return self.footage(comp, layer, item, t);
        }
        let (w, h) = match &layer.source {
            Source::Null | Source::Camera | Source::Light(_) => return None,
            Source::Solid { .. } | Source::Comp(_) => source_size(self.project, layer)?,
            other => {
                self.note(format!("\"{name}\" ({} layer) is not drawn yet", other.id()));
                return None;
            }
        };
        let mut s = self.detail(comp, layer, t, view);
        let mut area = self.seen(comp, layer, t, view, canvas, w, h);
        // a 3D layer: only the part the camera sees, as finely as it is seen there
        if let Some((part, seen)) = self.seen_part(comp, layer, t, view, canvas, Rect::from_size(w, h)) {
            (s, area) = (seen, part.intersect(&Rect::from_size(w, h)));
        }
        if area.is_empty() {
            return None;
        }
        let biggest = area.width().max(area.height()) * s;
        if biggest > MAX_SIDE {
            s *= MAX_SIDE / biggest;
        }
        // the pixels of the layer's own grid that the seen part touches
        let (i0, j0) = ((area.x0 * s).floor().max(0.0), (area.y0 * s).floor().max(0.0));
        let (i1, j1) = ((area.x1 * s).ceil().min((w * s).ceil()), (area.y1 * s).ceil().min((h * s).ceil()));
        if i1 <= i0 || j1 <= j0 {
            return None;
        }
        let (bw, bh) = ((i1 - i0) as usize, (j1 - j0) as usize);
        let img = match &layer.source {
            Source::Solid { color, .. } => {
                let p = self.px(*color, 1.0);
                let mut img = Image::filled(bw, bh, p);
                // the last column and row cover only part of a pixel when the size is not whole
                let (ex, ey) = (w * s, h * s);
                for y in 0..bh {
                    let cy = ((j0 + y as f64 + 1.0).min(ey) - (j0 + y as f64)).clamp(0.0, 1.0);
                    for x in 0..bw {
                        let cx = ((i0 + x as f64 + 1.0).min(ex) - (i0 + x as f64)).clamp(0.0, 1.0);
                        let k = (cx * cy) as f32;
                        if k < 1.0 {
                            img.px[y * bw + x] = p.map(|v| v * k);
                        }
                    }
                }
                img
            }
            Source::Comp(id) => {
                let set = self.instance(comp, layer, *id, t);
                let inner: &Comp = match &set {
                    Some(c) => c,
                    None => self.project.comp(*id)?,
                };
                let nt = self.eval.source_time(self.at(comp, layer), t);
                if nt < Tick::ZERO || nt >= inner.duration {
                    return None;
                }
                let mut img = Image::new(bw, bh);
                let inner_view = Mat3::translate(vec2(-i0, -j0)) * Mat3::scale(vec2(s, s));
                self.comp(inner, nt, inner_view, &mut img, 1.0, depth + 1);
                img
            }
            _ => return None,
        };
        Some(Buf { img, origin: vec2(i0 / s, j0 / s), scale: s })
    }
}

impl Renderer<'_> {
    /// A per-character 3D text layer's characters at `t`, drawn at `s` pixels to a unit: each
    /// picture, where it starts, its scale and where it stands in the layer's units.
    pub(crate) fn text_chars(&self, comp: &Comp, layer: &Layer, t: Tick, s: f64) -> Vec<text::CharPicture> {
        let at = self.at(comp, layer);
        let (out, notes) = text::chars_3d(layer, layer.layer_time(t), &|p| self.eval.value(p, at, t), &|p, vars| self.eval.value_with(p, at, t, vars), s, &|c, a| self.px(c, a));
        for n in notes {
            self.note(n);
        }
        out
    }

    /// Composition `id` as `layer` shows it at `t`: with what the layer sets of its essential
    /// properties (none when it sets nothing).
    fn instance(&self, comp: &Comp, layer: &Layer, id: u64, t: Tick) -> Option<std::sync::Arc<Comp>> {
        let at = self.at(comp, layer);
        veymelo_project::essential::instance(self.project, id, layer, &|p| self.eval.value(p, at, t))
    }

    /// The file drawn in place of item `id`, when its proxy is used.
    fn proxy(&self, id: u64) -> Option<&Footage> {
        let item = self.project.item(id)?;
        let p = item.proxy.as_ref()?;
        let used = match self.options.proxies {
            ProxyUse::None => false,
            ProxyUse::Current => p.on,
            ProxyUse::All => true,
            ProxyUse::Comps => matches!(item.kind, ItemKind::Comp(_)),
        };
        (used && p.footage.kind != FootageKind::Audio && p.footage.width > 0).then_some(&p.footage)
    }

    /// A footage layer's frame at `t`.
    /// A vector picture footage layer shows, drawn at the detail it is seen with (only where
    /// it is seen).
    #[allow(clippy::too_many_arguments)]
    fn vector(&self, comp: &Comp, layer: &Layer, path: &str, page: usize, t: Tick, view: Mat3, canvas: &Image) -> Option<Buf> {
        let doc = match veymelo_media::vector_page(path, page) {
            Ok(d) => d,
            Err(e) => {
                self.note(format!("\"{}\": {e}", layer.name));
                return None;
            }
        };
        let (w, h) = (doc.width.ceil().max(1.0), doc.height.ceil().max(1.0));
        let mut s = self.detail(comp, layer, t, view);
        let mut area = self.seen(comp, layer, t, view, canvas, w, h);
        // a 3D layer: only the part the camera sees, as finely as it is seen there
        if let Some((part, seen)) = self.seen_part(comp, layer, t, view, canvas, Rect::from_size(w, h)) {
            (s, area) = (seen, part.intersect(&Rect::from_size(w, h)));
        }
        if area.is_empty() {
            return None;
        }
        let biggest = area.width().max(area.height()) * s;
        if biggest > MAX_SIDE {
            s *= MAX_SIDE / biggest;
        }
        let (i0, j0) = ((area.x0 * s).floor().max(0.0), (area.y0 * s).floor().max(0.0));
        let (i1, j1) = ((area.x1 * s).ceil().min((w * s).ceil()), (area.y1 * s).ceil().min((h * s).ceil()));
        if i1 <= i0 || j1 <= j0 {
            return None;
        }
        let place = Mat3::translate(vec2(-i0, -j0)) * Mat3::scale(vec2(s, s));
        let color = |c: [f64; 4]| self.px(Rgba { r: c[0], g: c[1], b: c[2], a: 1.0 }, c[3]);
        let img = veymelo_svg::draw(&doc, place, (i1 - i0) as usize, (j1 - j0) as usize, &color);
        Some(Buf { img, origin: vec2(i0 / s, j0 / s), scale: s })
    }

    fn footage(&self, comp: &Comp, layer: &Layer, item: u64, t: Tick) -> Option<Buf> {
        let Some(ItemKind::Footage(f)) = self.project.item(item).map(|i| &i.kind) else {
            self.note(format!("\"{}\": its footage (item {item}) is not in the project", layer.name));
            return None;
        };
        if f.kind == FootageKind::Audio {
            return None;
        }
        self.frames(comp, layer, f, None, t)
    }

    /// The frame of `f` a layer shows at `t` (fitted to `fit` across and down, for a proxy),
    /// blended between frames when the layer says so and the composition allows it (mixed, or
    /// along how things move).
    fn frames(&self, comp: &Comp, layer: &Layer, f: &Footage, fit: Option<(f64, f64)>, t: Tick) -> Option<Buf> {
        let name = &layer.name;
        let st = self.eval.source_time(self.at(comp, layer), t).seconds();
        let (n, part) = veymelo_media::frame_at(f, st)?;
        // footage there is not (yet): colour bars where it would be
        let missing = f.kind != FootageKind::Placeholder && !std::path::Path::new(&f.frame_path(n)).exists();
        if f.kind == FootageKind::Placeholder || missing {
            if missing {
                self.note(format!("\"{name}\": {} is not there (colour bars stand in for it)", f.frame_path(n)));
            }
            let (w, h) = (((f.width as f64 * f.pixel_aspect).round() as usize).max(1), (f.height as usize).max(1));
            return Some(Buf { img: bars(w, h), origin: vec2(0.0, 0.0), scale: 1.0 });
        }
        let last = (f.duration.seconds() * f.frame_rate.as_f64()).round().max(1.0) as u64 - 1;
        let get = |k: u64| match veymelo_media::frame(f, k.min(last)) {
            Ok(img) => Some(img),
            Err(e) => {
                self.note(format!("\"{name}\": {e}"));
                None
            }
        };
        let blend = if comp.enable_frame_blending && f.kind != FootageKind::Image && part > 1e-4 && n < last { layer.switches.frame_blend } else { FrameBlend::Off };
        let img = match blend {
            FrameBlend::Off => (*get(n)?).clone(),
            FrameBlend::FrameMix => {
                let (a, b) = (get(n)?, get(n + 1)?);
                let k = part as f32;
                let mut img = (*a).clone();
                if a.width == b.width && a.height == b.height {
                    for (p, q) in img.px.iter_mut().zip(&b.px) {
                        *p = [0, 1, 2, 3].map(|c| p[c] + (q[c] - p[c]) * k);
                    }
                }
                img
            }
            FrameBlend::PixelMotion => {
                let (before, a, b, after) = (get(n.saturating_sub(1))?, get(n)?, get(n + 1)?, get(n + 2)?);
                veymelo_effects::time::tween(&before, &a, &b, &after, part as f32)
            }
        };
        // a proxy's pixels stretched over what it stands for (down too, when its shape differs)
        let (img, scale) = match fit {
            Some((w, h)) if w > 0.0 && h > 0.0 => {
                let scale = img.width as f64 / w;
                let rows = (h * scale).round().max(1.0) as usize;
                (if rows == img.height { img } else { stretch_rows(&img, rows) }, scale)
            }
            _ => (img, 1.0),
        };
        Some(Buf { img, origin: vec2(0.0, 0.0), scale })
    }
}

/// Colour bars `w` × `h`: what stands in for footage there is not.
fn bars(w: usize, h: usize) -> Image {
    // seven bars at three quarters (white, yellow, cyan, green, magenta, red, blue) above a
    // strip of the same in reverse order, every other one black, and a dark band
    const TOP: [[f32; 3]; 7] = [[0.75, 0.75, 0.75], [0.75, 0.75, 0.0], [0.0, 0.75, 0.75], [0.0, 0.75, 0.0], [0.75, 0.0, 0.75], [0.75, 0.0, 0.0], [0.0, 0.0, 0.75]];
    let mut img = Image::new(w, h);
    for y in 0..h {
        let band = y as f64 / h as f64;
        for x in 0..w {
            let k = (x * 7 / w).min(6);
            let c = if band < 0.67 {
                TOP[k]
            } else if band < 0.75 {
                if k.is_multiple_of(2) { TOP[6 - k] } else { [0.0; 3] }
            } else {
                [0.075; 3]
            };
            img.px[y * w + x] = [c[0], c[1], c[2], 1.0];
        }
    }
    img
}

/// A picture made `rows` tall, its rows mixed between the two nearest.
fn stretch_rows(img: &Image, rows: usize) -> Image {
    let mut out = Image::new(img.width, rows);
    let k = img.height as f64 / rows as f64;
    for y in 0..rows {
        let v = ((y as f64 + 0.5) * k - 0.5).clamp(0.0, (img.height - 1) as f64);
        let (y0, f) = (v.floor() as usize, (v - v.floor()) as f32);
        let y1 = (y0 + 1).min(img.height - 1);
        for x in 0..img.width {
            let (a, b) = (img.px[y0 * img.width + x], img.px[y1 * img.width + x]);
            out.px[y * img.width + x] = [0, 1, 2, 3].map(|c| a[c] + (b[c] - a[c]) * f);
        }
    }
    out
}

/// A thin outline through four corners (a wireframe).
fn outline(canvas: &mut Image, corners: [Vec2; 4], color: Rgba) {
    let p = [color.r as f32, color.g as f32, color.b as f32, 1.0];
    for k in 0..4 {
        let (a, b) = (corners[k], corners[(k + 1) % 4]);
        let steps = (b - a).length().ceil().max(1.0) as usize;
        for i in 0..=steps {
            let q = a.lerp(b, i as f64 / steps as f64);
            let (x, y) = (q.x.floor() as i64, q.y.floor() as i64);
            if x >= 0 && y >= 0 && (x as usize) < canvas.width && (y as usize) < canvas.height {
                canvas.px[y as usize * canvas.width + x as usize] = p;
            }
        }
    }
}

#[cfg(test)]
mod tests;
