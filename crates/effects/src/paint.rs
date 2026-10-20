//! Generators that paint: inside or along the layer's masks (Fill, Stroke, Scribble), along
//! lines found in a picture (Vegas), with a moving brush (Write-on, Glue Gun), from a point
//! (Paint Bucket, Eyedropper Fill), and woven threads.

use veymelo_keyframe::{Kind, Value};
use veymelo_project::PropDef;
use veymelo_raster::filter::{self, Edge};
use veymelo_raster::{Image, Px, Sampling};
use veymelo_time::Tick;

use crate::color::rgb3;
use crate::defs::{angle, choice, color, effect, flag, layer, num, pct, point, px, ranged};
use crate::draw::{self, Seg};
use crate::util::{hsl, luma, map_px, premul, rgb, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "generate";
const PAINT_STYLES: &[&str] = &["onOriginalImage", "onTransparent", "revealOriginalImage"];

const fn mask_number(id: &'static str, name: &'static str, v: f64) -> PropDef {
    PropDef::new(id, name, Kind::Number, Value::Number(v)).range(0.0, 1000.0).doc("which of the layer's masks, from 1 (0: none)")
}

/// `src` over `dst`, both premultiplied.
fn over(dst: Px, src: Px) -> Px {
    let k = 1.0 - src[3];
    [src[0] + dst[0] * k, src[1] + dst[1] * k, src[2] + dst[2] * k, src[3] + dst[3] * k]
}

/// Paint a colour onto the layer as much as `cover` says: over it, alone, or showing the
/// layer only where painted (`style` 0, 1, 2).
fn paint(img: &mut Image, cover: &[f32], c: [f32; 3], opacity: f32, style: u32) {
    let w = img.width;
    map_px(img, |x, y, p| {
        let a = (cover[y * w + x] * opacity).clamp(0.0, 1.0);
        match style {
            1 => premul(c, a),
            2 => p.map(|v| v * a),
            _ => over(p, premul(c, a)),
        }
    });
}

/// The masks with a number (or all of them), as outlines in the buffer's pixels: (points,
/// closed, inverted).
/// A mask's outline in buffer pixels, whether it is closed, and whether it is inverted.
type Outline = (Vec<[f64; 2]>, bool, bool);

fn masks_on(ctx: &Ctx, b: &Buf, number: usize, all: bool) -> Vec<Outline> {
    let to_px = |m: &crate::MaskShape| (m.points.iter().map(|q| {
        let p = b.pixel(veymelo_geom::vec2(q[0], q[1]));
        [p.x, p.y]
    }).collect(), m.closed, m.inverted);
    if all {
        ctx.masks.iter().filter(|m| !m.points.is_empty()).map(to_px).collect()
    } else {
        ctx.mask(number).filter(|m| !m.points.is_empty()).map(to_px).into_iter().collect()
    }
}

effect!(
    FILL_DEF,
    "fill",
    "Fill",
    "paints the layer's shape (or inside a mask) one colour",
    [
        mask_number("fillMask", "Fill Mask", 0.0),
        flag("allMasks", "All Masks", false),
        color("color", "Color", 1.0, 0.0, 0.0),
        flag("invert", "Invert", false).doc("paint outside instead (without a mask: the see-through parts)"),
        px("horizontalFeather", "Horizontal Feather", 0.0, 5000.0),
        px("verticalFeather", "Vertical Feather", 0.0, 5000.0),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0),
    ]
);
pub const FILL: Spec = Spec::new(&FILL_DEF, CATEGORY, fill);

fn fill(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = rgb3(ctx.color("color"));
    let invert = ctx.flag("invert");
    let op = ctx.num("opacity", 100.0) as f32 / 100.0;
    let shapes = masks_on(ctx, &b, ctx.num("fillMask", 0.0).round() as usize, ctx.flag("allMasks"));
    let shapes: Vec<(Vec<[f64; 2]>, bool)> = shapes.into_iter().filter(|s| s.0.len() >= 3).map(|(p, _, inv)| (p, inv)).collect();
    if !shapes.is_empty() {
        let (w, h) = (b.img.width, b.img.height);
        let mut cover = draw::filled(w, h, &shapes);
        let (fh, fv) = (ctx.num("horizontalFeather", 0.0).max(0.0) * b.scale, ctx.num("verticalFeather", 0.0).max(0.0) * b.scale);
        if fh > 0.0 || fv > 0.0 {
            let mut img = Image::new(w, h);
            for (p, v) in img.px.iter_mut().zip(&cover) {
                *p = [*v; 4];
            }
            cover = filter::gaussian(&img, fh * 0.5, fv * 0.5, Edge::Repeat).px.iter().map(|p| p[0]).collect();
        }
        map_px(&mut b.img, |x, y, p| {
            let m = cover[y * w + x];
            let m = if invert { 1.0 - m } else { m } * op;
            let a = p[3];
            [p[0] + (c[0] * a - p[0]) * m, p[1] + (c[1] * a - p[1]) * m, p[2] + (c[2] * a - p[2]) * m, a]
        });
        return b;
    }
    map_px(&mut b.img, |_, _, p| {
        let a = if invert { 1.0 - p[3] } else { p[3] };
        let f = premul(c, a);
        let na = if invert { p[3] + (a - p[3]) * op } else { p[3] };
        [p[0] + (f[0] - p[0]) * op, p[1] + (f[1] - p[1]) * op, p[2] + (f[2] - p[2]) * op, na]
    });
    b
}

effect!(
    STROKE_DEF,
    "stroke",
    "Stroke",
    "draws along a mask (or all of them), from start to end: animate the end to draw it on",
    [
        mask_number("path", "Path", 1.0),
        flag("allMasks", "All Masks", false),
        flag("strokeSequentially", "Stroke Sequentially", true).doc("with all masks: one after another, not all at once"),
        color("color", "Color", 1.0, 1.0, 1.0),
        px("brushSize", "Brush Size", 2.0, 50.0),
        pct("brushHardness", "Brush Hardness", 75.0, 0.0, 100.0),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0),
        pct("start", "Start", 0.0, 0.0, 100.0),
        pct("end", "End", 100.0, 0.0, 100.0),
        pct("spacing", "Spacing", 15.0, 0.0, 100.0).doc("above 50 the brush's dabs come apart"),
        choice("paintStyle", "Paint Style", PAINT_STYLES, 0),
    ]
);
pub const STROKE: Spec = Spec::new(&STROKE_DEF, CATEGORY, stroke);

