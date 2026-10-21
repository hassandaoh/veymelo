//! The ACES 1.x output transforms, written from the Academy's CTL (ACES 1.0.3 RRT and ODTs,
//! the 1.1 single-stage tone scale and output transform): what turns scene light in ACES2065-1
//! into light for a display.
//!
//! - The Reference Rendering Transform's sweeteners: a glow lifting dark saturated colours, a
//!   softening of saturated reds, the move into the AP1 rendering primaries (clamped) and a slight
//!   desaturation.
//! - SDR: the RRT's tone scale (segmented spline C5, 0.0001 to 10000 nits) and the 48-nit ODT
//!   tone scale (C9), scaled to code values between 0.02 and 48 nits; for video, a dim-surround
//!   gamma; a desaturation for the lower luminance; CIE XYZ, adapted from the ACES white to D65
//!   (Bradford), or a D60 white kept and scaled not to clip.
//! - HDR (1.1): the single-stage tone scale built for the display's black, middle grey and
//!   peak, clipped to the limiting primaries, adapted to D65.
//!
//! The results are CIE XYZ with D65 white as OpenColorIO's ACES configurations take them: SDR
//! relative (1 the display's white), HDR absolute with 1 at 100 nits.

use crate::space::{ACES_WHITE, AP0, AP1, D65, M3, apply, bradford, invert, mul, rgb_to_xyz};

/// The smallest positive half float, and the largest.
const HALF_MIN: f64 = 5.960_464_48e-8;
const HALF_MAX: f64 = 65504.0;

struct Mats {
    ap0_to_ap1: M3,
    ap1_to_xyz: M3,
    xyz_to_ap1: M3,
    d60_to_d65: M3,
    ap1_y: [f64; 3],
}

fn mats() -> &'static Mats {
    static M: std::sync::OnceLock<Mats> = std::sync::OnceLock::new();
    M.get_or_init(|| {
        let ap0 = rgb_to_xyz(AP0, ACES_WHITE);
        let ap1 = rgb_to_xyz(AP1, ACES_WHITE);
        let xyz_to_ap1 = invert(&ap1);
        Mats { ap0_to_ap1: mul(&xyz_to_ap1, &ap0), ap1_to_xyz: ap1, xyz_to_ap1, d60_to_d65: bradford(ACES_WHITE, D65), ap1_y: ap1[1] }
    })
}

/// `rgb` desaturated by `sat` (1: as it is) toward its luminance by `y`'s weights.
fn sat_adjust(rgb: [f64; 3], sat: f64, y: [f64; 3]) -> [f64; 3] {
    let l = rgb[0] * y[0] + rgb[1] * y[1] + rgb[2] * y[2];
    rgb.map(|v| sat * v + (1.0 - sat) * l)
}

fn rgb_2_saturation(c: [f64; 3]) -> f64 {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    (mx.max(1e-10) - mn.max(1e-10)) / mx.max(1e-2)
}

fn rgb_2_yc(c: [f64; 3]) -> f64 {
    let [r, g, b] = c;
    let chroma = (b * (b - g) + g * (g - r) + r * (r - b)).max(0.0).sqrt();
    (b + g + r + 1.75 * chroma) / 3.0
}

fn sigmoid_shaper(x: f64) -> f64 {
    let t = (1.0 - (x / 2.0).abs()).max(0.0);
    (1.0 + x.signum() * (1.0 - t * t)) / 2.0
}

fn glow_fwd(yc: f64, gain: f64, mid: f64) -> f64 {
    if yc <= 2.0 / 3.0 * mid {
        gain
    } else if yc >= 2.0 * mid {
        0.0
    } else {
        gain * (mid / yc - 0.5)
    }
}

fn rgb_2_hue(c: [f64; 3]) -> f64 {
    if c[0] == c[1] && c[1] == c[2] {
        return 0.0;
    }
    let h = (3f64.sqrt() * (c[1] - c[2])).atan2(2.0 * c[0] - c[1] - c[2]).to_degrees();
    if h < 0.0 { h + 360.0 } else { h }
}

