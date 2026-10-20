//! Paint: strokes laid on a layer by hand — painted with a brush, copied from somewhere with a
//! clone stamp, rubbed out with an eraser — each kept in the layer's Paint effect as settings
//! that can still be changed and animated: the path it went along, how much of it is drawn
//! (start and end), its brush tip, colour, opacity and flow, where it copies from, and a
//! transform of its own. A stroke shows from its own time, for a frame, a few, or to the end.
//!
//! ```text
//! Paint                     paintOnTransparent
//!   strokes/Brush 1         path, mode, from, length, pressure and what it does
//!     options               start, end, color, diameter, angle, hardness, roundness, spacing,
//!                           channels, opacity, flow
//!     transform             anchor, position, scale, rotation
//!   strokes/Clone 1         options also: cloneSource, clonePosition, cloneTimeShift, lockTime,
//!                           cloneTime
//!   strokes/Eraser 1        options also: eraseMode
//! ```
//!
//! A stroke is drawn as dabs of its tip along its path, `spacing` per cent of the tip (as wide as
//! it is there) apart: each dab an ellipse (the diameter across, roundness of that the other way,
//! turned by the angle) solid in its middle as far as its hardness says, fading to its edge. Flow
//! is how much each dab adds where it lands, opacity the most the whole stroke reaches. The pen's
//! pressure, one number for each point of the path, can shrink the tip and thin its flow and
//! opacity.

use veymelo_color::{BlendMode, blend};
use veymelo_geom::{Mat3, vec2};
use veymelo_keyframe::{Kind, ShapePath, Value};
use veymelo_project::{Body, DefNode, Group, GroupDef, PropDef, Unit};
use veymelo_raster::{Image, Px, Sampling, par};
use veymelo_time::Tick;

use crate::defs::{angle, choice, color, flag, layer, num, pct, point, px};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "paint";

/// How a stroke's colour mixes with what is under it: the modes a paint stroke can take.
pub const PAINT_MODES: &[&str] = &[
    "normal", "darken", "multiply", "colorBurn", "linearBurn", "add", "lighten", "screen", "colorDodge", "linearDodge", "overlay", "softLight", "hardLight", "linearLight", "vividLight",
    "pinLight", "difference", "exclusion", "hue", "saturation", "color", "luminosity",
];

/// What a brush or clone stroke paints into.
pub const CHANNELS: &[&str] = &["rgba", "rgb", "alpha"];

/// What an eraser rubs out.
pub const ERASE_MODES: &[&str] = &["layerAndPaint", "paintOnly", "lastStroke"];

const fn p(def: PropDef) -> DefNode {
    DefNode::Prop(def)
}

const PATH: PropDef =
    PropDef::new("path", "Path", Kind::Path, Value::Path(ShapePath { points: Vec::new(), in_tangents: Vec::new(), out_tangents: Vec::new(), closed: false, feather: veymelo_keyframe::Feathers::NONE }))
        .doc("the line it was drawn along, in the layer's pixels");
const MODE: PropDef = choice("mode", "Mode", PAINT_MODES, 0).setting().doc("how its colour mixes with what is under it");
const FROM: PropDef = num("from", "From", 0.0).unit(Unit::Seconds).setting().doc("when it starts to show, in the layer's own seconds");
const LENGTH: PropDef = num("length", "Length", 0.0).range(0.0, f64::INFINITY).unit(Unit::Seconds).setting().doc("how long it shows (0: to the end)");
const PRESSURE: PropDef = PropDef::new("pressure", "Pressure", Kind::Text, Value::Text(String::new())).setting().doc("the pen's pressure at each point of the path, 0 to 1, split by commas (none: full)");
const SIZE_BY_PRESSURE: PropDef = flag("sizeByPressure", "Size by Pressure", true).setting().doc("lighter pressure, a smaller tip");
const MIN_SIZE: PropDef = pct("minSize", "Minimum Size", 1.0, 0.0, 100.0).setting().doc("the tip's size at no pressure, in % of its diameter");
const OPACITY_BY_PRESSURE: PropDef = flag("opacityByPressure", "Opacity by Pressure", false).setting();
const FLOW_BY_PRESSURE: PropDef = flag("flowByPressure", "Flow by Pressure", false).setting();

