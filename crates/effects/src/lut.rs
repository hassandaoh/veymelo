//! Colour lookup tables from `.cube` files: a 1D table for each channel, a 3D lattice, or both,
//! with the domain they cover. Read once and kept while the file stays the same.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// A colour lookup table.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut {
    pub size1: usize,
    pub one: Vec<[f32; 3]>,
    pub size3: usize,
    /// The lattice, red changing fastest.
    pub three: Vec<[f32; 3]>,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
}

/// How a 3D table is read between its points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Between {
    Nearest,
    Trilinear,
    Tetrahedral,
}

/// A table from `.cube` text, or what is wrong with it.
pub fn parse(text: &str) -> Result<Lut, String> {
    let mut lut = Lut { size1: 0, one: Vec::new(), size3: 0, three: Vec::new(), lo: [0.0; 3], hi: [1.0; 3] };
    let mut rows: Vec<[f32; 3]> = Vec::new();
    let three = |w: &[&str]| -> Option<[f32; 3]> { Some([w.first()?.parse().ok()?, w.get(1)?.parse().ok()?, w.get(2)?.parse().ok()?]) };
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let bad = || format!("line {}: \"{line}\" is not understood", n + 1);
        match words[0] {
            "TITLE" => {}
            "LUT_1D_SIZE" => lut.size1 = words.get(1).and_then(|v| v.parse().ok()).ok_or_else(bad)?,
            "LUT_3D_SIZE" => lut.size3 = words.get(1).and_then(|v| v.parse().ok()).ok_or_else(bad)?,
            "DOMAIN_MIN" => lut.lo = three(&words[1..]).ok_or_else(bad)?,
            "DOMAIN_MAX" => lut.hi = three(&words[1..]).ok_or_else(bad)?,
            "LUT_1D_INPUT_RANGE" | "LUT_3D_INPUT_RANGE" => {
                let (lo, hi): (f32, f32) = (words.get(1).and_then(|v| v.parse().ok()).ok_or_else(bad)?, words.get(2).and_then(|v| v.parse().ok()).ok_or_else(bad)?);
                (lut.lo, lut.hi) = ([lo; 3], [hi; 3]);
            }
            _ => rows.push(three(&words).ok_or_else(bad)?),
        }
    }
    if lut.size1 > 65536 || lut.size3 > 256 {
        return Err("the table is too big".into());
    }
    let (n1, n3) = (lut.size1, lut.size3 * lut.size3 * lut.size3);
    if n1 + n3 != rows.len() || (lut.size1 == 1 || lut.size3 == 1) {
        return Err(format!("the table has {} rows; its sizes ask for {}", rows.len(), n1 + n3));
    }
    if n1 + n3 == 0 {
        return Err("the table is empty".into());
    }
    lut.one = rows[..n1].to_vec();
    lut.three = rows[n1..].to_vec();
    Ok(lut)
}

impl Lut {
    /// A colour from 0 to 1 within the table's domain.
    fn inside(&self, c: [f32; 3]) -> [f32; 3] {
        [0, 1, 2].map(|i| ((c[i] - self.lo[i]) / (self.hi[i] - self.lo[i]).max(1e-9)).clamp(0.0, 1.0))
    }

    /// A colour through the table.
    pub fn apply(&self, c: [f32; 3], between: Between) -> [f32; 3] {
        let mut c = c;
        if self.size1 > 1 {
            let t = self.inside(c);
            c = [0, 1, 2].map(|i| {
                let x = t[i] * (self.size1 - 1) as f32;
                if between == Between::Nearest {
                    return self.one[(x.round() as usize).min(self.size1 - 1)][i];
                }
                let i0 = (x.floor() as usize).min(self.size1 - 2);
                self.one[i0][i] + (self.one[i0 + 1][i] - self.one[i0][i]) * (x - i0 as f32)
            });
        }
        if self.size3 > 1 {
            let t = self.inside(c);
            c = match between {
                Between::Nearest => self.nearest(t),
                Between::Trilinear => self.trilinear(t),
                Between::Tetrahedral => self.tetrahedral(t),
            };
        }
        c
    }

    fn at(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        let n = self.size3;
        self.three[r + g * n + b * n * n]
    }

