//! Drawing a text layer: its document laid out; each character moved, scaled, turned, skewed,
//! faded, recoloured, blurred, spaced and swapped for another by the animators, as much as their
//! selectors pick it (each dimension on its own); set along a mask's path when the text says;
//! then its glyphs filled and stroked in their own looks (faux bold and italic drawn), the
//! fills and strokes in the order asked, each character put over the others as the text says,
//! underlines and lines through drawn with them. Colour glyphs (emoji) are drawn in their own
//! colours: their coloured layers, or the picture the font keeps for them.

use veymelo_color::{BlendMode, Rgba, blend};
use veymelo_geom::{Mat3, Rect, Vec2, vec2};
use veymelo_keyframe::{Justify, ShapePath, Value};
use veymelo_path::shape::Sub;
use veymelo_path::{Path, Rule, Stroke};
use veymelo_project::{Group, Layer, Node};
use veymelo_raster::{Image, Px, Sampling};
use veymelo_time::Tick;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::Read;

/// Reads a property with names its expression may use besides the usual ones.
pub(crate) type ReadWith<'r> = &'r dyn Fn(&veymelo_project::Property, &[(&str, Vec<f64>)]) -> Value;

/// How one character is changed: every animator's change, as much as it picks the character.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Change {
    pub offset: [f64; 3],
    pub anchor: [f64; 3],
    pub scale: [f64; 3],
    pub rotation: f64,
    pub rotation_x: f64,
    pub rotation_y: f64,
    pub skew: f64,
    pub skew_axis: f64,
    pub opacity: f64,
    fill: Option<Rgba>,
    fill_k: f64,
    fill_hsb: [f64; 3],
    fill_opacity: f64,
    stroke: Option<Rgba>,
    stroke_k: f64,
    stroke_hsb: [f64; 3],
    stroke_opacity: f64,
    stroke_width: f64,
    track_before: f64,
    track_after: f64,
    line_anchor: f64,
    line_anchor_k: f64,
    line_spacing: [f64; 2],
    char_offset: f64,
    char_value: Option<(f64, f64)>,
    char_range: u32,
    char_align: u32,
    blur: [f64; 2],
}

impl Default for Change {
    fn default() -> Change {
        Change {
            offset: [0.0; 3],
            anchor: [0.0; 3],
            scale: [100.0; 3],
            rotation: 0.0,
            rotation_x: 0.0,
            rotation_y: 0.0,
            skew: 0.0,
            skew_axis: 0.0,
            opacity: 100.0,
            fill: None,
            fill_k: 0.0,
            fill_hsb: [0.0; 3],
            fill_opacity: 100.0,
            stroke: None,
            stroke_k: 0.0,
            stroke_hsb: [0.0; 3],
            stroke_opacity: 100.0,
            stroke_width: 0.0,
            track_before: 0.0,
            track_after: 0.0,
            line_anchor: 0.0,
            line_anchor_k: 0.0,
            line_spacing: [0.0; 2],
            char_offset: 0.0,
            char_value: None,
            char_range: 0,
            char_align: 1,
            blur: [0.0; 2],
        }
    }
}

