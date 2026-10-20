//! Animated GIF (GIF89a), written by our own code.
//!
//! Each frame gets its own palette of up to 256 colours, found by cutting the frame's colours
//! into boxes at their widest spread (median cut), the picture's colours then spread between
//! the palette's by error diffusion so gradients do not band. Pixels mostly see-through become
//! the one see-through colour. After the first frame only the rectangle that changed is
//! written (where nothing is see-through), so a still background costs nothing. A frame's time
//! is kept in hundredths of a second, rounded so the frames stay on time.

/// A GIF being written.
pub struct Gif {
    out: Vec<u8>,
    w: usize,
    h: usize,
    fps: f64,
    frames: u64,
    /// The previous frame's colours as shown (for writing only what changed).
    shown: Option<Vec<[u8; 4]>>,
    dither: bool,
}

impl Gif {
    /// A GIF `w` × `h` at `fps` frames a second, playing round and round (`looped`), its colours
    /// spread by error diffusion (`dither`).
    pub fn new(w: usize, h: usize, fps: f64, looped: bool, dither: bool) -> Gif {
        let mut out = Vec::new();
        out.extend(b"GIF89a");
        out.extend((w as u16).to_le_bytes());
        out.extend((h as u16).to_le_bytes());
        // no global colour table; 8 bits a colour
        out.extend([0x70, 0, 0]);
        if looped {
            out.extend([0x21, 0xFF, 0x0B]);
            out.extend(b"NETSCAPE2.0");
            out.extend([0x03, 0x01, 0x00, 0x00, 0x00]);
        }
        Gif { out, w, h, fps, frames: 0, shown: None, dither }
    }

    /// A frame of straight (not premultiplied) eight-bit RGBA.
    pub fn frame(&mut self, rgba: &[u8]) {
        let (w, h) = (self.w, self.h);
        let px: Vec<[u8; 4]> = rgba.as_chunks::<4>().0.to_vec();
        let clear = px.iter().any(|p| p[3] < 128);
        // the rectangle that changed (all of it while anything is see-through)
        let (x0, y0, x1, y1) = match (&self.shown, clear) {
            (Some(prev), false) => {
                let mut r = (w, h, 0, 0);
                for y in 0..h {
                    for x in 0..w {
                        let (a, b) = (px[y * w + x], prev[y * w + x]);
                        if a[3] >= 128 && (a[0] != b[0] || a[1] != b[1] || a[2] != b[2]) || (a[3] >= 128) != (b[3] >= 128) {
                            r = (r.0.min(x), r.1.min(y), r.2.max(x + 1), r.3.max(y + 1));
                        }
                    }
                }
                if r.0 >= r.2 { (0, 0, 1, 1) } else { r }
            }
            _ => (0, 0, w, h),
        };
        let (rw, rh) = (x1 - x0, y1 - y0);
        let part: Vec<[u8; 4]> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| px[y * w + x]).collect();
        let palette = median_cut(&part, if clear { 255 } else { 256 });
        let indices = map(&part, rw, rh, &palette, clear, self.dither);
        // shown from now on: this frame's picture where it was written
        let mut shown = self.shown.take().unwrap_or_else(|| vec![[0; 4]; w * h]);
        if clear {
            shown = px.clone();
        } else {
            for y in y0..y1 {
                for x in x0..x1 {
                    shown[y * w + x] = px[y * w + x];
                }
            }
        }
        self.shown = Some(shown);
        // how long it shows, in hundredths of a second, kept on time
        let at = |k: u64| (k as f64 * 100.0 / self.fps).round() as u64;
        let delay = (at(self.frames + 1) - at(self.frames)).clamp(1, 65535) as u16;
        self.frames += 1;
        // graphic control: its time, the see-through colour, and what becomes of it after
        let transparent = clear as usize * palette.len();
        let disposal: u8 = if clear { 2 } else { 1 };
        self.out.extend([0x21, 0xF9, 0x04, (disposal << 2) | clear as u8]);
        self.out.extend(delay.to_le_bytes());
        self.out.extend([transparent as u8, 0]);
        // the image, with its own colour table
        let colours = palette.len() + clear as usize;
        let bits = (usize::BITS - (colours.max(2) - 1).leading_zeros()).max(1) as u8;
        self.out.push(0x2C);
        for v in [x0, y0, rw, rh] {
            self.out.extend((v as u16).to_le_bytes());
        }
        self.out.push(0x80 | (bits - 1));
        for k in 0..1usize << bits {
            let c = palette.get(k).copied().unwrap_or([0; 3]);
            self.out.extend(c);
        }
        let min = bits.max(2);
        self.out.push(min);
        let data = lzw(&indices, min);
        for block in data.chunks(255) {
            self.out.push(block.len() as u8);
            self.out.extend(block);
        }
        self.out.push(0);
    }

    /// The file, finished.
    pub fn finish(mut self) -> Vec<u8> {
        self.out.push(0x3B);
        self.out
    }
}

