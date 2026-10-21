//! An OpenType font read straight from its file: which glyph a character is, how far it moves
//! the pen, and its outline (the `glyf` table's contours, curves made of quadratic pieces, or
//! the `CFF `/`CFF2` table's charstrings, cubic curves).
//!
//! A variable font is read at any point of its design space (`Font::at`, e.g. weight 600):
//! the points of each outline move by the deltas `gvar` gives for that point (and the points it
//! leaves out are moved in proportion to their neighbours), and the advance widths by what
//! `HVAR` gives.
//!
//! Colour glyphs are read three ways: pictures (`sbix`, and `CBDT`'s PNGs), layers of outlines
//! each in one colour (`COLR` version 0), and graphs of paints — gradients, transforms and ways of
//! combining — over outlines (`COLR` version 1, [`colr`]).
//!
//! [`shape`] turns text into placed glyphs with the font's own OpenType rules (`GSUB`, `GPOS`),
//! and [`system`] finds the fonts installed on the computer.

mod cff;
pub mod colr;
pub mod names;
pub mod program;
mod type1;
pub mod shape;
pub mod system;

use veymelo_path::Path;
use std::sync::Arc;

/// What an outline is drawn into: a [`Path`] (its curves flattened as they come), or anything
/// that keeps the curves themselves.
pub trait Pen {
    fn move_to(&mut self, x: f64, y: f64);
    fn line_to(&mut self, x: f64, y: f64);
    /// A quadratic curve bent toward (`cx`, `cy`), to (`x`, `y`).
    fn quad_to(&mut self, cx: f64, cy: f64, x: f64, y: f64);
    /// A cubic curve with control points `a` and `b`, to (`x`, `y`).
    fn cubic_to(&mut self, ax: f64, ay: f64, bx: f64, by: f64, x: f64, y: f64);
    fn close(&mut self);
}

impl Pen for Path {
    fn move_to(&mut self, x: f64, y: f64) {
        Path::move_to(self, x, y);
    }

    fn line_to(&mut self, x: f64, y: f64) {
        Path::line_to(self, x, y);
    }

    fn quad_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) {
        Path::quad_to(self, cx, cy, x, y);
    }

    fn cubic_to(&mut self, ax: f64, ay: f64, bx: f64, by: f64, x: f64, y: f64) {
        Path::cubic_to(self, ax, ay, bx, by, x, y);
    }

    fn close(&mut self) {
        Path::close(self);
    }
}

/// One glyph of a composite: its glyph, its offset, whether the offset is x/y (else points
/// to match), and its 2×2 matrix.
type Part = (u16, (f64, f64), bool, [f64; 4]);

#[derive(Clone)]
pub struct Font {
    data: Arc<Vec<u8>>,
    /// Font units in one em.
    pub units: f64,
    /// Above and below the baseline (font units; `descent` is negative).
    pub ascent: f64,
    pub descent: f64,
    cmap: usize,
    cmap_format: u16,
    loca: usize,
    loca_long: bool,
    glyf: usize,
    hmtx: usize,
    metrics: usize,
    /// The design axes (tag, least, usual, most), and the tables that vary the font (0: none).
    axes: Vec<([u8; 4], f64, f64, f64)>,
    avar: usize,
    gvar: usize,
    hvar: usize,
    /// Where this instance is on each axis, from -1 to 1 (0: the usual design).
    coords: Vec<f64>,
    /// The same as asked, in the axes' own units (only the axes asked for).
    user: Vec<([u8; 4], f64)>,
    /// Which font of its file it is (a collection holds several).
    pub(crate) face: usize,
    /// Where the layout tables are (0: the font has none).
    pub(crate) gdef: usize,
    pub(crate) gsub: usize,
    pub(crate) gpos: usize,
    pub(crate) kern: usize,
    /// Apple's own layout tables (fonts made for the Mac): glyphs swapped and joined (`morx`),
    /// kerning and marks (`kerx`, with the anchor points of `ankr`), spacing by size (`trak`).
    pub(crate) morx: usize,
    pub(crate) kerx: usize,
    pub(crate) ankr: usize,
    pub(crate) trak: usize,
    /// Colour glyphs: pictures (`sbix`, `CBDT` found through `CBLC`), and layers of outlines
    /// or graphs of paints (`COLR`) coloured from a
    /// palette (`CPAL`).
    sbix: usize,
    cbdt: usize,
    cblc: usize,
    colr: usize,
    cpal: usize,
    /// PostScript outlines (`CFF ` or `CFF2`), read, when it has no `glyf`.
    cff: Option<Arc<cff::Cff>>,
    os2: usize,
    post: usize,
    vhea: usize,
    vmtx: usize,
    name: usize,
    /// How many glyphs it has.
    pub glyphs: u16,
}

/// A glyph's colour picture in the font (`sbix`): its file's bytes, where its bottom left is
/// from the glyph's origin (in its own pixels, y up), its pixels to an em, and its kind
/// (`png `, `jpg `, `tiff`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphPicture<'a> {
    pub bytes: &'a [u8],
    pub origin: (f64, f64),
    pub ppem: f64,
    pub kind: [u8; 4],
}