/// A number from -1 to 1, the same for the same inputs.
fn hash(a: u64, b: u64, c: u64) -> f64 {
    let mut h = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ c.wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 29;
    (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

/// A smooth wander from -1 to 1 for `unit` at time `t` (in wiggles).
fn wander(seed: u64, unit: u64, t: f64) -> f64 {
    let (k, f) = (t.floor(), t - t.floor());
    let s = f * f * (3.0 - 2.0 * f);
    let at = |i: f64| hash(seed, unit, i as i64 as u64);
    at(k) + (at(k + 1.0) - at(k)) * s
}

/// What unit each character is in, and how many units there are, by `based_on`.
fn units(based_on: u32, chars: &[char], word: &[usize], line: &[usize]) -> (Vec<Option<usize>>, usize) {
    match based_on {
        1 => {
            let mut n = 0;
            let u = chars
                .iter()
                .map(|c| {
                    if c.is_whitespace() {
                        None
                    } else {
                        n += 1;
                        Some(n - 1)
                    }
                })
                .collect();
            (u, n)
        }
        2 => (word.iter().zip(chars).map(|(w, c)| (!c.is_whitespace()).then_some(*w)).collect(), word.iter().max().map_or(0, |m| m + 1)),
        3 => (line.iter().map(|l| Some(*l)).collect(), line.iter().max().map_or(0, |m| m + 1)),
        _ => ((0..chars.len()).map(Some).collect(), chars.len()),
    }
}

/// What the text is, for the selectors: its characters and each one's word and line.
struct Units<'a> {
    chars: &'a [char],
    word: &'a [usize],
    line: &'a [usize],
}

/// How much one selector picks each character, each dimension (-1 to 1), and how it joins the
/// selectors above it (none: its result stands in their place).
fn select(g: &Group, t: Tick, read: Read, read_with: ReadWith, u: &Units, above: &[[f64; 3]]) -> (Option<u32>, Vec<[f64; 3]>) {
    let get = |p: &str| g.prop(p).ok().map(read);
    let num = |p: &str| get(p).map_or(0.0, |v| v.as_f64());
    let pick = |p: &str| get(p).map_or(0, |v| v.as_choice());
    let (unit_of, n) = units(pick("basedOn"), u.chars, u.word, u.line);
    match g.def.id {
        "wiggly" => {
            let (lo, hi) = (num("minAmount") / 100.0, num("maxAmount") / 100.0);
            let seed = num("seed").to_bits();
            let tt = t.seconds() * num("speed") + num("temporalPhase") / 360.0;
            let c = (num("correlation") / 100.0).clamp(0.0, 1.0);
            let locked = get("lockDimensions").is_some_and(|v| v.as_bool());
            let values = unit_of
                .iter()
                .map(|un| match un {
                    None => [0.0; 3],
                    Some(un) => [0u64, 1, 2].map(|d| {
                        let d = if locked { 0 } else { d };
                        let common = wander(seed ^ d.wrapping_mul(0x51), u32::MAX as u64, tt);
                        let own = wander(seed ^ d.wrapping_mul(0x51), *un as u64, tt + num("spatialPhase") / 360.0 * *un as f64);
                        let r = (common * c + own * (1.0 - c) + 1.0) / 2.0;
                        lo + (hi - lo) * r
                    }),
                })
                .collect();
            (Some(pick("mode")), values)
        }
        "expression" => {
            let Ok(amount) = g.prop("amount") else { return (None, vec![[1.0; 3]; u.chars.len()]) };
            let mut memo: Vec<Option<[f64; 3]>> = vec![None; n];
            let values = unit_of
                .iter()
                .enumerate()
                .map(|(i, un)| match un {
                    None => [0.0; 3],
                    Some(un) => {
                        if let Some(v) = memo[*un] {
                            return v;
                        }
                        let prev = above.get(i).copied().unwrap_or([1.0; 3]);
                        let vars = [("textIndex", vec![*un as f64 + 1.0]), ("textTotal", vec![n as f64]), ("selectorValue", prev.iter().map(|x| x * 100.0).collect())];
                        let v = read_with(amount, &vars).as_f64() / 100.0;
                        let v = [v.clamp(-1.0, 1.0); 3];
                        memo[*un] = Some(v);
                        v
                    }
                })
                .collect();
            (None, values)
        }
        _ => {
            let percent = pick("units") == 0;
            let scale = if percent { n as f64 / 100.0 } else { 1.0 };
            let (mut a, mut b) = (num("start") * scale, num("end") * scale);
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            let o = num("offset") * scale;
            let (a, b) = (a + o, b + o);
            // the order units are picked in: their own, or shuffled
            let mut order: Vec<usize> = (0..n).collect();
            if get("randomize").is_some_and(|v| v.as_bool()) {
                let seed = num("seed").to_bits();
                for i in (1..n).rev() {
                    let j = ((hash(seed, i as u64, 7) + 1.0) / 2.0 * (i + 1) as f64).floor().min(i as f64) as usize;
                    order.swap(i, j);
                }
            }
            let shape = pick("shape");
            let amount = num("amount") / 100.0;
            let (ease_hi, ease_lo) = (num("easeHigh") / 100.0, num("easeLow") / 100.0);
            let of_unit = |un: usize| -> f64 {
                let pos = order[un] as f64;
                let span = (b - a).max(1e-12);
                let tt = ((pos + 0.5 - a) / span).clamp(0.0, 1.0);
                let inside = pos + 0.5 >= a && pos + 0.5 <= b;
                let v = match shape {
                    // how much of the unit the range covers
                    0 => ((pos + 1.0).min(b) - pos.max(a)).clamp(0.0, 1.0),
                    1 => tt,
                    2 => 1.0 - tt,
                    3 if inside => 1.0 - (2.0 * tt - 1.0).abs(),
                    4 if inside => (1.0 - (2.0 * tt - 1.0).powi(2)).max(0.0).sqrt(),
                    5 if inside => (1.0 - (std::f64::consts::TAU * tt).cos()) / 2.0,
                    _ => 0.0,
                };
                // the ease of the shape's high and low ends
                let v = if (ease_hi != 0.0 || ease_lo != 0.0) && shape != 0 { eased(v, ease_hi, ease_lo) } else { v };
                v * amount
            };
            let values = unit_of.iter().map(|un| [un.map_or(0.0, of_unit); 3]).collect();
            (Some(pick("mode")), values)
        }
    }
}

/// A selection from 0 to 1 eased at its high and low ends (above zero: slower there).
fn eased(v: f64, hi: f64, lo: f64) -> f64 {
    let smooth = v * v * (3.0 - 2.0 * v);
    let w = if v > 0.5 { hi } else { lo };
    (v + (smooth - v) * w).clamp(0.0, 1.0)
}

/// How much an animator's selectors, together, pick each character, each dimension.
fn picked(animator: &Group, t: Tick, read: Read, read_with: ReadWith, u: &Units) -> Vec<[f64; 3]> {
    let mut out: Option<Vec<[f64; 3]>> = None;
    if let Ok(list) = animator.group("selectors") {
        for n in &list.children {
            let Node::Group(g) = n else { continue };
            if !g.enabled {
                continue;
            }
            let above = out.clone().unwrap_or_else(|| vec![[1.0; 3]; u.chars.len()]);
            let (mode, v) = select(g, t, read, read_with, u, &above);
            match mode {
                None => out = Some(v),
                Some(mode) => {
                    let r = out.get_or_insert_with(|| vec![[if mode == 1 { 1.0 } else { 0.0 }; 3]; u.chars.len()]);
                    for (r, v) in r.iter_mut().zip(&v) {
                        for d in 0..3 {
                            r[d] = match mode {
                                1 => r[d] - v[d],
                                2 => r[d] * v[d],
                                3 => r[d].min(v[d]),
                                4 => r[d].max(v[d]),
                                5 => (r[d] - v[d]).abs(),
                                _ => r[d] + v[d],
                            }
                            .clamp(-1.0, 1.0);
                        }
                    }
                }
            }
        }
    }
    // an animator with no selectors changes all of the text
    out.unwrap_or_else(|| vec![[1.0; 3]; u.chars.len()])
}

/// Every animator's changes to each character.
pub(crate) fn changes(layer: &Layer, t: Tick, read: Read, read_with: ReadWith, chars: &[char], word: &[usize], line: &[usize]) -> Vec<Change> {
    let mut out = vec![Change::default(); chars.len()];
    let Ok(list) = layer.props.group("text/animators") else { return out };
    let u = Units { chars, word, line };
    for n in &list.children {
        let Node::Group(a) = n else { continue };
        if !a.enabled {
            continue;
        }
        let sel = picked(a, t, read, read_with, &u);
        let get = |p: &str| a.prop(p).ok().map(read);
        // set: not as it starts (a property an animator holds by default changes nothing)
        let set = |p: &str| a.prop(p).is_ok_and(|q| !q.is_default());
        let v3 = |p: &str, d: f64| get(p).map_or([d; 3], |v| match v {
            Value::Vec3(q) => q,
            Value::Vec2(q) => [q[0], q[1], d],
            other => [other.as_f64(); 3],
        });
        let v2 = |p: &str| get(p).map_or([0.0; 2], |v| v.as_vec2());
        let num = |p: &str, d: f64| get(p).map_or(d, |v| v.as_f64());
        let col = |p: &str| get(p).map_or(Rgba::CLEAR, |v| v.as_color());
        let (pos, anchor, scale) = (v3("position", 0.0), v3("anchor", 0.0), v3("scale", 100.0));
        let (rot, rx, ry, skew, op) = (num("rotation", 0.0), num("rotationX", 0.0), num("rotationY", 0.0), num("skew", 0.0), num("opacity", 100.0));
        let skew_axis = set("skewAxis").then(|| num("skewAxis", 0.0));
        let (fc, sc) = (col("fillColor"), col("strokeColor"));
        let fill_hsb = [num("fillHue", 0.0), num("fillSaturation", 0.0), num("fillBrightness", 0.0)];
        let stroke_hsb = [num("strokeHue", 0.0), num("strokeSaturation", 0.0), num("strokeBrightness", 0.0)];
        let (fop, sop, sw, tr) = (num("fillOpacity", 100.0), num("strokeOpacity", 100.0), num("strokeWidth", 0.0), num("tracking", 0.0));
        let tracking_type = get("trackingType").map_or(0, |v| v.as_choice());
        let line_anchor = set("lineAnchor").then(|| num("lineAnchor", 0.0));
        let line_spacing = v2("lineSpacing");
        let char_value = set("characterValue").then(|| num("characterValue", 0.0));
        let char_range = set("characterRange").then(|| get("characterRange").map_or(0, |v| v.as_choice()));
        let char_align = set("characterAlignment").then(|| get("characterAlignment").map_or(1, |v| v.as_choice()));
        let char_offset = num("characterOffset", 0.0);
        let blur = v2("blur");
        for (c, k3) in out.iter_mut().zip(&sel) {
            if k3.iter().all(|x| *x == 0.0) {
                continue;
            }
            let k = k3[0];
            for d in 0..3 {
                c.offset[d] += pos[d] * k3[d];
                c.anchor[d] += anchor[d] * k3[d];
                c.scale[d] *= 1.0 + (scale[d] / 100.0 - 1.0) * k3[d];
            }
            c.rotation += rot * k;
            c.rotation_x += rx * k;
            c.rotation_y += ry * k;
            c.skew += skew * k;
            if let Some(a) = skew_axis {
                c.skew_axis = a;
            }
            c.opacity *= 1.0 + (op / 100.0 - 1.0) * k;
            if fc.a > 0.0 {
                c.fill = Some(fc.with_alpha(1.0));
                c.fill_k = (c.fill_k + fc.a * k).clamp(0.0, 1.0);
            }
            if sc.a > 0.0 {
                c.stroke = Some(sc.with_alpha(1.0));
                c.stroke_k = (c.stroke_k + sc.a * k).clamp(0.0, 1.0);
            }
            for d in 0..3 {
                c.fill_hsb[d] += fill_hsb[d] * k;
                c.stroke_hsb[d] += stroke_hsb[d] * k;
            }
            c.fill_opacity *= 1.0 + (fop / 100.0 - 1.0) * k;
            c.stroke_opacity *= 1.0 + (sop / 100.0 - 1.0) * k;
            c.stroke_width += sw * k;
            let tk = tr * k;
            match tracking_type {
                1 => c.track_before += tk,
                2 => c.track_after += tk,
                _ => {
                    c.track_before += tk / 2.0;
                    c.track_after += tk / 2.0;
                }
            }
            if let Some(a) = line_anchor
                && k.abs() >= c.line_anchor_k
            {
                c.line_anchor = a;
                c.line_anchor_k = k.abs();
            }
            c.line_spacing[0] += line_spacing[0] * k;
            c.line_spacing[1] += line_spacing[1] * k;
            c.char_offset += char_offset * k;
            if let Some(v) = char_value {
                c.char_value = Some((v, k));
            }
            if let Some(r) = char_range {
                c.char_range = r;
            }
            if let Some(a) = char_align {
                c.char_align = a;
            }
            c.blur[0] += (blur[0] * k3[0]).abs();
            c.blur[1] += (blur[1] * k3[1]).abs();
        }
    }
    out
}

/// The character a change puts in place of `ch` (none: it stays).
fn substitute(ch: char, x: &Change) -> Option<char> {
    if ch.is_whitespace() || (x.char_offset.round() == 0.0 && x.char_value.is_none()) {
        return None;
    }
    let mut code = ch as u32 as f64;
    if let Some((v, k)) = x.char_value {
        code += (v - code) * k.clamp(0.0, 1.0);
    }
    let off = x.char_offset.round() as i64;
    let base = code.round().max(0.0) as u32;
    let out = if x.char_range == 1 {
        char::from_u32((base as i64 + off).max(0) as u32)
    } else {
        let c = char::from_u32(base)?;
        let wrap = |lo: char, n: i64| char::from_u32((lo as i64 + (c as i64 - lo as i64 + off).rem_euclid(n)) as u32);
        if c.is_ascii_lowercase() {
            wrap('a', 26)
        } else if c.is_ascii_uppercase() {
            wrap('A', 26)
        } else if c.is_ascii_digit() {
            wrap('0', 10)
        } else {
            Some(c)
        }
    }?;
    (out != ch).then_some(out)
}

/// A colour moved toward `to` by `k`, its hue, saturation and brightness turned, its opacity
/// scaled.
fn recolour(base: Rgba, to: Option<Rgba>, k: f64, hsb: [f64; 3], opacity: f64) -> Rgba {
    let mut c = match to {
        Some(t) => base.lerp(t.with_alpha(base.a), k),
        None => base,
    };
    if hsb != [0.0; 3] {
        let (h, s, v) = veymelo_color::rgb_to_hsv(c.r, c.g, c.b);
        let (r, g, b) = veymelo_color::hsv_to_rgb(h + hsb[0], (s + hsb[1] / 100.0).clamp(0.0, 1.0), (v + hsb[2] / 100.0).clamp(0.0, 1.0));
        c = Rgba { r, g, b, ..c };
    }
    c.with_alpha(c.a * (opacity / 100.0).clamp(0.0, 1.0))
}

/// Glyph outlines as closed runs, for stroking.
fn runs(p: &Path) -> Vec<Sub> {
    p.contours().iter().map(|c| Sub::polygon(&c.iter().map(|q| [q.0, q.1]).collect::<Vec<_>>(), true)).collect()
}

fn mapped(p: &Path, m: &Mat3) -> Path {
    let mut out = Path::default();
    for c in p.contours() {
        for (k, q) in c.iter().enumerate() {
            let r = m.apply(vec2(q.0, q.1));
            if k == 0 {
                out.move_to(r.x, r.y);
            } else {
                out.line_to(r.x, r.y);
            }
        }
    }
    out
}

/// A path measured along its length: each point and how far along it is.
struct Measure {
    points: Vec<[f64; 2]>,
    at: Vec<f64>,
}

impl Measure {
    fn new(p: &ShapePath) -> Option<Measure> {
        let n = veymelo_path::bezier::pieces(p);
        if n == 0 {
            return None;
        }
        let mut points = vec![p.points[0]];
        for i in 0..n {
            let c = veymelo_path::bezier::piece(p, i);
            let straight = c[1] == c[0] && c[2] == c[3];
            let steps = if straight { 1 } else { 48 };
            for k in 1..=steps {
                points.push(veymelo_path::bezier::at(&c, k as f64 / steps as f64));
            }
        }
        let mut at = vec![0.0];
        for w in points.windows(2) {
            let last = *at.last().expect("not empty");
            at.push(last + (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]));
        }
        (at.last().copied().unwrap_or(0.0) > 1e-9).then_some(Measure { points, at })
    }

    fn length(&self) -> f64 {
        *self.at.last().expect("not empty")
    }

    /// The point `s` along it (going on straight past its ends) and its direction there.
    fn point(&self, s: f64) -> ([f64; 2], [f64; 2]) {
        let n = self.points.len();
        let k = match self.at.binary_search_by(|v| v.total_cmp(&s)) {
            Ok(k) => k.clamp(1, n - 1),
            Err(k) => k.clamp(1, n - 1),
        };
        let (a, b) = (self.points[k - 1], self.points[k]);
        let len = (self.at[k] - self.at[k - 1]).max(1e-12);
        let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
        let f = (s - self.at[k - 1]) / len;
        ([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f], dir)
    }
}