fn stroke(ctx: &Ctx, mut b: Buf) -> Buf {
    let all = ctx.flag("allMasks");
    let lines = masks_on(ctx, &b, ctx.num("path", 1.0).round() as usize, all);
    let size = (ctx.num("brushSize", 2.0) * b.scale).max(0.1);
    let r = size * 0.5;
    let (start, end) = (ctx.num("start", 0.0).clamp(0.0, 100.0) / 100.0, ctx.num("end", 100.0).clamp(0.0, 100.0) / 100.0);
    let spacing = ctx.num("spacing", 15.0) / 100.0;
    let sequential = ctx.flag("strokeSequentially");
    let lens: Vec<f64> = lines.iter().map(|(p, c, _)| draw::length(p, *c)).collect();
    let total: f64 = lens.iter().sum();
    let mut segs = Vec::new();
    let mut done = 0.0;
    for ((pts, closed, _), &len) in lines.iter().zip(&lens) {
        let (s0, s1) = if all && sequential { ((total * start - done).clamp(0.0, len), (total * end - done).clamp(0.0, len)) } else { (len * start, len * end) };
        done += len;
        if s1 <= s0 {
            continue;
        }
        if spacing <= 0.5 {
            draw::along(&draw::trim(pts, *closed, s0, s1), false, r, 1.0, &mut segs);
        } else {
            let step = (size * spacing).max(0.5);
            let mut s = s0;
            while s <= s1 + 1e-9 && segs.len() < 200_000 {
                let (q, _) = draw::point_at(pts, *closed, s);
                segs.push(Seg { a: q, b: q, r, v: 1.0 });
                s += step;
            }
        }
    }
    let cover = draw::segments(b.img.width, b.img.height, &segs, ctx.num("brushHardness", 75.0) / 100.0);
    paint(&mut b.img, &cover, rgb3(ctx.color("color")), ctx.num("opacity", 100.0) as f32 / 100.0, ctx.choice("paintStyle"));
    b
}

effect!(
    SCRIBBLE_DEF,
    "scribble",
    "Scribble",
    "fills a mask with scribbled lines, as if drawn by hand, wiggling over time",
    [
        choice("scribble", "Scribble", &["singleMask", "allMasks", "allMasksUsingModes"], 0),
        mask_number("mask", "Mask", 1.0),
        choice("fillType", "Fill Type", &["inside", "centeredEdge", "insideEdge", "outsideEdge", "leftEdge", "rightEdge"], 0),
        px("edgeWidth", "Edge Width", 10.0, 1000.0),
        choice("endCap", "End Cap", &["round", "butt", "projecting"], 0),
        choice("join", "Join", &["round", "miter", "bevel"], 0),
        ranged("miterLimit", "Miter Limit", 4.0, 1.0, 100.0),
        color("color", "Color", 1.0, 0.0, 0.0),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0),
        angle("angle", "Angle", 45.0),
        num("strokeWidth", "Stroke Width", 2.0).range(0.1, 50.0),
        pct("curviness", "Curviness", 50.0, 0.0, 100.0),
        pct("curvinessVariation", "Curviness Variation", 50.0, 0.0, 100.0),
        num("spacing", "Spacing", 5.0).range(0.1, 100.0),
        pct("spacingVariation", "Spacing Variation", 50.0, 0.0, 100.0),
        pct("pathOverlap", "Path Overlap", 0.0, -100.0, 100.0),
        pct("pathOverlapVariation", "Path Overlap Variation", 50.0, 0.0, 100.0),
        pct("start", "Start", 0.0, 0.0, 100.0),
        pct("end", "End", 100.0, 0.0, 100.0),
        choice("startEndApplyTo", "Start/End Apply To", &["allStrokes", "eachStroke"], 0),
        flag("fillPathsSequentially", "Fill Paths Sequentially", true),
        choice("wiggleType", "Wiggle Type", &["static", "jumpy", "smooth"], 1),
        ranged("wigglesPerSecond", "Wiggles/Second", 5.0, 0.0, 100.0),
        ranged("randomSeed", "Random Seed", 1.0, 0.0, 10_000.0),
        choice("composite", "Composite", PAINT_STYLES, 1),
    ]
);
pub const SCRIBBLE: Spec = Spec::new(&SCRIBBLE_DEF, CATEGORY, scribble);

