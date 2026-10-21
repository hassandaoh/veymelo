//! Deep parts: many samples a pixel, each with its own depth. A chunk holds a table of how many
//! samples its pixels have (each the count so far, along the chunk) and then the samples
//! (a chunk of scanlines line by line, each line channel by channel; a tile channel by channel),
//! made smaller as a flat part's are (none, RLE, ZIPS, ZIP). A deep part is read whole, and
//! flattened into planes as a compositor would: each pixel's samples from the nearest (by `Z`),
//! each laid under the ones in front by its alpha; its depth the nearest sample's.

use crate::{Compression, Exr, Kind, Plane, Reader, Sample, Window, codec, half};

/// A deep part's samples: how many each pixel has (row by row over its window), and each
/// channel's samples, every pixel's one after another.
#[derive(Clone, Debug, PartialEq)]
pub struct Deep {
    pub window: Window,
    pub counts: Vec<u32>,
    pub channels: Vec<(String, Vec<f32>)>,
}

impl Deep {
    /// Where each pixel's samples start.
    pub fn starts(&self) -> Vec<usize> {
        let mut s = Vec::with_capacity(self.counts.len() + 1);
        let mut at = 0usize;
        s.push(0);
        for &c in &self.counts {
            at += c as usize;
            s.push(at);
        }
        s
    }
}

/// The most samples read from one part (a file that says more is not read).
const MOST_SAMPLES: usize = 1 << 28;

impl Exr {
    /// Deep part `k`'s samples.
    pub fn deep(&self, bytes: &[u8], k: usize) -> Result<Deep, String> {
        let p = self.parts.get(k).ok_or("no such part")?;
        let tiled = match p.kind {
            Kind::DeepScanlines => false,
            Kind::DeepTiles => true,
            _ => return Err("not a deep part".into()),
        };
        if !matches!(p.compression, Compression::None | Compression::Rle | Compression::Zips | Compression::Zip) {
            return Err(format!("a deep part in {} compression (deep parts are none, RLE, ZIPS or ZIP)", p.compression.name()));
        }
        let d = p.data;
        let (w, h) = (d.width(), d.height());
        if w.saturating_mul(h) > crate::MOST {
            return Err(format!("a part {w} × {h} pixels is too big to read"));
        }
        let mut counts = vec![0u32; w * h];
        // each chunk's window and its samples' bytes, in the order they are laid out
        let mut found: Vec<(Window, Vec<u8>, Vec<u32>)> = Vec::new();
        for &at in &p.chunks {
            let mut r = Reader { b: bytes, at };
            if self.multipart && r.i32()? != k as i32 {
                return Err("a chunk table that points at another part's chunk".into());
            }
            let win = if tiled {
                let t = p.tiles.ok_or("a tiled part with no tiles")?;
                let (tx, ty, lx, ly) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
                if lx != 0 || ly != 0 {
                    continue;
                }
                let (x0, y0) = (d.x0 as i64 + tx as i64 * t.width as i64, d.y0 as i64 + ty as i64 * t.height as i64);
                if tx < 0 || ty < 0 || x0 > d.x1 as i64 || y0 > d.y1 as i64 {
                    return Err(format!("a tile ({tx}, {ty}) outside its part"));
                }
                Window { x0: x0 as i32, y0: y0 as i32, x1: (x0 + t.width as i64 - 1).min(d.x1 as i64) as i32, y1: (y0 + t.height as i64 - 1).min(d.y1 as i64) as i32 }
            } else {
                let y = r.i32()?;
                let lines = p.compression.lines().unwrap_or(1);
                if y < d.y0 || y > d.y1 {
                    return Err(format!("a chunk of scanlines at {y}, outside its part"));
                }
                Window { x0: d.x0, y0: y, x1: d.x1, y1: (y as i64 + lines as i64 - 1).min(d.y1 as i64) as i32 }
            };
            let (packed_table, packed_samples, unpacked) = (r.u64()? as usize, r.u64()? as usize, r.u64()? as usize);
            if unpacked > MOST_SAMPLES * 16 {
                return Err("a deep chunk bigger than is read".into());
            }
            let table_bytes = r.take(packed_table)?;
            let sample_bytes = r.take(packed_samples)?;
            let pixels = win.width() * win.height();
            let unpack = |data: &[u8], size: usize| -> Result<Vec<u8>, String> {
                if data.len() >= size || p.compression == Compression::None {
                    return Ok(data.get(..size).ok_or("a deep chunk smaller than it says")?.to_vec());
                }
                codec::decompress(p.compression, data, &p.channels, win, size)
            };
            let table: Vec<i32> = unpack(table_bytes, pixels * 4)?.as_chunks::<4>().0.iter().map(|b| i32::from_le_bytes(*b)).collect();
            // counts so far along the whole chunk, or (as some write them) along each line
            let per_line = {
                let along: i64 = table.last().copied().unwrap_or(0) as i64;
                let lines: i64 = table.chunks(win.width().max(1)).map(|l| *l.last().unwrap_or(&0) as i64).sum();
                let size: usize = p.channels.iter().map(|c| c.sample.size()).sum();
                along as usize * size != unpacked && lines as usize * size == unpacked
            };
            let mut chunk_counts = Vec::with_capacity(pixels);
            let mut prev = 0i32;
            for (i, &t) in table.iter().enumerate() {
                if per_line && i % win.width().max(1) == 0 {
                    prev = 0;
                }
                chunk_counts.push((t - prev).max(0) as u32);
                prev = t;
            }
            let samples = unpack(sample_bytes, unpacked)?;
            for (i, &c) in chunk_counts.iter().enumerate() {
                let (x, y) = (win.x0 as usize + i % win.width() - d.x0 as usize, win.y0 as usize + i / win.width() - d.y0 as usize);
                counts[y * w + x] = c;
            }
            found.push((win, samples, chunk_counts));
        }
        let total: usize = counts.iter().map(|&c| c as usize).sum();
        if total > MOST_SAMPLES {
            return Err(format!("{total} samples, more than are read"));
        }
        let mut starts = Vec::with_capacity(counts.len() + 1);
        let mut at = 0usize;
        for &c in &counts {
            starts.push(at);
            at += c as usize;
        }
        let mut channels: Vec<(String, Vec<f32>)> = p.channels.iter().map(|c| (c.name.clone(), vec![0.0; total])).collect();
        for (win, raw, chunk_counts) in &found {
            let ww = win.width();
            let value = |c: Sample, b: &[u8]| -> f32 {
                match c {
                    Sample::Half => half::to_f32(u16::from_le_bytes([b[0], b[1]])),
                    Sample::Float => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                    Sample::Uint => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
                }
            };
            let mut pos = 0usize;
            // one channel's samples over pixels `from..to` of the chunk
            let mut take = |ci: usize, from: usize, to: usize, channels: &mut Vec<(String, Vec<f32>)>| -> Result<(), String> {
                let c = p.channels[ci].sample;
                for i in from..to {
                    let (x, y) = (win.x0 as usize + i % ww - d.x0 as usize, win.y0 as usize + i / ww - d.y0 as usize);
                    let s = starts[y * w + x];
                    for k in 0..chunk_counts[i] as usize {
                        let b = raw.get(pos..pos + c.size()).ok_or("a deep chunk with fewer samples than its table says")?;
                        channels[ci].1[s + k] = value(c, b);
                        pos += c.size();
                    }
                }
                Ok(())
            };
            if tiled {
                for ci in 0..p.channels.len() {
                    take(ci, 0, chunk_counts.len(), &mut channels)?;
                }
            } else {
                for line in 0..win.height() {
                    for ci in 0..p.channels.len() {
                        take(ci, line * ww, (line + 1) * ww, &mut channels)?;
                    }
                }
            }
        }
        Ok(Deep { window: d, counts, channels })
    }
}

