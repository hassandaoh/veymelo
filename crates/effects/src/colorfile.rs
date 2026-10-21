//! Colour transforms as a list of steps — matrices, per-channel curves, lookup tables, ASC CDL
//! corrections, ranges, powers and logs — read from the files colourists exchange: `.cube`,
//! Autodesk `.3dl`, cineSpace `.csp`, Sony Imageworks `.spi1d` / `.spi3d` / `.spimtx`, ASC CDL
//! `.cc` / `.ccc` / `.cdl`, and the Academy Common LUT Format `.clf`. Every step runs both ways:
//! exactly for curves, matrices and CDLs, by searching for lookup tables.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use veymelo_color::space::{Curve, M3, apply as mat_apply, invert};

use crate::lut::{Between, Lut};

/// An ASC Colour Decision List correction: slope, offset and power for each channel, then
/// saturation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cdl {
    pub slope: [f64; 3],
    pub offset: [f64; 3],
    pub power: [f64; 3],
    pub sat: f64,
}

impl Default for Cdl {
    fn default() -> Cdl {
        Cdl { slope: [1.0; 3], offset: [0.0; 3], power: [1.0; 3], sat: 1.0 }
    }
}

const LUMA: [f64; 3] = [0.2126, 0.7152, 0.0722];

impl Cdl {
    /// The correction (ASC CDL 1.2: clamped to 0 to 1 when `clamp`; else powers mirrored below
    /// zero).
    pub fn apply(&self, c: [f64; 3], clamp: bool) -> [f64; 3] {
        let o = [0, 1, 2].map(|i| {
            let v = c[i] * self.slope[i] + self.offset[i];
            if clamp { v.clamp(0.0, 1.0).powf(self.power[i]) } else { v.signum() * v.abs().powf(self.power[i]) }
        });
        let l = o[0] * LUMA[0] + o[1] * LUMA[1] + o[2] * LUMA[2];
        let o = o.map(|v| l + self.sat * (v - l));
        if clamp { o.map(|v| v.clamp(0.0, 1.0)) } else { o }
    }

    /// The correction undone.
    pub fn undo(&self, c: [f64; 3], clamp: bool) -> [f64; 3] {
        let c = if clamp { c.map(|v| v.clamp(0.0, 1.0)) } else { c };
        let l = c[0] * LUMA[0] + c[1] * LUMA[1] + c[2] * LUMA[2];
        let s = if self.sat.abs() < 1e-9 { 1e-9 } else { self.sat };
        let o = c.map(|v| l + (v - l) / s);
        let r = [0, 1, 2].map(|i| {
            let v = if clamp { o[i].clamp(0.0, 1.0) } else { o[i] };
            let u = v.signum() * v.abs().powf(1.0 / self.power[i].max(1e-9));
            let sl = if self.slope[i].abs() < 1e-9 { 1e-9 } else { self.slope[i] };
            (u - self.offset[i]) / sl
        });
        if clamp { r.map(|v| v.clamp(0.0, 1.0)) } else { r }
    }
}

/// The power curve with a straight foot ("moncurve"): forward from written to linear, as sRGB's
/// curve is with `gamma` 2.4 and `offset` 0.055.
pub fn moncurve(v: f64, gamma: f64, offset: f64, forward: bool, mirror: bool) -> f64 {
    let (g, o) = (gamma.max(1.0 + 1e-9), offset.max(1e-9));
    let brk = o / (g - 1.0);
    let slope = (g - 1.0) / o * (o * g / ((g - 1.0) * (1.0 + o))).powf(g);
    let (s, a) = if mirror { (v.signum(), v.abs()) } else { (1.0, v) };
    let r = if forward {
        if a > brk { ((a + o) / (1.0 + o)).powf(g) } else { a * slope }
    } else if a > brk * slope {
        (1.0 + o) * a.powf(1.0 / g) - o
    } else {
        a / slope
    };
    s * r
}

/// One step of a colour transform.
#[derive(Clone, Debug)]
pub enum Step {
    /// Through a 3 × 3 matrix, then an offset added.
    Matrix { m: M3, offset: [f64; 3] },
    /// Each channel through its own curve of points (x rising), straight between them.
    Shaper([Vec<(f32, f32)>; 3]),
    /// Through a lookup table.
    Lut(Arc<Lut>),
    Cdl { cdl: Cdl, clamp: bool },
    /// From `lo_in`..`hi_in` to `lo_out`..`hi_out`, clamped there when `clamp`.
    Range { lo_in: [f64; 3], hi_in: [f64; 3], lo_out: [f64; 3], hi_out: [f64; 3], clamp: bool },
    /// A named curve: linear to written (`encode`) or back.
    Curve { curve: Curve, encode: bool },
    /// v to the power `e` (mirrored below zero, else clamped to zero).
    Power { e: [f64; 3], mirror: bool },
    MonCurve { gamma: [f64; 3], offset: [f64; 3], forward: bool, mirror: bool },
    /// Linear to logarithmic: log_slope · log_base(lin_slope · x + lin_offset) + log_offset
    /// (`to_log`), or back; below `brk` (when given) a straight line that meets it smoothly.
    Log { base: f64, log_slope: [f64; 3], log_offset: [f64; 3], lin_slope: [f64; 3], lin_offset: [f64; 3], brk: Option<[f64; 3]>, to_log: bool },
    /// Moved by `shift`, then held from `lo` to `hi` (back: only moved back).
    Clamp { shift: [f64; 3], lo: [f64; 3], hi: [f64; 3] },
    /// Another step run the other way.
    Back(Box<Step>),
    /// Colours past the gamut's edges squeezed in (see `compress_gamut_with`).
    GamutCompress { limit: [f64; 3], threshold: [f64; 3], power: f64 },
    /// Red, green, blue to hue (0 to 1), saturation and value (`to`), or back.
    Hsv { to: bool },
    /// CIE XYZ to its chromaticity x, y and its Y (`to`), or back.
    XyY { to: bool },
    /// Light of any brightness rolled smoothly under 1 (a fit of the ACES film look: Krzysztof
    /// Narkowicz, 2015), on each channel.
    ToneMap,
    /// An ACES output transform: ACES2065-1 to a display's CIE XYZ (or back).
    Aces(veymelo_color::aces::Output),
    /// OpenColorIO's grading transforms.
    GradingPrimary(Box<crate::ocio::grading::Primary>),
    GradingTone(Box<crate::ocio::grading::Tone>),
    GradingCurves(Box<crate::ocio::grading::Curves>),
}