pub(crate) fn u16_at(d: &[u8], at: usize) -> u16 {
    d.get(at..at + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
}

pub(crate) fn i16_at(d: &[u8], at: usize) -> i16 {
    u16_at(d, at) as i16
}

pub(crate) fn u32_at(d: &[u8], at: usize) -> u32 {
    d.get(at..at + 4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

pub(crate) fn byte_at(d: &[u8], at: usize) -> u8 {
    d.get(at).copied().unwrap_or(0)
}

/// A 2.14 fixed-point number.
pub(crate) fn f2_at(d: &[u8], at: usize) -> f64 {
    i16_at(d, at) as f64 / 16384.0
}

/// A 16.16 fixed-point number.
pub(crate) fn fixed_at(d: &[u8], at: usize) -> f64 {
    u32_at(d, at) as i32 as f64 / 65536.0
}

/// Strikes of pictures (their pixels to an em, and where they are), the nearest as big as `ppem`
/// or bigger first, then the bigger ones, then the smaller from the biggest down.
fn nearest_first<T: Copy>(mut order: Vec<(u16, T)>, ppem: f64) -> Vec<(u16, T)> {
    order.sort_by_key(|(p, _)| *p);
    let start = order.iter().position(|(p, _)| *p as f64 >= ppem).unwrap_or(order.len().saturating_sub(1));
    order[start..].iter().chain(order[..start].iter().rev()).copied().collect()
}

/// How many fonts a file holds (a collection, `.ttc`, holds several).
pub fn faces_in(data: &[u8]) -> usize {
    if data.starts_with(b"ttcf") { u32_at(data, 8) as usize } else { 1 }
}

impl Font {
    /// Reads the font file at `path` (the first font of a collection), at its usual design.
    pub fn open(path: &str) -> Result<Font, String> {
        Font::open_face(path, 0)
    }

    /// Reads font `index` of the file at `path`, at its usual design.
    pub fn open_face(path: &str, index: usize) -> Result<Font, String> {
        let data = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
        Font::from_data(data, index).map_err(|e| format!("{path}: {e}"))
    }

    /// The font `index` of a font file's bytes.
    pub fn from_data(data: Vec<u8>, index: usize) -> Result<Font, String> {
        Font::read(data, index, true)
    }

    /// A font as a document embeds it (a PDF's TrueType or OpenType font program): it may have
    /// no Unicode map, its glyphs then found by [`Font::glyph_in`] or [`Font::glyph_named`].
    pub fn embedded(data: Vec<u8>) -> Result<Font, String> {
        Font::read(data, 0, false)
    }

    fn read(data: Vec<u8>, index: usize, unicode: bool) -> Result<Font, String> {
        if index >= faces_in(&data) {
            return Err(format!("there is no font {index} in it"));
        }
        let start = if data.starts_with(b"ttcf") { u32_at(&data, 12 + 4 * index) as usize } else { 0 };
        let count = u16_at(&data, start + 4) as usize;
        let table = |tag: &[u8; 4]| -> Option<usize> {
            (0..count).map(|i| start + 12 + 16 * i).find(|&r| data.get(r..r + 4) == Some(tag)).map(|r| u32_at(&data, r + 8) as usize)
        };
        let need = |tag: &[u8; 4]| table(tag).ok_or_else(|| format!("it has no {} table", String::from_utf8_lossy(tag)));
        let (head, hhea, cmap, hmtx) = (need(b"head")?, need(b"hhea")?, need(b"cmap")?, need(b"hmtx")?);
        let (loca, glyf) = (table(b"loca").unwrap_or(0), table(b"glyf").unwrap_or(0));
        let glyph_count = table(b"maxp").map_or(u16::MAX, |m| u16_at(&data, m + 4));
        let cff = if glyf == 0 {
            match (table(b"CFF "), table(b"CFF2")) {
                (Some(at), _) => cff::Cff::read(&data, at, false, glyph_count as usize).map(Arc::new),
                (None, Some(at)) => cff::Cff::read(&data, at, true, glyph_count as usize).map(Arc::new),
                _ => None,
            }
        } else {
            None
        };
        // a font of pictures only (emoji) may have no outlines
        if glyf == 0 && cff.is_none() && table(b"sbix").is_none() && table(b"CBDT").is_none() {
            return Err("it has no outlines (no glyf, CFF or CFF2 table)".into());
        }
        // the Unicode map: the full one (format 12) when there is one, else the basic one (4)
        let maps = u16_at(&data, cmap + 2) as usize;
        let mut pick = None;
        for i in 0..maps {
            let r = cmap + 4 + 8 * i;
            let (platform, encoding) = (u16_at(&data, r), u16_at(&data, r + 2));
            let at = cmap + u32_at(&data, r + 4) as usize;
            let format = u16_at(&data, at);
            let unicode = platform == 0 || (platform == 3 && (encoding == 1 || encoding == 10));
            // the full map (12) over the basic one (4); a map of ranges to one glyph each (13)
            // only when there is nothing else
            let better = matches!((format, pick.map(|p: (usize, u16)| p.1)), (12, _) | (4, None | Some(13)) | (13, None));
            if unicode && better {
                pick = Some((at, format));
            }
        }
        let (cmap, cmap_format) = match pick {
            Some(p) => p,
            None if !unicode => (0, 0),
            None => return Err("it has no Unicode character map".into()),
        };
        // the design axes of a variable font
        let mut axes = vec![];
        if let Some(fvar) = table(b"fvar") {
            let first = fvar + u16_at(&data, fvar + 4) as usize;
            let (n, size) = (u16_at(&data, fvar + 8) as usize, u16_at(&data, fvar + 10) as usize);
            for i in 0..n {
                let r = first + i * size;
                let tag = [byte_at(&data, r), byte_at(&data, r + 1), byte_at(&data, r + 2), byte_at(&data, r + 3)];
                axes.push((tag, fixed_at(&data, r + 4), fixed_at(&data, r + 8), fixed_at(&data, r + 12)));
            }
        }
        let coords = vec![0.0; axes.len()];
        Ok(Font {
            units: u16_at(&data, head + 18) as f64,
            ascent: i16_at(&data, hhea + 4) as f64,
            descent: i16_at(&data, hhea + 6) as f64,
            loca_long: i16_at(&data, head + 50) == 1,
            metrics: u16_at(&data, hhea + 34) as usize,
            cmap,
            cmap_format,
            loca,
            glyf,
            hmtx,
            avar: table(b"avar").unwrap_or(0),
            gvar: table(b"gvar").unwrap_or(0),
            hvar: table(b"HVAR").unwrap_or(0),
            axes,
            coords,
            user: Vec::new(),
            face: index,
            gdef: table(b"GDEF").unwrap_or(0),
            gsub: table(b"GSUB").unwrap_or(0),
            gpos: table(b"GPOS").unwrap_or(0),
            kern: table(b"kern").unwrap_or(0),
            morx: table(b"morx").unwrap_or(0),
            kerx: table(b"kerx").unwrap_or(0),
            ankr: table(b"ankr").unwrap_or(0),
            trak: table(b"trak").unwrap_or(0),
            sbix: table(b"sbix").unwrap_or(0),
            cbdt: table(b"CBDT").unwrap_or(0),
            cblc: if table(b"CBDT").is_some() { table(b"CBLC").unwrap_or(0) } else { 0 },
            colr: table(b"COLR").unwrap_or(0),
            cpal: table(b"CPAL").unwrap_or(0),
            cff,
            os2: table(b"OS/2").unwrap_or(0),
            post: table(b"post").unwrap_or(0),
            vhea: table(b"vhea").unwrap_or(0),
            vmtx: table(b"vmtx").unwrap_or(0),
            name: table(b"name").unwrap_or(0),
            glyphs: glyph_count,
            data: Arc::new(data),
        })
    }

    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    /// The file's bytes, shared.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn shared_data(&self) -> Arc<Vec<u8>> {
        self.data.clone()
    }

    /// Where this instance was asked to be on its axes, in their own units.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn user_coords(&self) -> &[([u8; 4], f64)] {
        &self.user
    }

    /// What tells this font's file from others' (for keeping what is made from it).
    pub fn id(&self) -> usize {
        Arc::as_ptr(&self.data) as usize
    }

    /// Whether some of its glyphs are in colour (pictures or coloured layers).
    pub fn has_color(&self) -> bool {
        self.sbix != 0 || self.cblc != 0 || self.colr != 0
    }

    /// What tells this instance of the font from others (its file and where it is on its axes).
    pub fn instance_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        (self.id(), self.face).hash(&mut h);
        for c in &self.coords {
            c.to_bits().hash(&mut h);
        }
        h.finish()
    }

    /// Glyph `g`'s colour picture (`sbix`, else `CBDT`) from the strike nearest `ppem` pixels an
    /// em (the smallest as big or bigger, else the biggest): its file's bytes (PNG, JPEG or
    /// TIFF), where the picture's bottom left is from the glyph's origin (its own pixels, y up),
    /// and its pixels to an em. None when it has no picture.
    pub fn picture(&self, g: u16, ppem: f64) -> Option<GlyphPicture<'_>> {
        let d = self.data();
        let t = self.sbix;
        if t == 0 {
            return self.bitmap(g, ppem);
        }
        let strikes = u32_at(d, t + 4) as usize;
        let order: Vec<(u16, usize)> = (0..strikes).map(|i| {
            let at = t + u32_at(d, t + 8 + 4 * i) as usize;
            (u16_at(d, at), at)
        }).collect();
        for (size, at) in nearest_first(order, ppem) {
            let mut glyph = g;
            for _ in 0..4 {
                let off = |k: u16| u32_at(d, at + 4 + 4 * k as usize) as usize;
                let (from, to) = (off(glyph), off(glyph.saturating_add(1)));
                if to <= from + 8 || glyph >= self.glyphs {
                    break;
                }
                let rec = at + from;
                let kind = d.get(rec + 4..rec + 8)?;
                let bytes = d.get(rec + 8..at + to)?;
                if kind == b"dupe" {
                    glyph = u16_at(bytes, 0);
                    continue;
                }
                return Some(GlyphPicture { bytes, origin: (i16_at(d, rec) as f64, i16_at(d, rec + 2) as f64), ppem: size as f64, kind: [kind[0], kind[1], kind[2], kind[3]] });
            }
        }
        None
    }

    /// Glyph `g`'s PNG picture in `CBDT`, found through `CBLC`'s strikes (each a size and
    /// ranges of glyphs, their pictures one after another or listed), nearest `ppem` first.
    fn bitmap(&self, g: u16, ppem: f64) -> Option<GlyphPicture<'_>> {
        let (d, t, data) = (self.data(), self.cblc, self.cbdt);
        if t == 0 {
            return None;
        }
        let strikes = (u32_at(d, t + 4) as usize).min(256);
        let order: Vec<(u16, usize)> = (0..strikes).map(|i| t + 8 + 48 * i).map(|rec| (byte_at(d, rec + 45) as u16, rec)).collect();
        for (size, rec) in nearest_first(order, ppem) {
            if g < u16_at(d, rec + 40) || g > u16_at(d, rec + 42) {
                continue;
            }
            let array = t + u32_at(d, rec) as usize;
            for j in 0..(u32_at(d, rec + 8) as usize).min(65536) {
                let e = array + 8 * j;
                let first = u16_at(d, e);
                if g < first || g > u16_at(d, e + 2) {
                    continue;
                }
                let sub = array + u32_at(d, e + 4) as usize;
                let (index_format, image_format, image_at) = (u16_at(d, sub), u16_at(d, sub + 2), data + u32_at(d, sub + 4) as usize);
                let k = (g - first) as usize;
                // where its picture is, how long, and the metrics the index gives (8 bytes)
                let (at, len, metrics) = match index_format {
                    1 => {
                        let (a, b) = (u32_at(d, sub + 8 + 4 * k) as usize, u32_at(d, sub + 12 + 4 * k) as usize);
                        (image_at + a, b.saturating_sub(a), None)
                    }
                    2 => {
                        let size = u32_at(d, sub + 8) as usize;
                        (image_at + size * k, size, Some(sub + 12))
                    }
                    3 => {
                        let (a, b) = (u16_at(d, sub + 8 + 2 * k) as usize, u16_at(d, sub + 10 + 2 * k) as usize);
                        (image_at + a, b.saturating_sub(a), None)
                    }
                    4 => {
                        let n = u32_at(d, sub + 8) as usize;
                        let Some(i) = (0..n).find(|&i| u16_at(d, sub + 12 + 4 * i) == g) else { continue };
                        let (a, b) = (u16_at(d, sub + 14 + 4 * i) as usize, u16_at(d, sub + 18 + 4 * i) as usize);
                        (image_at + a, b.saturating_sub(a), None)
                    }
                    5 => {
                        let (size, n) = (u32_at(d, sub + 8) as usize, u32_at(d, sub + 20) as usize);
                        let Some(i) = (0..n).find(|&i| u16_at(d, sub + 24 + 2 * i) == g) else { continue };
                        (image_at + size * i, size, Some(sub + 12))
                    }
                    _ => continue,
                };
                if len == 0 {
                    continue;
                }
                // small metrics (height, width, x and y bearings) or big ones, before the PNG or
                // in the index; the bearings are from the origin to the top left, y up
                let (m, png) = match image_format {
                    17 => (at, at + 9),
                    18 => (at, at + 12),
                    19 => match metrics {
                        Some(m) => (m, at + 4),
                        None => continue,
                    },
                    _ => continue,
                };
                let bytes = d.get(png..png + u32_at(d, png - 4) as usize)?;
                let (height, bx, by) = (byte_at(d, m) as f64, byte_at(d, m + 2) as i8 as f64, byte_at(d, m + 3) as i8 as f64);
                return Some(GlyphPicture { bytes, origin: (bx, by - height), ppem: size as f64, kind: *b"png " });
            }
        }
        None
    }

    /// Colour `index` of the font's first palette (`CPAL`), straight sRGB RGBA; none when it has
    /// no such colour.
    pub(crate) fn palette(&self, index: u16) -> Option<[f32; 4]> {
        let (d, c) = (self.data(), self.cpal);
        if c == 0 || index == 0xFFFF {
            return None;
        }
        let (entries, records_at, first_record) = (u16_at(d, c + 2), c + u32_at(d, c + 8) as usize, u16_at(d, c + 12) as usize);
        if index >= entries {
            return None;
        }
        let r = records_at + 4 * (first_record + index as usize);
        Some([byte_at(d, r + 2), byte_at(d, r + 1), byte_at(d, r), byte_at(d, r + 3)].map(|v| v as f32 / 255.0))
    }

    /// Glyph `g`'s coloured layers (`COLR` version 0), bottom first: each a glyph and its colour
    /// (straight sRGB RGBA from palette 0; none: the text's own colour). None when it has none.
    pub fn color_layers(&self, g: u16) -> Option<Vec<(u16, Option<[f32; 4]>)>> {
        let d = self.data();
        let t = self.colr;
        if t == 0 {
            return None;
        }
        let (bases, base_at, layer_at) = (u16_at(d, t + 2) as usize, t + u32_at(d, t + 4) as usize, t + u32_at(d, t + 8) as usize);
        // the base records are in glyph order
        let (mut lo, mut hi) = (0usize, bases);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let id = u16_at(d, base_at + 6 * mid);
            if id == g {
                let (first, n) = (u16_at(d, base_at + 6 * mid + 2) as usize, u16_at(d, base_at + 6 * mid + 4) as usize);
                return Some((0..n).map(|k| {
                    let r = layer_at + 4 * (first + k);
                    (u16_at(d, r), self.palette(u16_at(d, r + 2)))
                }).collect());
            }
            if id < g {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        None
    }

    /// The name with `id` from its `name` table (1 family, 2 style, 4 full name, 6 PostScript
    /// name, 16 and 17 the family and style it belongs to when they differ), English first.
    pub fn name(&self, id: u16) -> Option<String> {
        let d = &self.data[..];
        if self.name == 0 {
            return None;
        }
        let (count, strings) = (u16_at(d, self.name + 2) as usize, self.name + u16_at(d, self.name + 4) as usize);
        let mut best: Option<(u32, String)> = None;
        for i in 0..count {
            let r = self.name + 6 + 12 * i;
            let (platform, encoding, lang, nid) = (u16_at(d, r), u16_at(d, r + 2), u16_at(d, r + 4), u16_at(d, r + 6));
            if nid != id {
                continue;
            }
            let (len, off) = (u16_at(d, r + 8) as usize, u16_at(d, r + 10) as usize);
            let Some(bytes) = d.get(strings + off..strings + off + len) else { continue };
            let (rank, text) = match (platform, encoding) {
                (3, 1) | (3, 10) | (0, _) => {
                    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                    (if platform == 3 && lang == 0x409 { 0 } else if platform == 0 { 1 } else { 2 }, String::from_utf16_lossy(&units))
                }
                // the Mac's own names, English when the language is 0 (some fonts have their
                // English names only here, and the others in other languages)
                (1, 0) => (if lang == 0 { 1 } else { 3 }, bytes.iter().map(|&b| b as char).collect()),
                _ => continue,
            };
            if best.as_ref().is_none_or(|(r, _)| rank < *r) {
                best = Some((rank, text));
            }
        }
        best.map(|(_, t)| t)
    }

    /// Where an underline goes (below the baseline, negative) and how thick it is, font units.
    pub fn underline(&self) -> (f64, f64) {
        let (pos, thick) = if self.post == 0 { (0, 0) } else { (i16_at(&self.data, self.post + 8), i16_at(&self.data, self.post + 10)) };
        (if pos == 0 { -0.1 * self.units } else { pos as f64 }, if thick <= 0 { 0.05 * self.units } else { thick as f64 })
    }

    /// Where a line through the letters goes (above the baseline) and how thick it is, font units.
    pub fn strikeout(&self) -> (f64, f64) {
        let (size, pos) = if self.os2 == 0 { (0, 0) } else { (i16_at(&self.data, self.os2 + 26), i16_at(&self.data, self.os2 + 28)) };
        (if pos <= 0 { 0.26 * self.units } else { pos as f64 }, if size <= 0 { 0.05 * self.units } else { size as f64 })
    }

    /// How far the glyph moves the pen down in vertical writing (font units): from `vmtx`, else
    /// the font's height.
    pub fn vertical_advance(&self, g: u16) -> f64 {
        if self.vmtx != 0 && self.vhea != 0 {
            let long = u16_at(&self.data, self.vhea + 34) as usize;
            if long > 0 {
                let i = (g as usize).min(long - 1);
                return u16_at(&self.data, self.vmtx + 4 * i) as f64;
            }
        }
        self.ascent - self.descent
    }

    /// The glyph's ink, `[left, bottom, right, top]` in font units (none for a blank glyph).
    pub fn bounds(&self, g: u16) -> Option<[f64; 4]> {
        self.bounds_placed(g, &|x, y| (x, y))
    }

    /// The ink of the glyph's outline put through `place`, `[least x, least y, most x, most y]`.
    pub fn bounds_placed(&self, g: u16, place: &dyn Fn(f64, f64) -> (f64, f64)) -> Option<[f64; 4]> {
        struct B {
            r: Option<[f64; 4]>,
            at: (f64, f64),
        }
        impl B {
            fn add(&mut self, x: f64, y: f64) {
                let r = self.r.get_or_insert([x, y, x, y]);
                *r = [r[0].min(x), r[1].min(y), r[2].max(x), r[3].max(y)];
            }
        }
        impl Pen for B {
            fn move_to(&mut self, x: f64, y: f64) {
                self.add(x, y);
                self.at = (x, y);
            }
            fn line_to(&mut self, x: f64, y: f64) {
                self.add(x, y);
                self.at = (x, y);
            }
            fn quad_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) {
                let (x0, y0) = self.at;
                for k in 1..=8 {
                    let t = k as f64 / 8.0;
                    let u = 1.0 - t;
                    self.add(u * u * x0 + 2.0 * u * t * cx + t * t * x, u * u * y0 + 2.0 * u * t * cy + t * t * y);
                }
                self.at = (x, y);
            }
            fn cubic_to(&mut self, ax: f64, ay: f64, bx: f64, by: f64, x: f64, y: f64) {
                let (x0, y0) = self.at;
                for k in 1..=12 {
                    let t = k as f64 / 12.0;
                    let u = 1.0 - t;
                    self.add(u * u * u * x0 + 3.0 * u * u * t * ax + 3.0 * u * t * t * bx + t * t * t * x, u * u * u * y0 + 3.0 * u * u * t * ay + 3.0 * u * t * t * by + t * t * t * y);
                }
                self.at = (x, y);
            }
            fn close(&mut self) {}
        }
        let mut b = B { r: None, at: (0.0, 0.0) };
        self.outline(g, &mut b, place);
        b.r
    }

    /// How heavy it is, 100 (thin) to 900 (black), 400 regular.
    pub fn weight(&self) -> u16 {
        if self.os2 == 0 { 400 } else { u16_at(&self.data, self.os2 + 4) }
    }

    /// Whether it slants (italic or oblique).
    pub fn italic(&self) -> bool {
        self.os2 != 0 && (u16_at(&self.data, self.os2 + 62) & 0x201) != 0
    }

    /// The height of a lowercase x and of capitals (font units), when the font says.
    pub fn x_height(&self) -> Option<f64> {
        (self.os2 != 0 && u16_at(&self.data, self.os2) >= 2).then(|| i16_at(&self.data, self.os2 + 86) as f64)
    }

    pub fn cap_height(&self) -> Option<f64> {
        (self.os2 != 0 && u16_at(&self.data, self.os2) >= 2).then(|| i16_at(&self.data, self.os2 + 88) as f64)
    }

    /// The design axes: tag, least, usual and most.
    pub fn axes(&self) -> Vec<(String, f64, f64, f64)> {
        self.axes.iter().map(|(t, a, b, c)| (String::from_utf8_lossy(t).into_owned(), *a, *b, *c)).collect()
    }

    /// The same font at another point of its design space, e.g. `[("wght", 600.0)]`. Axes not
    /// named stay at their usual value; values past an axis' ends are held at them.
    pub fn at(&self, settings: &[(&str, f64)]) -> Font {
        let mut f = self.clone();
        f.coords = self
            .axes
            .iter()
            .enumerate()
            .map(|(i, (tag, least, usual, most))| {
                let v = settings.iter().find(|(t, _)| t.as_bytes() == tag).map_or(*usual, |s| s.1).clamp(*least, *most);
                let n = if v < *usual { (v - usual) / (usual - least) } else if v > *usual { (v - usual) / (most - usual) } else { 0.0 };
                self.avar_map(i, n)
            })
            .collect();
        f.user = self.axes.iter().filter_map(|(tag, least, _, most)| settings.iter().find(|(t, _)| t.as_bytes() == tag).map(|s| (*tag, s.1.clamp(*least, *most)))).collect();
        f
    }

    /// Where this instance was asked to be on axis `tag` (its own units), if it was.
    pub fn setting(&self, tag: &str) -> Option<f64> {
        self.user.iter().find(|(t, _)| t == tag.as_bytes()).map(|(_, v)| *v)
    }

    /// The font's own remapping of a normalized axis value (its `avar` table), if it has one.
    fn avar_map(&self, axis: usize, v: f64) -> f64 {
        if self.avar == 0 {
            return v;
        }
        let d = &self.data;
        let mut r = self.avar + 8;
        for _ in 0..axis {
            r += 2 + 4 * u16_at(d, r) as usize;
        }
        let n = u16_at(d, r) as usize;
        let pair = |k: usize| (f2_at(d, r + 2 + 4 * k), f2_at(d, r + 4 + 4 * k));
        for k in 1..n {
            let ((f0, t0), (f1, t1)) = (pair(k - 1), pair(k));
            if v <= f1 {
                return if f1 > f0 { t0 + (v - f0) * (t1 - t0) / (f1 - f0) } else { t1 };
            }
        }
        v
    }

    pub(crate) fn varied(&self) -> bool {
        self.coords.iter().any(|&c| c != 0.0)
    }

    /// The glyph for a character (0, the missing glyph, when there is none).
    pub fn glyph(&self, c: char) -> u16 {
        if self.cmap == 0 {
            return 0;
        }
        self.mapped(self.cmap, self.cmap_format, c as u32)
    }

    /// The glyph the character map for `platform` and `encoding` (3 and 0: Windows symbols; 1
    /// and 0: Macintosh Roman) gives `code`; none when it has no such map or it gives none.
    pub fn glyph_in(&self, platform: u16, encoding: u16, code: u32) -> Option<u16> {
        let d = &self.data[..];
        let cmap = self.table(b"cmap")?;
        let maps = u16_at(d, cmap + 2) as usize;
        let at = (0..maps).map(|i| cmap + 4 + 8 * i).find(|&r| u16_at(d, r) == platform && u16_at(d, r + 2) == encoding).map(|r| cmap + u32_at(d, r + 4) as usize)?;
        let g = self.mapped(at, u16_at(d, at), code);
        (g != 0).then_some(g)
    }

    /// Where table `tag` is in the font's file.
    fn table(&self, tag: &[u8; 4]) -> Option<usize> {
        let d = &self.data[..];
        let start = if d.starts_with(b"ttcf") { u32_at(d, 12 + 4 * self.face) as usize } else { 0 };
        (0..u16_at(d, start + 4) as usize).map(|i| start + 12 + 16 * i).find(|&r| d.get(r..r + 4) == Some(tag)).map(|r| u32_at(d, r + 8) as usize)
    }

    /// The glyph named `name` in its `post` table (none when it names none so).
    pub fn glyph_named(&self, name: &str) -> Option<u16> {
        let d = &self.data[..];
        if self.post == 0 {
            return None;
        }
        match u32_at(d, self.post) {
            0x0001_0000 => (0..258.min(self.glyphs as usize)).find(|&i| names::mac_name(i) == Some(name)).map(|i| i as u16),
            0x0002_0000 => {
                let n = u16_at(d, self.post + 32) as usize;
                let indexes = self.post + 34;
                // the font's own names, one after another, each its length first
                let mut own = Vec::new();
                let mut p = indexes + 2 * n;
                while p < d.len() && own.len() < n {
                    let len = byte_at(d, p) as usize;
                    own.push(d.get(p + 1..p + 1 + len).unwrap_or(&[]));
                    p += 1 + len;
                }
                (0..n).find(|&g| {
                    let i = u16_at(d, indexes + 2 * g) as usize;
                    if i < 258 { names::mac_name(i) == Some(name) } else { own.get(i - 258).is_some_and(|b| *b == name.as_bytes()) }
                }).map(|g| g as u16)
            }
            _ => None,
        }
    }

    /// The glyph a character map subtable of `format` at `at` gives `c`.
    fn mapped(&self, at: usize, format: u16, c: u32) -> u16 {
        let d = &self.data[..];
        if format == 0 {
            return if c < 256 { byte_at(d, at + 6 + c as usize) as u16 } else { 0 };
        }
        if format == 6 {
            let (first, n) = (u16_at(d, at + 6) as u32, u16_at(d, at + 8) as u32);
            return if c >= first && c < first + n { u16_at(d, at + 10 + 2 * (c - first) as usize) } else { 0 };
        }
        if format == 12 || format == 13 {
            // groups in order of their characters: found by halving
            let groups = u32_at(d, at + 12) as usize;
            let (mut lo, mut hi) = (0, groups);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let r = at + 16 + 12 * mid;
                let (first, last, glyph) = (u32_at(d, r), u32_at(d, r + 4), u32_at(d, r + 8));
                if c < first {
                    hi = mid;
                } else if c > last {
                    lo = mid + 1;
                } else {
                    return if format == 12 { (glyph + c - first) as u16 } else { glyph as u16 };
                }
            }
            return 0;
        }
        if c > 0xFFFF || format != 4 {
            return 0;
        }
        let segs = u16_at(d, at + 6) as usize / 2;
        let ends = at + 14;
        let starts = ends + 2 * segs + 2;
        let deltas = starts + 2 * segs;
        let ranges = deltas + 2 * segs;
        for s in 0..segs {
            if c > u16_at(d, ends + 2 * s) as u32 {
                continue;
            }
            let first = u16_at(d, starts + 2 * s) as u32;
            if c < first {
                return 0;
            }
            let delta = u16_at(d, deltas + 2 * s);
            let range = u16_at(d, ranges + 2 * s) as usize;
            if range == 0 {
                return (c as u16).wrapping_add(delta);
            }
            let g = u16_at(d, ranges + 2 * s + range + 2 * (c - first) as usize);
            return if g == 0 { 0 } else { g.wrapping_add(delta) };
        }
        0
    }

    /// How far the glyph moves the pen (font units), at this instance.
    pub fn advance(&self, g: u16) -> f64 {
        let i = (g as usize).min(self.metrics.saturating_sub(1));
        u16_at(&self.data, self.hmtx + 4 * i) as f64 + self.advance_delta(g)
    }

    /// Where the glyph's data is in `glyf`.
    fn glyph_data(&self, g: u16) -> Option<usize> {
        if self.glyf == 0 {
            return None;
        }
        let g = g as usize;
        let d = &self.data;
        let (a, b) = if self.loca_long {
            (u32_at(d, self.loca + 4 * g) as usize, u32_at(d, self.loca + 4 * g + 4) as usize)
        } else {
            (2 * u16_at(d, self.loca + 2 * g) as usize, 2 * u16_at(d, self.loca + 2 * g + 2) as usize)
        };
        (b > a).then_some(self.glyf + a)
    }

    /// The glyph's outline, put through `place` (font units in, wherever they go out), drawn
    /// into `path`.
    pub fn outline(&self, g: u16, path: &mut dyn Pen, place: &dyn Fn(f64, f64) -> (f64, f64)) {
        if let Some(c) = &self.cff {
            c.outline(&self.data, g, path, place, &|list, region| self.region_scalar(list, region));
            return;
        }
        self.outline_depth(g, path, place, 0);
    }

    fn outline_depth(&self, g: u16, path: &mut dyn Pen, place: &dyn Fn(f64, f64) -> (f64, f64), depth: u32) {
        let Some(at) = self.glyph_data(g) else { return };
        if i16_at(&self.data, at) < 0 {
            if depth < 8 {
                self.composite(g, at + 10, path, place, depth);
            }
            return;
        }
        let (xs, ys, flags, ends) = self.points(g, at);
        let points = xs.len();
        let mut from = 0;
        for &end in &ends {
            let pts: Vec<((f64, f64), bool)> = (from..=end.min(points.saturating_sub(1))).map(|i| (place(xs[i], ys[i]), flags[i] & 1 != 0)).collect();
            contour(path, &pts);
            from = end + 1;
        }
    }

    /// Point `n` of a glyph's outline as the font numbers them (on the curve and off it), where
    /// the glyph is drawn from its own points (not made of others).
    pub(crate) fn contour_point(&self, g: u16, n: usize) -> Option<(f64, f64)> {
        let at = self.glyph_data(g)?;
        if i16_at(&self.data, at) < 0 {
            return None;
        }
        let (xs, ys, _, _) = self.points(g, at);
        Some((*xs.get(n)?, *ys.get(n)?))
    }

    /// The points of a glyph drawn from its own points (its data at `at`), as varied at this
    /// instance: across, up, their flags, and the last point of each contour.
    fn points(&self, g: u16, at: usize) -> (Vec<f64>, Vec<f64>, Vec<u8>, Vec<usize>) {
        let d = &self.data[..];
        let contours = i16_at(d, at).max(0) as usize;
        let ends: Vec<usize> = (0..contours).map(|i| u16_at(d, at + 10 + 2 * i) as usize).collect();
        let points = ends.last().map_or(0, |e| e + 1);
        let mut r = at + 10 + 2 * contours;
        r += 2 + u16_at(d, r) as usize; // the hinting instructions, not used
        // the flags, with their repeats
        let mut flags = Vec::with_capacity(points);
        while flags.len() < points {
            let f = byte_at(d, r);
            r += 1;
            flags.push(f);
            if f & 8 != 0 {
                let n = byte_at(d, r);
                r += 1;
                flags.extend(std::iter::repeat_n(f, n as usize));
            }
        }
        flags.truncate(points);
        let mut read = |short: u8, same: u8| -> Vec<f64> {
            let mut v = 0i32;
            flags
                .iter()
                .map(|&f| {
                    if f & short != 0 {
                        let b = byte_at(d, r) as i32;
                        r += 1;
                        v += if f & same != 0 { b } else { -b };
                    } else if f & same == 0 {
                        v += i16_at(d, r) as i32;
                        r += 2;
                    }
                    v as f64
                })
                .collect()
        };
        let mut xs = read(2, 16);
        let mut ys = read(4, 32);
        if self.varied() {
            let orig: Vec<(f64, f64)> = xs.iter().copied().zip(ys.iter().copied()).collect();
            if let Some(moves) = self.deltas(g, points + 4, &orig, &ends) {
                for i in 0..points {
                    xs[i] += moves[i].0;
                    ys[i] += moves[i].1;
                }
            }
        }
        (xs, ys, flags, ends)
    }

    /// A glyph made of other glyphs, each moved (and maybe scaled); in a variable font the
    /// moves vary too.
    fn composite(&self, g: u16, mut r: usize, path: &mut dyn Pen, place: &dyn Fn(f64, f64) -> (f64, f64), depth: u32) {
        let d = &self.data[..];
        let mut parts: Vec<Part> = vec![];
        loop {
            let flags = u16_at(d, r);
            let part = u16_at(d, r + 2);
            r += 4;
            let off = if flags & 1 != 0 {
                let v = (i16_at(d, r) as f64, i16_at(d, r + 2) as f64);
                r += 4;
                v
            } else {
                let v = (byte_at(d, r) as i8 as f64, byte_at(d, r + 1) as i8 as f64);
                r += 2;
                v
            };
            let mut m = [1.0, 0.0, 0.0, 1.0];
            if flags & 8 != 0 {
                m[0] = f2_at(d, r);
                m[3] = m[0];
                r += 2;
            } else if flags & 0x40 != 0 {
                (m[0], m[3]) = (f2_at(d, r), f2_at(d, r + 2));
                r += 4;
            } else if flags & 0x80 != 0 {
                m = [f2_at(d, r), f2_at(d, r + 2), f2_at(d, r + 4), f2_at(d, r + 6)];
                r += 8;
            }
            parts.push((part, off, flags & 2 != 0, m));
            if flags & 0x20 == 0 {
                break;
            }
        }
        let moves = if self.varied() { self.deltas(g, parts.len() + 4, &[], &[]) } else { None };
        for (k, (part, (mut dx, mut dy), xy, m)) in parts.into_iter().enumerate() {
            if let Some(mv) = &moves {
                dx += mv[k].0;
                dy += mv[k].1;
            }
            // point-matched parts (offsets that are not x/y values) are rare: placed unmoved
            let (dx, dy) = if xy { (dx, dy) } else { (0.0, 0.0) };
            let moved = |x: f64, y: f64| place(m[0] * x + m[2] * y + dx, m[1] * x + m[3] * y + dy);
            self.outline_depth(part, path, &moved, depth + 1);
        }
    }

    /// How far each of glyph `g`'s `total` points moves at this instance (its outline's points
    /// or its parts, then four phantom points), from `gvar`. `orig` and `ends` (the outline's
    /// points and where its contours end) let the points a variation leaves out follow their
    /// neighbours.
    fn deltas(&self, g: u16, total: usize, orig: &[(f64, f64)], ends: &[usize]) -> Option<Vec<(f64, f64)>> {
        if self.gvar == 0 {
            return None;
        }
        let (d, gv) = (&self.data[..], self.gvar);
        let axis_count = u16_at(d, gv + 4) as usize;
        let shared_tuples = gv + u32_at(d, gv + 8) as usize;
        let glyphs = u16_at(d, gv + 12) as usize;
        let long = u16_at(d, gv + 14) & 1 != 0;
        let array = gv + u32_at(d, gv + 16) as usize;
        let gi = g as usize;
        if gi >= glyphs {
            return None;
        }
        let (a, b) = if long {
            (u32_at(d, gv + 20 + 4 * gi) as usize, u32_at(d, gv + 24 + 4 * gi) as usize)
        } else {
            (2 * u16_at(d, gv + 20 + 2 * gi) as usize, 2 * u16_at(d, gv + 22 + 2 * gi) as usize)
        };
        if b <= a {
            return None;
        }
        let base = array + a;
        let head = u16_at(d, base);
        let count = (head & 0x0FFF) as usize;
        let mut data_at = base + u16_at(d, base + 2) as usize;
        let mut header = base + 4;
        let shared_points = if head & 0x8000 != 0 {
            let (p, next) = read_points(d, data_at);
            data_at = next;
            p
        } else {
            None
        };
        let tuple = |at: usize| -> Vec<f64> { (0..axis_count).map(|i| f2_at(d, at + 2 * i)).collect() };
        let mut out = vec![(0.0, 0.0); total];
        for _ in 0..count {
            let size = u16_at(d, header) as usize;
            let index = u16_at(d, header + 2);
            header += 4;
            let peak = if index & 0x8000 != 0 {
                let t = tuple(header);
                header += 2 * axis_count;
                t
            } else {
                tuple(shared_tuples + (index & 0x0FFF) as usize * axis_count * 2)
            };
            let region = if index & 0x4000 != 0 {
                let (s, e) = (tuple(header), tuple(header + 2 * axis_count));
                header += 4 * axis_count;
                Some((s, e))
            } else {
                None
            };
            let here = data_at;
            data_at += size;
            let scalar = tuple_scalar(&self.coords, &peak, region.as_ref());
            if scalar == 0.0 {
                continue;
            }
            let mut r = here;
            let points = if index & 0x2000 != 0 {
                let (p, next) = read_points(d, r);
                r = next;
                p
            } else {
                shared_points.clone()
            };
            let n = points.as_ref().map_or(total, |p| p.len());
            let (xs, next) = read_deltas(d, r, n);
            let (ys, _) = read_deltas(d, next, n);
            match points {
                None => {
                    for i in 0..total.min(n) {
                        out[i].0 += xs[i] * scalar;
                        out[i].1 += ys[i] * scalar;
                    }
                }
                Some(p) => {
                    let mut given = vec![None; total];
                    for (k, &i) in p.iter().enumerate() {
                        if i < total {
                            given[i] = Some((xs[k], ys[k]));
                        }
                    }
                    for (i, m) in infer(&given, orig, ends).into_iter().enumerate() {
                        out[i].0 += m.0 * scalar;
                        out[i].1 += m.1 * scalar;
                    }
                }
            }
        }
        Some(out)
    }

    /// How much wider or narrower the glyph's advance is at this instance (`HVAR`).
    fn advance_delta(&self, g: u16) -> f64 {
        if self.hvar == 0 || !self.varied() {
            return 0.0;
        }
        let (d, h) = (&self.data[..], self.hvar);
        let store = h + u32_at(d, h + 4) as usize;
        let map = u32_at(d, h + 8) as usize;
        let (outer, inner) = if map == 0 { (0, g as usize) } else { self.delta_set(h + map, g as usize) };
        self.item_delta(store, outer, inner)
    }

    /// Entry `i` of a delta-set index map at `at` (the last when there are fewer): which
    /// variation data and which row of it.
    pub(crate) fn delta_set(&self, at: usize, i: usize) -> (usize, usize) {
        let d = &self.data[..];
        let (format, entry) = (byte_at(d, at), byte_at(d, at + 1));
        let (count, first) = if format == 0 { (u16_at(d, at + 2) as usize, at + 4) } else { (u32_at(d, at + 2) as usize, at + 6) };
        let size = ((entry & 0x30) >> 4) as usize + 1;
        let inner_bits = (entry & 0x0F) as u32 + 1;
        let i = i.min(count.saturating_sub(1));
        let mut e = 0u32;
        for b in 0..size {
            e = (e << 8) | byte_at(d, first + i * size + b) as u32;
        }
        ((e >> inner_bits) as usize, (e & ((1 << inner_bits) - 1)) as usize)
    }

    /// One delta of an item variation store, at this instance.
    pub(crate) fn item_delta(&self, store: usize, outer: usize, inner: usize) -> f64 {
        let d = &self.data[..];
        let regions = store + u32_at(d, store + 2) as usize;
        if outer >= u16_at(d, store + 6) as usize {
            return 0.0;
        }
        let data = store + u32_at(d, store + 8 + 4 * outer) as usize;
        let items = u16_at(d, data) as usize;
        let words_raw = u16_at(d, data + 2);
        let long = words_raw & 0x8000 != 0;
        let words = (words_raw & 0x7FFF) as usize;
        let region_count = u16_at(d, data + 4) as usize;
        if inner >= items {
            return 0.0;
        }
        let row_size = if long { 4 * words + 2 * (region_count - words.min(region_count)) } else { 2 * words + (region_count - words.min(region_count)) };
        let mut r = data + 6 + 2 * region_count + inner * row_size;
        let mut sum = 0.0;
        for k in 0..region_count {
            let delta = match (k < words, long) {
                (true, true) => {
                    r += 4;
                    u32_at(d, r - 4) as i32 as f64
                }
                (true, false) | (false, true) => {
                    r += 2;
                    i16_at(d, r - 2) as f64
                }
                (false, false) => {
                    r += 1;
                    byte_at(d, r - 1) as i8 as f64
                }
            };
            sum += delta * self.region_scalar(regions, u16_at(d, data + 6 + 2 * k) as usize);
        }
        sum
    }

    /// How much a region of the design space counts at this instance.
    pub(crate) fn region_scalar(&self, list: usize, index: usize) -> f64 {
        let d = &self.data[..];
        let (axis_count, count) = (u16_at(d, list) as usize, u16_at(d, list + 2) as usize);
        if index >= count {
            return 0.0;
        }
        let rec = list + 4 + index * axis_count * 6;
        let mut s = 1.0;
        for a in 0..axis_count {
            let (start, peak, end) = (f2_at(d, rec + 6 * a), f2_at(d, rec + 6 * a + 2), f2_at(d, rec + 6 * a + 4));
            if start > peak || peak > end || (start < 0.0 && end > 0.0 && peak != 0.0) || peak == 0.0 {
                continue;
            }
            let v = self.coords.get(a).copied().unwrap_or(0.0);
            if v == peak {
                continue;
            }
            if v <= start || v >= end {
                return 0.0;
            }
            s *= if v < peak { (v - start) / (peak - start) } else { (end - v) / (end - peak) };
        }
        s
    }
}

