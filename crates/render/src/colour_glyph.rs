//! A colour glyph of paints (`COLR` version 1) drawn into a picture: the paint graph walked as
//! the font's rules say, every colour in linear light with its alpha multiplied in (gradients
//! between their stops so, and the ways of compositing and blending worked so), outlines
//! clipping what is painted inside them (their edges smooth), and the result turned back to
//! sRGB for the text to place like a glyph's own picture.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use veymelo_color::{linear_to_srgb, srgb_to_linear};
use veymelo_font::Font;
use veymelo_font::colr::{ColorGlyph, ColorLine, Extend, Ink, Paint, times};
use veymelo_path::{Path, Rule};
use veymelo_raster::Image;

/// The most pixels across or down a colour glyph's picture is drawn at.
const MOST: usize = 2048;

/// Linear-light RGBA, its alpha multiplied in.
type Px = [f32; 4];

/// A picture being painted: how many pixels across and down.
struct Canvas {
    w: usize,
    h: usize,
}

/// How much of each pixel a paint shows through (the outlines it is inside), and the box of
/// pixels it is not all nothing in.
#[derive(Clone)]
struct Clip {
    cover: Option<Arc<Vec<f32>>>,
    rect: [usize; 4],
}

fn linear(c: [f32; 4], alpha: f64) -> Px {
    let a = (c[3] as f64 * alpha).clamp(0.0, 1.0);
    let l = |v: f32| (srgb_to_linear(v as f64) * a) as f32;
    [l(c[0]), l(c[1]), l(c[2]), a as f32]
}

/// A colour line ready to read: its stops in linear light, and where it starts and ends.
struct Line {
    extend: Extend,
    stops: Vec<(f64, Px)>,
}

impl Line {
    /// The line, none when nothing is to be painted with it (stops all at one place, repeated).
    fn new(line: &ColorLine, text: [f32; 4]) -> Option<Line> {
        let stops: Vec<(f64, Px)> = line.stops.iter().map(|(o, ink)| (*o, ink_px(ink, text))).collect();
        let (first, last) = (stops.first().map_or(0.0, |s| s.0), stops.last().map_or(0.0, |s| s.0));
        if stops.len() > 1 && first == last && line.extend != Extend::Pad {
            return None;
        }
        Some(Line { extend: line.extend, stops })
    }

    fn at(&self, t: f64) -> Px {
        let s = &self.stops;
        match s.len() {
            0 => return [0.0; 4],
            1 => return s[0].1,
            _ => {}
        }
        let (a, b) = (s[0].0, s[s.len() - 1].0);
        let t = if b > a {
            let u = (t - a) / (b - a);
            let u = match self.extend {
                Extend::Pad => u,
                Extend::Repeat => u - u.floor(),
                Extend::Reflect => {
                    let v = u.rem_euclid(2.0);
                    if v > 1.0 { 2.0 - v } else { v }
                }
            };
            a + u * (b - a)
        } else {
            t
        };
        // the last stop at or before t, and the one after it (the first of those at one place
        // below it, the last at or above it)
        let i = s.partition_point(|x| x.0 <= t);
        if i == 0 {
            return s[0].1;
        }
        if i == s.len() {
            return s[i - 1].1;
        }
        let ((o0, c0), (o1, c1)) = (s[i - 1], s[i]);
        let f = ((t - o0) / (o1 - o0)) as f32;
        std::array::from_fn(|k| c0[k] + (c1[k] - c0[k]) * f)
    }
}

fn ink_px(ink: &Ink, text: [f32; 4]) -> Px {
    linear(ink.colour.unwrap_or(text), ink.alpha)
}

/// The inverse of `m` (none when it flattens everything to a line).
fn inverse(m: [f64; 6]) -> Option<[f64; 6]> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-12 {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(a * m[4] + c * m[5]), -(b * m[4] + d * m[5])])
}

/// Where a gradient is at a point of its own space (none: it paints nothing there).
type Place = Box<dyn Fn(f64, f64) -> Option<f64>>;

