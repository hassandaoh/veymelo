//! Photoshop documents written, a small part of the format: RGB layers (8 or 16 bits) with their
//! channels compressed any of the four ways, names (ASCII and Unicode), blend modes, opacity,
//! visibility, layer masks (with a feather) and group dividers, any additional information
//! blocks, image resources and the document's own blocks; and the blocks themselves for vector
//! masks, type layers and smart objects. Enough to make documents to read back.

use crate::Bounds;
use crate::descriptor::{Desc, write::versioned};

/// A layer to write: its name, rectangle, blend mode, opacity, flags, what kind of record (group
/// folder or divider), channels (id, compression, samples), mask (rectangle, default) and fill.
pub struct Write {
    pub name: &'static str,
    pub bounds: Bounds,
    pub blend: &'static [u8; 4],
    pub opacity: u8,
    pub hidden: bool,
    pub section: Option<u32>,
    pub channels: Vec<(i16, u16, Vec<u8>)>,
    pub mask: Option<(Bounds, u8)>,
    /// The mask's feather (pixels), and whether it was drawn from the vector mask.
    pub mask_feather: f64,
    pub mask_rendered: bool,
    /// More additional information blocks.
    pub info: Vec<([u8; 4], Vec<u8>)>,
}

/// A layer called `name` covering `bounds`, normal, opaque, shown, with no channels yet.
pub fn layer(name: &'static str, bounds: Bounds) -> Write {
    Write { name, bounds, blend: b"norm", opacity: 255, hidden: false, section: None, channels: Vec::new(), mask: None, mask_feather: 0.0, mask_rendered: false, info: Vec::new() }
}

/// A descriptor block as fill layers and smart objects keep them: its version, then it.
pub fn desc_block(d: &Desc) -> Vec<u8> {
    versioned(d)
}

/// Layer effects (`lfx2`): their version, then the descriptor's.
pub fn effects_block(d: &Desc) -> Vec<u8> {
    let mut out = 0u32.to_be_bytes().to_vec();
    out.extend(versioned(d));
    out
}

/// A subpath to write: its knots (control point before, knot, control point after; document
/// pixels), whether it closes and how it joins (−1 merged, 0 exclude, 1 combine, 2 subtract, 3
/// intersect).
pub type Subpath = (Vec<[[f64; 2]; 3]>, bool, i16);

/// A vector mask (`vmsk`) of subpaths for a document `w` × `h`; `flags` 1 inverted, 4 off.
pub fn vector_block(paths: &[Subpath], w: f64, h: f64, flags: u32) -> Vec<u8> {
    let mut out = 3u32.to_be_bytes().to_vec();
    out.extend(flags.to_be_bytes());
    let rec = |sel: u16, body: &[u8]| {
        let mut r = sel.to_be_bytes().to_vec();
        r.extend(body);
        r.resize(26, 0);
        r
    };
    out.extend(rec(6, &[]));
    out.extend(rec(8, &[]));
    let fixed = |v: f64| ((v * (1 << 24) as f64).round() as i32).to_be_bytes();
    for (knots, closed, op) in paths {
        let mut b = (knots.len() as u16).to_be_bytes().to_vec();
        b.extend(op.to_be_bytes());
        b.extend(1u16.to_be_bytes());
        out.extend(rec(if *closed { 0 } else { 3 }, &b));
        for k in knots {
            let mut b = Vec::new();
            for p in k {
                b.extend(fixed(p[1] / h));
                b.extend(fixed(p[0] / w));
            }
            out.extend(rec(if *closed { 2 } else { 5 }, &b));
        }
    }
    out
}

/// A type layer (`TySh`): its transform, then its text descriptor (holding `Txt ` and the
/// `EngineData`), then an empty warp.
pub fn type_block(transform: [f64; 6], text: &Desc) -> Vec<u8> {
    let mut out = 1u16.to_be_bytes().to_vec();
    for v in transform {
        out.extend(v.to_be_bytes());
    }
    out.extend(50u16.to_be_bytes());
    out.extend(versioned(text));
    out.extend(1u16.to_be_bytes());
    out.extend(versioned(&Desc { class: "warp".into(), items: Vec::new() }));
    out.extend([0; 16]);
    out
}

