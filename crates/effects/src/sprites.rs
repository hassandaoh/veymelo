//! Many small marks drawn at once — soft dots, discs, lit balls, bubbles, glints, squares,
//! triangles and streaks — each in its own colour, laid over what is there or added to it. The
//! picture is cut into bands of rows drawn side by side, each mark in the bands it touches.

use veymelo_raster::{Image, Px, par};

/// Rows a band has.
const BAND: usize = 16;

/// A mark's shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// A dot fading softly to its edge.
    Soft,
    /// A disc with a crisp (smoothed) edge.
    Disc,
    /// A ball lit from the upper left, with a shine.
    Ball,
    /// A thin ring with a glint, faintly filled.
    Bubble,
    /// A four-pointed glint, turned by `turn`.
    Glint,
    /// A square, turned by `turn`.
    Square,
    /// A triangle, turned by `turn`.
    Triangle,
    /// A streak from (x, y) back by (dx, dy), `r` thick each side, fading to its tail.
    Streak { dx: f32, dy: f32 },
    /// A line from (x, y) back by (dx, dy), `r` thick each side, even all along.
    Line { dx: f32, dy: f32 },
    /// A picture (one of those given to [`draw_with`]) on a square, turned by `turn`, or on a
    /// disc (`round`).
    Picture { index: u16, round: bool },
}

/// A mark, in the picture's pixels; `color` is not premultiplied (its alpha the mark's
/// opacity).
#[derive(Clone, Copy, Debug)]
pub struct Sprite {
    pub x: f32,
    pub y: f32,
    pub r: f32,
    pub color: [f32; 4],
    pub shape: Shape,
    pub turn: f32,
}

impl Sprite {
    pub fn new(x: f32, y: f32, r: f32, color: [f32; 4], shape: Shape) -> Sprite {
        Sprite { x, y, r, color, shape, turn: 0.0 }
    }

    /// The pixels it can touch: x0, y0, x1, y1.
    fn reach(&self) -> Option<[f32; 4]> {
        if self.color[3] <= 1e-5 || self.r <= 0.0 || !self.x.is_finite() || !self.y.is_finite() {
            return None;
        }
        let r = self.r.max(0.5) + 1.0;
        Some(match self.shape {
            Shape::Streak { dx, dy } | Shape::Line { dx, dy } => [self.x.min(self.x - dx) - r, self.y.min(self.y - dy) - r, self.x.max(self.x - dx) + r, self.y.max(self.y - dy) + r],
            Shape::Glint => [self.x - 2.0 * r, self.y - 2.0 * r, self.x + 2.0 * r, self.y + 2.0 * r],
            _ => [self.x - r, self.y - r, self.x + r, self.y + r],
        })
    }

    /// Its premultiplied colour at the pixel centred on (x, y), if it shows there (`pictures`
    /// for the pictured kind).
    fn at(&self, x: f32, y: f32, pictures: &[Image]) -> Option<Px> {
        // a mark smaller than a pixel keeps what it covers: drawn half a pixel wide, fainter
        let (r, faint) = if self.r < 0.5 { (0.5, self.r / 0.5) } else { (self.r, 1.0) };
        let (dx, dy) = (x - self.x, y - self.y);
        let edge = |e: f32| (e + 0.5).clamp(0.0, 1.0);
        let mut rgb = [self.color[0], self.color[1], self.color[2]];
        let turned = || {
            let (s, c) = self.turn.sin_cos();
            (dx * c + dy * s, -dx * s + dy * c)
        };
        let cover = match self.shape {
            Shape::Soft => {
                let d2 = (dx * dx + dy * dy) / (r * r);
                if d2 >= 1.0 {
                    return None;
                }
                (1.0 - d2) * (1.0 - d2)
            }
            Shape::Disc => edge(r - dx.hypot(dy)),
            Shape::Ball => {
                let d = dx.hypot(dy);
                let k = edge(r - d);
                if k <= 0.0 {
                    return None;
                }
                let (nx, ny) = (dx / r, dy / r);
                let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
                let (lx, ly, lz) = (-0.48, -0.58, 0.66);
                let lit = (nx * lx + ny * ly + nz * lz).max(0.0);
                let shine = (2.0 * lit * nz - lz).max(0.0).powi(20) * 0.6;
                let k2 = 0.2 + 0.8 * lit;
                rgb = rgb.map(|v| v * k2 + shine);
                k
            }
            Shape::Bubble => {
                let d = dx.hypot(dy) / r;
                if d > 1.0 + 1.0 / r {
                    return None;
                }
                let ring = (1.0 - ((d - 0.9) / 0.12).powi(2)).max(0.0);
                let glint = (1.0 - ((dx / r + 0.4).powi(2) + (dy / r + 0.4).powi(2)) / 0.04).max(0.0);
                (if d <= 1.0 { 0.12 } else { 0.0 } + ring * 0.8 + glint).min(1.0)
            }
            Shape::Glint => {
                let (u, v) = turned();
                let (u, v) = (u / r, v / r);
                let core = (1.0 - u.hypot(v)).max(0.0).powi(2);
                let across = (1.0 - u.abs() / 2.0).max(0.0) * (1.0 - v.abs() * 6.0).max(0.0);
                let down = (1.0 - v.abs() / 2.0).max(0.0) * (1.0 - u.abs() * 6.0).max(0.0);
                (core + across + down).min(1.0)
            }
            Shape::Square => {
                let (u, v) = turned();
                let e = r * 0.75;
                edge(e - u.abs()) * edge(e - v.abs())
            }
            Shape::Triangle => {
                let (u, v) = turned();
                let e = r * 0.5;
                (0..3)
                    .map(|k| {
                        let a = std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::TAU / 3.0;
                        let (s, c) = a.sin_cos();
                        edge(e - (u * c + v * s))
                    })
                    .fold(1.0, f32::min)
            }
            Shape::Picture { index, round } => {
                let pic = pictures.get(index as usize)?;
                let (u, v) = turned();
                let (u, v) = (u / r, v / r);
                let inside = if round { edge((1.0 - u.hypot(v)) * r) } else { edge((1.0 - u.abs()) * r) * edge((1.0 - v.abs()) * r) };
                if inside <= 0.0 {
                    return None;
                }
                let p = pic.sample(veymelo_raster::Sampling::Bilinear, ((u + 1.0) * 0.5 * pic.width as f32) as f64, ((v + 1.0) * 0.5 * pic.height as f32) as f64);
                let a = inside * self.color[3] * faint;
                return (p[3] * a > 1e-6).then(|| p.map(|c| c * a));
            }
            Shape::Streak { dx: lx, dy: ly } | Shape::Line { dx: lx, dy: ly } => {
                let l2 = lx * lx + ly * ly;
                // t from the head (0) back to the tail (1)
                let t = if l2 > 0.0 { ((-dx * lx - dy * ly) / l2).clamp(0.0, 1.0) } else { 0.0 };
                let (px, py) = (dx + lx * t, dy + ly * t);
                let taper = if matches!(self.shape, Shape::Streak { .. }) { 1.0 - 0.7 * t } else { 1.0 };
                edge(r - px.hypot(py)) * taper
            }
        };
        let a = cover * self.color[3] * faint;
        if a <= 1e-6 {
            return None;
        }
        Some([rgb[0] * a, rgb[1] * a, rgb[2] * a, a])
    }
}

