//! Older effects kept so old work opens the same: the layer turned in a simple perspective
//! (Basic 3D), the old Gaussian blur, text drawn on the layer (Basic Text) or along a path (Path
//! Text), and the old lightning.
//!
//! The text is set in real fonts by our own text engine, as the Text effects are.

use std::f64::consts::{PI, TAU};

use veymelo_geom::vec2;
use veymelo_keyframe::{Kind, Value};
use veymelo_path::Path;
use veymelo_project::PropDef;
use veymelo_raster::filter::{self, Edge};
use veymelo_raster::{Image, Px, Sampling};

use crate::defs::{angle, choice, color, effect, flag, point, ranged};
use crate::words::{Paint, font_for, glyph_in, paint, set_line};
use crate::{Buf, Ctx, Spec};

const CATEGORY: &str = "obsolete";

const fn text(id: &'static str, name: &'static str, v: &'static str) -> PropDef {
    PropDef::new(id, name, Kind::Text, Value::Text(String::new())).doc(v)
}

/// The frame a layer time falls in (for things that change every frame).
fn frame_of(ctx: &Ctx) -> i64 {
    (ctx.time.seconds() / ctx.frame.seconds().max(1e-9) + 1e-6).floor() as i64
}

// ---------------------------------------------------------------- Gaussian Blur (Legacy)

effect!(GAUSSIAN_DEF, "gaussianBlurLegacy", "Gaussian Blur (Legacy)", "the old Gaussian blur: spreading past the layer's edges, never repeating them", [
    ranged("blurriness", "Blurriness", 0.0, 0.0, 1000.0),
    choice("blurDimensions", "Blur Dimensions", &["horizontalAndVertical", "horizontal", "vertical"], 0),
]);
pub const GAUSSIAN: Spec = Spec::new(&GAUSSIAN_DEF, CATEGORY, gaussian);

fn gaussian(ctx: &Ctx, mut b: Buf) -> Buf {
    let s = ctx.num("blurriness", 0.0).max(0.0) * 0.5 * b.scale;
    if s < 0.01 {
        return b;
    }
    let (kx, ky) = match ctx.choice("blurDimensions") {
        1 => (1.0, 0.0),
        2 => (0.0, 1.0),
        _ => (1.0, 1.0),
    };
    let edge = if ctx.adjustment {
        Edge::Repeat
    } else {
        b.pad((s * 4.0).ceil() as usize + 1);
        Edge::Clear
    };
    b.img = filter::gaussian(&b.img, s * kx, s * ky, edge);
    b
}

// ---------------------------------------------------------------- Basic 3D

effect!(BASIC_3D_DEF, "basic3d", "Basic 3D", "the layer swivelled and tilted in a simple perspective, moved nearer or farther, with a glint of light on it", [
    angle("swivel", "Swivel", 0.0).doc("about the upright"),
    angle("tilt", "Tilt", 0.0).doc("about the level"),
    ranged("distanceToImage", "Distance to Image", 0.0, -1000.0, 30_000.0).doc("farther (above 0) or nearer"),
    flag("specularHighlight", "Specular Highlight", false),
    flag("drawPreviewWireframe", "Draw Preview Wireframe", false),
]);
pub const BASIC_3D: Spec = Spec::new(&BASIC_3D_DEF, CATEGORY, basic_3d);