/// Where a scan line `n·p = s` runs inside the fill (from, to along `d`), for each outline.
#[allow(clippy::too_many_arguments)]
fn scan(polys: &[(Vec<[f64; 2]>, bool, bool)], fill: u32, ew: f64, d: [f64; 2], n: [f64; 2], s: f64, out: &mut Vec<(f64, f64)>) {
    out.clear();
    for (pts, _, inverted) in polys {
        let m = pts.len();
        if m < 2 {
            continue;
        }
        // where it crosses the outline, and how steeply
        let mut xs: Vec<(f64, f64)> = Vec::new();
        for i in 0..m {
            let (a, b) = (pts[i], pts[(i + 1) % m]);
            let (na, nb) = (a[0] * n[0] + a[1] * n[1] - s, b[0] * n[0] + b[1] * n[1] - s);
            if (na > 0.0) == (nb > 0.0) {
                continue;
            }
            let t = na / (na - nb);
            let q = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
            let sin = ((ex * n[0] + ey * n[1]) / ex.hypot(ey).max(1e-9)).abs().max(0.2);
            xs.push((q[0] * d[0] + q[1] * d[1], sin));
        }
        xs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let area: f64 = (0..m).map(|i| pts[i][0] * pts[(i + 1) % m][1] - pts[(i + 1) % m][0] * pts[i][1]).sum();
        let left_inside = area < 0.0;
        let mode = match fill {
            4 => if left_inside { 2 } else { 3 },
            5 => if left_inside { 3 } else { 2 },
            f => f,
        };
        let mode = if *inverted && mode == 0 { 6 } else { mode };
        for (k, &(t, sin)) in xs.iter().enumerate() {
            let entering = k % 2 == 0;
            let l = ew / sin;
            match mode {
                0 if entering && k + 1 < xs.len() => out.push((t, xs[k + 1].0)),
                1 => out.push((t - l * 0.5, t + l * 0.5)),
                2 => out.push(if entering { (t, t + l) } else { (t - l, t) }),
                3 => out.push(if entering { (t - l, t) } else { (t, t + l) }),
                _ => {}
            }
        }
        if mode == 6 {
            // inverted: what lies outside, within the outline's reach
            let lo = pts.iter().map(|q| q[0] * d[0] + q[1] * d[1]).fold(f64::INFINITY, f64::min) - ew;
            let hi = pts.iter().map(|q| q[0] * d[0] + q[1] * d[1]).fold(f64::NEG_INFINITY, f64::max) + ew;
            let mut cur = lo;
            for pair in xs.chunks(2) {
                out.push((cur, pair[0].0));
                cur = pair.get(1).map_or(hi, |v| v.0);
            }
            out.push((cur, hi));
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut joined: Vec<(f64, f64)> = Vec::with_capacity(out.len());
    for &(a, b) in out.iter() {
        match joined.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => joined.push((a, b)),
        }
    }
    *out = joined;
}

fn scribble(ctx: &Ctx, mut b: Buf) -> Buf {
    let which = ctx.choice("scribble");
    let polys: Vec<_> = masks_on(ctx, &b, ctx.num("mask", 1.0).round().max(1.0) as usize, which != 0).into_iter().filter(|p| p.0.len() >= 3).map(|(p, c, i)| (p, c, i && which == 2)).collect();
    let style = ctx.choice("composite");
    if polys.is_empty() {
        if style == 1 {
            b.img = Image::new(b.img.width, b.img.height);
        }
        return b;
    }
    let sc = b.scale;
    let fill = ctx.choice("fillType");
    let ew = ctx.num("edgeWidth", 10.0).max(0.1) * sc;
    let ang = ctx.num("angle", 45.0).to_radians();
    let (d, n) = ([ang.cos(), ang.sin()], [-ang.sin(), ang.cos()]);
    let width = (ctx.num("strokeWidth", 2.0) * sc).max(0.1);
    let (curv, curv_var) = (ctx.num("curviness", 50.0) / 100.0, ctx.num("curvinessVariation", 50.0) / 100.0);
    let spacing = (ctx.num("spacing", 5.0) * sc).max(0.75).max(width * 0.25);
    let spacing_var = ctx.num("spacingVariation", 50.0) / 100.0;
    let (overlap, overlap_var) = (ctx.num("pathOverlap", 0.0) / 100.0, ctx.num("pathOverlapVariation", 50.0) / 100.0);
    let (start, end) = (ctx.num("start", 0.0).clamp(0.0, 100.0) / 100.0, ctx.num("end", 100.0).clamp(0.0, 100.0) / 100.0);
    let sequential = ctx.flag("fillPathsSequentially");
    let wps = ctx.num("wigglesPerSecond", 5.0).max(0.0);
    let seed = crate::noise::mix(&[ctx.num("randomSeed", 1.0) as i64 as u64, ctx.seed]);
    // the wiggle: new random numbers each step (jumpy), or eased from one step's to the next
    let u = (ctx.time.seconds() * wps).max(0.0);
    let (step, ease) = match ctx.choice("wiggleType") {
        0 => (0u64, 0f64),
        1 => (u.floor() as u64, 0.0),
        _ => {
            let f = u.fract();
            (u.floor() as u64, f * f * (3.0 - 2.0 * f))
        }
    };
    let rnd = |i: u64, k: u64| {
        let at = |s: u64| crate::noise::unit(crate::noise::mix(&[seed, i, k, s])) * 2.0 - 1.0;
        let a = at(step);
        if ease > 0.0 { a + (at(step + 1) - a) * ease } else { a }
    };
    let groups: Vec<Vec<Outline>> = if which == 0 || sequential { vec![polys.clone()] } else { polys.iter().map(|p| vec![p.clone()]).collect() };
    let mut lines: Vec<Vec<Vec<[f64; 2]>>> = Vec::new();
    let mut runs = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for (pts, _, _) in g {
            for q in pts {
                let v = q[0] * n[0] + q[1] * n[1];
                (lo, hi) = (lo.min(v), hi.max(v));
            }
        }
        let (lo, hi) = (lo - ew, hi + ew);
        // zig-zags: each run joins the nearest line that ended on the scan line before
        let mut zigs: Vec<(Vec<[f64; 2]>, u64)> = Vec::new();
        let (mut s, mut li) = (lo + spacing * 0.5, 0u64);
        let reach = spacing * 3.0 + ew;
        while s < hi && li < 20_000 {
            scan(g, fill, ew, d, n, s, &mut runs);
            for (ri, &(a, bb)) in runs.iter().enumerate() {
                let k = ((gi as u64) << 20) | ri as u64;
                let o1 = spacing * (overlap + overlap_var * rnd(li, k * 4) * 0.5).max(-0.45);
                let o2 = spacing * (overlap + overlap_var * rnd(li, k * 4 + 1) * 0.5).max(-0.45);
                let (a, bb) = (a - o1, bb + o2);
                if bb <= a {
                    continue;
                }
                let at = |t: f64| [d[0] * t + n[0] * s, d[1] * t + n[1] * s];
                let (pa, pb) = (at(a), at(bb));
                let dist = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]);
                let best = zigs
                    .iter()
                    .enumerate()
                    .filter(|(_, z)| li > 0 && z.1 == li - 1)
                    .map(|(i, z)| {
                        let last = *z.0.last().unwrap_or(&pa);
                        (i, dist(last, pa).min(dist(last, pb)))
                    })
                    .filter(|&(_, dd)| dd < reach)
                    .min_by(|x, y| x.1.total_cmp(&y.1));
                match best {
                    Some((zi, _)) => {
                        let last = *zigs[zi].0.last().unwrap_or(&pa);
                        let (p0, p1, way) = if dist(last, pa) <= dist(last, pb) { (pa, pb, -1.0) } else { (pb, pa, 1.0) };
                        // a rounded turn: a point pushed past the edge
                        let c = (curv + curv_var * rnd(li, k * 4 + 2) * 0.5).max(0.0);
                        let mid = [(last[0] + p0[0]) * 0.5 + d[0] * way * c * spacing, (last[1] + p0[1]) * 0.5 + d[1] * way * c * spacing];
                        zigs[zi].0.extend([mid, p0, p1]);
                        zigs[zi].1 = li;
                    }
                    None => zigs.push((vec![pa, pb], li)),
                }
            }
            s += spacing * (1.0 + spacing_var * rnd(li, 3) * 0.5).max(0.25);
            li += 1;
        }
        lines.push(zigs.into_iter().map(|(z, _)| if curv > 0.0 { draw::rounded(&z, 2) } else { z }).collect());
    }
    let each = ctx.choice("startEndApplyTo") == 1;
    let mut parts: Vec<Vec<[f64; 2]>> = Vec::new();
    for group in &lines {
        let lens: Vec<f64> = group.iter().map(|l| draw::length(l, false)).collect();
        let total: f64 = lens.iter().sum();
        let mut done = 0.0;
        for (l, len) in group.iter().zip(lens) {
            parts.push(if each { draw::trim(l, false, len * start, len * end) } else { draw::trim(l, false, (total * start - done).clamp(0.0, len), (total * end - done).clamp(0.0, len)) });
            done += len;
        }
    }
    let (cap, join) = (ctx.choice("endCap"), ctx.choice("join"));
    let r = width * 0.5;
    let (w, h) = (b.img.width, b.img.height);
    let cover = if cap == 0 && join == 0 {
        let mut segs = Vec::new();
        for p in &parts {
            draw::along(p, false, r, 1.0, &mut segs);
        }
        draw::segments(w, h, &segs, 0.9)
    } else {
        draw::convex(w, h, &draw::outline_pieces(&parts, r, cap, join, ctx.num("miterLimit", 4.0).max(1.0)))
    };
    paint(&mut b.img, &cover, rgb3(ctx.color("color")), ctx.num("opacity", 100.0) as f32 / 100.0, style);
    b
}