/// A smart object's placing (`SoLd`).
pub fn placed_block(d: &Desc) -> Vec<u8> {
    let mut out = b"soLD".to_vec();
    out.extend(4u32.to_be_bytes());
    out.extend(versioned(d));
    out
}

/// A file held for smart objects (one `lnk2` entry): its id, name, kind and bytes.
pub fn linked_block(id: &str, name: &str, kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut e = b"liFD".to_vec();
    e.extend(2u32.to_be_bytes());
    e.push(id.len() as u8);
    e.extend(id.as_bytes());
    let units: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    e.extend((units.len() as u32).to_be_bytes());
    for u in units {
        e.extend(u.to_be_bytes());
    }
    e.extend(kind);
    e.extend(b"    ");
    e.extend((data.len() as u64).to_be_bytes());
    e.push(0);
    e.extend(data);
    let mut out = (e.len() as u64).to_be_bytes().to_vec();
    out.extend(e);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    out
}

/// Samples as a channel's data compressed as `how` (`w` samples a row, `bytes` each).
pub fn encode(how: u16, s: &[u8], w: usize, bytes: usize) -> Vec<u8> {
    let mut out = how.to_be_bytes().to_vec();
    let row = (w * bytes).max(1);
    match how {
        0 => out.extend(s),
        1 => {
            let rows: Vec<Vec<u8>> = s
                .chunks(row)
                .map(|r| {
                    // literal runs only
                    let mut p = Vec::new();
                    for c in r.chunks(128) {
                        p.push((c.len() - 1) as u8);
                        p.extend(c);
                    }
                    p
                })
                .collect();
            for r in &rows {
                out.extend((r.len() as u16).to_be_bytes());
            }
            for r in rows {
                out.extend(r);
            }
        }
        2 => out.extend(veymelo_zip::zlib(s)),
        _ => {
            // differences along each row (8 or 16 bits)
            let mut d = s.to_vec();
            for r in d.chunks_mut(row) {
                if bytes == 1 {
                    for x in (1..r.len()).rev() {
                        r[x] = r[x].wrapping_sub(r[x - 1]);
                    }
                } else {
                    for x in (1..w).rev() {
                        let prev = u16::from_be_bytes([r[2 * x - 2], r[2 * x - 1]]);
                        let cur = u16::from_be_bytes([r[2 * x], r[2 * x + 1]]).wrapping_sub(prev);
                        r[2 * x..2 * x + 2].copy_from_slice(&cur.to_be_bytes());
                    }
                }
            }
            out.extend(veymelo_zip::zlib(&d));
        }
    }
    out
}

/// A document `w` × `h` of RGB `depth`-bit layers (given bottom first).
pub fn write(w: u32, h: u32, depth: u16, layers: &[Write]) -> Vec<u8> {
    write_with(w, h, depth, layers, &[], &[])
}

