//! DWAA and DWAB chunks (DreamWorks Animation's lossy compression): channels sorted by rules
//! the chunk carries (by the last part of their names and their kind) into three schemes —
//!
//! - lossy DCT: halves made perceptually even, cut into 8 × 8 blocks, each a discrete cosine
//!   transform whose first coefficient (DC) is kept apart, deflated, and the rest (AC) in zigzag
//!   order with runs of zeros, Huffman-coded or deflated; red, green and blue of one layer go
//!   together as luma and chroma (Rec. 709);
//! - RLE: each byte of the samples in its own plane, in runs, then deflated (alpha);
//! - the rest as they are, deflated.

use crate::codec::{runs, unsplit, unzlib};
use crate::{Channel, Sample, Window, half, huf, samples};

/// A rule: channels whose last name part is `suffix` (in any case, if so) and whose samples
/// are `sample` are packed by `scheme`; `rgb` says which of red, green and blue it is, if one.
struct Rule {
    suffix: String,
    any_case: bool,
    scheme: u8,
    rgb: Option<usize>,
    sample: Sample,
}

const UNKNOWN: u8 = 0;
const DCT: u8 = 1;
const RLE: u8 = 2;

/// The rules of files made before the rules were written in them.
fn old_rules() -> Vec<Rule> {
    let rule = |suffix: &str, scheme, rgb, sample| Rule { suffix: suffix.into(), any_case: false, scheme, rgb, sample };
    let mut v = Vec::new();
    for (s, rgb) in [("R", Some(0)), ("G", Some(1)), ("B", Some(2)), ("Y", None), ("BY", None), ("RY", None)] {
        v.push(rule(s, DCT, rgb, Sample::Half));
        v.push(rule(s, DCT, rgb, Sample::Float));
    }
    for sample in [Sample::Uint, Sample::Half, Sample::Float] {
        v.push(rule("A", RLE, None, sample));
    }
    v
}

fn sample_of(v: u8) -> Option<Sample> {
    match v {
        0 => Some(Sample::Uint),
        1 => Some(Sample::Half),
        2 => Some(Sample::Float),
        _ => None,
    }
}

/// The rules a chunk carries.
fn read_rules(b: &[u8]) -> Result<Vec<Rule>, String> {
    let mut v = Vec::new();
    let mut at = 0;
    while at < b.len() {
        let n = b[at..].iter().position(|c| *c == 0).ok_or("a DWA rule that never ends")?;
        let suffix = String::from_utf8_lossy(&b[at..at + n]).into_owned();
        at += n + 1;
        let (&flags, &kind) = (b.get(at).ok_or("a DWA rule cut short")?, b.get(at + 1).ok_or("a DWA rule cut short")?);
        at += 2;
        let rgb = (flags >> 4) as i32 - 1;
        let scheme = flags >> 2 & 3;
        if !(-1..3).contains(&rgb) || scheme > 2 {
            return Err("a DWA rule of an unknown kind".into());
        }
        let sample = sample_of(kind).ok_or("a DWA rule for samples of an unknown kind")?;
        v.push(Rule { suffix, any_case: flags & 1 != 0, scheme, rgb: (rgb >= 0).then_some(rgb as usize), sample });
    }
    Ok(v)
}

/// Perceptually even halves back to linear ones (e as OpenEXR's table has it, to come out the
/// same).
#[allow(clippy::approx_constant)]
fn to_linear() -> &'static [u16] {
    static TABLE: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let base = 2.718_281_8f64.powf(2.2) as f32;
        (0..=u16::MAX)
            .map(|i| {
                if i == 0 || i & 0x7c00 == 0x7c00 {
                    return 0;
                }
                let h = half::to_f32(i);
                let (sign, a) = (if h < 0.0 { -1.0f32 } else { 1.0 }, h.abs());
                half::from_f32(if a <= 1.0 { sign * a.powf(2.2) } else { sign * base.powf(a - 1.0) })
            })
            .collect()
    })
}

