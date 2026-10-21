//! ICC colour profiles read by our own code (ICC.1 versions 2 and 4), each colour of the device
//! they describe taken to sRGB: matrix and tone-curve profiles (RGB and grey), and lookup-table
//! profiles (`A2B0`: 8- and 16-bit tables, and version 4's curves, matrix and grid) for any
//! device space (CMYK most of all), through the profile connection space (XYZ or L*a*b*, D50),
//! adapted to D65 (Bradford) for sRGB.

/// A tone curve.
#[derive(Clone, Debug)]
pub enum Curve {
    Identity,
    Gamma(f64),
    Table(Vec<f64>),
    /// A parametric curve: its type (0 to 4) and parameters g, a, b, c, d, e, f.
    Param(u16, [f64; 7]),
}

impl Curve {
    pub fn at(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Curve::Identity => x,
            Curve::Gamma(g) => x.powf(*g),
            Curve::Table(t) => {
                if t.len() < 2 {
                    return t.first().copied().unwrap_or(x);
                }
                let p = x * (t.len() - 1) as f64;
                let i = (p.floor() as usize).min(t.len() - 2);
                t[i] + (t[i + 1] - t[i]) * (p - i as f64)
            }
            Curve::Param(kind, [g, a, b, c, d, e, f]) => match kind {
                0 => x.powf(*g),
                1 => {
                    if x >= -b / a {
                        (a * x + b).max(0.0).powf(*g)
                    } else {
                        0.0
                    }
                }
                2 => {
                    if x >= -b / a {
                        (a * x + b).max(0.0).powf(*g) + c
                    } else {
                        *c
                    }
                }
                3 => {
                    if x >= *d {
                        (a * x + b).max(0.0).powf(*g)
                    } else {
                        c * x
                    }
                }
                _ => {
                    if x >= *d {
                        (a * x + b).max(0.0).powf(*g) + e
                    } else {
                        c * x + f
                    }
                }
            },
        }
    }
}

/// A grid of colours over the device's inputs, read between its points.
#[derive(Clone, Debug)]
struct Grid {
    points: Vec<usize>,
    outputs: usize,
    values: Vec<f64>,
}

impl Grid {
    fn at(&self, input: &[f64]) -> Vec<f64> {
        let n = self.points.len();
        let mut base = vec![0usize; n];
        let mut frac = vec![0.0; n];
        for i in 0..n {
            let g = self.points[i].max(1);
            let p = input.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0) * (g - 1) as f64;
            base[i] = (p.floor() as usize).min(g.saturating_sub(2));
            frac[i] = if g < 2 { 0.0 } else { p - base[i] as f64 };
        }
        // the grid's first input changes slowest
        let mut strides = vec![1usize; n];
        for i in (0..n.saturating_sub(1)).rev() {
            strides[i] = strides[i + 1] * self.points[i + 1].max(1);
        }
        let mut out = vec![0.0; self.outputs];
        for corner in 0..(1usize << n) {
            let mut w = 1.0;
            let mut index = 0;
            for i in 0..n {
                let up = corner >> i & 1 == 1;
                if up && self.points[i] < 2 {
                    w = 0.0;
                    break;
                }
                w *= if up { frac[i] } else { 1.0 - frac[i] };
                index += (base[i] + usize::from(up)) * strides[i];
            }
            if w == 0.0 {
                continue;
            }
            for (k, o) in out.iter_mut().enumerate() {
                *o += w * self.values.get(index * self.outputs + k).copied().unwrap_or(0.0);
            }
        }
        out
    }
}

#[derive(Clone, Debug)]
enum Kind {
    /// RGB: tone curves, then the primaries' XYZ.
    Matrix { primaries: [[f64; 3]; 3], curves: [Curve; 3] },
    Gray(Curve),
    /// Lookup tables: input curves, a grid, middle curves, a matrix with offsets, output curves.
    Lut { input: Vec<Curve>, grid: Option<Grid>, middle: Vec<Curve>, matrix: Option<[f64; 12]>, output: Vec<Curve>, legacy_lab: bool },
}

#[derive(Clone, Debug)]
pub struct Profile {
    channels: usize,
    lab: bool,
    kind: Kind,
}