fn center_hue(h: f64, centre: f64) -> f64 {
    let x = h - centre;
    if x < -180.0 {
        x + 360.0
    } else if x > 180.0 {
        x - 360.0
    } else {
        x
    }
}

fn cubic_basis_shaper(x: f64, w: f64) -> f64 {
    const M: [[f64; 4]; 4] = [[-1.0 / 6.0, 3.0 / 6.0, -3.0 / 6.0, 1.0 / 6.0], [3.0 / 6.0, -6.0 / 6.0, 3.0 / 6.0, 0.0], [-3.0 / 6.0, 0.0, 3.0 / 6.0, 0.0], [1.0 / 6.0, 4.0 / 6.0, 1.0 / 6.0, 0.0]];
    let knots = [-w / 2.0, -w / 4.0, 0.0, w / 4.0, w / 2.0];
    if !(x > knots[0] && x < knots[4]) {
        return 0.0;
    }
    let coord = (x - knots[0]) * 4.0 / w;
    let j = coord.floor() as usize;
    let t = coord - j as f64;
    let m = [t * t * t, t * t, t, 1.0];
    let col = match j {
        3 => 0,
        2 => 1,
        1 => 2,
        0 => 3,
        _ => return 0.0,
    };
    (0..4).map(|i| m[i] * M[i][col]).sum::<f64>() * 1.5
}

/// The RRT's sweeteners: ACES2065-1 into the AP1 rendering space, before the tone scale.
pub fn rrt_sweeteners(aces: [f64; 3]) -> [f64; 3] {
    let m = mats();
    let sat = rgb_2_saturation(aces);
    // the glow
    let s = sigmoid_shaper((sat - 0.4) / 0.2);
    let glow = 1.0 + glow_fwd(rgb_2_yc(aces), 0.05 * s, 0.08);
    let mut a = aces.map(|v| v * glow);
    // the red modifier
    let hue_weight = cubic_basis_shaper(center_hue(rgb_2_hue(a), 0.0), 135.0);
    a[0] += hue_weight * sat * (0.03 - a[0]) * (1.0 - 0.82);
    // into the rendering space, desaturated a little
    let a = a.map(|v| v.max(0.0));
    let pre = apply(&m.ap0_to_ap1, a).map(|v| v.clamp(0.0, HALF_MAX));
    sat_adjust(pre, 0.96, m.ap1_y)
}

/// A segmented spline (quadratic B-spline pieces on a log-log scale): its ends and middle (x,
/// y), the slopes past its ends, its coefficients below and above the middle.
struct Spline {
    min: (f64, f64),
    mid: (f64, f64),
    max: (f64, f64),
    slope_low: f64,
    slope_high: f64,
    low: &'static [f64],
    high: &'static [f64],
}

const M1: [[f64; 3]; 3] = [[0.5, -1.0, 0.5], [-1.0, 1.0, 0.5], [0.5, 0.0, 0.0]];

/// A quadratic piece `j` of coefficients `c` at `t`.
fn piece(c: &[f64], j: usize, t: f64) -> f64 {
    let cf = [c[j], c[j + 1], c[j + 2]];
    let m = [t * t, t, 1.0];
    (0..3).map(|col| m[col] * (0..3).map(|k| cf[k] * M1[k][col]).sum::<f64>()).sum()
}

