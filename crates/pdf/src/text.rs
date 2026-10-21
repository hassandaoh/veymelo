//! A PDF's fonts: which glyph each code shows and how far it moves, from the font's own program
//! (TrueType or OpenType, CFF, Type 1) when the file holds it, else from the system's font of its
//! name (each character found through its glyph name or the file's map to Unicode); the codes
//! of composite fonts read by their CMap (Identity, or the file's own) and their CIDs taken to
//! glyphs; Type 3 fonts, whose glyphs are content streams of their own.

use std::collections::HashMap;
use std::sync::Arc;

use veymelo_font::names::{Encoding, code_of, encoded, unicode_of};
use veymelo_font::program::Program;
use veymelo_font::{Font, Pen};

use crate::file::File;
use crate::object::{Dict, Lexer, Obj};

/// Where a font's glyphs come from.
pub enum Outlines {
    TrueType(Arc<Font>),
    Program(Arc<Program>),
    /// The system's font of its name (each character looked up in it, or another that has it).
    System(Option<Arc<Font>>),
    Type3 { procs: Dict, matrix: [f64; 6], resources: Option<Dict> },
}

/// How a composite font's codes are read: its code space (byte lengths and ranges) and CIDs.
struct CMap {
    spaces: Vec<(usize, u32, u32)>,
    ranges: Vec<(u32, u32, u32)>,
    singles: HashMap<u32, u32>,
    identity: bool,
}

impl CMap {
    fn identity() -> CMap {
        CMap { spaces: vec![(2, 0, 0xFFFF)], ranges: Vec::new(), singles: HashMap::new(), identity: true }
    }

    fn cid(&self, code: u32) -> u32 {
        if let Some(&c) = self.singles.get(&code) {
            return c;
        }
        if let Some(&(lo, _, cid)) = self.ranges.iter().find(|(lo, hi, _)| (*lo..=*hi).contains(&code)) {
            return cid + code - lo;
        }
        if self.identity { code } else { 0 }
    }
}

/// Hex strings `<…>` and numbers of a CMap's text, read as numbers (with their lengths).
fn cmap_tokens(data: &[u8]) -> Vec<Obj> {
    let mut l = Lexer::new(data, 0);
    std::iter::from_fn(|| l.object()).collect()
}

fn code_of_bytes(b: &[u8]) -> u32 {
    b.iter().take(4).fold(0u32, |v, &x| (v << 8) | x as u32)
}

/// A CMap stream: code spaces and CIDs (`usecmap Identity` or none of its own: identity).
fn read_cmap(data: &[u8]) -> CMap {
    let t = cmap_tokens(data);
    let mut m = CMap { spaces: Vec::new(), ranges: Vec::new(), singles: HashMap::new(), identity: false };
    let mut i = 0;
    while i < t.len() {
        match &t[i] {
            Obj::Word(w) if w == "begincodespacerange" => {
                i += 1;
                while i + 1 < t.len() && !matches!(&t[i], Obj::Word(w) if w == "endcodespacerange") {
                    if let (Obj::Str(a), Obj::Str(b)) = (&t[i], &t[i + 1]) {
                        m.spaces.push((a.len().max(1), code_of_bytes(a), code_of_bytes(b)));
                    }
                    i += 2;
                }
            }
            Obj::Word(w) if w == "begincidrange" => {
                i += 1;
                while i + 2 < t.len() && !matches!(&t[i], Obj::Word(w) if w == "endcidrange") {
                    if let (Obj::Str(a), Obj::Str(b), Some(c)) = (&t[i], &t[i + 1], t[i + 2].as_i64()) {
                        m.ranges.push((code_of_bytes(a), code_of_bytes(b), c as u32));
                    }
                    i += 3;
                }
            }
            Obj::Word(w) if w == "begincidchar" => {
                i += 1;
                while i + 1 < t.len() && !matches!(&t[i], Obj::Word(w) if w == "endcidchar") {
                    if let (Obj::Str(a), Some(c)) = (&t[i], t[i + 1].as_i64()) {
                        m.singles.insert(code_of_bytes(a), c as u32);
                    }
                    i += 2;
                }
            }
            Obj::Name(n) if n.starts_with("Identity") && matches!(t.get(i + 1), Some(Obj::Word(w)) if w == "usecmap") => m.identity = true,
            _ => {}
        }
        i += 1;
    }
    if m.spaces.is_empty() {
        m.spaces.push((2, 0, 0xFFFF));
    }
    if m.ranges.is_empty() && m.singles.is_empty() {
        m.identity = true;
    }
    m
}