/// A curve of points read at `x` (straight past its ends from the last two points' line kept
/// level: the end value).
fn shape(curve: &[(f32, f32)], x: f32) -> f32 {
    if curve.len() < 2 {
        return x;
    }
    let i = curve.partition_point(|p| p.0 < x).clamp(1, curve.len() - 1);
    let (a, b) = (curve[i - 1], curve[i]);
    let t = ((x - a.0) / (b.0 - a.0).max(1e-12)).clamp(0.0, 1.0);
    a.1 + (b.1 - a.1) * t
}

/// The x a rising or falling curve reaches `y` at.
fn shape_back(curve: &[(f32, f32)], y: f32) -> f32 {
    let mut flipped: Vec<(f32, f32)> = curve.iter().map(|&(a, b)| (b, a)).collect();
    if flipped.first().zip(flipped.last()).is_some_and(|(f, l)| f.0 > l.0) {
        flipped.reverse();
    }
    shape(&flipped, y)
}

impl Step {
    fn log_parts(&self, i: usize) -> Option<(f64, f64, f64, f64, f64, Option<f64>)> {
        match self {
            Step::Log { base, log_slope, log_offset, lin_slope, lin_offset, brk, .. } => Some((*base, log_slope[i], log_offset[i], lin_slope[i], lin_offset[i], brk.map(|b| b[i]))),
            _ => None,
        }
    }

    /// A colour through the step (or back through it).
    pub fn run(&self, c: [f32; 3], between: Between, back: bool) -> [f32; 3] {
        let d = c.map(|v| v as f64);
        let out: [f64; 3] = match self {
            Step::Matrix { m, offset } => {
                if back {
                    mat_apply(&invert(m), [d[0] - offset[0], d[1] - offset[1], d[2] - offset[2]])
                } else {
                    let r = mat_apply(m, d);
                    [r[0] + offset[0], r[1] + offset[1], r[2] + offset[2]]
                }
            }
            Step::Shaper(s) => return [0, 1, 2].map(|i| if back { shape_back(&s[i], c[i]) } else { shape(&s[i], c[i]) }),
            Step::Lut(lut) => return if back { lut_back(lut, c, between) } else { lut.apply(c, between) },
            Step::Back(inner) => return inner.run(c, between, !back),
            Step::Clamp { shift, lo, hi } => [0, 1, 2].map(|i| if back { d[i] - shift[i] } else { (d[i] + shift[i]).clamp(lo[i], hi[i]) }),
            Step::GamutCompress { limit, threshold, power } => veymelo_color::space::compress_gamut_with(d, *limit, *threshold, *power, back),
            Step::Hsv { to } => {
                if *to != back {
                    let (h, s, v) = veymelo_color::rgb_to_hsv(d[0], d[1], d[2]);
                    [h / 360.0, s, v]
                } else {
                    let (r, g, b) = veymelo_color::hsv_to_rgb(d[0] * 360.0, d[1], d[2]);
                    [r, g, b]
                }
            }
            Step::XyY { to } => {
                if *to != back {
                    let sum = d[0] + d[1] + d[2];
                    if sum.abs() < 1e-12 { [0.3127, 0.3290, 0.0] } else { [d[0] / sum, d[1] / sum, d[1]] }
                } else {
                    let y = if d[1].abs() < 1e-12 { 1e-12 } else { d[1] };
                    [d[0] * d[2] / y, d[2], (1.0 - d[0] - d[1]) * d[2] / y]
                }
            }
            Step::ToneMap => d.map(|v| if back { tone_back(v) } else { tone(v) }),
            Step::GradingPrimary(p) => p.apply(d, back),
            Step::GradingTone(t) => {
                if back {
                    crate::ocio::grading::undo(&|c| t.forward(c), d)
                } else {
                    t.forward(d)
                }
            }
            Step::GradingCurves(c) => {
                if back {
                    crate::ocio::grading::undo(&|v| c.forward(v), d)
                } else {
                    c.forward(d)
                }
            }
            Step::Aces(out) => {
                if back {
                    out.back(d).unwrap_or([0.0; 3])
                } else {
                    out.apply(d)
                }
            }
            Step::Cdl { cdl, clamp } => {
                if back {
                    cdl.undo(d, *clamp)
                } else {
                    cdl.apply(d, *clamp)
                }
            }
            Step::Range { lo_in, hi_in, lo_out, hi_out, clamp } => [0, 1, 2].map(|i| {
                let (a, b, p, q) = if back { (lo_out[i], hi_out[i], lo_in[i], hi_in[i]) } else { (lo_in[i], hi_in[i], lo_out[i], hi_out[i]) };
                let v = if (b - a).abs() < 1e-12 { p } else { p + (d[i] - a) * (q - p) / (b - a) };
                if *clamp { v.clamp(p.min(q), p.max(q)) } else { v }
            }),
            Step::Curve { curve, encode } => d.map(|v| if *encode != back { curve.encode(v) } else { curve.decode(v) }),
            Step::Power { e, mirror } => [0, 1, 2].map(|i| {
                let k = if back { 1.0 / e[i].max(1e-9) } else { e[i] };
                if *mirror { d[i].signum() * d[i].abs().powf(k) } else { d[i].max(0.0).powf(k) }
            }),
            Step::MonCurve { gamma, offset, forward, mirror } => [0, 1, 2].map(|i| moncurve(d[i], gamma[i], offset[i], *forward != back, *mirror)),
            Step::Log { to_log, .. } => [0, 1, 2].map(|i| {
                let (base, ls, lo, ms, mo, brk) = self.log_parts(i).expect("a log");
                let ln_b = base.max(1.000_001).ln();
                // the straight line below the break, as steep as the curve there
                let line = brk.map(|b| {
                    let at = ls * ((ms * b + mo).max(1e-30).ln() / ln_b) + lo;
                    (b, at, ls * ms / ((ms * b + mo).max(1e-30) * ln_b))
                });
                if *to_log != back {
                    match line {
                        Some((b, at, k)) if d[i] <= b => at + k * (d[i] - b),
                        _ => ls * ((ms * d[i] + mo).max(1e-30).ln() / ln_b) + lo,
                    }
                } else {
                    match line {
                        Some((b, at, k)) if d[i] <= at => b + (d[i] - at) / k,
                        _ => ((base.powf((d[i] - lo) / ls)) - mo) / ms,
                    }
                }
            }),
        };
        out.map(|v| v as f32)
    }
}

