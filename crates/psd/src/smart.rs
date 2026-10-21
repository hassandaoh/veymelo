//! Smart objects: how a layer places what it holds (`SoLd`, or the older `PlLd`) — the content's
//! id, its four corners in the document, its size, whether it is warped or filtered — and the
//! files the document holds for them (`lnk2`, `lnkD`, `lnk3`: each one's id, name, kind and
//! bytes, or where a linked one is on disk).

use crate::Psd;
use crate::descriptor::{self, Desc, Item};

/// A smart object's placing.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// The id of the file it shows (a [`Linked`]'s).
    pub id: String,
    /// Where its content's top left, top right, bottom right and bottom left corners are in the
    /// document.
    pub corners: [[f64; 2]; 4],
    /// Its content's size, where it says.
    pub size: Option<[f64; 2]>,
    /// The page shown of content with pages (from 1).
    pub page: Option<u32>,
    pub warped: bool,
    pub filtered: bool,
    /// Its descriptor (none for the older placing).
    pub desc: Option<Desc>,
}

/// A file the document holds (or links) for its smart objects.
#[derive(Clone, Debug, PartialEq)]
pub struct Linked {
    pub id: String,
    /// The file's name when it was placed.
    pub name: String,
    /// Its kind's four letters (`png `, `JPEG`, `8BPS`, `PDF `…; blank where not said).
    pub kind: [u8; 4],
    /// Where its bytes are in the document, when it holds them.
    data: Option<(usize, usize)>,
    /// Where a linked file is on disk, when it is linked.
    pub path: Option<String>,
}

fn numbers(items: &[Item]) -> Vec<f64> {
    items.iter().filter_map(|i| if let Item::Double(v) | Item::Unit(_, v) = i { Some(*v) } else { None }).collect()
}

fn corners(n: &[f64]) -> Option<[[f64; 2]; 4]> {
    (n.len() >= 8).then(|| [[n[0], n[1]], [n[2], n[3]], [n[4], n[5]], [n[6], n[7]]])
}

/// A Pascal string (a length byte, then the bytes).
fn pascal(d: &[u8], at: &mut usize) -> Option<String> {
    let n = *d.get(*at)? as usize;
    let s = d.get(*at + 1..*at + 1 + n)?;
    *at += 1 + n;
    Some(String::from_utf8_lossy(s).into_owned())
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn u64_at(d: &[u8], at: usize) -> Option<u64> {
    d.get(at..at + 8).map(|b| u64::from_be_bytes(b.try_into().expect("eight bytes")))
}

/// The files of one `lnk2`/`lnkD`/`lnk3` block, which starts at `base` in the document.
fn linked_in(d: &[u8], base: usize, out: &mut Vec<Linked>) {
    let mut at = 0usize;
    while at + 8 <= d.len() {
        let Some(len) = u64_at(d, at).map(|l| l as usize) else { break };
        let end = (at + 8).saturating_add(len).min(d.len());
        let e = &d[..end];
        let mut k = at + 8;
        let entry = (|| -> Option<Linked> {
            let kind: [u8; 4] = e.get(k..k + 4)?.try_into().ok()?;
            let version = u32_at(e, k + 4)?;
            k += 8;
            let id = pascal(e, &mut k)?;
            let mut r = descriptor::Reader { d: e, at: k };
            let name = r.unicode().ok()?;
            k = r.at;
            let file_kind: [u8; 4] = e.get(k..k + 4)?.try_into().ok()?;
            let size = u64_at(e, k + 8)? as usize;
            k += 16;
            // the settings it was opened with, where it has them
            let opened = *e.get(k)? != 0;
            k += 1;
            if opened {
                let mut r = descriptor::Reader { d: e, at: k + 4 };
                r.desc(0).ok()?;
                k = r.at;
            }
            let mut path = None;
            let mut data = None;
            match &kind {
                b"liFD" => data = (k + size <= e.len()).then_some((base + k, size)),
                b"liFE" => {
                    let mut r = descriptor::Reader { d: e, at: k + 4 };
                    let file = r.desc(0).ok()?;
                    path = file.text("fullPath").or(file.text("originalPath")).map(|p| p.trim_start_matches("file://").to_string());
                    k = r.at;
                    if version > 3 {
                        k += 4 + 4 + 8;
                    }
                    let held = u64_at(e, k).unwrap_or(0) as usize;
                    k += 8;
                    if version > 2 && held > 0 && k + held <= e.len() {
                        data = Some((base + k, held));
                    }
                }
                _ => {}
            }
            Some(Linked { id, name, kind: file_kind, data, path })
        })();
        if let Some(l) = entry {
            out.push(l);
        }
        // each padded to four bytes
        at = end.div_ceil(4) * 4;
    }
}

impl Psd {
    /// Layer `i`'s smart object placing, where it is one.
    pub fn placed(&self, i: usize) -> Option<Placed> {
        if let Some(d) = self.block(i, b"SoLd").or_else(|| self.block(i, b"SoLE")) {
            // `soLD`, its version, then the descriptor's
            let desc = descriptor::read(d.get(8..)?, true).ok()?;
            let quad = desc.list("nonAffineTransform").map(numbers).and_then(|n| corners(&n)).or_else(|| desc.list("Trnf").map(numbers).and_then(|n| corners(&n)))?;
            let size = desc.desc("Sz  ").map(|s| [s.num("Wdth").unwrap_or(0.0), s.num("Hght").unwrap_or(0.0)]).filter(|s| s[0] > 0.0 && s[1] > 0.0);
            let warped = desc.desc("warp").is_some_and(|w| w.enumeration("warpStyle").is_some_and(|s| s != "warpNone") || w.desc("customEnvelopeWarp").is_some());
            return Some(Placed {
                id: desc.text("Idnt").or(desc.text("placed")).unwrap_or("").to_string(),
                corners: quad,
                size,
                page: desc.num("PgNm").map(|p| p.max(1.0) as u32),
                warped,
                filtered: desc.get("filterFX").is_some(),
                desc: Some(desc),
            });
        }
        // the older placing: `plcL`, its version, the id, the page and pages, anti-aliasing,
        // the kind, then the corners
        let d = self.block(i, b"PlLd").or_else(|| self.block(i, b"plLd"))?;
        let mut k = 8;
        let id = pascal(d, &mut k)?;
        let page = u32_at(d, k)?;
        k += 16;
        let n: Vec<f64> = (0..8).filter_map(|j| d.get(k + 8 * j..k + 8 * j + 8).map(|b| f64::from_be_bytes(b.try_into().expect("eight bytes")))).collect();
        Some(Placed { id, corners: corners(&n)?, size: None, page: Some(page.max(1)), warped: false, filtered: false, desc: None })
    }

    /// The files the document holds or links for its smart objects.
    pub fn linked_files(&self) -> Vec<Linked> {
        let mut out = Vec::new();
        for b in self.global.iter().filter(|b| matches!(&b.key, b"lnk2" | b"lnkD" | b"lnk3")) {
            if let Some(d) = self.data.get(b.at..b.at + b.len) {
                linked_in(d, b.at, &mut out);
            }
        }
        out
    }

    /// A held file's bytes.
    pub fn linked_data(&self, l: &Linked) -> Option<&[u8]> {
        let (at, n) = l.data?;
        self.data.get(at..at + n)
    }
}