impl Spline {
    fn at(&self, x: f64) -> f64 {
        let logx = x.max(HALF_MIN).log10();
        let (lmin, lmid, lmax) = (self.min.0.log10(), self.mid.0.log10(), self.max.0.log10());
        let logy = if logx <= lmin {
            logx * self.slope_low + (self.min.1.log10() - self.slope_low * lmin)
        } else if logx < lmid {
            let coord = (self.low.len() - 3) as f64 * (logx - lmin) / (lmid - lmin);
            let j = (coord.floor() as usize).min(self.low.len() - 3);
            piece(self.low, j, coord - j as f64)
        } else if logx < lmax {
            let coord = (self.high.len() - 3) as f64 * (logx - lmid) / (lmax - lmid);
            let j = (coord.floor() as usize).min(self.high.len() - 3);
            piece(self.high, j, coord - j as f64)
        } else {
            logx * self.slope_high + (self.max.1.log10() - self.slope_high * lmax)
        };
        10f64.powf(logy)
    }
}

const C5_LOW: [f64; 6] = [-4.0, -4.0, -3.157_376_577_3, -0.485_249_995_8, 1.847_732_470_6, 1.847_732_470_6];
const C5_HIGH: [f64; 6] = [-0.718_548_242_5, 2.081_030_717_2, 3.668_124_123_7, 4.0, 4.0, 4.0];
const C9_LOW: [f64; 10] = [-1.698_970_004_3, -1.698_970_004_3, -1.4779, -1.2291, -0.8648, -0.448, 0.00518, 0.451_108_033_4, 0.911_374_441_4, 0.911_374_441_4];
const C9_HIGH: [f64; 10] = [0.515_438_696_5, 0.847_043_778_3, 1.1358, 1.3802, 1.5197, 1.5985, 1.6467, 1.674_609_135_7, 1.687_873_339, 1.687_873_339];

/// The RRT's tone scale: ACES (AP1) to luminance, 0.0001 to 10000 nits.
pub fn segmented_spline_c5(x: f64) -> f64 {
    Spline { min: (0.18 * 2f64.powi(-15), 0.0001), mid: (0.18, 4.8), max: (0.18 * 2f64.powi(18), 10000.0), slope_low: 0.0, slope_high: 0.0, low: &C5_LOW, high: &C5_HIGH }.at(x)
}

/// The 48-nit ODT's tone scale: the RRT's output to a cinema's luminance, 0.02 to 48 nits.
pub fn segmented_spline_c9(x: f64) -> f64 {
    let s = Spline {
        min: (segmented_spline_c5(0.18 * 2f64.powf(-6.5)), 0.02),
        mid: (segmented_spline_c5(0.18), 4.8),
        max: (segmented_spline_c5(0.18 * 2f64.powf(6.5)), 48.0),
        slope_low: 0.0,
        slope_high: 0.04,
        low: &C9_LOW,
        high: &C9_HIGH,
    };
    s.at(x)
}

fn xyz_to_xyy(c: [f64; 3]) -> [f64; 3] {
    let d = (c[0] + c[1] + c[2]).max(1e-10);
    [c[0] / d, c[1] / d, c[1]]
}

fn xyy_to_xyz(c: [f64; 3]) -> [f64; 3] {
    let y = c[1].max(1e-10);
    [c[0] * c[2] / y, c[2], (1.0 - c[0] - c[1]) * c[2] / y]
}

/// AP1 linear code values from a dark surround to a dim one (a gamma on their luminance).
fn dark_to_dim(lin: [f64; 3]) -> [f64; 3] {
    let m = mats();
    let mut v = xyz_to_xyy(apply(&m.ap1_to_xyz, lin));
    v[2] = v[2].max(0.0).powf(0.9811);
    apply(&m.xyz_to_ap1, xyy_to_xyz(v))
}

/// An SDR output: ACES2065-1 to CIE XYZ (D65 white, 1 the display's white) through the RRT and
/// the 48-nit ODT tone scale; `dim` for video (a dim surround); `d60` keeps the ACES white
/// (scaled so it does not clip on a D65 display).
pub fn sdr(aces: [f64; 3], dim: bool, d60: Option<f64>) -> [f64; 3] {
    let m = mats();
    let pre = rrt_sweeteners(aces);
    let post = pre.map(|v| segmented_spline_c9(segmented_spline_c5(v)));
    // code values (unclipped, as OpenColorIO's builtins leave them)
    let mut lin = post.map(|y| (y - 0.02) / (48.0 - 0.02));
    if dim {
        lin = dark_to_dim(lin);
    }
    let lin = sat_adjust(lin, 0.93, m.ap1_y);
    let xyz = apply(&m.ap1_to_xyz, lin);
    match d60 {
        None => apply(&m.d60_to_d65, xyz),
        Some(scale) => xyz.map(|v| v * scale),
    }
}

