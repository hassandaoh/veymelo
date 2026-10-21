//! OpenColorIO's grading transforms (v2): GradingPrimary (brightness, contrast, gamma, offset,
//! exposure, lift, gain, pivots, saturation, clamps, as a colourist's primary controls for log,
//! scene-linear or video colours), GradingTone (blacks, shadows, midtones, highlights and whites,
//! each by its own smooth curve over its part of the range, and an S-contrast) and GradingRGBCurve
//! (a smooth curve through control points for each channel and one for all). The maths is
//! OpenColorIO's, as its CPU renderers do it (the BSD-3-Clause OpenColorIO project): the same
//! computed values, the same pieces of curve, the same order. A scene-linear tone or curve
//! grade works on log colours (OpenColorIO's own lin-to-log) and back.
//!
//! The tone and curve grades change each channel on its own and only ever raise it, so they are
//! undone channel by channel by searching for what makes the colour given.

use super::yaml::Yaml;

/// Which colours a grade is made for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Log,
    Lin,
    Video,
}

impl Style {
    pub fn of(text: &str) -> Style {
        match text.trim().to_ascii_lowercase().as_str() {
            "linear" | "lin" => Style::Lin,
            "video" => Style::Video,
            _ => Style::Log,
        }
    }
}

/// OpenColorIO's scene-linear to log, and back (for linear grades).
const XBRK: f64 = 0.004_131_837_473_948_395;
const SHIFT: f64 = -0.000_157_849_851_665_374;
const LGAIN: f64 = 363.034_608_563;
const LOFFS: f64 = -7.0;
const YBRK: f64 = -5.5;

fn lin_log(x: f64) -> f64 {
    if x > XBRK { ((x + SHIFT) / (0.18 + SHIFT)).log2() } else { x * LGAIN + LOFFS }
}

fn log_lin(y: f64) -> f64 {
    if y < YBRK { (y - LOFFS) / LGAIN } else { 2f64.powf(y) * (0.18 + SHIFT) - SHIFT }
}

/// Red, green, blue and a master value.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rgbm {
    rgb: [f64; 3],
    master: f64,
}

impl Rgbm {
    fn read(y: &Yaml, key: &str, d: f64) -> Rgbm {
        let Some(m) = y.get(key) else { return Rgbm { rgb: [d; 3], master: d } };
        let rgb = m.get("rgb").map(Yaml::nums).unwrap_or_default();
        let master = m.get("master").map(Yaml::nums).and_then(|v| v.first().copied()).unwrap_or(d);
        Rgbm { rgb: if rgb.len() >= 3 { [rgb[0], rgb[1], rgb[2]] } else { [d; 3] }, master }
    }
}

fn num(y: &Yaml, path: &[&str], d: f64) -> f64 {
    let mut at = y;
    for k in path {
        match at.get(k) {
            Some(n) => at = n,
            None => return d,
        }
    }
    at.nums().first().copied().unwrap_or(d)
}

const LUMA: [f64; 3] = [0.2126, 0.7152, 0.0722];

fn saturate(c: [f64; 3], sat: f64) -> [f64; 3] {
    if sat == 1.0 {
        return c;
    }
    let l = c[0] * LUMA[0] + c[1] * LUMA[1] + c[2] * LUMA[2];
    c.map(|v| l + sat * (v - l))
}

fn gamma(c: [f64; 3], g: [f64; 3], black: f64, white: f64) -> [f64; 3] {
    let r = white - black;
    std::array::from_fn(|i| {
        let d = c[i] - black;
        (d.abs() / r).powf(g[i]) * d.signum() * r + black
    })
}

/// A primary grade, its values worked out as OpenColorIO works them out.
#[derive(Clone, Debug, PartialEq)]
pub struct Primary {
    style: Style,
    brightness: [f64; 3],
    contrast: [f64; 3],
    gamma: [f64; 3],
    offset: [f64; 3],
    exposure: [f64; 3],
    slope: [f64; 3],
    pivot: f64,
    black: f64,
    white: f64,
    saturation: f64,
    clamp: (f64, f64),
}

impl Primary {
    pub fn read(y: &Yaml) -> Primary {
        let style = Style::of(y.text("style"));
        let pivot_raw = num(y, &["pivot", "contrast"], if style == Style::Log { -0.2 } else { 0.18 });
        let (black, white) = (num(y, &["pivot", "black"], 0.0), num(y, &["pivot", "white"], 1.0));
        let both = |r: Rgbm, f: &dyn Fn(f64, f64) -> f64| -> [f64; 3] { r.rgb.map(|v| f(v, r.master)) };
        let (brightness, contrast, gamma_v, offset, exposure, lift, gain) = (Rgbm::read(y, "brightness", 0.0), Rgbm::read(y, "contrast", 1.0), Rgbm::read(y, "gamma", 1.0), Rgbm::read(y, "offset", 0.0), Rgbm::read(y, "exposure", 0.0), Rgbm::read(y, "lift", 0.0), Rgbm::read(y, "gain", 1.0));
        let lift_total = both(lift, &|v, m| v + m);
        let gains = both(gain, &|v, m| if v * m == 0.0 { 1.0 } else { v * m });
        Primary {
            style,
            brightness: both(brightness, &|v, m| (v + m) * 6.25 / 1023.0),
            contrast: both(contrast, &|v, m| v * m),
            gamma: both(gamma_v, &|v, m| 1.0 / (v * m)),
            offset: match style {
                Style::Video => std::array::from_fn(|i| offset.rgb[i] + offset.master + lift_total[i]),
                _ => both(offset, &|v, m| v + m),
            },
            exposure: both(exposure, &|v, m| 2f64.powf(v + m)),
            slope: std::array::from_fn(|i| {
                let d = white / gains[i] + lift_total[i] - black;
                (white - black) / if d == 0.0 { 1.0 } else { d }
            }),
            pivot: if style == Style::Log { 0.5 + 0.5 * pivot_raw } else { 0.18 * 2f64.powf(pivot_raw) },
            black,
            white,
            saturation: y.get("saturation").map(Yaml::nums).and_then(|v| v.first().copied()).unwrap_or(1.0),
            clamp: (num(y, &["clamp", "black"], f64::MIN), num(y, &["clamp", "white"], f64::MAX)),
        }
    }