effect!(
    VEGAS_DEF,
    "vegas",
    "Vegas",
    "lights running round the edges of what is in a picture (or along a mask), like the lights of a sign",
    [
        choice("stroke", "Stroke", &["imageContours", "maskPath"], 0),
        layer("inputLayer", "Input Layer").doc("the picture whose edges are followed (none: this layer)"),
        flag("invertInput", "Invert Input", false),
        choice("ifLayerSizesDiffer", "If Layer Sizes Differ", &["center", "stretchToFit"], 0),
        choice("channel", "Channel", &["intensity", "red", "green", "blue", "alpha"], 0),
        pct("threshold", "Threshold", 50.0, 0.0, 100.0),
        px("preBlur", "Pre-Blur", 1.0, 100.0),
        ranged("tolerance", "Tolerance", 0.01, 0.0, 1.0),
        choice("render", "Render", &["allContours", "selectedContour"], 0),
        ranged("selectedContour", "Selected Contour", 1.0, 1.0, 1000.0),
        choice("shorterContoursHave", "Shorter Contours Have", &["sameNumberOfSegments", "fewerSegments"], 0),
        mask_number("path", "Path", 1.0),
        ranged("segments", "Segments", 32.0, 1.0, 1000.0),
        ranged("length", "Length", 0.5, 0.0, 1.0).doc("how much of each stretch is lit"),
        choice("segmentDistribution", "Segment Distribution", &["bunched", "even"], 0),
        angle("rotation", "Rotation", 0.0).doc("moves the lights round (animate it)"),
        flag("randomPhase", "Random Phase", false),
        ranged("randomSeed", "Random Seed", 1.0, 1.0, 10_000.0),
        choice("blendMode", "Blend Mode", &["transparent", "over", "under", "stencil"], 1),
        color("color", "Color", 1.0, 1.0, 1.0),
        num("width", "Width", 2.0).range(0.0, 100.0),
        ranged("hardness", "Hardness", 0.5, 0.0, 1.0),
        ranged("startOpacity", "Start Opacity", 1.0, 0.0, 1.0),
        ranged("midpointOpacity", "Mid-point Opacity", 0.5, 0.0, 1.0),
        ranged("midpointPosition", "Mid-point Position", 0.5, 0.0, 1.0),
        ranged("endOpacity", "End Opacity", 0.0, 0.0, 1.0),
    ]
);
pub const VEGAS: Spec = Spec::new(&VEGAS_DEF, CATEGORY, vegas);

