//! Colour spaces, each colour in it taken to sRGB: grey and RGB as they are, CMYK through the
//! system's generic CMYK profile (as the system's own drawing takes it; plainly when there is
//! none), ICC-based colours through their profiles, calibrated grey and RGB through their gamma
//! and primaries, CIE L*a*b*, indexed colours, separations and DeviceN through their tint
//! functions (`None` paints nothing), and patterns.

use std::sync::{Arc, OnceLock};

use veymelo_color::icc::{Profile, lab_to_xyz, xyz_d50_to_srgb};

use crate::file::File;
use crate::function::Function;
use crate::object::{Dict, Obj};

#[derive(Clone, Debug)]
pub enum Space {
    Gray,
    Rgb,
    Cmyk,
    Lab { range: [f64; 4] },
    Icc(Arc<Profile>),
    Indexed { base: Box<Space>, hival: usize, lookup: Vec<u8> },
    /// A separation or DeviceN: its tints through its function to its alternate space.
    Tint { n: usize, alternate: Box<Space>, function: Option<Function>, none: bool },
    Pattern(Option<Box<Space>>),
}

/// The system's generic CMYK profile (as its own drawing of DeviceCMYK uses).
fn generic_cmyk() -> Option<&'static Profile> {
    static P: OnceLock<Option<Profile>> = OnceLock::new();
    P.get_or_init(|| std::fs::read("/System/Library/ColorSync/Profiles/Generic CMYK Profile.icc").ok().and_then(|d| Profile::read(&d))).as_ref()
}

impl Space {
    /// The colour space `o` names or is (looked up in the resources' colour spaces).
    pub fn read(file: &File, o: &Obj, resources: &Dict) -> Space {
        Space::read_depth(file, o, resources, 0)
    }