/// How much a variation counts at `coords`: 1 at its peak, falling to 0 at the usual design
/// (or at the ends of its region).
fn tuple_scalar(coords: &[f64], peak: &[f64], region: Option<&(Vec<f64>, Vec<f64>)>) -> f64 {
    let mut s = 1.0;
    for (i, &p) in peak.iter().enumerate() {
        if p == 0.0 {
            continue;
        }
        let v = coords.get(i).copied().unwrap_or(0.0);
        if v == p {
            continue;
        }
        match region {
            Some((start, end)) => {
                let (a, b) = (start[i], end[i]);
                if v < a || v > b {
                    return 0.0;
                }
                s *= if v < p { (v - a) / (p - a) } else { (b - v) / (b - p) };
            }
            None => {
                if v == 0.0 || v < p.min(0.0) || v > p.max(0.0) {
                    return 0.0;
                }
                s *= v / p;
            }
        }
    }
    s
}

/// The point numbers a variation names (None: all of them), and where its data goes on.
fn read_points(d: &[u8], at: usize) -> (Option<Vec<usize>>, usize) {
    let first = byte_at(d, at);
    let mut r = at + 1;
    if first == 0 {
        return (None, r);
    }
    let count = if first & 0x80 != 0 {
        let c = (((first & 0x7F) as usize) << 8) | byte_at(d, r) as usize;
        r += 1;
        c
    } else {
        first as usize
    };
    let mut points = Vec::with_capacity(count);
    let mut last = 0usize;
    while points.len() < count && r < d.len() {
        let control = byte_at(d, r);
        r += 1;
        let words = control & 0x80 != 0;
        for _ in 0..(control & 0x7F) as usize + 1 {
            if points.len() >= count {
                break;
            }
            last += if words {
                r += 2;
                u16_at(d, r - 2) as usize
            } else {
                r += 1;
                byte_at(d, r - 1) as usize
            };
            points.push(last);
        }
    }
    (Some(points), r)
}