fn basic_3d(ctx: &Ctx, mut b: Buf) -> Buf {
    let (swivel, tilt) = (ctx.num("swivel", 0.0).to_radians(), ctx.num("tilt", 0.0).to_radians());
    let distance = ctx.num("distanceToImage", 0.0) * b.scale;
    let (shine, wire) = (ctx.flag("specularHighlight"), ctx.flag("drawPreviewWireframe"));
    if swivel == 0.0 && tilt == 0.0 && distance == 0.0 && !shine && !wire {
        return b;
    }
    let r = ctx.bounds;
    let mid = b.pixel(r.center());
    let (half_w, half_h) = (r.width() * b.scale * 0.5, r.height() * b.scale * 0.5);
    // the eye in front of the middle, as far as a standard lens would be for the layer's width
    let eye = (half_w.max(half_h) * 2.0 * 1.39).max(1.0);
    // the plane turned: its across, its down and its face
    let (cs, ss) = (swivel.cos(), swivel.sin());
    let (ct, st) = (tilt.cos(), tilt.sin());
    let across = [cs, 0.0, -ss];
    let down = [ss * st, ct, cs * st];
    let face = [ss * ct, -st, cs * ct];
    let dot = |a: [f64; 3], c: [f64; 3]| a[0] * c[0] + a[1] * c[1] + a[2] * c[2];
    // toward the light: above, to the left and in front
    let light = {
        let l = [-0.5, -0.6, -1.0];
        let k = 1.0 / dot(l, l).sqrt();
        l.map(|v: f64| v * k)
    };
    // the point on the plane a ray through screen point (x, y) meets
    let hit = |x: f64, y: f64| -> Option<(f64, f64, [f64; 3])> {
        let d = [x - mid.x, y - mid.y, eye];
        let across_d = dot(d, face);
        if across_d.abs() < 1e-12 {
            return None;
        }
        let t = (eye + distance) * face[2] / across_d;
        if t <= 0.0 {
            return None;
        }
        let p = [d[0] * t, d[1] * t, d[2] * t - eye - distance];
        Some((dot(p, across), dot(p, down), d))
    };
    let src = b.img.clone();
    let (w, h) = (b.img.width, b.img.height);
    let mut out = Image::new(w, h);
    veymelo_raster::par::rows(&mut out.px, w, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let Some((u, v, d)) = hit(px, py) else { continue };
            // where the plane is turned far from the eye, a pixel spans many of the layer's: as
            // many samples
            let span = [hit(px + 1.0, py), hit(px, py + 1.0)].iter().flatten().map(|(a, c, _)| (a - u).hypot(c - v)).fold(1.0, f64::max);
            let n = (span.ceil() as usize).clamp(1, 4);
            let mut c = [0f32; 4];
            for j in 0..n {
                for i in 0..n {
                    let (sx, sy) = (px - 0.5 + (i as f64 + 0.5) / n as f64, py - 0.5 + (j as f64 + 0.5) / n as f64);
                    if let Some((su, sv, _)) = hit(sx, sy) {
                        let s = src.sample(Sampling::Bilinear, mid.x + su, mid.y + sv);
                        (0..4).for_each(|k| c[k] += s[k] / (n * n) as f32);
                    }
                }
            }
            if shine && c[3] > 0.0 {
                // the light seen in the plane as in a mirror
                let l = dot(d, d).sqrt();
                let view = d.map(|v| v / l);
                let k = 2.0 * dot(view, face);
                let mirrored = [view[0] - k * face[0], view[1] - k * face[1], view[2] - k * face[2]];
                let s = dot(mirrored, light).max(0.0).powi(160) as f32;
                let a = c[3];
                for ch in c.iter_mut().take(3) {
                    *ch += s * (a - *ch).max(0.0);
                }
            }
            if wire {
                let edge = (u.abs() - half_w).abs().min((v.abs() - half_h).abs());
                if edge < 0.75 * span && u.abs() <= half_w + 0.75 * span && v.abs() <= half_h + 0.75 * span {
                    c = [1.0; 4];
                }
            }
            *o = c;
        }
    });
    b.img = out;
    b
}

// ---------------------------------------------------------------- Basic Text

const DISPLAY: &[&str] = &["fillOnly", "strokeOnly", "fillOverStroke", "strokeOverFill"];

/// How the text is painted, from the settings.
fn paint_of(ctx: &Ctx) -> Paint {
    Paint { display: ctx.choice("displayOptions"), fill: ctx.color("fillColor"), stroke: ctx.color("strokeColor"), width: ctx.num("strokeWidth", 2.0).max(0.0), opacity: 1.0 }
}

/// The text drawn over the layer, or alone.
fn finish(ctx: &Ctx, mut b: Buf, draw: impl Fn(&Buf, &mut Image)) -> Buf {
    let mut img = if ctx.flag("compositeOnOriginal") { b.img.clone() } else { Image::new(b.img.width, b.img.height) };
    draw(&b, &mut img);
    b.img = img;
    b
}

