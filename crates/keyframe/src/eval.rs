//! The value keys give at a moment: in time, eased per number; for points (positions, anchor
//! points) along a curved path whose speed the eases set.
//!
//! On a path, a key's tangents are its own, or with auto on, a sixth of the way from the key
//! before it to the key after it (an end key toward its one neighbour), so the path passes
//! smoothly through the keys. The point moves along the path by its length: a linear side keeps
//! one speed, an eased one the speed and influence it was given (in pixels a second).
//!
//! A side with auto Bezier takes its speed from its neighbours: the slope from the key before
//! to the key after, or at an end, the slope to its one neighbour (so two auto keys move
//! straight).

use veymelo_time::Tick;

use crate::{Ease, Interp, Keyframe, Value, eased};

/// The value the keys give at `t` (the first key's before it, the last key's after it).
/// `spatial`: points move along curved paths between keys.
pub fn evaluate(keys: &[Keyframe], t: Tick, spatial: bool) -> Option<Value> {
    let first = keys.first()?;
    if t <= first.time {
        return Some(first.value.clone());
    }
    let last = keys.last()?;
    if t >= last.time {
        return Some(last.value.clone());
    }
    let i = keys.partition_point(|k| k.time <= t) - 1;
    Some(between(keys, i, t, spatial))
}

fn is_point(v: &Value) -> bool {
    matches!(v, Value::Vec2(_) | Value::Vec3(_))
}

