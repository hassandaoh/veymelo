//! A composition: a frame size, a frame rate, a length, and layers stacked top first.

use veymelo_color::Rgba;
use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{Kind, Value};
use veymelo_time::{FrameRate, Tick, TimeRange};

use crate::defs::Catalog;
use crate::layer::{Layer, Marker};
use crate::read::{LoadError, Obj, fail};

/// The largest a composition (or a solid) may be across or down.
pub const MAX_SIZE: u32 = 30_000;
/// The longest a composition may last, in seconds (a day).
pub const MAX_SECONDS: f64 = 86_400.0;

/// How motion blur is taken: the shutter is open `angle` degrees of a frame (360 = the whole
/// frame), opening `phase` degrees from the frame's time, and the blur is made of up to
/// `samples` pictures (`adaptive_limit` when a layer moves far).
#[derive(Clone, Debug, PartialEq)]
pub struct MotionBlur {
    pub angle: f64,
    pub phase: f64,
    pub samples: u32,
    pub adaptive_limit: u32,
}

impl Default for MotionBlur {
    fn default() -> MotionBlur {
        MotionBlur { angle: 180.0, phase: -90.0, samples: 16, adaptive_limit: 128 }
    }
}

/// How light bounces in its 3D scenes (global illumination): what falls on a surface lights
/// what is round it, `amount` of it (0: none, 1: all), going on from surface to surface
/// `bounces` times.
#[derive(Clone, Debug, PartialEq)]
pub struct Bounce {
    pub amount: f64,
    pub bounces: u32,
}

impl Default for Bounce {
    fn default() -> Bounce {
        Bounce { amount: 0.0, bounces: 2 }
    }
}

/// The most times light bounces.
pub const MOST_BOUNCES: u32 = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct Comp {
    pub width: u32,
    pub height: u32,
    /// How wide a pixel is for how tall (1 for square pixels).
    pub pixel_aspect: f64,
    pub frame_rate: FrameRate,
    pub duration: Tick,
    /// What its first frame's time is called (shown times start here; layer times start at 0).
    pub start: Tick,
    pub background: Rgba,
    /// The part that previews and renders by default.
    pub work_area: TimeRange,
    /// The frame that stands for it (a thumbnail, the picture a still render makes).
    pub poster: Tick,
    /// Top layer first.
    pub layers: Vec<Layer>,
    pub markers: Vec<Marker>,
    pub motion_blur: MotionBlur,
    /// Shy layers are left out of the layer list.
    pub hide_shy: bool,
    /// Layers with their motion blur switch on are blurred.
    pub enable_motion_blur: bool,
    pub enable_frame_blending: bool,
    pub draft_3d: bool,
    /// How light bounces in its 3D scenes.
    pub bounce: Bounce,
    /// What it opens to the layers that show it.
    pub essentials: crate::essential::Essentials,
}

