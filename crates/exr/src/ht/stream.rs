//! A JPEG 2000 codestream of HT code-blocks (Part 15) decoded to its components' whole numbers:
//! the main header (SIZ, COD/COC, QCD/QCC, NLT; the rest passed over), its tiles' parts, each
//! tile's packets in the progression its COD names (their headers' tag trees, pass counts and
//! lengths, then the code-blocks' bytes), each code-block's cleanup pass, the subbands' samples
//! put back through the reversible 5/3 wavelet, then the reversible colour transform, the DC
//! level shift and the binary-complement non-linearity.

use super::block;

#[derive(Clone, Copy, Debug)]
struct Comp {
    bits: u32,
    signed: bool,
    dx: u32,
    dy: u32,
}

#[derive(Clone, Debug)]
struct Cod {
    order: u8,
    layers: u32,
    mct: bool,
    levels: u32,
    xcb: u32,
    ycb: u32,
    reversible: bool,
    /// Each resolution's precinct size exponents (from the lowest).
    precincts: Vec<(u32, u32)>,
    sop: bool,
    eph: bool,
}

#[derive(Clone, Debug)]
struct Qcd {
    guard: u32,
    /// Each subband's exponent, LL first, then HL, LH, HH from the deepest level up.
    exps: Vec<u32>,
}

/// A decoded component: its size and samples, row by row.
pub(crate) struct Plane {
    pub width: usize,
    pub height: usize,
    pub samples: Vec<i64>,
}

fn be16(d: &[u8], at: usize) -> u32 {
    d.get(at..at + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]) as u32)
}

fn be32(d: &[u8], at: usize) -> u32 {
    d.get(at..at + 4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn ceil_div(a: i64, b: i64) -> i64 {
    -((-a).div_euclid(b))
}

fn read_cod(b: &[u8], with_scod: bool) -> Cod {
    // COD: Scod, then progression, layers, MCT; COC: its component's Scoc first
    let scod = if with_scod { b[0] } else { 0 };
    let (order, layers, mct) = (b[1], be16(b, 2), b[4] != 0);
    let s = 5;
    let levels = b[s] as u32;
    let mut precincts = vec![(15, 15); levels as usize + 1];
    if scod & 1 != 0 {
        for (r, slot) in precincts.iter_mut().enumerate() {
            let v = *b.get(s + 5 + r).unwrap_or(&0xFF);
            *slot = ((v & 0xF) as u32, (v >> 4) as u32);
        }
    }
    Cod { order, layers, mct, levels, xcb: b[s + 1] as u32 + 2, ycb: b[s + 2] as u32 + 2, reversible: b[s + 4] == 1, precincts, sop: scod & 2 != 0, eph: scod & 4 != 0 }
}

fn read_qcd(b: &[u8]) -> Qcd {
    let s = b[0];
    let guard = (s >> 5) as u32;
    let exps = match s & 0x1F {
        0 => b[1..].iter().map(|v| (v >> 3) as u32).collect(),
        1 => vec![be16(b, 1) >> 11],
        _ => b[1..].chunks(2).map(|c| (u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) >> 11) as u32).collect(),
    };
    Qcd { guard, exps }
}

/// A code-block: where it is in its subband, what is known of it, its bytes.
#[derive(Clone, Default)]
struct Block {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
    included: bool,
    missing: u32,
    lblock: u32,
    passes: u32,
    len1: usize,
    len2: usize,
    data: Vec<u8>,
}

/// A tag tree over a grid of code-blocks.
struct TagTree {
    levels: Vec<(usize, usize)>,
    value: Vec<Vec<u32>>,
    low: Vec<Vec<u32>>,
}

impl TagTree {
    fn new(w: usize, h: usize) -> TagTree {
        let mut levels = vec![(w.max(1), h.max(1))];
        while levels.last().is_some_and(|&(a, b)| a > 1 || b > 1) {
            let (a, b) = *levels.last().expect("a level");
            levels.push((a.div_ceil(2), b.div_ceil(2)));
        }
        let value = levels.iter().map(|&(a, b)| vec![u32::MAX; a * b]).collect();
        let low = levels.iter().map(|&(a, b)| vec![0; a * b]).collect();
        TagTree { levels, value, low }
    }

    /// Whether leaf (`x`, `y`)'s value is below `threshold`, reading as few bits as that needs.
    fn below(&mut self, x: usize, y: usize, threshold: u32, bits: &mut HeaderBits) -> Result<bool, String> {
        let mut low = 0;
        for l in (0..self.levels.len()).rev() {
            let (w, _) = self.levels[l];
            let i = (y >> l) * w + (x >> l);
            if low > self.low[l][i] {
                self.low[l][i] = low;
            } else {
                low = self.low[l][i];
            }
            while low < threshold && low < self.value[l][i] {
                if bits.bit()? == 1 {
                    self.value[l][i] = low;
                } else {
                    low += 1;
                }
            }
            self.low[l][i] = low;
        }
        Ok(self.value[0][y * self.levels[0].0 + x] < threshold)
    }

    /// Leaf (`x`, `y`)'s whole value.
    fn value(&mut self, x: usize, y: usize, bits: &mut HeaderBits) -> Result<u32, String> {
        let mut t = 1;
        while !self.below(x, y, t, bits)? {
            t += 1;
            if t > 64 {
                return Err("a tag tree that never ends".into());
            }
        }
        Ok(self.value[0][y * self.levels[0].0 + x])
    }
}

/// A packet header's bits: from the top of each byte, a byte after one of all ones giving seven.
struct HeaderBits<'a> {
    d: &'a [u8],
    pos: usize,
    byte: u8,
    left: u32,
    last_ff: bool,
}