fn vegas(ctx: &Ctx, mut b: Buf) -> Buf {
    let (w, h) = (b.img.width, b.img.height);
    let sc = b.scale;
    let lines: Vec<(Vec<[f64; 2]>, bool)> = if ctx.choice("stroke") == 1 {
        masks_on(ctx, &b, ctx.num("path", 1.0).round() as usize, false).into_iter().map(|(p, c, _)| (p, c)).collect()
    } else {
        let src = ctx.layer_over("inputLayer", &b, Take::All, ctx.choice("ifLayerSizesDiffer") == 1).unwrap_or_else(|| b.img.clone());
        let ch = ctx.choice("channel");
        let inv = ctx.flag("invertInput");
        let mut plane = Image::new(w, h);
        for (o, p) in plane.px.iter_mut().zip(&src.px) {
            let (c, a) = unpremul(*p);
            let v = match ch {
                1 => c[0],
                2 => c[1],
                3 => c[2],
                4 => a,
                _ => luma(c) * a,
            };
            *o = [if inv { 1.0 - v } else { v }; 4];
        }
        let blur = ctx.num("preBlur", 1.0) * sc;
        if blur > 0.05 {
            plane = filter::gaussian(&plane, blur, blur, Edge::Repeat);
        }
        let values: Vec<f32> = plane.px.iter().map(|p| p[0]).collect();
        let mut found = draw::contours(&values, w, h, ctx.num("threshold", 50.0) as f32 / 100.0);
        let tol = ctx.num("tolerance", 0.01).max(0.0) * w.max(h) as f64 * 0.1;
        if tol > 0.0 {
            for (c, _) in found.iter_mut() {
                let mut kept: Vec<[f64; 2]> = Vec::with_capacity(c.len());
                for q in c.iter() {
                    if kept.last().is_none_or(|l| (l[0] - q[0]).hypot(l[1] - q[1]) >= tol) {
                        kept.push(*q);
                    }
                }
                *c = kept;
            }
        }
        found.sort_by(|a, b| draw::length(&b.0, b.1).total_cmp(&draw::length(&a.0, a.1)));
        if ctx.choice("render") == 1 {
            found.into_iter().skip(ctx.num("selectedContour", 1.0).round().max(1.0) as usize - 1).take(1).collect()
        } else {
            found
        }
    };
    let count = ctx.num("segments", 32.0).round().clamp(1.0, 1000.0) as usize;
    let fewer = ctx.choice("stroke") == 0 && ctx.choice("shorterContoursHave") == 1;
    let longest = lines.iter().map(|(p, c)| draw::length(p, *c)).fold(0.0, f64::max);
    let lit = ctx.num("length", 0.5).clamp(0.0, 1.0);
    let bunched = ctx.choice("segmentDistribution") == 0;
    let turn = ctx.num("rotation", 0.0) / 360.0;
    let random_phase = ctx.flag("randomPhase");
    let seed = crate::noise::mix(&[ctx.num("randomSeed", 1.0) as i64 as u64, ctx.seed]);
    let rnd = |a: u64, b: u64| crate::noise::unit(crate::noise::mix(&[seed, a, b]));
    let width = (ctx.num("width", 2.0) * sc).max(0.1);
    let (o0, om, o1, mp) = (ctx.num("startOpacity", 1.0) as f32, ctx.num("midpointOpacity", 0.5) as f32, ctx.num("endOpacity", 0.0) as f32, ctx.num("midpointPosition", 0.5).clamp(0.01, 0.99));
    let mut segs = Vec::new();
    for (ci, (pts, closed)) in lines.iter().enumerate() {
        let len = draw::length(pts, *closed);
        if len <= 0.0 {
            continue;
        }
        let phase = turn + if random_phase { rnd(ci as u64, 1) } else { 0.0 };
        let count = if fewer && longest > 0.0 { ((count as f64 * len / longest).round() as usize).max(1) } else { count };
        let each = len / count as f64 * lit;
        for i in 0..count {
            let base = if bunched { (i as f64 + (rnd(ci as u64, i as u64 + 7) - 0.5) * 0.8) / count as f64 } else { i as f64 / count as f64 };
            let s0 = (base + phase).rem_euclid(1.0) * len;
            // a dozen pieces, so the opacity can change along it
            let mut prev: Option<[f64; 2]> = None;
            for k in 0..=12 {
                let f = k as f64 / 12.0;
                let mut s = s0 + each * f;
                if *closed {
                    s = s.rem_euclid(len);
                } else if s > len {
                    break;
                }
                let (q, _) = draw::point_at(pts, *closed, s);
                let op = if f < mp { o0 + (om - o0) * (f / mp) as f32 } else { om + (o1 - om) * ((f - mp) / (1.0 - mp)) as f32 };
                if let Some(pv) = prev
                    && (pv[0] - q[0]).powi(2) + (pv[1] - q[1]).powi(2) < (each * 0.5).powi(2) + 4.0
                {
                    segs.push(Seg { a: pv, b: q, r: width * 0.5, v: op.clamp(0.0, 1.0) });
                }
                prev = Some(q);
            }
        }
    }
    let cover = draw::segments(w, h, &segs, ctx.num("hardness", 0.5).clamp(0.0, 1.0));
    let c = rgb3(ctx.color("color"));
    match ctx.choice("blendMode") {
        1 => paint(&mut b.img, &cover, c, 1.0, 0),
        2 => map_px(&mut b.img, |x, y, p| over(premul(c, cover[y * w + x]), p)),
        3 => paint(&mut b.img, &cover, c, 1.0, 2),
        _ => paint(&mut b.img, &cover, c, 1.0, 1),
    }
    b
}

effect!(
    WRITE_ON_DEF,
    "writeOn",
    "Write-on",
    "draws a line wherever the brush has been: animate the brush's position",
    [
        point("brushPosition", "Brush Position"),
        color("color", "Color", 1.0, 1.0, 1.0),
        px("brushSize", "Brush Size", 6.0, 50.0),
        pct("brushHardness", "Brush Hardness", 75.0, 0.0, 100.0),
        pct("brushOpacity", "Brush Opacity", 100.0, 0.0, 100.0),
        ranged("strokeLength", "Stroke Length (secs)", 0.0, 0.0, 30.0).doc("how long a dab stays (0: for ever)"),
        ranged("brushSpacing", "Brush Spacing (secs)", 0.001, 0.001, 5.0).doc("how often the brush dabs"),
        choice("paintTimeProperties", "Paint Time Properties", &["none", "opacity", "color"], 0).doc("each dab keeps the opacity (or colour) it had"),
        choice("brushTimeProperties", "Brush Time Properties", &["none", "size", "hardness", "sizeAndHardness"], 0),
        choice("paintStyle", "Paint Style", PAINT_STYLES, 0),
    ]
);
pub const WRITE_ON: Spec = Spec::new(&WRITE_ON_DEF, CATEGORY, write_on).points(&[("brushPosition", [0.5, 0.5])]);

/// The most dabs one frame draws; longer strokes lay them further apart, joined.
const MOST_DABS: i64 = 4096;

fn write_on(ctx: &Ctx, mut b: Buf) -> Buf {
    let own_opacity = ctx.choice("paintTimeProperties") == 1;
    let own_size = matches!(ctx.choice("brushTimeProperties"), 1 | 3);
    let t = ctx.time.seconds();
    let spacing = ctx.num("brushSpacing", 0.001).max(0.001);
    let life = ctx.num("strokeLength", 0.0).max(0.0);
    let now_r = (ctx.num("brushSize", 6.0) * b.scale * 0.5).max(0.05);
    let dab = |at: Tick| {
        let v2 = ctx.value_at("brushPosition", at).map_or([0.0; 2], |v| v.as_vec2());
        let q = b.pixel(veymelo_geom::vec2(v2[0], v2[1]));
        let r = if own_size { (ctx.value_at("brushSize", at).map_or(6.0, |v| v.as_f64()) * b.scale * 0.5).max(0.05) } else { now_r };
        let v = if own_opacity { (ctx.value_at("brushOpacity", at).map_or(100.0, |v| v.as_f64()) / 100.0).clamp(0.0, 1.0) as f32 } else { 1.0 };
        ([q.x, q.y], r, v)
    };
    let mut segs = Vec::new();
    if t >= 0.0 {
        let first = if life > 0.0 { (t - life).max(0.0) } else { 0.0 };
        let (k0, k1) = ((first / spacing - 1e-9).ceil().max(0.0) as i64, (t / spacing + 1e-9).floor().max(0.0) as i64);
        let stride = ((k1 - k0 + 1) / MOST_DABS + 1).max(1);
        let mut prev: Option<[f64; 2]> = None;
        let mut k = k0;
        while k <= k1 {
            let (q, r, v) = dab(Tick::from_seconds(k as f64 * spacing));
            // dabs laid further apart than the spacing are joined into a line
            segs.push(Seg { a: if stride > 1 { prev.unwrap_or(q) } else { q }, b: q, r, v });
            prev = Some(q);
            k += stride;
        }
    }
    if segs.is_empty() {
        let (q, r, v) = dab(ctx.time);
        segs.push(Seg { a: q, b: q, r, v });
    }
    let cover = draw::segments(b.img.width, b.img.height, &segs, ctx.num("brushHardness", 75.0) / 100.0);
    let opacity = if own_opacity { 1.0 } else { ctx.num("brushOpacity", 100.0) as f32 / 100.0 };
    paint(&mut b.img, &cover, rgb3(ctx.color("color")), opacity, ctx.choice("paintStyle"));
    b
}

