//! Photoshop documents (`.psd`, and `.psb` large documents), read by our own reader from Adobe's
//! published Photoshop File Formats Specification: the file header, colour mode data and image
//! resources (passed over), the layer and mask information (each layer's record: its bounds,
//! channels, blend mode, opacity, clipping, visibility, layer mask and name; the additional
//! information it carries: its Unicode name, group dividers with their blend mode, fill opacity,
//! and whether it is an adjustment, type or smart object layer), and each layer's channels in
//! every compression (raw, RLE, ZIP with or without prediction) at 8, 16 and 32 bits, in RGB,
//! grayscale, CMYK and Lab.
//!
//! Layers are kept as the file keeps them, bottom first; [`Psd::tree`] lays them out top first,
//! groups holding what is in them.

use std::sync::Arc;

pub mod write;

/// The rectangle a layer covers: top, left, bottom, right (document pixels).
pub type Bounds = [i32; 4];

/// A layer's mask: the rectangle it covers, what it is outside it (0 hidden, 255 shown), and
/// whether it is switched off.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub bounds: Bounds,
    pub default: u8,
    pub disabled: bool,
}

/// What a layer record is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Pixels (or nothing).
    Pixels,
    /// A group's folder (open or closed), which comes after what is in it.
    Group,
    /// The hidden divider that ends a group, which comes before what is in it.
    GroupEnd,
}

/// A channel of a layer: its id (0, 1, 2… colour, −1 transparency, −2 the layer mask, −3 the
/// real mask) and where its data is in the file.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub id: i16,
    at: usize,
    len: usize,
}

/// A layer record.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub name: String,
    pub bounds: Bounds,
    /// Its blend mode's key (`norm`, `mul `, `scrn`…; a group's `pass` for pass through).
    pub blend: [u8; 4],
    pub opacity: u8,
    /// Its fill opacity (the pixels', not their effects').
    pub fill: u8,
    pub clipped: bool,
    pub hidden: bool,
    pub kind: Kind,
    pub mask: Option<Mask>,
    pub channels: Vec<Channel>,
    /// An adjustment or fill layer (levels, curves, hue…; a solid colour, gradient or pattern
    /// fill, whose pixels the file keeps only as a stand-in), a type layer, a smart object.
    pub adjustment: bool,
    pub text: bool,
    pub smart: bool,
}

/// A layer's pixels: where its rectangle starts in the document, its size, and its colours
/// (straight sRGB) and alpha from 0 to 1, row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct Pixels {
    pub left: i32,
    pub top: i32,
    pub width: usize,
    pub height: usize,
    pub px: Vec<[f32; 4]>,
}

/// Layers as a tree, top first: a layer (its index), or a group (its folder's index) with what is
/// in it.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Layer(usize),
    Group(usize, Vec<Node>),
}

/// A Photoshop document.
#[derive(Clone, Debug)]
pub struct Psd {
    pub width: u32,
    pub height: u32,
    /// Bits a channel: 1, 8, 16 or 32.
    pub depth: u16,
    /// 1 grayscale, 3 RGB, 4 CMYK, 9 Lab (others: 0 bitmap, 2 indexed, 7 multichannel, 8 duotone).
    pub mode: u16,
    /// A large document (`.psb`).
    pub big: bool,
    pub layers: Vec<Layer>,
    data: Arc<Vec<u8>>,
}

/// Reads big-endian numbers off the file.
struct Read<'a> {
    d: &'a [u8],
    at: usize,
}