    /// The lattice cell a colour is in, and how far into it.
    fn cell(&self, t: [f32; 3]) -> ([usize; 3], [f32; 3]) {
        let s = (self.size3 - 1) as f32;
        let mut i = [0usize; 3];
        let mut f = [0f32; 3];
        for k in 0..3 {
            let v = t[k] * s;
            i[k] = (v.floor() as usize).min(self.size3 - 2);
            f[k] = v - i[k] as f32;
        }
        (i, f)
    }

    fn nearest(&self, t: [f32; 3]) -> [f32; 3] {
        let s = (self.size3 - 1) as f32;
        let i = |x: f32| ((x * s).round() as usize).min(self.size3 - 1);
        self.at(i(t[0]), i(t[1]), i(t[2]))
    }

    fn trilinear(&self, t: [f32; 3]) -> [f32; 3] {
        let ([r, g, b], [fr, fg, fb]) = self.cell(t);
        let lerp = |a: [f32; 3], c: [f32; 3], k: f32| [0, 1, 2].map(|i| a[i] + (c[i] - a[i]) * k);
        let c00 = lerp(self.at(r, g, b), self.at(r + 1, g, b), fr);
        let c10 = lerp(self.at(r, g + 1, b), self.at(r + 1, g + 1, b), fr);
        let c01 = lerp(self.at(r, g, b + 1), self.at(r + 1, g, b + 1), fr);
        let c11 = lerp(self.at(r, g + 1, b + 1), self.at(r + 1, g + 1, b + 1), fr);
        lerp(lerp(c00, c10, fg), lerp(c01, c11, fg), fb)
    }

    /// Between the four corners of the tetrahedron of the cell the colour is in (smoother along
    /// the grey axis than trilinear).
    fn tetrahedral(&self, t: [f32; 3]) -> [f32; 3] {
        let ([r, g, b], [x, y, z]) = self.cell(t);
        let c000 = self.at(r, g, b);
        let c111 = self.at(r + 1, g + 1, b + 1);
        let mix = |a: [f32; 3], wa: f32, p: [f32; 3], wp: f32, q: [f32; 3], wq: f32, c: [f32; 3], wc: f32| [0, 1, 2].map(|i| a[i] * wa + p[i] * wp + q[i] * wq + c[i] * wc);
        if x >= y && y >= z {
            mix(c000, 1.0 - x, self.at(r + 1, g, b), x - y, self.at(r + 1, g + 1, b), y - z, c111, z)
        } else if x >= z && z >= y {
            mix(c000, 1.0 - x, self.at(r + 1, g, b), x - z, self.at(r + 1, g, b + 1), z - y, c111, y)
        } else if z >= x && x >= y {
            mix(c000, 1.0 - z, self.at(r, g, b + 1), z - x, self.at(r + 1, g, b + 1), x - y, c111, y)
        } else if y >= x && x >= z {
            mix(c000, 1.0 - y, self.at(r, g + 1, b), y - x, self.at(r + 1, g + 1, b), x - z, c111, z)
        } else if y >= z && z >= x {
            mix(c000, 1.0 - y, self.at(r, g + 1, b), y - z, self.at(r, g + 1, b + 1), z - x, c111, x)
        } else {
            mix(c000, 1.0 - z, self.at(r, g, b + 1), z - y, self.at(r, g + 1, b + 1), y - x, c111, x)
        }
    }
}

type Kept = HashMap<String, (Option<SystemTime>, Result<Arc<Lut>, String>)>;

/// The table in file `path` (or `.cube` text given in place of a path), read once while the
/// file is unchanged.
pub fn load(path: &str) -> Result<Arc<Lut>, String> {
    static KEPT: Mutex<Option<Kept>> = Mutex::new(None);
    let path = path.trim();
    if path.contains('\n') || path.contains("LUT_") {
        return parse(path).map(Arc::new);
    }
    let changed = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Ok(kept) = KEPT.lock()
        && let Some((when, lut)) = kept.as_ref().and_then(|k| k.get(path))
        && *when == changed
    {
        return lut.clone();
    }
    let lut = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}")).and_then(|t| parse(&t).map_err(|e| format!("{path}: {e}"))).map(Arc::new);
    if let Ok(mut kept) = KEPT.lock() {
        kept.get_or_insert_with(HashMap::new).insert(path.to_string(), (changed, lut.clone()));
    }
    lut
}
