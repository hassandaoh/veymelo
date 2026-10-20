//! PNG pictures (8-bit RGBA), our own writer: each row filtered the way that leaves the least
//! to store, then deflated.

pub use veymelo_zip::crc32;
use veymelo_zip::zlib;

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Row `row` filtered with filter `f` (0 none, 1 sub, 2 up, 3 average, 4 paeth), `up` the row above.
fn filter(f: u8, row: &[u8], up: &[u8], out: &mut Vec<u8>) {
    const BPP: usize = 4;
    out.push(f);
    for i in 0..row.len() {
        let a = if i >= BPP { row[i - BPP] } else { 0 };
        let b = up[i];
        let c = if i >= BPP { up[i - BPP] } else { 0 };
        let pred = match f {
            0 => 0,
            1 => a,
            2 => b,
            3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
            _ => paeth(a, b, c),
        };
        out.push(row[i].wrapping_sub(pred));
    }
}

/// A PNG of `width` × `height` pixels, `rgba` four bytes each, row by row from the top.
pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    assert_eq!(rgba.len(), width as usize * height as usize * 4, "four bytes a pixel");
    let stride = width as usize * 4;
    let zero = vec![0u8; stride];
    let mut filtered = Vec::with_capacity((stride + 1) * height as usize);
    let mut trial = Vec::with_capacity(stride + 1);
    for y in 0..height as usize {
        let row = &rgba[y * stride..(y + 1) * stride];
        let up = if y == 0 { &zero[..] } else { &rgba[(y - 1) * stride..y * stride] };
        // the filter whose bytes are nearest zero, as signed numbers
        let mut best = (u64::MAX, 0u8);
        for f in 0..5u8 {
            trial.clear();
            filter(f, row, up, &mut trial);
            let cost: u64 = trial[1..].iter().map(|&b| u64::from((b as i8).unsigned_abs())).sum();
            if cost < best.0 {
                best = (cost, f);
            }
        }
        filter(best.1, row, up, &mut filtered);
    }
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib(&filtered));
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pixels of a PNG this writer made, read back.
    pub fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        let (mut at, mut size, mut idat) = (8, (0, 0), Vec::new());
        while at < png.len() {
            let n = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let kind = &png[at + 4..at + 8];
            let data = &png[at + 8..at + 8 + n];
            let crc = u32::from_be_bytes(png[at + 8 + n..at + 12 + n].try_into().unwrap());
            assert_eq!(crc, crc32(&png[at + 4..at + 8 + n]), "crc of {}", String::from_utf8_lossy(kind));
            match kind {
                b"IHDR" => size = (u32::from_be_bytes(data[..4].try_into().unwrap()), u32::from_be_bytes(data[4..8].try_into().unwrap())),
                b"IDAT" => idat.extend_from_slice(data),
                _ => {}
            }
            at += 12 + n;
        }
        let raw = veymelo_zip::inflate(&idat[2..idat.len() - 4]).unwrap();
        let stride = size.0 as usize * 4;
        let mut out = vec![0u8; stride * size.1 as usize];
        for y in 0..size.1 as usize {
            let f = raw[y * (stride + 1)];
            for i in 0..stride {
                let x = raw[y * (stride + 1) + 1 + i];
                let a = if i >= 4 { out[y * stride + i - 4] } else { 0 };
                let b = if y > 0 { out[(y - 1) * stride + i] } else { 0 };
                let c = if i >= 4 && y > 0 { out[(y - 1) * stride + i - 4] } else { 0 };
                let pred = match f {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                    _ => paeth(a, b, c),
                };
                out[y * stride + i] = x.wrapping_add(pred);
            }
        }
        (size.0, size.1, out)
    }

    #[test]
    fn crc_of_a_known_text() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn pixels_come_back() {
        let (w, h) = (37u32, 23u32);
        let px: Vec<u8> = (0..w * h * 4).map(|i| ((i * 7919) % 251) as u8).collect();
        assert_eq!(decode(&encode(w, h, &px)), (w, h, px));
        let flat = vec![255u8; 1920 * 1080 * 4];
        let png = encode(1920, 1080, &flat);
        assert!(png.len() < 20_000, "a flat picture is small: {}", png.len());
        assert_eq!(decode(&png).2, flat);
    }
}