/// `count` packed deltas, and where the data goes on.
fn read_deltas(d: &[u8], at: usize, count: usize) -> (Vec<f64>, usize) {
    let mut out = Vec::with_capacity(count);
    let mut r = at;
    while out.len() < count && r < d.len() {
        let control = byte_at(d, r);
        r += 1;
        for _ in 0..(control & 0x3F) as usize + 1 {
            if out.len() >= count {
                break;
            }
            out.push(if control & 0x80 != 0 {
                0.0
            } else if control & 0x40 != 0 {
                r += 2;
                i16_at(d, r - 2) as f64
            } else {
                r += 1;
                byte_at(d, r - 1) as i8 as f64
            });
        }
    }
    out.resize(count, 0.0);
    (out, r)
}

/// The moves of every point, from the moves of the points given: on each contour, a point not
/// given moves like the given points on either side of it (in proportion to where it lies
/// between them, each axis apart); a contour with one given point moves with it; one with none
/// stays. Points past the contours (the phantom ones) keep what was given, or nothing.
fn infer(given: &[Option<(f64, f64)>], orig: &[(f64, f64)], ends: &[usize]) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = given.iter().map(|m| m.unwrap_or((0.0, 0.0))).collect();
    let between = |c: f64, c1: f64, c2: f64, d1: f64, d2: f64| -> f64 {
        if c1 == c2 {
            if d1 == d2 { d1 } else { 0.0 }
        } else if c <= c1.min(c2) {
            if c1 < c2 { d1 } else { d2 }
        } else if c >= c1.max(c2) {
            if c1 > c2 { d1 } else { d2 }
        } else {
            d1 + (c - c1) * (d2 - d1) / (c2 - c1)
        }
    };
    let mut start = 0;
    for &end in ends {
        if end >= orig.len() || end >= given.len() {
            break;
        }
        let known: Vec<usize> = (start..=end).filter(|&i| given[i].is_some()).collect();
        match known.len() {
            0 => {}
            1 => {
                let m = given[known[0]].unwrap_or((0.0, 0.0));
                for o in out.iter_mut().take(end + 1).skip(start) {
                    *o = m;
                }
            }
            _ => {
                let next = |i: usize| if i == end { start } else { i + 1 };
                for (k, &r1) in known.iter().enumerate() {
                    let r2 = known[(k + 1) % known.len()];
                    let (m1, m2) = (given[r1].unwrap_or_default(), given[r2].unwrap_or_default());
                    let mut i = next(r1);
                    while i != r2 {
                        out[i] = (between(orig[i].0, orig[r1].0, orig[r2].0, m1.0, m2.0), between(orig[i].1, orig[r1].1, orig[r2].1, m1.1, m2.1));
                        i = next(i);
                    }
                }
            }
        }
        start = end + 1;
    }
    out
}