effect!(
    PAINT_BUCKET_DEF,
    "paintBucket",
    "Paint Bucket",
    "fills the area round a point that is like it, as a paint bucket does",
    [
        point("fillPoint", "Fill Point"),
        choice("fillSelector", "Fill Selector", &["colorAndAlpha", "straightColor", "transparency", "opacity", "alphaChannel"], 0),
        ranged("tolerance", "Tolerance", 10.0, 0.0, 255.0),
        flag("viewThreshold", "View Threshold", false).doc("show what is filled, white on black"),
        choice("stroke", "Stroke", &["antialias", "feather", "spread", "choke", "stroke"], 0),
        px("featherSoftness", "Feather Softness", 10.0, 100.0),
        px("spreadRadius", "Spread Radius", 3.0, 100.0),
        px("strokeWidth", "Stroke Width", 2.0, 100.0),
        color("color", "Color", 1.0, 0.0, 0.0),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0),
        choice("blendingMode", "Blending Mode", &["normal", "add", "multiply", "screen", "hue", "saturation", "color", "luminosity", "behind", "fillOnly"], 0),
    ]
);
pub const PAINT_BUCKET: Spec = Spec::new(&PAINT_BUCKET_DEF, CATEGORY, paint_bucket).points(&[("fillPoint", [0.5, 0.5])]);

fn paint_bucket(ctx: &Ctx, mut b: Buf) -> Buf {
    let (w, h) = (b.img.width, b.img.height);
    if w == 0 || h == 0 {
        return b;
    }
    let fp = b.pixel(ctx.point("fillPoint"));
    let (sx, sy) = (fp.x.floor() as i64, fp.y.floor() as i64);
    let sel = ctx.choice("fillSelector");
    let tol = ctx.num("tolerance", 10.0) as f32 / 255.0;
    let key = |p: Px| -> Px {
        let (c, a) = unpremul(p);
        match sel {
            1 => [c[0], c[1], c[2], 0.0],
            2 => [0.0, 0.0, 0.0, if a <= 1e-3 { 0.0 } else { 1.0 }],
            3 | 4 => [0.0, 0.0, 0.0, a],
            _ => p,
        }
    };
    // the area like the fill point, joined to it (a flood fill along rows)
    let mut region = vec![0f32; w * h];
    if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
        let seed = key(b.img.px[sy as usize * w + sx as usize]);
        let like = |i: usize| {
            let k = key(b.img.px[i]);
            if sel == 2 { k[3] == seed[3] } else { (0..4).all(|j| (k[j] - seed[j]).abs() <= tol + 1e-6) }
        };
        let mut seen = vec![false; w * h];
        let mut stack = vec![(sx as usize, sy as usize)];
        while let Some((x, y)) = stack.pop() {
            let row = y * w;
            if seen[row + x] || !like(row + x) {
                continue;
            }
            let (mut l, mut r) = (x, x);
            while l > 0 && !seen[row + l - 1] && like(row + l - 1) {
                l -= 1;
            }
            while r + 1 < w && !seen[row + r + 1] && like(row + r + 1) {
                r += 1;
            }
            for i in l..=r {
                seen[row + i] = true;
                region[row + i] = 1.0;
                if y > 0 && !seen[row - w + i] {
                    stack.push((i, y - 1));
                }
                if y + 1 < h && !seen[row + w + i] {
                    stack.push((i, y + 1));
                }
            }
        }
    }
    if ctx.flag("viewThreshold") {
        for (p, v) in b.img.px.iter_mut().zip(&region) {
            *p = [*v, *v, *v, 1.0];
        }
        return b;
    }
    let sc = b.scale;
    let soft = |plane: &[f32], s: f64| -> Vec<f32> {
        if s <= 0.05 {
            return plane.to_vec();
        }
        let mut img = Image::new(w, h);
        for (p, v) in img.px.iter_mut().zip(plane) {
            *p = [*v; 4];
        }
        filter::gaussian(&img, s, s, Edge::Repeat).px.iter().map(|p| p[0]).collect()
    };
    let cover: Vec<f32> = match ctx.choice("stroke") {
        1 => soft(&region, ctx.num("featherSoftness", 10.0) * sc * 0.5),
        2 => draw::grown(&region, w, h, ctx.num("spreadRadius", 3.0) * sc),
        3 => draw::grown(&region, w, h, -ctx.num("spreadRadius", 3.0) * sc),
        4 => {
            let r = ctx.num("strokeWidth", 2.0) * sc * 0.5;
            draw::grown(&region, w, h, r).iter().zip(draw::grown(&region, w, h, -r)).map(|(a, b)| (a - b).max(0.0)).collect()
        }
        // antialias: the inside stays solid, only the boundary softens
        _ => soft(&region, 0.5).iter().zip(&region).map(|(a, r)| if *r > 0.0 { a.max(0.5) } else { *a }).collect(),
    };
    let c = rgb3(ctx.color("color"));
    let op = ctx.num("opacity", 100.0) as f32 / 100.0;
    let mode = ctx.choice("blendingMode");
    map_px(&mut b.img, |x, y, p| {
        let k = (cover[y * w + x] * op).clamp(0.0, 1.0);
        if k <= 0.0 {
            return if mode == 9 { [0.0; 4] } else { p };
        }
        let (dc, da) = unpremul(p);
        let mixed = match mode {
            1 => [dc[0] + c[0], dc[1] + c[1], dc[2] + c[2]],
            2 => [dc[0] * c[0], dc[1] * c[1], dc[2] * c[2]],
            3 => [0, 1, 2].map(|i| 1.0 - (1.0 - dc[i]) * (1.0 - c[i])),
            4..=7 => {
                let ((dh, ds, dl), (fh, fs, fl)) = (hsl(dc), hsl(c));
                match mode {
                    4 => rgb(fh, ds, dl),
                    5 => rgb(dh, fs, dl),
                    6 => rgb(fh, fs, dl),
                    _ => rgb(dh, ds, fl),
                }
            }
            _ => c,
        };
        match mode {
            8 => over(premul(c, k), p),
            9 => premul(c, k),
            0 => over(p, premul(c, k)),
            _ if da <= 1e-6 => p,
            _ => premul([0, 1, 2].map(|i| dc[i] + (mixed[i] - dc[i]) * k), da),
        }
    });
    b
}

