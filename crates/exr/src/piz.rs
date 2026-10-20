//! PIZ chunks: each channel's 16-bit words (a float's two halves on their own) in its own
//! block, the values used numbered in order (a bitmap says which), a wavelet transform of each
//! block, then Huffman codes over all of them.

use crate::{Channel, Window, huf, samples};

/// Two words of a wavelet level put back, 14-bit values (when every number fits).
fn wdec14(l: u16, h: u16) -> (u16, u16) {
    let (ls, hs) = (l as i16 as i32, h as i16 as i32);
    let ai = ls + (hs & 1) + (hs >> 1);
    let (a, b) = (ai as i16, (ai - hs) as i16);
    (a as u16, b as u16)
}

/// Two words of a wavelet level put back, 16-bit values.
fn wdec16(l: u16, h: u16) -> (u16, u16) {
    const A_OFFSET: i32 = 1 << 15;
    const MOD_MASK: i32 = (1 << 16) - 1;
    let (m, d) = (l as i32, h as i32);
    let bb = (m - (d >> 1)) & MOD_MASK;
    let aa = (d + bb - A_OFFSET) & MOD_MASK;
    (aa as u16, bb as u16)
}

/// The wavelet transform undone over `nx` × `ny` words of `v` from `at`, `ox` apart across and
/// `oy` down; `most` is the highest value numbered.
fn unwavelet(v: &mut [u16], at: usize, nx: usize, ox: usize, ny: usize, oy: usize, most: u16) {
    let w14 = most < 1 << 14;
    let dec = if w14 { wdec14 } else { wdec16 };
    let n = nx.min(ny);
    let mut p = 1;
    while p <= n {
        p <<= 1;
    }
    p >>= 1;
    let mut p2 = p;
    p >>= 1;
    while p >= 1 {
        let (oy1, oy2, ox1, ox2) = (oy * p, oy * p2, ox * p, ox * p2);
        let ey = at + oy * (ny - p2);
        let mut py = at;
        while py <= ey {
            let ex = py + ox * (nx - p2);
            let mut px = py;
            while px <= ex {
                let (p01, p10) = (px + ox1, px + oy1);
                let p11 = p10 + ox1;
                let (i00, i10) = dec(v[px], v[p10]);
                let (i01, i11) = dec(v[p01], v[p11]);
                (v[px], v[p01]) = dec(i00, i01);
                (v[p10], v[p11]) = dec(i10, i11);
                px += ox2;
            }
            // the odd column left over
            if nx & p != 0 {
                let p10 = px + oy1;
                let (i00, b) = dec(v[px], v[p10]);
                v[p10] = b;
                v[px] = i00;
            }
            py += oy2;
        }
        // the odd row left over
        if ny & p != 0 {
            let ex = py + ox * (nx - p2);
            let mut px = py;
            while px <= ex {
                let p01 = px + ox1;
                let (i00, b) = dec(v[px], v[p01]);
                v[p01] = b;
                v[px] = i00;
                px += ox2;
            }
        }
        p2 = p;
        p >>= 1;
    }
}

pub(crate) fn decompress(data: &[u8], channels: &[Channel], w: Window, size: usize) -> Result<Vec<u8>, String> {
    let short = || "a PIZ chunk cut short".to_string();
    let words = size / 2;
    // the values used: a bitmap from its first to its last non-zero byte
    let lo = u16::from_le_bytes(data.get(0..2).ok_or_else(short)?.try_into().expect("two bytes")) as usize;
    let hi = u16::from_le_bytes(data.get(2..4).ok_or_else(short)?.try_into().expect("two bytes")) as usize;
    let mut bitmap = [0u8; 8192];
    let mut at = 4;
    if hi >= 8192 {
        return Err("a PIZ chunk whose bitmap is too big".into());
    }
    if lo <= hi {
        bitmap[lo..=hi].copy_from_slice(data.get(at..at + hi - lo + 1).ok_or_else(short)?);
        at += hi - lo + 1;
    }
    let mut lut = vec![0u16; 1 << 16];
    let mut k = 0;
    for i in 0..1 << 16 {
        if i == 0 || bitmap[i >> 3] & (1 << (i & 7)) != 0 {
            lut[k] = i as u16;
            k += 1;
        }
    }
    let most = (k - 1) as u16;
    let n = i32::from_le_bytes(data.get(at..at + 4).ok_or_else(short)?.try_into().expect("four bytes"));
    at += 4;
    let codes = data.get(at..at + usize::try_from(n).map_err(|_| short())?).ok_or_else(short)?;
    let mut v = huf::decode(codes, words)?;
    // each channel's block, its words in their own rows
    let mut blocks = Vec::new();
    let mut start = 0;
    for c in channels {
        let (nx, ny) = (samples(c.sampling[0], w.x0, w.x1), samples(c.sampling[1], w.y0, w.y1));
        let size = c.sample.size() / 2;
        if nx > 0 && ny > 0 {
            for j in 0..size {
                unwavelet(&mut v, start + j, nx, size, ny, nx * size, most);
            }
        }
        blocks.push((start, nx * size));
        start += nx * ny * size;
    }
    for x in v.iter_mut() {
        *x = lut[*x as usize];
    }
    // back to scanlines
    let mut out = Vec::with_capacity(size);
    for y in w.y0..=w.y1 {
        for (c, (at, row)) in channels.iter().zip(blocks.iter_mut()) {
            if y.rem_euclid(c.sampling[1]) != 0 {
                continue;
            }
            for x in &v[*at..*at + *row] {
                out.extend(x.to_le_bytes());
            }
            *at += *row;
        }
    }
    Ok(out)
}
