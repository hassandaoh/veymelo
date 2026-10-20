//! Colour spaces: three primaries and a white (as chromaticities), and how values are written
//! down (a transfer curve), from the published standards and the camera makers' white papers.
//! Colours go from one space to another through CIE XYZ with a D65 white, spaces with another
//! white adapted to it by the Bradford transform.
//!
//! Linear values are scene light with 1.0 for a white card (0.18 a grey card), except for the
//! display curves PQ and BT.1886, where 1.0 is 100 cd/m² on the display.

/// A 3 × 3 matrix, rows first.
pub type M3 = [[f64; 3]; 3];

pub const IDENTITY: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub fn mul(a: &M3, b: &M3) -> M3 {
    let mut m = [[0.0; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

pub fn apply(m: &M3, c: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * c[0] + m[i][1] * c[1] + m[i][2] * c[2])
}

/// The inverse (the identity for a matrix that has none).
pub fn invert(m: &M3) -> M3 {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-300 {
        return IDENTITY;
    }
    let k = 1.0 / det;
    [
        [(e * i - f * h) * k, (c * h - b * i) * k, (b * f - c * e) * k],
        [(f * g - d * i) * k, (a * i - c * g) * k, (c * d - a * f) * k],
        [(d * h - e * g) * k, (b * g - a * h) * k, (a * e - b * d) * k],
    ]
}

/// A chromaticity as XYZ with Y = 1.
fn xyz_of(xy: [f64; 2]) -> [f64; 3] {
    // (imaginary primaries can lie below y = 0)
    let y = if xy[1].abs() < 1e-12 { 1e-12 } else { xy[1] };
    [xy[0] / y, 1.0, (1.0 - xy[0] - xy[1]) / y]
}

/// From RGB with these primaries and white to XYZ (the white at Y = 1).
pub fn rgb_to_xyz(primaries: [[f64; 2]; 3], white: [f64; 2]) -> M3 {
    let p = primaries.map(xyz_of);
    let m = [[p[0][0], p[1][0], p[2][0]], [p[0][1], p[1][1], p[2][1]], [p[0][2], p[1][2], p[2][2]]];
    let s = apply(&invert(&m), xyz_of(white));
    [0, 1, 2].map(|i| [m[i][0] * s[0], m[i][1] * s[1], m[i][2] * s[2]])
}

/// XYZ seen under white `from` as it looks under white `to` (Bradford).
pub fn bradford(from: [f64; 2], to: [f64; 2]) -> M3 {
    const B: M3 = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];
    if (from[0] - to[0]).abs() < 1e-12 && (from[1] - to[1]).abs() < 1e-12 {
        return IDENTITY;
    }
    let (s, d) = (apply(&B, xyz_of(from)), apply(&B, xyz_of(to)));
    let scale = [[d[0] / s[0], 0.0, 0.0], [0.0, d[1] / s[1], 0.0], [0.0, 0.0, d[2] / s[2]]];
    mul(&invert(&B), &mul(&scale, &B))
}

pub const D65: [f64; 2] = [0.3127, 0.3290];
pub const D50: [f64; 2] = [0.3457, 0.3585];
pub const ACES_WHITE: [f64; 2] = [0.32168, 0.33767];
pub const DCI_WHITE: [f64; 2] = [0.314, 0.351];

pub const REC709: [[f64; 2]; 3] = [[0.640, 0.330], [0.300, 0.600], [0.150, 0.060]];
pub const REC2020: [[f64; 2]; 3] = [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046]];
pub const P3: [[f64; 2]; 3] = [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060]];
pub const AP0: [[f64; 2]; 3] = [[0.7347, 0.2653], [0.0, 1.0], [0.0001, -0.0770]];
pub const AP1: [[f64; 2]; 3] = [[0.713, 0.293], [0.165, 0.830], [0.128, 0.044]];
pub const ADOBE_RGB: [[f64; 2]; 3] = [[0.640, 0.330], [0.210, 0.710], [0.150, 0.060]];
pub const PROPHOTO: [[f64; 2]; 3] = [[0.7347, 0.2653], [0.1596, 0.8404], [0.0366, 0.0001]];
pub const ARRI_WIDE_GAMUT_3: [[f64; 2]; 3] = [[0.6840, 0.3130], [0.2210, 0.8480], [0.0861, -0.1020]];
pub const ARRI_WIDE_GAMUT_4: [[f64; 2]; 3] = [[0.7347, 0.2653], [0.1424, 0.8576], [0.0991, -0.0308]];
pub const S_GAMUT3: [[f64; 2]; 3] = [[0.730, 0.280], [0.140, 0.855], [0.100, -0.050]];
pub const S_GAMUT3_CINE: [[f64; 2]; 3] = [[0.766, 0.275], [0.225, 0.800], [0.089, -0.087]];
pub const V_GAMUT: [[f64; 2]; 3] = [[0.730, 0.280], [0.165, 0.840], [0.100, -0.030]];
pub const CINEMA_GAMUT: [[f64; 2]; 3] = [[0.740, 0.270], [0.170, 1.140], [0.080, -0.100]];
pub const RED_WIDE_GAMUT: [[f64; 2]; 3] = [[0.780308, 0.304253], [0.121595, 1.493994], [0.095612, -0.084589]];
pub const BMD_WIDE_GAMUT_5: [[f64; 2]; 3] = [[0.717_721_5, 0.317_118_1], [0.228_041_0, 0.861_569_0], [0.100_584_1, -0.082_045_2]];
pub const D_GAMUT: [[f64; 2]; 3] = [[0.71, 0.31], [0.21, 0.88], [0.09, -0.08]];

