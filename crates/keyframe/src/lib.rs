//! Values that change over time: keyframes, and how a value goes from one to the next.
//!
//! Between two keys a value moves in a straight line (linear), along a curve (Bezier) or not at
//! all until the next key (hold). A Bezier side is set like the velocity of a key in compositing
//! tools: the *speed* the value leaves or arrives with (its units a second) and the *influence*,
//! how far toward the neighbouring key that speed reaches (a fraction of the time between them,
//! 0.001 to 1). A linear side next to a Bezier one acts as a Bezier side on the straight line.
//!
//! Each number of a value (red, green, blue and alpha of a colour, the sides of a scale) is
//! eased on its own; a point (a position) moves along a curved path instead, see [`evaluate`].

mod eval;
pub mod text;
mod value;

pub use eval::{evaluate, retime_roving, reverse, segment_length, side_ease, spatial_tangents, speed, velocity};
pub use text::{Caps, CharStyle, Composer, Direction, Justify, Kerning, ParaStyle, Position, Run, TextDoc};
pub use value::{FeatherPoint, Feathers, Gradient, Kind, ShapePath, Value, describe};

use veymelo_json::{Map, Value as Json};
use veymelo_time::Tick;

/// How a value moves on one side of a keyframe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Interp {
    #[default]
    Linear,
    Bezier,
    Hold,
}

impl Interp {
    pub const ALL: [Interp; 3] = [Interp::Linear, Interp::Bezier, Interp::Hold];

    pub fn id(self) -> &'static str {
        match self {
            Interp::Linear => "linear",
            Interp::Bezier => "bezier",
            Interp::Hold => "hold",
        }
    }

    pub fn from_id(id: &str) -> Option<Interp> {
        Interp::ALL.into_iter().find(|i| i.id() == id)
    }
}

/// The speed a value leaves or reaches a key with (its units a second) and how far that speed
/// reaches: `influence` is a fraction (0.001 to 1) of the time to the neighbouring key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ease {
    pub speed: f64,
    pub influence: f64,
}

impl Ease {
    /// Easy ease: stopped at the key, the stop reaching a third of the way.
    pub const EASY: Ease = Ease { speed: 0.0, influence: 1.0 / 3.0 };
    /// The least influence a side can have.
    pub const MIN_INFLUENCE: f64 = 0.001;
}

impl Default for Ease {
    fn default() -> Ease {
        Ease::EASY
    }
}

/// A value at a moment, and how it leaves and arrives.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub time: Tick,
    pub value: Value,
    pub interp_in: Interp,
    pub interp_out: Interp,
    /// The ease of each number of the value, or one for all of them; none means easy ease.
    pub ease_in: Vec<Ease>,
    pub ease_out: Vec<Ease>,
    /// Bezier sides whose speed comes from the neighbouring keys.
    pub auto_bezier: bool,
    /// Bezier sides that keep one speed through the key.
    pub continuous: bool,
    /// The curve a position takes through the key: tangents relative to the key's value,
    /// used when `spatial_auto` is off (on, they come from the neighbouring keys).
    pub spatial_in: [f64; 3],
    pub spatial_out: [f64; 3],
    pub spatial_auto: bool,
    pub spatial_continuous: bool,
    /// A key whose time follows from its neighbours so the value moves at one speed through it.
    pub roving: bool,
    /// A colour label, 0 for none, up to 16.
    pub label: u8,
}

impl Keyframe {
    /// A linear key.
    pub fn new(time: Tick, value: Value) -> Keyframe {
        Keyframe {
            time,
            value,
            interp_in: Interp::Linear,
            interp_out: Interp::Linear,
            ease_in: Vec::new(),
            ease_out: Vec::new(),
            auto_bezier: false,
            continuous: false,
            spatial_in: [0.0; 3],
            spatial_out: [0.0; 3],
            spatial_auto: true,
            spatial_continuous: false,
            roving: false,
            label: 0,
        }
    }

    /// Easy ease on both sides.
    pub fn eased(mut self) -> Keyframe {
        self.interp_in = Interp::Bezier;
        self.interp_out = Interp::Bezier;
        self.ease_in.clear();
        self.ease_out.clear();
        self
    }

    /// Held: the value stays until the next key.
    pub fn hold(mut self) -> Keyframe {
        self.interp_in = Interp::Hold;
        self.interp_out = Interp::Hold;
        self
    }