impl HeaderBits<'_> {
    fn bit(&mut self) -> Result<u32, String> {
        if self.left == 0 {
            let b = *self.d.get(self.pos).ok_or("a packet header cut short")?;
            self.pos += 1;
            self.left = if self.last_ff { 7 } else { 8 };
            self.last_ff = b == 0xFF;
            self.byte = b;
        }
        self.left -= 1;
        Ok((self.byte >> self.left) as u32 & 1)
    }

    fn bits(&mut self, n: u32) -> Result<u32, String> {
        let mut v = 0;
        for _ in 0..n {
            v = v << 1 | self.bit()?;
        }
        Ok(v)
    }

    /// The header done: the place after it (past a byte stuffed after one of all ones).
    fn end(&self) -> usize {
        if self.last_ff { self.pos + 1 } else { self.pos }
    }
}

/// A subband of a resolution: its rectangle, its exponent's place, its samples.
struct Band {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
    /// Its exponent's place in the quantization list.
    q: usize,
    samples: Vec<i64>,
}

/// A precinct's share of a band: its code-blocks (a grid) and their tag trees.
struct Share {
    w: usize,
    h: usize,
    blocks: Vec<Block>,
    inclusion: TagTree,
    zeros: TagTree,
}

struct Resolution {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
    bands: Vec<Band>,
    /// Precincts across and down, and each precinct's shares of the bands.
    pw: usize,
    ph: usize,
    precincts: Vec<Vec<Share>>,
    ppx: u32,
    ppy: u32,
}

struct TileComp {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
    res: Vec<Resolution>,
}

pub(crate) struct Codestream {
    width: usize,
    height: usize,
    origin: (i64, i64),
    tile: (i64, i64, i64, i64),
    comps: Vec<Comp>,
    cod: Cod,
    cocs: Vec<Option<Cod>>,
    qcd: Qcd,
    qccs: Vec<Option<Qcd>>,
    /// Each component's non-linearity (3: binary complement).
    nlt: Vec<u8>,
    /// Each tile's bytes (its parts' data, in order).
    tiles: Vec<Vec<u8>>,
}