/// The ACES fit: x (2.51 x + 0.03) / (x (2.43 x + 0.59) + 0.14), its input scaled by 0.6, held
/// under 1; straight below zero.
fn tone(v: f64) -> f64 {
    if v <= 0.0 {
        return v * 0.03 / 0.14 * 0.6;
    }
    let x = v * 0.6;
    (x * (2.51 * x + 0.03) / (x * (2.43 * x + 0.59) + 0.14)).min(1.0)
}

/// `tone` undone (the root of y (2.43 x² + 0.59 x + 0.14) = 2.51 x² + 0.03 x that is not
/// negative).
fn tone_back(y: f64) -> f64 {
    if y <= 0.0 {
        return y / (0.03 / 0.14 * 0.6);
    }
    let y = y.min(0.9999);
    let (a, b, c) = (2.43 * y - 2.51, 0.59 * y - 0.03, 0.14 * y);
    let x = (-b - (b * b - 4.0 * a * c).max(0.0).sqrt()) / (2.0 * a);
    x / 0.6
}

/// The colour a table turns into `c`: each channel's 1D table searched (it rises or falls), the
/// lattice by Newton's method from `c`.
fn lut_back(lut: &Lut, c: [f32; 3], between: Between) -> [f32; 3] {
    let mut x = c;
    if lut.size3 > 1 {
        let three = Lut { size1: 0, one: Vec::new(), ..lut.clone() };
        let f = |p: [f32; 3]| three.apply(p, between);
        let mut p = c;
        for _ in 0..30 {
            let y = f(p);
            let e = [c[0] - y[0], c[1] - y[1], c[2] - y[2]];
            if e.iter().all(|v| v.abs() < 1e-6) {
                break;
            }
            // the table's slope here, by small steps, and the step that undoes the error
            let h = 1e-3;
            let mut j = [[0f64; 3]; 3];
            for (k, row) in (0..3).map(|k| (k, f({
                let mut q = p;
                q[k] += h;
                q
            }))) {
                for i in 0..3 {
                    j[i][k] = ((row[i] - y[i]) / h) as f64;
                }
            }
            let det = j[0][0] * (j[1][1] * j[2][2] - j[1][2] * j[2][1]) - j[0][1] * (j[1][0] * j[2][2] - j[1][2] * j[2][0]) + j[0][2] * (j[1][0] * j[2][1] - j[1][1] * j[2][0]);
            let dx = if det.abs() > 1e-9 { mat_apply(&invert(&j), e.map(|v| v as f64)) } else { e.map(|v| v as f64) };
            p = [0, 1, 2].map(|i| (p[i] as f64 + dx[i]).clamp(lut.lo[i] as f64, lut.hi[i] as f64) as f32);
        }
        x = p;
    }
    if lut.size1 > 1 {
        x = [0, 1, 2].map(|i| {
            let curve: Vec<(f32, f32)> = (0..lut.size1).map(|k| (lut.lo[i] + (lut.hi[i] - lut.lo[i]) * k as f32 / (lut.size1 - 1) as f32, lut.one[k][i])).collect();
            shape_back(&curve, x[i])
        });
    }
    x
}

