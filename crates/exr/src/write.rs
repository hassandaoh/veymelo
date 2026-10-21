//! OpenEXR files written: one image, its scanlines in order, every channel 32-bit float (or
//! 16-bit half), ZIP-compressed sixteen lines at a time. A picture's layers go in as channels
//! named after them ("Title.R", "Title.G" …) beside the whole picture's R, G, B, A, as
//! multi-layer EXR readers take them.

/// One channel: its name and its samples, row by row from the top.
pub struct Channel {
    pub name: String,
    pub samples: Vec<f32>,
}

pub use crate::half::from_f32 as half;

fn attribute(out: &mut Vec<u8>, name: &str, kind: &str, value: &[u8]) {
    out.extend(name.as_bytes());
    out.push(0);
    out.extend(kind.as_bytes());
    out.push(0);
    out.extend((value.len() as i32).to_le_bytes());
    out.extend(value);
}

/// Bytes made smaller as EXR's ZIP does: split into evens and odds, each told as its step from
/// the one before, then deflated.
pub(crate) fn zip_bytes(raw: &[u8]) -> Vec<u8> {
    let mut split = Vec::with_capacity(raw.len());
    split.extend(raw.iter().step_by(2));
    split.extend(raw.iter().skip(1).step_by(2));
    let mut last = split.first().copied().unwrap_or(0) as i32;
    for b in split.iter_mut().skip(1) {
        let now = *b as i32;
        *b = (now - last + 128 + 256) as u8;
        last = now;
    }
    veymelo_zip::zlib(&split)
}

/// Sixteen lines' bytes made smaller as EXR's ZIP does (kept as they are when that is no
/// smaller).
fn zip_block(raw: &[u8]) -> Vec<u8> {
    let z = zip_bytes(raw);
    if z.len() < raw.len() { z } else { raw.to_vec() }
}

/// The bytes of an EXR file `width` × `height` holding `channels` (in any order; they are
/// written by name, as the format asks), as halves or as floats.
pub fn encode(width: usize, height: usize, channels: &[Channel], halves: bool) -> Vec<u8> {
    let mut order: Vec<&Channel> = channels.iter().collect();
    order.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    let long_names = order.iter().any(|c| c.name.len() > 31);
    let mut out = vec![0x76, 0x2f, 0x31, 0x01];
    // version 2, one image of scanlines; names past 31 bytes said so
    out.extend((2u32 | if long_names { 0x400 } else { 0 }).to_le_bytes());
    let mut list = Vec::new();
    for c in &order {
        list.extend(c.name.as_bytes());
        list.push(0);
        list.extend((if halves { 1i32 } else { 2i32 }).to_le_bytes());
        list.extend([0, 0, 0, 0]);
        list.extend(1i32.to_le_bytes());
        list.extend(1i32.to_le_bytes());
    }
    list.push(0);
    let (w, h) = (width as i32, height as i32);
    let window: Vec<u8> = [0, 0, w - 1, h - 1].iter().flat_map(|v: &i32| v.to_le_bytes()).collect();
    attribute(&mut out, "channels", "chlist", &list);
    attribute(&mut out, "compression", "compression", &[3]);
    attribute(&mut out, "dataWindow", "box2i", &window);
    attribute(&mut out, "displayWindow", "box2i", &window);
    attribute(&mut out, "lineOrder", "lineOrder", &[0]);
    attribute(&mut out, "pixelAspectRatio", "float", &1f32.to_le_bytes());
    attribute(&mut out, "screenWindowCenter", "v2f", &[0f32.to_le_bytes(), 0f32.to_le_bytes()].concat());
    attribute(&mut out, "screenWindowWidth", "float", &1f32.to_le_bytes());
    out.push(0);
    // where each block of sixteen lines starts, then the blocks
    let blocks = height.div_ceil(16);
    let table = out.len();
    out.resize(table + 8 * blocks, 0);
    for k in 0..blocks {
        let (y0, y1) = (k * 16, ((k + 1) * 16).min(height));
        let mut raw = Vec::with_capacity((y1 - y0) * width * order.len() * 4);
        for y in y0..y1 {
            for c in &order {
                for v in &c.samples[y * width..(y + 1) * width] {
                    if halves {
                        raw.extend(half(*v).to_le_bytes());
                    } else {
                        raw.extend(v.to_le_bytes());
                    }
                }
            }
        }
        let data = zip_block(&raw);
        let at = out.len() as u64;
        out[table + 8 * k..table + 8 * k + 8].copy_from_slice(&at.to_le_bytes());
        out.extend((y0 as i32).to_le_bytes());
        out.extend((data.len() as i32).to_le_bytes());
        out.extend(data);
    }
    out
}