/// Raster position of each of a block's coefficients in zigzag order.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// The inverse DCT of an 8 × 8 block, rows then columns, as OpenEXR does it (π cut short and
/// its sums of products fused as a compiler fuses them, to come out as OpenEXR's do to the
/// bit).
#[allow(clippy::approx_constant)]
fn idct(data: &mut [f32; 64]) {
    let pi = 3.14159f32;
    let a = 0.5 * (pi / 4.0).cos();
    let b = 0.5 * (pi / 16.0).cos();
    let c = 0.5 * (pi / 8.0).cos();
    let d = 0.5 * (3.0 * pi / 16.0).cos();
    let e = 0.5 * (5.0 * pi / 16.0).cos();
    let f = 0.5 * (3.0 * pi / 8.0).cos();
    let g = 0.5 * (7.0 * pi / 16.0).cos();
    let pass = |x: [f32; 8]| -> [f32; 8] {
        let alpha = [c * x[2], f * x[2], c * x[6], f * x[6]];
        let beta = [
            g.mul_add(x[7], e.mul_add(x[5], b.mul_add(x[1], d * x[3]))),
            (-e).mul_add(x[7], (-b).mul_add(x[5], d.mul_add(x[1], -(g * x[3])))),
            d.mul_add(x[7], g.mul_add(x[5], e.mul_add(x[1], -(b * x[3])))),
            (-b).mul_add(x[7], d.mul_add(x[5], g.mul_add(x[1], -(e * x[3])))),
        ];
        let theta = [a * (x[0] + x[4]), alpha[0] + alpha[3], alpha[1] - alpha[2], a * (x[0] - x[4])];
        let gamma = [theta[0] + theta[1], theta[3] + theta[2], theta[3] - theta[2], theta[0] - theta[1]];
        [
            gamma[0] + beta[0],
            gamma[1] + beta[1],
            gamma[2] + beta[2],
            gamma[3] + beta[3],
            gamma[3] - beta[3],
            gamma[2] - beta[2],
            gamma[1] - beta[1],
            gamma[0] - beta[0],
        ]
    };
    for row in 0..8 {
        let x: [f32; 8] = data[row * 8..row * 8 + 8].try_into().expect("a row");
        data[row * 8..row * 8 + 8].copy_from_slice(&pass(x));
    }
    for col in 0..8 {
        let x: [f32; 8] = std::array::from_fn(|k| data[k * 8 + col]);
        for (k, v) in pass(x).into_iter().enumerate() {
            data[k * 8 + col] = v;
        }
    }
}

/// Luma and chroma (Rec. 709) back to red, green and blue (fused as [`idct`]'s sums are).
fn from_ycbcr(y: f32, cb: f32, cr: f32) -> [f32; 3] {
    [1.5747f32.mul_add(cr, y), (-0.4682f32).mul_add(cr, (-0.1873f32).mul_add(cb, y)), 1.8556f32.mul_add(cb, y)]
}

/// A channel's samples in the chunk.
struct Plan {
    nx: usize,
    ny: usize,
    sample: Sample,
    scheme: u8,
    linear: bool,
    /// Its scanlines as they come out, filled as they are decoded.
    rows: Vec<u8>,
}

/// Where the next AC and DC coefficients are.
struct Coefficients<'a> {
    ac: &'a [u16],
    ac_at: usize,
    dc: &'a [u16],
    dc_at: usize,
}