/// A colour transform: steps one after another.
#[derive(Clone, Debug, Default)]
pub struct Transform {
    pub steps: Vec<Step>,
    /// What the file asked for that is not done.
    pub skipped: Vec<String>,
}

impl Transform {
    pub fn of(steps: Vec<Step>) -> Transform {
        Transform { steps, skipped: Vec::new() }
    }

    /// A colour through every step (or back through them, last first).
    pub fn run(&self, c: [f32; 3], between: Between, back: bool) -> [f32; 3] {
        if back { self.steps.iter().rev().fold(c, |c, s| s.run(c, between, true)) } else { self.steps.iter().fold(c, |c, s| s.run(c, between, false)) }
    }
}

// ---------------------------------------------------------------- a small XML reader

/// An XML element: its name (without a namespace), attributes, the text right in it and the
/// elements in it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<Element>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Every element named so, at any depth (this one first).
    pub fn all<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>) {
        if self.name.eq_ignore_ascii_case(name) {
            out.push(self);
        }
        for c in &self.children {
            c.all(name, out);
        }
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The document's root element (comments, declarations and processing instructions skipped).
pub(crate) fn parse_xml(text: &str) -> Option<Element> {
    let mut stack: Vec<Element> = vec![Element::default()];
    let mut rest = text;
    while let Some(lt) = rest.find('<') {
        let before = &rest[..lt];
        if let Some(top) = stack.last_mut() {
            top.text.push_str(&unescape(before));
        }
        rest = &rest[lt..];
        if let Some(r) = rest.strip_prefix("<!--") {
            rest = &r[r.find("-->")? + 3..];
            continue;
        }
        if let Some(r) = rest.strip_prefix("<![CDATA[") {
            let end = r.find("]]>")?;
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&r[..end]);
            }
            rest = &r[end + 3..];
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            rest = &rest[rest.find('>')? + 1..];
            continue;
        }
        let gt = rest.find('>')?;
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        if let Some(name) = tag.strip_prefix('/') {
            let done = stack.pop()?;
            let name = name.trim().rsplit(':').next().unwrap_or("");
            if !done.name.eq_ignore_ascii_case(name) {
                return None;
            }
            stack.last_mut()?.children.push(done);
            continue;
        }
        let closed = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let name_end = tag.find(char::is_whitespace).unwrap_or(tag.len());
        let mut el = Element { name: tag[..name_end].rsplit(':').next().unwrap_or("").to_string(), ..Element::default() };
        let mut a = &tag[name_end..];
        while let Some(eq) = a.find('=') {
            let key = a[..eq].trim().rsplit(':').next().unwrap_or("").to_string();
            let v = a[eq + 1..].trim_start();
            let q = v.chars().next()?;
            if q != '"' && q != '\'' {
                return None;
            }
            let end = v[1..].find(q)? + 1;
            el.attrs.push((key, unescape(&v[1..end])));
            a = &v[end + 1..];
        }
        if closed {
            stack.last_mut()?.children.push(el);
        } else {
            stack.push(el);
        }
        if stack.len() > 64 {
            return None;
        }
    }
    let mut root = stack.pop()?;
    if !stack.is_empty() {
        return None;
    }
    root.children.pop()
}

fn numbers(s: &str) -> Vec<f64> {
    s.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).filter_map(|t| t.parse().ok()).collect()
}

