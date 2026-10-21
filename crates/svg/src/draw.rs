//! An SVG picture drawn at any size: each shape's outline filled by its rule and stroked (its
//! width, caps, joins, miter limit and dashes) in its paint (a colour, or a gradient worked out
//! pixel by pixel), a group or shape that fades with more than one thing in it drawn on its own
//! first so its parts do not show through each other.
//!
//! A group cut to outlines, seen through a mask or blended is drawn on its own too, only over
//! the part its outlines leave (each outline's edge smooth), then laid on what is under it.
//! Pictures are laid on through their transforms; a mesh's triangles are shaded between their
//! corners, each pixel's middle in one triangle only (so shared edges do not show).

use veymelo_color::{BlendMode, blend};
use veymelo_geom::{Mat3, vec2};
use veymelo_path::{Path, Rule, Sub};
use veymelo_raster::{Image, Px, Sampling};

use crate::{Doc, GradientKind, Group, Mesh, Node, Paint, Picture, Shape, Spread};

/// `doc` drawn onto a picture `w` × `h` through `view` (from the picture's pixels to these),
/// `color` taking a straight sRGB colour and alpha to a pixel.
pub fn draw(doc: &Doc, view: Mat3, w: usize, h: usize, color: &dyn Fn([f64; 4]) -> Px) -> Image {
    let mut img = Image::new(w, h);
    group(&doc.root, view, 1.0, &mut img, color);
    img
}

/// `top` laid over `under`, faded to `opacity`.
fn over(under: &mut Image, top: &Image, opacity: f32) {
    for (d, s) in under.px.iter_mut().zip(&top.px) {
        if s[3] > 0.0 {
            let k = 1.0 - s[3] * opacity;
            *d = std::array::from_fn(|c| s[c] * opacity + d[c] * k);
        }
    }
}

/// Straight sRGB as it is, its alpha multiplied in (for masks, which are measured in it).
fn plain(c: [f64; 4]) -> Px {
    [(c[0] * c[3]) as f32, (c[1] * c[3]) as f32, (c[2] * c[3]) as f32, c[3] as f32]
}

fn group(g: &Group, m: Mat3, opacity: f64, img: &mut Image, color: &dyn Fn([f64; 4]) -> Px) {
    let (m, o) = (m * g.transform, opacity * g.opacity);
    if o <= 0.0 {
        return;
    }
    let alone = !g.clip.is_empty() || g.mask.is_some() || g.blend != BlendMode::Normal;
    if !alone {
        if o < 1.0 && g.children.len() > 1 {
            let mut own = Image::new(img.width, img.height);
            for c in &g.children {
                node(c, m, 1.0, &mut own, color);
            }
            over(img, &own, o as f32);
        } else {
            for c in &g.children {
                node(c, m, o, img, color);
            }
        }
        return;
    }
    // the part its outlines leave, and how much of each pixel there
    let mut rect = [0, 0, img.width, img.height];
    let mut masks = Vec::with_capacity(g.clip.len());
    for c in &g.clip {
        let flat = veymelo_path::shape::flatten(&c.subs.iter().map(|sub| sub.map(&|p| {
            let q = m.apply(vec2(p[0], p[1]));
            [q.x, q.y]
        })).collect::<Vec<Sub>>());
        let Some(k) = veymelo_path::fill(&flat, if c.even_odd { Rule::EvenOdd } else { Rule::NonZero }, img.width, img.height) else { return };
        rect = [rect[0].max(k.left), rect[1].max(k.top), rect[2].min(k.left + k.width), rect[3].min(k.top + k.height)];
        masks.push(k);
    }
    let [x0, y0, x1, y1] = rect;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let shift = Mat3::translate(vec2(-(x0 as f64), -(y0 as f64))) * m;
    let mut own = Image::new(w, h);
    for c in &g.children {
        node(c, shift, 1.0, &mut own, color);
    }
    // how much shows through the mask: its brightness over its backdrop, or its alpha
    let seen: Option<Vec<f32>> = g.mask.as_ref().map(|mask| {
        let mut mi = Image::new(w, h);
        if mask.luminosity {
            let b = plain([mask.backdrop[0], mask.backdrop[1], mask.backdrop[2], 1.0]);
            mi.px.iter_mut().for_each(|q| *q = b);
        }
        group(&mask.content, shift, 1.0, &mut mi, &plain);
        mi.px
            .iter()
            .map(|q| {
                let v = if mask.luminosity { 0.3 * q[0] + 0.59 * q[1] + 0.11 * q[2] } else { q[3] };
                let v = v.clamp(0.0, 1.0);
                match &mask.transfer {
                    Some(t) if !t.is_empty() => t[((v * (t.len() - 1) as f32).round() as usize).min(t.len() - 1)] as f32,
                    _ => v,
                }
            })
            .collect()
    });
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut k = o as f32;
            for c in &masks {
                k *= c.at(x0 + x, y0 + y);
            }
            if let Some(s) = &seen {
                k *= s[i];
            }
            let s = own.px[i];
            if k <= 0.0 || s[3] <= 0.0 {
                continue;
            }
            let d = &mut img.px[(y0 + y) * img.width + x0 + x];
            *d = blend(g.blend, *d, s.map(|v| v * k), 0.0);
        }
    }
}

