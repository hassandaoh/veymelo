//! Numbers that look random but are the same every time, from their seeds alone.

/// Mix a number so every bit of it moves every bit of the result.
pub fn hash(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Several seeds as one.
pub fn mix(parts: &[u64]) -> u64 {
    parts.iter().fold(0x2545_F491_4F6C_DD1D, |h, p| hash(h ^ hash(*p)))
}

/// From 0 up to (not including) 1.
pub fn unit(seed: u64) -> f64 {
    (hash(seed) >> 11) as f64 / (1u64 << 53) as f64
}

/// The slope at a corner of the lattice: one of the twelve directions to a cube's edges.
#[inline]
fn slope(h: u64, x: f64, y: f64, z: f64) -> f64 {
    match h % 12 {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        3 => -x - y,
        4 => x + z,
        5 => -x + z,
        6 => x - z,
        7 => -x - z,
        8 => y + z,
        9 => -y + z,
        10 => y - z,
        _ => -y - z,
    }
}

/// Smooth noise in three dimensions: about -1 to 1, 0 on the whole numbers, from `seed`.
/// With a `period` it repeats every that many along z (so an evolution can loop).
pub fn smooth(x: f64, y: f64, z: f64, seed: u64, period: Option<i64>) -> f64 {
    let (fx, fy, fz) = (x.floor(), y.floor(), z.floor());
    let (u, v, w) = (x - fx, y - fy, z - fz);
    let (ix, iy, iz) = (fx as i64, fy as i64, fz as i64);
    let fade = |t: f64| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (a, b, c) = (fade(u), fade(v), fade(w));
    let corner = |dx: i64, dy: i64, dz: i64| {
        let k = match period {
            Some(p) if p > 0 => (iz + dz).rem_euclid(p),
            _ => iz + dz,
        };
        let h = mix(&[seed, (ix + dx) as u64, (iy + dy) as u64, k as u64]);
        slope(h, u - dx as f64, v - dy as f64, w - dz as f64)
    };
    let lerp = |t: f64, p: f64, q: f64| p + (q - p) * t;
    let x00 = lerp(a, corner(0, 0, 0), corner(1, 0, 0));
    let x10 = lerp(a, corner(0, 1, 0), corner(1, 1, 0));
    let x01 = lerp(a, corner(0, 0, 1), corner(1, 0, 1));
    let x11 = lerp(a, corner(0, 1, 1), corner(1, 1, 1));
    (lerp(c, lerp(b, x00, x10), lerp(b, x01, x11)) * 0.97).clamp(-1.0, 1.0)
}

/// Octaves of [`smooth`] noise, each twice as fine as the one before and `falloff` times as
/// strong, turned a little against each other so no grid shows: about -1 to 1. A part octave
/// (2.5) counts by its part.
pub fn fractal(x: f64, y: f64, z: f64, seed: u64, octaves: f64, falloff: f64, period: Option<i64>) -> f64 {
    let n = octaves.ceil().max(1.0) as usize;
    let part = octaves - octaves.floor();
    let (mut sum, mut total, mut strength, mut fine) = (0.0, 0.0, 1.0, 1.0);
    let (s, c) = 0.61f64.sin_cos();
    let (mut px, mut py) = (x, y);
    for o in 0..n {
        let weight = if o + 1 == n && part > 0.0 { part } else { 1.0 };
        sum += smooth(px * fine + o as f64 * 17.3, py * fine + o as f64 * 31.7, z, mix(&[seed, o as u64]), period) * strength * weight;
        total += strength * weight;
        strength *= falloff;
        fine *= 2.0;
        (px, py) = (px * c - py * s, px * s + py * c);
    }
    sum / total.max(1e-9)
}
