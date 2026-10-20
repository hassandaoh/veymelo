//! Small sums the tracker does: a few linear equations solved, and the map that takes points
//! where they were to where they are — moved, turned and scaled (two points), slanted too (three:
//! a parallel corner pin), or seen in perspective (four).

/// `m x = b` solved by elimination with the largest pivot first; none when `m` is singular.
pub fn linear<const N: usize>(mut m: [[f64; N]; N], mut b: [f64; N]) -> Option<[f64; N]> {
    for col in 0..N {
        let pivot = (col..N).max_by(|&i, &j| m[i][col].abs().total_cmp(&m[j][col].abs()))?;
        if m[pivot][col].abs() < 1e-12 {
            return None;
        }
        m.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..N {
            let f = m[row][col] / m[col][col];
            if f != 0.0 {
                let top = m[col];
                for (k, v) in m[row].iter_mut().enumerate().skip(col) {
                    *v -= f * top[k];
                }
                b[row] -= f * b[col];
            }
        }
    }
    let mut x = [0.0; N];
    for row in (0..N).rev() {
        let s: f64 = (row + 1..N).map(|k| m[row][k] * x[k]).sum();
        x[row] = (b[row] - s) / m[row][row];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// A map of the plane as a 3 × 3 matrix on (x, y, 1), divided through by its last number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Warp(pub [[f64; 3]; 3]);

impl Warp {
    pub const NONE: Warp = Warp([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    /// The map back; none when it squashes the plane flat.
    pub fn inverse(&self) -> Option<Warp> {
        let m = &self.0;
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        if det.abs() < 1e-15 {
            return None;
        }
        let c = |a: usize, b: usize, c: usize, d: usize| m[a][b] * m[c][d] - m[a][d] * m[c][b];
        let adj = [[c(1, 1, 2, 2), -c(0, 1, 2, 2), c(0, 1, 1, 2)], [-c(1, 0, 2, 2), c(0, 0, 2, 2), -c(0, 0, 1, 2)], [c(1, 0, 2, 1), -c(0, 0, 2, 1), c(0, 0, 1, 1)]];
        Some(Warp(adj.map(|r| r.map(|v| v / det))))
    }

    /// Where it takes `p`.
    pub fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        let m = &self.0;
        let w = m[2][0] * p[0] + m[2][1] * p[1] + m[2][2];
        let w = if w.abs() < 1e-12 { 1e-12 } else { w };
        [(m[0][0] * p[0] + m[0][1] * p[1] + m[0][2]) / w, (m[1][0] * p[0] + m[1][1] * p[1] + m[1][2]) / w]
    }

    /// The perspective map taking the four points `from` to `to`.
    pub fn perspective(from: [[f64; 2]; 4], to: [[f64; 2]; 4]) -> Option<Warp> {
        let mut m = [[0.0; 8]; 8];
        let mut b = [0.0; 8];
        for k in 0..4 {
            let ([x, y], [u, v]) = (from[k], to[k]);
            m[2 * k] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y];
            m[2 * k + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y];
            (b[2 * k], b[2 * k + 1]) = (u, v);
        }
        let h = linear(m, b)?;
        Some(Warp([[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]]))
    }

    /// The map that moves, turns, scales and slants the three points `from` onto `to`.
    pub fn parallel(from: [[f64; 2]; 3], to: [[f64; 2]; 3]) -> Option<Warp> {
        let m = [[from[0][0], from[0][1], 1.0], [from[1][0], from[1][1], 1.0], [from[2][0], from[2][1], 1.0]];
        let xs = linear(m, [to[0][0], to[1][0], to[2][0]])?;
        let ys = linear(m, [to[0][1], to[1][1], to[2][1]])?;
        Some(Warp([xs, ys, [0.0, 0.0, 1.0]]))
    }

    /// The map that moves, turns and scales `from[0]` and `from[1]` onto `to[0]` and `to[1]`.
    pub fn similar(from: [[f64; 2]; 2], to: [[f64; 2]; 2]) -> Option<Warp> {
        let (d, e) = ([from[1][0] - from[0][0], from[1][1] - from[0][1]], [to[1][0] - to[0][0], to[1][1] - to[0][1]]);
        let n = d[0] * d[0] + d[1] * d[1];
        if n < 1e-12 {
            return None;
        }
        // e = (a + b i) d, as complex numbers
        let (a, b) = ((e[0] * d[0] + e[1] * d[1]) / n, (e[1] * d[0] - e[0] * d[1]) / n);
        let t = [to[0][0] - (a * from[0][0] - b * from[0][1]), to[0][1] - (b * from[0][0] + a * from[0][1])];
        Some(Warp([[a, -b, t[0]], [b, a, t[1]], [0.0, 0.0, 1.0]]))
    }
}

/// The angle (degrees) and length of the line from `a` to `b`.
pub fn angle_and_length(a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    (dy.atan2(dx).to_degrees(), (dx * dx + dy * dy).sqrt())
}

/// `angle` moved by whole turns to be as near `previous` as it can be (so a turning track goes
/// on past 180° rather than jumping back).
pub fn nearest_turn(previous: f64, angle: f64) -> f64 {
    angle + ((previous - angle) / 360.0).round() * 360.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
    }

    #[test]
    fn maps_from_points() {
        let w = Warp([[1.1, 0.1, 5.0], [0.05, 0.9, -3.0], [0.001, 0.0005, 1.0]]);
        let from = [[0.0, 0.0], [100.0, 0.0], [0.0, 100.0], [100.0, 100.0]];
        let got = Warp::perspective(from, from.map(|p| w.apply(p))).unwrap();
        for p in [[30.0, 70.0], [-20.0, 140.0]] {
            assert!(near(got.apply(p), w.apply(p)));
        }
        let slant = Warp([[1.2, 0.3, 4.0], [-0.1, 0.8, 9.0], [0.0, 0.0, 1.0]]);
        let three = [[0.0, 0.0], [50.0, 10.0], [5.0, 60.0]];
        assert!(near(Warp::parallel(three, three.map(|p| slant.apply(p))).unwrap().apply([20.0, 20.0]), slant.apply([20.0, 20.0])));
        // turned a quarter and doubled about the first point, then moved
        let s = Warp::similar([[0.0, 0.0], [10.0, 0.0]], [[100.0, 100.0], [100.0, 120.0]]).unwrap();
        assert!(near(s.apply([0.0, 5.0]), [90.0, 100.0]));
        assert!(Warp::perspective([[0.0, 0.0]; 4], from).is_none(), "all at one point");
        assert_eq!(nearest_turn(170.0, -175.0), 185.0);
        assert_eq!(angle_and_length([0.0, 0.0], [0.0, 2.0]), (90.0, 2.0));
    }
}
