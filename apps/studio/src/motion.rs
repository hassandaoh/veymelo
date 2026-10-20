//! How things move: springs that ease toward where they are sent, curves from 0 to 1, how a
//! pull weakens with distance, and an even-looking scatter for a person's uneven pace.

/// A spring's place and speed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spring {
    pub x: f64,
    pub v: f64,
}

impl Spring {
    pub const fn new() -> Spring {
        Spring { x: 0.0, v: 0.0 }
    }

    /// Moves toward `aim` for `dt` seconds; `stiff` and `damp` set how. Gives whether it is
    /// still moving.
    pub fn toward(&mut self, aim: f64, dt: f64, stiff: f64, damp: f64) -> bool {
        spring(&mut self.x, &mut self.v, aim, dt, stiff, damp)
    }

    /// At rest where it is.
    pub fn still(&self) -> bool {
        self.v == 0.0
    }
}

/// Moves a spring at `x` (going at `v`) toward `aim` for `dt` seconds; `stiff` and `damp` set
/// how it moves. Gives whether it is still moving (once it is close enough and slow enough,
/// it is put exactly at `aim`).
pub fn spring(x: &mut f64, v: &mut f64, aim: f64, dt: f64, stiff: f64, damp: f64) -> bool {
    let mut left = dt;
    while left > 0.0 {
        let h = left.min(1.0 / 240.0);
        *v += (stiff * (aim - *x) - damp * *v) * h;
        *x += *v * h;
        left -= h;
    }
    let moving = (aim - *x).abs() > 0.002 || v.abs() > 0.01;
    if !moving {
        (*x, *v) = (aim, 0.0);
    }
    moving
}

/// How strongly something `d` points away feels what reaches `reach` points: all of it at no
/// distance, about a third at two fifths of the way, under a tenth at the end, nothing past.
pub fn reach(d: f64, reach: f64) -> f64 {
    if d >= 1.25 * reach { 0.0 } else { (-2.8 * d / reach).exp() }
}

/// Rises from 0 to 1 and settles there without going past it (a well-damped spring), as `p`
/// goes from 0 to 1.
pub fn settle(p: f64) -> f64 {
    let w = 9.0 * p.clamp(0.0, 1.0);
    1.0 - (1.0 + w) * (-w).exp()
}

/// 0 to 1, eased in and out.
pub fn smooth(p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

/// A number from 0 to 1 that looks random but is the same every time for `i`.
pub fn jitter(i: usize) -> f64 {
    let mut x = (i as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x % 10_000) as f64 / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointer_reaches_less_the_farther_it_is() {
        assert_eq!(reach(0.0, 100.0), 1.0);
        assert!((reach(42.0, 100.0) - 0.31).abs() < 0.01);
        assert!(reach(100.0, 100.0) < 0.1);
        assert_eq!(reach(130.0, 100.0), 0.0);
    }

    #[test]
    fn a_spring_comes_to_rest_where_it_aims() {
        let mut s = Spring::new();
        let mut t = 0.0;
        while s.toward(1.0, 1.0 / 60.0, 170.0, 22.0) {
            t += 1.0 / 60.0;
            assert!(t < 2.0, "it settles");
        }
        assert_eq!(s.x, 1.0);
        assert!(t > 0.2, "it takes a moment: {t}");
    }

    #[test]
    fn a_letter_settles_into_place() {
        assert_eq!(settle(0.0), 0.0);
        assert!(settle(0.5) > 0.9 && settle(1.0) > 0.998 && settle(1.0) <= 1.0);
    }
}