    pub fn apply(&self, c: [f64; 3], back: bool) -> [f64; 3] {
        let gamma_on = self.gamma.iter().any(|g| *g != 1.0);
        let clamp = |c: [f64; 3]| c.map(|v| v.max(self.clamp.0).min(self.clamp.1));
        if !back {
            let c = match self.style {
                Style::Log => {
                    let c: [f64; 3] = std::array::from_fn(|i| (c[i] + self.brightness[i] - self.pivot) * self.contrast[i] + self.pivot);
                    if gamma_on { gamma(c, self.gamma, self.black, self.white) } else { c }
                }
                Style::Lin => {
                    let c: [f64; 3] = std::array::from_fn(|i| (c[i] + self.offset[i]) * self.exposure[i]);
                    if self.contrast.iter().any(|v| *v != 1.0) { std::array::from_fn(|i| c[i].signum() * self.pivot.abs() * (c[i] / self.pivot).abs().powf(self.contrast[i])) } else { c }
                }
                Style::Video => {
                    let c: [f64; 3] = std::array::from_fn(|i| (c[i] + self.offset[i] - self.black) * self.slope[i] + self.black);
                    if gamma_on { gamma(c, self.gamma, self.black, self.white) } else { c }
                }
            };
            clamp(saturate(c, self.saturation))
        } else {
            let c = saturate(clamp(c), 1.0 / if self.saturation != 0.0 { self.saturation } else { 1.0 });
            let ungamma = |c: [f64; 3]| if gamma_on { gamma(c, self.gamma.map(|g| 1.0 / g), self.black, self.white) } else { c };
            match self.style {
                Style::Log => {
                    let c = ungamma(c);
                    std::array::from_fn(|i| {
                        let k = if self.contrast[i] == 0.0 { 1.0 } else { 1.0 / self.contrast[i] };
                        (c[i] - self.pivot) * k + self.pivot - self.brightness[i]
                    })
                }
                Style::Lin => {
                    let c: [f64; 3] = if self.contrast.iter().any(|v| *v != 1.0) { std::array::from_fn(|i| c[i].signum() * self.pivot.abs() * (c[i] / self.pivot).abs().powf(1.0 / self.contrast[i])) } else { c };
                    std::array::from_fn(|i| c[i] / self.exposure[i] - self.offset[i])
                }
                Style::Video => {
                    let c = ungamma(c);
                    std::array::from_fn(|i| (c[i] - self.black) / self.slope[i] + self.black - self.offset[i])
                }
            }
        }
    }
}

/// A part of the range a tone grade changes: red, green, blue and master, its start and width (or
/// centre, or pivot).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Zone {
    v: [f64; 4],
    start: f64,
    width: f64,
}

impl Zone {
    fn read(y: &Yaml, key: &str, start_key: &str, width_key: &str, d: (f64, f64)) -> Zone {
        let Some(z) = y.get(key) else { return Zone { v: [1.0; 4], start: d.0, width: d.1 } };
        let rgb = z.get("rgb").map(Yaml::nums).unwrap_or_default();
        let master = z.get("master").map(Yaml::nums).and_then(|v| v.first().copied()).unwrap_or(1.0);
        let one = |k: &str, d: f64| z.get(k).map(Yaml::nums).and_then(|v| v.first().copied()).unwrap_or(d);
        Zone { v: if rgb.len() >= 3 { [rgb[0], rgb[1], rgb[2], master] } else { [1.0, 1.0, 1.0, master] }, start: one(start_key, d.0), width: one(width_key, d.1) }
    }
}

/// A curve's knots: where they are across and up, and its slopes there.
type Knots<const N: usize, const M: usize> = ([f64; N], [f64; N], [f64; M]);

/// A tone grade's curves, worked out as OpenColorIO works them out.
#[derive(Clone, Debug, PartialEq)]
pub struct Tone {
    style: Style,
    zones: [Zone; 5],
    scontrast: f64,
    pivot: f64,
    mids: [Knots<6, 6>; 4],
    hs: [[Knots<3, 2>; 4]; 2],
    wb: [[(Knots<2, 2>, f64); 4]; 2],
    sc: [([f64; 4], [f64; 4], [f64; 2]); 2],
    identity: bool,
}