fn linear_place(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2]) -> Option<Place> {
    // p1 moved along p0–p2 until the line to it crosses the bands of colour square on
    let (v, r) = ([p1[0] - p0[0], p1[1] - p0[1]], [p2[0] - p0[0], p2[1] - p0[1]]);
    let n = [-r[1], r[0]];
    let nn = n[0] * n[0] + n[1] * n[1];
    let vn = v[0] * n[0] + v[1] * n[1];
    if v == [0.0, 0.0] || nn < 1e-12 || (vn * vn) < 1e-9 * nn * (v[0] * v[0] + v[1] * v[1]) {
        return None;
    }
    let along = [n[0] * vn / nn, n[1] * vn / nn];
    let len = along[0] * along[0] + along[1] * along[1];
    Some(Box::new(move |x, y| Some(((x - p0[0]) * along[0] + (y - p0[1]) * along[1]) / len)))
}

fn radial_place(c0: [f64; 2], r0: f64, c1: [f64; 2], r1: f64) -> Option<Place> {
    if c0 == c1 && r0 == r1 {
        return None;
    }
    let (cd, dr) = ([c1[0] - c0[0], c1[1] - c0[1]], r1 - r0);
    let a = cd[0] * cd[0] + cd[1] * cd[1] - dr * dr;
    // the circle furthest along the line that passes through the point, with a radius
    Some(Box::new(move |x, y| {
        let pd = [x - c0[0], y - c0[1]];
        let b = pd[0] * cd[0] + pd[1] * cd[1] + r0 * dr;
        let c = pd[0] * pd[0] + pd[1] * pd[1] - r0 * r0;
        let ok = |w: f64| (r0 + w * dr >= 0.0).then_some(w);
        if a.abs() < 1e-9 {
            return if b.abs() < 1e-12 { None } else { ok(c / (2.0 * b)) };
        }
        let disc = b * b - a * c;
        if disc < 0.0 {
            return None;
        }
        let s = disc.sqrt();
        let (w1, w2) = ((b + s) / a, (b - s) / a);
        let (hi, lo) = if w1 > w2 { (w1, w2) } else { (w2, w1) };
        ok(hi).or_else(|| ok(lo))
    }))
}

fn sweep_place(centre: [f64; 2], start: f64, end: f64, extend: Extend) -> Option<Place> {
    if start == end && extend != Extend::Pad {
        return None;
    }
    Some(Box::new(move |x, y| {
        let deg = (y - centre[1]).atan2(x - centre[0]).to_degrees().rem_euclid(360.0);
        Some(if start == end { if deg < start { f64::NEG_INFINITY } else { f64::INFINITY } } else { (deg - start) / (end - start) })
    }))
}

impl Canvas {
    fn full(&self) -> Clip {
        Clip { cover: None, rect: [0, 0, self.w, self.h] }
    }

    fn blank(&self) -> Vec<Px> {
        vec![[0.0; 4]; self.w * self.h]
    }