fn node(n: &Node, m: Mat3, opacity: f64, img: &mut Image, color: &dyn Fn([f64; 4]) -> Px) {
    match n {
        Node::Group(g) => group(g, m, opacity, img, color),
        Node::Shape(s) => shape(s, m, opacity, img, color),
        Node::Picture(p) => picture(p, m, opacity, img, color),
        Node::Mesh(mesh) => shaded(mesh, m, opacity, img, color),
    }
}

/// A path's points moved by `m`.
fn mapped(p: &Path, m: &Mat3) -> Path {
    let mut out = Path::default();
    for c in p.contours() {
        for (k, &(x, y)) in c.iter().enumerate() {
            let q = m.apply(vec2(x, y));
            if k == 0 {
                out.move_to(q.x, q.y);
            } else {
                out.line_to(q.x, q.y);
            }
        }
    }
    out
}

fn shape(s: &Shape, m: Mat3, opacity: f64, img: &mut Image, color: &dyn Fn([f64; 4]) -> Px) {
    let (m, o) = (m * s.transform, opacity * s.opacity);
    if o <= 0.0 {
        return;
    }
    if (o < 1.0 && s.fill.is_some() && s.stroke.is_some()) || (s.blend != BlendMode::Normal && s.fill.is_some() && s.stroke.is_some()) {
        let mut own = Image::new(img.width, img.height);
        shape(&Shape { opacity: 1.0, transform: Mat3::IDENTITY, blend: BlendMode::Normal, ..s.clone() }, m, 1.0, &mut own, color);
        if s.blend == BlendMode::Normal {
            over(img, &own, o as f32);
        } else {
            for (d, q) in img.px.iter_mut().zip(&own.px) {
                if q[3] > 0.0 {
                    *d = blend(s.blend, *d, q.map(|v| v * o as f32), 0.0);
                }
            }
        }
        return;
    }
    let subs = s.geom.subs();
    let bounds = s.geom.bounds().unwrap_or([0.0, 0.0, 1.0, 1.0]);
    let Some(unview) = m.inverse() else { return };
    if let Some(fill) = &s.fill {
        let flat = veymelo_path::shape::flatten(&subs.iter().map(|sub| sub.map(&|p| {
            let q = m.apply(vec2(p[0], p[1]));
            [q.x, q.y]
        })).collect::<Vec<Sub>>());
        let rule = if s.even_odd { Rule::EvenOdd } else { Rule::NonZero };
        paint(img, &flat, rule, fill, s.fill_opacity * o, &unview, bounds, s.blend, color);
    }
    if let Some(st) = &s.stroke {
        // stroked at about the size it is drawn (so its curves are as smooth), then laid on
        let k = m.mean_scale().max(1e-9);
        let scaled: Vec<Sub> = subs.iter().map(|sub| sub.map(&|p| [p[0] * k, p[1] * k])).collect();
        let outline = veymelo_path::stroke(
            &scaled,
            &veymelo_path::Stroke { width: st.width * k, cap: st.cap, join: st.join, miter_limit: st.miter, dashes: st.dashes.iter().map(|d| d * k).collect(), dash_offset: st.dash_offset * k, ..veymelo_path::Stroke::default() },
        );
        let laid = mapped(&outline, &(m * Mat3::scale(vec2(1.0 / k, 1.0 / k))));
        paint(img, &laid, Rule::NonZero, &st.paint, st.opacity * o, &unview, bounds, s.blend, color);
    }
}