/// How a space writes its values down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Linear,
    /// IEC 61966-2-1.
    Srgb,
    /// A plain power (`decode` = v^g), mirrored below zero.
    Gamma(f64),
    /// The ITU-R BT.709 camera curve.
    Rec709,
    /// ROMM RGB (ProPhoto): 1.8 with a straight foot.
    Romm,
    /// Academy S-2016-001.
    AcesCct,
    /// Academy S-2014-003.
    AcesCc,
    /// SMPTE ST 2084, 1.0 = 100 cd/m².
    Pq,
    /// ITU-R BT.2100 hybrid log-gamma (the scene curve).
    Hlg,
    /// ARRI LogC3 at EI 800.
    LogC3,
    LogC4,
    SLog3,
    VLog,
    /// Canon Log 2 and 3 (on 0 to 1 IRE, a white card at 0.9 of the curve's 1, as Canon's
    /// conversions for ACES do).
    CanonLog2,
    CanonLog3,
    /// RED Log3G10.
    Log3G10,
    /// Blackmagic Design Film Generation 5.
    BmdFilm5,
    /// DJI D-Log.
    DLog,
}

const ACESCC_LOW: f64 = (-16.0 + 9.72) / 17.52;
const ACESCC_TOE: f64 = (-15.0 + 9.72) / 17.52;
const SLOG3_CUT: f64 = 171.210_294_692_9;

