//! An SVG picture drawn at any size: each shape's outline filled by its rule and stroked (its
//! width, caps, joins, miter limit and dashes) in its paint (a colour, or a gradient worked out
//! pixel by pixel), a group or shape that fades with more than one thing in it drawn on its own
//! first so its parts do not show through each other.

use veymelo_geom::{Mat3, vec2};
use veymelo_path::{Path, Rule, Sub};
use veymelo_raster::{Image, Px};

use crate::{Doc, GradientKind, Group, Node, Paint, Shape, Spread};

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

fn group(g: &Group, m: Mat3, opacity: f64, img: &mut Image, color: &dyn Fn([f64; 4]) -> Px) {
    let (m, o) = (m * g.transform, opacity * g.opacity);
    if o <= 0.0 {
        return;
    }
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
}

fn node(n: &Node, m: Mat3, opacity: f64, img: &mut Image, color: &dyn Fn([f64; 4]) -> Px) {
    match n {
        Node::Group(g) => group(g, m, opacity, img, color),
        Node::Shape(s) => shape(s, m, opacity, img, color),
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
    if o < 1.0 && s.fill.is_some() && s.stroke.is_some() {
        let mut own = Image::new(img.width, img.height);
        shape(&Shape { opacity: 1.0, transform: Mat3::IDENTITY, ..s.clone() }, m, 1.0, &mut own, color);
        over(img, &own, o as f32);
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
        paint(img, &flat, rule, fill, s.fill_opacity * o, &unview, bounds, color);
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
        paint(img, &laid, Rule::NonZero, &st.paint, st.opacity * o, &unview, bounds, color);
    }
}

/// The inside of `path` (by `rule`) painted with `p` at `opacity`; `unview` takes the picture's
/// pixels back to the shape's units, whose box is `bounds` (for gradients in its box's units).
#[allow(clippy::too_many_arguments)]
fn paint(img: &mut Image, path: &Path, rule: Rule, p: &Paint, opacity: f64, unview: &Mat3, bounds: [f64; 4], color: &dyn Fn([f64; 4]) -> Px) {
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
        };
        let t = match g.spread {
            Spread::Pad => t.clamp(0.0, 1.0),
            Spread::Repeat => t - t.floor(),
            Spread::Reflect => {
                let m = t.rem_euclid(2.0);
                if m > 1.0 { 2.0 - m } else { m }
            }
        };
        let stops = &g.stops;
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
            *d = std::array::from_fn(|c| s[c] * k + d[c] * (1.0 - a));
        }
    }
}