fn point(v: &Value) -> [f64; 3] {
    v.as_vec3()
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn norm(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn secs(keys: &[Keyframe], a: usize, b: usize) -> f64 {
    (keys[b].time - keys[a].time).seconds()
}

/// The value between key `i` and the next at `t`.
fn between(keys: &[Keyframe], i: usize, t: Tick, spatial: bool) -> Value {
    let (a, b) = (&keys[i], &keys[i + 1]);
    if a.interp_out == Interp::Hold || !a.value.interpolates() {
        return a.value.clone();
    }
    let dur = secs(keys, i, i + 1);
    let x = ((t - a.time).seconds() / dur).clamp(0.0, 1.0);
    let straight_both = a.interp_out == Interp::Linear && b.interp_in != Interp::Bezier;
    if spatial && is_point(&a.value) && is_point(&b.value) {
        let curve = segment(keys, i);
        let len = curve.length();
        let f = if straight_both || !(len > 1e-12 && len.is_finite()) {
            x
        } else {
            eased(0.0, len, dur, side(keys, i, 0, true, true), side(keys, i + 1, 0, false, true), x) / len
        };
        let p = if curve.is_straight() { add(curve.p[0], sub(curve.p[3], curve.p[0]).map(|v| v * f)) } else { curve.at_length(f * len) };
        return a.value.with_components(|k| p[k]);
    }
    if straight_both {
        return a.value.lerp(&b.value, x);
    }
    if a.value.dims() == 0 || a.value.dims() != b.value.dims() {
        // a path eases its progress, 0 to 1 (speed is progress a second)
        let f = eased(0.0, 1.0, dur, side(keys, i, 0, true, false), side(keys, i + 1, 0, false, false), x);
        return a.value.lerp(&b.value, f);
    }
    a.value.with_components(|d| eased(a.value.component(d), b.value.component(d), dur, side(keys, i, d, true, false), side(keys, i + 1, d, false, false), x))
}

/// How fast number `d` of the value (or the point along its path) changes on segment `seg`
/// when it moves straight: its change over the segment's time.
fn straight_speed(keys: &[Keyframe], seg: usize, d: usize, spatial: bool) -> f64 {
    let dur = secs(keys, seg, seg + 1).max(1e-12);
    if spatial {
        segment_length(keys, seg) / dur
    } else if keys[seg].value.dims() == 0 {
        1.0 / dur
    } else {
        (keys[seg + 1].value.component(d) - keys[seg].value.component(d)) / dur
    }
}

/// The ease key `i` leaves with (`out`) or arrives with, for number `d` (a point's along its
/// path, `spatial`), as the value moves by it: a linear side's straight speed, an auto side's
/// from its neighbours, else its own.
pub fn side_ease(keys: &[Keyframe], i: usize, d: usize, out: bool, spatial: bool) -> Ease {
    side(keys, i, d, out, spatial)
}

fn side(keys: &[Keyframe], i: usize, d: usize, out: bool, spatial: bool) -> Ease {
    let k = &keys[i];
    let interp = if out { k.interp_out } else { k.interp_in };
    // the segment this side is on
    let seg = if out { i } else { i - 1 };
    if interp != Interp::Bezier {
        return Ease { speed: straight_speed(keys, seg, d, spatial), influence: 1.0 / 3.0 };
    }
    if k.auto_bezier {
        let speed = if i > 0 && i + 1 < keys.len() {
            let dt = secs(keys, i - 1, i + 1).max(1e-12);
            if spatial {
                (segment_length(keys, i - 1) + segment_length(keys, i)) / dt
            } else if k.value.dims() == 0 {
                1.0 / dt
            } else {
                (keys[i + 1].value.component(d) - keys[i - 1].value.component(d)) / dt
            }
        } else {
            straight_speed(keys, seg, d, spatial)
        };
        return Ease { speed, influence: 1.0 / 6.0 };
    }
    if out { k.ease_out_of(d) } else { k.ease_in_of(d) }
}

/// Key `i`'s tangents on the path, in and out (relative to its value).
pub fn spatial_tangents(keys: &[Keyframe], i: usize) -> ([f64; 3], [f64; 3]) {
    let Some(k) = keys.get(i) else { return ([0.0; 3], [0.0; 3]) };
    if !k.spatial_auto {
        return (k.spatial_in, k.spatial_out);
    }
    if keys.len() < 2 {
        return ([0.0; 3], [0.0; 3]);
    }
    let before = point(&keys[i.saturating_sub(1)].value);
    let after = point(&keys[(i + 1).min(keys.len() - 1)].value);
    let out = sub(after, before).map(|v| v / 6.0);
    (out.map(|v| -v), out)
}

/// The cubic of the path from key `i` to the next.
fn segment(keys: &[Keyframe], i: usize) -> Curve {
    let (p0, p3) = (point(&keys[i].value), point(&keys[i + 1].value));
    let (_, out) = spatial_tangents(keys, i);
    let (inn, _) = spatial_tangents(keys, i + 1);
    Curve::new([p0, add(p0, out), add(p3, inn), p3])
}

/// The length of the path from key `i` to the next.
pub fn segment_length(keys: &[Keyframe], i: usize) -> f64 {
    if i + 1 >= keys.len() || !is_point(&keys[i].value) {
        return 0.0;
    }
    segment(keys, i).length()
}

/// A cubic Bezier in space, with its length measured along it.
struct Curve {
    p: [[f64; 3]; 4],
    /// The length from the start to u = k / CELLS.
    lengths: Vec<f64>,
}

const CELLS: usize = 64;

/// Gauss-Legendre: five points and their weights on -1…1.
const GAUSS: [(f64, f64); 5] = [
    (0.0, 0.568_888_888_888_888_9),
    (-0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
    (0.906_179_845_938_664, 0.236_926_885_056_189_1),
];

impl Curve {
    fn new(p: [[f64; 3]; 4]) -> Curve {
        let mut c = Curve { p, lengths: Vec::with_capacity(CELLS + 1) };
        let mut acc = 0.0;
        c.lengths.push(0.0);
        for k in 0..CELLS {
            acc += c.arc(k as f64 / CELLS as f64, (k + 1) as f64 / CELLS as f64);
            c.lengths.push(acc);
        }
        c
    }

    fn at(&self, u: f64) -> [f64; 3] {
        let v = 1.0 - u;
        let (a, b, c, d) = (v * v * v, 3.0 * v * v * u, 3.0 * v * u * u, u * u * u);
        let p = &self.p;
        [0, 1, 2].map(|k| a * p[0][k] + b * p[1][k] + c * p[2][k] + d * p[3][k])
    }

    /// How fast the point moves along the curve as u changes.
    fn speed(&self, u: f64) -> f64 {
        let v = 1.0 - u;
        let p = &self.p;
        norm([0, 1, 2].map(|k| 3.0 * v * v * (p[1][k] - p[0][k]) + 6.0 * v * u * (p[2][k] - p[1][k]) + 3.0 * u * u * (p[3][k] - p[2][k])))
    }

    /// The length between u = `a` and u = `b`.
    fn arc(&self, a: f64, b: f64) -> f64 {
        let (m, h) = ((a + b) / 2.0, (b - a) / 2.0);
        GAUSS.iter().map(|(x, w)| w * self.speed(m + h * x)).sum::<f64>() * h
    }

    fn length(&self) -> f64 {
        *self.lengths.last().unwrap_or(&0.0)
    }

    /// Both handles lie on the line from start to end: the curve is that line.
    fn is_straight(&self) -> bool {
        let on = |c: [f64; 3]| {
            let (d, v) = (sub(self.p[3], self.p[0]), sub(c, self.p[0]));
            let dd = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            if dd <= 1e-24 {
                return norm(v) <= 1e-12;
            }
            let t = (v[0] * d[0] + v[1] * d[1] + v[2] * d[2]) / dd;
            let off = sub(v, d.map(|x| x * t));
            (0.0..=1.0).contains(&t) && off[0] * off[0] + off[1] * off[1] + off[2] * off[2] <= dd * 1e-18
        };
        on(self.p[1]) && on(self.p[2])
    }

    /// The point `s` along the curve.
    fn at_length(&self, s: f64) -> [f64; 3] {
        let len = self.length();
        if !(len > 0.0 && len.is_finite()) {
            return self.p[0];
        }
        let s = s.clamp(0.0, len);
        let k = self.lengths.partition_point(|&l| l < s).clamp(1, CELLS);
        let (l0, l1) = (self.lengths[k - 1], self.lengths[k]);
        let (u0, u1) = ((k - 1) as f64 / CELLS as f64, k as f64 / CELLS as f64);
        let mut u = u0 + if l1 > l0 { (s - l0) / (l1 - l0) } else { 0.0 } * (u1 - u0);
        // a few Newton steps on the true length, so the speed is smooth, not a step per cell
        for _ in 0..4 {
            let v = self.speed(u);
            if v <= 1e-12 {
                break;
            }
            let err = l0 + self.arc(u0, u) - s;
            u = (u - err / v).clamp(u0, u1);
            if err.abs() <= 1e-12 * len.max(1.0) {
                break;
            }
        }
        self.at(u)
    }
}

/// How fast each number of the value changes at `t` (units a second).
pub fn velocity(keys: &[Keyframe], t: Tick, spatial: bool) -> Vec<f64> {
    let h = Tick::from_seconds(0.0005);
    let (Some(a), Some(b)) = (evaluate(keys, t - h, spatial), evaluate(keys, t + h, spatial)) else { return Vec::new() };
    let dt = 2.0 * h.seconds();
    (0..a.dims()).map(|d| (b.component(d) - a.component(d)) / dt).collect()
}

/// How fast the value changes at `t`, all its numbers together.
pub fn speed(keys: &[Keyframe], t: Tick, spatial: bool) -> f64 {
    velocity(keys, t, spatial).iter().map(|v| v * v).sum::<f64>().sqrt()
}

/// Give roving keys their times: between two keys that do not rove, each roving key goes where
/// the value would be at one speed (by path length, or for other values by how far it changes).
/// The first and last keys never rove. Whether any time changed.
pub fn retime_roving(keys: &mut [Keyframe], spatial: bool) -> bool {
    let n = keys.len();
    if n < 3 {
        keys.iter_mut().for_each(|k| k.roving = false);
        return false;
    }
    keys[0].roving = false;
    keys[n - 1].roving = false;
    let mut changed = false;
    let mut a = 0;
    while a + 1 < n {
        let b = (a + 1..n).find(|&j| !keys[j].roving).expect("the last key does not rove");
        if b > a + 1 {
            let mut along = vec![0.0];
            for i in a..b {
                let step = if spatial && is_point(&keys[i].value) {
                    segment_length(keys, i)
                } else {
                    (0..keys[i].value.dims()).map(|d| (keys[i + 1].value.component(d) - keys[i].value.component(d)).powi(2)).sum::<f64>().sqrt()
                };
                along.push(along.last().expect("not empty") + step);
            }
            let total = *along.last().expect("not empty");
            let (t0, t1) = (keys[a].time.0 as i128, keys[b].time.0 as i128);
            for (o, i) in (a + 1..b).enumerate() {
                let f = if total > 1e-9 { along[o + 1] / total } else { (o + 1) as f64 / (b - a) as f64 };
                // strictly between its neighbours, so no two keys share a moment
                let lo = keys[i - 1].time.0 as i128 + 1;
                let hi = t1 - (b - i) as i128;
                let mut at = t0 + ((t1 - t0) as f64 * f).round() as i128;
                if lo <= hi {
                    at = at.clamp(lo, hi);
                }
                let at = Tick(at.clamp(i64::MIN as i128, i64::MAX as i128) as i64);
                if at != keys[i].time {
                    keys[i].time = at;
                    changed = true;
                }
                // speed passes straight through a roving key
                keys[i].interp_in = Interp::Linear;
                keys[i].interp_out = Interp::Linear;
            }
        }
        a = b;
    }
    changed
}

/// Turn keys around in time between the first and the last: what happened first happens last.
pub fn reverse(keys: &mut [Keyframe]) {
    let (Some(first), Some(last)) = (keys.first().map(|k| k.time.0 as i128), keys.last().map(|k| k.time.0 as i128)) else { return };
    for k in keys.iter_mut() {
        k.time = Tick((first + last - k.time.0 as i128).clamp(i64::MIN as i128, i64::MAX as i128) as i64);
        std::mem::swap(&mut k.interp_in, &mut k.interp_out);
        std::mem::swap(&mut k.ease_in, &mut k.ease_out);
        std::mem::swap(&mut k.spatial_in, &mut k.spatial_out);
    }
    keys.reverse();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: f64) -> Tick {
        Tick::from_seconds(x)
    }

    fn p2(x: f64, y: f64) -> Value {
        Value::Vec2([x, y])
    }

    fn at(keys: &[Keyframe], t: f64) -> [f64; 2] {
        evaluate(keys, s(t), true).unwrap().as_vec2()
    }

    #[test]
    fn a_straight_path_is_walked_at_one_speed() {
        let keys = vec![Keyframe::new(s(0.0), p2(0.0, 0.0)), Keyframe::new(s(1.0), p2(300.0, 400.0))];
        let p = at(&keys, 0.5);
        assert_eq!(p, [150.0, 200.0]);
        assert!((segment_length(&keys, 0) - 500.0).abs() < 1e-9);
    }

    #[test]
    fn auto_tangents_pass_smoothly_through_the_keys() {
        let keys = vec![Keyframe::new(s(0.0), p2(0.0, 0.0)), Keyframe::new(s(1.0), p2(100.0, 100.0)), Keyframe::new(s(2.0), p2(200.0, 0.0))];
        assert_eq!(at(&keys, 1.0), [100.0, 100.0]);
        let m = at(&keys, 0.5);
        assert!((m[0] - m[1]).abs() > 1.0, "the path bends: {m:?}");
        // the key's tangents are a sixth of the way from the key before to the key after
        let (inn, out) = spatial_tangents(&keys, 1);
        assert_eq!((inn, out), ([-200.0 / 6.0, 0.0, 0.0], [200.0 / 6.0, 0.0, 0.0]));
        // an end key points to its one neighbour
        assert_eq!(spatial_tangents(&keys, 0).1, [100.0 / 6.0, 100.0 / 6.0, 0.0]);
        // and the motion is level through the middle key, as it rounds the top
        let v = velocity(&keys, s(1.0), true);
        assert!(v[1].abs() < 1.0, "{v:?}");
    }

    #[test]
    fn linear_keys_on_a_curve_keep_one_speed() {
        let mut a = Keyframe::new(s(0.0), p2(0.0, 0.0));
        let mut b = Keyframe::new(s(2.0), p2(400.0, 0.0));
        a.spatial_auto = false;
        b.spatial_auto = false;
        a.spatial_out = [50.0, -300.0, 0.0];
        b.spatial_in = [80.0, 250.0, 0.0];
        let keys = vec![a, b];
        let len = segment_length(&keys, 0);
        assert!(len > 400.0);
        for i in 1..200 {
            let v = speed(&keys, s(i as f64 * 0.01), true);
            assert!((v - len / 2.0).abs() < 1e-3 * len, "at {}: {v} vs {}", i as f64 * 0.01, len / 2.0);
        }
    }

    #[test]
    fn eased_motion_on_a_path_starts_and_stops_slowly() {
        let keys = vec![Keyframe::new(s(0.0), p2(0.0, 0.0)).eased(), Keyframe::new(s(1.0), p2(100.0, 50.0)).eased()];
        assert!(speed(&keys, s(0.01), true) < 20.0);
        assert!(speed(&keys, s(0.5), true) > 150.0);
        let mid = at(&keys, 0.5);
        assert!((mid[0] - 50.0).abs() < 1e-6 && (mid[1] - 25.0).abs() < 1e-6, "{mid:?}");
    }

    #[test]
    fn two_auto_bezier_keys_move_straight() {
        let mut a = Keyframe::new(s(0.0), Value::Number(0.0)).eased();
        let mut b = Keyframe::new(s(2.0), Value::Number(100.0)).eased();
        a.auto_bezier = true;
        b.auto_bezier = true;
        let keys = vec![a, b];
        for i in 0..=20 {
            let t = i as f64 * 0.1;
            assert!((evaluate(&keys, s(t), false).unwrap().as_f64() - 50.0 * t).abs() < 1e-6, "{t}");
        }
    }

    #[test]
    fn an_auto_key_in_the_middle_is_smooth() {
        let mut keys = vec![Keyframe::new(s(0.0), Value::Number(0.0)), Keyframe::new(s(1.0), Value::Number(50.0)).eased(), Keyframe::new(s(3.0), Value::Number(250.0))];
        keys[1].auto_bezier = true;
        // the speed into the key and out of it is the slope from the key before to the key after
        let (before, after) = (velocity(&keys, s(0.999), false)[0], velocity(&keys, s(1.001), false)[0]);
        assert!((before - after).abs() < 1.0, "{before} {after}");
        assert!((after - 250.0 / 3.0).abs() < 1.0, "{after}");
    }

    #[test]
    fn roving_keys_get_times_for_one_speed() {
        let mut keys = vec![Keyframe::new(s(0.0), p2(0.0, 0.0)), Keyframe::new(s(0.2), p2(300.0, 0.0)), Keyframe::new(s(2.0), p2(400.0, 0.0))];
        keys.iter_mut().for_each(|k| k.spatial_auto = false);
        keys[1].roving = true;
        assert!(retime_roving(&mut keys, true));
        assert!((keys[1].time.seconds() - 1.5).abs() < 1e-6, "300 of 400 px: three quarters of the way");
        assert!((speed(&keys, s(0.5), true) - 200.0).abs() < 1.0 && (speed(&keys, s(1.8), true) - 200.0).abs() < 1.0);
        keys[0].roving = true;
        retime_roving(&mut keys, true);
        assert!(!keys[0].roving, "the first key cannot rove");
        // keys that would land on one moment are kept apart
        let mut flat: Vec<Keyframe> = [0.0, 0.0, 40.0, 40.0].iter().enumerate().map(|(i, x)| Keyframe::new(s(i as f64), p2(*x, 0.0))).collect();
        flat.iter_mut().for_each(|k| k.spatial_auto = false);
        flat[1].roving = true;
        flat[2].roving = true;
        retime_roving(&mut flat, true);
        assert!(flat.windows(2).all(|w| w[0].time < w[1].time));
    }

    #[test]
    fn reversed_keys_play_backwards() {
        let mut keys = vec![Keyframe::new(s(1.0), Value::Number(0.0)).eased(), Keyframe::new(s(2.0), Value::Number(5.0)), Keyframe::new(s(4.0), Value::Number(10.0)).hold()];
        reverse(&mut keys);
        assert_eq!(keys.iter().map(|k| k.time.seconds()).collect::<Vec<_>>(), [1.0, 3.0, 4.0]);
        assert_eq!(keys[0].value, Value::Number(10.0));
        assert_eq!(keys[2].interp_in, Interp::Bezier);
        assert_eq!(keys[0].interp_out, Interp::Hold);
    }

    #[test]
    fn huge_or_broken_paths_do_not_fail() {
        for path in [[[-1.7e308, 0.0], [1.7e308, 0.0], [-1.7e308, 5.0]], [[0.0, 0.0], [f64::NAN, 10.0], [20.0, 0.0]]] {
            let keys: Vec<Keyframe> = path.iter().enumerate().map(|(i, &v)| Keyframe::new(s(i as f64), Value::Vec2(v)).eased()).collect();
            for f in 0..=20 {
                assert!(evaluate(&keys, s(f as f64 * 0.1), true).is_some());
            }
        }
    }
}