/// A picture laid on through its transform, its colours taken as `color` takes them.
fn picture(p: &Picture, m: Mat3, opacity: f64, img: &mut Image, color: &dyn Fn([f64; 4]) -> Px) {
    let o = opacity * p.opacity;
    if o <= 0.0 || p.image.width == 0 || p.image.height == 0 {
        return;
    }
    let mut work = Image::new(p.image.width, p.image.height);
    for (d, q) in work.px.iter_mut().zip(p.image.px.iter()) {
        if q[3] > 0.0 {
            let a = q[3] as f64;
            *d = color([q[0] as f64 / a, q[1] as f64 / a, q[2] as f64 / a, a]);
        }
    }
    let how = if p.smooth { Sampling::Bilinear } else { Sampling::Nearest };
    veymelo_raster::place(img, &work, &(m * p.transform), how, p.blend, o as f32, 0);
}

/// A mesh's triangles, each pixel whose middle is inside one shaded between its corners.
fn shaded(mesh: &Mesh, m: Mat3, opacity: f64, img: &mut Image, color: &dyn Fn([f64; 4]) -> Px) {
    let (m, o) = (m * mesh.transform, opacity * mesh.opacity);
    if o <= 0.0 {
        return;
    }
    let (w, h) = (img.width, img.height);
    for t in &mesh.triangles {
        let p: [[f64; 2]; 3] = t.map(|(q, _)| {
            let q = m.apply(vec2(q[0], q[1]));
            [q.x, q.y]
        });
        let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
        if area.abs() < 1e-12 {
            continue;
        }
        // each edge's side, taken the same way round for both turnings; a pixel on an edge goes to
        // the triangle the edge is the top or left of
        let s = area.signum();
        let edge = |a: [f64; 2], b: [f64; 2], x: f64, y: f64| s * ((b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]));
        let top_left = |a: [f64; 2], b: [f64; 2]| {
            let (dx, dy) = (s * (b[0] - a[0]), s * (b[1] - a[1]));
            (dy == 0.0 && dx < 0.0) || dy > 0.0
        };
        let tl = [top_left(p[1], p[2]), top_left(p[2], p[0]), top_left(p[0], p[1])];
        let lo = |k: usize| p.iter().map(|q| q[k]).fold(f64::INFINITY, f64::min);
        let hi = |k: usize| p.iter().map(|q| q[k]).fold(f64::NEG_INFINITY, f64::max);
        let (xa, xb) = ((lo(0) - 0.5).floor().max(0.0) as usize, ((hi(0) - 0.5).ceil().max(0.0) as usize + 1).min(w));
        let (ya, yb) = ((lo(1) - 0.5).floor().max(0.0) as usize, ((hi(1) - 0.5).ceil().max(0.0) as usize + 1).min(h));
        for y in ya..yb {
            for x in xa..xb {
                let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
                let e = [edge(p[1], p[2], cx, cy), edge(p[2], p[0], cx, cy), edge(p[0], p[1], cx, cy)];
                if (0..3).any(|k| e[k] < 0.0 || (e[k] == 0.0 && !tl[k])) {
                    continue;
                }
                let sum = e[0] + e[1] + e[2];
                let c: [f64; 4] = std::array::from_fn(|k| (t[0].1[k] * e[0] + t[1].1[k] * e[1] + t[2].1[k] * e[2]) / sum);
                let q = color([c[0], c[1], c[2], c[3] * o]);
                let d = &mut img.px[y * w + x];
                *d = blend(mesh.blend, *d, q, 0.0);
            }
        }
    }
}

/// Where along a gradient between two circles a point is: the furthest circle through it that
/// has a radius (none: no circle passes through it).
fn on_circles(q: [f64; 2], c0: [f64; 2], r0: f64, c1: [f64; 2], r1: f64) -> Option<f64> {
    let (cd, dr) = ([c1[0] - c0[0], c1[1] - c0[1]], r1 - r0);
    let a = cd[0] * cd[0] + cd[1] * cd[1] - dr * dr;
    let pd = [q[0] - c0[0], q[1] - c0[1]];
    let b = pd[0] * cd[0] + pd[1] * cd[1] + r0 * dr;
    let c = pd[0] * pd[0] + pd[1] * pd[1] - r0 * r0;
    let ok = |t: f64| (r0 + t * dr >= 0.0).then_some(t);
    if a.abs() < 1e-9 {
        return if b.abs() < 1e-12 { None } else { ok(c / (2.0 * b)) };
    }
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    let (t1, t2) = ((b + s) / a, (b - s) / a);
    let (hi, lo) = if t1 > t2 { (t1, t2) } else { (t2, t1) };
    ok(hi).or_else(|| ok(lo))
}

