//! Pictures in a PDF made pixels: their samples read at their bit depths through their colour
//! space and decode ranges (JPEG and JPEG 2000 left to the system), stencil masks painted in the
//! fill colour, and what shows of them from their soft masks, stencil masks or colour keys.

use std::sync::Arc;

use veymelo_raster::Image;

use crate::colour::Space;
use crate::file::File;
use crate::filters::{decode, filters_of};
use crate::object::{Dict, Obj};

/// A picture's dictionary keys, long or short (inline pictures abbreviate them).
fn key<'a>(d: &'a Dict, long: &str, short: &str) -> Option<&'a Obj> {
    d.get(long).or_else(|| d.get(short))
}

/// Inline pictures' short names for colour spaces and filters.
fn expand(o: &Obj) -> Obj {
    match o {
        Obj::Name(n) => Obj::Name(
            match n.as_str() {
                "G" => "DeviceGray",
                "RGB" => "DeviceRGB",
                "CMYK" => "DeviceCMYK",
                "I" => "Indexed",
                other => other,
            }
            .to_string(),
        ),
        Obj::Array(a) => Obj::Array(a.iter().map(expand).collect()),
        other => other.clone(),
    }
}

/// The samples of a picture `w` × `h`: each pixel's `n` components, from 0 to the largest the
/// bit depth holds, as numbers from 0 to 1.
fn samples(data: &[u8], w: usize, h: usize, n: usize, bpc: u32) -> Vec<f64> {
    let row = (w * n * bpc as usize).div_ceil(8);
    let max = ((1u64 << bpc) - 1) as f64;
    let mut out = Vec::with_capacity(w * h * n);
    for y in 0..h {
        let r = &data[(y * row).min(data.len())..((y + 1) * row).min(data.len())];
        for k in 0..w * n {
            let v = match bpc {
                8 => *r.get(k).unwrap_or(&0) as u64,
                16 => (*r.get(2 * k).unwrap_or(&0) as u64) << 8 | *r.get(2 * k + 1).unwrap_or(&0) as u64,
                _ => {
                    let bit = k * bpc as usize;
                    let mut v = 0u64;
                    for b in 0..bpc as usize {
                        let at = bit + b;
                        v = (v << 1) | ((*r.get(at / 8).unwrap_or(&0) >> (7 - at % 8)) & 1) as u64;
                    }
                    v
                }
            };
            out.push(v as f64 / max);
        }
    }
    out
}