const START: PropDef = pct("start", "Start", 0.0, 0.0, 100.0).doc("where along its path it begins to be drawn");
const END: PropDef = pct("end", "End", 100.0, 0.0, 100.0).doc("where along its path it stops (animated from 0, it writes itself on)");
const COLOR: PropDef = color("color", "Color", 1.0, 1.0, 1.0);
const DIAMETER: PropDef = px("diameter", "Diameter", 10.0, 2500.0).doc("how wide its tip is");
const ANGLE: PropDef = angle("angle", "Angle", 0.0).doc("how its tip is turned (when it is not round)");
const HARDNESS: PropDef = pct("hardness", "Hardness", 100.0, 0.0, 100.0).doc("how much of its tip is solid before it fades to the edge");
const ROUNDNESS: PropDef = pct("roundness", "Roundness", 100.0, 0.0, 100.0).doc("how wide its tip is the other way, in % of the diameter");
const SPACING: PropDef = pct("spacing", "Spacing", 25.0, 1.0, 1000.0).doc("how far apart its dabs are, in % of the diameter");
const CHANNEL: PropDef = choice("channels", "Channels", CHANNELS, 0).setting().doc("rgba: colour and opacity; rgb: colour only, where the layer is; alpha: its brightness becomes the layer's opacity");
const OPACITY: PropDef = pct("opacity", "Opacity", 100.0, 0.0, 100.0).doc("the most it covers");
const FLOW: PropDef = pct("flow", "Flow", 100.0, 0.0, 100.0).doc("how much each dab adds where it lands");
const CLONE_SOURCE: PropDef = layer("cloneSource", "Clone Source").setting().doc("the layer it copies from (none: this one)");
const CLONE_POSITION: PropDef = point("clonePosition", "Clone Position").doc("where in the source its first point copies from, in the source's pixels");
const CLONE_SHIFT: PropDef = num("cloneTimeShift", "Clone Time Shift", 0.0).unit(Unit::Seconds).doc("how much later (below 0: earlier) the source is copied from");
const LOCK_TIME: PropDef = flag("lockTime", "Lock Source Time", false).setting().doc("always copy the source at cloneTime");
const CLONE_TIME: PropDef = num("cloneTime", "Clone Time", 0.0).unit(Unit::Seconds).doc("with lockTime: the source's own time it copies, whatever the time");
const ERASE_MODE: PropDef = choice("eraseMode", "Erase", ERASE_MODES, 0).setting().doc("layerAndPaint: the layer and the paint under it; paintOnly: only the strokes before it; lastStroke: only the stroke just before it");

static BRUSH_OPTIONS: GroupDef = GroupDef {
    id: "options",
    name: "Stroke Options",
    body: Body::Fixed(&[p(START), p(END), p(COLOR), p(DIAMETER), p(ANGLE), p(HARDNESS), p(ROUNDNESS), p(SPACING), p(CHANNEL), p(OPACITY), p(FLOW)]),
    doc: "how it is drawn",
};

static CLONE_OPTIONS: GroupDef = GroupDef {
    id: "options",
    name: "Stroke Options",
    body: Body::Fixed(&[
        p(START),
        p(END),
        p(DIAMETER),
        p(ANGLE),
        p(HARDNESS),
        p(ROUNDNESS),
        p(SPACING),
        p(CHANNEL),
        p(OPACITY),
        p(FLOW),
        p(CLONE_SOURCE),
        p(CLONE_POSITION),
        p(CLONE_SHIFT),
        p(LOCK_TIME),
        p(CLONE_TIME),
    ]),
    doc: "how it is drawn and where it copies from",
};

static ERASER_OPTIONS: GroupDef = GroupDef {
    id: "options",
    name: "Stroke Options",
    body: Body::Fixed(&[p(START), p(END), p(DIAMETER), p(ANGLE), p(HARDNESS), p(ROUNDNESS), p(SPACING), p(OPACITY), p(FLOW), p(ERASE_MODE)]),
    doc: "how it is drawn and what it rubs out",
};