impl Codestream {
    pub(crate) fn read(d: &[u8]) -> Result<Codestream, String> {
        if be16(d, 0) != 0xFF4F {
            return Err("an HT chunk whose codestream does not start".into());
        }
        let mut at = 2;
        let mut siz = None;
        let mut cod = None;
        let mut qcd = None;
        let mut cocs: Vec<Option<Cod>> = Vec::new();
        let mut qccs: Vec<Option<Qcd>> = Vec::new();
        let mut nlt: Vec<u8> = Vec::new();
        let mut tiles: Vec<Vec<u8>> = Vec::new();
        loop {
            let m = be16(d, at);
            if m == 0xFFD9 || at + 4 > d.len() {
                break;
            }
            let len = be16(d, at + 2) as usize;
            let body = d.get(at + 4..at + 2 + len).ok_or("a marker cut short")?;
            match m {
                0xFF51 => {
                    let n = be16(body, 34) as usize;
                    let comps: Vec<Comp> = (0..n).map(|i| {
                        let s = body[36 + 3 * i];
                        Comp { bits: (s & 0x7F) as u32 + 1, signed: s & 0x80 != 0, dx: body[37 + 3 * i].max(1) as u32, dy: body[38 + 3 * i].max(1) as u32 }
                    }).collect();
                    cocs = vec![None; n];
                    qccs = vec![None; n];
                    nlt = vec![0; n];
                    siz = Some((be32(body, 2), be32(body, 6), be32(body, 10), be32(body, 14), be32(body, 18), be32(body, 22), be32(body, 26), be32(body, 30), comps));
                }
                0xFF52 => cod = Some(read_cod(body, true)),
                0xFF53 => {
                    let wide = cocs.len() > 256;
                    let c = if wide { be16(body, 0) as usize } else { body[0] as usize };
                    let rest = &body[if wide { 2 } else { 1 }..];
                    // Scoc then the SPcoc as COD's from its levels
                    let mut b = vec![rest[0], 0, 0, 1, 0];
                    b.extend(&rest[1..]);
                    if let Some(slot) = cocs.get_mut(c) {
                        *slot = Some(read_cod(&b, true));
                    }
                }
                0xFF5C => qcd = Some(read_qcd(body)),
                0xFF5D => {
                    let wide = qccs.len() > 256;
                    let c = if wide { be16(body, 0) as usize } else { body[0] as usize };
                    if let Some(slot) = qccs.get_mut(c) {
                        *slot = Some(read_qcd(&body[if wide { 2 } else { 1 }..]));
                    }
                }
                0xFF76 => {
                    let c = be16(body, 0) as usize;
                    let t = body.get(3).copied().unwrap_or(0);
                    if c == 0xFFFF {
                        nlt.iter_mut().for_each(|v| *v = t);
                    } else if let Some(slot) = nlt.get_mut(c) {
                        *slot = t;
                    }
                }
                0xFF90 => {
                    // a tile-part: its tile, its length, its data after SOD
                    let (index, psot) = (be16(body, 0) as usize, be32(body, 2) as usize);
                    let end = if psot == 0 { d.len().saturating_sub(2) } else { (at + psot).min(d.len()) };
                    let mut p = at + 2 + len;
                    while p + 2 <= end && be16(d, p) != 0xFF93 {
                        p += 2 + be16(d, p + 2) as usize;
                    }
                    if tiles.len() <= index {
                        tiles.resize(index + 1, Vec::new());
                    }
                    tiles[index].extend(d.get(p + 2..end).unwrap_or(&[]));
                    at = end;
                    continue;
                }
                _ => {}
            }
            at += 2 + len;
        }
        let (xsiz, ysiz, xo, yo, xt, yt, xto, yto, comps) = siz.ok_or("a codestream with no SIZ")?;
        let cod = cod.ok_or("a codestream with no COD")?;
        let qcd = qcd.ok_or("a codestream with no QCD")?;
        if !cod.reversible {
            return Err("an HTJ2K chunk coded lossily (the 9/7 wavelet), which is not read yet".into());
        }
        Ok(Codestream { width: (xsiz - xo) as usize, height: (ysiz - yo) as usize, origin: (xo as i64, yo as i64), tile: (xt as i64, yt as i64, xto as i64, yto as i64), comps, cod, cocs, qcd, qccs, nlt, tiles })
    }

    fn cod_of(&self, c: usize) -> &Cod {
        self.cocs.get(c).and_then(|x| x.as_ref()).unwrap_or(&self.cod)
    }