    /// The ease number `d` of the value arrives with.
    pub fn ease_in_of(&self, d: usize) -> Ease {
        self.ease_in.get(d).or(self.ease_in.first()).copied().unwrap_or_default()
    }

    /// The ease number `d` of the value leaves with.
    pub fn ease_out_of(&self, d: usize) -> Ease {
        self.ease_out.get(d).or(self.ease_out.first()).copied().unwrap_or_default()
    }

    /// As JSON, writing only what is not the default. Time is in seconds.
    pub fn to_json(&self, kind: Kind) -> Json {
        let mut o = Map::new();
        o.insert("t", self.time.seconds());
        o.insert("v", self.value.to_json(kind));
        if self.interp_in != Interp::Linear {
            o.insert("in", self.interp_in.id());
        }
        if self.interp_out != Interp::Linear {
            o.insert("out", self.interp_out.id());
        }
        if let Some(e) = ease_json(&self.ease_in) {
            o.insert("easeIn", e);
        }
        if let Some(e) = ease_json(&self.ease_out) {
            o.insert("easeOut", e);
        }
        if self.auto_bezier {
            o.insert("autoBezier", true);
        }
        if self.continuous {
            o.insert("continuous", true);
        }
        if !self.spatial_auto {
            let n = if matches!(self.value, Value::Vec2(_)) { 2 } else { 3 };
            o.insert("spatialAuto", false);
            o.insert("spatialIn", Json::array(self.spatial_in[..n].iter().copied()));
            o.insert("spatialOut", Json::array(self.spatial_out[..n].iter().copied()));
        }
        if self.spatial_continuous {
            o.insert("spatialContinuous", true);
        }
        if self.roving {
            o.insert("roving", true);
        }
        if self.label != 0 {
            o.insert("label", self.label as f64);
        }
        Json::Object(o)
    }

    /// The key `j` writes for a property of kind `kind`, or what is wrong with it.
    pub fn from_json(kind: Kind, j: &Json) -> Result<Keyframe, String> {
        const FIELDS: [&str; 14] =
            ["t", "v", "in", "out", "easeIn", "easeOut", "autoBezier", "continuous", "spatialAuto", "spatialIn", "spatialOut", "spatialContinuous", "roving", "label"];
        let o = j.as_object().ok_or_else(|| format!("a keyframe is an object like {{\"t\": 1, \"v\": …}}, got {}", describe(j)))?;
        for key in o.keys() {
            if !FIELDS.contains(&key) {
                return Err(format!("a keyframe has no \"{key}\" (it has {})", FIELDS.join(", ")));
            }
        }
        let t = o.get("t").ok_or("a keyframe needs \"t\", its time in seconds")?;
        let t = t.as_f64().filter(|t| t.is_finite() && t.abs() <= 1e9).ok_or_else(|| format!("\"t\" is a time in seconds, got {}", describe(t)))?;
        let v = o.get("v").ok_or("a keyframe needs \"v\", its value")?;
        let mut key = Keyframe::new(Tick::from_seconds(t), Value::from_json(kind, v).map_err(|e| format!("\"v\": {e}"))?);
        let flag = |name: &str, default: bool| -> Result<bool, String> {
            match o.get(name) {
                None => Ok(default),
                Some(b) => b.as_bool().ok_or_else(|| format!("\"{name}\" should be true or false, got {}", describe(b))),
            }
        };
        let interp = |name: &str| -> Result<Interp, String> {
            match o.get(name) {
                None => Ok(Interp::Linear),
                Some(i) => i.as_str().and_then(Interp::from_id).ok_or_else(|| format!("\"{name}\" is \"linear\", \"bezier\" or \"hold\", got {}", describe(i))),
            }
        };
        let dims = key.value.dims().max(1);
        key.interp_in = interp("in")?;
        key.interp_out = interp("out")?;
        key.ease_in = ease_from_json(o.get("easeIn"), dims).map_err(|e| format!("\"easeIn\": {e}"))?;
        key.ease_out = ease_from_json(o.get("easeOut"), dims).map_err(|e| format!("\"easeOut\": {e}"))?;
        key.auto_bezier = flag("autoBezier", false)?;
        key.continuous = flag("continuous", false)?;
        key.spatial_auto = flag("spatialAuto", true)?;
        key.spatial_continuous = flag("spatialContinuous", false)?;
        key.roving = flag("roving", false)?;
        for (name, tangent) in [("spatialIn", &mut key.spatial_in), ("spatialOut", &mut key.spatial_out)] {
            if let Some(v) = o.get(name) {
                let Some(Value::Vec3(p)) = Value::from_json(Kind::Vec3, v).ok() else { return Err(format!("\"{name}\" is a tangent [x, y] or [x, y, z], got {}", describe(v))) };
                *tangent = p;
            }
        }
        if let Some(l) = o.get("label") {
            key.label = l.as_u64().filter(|l| *l <= 16).ok_or_else(|| format!("\"label\" is a colour label from 0 to 16, got {}", describe(l)))? as u8;
        }
        Ok(key)
    }
}