effect!(BASIC_TEXT_DEF, "basicText", "Basic Text", "lines of text drawn on the layer, filled, outlined or both", [
    text("text", "Text", "lines apart by line breaks"),
    text("font", "Font", "an installed font's name or a font file; empty: the system's own"),
    choice("alignment", "Alignment", &["left", "center", "right"], 1),
    point("position", "Position"),
    choice("displayOptions", "Display Options", DISPLAY, 0),
    color("fillColor", "Fill Color", 1.0, 0.0, 0.0),
    color("strokeColor", "Stroke Color", 1.0, 1.0, 1.0),
    ranged("strokeWidth", "Stroke Width", 2.0, 0.0, 50.0),
    ranged("size", "Size", 36.0, 0.0, 512.0),
    ranged("tracking", "Tracking", 0.0, -100.0, 100.0),
    ranged("lineSpacing", "Line Spacing", 100.0, 0.0, 1000.0),
    flag("compositeOnOriginal", "Composite On Original", true),
]);
pub const BASIC_TEXT: Spec = Spec::new(&BASIC_TEXT_DEF, CATEGORY, basic_text).points(&[("position", [0.5, 0.5])]);

fn value_text(ctx: &Ctx, id: &str) -> String {
    ctx.value_in(ctx.effect, id).map(|v| v.as_text().to_string()).unwrap_or_default()
}

fn basic_text(ctx: &Ctx, b: Buf) -> Buf {
    let words = value_text(ctx, "text");
    let Some(font) = font_for(&value_text(ctx, "font"), 400.0) else {
        ctx.note("no font could be found: nothing drawn".into());
        return b;
    };
    let size = ctx.num("size", 36.0).max(0.5);
    let (tracking, lead) = (ctx.num("tracking", 0.0), size * ctx.num("lineSpacing", 100.0) / 100.0 * 1.2);
    let at = ctx.point("position");
    let align = ctx.choice("alignment");
    let lines: Vec<&str> = words.split(['\n', '\r']).collect();
    let mut outline = Path::default();
    for (i, line) in lines.iter().enumerate() {
        let y = at.y + (i as f64 - (lines.len() as f64 - 1.0) * 0.5) * lead;
        let (p, _) = set_line(line, &font, size, tracking, false, align, vec2(at.x, y));
        for c in p.contours() {
            for (k, q) in c.iter().enumerate() {
                if k == 0 { outline.move_to(q.0, q.1) } else { outline.line_to(q.0, q.1) }
            }
            outline.close();
        }
    }
    let look = paint_of(ctx);
    finish(ctx, b, |b, img| paint(b, img, &outline, &look))
}

// ---------------------------------------------------------------- Path Text

effect!(PATH_TEXT_DEF, "pathText", "Path Text", "text set along a curve, a circle, a line or one of the layer's masks: each letter turned with it, spaced, sheared, scaled, faded in and jittered", [
    text("text", "Text", "lines apart by line breaks (stacked across the path)"),
    text("font", "Font", "an installed font's name or a font file; empty: the system's own"),
    choice("shapeType", "Shape Type", &["bezier", "circle", "loop", "line"], 0),
    point("tangent1", "Tangent 1/Circle Point"),
    point("vertex1", "Vertex 1/Circle Center"),
    point("tangent2", "Tangent 2"),
    point("vertex2", "Vertex 2/Line Right"),
    ranged("customPath", "Custom Path", 0.0, 0.0, 32.0).doc("a mask's number to follow instead (0: none)"),
    flag("reversePath", "Reverse Path", false),
    choice("displayOptions", "Options", DISPLAY, 0),
    color("fillColor", "Fill Color", 1.0, 0.0, 0.0),
    color("strokeColor", "Stroke Color", 1.0, 1.0, 1.0),
    ranged("strokeWidth", "Stroke Width", 2.0, 0.0, 50.0),
    ranged("size", "Size", 36.0, 0.0, 512.0),
    ranged("tracking", "Tracking", 0.0, -100.0, 100.0),
    text("kerning", "Kerning", "the gap after a letter: index:amount pairs (from 0, in thousandths of the size), spaces between"),
    angle("characterRotation", "Character Rotation", 0.0),
    ranged("characterShear", "Character Shear", 0.0, -70.0, 70.0),
    ranged("horizontalShear", "Horizontal Shear", 0.0, -70.0, 70.0),
    ranged("horizontalScale", "Horizontal Scale", 100.0, 1.0, 1000.0),
    ranged("verticalScale", "Vertical Scale", 100.0, 1.0, 1000.0),
    choice("alignment", "Alignment", &["left", "right", "center", "force"], 0),
    ranged("leftMargin", "Left Margin", 0.0, -10_000.0, 10_000.0),
    ranged("rightMargin", "Right Margin", 0.0, -10_000.0, 10_000.0),
    ranged("lineSpacing", "Line Spacing", 43.0, -1000.0, 1000.0),
    ranged("baselineShift", "Baseline Shift", 0.0, -1000.0, 1000.0),
    ranged("visibleCharacters", "Visible Characters", 1024.0, 0.0, 1024.0),
    ranged("fadeTime", "Fade Time", 0.0, 0.0, 100.0).doc("letters fading in over this share of them, as Visible Characters grows"),
    ranged("baselineJitterMax", "Baseline Jitter Max", 0.0, 0.0, 1000.0),
    ranged("kerningJitterMax", "Kerning Jitter Max", 0.0, 0.0, 1000.0),
    ranged("rotationJitterMax", "Rotation Jitter Max", 0.0, 0.0, 360.0),
    ranged("scaleJitterMax", "Scale Jitter Max", 0.0, 0.0, 1000.0),
    flag("compositeOnOriginal", "Composite On Original", true),
]);
pub const PATH_TEXT: Spec = Spec::new(&PATH_TEXT_DEF, CATEGORY, path_text).points(&[("tangent1", [0.3, 0.3]), ("vertex1", [0.1, 0.6]), ("tangent2", [0.7, 0.3]), ("vertex2", [0.9, 0.6])]);