static TRANSFORM: GroupDef = GroupDef {
    id: "transform",
    name: "Transform",
    body: Body::Fixed(&[
        p(point("anchor", "Anchor Point").doc("the point it is scaled and turned about")),
        p(point("position", "Position").doc("where its anchor point goes")),
        p(PropDef::new("scale", "Scale", Kind::Vec2, Value::Vec2([100.0, 100.0])).unit(Unit::Percent)),
        p(angle("rotation", "Rotation", 0.0)),
    ]),
    doc: "the stroke moved, scaled and turned as a whole",
};

pub static BRUSH: GroupDef = GroupDef {
    id: "brushStroke",
    name: "Brush",
    body: Body::Fixed(&[
        p(PATH),
        DefNode::Group(&BRUSH_OPTIONS),
        DefNode::Group(&TRANSFORM),
        p(MODE),
        p(FROM),
        p(LENGTH),
        p(PRESSURE),
        p(SIZE_BY_PRESSURE),
        p(MIN_SIZE),
        p(OPACITY_BY_PRESSURE),
        p(FLOW_BY_PRESSURE),
    ]),
    doc: "colour painted along a path",
};

pub static CLONE: GroupDef = GroupDef {
    id: "cloneStroke",
    name: "Clone",
    body: Body::Fixed(&[
        p(PATH),
        DefNode::Group(&CLONE_OPTIONS),
        DefNode::Group(&TRANSFORM),
        p(MODE),
        p(FROM),
        p(LENGTH),
        p(PRESSURE),
        p(SIZE_BY_PRESSURE),
        p(MIN_SIZE),
        p(OPACITY_BY_PRESSURE),
        p(FLOW_BY_PRESSURE),
    ]),
    doc: "pixels copied along a path from elsewhere on this layer or another, now or at another time",
};

pub static ERASER: GroupDef = GroupDef {
    id: "eraserStroke",
    name: "Eraser",
    body: Body::Fixed(&[p(PATH), DefNode::Group(&ERASER_OPTIONS), DefNode::Group(&TRANSFORM), p(FROM), p(LENGTH), p(PRESSURE), p(SIZE_BY_PRESSURE), p(MIN_SIZE), p(OPACITY_BY_PRESSURE), p(FLOW_BY_PRESSURE)]),
    doc: "the layer, or paint, rubbed out along a path",
};

static STROKES: GroupDef = GroupDef { id: "strokes", name: "Strokes", body: Body::List("paintStrokes"), doc: "the strokes, painted one after another" };

pub static PAINT_DEF: GroupDef = GroupDef {
    id: "paint",
    name: "Paint",
    body: Body::Fixed(&[p(flag("paintOnTransparent", "Paint on Transparent", false).doc("the strokes alone, on nothing: the layer itself not shown")), DefNode::Group(&STROKES)]),
    doc: "strokes painted, cloned and erased on the layer by hand, each still there to change and animate",
};

pub const PAINT: Spec = Spec { lists: &[("paintStrokes", &BRUSH), ("paintStrokes", &CLONE), ("paintStrokes", &ERASER)], ..Spec::new(&PAINT_DEF, CATEGORY, run) };

pub const ALL: [Spec; 1] = [PAINT];

/// A brush tip to start a stroke with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tip {
    pub name: &'static str,
    pub diameter: f64,
    pub angle: f64,
    pub roundness: f64,
    pub hardness: f64,
    pub spacing: f64,
}

const fn round(name: &'static str, diameter: f64, hardness: f64) -> Tip {
    Tip { name, diameter, angle: 0.0, roundness: 100.0, hardness, spacing: 25.0 }
}

/// The brush tips there are to start from: hard and soft round ones of many sizes, and flat
/// ones for calligraphy.
pub const TIPS: &[Tip] = &[
    round("hard 1", 1.0, 100.0),
    round("hard 3", 3.0, 100.0),
    round("hard 5", 5.0, 100.0),
    round("hard 10", 10.0, 100.0),
    round("hard 20", 20.0, 100.0),
    round("hard 40", 40.0, 100.0),
    round("soft 5", 5.0, 0.0),
    round("soft 10", 10.0, 0.0),
    round("soft 20", 20.0, 0.0),
    round("soft 40", 40.0, 0.0),
    round("soft 80", 80.0, 0.0),
    round("soft 150", 150.0, 0.0),
    round("soft 300", 300.0, 0.0),
    round("medium 25", 25.0, 50.0),
    Tip { name: "flat 20", diameter: 20.0, angle: 45.0, roundness: 25.0, hardness: 90.0, spacing: 10.0 },
    Tip { name: "flat 40", diameter: 40.0, angle: -30.0, roundness: 15.0, hardness: 80.0, spacing: 8.0 },
    Tip { name: "chisel 12", diameter: 12.0, angle: 90.0, roundness: 35.0, hardness: 100.0, spacing: 12.0 },
];