/// An ease list as JSON: nothing for easy ease, one object when every number eases the same.
fn ease_json(list: &[Ease]) -> Option<Json> {
    let one = |e: &Ease| Json::object([("speed", e.speed), ("influence", e.influence)]);
    let first = list.first()?;
    if list.iter().all(|e| *e == Ease::EASY) {
        None
    } else if list.iter().all(|e| e == first) {
        Some(one(first))
    } else {
        Some(Json::array(list.iter().map(one)))
    }
}

/// An ease list from JSON (one object, or one for each of the value's `dims` numbers), kept in
/// the shortest form that means the same.
fn ease_from_json(j: Option<&Json>, dims: usize) -> Result<Vec<Ease>, String> {
    let Some(j) = j else { return Ok(Vec::new()) };
    let one = |e: &Json| -> Result<Ease, String> {
        let o = e.as_object().ok_or_else(|| format!("an ease is {{\"speed\": 0, \"influence\": 0.33}}, got {}", describe(e)))?;
        if let Some(k) = o.keys().find(|k| *k != "speed" && *k != "influence") {
            return Err(format!("an ease has no \"{k}\" (it has speed, influence)"));
        }
        let speed = match o.get("speed") {
            None => 0.0,
            Some(s) => s.as_f64().filter(|s| s.is_finite()).ok_or_else(|| format!("\"speed\" is a number, got {}", describe(s)))?,
        };
        let influence = match o.get("influence") {
            None => Ease::EASY.influence,
            Some(i) => i.as_f64().filter(|i| (0.0..=1.0).contains(i)).ok_or_else(|| format!("\"influence\" is a fraction from 0 to 1, got {}", describe(i)))?,
        };
        Ok(Ease { speed, influence })
    };
    let list = match j {
        Json::Array(a) if a.len() == dims => a.iter().map(one).collect::<Result<Vec<_>, _>>()?,
        Json::Array(a) => return Err(format!("a list of eases needs one for each of the value's {dims} numbers, got {}", a.len())),
        e => vec![one(e)?],
    };
    Ok(if list.iter().all(|e| *e == Ease::EASY) {
        Vec::new()
    } else if list.iter().all(|e| *e == list[0]) {
        vec![list[0]]
    } else {
        list
    })
}

/// Keys as JSON, in time order.
pub fn keys_to_json(keys: &[Keyframe], kind: Kind) -> Json {
    Json::array(keys.iter().map(|k| k.to_json(kind)))
}

/// Keys from JSON, put in time order; two keys may not share a moment.
pub fn keys_from_json(kind: Kind, j: &Json) -> Result<Vec<Keyframe>, String> {
    let a = j.as_array().ok_or_else(|| format!("keys are a list of keyframes, got {}", describe(j)))?;
    let mut keys = a.iter().enumerate().map(|(i, k)| Keyframe::from_json(kind, k).map_err(|e| format!("key {i}: {e}"))).collect::<Result<Vec<_>, _>>()?;
    keys.sort_by_key(|k| k.time);
    if let Some(w) = keys.windows(2).find(|w| w[0].time == w[1].time) {
        return Err(format!("two keys at {} s", w[0].time.seconds()));
    }
    Ok(keys)
}