/// The layer a channel is of (what comes before its last dot).
fn layer_of(name: &str) -> &str {
    name.rsplit_once('.').map_or("", |(l, _)| l)
}

fn last_of(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(_, c)| c)
}

/// A deep part flattened into a plane a channel: each pixel's samples from the nearest, the
/// colours of each layer laid under those in front by that layer's alpha (`A`); depths (`Z`,
/// `ZBack`) the nearest sample's; other channels (numbers, ids) the nearest sample's too.
pub fn flatten(deep: &Deep, part: usize) -> Vec<Plane> {
    let starts = deep.starts();
    let n = deep.counts.len();
    let find = |want: &str| deep.channels.iter().position(|(c, _)| c == want);
    let z = deep.channels.iter().position(|(c, _)| last_of(c) == "Z");
    let mut out: Vec<Vec<f32>> = deep.channels.iter().map(|_| vec![0.0; n]).collect();
    // each channel's alpha: its layer's A (none: it is not a colour)
    let alpha: Vec<Option<usize>> = deep
        .channels
        .iter()
        .map(|(c, _)| {
            let last = last_of(c);
            if matches!(last, "Z" | "ZBack" | "id" | "ID") {
                return None;
            }
            let l = layer_of(c);
            find(&if l.is_empty() { "A".to_string() } else { format!("{l}.A") })
        })
        .collect();
    let mut order: Vec<usize> = Vec::new();
    for p in 0..n {
        let (a, b) = (starts[p], starts[p + 1]);
        if a == b {
            continue;
        }
        order.clear();
        order.extend(a..b);
        if let Some(zc) = z {
            let zs = &deep.channels[zc].1;
            order.sort_by(|&i, &j| zs[i].total_cmp(&zs[j]));
        }
        for (ci, (_, s)) in deep.channels.iter().enumerate() {
            match alpha[ci] {
                Some(ai) => {
                    let alphas = &deep.channels[ai].1;
                    let (mut sum, mut through) = (0.0f32, 1.0f32);
                    for &i in &order {
                        sum += through * s[i];
                        through *= 1.0 - alphas[i].clamp(0.0, 1.0);
                    }
                    out[ci][p] = sum;
                }
                None => out[ci][p] = s[order[0]],
            }
        }
    }
    deep.channels.iter().zip(out).map(|((name, _), samples)| Plane { name: name.clone(), part, window: deep.window, samples }).collect()
}