/// A path as points (layer units), whether it closes, and its length.
struct Line {
    points: Vec<[f64; 2]>,
    closed: bool,
    length: f64,
}

impl Line {
    fn new(points: Vec<[f64; 2]>, closed: bool) -> Line {
        let closed = closed && points.len() > 2;
        let mut l = Line { points, closed, length: 0.0 };
        l.length = l.pieces().map(|(a, z)| (z[0] - a[0]).hypot(z[1] - a[1])).sum();
        l
    }

    fn pieces(&self) -> impl Iterator<Item = ([f64; 2], [f64; 2])> + '_ {
        let n = self.points.len();
        let m = if self.closed { n } else { n.saturating_sub(1) };
        (0..m).map(move |i| (self.points[i], self.points[(i + 1) % n]))
    }

    /// The point `s` along it and the way it heads there (past its ends, on along its first or
    /// last piece).
    fn at(&self, s: f64) -> ([f64; 2], [f64; 2]) {
        let mut left = s;
        let mut last = ([0.0; 2], [1.0, 0.0]);
        for (k, (a, z)) in self.pieces().enumerate() {
            let len = (z[0] - a[0]).hypot(z[1] - a[1]);
            let dir = if len > 1e-12 { [(z[0] - a[0]) / len, (z[1] - a[1]) / len] } else { [1.0, 0.0] };
            if left <= len || (k == 0 && left < 0.0) {
                return ([a[0] + dir[0] * left, a[1] + dir[1] * left], dir);
            }
            left -= len;
            last = (z, dir);
        }
        ([last.0[0] + last.1[0] * left, last.0[1] + last.1[1] * left], last.1)
    }
}

fn path_of(ctx: &Ctx) -> Line {
    let mask = ctx.num("customPath", 0.0).round() as usize;
    let pt = |id: &str| {
        let p = ctx.point(id);
        [p.x, p.y]
    };
    let mut line = if let Some(m) = (mask > 0).then(|| ctx.mask(mask)).flatten().filter(|m| m.points.len() > 1) {
        Line::new(m.points.clone(), m.closed)
    } else {
        match ctx.choice("shapeType") {
            1 | 2 => {
                let (c, s) = (pt("vertex1"), pt("tangent1"));
                let r = (s[0] - c[0]).hypot(s[1] - c[1]).max(1.0);
                let a0 = (s[1] - c[1]).atan2(s[0] - c[0]);
                let n = ((r * TAU / 2.0).ceil() as usize).clamp(64, 1024);
                Line::new((0..n).map(|i| {
                    let a = a0 + TAU * i as f64 / n as f64;
                    [c[0] + a.cos() * r, c[1] + a.sin() * r]
                }).collect(), true)
            }
            3 => Line::new(vec![pt("vertex1"), pt("vertex2")], false),
            _ => {
                let (v1, t1, t2, v2) = (pt("vertex1"), pt("tangent1"), pt("tangent2"), pt("vertex2"));
                let reach = (t1[0] - v1[0]).hypot(t1[1] - v1[1]) + (t2[0] - t1[0]).hypot(t2[1] - t1[1]) + (v2[0] - t2[0]).hypot(v2[1] - t2[1]);
                let n = ((reach / 2.0).ceil() as usize).clamp(16, 1024);
                Line::new((0..=n).map(|i| {
                    let t = i as f64 / n as f64;
                    let u = 1.0 - t;
                    let f = |k: usize| u * u * u * v1[k] + 3.0 * u * u * t * t1[k] + 3.0 * u * t * t * t2[k] + t * t * t * v2[k];
                    [f(0), f(1)]
                }).collect(), false)
            }
        }
    };
    if ctx.flag("reversePath") {
        line.points.reverse();
    }
    line
}

