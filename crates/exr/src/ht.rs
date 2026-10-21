//! OpenEXR's HTJ2K compressions (HTJ2K256 and HTJ2K32, 256 or 32 scanlines a chunk): a chunk is
//! a short header — "HT", its length, and which channel each of the codestream's components is
//! (red, green and blue first, so the colour transform can take them) — then a JPEG 2000
//! codestream of HT code-blocks (Part 15), lossless. The components decoded are laid out as a
//! flat chunk is, line by line, channel by channel: halves as the 16 bits each component holds,
//! floats and whole numbers as 32.

mod block;
mod stream;
mod tables;

use crate::{Channel, Sample, Window, samples};

/// A chunk over `w` unpacked to its pixels' `size` bytes.
pub(crate) fn decompress(data: &[u8], channels: &[Channel], w: Window, size: usize) -> Result<Vec<u8>, String> {
    if data.len() < 6 || &data[..2] != b"HT" {
        return Err("an HTJ2K chunk without its header".into());
    }
    let plen = u32::from_be_bytes([data[2], data[3], data[4], data[5]]) as usize;
    let head = data.get(6..6 + plen).ok_or("an HTJ2K chunk header cut short")?;
    let n = u16::from_be_bytes([*head.first().unwrap_or(&0), *head.get(1).unwrap_or(&0)]) as usize;
    if n != channels.len() {
        return Err(format!("an HTJ2K chunk of {n} components for {} channels", channels.len()));
    }
    let to_file: Vec<usize> = (0..n).map(|k| u16::from_be_bytes([head[2 + 2 * k], head[3 + 2 * k]]) as usize).collect();
    if to_file.iter().any(|&f| f >= n) {
        return Err("an HTJ2K chunk whose channel map points nowhere".into());
    }
    let planes = stream::Codestream::read(&data[6 + plen..])?.decode()?;
    if planes.len() != n {
        return Err("an HTJ2K codestream with another number of components".into());
    }
    // each file channel's plane
    let mut of_file = vec![0usize; n];
    for (k, &f) in to_file.iter().enumerate() {
        of_file[f] = k;
    }
    let mut out = Vec::with_capacity(size);
    let mut rows = vec![0usize; n];
    for y in w.y0..=w.y1 {
        for (f, c) in channels.iter().enumerate() {
            if y.rem_euclid(c.sampling[1]) != 0 {
                continue;
            }
            let count = samples(c.sampling[0], w.x0, w.x1);
            let p = &planes[of_file[f]];
            let row = rows[f];
            rows[f] += 1;
            for x in 0..count {
                let v = if x < p.width && row < p.height { p.samples[row * p.width + x] } else { 0 };
                match c.sample {
                    Sample::Half => out.extend((v as i16).to_le_bytes()),
                    Sample::Float => out.extend((v as i32).to_le_bytes()),
                    Sample::Uint => out.extend((v as u32).to_le_bytes()),
                }
            }
        }
    }
    Ok(out)
}