fn three(s: &str) -> Option<[f64; 3]> {
    let v = numbers(s);
    match v.len() {
        1 => Some([v[0]; 3]),
        n if n >= 3 => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

// ---------------------------------------------------------------- the files

/// ASC CDL corrections from `.cc` / `.ccc` / `.cdl` XML, each with its id.
pub fn parse_cdl(text: &str) -> Result<Vec<(String, Cdl)>, String> {
    let root = parse_xml(text).ok_or("not readable XML")?;
    let mut found = Vec::new();
    root.all("ColorCorrection", &mut found);
    let list: Vec<(String, Cdl)> = found
        .iter()
        .map(|cc| {
            let sop = cc.child("SOPNode").unwrap_or(cc);
            let mut cdl = Cdl::default();
            if let Some(v) = sop.child("Slope").and_then(|e| three(&e.text)) {
                cdl.slope = v;
            }
            if let Some(v) = sop.child("Offset").and_then(|e| three(&e.text)) {
                cdl.offset = v;
            }
            if let Some(v) = sop.child("Power").and_then(|e| three(&e.text)) {
                cdl.power = v;
            }
            let sat = cc.child("SatNode").or_else(|| cc.child("SATNode")).unwrap_or(cc);
            if let Some(v) = sat.child("Saturation").and_then(|e| e.text.trim().parse().ok()) {
                cdl.sat = v;
            }
            (cc.attr("id").unwrap_or("").to_string(), cdl)
        })
        .collect();
    if list.is_empty() { Err("no ColorCorrection in it".into()) } else { Ok(list) }
}

/// A Sony Imageworks `.spimtx`: three rows of four (a matrix row and an offset in 16-bit code
/// values).
pub fn parse_spimtx(text: &str) -> Result<Transform, String> {
    let v = numbers(text);
    if v.len() != 12 || v.iter().any(|x| !x.is_finite()) {
        return Err(format!("a matrix needs 12 numbers, not {}", v.len()));
    }
    let m = [[v[0], v[1], v[2]], [v[4], v[5], v[6]], [v[8], v[9], v[10]]];
    Ok(Transform::of(vec![Step::Matrix { m, offset: [v[3] / 65535.0, v[7] / 65535.0, v[11] / 65535.0] }]))
}

/// A Sony Imageworks `.spi1d` table.
pub fn parse_spi1d(text: &str) -> Result<Transform, String> {
    let (mut from, mut len, mut comps) = ([0.0f32, 1.0], 0usize, 1usize);
    let mut values: Vec<f32> = Vec::new();
    let mut body = false;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if body {
            if line.starts_with('}') {
                break;
            }
            values.extend(line.split_whitespace().filter_map(|t| t.parse::<f32>().ok()));
            continue;
        }
        let mut t = line.split_whitespace();
        match t.next() {
            Some("{") => body = true,
            Some("From") => from = [t.next().and_then(|v| v.parse().ok()).ok_or("bad From")?, t.next().and_then(|v| v.parse().ok()).ok_or("bad From")?],
            Some("Length") => len = t.next().and_then(|v| v.parse().ok()).ok_or("bad Length")?,
            Some("Components") => comps = t.next().and_then(|v| v.parse().ok()).ok_or("bad Components")?,
            _ => {}
        }
    }
    if !(2..=1 << 20).contains(&len) || !matches!(comps, 1 | 3) || values.len() != len * comps || from[1] <= from[0] {
        return Err("the table's size, components or range do not agree".into());
    }
    let one = values.chunks_exact(comps).map(|c| if comps == 3 { [c[0], c[1], c[2]] } else { [c[0]; 3] }).collect();
    Ok(Transform::of(vec![Step::Lut(Arc::new(Lut { size1: len, one, size3: 0, three: Vec::new(), lo: [from[0]; 3], hi: [from[1]; 3] }))]))
}

/// A Sony Imageworks `.spi3d` lattice: `SPILUT 1.0`, `3 3`, the sizes, then `r g b  R G B`
/// lines (indices and colour).
pub fn parse_spi3d(text: &str) -> Result<Transform, String> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    if !lines.next().is_some_and(|l| l.starts_with("SPILUT")) {
        return Err("not an SPILUT".into());
    }
    lines.next();
    let size: Vec<usize> = lines.next().map(|l| l.split_whitespace().filter_map(|t| t.parse().ok()).collect()).unwrap_or_default();
    let n = size.first().copied().unwrap_or(0);
    if size.len() != 3 || size.iter().any(|s| *s != n) || !(2..=129).contains(&n) {
        return Err("the lattice's size is not understood".into());
    }
    let mut three = vec![[0f32; 3]; n * n * n];
    let mut seen = 0;
    for line in lines {
        let v: Vec<&str> = line.split_whitespace().collect();
        let [r, g, b, cr, cg, cb] = v.as_slice() else { return Err(format!("\"{line}\" is not a lattice point")) };
        let idx: Option<(usize, usize, usize)> = (|| Some((r.parse().ok()?, g.parse().ok()?, b.parse().ok()?)))();
        let col: Option<[f32; 3]> = (|| Some([cr.parse().ok()?, cg.parse().ok()?, cb.parse().ok()?]))();
        let ((r, g, b), col) = idx.zip(col).ok_or_else(|| format!("\"{line}\" is not a lattice point"))?;
        if r >= n || g >= n || b >= n {
            return Err(format!("\"{line}\" is outside the lattice"));
        }
        three[r + g * n + b * n * n] = col;
        seen += 1;
    }
    if seen != n * n * n {
        return Err(format!("the lattice has {seen} points of {}", n * n * n));
    }
    Ok(Transform::of(vec![Step::Lut(Arc::new(Lut { size1: 0, one: Vec::new(), size3: n, three, lo: [0.0; 3], hi: [1.0; 3] }))]))
}

/// An Autodesk `.3dl` lattice: whole numbers, blue changing fastest, after an optional line of
/// the input mesh; the output's bits from a `Mesh` line or the largest value.
pub fn parse_3dl(text: &str) -> Result<Transform, String> {
    let mut mesh: Option<Vec<f64>> = None;
    let mut rows: Vec<[f64; 3]> = Vec::new();
    let mut out_bits: Option<u32> = None;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with('<') {
            continue;
        }
        if line.starts_with(|c: char| c.is_ascii_alphabetic()) {
            let t: Vec<&str> = line.split_whitespace().collect();
            if t[0].eq_ignore_ascii_case("mesh") && t.len() >= 3 {
                out_bits = t[2].parse().ok();
            }
            continue;
        }
        let v = numbers(line);
        // the input mesh: as many points as the lattice is wide (any count but three)
        if v.len() >= 2 && v.len() != 3 && mesh.is_none() && rows.is_empty() {
            mesh = Some(v);
        } else if v.len() == 3 {
            rows.push([v[0], v[1], v[2]]);
        } else {
            return Err(format!("\"{line}\" is not understood"));
        }
    }
    let cube = |k: usize| {
        let n = (k as f64).cbrt().round() as usize;
        n * n * n == k
    };
    // a three-point mesh looks like a row: tell them apart by the count
    if mesh.is_none() && !cube(rows.len()) && cube(rows.len() - 1) {
        mesh = Some(rows.remove(0).to_vec());
    }
    let n = mesh.as_ref().map_or_else(|| (rows.len() as f64).cbrt().round() as usize, Vec::len);
    if n < 2 || rows.len() != n * n * n {
        return Err(format!("{} rows do not make a lattice", rows.len()));
    }
    let most = rows.iter().flatten().fold(0f64, |m, v| m.max(*v));
    let top = match out_bits {
        Some(b) if (1..32).contains(&b) => (1u64 << b) as f64 - 1.0,
        _ => [255.0, 1023.0, 4095.0, 65535.0].into_iter().find(|m| most <= *m).unwrap_or(most.max(1.0)),
    };
    let mut three = vec![[0f32; 3]; n * n * n];
    for (k, r) in rows.iter().enumerate() {
        let (ri, gi, bi) = (k / (n * n), (k / n) % n, k % n);
        three[ri + gi * n + bi * n * n] = r.map(|v| (v / top) as f32);
    }
    Ok(Transform::of(vec![Step::Lut(Arc::new(Lut { size1: 0, one: Vec::new(), size3: n, three, lo: [0.0; 3], hi: [1.0; 3] }))]))
}