/// What a stroke does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokeKind {
    Brush,
    Clone,
    Eraser,
}

impl StrokeKind {
    pub const IDS: [&'static str; 3] = ["brush", "clone", "eraser"];

    pub fn def(self) -> &'static GroupDef {
        match self {
            StrokeKind::Brush => &BRUSH,
            StrokeKind::Clone => &CLONE,
            StrokeKind::Eraser => &ERASER,
        }
    }

    pub fn of(def: &GroupDef) -> Option<StrokeKind> {
        match def.id {
            "brushStroke" => Some(StrokeKind::Brush),
            "cloneStroke" => Some(StrokeKind::Clone),
            "eraserStroke" => Some(StrokeKind::Eraser),
            _ => None,
        }
    }
}

/// A stroke, read at the frame's time.
#[derive(Clone, Debug)]
pub struct Stroke {
    pub kind: StrokeKind,
    pub path: ShapePath,
    pub pressure: Vec<f64>,
    pub mode: BlendMode,
    pub start: f64,
    pub end: f64,
    /// Straight colour.
    pub color: [f32; 4],
    pub diameter: f64,
    pub angle: f64,
    pub hardness: f64,
    pub roundness: f64,
    pub spacing: f64,
    pub channels: u32,
    pub opacity: f64,
    pub flow: f64,
    pub size_by_pressure: bool,
    pub min_size: f64,
    pub opacity_by_pressure: bool,
    pub flow_by_pressure: bool,
    pub erase: u32,
    pub clone_source: Option<u64>,
    pub clone_position: [f64; 2],
    pub clone_shift: f64,
    pub lock_time: bool,
    pub clone_time: f64,
    /// From the path's points to the layer's.
    pub transform: Mat3,
}

/// Whether a stroke shows at layer time `t`.
fn shows(g: &Group, ctx: &Ctx) -> bool {
    let n = |path: &str, d: f64| g.prop(path).map_or(d, |p| p.value.as_f64());
    let (from, length, t) = (n("from", 0.0), n("length", 0.0), ctx.time.seconds());
    g.enabled && t >= from - 1e-6 && (length <= 0.0 || t < from + length - 1e-6)
}

/// A stroke, read now.
pub fn stroke_at(ctx: &Ctx, g: &Group) -> Option<Stroke> {
    let kind = StrokeKind::of(g.def)?;
    let path = ctx.value_in(g, "path").and_then(|v| v.as_path().cloned()).unwrap_or_default();
    let n = |path: &str, d: f64| ctx.num_in(g, path, d);
    let b = |path: &str, d: bool| ctx.value_in(g, path).map_or(d, |v| v.as_bool());
    let c = |path: &str| ctx.value_in(g, path).map_or(0, |v| v.as_choice());
    let col = ctx.value_in(g, "options/color").map_or(veymelo_color::Rgba::WHITE, |v| v.as_color());
    let pressure = ctx.value_in(g, "pressure").map(|v| v.as_text().split(',').filter_map(|x| x.trim().parse::<f64>().ok()).map(|x| x.clamp(0.0, 1.0)).collect()).unwrap_or_default();
    let mode = PAINT_MODES.get(c("mode") as usize).and_then(|m| BlendMode::from_id(m)).unwrap_or_default();
    let scale = ctx.v2_in(g, "transform/scale", [100.0, 100.0]);
    let (anchor, position) = (ctx.v2_in(g, "transform/anchor", [0.0; 2]), ctx.v2_in(g, "transform/position", [0.0; 2]));
    let transform = Mat3::translate(vec2(position[0], position[1]))
        * Mat3::rotate_deg(n("transform/rotation", 0.0))
        * Mat3::scale(vec2(scale[0] / 100.0, scale[1] / 100.0))
        * Mat3::translate(vec2(-anchor[0], -anchor[1]));
    Some(Stroke {
        kind,
        path,
        pressure,
        mode,
        start: n("options/start", 0.0),
        end: n("options/end", 100.0),
        color: [col.r as f32, col.g as f32, col.b as f32, col.a as f32],
        diameter: n("options/diameter", 10.0),
        angle: n("options/angle", 0.0),
        hardness: n("options/hardness", 100.0),
        roundness: n("options/roundness", 100.0),
        spacing: n("options/spacing", 25.0),
        channels: c("options/channels"),
        opacity: n("options/opacity", 100.0),
        flow: n("options/flow", 100.0),
        size_by_pressure: b("sizeByPressure", true),
        min_size: n("minSize", 1.0),
        opacity_by_pressure: b("opacityByPressure", false),
        flow_by_pressure: b("flowByPressure", false),
        erase: c("options/eraseMode"),
        clone_source: ctx.value_in(g, "options/cloneSource").and_then(|v| v.as_layer()),
        clone_position: ctx.v2_in(g, "options/clonePosition", path_start(g)),
        clone_shift: n("options/cloneTimeShift", 0.0),
        lock_time: b("options/lockTime", false),
        clone_time: n("options/cloneTime", 0.0),
        transform,
    })
}