fn path_text(ctx: &Ctx, b: Buf) -> Buf {
    let words: Vec<char> = value_text(ctx, "text").chars().collect();
    let Some(font) = font_for(&value_text(ctx, "font"), 400.0) else {
        ctx.note("no font could be found: nothing drawn".into());
        return b;
    };
    let line = path_of(ctx);
    let size = ctx.num("size", 36.0).max(0.5);
    let tracking = ctx.num("tracking", 0.0);
    let (margin_l, margin_r) = (ctx.num("leftMargin", 0.0), ctx.num("rightMargin", 0.0));
    let (shift, spacing) = (ctx.num("baselineShift", 0.0), ctx.num("lineSpacing", 43.0));
    let turn = ctx.num("characterRotation", 0.0).to_radians();
    let shear = ctx.num("characterShear", 0.0).to_radians().tan() + ctx.num("horizontalShear", 0.0).to_radians().tan();
    let (sx, sy) = (ctx.num("horizontalScale", 100.0) / 100.0, ctx.num("verticalScale", 100.0) / 100.0);
    let kerning: Vec<(usize, f64)> = value_text(ctx, "kerning")
        .split_whitespace()
        .filter_map(|t| {
            let (i, v) = t.split_once(':')?;
            Some((i.trim().parse().ok()?, v.trim().parse().ok()?))
        })
        .collect();
    let kern = |i: usize| kerning.iter().filter(|(k, _)| *k == i).map(|(_, v)| v / 1000.0 * size).sum::<f64>();
    // jitter: each letter its own, new every frame
    let frame = frame_of(ctx) as u64;
    let jitter = |i: usize, what: u64| crate::sprites::rand2(i as u64, what ^ frame.wrapping_mul(0x9e37_79b9), ctx.seed) as f64;
    let (j_base, j_kern, j_turn, j_scale) = (ctx.num("baselineJitterMax", 0.0), ctx.num("kerningJitterMax", 0.0), ctx.num("rotationJitterMax", 0.0).to_radians(), ctx.num("scaleJitterMax", 0.0) / 100.0);
    // fading in as Visible Characters grows
    let visible = ctx.num("visibleCharacters", 1024.0).max(0.0);
    let fade = (ctx.num("fadeTime", 0.0) / 100.0).clamp(0.0, 1.0) * words.len().max(1) as f64;
    let shown = |i: usize| if fade <= 0.0 { if (i as f64) < visible.floor() { 1.0 } else { 0.0 } } else { ((visible - i as f64) / fade).clamp(0.0, 1.0) };
    let looped = line.closed;
    let align = if ctx.mask(ctx.num("customPath", 0.0).round() as usize).is_none() && ctx.choice("shapeType") == 2 { 3 } else { ctx.choice("alignment") };
    // the letters by how much they show, each set as outlines
    let mut sets: Vec<(f32, Path)> = Vec::new();
    let mut index = 0;
    for (row, text_line) in words.split(|c| *c == '\n' || *c == '\r').enumerate() {
        let first = index;
        index += text_line.len() + 1;
        let glyphs: Vec<(veymelo_text::FontRef, u16, f64)> = text_line.iter().map(|&c| {
            let (face, g) = glyph_in(&font, c);
            let adv = face.advance(g) * size / face.units.max(1.0) * sx;
            (face, g, adv)
        }).collect();
        let n = glyphs.len();
        let gaps: Vec<f64> = (0..n).map(|j| tracking + kern(first + j) + if j_kern != 0.0 { jitter(first + j, 2) * j_kern } else { 0.0 }).collect();
        let natural: f64 = glyphs.iter().map(|g| g.2).sum::<f64>() + gaps.iter().take(n.saturating_sub(1)).sum::<f64>();
        let room = (line.length - margin_l - margin_r).max(0.0);
        let (mut pen, extra) = match align {
            1 => (line.length - margin_r - natural, 0.0),
            2 => (margin_l + (room - natural) * 0.5, 0.0),
            // spread to fill the room (round a loop, its whole way, the last letter meeting the
            // first)
            3 => {
                let gaps_n = if looped && ctx.choice("shapeType") == 2 { n as f64 } else { n.saturating_sub(1) as f64 };
                (margin_l, if gaps_n > 0.0 { (room - natural) / gaps_n } else { 0.0 })
            }
            _ => (margin_l, 0.0),
        };
        for (j, (face, g, adv)) in glyphs.iter().enumerate() {
            let (g, adv) = (*g, *adv);
            let i = first + j;
            let middle = pen + adv * 0.5;
            pen += adv + gaps[j] + extra;
            let alpha = shown(i) as f32;
            if alpha <= 0.0 || (!looped && (middle < 0.0 || middle > line.length)) {
                continue;
            }
            let (at, dir) = line.at(if looped { middle.rem_euclid(line.length.max(1e-9)) } else { middle });
            let a = dir[1].atan2(dir[0]) + turn + if j_turn != 0.0 { jitter(i, 3) * j_turn } else { 0.0 };
            let (sn, cs) = a.sin_cos();
            let grow = if j_scale != 0.0 { (1.0 + jitter(i, 4) * j_scale).max(0.05) } else { 1.0 };
            let lift = shift + if j_base != 0.0 { jitter(i, 1) * j_base } else { 0.0 } - row as f64 * spacing;
            let set = match sets.iter().position(|(v, _)| (*v - alpha).abs() < 1e-4) {
                Some(s) => s,
                None => {
                    sets.push((alpha, Path::default()));
                    sets.len() - 1
                }
            };
            // the letter about its middle on the baseline: scaled, sheared, lifted, turned
            let k = size / face.units.max(1.0);
            face.outline(g, &mut sets[set].1, &|fx, fy| {
                let (x, y) = ((fx * k * sx - adv * 0.5) * grow, -fy * k * sy * grow);
                let (x, y) = (x - y * shear, y - lift);
                (at[0] + x * cs - y * sn, at[1] + x * sn + y * cs)
            });
        }
    }
    let look = paint_of(ctx);
    finish(ctx, b, |b, img| {
        for (alpha, outline) in &sets {
            paint(b, img, outline, &Paint { opacity: *alpha, ..look });
        }
    })
}