/// A ToUnicode map: codes to the text they stand for.
fn read_to_unicode(data: &[u8]) -> HashMap<u32, String> {
    let t = cmap_tokens(data);
    let utf16 = |b: &[u8]| -> String {
        let units: Vec<u16> = b.chunks(2).map(|c| (c[0] as u16) << 8 | *c.get(1).unwrap_or(&0) as u16).collect();
        String::from_utf16_lossy(&units)
    };
    let mut out = HashMap::new();
    let mut i = 0;
    while i < t.len() {
        match &t[i] {
            Obj::Word(w) if w == "beginbfchar" => {
                i += 1;
                while i + 1 < t.len() && !matches!(&t[i], Obj::Word(w) if w == "endbfchar") {
                    if let (Obj::Str(a), Obj::Str(b)) = (&t[i], &t[i + 1]) {
                        out.insert(code_of_bytes(a), utf16(b));
                    }
                    i += 2;
                }
            }
            Obj::Word(w) if w == "beginbfrange" => {
                i += 1;
                while i + 2 < t.len() && !matches!(&t[i], Obj::Word(w) if w == "endbfrange") {
                    if let (Obj::Str(a), Obj::Str(b)) = (&t[i], &t[i + 1]) {
                        let (lo, hi) = (code_of_bytes(a), code_of_bytes(b));
                        match &t[i + 2] {
                            Obj::Str(d) => {
                                // the last unit counts up
                                let mut units: Vec<u16> = d.chunks(2).map(|c| (c[0] as u16) << 8 | *c.get(1).unwrap_or(&0) as u16).collect();
                                for code in lo..=hi.min(lo + 0xFFFF) {
                                    out.insert(code, String::from_utf16_lossy(&units));
                                    if let Some(last) = units.last_mut() {
                                        *last = last.wrapping_add(1);
                                    }
                                }
                            }
                            Obj::Array(a) => {
                                for (k, d) in a.iter().enumerate() {
                                    if let Obj::Str(d) = d {
                                        out.insert(lo + k as u32, utf16(d));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    i += 3;
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

pub struct PdfFont {
    pub outlines: Outlines,
    /// Simple fonts: each code's glyph and name.
    glyphs: Vec<Option<u16>>,
    names: Vec<Option<String>>,
    /// Composite fonts: the CMap and CID to glyph map.
    cmap: Option<CMap>,
    cid_to_gid: Option<Vec<u16>>,
    /// Each code's (simple) or CID's (composite) width, in thousandths of an em (Type 3: glyph
    /// units), and the width when none is given.
    widths: HashMap<u32, f64>,
    default_width: f64,
    pub vertical: bool,
    vertical_metrics: HashMap<u32, [f64; 3]>,
    default_vertical: [f64; 2],
    to_unicode: Option<HashMap<u32, String>>,
    /// The glyphs' units to text space (an em of 1).
    units: [f64; 6],
}

fn nums(o: Option<Obj>) -> Vec<f64> {
    o.and_then(|o| o.as_array().map(|a| a.iter().filter_map(Obj::as_f64).collect())).unwrap_or_default()
}

/// The base font's name without its subset tag (`ABCDEF+`).
fn base_name(n: &str) -> &str {
    match n.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) => rest,
        _ => n,
    }
}

/// A font the file does not hold: the system's of its name, or (noted) others that have its
/// characters.
fn system(name: &str, notes: &mut Vec<String>) -> Outlines {
    let f = system_font(name);
    if f.is_none() {
        notes.push(format!("the font {name}, neither in the file nor on this computer (its text drawn in fonts that have its characters)"));
    }
    Outlines::System(f)
}

/// The system font a PDF names (PostScript name first, then family and style guessed from it).
fn system_font(name: &str) -> Option<Arc<Font>> {
    let name = base_name(name);
    let standard = match name {
        "Times-Roman" | "Times" => Some(("Times", 400.0, false)),
        "Times-Bold" => Some(("Times", 700.0, false)),
        "Times-Italic" => Some(("Times", 400.0, true)),
        "Times-BoldItalic" => Some(("Times", 700.0, true)),
        "Courier" => Some(("Courier", 400.0, false)),
        "Courier-Bold" => Some(("Courier", 700.0, false)),
        "Courier-Oblique" => Some(("Courier", 400.0, true)),
        "Courier-BoldOblique" => Some(("Courier", 700.0, true)),
        "Symbol" => Some(("Symbol", 400.0, false)),
        "ZapfDingbats" => Some(("Zapf Dingbats", 400.0, false)),
        _ => None,
    };
    if let Some((f, w, i)) = standard
        && let Some(font) = veymelo_text::resolve(f, w, i)
    {
        return Some(font);
    }
    if let Some(f) = veymelo_text::resolve(name, 400.0, false) {
        return Some(f);
    }
    let (family, style) = name.split_once([',', '-']).unwrap_or((name, ""));
    let style = style.to_lowercase();
    let bold = style.contains("bold") || style.contains("black") || style.contains("heavy");
    let italic = style.contains("italic") || style.contains("oblique");
    let family = family.trim_end_matches("PSMT").trim_end_matches("MT").trim_end_matches("PS");
    // "TimesNewRoman" to "Times New Roman"
    let mut spaced = String::new();
    for (i, c) in family.char_indices() {
        if i > 0 && c.is_ascii_uppercase() && !family[..i].ends_with(' ') {
            spaced.push(' ');
        }
        spaced.push(c);
    }
    let weight = if bold { 700.0 } else { 400.0 };
    veymelo_text::resolve(family, weight, italic).or_else(|| veymelo_text::resolve(&spaced, weight, italic))
}

impl PdfFont {
    pub fn read(file: &File, dict: &Dict, notes: &mut Vec<String>) -> PdfFont {
        let get = |d: &Dict, k: &str| file.at(d, k);
        let subtype = get(dict, "Subtype").and_then(|s| s.as_name().map(str::to_string)).unwrap_or_default();
        let name = get(dict, "BaseFont").and_then(|n| n.as_name().map(|n| base_name(n).to_string())).unwrap_or_else(|| subtype.clone());
        let to_unicode = match get(dict, "ToUnicode") {
            Some(Obj::Stream(s)) => file.decoded(&s).ok().map(|d| read_to_unicode(&d.data)),
            _ => None,
        };
        if subtype == "Type0" {
            return PdfFont::composite(file, dict, name, to_unicode, notes);
        }
        let desc = get(dict, "FontDescriptor").and_then(|d| d.as_dict().cloned()).unwrap_or_default();
        let flags = get(&desc, "Flags").and_then(|f| f.as_i64()).unwrap_or(0);
        let symbolic = flags & 4 != 0 && flags & 32 == 0;
        // the program it holds
        let outlines = if subtype == "Type3" {
            let m = nums(get(dict, "FontMatrix"));
            Outlines::Type3 {
                procs: get(dict, "CharProcs").and_then(|p| p.as_dict().cloned()).unwrap_or_default(),
                matrix: <[f64; 6]>::try_from(m).unwrap_or([0.001, 0.0, 0.0, 0.001, 0.0, 0.0]),
                resources: get(dict, "Resources").and_then(|r| r.as_dict().cloned()),
            }
        } else {
            embedded(file, &desc, notes).unwrap_or_else(|| system(&name, notes))
        };
        // each code's name: the encoding (and its differences) over the font's own
        let enc = get(dict, "Encoding");
        let base_of = |n: &str| match n {
            "WinAnsiEncoding" => Some(Encoding::WinAnsi),
            "MacRomanEncoding" => Some(Encoding::MacRoman),
            "StandardEncoding" => Some(Encoding::Standard),
            _ => None,
        };
        let own_encoding = matches!(outlines, Outlines::Program(_) | Outlines::Type3 { .. }) || symbolic || matches!(name.as_str(), "Symbol" | "ZapfDingbats");
        let base = match &enc {
            Some(Obj::Name(n)) => base_of(n),
            Some(o) => o.as_dict().and_then(|d| d.get("BaseEncoding")).and_then(|b| b.as_name()).and_then(base_of),
            None => None,
        }
        .or(if own_encoding { None } else { Some(Encoding::Standard) });
        let mut names: Vec<Option<String>> = (0..256).map(|c| base.and_then(|e| encoded(e, c as u8)).map(str::to_string)).collect();
        if let Some(diffs) = enc.as_ref().and_then(|e| e.as_dict()).and_then(|d| d.get("Differences")).map(|d| file.resolve(d)) {
            let mut code = 0usize;
            for item in diffs.as_array().unwrap_or(&[]) {
                match item {
                    Obj::Int(c) => code = (*c).clamp(0, 255) as usize,
                    Obj::Name(n) => {
                        if code < 256 {
                            names[code] = Some(n.clone());
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
        }
        // each code's glyph
        let glyphs: Vec<Option<u16>> = (0..256u32)
            .map(|c| {
                let n = names[c as usize].as_deref();
                match &outlines {
                    Outlines::Program(p) => n.and_then(|n| p.glyph_named(n)).or_else(|| p.glyph_coded(c as u8)),
                    Outlines::TrueType(f) => true_type_glyph(f, c as u8, n, symbolic),
                    Outlines::System(Some(f)) => {
                        let ch = n.and_then(unicode_of).or_else(|| char::from_u32(c));
                        ch.map(|ch| f.glyph(ch)).filter(|&g| g != 0)
                    }
                    _ => None,
                }
            })
            .collect();
        let first = get(dict, "FirstChar").and_then(|f| f.as_i64()).unwrap_or(0);
        let mut widths = HashMap::new();
        for (k, w) in nums(get(dict, "Widths")).into_iter().enumerate() {
            widths.insert((first + k as i64).max(0) as u32, w);
        }
        let units = match &outlines {
            Outlines::Type3 { matrix, .. } => *matrix,
            _ => [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
        };
        let default_width = get(&desc, "MissingWidth").and_then(|w| w.as_f64()).unwrap_or(f64::NAN);
        PdfFont { outlines, glyphs, names, cmap: None, cid_to_gid: None, widths, default_width, vertical: false, vertical_metrics: HashMap::new(), default_vertical: [880.0, -1000.0], to_unicode, units }
    }

    fn composite(file: &File, dict: &Dict, name: String, to_unicode: Option<HashMap<u32, String>>, notes: &mut Vec<String>) -> PdfFont {
        let get = |d: &Dict, k: &str| file.at(d, k);
        let (cmap, vertical) = match get(dict, "Encoding") {
            Some(Obj::Name(n)) => {
                if !n.starts_with("Identity") {
                    notes.push(format!("the CMap {n} (read as Identity)"));
                }
                (CMap::identity(), n.ends_with("-V"))
            }
            Some(Obj::Stream(s)) => {
                let v = s.dict.get("WMode").and_then(Obj::as_i64) == Some(1);
                (file.decoded(&s).map(|d| read_cmap(&d.data)).unwrap_or_else(|_| CMap::identity()), v)
            }
            _ => (CMap::identity(), false),
        };
        let desc_font = get(dict, "DescendantFonts").and_then(|d| d.as_array().and_then(|a| a.first().cloned())).map(|d| file.resolve(&d)).and_then(|d| d.as_dict().cloned()).unwrap_or_default();
        let desc = get(&desc_font, "FontDescriptor").and_then(|d| d.as_dict().cloned()).unwrap_or_default();
        let outlines = embedded(file, &desc, notes).unwrap_or_else(|| system(&name, notes));
        let cid_to_gid = match get(&desc_font, "CIDToGIDMap") {
            Some(Obj::Stream(s)) => file.decoded(&s).ok().map(|d| d.data.chunks(2).map(|c| (c[0] as u16) << 8 | *c.get(1).unwrap_or(&0) as u16).collect()),
            _ => None,
        };
        // widths: c [w …] or c0 c1 w
        let mut widths = HashMap::new();
        let w = get(&desc_font, "W").and_then(|w| w.as_array().map(<[Obj]>::to_vec)).unwrap_or_default();
        let mut i = 0;
        while i < w.len() {
            let Some(c) = w[i].as_i64() else { break };
            match w.get(i + 1).map(|o| file.resolve(o)) {
                Some(Obj::Array(list)) => {
                    for (k, v) in list.iter().enumerate() {
                        if let Some(v) = v.as_f64() {
                            widths.insert((c + k as i64) as u32, v);
                        }
                    }
                    i += 2;
                }
                Some(o) => {
                    let (Some(hi), Some(v)) = (o.as_i64(), w.get(i + 2).and_then(Obj::as_f64)) else { break };
                    for cid in c..=hi.min(c + 0xFFFF) {
                        widths.insert(cid as u32, v);
                    }
                    i += 3;
                }
                None => break,
            }
        }
        let mut vertical_metrics = HashMap::new();
        let w2 = get(&desc_font, "W2").and_then(|w| w.as_array().map(<[Obj]>::to_vec)).unwrap_or_default();
        let mut i = 0;
        while i < w2.len() {
            let Some(c) = w2[i].as_i64() else { break };
            match w2.get(i + 1).map(|o| file.resolve(o)) {
                Some(Obj::Array(list)) => {
                    for (k, t) in list.chunks(3).enumerate() {
                        let v: Vec<f64> = t.iter().filter_map(Obj::as_f64).collect();
                        if v.len() == 3 {
                            vertical_metrics.insert((c + k as i64) as u32, [v[0], v[1], v[2]]);
                        }
                    }
                    i += 2;
                }
                Some(o) => {
                    let hi = o.as_i64().unwrap_or(c);
                    let v: Vec<f64> = w2[i + 2..(i + 5).min(w2.len())].iter().filter_map(Obj::as_f64).collect();
                    if v.len() == 3 {
                        for cid in c..=hi.min(c + 0xFFFF) {
                            vertical_metrics.insert(cid as u32, [v[0], v[1], v[2]]);
                        }
                    }
                    i += 5;
                }
                None => break,
            }
        }
        let dw2 = nums(get(&desc_font, "DW2"));
        PdfFont {
            outlines,
            glyphs: Vec::new(),
            names: Vec::new(),
            cmap: Some(cmap),
            cid_to_gid,
            widths,
            default_width: get(&desc_font, "DW").and_then(|w| w.as_f64()).unwrap_or(1000.0),
            vertical,
            vertical_metrics,
            default_vertical: [dw2.first().copied().unwrap_or(880.0), dw2.get(1).copied().unwrap_or(-1000.0)],
            to_unicode,
            units: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
        }
    }

    /// The codes in a string: each code and how many bytes it took.
    pub fn codes(&self, s: &[u8]) -> Vec<(u32, usize)> {
        let Some(cmap) = &self.cmap else { return s.iter().map(|&b| (b as u32, 1)).collect() };
        let mut out = Vec::new();
        let mut p = 0;
        while p < s.len() {
            let mut taken = None;
            for len in 1..=4 {
                if p + len > s.len() {
                    break;
                }
                let code = code_of_bytes(&s[p..p + len]);
                if cmap.spaces.iter().any(|&(n, lo, hi)| n == len && (lo..=hi).contains(&code)) {
                    taken = Some((code, len));
                    break;
                }
            }
            let (code, len) = taken.unwrap_or_else(|| {
                let n = cmap.spaces.first().map_or(1, |s| s.0).min(s.len() - p);
                (code_of_bytes(&s[p..p + n]), n)
            });
            out.push((code, len));
            p += len;
        }
        out
    }

    /// The text a code stands for (its map to Unicode, else its glyph name's character).
    pub fn text(&self, code: u32) -> String {
        if let Some(t) = self.to_unicode.as_ref().and_then(|m| m.get(&code)) {
            return t.clone();
        }
        if self.cmap.is_none() {
            if let Some(c) = self.names.get(code as usize).cloned().flatten().and_then(|n| unicode_of(&n)) {
                return c.to_string();
            }
            return char::from_u32(code).filter(|c| !c.is_control()).map(String::from).unwrap_or_default();
        }
        String::new()
    }

    /// How far a code moves the pen (an em of 1; Type 3: in text space by its matrix).
    pub fn width(&self, code: u32) -> f64 {
        let key = match &self.cmap {
            Some(c) => c.cid(code),
            None => code,
        };
        let w = self.widths.get(&key).copied().unwrap_or(f64::NAN);
        let w = if w.is_nan() {
            if self.default_width.is_nan() { self.own_width(code).unwrap_or(0.0) / self.units[0] } else { self.default_width }
        } else {
            w
        };
        w * self.units[0]
    }

    /// A vertical font's advance down (negative: down) and where its glyph's origin is from
    /// the horizontal one (an em of 1).
    pub fn vertical(&self, code: u32) -> (f64, [f64; 2]) {
        let cid = self.cmap.as_ref().map_or(code, |c| c.cid(code));
        match self.vertical_metrics.get(&cid) {
            Some(&[w1, vx, vy]) => (w1 / 1000.0, [vx / 1000.0, vy / 1000.0]),
            None => (self.default_vertical[1] / 1000.0, [self.width(code) / 2.0, self.default_vertical[0] / 1000.0]),
        }
    }

    /// The glyph's own advance when the file gives none (system fonts), in thousandths.
    fn own_width(&self, code: u32) -> Option<f64> {
        match &self.outlines {
            Outlines::System(Some(f)) | Outlines::TrueType(f) => {
                let g = self.glyph(code)?;
                Some(f.advance(g) / f.units * 1000.0)
            }
            _ => None,
        }
    }

    /// The glyph a code shows (none: a code it has no glyph for).
    pub fn glyph(&self, code: u32) -> Option<u16> {
        match &self.cmap {
            None => self.glyphs.get(code as usize).copied().flatten(),
            Some(c) => {
                let cid = c.cid(code);
                match &self.outlines {
                    Outlines::Program(p) => p.glyph_of_cid(cid as u16),
                    Outlines::TrueType(f) => Some(match &self.cid_to_gid {
                        Some(m) => *m.get(cid as usize)?,
                        None => cid as u16,
                    })
                    .filter(|&g| (g as usize) < f.glyphs as usize),
                    Outlines::System(Some(f)) => self.text(code).chars().next().map(|ch| f.glyph(ch)).filter(|&g| g != 0),
                    _ => None,
                }
            }
        }
    }

    /// The Type 3 glyph procedure a code names.
    pub fn type3_proc(&self, code: u32) -> Option<&str> {
        self.names.get(code as usize).and_then(|n| n.as_deref())
    }

    /// Glyph `g`'s outline in text space (an em of 1), through `place`, into `pen`.
    pub fn outline(&self, code: u32, pen: &mut dyn Pen, place: &dyn Fn(f64, f64) -> (f64, f64)) -> bool {
        match &self.outlines {
            Outlines::Program(p) => {
                let Some(g) = self.glyph(code) else { return false };
                let m = p.matrix();
                p.outline(g, pen, &|x, y| place(m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]));
                true
            }
            Outlines::TrueType(f) => {
                let Some(g) = self.glyph(code) else { return false };
                let k = 1.0 / f.units;
                f.outline(g, pen, &|x, y| place(x * k, y * k));
                true
            }
            Outlines::System(font) => {
                // the glyph, else the character in another font that has it
                if let (Some(f), Some(g)) = (font, self.glyph(code)) {
                    let k = 1.0 / f.units;
                    f.outline(g, pen, &|x, y| place(x * k, y * k));
                    return true;
                }
                let Some(ch) = self.text(code).chars().next() else { return false };
                let Some(face) = veymelo_font::system::fallback_for(ch) else { return false };
                let Ok(f) = Font::open_face(&face.path, face.index) else { return false };
                let g = f.glyph(ch);
                let k = 1.0 / f.units;
                f.outline(g, pen, &|x, y| place(x * k, y * k));
                true
            }
            Outlines::Type3 { .. } => false,
        }
    }
}

/// The glyph of a TrueType font for `code` (named `name` by the encoding), as the PDF reference
/// says to look: through Unicode or the Macintosh map, the symbol map, the font's own names.
fn true_type_glyph(f: &Font, code: u8, name: Option<&str>, symbolic: bool) -> Option<u16> {
    if !symbolic {
        let ch = name.and_then(unicode_of).or_else(|| encoded(Encoding::Standard, code).and_then(unicode_of));
        if let Some(ch) = ch
            && let g = f.glyph(ch)
            && g != 0
        {
            return Some(g);
        }
        if let Some(mac) = name.and_then(|n| code_of(Encoding::MacRoman, n)).or(Some(code))
            && let Some(g) = f.glyph_in(1, 0, mac as u32)
        {
            return Some(g);
        }
    }
    for c in [0xF000 + code as u32, 0xF100 + code as u32, 0xF200 + code as u32, code as u32] {
        if let Some(g) = f.glyph_in(3, 0, c) {
            return Some(g);
        }
    }
    if let Some(g) = f.glyph_in(1, 0, code as u32) {
        return Some(g);
    }
    if let Some(g) = name.and_then(|n| f.glyph_named(n)) {
        return Some(g);
    }
    let g = f.glyph(char::from(code));
    (g != 0).then_some(g)
}

/// The font program a descriptor holds.
fn embedded(file: &File, desc: &Dict, notes: &mut Vec<String>) -> Option<Outlines> {
    let stream = |k: &str| match file.at(desc, k) {
        Some(Obj::Stream(s)) => Some(s),
        _ => None,
    };
    if let Some(s) = stream("FontFile2") {
        let data = file.decoded(&s).ok()?.data;
        return match Font::embedded(data) {
            Ok(f) => Some(Outlines::TrueType(Arc::new(f))),
            Err(e) => {
                notes.push(format!("a TrueType font that cannot be read ({e})"));
                None
            }
        };
    }
    if let Some(s) = stream("FontFile3") {
        let data = file.decoded(&s).ok()?.data;
        if s.dict.get("Subtype").and_then(Obj::as_name) == Some("OpenType") || data.starts_with(b"OTTO") || data.starts_with(&[0, 1, 0, 0]) {
            // its CFF table on its own (to find glyphs by name or CID), else the whole font
            if let Some(cff) = cff_table(&data)
                && let Some(p) = Program::cff(cff)
            {
                return Some(Outlines::Program(Arc::new(p)));
            }
            return Font::embedded(data).ok().map(|f| Outlines::TrueType(Arc::new(f)));
        }
        return match Program::cff(data) {
            Some(p) => Some(Outlines::Program(Arc::new(p))),
            None => {
                notes.push("a CFF font that cannot be read".into());
                None
            }
        };
    }
    if let Some(s) = stream("FontFile") {
        let data = file.decoded(&s).ok()?.data;
        return match Program::type1(&data) {
            Some(p) => Some(Outlines::Program(Arc::new(p))),
            None => {
                notes.push("a Type 1 font that cannot be read".into());
                None
            }
        };
    }
    None
}

/// An OpenType font's `CFF ` table.
fn cff_table(d: &[u8]) -> Option<Vec<u8>> {
    let n = u16::from_be_bytes([*d.get(4)?, *d.get(5)?]) as usize;
    (0..n).map(|i| 12 + 16 * i).find(|&r| d.get(r..r + 4) == Some(b"CFF ")).and_then(|r| {
        let at = u32::from_be_bytes(d.get(r + 8..r + 12)?.try_into().ok()?) as usize;
        let len = u32::from_be_bytes(d.get(r + 12..r + 16)?.try_into().ok()?) as usize;
        d.get(at..at + len).map(<[u8]>::to_vec)
    })
}