/// The faux-cubic curve from `x0` to `x2` (y = x at both) with slopes `m0` and `m2`: forward, or
/// back.
fn faux_cubic(t: f64, x0: f64, x2: f64, m0: f64, m2: f64, back: bool) -> f64 {
    let (y0, y2) = (x0, x2);
    let x1 = x0 + 0.5 * (x2 - x0);
    let y1 = (0.5 / ((x2 - x1) + (x1 - x0))) * ((2.0 * y0 + m0 * (x1 - x0)) * (x2 - x1) + (2.0 * y2 - m2 * (x2 - x1)) * (x1 - x0));
    if !back {
        let (tl, tr) = ((t - x0) / (x1 - x0), (t - x1) / (x2 - x1));
        let fl = y0 * (1.0 - tl * tl) + y1 * tl * tl + m0 * (1.0 - tl) * tl * (x1 - x0);
        let fr = y1 * (1.0 - tr) * (1.0 - tr) + y2 * (2.0 - tr) * tr + m2 * (tr - 1.0) * tr * (x2 - x1);
        let mut res = if t < x1 { fl } else { fr };
        if t < x0 {
            res = y0 + (t - x0) * m0;
        }
        if t > x2 {
            res = y2 + (t - x2) * m2;
        }
        res
    } else {
        let (cl, bl, al) = (y0 - t, m0 * (x1 - x0), y1 - y0 - m0 * (x1 - x0));
        let outl = 2.0 * cl / (-(bl * bl - 4.0 * al * cl).sqrt() - bl) * (x1 - x0) + x0;
        let (cr, br, ar) = (y1 - t, 2.0 * y2 - 2.0 * y1 - m2 * (x2 - x1), y1 - y2 + m2 * (x2 - x1));
        let outr = 2.0 * cr / (-(br * br - 4.0 * ar * cr).sqrt() - br) * (x2 - x1) + x1;
        let mut res = if t < y1 { outl } else { outr };
        if t < y0 {
            res = x0 + (t - y0) / m0;
        }
        if t > y2 {
            res = x2 + (t - y2) / m2;
        }
        res
    }
}

fn highlight_eval(t: f64, start: f64, pivot: f64, val: f64) -> f64 {
    let v = 2.0 - val;
    if v <= 1.0 { faux_cubic(t, start, pivot, 1.0, 0.01f64.max(v), false) } else { faux_cubic(t, start, pivot, 1.0, 0.01f64.max(val), true) }
}

fn shadow_eval(t: f64, start: f64, pivot: f64, val: f64) -> f64 {
    if val <= 1.0 { faux_cubic(t, start, pivot, 0.01f64.max(val), 1.0, false) } else { faux_cubic(t, start, pivot, 0.01f64.max(2.0 - val), 1.0, true) }
}

impl Tone {
    pub fn read(y: &Yaml) -> Tone {
        let style = Style::of(y.text("style"));
        let d: [(f64, f64); 5] = match style {
            Style::Lin => [(0.0, 4.0), (2.0, -7.0), (0.0, 8.0), (-2.0, 9.0), (0.0, 8.0)],
            Style::Log => [(0.4, 0.4), (0.5, 0.0), (0.4, 0.6), (0.3, 1.0), (0.4, 0.5)],
            Style::Video => [(0.4, 0.4), (0.6, 0.0), (0.4, 0.7), (0.2, 1.0), (0.5, 0.5)],
        };
        let zones = [
            Zone::read(y, "blacks", "start", "width", d[0]),
            Zone::read(y, "shadows", "start", "pivot", d[1]),
            Zone::read(y, "midtones", "center", "width", d[2]),
            Zone::read(y, "highlights", "start", "pivot", d[3]),
            Zone::read(y, "whites", "start", "width", d[4]),
        ];
        let scontrast = y.get("s_contrast").map(Yaml::nums).and_then(|v| v.first().copied()).unwrap_or(1.0);
        let mut t = Tone { style, zones, scontrast, pivot: 0.0, mids: [([0.0; 6], [0.0; 6], [0.0; 6]); 4], hs: [[([0.0; 3], [0.0; 3], [0.0; 2]); 4]; 2], wb: [[(([0.0; 2], [0.0; 2], [0.0; 2]), 1.0); 4]; 2], sc: [([0.0; 4], [0.0; 4], [0.0; 2]); 2], identity: false };
        t.prepare();
        t
    }

