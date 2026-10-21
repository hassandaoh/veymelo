//! Adjustment layers' settings, each as the specification lays its block out (or, for the newer
//! ones, the descriptor it holds). Tones are from 0 to 1, percents as percents, hues in degrees.

use crate::Psd;
use crate::descriptor::{self, Desc};

/// One channel's Levels: input black and white, output black and white (0 to 1), gamma.
pub type Level = [f64; 5];

/// A hue range of Hue/Saturation: where its fade in starts, where it is whole from and to, where
/// its fade out ends (degrees); and its hue, saturation and lightness changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HueRange {
    pub range: [f64; 4],
    pub hsl: [f64; 3],
}

/// An adjustment layer's settings.
#[derive(Clone, Debug, PartialEq)]
pub enum Adjustment {
    /// The composite's, then each colour channel's.
    Levels(Vec<Level>),
    /// Each curve: its channel (0 the composite, 1 red, 2 green, 3 blue) and its points (input,
    /// output; 0 to 1).
    Curves(Vec<(u16, Vec<[f64; 2]>)>),
    BrightnessContrast { brightness: f64, contrast: f64, legacy: bool },
    /// Colorize's hue, saturation and lightness when on; the master's; the six ranges' (reds,
    /// yellows, greens, cyans, blues, magentas).
    HueSaturation { colorize: Option<[f64; 3]>, master: [f64; 3], ranges: Vec<HueRange> },
    /// Shadows', midtones' and highlights' cyan–red, magenta–green and yellow–blue (−100 to 100).
    ColorBalance { shadows: [f64; 3], midtones: [f64; 3], highlights: [f64; 3], luminosity: bool },
    Invert,
    /// The level (1 to 255).
    Threshold(f64),
    /// The levels (2 to 255).
    Posterize(f64),
    Exposure { exposure: f64, offset: f64, gamma: f64 },
    Vibrance { vibrance: f64, saturation: f64 },
    /// Reds, yellows, greens, cyans, blues, magentas (percent), and the tint where it is on.
    BlackWhite { mix: [f64; 6], tint: Option<[f64; 3]> },
    PhotoFilter { color: [f64; 3], density: f64, luminosity: bool },
    /// Each output channel's red, green, blue and constant (percent).
    ChannelMixer { monochrome: bool, rows: [[f64; 4]; 3] },
    /// Reds, yellows, greens, cyans, blues, magentas, whites, neutrals, blacks: each one's cyan,
    /// magenta, yellow and black (−100 to 100).
    SelectiveColor { absolute: bool, ranges: [[f64; 4]; 9] },
    /// One not read (its key).
    Other([u8; 4]),
}

/// The adjustment keys in the order looked for (the newer of two kept for one first).
const KEYS: [&[u8; 4]; 18] = [b"levl", b"curv", b"CgEd", b"brit", b"hue2", b"hue ", b"blnc", b"nvrt", b"thrs", b"post", b"expA", b"vibA", b"blwh", b"phfl", b"mixr", b"selc", b"grdm", b"clrL"];

fn u16_at(d: &[u8], k: usize) -> f64 {
    d.get(k..k + 2).map_or(0.0, |b| u16::from_be_bytes([b[0], b[1]]) as f64)
}

fn i16_at(d: &[u8], k: usize) -> f64 {
    d.get(k..k + 2).map_or(0.0, |b| i16::from_be_bytes([b[0], b[1]]) as f64)
}

fn f32_at(d: &[u8], k: usize) -> f64 {
    d.get(k..k + 4).map_or(0.0, |b| f32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64)
}

fn u32_at(d: &[u8], k: usize) -> f64 {
    d.get(k..k + 4).map_or(0.0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64)
}

/// Curves' points: `n` (output, input) pairs from `k`, 0 to 255.
fn points(d: &[u8], k: usize, n: usize) -> Vec<[f64; 2]> {
    (0..n).map(|j| [u16_at(d, k + 4 * j + 2) / 255.0, u16_at(d, k + 4 * j) / 255.0]).collect()
}

/// Levels: version, then 29 records of input floor and ceiling, output floor and ceiling (0 to
/// 255) and gamma (hundredths); the composite's first, then each channel's.
fn levels(d: &[u8]) -> Adjustment {
    let n = ((d.len().saturating_sub(2)) / 10).min(4);
    Adjustment::Levels(
        (0..n)
            .map(|c| {
                let k = 2 + 10 * c;
                [u16_at(d, k) / 255.0, u16_at(d, k + 2) / 255.0, u16_at(d, k + 4) / 255.0, u16_at(d, k + 6) / 255.0, u16_at(d, k + 8).max(1.0) / 100.0]
            })
            .collect(),
    )
}