/// `sprites` drawn into `img`, later ones over earlier ones (or `add`ed, for light).
pub fn draw(img: &mut Image, sprites: &[Sprite], add: bool) {
    draw_with(img, sprites, add, &[]);
}

/// The same, pictured marks showing `pictures`.
pub fn draw_with(img: &mut Image, sprites: &[Sprite], add: bool, pictures: &[Image]) {
    let (w, h) = (img.width, img.height);
    if w == 0 || h == 0 || sprites.is_empty() {
        return;
    }
    let bands = h.div_ceil(BAND);
    let boxes: Vec<Option<[usize; 4]>> = sprites
        .iter()
        .map(|s| {
            let b = s.reach()?;
            if b[2] < 0.0 || b[3] < 0.0 || b[0] >= w as f32 || b[1] >= h as f32 {
                return None;
            }
            Some([b[0].floor().max(0.0) as usize, b[1].floor().max(0.0) as usize, (b[2].ceil() as usize).min(w - 1), (b[3].ceil() as usize).min(h - 1)])
        })
        .collect();
    let mut in_band: Vec<Vec<u32>> = vec![Vec::new(); bands];
    for (i, b) in boxes.iter().enumerate() {
        if let Some(b) = b {
            for band in in_band.iter_mut().take(b[3] / BAND + 1).skip(b[1] / BAND) {
                band.push(i as u32);
            }
        }
    }
    par::chunks(&mut img.px, w * BAND, |band, chunk| {
        let top = band * BAND;
        let rows = chunk.len() / w;
        for &i in &in_band[band] {
            let (s, b) = (&sprites[i as usize], boxes[i as usize].expect("in a band"));
            for y in b[1].max(top)..=b[3].min(top + rows - 1) {
                for x in b[0]..=b[2] {
                    let Some(c) = s.at(x as f32 + 0.5, y as f32 + 0.5, pictures) else { continue };
                    let d = &mut chunk[(y - top) * w + x];
                    *d = if add {
                        [d[0] + c[0], d[1] + c[1], d[2] + c[2], (d[3] + c[3]).min(1.0)]
                    } else {
                        let k = 1.0 - c[3];
                        [c[0] + d[0] * k, c[1] + d[1] * k, c[2] + d[2] * k, c[3] + d[3] * k]
                    };
                }
            }
        }
    });
}

/// A number from 0 to 1 for (`i`, `k`) and `seed`, the same each time.
pub fn rand(i: u64, k: u64, seed: u64) -> f32 {
    crate::noise::unit(crate::noise::hash(seed ^ i.wrapping_mul(0x9E37_79B9_7F4A_7C15)) ^ k.wrapping_mul(0xC2B2_AE3D_27D4_EB4F)) as f32
}

/// A number from -1 to 1 the same way.
pub fn rand2(i: u64, k: u64, seed: u64) -> f32 {
    rand(i, k, seed) * 2.0 - 1.0
}