/// A cineSpace `.csp`: a curve for each channel (the pre-LUT), then a 1D table or a lattice.
pub fn parse_csp(text: &str) -> Result<Transform, String> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty()).peekable();
    if !lines.next().is_some_and(|l| l.starts_with("CSPLUTV100")) {
        return Err("not a CSPLUTV100".into());
    }
    let kind = lines.next().unwrap_or("").to_string();
    if lines.peek().is_some_and(|l| l.starts_with("BEGIN METADATA")) {
        for l in lines.by_ref() {
            if l.starts_with("END METADATA") {
                break;
            }
        }
    }
    let row = |l: Option<&str>| -> Vec<f32> { l.map(|l| l.split_whitespace().filter_map(|t| t.parse().ok()).collect()).unwrap_or_default() };
    let mut shaper: [Vec<(f32, f32)>; 3] = Default::default();
    for ch in shaper.iter_mut() {
        let n: usize = lines.next().and_then(|l| l.parse().ok()).ok_or("a pre-LUT count is missing")?;
        let (ins, outs) = (row(lines.next()), row(lines.next()));
        if ins.len() != n || outs.len() != n || n < 2 {
            return Err("a pre-LUT's points do not agree with its count".into());
        }
        *ch = ins.into_iter().zip(outs).collect();
    }
    fn take<'a>(lines: &mut impl Iterator<Item = &'a str>, k: usize) -> Result<Vec<[f32; 3]>, String> {
        (0..k)
            .map(|_| {
                let v: Vec<f32> = lines.next().map(|l| l.split_whitespace().filter_map(|t| t.parse().ok()).collect()).unwrap_or_default();
                if v.len() == 3 { Ok([v[0], v[1], v[2]]) } else { Err("a table row is not three numbers".to_string()) }
            })
            .collect()
    }
    let lut = if kind.starts_with("3D") {
        let sz = row(lines.next());
        if sz.len() != 3 || sz[0] != sz[1] || sz[1] != sz[2] || sz[0] < 2.0 || sz[0] > 256.0 {
            return Err("the lattice's size is not understood".into());
        }
        let n = sz[0] as usize;
        Lut { size1: 0, one: Vec::new(), size3: n, three: take(&mut lines, n * n * n)?, lo: [0.0; 3], hi: [1.0; 3] }
    } else {
        let n: usize = lines.next().and_then(|l| l.parse().ok()).ok_or("the table's size is missing")?;
        if !(2..=1 << 20).contains(&n) {
            return Err("the table's size is not understood".into());
        }
        Lut { size1: n, one: take(&mut lines, n)?, size3: 0, three: Vec::new(), lo: [0.0; 3], hi: [1.0; 3] }
    };
    Ok(Transform::of(vec![Step::Shaper(shaper), Step::Lut(Arc::new(lut))]))
}

/// What a CLF bit depth's 1.0 is written as.
fn depth_scale(d: Option<&str>) -> f64 {
    match d.unwrap_or("32f") {
        "8i" => 255.0,
        "10i" => 1023.0,
        "12i" => 4095.0,
        "16i" => 65535.0,
        _ => 1.0,
    }
}