fn u16_at(d: &[u8], at: usize) -> u16 {
    d.get(at..at + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
}

fn u32_at(d: &[u8], at: usize) -> u32 {
    d.get(at..at + 4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn s15(d: &[u8], at: usize) -> f64 {
    u32_at(d, at) as i32 as f64 / 65536.0
}

/// A curve at `at` (`curv` or `para`) and how many bytes it takes (to the next four).
fn curve(d: &[u8], at: usize) -> Option<(Curve, usize)> {
    match d.get(at..at + 4)? {
        b"curv" => {
            let n = u32_at(d, at + 8) as usize;
            let c = match n {
                0 => Curve::Identity,
                1 => Curve::Gamma(u16_at(d, at + 12) as f64 / 256.0),
                _ => Curve::Table((0..n.min(1 << 16)).map(|i| u16_at(d, at + 12 + 2 * i) as f64 / 65535.0).collect()),
            };
            Some((c, (12 + 2 * n).div_ceil(4) * 4))
        }
        b"para" => {
            let kind = u16_at(d, at + 8);
            let count = [1, 3, 4, 5, 7][(kind as usize).min(4)];
            let mut p = [0.0; 7];
            for (k, slot) in p.iter_mut().enumerate().take(count) {
                *slot = s15(d, at + 12 + 4 * k);
            }
            Some((Curve::Param(kind.min(4), p), 12 + 4 * count))
        }
        _ => None,
    }
}

fn curves(d: &[u8], at: usize, n: usize) -> Option<Vec<Curve>> {
    let mut out = Vec::with_capacity(n);
    let mut p = at;
    for _ in 0..n {
        let (c, len) = curve(d, p)?;
        out.push(c);
        p += len;
    }
    Some(out)
}

impl Profile {
    /// A profile from its file's bytes (none when it is not one that can be read).
    pub fn read(d: &[u8]) -> Option<Profile> {
        if d.len() < 132 || d.get(36..40)? != b"acsp" {
            return None;
        }
        let space = d.get(16..20)?;
        let channels = match space {
            b"GRAY" => 1,
            b"RGB " | b"Lab " | b"XYZ " | b"YCbr" | b"Luv " | b"HSV " | b"HLS " | b"CMY " => 3,
            b"CMYK" => 4,
            [n @ b'2'..=b'9', b'C', b'L', b'R'] => (n - b'0') as usize,
            [n @ b'A'..=b'F', b'C', b'L', b'R'] => (n - b'A' + 10) as usize,
            _ => return None,
        };
        let lab = d.get(20..24)? == b"Lab ";
        let tags = u32_at(d, 128) as usize;
        let tag = |sig: &[u8; 4]| -> Option<usize> { (0..tags.min(1000)).map(|i| 132 + 12 * i).find(|&r| d.get(r..r + 4) == Some(sig)).map(|r| u32_at(d, r + 4) as usize) };
        // a lookup table, when it has one
        if let Some(at) = tag(b"A2B0") {
            let kind = match d.get(at..at + 4)? {
                b"mft2" | b"mft1" => {
                    let wide = d.get(at..at + 4)? == b"mft2";
                    let (inputs, outputs, points) = (d[at + 8] as usize, d[at + 9] as usize, d[at + 10] as usize);
                    let (n_in, n_out, mut p) = if wide { (u16_at(d, at + 48) as usize, u16_at(d, at + 50) as usize, at + 52) } else { (256, 256, at + 48) };
                    let value = |p: usize| if wide { u16_at(d, p) as f64 / 65535.0 } else { *d.get(p).unwrap_or(&0) as f64 / 255.0 };
                    let size = if wide { 2 } else { 1 };
                    let mut input = Vec::with_capacity(inputs);
                    for _ in 0..inputs {
                        input.push(Curve::Table((0..n_in).map(|i| value(p + size * i)).collect()));
                        p += size * n_in;
                    }
                    let cells = points.checked_pow(inputs as u32)?.checked_mul(outputs)?;
                    if cells > 1 << 24 {
                        return None;
                    }
                    let values: Vec<f64> = (0..cells).map(|i| value(p + size * i)).collect();
                    p += size * cells;
                    let mut output = Vec::with_capacity(outputs);
                    for _ in 0..outputs {
                        output.push(Curve::Table((0..n_out).map(|i| value(p + size * i)).collect()));
                        p += size * n_out;
                    }
                    Kind::Lut { input, grid: Some(Grid { points: vec![points; inputs], outputs, values }), middle: Vec::new(), matrix: None, output, legacy_lab: wide }
                }
                b"mAB " => {
                    let (inputs, outputs) = (d[at + 8] as usize, d[at + 9] as usize);
                    let off = |k: usize| match u32_at(d, at + 12 + 4 * k) as usize {
                        0 => None,
                        o => Some(at + o),
                    };
                    let output = off(0).and_then(|p| curves(d, p, outputs)).unwrap_or_default();
                    let matrix = off(1).map(|p| std::array::from_fn(|k| s15(d, p + 4 * k)));
                    let middle = off(2).and_then(|p| curves(d, p, outputs)).unwrap_or_default();
                    let grid = off(3).and_then(|p| {
                        let points: Vec<usize> = (0..inputs).map(|i| d[p + i] as usize).collect();
                        let precision = d[p + 16] as usize;
                        let cells = points.iter().try_fold(1usize, |a, &g| a.checked_mul(g.max(1)))?.checked_mul(outputs)?;
                        if cells > 1 << 24 {
                            return None;
                        }
                        let values = (0..cells).map(|i| if precision == 1 { d[p + 20 + i] as f64 / 255.0 } else { u16_at(d, p + 20 + 2 * i) as f64 / 65535.0 }).collect();
                        Some(Grid { points, outputs, values })
                    });
                    let input = off(4).and_then(|p| curves(d, p, inputs)).unwrap_or_default();
                    Kind::Lut { input, grid, middle, matrix, output, legacy_lab: false }
                }
                _ => return None,
            };
            return Some(Profile { channels, lab, kind });
        }
        if channels == 1 {
            let (c, _) = curve(d, tag(b"kTRC")?)?;
            return Some(Profile { channels, lab, kind: Kind::Gray(c) });
        }
        if space == b"RGB " {
            let xyz = |sig: &[u8; 4]| -> Option<[f64; 3]> {
                let at = tag(sig)?;
                (d.get(at..at + 4)? == b"XYZ ").then(|| [s15(d, at + 8), s15(d, at + 12), s15(d, at + 16)])
            };
            let (r, g, b) = (xyz(b"rXYZ")?, xyz(b"gXYZ")?, xyz(b"bXYZ")?);
            let c = |sig: &[u8; 4]| curve(d, tag(sig)?).map(|c| c.0);
            return Some(Profile { channels, lab, kind: Kind::Matrix { primaries: [r, g, b], curves: [c(b"rTRC")?, c(b"gTRC")?, c(b"bTRC")?] } });
        }
        None
    }

    /// A matrix profile from its parts: the XYZ (D50) of its primaries and each one's gamma.
    pub fn from_primaries(primaries: [[f64; 3]; 3], gamma: [f64; 3]) -> Profile {
        Profile { channels: 3, lab: false, kind: Kind::Matrix { primaries, curves: gamma.map(Curve::Gamma) } }
    }

    /// A grey profile of `gamma`.
    pub fn gray(gamma: f64) -> Profile {
        Profile { channels: 1, lab: false, kind: Kind::Gray(Curve::Gamma(gamma)) }
    }

    /// How many components its device colours have.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// A device colour (each component 0 to 1) in the profile connection space: XYZ (D50).
    pub fn to_xyz(&self, c: &[f64]) -> [f64; 3] {
        let at = |i: usize| c.get(i).copied().unwrap_or(0.0);
        match &self.kind {
            Kind::Matrix { primaries: [r, g, b], curves } => {
                let (lr, lg, lb) = (curves[0].at(at(0)), curves[1].at(at(1)), curves[2].at(at(2)));
                std::array::from_fn(|k| r[k] * lr + g[k] * lg + b[k] * lb)
            }
            Kind::Gray(curve) => {
                let y = curve.at(at(0));
                [0.9642 * y, y, 0.8249 * y]
            }
            Kind::Lut { input, grid, middle, matrix, output, legacy_lab } => {
                let mut v: Vec<f64> = (0..self.channels).map(|i| input.get(i).map_or(at(i), |c| c.at(at(i)))).collect();
                if let Some(g) = grid {
                    v = g.at(&v);
                }
                if !middle.is_empty() {
                    v = v.iter().enumerate().map(|(i, &x)| middle.get(i).map_or(x, |c| c.at(x))).collect();
                }
                if let Some(m) = matrix
                    && v.len() >= 3
                {
                    v = (0..3).map(|r| m[3 * r] * v[0] + m[3 * r + 1] * v[1] + m[3 * r + 2] * v[2] + m[9 + r]).collect();
                }
                let v: Vec<f64> = v.iter().enumerate().map(|(i, &x)| output.get(i).map_or(x, |c| c.at(x))).collect();
                let get = |i: usize| v.get(i).copied().unwrap_or(0.0);
                if self.lab {
                    // version 2's 16-bit L*a*b* runs to 65280, version 4's to 65535
                    let k = if *legacy_lab { 65535.0 / 65280.0 } else { 1.0 };
                    lab_to_xyz([get(0) * k * 100.0, get(1) * k * 255.0 - 128.0, get(2) * k * 255.0 - 128.0])
                } else {
                    // XYZ as 1.15 fixed point: 1.0 is 32768
                    let k = 65535.0 / 32768.0;
                    [get(0) * k, get(1) * k, get(2) * k]
                }
            }
        }
    }

    /// A device colour in sRGB (each component 0 to 1, encoded).
    pub fn to_srgb(&self, c: &[f64]) -> [f64; 3] {
        xyz_d50_to_srgb(self.to_xyz(c))
    }
}

/// CIE L*a*b* (D50) to XYZ (D50).
pub fn lab_to_xyz(lab: [f64; 3]) -> [f64; 3] {
    let fy = (lab[0] + 16.0) / 116.0;
    let (fx, fz) = (fy + lab[1] / 500.0, fy - lab[2] / 200.0);
    let g = |t: f64| if t > 6.0 / 29.0 { t * t * t } else { 108.0 / 841.0 * (t - 4.0 / 29.0) };
    [0.9642 * g(fx), g(fy), 0.8249 * g(fz)]
}

/// XYZ (D50, the connection space's) to sRGB, adapted to D65 by Bradford, clipped and encoded.
pub fn xyz_d50_to_srgb(xyz: [f64; 3]) -> [f64; 3] {
    const M: [[f64; 3]; 3] = [[3.133_856_1, -1.616_866_7, -0.490_614_6], [-0.978_768_4, 1.916_141_5, 0.033_454], [0.071_945_3, -0.228_991_4, 1.405_242_7]];
    std::array::from_fn(|r| {
        let v = (M[r][0] * xyz[0] + M[r][1] * xyz[1] + M[r][2] * xyz[2]).clamp(0.0, 1.0);
        if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_systems_profiles_take_colours_to_srgb() {
        // sRGB's own profile keeps sRGB as it is
        let Ok(srgb) = std::fs::read("/System/Library/ColorSync/Profiles/sRGB Profile.icc") else { return };
        let p = Profile::read(&srgb).unwrap();
        for c in [[1.0, 0.0, 0.0], [0.2, 0.5, 0.8], [1.0, 1.0, 1.0]] {
            let out = p.to_srgb(&c);
            assert!(out.iter().zip(c).all(|(a, b)| (a - b).abs() < 0.01), "{c:?} {out:?}");
        }
        // CMYK: paper white, and cyan as a press prints it (no green as bright as the screen's)
        let Ok(cmyk) = std::fs::read("/System/Library/ColorSync/Profiles/Generic CMYK Profile.icc") else { return };
        let p = Profile::read(&cmyk).unwrap();
        assert_eq!(p.channels(), 4);
        let white = p.to_srgb(&[0.0, 0.0, 0.0, 0.0]);
        assert!(white.iter().all(|&v| v > 0.9), "{white:?}");
        let cyan = p.to_srgb(&[1.0, 0.0, 0.0, 0.0]);
        assert!(cyan[0] < 0.2 && cyan[1] > 0.45 && cyan[1] < 0.8 && cyan[2] > 0.75, "{cyan:?}");
        let black = p.to_srgb(&[0.0, 0.0, 0.0, 1.0]);
        assert!(black.iter().all(|&v| v < 0.3), "{black:?}");
    }
}