    fn prepare(&mut self) {
        let (top, top_sc, bottom, pivot) = match self.style {
            Style::Lin => (7.5, 6.5, -5.5, 0.0),
            _ => (1.0, 1.0, 0.0, 0.4),
        };
        self.pivot = pivot;
        let [blacks, shadows, mids, highs, whites] = self.zones;
        self.identity = self.zones.iter().all(|z| z.v == [1.0; 4]) && self.scontrast == 1.0;
        if self.identity {
            return;
        }
        // the highlights' and shadows' ranges, and the whites' and blacks' moved through them
        let (h_start, h_width) = (highs.start.min(highs.width - 0.01), highs.width);
        let ws = highlight_eval(whites.start, h_start, h_width, highs.v[3]);
        let we = highlight_eval(whites.start + whites.width, h_start, h_width, highs.v[3]);
        let (whites_start, whites_width) = (ws, we - ws);
        let (s_start, s_width) = (shadows.start.max(shadows.width + 0.01), shadows.width);
        let bs = shadow_eval(blacks.start, s_width, s_start, shadows.v[3]);
        let be = shadow_eval(blacks.start - blacks.width, s_width, s_start, shadows.v[3]);
        let (blacks_start, blacks_width) = (bs, bs - be);
        // midtones
        for ch in 0..4 {
            let a = mids.v[ch].clamp(0.01, 1.99);
            if a == 1.0 {
                continue;
            }
            let (x0, x5) = (bottom, top);
            let w = mids.width.clamp(0.01, (x5 - x0) * 0.95);
            let c = mids.start.clamp(x0 + 0.51 * w, x5 - 0.51 * w);
            let x1 = c - 0.5 * w;
            let x4 = x1 + w;
            let x2 = x1 + 0.25 * (x4 - x1);
            let x3 = x1 + 0.75 * (x4 - x1);
            let (y0, m0, m5) = (x0, 1.0, 1.0);
            let k = (a - 1.0) * (1.0 - 0.1);
            let (m2, m3) = (1.0 + k, 1.0 - k);
            let (mut m1, mut m4) = (1.0 + k * 0.4, 1.0 - k * 0.4);
            if c <= (x5 + x0) / 2.0 {
                let area = (x1 - x0) * (m1 - m0) / 2.0 + (x2 - x1) * ((m1 - m0) + (m2 - m1) / 2.0) + (c - x2) * (m2 - m0) / 2.0;
                m4 = (-0.5 * (x5 - x4) * m5 + (x4 - x3) * (0.5 * m3 - m5) + (x3 - c) * (m3 - m5) / 2.0 + area) / (-0.5 * (x5 - x3));
            } else {
                let area = (x5 - x4) * (m4 - m5) / 2.0 + (x4 - x3) * ((m4 - m5) + (m3 - m4) / 2.0) + (x3 - c) * (m3 - m5) / 2.0;
                m1 = (-0.5 * (x1 - x0) * m0 + (x2 - x1) * (0.5 * m2 - m0) + (c - x2) * (m2 - m0) / 2.0 + area) / (-0.5 * (x2 - x0));
            }
            let y1 = y0 + (m0 + m1) * (x1 - x0) / 2.0;
            let y2 = y1 + (m1 + m2) * (x2 - x1) / 2.0;
            let y3 = y2 + (m2 + m3) * (x3 - x2) / 2.0;
            let y4 = y3 + (m3 + m4) * (x4 - x3) / 2.0;
            let y5 = y4 + (m4 + m5) * (x5 - x4) / 2.0;
            self.mids[ch] = ([x0, x1, x2, x3, x4, x5], [y0, y1, y2, y3, y4, y5], [m0, m1, m2, m3, m4, m5]);
        }
        // highlights and shadows
        for (idx, shadow) in [(0usize, false), (1, true)] {
            for ch in 0..4 {
                let val = if shadow { shadows.v[ch] } else { 2.0 - highs.v[ch] };
                if val == 1.0 {
                    continue;
                }
                let (start, pivot) = if shadow { (s_start, s_width) } else { (h_start, h_width) };
                let (x0, x2) = if shadow { (pivot, start) } else { (start, pivot) };
                let (y0, y2, x1) = (x0, x2, x0 + 0.5 * (x2 - x0));
                let (m0, m2) = if val < 1.0 { if shadow { (0.01f64.max(val), 1.0) } else { (1.0, 0.01f64.max(val)) } } else if shadow { (0.01f64.max(2.0 - val), 1.0) } else { (1.0, 0.01f64.max(2.0 - val)) };
                let y1 = (0.5 / ((x2 - x1) + (x1 - x0))) * ((2.0 * y0 + m0 * (x1 - x0)) * (x2 - x1) + (2.0 * y2 - m2 * (x2 - x1)) * (x1 - x0));
                self.hs[idx][ch] = ([x0, x1, x2], [y0, y1, y2], [m0, m2]);
            }
        }
        // whites and blacks
        for (idx, black) in [(0usize, false), (1, true)] {
            for ch in 0..4 {
                let (start, width) = if black { (blacks_start, blacks_width) } else { (whites_start, whites_width) };
                let val = if black { blacks.v[ch] } else { whites.v[ch] };
                let x0 = if black { start - width } else { start };
                let x1 = if black { start } else { x0 + width };
                let t = if black { 2.0 - val } else { val };
                let (mut m0, mut m1, mut y0, mut y1, mut gain) = (1.0, 1.0, x0, x1, 1.0);
                if t < 1.0 {
                    if black {
                        m0 = 0.01f64.max(2.0 - val);
                        y1 = x1;
                        y0 = y1 - (m0 + m1) * (x1 - x0) / 2.0;
                    } else {
                        m1 = 0.01f64.max(val);
                        y0 = x0;
                        y1 = y0 + (m0 + m1) * (x1 - x0) / 2.0;
                    }
                } else if t > 1.0 {
                    if black {
                        m0 = 0.01f64.max(val);
                        y1 = x1;
                        y0 = y1 - (m0 + m1) * (x1 - x0) / 2.0;
                    } else {
                        m1 = 0.01f64.max(2.0 - val);
                        y0 = x0;
                    }
                    gain = (m0 + m1) / 2.0;
                }
                self.wb[idx][ch] = (([x0, x1], [y0, y1], [m0, m1]), gain);
            }
        }
        // the S-contrast's ends
        if self.scontrast != 1.0 {
            let sc = self.scontrast;
            let c = if sc > 1.0 { 1.0 / (1.8125 - 0.8125 * sc.min(1.99)) } else { 0.28125 + 0.71875 * sc.max(0.01) };
            // top
            let (x3, y3) = (top_sc, top_sc);
            let y0 = pivot + (y3 - pivot) * 0.25;
            let m0 = c;
            let x0 = pivot + (y0 - pivot) / m0;
            let min_w = (x3 - x0) * 0.3;
            let mut m3 = 1.0 / m0;
            let center = (y3 - y0 - m3 * x3 + m0 * x0) / (m0 - m3);
            let (mut x1, mut x2) = (x0, 2.0 * center - x0);
            if x2 > x3 {
                x2 = x3;
                x1 = 2.0 * center - x2;
            } else if x2 - x1 < min_w {
                x2 = x1 + min_w;
                let nc = (x1 + x2) / 2.0;
                m3 = (y3 - y0 + m0 * x0 - nc * m0) / (x3 - nc);
            }
            let y1 = y0;
            let y2 = y1 + (m0 + m3) * (x2 - x1) / 2.0;
            self.sc[0] = ([x0, x1, x2, x3], [y0, y1, y2, y3], [m0, m3]);
            // bottom
            let (x0, y0) = (bottom, bottom);
            let y3 = pivot - (pivot - y0) * 0.25;
            let m3 = c;
            let x3 = pivot - (pivot - y3) / m3;
            let min_w = (x3 - x0) * 0.3;
            let mut m0 = 1.0 / m3;
            let center = (y3 - y0 - m3 * x3 + m0 * x0) / (m0 - m3);
            let (mut x1, mut x2) = (2.0 * center - x3, x3);
            if x1 < x0 {
                x1 = x0;
                x2 = 2.0 * center - x1;
            } else if x2 - x1 < min_w {
                x1 = x2 - min_w;
                let nc = (x1 + x2) / 2.0;
                m0 = (y3 - y0 - m3 * x3 + nc * m3) / (nc - x0);
            }
            let y2 = y3;
            let y1 = y2 - (m0 + m3) * (x2 - x1) / 2.0;
            self.sc[1] = ([x0, x1, x2, x3], [y0, y1, y2, y3], [m0, m3]);
        }
    }