impl Read<'_> {
    fn bytes(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.d.len()).ok_or("the file is cut short")?;
        let out = &self.d[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32(&mut self) -> Result<i32, String> {
        Ok(self.u32()? as i32)
    }

    fn u64(&mut self) -> Result<u64, String> {
        let b = self.bytes(8)?;
        Ok(u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    /// A length: four bytes, or eight in a large document where `wide`.
    fn len(&mut self, wide: bool) -> Result<usize, String> {
        Ok(if wide { self.u64()? as usize } else { self.u32()? as usize })
    }
}

/// The additional information keys whose lengths are eight bytes in a large document.
const WIDE_KEYS: [&[u8; 4]; 13] = [b"LMsk", b"Lr16", b"Lr32", b"Layr", b"Mt16", b"Mt32", b"Mtrn", b"Alph", b"FMsk", b"lnk2", b"FEid", b"FXid", b"PxSD"];

/// The keys of adjustment layers' information.
const ADJUSTMENTS: [&[u8; 4]; 22] = [
    b"levl", b"curv", b"brit", b"blnc", b"hue ", b"hue2", b"selc", b"thrs", b"nvrt", b"post", b"mixr", b"grdm", b"phfl", b"expA", b"vibA", b"blwh", b"CgEd", b"clrL", b"SoCo", b"GdFl", b"PtFl", b"brst",
];

/// PackBits: runs of bytes copied and bytes repeated, until `n` bytes are made.
fn unpack(src: &[u8], n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < src.len() && out.len() < n {
        let h = src[i] as i8;
        i += 1;
        if h >= 0 {
            let k = h as usize + 1;
            out.extend_from_slice(&src[i..(i + k).min(src.len())]);
            i += k;
        } else if h != -128 {
            let k = (1 - h as i32) as usize;
            if let Some(&b) = src.get(i) {
                out.extend(std::iter::repeat_n(b, k));
            }
            i += 1;
        }
    }
    out.resize(n, 0);
    out
}

/// A channel's samples (`w` × `h`, `bytes` each, big-endian), from its data in the file.
fn samples(data: &[u8], w: usize, h: usize, bytes: usize, big: bool) -> Result<Vec<u8>, String> {
    let mut r = Read { d: data, at: 0 };
    let compression = r.u16()?;
    let n = w * h * bytes;
    let rest = &data[2..];
    Ok(match compression {
        0 => {
            let mut v = rest[..n.min(rest.len())].to_vec();
            v.resize(n, 0);
            v
        }
        1 => {
            // the rows' packed lengths, then the rows
            let (each, counts) = if big { (4, h * 4) } else { (2, h * 2) };
            let counts = rest.get(..counts).ok_or("an RLE channel is cut short")?;
            let mut at = counts.len();
            let mut out = Vec::with_capacity(n);
            for row in 0..h {
                let c = &counts[row * each..row * each + each];
                let len = if each == 4 { u32::from_be_bytes([c[0], c[1], c[2], c[3]]) as usize } else { u16::from_be_bytes([c[0], c[1]]) as usize };
                let end = (at + len).min(rest.len());
                out.extend(unpack(&rest[at..end], w * bytes));
                at = end;
            }
            out
        }
        2 | 3 => {
            let mut v = veymelo_zip::unzlib(rest)?;
            v.resize(n, 0);
            if compression == 3 {
                // each row stored as differences from the sample before
                for row in v.chunks_mut((w * bytes).max(1)) {
                    match bytes {
                        1 => {
                            for x in 1..row.len() {
                                row[x] = row[x].wrapping_add(row[x - 1]);
                            }
                        }
                        2 => {
                            for x in 1..w {
                                let prev = u16::from_be_bytes([row[2 * x - 2], row[2 * x - 1]]);
                                let cur = u16::from_be_bytes([row[2 * x], row[2 * x + 1]]).wrapping_add(prev);
                                row[2 * x..2 * x + 2].copy_from_slice(&cur.to_be_bytes());
                            }
                        }
                        _ => {
                            // 32 bits: the bytes differenced in a row, then laid out as four
                            // planes (each sample's first bytes, then its second bytes…)
                            for x in 1..row.len() {
                                row[x] = row[x].wrapping_add(row[x - 1]);
                            }
                            let planes = row.to_vec();
                            for x in 0..w {
                                for b in 0..4 {
                                    row[4 * x + b] = planes[b * w + x];
                                }
                            }
                        }
                    }
                }
            }
            v
        }
        other => return Err(format!("a channel is compressed in a way not known ({other})")),
    })
}

/// sRGB from linear light.
fn encoded(v: f32) -> f32 {
    let v = v.max(0.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// sRGB from CIE Lab (D50, as Photoshop's Lab is).
fn lab_to_rgb(l: f32, a: f32, b: f32) -> [f32; 3] {
    let fy = (l + 16.0) / 116.0;
    let (fx, fz) = (fy + a / 500.0, fy - b / 200.0);
    let f = |t: f32| if t > 6.0 / 29.0 { t * t * t } else { 3.0 * (6.0f32 / 29.0).powi(2) * (t - 4.0 / 29.0) };
    let (x, y, z) = (0.9642 * f(fx), f(fy), 0.8249 * f(fz));
    // D50 XYZ to linear sRGB (Bradford adapted)
    let r = 3.133_856 * x - 1.616_867 * y - 0.490_615 * z;
    let g = -0.978_768 * x + 1.916_141 * y + 0.033_454 * z;
    let bl = 0.071_945 * x - 0.228_991 * y + 1.405_243 * z;
    [encoded(r).min(1.0), encoded(g).min(1.0), encoded(bl).min(1.0)]
}

impl Psd {
    /// The layers top first, groups holding what is in them (dividers left out).
    pub fn tree(&self) -> Vec<Node> {
        // the file goes bottom up: a divider opens a group, its folder closes it
        let mut stack: Vec<Vec<Node>> = vec![Vec::new()];
        for (i, l) in self.layers.iter().enumerate() {
            match l.kind {
                Kind::GroupEnd => stack.push(Vec::new()),
                Kind::Group => {
                    let mut kids = if stack.len() > 1 { stack.pop().expect("more than one") } else { Vec::new() };
                    kids.reverse();
                    stack.last_mut().expect("the bottom").push(Node::Group(i, kids));
                }
                Kind::Pixels => stack.last_mut().expect("the bottom").push(Node::Layer(i)),
            }
        }
        // groups never closed join what holds them
        while stack.len() > 1 {
            let kids = stack.pop().expect("more than one");
            stack.last_mut().expect("the bottom").extend(kids);
        }
        let mut top = stack.pop().unwrap_or_default();
        top.reverse();
        top
    }

    /// Layer `i`'s pixels, its layer mask applied (when it is on). None for a layer with no
    /// pixels.
    pub fn pixels(&self, i: usize) -> Option<Pixels> {
        let l = self.layers.get(i)?;
        let [top, left, bottom, right] = l.bounds;
        let (w, h) = ((right - left).max(0) as usize, (bottom - top).max(0) as usize);
        if w == 0 || h == 0 || w * h > 1 << 28 {
            return None;
        }
        let bytes = match self.depth {
            16 => 2,
            32 => 4,
            _ => 1,
        };
        let value = |s: &[u8], k: usize| -> f32 {
            match bytes {
                2 => u16::from_be_bytes([s[2 * k], s[2 * k + 1]]) as f32 / 65535.0,
                4 => encoded(f32::from_be_bytes([s[4 * k], s[4 * k + 1], s[4 * k + 2], s[4 * k + 3]])),
                _ => s[k] as f32 / 255.0,
            }
        };
        let channel = |id: i16| -> Option<Vec<f32>> {
            let c = l.channels.iter().find(|c| c.id == id)?;
            let data = self.data.get(c.at..c.at + c.len)?;
            let s = samples(data, w, h, bytes, self.big).ok()?;
            Some((0..w * h).map(|k| value(&s, k)).collect())
        };
        let colours = match self.mode {
            1 | 8 => 1,
            4 => 4,
            _ => 3,
        };
        let planes: Vec<Option<Vec<f32>>> = (0..colours).map(|c| channel(c as i16)).collect();
        let alpha = channel(-1);
        let mut out: Vec<[f32; 4]> = (0..w * h)
            .map(|k| {
                let get = |c: usize| planes.get(c).and_then(|p| p.as_ref()).map_or(0.0, |p| p[k]);
                let rgb = match self.mode {
                    1 | 8 => [get(0); 3],
                    // stored inverted: 1 is no ink
                    4 => {
                        let kk = get(3);
                        [get(0) * kk, get(1) * kk, get(2) * kk]
                    }
                    9 => lab_to_rgb(get(0) * 100.0, get(1) * 255.0 - 128.0, get(2) * 255.0 - 128.0),
                    _ => [get(0), get(1), get(2)],
                };
                [rgb[0], rgb[1], rgb[2], alpha.as_ref().map_or(1.0, |a| a[k])]
            })
            .collect();
        // the layer mask, outside its rectangle as its default says
        if let Some(m) = l.mask.as_ref().filter(|m| !m.disabled) {
            let [mt, ml, mb, mr] = m.bounds;
            let (mw, mh) = ((mr - ml).max(0) as usize, (mb - mt).max(0) as usize);
            let plane = l
                .channels
                .iter()
                .find(|c| c.id == -2)
                .filter(|_| mw > 0 && mh > 0)
                .and_then(|c| self.data.get(c.at..c.at + c.len))
                .and_then(|d| samples(d, mw, mh, bytes, self.big).ok())
                .map(|s| (0..mw * mh).map(|k| value(&s, k)).collect::<Vec<f32>>());
            let outside = m.default as f32 / 255.0;
            for y in 0..h {
                for x in 0..w {
                    let (dx, dy) = (left + x as i32 - ml, top + y as i32 - mt);
                    let k = match &plane {
                        Some(p) if dx >= 0 && dy >= 0 && (dx as usize) < mw && (dy as usize) < mh => p[dy as usize * mw + dx as usize],
                        _ => outside,
                    };
                    out[y * w + x][3] *= k;
                }
            }
        }
        Some(Pixels { left, top, width: w, height: h, px: out })
    }
}

/// A Photoshop document from its file's bytes.
pub fn parse(bytes: Vec<u8>) -> Result<Psd, String> {
    let d = &bytes[..];
    let mut r = Read { d, at: 0 };
    if r.bytes(4)? != b"8BPS" {
        return Err("it is not a Photoshop document".into());
    }
    let version = r.u16()?;
    if version != 1 && version != 2 {
        return Err(format!("its version ({version}) is not one known"));
    }
    let big = version == 2;
    r.bytes(6)?;
    let _channels = r.u16()?;
    let (height, width) = (r.u32()?, r.u32()?);
    let (depth, mode) = (r.u16()?, r.u16()?);
    // colour mode data, image resources
    let n = r.u32()? as usize;
    r.bytes(n)?;
    let n = r.u32()? as usize;
    r.bytes(n)?;
    let mut layers = Vec::new();
    let info_len = r.len(big)?;
    let info_end = r.at + info_len;
    if info_len > 0 {
        let layers_len = r.len(big)?;
        let layers_end = r.at + layers_len;
        if layers_len > 0 {
            let count = (r.u16()? as i16).unsigned_abs() as usize;
            for _ in 0..count {
                let bounds = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
                let nch = r.u16()? as usize;
                let mut channels = Vec::with_capacity(nch);
                for _ in 0..nch {
                    let id = r.u16()? as i16;
                    let len = r.len(big)?;
                    channels.push(Channel { id, at: 0, len });
                }
                if r.bytes(4)? != b"8BIM" {
                    return Err("a layer record's blend mode is not signed".into());
                }
                let blend: [u8; 4] = r.bytes(4)?.try_into().expect("four bytes");
                let (opacity, clipping, flags) = (r.u8()?, r.u8()?, r.u8()?);
                r.u8()?;
                let extra = r.u32()? as usize;
                let extra_end = r.at + extra;
                // the layer mask
                let mask_len = r.u32()? as usize;
                let mask_end = r.at + mask_len;
                let mask = if mask_len >= 20 {
                    let mb = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
                    let (default, mflags) = (r.u8()?, r.u8()?);
                    Some(Mask { bounds: mb, default, disabled: mflags & 2 != 0 })
                } else {
                    None
                };
                r.at = mask_end;
                // blending ranges
                let n = r.u32()? as usize;
                r.bytes(n)?;
                // the name, padded to four bytes with its length
                let len = r.u8()? as usize;
                let mut name = String::from_utf8_lossy(r.bytes(len)?).to_string();
                r.bytes((4 - (len + 1) % 4) % 4)?;
                let mut layer = Layer { name: String::new(), bounds, blend, opacity, fill: 255, clipped: clipping != 0, hidden: flags & 2 != 0, kind: Kind::Pixels, mask, channels, adjustment: false, text: false, smart: false };
                // additional information
                while r.at + 12 <= extra_end {
                    let sig = r.bytes(4)?;
                    if sig != b"8BIM" && sig != b"8B64" {
                        break;
                    }
                    let key: [u8; 4] = r.bytes(4)?.try_into().expect("four bytes");
                    let len = r.len(big && WIDE_KEYS.contains(&&key))?;
                    let start = r.at;
                    let block = d.get(start..start + len).ok_or("additional layer information is cut short")?;
                    match &key {
                        b"luni" if block.len() >= 4 => {
                            let n = u32::from_be_bytes([block[0], block[1], block[2], block[3]]) as usize;
                            let units: Vec<u16> = block[4..].as_chunks::<2>().0.iter().take(n).map(|c| u16::from_be_bytes(*c)).collect();
                            name = String::from_utf16_lossy(&units).trim_end_matches('\0').to_string();
                        }
                        b"lsct" | b"lsdk" if block.len() >= 4 => {
                            layer.kind = match u32::from_be_bytes([block[0], block[1], block[2], block[3]]) {
                                1 | 2 => Kind::Group,
                                3 => Kind::GroupEnd,
                                _ => layer.kind,
                            };
                            if block.len() >= 12 && &block[4..8] == b"8BIM" {
                                layer.blend.copy_from_slice(&block[8..12]);
                            }
                        }
                        b"iOpa" if !block.is_empty() => layer.fill = block[0],
                        b"TySh" | b"tySh" => layer.text = true,
                        b"SoLd" | b"SoLE" | b"PlLd" | b"plLd" => layer.smart = true,
                        k if ADJUSTMENTS.contains(&k) => layer.adjustment = true,
                        _ => {}
                    }
                    r.at = start + len + len % 2;
                }
                layer.name = name;
                r.at = extra_end;
                layers.push(layer);
            }
            // each layer's channels, one after another
            for l in &mut layers {
                for c in &mut l.channels {
                    c.at = r.at;
                    r.bytes(c.len)?;
                }
            }
        }
        let _ = layers_end;
    }
    let _ = info_end;
    Ok(Psd { width, height, depth, mode, big, layers, data: Arc::new(bytes) })
}

#[cfg(test)]
mod tests;
