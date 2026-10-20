//! A chunk's bytes unpacked to its pixels as the file lays them out (scanline by scanline,
//! channel by channel, each sample little-endian), for every compression read; the simple
//! ones here (RLE, ZIP, PXR24).

use crate::{Channel, Compression, Sample, Window, b44, dwa, piz, samples};

/// `data`, a chunk over `w` packed by `c`, unpacked to its `size` bytes.
pub(crate) fn decompress(c: Compression, data: &[u8], channels: &[Channel], w: Window, size: usize) -> Result<Vec<u8>, String> {
    let out = match c {
        Compression::None => data.to_vec(),
        Compression::Rle => unsplit(runs(data, size)?),
        Compression::Zips | Compression::Zip => unsplit(unzlib(data, size)?),
        Compression::Pxr24 => pxr24(&unzlib(data, size)?, channels, w, size)?,
        Compression::Piz => piz::decompress(data, channels, w, size)?,
        Compression::B44 | Compression::B44a => b44::decompress(data, channels, w, size)?,
        Compression::Dwaa | Compression::Dwab => dwa::decompress(data, channels, w, size)?,
        Compression::Other(_) => return Err(format!("{} compression is not read yet", c.name())),
    };
    if out.len() != size {
        return Err(format!("a {} chunk that unpacks to {} bytes, not {size}", c.name(), out.len()));
    }
    Ok(out)
}

/// A zlib stream inflated, as long as it comes to no more than `limit` bytes.
pub(crate) fn unzlib(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let (&cmf, &flg) = (data.first().ok_or("an empty zlib stream")?, data.get(1).ok_or("a zlib stream with no header")?);
    if cmf & 0x0f != 8 || (u16::from(cmf) << 8 | u16::from(flg)) % 31 != 0 || flg & 0x20 != 0 {
        return Err("a chunk that is not a zlib stream".into());
    }
    let body = data.get(2..data.len().saturating_sub(4)).ok_or("a zlib stream cut off")?;
    let out = veymelo_zip::inflate_up_to(body, limit)?;
    let want = data.get(data.len() - 4..).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    if want != Some(veymelo_zip::adler32(&out)) {
        return Err("a zlib stream whose check sum does not match".into());
    }
    Ok(out)
}

/// Runs of bytes undone: a count below zero, that many bytes as they are; else the next byte
/// that many and one more times.
pub(crate) fn runs(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(limit);
    let mut i = 0;
    while i < data.len() {
        let n = data[i] as i8;
        i += 1;
        if n < 0 {
            let n = -(n as i32) as usize;
            out.extend_from_slice(data.get(i..i + n).ok_or("a run cut short")?);
            i += n;
        } else {
            let b = *data.get(i).ok_or("a run cut short")?;
            out.resize(out.len() + n as usize + 1, b);
            i += 1;
        }
        if out.len() > limit {
            return Err("runs that come to more than the chunk holds".into());
        }
    }
    Ok(out)
}

/// The bytes of ZIP and RLE chunks put back: each kept as its step from the one before, the
/// evens first, then the odds.
pub(crate) fn unsplit(mut t: Vec<u8>) -> Vec<u8> {
    for i in 1..t.len() {
        t[i] = t[i - 1].wrapping_add(t[i]).wrapping_sub(128);
    }
    let half = t.len().div_ceil(2);
    let mut out = vec![0; t.len()];
    for (i, b) in t[..half].iter().enumerate() {
        out[2 * i] = *b;
    }
    for (i, b) in t[half..].iter().enumerate() {
        out[2 * i + 1] = *b;
    }
    out
}

/// PXR24: each scanline's channel kept as steps from the sample before, its bytes in planes
/// (floats cut to their top 24 bits).
fn pxr24(t: &[u8], channels: &[Channel], w: Window, size: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(size);
    let mut at = 0;
    for y in w.y0..=w.y1 {
        for c in channels {
            if y.rem_euclid(c.sampling[1]) != 0 {
                continue;
            }
            let n = samples(c.sampling[0], w.x0, w.x1);
            let planes = match c.sample {
                Sample::Uint => 4,
                Sample::Half => 2,
                Sample::Float => 3,
            };
            let b = t.get(at..at + n * planes).ok_or("a PXR24 chunk cut short")?;
            at += n * planes;
            let mut v: u32 = 0;
            for i in 0..n {
                let step = (0..planes).fold(0u32, |s, k| s << 8 | b[k * n + i] as u32);
                match c.sample {
                    Sample::Half => {
                        v = (v as u16).wrapping_add(step as u16) as u32;
                        out.extend((v as u16).to_le_bytes());
                    }
                    Sample::Uint => {
                        v = v.wrapping_add(step);
                        out.extend(v.to_le_bytes());
                    }
                    Sample::Float => {
                        v = v.wrapping_add(step << 8);
                        out.extend(v.to_le_bytes());
                    }
                }
            }
        }
    }
    Ok(out)
}