    fn mids(&self, ch: usize, t: f64) -> f64 {
        let a = self.zones[2].v[ch].clamp(0.01, 1.99);
        if a == 1.0 {
            return t;
        }
        let (x, y, m) = &self.mids[ch];
        let seg = |k: usize| {
            let tk = (t - x[k]) / (x[k + 1] - x[k]);
            tk * (x[k + 1] - x[k]) * (tk * 0.5 * (m[k + 1] - m[k]) + m[k]) + y[k]
        };
        let mut res = if t < x[1] { seg(0) } else { seg(1) };
        if t > x[2] {
            res = seg(2);
        }
        if t > x[3] {
            res = seg(3);
        }
        if t > x[4] {
            res = seg(4);
        }
        if t < x[0] {
            res = y[0] + (t - x[0]) * m[0];
        }
        if t > x[5] {
            res = y[5] + (t - x[5]) * m[5];
        }
        res
    }

    fn highlight_shadow(&self, shadow: bool, ch: usize, t: f64) -> f64 {
        let val = if shadow { self.zones[1].v[ch] } else { 2.0 - self.zones[3].v[ch] };
        if val == 1.0 {
            return t;
        }
        let ([x0, x1, x2], [y0, y1, y2], [m0, m2]) = self.hs[shadow as usize][ch];
        if val < 1.0 {
            let (tl, tr) = ((t - x0) / (x1 - x0), (t - x1) / (x2 - x1));
            let fl = y0 * (1.0 - tl * tl) + y1 * tl * tl + m0 * (1.0 - tl) * tl * (x1 - x0);
            let fr = y1 * (1.0 - tr) * (1.0 - tr) + y2 * (2.0 - tr) * tr + m2 * (tr - 1.0) * tr * (x2 - x1);
            let mut res = if t < x1 { fl } else { fr };
            if t < x0 {
                res = (t - x0) * m0 + y0;
            }
            if t >= x2 {
                res = (t - x2) * m2 + y2;
            }
            res
        } else {
            let (bl, al, cl) = (m0 * (x1 - x0), y1 - y0 - m0 * (x1 - x0), y0 - t);
            let outl = (-2.0 * cl) / ((bl * bl - 4.0 * al * cl).sqrt() + bl) * (x1 - x0) + x0;
            let (br, ar, cr) = (2.0 * y2 - 2.0 * y1 - m2 * (x2 - x1), y1 - y2 + m2 * (x2 - x1), y1 - t);
            let outr = (-2.0 * cr) / ((br * br - 4.0 * ar * cr).sqrt() + br) * (x2 - x1) + x1;
            let mut res = if t < y1 { outl } else { outr };
            if t < y0 {
                res = (t - y0) / m0 + x0;
            }
            if t >= y2 {
                res = (t - y2) / m2 + x2;
            }
            res
        }
    }

    fn white_black(&self, black: bool, ch: usize, t: f64) -> f64 {
        let val = if black { self.zones[0].v[ch] } else { self.zones[4].v[ch] };
        let (([x0, x1], [y0, y1], [m0, m1]), gain) = self.wb[black as usize][ch];
        let mtest = if black { 2.0 - val } else { val };
        if mtest < 1.0 {
            let tl = (t - x0) / (x1 - x0);
            let mut res = tl * (x1 - x0) * (tl * 0.5 * (m1 - m0) + m0) + y0;
            if t < x0 {
                res = y0 + (t - x0) * m0;
            }
            if t >= x1 {
                res = y1 + (t - x1) * m1;
            }
            res
        } else if mtest > 1.0 {
            let mut t = if black { (t - x1) * gain + x1 } else { (t - x0) * gain + x0 };
            let (a, b) = (0.5 * (m1 - m0) * (x1 - x0), m0 * (x1 - x0));
            let c = y0 - t;
            let mut res = (-2.0 * c) / ((b * b - 4.0 * a * c).sqrt() + b) * (x1 - x0) + x0;
            if t < y0 {
                res = x0 + (t - y0) / m0;
            }
            if black {
                if t >= y1 {
                    res = x1 + (t - y1) / m1;
                }
                (res - x1) / gain + x1
            } else {
                res = (res - x0) / gain + x0;
                let new_y1 = (x1 - x0) / gain + x0;
                let xd = x0 + 0.99 * (x1 - x0);
                let md = 1.0 / (m0 + (xd - x0) * (m1 - m0) / (x1 - x0));
                let aa = 0.5 * (1.0 / m1 - md) / (x1 - xd);
                let bb = 1.0 / m1 - 2.0 * aa * x1;
                let cc = new_y1 - bb * x1 - aa * x1 * x1;
                t = (t - x0) / gain + x0;
                if t >= x1 {
                    res = (aa * t + bb) * t + cc;
                }
                res
            }
        } else {
            t
        }
    }