    fn qcd_of(&self, c: usize) -> &Qcd {
        self.qccs.get(c).and_then(|x| x.as_ref()).unwrap_or(&self.qcd)
    }

    /// Every component's samples.
    pub(crate) fn decode(&self) -> Result<Vec<Plane>, String> {
        let (xo, yo) = self.origin;
        let (xsiz, ysiz) = (xo + self.width as i64, yo + self.height as i64);
        let mut planes: Vec<Plane> = self
            .comps
            .iter()
            .map(|c| {
                let (w, h) = ((ceil_div(xsiz, c.dx as i64) - ceil_div(xo, c.dx as i64)) as usize, (ceil_div(ysiz, c.dy as i64) - ceil_div(yo, c.dy as i64)) as usize);
                Plane { width: w, height: h, samples: vec![0; w * h] }
            })
            .collect();
        let (xt, yt, xto, yto) = self.tile;
        let across = ceil_div(xsiz - xto, xt.max(1)).max(1) as usize;
        for (t, data) in self.tiles.iter().enumerate() {
            let (p, q) = ((t % across) as i64, (t / across) as i64);
            let tx0 = (xto + p * xt).max(xo);
            let ty0 = (yto + q * yt).max(yo);
            let tx1 = (xto + (p + 1) * xt).min(xsiz);
            let ty1 = (yto + (q + 1) * yt).min(ysiz);
            let tile = self.tile_decode(data, (tx0, ty0, tx1, ty1))?;
            for (c, (tc, samples)) in tile.into_iter().enumerate() {
                let comp = self.comps[c];
                let pl = &mut planes[c];
                let (cx0, cy0) = (ceil_div(xo, comp.dx as i64), ceil_div(yo, comp.dy as i64));
                let w = (tc.x1 - tc.x0) as usize;
                for y in tc.y0..tc.y1 {
                    for x in tc.x0..tc.x1 {
                        let (px, py) = ((x - cx0) as usize, (y - cy0) as usize);
                        if px < pl.width && py < pl.height {
                            pl.samples[py * pl.width + px] = samples[(y - tc.y0) as usize * w + (x - tc.x0) as usize];
                        }
                    }
                }
            }
        }
        Ok(planes)
    }

