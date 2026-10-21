//! Photoshop documents (`.psd`, and `.psb` large documents), read by our own reader from Adobe's
//! published Photoshop File Formats Specification: the file header, colour mode data, image
//! resources (kept, the global light read from them), the layer and mask information (each
//! layer's record: its bounds, channels, blend mode, opacity, clipping, visibility, layer mask
//! with its feather and density, and name; the additional information it carries, kept by key:
//! its Unicode name, group dividers with their blend mode, fill opacity, and what reads the rest —
//! vector masks, layer effects, fill and adjustment layers' settings, type layers' text and
//! styles, smart objects' placing), the document's own additional information (the layers of 16
//! and 32-bit documents, the files smart objects hold), and each layer's channels in every
//! compression (raw, RLE, ZIP with or without prediction) at 8, 16 and 32 bits, in RGB,
//! grayscale, CMYK and Lab.
//!
//! Layers are kept as the file keeps them, bottom first; [`Psd::tree`] lays them out top first,
//! groups holding what is in them.

use std::sync::Arc;

pub mod adjust;
pub mod descriptor;
pub mod looks;
pub mod smart;
pub mod typeset;
pub mod vector;
pub mod write;

pub use descriptor::{Desc, Item};

/// The rectangle a layer covers: top, left, bottom, right (document pixels).
pub type Bounds = [i32; 4];

/// A layer's mask: the rectangle it covers, what it is outside it (0 hidden, 255 shown), whether
/// it is switched off, how soft its edge is made (a feather, pixels) and how much it hides
/// (its density, 0 to 1), and the channel that holds it (−2, or −3 where the layer also has a
/// vector mask, −2 then holding that drawn).
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub bounds: Bounds,
    pub default: u8,
    pub disabled: bool,
    pub feather: f64,
    pub density: f64,
    pub channel: i16,
    /// It was drawn from other data (a vector mask), not painted.
    pub rendered: bool,
}

/// A block of additional information: its key, and where its data is in the file.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub key: [u8; 4],
    at: usize,
    len: usize,
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
    /// Its vector mask's feather (pixels) and density (0 to 1).
    pub vector_feather: f64,
    pub vector_density: f64,
    /// The additional information it carries, by key.
    pub info: Vec<Info>,
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
    /// Image resources: each one's id and where its data is.
    pub resources: Vec<(u16, Info)>,
    /// The document's own additional information, by key.
    pub global: Vec<Info>,
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

/// A mask softened by its feather (`r` pixels, as a Gaussian of about half that spread): three
/// box blurs across and down, what is past its edges taken as `outside`.
fn soften(v: &mut [f32], w: usize, h: usize, r: f64, outside: f32) {
    let k = ((r / 2.0).round() as usize).clamp(1, 2048);
    let mut line = Vec::new();
    let mut pass = |v: &mut [f32], n: usize, step: usize, count: usize, lead: usize| {
        for i in 0..count {
            let start = i * lead;
            line.clear();
            line.extend((0..n).map(|j| v[start + j * step]));
            let at = |j: isize| if j < 0 || j >= n as isize { outside } else { line[j as usize] };
            let mut sum: f32 = (-(k as isize)..=k as isize).map(at).sum();
            let width = (2 * k + 1) as f32;
            for j in 0..n {
                v[start + j * step] = sum / width;
                sum += at(j as isize + k as isize + 1) - at(j as isize - k as isize);
            }
        }
    };
    for _ in 0..3 {
        pass(v, w, 1, h, w);
        pass(v, h, w, w, 1);
    }
}

impl Psd {
    /// Additional information block `key` of layer `i`.
    pub fn block(&self, i: usize, key: &[u8; 4]) -> Option<&[u8]> {
        let b = self.layers.get(i)?.info.iter().find(|b| &b.key == key)?;
        self.data.get(b.at..b.at + b.len)
    }

    /// The document's own additional information block `key`.
    pub fn global(&self, key: &[u8; 4]) -> Option<&[u8]> {
        let b = self.global.iter().find(|b| &b.key == key)?;
        self.data.get(b.at..b.at + b.len)
    }

    /// Image resource `id`.
    pub fn resource(&self, id: u16) -> Option<&[u8]> {
        let (_, b) = self.resources.iter().find(|(i, _)| *i == id)?;
        self.data.get(b.at..b.at + b.len)
    }