    fn scontrast(&self, t: f64) -> f64 {
        if self.scontrast == 1.0 {
            return t;
        }
        let sc = self.scontrast;
        let c = if sc > 1.0 { 1.0 / (1.8125 - 0.8125 * sc.min(1.99)) } else { 0.28125 + 0.71875 * sc.max(0.01) };
        let mut out = (t - self.pivot) * c + self.pivot;
        let ([_, x1, x2, _], [_, y1, y2, _], [m0, m3]) = self.sc[0];
        let tr = (t - x1) / (x2 - x1);
        if t >= x1 {
            out = tr * (x2 - x1) * (tr * 0.5 * (m3 - m0) + m0) + y1;
        }
        if t >= x2 {
            out = y2 + (t - x2) * m3;
        }
        let ([_, x1, x2, _], [_, y1, _, _], [m0, m3]) = self.sc[1];
        let tr = (t - x1) / (x2 - x1);
        if t < x2 {
            out = tr * (x2 - x1) * (tr * 0.5 * (m3 - m0) + m0) + y1;
        }
        if t < x1 {
            out = y1 + (t - x1) * m0;
        }
        out
    }

    /// A colour graded (forward only).
    pub fn forward(&self, c: [f64; 3]) -> [f64; 3] {
        if self.identity {
            return c;
        }
        let lin = self.style == Style::Lin;
        let mut c = if lin { c.map(lin_log) } else { c };
        let each = |c: &mut [f64; 3], f: &dyn Fn(usize, f64) -> f64| {
            for (i, v) in c.iter_mut().enumerate() {
                *v = f(i, *v);
            }
            for v in c.iter_mut() {
                *v = f(3, *v);
            }
        };
        each(&mut c, &|ch, t| self.mids(ch, t));
        each(&mut c, &|ch, t| self.highlight_shadow(false, ch, t));
        each(&mut c, &|ch, t| self.white_black(false, ch, t));
        each(&mut c, &|ch, t| self.highlight_shadow(true, ch, t));
        each(&mut c, &|ch, t| self.white_black(true, ch, t));
        let c = c.map(|t| self.scontrast(t));
        let c = if lin { c.map(log_lin) } else { c };
        c.map(|v| v.min(65504.0))
    }
}

/// A smooth curve of quadratic pieces through control points (OpenColorIO's RGB B-spline).
#[derive(Clone, Debug, PartialEq)]
pub struct Spline {
    knots: Vec<f64>,
    a: Vec<f64>,
    b: Vec<f64>,
    c: Vec<f64>,
}

impl Spline {
    /// The curve through `pts` (x rising), with slopes given (any not zero) or found; none when
    /// it is the line y = x.
    pub fn new(pts: &[[f64; 2]], given: &[f64]) -> Option<Spline> {
        let n = pts.len();
        if n < 2 || (pts.iter().all(|p| p[0] == p[1]) && given.iter().all(|s| *s == 0.0)) {
            return None;
        }
        let mut m: Vec<f64> = if given.len() == n && given.iter().any(|s| *s != 0.0) { given.to_vec() } else { estimate(pts) };
        let mut s = fit(pts, &m);
        // keep it rising: where a split piece would turn back, the slopes eased
        let mut adjusted = false;
        let (mut i, mut j) = (0usize, 1usize);
        while j < s.knots.len() && i + 1 < n {
            let xi = s.knots[j];
            if (xi - pts[i + 1][0]).abs() < 1e-12 {
                i += 1;
                j += 1;
                continue;
            }
            let (x0, x1, y0, y1) = (pts[i][0], pts[i + 1][0], pts[i][1], pts[i + 1][1]);
            let bar = (2.0 * (y1 - y0) - (xi - x0) * m[i] - (x1 - xi) * m[i + 1]) / (x1 - x0);
            if bar < 0.0 {
                let secant = (y1 - y0) / (x1 - x0);
                let blend = ((xi - x0) * m[i] + (x1 - xi) * m[i + 1]) / (x1 - x0);
                let aim = (0.01 * 0.5 * (m[i] + m[i + 1])).min(secant);
                let k = (2.0 * secant - aim) / blend;
                m[i] *= k;
                m[i + 1] *= k;
                adjusted = true;
            }
            j += 1;
        }
        if adjusted {
            s = fit(pts, &m);
        }
        Some(s)
    }

    pub fn at(&self, x: f64) -> f64 {
        let n = self.a.len();
        let (start, end) = (self.knots[0], self.knots[self.knots.len() - 1]);
        if x <= start {
            return (x - start) * self.b[0] + self.c[0];
        }
        if x >= end {
            let kn = self.knots[self.knots.len() - 2];
            let t = end - kn;
            let (a, b, c) = (self.a[n - 1], self.b[n - 1], self.c[n - 1]);
            return (x - end) * (2.0 * a * t + b) + (a * t + b) * t + c;
        }
        let i = (0..self.knots.len() - 2).find(|&i| x < self.knots[i + 1]).unwrap_or(self.knots.len() - 2);
        let t = x - self.knots[i];
        (self.a[i] * t + self.b[i]) * t + self.c[i]
    }
}