    /// A tile's components: each tile-component (with its rectangle) and its samples.
    fn tile_decode(&self, data: &[u8], (tx0, ty0, tx1, ty1): (i64, i64, i64, i64)) -> Result<Vec<(TileComp, Vec<i64>)>, String> {
        let mut tcs: Vec<TileComp> = Vec::new();
        for (c, comp) in self.comps.iter().enumerate() {
            let cod = self.cod_of(c);
            let (dx, dy) = (comp.dx as i64, comp.dy as i64);
            let (x0, y0, x1, y1) = (ceil_div(tx0, dx), ceil_div(ty0, dy), ceil_div(tx1, dx), ceil_div(ty1, dy));
            let nl = cod.levels;
            let mut res = Vec::new();
            for r in 0..=nl {
                let s = 1i64 << (nl - r);
                let (rx0, ry0, rx1, ry1) = (ceil_div(x0, s), ceil_div(y0, s), ceil_div(x1, s), ceil_div(y1, s));
                let (ppx, ppy) = cod.precincts.get(r as usize).copied().unwrap_or((15, 15));
                let kinds: &[usize] = if r == 0 { &[0] } else { &[1, 2, 3] };
                let nb = if r == 0 { nl } else { nl - r + 1 };
                let mut bands = Vec::new();
                for &kind in kinds {
                    let (xob, yob) = match kind {
                        1 => (1, 0),
                        2 => (0, 1),
                        3 => (1, 1),
                        _ => (0, 0),
                    };
                    let half = if nb == 0 { 0 } else { 1i64 << (nb - 1) };
                    let sb = 1i64 << nb;
                    let (bx0, by0, bx1, by1) = (ceil_div(x0 - half * xob, sb), ceil_div(y0 - half * yob, sb), ceil_div(x1 - half * xob, sb), ceil_div(y1 - half * yob, sb));
                    let q = if r == 0 { 0 } else { 1 + 3 * (r as usize - 1) + (kind - 1) };
                    bands.push(Band { x0: bx0, y0: by0, x1: bx1, y1: by1, q, samples: vec![0; ((bx1 - bx0).max(0) * (by1 - by0).max(0)) as usize] });
                }
                // precincts over the resolution, and each one's code-blocks in each band
                let (pw, ph) = if rx1 > rx0 && ry1 > ry0 {
                    ((ceil_div(rx1, 1 << ppx) - (rx0 >> ppx)) as usize, (ceil_div(ry1, 1 << ppy) - (ry0 >> ppy)) as usize)
                } else {
                    (0, 0)
                };
                let (bpx, bpy) = if r == 0 { (ppx, ppy) } else { (ppx.saturating_sub(1), ppy.saturating_sub(1)) };
                let (xcb, ycb) = (cod.xcb.min(bpx), cod.ycb.min(bpy));
                let mut precincts = Vec::with_capacity(pw * ph);
                for k in 0..pw * ph {
                    let (kx, ky) = ((k % pw) as i64, (k / pw) as i64);
                    let mut shares = Vec::new();
                    for b in &bands {
                        // the precinct on the band's grid
                        let (px0, py0) = (((rx0 >> ppx) + kx) << bpx, ((ry0 >> ppy) + ky) << bpy);
                        let (px1, py1) = (px0 + (1 << bpx), py0 + (1 << bpy));
                        let (ax0, ay0, ax1, ay1) = (px0.max(b.x0), py0.max(b.y0), px1.min(b.x1), py1.min(b.y1));
                        let (cw, ch) = if ax1 > ax0 && ay1 > ay0 { ((ceil_div(ax1, 1 << xcb) - (ax0 >> xcb)) as usize, (ceil_div(ay1, 1 << ycb) - (ay0 >> ycb)) as usize) } else { (0, 0) };
                        let mut blocks = Vec::with_capacity(cw * ch);
                        for j in 0..ch {
                            for i in 0..cw {
                                let (cx0, cy0) = (((ax0 >> xcb) + i as i64) << xcb, ((ay0 >> ycb) + j as i64) << ycb);
                                blocks.push(Block { x0: cx0.max(ax0), y0: cy0.max(ay0), x1: (cx0 + (1 << xcb)).min(ax1), y1: (cy0 + (1 << ycb)).min(ay1), lblock: 3, ..Block::default() });
                            }
                        }
                        shares.push(Share { w: cw, h: ch, blocks, inclusion: TagTree::new(cw, ch), zeros: TagTree::new(cw, ch) });
                    }
                    precincts.push(shares);
                }
                res.push(Resolution { x0: rx0, y0: ry0, x1: rx1, y1: ry1, bands, pw, ph, precincts, ppx, ppy });
            }
            tcs.push(TileComp { x0, y0, x1, y1, res });
        }
        // the packets, in order
        let mut at = 0usize;
        for (l, r, c, k) in self.order(&tcs, (tx0, ty0, tx1, ty1)) {
            if at >= data.len() {
                break;
            }
            at = self.packet(data, at, &mut tcs[c].res[r].precincts[k], l, c)?;
        }
        // each code-block decoded into its band; the bands put back together
        let mut out = Vec::new();
        for (c, mut tc) in tcs.into_iter().enumerate() {
            let qcd = self.qcd_of(c);
            for res in &mut tc.res {
                for share in res.precincts.iter_mut().flat_map(|p| p.iter_mut().enumerate()) {
                    let (bi, share) = share;
                    let band = &mut res.bands[bi];
                    let eps = qcd.exps.get(band.q).copied().or(qcd.exps.last().copied()).unwrap_or(0);
                    let mb = (qcd.guard + eps).saturating_sub(1);
                    for blk in &share.blocks {
                        let (w, h) = ((blk.x1 - blk.x0) as usize, (blk.y1 - blk.y0) as usize);
                        if blk.data.is_empty() || w == 0 || h == 0 {
                            continue;
                        }
                        if blk.passes > 1 && blk.len2 > 0 {
                            return Err("an HT code-block with refinement passes, which are not read yet".into());
                        }
                        let coded = block::cleanup(&blk.data, blk.len1, blk.missing, w, h)?;
                        if mb > 62 {
                            return Err("a subband with more bit-planes than are read".into());
                        }
                        let shift = 63 - mb;
                        let bw = (band.x1 - band.x0) as usize;
                        for y in 0..h {
                            for x in 0..w {
                                let v = coded[y * w + x];
                                let mag = ((v & !(1 << 63)) >> shift) as i64;
                                let val = if v >> 63 == 1 { -mag } else { mag };
                                band.samples[(blk.y0 - band.y0) as usize * bw + (blk.x0 - band.x0) as usize + y * bw + x] = val;
                            }
                        }
                    }
                }
            }
            let samples = synthesize(&tc);
            out.push((tc, samples));
        }
        // the colour transform back, the DC level shift, the non-linearity
        if self.cod.mct && out.len() >= 3 && out[0].1.len() == out[1].1.len() && out[1].1.len() == out[2].1.len() {
            let n = out[0].1.len();
            for i in 0..n {
                let (y0, y1, y2) = (out[0].1[i], out[1].1[i], out[2].1[i]);
                let g = y0 - ((y1 + y2) >> 2);
                out[0].1[i] = y2 + g;
                out[1].1[i] = g;
                out[2].1[i] = y1 + g;
            }
        }
        for (c, (_, s)) in out.iter_mut().enumerate() {
            let comp = self.comps[c];
            if !comp.signed {
                let half = 1i64 << (comp.bits - 1);
                s.iter_mut().for_each(|v| *v += half);
            }
            if self.nlt.get(c) == Some(&3) {
                // binary complement: the negative values' bits below the sign turned over
                let mask = (1i64 << (comp.bits - 1)) - 1;
                s.iter_mut().for_each(|v| {
                    if *v < 0 {
                        *v ^= mask;
                    }
                });
            }
        }
        Ok(out)
    }