/// A path's first point as stored.
fn path_start(g: &Group) -> [f64; 2] {
    g.prop("path").ok().and_then(|p| p.value.as_path().and_then(|s| s.points.first().copied())).unwrap_or([0.0; 2])
}

/// The strokes of a Paint effect that show now, in the order they are painted.
pub fn read(ctx: &Ctx) -> Vec<Stroke> {
    let Ok(list) = ctx.effect.group("strokes") else { return Vec::new() };
    list.children.iter().filter_map(|n| n.as_group()).filter(|g| shows(g, ctx)).filter_map(|g| stroke_at(ctx, g)).collect()
}

impl Stroke {
    /// The path as a line of points in the layer's units (its curves cut fine), each with the
    /// pen's pressure there.
    pub fn line(&self) -> Vec<([f64; 2], f64)> {
        let p = &self.path;
        let n = p.points.len();
        if n == 0 {
            return Vec::new();
        }
        let pressure = |i: usize| if self.pressure.len() == n { self.pressure[i] } else { 1.0 };
        let at = |q: [f64; 2]| {
            let v = self.transform.apply(vec2(q[0], q[1]));
            [v.x, v.y]
        };
        let mut out = vec![(at(p.points[0]), pressure(0))];
        let pieces = if p.closed { n } else { n - 1 };
        for i in 0..pieces {
            let j = (i + 1) % n;
            let (a, b) = (p.points[i], p.points[j]);
            let o = p.out_tangents.get(i).copied().unwrap_or([0.0; 2]);
            let t = p.in_tangents.get(j).copied().unwrap_or([0.0; 2]);
            let (c1, c2) = ([a[0] + o[0], a[1] + o[1]], [b[0] + t[0], b[1] + t[1]]);
            // about a point every two units along the curve's hull
            let hull = |u: [f64; 2], v: [f64; 2]| ((v[0] - u[0]).powi(2) + (v[1] - u[1]).powi(2)).sqrt();
            let steps = if o == [0.0; 2] && t == [0.0; 2] { 1 } else { ((hull(a, c1) + hull(c1, c2) + hull(c2, b)) / 2.0).ceil().clamp(2.0, 128.0) as usize };
            for k in 1..=steps {
                let u = k as f64 / steps as f64;
                let v = 1.0 - u;
                let q = [
                    v * v * v * a[0] + 3.0 * v * v * u * c1[0] + 3.0 * v * u * u * c2[0] + u * u * u * b[0],
                    v * v * v * a[1] + 3.0 * v * v * u * c1[1] + 3.0 * v * u * u * c2[1] + u * u * u * b[1],
                ];
                out.push((at(q), pressure(i) + (pressure(j) - pressure(i)) * u));
            }
        }
        out
    }
}

/// One print of the tip, in the buffer's pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dab {
    pub x: f64,
    pub y: f64,
    /// Half the diameter.
    pub r: f64,
    /// Roundness, 0 to 1.
    pub ratio: f64,
    pub cos: f64,
    pub sin: f64,
    /// 0 to 1.
    pub hardness: f64,
    /// How much it adds: flow, 0 to 1.
    pub weight: f64,
}