/// Slopes at control points, from the secants round them (weighed by their lengths; runs of
/// equal secants as one).
fn estimate(p: &[[f64; 2]]) -> Vec<f64> {
    let n = p.len();
    let d: Vec<f64> = (0..n - 1).map(|i| (p[i + 1][1] - p[i][1]) / (p[i + 1][0] - p[i][0])).collect();
    let mut l: Vec<f64> = (0..n - 1).map(|i| (p[i + 1][0] - p[i][0]).hypot(p[i + 1][1] - p[i][1])).collect();
    let mut m = vec![0.0; n];
    if n == 2 {
        return vec![d[0], d[0]];
    }
    let mut i = 0;
    loop {
        let mut j = i;
        let mut dl = l[i];
        while j < n - 2 && (d[j + 1] - d[j]).abs() < 1e-6 {
            dl += l[j + 1];
            j += 1;
        }
        for k in l.iter_mut().take(j + 1).skip(i) {
            *k = dl;
        }
        if j >= n - 3 {
            break;
        }
        i = j + 1;
    }
    for k in 1..n - 1 {
        m[k] = (l[k] * d[k] + l[k - 1] * d[k - 1]) / (l[k] + l[k - 1]);
    }
    m[0] = 0.01f64.max(0.5 * (3.0 * d[0] - m[1]));
    m[n - 1] = 0.01f64.max(0.5 * (3.0 * d[n - 2] - m[n - 2]));
    m
}

/// Quadratic pieces through `p` with slopes `m` (a segment split in two where one piece cannot
/// meet both slopes).
fn fit(p: &[[f64; 2]], m: &[f64]) -> Spline {
    let mut s = Spline { knots: vec![p[0][0]], a: Vec::new(), b: Vec::new(), c: Vec::new() };
    for i in 0..p.len() - 1 {
        let (x0, x1, y0) = (p[i][0], p[i + 1][0], p[i][1]);
        let dx = x1 - x0;
        let d = (p[i + 1][1] - y0) / dx;
        if ((m[i] + m[i + 1]) - 2.0 * d).abs() < 1e-6 {
            s.a.push(0.5 * (m[i + 1] - m[i]) / dx);
            s.b.push(m[i]);
            s.c.push(y0);
        } else {
            let (aa, bb) = (m[i] - d, m[i + 1] - d);
            let ksi = if aa * bb >= 0.0 {
                (x0 + x1) / 2.0
            } else if aa.abs() > bb.abs() {
                x1 + aa * dx / (m[i + 1] - m[i])
            } else {
                x0 + bb * dx / (m[i + 1] - m[i])
            };
            let bar = (2.0 * d - m[i + 1]) + (m[i + 1] - m[i]) * (ksi - x0) / dx;
            let eta = (bar - m[i]) / (ksi - x0);
            s.a.push(0.5 * eta);
            s.b.push(m[i]);
            s.c.push(y0);
            s.a.push(0.5 * (m[i + 1] - bar) / (x1 - ksi));
            s.b.push(bar);
            s.c.push(y0 + m[i] * (ksi - x0) + 0.5 * eta * (ksi - x0) * (ksi - x0));
            s.knots.push(ksi);
        }
        s.knots.push(x1);
    }
    s
}

/// An RGB curve grade: a curve for red, green, blue and all three.
#[derive(Clone, Debug, PartialEq)]
pub struct Curves {
    lin: bool,
    curves: [Option<Spline>; 4],
}

impl Curves {
    pub fn read(y: &Yaml) -> Curves {
        let style = Style::of(y.text("style"));
        let curve = |key: &str| -> Option<Spline> {
            let c = y.get(key)?;
            let v = c.get("control_points").map(Yaml::nums).unwrap_or_default();
            let pts: Vec<[f64; 2]> = v.as_chunks::<2>().0.to_vec();
            Spline::new(&pts, &c.get("slopes").map(Yaml::nums).unwrap_or_default())
        };
        Curves { lin: style == Style::Lin && !y.flag("lintolog_bypass", false), curves: [curve("red"), curve("green"), curve("blue"), curve("master")] }
    }

    pub fn forward(&self, c: [f64; 3]) -> [f64; 3] {
        let mut c = if self.lin { c.map(lin_log) } else { c };
        for (i, v) in c.iter_mut().enumerate() {
            if let Some(s) = &self.curves[i] {
                *v = s.at(*v);
            }
        }
        if let Some(s) = &self.curves[3] {
            c = c.map(|v| s.at(v));
        }
        if self.lin { c.map(log_lin) } else { c }
    }
}