/// Curves: a byte, the version (1 or 4), which curves there are (version 1: one bit a channel;
/// 4: their number), each curve's points; then, where it follows, the same again with each
/// curve's channel said (`Crv `).
fn curves(d: &[u8]) -> Adjustment {
    let version = u16_at(d, 1) as u16;
    let mask = u32_at(d, 3) as u32;
    let count = if version == 1 { mask.count_ones() as usize } else { mask as usize };
    let channels: Vec<u16> = if version == 1 { (0..32).filter(|b| mask & (1 << b) != 0).collect() } else { (0..count as u16).collect() };
    let mut out = Vec::new();
    let mut k = 7;
    for c in channels.iter().take(count.min(32)) {
        let n = u16_at(d, k) as usize;
        if n > 19 || k + 2 + 4 * n > d.len() {
            break;
        }
        out.push((*c, points(d, k + 2, n)));
        k += 2 + 4 * n;
    }
    // the extra data: its channels named
    if let Some(at) = d.get(k..).and_then(|rest| rest.windows(4).position(|w| w == b"Crv ")).map(|p| k + p + 4) {
        let n = u32_at(d, at + 2) as usize;
        let mut k = at + 6;
        let mut named = Vec::new();
        for _ in 0..n.min(64) {
            let (c, m) = (u16_at(d, k) as u16, u16_at(d, k + 2) as usize);
            if m > 19 || k + 4 + 4 * m > d.len() {
                break;
            }
            named.push((c, points(d, k + 4, m)));
            k += 4 + 4 * m;
        }
        if !named.is_empty() {
            out = named;
        }
    }
    Adjustment::Curves(out)
}

/// Hue/Saturation: version, colorize on, a byte, colorize's hue, saturation and lightness, the
/// master's, then six ranges of four range values and three changes.
fn hue_saturation(d: &[u8]) -> Adjustment {
    let colorize = d.get(2).is_some_and(|&b| b != 0).then(|| [i16_at(d, 4), i16_at(d, 6), i16_at(d, 8)]);
    let master = [i16_at(d, 10), i16_at(d, 12), i16_at(d, 14)];
    let ranges = (0..6)
        .map(|r| {
            let k = 16 + 14 * r;
            HueRange { range: [i16_at(d, k), i16_at(d, k + 2), i16_at(d, k + 4), i16_at(d, k + 6)], hsl: [i16_at(d, k + 8), i16_at(d, k + 10), i16_at(d, k + 12)] }
        })
        .collect();
    Adjustment::HueSaturation { colorize, master, ranges }
}

/// Photo Filter: version (3: the colour as XYZ; 2: a colour space and four values), density,
/// preserve luminosity.
fn photo_filter(d: &[u8]) -> Adjustment {
    let version = u16_at(d, 0) as u16;
    let (color, k) = if version == 3 {
        // XYZ, D50, kept as whole numbers: read in the unit that keeps white near 1
        let raw = [u32_at(d, 2), u32_at(d, 6), u32_at(d, 10)];
        let most = raw.iter().cloned().fold(0.0, f64::max);
        let unit = [1.0, 100.0, 10_000.0, 65_536.0, 1_000_000.0].into_iter().find(|u| most / u <= 1.2).unwrap_or(1_000_000.0);
        let [x, y, z] = raw.map(|v| v / unit);
        (xyz_d50_to_srgb(x, y, z), 14)
    } else {
        let space = u16_at(d, 2);
        let c = [u16_at(d, 4), u16_at(d, 6), u16_at(d, 8), u16_at(d, 10)].map(|v| v / 65535.0);
        let rgb = match space as u16 {
            // HSB
            1 => {
                let (h, s, v) = (c[0], c[1], c[2]);
                let i = (h * 6.0).floor();
                let f = h * 6.0 - i;
                let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
                match i as i64 % 6 {
                    0 => [v, t, p],
                    1 => [q, v, p],
                    2 => [p, v, t],
                    3 => [p, q, v],
                    4 => [t, p, v],
                    _ => [v, p, q],
                }
            }
            // CMYK, stored as no ink at the top
            2 => [c[0] * c[3], c[1] * c[3], c[2] * c[3]],
            // greys
            8 => [1.0 - c[0] * 65535.0 / 10000.0; 3],
            _ => [c[0], c[1], c[2]],
        };
        (rgb, 12)
    };
    Adjustment::PhotoFilter { color: color.map(|v| v.clamp(0.0, 1.0)), density: u32_at(d, k), luminosity: d.get(k + 4).is_some_and(|&b| b != 0) }
}