    /// The order of a tile's packets: (layer, resolution, component, precinct).
    fn order(&self, tcs: &[TileComp], (tx0, ty0, tx1, ty1): (i64, i64, i64, i64)) -> Vec<(u32, usize, usize, usize)> {
        let layers = self.cod.layers.max(1);
        let max_r = tcs.iter().map(|t| t.res.len()).max().unwrap_or(0);
        let mut out = Vec::new();
        match self.cod.order {
            0 => {
                for l in 0..layers {
                    for r in 0..max_r {
                        for (c, t) in tcs.iter().enumerate() {
                            if let Some(res) = t.res.get(r) {
                                out.extend((0..res.pw * res.ph).map(|k| (l, r, c, k)));
                            }
                        }
                    }
                }
            }
            1 => {
                for r in 0..max_r {
                    for l in 0..layers {
                        for (c, t) in tcs.iter().enumerate() {
                            if let Some(res) = t.res.get(r) {
                                out.extend((0..res.pw * res.ph).map(|k| (l, r, c, k)));
                            }
                        }
                    }
                }
            }
            order => {
                // by place on the reference grid: each precinct where it starts
                let step = |dim: usize| -> i64 {
                    let mut s = i64::MAX;
                    for (c, t) in tcs.iter().enumerate() {
                        let comp = self.comps[c];
                        let d = if dim == 0 { comp.dx } else { comp.dy } as i64;
                        let nl = t.res.len() as u32 - 1;
                        for (r, res) in t.res.iter().enumerate() {
                            let pp = if dim == 0 { res.ppx } else { res.ppy };
                            s = s.min(d << (pp + nl - r as u32).min(40));
                        }
                    }
                    s.max(1)
                };
                let (xs, ys) = (step(0), step(1));
                let precinct_at = |c: usize, r: usize, x: i64, y: i64| -> Option<usize> {
                    let t = &tcs[c];
                    let res = t.res.get(r)?;
                    if res.x1 <= res.x0 || res.y1 <= res.y0 {
                        return None;
                    }
                    let comp = self.comps[c];
                    let nl = t.res.len() as u32 - 1;
                    let level = nl - r as u32;
                    let (dx, dy) = (comp.dx as i64, comp.dy as i64);
                    let starts = |v: i64, t0: i64, d: i64, pp: u32, r0: i64| v % (d << (pp + level)) == 0 || (v == t0 && (r0 << level) % (1i64 << (pp + level)) != 0);
                    if !starts(y, ty0, dy, res.ppy, res.y0) || !starts(x, tx0, dx, res.ppx, res.x0) {
                        return None;
                    }
                    let kx = (ceil_div(x, dx << level) >> res.ppx) - (res.x0 >> res.ppx);
                    let ky = (ceil_div(y, dy << level) >> res.ppy) - (res.y0 >> res.ppy);
                    (kx >= 0 && ky >= 0 && (kx as usize) < res.pw && (ky as usize) < res.ph).then_some(ky as usize * res.pw + kx as usize)
                };
                // the places on the grid: the tile's corner, then each multiple of the step
                let places = |t0: i64, t1: i64, step: i64| -> Vec<i64> {
                    let mut v = Vec::new();
                    let mut p = t0;
                    while p < t1 && v.len() < 1 << 20 {
                        v.push(p);
                        p = (p.div_euclid(step) + 1) * step;
                    }
                    v
                };
                let (yl, xl) = (places(ty0, ty1, ys), places(tx0, tx1, xs));
                match order {
                    2 => {
                        for r in 0..max_r {
                            for &y in &yl {
                                for &x in &xl {
                                    for c in 0..tcs.len() {
                                        if let Some(k) = precinct_at(c, r, x, y) {
                                            out.extend((0..layers).map(|l| (l, r, c, k)));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    3 => {
                        for &y in &yl {
                            for &x in &xl {
                                for (c, t) in tcs.iter().enumerate() {
                                    for r in 0..t.res.len() {
                                        if let Some(k) = precinct_at(c, r, x, y) {
                                            out.extend((0..layers).map(|l| (l, r, c, k)));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {
                        for (c, t) in tcs.iter().enumerate() {
                            for &y in &yl {
                                for &x in &xl {
                                    for r in 0..t.res.len() {
                                        if let Some(k) = precinct_at(c, r, x, y) {
                                            out.extend((0..layers).map(|l| (l, r, c, k)));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// The packet at `at` of a precinct's layer `l`: its header read into the code-blocks, then
    /// their bytes. Where the next packet starts.
    fn packet(&self, data: &[u8], mut at: usize, shares: &mut [Share], l: u32, c: usize) -> Result<usize, String> {
        let cod = self.cod_of(c);
        if cod.sop && be16(data, at) == 0xFF91 {
            at += 6;
        }
        let mut bits = HeaderBits { d: data, pos: at, byte: 0, left: 0, last_ff: false };
        let mut lengths: Vec<(usize, usize, usize, usize)> = Vec::new();
        if bits.bit()? == 1 {
            for (si, share) in shares.iter_mut().enumerate() {
                for j in 0..share.h {
                    for i in 0..share.w {
                        let bi = j * share.w + i;
                        let first = !share.blocks[bi].included;
                        let now = if first { share.inclusion.below(i, j, l + 1, &mut bits)? } else { bits.bit()? == 1 };
                        if !now {
                            continue;
                        }
                        if first {
                            share.blocks[bi].missing = share.zeros.value(i, j, &mut bits)?;
                            share.blocks[bi].included = true;
                        }
                        // passes
                        let n = if bits.bit()? == 0 {
                            1
                        } else if bits.bit()? == 0 {
                            2
                        } else {
                            let v = bits.bits(2)?;
                            if v < 3 {
                                3 + v
                            } else {
                                let v = bits.bits(5)?;
                                if v < 31 { 6 + v } else { 37 + bits.bits(7)? }
                            }
                        };
                        let blk = &mut share.blocks[bi];
                        while bits.bit()? == 1 {
                            blk.lblock += 1;
                        }
                        // placeholder passes add to the missing bit-planes; the passes coded
                        let p = (n - 1) / 3;
                        let coded = n - 3 * p;
                        if first {
                            blk.missing += p;
                        }
                        let len1 = bits.bits(blk.lblock + (31 - (3 * p + 1).leading_zeros()))? as usize;
                        let len2 = if coded > 1 { bits.bits(blk.lblock + u32::from(coded > 2))? as usize } else { 0 };
                        blk.passes = coded;
                        lengths.push((si, bi, len1, len2));
                    }
                }
            }
        }
        at = bits.end();
        if cod.eph && be16(data, at) == 0xFF92 {
            at += 2;
        }
        for (si, bi, len1, len2) in lengths {
            let body = data.get(at..at + len1 + len2).ok_or("a packet's code-blocks cut short")?;
            let blk = &mut shares[si].blocks[bi];
            blk.len1 += len1;
            blk.len2 += len2;
            blk.data.extend(body);
            at += len1 + len2;
        }
        Ok(at)
    }
}

/// The 5/3 wavelet undone along one line of `n` samples whose first is at `i0` (the even places
/// low-pass, the odd high-pass), the ends mirrored.
fn line53(y: &mut [i64], i0: i64) {
    let n = y.len();
    if n == 1 {
        if i0 % 2 != 0 {
            y[0] /= 2;
        }
        return;
    }
    let odd = |k: usize| (i0 + k as i64).rem_euclid(2) == 1;
    // the place mirrored into the line
    let at = |k: i64| -> usize {
        let m = 2 * (n as i64 - 1);
        let k = k.rem_euclid(m);
        (if k >= n as i64 { m - k } else { k }) as usize
    };
    let orig = y.to_vec();
    for k in 0..n {
        if !odd(k) {
            let (a, b) = (orig[at(k as i64 - 1)], orig[at(k as i64 + 1)]);
            y[k] = orig[k] - ((a + b + 2) >> 2);
        }
    }
    let evens = y.to_vec();
    for k in 0..n {
        if odd(k) {
            let (a, b) = (evens[at(k as i64 - 1)], evens[at(k as i64 + 1)]);
            y[k] = orig[k] + ((a + b) >> 1);
        }
    }
}

/// A tile-component's samples from its subbands, level by level.
fn synthesize(tc: &TileComp) -> Vec<i64> {
    let ll = &tc.res[0].bands[0];
    let mut cur = ll.samples.clone();
    for r in 1..tc.res.len() {
        let res = &tc.res[r];
        let (w, h) = ((res.x1 - res.x0).max(0) as usize, (res.y1 - res.y0).max(0) as usize);
        let prev = &tc.res[r - 1];
        let (pw, px0, py0) = ((prev.x1 - prev.x0).max(0) as usize, prev.x0, prev.y0);
        let mut a = vec![0i64; w * h];
        for y in 0..h {
            for x in 0..w {
                let (u, v) = (res.x0 + x as i64, res.y0 + y as i64);
                let (ux, vy) = (u.rem_euclid(2), v.rem_euclid(2));
                let val = match (ux, vy) {
                    (0, 0) => cur.get(((v.div_euclid(2) - py0) * pw as i64 + (u.div_euclid(2) - px0)) as usize).copied().unwrap_or(0),
                    _ => {
                        let b = &res.bands[match (ux, vy) {
                            (1, 0) => 0,
                            (0, 1) => 1,
                            _ => 2,
                        }];
                        let bw = b.x1 - b.x0;
                        let (bx, by) = (u.div_euclid(2) - b.x0, v.div_euclid(2) - b.y0);
                        b.samples.get((by * bw + bx) as usize).copied().unwrap_or(0)
                    }
                };
                a[y * w + x] = val;
            }
        }
        // across each row, then down each column
        if w > 0 {
            for row in a.chunks_mut(w) {
                line53(row, res.x0);
            }
        }
        let mut col = vec![0i64; h];
        for x in 0..w {
            for y in 0..h {
                col[y] = a[y * w + x];
            }
            if h > 0 {
                line53(&mut col, res.y0);
            }
            for y in 0..h {
                a[y * w + x] = col[y];
            }
        }
        cur = a;
    }
    cur
}