// ---------------------------------------------------------------- Lightning

effect!(LIGHTNING_DEF, "lightning", "Lightning", "the old lightning: a jagged bolt between two points, branching, crackling as it goes, drawn with a bright core in a coloured glow", [
    point("startPoint", "Start Point"),
    point("endPoint", "End Point"),
    ranged("segments", "Segments", 7.0, 1.0, 64.0),
    ranged("amplitude", "Amplitude", 10.0, 0.0, 1000.0),
    ranged("detailLevel", "Detail Level", 2.0, 0.0, 6.0),
    ranged("detailAmplitude", "Detail Amplitude", 0.3, 0.0, 1.0),
    ranged("branching", "Branching", 0.4, 0.0, 1.0),
    ranged("rebranching", "Rebranching", 0.3, 0.0, 1.0),
    angle("branchAngle", "Branch Angle", 20.0),
    ranged("branchSegLength", "Branch Seg. Length", 0.5, 0.0, 2.0),
    ranged("branchSegments", "Branch Segments", 4.0, 1.0, 32.0),
    ranged("branchWidth", "Branch Width", 0.7, 0.0, 1.0),
    ranged("speed", "Speed", 10.0, -100.0, 100.0),
    ranged("stability", "Stability", 0.3, 0.0, 1.0),
    flag("fixedEndpoint", "Fixed Endpoint", true),
    ranged("width", "Width", 10.0, 0.0, 100.0),
    ranged("widthVariation", "Width Variation", 0.25, 0.0, 1.0),
    ranged("coreWidth", "Core Width", 0.4, 0.0, 1.0),
    color("outsideColor", "Outside Color", 0.36, 0.25, 0.9),
    color("insideColor", "Inside Color", 1.0, 1.0, 1.0),
    ranged("pullForce", "Pull Force", 0.0, -100.0, 100.0),
    angle("pullDirection", "Pull Direction", 0.0),
    ranged("randomSeed", "Random Seed", 1.0, 0.0, 10_000.0),
    choice("blendingMode", "Blending Mode", &["normal", "add", "screen"], 0),
    flag("rerunAtEachFrame", "Rerun At Each Frame", false).doc("a new bolt every frame"),
]);
pub const LIGHTNING: Spec = Spec::new(&LIGHTNING_DEF, CATEGORY, lightning).points(&[("startPoint", [0.1, 0.5]), ("endPoint", [0.9, 0.5])]);