/// A document as [`write`] makes, with image resources (id, data) and blocks of its own.
pub fn write_with(w: u32, h: u32, depth: u16, layers: &[Write], resources: &[(u16, Vec<u8>)], global: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let bytes = (depth / 8).max(1) as usize;
    let mut f = b"8BPS".to_vec();
    f.extend(1u16.to_be_bytes());
    f.extend([0; 6]);
    f.extend(3u16.to_be_bytes());
    f.extend(h.to_be_bytes());
    f.extend(w.to_be_bytes());
    f.extend(depth.to_be_bytes());
    f.extend(3u16.to_be_bytes());
    f.extend(0u32.to_be_bytes());
    let mut res = Vec::new();
    for (id, d) in resources {
        res.extend(b"8BIM");
        res.extend(id.to_be_bytes());
        res.extend([0, 0]);
        res.extend((d.len() as u32).to_be_bytes());
        res.extend(d);
        if d.len() % 2 == 1 {
            res.push(0);
        }
    }
    f.extend((res.len() as u32).to_be_bytes());
    f.extend(res);
    let mut records = Vec::new();
    let mut data = Vec::new();
    for l in layers {
        let [t, lft, b, r] = l.bounds;
        let lw = (r - lft).max(0) as usize;
        let encoded: Vec<(i16, Vec<u8>)> = l
            .channels
            .iter()
            .map(|(id, how, s)| {
                let cw = if *id == -2 { l.mask.map_or(lw, |m| (m.0[3] - m.0[1]) as usize) } else { lw };
                (*id, encode(*how, s, cw, bytes))
            })
            .collect();
        for v in [t, lft, b, r] {
            records.extend(v.to_be_bytes());
        }
        records.extend((encoded.len() as u16).to_be_bytes());
        for (id, e) in &encoded {
            records.extend(id.to_be_bytes());
            records.extend((e.len() as u32).to_be_bytes());
        }
        records.extend(b"8BIM");
        records.extend(l.blend);
        records.extend([l.opacity, 0, if l.hidden { 2 } else { 0 }, 0]);
        let mut extra = Vec::new();
        match l.mask {
            Some((mb, default)) => {
                let mut m = Vec::new();
                for v in mb {
                    m.extend(v.to_be_bytes());
                }
                let flags = if l.mask_rendered { 8 } else { 0 } | if l.mask_feather > 0.0 { 16 } else { 0 };
                m.extend([default, flags]);
                if l.mask_feather > 0.0 {
                    m.push(2);
                    m.extend(l.mask_feather.to_be_bytes());
                }
                m.resize(m.len().max(20), 0);
                extra.extend((m.len() as u32).to_be_bytes());
                extra.extend(m);
            }
            None => extra.extend(0u32.to_be_bytes()),
        }
        extra.extend(0u32.to_be_bytes());
        // the name, ASCII and Unicode
        let n = l.name.len();
        extra.push(n as u8);
        extra.extend(l.name.as_bytes());
        extra.extend(std::iter::repeat_n(0, (4 - (n + 1) % 4) % 4));
        let units: Vec<u16> = l.name.encode_utf16().collect();
        let mut luni = (units.len() as u32).to_be_bytes().to_vec();
        for u in units {
            luni.extend(u.to_be_bytes());
        }
        if luni.len() % 2 == 1 {
            luni.push(0);
        }
        extra.extend(b"8BIMluni");
        extra.extend((luni.len() as u32).to_be_bytes());
        extra.extend(luni);
        if let Some(kind) = l.section {
            extra.extend(b"8BIMlsct");
            extra.extend(12u32.to_be_bytes());
            extra.extend(kind.to_be_bytes());
            extra.extend(b"8BIM");
            extra.extend(l.blend);
        }
        for (key, d) in &l.info {
            extra.extend(b"8BIM");
            extra.extend(key);
            let mut d = d.clone();
            if d.len() % 2 == 1 {
                d.push(0);
            }
            extra.extend((d.len() as u32).to_be_bytes());
            extra.extend(d);
        }
        records.extend((extra.len() as u32).to_be_bytes());
        records.extend(extra);
        for (_, e) in encoded {
            data.extend(e);
        }
    }
    let mut info = (layers.len() as u16).to_be_bytes().to_vec();
    info.extend(records);
    info.extend(data);
    if info.len() % 2 == 1 {
        info.push(0);
    }
    let mut tail = 0u32.to_be_bytes().to_vec();
    for (key, d) in global {
        tail.extend(b"8BIM");
        tail.extend(key);
        tail.extend((d.len() as u32).to_be_bytes());
        tail.extend(d);
        while !tail.len().is_multiple_of(4) {
            tail.push(0);
        }
    }
    f.extend(((info.len() + 4 + tail.len()) as u32).to_be_bytes());
    f.extend((info.len() as u32).to_be_bytes());
    f.extend(info);
    f.extend(tail);
    // the merged picture (not read): raw, empty
    f.extend(0u16.to_be_bytes());
    f
}