/// An Academy Common LUT Format `.clf` (and its older `.ctf`-like ProcessList): matrices, 1D
/// and 3D tables, ranges, CDLs, exponents and logs, in their bit depths.
pub fn parse_clf(text: &str) -> Result<Transform, String> {
    let root = parse_xml(text).ok_or("not readable XML")?;
    if !root.name.eq_ignore_ascii_case("ProcessList") {
        return Err(format!("a {} where a ProcessList should be", root.name));
    }
    let mut t = Transform::default();
    for node in &root.children {
        let (si, so) = (depth_scale(node.attr("inBitDepth")), depth_scale(node.attr("outBitDepth")));
        let array = || node.child("Array").map(|a| (numbers(a.attr("dim").unwrap_or("")), numbers(&a.text)));
        match node.name.as_str() {
            "Matrix" => {
                let (dim, v) = array().ok_or("a Matrix without its Array")?;
                let cols = if dim.len() >= 2 { dim[1] as usize } else { 3 };
                if !(cols == 3 || cols == 4) || v.len() < 3 * cols {
                    return Err("a Matrix's Array is not 3 × 3 or 3 × 4".into());
                }
                let m = [0, 1, 2].map(|r| [0, 1, 2].map(|c| v[r * cols + c] * si / so));
                let offset = [0, 1, 2].map(|r| if cols == 4 { v[r * cols + 3] / so } else { 0.0 });
                t.steps.push(Step::Matrix { m, offset });
            }
            "LUT1D" | "InvLUT1D" => {
                if node.attr("halfDomain").is_some() {
                    t.skipped.push("a LUT1D over half-float values".into());
                    continue;
                }
                let (dim, v) = array().ok_or("a LUT1D without its Array")?;
                let (n, comps) = (dim.first().copied().unwrap_or(0.0) as usize, dim.get(1).copied().unwrap_or(1.0) as usize);
                if n < 2 || !matches!(comps, 1 | 3) || v.len() != n * comps {
                    return Err("a LUT1D's Array does not agree with its dimensions".into());
                }
                let one = v.chunks_exact(comps).map(|c| if comps == 3 { [c[0], c[1], c[2]] } else { [c[0]; 3] }.map(|x| (x / so) as f32)).collect();
                let lut = Arc::new(Lut { size1: n, one, size3: 0, three: Vec::new(), lo: [0.0; 3], hi: [1.0; 3] });
                t.steps.push(Step::Lut(lut));
                if node.name == "InvLUT1D" {
                    // an inverse table in a list: run it backwards by a pair of shapers
                    if let Some(Step::Lut(l)) = t.steps.pop() {
                        let curve = |i: usize| (0..l.size1).map(|k| (l.one[k][i], k as f32 / (l.size1 - 1) as f32)).collect::<Vec<_>>();
                        let mut s = [curve(0), curve(1), curve(2)];
                        for c in &mut s {
                            if c.first().zip(c.last()).is_some_and(|(f, z)| f.0 > z.0) {
                                c.reverse();
                            }
                        }
                        t.steps.push(Step::Shaper(s));
                    }
                }
            }
            "LUT3D" | "InvLUT3D" => {
                let (dim, v) = array().ok_or("a LUT3D without its Array")?;
                let n = dim.first().copied().unwrap_or(0.0) as usize;
                if !(2..=129).contains(&n) || v.len() != n * n * n * 3 {
                    return Err("a LUT3D's Array does not agree with its dimensions".into());
                }
                // CLF lists blue fastest; ours is red fastest
                let mut three = vec![[0f32; 3]; n * n * n];
                for (k, c) in v.as_chunks::<3>().0.iter().enumerate() {
                    let (ri, gi, bi) = (k / (n * n), (k / n) % n, k % n);
                    three[ri + gi * n + bi * n * n] = [c[0], c[1], c[2]].map(|x| (x / so) as f32);
                }
                let lut = Lut { size1: 0, one: Vec::new(), size3: n, three, lo: [0.0; 3], hi: [1.0; 3] };
                if node.name == "InvLUT3D" {
                    t.skipped.push("an inverse LUT3D is run forwards".into());
                }
                t.steps.push(Step::Lut(Arc::new(lut)));
            }
            "Range" => {
                let get = |k: &str| node.child(k).and_then(|e| e.text.trim().parse::<f64>().ok());
                let (lo_in, hi_in, lo_out, hi_out) = (get("minInValue"), get("maxInValue"), get("minOutValue"), get("maxOutValue"));
                let clamp = node.attr("style").is_none_or(|s| s.eq_ignore_ascii_case("clamp"));
                match (lo_in, hi_in, lo_out, hi_out) {
                    (Some(a), Some(b), Some(p), Some(q)) => t.steps.push(Step::Range { lo_in: [a / si; 3], hi_in: [b / si; 3], lo_out: [p / so; 3], hi_out: [q / so; 3], clamp }),
                    _ => {
                        // a floor, a ceiling or both, each where its in and out values are: moved
                        // by the floor's (else the ceiling's) difference, then held there
                        let shift = lo_in.zip(lo_out).map(|(a, p)| p / so - a / si).or_else(|| hi_in.zip(hi_out).map(|(b, q)| q / so - b / si)).unwrap_or(0.0);
                        let (lo, hi) = (lo_out.map_or(f64::NEG_INFINITY, |v| v / so), hi_out.map_or(f64::INFINITY, |v| v / so));
                        t.steps.push(Step::Clamp { shift: [shift; 3], lo: [lo; 3], hi: [hi; 3] });
                    }
                }
            }
            "ASC_CDL" => {
                let sop = node.child("SOPNode");
                let mut cdl = Cdl::default();
                if let Some(s) = sop {
                    cdl.slope = s.child("Slope").and_then(|e| three(&e.text)).unwrap_or(cdl.slope);
                    cdl.offset = s.child("Offset").and_then(|e| three(&e.text)).unwrap_or(cdl.offset);
                    cdl.power = s.child("Power").and_then(|e| three(&e.text)).unwrap_or(cdl.power);
                }
                if let Some(v) = node.child("SatNode").and_then(|s| s.child("Saturation")).and_then(|e| e.text.trim().parse().ok()) {
                    cdl.sat = v;
                }
                let style = node.attr("style").unwrap_or("Fwd").to_ascii_lowercase();
                let step = Step::Cdl { cdl, clamp: !style.contains("noclamp") };
                t.steps.push(if style.starts_with("rev") { Step::Back(Box::new(step)) } else { step });
            }
            "Exponent" => {
                let style = node.attr("style").unwrap_or("basicFwd").to_ascii_lowercase();
                let params: Vec<&Element> = node.children.iter().filter(|c| c.name == "ExponentParams").collect();
                let mut e = [1.0; 3];
                let mut off = [0.0; 3];
                for p in &params {
                    let ch: Vec<usize> = match p.attr("channel") {
                        Some("R") => vec![0],
                        Some("G") => vec![1],
                        Some("B") => vec![2],
                        _ => vec![0, 1, 2],
                    };
                    for i in ch {
                        e[i] = p.attr("exponent").and_then(|v| v.parse().ok()).unwrap_or(1.0);
                        off[i] = p.attr("offset").and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    }
                }
                let fwd = style.ends_with("fwd");
                let mirror = style.contains("mirror");
                let step = if style.starts_with("moncurve") {
                    Step::MonCurve { gamma: e, offset: off, forward: fwd, mirror }
                } else {
                    Step::Power { e: if fwd { e } else { e.map(|v| 1.0 / v.max(1e-9)) }, mirror: mirror || style.contains("passthru") }
                };
                t.steps.push(step);
            }
            "Log" => {
                let style = node.attr("style").unwrap_or("log10").to_ascii_lowercase();
                let simple = |base: f64, to_log: bool| Step::Log { base, log_slope: [1.0; 3], log_offset: [0.0; 3], lin_slope: [1.0; 3], lin_offset: [0.0; 3], brk: None, to_log };
                let step = match style.as_str() {
                    "log10" => simple(10.0, true),
                    "antilog10" => simple(10.0, false),
                    "log2" => simple(2.0, true),
                    "antilog2" => simple(2.0, false),
                    _ => {
                        let p = node.child("LogParams");
                        let num = |k: &str, d: f64| p.and_then(|p| p.attr(k)).and_then(|v| v.parse().ok()).unwrap_or(d);
                        let base = num("base", 2.0);
                        let brk = p.and_then(|p| p.attr("linSideBreak")).and_then(|v| v.parse().ok()).map(|b: f64| [b; 3]);
                        Step::Log {
                            base,
                            log_slope: [num("logSideSlope", 1.0); 3],
                            log_offset: [num("logSideOffset", 0.0); 3],
                            lin_slope: [num("linSideSlope", 1.0); 3],
                            lin_offset: [num("linSideOffset", 0.0); 3],
                            brk: if style.starts_with("camera") { brk } else { None },
                            to_log: style.contains("lintolog"),
                        }
                    }
                };
                t.steps.push(step);
            }
            "Description" | "InputDescriptor" | "OutputDescriptor" | "Info" => {}
            other => t.skipped.push(format!("a {other} step")),
        }
    }
    Ok(t)
}

