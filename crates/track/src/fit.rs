//! How the picture as a whole moved, from points followed between two frames, ignoring the
//! points that moved otherwise (something passing in front, a bad match): RANSAC — fits to a few
//! points chosen at random, kept the one most others agree with — then refined by least squares
//! on those that agree. As a move, as a move with a turn and a scale, or as a perspective.

use crate::solve::{Warp, linear};

/// A point where it was and where it is.
pub type Pair = ([f64; 2], [f64; 2]);

/// What kind of map a fit is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Move,
    Similar,
    /// Moved, turned, scaled and slanted.
    Slant,
    Perspective,
}

impl Model {
    fn needs(self) -> usize {
        match self {
            Model::Move => 1,
            Model::Similar => 2,
            Model::Slant => 3,
            Model::Perspective => 4,
        }
    }
}

/// Numbers that look random, the same each time (so an analysis gives the same answer again).
struct Dice(u64);

impl Dice {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

/// The least-squares map of `model` taking each `.0` to its `.1`.
pub fn least_squares(model: Model, pairs: &[Pair]) -> Option<Warp> {
    if pairs.len() < model.needs() {
        return None;
    }
    let n = pairs.len() as f64;
    match model {
        Model::Move => {
            let (dx, dy) = pairs.iter().fold((0.0, 0.0), |(x, y), (a, b)| (x + b[0] - a[0], y + b[1] - a[1]));
            Some(Warp([[1.0, 0.0, dx / n], [0.0, 1.0, dy / n], [0.0, 0.0, 1.0]]))
        }
        Model::Similar => {
            // about the means: b − mb = (a + b i)(p − mp) as complex numbers
            let mean = |f: &dyn Fn(&Pair) -> [f64; 2]| pairs.iter().map(f).fold([0.0; 2], |s, q| [s[0] + q[0] / n, s[1] + q[1] / n]);
            let (mp, mq) = (mean(&|p| p.0), mean(&|p| p.1));
            let (mut re, mut im, mut norm) = (0.0, 0.0, 0.0);
            for (p, q) in pairs {
                let (u, v) = ([p[0] - mp[0], p[1] - mp[1]], [q[0] - mq[0], q[1] - mq[1]]);
                re += u[0] * v[0] + u[1] * v[1];
                im += u[0] * v[1] - u[1] * v[0];
                norm += u[0] * u[0] + u[1] * u[1];
            }
            if norm < 1e-12 {
                return None;
            }
            let (a, b) = (re / norm, im / norm);
            Some(Warp([[a, -b, mq[0] - (a * mp[0] - b * mp[1])], [b, a, mq[1] - (b * mp[0] + a * mp[1])], [0.0, 0.0, 1.0]]))
        }
        Model::Slant => {
            // each of x' and y' as a + b x + c y, about the means
            let mean = |f: &dyn Fn(&Pair) -> [f64; 2]| pairs.iter().map(f).fold([0.0; 2], |s, q| [s[0] + q[0] / n, s[1] + q[1] / n]);
            let (mp, mq) = (mean(&|p| p.0), mean(&|p| p.1));
            let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
            let (mut ux, mut uy, mut vx, mut vy) = (0.0, 0.0, 0.0, 0.0);
            for (p, q) in pairs {
                let (x, y, u, v) = (p[0] - mp[0], p[1] - mp[1], q[0] - mq[0], q[1] - mq[1]);
                (sxx, sxy, syy) = (sxx + x * x, sxy + x * y, syy + y * y);
                (ux, uy, vx, vy) = (ux + u * x, uy + u * y, vx + v * x, vy + v * y);
            }
            let m = [[sxx, sxy], [sxy, syy]];
            let [a, b] = linear(m, [ux, uy])?;
            let [c, d] = linear(m, [vx, vy])?;
            Some(Warp([[a, b, mq[0] - a * mp[0] - b * mp[1]], [c, d, mq[1] - c * mp[0] - d * mp[1]], [0.0, 0.0, 1.0]]))
        }
        Model::Perspective => {
            // in coordinates about the means and of unit spread, so the sums keep their sense
            let norm = |f: &dyn Fn(&Pair) -> [f64; 2]| {
                let m = pairs.iter().map(f).fold([0.0; 2], |s, q| [s[0] + q[0] / n, s[1] + q[1] / n]);
                let spread = pairs.iter().map(f).map(|q| (q[0] - m[0]).hypot(q[1] - m[1])).sum::<f64>() / n;
                let k = if spread > 1e-12 { std::f64::consts::SQRT_2 / spread } else { 1.0 };
                (m, k)
            };
            let ((mp, kp), (mq, kq)) = (norm(&|p| p.0), norm(&|p| p.1));
            let mut ata = [[0.0f64; 8]; 8];
            let mut atb = [0.0f64; 8];
            for (p, q) in pairs {
                let (x, y) = ((p[0] - mp[0]) * kp, (p[1] - mp[1]) * kp);
                let (u, v) = ((q[0] - mq[0]) * kq, (q[1] - mq[1]) * kq);
                for (row, rhs) in [([x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y], u), ([0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y], v)] {
                    for i in 0..8 {
                        atb[i] += row[i] * rhs;
                        for j in 0..8 {
                            ata[i][j] += row[i] * row[j];
                        }
                    }
                }
            }
            let h = linear(ata, atb)?;
            let hn = [[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]];
            // back from the normalised coordinates: T_q⁻¹ H T_p
            let tp = [[kp, 0.0, -kp * mp[0]], [0.0, kp, -kp * mp[1]], [0.0, 0.0, 1.0]];
            let tq_inv = [[1.0 / kq, 0.0, mq[0]], [0.0, 1.0 / kq, mq[1]], [0.0, 0.0, 1.0]];
            let m = times(&tq_inv, &times(&hn, &tp));
            (m[2][2].abs() > 1e-12).then(|| Warp(m.map(|r| r.map(|v| v / m[2][2]))))
        }
    }
}

/// Matrix product.
pub fn times(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut m = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

/// The map of `model` most of `pairs` agree with (within `tolerance` pixels), refined on those,
/// and how many agree.
pub fn robust(model: Model, pairs: &[Pair], tolerance: f64) -> Option<(Warp, usize)> {
    let need = model.needs();
    if pairs.len() < need {
        return None;
    }
    let agree = |w: &Warp| -> Vec<usize> {
        (0..pairs.len())
            .filter(|&i| {
                let q = w.apply(pairs[i].0);
                (q[0] - pairs[i].1[0]).hypot(q[1] - pairs[i].1[1]) <= tolerance
            })
            .collect()
    };
    let mut dice = Dice(0x2545_F491_4F6C_DD1D ^ pairs.len() as u64);
    let mut best: Vec<usize> = Vec::new();
    for _ in 0..200 {
        let mut pick: Vec<usize> = Vec::with_capacity(need);
        while pick.len() < need {
            let i = dice.below(pairs.len());
            if !pick.contains(&i) {
                pick.push(i);
            }
        }
        let some: Vec<_> = pick.iter().map(|&i| pairs[i]).collect();
        let Some(w) = least_squares(model, &some) else { continue };
        let with = agree(&w);
        if with.len() > best.len() {
            best = with;
            if best.len() == pairs.len() {
                break;
            }
        }
    }
    if best.len() < need {
        return None;
    }
    // refined on those that agree, and once more on those that agree with that
    let mut w = least_squares(model, &best.iter().map(|&i| pairs[i]).collect::<Vec<_>>())?;
    let again = agree(&w);
    if again.len() >= need {
        w = least_squares(model, &again.iter().map(|&i| pairs[i]).collect::<Vec<_>>()).unwrap_or(w);
    }
    Some((w, again.len().max(best.len())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whole_found_past_the_odd_ones() {
        let truth = Warp([[1.01, -0.03, 4.0], [0.02, 0.99, -2.5], [0.0001, -0.00005, 1.0]]);
        let mut dice = Dice(7);
        let mut pairs: Vec<([f64; 2], [f64; 2])> = (0..80)
            .map(|_| {
                let p = [dice.below(600) as f64, dice.below(400) as f64];
                (p, truth.apply(p))
            })
            .collect();
        // a quarter moving some other way
        for (k, pair) in pairs.iter_mut().enumerate().filter(|(k, _)| k % 4 == 0) {
            pair.1 = [pair.1[0] + 30.0 + k as f64, pair.1[1] - 12.0];
        }
        let (w, n) = robust(Model::Perspective, &pairs, 1.0).unwrap();
        assert_eq!(n, 60);
        for p in [[100.0, 100.0], [500.0, 300.0]] {
            let (a, b) = (w.apply(p), truth.apply(p));
            assert!((a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6, "{a:?} {b:?}");
        }
        let turned = Warp([[0.98, -0.1, 7.0], [0.1, 0.98, 3.0], [0.0, 0.0, 1.0]]);
        let pairs: Vec<_> = pairs.iter().enumerate().map(|(k, (p, _))| (*p, if k % 5 == 0 { [0.0, 0.0] } else { turned.apply(*p) })).collect();
        let (s, n) = robust(Model::Similar, &pairs, 1.0).unwrap();
        assert_eq!(n, 64);
        assert!((s.0[0][0] - 0.98).abs() < 1e-9 && (s.0[1][0] - 0.1).abs() < 1e-9);
        let slant = Warp([[1.1, 0.25, 7.0], [-0.05, 0.9, 3.0], [0.0, 0.0, 1.0]]);
        let three: Vec<_> = (0..20).map(|k| ([k as f64 * 13.0 % 97.0, k as f64 * 29.0 % 83.0], slant.apply([k as f64 * 13.0 % 97.0, k as f64 * 29.0 % 83.0]))).collect();
        let (a, n) = robust(Model::Slant, &three, 0.5).unwrap();
        assert_eq!(n, 20);
        assert!(a.0.iter().flatten().zip(slant.0.iter().flatten()).all(|(x, y)| (x - y).abs() < 1e-9), "{a:?}");
        let (m, _) = robust(Model::Move, &[([0.0, 0.0], [3.0, 4.0]), ([5.0, 5.0], [8.0, 9.0]), ([1.0, 1.0], [50.0, 0.0])], 0.5).unwrap();
        assert_eq!((m.0[0][2], m.0[1][2]), (3.0, 4.0));
    }
}