/// A point of a tone scale: its ACES value, its luminance, and its slope there.
type Point = (f64, f64, f64);

/// The single-stage tone scale's shape for a display from `min` to `max` nits, its exposure
/// shifted by `shift` stops.
#[derive(Clone, Debug, PartialEq)]
pub struct Ssts {
    min: (f64, f64, f64),
    mid: (f64, f64, f64),
    max: (f64, f64, f64),
    low: [f64; 6],
    high: [f64; 6],
}

fn interpolate(table: &[(f64, f64)], x: f64) -> f64 {
    if x <= table[0].0 {
        return table[0].1;
    }
    let last = table[table.len() - 1];
    if x >= last.0 {
        return last.1;
    }
    let i = table.iter().position(|p| p.0 > x).unwrap_or(table.len() - 1);
    let (a, b) = (table[i - 1], table[i]);
    a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0)
}

fn aces_min(min_lum: f64) -> f64 {
    0.18 * 2f64.powf(interpolate(&[(0.0001f64.log10(), -15.0), (0.02f64.log10(), -6.5)], min_lum.log10()))
}

fn aces_max(max_lum: f64) -> f64 {
    0.18 * 2f64.powf(interpolate(&[(48f64.log10(), 6.5), (10000f64.log10(), 18.0)], max_lum.log10()))
}

impl Ssts {
    pub fn new(min_lum: f64, max_lum: f64, shift: f64) -> Ssts {
        let (lo, mid, hi): (Point, Point, Point) = ((aces_min(min_lum), min_lum, 0.0), (0.18, 4.8, 1.55), (aces_max(max_lum), max_lum, 0.0));
        let line = |p: (f64, f64, f64), x: f64| p.2 * x + (p.1.log10() - p.2 * p.0.log10());
        // below the middle: two coefficients straddling each end, the bend's between them
        let inc = (mid.0.log10() - lo.0.log10()) / 3.0;
        let pct_low = interpolate(&[(-15.0, 0.18), (-6.5, 0.35)], (lo.0 / 0.18).log2());
        let low4 = [line(lo, lo.0.log10() - 0.5 * inc), line(lo, lo.0.log10() + 0.5 * inc), lo.1.log10() + pct_low * (mid.1.log10() - lo.1.log10()), line(mid, mid.0.log10() - 0.5 * inc), line(mid, mid.0.log10() + 0.5 * inc)];
        let inc = (hi.0.log10() - mid.0.log10()) / 3.0;
        let pct_high = interpolate(&[(6.5, 0.89), (18.0, 0.90)], (hi.0 / 0.18).log2());
        let high4 = [line(mid, mid.0.log10() - 0.5 * inc), line(mid, mid.0.log10() + 0.5 * inc), mid.1.log10() + pct_high * (hi.1.log10() - mid.1.log10()), line(hi, hi.0.log10() - 0.5 * inc), line(hi, hi.0.log10() + 0.5 * inc)];
        let moved = |x: f64| 2f64.powf(x.log2() - shift);
        Ssts {
            min: (moved(lo.0), lo.1, lo.2),
            mid: (moved(mid.0), mid.1, mid.2),
            max: (moved(hi.0), hi.1, hi.2),
            low: [low4[0], low4[1], low4[2], low4[3], low4[4], low4[4]],
            high: [high4[0], high4[1], high4[2], high4[3], high4[4], high4[4]],
        }
    }