/// A file's transform by its kind (from its extension or what is in it); for a CDL file with
/// several corrections, the one with id `which` (else the first).
pub fn parse(text: &str, ext: &str, which: &str) -> Result<Transform, String> {
    let ext = ext.to_ascii_lowercase();
    let head = text.trim_start();
    if matches!(ext.as_str(), "cc" | "ccc" | "cdl") || head.contains("<ColorCorrection") {
        let list = parse_cdl(text)?;
        let cdl = list.iter().find(|(id, _)| !which.is_empty() && id == which).or(list.first()).map(|(_, c)| *c).unwrap_or_default();
        return Ok(Transform::of(vec![Step::Cdl { cdl, clamp: true }]));
    }
    if ext == "clf" || head.contains("<ProcessList") {
        return parse_clf(text);
    }
    if ext == "csp" || head.starts_with("CSPLUTV100") {
        return parse_csp(text);
    }
    if ext == "spimtx" {
        return parse_spimtx(text);
    }
    if ext == "spi1d" || head.starts_with("Version") {
        return parse_spi1d(text);
    }
    if ext == "spi3d" || head.starts_with("SPILUT") {
        return parse_spi3d(text);
    }
    if ext == "cube" || text.contains("LUT_3D_SIZE") || text.contains("LUT_1D_SIZE") {
        return crate::lut::parse(text).map(|l| Transform::of(vec![Step::Lut(Arc::new(l))]));
    }
    parse_3dl(text)
}

type Kept = HashMap<(String, String), (Option<SystemTime>, Result<Arc<Transform>, String>)>;

/// The transform in file `path` (or the file's text given in its place), read once while the
/// file is unchanged.
pub fn load(path: &str, which: &str) -> Result<Arc<Transform>, String> {
    static KEPT: Mutex<Option<Kept>> = Mutex::new(None);
    let path = path.trim();
    if path.is_empty() {
        return Err("no file is named".into());
    }
    if path.contains('\n') {
        return parse(path, "", which).map(Arc::new);
    }
    let changed = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let key = (path.to_string(), which.to_string());
    if let Ok(kept) = KEPT.lock()
        && let Some((when, t)) = kept.as_ref().and_then(|k| k.get(&key))
        && *when == changed
    {
        return t.clone();
    }
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("");
    let t = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}")).and_then(|text| parse(&text, ext, which).map_err(|e| format!("{path}: {e}"))).map(Arc::new);
    if let Ok(mut kept) = KEPT.lock() {
        kept.get_or_insert_with(HashMap::new).insert(key, (changed, t.clone()));
    }
    t
}

#[cfg(test)]
mod tests;