impl Curve {
    /// A written value as linear light.
    pub fn decode(self, v: f64) -> f64 {
        match self {
            Curve::Linear => v,
            Curve::Srgb => {
                let a = v.abs();
                v.signum() * if a <= 0.04045 { a / 12.92 } else { ((a + 0.055) / 1.055).powf(2.4) }
            }
            Curve::Gamma(g) => v.signum() * v.abs().powf(g),
            Curve::Rec709 => {
                let a = v.abs();
                v.signum() * if a < 0.081 { a / 4.5 } else { ((a + 0.099) / 1.099).powf(1.0 / 0.45) }
            }
            Curve::Romm => {
                if v < 16.0 / 512.0 {
                    v / 16.0
                } else {
                    v.powf(1.8)
                }
            }
            Curve::AcesCct => {
                if v <= 0.155_251_141_552_511 {
                    (v - 0.072_905_534_195_835_5) / 10.540_237_741_654_5
                } else {
                    2f64.powf(v * 17.52 - 9.72).min(65504.0)
                }
            }
            Curve::AcesCc => {
                if v < ACESCC_TOE {
                    (2f64.powf(v * 17.52 - 9.72) - 2f64.powi(-16)) * 2.0
                } else {
                    2f64.powf(v * 17.52 - 9.72).min(65504.0)
                }
            }
            Curve::Pq => {
                let (m1, m2, c1, c2, c3) = pq();
                let e = v.max(0.0).powf(1.0 / m2);
                ((e - c1).max(0.0) / (c2 - c3 * e)).powf(1.0 / m1) * 100.0
            }
            Curve::Hlg => {
                let (a, b, c) = hlg();
                if v <= 0.5 { v * v / 3.0 } else { (((v - c) / a).exp() + b) / 12.0 }
            }
            Curve::LogC3 => {
                let (cut, a, b, c, d, e, f) = LOGC3;
                if v > e * cut + f { (10f64.powf((v - d) / c) - b) / a } else { (v - f) / e }
            }
            Curve::LogC4 => {
                let (a, b, c, s, t) = logc4();
                if v >= 0.0 { (2f64.powf(14.0 * (v - c) / b + 6.0) - 64.0) / a } else { v * s + t }
            }
            Curve::SLog3 => {
                if v >= SLOG3_CUT / 1023.0 {
                    10f64.powf((v * 1023.0 - 420.0) / 261.5) * 0.19 - 0.01
                } else {
                    (v * 1023.0 - 95.0) * 0.01125 / (SLOG3_CUT - 95.0)
                }
            }
            Curve::VLog => {
                if v < 0.181 {
                    (v - 0.125) / 5.6
                } else {
                    10f64.powf((v - 0.598_206) / 0.241_514) - 0.008_73
                }
            }
            Curve::CanonLog2 => {
                0.9 * if v < 0.092_864_125 {
                    -(10f64.powf((0.092_864_125 - v) / 0.241_360_77) - 1.0) / 87.099_375_46
                } else {
                    (10f64.powf((v - 0.092_864_125) / 0.241_360_77) - 1.0) / 87.099_375_46
                }
            }
            Curve::CanonLog3 => {
                0.9 * if v < 0.097_465_473 {
                    -(10f64.powf((0.127_839_01 - v) / 0.367_268_45) - 1.0) / 14.983_25
                } else if v <= 0.152_778_91 {
                    (v - 0.125_122_19) / 1.975_479_8
                } else {
                    (10f64.powf((v - 0.122_405_37) / 0.367_268_45) - 1.0) / 14.983_25
                }
            }
            Curve::Log3G10 => {
                let (a, b, c, g) = LOG3G10;
                if v < 0.0 { v / g - c } else { (10f64.powf(v / a) - 1.0) / b - c }
            }
            Curve::BmdFilm5 => {
                let (a, b, c, d, e, cut) = BMD5;
                if v < d * cut + e { (v - e) / d } else { ((v - c) / a).exp() - b }
            }
            Curve::DLog => {
                if v <= 0.14 {
                    (v - 0.0929) / 6.025
                } else {
                    // the exact inverse of the curve (DJI's own rounds these numbers)
                    (10f64.powf((v - 0.584_555) / 0.256_663) - 0.0108) / 0.9892
                }
            }
        }
    }