/// The lossy DCT channels of `set` (one, or red, green and blue together) decoded into their
/// rows, as halves (floats from them for float channels).
fn dct(plans: &mut [Plan], set: &[usize], co: &mut Coefficients, linear: bool) -> Result<(), String> {
    let (w, h) = (plans[set[0]].nx, plans[set[0]].ny);
    let (bw, bh) = (w.div_ceil(8), h.div_ceil(8));
    let k = set.len();
    // each component's DC coefficients come together, block by block
    let dc_of: Vec<usize> = (0..k).map(|c| co.dc_at + c * bw * bh).collect();
    if co.dc_at + k * bw * bh > co.dc.len() {
        return Err("a DWA chunk with too few DC coefficients".into());
    }
    let table = to_linear();
    let mut halves: Vec<Vec<u16>> = vec![vec![0; w * h]; k];
    for by in 0..bh {
        for bx in 0..bw {
            let mut blocks = [[0f32; 64]; 3];
            let mut constant = true;
            for c in 0..k {
                let mut zig = [0u16; 64];
                zig[0] = co.dc[dc_of[c] + by * bw + bx];
                // the AC coefficients, runs of zeros packed
                let mut last = 0;
                let mut i = 1;
                while i < 64 {
                    let v = *co.ac.get(co.ac_at).ok_or("a DWA chunk with too few AC coefficients")?;
                    co.ac_at += 1;
                    if v == 0xff00 {
                        break;
                    } else if v >> 8 == 0xff {
                        i += (v & 0xff) as usize;
                    } else {
                        zig[i] = v;
                        last = i;
                        i += 1;
                    }
                }
                let block = &mut blocks[c];
                if last == 0 {
                    block.fill(half::to_f32(zig[0]) * 0.353_553_6 * 0.353_553_6);
                } else {
                    constant = false;
                    for (z, &at) in ZIGZAG.iter().enumerate() {
                        block[at] = half::to_f32(zig[z]);
                    }
                    idct(block);
                }
            }
            if k == 3 {
                let n = if constant { 1 } else { 64 };
                let [y, cb, cr] = &mut blocks;
                for ((y, cb), cr) in y.iter_mut().zip(cb.iter_mut()).zip(cr.iter_mut()).take(n) {
                    [*y, *cb, *cr] = from_ycbcr(*y, *cb, *cr);
                }
                if constant {
                    for b in blocks.iter_mut() {
                        let v = b[0];
                        b.fill(v);
                    }
                }
            }
            for c in 0..k {
                for dy in 0..8.min(h - by * 8) {
                    for dx in 0..8.min(w - bx * 8) {
                        let v = half::from_f32(blocks[c][dy * 8 + dx]);
                        halves[c][(by * 8 + dy) * w + bx * 8 + dx] = if linear || !plans[set[c]].linear { table[v as usize] } else { v };
                    }
                }
            }
        }
    }
    co.dc_at += k * bw * bh;
    for (c, &p) in set.iter().enumerate() {
        let plan = &mut plans[p];
        plan.rows = match plan.sample {
            Sample::Float => halves[c].iter().flat_map(|v| half::to_f32(*v).to_le_bytes()).collect(),
            _ => halves[c].iter().flat_map(|v| v.to_le_bytes()).collect(),
        };
    }
    Ok(())
}