impl Dab {
    /// How much of the pixel centred at (`x`, `y`) it covers.
    #[inline]
    pub fn cover(&self, x: f64, y: f64) -> f64 {
        let (dx, dy) = (x - self.x, y - self.y);
        let (r, minor) = (self.r.max(1e-6), (self.r * self.ratio).max(1e-6));
        let (u, v) = ((dx * self.cos + dy * self.sin) / r, (-dx * self.sin + dy * self.cos) / minor);
        let d = (u * u + v * v).sqrt();
        // the edge smoothed over a pixel, measured across the narrow way
        let edge = (0.5 - (d - 1.0) * minor.max(0.5)).clamp(0.0, 1.0);
        let h = self.hardness.clamp(0.0, 1.0);
        if h >= 0.999 || d <= h {
            return edge;
        }
        let f = ((d - h) / (1.0 - h)).clamp(0.0, 1.0);
        (1.0 - f * f * (3.0 - 2.0 * f)) * edge
    }
}

/// The dabs of a stroke from its start to its end, in the pixels of `buf`.
pub fn dabs(s: &Stroke, buf: &Buf) -> Vec<Dab> {
    let line = s.line();
    if line.is_empty() {
        return Vec::new();
    }
    // the tip grows with the stroke's own scale
    let m = s.transform.0;
    let grow = ((m[0][0].powi(2) + m[1][0].powi(2)).sqrt() + (m[0][1].powi(2) + m[1][1].powi(2)).sqrt()) / 2.0;
    let d = s.diameter.max(0.0) * grow;
    if d <= 0.0 {
        return Vec::new();
    }
    let mut along = vec![0.0];
    for w in line.windows(2) {
        let (a, b) = (w[0].0, w[1].0);
        along.push(along.last().copied().unwrap_or(0.0) + ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt());
    }
    let total = along.last().copied().unwrap_or(0.0);
    let (a, b) = ((s.start / 100.0).clamp(0.0, 1.0), (s.end / 100.0).clamp(0.0, 1.0));
    if b <= a {
        return Vec::new();
    }
    let at = |l: f64| -> ([f64; 2], f64) {
        if line.len() == 1 {
            return line[0];
        }
        let i = along.partition_point(|c| *c <= l).clamp(1, line.len() - 1);
        let (c0, c1) = (along[i - 1], along[i]);
        let u = if c1 > c0 { ((l - c0) / (c1 - c0)).clamp(0.0, 1.0) } else { 0.0 };
        let ((p0, q0), (p1, q1)) = (line[i - 1], line[i]);
        ([p0[0] + (p1[0] - p0[0]) * u, p0[1] + (p1[1] - p0[1]) * u], q0 + (q1 - q0) * u)
    };
    let (sin, cos) = s.angle.to_radians().sin_cos();
    let mut out = Vec::new();
    let mut l = a * total;
    loop {
        let (q, pressure) = at(l);
        let pressure = pressure.clamp(0.0, 1.0);
        let size = if s.size_by_pressure { (s.min_size / 100.0 + (1.0 - s.min_size / 100.0) * pressure).clamp(0.0, 1.0) } else { 1.0 };
        let mut weight = (s.flow / 100.0).clamp(0.0, 1.0);
        if s.flow_by_pressure {
            weight *= pressure;
        }
        if s.opacity_by_pressure {
            weight *= pressure;
        }
        let at = buf.pixel(vec2(q[0], q[1]));
        out.push(Dab { x: at.x, y: at.y, r: d / 2.0 * size * buf.scale, ratio: (s.roundness / 100.0).clamp(0.01, 1.0), cos, sin, hardness: s.hardness / 100.0, weight });
        // the next dab as far on as the tip is wide here: nearer where pressure shrinks it
        l += (d * size * s.spacing / 100.0).max(0.1);
        if total <= 0.0 || l > b * total + 1e-9 {
            break;
        }
    }
    out
}

/// How much a stroke covers each pixel of a box of the buffer.
#[derive(Clone, Debug, Default)]
pub struct Cover {
    pub x0: usize,
    pub y0: usize,
    pub w: usize,
    pub h: usize,
    pub k: Vec<f32>,
}