    /// Linear light written as the curve writes it.
    pub fn encode(self, x: f64) -> f64 {
        match self {
            Curve::Linear => x,
            Curve::Srgb => {
                let a = x.abs();
                x.signum() * if a <= 0.003_130_8 { a * 12.92 } else { 1.055 * a.powf(1.0 / 2.4) - 0.055 }
            }
            Curve::Gamma(g) => x.signum() * x.abs().powf(1.0 / g),
            Curve::Rec709 => {
                let a = x.abs();
                x.signum() * if a < 0.018 { a * 4.5 } else { 1.099 * a.powf(0.45) - 0.099 }
            }
            Curve::Romm => {
                if x < 1.0 / 512.0 {
                    x * 16.0
                } else {
                    x.powf(1.0 / 1.8)
                }
            }
            Curve::AcesCct => {
                if x <= 0.007_812_5 {
                    10.540_237_741_654_5 * x + 0.072_905_534_195_835_5
                } else {
                    (x.log2() + 9.72) / 17.52
                }
            }
            Curve::AcesCc => {
                if x <= 0.0 {
                    ACESCC_LOW
                } else if x < 2f64.powi(-15) {
                    ((2f64.powi(-16) + x * 0.5).log2() + 9.72) / 17.52
                } else {
                    (x.log2() + 9.72) / 17.52
                }
            }
            Curve::Pq => {
                let (m1, m2, c1, c2, c3) = pq();
                let y = (x / 100.0).max(0.0).powf(m1);
                ((c1 + c2 * y) / (1.0 + c3 * y)).powf(m2)
            }
            Curve::Hlg => {
                let (a, b, c) = hlg();
                let x = x.max(0.0);
                if x <= 1.0 / 12.0 { (3.0 * x).sqrt() } else { a * (12.0 * x - b).ln() + c }
            }
            Curve::LogC3 => {
                let (cut, a, b, c, d, e, f) = LOGC3;
                if x > cut { c * (a * x + b).log10() + d } else { e * x + f }
            }
            Curve::LogC4 => {
                let (a, b, c, s, t) = logc4();
                if x >= t { ((a * x + 64.0).log2() - 6.0) / 14.0 * b + c } else { (x - t) / s }
            }
            Curve::SLog3 => {
                if x >= 0.01125 {
                    (420.0 + ((x + 0.01) / 0.19).log10() * 261.5) / 1023.0
                } else {
                    (x * (SLOG3_CUT - 95.0) / 0.01125 + 95.0) / 1023.0
                }
            }
            Curve::VLog => {
                if x < 0.01 {
                    5.6 * x + 0.125
                } else {
                    0.241_514 * (x + 0.008_73).log10() + 0.598_206
                }
            }
            Curve::CanonLog2 => {
                let x = x / 0.9;
                if x < 0.0 {
                    -0.241_360_77 * (-x * 87.099_375_46 + 1.0).log10() + 0.092_864_125
                } else {
                    0.241_360_77 * (x * 87.099_375_46 + 1.0).log10() + 0.092_864_125
                }
            }
            Curve::CanonLog3 => {
                let x = x / 0.9;
                if x < -0.014 {
                    -0.367_268_45 * (-x * 14.983_25 + 1.0).log10() + 0.127_839_01
                } else if x <= 0.014 {
                    1.975_479_8 * x + 0.125_122_19
                } else {
                    0.367_268_45 * (x * 14.983_25 + 1.0).log10() + 0.122_405_37
                }
            }
            Curve::Log3G10 => {
                let (a, b, c, g) = LOG3G10;
                let x = x + c;
                if x < 0.0 { x * g } else { a * (x * b + 1.0).log10() }
            }
            Curve::BmdFilm5 => {
                let (a, b, c, d, e, cut) = BMD5;
                if x < cut { d * x + e } else { a * (x + b).ln() + c }
            }
            Curve::DLog => {
                if x <= 0.0078 {
                    6.025 * x + 0.0929
                } else {
                    (x * 0.9892 + 0.0108).log10() * 0.256_663 + 0.584_555
                }
            }
        }
    }
}

fn pq() -> (f64, f64, f64, f64, f64) {
    (2610.0 / 16384.0, 2523.0 / 4096.0 * 128.0, 3424.0 / 4096.0, 2413.0 / 4096.0 * 32.0, 2392.0 / 4096.0 * 32.0)
}

fn hlg() -> (f64, f64, f64) {
    let a = 0.178_832_77;
    let b = 1.0 - 4.0 * a;
    (a, b, 0.5 - a * (4.0 * a).ln())
}

/// LogC3 at EI 800: cut, a, b, c, d, e, f.
const LOGC3: (f64, f64, f64, f64, f64, f64, f64) = (0.010_591, 5.555_556, 0.052_272, 0.247_190, 0.385_537, 5.367_655, 0.092_809);
/// Log3G10: a, b, c, g.
const LOG3G10: (f64, f64, f64, f64) = (0.224_282, 155.975_327, 0.01, 15.1927);
/// Film Gen 5: a, b, c, d, e and the linear cut.
const BMD5: (f64, f64, f64, f64, f64, f64) = (0.086_928_760_654_912_24, 0.005_494_072_432_257_808, 0.530_013_339_229_193_9, 8.283_605_932_402_494, 0.092_465_753_424_657_53, 0.005);