/// A text layer's glyphs at its time `t`, each changed by the animators (`three`: each
/// character's change kept as its own matrix in depth instead), with their looks; and notes.
/// A colour glyph's picture file read into premultiplied sRGB, kept by its bytes (none: the
/// system could not read it).
fn glyph_picture(bytes: &[u8]) -> Option<Arc<Image>> {
    static KEPT: OnceLock<Mutex<HashMap<u64, Option<Arc<Image>>>>> = OnceLock::new();
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        bytes.hash(&mut h);
        h.finish()
    };
    let kept = KEPT.get_or_init(Default::default);
    if let Some(p) = kept.lock().ok().and_then(|k| k.get(&key).cloned()) {
        return p;
    }
    let p = veymelo_os::read_picture_bytes(bytes).ok().map(|p| Arc::new(Image { width: p.width, height: p.height, px: p.px.into() }));
    if let Ok(mut k) = kept.lock() {
        if k.len() > 4096 {
            k.clear();
        }
        k.insert(key, p.clone());
    }
    p
}

/// `marks` drawn at `s` pixels to a unit (colour pictures are taken from the font at that size).
#[allow(clippy::too_many_arguments)]
fn marks(layer: &Layer, t: Tick, read: Read, read_with: ReadWith, three: bool, s: f64) -> (Vec<Mark>, Vec<String>, bool, usize, usize) {
    let mut notes = Vec::new();
    let Some(doc) = layer.props.prop("text/sourceText").ok().and_then(|p| read(p).doc()) else { return (Vec::new(), notes, false, 0, 0) };
    let mut lay = veymelo_text::lay_out(&doc);
    if lay.no_font {
        notes.push(format!("\"{}\": a font it asks for is not there; the system font stands in", layer.name));
    }
    if !lay.missing.is_empty() {
        notes.push(format!("\"{}\": no font has {}", layer.name, lay.missing.iter().collect::<String>()));
    }
    let chars: Vec<char> = doc.text.chars().collect();
    // each character's word and line, and where its glyphs reach across
    let (mut word, mut line) = (vec![0usize; chars.len()], vec![0usize; chars.len()]);
    for g in &lay.glyphs {
        if g.char_index < chars.len() {
            word[g.char_index] = g.word;
            line[g.char_index] = g.line;
        }
    }
    let seen_char: Vec<bool> = {
        let mut v = vec![false; chars.len()];
        for g in &lay.glyphs {
            if g.char_index < v.len() {
                v[g.char_index] = true;
            }
        }
        v
    };
    for i in 1..chars.len() {
        if !seen_char[i] {
            word[i] = word[i - 1];
            line[i] = line[i - 1];
        }
    }
    let change = changes(layer, t, read, read_with, &chars, &word, &line);
    // characters swapped for others: the line set again when asked, else each in its own room
    let subs: Vec<Option<char>> = chars.iter().zip(&change).map(|(c, x)| substitute(*c, x)).collect();
    if subs.iter().zip(&change).any(|(s, x)| s.is_some() && x.char_align == 3) {
        let text: String = chars.iter().zip(&subs).map(|(c, s)| s.unwrap_or(*c)).collect();
        let d2 = veymelo_keyframe::TextDoc { text, ..doc.clone() };
        lay = veymelo_text::lay_out(&d2);
    }
    // tracking from animators spreads each line from where its anchor holds it
    let mut shift = vec![0.0f64; chars.len()];
    let mut by_line: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for (k, g) in lay.glyphs.iter().enumerate() {
        by_line.entry(g.line).or_default().push(k);
    }
    for (ln, glyphs) in &by_line {
        let para = lay.lines.get(*ln).map_or(0, |l| l.paragraph);
        let (mut la, mut la_k) = (
            match doc.para_at(para).justify {
                Justify::Center | Justify::JustifyCenter => 50.0,
                Justify::Right | Justify::JustifyRight => 100.0,
                _ => 0.0,
            },
            0.0,
        );
        let mut pen = 0.0;
        let mut last: Option<usize> = None;
        for &k in glyphs {
            let g = &lay.glyphs[k];
            let i = g.char_index.min(chars.len().saturating_sub(1));
            if last != Some(i) {
                if let Some(p) = last {
                    pen += change[p].track_after / 1000.0 * lay.styles[g.style].size;
                }
                pen += change[i].track_before / 1000.0 * lay.styles[g.style].size;
                shift[i] = pen;
                last = Some(i);
                if change[i].line_anchor_k > la_k {
                    la = change[i].line_anchor;
                    la_k = change[i].line_anchor_k;
                }
            }
        }
        if let Some(p) = last {
            pen += change[p].track_after / 1000.0 * doc.size;
        }
        let back = pen * la / 100.0;
        let mut done = vec![false; chars.len()];
        for &k in glyphs {
            let i = lay.glyphs[k].char_index.min(chars.len().saturating_sub(1));
            if !done[i] {
                shift[i] -= back;
                done[i] = true;
            }
        }
    }
    // what each character turns and scales about
    let grouping = layer.props.prop("text/moreOptions/anchorGrouping").map_or(0, |p| read(p).as_choice());
    let align = layer.props.prop("text/moreOptions/groupingAlignment").map_or([0.0; 2], |p| read(p).as_vec2());
    let group_of = |i: usize| -> usize {
        match grouping {
            1 => word[i],
            2 => line[i],
            3 => 0,
            _ => i,
        }
    };
    let origin = |g: &veymelo_text::Placed| -> (f64, f64) {
        let i = g.char_index.min(chars.len().saturating_sub(1));
        let c = &change[i];
        (g.x + shift[i] + c.line_spacing[0] * g.line as f64, g.y + c.line_spacing[1] * g.line as f64)
    };
    let mut span: Vec<(f64, f64, f64)> = vec![(f64::MAX, f64::MIN, 0.0); chars.len()];
    let mut group_span: std::collections::HashMap<usize, (f64, f64, f64)> = std::collections::HashMap::new();
    for g in &lay.glyphs {
        if g.char_index >= chars.len() || chars[g.char_index].is_whitespace() && grouping == 1 {
            continue;
        }
        let (ox, oy) = origin(g);
        let sp = &mut span[g.char_index];
        *sp = (sp.0.min(ox), sp.1.max(ox + g.advance), oy);
        let e = group_span.entry(group_of(g.char_index)).or_insert((f64::MAX, f64::MIN, oy));
        *e = (e.0.min(ox), e.1.max(ox + g.advance), e.2);
    }
    // set along a mask's path
    let on_path = text_path(layer, read);
    let mut arc = vec![0.0f64; chars.len()];
    let first_baseline = lay.lines.first().map_or(0.0, |l| l.y);
    if let Some((m, _, force, first_margin, last_margin)) = &on_path {
        for l in &lay.lines {
            let justify = doc.para_at(l.paragraph).justify;
            let room = m.length() - first_margin - last_margin;
            for i in l.chars.clone() {
                if i >= chars.len() || span[i].0 == f64::MAX {
                    continue;
                }
                let centre = (span[i].0 + span[i].1) / 2.0;
                let along = centre - l.x;
                arc[i] = if *force && l.width > 0.0 {
                    first_margin + along * room / l.width
                } else {
                    match justify {
                        Justify::Center | Justify::JustifyCenter => first_margin + (room - l.width) / 2.0 + along,
                        Justify::Right | Justify::JustifyRight => m.length() - last_margin - (l.width - along),
                        _ => first_margin + along,
                    }
                };
            }
        }
    }
    // each glyph's outline, changed, with its look
    let mut marks: Vec<Mark> = Vec::new();
    for g in &lay.glyphs {
        let i = g.char_index.min(chars.len().saturating_sub(1));
        let c = &change[i];
        let st = &lay.styles[g.style];
        // a swapped character in its own room: its glyph from the same font, aligned in it
        let (glyph, nudge) = match subs[i] {
            Some(sub) if c.char_align != 3 => {
                let id = g.font.glyph(sub);
                if id == 0 {
                    (g.glyph, 0.0)
                } else {
                    let adv = g.font.advance(id) * g.scale_x;
                    (id, match c.char_align {
                        0 => 0.0,
                        2 => g.advance - adv,
                        _ => (g.advance - adv) / 2.0,
                    })
                }
            }
            _ => (g.glyph, 0.0),
        };
        let mut outline = Path::default();
        g.font.outline(glyph, &mut outline, &|fx, fy| {
            let (x, y) = g.map(fx, fy);
            (x + nudge, y)
        });
        // a colour glyph: its graph of paints, else its coloured layers, else its picture
        let painted = if g.font.has_color() { g.font.color_glyph(glyph) } else { None };
        let layers = if painted.is_none() && g.font.has_color() { g.font.color_layers(glyph) } else { None };
        let ppem = g.font.units * g.scale.abs() * s * c.scale[0].abs().max(c.scale[1].abs()) / 100.0;
        let picture = if painted.is_none() && layers.is_none() && g.font.has_color() { g.font.picture(glyph, ppem) } else { None };
        let pixels = picture.and_then(|p| {
            let img = glyph_picture(p.bytes);
            if img.is_none() && !notes.iter().any(|n: &String| n.contains("colour glyph")) {
                notes.push(format!("\"{}\": a colour glyph's picture cannot be read here", layer.name));
            }
            img.map(|img| (img, p.origin, p.ppem))
        });
        if outline.contours().is_empty() && painted.is_none() && layers.is_none() && pixels.is_none() {
            continue;
        }
        let (ox, oy) = origin(g);
        let (dx, dy) = (ox - g.x, oy - g.y);
        let gs = group_span.get(&group_of(i)).copied().unwrap_or((ox, ox + g.advance, oy));
        let pivot = vec2((gs.0 + gs.1) / 2.0 + align[0] / 100.0 * (gs.1 - gs.0) + c.anchor[0], gs.2 - align[1] / 100.0 * st.size + c.anchor[1]);
        let char_m = Mat3::rotate_deg(c.rotation) * Mat3::skew_deg(-c.skew, c.skew_axis) * Mat3::scale(vec2(c.scale[0] / 100.0, c.scale[1] / 100.0));
        // in depth: the character turned about x and y too, and moved toward or away
        let char_m3 = veymelo_geom::Mat4::rotate_z(c.rotation)
            * veymelo_geom::Mat4::rotate_y(c.rotation_y)
            * veymelo_geom::Mat4::rotate_x(c.rotation_x)
            * veymelo_geom::Mat4::from_mat3(&Mat3::skew_deg(-c.skew, c.skew_axis))
            * veymelo_geom::Mat4::scale(veymelo_geom::vec3(c.scale[0] / 100.0, c.scale[1] / 100.0, c.scale[2] / 100.0));
        let pivot3 = veymelo_geom::vec3(pivot.x, pivot.y, c.anchor[2]);
        let (m, m3) = match &on_path {
            Some((pm, perp, ..)) => {
                // across: along the path; up and down: off it
                let centre = if span[i].0 == f64::MAX { pivot.x } else { (span[i].0 + span[i].1) / 2.0 };
                let s_along = arc[i] + (pivot.x - centre) + c.offset[0];
                let off = pivot.y - first_baseline + c.offset[1];
                let (p, d) = pm.point(s_along);
                let (normal_x, normal_y) = (-d[1], d[0]);
                let at = vec2(p[0] + normal_x * off, p[1] + normal_y * off);
                let deg = if *perp { d[1].atan2(d[0]).to_degrees() } else { 0.0 };
                let m = Mat3::translate(at) * Mat3::rotate_deg(deg) * char_m * Mat3::translate(-pivot) * Mat3::translate(vec2(dx, dy));
                let m3 = veymelo_geom::Mat4::translate(veymelo_geom::vec3(at.x, at.y, c.offset[2])) * veymelo_geom::Mat4::rotate_z(deg) * char_m3 * veymelo_geom::Mat4::translate(-pivot3) * veymelo_geom::Mat4::translate(veymelo_geom::vec3(dx, dy, 0.0));
                (m, m3)
            }
            None => {
                let m = Mat3::translate(vec2(c.offset[0] + dx, c.offset[1] + dy)) * Mat3::translate(pivot) * char_m * Mat3::translate(-pivot);
                let m3 = veymelo_geom::Mat4::translate(veymelo_geom::vec3(c.offset[0] + dx, c.offset[1] + dy, c.offset[2])) * veymelo_geom::Mat4::translate(pivot3) * char_m3 * veymelo_geom::Mat4::translate(-pivot3);
                (m, m3)
            }
        };
        // per-character 3D: the outline as it is set, the change kept for the scene
        let (m, m3) = if three { (Mat3::IDENTITY, m3) } else { (m, veymelo_geom::Mat4::IDENTITY) };
        let outline = mapped(&outline, &m);
        let fill = st.fill.map(|f| recolour(f, c.fill, c.fill_k, c.fill_hsb, c.fill_opacity));
        let opacity = (c.opacity / 100.0).clamp(0.0, 1.0);
        let stroke_of = |outline: &Path| {
            let stroke = st.stroke.or(c.stroke.filter(|_| c.stroke_k > 0.0)).map(|f| recolour(f, c.stroke, c.stroke_k, c.stroke_hsb, c.stroke_opacity));
            let width = (st.stroke_width + c.stroke_width).max(0.0);
            let path = (stroke.is_some() && width > 0.0 && !outline.contours().is_empty()).then(|| veymelo_path::stroke(&runs(outline), &Stroke { width, ..Stroke::default() }));
            (path, stroke)
        };
        if let Some(layers) = layers.filter(|_| fill.is_some()) {
            // its layers bottom first, each in its own colour (none: the text's), the stroke
            // round the whole glyph under them all or over them all
            let (stroke_path, stroke) = stroke_of(&outline);
            let n = layers.len();
            for (k, (lg, colour)) in layers.into_iter().enumerate() {
                let mut part = Path::default();
                g.font.outline(lg, &mut part, &|fx, fy| {
                    let (x, y) = g.map(fx, fy);
                    (x + nudge, y)
                });
                let colour = colour.map(|[r, g, b, a]| Rgba { r: r as f64, g: g as f64, b: b as f64, a: a as f64 }).or(fill);
                let carries = if doc.stroke_over { k + 1 == n } else { k == 0 };
                marks.push(Mark { fill: Some(mapped(&part, &m)), stroke: stroke_path.clone().filter(|_| carries), fill_color: colour, stroke_color: stroke, opacity, stroke_over: doc.stroke_over, blur: c.blur, char_index: i, m3, picture: None });
            }
            continue;
        }
        // a graph of paints drawn into a picture in the text's colour, at the size it is seen
        let pixels = match (&painted, fill) {
            (Some(cg), Some(f)) => crate::colour_glyph::draw(&g.font, glyph, cg, ppem, [f.r, f.g, f.b, f.a].map(|v| v as f32)),
            _ => pixels,
        };
        if let Some((img, origin, strike)) = pixels.filter(|_| fill.is_some()) {
            // the picture's pixels (y down from its top) to font units (y up), to the layer
            let upp = g.font.units / strike.max(1.0);
            let h = img.height as f64;
            let at = |bx: f64, by: f64| {
                let (x, y) = g.map((origin.0 + bx) * upp, (origin.1 + h - by) * upp);
                vec2(x + nudge, y)
            };
            let (o, ex, ey) = (at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0));
            let to_layer = m * Mat3([[ex.x - o.x, ey.x - o.x, o.x], [ex.y - o.y, ey.y - o.y, o.y], [0.0, 0.0, 1.0]]);
            let (stroke_path, stroke) = stroke_of(&outline);
            marks.push(Mark { fill: None, stroke: stroke_path, fill_color: None, stroke_color: stroke, opacity, stroke_over: doc.stroke_over, blur: c.blur, char_index: i, m3, picture: Some((img, to_layer)) });
            continue;
        }
        let stroke = st.stroke.or(c.stroke.filter(|_| c.stroke_k > 0.0)).map(|f| recolour(f, c.stroke, c.stroke_k, c.stroke_hsb, c.stroke_opacity));
        let width = (st.stroke_width + c.stroke_width).max(0.0);
        let stroke_path = (stroke.is_some() && width > 0.0).then(|| veymelo_path::stroke(&runs(&outline), &Stroke { width, ..Stroke::default() }));
        let mut fill_path = fill.map(|_| outline.clone());
        // faux bold: the fill thickened by a stroke of its own colour
        if st.faux_bold
            && let Some(fp) = &fill_path
        {
            let thick = veymelo_path::stroke(&runs(fp), &Stroke { width: st.size / 30.0 * c.scale[0].abs() / 100.0, ..Stroke::default() });
            let mut both = fp.clone();
            for k in thick.contours() {
                for (j, q) in k.iter().enumerate() {
                    if j == 0 {
                        both.move_to(q.0, q.1);
                    } else {
                        both.line_to(q.0, q.1);
                    }
                }
                both.close();
            }
            fill_path = Some(both);
        }
        marks.push(Mark { fill: fill_path, stroke: stroke_path, fill_color: fill, stroke_color: stroke, opacity, stroke_over: doc.stroke_over, blur: c.blur, char_index: i, m3, picture: None });
    }
    // underlines and lines through, in their letters' colour
    for d in &lay.decorations {
        let st = &lay.styles[d.style];
        let Some(colour) = st.fill.or(st.stroke) else { continue };
        let mut p = Path::default();
        let (y0, y1) = (d.y - d.thickness / 2.0, d.y + d.thickness / 2.0);
        p.move_to(d.x0, y0);
        p.line_to(d.x1, y0);
        p.line_to(d.x1, y1);
        p.line_to(d.x0, y1);
        p.close();
        marks.push(Mark { fill: Some(p), stroke: None, fill_color: Some(colour), stroke_color: None, opacity: 1.0, stroke_over: false, blur: [0.0; 2], char_index: usize::MAX, m3: veymelo_geom::Mat4::IDENTITY, picture: None });
    }
    let blend = layer.props.prop("text/moreOptions/interCharBlend").map_or(0, |p| read(p).as_choice()) as usize;
    let order = layer.props.prop("text/moreOptions/fillStroke").map_or(0, |p| read(p).as_choice()) as usize;
    (marks, notes, doc.stroke_over, blend, order)
}