    /// The light layer effects share: its angle and altitude, degrees (Photoshop's 120 and 30
    /// when the document does not say).
    pub fn global_light(&self) -> (f64, f64) {
        let int = |id: u16| self.resource(id).filter(|b| b.len() >= 4).map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64);
        (int(1037).unwrap_or(120.0), int(1049).unwrap_or(30.0))
    }

    /// How much layer `i`'s mask lets through over the rectangle `w` × `h` from (`left`, `top`):
    /// outside the mask's own rectangle as its default says, softened and lessened as its
    /// feather and density say. None where it has no mask on (or only one drawn from its vector
    /// mask).
    pub fn mask_over(&self, i: usize, left: i32, top: i32, w: usize, h: usize) -> Option<Vec<f32>> {
        let l = self.layers.get(i)?;
        let drawn = |m: &&Mask| m.rendered && l.info.iter().any(|b| &b.key == b"vmsk" || &b.key == b"vsms");
        let m = l.mask.as_ref().filter(|m| !m.disabled && !drawn(m))?;
        let bytes = match self.depth {
            16 => 2,
            32 => 4,
            _ => 1,
        };
        let [mt, ml, mb, mr] = m.bounds;
        let (mw, mh) = ((mr - ml).max(0) as usize, (mb - mt).max(0) as usize);
        // the real user mask where it is kept apart from the vector mask drawn
        let c = l.channels.iter().find(|c| c.id == m.channel).or_else(|| l.channels.iter().find(|c| c.id == -2));
        let plane = c
            .filter(|_| mw > 0 && mh > 0 && mw * mh <= 1 << 28)
            .and_then(|c| self.data.get(c.at..c.at + c.len))
            .and_then(|d| samples(d, mw, mh, bytes, self.big).ok())
            .map(|s| {
                (0..mw * mh)
                    .map(|k| match bytes {
                        2 => u16::from_be_bytes([s[2 * k], s[2 * k + 1]]) as f32 / 65535.0,
                        4 => f32::from_be_bytes([s[4 * k], s[4 * k + 1], s[4 * k + 2], s[4 * k + 3]]),
                        _ => s[k] as f32 / 255.0,
                    })
                    .collect::<Vec<f32>>()
            });
        let outside = m.default as f32 / 255.0;
        let mut cover = vec![outside; w * h];
        for (y, row) in cover.chunks_mut(w.max(1)).enumerate() {
            for (x, k) in row.iter_mut().enumerate() {
                let (dx, dy) = (left + x as i32 - ml, top + y as i32 - mt);
                if let Some(p) = &plane
                    && dx >= 0
                    && dy >= 0
                    && (dx as usize) < mw
                    && (dy as usize) < mh
                {
                    *k = p[dy as usize * mw + dx as usize];
                }
            }
        }
        if m.feather > 0.0 {
            soften(&mut cover, w, h, m.feather, outside);
        }
        let density = m.density.clamp(0.0, 1.0) as f32;
        cover.iter_mut().for_each(|k| *k = 1.0 - density * (1.0 - *k));
        Some(cover)
    }

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

    /// Layer `i`'s pixels, its layer mask applied (when it is on; a mask drawn from its vector
    /// mask is left to that, read apart by [`Psd::vector_mask`]). None for a layer with no
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
        if let Some(cover) = self.mask_over(i, left, top, w, h) {
            for (p, k) in out.iter_mut().zip(&cover) {
                p[3] *= k;
            }
        }
        Some(Pixels { left, top, width: w, height: h, px: out })
    }
}

/// The layer mask's record (`r` at its start, after its length): its rectangle, default, flags,
/// its and the vector mask's feather and density, and (where the layer has both) the real user
/// mask's rectangle, default and flags.
fn mask_record(r: &mut Read, len: usize) -> Result<(Option<Mask>, f64, f64), String> {
    let end = r.at + len;
    let (mut vf, mut vd) = (0.0, 1.0);
    if len < 18 {
        r.at = end;
        return Ok((None, vf, vd));
    }
    let bounds = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
    let (default, flags) = (r.u8()?, r.u8()?);
    let mut m = Mask { bounds, default, disabled: flags & 2 != 0, feather: 0.0, density: 1.0, channel: -2, rendered: flags & 8 != 0 };
    // the masks' parameters, each there if its bit says so
    if flags & 16 != 0 && r.at < end {
        let which = r.u8()?;
        if which & 1 != 0 {
            m.density = r.u8()? as f64 / 255.0;
        }
        if which & 2 != 0 {
            m.feather = f64::from_bits(r.u64()?);
        }
        if which & 4 != 0 {
            vd = r.u8()? as f64 / 255.0;
        }
        if which & 8 != 0 {
            vf = f64::from_bits(r.u64()?);
        }
    }
    // the real user mask (where the layer has a vector mask too), where there is room for it
    if end >= r.at + 18 {
        let (rflags, rdefault) = (r.u8()?, r.u8()?);
        let rb = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
        m = Mask { bounds: rb, default: rdefault, disabled: rflags & 2 != 0, channel: -3, rendered: false, ..m };
    }
    r.at = end;
    Ok((Some(m), vf, vd))
}