/// LogC4: a, b, c, s, t.
fn logc4() -> (f64, f64, f64, f64, f64) {
    let a = (2f64.powi(18) - 16.0) / 117.45;
    let b = (1023.0 - 95.0) / 1023.0;
    let c = 95.0 / 1023.0;
    let s = (7.0 * std::f64::consts::LN_2 * 2f64.powf(7.0 - 14.0 * c / b)) / (a * b);
    let t = (2f64.powf(14.0 * (-c / b) + 6.0) - 64.0) / a;
    (a, b, c, s, t)
}

/// A colour space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Space {
    pub name: &'static str,
    pub primaries: [[f64; 2]; 3],
    pub white: [f64; 2],
    pub curve: Curve,
    /// Values meant for a display (an output), not light in a scene.
    pub display: bool,
}

const fn space(name: &'static str, primaries: [[f64; 2]; 3], white: [f64; 2], curve: Curve) -> Space {
    Space { name, primaries, white, curve, display: false }
}

const fn screen(name: &'static str, primaries: [[f64; 2]; 3], white: [f64; 2], curve: Curve) -> Space {
    Space { name, primaries, white, curve, display: true }
}

/// The spaces known by name (with the names the ACES configurations use where they have one).
pub const SPACES: &[Space] = &[
    space("ACES2065-1", AP0, ACES_WHITE, Curve::Linear),
    space("ACEScg", AP1, ACES_WHITE, Curve::Linear),
    space("ACEScct", AP1, ACES_WHITE, Curve::AcesCct),
    space("ACEScc", AP1, ACES_WHITE, Curve::AcesCc),
    space("Linear Rec.709 (sRGB)", REC709, D65, Curve::Linear),
    space("Linear Rec.2020", REC2020, D65, Curve::Linear),
    space("Linear P3-D65", P3, D65, Curve::Linear),
    space("sRGB - Texture", REC709, D65, Curve::Srgb),
    space("Rec.709 - Camera", REC709, D65, Curve::Rec709),
    space("Adobe RGB (1998)", ADOBE_RGB, D65, Curve::Gamma(563.0 / 256.0)),
    space("ProPhoto RGB", PROPHOTO, D50, Curve::Romm),
    space("ARRI LogC3 (EI800) - AWG3", ARRI_WIDE_GAMUT_3, D65, Curve::LogC3),
    space("Linear ARRI Wide Gamut 3", ARRI_WIDE_GAMUT_3, D65, Curve::Linear),
    space("ARRI LogC4 - AWG4", ARRI_WIDE_GAMUT_4, D65, Curve::LogC4),
    space("Linear ARRI Wide Gamut 4", ARRI_WIDE_GAMUT_4, D65, Curve::Linear),
    space("S-Log3 S-Gamut3", S_GAMUT3, D65, Curve::SLog3),
    space("S-Log3 S-Gamut3.Cine", S_GAMUT3_CINE, D65, Curve::SLog3),
    space("Linear S-Gamut3", S_GAMUT3, D65, Curve::Linear),
    space("V-Log V-Gamut", V_GAMUT, D65, Curve::VLog),
    space("Linear V-Gamut", V_GAMUT, D65, Curve::Linear),
    space("Canon Log 2 CinemaGamut D55", CINEMA_GAMUT, D65, Curve::CanonLog2),
    space("Canon Log 3 CinemaGamut D55", CINEMA_GAMUT, D65, Curve::CanonLog3),
    space("Linear CinemaGamut", CINEMA_GAMUT, D65, Curve::Linear),
    space("Log3G10 REDWideGamutRGB", RED_WIDE_GAMUT, D65, Curve::Log3G10),
    space("Linear REDWideGamutRGB", RED_WIDE_GAMUT, D65, Curve::Linear),
    space("BMDFilm WideGamut Gen5", BMD_WIDE_GAMUT_5, D65, Curve::BmdFilm5),
    space("Linear BMD WideGamut Gen5", BMD_WIDE_GAMUT_5, D65, Curve::Linear),
    space("DJI D-Log D-Gamut", D_GAMUT, D65, Curve::DLog),
    space("Rec.2100-HLG (scene)", REC2020, D65, Curve::Hlg),
    space("CIE XYZ D65", [[1.0, 0.0], [0.0, 1.0], [0.0, 0.0]], D65, Curve::Linear),
    screen("sRGB - Display", REC709, D65, Curve::Srgb),
    screen("Rec.1886 Rec.709 - Display", REC709, D65, Curve::Gamma(2.4)),
    screen("Rec.1886 Rec.2020 - Display", REC2020, D65, Curve::Gamma(2.4)),
    screen("Display P3 - Display", P3, D65, Curve::Srgb),
    screen("P3-D65 - Display", P3, D65, Curve::Gamma(2.6)),
    screen("P3-DCI - Display", P3, DCI_WHITE, Curve::Gamma(2.6)),
    screen("Rec.2100-PQ - Display", REC2020, D65, Curve::Pq),
    screen("Gamma 2.2 Rec.709 - Display", REC709, D65, Curve::Gamma(2.2)),
];