/// A text layer at its time `t` (`read` gives its properties' values; `read_with` with names an
/// expression selector's amount reads), drawn at `s` pixels to a unit, only where `seen`: the
/// picture, where its first pixel starts, the pixels to a unit it was drawn at, and notes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(layer: &Layer, t: Tick, read: Read, read_with: ReadWith, s: f64, seen: Option<Rect>, px: &dyn Fn(Rgba, f64) -> Px) -> (Option<(Image, Vec2, f64)>, Vec<String>) {
    let (marks, notes, _, blend, order) = marks(layer, t, read, read_with, false, s);
    match paint_marks(&marks, s, seen, px, blend, order) {
        Some(drawn) => (Some(drawn), notes),
        None => (None, notes),
    }
}

/// A character's picture, where its corner sits, its opacity, and its own matrix.
pub(crate) type CharPicture = (Image, Vec2, f64, veymelo_geom::Mat4);

/// Per-character 3D: each character's picture (at `s` pixels to a unit) and its own matrix,
/// from the layer's units to where it stands in them; and notes.
pub(crate) fn chars_3d(layer: &Layer, t: Tick, read: Read, read_with: ReadWith, s: f64, px: &dyn Fn(Rgba, f64) -> Px) -> (Vec<CharPicture>, Vec<String>) {
    let (marks, notes, _, blend, order) = marks(layer, t, read, read_with, true, s);
    let mut keys: Vec<usize> = marks.iter().map(|m| m.char_index).collect();
    keys.sort();
    keys.dedup();
    let mut out = Vec::new();
    for k in keys {
        let group: Vec<Mark> = marks.iter().filter(|m| m.char_index == k).cloned().collect();
        let m3 = group[0].m3;
        if let Some((img, at, scale)) = paint_marks(&group, s, None, px, blend, order) {
            out.push((img, at, scale, m3));
        }
    }
    (out, notes)
}

