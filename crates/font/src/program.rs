//! Font programs on their own, as PDF files embed them: a bare CFF font (`Type1C`, or a CID-keyed
//! `CIDFontType0C`) or a Type 1 font. Each glyph is found by its name, by the code of the font's
//! own encoding, or (CID-keyed) by its CID; its outline is drawn in the font's units, which its
//! matrix takes to text space (an em of 1).

use std::collections::HashMap;

use crate::cff::{Cff, dict_get, read_dict, read_index, u8_at, uint};
use crate::names::{Encoding, cff_string, encoded};
use crate::type1::Type1;
use crate::Pen;

enum Kind {
    Cff { data: Vec<u8>, cff: Cff },
    Type1(Type1),
}

/// A font program, read.
pub struct Program {
    kind: Kind,
    names: HashMap<String, u16>,
    codes: Vec<Option<u16>>,
    cids: Option<HashMap<u16, u16>>,
    matrix: [f64; 6],
    glyphs: usize,
}

impl Program {
    /// A bare CFF font (its first, the only one a PDF holds).
    pub fn cff(data: Vec<u8>) -> Option<Program> {
        let d = &data[..];
        let header = u8_at(d, 2);
        let (_names, p) = read_index(d, header, false);
        let (tops, p) = read_index(d, p, false);
        let (strings, _) = read_index(d, p, false);
        let (a, b) = tops.get(0)?;
        let top = read_dict(d, a, b);
        let charstrings = read_index(d, *dict_get(&top, 17)?.first()? as usize, false).0;
        let glyphs = charstrings.len();
        let cff = Cff::read(d, 0, false, glyphs)?;
        let string = |sid: usize| -> String {
            if sid < 391 {
                cff_string(sid).unwrap_or("").to_string()
            } else {
                strings.get(sid - 391).map(|(a, b)| String::from_utf8_lossy(&d[a..b]).into_owned()).unwrap_or_default()
            }
        };
        let cid_keyed = dict_get(&top, 1230).is_some();
        // each glyph's string number (or CID): the charset
        let mut sids = vec![0usize; glyphs];
        match dict_get(&top, 15).and_then(|v| v.first()).map_or(0, |v| *v as usize) {
            0 => sids.iter_mut().enumerate().for_each(|(g, s)| *s = g),
            1 | 2 => {}
            at => {
                let mut p = at + 1;
                match u8_at(d, at) {
                    0 => {
                        for s in sids.iter_mut().skip(1) {
                            *s = uint(d, p, 2);
                            p += 2;
                        }
                    }
                    f @ (1 | 2) => {
                        let mut g = 1;
                        while g < glyphs && p < d.len() {
                            let first = uint(d, p, 2);
                            let left = if f == 1 { u8_at(d, p + 2) } else { uint(d, p + 2, 2) };
                            p += if f == 1 { 3 } else { 4 };
                            for k in 0..=left {
                                if g < glyphs {
                                    sids[g] = first + k;
                                    g += 1;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut names = HashMap::new();
        let mut cids = None;
        if cid_keyed {
            cids = Some(sids.iter().enumerate().map(|(g, &c)| (c as u16, g as u16)).collect());
        } else {
            for (g, &sid) in sids.iter().enumerate() {
                names.entry(string(sid)).or_insert(g as u16);
            }
        }
        // the font's own codes: Standard's, or its own list (with more codes after it)
        let mut codes = vec![None; 256];
        if !cid_keyed {
            match dict_get(&top, 16).and_then(|v| v.first()).map_or(0, |v| *v as usize) {
                0 => {
                    for (c, slot) in codes.iter_mut().enumerate() {
                        *slot = encoded(Encoding::Standard, c as u8).and_then(|n| names.get(n).copied());
                    }
                }
                1 => {}
                at => {
                    let format = u8_at(d, at);
                    let mut p = at + 1;
                    match format & 0x7F {
                        0 => {
                            let n = u8_at(d, p);
                            for g in 1..=n {
                                codes[u8_at(d, p + g)] = Some(g as u16);
                            }
                            p += 1 + n;
                        }
                        1 => {
                            let n = u8_at(d, p);
                            let mut g = 1u16;
                            for r in 0..n {
                                let (first, left) = (u8_at(d, p + 1 + 2 * r), u8_at(d, p + 2 + 2 * r));
                                for slot in codes.iter_mut().take((first + left).min(255) + 1).skip(first) {
                                    *slot = Some(g);
                                    g += 1;
                                }
                            }
                            p += 1 + 2 * n;
                        }
                        _ => {}
                    }
                    if format & 0x80 != 0 {
                        for k in 0..u8_at(d, p) {
                            let r = p + 1 + 3 * k;
                            codes[u8_at(d, r)] = names.get(&string(uint(d, r + 1, 2))).copied();
                        }
                    }
                }
            }
        }
        let matrix = |dict: &[(u16, Vec<f64>)]| dict_get(dict, 1207).and_then(|v| <[f64; 6]>::try_from(v).ok());
        let mut m = matrix(&top);
        if cid_keyed && let Some(fd_array) = dict_get(&top, 1236).and_then(|v| v.first()) {
            let (fds, _) = read_index(d, *fd_array as usize, false);
            if let Some((a, b)) = fds.get(0)
                && let Some(fm) = matrix(&read_dict(d, a, b))
            {
                m = Some(match m {
                    Some(t) => crate::colr::times(t, fm),
                    None => fm,
                });
            }
        }
        Some(Program { kind: Kind::Cff { cff, data }, names, codes, cids, matrix: m.unwrap_or([0.001, 0.0, 0.0, 0.001, 0.0, 0.0]), glyphs })
    }

    /// A Type 1 font (`.pfb` segments, or its clear text then its locked part).
    pub fn type1(data: &[u8]) -> Option<Program> {
        let t = Type1::read(data)?;
        let names: HashMap<String, u16> = t.by_name.iter().map(|(n, &g)| (n.clone(), g as u16)).collect();
        let codes = (0..256)
            .map(|c| match &t.encoding {
                Some(e) => e[c].as_ref().and_then(|n| names.get(n).copied()),
                None => encoded(Encoding::Standard, c as u8).and_then(|n| names.get(n).copied()),
            })
            .collect();
        let (matrix, glyphs) = (t.matrix, t.names.len());
        Some(Program { kind: Kind::Type1(t), names, codes, cids: None, matrix, glyphs })
    }

    /// How many glyphs it has.
    pub fn glyphs(&self) -> usize {
        self.glyphs
    }

    /// The glyph named `name`.
    pub fn glyph_named(&self, name: &str) -> Option<u16> {
        self.names.get(name).copied()
    }

    /// The glyph the font's own encoding gives `code`.
    pub fn glyph_coded(&self, code: u8) -> Option<u16> {
        self.codes.get(code as usize).copied().flatten()
    }

    /// The glyph of CID `cid` (a font that is not CID-keyed: the glyph numbered so).
    pub fn glyph_of_cid(&self, cid: u16) -> Option<u16> {
        match &self.cids {
            Some(m) => m.get(&cid).copied(),
            None => ((cid as usize) < self.glyphs).then_some(cid),
        }
    }

    /// Whether glyphs are found by CID.
    pub fn cid_keyed(&self) -> bool {
        self.cids.is_some()
    }

    /// From the font's units to text space (an em of 1): `[a, b, c, d, e, f]`.
    pub fn matrix(&self) -> [f64; 6] {
        self.matrix
    }

    /// Glyph `g`'s outline in the font's units, through `place`, into `pen`; how far it moves
    /// the pen when the font says (Type 1).
    pub fn outline(&self, g: u16, pen: &mut dyn Pen, place: &dyn Fn(f64, f64) -> (f64, f64)) -> Option<f64> {
        match &self.kind {
            Kind::Cff { cff, data } => {
                cff.outline(data, g, pen, place, &|_, _| 0.0);
                None
            }
            Kind::Type1(t) => t.outline(g as usize, pen, place),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Font;
    use crate::names::unicode_of;
    use veymelo_path::Path;

    fn extent(f: &dyn Fn(&mut Path)) -> Option<(f64, f64, f64, f64)> {
        let mut p = Path::default();
        f(&mut p);
        p.extent()
    }

    #[test]
    fn a_bare_cff_font_finds_glyphs_by_name_and_code() {
        // the CFF table of a font on the system, on its own
        let Ok(bytes) = std::fs::read("/System/Library/Fonts/Supplemental/STIXGeneral.otf").or_else(|_| std::fs::read("/Library/Fonts/STIXGeneral.otf")) else { return };
        let font = Font::from_data(bytes.clone(), 0).unwrap();
        let n = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        let rec = (0..n).map(|i| 12 + 16 * i).find(|&r| &bytes[r..r + 4] == b"CFF ").unwrap();
        let at = u32::from_be_bytes(bytes[rec + 8..rec + 12].try_into().unwrap()) as usize;
        let len = u32::from_be_bytes(bytes[rec + 12..rec + 16].try_into().unwrap()) as usize;
        let p = Program::cff(bytes[at..at + len].to_vec()).unwrap();
        assert!(!p.cid_keyed());
        assert_eq!(p.glyphs(), font.glyphs as usize);
        assert!((p.matrix()[0] - 1.0 / font.units).abs() < 1e-9);
        // its names agree with its Unicode map, and its outlines with the font's
        for name in ["A", "a", "zero", "eacute", "quoteright", "fi", "germandbls", "Aring"] {
            let c = unicode_of(name).unwrap();
            let g = font.glyph(c);
            if g == 0 {
                continue;
            }
            assert_eq!(p.glyph_named(name), Some(g), "{name}");
            let mine = extent(&|path| {
                p.outline(g, path, &|x, y| (x, y));
            });
            let theirs = extent(&|path| font.outline(g, path, &|x, y| (x, y)));
            assert_eq!(mine, theirs, "{name}");
        }
        // Standard's code for A
        assert_eq!(p.glyph_coded(65), p.glyph_named("A"));
    }

    /// `plain` locked with key `r` after four bytes of `pad`.
    fn lock(plain: &[u8], mut r: u16, pad: u8) -> Vec<u8> {
        let mut out = Vec::new();
        for &b in [pad; 4].iter().chain(plain) {
            let c = b ^ (r >> 8) as u8;
            out.push(c);
            r = (c as u16).wrapping_add(r).wrapping_mul(52845).wrapping_add(22719);
        }
        out
    }

    /// A charstring's numbers and operators (operator n as `OP + n`, escaped ones `OP + 1200 + n`).
    const OP: i32 = 1 << 20;

    fn charstring(items: &[i32]) -> Vec<u8> {
        let mut out = Vec::new();
        for &v in items {
            if v >= OP + 1200 {
                out.extend([12, (v - OP - 1200) as u8]);
            } else if v >= OP {
                out.push((v - OP) as u8);
            } else if (-107..=107).contains(&v) {
                out.push((v + 139) as u8);
            } else {
                out.push(255);
                out.extend(v.to_be_bytes());
            }
        }
        out
    }

    fn type1_font(hex: bool) -> Vec<u8> {
        let (hsbw, rmoveto, rlineto, closepath, endchar, callsubr, ret) = (OP + 13, OP + 21, OP + 5, OP + 9, OP + 14, OP + 10, OP + 11);
        let (seac, callother, pop, setcp) = (OP + 1206, OP + 1216, OP + 1217, OP + 1233);
        let square = charstring(&[50, 600, hsbw, 0, 0, rmoveto, 500, 0, rlineto, 0, 700, rlineto, -500, 0, rlineto, closepath, endchar]);
        let acute = charstring(&[20, 300, hsbw, 200, 800, rmoveto, 100, 0, rlineto, 0, 100, rlineto, closepath, endchar]);
        // a line drawn by a subroutine
        let by_subr = charstring(&[0, 500, hsbw, 0, 0, rmoveto, 5, callsubr, closepath, endchar]);
        let subr5 = charstring(&[400, 0, rlineto, 0, 400, rlineto, ret]);
        // A with an acute 150 across: Standard's 65 and 194
        let accented = charstring(&[50, 600, hsbw, 20, 150, 0, 65, 194, seac]);
        // a flex: from (0, 0) two curves to (200, 0) through a bump
        let mut flex = vec![0, 400, hsbw, 0, 0, rmoveto, 0, 1, callother];
        for (dx, dy) in [(100, 10), (-100, -10), (30, 0), (40, 10), (30, 0), (40, -10), (60, 0)] {
            flex.extend([dx, dy, rmoveto, 0, 2, callother]);
        }
        flex.extend([50, 200, 0, 3, 0, callother, pop, pop, setcp, 0, 0, rlineto, closepath, endchar]);
        let flex = charstring(&flex);
        let mut private = b"dup /Private 8 dict dup begin /lenIV 4 def /Subrs 6 array\n".to_vec();
        for i in 0..6 {
            let s = lock(&if i == 5 { subr5.clone() } else { charstring(&[ret]) }, 4330, 7);
            private.extend(format!("dup {i} {} RD ", s.len()).as_bytes());
            private.extend(s);
            private.extend(b" NP\n");
        }
        private.extend(b"ND 2 index /CharStrings 6 dict dup begin\n");
        for (name, cs) in [(".notdef", charstring(&[0, 0, hsbw, endchar])), ("A", square), ("acute", acute), ("line", by_subr), ("Aacute", accented), ("bump", flex)] {
            let s = lock(&cs, 4330, 3);
            private.extend(format!("/{name} {} RD ", s.len()).as_bytes());
            private.extend(s);
            private.extend(b" ND\n");
        }
        private.extend(b"end end\nmark currentfile closefile\n");
        let mut out = b"%!PS-AdobeFont-1.0: Test 001\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\ndup 65 /A put\ndup 66 /line put\ndup 67 /bump put\nreadonly def\ncurrentfile eexec\n".to_vec();
        let locked = lock(&private, 55665, 9);
        if hex {
            for chunk in locked.chunks(32) {
                out.extend(chunk.iter().map(|b| format!("{b:02x}")).collect::<String>().as_bytes());
                out.push(b'\n');
            }
        } else {
            out.extend(locked);
        }
        out.extend(b"\n0000000000000000\ncleartomark\n");
        out
    }

    #[test]
    fn a_type1_font_draws_its_charstrings() {
        for hex in [false, true] {
            let p = Program::type1(&type1_font(hex)).unwrap();
            assert_eq!(p.glyphs(), 6);
            assert_eq!(p.matrix(), [0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
            let a = p.glyph_coded(65).unwrap();
            assert_eq!(Some(a), p.glyph_named("A"));
            let mut path = Path::default();
            assert_eq!(p.outline(a, &mut path, &|x, y| (x, y)), Some(600.0));
            // drawn from its side bearing
            assert_eq!(path.extent(), Some((50.0, 0.0, 550.0, 700.0)));
            let line = p.glyph_coded(66).unwrap();
            let e = extent(&|path| {
                p.outline(line, path, &|x, y| (x, y));
            });
            assert_eq!(e, Some((0.0, 0.0, 400.0, 400.0)));
            // the accented A: the square and the acute moved 150 across (less its side bearing)
            let e = extent(&|path| {
                p.outline(p.glyph_named("Aacute").unwrap(), path, &|x, y| (x, y));
            });
            assert_eq!(e, Some((50.0, 0.0, 550.0, 900.0)));
            // the flex bump: up to about 10 between 0 and 200
            let mut path = Path::default();
            p.outline(p.glyph_coded(67).unwrap(), &mut path, &|x, y| (x, y));
            let (x0, y0, x1, y1) = path.extent().unwrap();
            assert!((x0 - 0.0).abs() < 1e-9 && (x1 - 200.0).abs() < 1e-9 && y0 > -1.0 && y1 > 5.0 && y1 <= 10.0, "{:?}", path.extent());
        }
    }
}