/// A picture's pixels (premultiplied sRGB) from its dictionary and bytes (still encoded),
/// `fill` the colour a stencil mask paints; whether it asks to be smoothed.
pub fn picture(file: &File, d: &Dict, raw: &[u8], resources: &Dict, inline: bool, fill: [f64; 4], notes: &mut Vec<String>) -> Option<(Arc<Image>, bool)> {
    let get = |long: &str, short: &str| key(d, long, short).map(|o| file.resolve(o));
    let w = get("Width", "W")?.as_i64()?.clamp(1, 1 << 15) as usize;
    let h = get("Height", "H")?.as_i64()?.clamp(1, 1 << 15) as usize;
    let mask = get("ImageMask", "IM").and_then(|v| v.as_bool()) == Some(true);
    let bpc = if mask { 1 } else { get("BitsPerComponent", "BPC").and_then(|v| v.as_i64()).unwrap_or(8).clamp(1, 16) as u32 };
    let smooth = get("Interpolate", "I").and_then(|v| v.as_bool()).unwrap_or(false) || !mask;
    // its filters (inline ones abbreviated)
    let mut fd = d.clone();
    if inline {
        for (long, short) in [("Filter", "F"), ("DecodeParms", "DP")] {
            if let Some(v) = d.get(short) {
                fd.insert(long, expand(v));
            }
        }
    }
    let dec = decode(raw, &filters_of(&fd, false)).map_err(|e| notes.push(format!("a picture: {e}"))).ok()?;
    let space_obj = get("ColorSpace", "CS").map(|o| if inline { expand(&o) } else { o });
    let space = space_obj.as_ref().map(|o| Space::read(file, o, resources)).unwrap_or(Space::Gray);
    let n = if mask { 1 } else { space.components().max(1) };
    let decode_arr: Vec<f64> = get("Decode", "D").and_then(|a| a.as_array().map(|a| a.iter().filter_map(Obj::as_f64).collect())).unwrap_or_else(|| if mask { vec![0.0, 1.0] } else { space.decode(bpc) });
    let mut px: Vec<[f32; 4]> = match dec.codec.as_deref() {
        Some("DCTDecode" | "JPXDecode") => {
            let p = veymelo_os::read_picture_bytes(&dec.data).map_err(|e| notes.push(format!("a picture the system cannot read ({e})"))).ok()?;
            // the codestream's own size wins over the dictionary's
            let inverted = decode_arr.len() >= 2 && decode_arr[0] > decode_arr[1];
            let mut px = p.px;
            if inverted {
                for q in px.iter_mut() {
                    *q = [q[3] - q[0], q[3] - q[1], q[3] - q[2], q[3]];
                }
            }
            return finish(file, d, resources, Image { width: p.width, height: p.height, px: px.into() }, smooth, notes);
        }
        Some(other) => {
            notes.push(format!("pictures in {other}"));
            return None;
        }
        None => {
            let s = samples(&dec.data, w, h, n, bpc);
            if mask {
                // 0 paints, unless its decode says 1
                let paints_on = decode_arr.first().copied().unwrap_or(0.0) > 0.5;
                s.iter()
                    .map(|&v| {
                        let on = (v > 0.5) == paints_on;
                        let a = if on { fill[3] } else { 0.0 } as f32;
                        [fill[0] as f32 * a, fill[1] as f32 * a, fill[2] as f32 * a, a]
                    })
                    .collect()
            } else {
                s.chunks(n)
                    .map(|c| {
                        let comps: Vec<f64> = c.iter().enumerate().map(|(k, &v)| decode_arr.get(2 * k).copied().unwrap_or(0.0) + v * (decode_arr.get(2 * k + 1).copied().unwrap_or(1.0) - decode_arr.get(2 * k).copied().unwrap_or(0.0))).collect();
                        match space.rgb(&comps) {
                            Some([r, g, b]) => [r as f32, g as f32, b as f32, 1.0],
                            None => [0.0; 4],
                        }
                    })
                    .collect()
            }
        }
    };
    // colour keys: samples in every range shown as nothing
    if let Some(Obj::Array(keys)) = get("Mask", "Mask") {
        let ranges: Vec<i64> = keys.iter().filter_map(Obj::as_i64).collect();
        if ranges.len() >= 2 * n {
            let raw = samples(&dec.data, w, h, n, bpc);
            let max = ((1u64 << bpc) - 1) as f64;
            for (i, c) in raw.chunks(n).enumerate() {
                if c.iter().enumerate().all(|(k, &v)| {
                    let s = (v * max).round() as i64;
                    s >= ranges[2 * k] && s <= ranges[2 * k + 1]
                }) {
                    px[i] = [0.0; 4];
                }
            }
        }
    }
    finish(file, d, resources, Image { width: w, height: h, px: px.into() }, smooth, notes)
}

/// What shows of a picture: its soft mask, or its stencil mask, stretched over it.
fn finish(file: &File, d: &Dict, resources: &Dict, mut img: Image, smooth: bool, notes: &mut Vec<String>) -> Option<(Arc<Image>, bool)> {
    let soft = d.get("SMask").map(|o| file.resolve(o));
    let stencil = match d.get("Mask").map(|o| file.resolve(o)) {
        Some(Obj::Stream(s)) => Some(s),
        _ => None,
    };
    let alpha: Option<Image> = match (soft, stencil) {
        (Some(Obj::Stream(s)), _) => {
            let mut sd = s.dict.clone();
            sd.insert("ColorSpace", Obj::Name("DeviceGray".into()));
            let plain_white = [1.0; 4];
            picture(file, &sd, &raw_of(file, &s)?, resources, false, plain_white, notes).map(|(m, _)| (*m).clone())
        }
        (_, Some(s)) => {
            // a stencil's 1s are what is left out, as an image mask's are
            let mut sd = s.dict.clone();
            sd.insert("ImageMask", Obj::Bool(true));
            picture(file, &sd, &raw_of(file, &s)?, resources, false, [1.0; 4], notes).map(|(m, _)| (*m).clone())
        }
        _ => None,
    };
    if let Some(a) = alpha {
        // the mask's pixels stretched over the picture's
        let (w, h) = (img.width, img.height);
        for y in 0..h {
            let my = (y * a.height / h).min(a.height - 1);
            for x in 0..w {
                let mx = (x * a.width / w).min(a.width - 1);
                let q = a.px[my * a.width + mx];
                // a soft mask's grey, a stencil's white where it shows
                let k = q[0];
                let p = &mut img.px[y * w + x];
                *p = p.map(|v| v * k);
            }
        }
    }
    Some((Arc::new(img), smooth))
}

/// A stream's bytes as the file has them (unlocked when the file is locked).
fn raw_of(file: &File, s: &crate::object::Stream) -> Option<Vec<u8>> {
    file.unlocked(s)
}