/// Up to `most` colours standing for those of the (mostly solid) pixels: the set split again
/// and again at the middle of its widest spread, each box its mean.
fn median_cut(px: &[[u8; 4]], most: usize) -> Vec<[u8; 3]> {
    let mut colours: Vec<[u8; 3]> = px.iter().filter(|p| p[3] >= 128).map(|p| [p[0], p[1], p[2]]).collect();
    if colours.is_empty() {
        return vec![[0; 3]];
    }
    // few enough already: each its own
    colours.sort_unstable();
    let mut distinct = colours.clone();
    distinct.dedup();
    if distinct.len() <= most {
        return distinct;
    }
    let mut boxes: Vec<Vec<[u8; 3]>> = vec![colours];
    while boxes.len() < most {
        // the box with the widest spread (weighted by how many it holds) is split
        let spread = |b: &Vec<[u8; 3]>| -> (usize, u8) {
            let (lo, hi) = b.iter().fold(([255u8; 3], [0u8; 3]), |(lo, hi), c| ([lo[0].min(c[0]), lo[1].min(c[1]), lo[2].min(c[2])], [hi[0].max(c[0]), hi[1].max(c[1]), hi[2].max(c[2])]));
            let r = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
            let k = (0..3).max_by_key(|&k| r[k]).unwrap_or(0);
            (k, r[k])
        };
        let Some((i, (axis, width))) = boxes.iter().enumerate().map(|(i, b)| (i, spread(b))).max_by_key(|(i, (_, wd))| *wd as usize * boxes[*i].len().min(1 << 16)) else { break };
        if width == 0 {
            break;
        }
        let mut b = boxes.swap_remove(i);
        b.sort_unstable_by_key(|c| c[axis]);
        let half = b.split_off(b.len() / 2);
        boxes.push(b);
        boxes.push(half);
    }
    boxes
        .iter()
        .filter(|b| !b.is_empty())
        .map(|b| {
            let n = b.len() as u64;
            let s = b.iter().fold([0u64; 3], |s, c| [s[0] + c[0] as u64, s[1] + c[1] as u64, s[2] + c[2] as u64]);
            [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8]
        })
        .collect()
}

/// Each pixel's palette colour (the see-through one, after the palette, where mostly clear),
/// the errors spread to the neighbours still to come (Floyd and Steinberg).
fn map(px: &[[u8; 4]], w: usize, h: usize, palette: &[[u8; 3]], clear: bool, dither: bool) -> Vec<u8> {
    let mut cache: Vec<u16> = vec![u16::MAX; 32 * 32 * 32];
    let mut nearest = |c: [i32; 3]| -> usize {
        let c = c.map(|v| v.clamp(0, 255));
        let key = ((c[0] >> 3) << 10 | (c[1] >> 3) << 5 | (c[2] >> 3)) as usize;
        if cache[key] != u16::MAX && !dither {
            return cache[key] as usize;
        }
        let best = palette
            .iter()
            .enumerate()
            .min_by_key(|(_, p)| {
                let d = [p[0] as i32 - c[0], p[1] as i32 - c[1], p[2] as i32 - c[2]];
                // weighed as the eye sees the differences
                2 * d[0] * d[0] + 4 * d[1] * d[1] + 3 * d[2] * d[2]
            })
            .map_or(0, |(i, _)| i);
        cache[key] = best as u16;
        best
    };
    let mut err = vec![[0i32; 3]; (w + 2) * 2];
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let (cur, next) = (y % 2, (y + 1) % 2);
        for e in &mut err[next * (w + 2)..(next + 1) * (w + 2)] {
            *e = [0; 3];
        }
        for x in 0..w {
            let p = px[y * w + x];
            if clear && p[3] < 128 {
                out[y * w + x] = palette.len() as u8;
                continue;
            }
            let e = err[cur * (w + 2) + x + 1];
            let want = [p[0] as i32 + e[0] / 16, p[1] as i32 + e[1] / 16, p[2] as i32 + e[2] / 16];
            let k = nearest(want);
            out[y * w + x] = k as u8;
            if dither {
                let got = palette[k];
                let d = [want[0] - got[0] as i32, want[1] - got[1] as i32, want[2] - got[2] as i32].map(|v| v.clamp(-255, 255));
                for (dx, row, k) in [(1i64, cur, 7), (-1, next, 3), (0, next, 5), (1, next, 1)] {
                    let i = row * (w + 2) + (x as i64 + 1 + dx) as usize;
                    for c in 0..3 {
                        err[i][c] += d[c] * k;
                    }
                }
            }
        }
    }
    out
}