    /// `p` painted over `onto` (through `clip`), its space taken to the picture's pixels by `m`.
    fn draw(&self, font: &Font, p: &Paint, m: [f64; 6], clip: &Clip, text: [f32; 4], onto: &mut [Px]) {
        let [x0, y0, x1, y1] = clip.rect;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let over = |onto: &mut [Px], i: usize, c: Px| {
            let k = clip.cover.as_ref().map_or(1.0, |v| v[i]);
            if k > 0.0 {
                let keep = 1.0 - c[3] * k;
                for j in 0..4 {
                    onto[i][j] = c[j] * k + onto[i][j] * keep;
                }
            }
        };
        let gradient = |onto: &mut [Px], line: &ColorLine, place: Option<Place>| {
            let (Some(line), Some(place), Some(back)) = (Line::new(line, text), place, inverse(m)) else { return };
            for y in y0..y1 {
                for x in x0..x1 {
                    let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                    let (u, v) = (back[0] * px + back[2] * py + back[4], back[1] * px + back[3] * py + back[5]);
                    if let Some(t) = place(u, v) {
                        over(onto, y * self.w + x, line.at(t));
                    }
                }
            }
        };
        match p {
            Paint::Nothing => {}
            Paint::Layers(kids) => {
                for k in kids {
                    self.draw(font, k, m, clip, text, onto);
                }
            }
            Paint::Solid(ink) => {
                let c = ink_px(ink, text);
                for y in y0..y1 {
                    for x in x0..x1 {
                        over(onto, y * self.w + x, c);
                    }
                }
            }
            Paint::Linear { line, p0, p1, p2 } => gradient(onto, line, linear_place(*p0, *p1, *p2)),
            Paint::Radial { line, c0, r0, c1, r1 } => gradient(onto, line, radial_place(*c0, *r0, *c1, *r1)),
            Paint::Sweep { line, centre, start, end } => gradient(onto, line, sweep_place(*centre, *start, *end, line.extend)),
            Paint::Glyph { glyph, paint } => {
                let mut path = Path::default();
                font.outline(*glyph, &mut path, &|x, y| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]));
                let Some(mask) = veymelo_path::fill(&path, Rule::NonZero, self.w, self.h) else { return };
                let rect = [x0.max(mask.left), y0.max(mask.top), x1.min(mask.left + mask.width), y1.min(mask.top + mask.height)];
                if rect[0] >= rect[2] || rect[1] >= rect[3] {
                    return;
                }
                let mut cover = vec![0.0f32; self.w * self.h];
                for y in rect[1]..rect[3] {
                    for x in rect[0]..rect[2] {
                        let i = y * self.w + x;
                        cover[i] = mask.at(x, y) * clip.cover.as_ref().map_or(1.0, |v| v[i]);
                    }
                }
                self.draw(font, paint, m, &Clip { cover: Some(Arc::new(cover)), rect }, text, onto);
            }
            Paint::Transform { m: t, paint } => self.draw(font, paint, times(m, *t), clip, text, onto),
            Paint::Composite { source, mode, backdrop } => {
                // each painted by itself, put together, then over what is there through the clip
                let open = Clip { cover: None, rect: clip.rect };
                let (mut s, mut b) = (self.blank(), self.blank());
                self.draw(font, backdrop, m, &open, text, &mut b);
                self.draw(font, source, m, &open, text, &mut s);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let i = y * self.w + x;
                        over(onto, i, composite(*mode, s[i], b[i]));
                    }
                }
            }
        }
    }
}

/// Source `s` put on backdrop `b` the way COLR's `mode` says (Compositing and Blending Level 1).
pub(crate) fn composite(mode: u8, s: Px, b: Px) -> Px {
    let (sa, ba) = (s[3], b[3]);
    let duff = |fa: f32, fb: f32| -> Px { std::array::from_fn(|k| (s[k] * fa + b[k] * fb).min(1.0)) };
    match mode {
        0 => [0.0; 4],
        1 => s,
        2 => b,
        3 => duff(1.0, 1.0 - sa),
        4 => duff(1.0 - ba, 1.0),
        5 => duff(ba, 0.0),
        6 => duff(0.0, sa),
        7 => duff(1.0 - ba, 0.0),
        8 => duff(0.0, 1.0 - sa),
        9 => duff(ba, 1.0 - sa),
        10 => duff(1.0 - ba, sa),
        11 => duff(1.0 - ba, 1.0 - sa),
        12 => duff(1.0, 1.0),
        // the blend modes, as layers blend (the same W3C formulas)
        13..=27 => {
            use veymelo_color::BlendMode as B;
            const MODES: [B; 15] = [B::Screen, B::Overlay, B::Darken, B::Lighten, B::ColorDodge, B::ColorBurn, B::HardLight, B::SoftLight, B::Difference, B::Exclusion, B::Multiply, B::Hue, B::Saturation, B::Color, B::Luminosity];
            veymelo_color::blend(MODES[mode as usize - 13], b, s, 0.0)
        }
        _ => [0.0; 4],
    }
}

/// What a drawn colour glyph is kept by: its font's instance, the glyph, its pixels to an em and
/// the text's colour.
type Drawn = (u64, u16, u64, [u32; 4]);