/// How bolts are made.
struct Bolt {
    seed: u64,
    /// How far along their crackling they are.
    phase: f64,
    stability: f64,
    detail: u32,
    detail_amount: f64,
    pull: [f64; 2],
    made: u64,
}

impl Bolt {
    /// Smooth noise from -1 to 1 along `t`, its own for `i`.
    fn wobble(&self, i: u64, t: f64) -> f64 {
        crate::noise::smooth(t, 0.5, 0.5, crate::noise::mix(&[self.seed, i]), None).clamp(-1.0, 1.0) * 1.4
    }

    /// A bolt from `a` to `z` in `n` pieces swinging `amount` aside (held at both ends, or free
    /// at its far end), its pieces split again and again for detail.
    fn path(&mut self, a: [f64; 2], z: [f64; 2], n: usize, amount: f64, held: bool) -> Vec<[f64; 2]> {
        let (dx, dy) = (z[0] - a[0], z[1] - a[1]);
        let len = dx.hypot(dy).max(1e-9);
        let side = [-dy / len, dx / len];
        let id = self.made;
        self.made += 1;
        let speed = 1.0 - self.stability * 0.9;
        let mut pts: Vec<[f64; 2]> = (0..=n)
            .map(|i| {
                let t = i as f64 / n as f64;
                let hold = if held { (t * PI).sin() } else { t.sqrt() };
                let off = self.wobble(id * 1000 + i as u64, self.phase * speed + i as f64 * 0.37) * amount * hold;
                let pull = t * t * len * 0.25;
                [a[0] + dx * t + side[0] * off + self.pull[0] * pull, a[1] + dy * t + side[1] * off + self.pull[1] * pull]
            })
            .collect();
        for level in 0..self.detail {
            let mut finer = Vec::with_capacity(pts.len() * 2);
            for (j, w) in pts.windows(2).enumerate() {
                let (p, q) = (w[0], w[1]);
                let (ex, ey) = (q[0] - p[0], q[1] - p[1]);
                let l = ex.hypot(ey).max(1e-9);
                let off = self.wobble((id * 7919 + level as u64 * 131 + j as u64) | (1 << 40), self.phase * 1.7 + j as f64 * 0.61) * l * self.detail_amount;
                finer.push(p);
                finer.push([(p[0] + q[0]) * 0.5 - ey / l * off, (p[1] + q[1]) * 0.5 + ex / l * off]);
            }
            finer.push(*pts.last().unwrap_or(&a));
            pts = finer;
        }
        pts
    }
}