effect!(
    EYEDROPPER_DEF,
    "eyedropperFill",
    "Eyedropper Fill",
    "fills the layer with the colour at (or round) a point of it",
    [
        point("samplePoint", "Sample Point"),
        px("sampleRadius", "Sample Radius", 0.0, 1000.0),
        choice("averagePixelColors", "Average Pixel Colors", &["skipEmpty", "all", "allPremultiplied", "includingAlpha"], 0),
        flag("maintainOriginalAlpha", "Maintain Original Alpha", false),
        pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),
    ]
);
pub const EYEDROPPER: Spec = Spec::new(&EYEDROPPER_DEF, CATEGORY, eyedropper).points(&[("samplePoint", [0.5, 0.5])]);

fn eyedropper(ctx: &Ctx, mut b: Buf) -> Buf {
    let (w, h) = (b.img.width as i64, b.img.height as i64);
    let c = b.pixel(ctx.point("samplePoint"));
    let r = (ctx.num("sampleRadius", 0.0) * b.scale).max(0.0);
    let mode = ctx.choice("averagePixelColors");
    let (cx, cy, ri) = (c.x.floor() as i64, c.y.floor() as i64, r.ceil() as i64);
    let (mut acc, mut n) = ([0f64; 4], 0f64);
    for y in (cy - ri).max(0)..=(cy + ri).min(h - 1) {
        for x in (cx - ri).max(0)..=(cx + ri).min(w - 1) {
            if ((x - cx).pow(2) + (y - cy).pow(2)) as f64 > r * r + 0.5 {
                continue;
            }
            let p = b.img.px[(y * w + x) as usize];
            let (s, _) = unpremul(p);
            match mode {
                0 if p[3] <= 1e-4 => continue,
                0 | 1 => (0..3).for_each(|k| acc[k] += s[k] as f64),
                2 => (0..3).for_each(|k| acc[k] += p[k] as f64),
                _ => (0..4).for_each(|k| acc[k] += p[k] as f64),
            }
            n += 1.0;
        }
    }
    let fill: Px = if n <= 0.0 {
        [0.0; 4]
    } else if mode == 3 {
        let a = (acc[3] / n) as f32;
        if a > 1e-6 { [(acc[0] / n) as f32 / a, (acc[1] / n) as f32 / a, (acc[2] / n) as f32 / a, a] } else { [0.0; 4] }
    } else {
        [(acc[0] / n) as f32, (acc[1] / n) as f32, (acc[2] / n) as f32, 1.0]
    };
    let keep_alpha = ctx.flag("maintainOriginalAlpha");
    let keep = ctx.num("blendWithOriginal", 0.0) as f32 / 100.0;
    map_px(&mut b.img, |_, _, p| {
        let f = premul([fill[0], fill[1], fill[2]], if keep_alpha { p[3] } else { fill[3] });
        [0, 1, 2, 3].map(|i| f[i] + (p[i] - f[i]) * keep)
    });
    b
}

effect!(
    GLUE_GUN_DEF,
    "glueGun",
    "Glue Gun",
    "a bead of glossy glue where the brush is, bending what is under it",
    [
        point("brushPosition", "Brush Position"),
        px("strokeWidth", "Stroke Width", 25.0, 500.0),
        ranged("density", "Density", 5.0, 0.0, 32.0),
        ranged("timeSpan", "Time Span (sec)", 1.0, 0.0, 60.0).doc("how long the glue stays behind the brush"),
        angle("reflection", "Reflection", 45.0).doc("where the light comes from"),
        pct("strength", "Strength", 100.0, 0.0, 100.0),
        choice("style", "Style", &["overLayer", "glueOnly"], 0),
    ]
);
pub const GLUE_GUN: Spec = Spec::new(&GLUE_GUN_DEF, CATEGORY, glue_gun).points(&[("brushPosition", [0.5, 0.5])]);