/// Marks painted into a picture at `s` pixels to a unit (only where `seen`): it, where its
/// first pixel starts, and the pixels to a unit it was drawn at.
fn paint_marks(marks: &[Mark], s: f64, seen: Option<Rect>, px: &dyn Fn(Rgba, f64) -> Px, mode: usize, order: usize) -> Option<(Image, Vec2, f64)> {
    let mut bounds = Rect::EMPTY;
    for m in marks {
        let pad = m.blur[0].max(m.blur[1]) * 1.5;
        for p in [&m.fill, &m.stroke].into_iter().flatten() {
            if let Some((x0, y0, x1, y1)) = p.extent() {
                bounds = bounds.union(&Rect::new(x0 - pad, y0 - pad, x1 + pad, y1 + pad));
            }
        }
        if let Some((img, to)) = &m.picture {
            let r = to.map_rect(&Rect::from_size(img.width as f64, img.height as f64));
            bounds = bounds.union(&Rect::new(r.x0 - pad, r.y0 - pad, r.x1 + pad, r.y1 + pad));
        }
    }
    let area = match seen {
        Some(r) => bounds.intersect(&r),
        None => bounds,
    };
    if area.is_empty() || !area.width().is_finite() {
        return None;
    }
    let s = s.min(8192.0 / area.width().max(area.height()).max(1e-9));
    let (i0, j0) = ((area.x0 * s).floor(), (area.y0 * s).floor());
    let (w, h) = (((area.x1 * s).ceil() - i0).max(1.0) as usize, ((area.y1 * s).ceil() - j0).max(1.0) as usize);
    let mut img = Image::new(w, h);
    let to_buf = Mat3::translate(vec2(-i0, -j0)) * Mat3::scale(vec2(s, s));
    let blend_mode = BlendMode::from_id(veymelo_project::text::INTER_CHAR_BLEND.get(mode).copied().unwrap_or("normal")).unwrap_or(BlendMode::Normal);
    // a mark's part painted: straight onto the picture, or blurred on a patch of its own first
    let paint = |p: &Path, color: Rgba, opacity: f64, blur: [f64; 2], img: &mut Image| {
        let src = px(color, opacity);
        if blur[0] <= 0.05 && blur[1] <= 0.05 {
            let Some(cover) = veymelo_path::fill(&mapped(p, &to_buf), Rule::NonZero, w, h) else { return };
            for y in cover.top..cover.top + cover.height {
                for x in cover.left..cover.left + cover.width {
                    let k = cover.at(x, y);
                    if k > 0.0 {
                        let d = &mut img.px[y * w + x];
                        *d = blend(blend_mode, *d, src.map(|v| v * k), 0.0);
                    }
                }
            }
            return;
        }
        let Some(cover) = veymelo_path::fill(&mapped(p, &to_buf), Rule::NonZero, w, h) else { return };
        let mut patch = Image::new(w, h);
        for y in cover.top..cover.top + cover.height {
            for x in cover.left..cover.left + cover.width {
                let k = cover.at(x, y);
                if k > 0.0 {
                    patch.px[y * w + x] = src.map(|v| v * k);
                }
            }
        }
        let patch = veymelo_raster::filter::gaussian(&patch, blur[0] * s / 2.0, blur[1] * s / 2.0, veymelo_raster::filter::Edge::Clear);
        for (d, q) in img.px.iter_mut().zip(&patch.px) {
            if q[3] > 0.0 {
                *d = blend(blend_mode, *d, *q, 0.0);
            }
        }
    };
    // a colour picture in the working colours, drawn through its matrix like a fill
    let picture = |pic: &Image, to: &Mat3, opacity: f64, blur: [f64; 2], img: &mut Image| {
        let work = Image { width: pic.width, height: pic.height, px: pic.px.iter().map(|q| {
            if q[3] <= 0.0 {
                return [0.0; 4];
            }
            let a = q[3] as f64;
            px(Rgba { r: q[0] as f64 / a, g: q[1] as f64 / a, b: q[2] as f64 / a, a }, 1.0)
        }).collect() };
        let m = to_buf * *to;
        if blur[0] <= 0.05 && blur[1] <= 0.05 {
            veymelo_raster::place(img, &work, &m, Sampling::Bilinear, blend_mode, opacity as f32, 0);
            return;
        }
        let mut patch = Image::new(w, h);
        veymelo_raster::place(&mut patch, &work, &m, Sampling::Bilinear, BlendMode::Normal, opacity as f32, 0);
        let patch = veymelo_raster::filter::gaussian(&patch, blur[0] * s / 2.0, blur[1] * s / 2.0, veymelo_raster::filter::Edge::Clear);
        for (d, q) in img.px.iter_mut().zip(&patch.px) {
            if q[3] > 0.0 {
                *d = blend(blend_mode, *d, *q, 0.0);
            }
        }
    };
    match order {
        // every fill over every stroke, or every stroke over every fill
        1 | 2 => {
            let parts: [bool; 2] = if order == 1 { [false, true] } else { [true, false] };
            for fills in parts {
                for m in marks {
                    if m.opacity <= 0.0 {
                        continue;
                    }
                    let part = if fills { m.fill.as_ref().zip(m.fill_color) } else { m.stroke.as_ref().zip(m.stroke_color) };
                    if let Some((p, c)) = part {
                        paint(p, c, m.opacity, m.blur, &mut img);
                    }
                    if let (true, Some((pic, to))) = (fills, &m.picture) {
                        picture(pic, to, m.opacity, m.blur, &mut img);
                    }
                }
            }
        }
        _ => {
            for m in marks {
                if m.opacity <= 0.0 {
                    continue;
                }
                let fill = m.fill.as_ref().zip(m.fill_color);
                let stroke = m.stroke.as_ref().zip(m.stroke_color);
                if let (false, Some((pic, to))) = (m.stroke_over, &m.picture) {
                    if let Some((p, c)) = stroke {
                        paint(p, c, m.opacity, m.blur, &mut img);
                    }
                    picture(pic, to, m.opacity, m.blur, &mut img);
                    continue;
                }
                let (first, second) = if m.stroke_over { (fill, stroke) } else { (stroke, fill) };
                for (p, c) in [first, second].into_iter().flatten() {
                    paint(p, c, m.opacity, m.blur, &mut img);
                }
                if let (true, Some((pic, to))) = (m.stroke_over, &m.picture) {
                    picture(pic, to, m.opacity, m.blur, &mut img);
                    if let Some((p, c)) = stroke {
                        paint(p, c, m.opacity, m.blur, &mut img);
                    }
                }
            }
        }
    }
    Some((img, vec2(i0 / s, j0 / s), s))
}