/// One closed contour of on- and off-curve points into the path: two off-curve points in a row
/// have an on-curve point halfway between them.
fn contour(path: &mut dyn Pen, pts: &[((f64, f64), bool)]) {
    let n = pts.len();
    if n < 2 {
        return;
    }
    let mid = |a: (f64, f64), b: (f64, f64)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    // start on an on-curve point (or halfway between the first two off-curve ones)
    let first = pts.iter().position(|p| p.1);
    let (start, from) = match first {
        Some(i) => (pts[i].0, i),
        None => (mid(pts[0].0, pts[1].0), 0),
    };
    path.move_to(start.0, start.1);
    let mut ctrl: Option<(f64, f64)> = None;
    for k in 1..=n {
        let (p, on) = pts[(from + k) % n];
        match (on, ctrl) {
            (true, None) => path.line_to(p.0, p.1),
            (true, Some(c)) => {
                path.quad_to(c.0, c.1, p.0, p.1);
                ctrl = None;
            }
            (false, None) => ctrl = Some(p),
            (false, Some(c)) => {
                let m = mid(c, p);
                path.quad_to(c.0, c.1, m.0, m.1);
                ctrl = Some(p);
            }
        }
    }
    if let Some(c) = ctrl {
        path.quad_to(c.0, c.1, start.0, start.1);
    }
    path.close();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_left_out_moves_in_proportion_to_its_neighbours() {
        // a square contour; the corners at x 0 and x 100 move by 10 and 30, the point between
        // them (x 50) by 20, the far corners keep their own
        let orig = [(0.0, 0.0), (50.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
        let given = [Some((10.0, 0.0)), None, Some((30.0, 0.0)), None, None];
        let out = infer(&given, &orig, &[4]);
        assert_eq!(out[1], (20.0, 0.0));
        // x 100 lies at the edge: the move of the reference with x 100
        assert_eq!(out[3].0, 30.0);
        assert_eq!(out[4].0, 10.0);
    }

    #[test]
    fn a_variation_counts_in_proportion_to_how_far_toward_its_peak() {
        assert_eq!(tuple_scalar(&[0.5], &[1.0], None), 0.5);
        assert_eq!(tuple_scalar(&[-0.5], &[1.0], None), 0.0);
        assert_eq!(tuple_scalar(&[0.0], &[1.0], None), 0.0);
        assert_eq!(tuple_scalar(&[1.0], &[1.0], None), 1.0);
    }

    #[test]
    fn the_system_font_is_heavier_and_wider_at_a_heavier_weight() {
        let Ok(font) = Font::open("/System/Library/Fonts/SFNS.ttf") else { return };
        let (regular, bold) = (font.at(&[("wght", 400.0)]), font.at(&[("wght", 700.0)]));
        let o = font.glyph('o');
        assert!(bold.advance(o) > regular.advance(o) + 20.0, "{} {}", regular.advance(o), bold.advance(o));
        // the stroke of the "l" is thicker: its outline is wider
        let width = |f: &Font| {
            let mut p = Path::default();
            f.outline(f.glyph('l'), &mut p, &|x, y| (x, y));
            let (x0, _, x1, _) = p.extent().unwrap();
            x1 - x0
        };
        assert!(width(&bold) > width(&regular) * 1.2, "{} {}", width(&regular), width(&bold));
    }
}