/// A grade that changes each channel on its own, only raising it, undone: for each channel the
/// value that `forward` takes to the one given (by halving the range it lies in).
pub fn undo(forward: &dyn Fn([f64; 3]) -> [f64; 3], c: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| {
        let f = |x: f64| forward([x; 3])[i];
        let (mut lo, mut hi) = (-1.0f64, 1.0f64);
        while f(lo) > c[i] && lo > -1e6 {
            lo *= 4.0;
        }
        while f(hi) < c[i] && hi < 1e6 {
            hi *= 4.0;
        }
        for _ in 0..100 {
            let m = (lo + hi) / 2.0;
            if f(m) < c[i] { lo = m } else { hi = m }
        }
        (lo + hi) / 2.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocio::yaml;

    fn read(text: &str) -> Yaml {
        yaml::parse(text)
    }

    #[test]
    fn primary_controls_move_colours_as_ocio_does() {
        // log: brightness adds (master + channel) × 6.25 / 1023, contrast about 0.4
        let p = Primary::read(&read("style: log\nbrightness: {rgb: [0, 0, 0], master: 10}\ncontrast: {rgb: [1, 1, 1], master: 1.5}\n"));
        let out = p.apply([0.5; 3], false);
        let want = (0.5 + 10.0 * 6.25 / 1023.0 - 0.4) * 1.5 + 0.4;
        assert!((out[0] - want).abs() < 1e-12, "{out:?} {want}");
        let back = p.apply(out, true);
        assert!((back[0] - 0.5).abs() < 1e-12);
        // linear: offset then exposure in stops; contrast about 0.18 × 2^0.18
        let l = Primary::read(&read("style: linear\nexposure: {rgb: [0, 0, 0], master: 1}\n"));
        assert!((l.apply([0.25; 3], false)[1] - 0.5).abs() < 1e-12);
        let lc = Primary::read(&read("style: linear\ncontrast: {rgb: [1, 1, 1], master: 2}\npivot: {contrast: 0}\n"));
        assert!((lc.apply([0.36; 3], false)[0] - 0.72).abs() < 1e-12, "doubled away from 0.18");
        // video: lift and gain
        let v = Primary::read(&read("style: video\nlift: {rgb: [0, 0, 0], master: 0.1}\ngain: {rgb: [1, 1, 1], master: 0.5}\n"));
        let a = v.apply([0.0, 0.5, 1.0], false);
        let slope = 1.0 / (1.0 / 0.5 + 0.1);
        assert!((a[0] - 0.1 * slope).abs() < 1e-12 && (a[2] - 1.1 * slope).abs() < 1e-12, "{a:?}");
        let b = v.apply(a, true);
        assert!((b[1] - 0.5).abs() < 1e-12);
        // saturation toward Rec.709 luma, clamped
        let s = Primary::read(&read("style: log\nsaturation: 0\nclamp: {black: 0, white: 0.6}\n"));
        let g = s.apply([0.9, 0.2, 0.1], false);
        assert!((g[0] - g[2]).abs() < 1e-12 && g[0] <= 0.6, "{g:?}");
    }

    #[test]
    fn a_tone_grade_bends_its_zone_and_leaves_the_rest() {
        let identity = Tone::read(&read("style: log\n"));
        assert_eq!(identity.forward([0.1, 0.5, 0.9]), [0.1, 0.5, 0.9]);
        // midtones raised: the middle up, the ends where they were
        let up = Tone::read(&read("style: log\nmidtones: {rgb: [1, 1, 1], master: 1.5, center: 0.4, width: 0.6}\n"));
        let mid = up.forward([0.4; 3])[0];
        assert!(mid > 0.42, "{mid}");
        assert!((up.forward([0.0; 3])[0]).abs() < 1e-9 && (up.forward([1.0; 3])[0] - 1.0).abs() < 1e-9);
        // each rising, and undone
        for zone in ["blacks: {rgb: [1, 1, 1], master: 1.6, start: 0.4, width: 0.4}", "shadows: {rgb: [1, 1, 1], master: 0.6, start: 0.5, pivot: 0}", "highlights: {rgb: [1, 1, 1], master: 1.5, start: 0.3, pivot: 1}", "whites: {rgb: [1, 1, 1], master: 0.7, start: 0.4, width: 0.5}", "s_contrast: 1.4", "midtones: {rgb: [1.3, 1, 0.8], master: 1, center: 0.4, width: 0.6}"] {
            let t = Tone::read(&read(&format!("style: log\n{zone}\n")));
            let mut last = f64::NEG_INFINITY;
            for k in 0..200 {
                let x = -0.2 + 1.4 * k as f64 / 200.0;
                let y = t.forward([x; 3])[1];
                assert!(y >= last - 1e-9, "{zone} at {x}");
                last = y;
            }
            let c = [0.2, 0.45, 0.8];
            let back = undo(&|c| t.forward(c), t.forward(c));
            assert!((0..3).all(|i| (back[i] - c[i]).abs() < 1e-7), "{zone}: {back:?}");
        }
        // linear: in log and back, so 0.18 stays where midtones about it centre
        let lin = Tone::read(&read("style: linear\nwhites: {rgb: [1, 1, 1], master: 0.6, start: 0, width: 8}\n"));
        let bright = lin.forward([16.0; 3])[0];
        assert!(bright < 16.0 && bright > 1.0, "{bright}");
    }

    #[test]
    fn a_curve_passes_through_its_points() {
        let s = Spline::new(&[[0.0, 0.0], [0.3, 0.45], [0.7, 0.8], [1.0, 1.0]], &[]).unwrap();
        for p in [[0.0, 0.0], [0.3, 0.45], [0.7, 0.8], [1.0, 1.0]] {
            assert!((s.at(p[0]) - p[1]).abs() < 1e-9, "{p:?} {}", s.at(p[0]));
        }
        let mut last = f64::NEG_INFINITY;
        for k in 0..=400 {
            let y = s.at(-0.2 + 1.4 * k as f64 / 400.0);
            assert!(y >= last - 1e-12);
            last = y;
        }
        assert!(Spline::new(&[[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]], &[]).is_none(), "the line y = x");
        let c = Curves::read(&read("style: log\nred: {control_points: [0, 0, 0.5, 0.7, 1, 1]}\nmaster: {control_points: [0, 0.1, 1, 0.9]}\n"));
        let out = c.forward([0.5, 0.5, 0.5]);
        assert!((out[0] - (0.1 + 0.8 * 0.7)).abs() < 1e-9, "{out:?}");
        assert!((out[1] - 0.5).abs() < 1e-9);
    }
}