/// The value `x` of the way (0 to 1) through the time of a curve from `v0` to `v1` that lasts
/// `dur` seconds, leaving with `out` and arriving with `inn`.
///
/// The curve is a cubic Bezier in time and value: it starts at (0, v0) heading along the
/// leaving speed for `out.influence` of the time, and ends at (1, v1) arriving along its speed
/// over `inn.influence`. The two influences may add up to more than the whole time.
pub fn eased(v0: f64, v1: f64, dur: f64, out: Ease, inn: Ease, x: f64) -> f64 {
    let i0 = out.influence.clamp(Ease::MIN_INFLUENCE, 1.0);
    let i1 = inn.influence.clamp(Ease::MIN_INFLUENCE, 1.0);
    let u = time_to_u(i0, 1.0 - i1, x);
    cubic(v0, v0 + out.speed * i0 * dur, v1 - inn.speed * i1 * dur, v1, u)
}

fn cubic(p0: f64, p1: f64, p2: f64, p3: f64, u: f64) -> f64 {
    let v = 1.0 - u;
    v * v * v * p0 + 3.0 * v * v * u * p1 + 3.0 * v * u * u * p2 + u * u * u * p3
}

fn cubic_slope(p0: f64, p1: f64, p2: f64, p3: f64, u: f64) -> f64 {
    let v = 1.0 - u;
    3.0 * v * v * (p1 - p0) + 6.0 * v * u * (p2 - p1) + 3.0 * u * u * (p3 - p2)
}

/// Where on the curve (0 to 1) the time is `x`, for time handles at `x1` and `x2`. With both
/// in 0 to 1, time only ever moves forward along the curve, so there is one answer.
fn time_to_u(x1: f64, x2: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    if x1 == 1.0 && x2 == 0.0 {
        // full influence on both sides stops time for a moment halfway, x = 1/2 + 4 (u - 1/2)³;
        // solving it by search there would lose most of the digits
        return 0.5 + ((x - 0.5) / 4.0).cbrt();
    }
    // Newton's steps, kept inside a bracket that halves when a step would leave it
    let (mut lo, mut hi, mut u) = (0.0, 1.0, x);
    for _ in 0..100 {
        let f = cubic(0.0, x1, x2, 1.0, u) - x;
        if f == 0.0 {
            return u;
        }
        if f < 0.0 {
            lo = u;
        } else {
            hi = u;
        }
        let n = u - f / cubic_slope(0.0, x1, x2, 1.0, u);
        let next = if n > lo && n < hi { n } else { 0.5 * (lo + hi) };
        if (next - u).abs() < 1e-15 {
            return next;
        }
        u = next;
    }
    u
}

/// Put `key` in its place among `keys`; a key already at that moment takes the new value and
/// keeps how it moves. Returns where it went.
pub fn set_key(keys: &mut Vec<Keyframe>, key: Keyframe) -> usize {
    match keys.binary_search_by_key(&key.time, |k| k.time) {
        Ok(i) => {
            keys[i].value = key.value;
            i
        }
        Err(i) => {
            keys.insert(i, key);
            i
        }
    }
}