/// The inside of `path` (by `rule`) painted with `p` at `opacity`, mixed by `mode`; `unview`
/// takes the picture's pixels back to the shape's units, whose box is `bounds` (for gradients in
/// its box's units).
#[allow(clippy::too_many_arguments)]
fn paint(img: &mut Image, path: &Path, rule: Rule, p: &Paint, opacity: f64, unview: &Mat3, bounds: [f64; 4], mode: BlendMode, color: &dyn Fn([f64; 4]) -> Px) {
    if opacity <= 0.0 {
        return;
    }
    let Some(cover) = veymelo_path::fill(path, rule, img.width, img.height) else { return };
    let w = img.width;
    let solid = match p {
        Paint::Color(c) => Some(color([c[0], c[1], c[2], opacity])),
        Paint::Gradient(_) => None,
    };
    // a gradient: each pixel's place along it, from the shape's units (its box's, when it says)
    let along = |x: usize, y: usize| -> Px {
        let Paint::Gradient(g) = p else { return [0.0; 4] };
        let mut q = unview.apply(vec2(x as f64 + 0.5, y as f64 + 0.5));
        if g.box_units {
            let (bw, bh) = ((bounds[2] - bounds[0]).max(1e-12), (bounds[3] - bounds[1]).max(1e-12));
            q = vec2((q.x - bounds[0]) / bw, (q.y - bounds[1]) / bh);
        }
        if let Some(inv) = g.transform.inverse() {
            q = inv.apply(q);
        }
        let t = match g.kind {
            GradientKind::Linear { x1, y1, x2, y2 } => {
                let (dx, dy) = (x2 - x1, y2 - y1);
                let len2 = dx * dx + dy * dy;
                if len2 < 1e-24 { 1.0 } else { ((q.x - x1) * dx + (q.y - y1) * dy) / len2 }
            }
            GradientKind::Radial { cx, cy, r, fx, fy } => {
                if r <= 1e-12 {
                    1.0
                } else {
                    // the focal point kept just inside the circle
                    let (mut ddx, mut ddy) = (cx - fx, cy - fy);
                    let reach = ddx.hypot(ddy);
                    let (fx, fy) = if reach > r * 0.999 {
                        let k = r * 0.999 / reach;
                        (ddx, ddy) = (ddx * k, ddy * k);
                        (cx - ddx, cy - ddy)
                    } else {
                        (fx, fy)
                    };
                    // the circle through the point, its centre on the line from the focal point
                    // to the centre, its radius as far along it: t with |e − t·d| = t·r
                    let (ex, ey) = (q.x - fx, q.y - fy);
                    let a = ddx * ddx + ddy * ddy - r * r;
                    let b = ex * ddx + ey * ddy;
                    let c = ex * ex + ey * ey;
                    (b - (b * b - a * c).max(0.0).sqrt()) / a
                }
            }
            GradientKind::Circles { x0, y0, r0, x1, y1, r1 } => match on_circles([q.x, q.y], [x0, y0], r0, [x1, y1], r1) {
                Some(t) => t,
                None => return [0.0; 4],
            },
        };
        if (t < 0.0 && !g.ends[0]) || (t > 1.0 && !g.ends[1]) {
            return [0.0; 4];
        }
        let t = match g.spread {
            Spread::Pad => t.clamp(0.0, 1.0),
            Spread::Repeat => t - t.floor(),
            Spread::Reflect => {
                let m = t.rem_euclid(2.0);
                if m > 1.0 { 2.0 - m } else { m }
            }
        };
        let stops = &g.stops;
        if stops.is_empty() {
            return [0.0; 4];
        }
        let c = match stops.iter().position(|s| s.0 >= t) {
            None => stops[stops.len() - 1].1,
            Some(0) => stops[0].1,
            Some(k) => {
                let (a, b) = (stops[k - 1], stops[k]);
                let f = if b.0 - a.0 < 1e-12 { 1.0 } else { (t - a.0) / (b.0 - a.0) };
                std::array::from_fn(|i| a.1[i] + (b.1[i] - a.1[i]) * f)
            }
        };
        color([c[0], c[1], c[2], c[3] * opacity])
    };
    for y in cover.top..cover.top + cover.height {
        for x in cover.left..cover.left + cover.width {
            let k = cover.at(x, y);
            if k <= 0.0 {
                continue;
            }
            let s = solid.unwrap_or_else(|| along(x, y));
            let d = &mut img.px[y * w + x];
            let a = s[3] * k;
            if a <= 0.0 {
                continue;
            }
            *d = if mode == BlendMode::Normal { std::array::from_fn(|c| s[c] * k + d[c] * (1.0 - a)) } else { blend(mode, *d, s.map(|v| v * k), 0.0) };
        }
    }
}