impl Cover {
    #[inline]
    pub fn at(&self, x: usize, y: usize) -> f32 {
        if x < self.x0 || y < self.y0 || x >= self.x0 + self.w || y >= self.y0 + self.h { 0.0 } else { self.k[(y - self.y0) * self.w + x - self.x0] }
    }

    /// Less of it, by how much `other` covers.
    fn rub(&mut self, other: &Cover) {
        let (x0, y0, w) = (self.x0, self.y0, self.w);
        par::rows(&mut self.k, w.max(1), |j, row| {
            for (i, k) in row.iter_mut().enumerate() {
                *k *= 1.0 - other.at(x0 + i, y0 + j);
            }
        });
    }
}

/// What a stroke covers in a `width` × `height` buffer: each dab adds its flow where it lands
/// (so overlapping dabs build up), and the stroke reaches at most its opacity.
pub fn cover(s: &Stroke, dabs: &[Dab], width: usize, height: usize) -> Cover {
    let reach = |d: &Dab| d.r + 1.0;
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for d in dabs {
        (x0, y0, x1, y1) = (x0.min(d.x - reach(d)), y0.min(d.y - reach(d)), x1.max(d.x + reach(d)), y1.max(d.y + reach(d)));
    }
    if dabs.is_empty() || x1 <= 0.0 || y1 <= 0.0 || x0 >= width as f64 || y0 >= height as f64 {
        return Cover::default();
    }
    let (x0, y0) = (x0.floor().max(0.0) as usize, y0.floor().max(0.0) as usize);
    let (x1, y1) = ((x1.ceil() as usize).min(width), (y1.ceil() as usize).min(height));
    let (w, h) = (x1 - x0, y1 - y0);
    let opacity = (s.opacity / 100.0).clamp(0.0, 1.0) as f32;
    let mut k = vec![0.0f32; w * h];
    par::rows(&mut k, w, |j, row| {
        let y = (y0 + j) as f64 + 0.5;
        let mut left = vec![1.0f64; w];
        for d in dabs.iter().filter(|d| (y - d.y).abs() <= reach(d)) {
            let a = ((d.x - reach(d)).floor().max(x0 as f64) as usize).max(x0);
            let b = ((d.x + reach(d)).ceil().max(0.0) as usize).min(x1);
            for x in a..b {
                let c = d.cover(x as f64 + 0.5, y);
                if c > 0.0 {
                    left[x - x0] *= 1.0 - d.weight * c;
                }
            }
        }
        for (o, l) in row.iter_mut().zip(left) {
            *o = opacity * (1.0 - l as f32);
        }
    });
    Cover { x0, y0, w, h, k }
}

/// Paint straight colour `col` of opacity `a`, `k` of the way, onto premultiplied `dst` with the
/// stroke's mode, into its channels.
#[inline]
fn paint(dst: &mut Px, col: [f32; 3], a: f32, k: f32, mode: BlendMode, channels: u32) {
    let k = (k * a).clamp(0.0, 1.0);
    if k <= 0.0 {
        return;
    }
    match channels {
        // the colour alone, where the layer is
        1 => {
            let da = dst[3];
            if da <= 0.0 {
                return;
            }
            let was = [dst[0] / da, dst[1] / da, dst[2] / da];
            let mixed = blend(mode, [was[0], was[1], was[2], 1.0], [col[0], col[1], col[2], 1.0], 0.0);
            for i in 0..3 {
                dst[i] = (was[i] + (mixed[i] - was[i]) * k) * da;
            }
        }
        // its brightness becomes the layer's opacity
        2 => {
            let da = dst[3];
            let lum = veymelo_color::luminance(col[0] as f64, col[1] as f64, col[2] as f64) as f32;
            let na = da + (lum - da) * k;
            let f = if da > 0.0 { na / da } else { 0.0 };
            for c in dst.iter_mut().take(3) {
                *c *= f;
            }
            dst[3] = na;
        }
        _ => *dst = blend(mode, *dst, [col[0] * k, col[1] * k, col[2] * k, k], 0.0),
    }
}

