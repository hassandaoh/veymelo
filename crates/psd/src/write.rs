//! Photoshop documents written, a small part of the format: RGB layers (8 or 16 bits) with their
//! channels compressed any of the four ways, names (ASCII and Unicode), blend modes, opacity,
//! visibility, layer masks and group dividers. Enough to make documents to read back.

use crate::Bounds;

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
}

/// A layer called `name` covering `bounds`, normal, opaque, shown, with no channels yet.
pub fn layer(name: &'static str, bounds: Bounds) -> Write {
    Write { name, bounds, blend: b"norm", opacity: 255, hidden: false, section: None, channels: Vec::new(), mask: None }
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
    f.extend(0u32.to_be_bytes());
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
                extra.extend(20u32.to_be_bytes());
                for v in mb {
                    extra.extend(v.to_be_bytes());
                }
                extra.extend([default, 0, 0, 0]);
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
    f.extend(((info.len() + 4 + 4) as u32).to_be_bytes());
    f.extend((info.len() as u32).to_be_bytes());
    f.extend(info);
    f.extend(0u32.to_be_bytes());
    // the merged picture (not read): raw, empty
    f.extend(0u16.to_be_bytes());
    f
}