impl Comp {
    /// Its protected stretches of time (its markers so marked: Responsive Design), joined and in
    /// order, in seconds within it.
    pub fn protected(&self) -> Vec<(f64, f64)> {
        let d = self.duration.seconds();
        let mut spans: Vec<(f64, f64)> = self
            .markers
            .iter()
            .filter(|m| m.protected && m.duration > Tick::ZERO)
            .map(|m| (m.time.seconds().clamp(0.0, d), (m.time + m.duration).seconds().clamp(0.0, d)))
            .filter(|(a, b)| b > a)
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut out: Vec<(f64, f64)> = Vec::new();
        for (a, b) in spans {
            match out.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => out.push((a, b)),
            }
        }
        out
    }

    /// The time of it a layer shows that is stretched by `stretch` (1: as it is), `elapsed`
    /// seconds after the layer's start: its protected stretches play at their own speed and the
    /// rest takes all of the stretch. None when it has none, or the rest cannot take the
    /// stretch (played backwards, or made shorter than the protected stretches are).
    pub fn responsive_time(&self, stretch: f64, elapsed: f64) -> Option<Tick> {
        let kept = self.protected();
        if kept.is_empty() || stretch <= 0.0 {
            return None;
        }
        let d = self.duration.seconds();
        let fixed: f64 = kept.iter().map(|(a, b)| b - a).sum();
        let free = d - fixed;
        // how much longer the free stretches play
        let k = if free > 1e-9 { (d * stretch - fixed) / free } else { 1.0 };
        if k <= 1e-9 {
            return None;
        }
        if elapsed < 0.0 {
            return Some(Tick::from_seconds(elapsed / k));
        }
        let mut pieces = Vec::new();
        let mut at = 0.0;
        for &(a, b) in &kept {
            if a > at {
                pieces.push((at, a, k));
            }
            pieces.push((a, b, 1.0));
            at = b;
        }
        if d > at {
            pieces.push((at, d, k));
        }
        let mut played = 0.0;
        for (a, b, slow) in pieces {
            let len = (b - a) * slow;
            if elapsed < played + len {
                return Some(Tick::from_seconds(a + (elapsed - played) / slow));
            }
            played += len;
        }
        Some(Tick::from_seconds(d + (elapsed - played) / k))
    }

    pub fn new(width: u32, height: u32, frame_rate: FrameRate, duration: Tick) -> Comp {
        Comp {
            width,
            height,
            pixel_aspect: 1.0,
            frame_rate,
            duration,
            start: Tick::ZERO,
            background: Rgba::BLACK,
            work_area: TimeRange::new(Tick::ZERO, duration),
            poster: Tick::ZERO,
            essentials: Default::default(),
            layers: Vec::new(),
            markers: Vec::new(),
            motion_blur: MotionBlur::default(),
            hide_shy: false,
            enable_motion_blur: true,
            enable_frame_blending: true,
            draft_3d: false,
            bounce: Bounce::default(),
        }
    }

    pub fn layer(&self, id: u64) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn layer_mut(&mut self, id: u64) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    /// How many frames it has (a last part shorter than a frame counts as one).
    pub fn frames(&self) -> i64 {
        let f = self.frame_rate.frame_at(self.duration);
        if self.frame_rate.tick_of(f) < self.duration { f + 1 } else { f }
    }

    /// Its fields, into the object of the item it is.
    pub(crate) fn write(&self, o: &mut Map) {
        o.insert("width", self.width as f64);
        o.insert("height", self.height as f64);
        if self.pixel_aspect != 1.0 {
            o.insert("pixelAspect", self.pixel_aspect);
        }
        o.insert("fps", fps_json(self.frame_rate));
        o.insert("duration", self.duration.seconds());
        if self.start != Tick::ZERO {
            o.insert("start", self.start.seconds());
        }
        if self.background != Rgba::BLACK {
            o.insert("background", Value::Color(self.background).to_json(Kind::Color));
        }
        if self.work_area != TimeRange::new(Tick::ZERO, self.duration) {
            o.insert("workArea", [self.work_area.start.seconds(), self.work_area.end().seconds()]);
        }
        if self.poster != Tick::ZERO {
            o.insert("posterTime", self.poster.seconds());
        }
        if self.motion_blur != MotionBlur::default() {
            let m = &self.motion_blur;
            o.insert("motionBlur", Json::object([("angle", m.angle), ("phase", m.phase), ("samples", m.samples as f64), ("adaptiveLimit", m.adaptive_limit as f64)]));
        }
        if self.bounce != Bounce::default() {
            o.insert("lightBounce", Json::object([("amount", self.bounce.amount), ("bounces", self.bounce.bounces as f64)]));
        }
        for (key, on, default) in [("hideShy", self.hide_shy, false), ("enableMotionBlur", self.enable_motion_blur, true), ("enableFrameBlending", self.enable_frame_blending, true), ("draft3d", self.draft_3d, false)] {
            if on != default {
                o.insert(key, on);
            }
        }
        if !self.markers.is_empty() {
            o.insert("markers", Json::array(self.markers.iter().map(Marker::to_json)));
        }
        if !self.essentials.is_empty() {
            o.insert("essentials", self.essentials.to_json());
        }
        o.insert("layers", Json::array(self.layers.iter().map(Layer::to_json)));
    }

    pub(crate) const FIELDS: [&'static str; 17] =
        ["width", "height", "pixelAspect", "fps", "duration", "start", "background", "workArea", "posterTime", "motionBlur", "lightBounce", "hideShy", "enableMotionBlur", "enableFrameBlending", "draft3d", "essentials", "layers"];

    /// A composition from the object of its item (whose other fields are the item's).
    pub(crate) fn read(o: &Obj, catalog: &Catalog) -> Result<Comp, LoadError> {
        let width = o.need_whole("width", 1, MAX_SIZE as u64)? as u32;
        let height = o.need_whole("height", 1, MAX_SIZE as u64)? as u32;
        let frame_rate = match o.get("fps") {
            None => return fail(o.at, "\"fps\" is missing (30, 29.97, \"30000/1001\"…)"),
            Some(j) => fps_from_json(j).ok_or_else(|| LoadError { at: o.at("fps"), message: format!("expected frames a second (30, 29.97, \"30000/1001\"), got {}", veymelo_keyframe::describe(j)) })?,
        };
        let Some(duration) = o.time("duration")? else { return fail(o.at, "\"duration\" is missing, its length in seconds") };
        if duration <= Tick::ZERO || duration.seconds() > MAX_SECONDS {
            return fail(&o.at("duration"), format!("a composition lasts more than nothing and at most {MAX_SECONDS} s"));
        }
        let mut c = Comp::new(width, height, frame_rate, duration);
        c.pixel_aspect = o.num_or("pixelAspect", 1.0)?;
        if !(0.01..=100.0).contains(&c.pixel_aspect) {
            return fail(&o.at("pixelAspect"), "a pixel's width for its height, from 0.01 to 100");
        }
        c.start = o.time_or("start", Tick::ZERO)?;
        if let Some(b) = o.get("background") {
            c.background = Value::from_json(Kind::Color, b).map_err(|e| LoadError { at: o.at("background"), message: e })?.as_color();
        }
        if let Some(w) = o.get("workArea") {
            let pair = w.as_array().filter(|a| a.len() == 2).and_then(|a| Some((a[0].as_f64()?, a[1].as_f64()?)));
            match pair {
                Some((a, b)) if 0.0 <= a && a < b && Tick::from_seconds(b) <= duration => c.work_area = TimeRange::from_bounds(Tick::from_seconds(a), Tick::from_seconds(b)),
                _ => return fail(&o.at("workArea"), "the work area is [start, end] in seconds, inside the composition"),
            }
        }
        c.poster = o.time_or("posterTime", Tick::ZERO)?;
        if c.poster < Tick::ZERO || c.poster >= duration {
            return fail(&o.at("posterTime"), "the poster time is inside the composition");
        }
        if let Some(m) = o.get("motionBlur") {
            let at = o.at("motionBlur");
            let m = Obj::new(m, &at, "motion blur")?;
            m.only(&["angle", "phase", "samples", "adaptiveLimit"])?;
            let d = MotionBlur::default();
            c.motion_blur = MotionBlur {
                angle: m.num_or("angle", d.angle)?,
                phase: m.num_or("phase", d.phase)?,
                samples: m.whole("samples", 2, 64)?.map_or(d.samples, |v| v as u32),
                adaptive_limit: m.whole("adaptiveLimit", 16, 256)?.map_or(d.adaptive_limit, |v| v as u32),
            };
            if !(0.0..=720.0).contains(&c.motion_blur.angle) || !(-360.0..=360.0).contains(&c.motion_blur.phase) {
                return fail(&at, "the shutter angle is 0 to 720 degrees and its phase -360 to 360");
            }
        }
        if let Some(b) = o.get("lightBounce") {
            let at = o.at("lightBounce");
            let b = Obj::new(b, &at, "light bounce")?;
            b.only(&["amount", "bounces"])?;
            let d = Bounce::default();
            c.bounce = Bounce { amount: b.num_or("amount", d.amount)?, bounces: b.whole("bounces", 1, MOST_BOUNCES as u64)?.map_or(d.bounces, |v| v as u32) };
            if !(0.0..=4.0).contains(&c.bounce.amount) {
                return fail(&at, "how much light bounces is 0 to 4 (1: all of it)");
            }
        }
        c.hide_shy = o.flag("hideShy", false)?;
        c.enable_motion_blur = o.flag("enableMotionBlur", true)?;
        c.enable_frame_blending = o.flag("enableFrameBlending", true)?;
        c.draft_3d = o.flag("draft3d", false)?;
        for (i, m) in o.list("markers")?.iter().enumerate() {
            c.markers.push(Marker::from_json(m, &format!("{}[{i}]", o.at("markers")))?);
        }
        c.markers.sort_by_key(|m| m.time);
        if let Some(e) = o.get("essentials") {
            c.essentials = crate::essential::Essentials::from_json(e, &o.at("essentials"))?;
        }
        for (i, l) in o.list("layers")?.iter().enumerate() {
            c.layers.push(Layer::from_json(l, duration, catalog, &format!("{}[{i}]", o.at("layers")))?);
        }
        Ok(c)
    }
}

/// A frame rate as a number when it is a whole one, else exactly as a fraction.
pub fn fps_json(r: FrameRate) -> Json {
    if r.den == 1 { Json::Number(r.num as f64) } else { Json::from(format!("{}/{}", r.num, r.den)) }
}

/// A frame rate from a number (29.97 is NTSC's 30000/1001) or a fraction `"30000/1001"`.
pub fn fps_from_json(j: &Json) -> Option<FrameRate> {
    match j {
        Json::Number(n) => FrameRate::from_f64(*n),
        Json::String(s) => {
            let (a, b) = s.split_once('/')?;
            FrameRate::new(a.trim().parse().ok()?, b.trim().parse().ok()?)
        }
        _ => None,
    }
}