/// sRGB (0 to 1) from D50 XYZ.
fn xyz_d50_to_srgb(x: f64, y: f64, z: f64) -> [f64; 3] {
    let r = 3.133_856 * x - 1.616_867 * y - 0.490_615 * z;
    let g = -0.978_768 * x + 1.916_141 * y + 0.033_454 * z;
    let b = 0.071_945 * x - 0.228_991 * y + 1.405_243 * z;
    [r, g, b].map(|v| {
        let v = v.max(0.0);
        if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
    })
}

/// A descriptor block (its version first).
fn desc(d: &[u8]) -> Option<Desc> {
    descriptor::read(d, true).ok()
}

impl Psd {
    /// Layer `i`'s adjustment, where it is an adjustment layer.
    pub fn adjustment(&self, i: usize) -> Option<Adjustment> {
        let (key, d) = KEYS.iter().find_map(|k| self.block(i, k).map(|d| (**k, d)))?;
        Some(match &key {
            b"levl" => levels(d),
            b"curv" => curves(d),
            b"CgEd" => {
                let c = desc(d)?;
                Adjustment::BrightnessContrast { brightness: c.num("Brgh").unwrap_or(0.0), contrast: c.num("Cntr").unwrap_or(0.0), legacy: c.flag("useLegacy").unwrap_or(false) }
            }
            b"brit" => Adjustment::BrightnessContrast { brightness: i16_at(d, 0), contrast: i16_at(d, 2), legacy: true },
            b"hue2" | b"hue " => hue_saturation(d),
            b"blnc" => {
                let v = |k: usize| [i16_at(d, k), i16_at(d, k + 2), i16_at(d, k + 4)];
                Adjustment::ColorBalance { shadows: v(0), midtones: v(6), highlights: v(12), luminosity: d.get(18).is_some_and(|&b| b != 0) }
            }
            b"nvrt" => Adjustment::Invert,
            b"thrs" => Adjustment::Threshold(u16_at(d, 0)),
            b"post" => Adjustment::Posterize(u16_at(d, 0)),
            b"expA" => Adjustment::Exposure { exposure: f32_at(d, 2), offset: f32_at(d, 6), gamma: f32_at(d, 10) },
            b"vibA" => {
                let c = desc(d)?;
                Adjustment::Vibrance { vibrance: c.num("vibrance").unwrap_or(0.0), saturation: c.num("Strt").unwrap_or(0.0) }
            }
            b"blwh" => {
                let c = desc(d)?;
                let n = |k: &str, v: f64| c.num(k).unwrap_or(v);
                let mix = [n("Rd  ", 40.0), n("Yllw", 60.0), n("Grn ", 40.0), n("Cyn ", 60.0), n("Bl  ", 20.0), n("Mgnt", 80.0)];
                Adjustment::BlackWhite { mix, tint: c.flag("useTint").unwrap_or(false).then(|| c.color("tintColor")).flatten() }
            }
            b"phfl" => photo_filter(d),
            b"mixr" => {
                // version, monochrome, then each output channel's red, green, blue, (black) and
                // constant
                let row = |r: usize| {
                    let k = 4 + 10 * r;
                    [i16_at(d, k), i16_at(d, k + 2), i16_at(d, k + 4), i16_at(d, k + 8)]
                };
                Adjustment::ChannelMixer { monochrome: u16_at(d, 2) != 0.0, rows: [row(0), row(1), row(2)] }
            }
            b"selc" => {
                // version, method, a record kept back, then nine of cyan, magenta, yellow, black
                let r = |n: usize| {
                    let k = 4 + 8 * (n + 1);
                    [i16_at(d, k), i16_at(d, k + 2), i16_at(d, k + 4), i16_at(d, k + 6)]
                };
                Adjustment::SelectiveColor { absolute: u16_at(d, 2) != 0.0, ranges: std::array::from_fn(r) }
            }
            other => Adjustment::Other(*other),
        })
    }
}
