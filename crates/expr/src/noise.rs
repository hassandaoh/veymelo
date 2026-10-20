//! Numbers that look random but are the same every time: for `random`, `noise` and `wiggle`.
//! Each comes only from its seeds, so a frame is the same however often and in whatever order
//! it is drawn.

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

/// A seed for a piece of text (a property's path).
pub fn of_text(s: &str) -> u64 {
    s.bytes().fold(0xCBF2_9CE4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01B3))
}

/// From 0 up to (not including) 1.
pub fn unit(seed: u64) -> f64 {
    (hash(seed) >> 11) as f64 / (1u64 << 53) as f64
}

/// Smoothly from 0 to 1, with no jump in speed or in its change.
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// The value at whole step `i`, from -1 to 1.
fn at_step(seed: u64, i: i64) -> f64 {
    unit(seed ^ hash(i as u64)) * 2.0 - 1.0
}

/// Smooth noise along `x`, from -1 to 1: a value at each whole step, faded between them.
pub fn noise1(seed: u64, x: f64) -> f64 {
    if !x.is_finite() {
        return 0.0;
    }
    let i = x.floor();
    let u = fade(x - i);
    // past what a whole number holds the steps are all one; it stays still there
    let i = i.clamp(i64::MIN as f64 / 2.0, i64::MAX as f64 / 2.0) as i64;
    let (a, b) = (at_step(seed, i), at_step(seed, i + 1));
    a + (b - a) * u
}

/// Smooth noise through space, from -1 to 1: near points give near values.
pub fn noise3(p: [f64; 3]) -> f64 {
    if !p.iter().all(|x| x.is_finite()) {
        return 0.0;
    }
    let f = p.map(f64::floor);
    let u = [fade(p[0] - f[0]), fade(p[1] - f[1]), fade(p[2] - f[2])];
    let i = f.map(|v| v.clamp(i64::MIN as f64 / 2.0, i64::MAX as f64 / 2.0) as i64);
    let corner = |dx: i64, dy: i64, dz: i64| unit(mix(&[(i[0] + dx) as u64, (i[1] + dy) as u64, (i[2] + dz) as u64])) * 2.0 - 1.0;
    let l = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let x00 = l(corner(0, 0, 0), corner(1, 0, 0), u[0]);
    let x10 = l(corner(0, 1, 0), corner(1, 1, 0), u[0]);
    let x01 = l(corner(0, 0, 1), corner(1, 0, 1), u[0]);
    let x11 = l(corner(0, 1, 1), corner(1, 1, 1), u[0]);
    l(l(x00, x10, u[1]), l(x01, x11, u[1]), u[2])
}

/// How far each of `dims` parts wiggles at time `t`: `octaves` layers of noise, each twice as
/// quick as the one before and `amp_mult` times as big. With one octave a part stays within
/// `amp` of where it is.
pub fn wiggle(seed: u64, dims: usize, freq: f64, amp: f64, octaves: f64, amp_mult: f64, t: f64) -> Vec<f64> {
    let octaves = if octaves.is_finite() { octaves.clamp(1.0, 16.0) as usize } else { 1 };
    (0..dims)
        .map(|d| {
            let (mut a, mut f, mut sum) = (amp, freq, 0.0);
            for k in 0..octaves {
                let s = mix(&[seed, d as u64, k as u64]);
                // each octave starts somewhere of its own, so they do not all pass zero together
                sum += a * noise1(s, f * t + unit(s) * 1000.0);
                a *= amp_mult;
                f *= 2.0;
            }
            sum
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_bounded_and_the_same_every_time() {
        let mut prev = noise1(7, 0.0);
        for i in 1..10_000 {
            let x = i as f64 * 0.001;
            let v = noise1(7, x);
            assert!((-1.0..=1.0).contains(&v));
            assert!((v - prev).abs() < 0.01, "a jump at {x}");
            prev = v;
        }
        assert_eq!(noise1(3, 1.25), noise1(3, 1.25));
        assert_ne!(noise1(3, 1.25), noise1(4, 1.25));
        assert!((-1.0..=1.0).contains(&noise3([0.3, 4.2, -7.5])));
        assert!((noise3([0.3, 4.2, 1.0]) - noise3([0.3001, 4.2, 1.0])).abs() < 0.01);
    }

    #[test]
    fn wiggles_stay_within_their_amount() {
        for i in 0..2000 {
            let w = wiggle(99, 2, 3.0, 50.0, 1.0, 0.5, i as f64 / 30.0);
            assert!(w.iter().all(|v| v.abs() <= 50.0));
        }
        // not still at the start
        assert!(wiggle(99, 1, 2.0, 50.0, 1.0, 0.5, 0.0)[0].abs() > 0.0);
        for x in [1e300, -1e300, 9.3e18] {
            assert!(noise1(5, x).is_finite() && noise3([x, -x, x]).is_finite());
        }
        assert!(wiggle(1, 2, 9_007_199_254_740_993.0, 30.0, 1.0, 0.5, 1e6).iter().all(|v| v.abs() <= 30.0));
    }
}