/// Colour glyph `glyph` of `font` drawn at `ppem` pixels to an em, `text` (straight sRGB RGBA)
/// its own colour: its picture (premultiplied sRGB), and where the picture's bottom left is from
/// the glyph's origin in its pixels (y up). None when it paints nothing.
pub(crate) fn draw(font: &Font, glyph: u16, g: &ColorGlyph, ppem: f64, text: [f32; 4]) -> Option<(Arc<Image>, (f64, f64), f64)> {
    let [bx0, by0, bx1, by1] = g.bounds;
    if !(bx1 > bx0 && by1 > by0) || !bx0.is_finite() || !by1.is_finite() {
        return None;
    }
    // no bigger than MOST across or down
    let ppem = ppem.max(1.0).min(MOST as f64 * font.units / (bx1 - bx0).max(by1 - by0));
    let k = ppem / font.units;
    let (px0, py0, px1, py1) = ((bx0 * k).floor(), (by0 * k).floor(), (bx1 * k).ceil(), (by1 * k).ceil());
    let (w, h) = ((px1 - px0) as usize, (py1 - py0) as usize);
    if w == 0 || h == 0 {
        return None;
    }
    static KEPT: OnceLock<Mutex<HashMap<Drawn, Arc<Image>>>> = OnceLock::new();
    let key = (font.instance_key(), glyph, ppem.to_bits(), text.map(f32::to_bits));
    let kept = KEPT.get_or_init(Default::default);
    if let Some(p) = kept.lock().ok().and_then(|k| k.get(&key).cloned()) {
        return Some((p, (px0, py0), ppem));
    }
    let canvas = Canvas { w, h };
    let mut px = canvas.blank();
    // font units (y up) to the picture's pixels (y down from its top)
    let m = [k, 0.0, 0.0, -k, -px0, py1];
    canvas.draw(font, &g.paint, m, &canvas.full(), text, &mut px);
    // back to sRGB, its alpha multiplied in again
    let px: Vec<[f32; 4]> = px
        .into_iter()
        .map(|c| {
            let a = c[3].clamp(0.0, 1.0);
            if a <= 0.0 {
                return [0.0; 4];
            }
            let e = |v: f32| (linear_to_srgb((v / a).clamp(0.0, 1.0) as f64) as f32) * a;
            [e(c[0]), e(c[1]), e(c[2]), a]
        })
        .collect();
    let img = Arc::new(Image { width: w, height: h, px: px.into() });
    if let Ok(mut k) = kept.lock() {
        if k.len() > 4096 {
            k.clear();
        }
        k.insert(key, img.clone());
    }
    Some((img, (px0, py0), ppem))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

    fn line(extend: Extend, stops: &[(f64, [f32; 4])]) -> ColorLine {
        ColorLine { extend, stops: stops.iter().map(|&(o, c)| (o, Ink { colour: Some(c), alpha: 1.0 })).collect() }
    }

    fn near(a: Px, b: Px) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn a_colour_line_carries_on_as_it_says() {
        let stops = [(0.0, RED), (1.0, BLUE)];
        let pad = Line::new(&line(Extend::Pad, &stops), RED).unwrap();
        assert!(near(pad.at(-1.0), RED) && near(pad.at(2.0), BLUE));
        assert!(near(pad.at(0.25), [0.75, 0.0, 0.25, 1.0]));
        let repeat = Line::new(&line(Extend::Repeat, &stops), RED).unwrap();
        assert!(near(repeat.at(1.25), pad.at(0.25)) && near(repeat.at(-0.75), pad.at(0.25)));
        let reflect = Line::new(&line(Extend::Reflect, &stops), RED).unwrap();
        assert!(near(reflect.at(1.25), pad.at(0.75)) && near(reflect.at(-0.25), pad.at(0.25)));
        // two stops at one place: the first below it, the last at it and above
        let sharp = Line::new(&line(Extend::Pad, &[(0.0, RED), (0.5, RED), (0.5, BLUE), (1.0, BLUE)]), RED).unwrap();
        assert!(near(sharp.at(0.49), RED) && near(sharp.at(0.5), BLUE));
        let all_at_one = [(0.5, RED), (0.5, BLUE)];
        assert!(near(Line::new(&line(Extend::Pad, &all_at_one), RED).unwrap().at(0.2), RED));
        assert!(Line::new(&line(Extend::Repeat, &all_at_one), RED).is_none());
        // colours are mixed in linear light with their alpha in, the text's own where none
        let half = Line::new(&ColorLine { extend: Extend::Pad, stops: vec![(0.0, Ink { colour: Some([0.5, 0.5, 0.5, 1.0]), alpha: 0.5 }), (1.0, Ink { colour: None, alpha: 1.0 })] }, BLUE).unwrap();
        let g = srgb_to_linear(0.5) as f32 * 0.5;
        assert!(near(half.at(0.0), [g, g, g, 0.5]) && near(half.at(1.0), BLUE));
    }

    #[test]
    fn gradients_place_each_point() {
        // linear: along x, the bands turned to lie along p0–p2
        let lin = linear_place([0.0, 0.0], [100.0, 0.0], [0.0, 50.0]).unwrap();
        assert_eq!(lin(25.0, 70.0), Some(0.25));
        let slanted = linear_place([0.0, 0.0], [100.0, 0.0], [100.0, 100.0]).unwrap();
        // p1 at 1, and on along each band: (50, 0) halfway, (100, 100) on p0's band
        assert!((slanted(100.0, 0.0).unwrap() - 1.0).abs() < 1e-9 && (slanted(50.0, 0.0).unwrap() - 0.5).abs() < 1e-9);
        assert!(slanted(100.0, 100.0).unwrap().abs() < 1e-9 && (slanted(150.0, 50.0).unwrap() - 1.0).abs() < 1e-9);
        assert!(linear_place([0.0, 0.0], [100.0, 0.0], [50.0, 0.0]).is_none());
        // radial between circles about one centre, inside the inner one before 0
        let rings = radial_place([0.0, 0.0], 10.0, [0.0, 0.0], 50.0).unwrap();
        assert!((rings(30.0, 0.0).unwrap() - 0.5).abs() < 1e-9 && (rings(0.0, 5.0).unwrap() + 0.125).abs() < 1e-9);
        // a cone from a point: the furthest circle through a point wins, nothing behind it
        let cone = radial_place([0.0, 0.0], 0.0, [100.0, 0.0], 50.0).unwrap();
        assert!((cone(200.0, 0.0).unwrap() - 4.0).abs() < 1e-9);
        assert_eq!(cone(-50.0, 0.0), None);
        assert!(radial_place([1.0, 1.0], 5.0, [1.0, 1.0], 5.0).is_none());
        // sweep: counter-clockwise from x, either way round
        let sweep = sweep_place([0.0, 0.0], 0.0, 90.0, Extend::Pad).unwrap();
        assert!((sweep(1.0, 1.0).unwrap() - 0.5).abs() < 1e-9 && (sweep(0.0, -1.0).unwrap() - 3.0).abs() < 1e-9);
        let back = sweep_place([0.0, 0.0], 90.0, 0.0, Extend::Pad).unwrap();
        assert!((back(1.0, 1.0).unwrap() - 0.5).abs() < 1e-9 && (back(0.0, 1.0).unwrap()).abs() < 1e-9);
        assert!(sweep_place([0.0, 0.0], 30.0, 30.0, Extend::Repeat).is_none());
    }

    #[test]
    fn paintings_are_put_together_by_each_mode() {
        let half_red: Px = [0.5, 0.0, 0.0, 0.5];
        let blue: Px = BLUE;
        assert_eq!(composite(0, half_red, blue), [0.0; 4]);
        assert_eq!(composite(3, half_red, blue), [0.5, 0.0, 0.5, 1.0]);
        assert_eq!(composite(4, half_red, blue), blue);
        assert_eq!(composite(5, half_red, [0.0, 0.0, 0.5, 0.5]), [0.25, 0.0, 0.0, 0.25]);
        assert_eq!(composite(8, half_red, blue), [0.0, 0.0, 0.5, 0.5]);
        assert_eq!(composite(11, RED, blue), [0.0; 4]);
        assert_eq!(composite(12, RED, [0.5, 0.0, 1.0, 1.0]), [1.0, 0.0, 1.0, 1.0]);
        // blends where both are whole: the blend itself
        let grey: Px = [0.5, 0.5, 0.5, 1.0];
        assert!(near(composite(23, RED, grey), [0.5, 0.0, 0.0, 1.0]));
        assert!(near(composite(13, RED, grey), [1.0, 0.5, 0.5, 1.0]));
        assert!(near(composite(21, RED, grey), [0.5, 0.5, 0.5, 1.0]));
        assert!(near(composite(15, RED, grey), [0.5, 0.0, 0.0, 1.0]));
        // a hue's colour at the backdrop's lightness and colourfulness
        let out = composite(24, BLUE, [0.8, 0.2, 0.2, 1.0]);
        let lum = |c: [f32; 3]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
        assert!(out[2] > out[0] && out[2] > out[1] && (lum([out[0], out[1], out[2]]) - lum([0.8, 0.2, 0.2])).abs() < 1e-4, "{out:?}");
        // a blend over nothing is the source as it is
        assert!(near(composite(23, half_red, [0.0; 4]), half_red));
    }
}
