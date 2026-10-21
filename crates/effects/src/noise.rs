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

/// A double as two floats that add up to it (the second what the first leaves out), as the GPU
/// holds places (noise.metal's df).
pub(crate) fn split(v: f64) -> [f32; 2] {
    let hi = v as f32;
    [hi, (v - hi as f64) as f32]
}

/// What z is to [`smooth`] on the GPU, worked out once (noise.metal's Depth): its whole part, the
/// period (0: none), how far past the whole part, and that faded.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct GpuDepth {
    iz: i64,
    period: i64,
    w: f32,
    c: f32,
}

unsafe impl veymelo_raster::gpu::Plain for GpuDepth {}

pub(crate) fn gpu_depth(z: f64, period: Option<i64>) -> GpuDepth {
    let fz = z.floor();
    let w = z - fz;
    GpuDepth { iz: fz as i64, period: period.filter(|p| *p > 0).unwrap_or(0), w: w as f32, c: (w * w * w * (w * (w * 6.0 - 15.0) + 10.0)) as f32 }
}

/// The most octaves the GPU takes.
pub(crate) const GPU_OCTAVES: usize = 24;

/// [`fractal`]'s settings worked out once for the GPU (noise.metal's Octaves): how many octaves,
/// the last one's part, the fall in strength, what the strengths add up to, the turn between
/// octaves, and each octave's shift.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct GpuOctaves {
    n: u32,
    part: f32,
    falloff: f32,
    total: f32,
    turn_c: [f32; 2],
    turn_s: [f32; 2],
    shift: [[f32; 4]; GPU_OCTAVES],
}

unsafe impl veymelo_raster::gpu::Plain for GpuOctaves {}

/// [`fractal`]'s `octaves` and `falloff` for the GPU (None: more octaves than it takes).
pub(crate) fn gpu_octaves(octaves: f64, falloff: f64) -> Option<GpuOctaves> {
    let n = octaves.ceil().max(1.0) as usize;
    if n > GPU_OCTAVES {
        return None;
    }
    let part = octaves - octaves.floor();
    let (mut total, mut strength) = (0.0, 1.0);
    let mut shift = [[0f32; 4]; GPU_OCTAVES];
    for (o, s) in shift.iter_mut().enumerate().take(n) {
        let weight = if o + 1 == n && part > 0.0 { part } else { 1.0 };
        total += strength * weight;
        strength *= falloff;
        let (x, y) = (split(o as f64 * 17.3), split(o as f64 * 31.7));
        *s = [x[0], x[1], y[0], y[1]];
    }
    let (s, c) = 0.61f64.sin_cos();
    Some(GpuOctaves { n: n as u32, part: part as f32, falloff: falloff as f32, total: total as f32, turn_c: split(c), turn_s: split(s), shift })
}

#[cfg(test)]
mod gpu_tests {
    /// The GPU's noise is the processor's to within a millionth (its places exact, its sums in
    /// floats), over many places, octaves and depths.
    #[test]
    fn the_gpu_noise_is_the_processors() {
        if !veymelo_raster::gpu::available() {
            return;
        }
        use veymelo_raster::gpu;
        const PROBE: &str = "struct Probe { uint w; float scale; df origin; ulong seed; Depth z; };\n\
            kernel void probe(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], constant Octaves &oct [[buffer(2)]], constant Probe &p [[buffer(3)]], uint2 g [[thread_position_in_grid]]) {\n\
                df x = df_add(df_mul(df_of(float(g.x)), df_of(p.scale)), p.origin);\n\
                df y = df_add(df_mul(df_of(float(g.y)), df_of(p.scale)), p.origin);\n\
                out[g.y * p.w + g.x] = float4(fractal_noise(x, y, p.z, p.seed, oct), 0.0, 0.0, 0.0);\n\
            }";
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Probe {
            w: u32,
            scale: f32,
            origin: [f32; 2],
            seed: u64,
            z: super::GpuDepth,
        }
        unsafe impl gpu::Plain for Probe {}
        let (w, h) = (128, 96);
        let img = veymelo_raster::Image::new(w, h);
        for (octaves, falloff, origin, scale, z) in [(1.0, 0.5, 0.0, 0.037, 0.3), (4.6, 0.5, -3.7, 0.11, 2.75), (10.0, 0.3, 120.25, 0.013, 7.0)] {
            let p = Probe { w: w as u32, scale: scale as f32, origin: super::split(origin), seed: 99, z: super::gpu_depth(z, None) };
            let kernel = gpu::Kernel { source: gpu::program(&[gpu::COMMON, include_str!("noise.metal"), PROBE]), name: "probe" };
            let out = gpu::apply(&img, &[gpu::Op::new(kernel, &p, [w, h]).with(&[super::gpu_octaves(octaves, falloff).unwrap()])]).unwrap();
            let mut most = 0f64;
            for y in 0..h {
                for x in 0..w {
                    let (u, v) = (x as f32 as f64 * scale as f32 as f64 + origin, y as f32 as f64 * scale as f32 as f64 + origin);
                    let cpu = super::fractal(u, v, z, 99, octaves, falloff, None);
                    most = most.max((out.px[y * w + x][0] as f64 - cpu).abs());
                }
            }
            assert!(most <= 1e-6, "octaves {octaves}: {most:e}");
        }
    }
}