/// The space with this name (or one of its common other names), ignoring case and spacing.
pub fn find(name: &str) -> Option<&'static Space> {
    let key = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    let k = key(name);
    let alias = match k.as_str() {
        "srgb" | "srgbiec6196621" => "sRGB - Texture",
        "rec709" | "hdtvrec709" | "bt709" => "Rec.1886 Rec.709 - Display",
        "rec2020" | "bt2020" => "Rec.1886 Rec.2020 - Display",
        "displayp3" | "p3" => "Display P3 - Display",
        "linear" | "linearsrgb" | "lin709" | "linearrec709" => "Linear Rec.709 (sRGB)",
        "aces" | "ap0" => "ACES2065-1",
        "xyz" | "ciexyz" => "CIE XYZ D65",
        _ => "",
    };
    let k = if alias.is_empty() { k } else { key(alias) };
    SPACES.iter().find(|s| key(s.name) == k)
}

impl Space {
    /// From this space's linear RGB to XYZ with a D65 white (`adapt`: its white made D65's, as
    /// the eye does; else as measured).
    pub fn to_xyz(&self, adapt: bool) -> M3 {
        if self.primaries == [[1.0, 0.0], [0.0, 1.0], [0.0, 0.0]] {
            return IDENTITY;
        }
        let m = rgb_to_xyz(self.primaries, self.white);
        if adapt { mul(&bradford(self.white, D65), &m) } else { m }
    }
}

/// From `a`'s linear RGB to `b`'s (whites adapted).
pub fn matrix(a: &Space, b: &Space) -> M3 {
    mul(&invert(&b.to_xyz(true)), &a.to_xyz(true))
}

/// The ACES reference gamut compression (Academy, 2022): colours past the edges of the gamut
/// (a channel below zero) brought back inside, those well inside left alone. On linear values.
pub fn compress_gamut(c: [f64; 3]) -> [f64; 3] {
    compress_gamut_with(c, [1.147, 1.264, 1.312], [0.815, 0.803, 0.880], 1.2, false)
}

/// The same with its own settings: how far out each channel's distance from the achromatic
/// may reach (`limit`, cyan, magenta, yellow) and still land on the edge, where squeezing
/// starts (`threshold`) and how hard it bends (`power`); `back` undoes it.
pub fn compress_gamut_with(c: [f64; 3], limit: [f64; 3], threshold: [f64; 3], power: f64, back: bool) -> [f64; 3] {
    let ach = c[0].max(c[1]).max(c[2]);
    if ach <= 0.0 {
        return c;
    }
    let p = power.max(1e-6);
    let squeeze = |d: f64, lim: f64, thr: f64| {
        if d < thr || lim <= thr {
            return d;
        }
        // the scale that puts the limit at 1
        let s = (lim - thr) / ((((1.0 - thr) / (lim - thr)).powf(-p)) - 1.0).powf(1.0 / p);
        if back {
            let y = ((d - thr) / s).min(1.0 - 1e-12);
            thr + s * y / (1.0 - y.powf(p)).powf(1.0 / p)
        } else {
            let n = (d - thr) / s;
            thr + s * n / (1.0 + n.powf(p)).powf(1.0 / p)
        }
    };
    [0, 1, 2].map(|i| {
        let d = (ach - c[i]) / ach.abs();
        ach - squeeze(d, limit[i], threshold[i]) * ach.abs()
    })
}

#[cfg(test)]
mod tests;