fn lightning(ctx: &Ctx, mut b: Buf) -> Buf {
    let (start, end) = (b.pixel(ctx.point("startPoint")), b.pixel(ctx.point("endPoint")));
    let s = b.scale;
    let again = ctx.flag("rerunAtEachFrame");
    let seed = crate::noise::mix(&[ctx.seed, ctx.num("randomSeed", 1.0) as u64, if again { frame_of(ctx) as u64 } else { u64::MAX }]);
    let pull_at = (ctx.num("pullDirection", 0.0) - 90.0).to_radians();
    let pull = ctx.num("pullForce", 0.0) / 100.0;
    let mut bolt = Bolt {
        seed,
        phase: if again { 0.0 } else { ctx.time.seconds() * ctx.num("speed", 10.0) * 0.25 },
        stability: ctx.num("stability", 0.3).clamp(0.0, 1.0),
        detail: ctx.num("detailLevel", 2.0).round().clamp(0.0, 6.0) as u32,
        detail_amount: ctx.num("detailAmplitude", 0.3).clamp(0.0, 1.0) * 0.5,
        pull: [pull_at.cos() * pull, pull_at.sin() * pull],
        made: 0,
    };
    let pieces = ctx.num("segments", 7.0).round().clamp(1.0, 64.0) as usize;
    let amount = ctx.num("amplitude", 10.0) * s;
    let main = bolt.path([start.x, start.y], [end.x, end.y], pieces, amount, ctx.flag("fixedEndpoint"));
    let (branching, rebranching) = (ctx.num("branching", 0.4).clamp(0.0, 1.0), ctx.num("rebranching", 0.3).clamp(0.0, 1.0));
    let spread = ctx.num("branchAngle", 20.0).to_radians();
    let branch_len = ctx.num("branchSegLength", 0.5).max(0.0);
    let branch_pieces = ctx.num("branchSegments", 4.0).round().clamp(1.0, 32.0) as usize;
    let branch_width = ctx.num("branchWidth", 0.7).clamp(0.0, 1.0);
    let piece_len = main.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum::<f64>() / pieces as f64;
    // the bolt and its branches (and theirs), each with its width
    let mut lines: Vec<(Vec<[f64; 2]>, f64)> = vec![(main.clone(), 1.0)];
    let mut todo = vec![(main, 1.0, 0u32)];
    while let Some((pts, w, depth)) = todo.pop() {
        if depth >= 3 || lines.len() > 256 {
            continue;
        }
        let chance = if depth == 0 { branching } else { rebranching };
        for (i, pair) in pts.windows(2).enumerate() {
            let r = |k: u64| crate::sprites::rand(i as u64 + depth as u64 * 977, k ^ ((lines.len() as u64) << 8), seed) as f64;
            if r(1) >= chance * 0.35 {
                continue;
            }
            let (p, q) = (pair[0], pair[1]);
            let heading = (q[1] - p[1]).atan2(q[0] - p[0]);
            let a = heading + if r(2) < 0.5 { -1.0 } else { 1.0 } * spread * (0.5 + r(3));
            let l = piece_len * branch_len * branch_pieces as f64;
            let fork = bolt.path(p, [p[0] + a.cos() * l, p[1] + a.sin() * l], branch_pieces, amount * 0.5, false);
            lines.push((fork.clone(), w * branch_width));
            todo.push((fork, w * branch_width, depth + 1));
        }
    }
    let width = (ctx.num("width", 10.0) * s).max(0.2);
    let vary = ctx.num("widthVariation", 0.25).clamp(0.0, 1.0);
    let core = ctx.num("coreWidth", 0.4).clamp(0.0, 1.0);
    let mut segs = Vec::new();
    for (li, (pts, w)) in lines.iter().enumerate() {
        for (i, pair) in pts.windows(2).enumerate() {
            let v = 1.0 - vary * crate::sprites::rand(i as u64, li as u64, seed ^ 0x77) as f64;
            segs.push(crate::draw::Seg { a: pair[0], b: pair[1], r: width * 0.5 * w * v, v: 1.0 });
        }
    }
    let (w, h) = (b.img.width, b.img.height);
    let outer = crate::draw::segments(w, h, &segs, 0.0);
    let cores: Vec<crate::draw::Seg> = segs.iter().map(|g| crate::draw::Seg { r: (g.r * core).max(0.3), ..*g }).collect();
    let inner = crate::draw::segments(w, h, &cores, 0.6);
    let mut plane = Image::new(w, h);
    for (p, v) in plane.px.iter_mut().zip(&outer) {
        *p = [*v; 4];
    }
    let glow = filter::gaussian(&plane, width * 0.5, width * 0.5, Edge::Clear);
    let (oc, ic) = (ctx.color("outsideColor"), ctx.color("insideColor"));
    let (oc, ic) = ([oc.r as f32, oc.g as f32, oc.b as f32], [ic.r as f32, ic.g as f32, ic.b as f32]);
    let mode = ctx.choice("blendingMode");
    for (i, px) in b.img.px.iter_mut().enumerate() {
        let o = outer[i].max(glow.px[i][0] * 0.8);
        let c = inner[i];
        let a = o.max(c).min(1.0);
        if a <= 0.0 {
            continue;
        }
        // the inside colour in the core, the outside colour about it
        let t = c / a;
        let col: [f32; 3] = std::array::from_fn(|k| oc[k] + (ic[k] - oc[k]) * t);
        let l: Px = [col[0] * a, col[1] * a, col[2] * a, a];
        *px = match mode {
            1 => [px[0] + l[0], px[1] + l[1], px[2] + l[2], (px[3] + a * (1.0 - px[3])).min(1.0)],
            2 => std::array::from_fn(|k| px[k] + l[k] - px[k] * l[k]),
            _ => {
                let k = 1.0 - a;
                [l[0] + px[0] * k, l[1] + px[1] * k, l[2] + px[2] * k, a + px[3] * k]
            }
        };
    }
    b
}

pub const ALL: [Spec; 5] = [BASIC_3D, BASIC_TEXT, GAUSSIAN, LIGHTNING, PATH_TEXT];

#[cfg(test)]
mod tests;