    pub fn at(&self, x: f64) -> f64 {
        let logx = x.max(HALF_MIN).log10();
        let (lmin, lmid, lmax) = (self.min.0.log10(), self.mid.0.log10(), self.max.0.log10());
        let logy = if logx <= lmin {
            logx * self.min.2 + (self.min.1.log10() - self.min.2 * lmin)
        } else if logx < lmid {
            let coord = 3.0 * (logx - lmin) / (lmid - lmin);
            let j = (coord.floor() as usize).min(3);
            piece(&self.low, j, coord - j as f64)
        } else if logx < lmax {
            let coord = 3.0 * (logx - lmid) / (lmax - lmid);
            let j = (coord.floor() as usize).min(3);
            piece(&self.high, j, coord - j as f64)
        } else {
            logx * self.max.2 + (self.max.1.log10() - self.max.2 * lmax)
        };
        10f64.powf(logy)
    }

    /// The x it gives `y` at (it rises throughout).
    pub fn back(&self, y: f64) -> f64 {
        let (mut lo, mut hi) = (-20.0f64, 20.0f64);
        for _ in 0..200 {
            let m = (lo + hi) / 2.0;
            if self.at(10f64.powf(m)) < y { lo = m } else { hi = m }
        }
        10f64.powf((lo + hi) / 2.0)
    }
}

/// An HDR output (ACES 1.1): ACES2065-1 to CIE XYZ (D65 white, 1 at 100 nits) for a display
/// from `min` to `max` nits with middle grey at `mid`, clipped to `limit` primaries (D65 white).
pub fn hdr(aces: [f64; 3], min: f64, mid: f64, max: f64, limit: [[f64; 2]; 3]) -> [f64; 3] {
    let m = mats();
    let tone = tone_for(min, mid, max);
    let pre = rrt_sweeteners(aces);
    let post = pre.map(|v| tone.at(v));
    let lin = post.map(|y| (y - min) / (max - min));
    let xyz = apply(&m.ap1_to_xyz, lin);
    // clipped to the limiting primaries
    let to_limit = rgb_to_xyz(limit, D65);
    let rgb = apply(&invert(&to_limit), xyz).map(|v| v.clamp(0.0, 1.0));
    let xyz = apply(&m.d60_to_d65, apply(&to_limit, rgb));
    // black stretched to none, peak at `max` nits, 1 at 100
    xyz.map(|v| v * max / 100.0)
}

/// The tone scale for a display from `min` to `max` nits, shifted so ACES 0.18 lands at `mid`.
fn tone_for(min: f64, mid: f64, max: f64) -> std::sync::Arc<Ssts> {
    static KEPT: std::sync::Mutex<Vec<([u64; 3], std::sync::Arc<Ssts>)>> = std::sync::Mutex::new(Vec::new());
    let key = [min.to_bits(), mid.to_bits(), max.to_bits()];
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, t)) = kept.iter().find(|(k, _)| *k == key) {
        return t.clone();
    }
    let plain = Ssts::new(min, max, 0.0);
    let shift = plain.back(mid).log2() - 0.18f64.log2();
    let t = std::sync::Arc::new(Ssts::new(min, max, shift));
    kept.push((key, t.clone()));
    t
}

/// An output transform as OpenColorIO's ACES configurations name them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Output {
    Sdr { dim: bool, d60: Option<f64> },
    Hdr { min: f64, mid: f64, max: f64, limit: [[f64; 2]; 3] },
}