/// A glyph's outline, changed, with its look: the character it is of, (per-character 3D) the
/// character's own place in depth, and a colour glyph's picture with the matrix from its
/// pixels to the layer.
#[derive(Clone)]
struct Mark {
    fill: Option<Path>,
    stroke: Option<Path>,
    fill_color: Option<Rgba>,
    stroke_color: Option<Rgba>,
    opacity: f64,
    stroke_over: bool,
    blur: [f64; 2],
    char_index: usize,
    m3: veymelo_geom::Mat4,
    picture: Option<(Arc<Image>, Mat3)>,
}

/// The mask path the text is set along, measured, and how: (its measure, perpendicular,
/// forced alignment, first margin, last margin).
fn text_path(layer: &Layer, read: Read) -> Option<(Measure, bool, bool, f64, f64)> {
    let po = layer.props.group("text/pathOptions").ok()?;
    let get = |p: &str| po.prop(p).ok().map(read);
    let k = get("path").map_or(0.0, |v| v.as_f64()).round() as usize;
    if k == 0 {
        return None;
    }
    let masks = layer.props.group("masks").ok()?;
    let mask = masks.children.get(k - 1)?.as_group()?;
    let Value::Path(mut sp) = read(mask.prop("path").ok()?) else { return None };
    if get("reversePath").is_some_and(|v| v.as_bool()) {
        sp = veymelo_path::bezier::reversed(&sp);
    }
    let m = Measure::new(&sp)?;
    let flag = |p: &str, d: bool| get(p).map_or(d, |v| v.as_bool());
    let num = |p: &str| get(p).map_or(0.0, |v| v.as_f64());
    Some((m, flag("perpendicular", true), flag("forceAlignment", false), num("firstMargin"), num("lastMargin")))
}
