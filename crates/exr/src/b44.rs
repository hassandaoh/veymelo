//! B44 and B44A chunks: each half channel in 4 × 4 blocks of 14 bytes (its first value, then
//! the steps between the others, all shifted by one amount), or of 3 bytes when all sixteen
//! are the same (B44A); other channels as they are. Channels marked perceptually linear were
//! kept as logarithms.

use crate::{Channel, Sample, Window, half, samples};

/// A block of fourteen bytes.
fn unpack14(b: &[u8]) -> [u16; 16] {
    let b: Vec<u32> = b.iter().map(|v| *v as u32).collect();
    let mut s = [0u32; 16];
    s[0] = b[0] << 8 | b[1];
    let shift = b[2] >> 2;
    let bias = 0x20u32 << shift;
    let step = |v: u32| (v & 0x3f) << shift;
    s[4] = s[0].wrapping_add(step(b[2] << 4 | b[3] >> 4)).wrapping_sub(bias);
    s[8] = s[4].wrapping_add(step(b[3] << 2 | b[4] >> 6)).wrapping_sub(bias);
    s[12] = s[8].wrapping_add(step(b[4])).wrapping_sub(bias);
    s[1] = s[0].wrapping_add(step(b[5] >> 2)).wrapping_sub(bias);
    s[5] = s[4].wrapping_add(step(b[5] << 4 | b[6] >> 4)).wrapping_sub(bias);
    s[9] = s[8].wrapping_add(step(b[6] << 2 | b[7] >> 6)).wrapping_sub(bias);
    s[13] = s[12].wrapping_add(step(b[7])).wrapping_sub(bias);
    s[2] = s[1].wrapping_add(step(b[8] >> 2)).wrapping_sub(bias);
    s[6] = s[5].wrapping_add(step(b[8] << 4 | b[9] >> 4)).wrapping_sub(bias);
    s[10] = s[9].wrapping_add(step(b[9] << 2 | b[10] >> 6)).wrapping_sub(bias);
    s[14] = s[13].wrapping_add(step(b[10])).wrapping_sub(bias);
    s[3] = s[2].wrapping_add(step(b[11] >> 2)).wrapping_sub(bias);
    s[7] = s[6].wrapping_add(step(b[11] << 4 | b[12] >> 4)).wrapping_sub(bias);
    s[11] = s[10].wrapping_add(step(b[12] << 2 | b[13] >> 6)).wrapping_sub(bias);
    s[15] = s[14].wrapping_add(step(b[13])).wrapping_sub(bias);
    s.map(|v| ordered(v as u16))
}

/// A value kept so that halves sort as numbers, back as a half.
fn ordered(v: u16) -> u16 {
    if v & 0x8000 != 0 { v & 0x7fff } else { !v }
}

/// e^(x/8) of a half `x`, as a half (the logarithms perceptually linear channels were kept as).
fn exp8(x: u16) -> u16 {
    let h = half::to_f32(x) as f64;
    if !h.is_finite() {
        return 0;
    }
    if h >= 8.0 * 65504f64.ln() {
        return 0x7bff;
    }
    half::from_f32((h / 8.0).exp() as f32)
}

pub(crate) fn decompress(data: &[u8], channels: &[Channel], w: Window, size: usize) -> Result<Vec<u8>, String> {
    let mut planes: Vec<Vec<u8>> = Vec::new();
    let mut at = 0;
    for c in channels {
        let (nx, ny) = (samples(c.sampling[0], w.x0, w.x1), samples(c.sampling[1], w.y0, w.y1));
        if c.sample != Sample::Half {
            let n = nx * ny * c.sample.size();
            planes.push(data.get(at..at + n).ok_or("a B44 chunk cut short")?.to_vec());
            at += n;
            continue;
        }
        let mut plane = vec![0u16; nx * ny];
        for by in (0..ny).step_by(4) {
            for bx in (0..nx).step_by(4) {
                let flat = *data.get(at + 2).ok_or("a B44 chunk cut short")? >= 13 << 2;
                let mut s = if flat {
                    let v = ordered(u16::from_be_bytes([data[at], data[at + 1]]));
                    at += 3;
                    [v; 16]
                } else {
                    let s = unpack14(data.get(at..at + 14).ok_or("a B44 chunk cut short")?);
                    at += 14;
                    s
                };
                if c.linear {
                    s = s.map(exp8);
                }
                for dy in 0..4.min(ny - by) {
                    for dx in 0..4.min(nx - bx) {
                        plane[(by + dy) * nx + bx + dx] = s[dy * 4 + dx];
                    }
                }
            }
        }
        planes.push(plane.iter().flat_map(|v| v.to_le_bytes()).collect());
    }
    // back to scanlines
    let mut out = Vec::with_capacity(size);
    let mut rows = vec![0usize; channels.len()];
    for y in w.y0..=w.y1 {
        for (k, c) in channels.iter().enumerate() {
            if y.rem_euclid(c.sampling[1]) != 0 {
                continue;
            }
            let row = samples(c.sampling[0], w.x0, w.x1) * c.sample.size();
            out.extend_from_slice(&planes[k][rows[k]..rows[k] + row]);
            rows[k] += row;
        }
    }
    Ok(out)
}