    fn read_depth(file: &File, o: &Obj, resources: &Dict, depth: usize) -> Space {
        if depth > 8 {
            return Space::Gray;
        }
        let o = file.resolve(o);
        match &o {
            Obj::Name(n) => match n.as_str() {
                "DeviceGray" | "G" | "CalGray" => Space::Gray,
                "DeviceRGB" | "RGB" | "CalRGB" => Space::Rgb,
                "DeviceCMYK" | "CMYK" => Space::Cmyk,
                "Pattern" => Space::Pattern(None),
                other => match resources.get("ColorSpace").map(|c| file.resolve(c)).and_then(|c| c.as_dict().and_then(|d| d.get(other).cloned())) {
                    Some(found) => Space::read_depth(file, &found, resources, depth + 1),
                    None => Space::Gray,
                },
            },
            Obj::Array(a) => {
                let kind = a.first().and_then(Obj::as_name).unwrap_or("");
                let arg = |i: usize| a.get(i).map(|x| file.resolve(x)).unwrap_or(Obj::Null);
                match kind {
                    "CalGray" | "CalRGB" => {
                        let d = arg(1).as_dict().cloned().unwrap_or_default();
                        let v = |k: &str| file.at(&d, k).and_then(|a| a.as_array().map(|a| a.iter().filter_map(Obj::as_f64).collect::<Vec<f64>>())).unwrap_or_default();
                        let w = v("WhitePoint");
                        let white = [w.first().copied().unwrap_or(0.9505), w.get(1).copied().unwrap_or(1.0), w.get(2).copied().unwrap_or(1.089)];
                        // to D50 by the white points' ratio
                        let d50 = [0.9642, 1.0, 0.8249];
                        if kind == "CalGray" {
                            let g = file.at(&d, "Gamma").and_then(|g| g.as_f64()).unwrap_or(1.0);
                            return Space::Icc(Arc::new(Profile::gray(g)));
                        }
                        let g = v("Gamma");
                        let m = v("Matrix");
                        let m = if m.len() == 9 { m } else { vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] };
                        let prim = |k: usize| std::array::from_fn(|c| m[3 * k + c] * d50[c] / white[c]);
                        Space::Icc(Arc::new(Profile::from_primaries([prim(0), prim(1), prim(2)], [g.first().copied().unwrap_or(1.0), g.get(1).copied().unwrap_or(1.0), g.get(2).copied().unwrap_or(1.0)])))
                    }
                    "DeviceGray" | "G" => Space::Gray,
                    "DeviceRGB" | "RGB" => Space::Rgb,
                    "DeviceCMYK" | "CMYK" => Space::Cmyk,
                    "ICCBased" => {
                        let s = arg(1);
                        let d = s.as_dict().cloned().unwrap_or_default();
                        let n = file.at(&d, "N").and_then(|n| n.as_i64()).unwrap_or(3) as usize;
                        if let Obj::Stream(st) = &s
                            && let Some(p) = file.profile(st, a.get(1))
                            && p.channels() == n
                        {
                            return Space::Icc(p);
                        }
                        if let Some(alt) = d.get("Alternate") {
                            let alt = Space::read_depth(file, alt, resources, depth + 1);
                            if !matches!(alt, Space::Pattern(_) | Space::Indexed { .. }) {
                                return alt;
                            }
                        }
                        match file.at(&d, "N").and_then(|n| n.as_i64()).unwrap_or(3) {
                            1 => Space::Gray,
                            4 => Space::Cmyk,
                            _ => Space::Rgb,
                        }
                    }
                    "Lab" => {
                        let d = arg(1).as_dict().cloned().unwrap_or_default();
                        let v = |k: &str| file.at(&d, k).and_then(|a| a.as_array().map(|a| a.iter().filter_map(Obj::as_f64).collect::<Vec<f64>>())).unwrap_or_default();
                        let r = v("Range");
                        Space::Lab {
                            range: [r.first().copied().unwrap_or(-100.0), r.get(1).copied().unwrap_or(100.0), r.get(2).copied().unwrap_or(-100.0), r.get(3).copied().unwrap_or(100.0)],
                        }
                    }
                    "Indexed" | "I" => {
                        let base = Space::read_depth(file, &a.get(1).cloned().unwrap_or(Obj::Null), resources, depth + 1);
                        let hival = arg(2).as_i64().unwrap_or(0).clamp(0, 255) as usize;
                        let lookup = match arg(3) {
                            Obj::Str(s) => s,
                            Obj::Stream(s) => file.decoded(&s).map(|d| d.data).unwrap_or_default(),
                            _ => Vec::new(),
                        };
                        Space::Indexed { base: Box::new(base), hival, lookup }
                    }
                    "Separation" | "DeviceN" => {
                        let names: Vec<String> = match arg(1) {
                            Obj::Name(n) => vec![n],
                            Obj::Array(v) => v.iter().filter_map(|x| x.as_name().map(str::to_string)).collect(),
                            _ => Vec::new(),
                        };
                        let none = names.iter().all(|n| n == "None") && !names.is_empty();
                        let alternate = Space::read_depth(file, &a.get(2).cloned().unwrap_or(Obj::Null), resources, depth + 1);
                        Space::Tint { n: names.len().max(1), alternate: Box::new(alternate), function: Function::read(file, &arg(3)), none }
                    }
                    "Pattern" => Space::Pattern(a.get(1).map(|b| Box::new(Space::read_depth(file, b, resources, depth + 1)))),
                    _ => Space::Gray,
                }
            }
            _ => Space::Gray,
        }
    }

    pub fn components(&self) -> usize {
        match self {
            Space::Gray | Space::Indexed { .. } => 1,
            Space::Rgb | Space::Lab { .. } => 3,
            Space::Icc(p) => p.channels(),
            Space::Cmyk => 4,
            Space::Tint { n, .. } => *n,
            Space::Pattern(_) => 0,
        }
    }

    /// The colour a space starts with.
    pub fn initial(&self) -> Vec<f64> {
        match self {
            Space::Cmyk => vec![0.0, 0.0, 0.0, 1.0],
            Space::Tint { n, .. } => vec![1.0; *n],
            Space::Lab { .. } => vec![0.0, 0.0, 0.0],
            other => vec![0.0; other.components()],
        }
    }

    /// The ranges an image's samples are read into, by default.
    pub fn decode(&self, bpc: u32) -> Vec<f64> {
        match self {
            Space::Indexed { .. } => vec![0.0, ((1u64 << bpc) - 1) as f64],
            Space::Lab { range, .. } => vec![0.0, 100.0, range[0], range[1], range[2], range[3]],
            other => (0..other.components()).flat_map(|_| [0.0, 1.0]).collect(),
        }
    }

    /// Colour `c` in sRGB; none when it paints nothing (a separation named None).
    pub fn rgb(&self, c: &[f64]) -> Option<[f64; 3]> {
        let at = |i: usize| c.get(i).copied().unwrap_or(0.0);
        Some(match self {
            Space::Gray => {
                let g = at(0).clamp(0.0, 1.0);
                [g; 3]
            }
            Space::Rgb => [at(0), at(1), at(2)].map(|v| v.clamp(0.0, 1.0)),
            Space::Cmyk => {
                let c4 = [at(0), at(1), at(2), at(3)].map(|v| v.clamp(0.0, 1.0));
                match generic_cmyk() {
                    Some(p) => p.to_srgb(&c4),
                    None => {
                        let [cc, m, y, k] = c4;
                        [(1.0 - cc) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)]
                    }
                }
            }
            Space::Icc(p) => {
                let c: Vec<f64> = (0..p.channels()).map(|i| at(i).clamp(0.0, 1.0)).collect();
                p.to_srgb(&c)
            }
            Space::Lab { range, .. } => {
                // its colours taken as D50's (the connection space's white)
                let (l, a, b) = (at(0).clamp(0.0, 100.0), at(1).clamp(range[0], range[1]), at(2).clamp(range[2], range[3]));
                xyz_d50_to_srgb(lab_to_xyz([l, a, b]))
            }
            Space::Indexed { base, hival, lookup } => {
                let i = (at(0).round().max(0.0) as usize).min(*hival);
                let n = base.components();
                let comps: Vec<f64> = (0..n).map(|k| *lookup.get(i * n + k).unwrap_or(&0) as f64 / 255.0).collect();
                // L*a*b* entries are in its ranges
                let comps = match &**base {
                    Space::Lab { range, .. } => vec![comps[0] * 100.0, range[0] + comps[1] * (range[1] - range[0]), range[2] + comps[2] * (range[3] - range[2])],
                    _ => comps,
                };
                return base.rgb(&comps);
            }
            Space::Tint { alternate, function, none, n } => {
                if *none {
                    return None;
                }
                let input: Vec<f64> = (0..*n).map(at).collect();
                let out = match function {
                    Some(f) => f.eval(&input),
                    None => input,
                };
                return alternate.rgb(&out);
            }
            Space::Pattern(_) => [0.0; 3],
        })
    }
}