/// What a clone stroke copies from: `own`, this layer as it is painted so far (painting on
/// transparency, or at another time: as it comes to Paint), or another layer, with its own
/// pixels-to-units.
fn source(ctx: &Ctx, s: &Stroke, own: &Buf) -> Option<Buf> {
    let shift = if s.lock_time { s.clone_time - ctx.time.seconds() } else { s.clone_shift };
    let now = shift.abs() < 1e-9;
    match (s.clone_source, ctx.host) {
        (None, _) if now => Some(own.clone()),
        (None, Some(h)) => h.this_at(ctx.time + Tick::from_seconds(shift), ctx.index),
        (Some(id), Some(h)) if now => h.layer(id, Take::All).map(|(b, _)| b),
        (Some(id), Some(h)) => h.layer_at(id, Tick::from_seconds(shift), Take::All).map(|(b, _)| b),
        _ => None,
    }
}

fn run(ctx: &Ctx, mut buf: Buf) -> Buf {
    let strokes = read(ctx);
    let transparent = ctx.flag("paintOnTransparent");
    if strokes.is_empty() && !transparent {
        return buf;
    }
    // room to paint over the whole layer, not only where it shows
    buf.cover(ctx.bounds);
    let (w, h) = (buf.img.width, buf.img.height);
    let mut covers: Vec<Cover> = strokes.iter().map(|s| cover(s, &dabs(s, &buf), w, h)).collect();
    // erasers of paint alone take from the strokes before them (or the one just before)
    for j in 0..strokes.len() {
        let s = &strokes[j];
        if s.kind != StrokeKind::Eraser || s.erase == 0 {
            continue;
        }
        let rubber = covers[j].clone();
        let mut before = (0..j).filter(|i| strokes[*i].kind != StrokeKind::Eraser);
        let hit: Vec<usize> = if s.erase == 1 { before.collect() } else { before.next_back().into_iter().collect() };
        for i in hit {
            covers[i].rub(&rubber);
        }
    }
    let mut out = if transparent { Image::new(w, h) } else { buf.img.clone() };
    for (s, c) in strokes.iter().zip(&covers) {
        if c.w == 0 || c.h == 0 {
            continue;
        }
        match s.kind {
            StrokeKind::Eraser if s.erase == 0 => par::rows(&mut out.px, w, |y, row| {
                if y < c.y0 || y >= c.y0 + c.h {
                    return;
                }
                for (x, p) in row.iter_mut().enumerate().skip(c.x0).take(c.w) {
                    let k = 1.0 - c.at(x, y);
                    *p = p.map(|v| v * k);
                }
            }),
            StrokeKind::Eraser => {}
            StrokeKind::Brush => {
                let col = [s.color[0], s.color[1], s.color[2]];
                par::rows(&mut out.px, w, |y, row| {
                    if y < c.y0 || y >= c.y0 + c.h {
                        return;
                    }
                    for (x, p) in row.iter_mut().enumerate().skip(c.x0).take(c.w) {
                        paint(p, col, s.color[3], c.at(x, y), s.mode, s.channels);
                    }
                });
            }
            StrokeKind::Clone => {
                // on nothing, the layer as it came is what there is to copy
                let so_far = if transparent { buf.clone() } else { Buf { img: out.clone(), ..buf.clone() } };
                let Some(from) = source(ctx, s, &so_far) else {
                    ctx.note("a clone stroke's source cannot be drawn then, so it copies nothing".into());
                    continue;
                };
                // a point of this layer copies the source's point this far from it
                let first = s.line().first().map_or([0.0; 2], |p| p.0);
                let off = vec2(s.clone_position[0] - first[0], s.clone_position[1] - first[1]);
                let to_layer = buf.to_layer();
                par::rows(&mut out.px, w, |y, row| {
                    if y < c.y0 || y >= c.y0 + c.h {
                        return;
                    }
                    for (x, p) in row.iter_mut().enumerate().skip(c.x0).take(c.w) {
                        let k = c.at(x, y);
                        if k <= 0.0 {
                            continue;
                        }
                        let q = from.pixel(to_layer.apply(vec2(x as f64 + 0.5, y as f64 + 0.5)) + off);
                        let sp = from.img.sample(Sampling::Bilinear, q.x, q.y);
                        if sp[3] > 0.0 {
                            paint(p, [sp[0] / sp[3], sp[1] / sp[3], sp[2] / sp[3]], sp[3], k, s.mode, s.channels);
                        }
                    }
                });
            }
        }
    }
    Buf { img: out, ..buf }
}

#[cfg(test)]
mod tests;