/// Blocks of additional information from `r.at` up to `end`.
fn info_blocks(r: &mut Read, end: usize, big: bool, global: bool) -> Result<Vec<Info>, String> {
    let mut out = Vec::new();
    while r.at + 12 <= end {
        let sig = r.bytes(4)?;
        if sig != b"8BIM" && sig != b"8B64" {
            break;
        }
        let key: [u8; 4] = r.bytes(4)?.try_into().expect("four bytes");
        let len = r.len(big && WIDE_KEYS.contains(&&key))?;
        let at = r.at;
        if at.checked_add(len).is_none_or(|e| e > r.d.len()) {
            return Err("additional layer information is cut short".into());
        }
        out.push(Info { key, at, len });
        // a layer's blocks are padded to an even length, the document's to four bytes
        r.at = at + len;
        let pad = if global { (4 - len % 4) % 4 } else { len % 2 };
        if r.at + pad <= end {
            r.at += pad;
        }
    }
    Ok(out)
}

/// The layers of a layer info section (its count, records and channels) from `r.at`.
fn read_layers(r: &mut Read, big: bool) -> Result<Vec<Layer>, String> {
    let d = r.d;
    let count = (r.u16()? as i16).unsigned_abs() as usize;
    let mut layers = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let bounds = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
        let nch = r.u16()? as usize;
        let mut channels = Vec::with_capacity(nch.min(64));
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
        let (mask, vector_feather, vector_density) = mask_record(r, mask_len)?;
        // blending ranges
        let n = r.u32()? as usize;
        r.bytes(n)?;
        // the name, padded to four bytes with its length
        let len = r.u8()? as usize;
        let mut name = String::from_utf8_lossy(r.bytes(len)?).to_string();
        r.bytes((4 - (len + 1) % 4) % 4)?;
        let mut layer = Layer {
            name: String::new(),
            bounds,
            blend,
            opacity,
            fill: 255,
            clipped: clipping != 0,
            hidden: flags & 2 != 0,
            kind: Kind::Pixels,
            mask,
            channels,
            adjustment: false,
            text: false,
            smart: false,
            vector_feather,
            vector_density,
            info: Vec::new(),
        };
        layer.info = info_blocks(r, extra_end, big, false)?;
        for b in &layer.info {
            let block = &d[b.at..b.at + b.len];
            match &b.key {
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
    Ok(layers)
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
    // colour mode data
    let n = r.u32()? as usize;
    r.bytes(n)?;
    // image resources: each signed, its id, a padded name, its data padded to an even length
    let n = r.u32()? as usize;
    let end = r.at + n;
    let mut resources = Vec::new();
    while r.at + 12 <= end {
        if r.bytes(4)? != b"8BIM" {
            break;
        }
        let id = r.u16()?;
        let name = r.u8()? as usize;
        r.bytes(name + (name + 1) % 2)?;
        let len = r.u32()? as usize;
        let at = r.at;
        if at + len > end {
            break;
        }
        resources.push((id, Info { key: *b"8BIM", at, len }));
        r.at = at + len + len % 2;
    }
    r.at = end;
    let mut layers = Vec::new();
    let mut global = Vec::new();
    let info_len = r.len(big)?;
    let info_end = r.at.checked_add(info_len).filter(|&e| e <= d.len()).ok_or("the file is cut short")?;
    if info_len > 0 {
        let layers_len = r.len(big)?;
        let layers_end = r.at + layers_len;
        if layers_len > 0 {
            layers = read_layers(&mut r, big)?;
        }
        // the global layer mask, then the document's own blocks
        r.at = layers_end;
        if r.at + 4 <= info_end {
            let n = r.u32()? as usize;
            r.at = (r.at + n).min(info_end);
            global = info_blocks(&mut r, info_end, big, true)?;
        }
        // a 16 or 32-bit document keeps its layers in a block of its own
        if layers.is_empty()
            && let Some(b) = global.iter().find(|b| &b.key == b"Lr16" || &b.key == b"Lr32")
        {
            let mut lr = Read { d, at: b.at };
            layers = read_layers(&mut lr, big)?;
        }
    }
    Ok(Psd { width, height, depth, mode, big, layers, resources, global, data: Arc::new(bytes) })
}

#[cfg(test)]
mod tests;