/// The indices packed by LZW, codes starting `min + 1` bits wide, least significant bits
/// first, the table started again when it is full.
fn lzw(indices: &[u8], min: u8) -> Vec<u8> {
    let clear = 1u32 << min;
    let end = clear + 1;
    let mut out = Vec::new();
    let (mut acc, mut nbits) = (0u64, 0u32);
    let mut width = min as u32 + 1;
    let mut emit = |code: u32, width: u32, out: &mut Vec<u8>| {
        acc |= (code as u64) << nbits;
        nbits += width;
        while nbits >= 8 {
            out.push(acc as u8);
            acc >>= 8;
            nbits -= 8;
        }
    };
    // codes for strings: (prefix code, next index) → code
    let mut table: std::collections::HashMap<(u32, u8), u32> = std::collections::HashMap::new();
    let mut next = end + 1;
    emit(clear, width, &mut out);
    let mut it = indices.iter();
    let Some(&first) = it.next() else {
        emit(end, width, &mut out);
        if nbits > 0 {
            out.push(acc as u8);
        }
        return out;
    };
    let mut prefix = first as u32;
    for &k in it {
        if let Some(&code) = table.get(&(prefix, k)) {
            prefix = code;
            continue;
        }
        emit(prefix, width, &mut out);
        if next < 4096 {
            table.insert((prefix, k), next);
            next += 1;
            if next > (1 << width) && width < 12 {
                width += 1;
            }
        } else {
            emit(clear, width, &mut out);
            table.clear();
            next = end + 1;
            width = min as u32 + 1;
        }
        prefix = k as u32;
    }
    emit(prefix, width, &mut out);
    emit(end, width, &mut out);
    if nbits > 0 {
        out.push(acc as u8);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The indices back out of LZW data (a reader of our own, for the tests).
    fn unlzw(data: &[u8], min: u8) -> Vec<u8> {
        let clear = 1u32 << min;
        let end = clear + 1;
        let (mut acc, mut nbits, mut pos) = (0u64, 0u32, 0usize);
        let mut width = min as u32 + 1;
        let mut table: Vec<Vec<u8>> = (0..clear).map(|i| vec![i as u8]).chain([vec![], vec![]]).collect();
        let mut prev: Option<Vec<u8>> = None;
        let mut out = Vec::new();
        loop {
            while nbits < width && pos < data.len() {
                acc |= (data[pos] as u64) << nbits;
                pos += 1;
                nbits += 8;
            }
            if nbits < width {
                break;
            }
            let code = (acc & ((1 << width) - 1)) as u32;
            acc >>= width;
            nbits -= width;
            if code == clear {
                table.truncate(end as usize + 1);
                width = min as u32 + 1;
                prev = None;
                continue;
            }
            if code == end {
                break;
            }
            let entry = if (code as usize) < table.len() {
                table[code as usize].clone()
            } else {
                let mut p = prev.clone().unwrap();
                p.push(p[0]);
                p
            };
            out.extend(&entry);
            if let Some(mut p) = prev.take() {
                p.push(entry[0]);
                table.push(p);
                if table.len() == (1 << width) && width < 12 {
                    width += 1;
                }
            }
            prev = Some(entry);
        }
        out
    }

    #[test]
    fn lzw_comes_back() {
        let mut data: Vec<u8> = (0..20_000).map(|i| ((i * 7 + i / 13) % 5) as u8).collect();
        data.extend(std::iter::repeat_n(3u8, 9000));
        assert_eq!(unlzw(&lzw(&data, 3), 3), data);
        let wide: Vec<u8> = (0..50_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        assert_eq!(unlzw(&lzw(&wide, 8), 8), wide, "past a full table");
    }

    #[test]
    fn few_colours_are_kept_exactly_and_many_are_cut() {
        let px: Vec<[u8; 4]> = (0..100).map(|i| [(i % 3) as u8 * 100, 50, 0, 255]).collect();
        assert_eq!(median_cut(&px, 256).len(), 3);
        let many: Vec<[u8; 4]> = (0..65_536u32).map(|i| [(i & 255) as u8, (i >> 8) as u8, 128, 255]).collect();
        let p = median_cut(&many, 256);
        assert_eq!(p.len(), 256);
    }

    #[test]
    fn a_gif_is_whole_and_timed() {
        let (w, h) = (16, 8);
        let mut g = Gif::new(w, h, 30.0, true, true);
        for f in 0..3u8 {
            let rgba: Vec<u8> = (0..w * h).flat_map(|i| [(i * 16) as u8, f * 80, 40, if i % 7 == 0 { 0 } else { 255 }]).collect();
            g.frame(&rgba);
        }
        let file = g.finish();
        assert_eq!(&file[..6], b"GIF89a");
        assert_eq!(*file.last().unwrap(), 0x3B);
        // three frames' controls; 30 a second is 3, 4, 3 hundredths (on time)
        let delays: Vec<u16> = file.windows(4).enumerate().filter(|(_, w)| w[0] == 0x21 && w[1] == 0xF9 && w[2] == 4).map(|(i, _)| u16::from_le_bytes([file[i + 4], file[i + 5]])).collect();
        assert_eq!(delays, vec![3, 4, 3]);
    }
}