/// The key exactly at `t`.
pub fn key_at(keys: &[Keyframe], t: Tick) -> Option<usize> {
    keys.binary_search_by_key(&t, |k| k.time).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: f64) -> Tick {
        Tick::from_seconds(x)
    }

    fn num(keys: &[Keyframe], t: f64) -> f64 {
        evaluate(keys, s(t), false).unwrap().as_f64()
    }

    fn two(a: Keyframe, b: Keyframe) -> Vec<Keyframe> {
        vec![a, b]
    }

    #[test]
    fn linear_hold_and_the_ends() {
        let keys = two(Keyframe::new(s(0.0), Value::Number(0.0)), Keyframe::new(s(2.0), Value::Number(100.0)));
        assert_eq!(num(&keys, 1.0), 50.0);
        assert_eq!(num(&keys, -1.0), 0.0);
        assert_eq!(num(&keys, 3.0), 100.0);
        assert_eq!(evaluate(&[], s(0.0), false), None);
        let held = two(Keyframe::new(s(0.0), Value::Number(0.0)).hold(), Keyframe::new(s(2.0), Value::Number(100.0)));
        assert_eq!(num(&held, 1.999), 0.0);
        assert_eq!(num(&held, 2.0), 100.0);
        // a held key holds what comes after it; the way into it is still a line
        let into = two(Keyframe::new(s(0.0), Value::Number(0.0)), Keyframe::new(s(2.0), Value::Number(100.0)).hold());
        assert_eq!(num(&into, 0.5), 25.0);
        // a text has no in-between
        let words = two(Keyframe::new(s(0.0), Value::Text("a".into())), Keyframe::new(s(1.0), Value::Text("b".into())));
        assert_eq!(evaluate(&words, s(0.9), false), Some(Value::Text("a".into())));
    }

    #[test]
    fn easy_ease_is_symmetric_and_slow_at_the_ends() {
        let keys = two(Keyframe::new(s(0.0), Value::Number(0.0)).eased(), Keyframe::new(s(1.0), Value::Number(100.0)).eased());
        assert!((num(&keys, 0.5) - 50.0).abs() < 1e-9);
        for t in [0.1, 0.2, 0.3, 0.4] {
            assert!((num(&keys, t) + num(&keys, 1.0 - t) - 100.0).abs() < 1e-9, "symmetric at {t}");
        }
        assert!(num(&keys, 0.05) < 1.0, "starts slowly: {}", num(&keys, 0.05));
        assert!(num(&keys, 0.6) - num(&keys, 0.4) > 20.0 * 1.2, "fast in the middle");
    }

    #[test]
    fn a_curve_with_the_straight_speed_is_the_line() {
        let mut a = Keyframe::new(s(0.0), Value::Number(0.0));
        let mut b = Keyframe::new(s(2.0), Value::Number(100.0));
        a.interp_out = Interp::Bezier;
        b.interp_in = Interp::Bezier;
        a.ease_out = vec![Ease { speed: 50.0, influence: 1.0 / 3.0 }];
        b.ease_in = vec![Ease { speed: 50.0, influence: 1.0 / 3.0 }];
        let keys = two(a, b);
        for i in 0..=20 {
            let t = i as f64 * 0.1;
            assert!((num(&keys, t) - 50.0 * t).abs() < 1e-9, "{t}");
        }
    }

    /// The curve, found by stepping along it finely instead of solving for it.
    fn by_walking(v0: f64, v1: f64, dur: f64, out: Ease, inn: Ease, x: f64) -> f64 {
        let (i0, i1) = (out.influence, inn.influence);
        let (mut best, mut gap) = (0.0, f64::MAX);
        for k in 0..=200_000 {
            let u = k as f64 / 200_000.0;
            let d = (cubic(0.0, i0, 1.0 - i1, 1.0, u) - x).abs();
            if d < gap {
                gap = d;
                best = cubic(v0, v0 + out.speed * i0 * dur, v1 - inn.speed * i1 * dur, v1, u);
            }
        }
        best
    }

    #[test]
    fn eases_land_where_the_curve_is() {
        for (out, inn) in [
            (Ease { speed: 0.0, influence: 0.8 }, Ease { speed: 0.0, influence: 0.8 }),
            (Ease { speed: 20.0, influence: 0.8 }, Ease { speed: 80.0, influence: 0.6 }),
            (Ease { speed: 300.0, influence: 0.1 }, Ease { speed: -40.0, influence: 0.95 }),
            (Ease { speed: 0.0, influence: 1.0 }, Ease { speed: 0.0, influence: 0.001 }),
        ] {
            for x in [0.01, 0.1, 0.25, 0.4, 0.5, 0.75, 0.99] {
                let (a, b) = (eased(0.0, 100.0, 1.0, out, inn, x), by_walking(0.0, 100.0, 1.0, out, inn, x));
                assert!((a - b).abs() < 0.05, "{out:?} {inn:?} at {x}: {a} vs {b}");
            }
        }
    }

    /// The curve at time `x`, found by halving the stretch of the curve the time lies in
    /// (time only ever moves forward along it, even when the influences add up past 100 %).
    fn by_halving(v0: f64, v1: f64, dur: f64, out: Ease, inn: Ease, x: f64) -> f64 {
        let (i0, i1) = (out.influence, inn.influence);
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            if cubic(0.0, i0, 1.0 - i1, 1.0, mid) < x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        cubic(v0, v0 + out.speed * i0 * dur, v1 - inn.speed * i1 * dur, v1, 0.5 * (lo + hi))
    }

    #[test]
    fn overlapping_influences_keep_both_handles_whole() {
        // influences adding up past 100 % are not scaled down: each handle keeps its own reach,
        // and the time still moves forward along the curve
        for (from, to, dur, out, inn) in [
            (0.0, 100.0, 1.0, (0.0, 0.9), (0.0, 0.7)),
            (10.0, -50.0, 2.0, (35.0, 0.65), (-120.0, 0.85)),
            (-3.0, 3.0, 0.5, (0.0, 1.0), (40.0, 0.95)),
            (200.0, 0.0, 1.5, (-300.0, 0.55), (10.0, 0.75)),
        ] {
            let (out, inn) = (Ease { speed: out.0, influence: out.1 }, Ease { speed: inn.0, influence: inn.1 });
            let mut a = Keyframe::new(s(0.0), Value::Number(from));
            let mut b = Keyframe::new(s(dur), Value::Number(to));
            a.interp_out = Interp::Bezier;
            b.interp_in = Interp::Bezier;
            a.ease_out = vec![out];
            b.ease_in = vec![inn];
            let keys = two(a, b);
            for k in 1..20 {
                let x = k as f64 / 20.0;
                let (got, want) = (num(&keys, x * dur), by_halving(from, to, dur, out, inn, x));
                assert!((got - want).abs() < 1e-9, "{out:?} {inn:?} at {x}: {got} vs {want}");
            }
        }
    }

    #[test]
    fn full_influence_keeps_its_digits_halfway() {
        let full = Ease { speed: 0.0, influence: 1.0 };
        let mut last = 0.0;
        for x in [0.5 - 1e-6, 0.5 - 1e-9, 0.5 - 1e-12, 0.5, 0.5 + 1e-12, 0.5 + 1e-9, 0.5 + 1e-6] {
            let u = 0.5 + ((x - 0.5) / 4.0_f64).cbrt();
            let want = cubic(0.0, 0.0, 1.0, 1.0, u);
            let got = eased(0.0, 1.0, 1.0, full, full, x);
            assert!((got - want).abs() < 1e-12, "{x}: {got} vs {want}");
            assert!(got >= last, "never goes back");
            last = got;
        }
    }

    #[test]
    fn eased_values_stay_between_their_keys() {
        // with no speed at either end, no influence can push the value past its keys
        let steps = [0.001, 0.05, 0.2, 1.0 / 3.0, 0.5, 0.8, 0.999, 1.0];
        for i0 in steps {
            for i1 in steps {
                let mut prev = 0.0;
                for k in 0..=100 {
                    let v = eased(0.0, 10.0, 1.0, Ease { speed: 0.0, influence: i0 }, Ease { speed: 0.0, influence: i1 }, k as f64 / 100.0);
                    assert!((-1e-9..=10.0 + 1e-9).contains(&v) && v >= prev - 1e-9, "{i0} {i1} at {k}: {v}");
                    prev = v;
                }
            }
        }
    }

    #[test]
    fn each_number_eases_on_its_own() {
        let mut a = Keyframe::new(s(0.0), Value::Vec2([0.0, 0.0]));
        let b = Keyframe::new(s(1.0), Value::Vec2([100.0, 100.0]));
        a.interp_out = Interp::Bezier;
        a.ease_out = vec![Ease::EASY, Ease { speed: 100.0, influence: 1.0 / 3.0 }];
        let p = evaluate(&two(a, b), s(0.25), false).unwrap().as_vec2();
        assert!(p[0] < 20.0, "x starts from rest: {p:?}");
        assert!((p[1] - 25.0).abs() < 1e-9, "y keeps the straight speed: {p:?}");
    }

    #[test]
    fn paths_and_colours_move() {
        let a = Value::Path(ShapePath::polygon(&[[0.0, 0.0], [10.0, 0.0]], false));
        let b = Value::Path(ShapePath::polygon(&[[0.0, 20.0], [10.0, 20.0]], false));
        let keys = two(Keyframe::new(s(0.0), a).eased(), Keyframe::new(s(1.0), b).eased());
        let mid = evaluate(&keys, s(0.5), false).unwrap();
        assert!((mid.as_path().unwrap().points[0][1] - 10.0).abs() < 1e-9);
        let early = evaluate(&keys, s(0.1), false).unwrap();
        assert!(early.as_path().unwrap().points[0][1] < 2.0, "eased");
        let c = |r| Value::Color(veymelo_color::rgba(r, 0.0, 0.0, 1.0));
        let keys = two(Keyframe::new(s(0.0), c(0.0)), Keyframe::new(s(1.0), c(1.0)));
        assert_eq!(evaluate(&keys, s(0.25), false), Some(c(0.25)));
    }

    #[test]
    fn setting_a_key_keeps_its_place_and_its_way() {
        let mut keys = Vec::new();
        assert_eq!(set_key(&mut keys, Keyframe::new(s(1.0), Value::Number(1.0)).eased()), 0);
        assert_eq!(set_key(&mut keys, Keyframe::new(s(0.0), Value::Number(0.0))), 0);
        assert_eq!(set_key(&mut keys, Keyframe::new(s(1.0), Value::Number(5.0))), 1);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[1].value, Value::Number(5.0));
        assert_eq!(keys[1].interp_in, Interp::Bezier);
        assert_eq!(key_at(&keys, s(1.0)), Some(1));
        assert_eq!(key_at(&keys, s(0.5)), None);
    }

    #[test]
    fn keys_in_json_write_only_what_differs() {
        let kind = Kind::Number;
        let mut b = Keyframe::new(s(1.5), Value::Number(100.0)).eased();
        b.ease_out = vec![Ease { speed: 40.0, influence: 0.5 }];
        b.label = 3;
        let keys = vec![Keyframe::new(s(0.0), Value::Number(0.0)), b];
        let j = keys_to_json(&keys, kind);
        assert_eq!(j.to_json(), r#"[{"t":0,"v":0},{"t":1.5,"v":100,"in":"bezier","out":"bezier","easeOut":{"speed":40,"influence":0.5},"label":3}]"#);
        assert_eq!(keys_from_json(kind, &j), Ok(keys));
        // per-number eases, spatial tangents, and a frame time that is not a round number of seconds
        let mut k = Keyframe::new(Tick::from_units(1001, 30000), Value::Vec2([1.0, 2.0]));
        k.interp_out = Interp::Bezier;
        k.ease_out = vec![Ease { speed: 1.0, influence: 0.2 }, Ease { speed: -3.0, influence: 0.9 }];
        k.spatial_auto = false;
        k.spatial_out = [10.0, -4.0, 0.0];
        k.roving = true;
        let j = keys_to_json(std::slice::from_ref(&k), Kind::Vec2);
        assert!(j.to_json().contains(r#""spatialOut":[10,-4]"#), "{}", j.to_json());
        assert_eq!(keys_from_json(Kind::Vec2, &j), Ok(vec![k]));
        // read in any order, and easy eases written out mean the same as none
        let j = Json::parse(r#"[{"t": 2, "v": 1, "in": "bezier", "easeIn": [{"speed": 0, "influence": 0.3333333333333333}]}, {"t": 0, "v": 0}]"#).unwrap();
        let keys = keys_from_json(kind, &j).unwrap();
        assert_eq!(keys[0].time, Tick::ZERO);
        assert!(keys[1].ease_in.is_empty());
    }

    #[test]
    fn mistakes_in_keys_say_where() {
        let read = |kind: Kind, text: &str| keys_from_json(kind, &Json::parse(text).unwrap()).unwrap_err();
        assert_eq!(read(Kind::Number, r#"[{"t": 0, "v": 0}, {"t": 0, "v": 1}]"#), "two keys at 0 s");
        assert_eq!(read(Kind::Number, r#"[{"t": 0}]"#), "key 0: a keyframe needs \"v\", its value");
        assert_eq!(read(Kind::Number, r#"[{"t": 0, "v": 0}, {"t": 1, "v": [1]}]"#), "key 1: \"v\": expected a number, got an array of 1");
        assert_eq!(read(Kind::Number, r#"[{"t": 0, "v": 0, "ease": 1}]"#).split(" (").next().unwrap(), "key 0: a keyframe has no \"ease\"");
        assert_eq!(read(Kind::Number, r#"[{"t": 0, "v": 0, "in": "smooth"}]"#), "key 0: \"in\" is \"linear\", \"bezier\" or \"hold\", got \"smooth\"");
        assert_eq!(read(Kind::Vec2, r#"[{"t": 0, "v": [0, 0], "easeIn": [{}, {}, {}]}]"#), "key 0: \"easeIn\": a list of eases needs one for each of the value's 2 numbers, got 3");
        assert_eq!(read(Kind::Number, r#"[{"t": 0, "v": 0, "easeOut": {"influence": 2}}]"#), "key 0: \"easeOut\": \"influence\" is a fraction from 0 to 1, got 2");
        assert_eq!(read(Kind::Number, r#"[{"t": "1s", "v": 0}]"#), "key 0: \"t\" is a time in seconds, got \"1s\"");
    }
}