fn glue_gun(ctx: &Ctx, mut b: Buf) -> Buf {
    let rad = (ctx.num("strokeWidth", 25.0) * b.scale * 0.5).max(0.5);
    let density = ctx.num("density", 5.0).max(0.0);
    let refl = ctx.num("reflection", 45.0).to_radians();
    let strength = ctx.num("strength", 100.0) as f32 / 100.0;
    let glue_only = ctx.choice("style") == 1;
    let span = ctx.num("timeSpan", 1.0).max(0.0);
    // the brush's path over the time span: beads along it
    let at = |t: Tick| {
        let v = ctx.value_at("brushPosition", t).map_or([0.0; 2], |v| v.as_vec2());
        let p = b.pixel(veymelo_geom::vec2(v[0], v[1]));
        [p.x, p.y]
    };
    let beads = (density.round() as usize).clamp(1, 32);
    let steps = if span > 0.0 { ((span / ctx.frame.seconds().max(1e-3)).round() as usize).clamp(1, 120) } else { 0 };
    let mut centres: Vec<([f64; 2], f64)> = Vec::new();
    for s in 0..=steps {
        let t = ctx.time - Tick::from_seconds(span * s as f64 / steps.max(1) as f64);
        if t < Tick::ZERO {
            break;
        }
        let q = at(t);
        for i in 0..beads {
            let h = |k: u64| crate::noise::unit(crate::noise::mix(&[ctx.seed, i as u64, k, s as u64]));
            let (a, d) = (h(1) * std::f64::consts::TAU, if i == 0 { 0.0 } else { h(2) * rad * 0.35 });
            centres.push(([q[0] + a.cos() * d, q[1] + a.sin() * d], rad * (0.75 + 0.25 * h(3))));
        }
    }
    let src = b.img.clone();
    let light = [refl.cos() * 0.6, -refl.sin() * 0.6, 0.53];
    map_px(&mut b.img, |x, y, p| {
        let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
        // the glue's height: the highest of the beads' domes
        let mut best = (0f64, [0f64; 2], 1f64);
        for &(c, r) in &centres {
            let (dx, dy) = ((fx - c[0]) / r, (fy - c[1]) / r);
            let d2 = dx * dx + dy * dy;
            if d2 < 1.0 && (1.0 - d2).sqrt() > best.0 {
                best = ((1.0 - d2).sqrt(), [dx, dy], r);
            }
        }
        if best.0 <= 0.0 {
            return if glue_only { [0.0; 4] } else { p };
        }
        let n = [best.1[0], best.1[1], best.0];
        let lit = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).max(0.0);
        let shine = lit.powi(24) as f32;
        // seen through the bead
        let s = src.sample(Sampling::Bilinear, (fx - n[0] * best.2 * 0.5 * strength as f64).clamp(0.5, src.width as f64 - 0.5), (fy - n[1] * best.2 * 0.5 * strength as f64).clamp(0.5, src.height as f64 - 0.5));
        let edge = { let t = (best.0 as f32 / 0.15).clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) };
        let shade = 0.65 + 0.35 * lit as f32;
        let a = s[3].max(0.6);
        let g = [(s[0] * shade + shine).min(a + shine), (s[1] * shade + shine).min(a + shine), (s[2] * shade + shine).min(a + shine), a.min(1.0)];
        let base = if glue_only { [0.0; 4] } else { p };
        let k = edge * strength.clamp(0.0, 1.0).max(0.3);
        let mut o = [0, 1, 2, 3].map(|i| base[i] + (g[i] - base[i]) * k);
        o[3] = o[3].clamp(0.0, 1.0);
        o
    });
    b
}

effect!(
    THREADS_DEF,
    "threads",
    "Threads",
    "weaves the picture into threads, crossing over and under",
    [
        px("width", "Width", 50.0, 1000.0),
        px("height", "Height", 50.0, 1000.0),
        ranged("overlaps", "Overlaps", 1.0, 1.0, 20.0),
        angle("direction", "Direction", 0.0),
        point("center", "Center"),
        pct("coverage", "Coverage", 80.0, 0.0, 100.0),
        pct("shadowing", "Shadowing", 50.0, 0.0, 100.0),
        pct("texture", "Texture", 10.0, 0.0, 100.0),
    ]
);
pub const THREADS: Spec = Spec::new(&THREADS_DEF, CATEGORY, threads).points(&[("center", [0.5, 0.5])]);

fn threads(ctx: &Ctx, mut b: Buf) -> Buf {
    let sc = b.scale;
    let (tw, th) = ((ctx.num("width", 50.0) * sc).max(1.0), (ctx.num("height", 50.0) * sc).max(1.0));
    let overlaps = ctx.num("overlaps", 1.0).round().clamp(1.0, 20.0) as i64;
    let (sn, cs) = ctx.num("direction", 0.0).to_radians().sin_cos();
    let c = b.pixel(ctx.point("center"));
    let coverage = (ctx.num("coverage", 80.0) / 100.0).clamp(0.0, 1.0);
    let shadow = ctx.num("shadowing", 50.0) as f32 / 100.0;
    let texture = ctx.num("texture", 10.0) as f32 / 100.0;
    let src = b.img.clone();
    let to_px = |u: f64, v: f64| (c.x + u * cs - v * sn, c.y + u * sn + v * cs);
    map_px(&mut b.img, |x, y, _| {
        let (dx, dy) = (x as f64 + 0.5 - c.x, y as f64 + 0.5 - c.y);
        let (u, v) = (dx * cs + dy * sn, -dx * sn + dy * cs);
        let (ci, cj) = ((u / tw).floor() as i64, (v / th).floor() as i64);
        let (fu, fv) = (u / tw - ci as f64, v / th - cj as f64);
        let (in_h, in_v) = ((fv - 0.5).abs() < coverage * 0.5, (fu - 0.5).abs() < coverage * 0.5);
        // a twill: the across thread is on top for `overlaps` cells, then under for as many
        let h_top = (ci + cj).rem_euclid(2 * overlaps) < overlaps;
        let horizontal = match (in_h, in_v) {
            (true, true) => h_top,
            (true, false) => true,
            (false, true) => false,
            _ => return [0.0; 4],
        };
        // the thread's colour from the picture along its middle
        let (sx, sy) = if horizontal { to_px(u, (cj as f64 + 0.5) * th) } else { to_px((ci as f64 + 0.5) * tw, v) };
        let s = src.sample(Sampling::Bilinear, sx.clamp(0.5, src.width as f64 - 0.5), sy.clamp(0.5, src.height as f64 - 0.5));
        let across = if horizontal { (fv - 0.5) / (coverage * 0.5).max(1e-6) } else { (fu - 0.5) / (coverage * 0.5).max(1e-6) };
        let along = if horizontal { fu } else { fv };
        let round = (1.0 - across * across).max(0.0).sqrt() as f32;
        let dive = 1.0 - (along - 0.5).abs() as f32 * 2.0;
        let under = if horizontal == h_top { 0.0 } else { 1.0 };
        let shade = 1.0 - shadow * (1.0 - round) * 0.8 - shadow * 0.25 * (1.0 - dive) * under;
        let fibre = if texture > 0.0 { 1.0 - texture * 0.3 * crate::noise::unit(crate::noise::mix(&[(along * 64.0) as u64, (across * 8.0 + 8.0) as u64, ci as u64, cj as u64])) as f32 } else { 1.0 };
        let k = (shade * fibre).max(0.0);
        [s[0] * k, s[1] * k, s[2] * k, s[3]]
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 9] = [FILL, STROKE, SCRIBBLE, VEGAS, WRITE_ON, PAINT_BUCKET, EYEDROPPER, GLUE_GUN, THREADS];

#[cfg(test)]
mod tests;