impl Output {
    /// The output a BuiltinTransform style names (ACES 1.x), if this knows it.
    pub fn named(style: &str) -> Option<Output> {
        use crate::space::{P3, REC2020};
        let rest = style.trim().strip_prefix("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - ")?;
        Some(match rest {
            "SDR-VIDEO_1.0" => Output::Sdr { dim: true, d60: None },
            "SDR-CINEMA_1.0" => Output::Sdr { dim: false, d60: None },
            "SDR-VIDEO-D60sim-D65_1.0" => Output::Sdr { dim: true, d60: Some(0.955) },
            "SDR-CINEMA-D60sim-D65_1.1" => Output::Sdr { dim: false, d60: Some(0.964) },
            "HDR-VIDEO-1000nit-15nit-REC2020lim_1.1" => Output::Hdr { min: 0.0001, mid: 15.0, max: 1000.0, limit: REC2020 },
            "HDR-VIDEO-1000nit-15nit-P3lim_1.1" => Output::Hdr { min: 0.0001, mid: 15.0, max: 1000.0, limit: P3 },
            "HDR-VIDEO-2000nit-15nit-REC2020lim_1.1" => Output::Hdr { min: 0.0001, mid: 15.0, max: 2000.0, limit: REC2020 },
            "HDR-VIDEO-2000nit-15nit-P3lim_1.1" => Output::Hdr { min: 0.0001, mid: 15.0, max: 2000.0, limit: P3 },
            "HDR-VIDEO-4000nit-15nit-REC2020lim_1.1" => Output::Hdr { min: 0.0001, mid: 15.0, max: 4000.0, limit: REC2020 },
            "HDR-VIDEO-4000nit-15nit-P3lim_1.1" => Output::Hdr { min: 0.0001, mid: 15.0, max: 4000.0, limit: P3 },
            "HDR-CINEMA-108nit-7.2nit-P3lim_1.1" => Output::Hdr { min: 0.0001, mid: 7.2, max: 108.0, limit: P3 },
            _ => return None,
        })
    }

    pub fn apply(&self, aces: [f64; 3]) -> [f64; 3] {
        match *self {
            Output::Sdr { dim, d60 } => sdr(aces, dim, d60),
            Output::Hdr { min, mid, max, limit } => hdr(aces, min, mid, max, limit),
        }
    }