pub(crate) fn decompress(data: &[u8], channels: &[Channel], w: Window, size: usize) -> Result<Vec<u8>, String> {
    let short = || "a DWA chunk cut short".to_string();
    let count = |k: usize| -> Result<usize, String> {
        let b = data.get(k * 8..k * 8 + 8).ok_or_else(short)?;
        usize::try_from(u64::from_le_bytes(b.try_into().expect("eight bytes"))).map_err(|_| short())
    };
    let version = count(0)?;
    let (unknown_size, unknown_packed) = (count(1)?, count(2)?);
    let (ac_packed, dc_packed) = (count(3)?, count(4)?);
    let (rle_packed, rle_size, rle_raw) = (count(5)?, count(6)?, count(7)?);
    let (ac_count, dc_count, ac_deflated) = (count(8)?, count(9)?, count(10)? == 1);
    if version > 2 {
        return Err(format!("DWA version {version} is not read yet"));
    }
    let mut at = 88;
    let rules = if version >= 2 {
        let n = u16::from_le_bytes(data.get(at..at + 2).ok_or_else(short)?.try_into().expect("two bytes")) as usize;
        if n < 2 {
            return Err("a DWA chunk with broken rules".into());
        }
        let rules = read_rules(data.get(at + 2..at + n).ok_or_else(short)?)?;
        at += n;
        rules
    } else {
        old_rules()
    };
    let take = |at: &mut usize, n: usize| -> Result<&[u8], String> {
        let b = data.get(*at..at.checked_add(n).ok_or_else(short)?).ok_or_else(short)?;
        *at += n;
        Ok(b)
    };
    let (unknown_b, ac_b, dc_b, rle_b) = (take(&mut at, unknown_packed)?, take(&mut at, ac_packed)?, take(&mut at, dc_packed)?, take(&mut at, rle_packed)?);
    if ac_count > size || dc_count > size || unknown_size > size || rle_raw > size || rle_size > 2 * size + 16 {
        return Err("a DWA chunk that says it holds more than it can".into());
    }
    // each channel's scheme, and red, green and blue sets by layer
    let mut plans: Vec<Plan> = Vec::new();
    let mut sets: Vec<(String, [Option<usize>; 3])> = Vec::new();
    for (k, c) in channels.iter().enumerate() {
        let (prefix, suffix) = match c.name.rfind('.') {
            Some(i) => (&c.name[..i], &c.name[i + 1..]),
            None => ("", c.name.as_str()),
        };
        let mut scheme = UNKNOWN;
        for r in &rules {
            let same = if r.any_case { r.suffix.eq_ignore_ascii_case(suffix) } else { r.suffix == suffix };
            if same && r.sample == c.sample {
                scheme = r.scheme;
                if let Some(i) = r.rgb {
                    let at = match sets.iter().position(|(p, _)| p == prefix) {
                        Some(at) => at,
                        None => {
                            sets.push((prefix.to_string(), [None; 3]));
                            sets.len() - 1
                        }
                    };
                    sets[at].1[i] = Some(k);
                }
            }
        }
        let (nx, ny) = (samples(c.sampling[0], w.x0, w.x1), samples(c.sampling[1], w.y0, w.y1));
        plans.push(Plan { nx, ny, sample: c.sample, scheme, linear: c.linear, rows: Vec::new() });
    }
    sets.sort_by(|a, b| a.0.cmp(&b.0));
    // the unpacked streams
    let unknown = if unknown_packed > 0 { unzlib(unknown_b, unknown_size)? } else { Vec::new() };
    let ac: Vec<u16> = if ac_packed == 0 {
        Vec::new()
    } else if ac_deflated {
        unzlib(ac_b, ac_count * 2)?.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect()
    } else {
        huf::decode(ac_b, ac_count)?
    };
    let dc: Vec<u16> = if dc_packed > 0 { unsplit(unzlib(dc_b, dc_count * 2)?).as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect() } else { Vec::new() };
    let rle = if rle_raw > 0 { runs(&unzlib(rle_b, rle_size)?, rle_raw)? } else { Vec::new() };
    let mut co = Coefficients { ac: &ac, ac_at: 0, dc: &dc, dc_at: 0 };
    let mut done = vec![false; plans.len()];
    for (_, set) in &sets {
        let [Some(r), Some(g), Some(b)] = *set else { continue };
        if [r, g, b].iter().any(|k| plans[*k].scheme != DCT || plans[*k].nx != plans[r].nx || plans[*k].ny != plans[r].ny) {
            return Err("a DWA chunk whose red, green and blue are not packed alike".into());
        }
        dct(&mut plans, &[r, g, b], &mut co, true)?;
        done[r] = true;
        done[g] = true;
        done[b] = true;
    }
    let (mut unknown_at, mut rle_at) = (0, 0);
    for k in 0..plans.len() {
        if done[k] {
            continue;
        }
        let (count, s) = (plans[k].nx * plans[k].ny, plans[k].sample.size());
        let n = count * s;
        match plans[k].scheme {
            DCT => dct(&mut plans, &[k], &mut co, false)?,
            RLE => {
                // each byte of the samples in its own plane
                let b = rle.get(rle_at..rle_at + n).ok_or("a DWA chunk with too few run bytes")?;
                rle_at += n;
                plans[k].rows = (0..n).map(|i| b[(i % s) * count + i / s]).collect();
            }
            _ => {
                plans[k].rows = unknown.get(unknown_at..unknown_at + n).ok_or("a DWA chunk with too few bytes")?.to_vec();
                unknown_at += n;
            }
        }
    }
    // back to scanlines
    let mut out = Vec::with_capacity(size);
    let mut rows = vec![0usize; channels.len()];
    for y in w.y0..=w.y1 {
        for (k, c) in channels.iter().enumerate() {
            if y.rem_euclid(c.sampling[1]) != 0 {
                continue;
            }
            let row = plans[k].nx * c.sample.size();
            out.extend_from_slice(plans[k].rows.get(rows[k]..rows[k] + row).ok_or("a DWA chunk with too few rows")?);
            rows[k] += row;
        }
    }
    Ok(out)
}