    /// The ACES colour it makes `xyz` from (found by Newton's steps; none where it does not
    /// settle, as for colours it never makes).
    pub fn back(&self, xyz: [f64; 3]) -> Option<[f64; 3]> {
        let mut x = [0.18; 3];
        for _ in 0..60 {
            let f = self.apply(x);
            let e = [f[0] - xyz[0], f[1] - xyz[1], f[2] - xyz[2]];
            if e.iter().map(|v| v.abs()).fold(0.0, f64::max) < 1e-7 {
                return Some(x);
            }
            // the slope, each channel nudged
            let mut j = [[0.0; 3]; 3];
            for c in 0..3 {
                let h = 1e-6 * x[c].abs().max(1e-4);
                let mut y = x;
                y[c] += h;
                let g = self.apply(y);
                for r in 0..3 {
                    j[r][c] = (g[r] - f[r]) / h;
                }
            }
            let step = apply(&invert(&j), e);
            if step.iter().any(|v| !v.is_finite()) {
                return None;
            }
            // no further than halfway to nothing each time
            for c in 0..3 {
                x[c] = (x[c] - step[c]).max(x[c] * 0.5);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tone_scales_meet_their_points() {
        // the RRT: middle grey to 4.8 nits, its ends to 0.0001 and 10000
        assert!((segmented_spline_c5(0.18) - 4.8).abs() < 1e-3);
        assert!((segmented_spline_c5(0.18 * 2f64.powi(-15)) - 0.0001).abs() < 1e-7);
        assert!((segmented_spline_c5(0.18 * 2f64.powi(18)) - 10000.0).abs() < 1.0);
        // the ODT: 4.8 nits stays, its ends at 0.02 and 48
        assert!((segmented_spline_c9(segmented_spline_c5(0.18)) - 4.8).abs() < 1e-3);
        assert!((segmented_spline_c9(segmented_spline_c5(0.18 * 2f64.powf(6.5))) - 48.0).abs() < 1e-2);
        assert!((segmented_spline_c9(segmented_spline_c5(0.18 * 2f64.powf(-6.5))) - 0.02).abs() < 1e-4);
        // rising throughout, without jumps where the pieces meet
        let mut last = 0.0;
        for k in 0..2000 {
            let x = 0.18 * 2f64.powf(-16.0 + 35.0 * k as f64 / 2000.0);
            let y = segmented_spline_c9(segmented_spline_c5(x));
            assert!(y >= last - 1e-12, "{x}");
            assert!(k == 0 || y / last < 1.05, "a jump at {x}");
            last = y;
        }
    }

    #[test]
    fn middle_grey_lands_where_aces_puts_it() {
        // a neutral 0.18 to 4.8 nits of 48 (with the dim surround's gamma), neutral and D65
        let xyz = sdr([0.18; 3], true, None);
        let want = ((4.8 - 0.02) / (48.0 - 0.02f64)).powf(0.9811);
        assert!((xyz[1] - want).abs() < 1e-4, "{xyz:?} {want}");
        let d65 = crate::space::rgb_to_xyz(crate::space::REC709, D65);
        let rgb = apply(&invert(&d65), xyz);
        assert!((rgb[0] - rgb[1]).abs() < 1e-4 && (rgb[2] - rgb[1]).abs() < 1e-4, "{rgb:?}");
        // HDR: middle grey at 15 nits of 1000
        let h = hdr([0.18; 3], 0.0001, 15.0, 1000.0, crate::space::REC2020);
        let nits = h[1] * 100.0;
        assert!((nits - 15.0 * 1000.0 / (1000.0 - 0.0001)).abs() < 0.05, "{nits}");
        // and its peak no more than the display's
        let top = hdr([1000.0; 3], 0.0001, 15.0, 1000.0, crate::space::REC2020);
        assert!(top[1] * 100.0 <= 1000.0 + 1e-6 && top[1] * 100.0 > 900.0, "{top:?}");
    }

    #[test]
    fn the_hdr_tone_scale_meets_opencolorios_points() {
        // OpenColorIO's 1000-nit curve, as log10 in and out
        let t = tone_for(0.0001, 15.0, 1000.0);
        for (x, y) in [(-5.600_501_55, -4.0), (-4.095_351_57, -3.578_688_29), (-2.590_201_59, -1.821_313_29), (-1.085_051_61, 0.681_241_24), (0.223_470_59, 2.226_735_03), (1.531_992_79, 2.879_062_06), (2.840_515, 3.0)] {
            let got = t.at(10f64.powf(x)).log10();
            assert!((got - y).abs() < 2e-6, "at {x}: {got} for {y}");
        }
    }

    #[test]
    fn saturated_reds_are_softened_and_neutrals_not() {
        let n = rrt_sweeteners([0.5; 3]);
        assert!((n[0] - n[1]).abs() < 1e-9 && (n[1] - n[2]).abs() < 1e-9);
        // a strong red in ACES: its red pulled toward the pivot, less than as it was
        let red = [2.0, 0.05, 0.05];
        let m = mats();
        let plain = apply(&m.ap0_to_ap1, red);
        let swept = rrt_sweeteners(red);
        assert!(swept[0] < plain[0], "{swept:?} {plain:?}");
    }

    #[test]
    fn an_output_undone() {
        for out in [Output::named("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-VIDEO_1.0").unwrap(), Output::named("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-VIDEO-1000nit-15nit-P3lim_1.1").unwrap()] {
            // (a colour clipped to the limiting primaries comes back as one that makes the same)
            for c in [[0.18, 0.18, 0.18], [0.5, 0.3, 0.1], [0.05, 0.1, 0.2]] {
                let made = out.apply(c);
                let again = out.apply(out.back(made).expect("found"));
                for i in 0..3 {
                    assert!((again[i] - made[i]).abs() < 1e-6, "{out:?} {c:?} {again:?} {made:?}");
                }
            }
            let grey = out.back(out.apply([0.18; 3])).unwrap();
            assert!(grey.iter().all(|v| (v - 0.18).abs() < 1e-5), "{grey:?}");
        }
    }
}
