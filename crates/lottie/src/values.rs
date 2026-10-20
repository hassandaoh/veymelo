//! Animated values: our properties (a value, or keys) to Lottie's `{a, k}` and back.
//!
//! A Lottie key holds the value a segment starts from (`s`) and the segment's ease as a cubic in
//! time and progress: `o` leaving this key and `i` arriving at the next, each `x` (a fraction of
//! the segment's time) and `y` (a fraction of the change). A side's speed and influence are
//! that curve: over a segment of `d` seconds changing by `Δv`,
//!
//! ```text
//! o.x = influence_out    o.y = speed_out · influence_out · d / Δv
//! i.x = 1 − influence_in i.y = 1 − speed_in · influence_in · d / Δv
//! ```
//!
//! taken per number of the value, or for a point along its path by the path's length, with the
//! path's tangents as `to` and `ti`. A hold key has `h: 1`. Key times are frames of the layer's
//! own time.

use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{Ease, Gradient, Interp, Keyframe, Kind, ShapePath, Value, segment_length, side_ease, spatial_tangents};
use veymelo_project::{Expression, Property};
use veymelo_time::{FrameRate, Tick};

/// What an export or import carries along: the frame rate, whether expressions go along, and
/// what could not be carried over.
pub struct Ctx {
    pub rate: FrameRate,
    pub expressions: bool,
    pub warnings: Vec<String>,
}

impl Ctx {
    pub fn warn(&mut self, w: impl Into<String>) {
        let w = w.into();
        if !self.warnings.contains(&w) {
            self.warnings.push(w);
        }
    }

    /// The frame (fractional) of a time.
    pub fn frame(&self, t: Tick) -> f64 {
        let f = t.seconds() * self.rate.as_f64();
        if (f - f.round()).abs() < 1e-6 { f.round() } else { round6(f) }
    }

    /// The time of a frame.
    pub fn tick(&self, f: f64) -> Tick {
        Tick::from_seconds(f / self.rate.as_f64())
    }
}

pub fn round6(v: f64) -> f64 {
    if v.is_finite() { (v * 1e6).round() / 1e6 } else { 0.0 }
}

pub fn num(v: f64) -> Json {
    Json::Number(round6(v))
}

pub fn nums(v: &[f64]) -> Json {
    Json::array(v.iter().map(|x| num(*x)))
}

/// A value as Lottie writes it: numbers as themselves, points and colours as lists, a path as
/// `{c, v, i, o}`, a gradient as its stops one after another (colours, then alphas).
pub fn value_json(v: &Value) -> Json {
    match v {
        Value::Number(n) => num(*n),
        Value::Vec2(a) => nums(a),
        Value::Vec3(a) => nums(a),
        Value::Color(c) => nums(&[c.r, c.g, c.b, c.a]),
        Value::Bool(b) => num(*b as u8 as f64),
        Value::Choice(c) => num(*c as f64),
        Value::Path(p) => path_json(p),
        Value::Gradient(g) => nums(&gradient_stops(g)),
        other => Json::from(other.as_text()),
    }
}

pub fn path_json(p: &ShapePath) -> Json {
    let pts = |v: &[[f64; 2]]| Json::array(v.iter().map(|q| nums(q)));
    Json::object([("c", Json::Bool(p.closed)), ("v", pts(&p.points)), ("i", pts(&p.in_tangents)), ("o", pts(&p.out_tangents))])
}

/// A gradient's stops as Lottie keeps them: place, r, g, b for each colour, then place, alpha
/// for each opacity. An empty gradient is white to black.
pub fn gradient_stops(g: &Gradient) -> Vec<f64> {
    let d;
    let g = if g.colors.is_empty() {
        d = Gradient::default();
        &d
    } else {
        g
    };
    let mut out = Vec::new();
    for (t, c) in &g.colors {
        out.extend([*t, c[0], c[1], c[2]]);
    }
    for (t, a) in &g.opacities {
        out.extend([*t, *a]);
    }
    out
}

/// A key's value as Lottie's `s` (always a list; a path in a one-element list).
fn start_json(v: &Value) -> Json {
    match value_json(v) {
        Json::Array(a) => Json::Array(a),
        other => Json::Array(vec![other]),
    }
}

/// A property as a Lottie animated property, its value turned by `conv` (most: as it is).
pub fn prop_json(cx: &mut Ctx, p: &Property, conv: &dyn Fn(&Value) -> Value, label: &str) -> Json {
    let mut o = Map::new();
    if p.keys.is_empty() {
        o.insert("a", 0.0);
        o.insert("k", value_json(&conv(&p.value)));
    } else {
        o.insert("a", 1.0);
        o.insert("k", keys_json(cx, p, conv));
    }
    if let Some(e) = p.expression.as_ref().filter(|e| e.enabled && !e.text.trim().is_empty()) {
        if cx.expressions {
            o.insert("x", e.text.as_str());
        } else {
            cx.warn(format!("{label}: its expression is left out (expressions: true sends them along)"));
        }
    }
    Json::Object(o)
}

fn keys_json(cx: &mut Ctx, p: &Property, conv: &dyn Fn(&Value) -> Value) -> Json {
    // eases are worked out on our own values (a speed and the change it is a share of are in
    // the same units, so the share is the same whatever `conv` turns them into)
    let keys = &p.keys;
    let spatial = p.def.spatial && matches!(p.value, Value::Vec2(_) | Value::Vec3(_));
    let mut out = Vec::with_capacity(keys.len());
    for (i, k) in keys.iter().enumerate() {
        let mut o = Map::new();
        o.insert("t", num(cx.frame(k.time)));
        o.insert("s", start_json(&conv(&k.value)));
        if i + 1 == keys.len() {
            out.push(Json::Object(o));
            continue;
        }
        if k.interp_out == Interp::Hold || !k.value.interpolates() {
            o.insert("h", 1.0);
            out.push(Json::Object(o));
            continue;
        }
        let (ox, oy, ix, iy) = segment(keys, i, spatial);
        o.insert("o", Json::object([("x", nums(&ox)), ("y", nums(&oy))]));
        o.insert("i", Json::object([("x", nums(&ix)), ("y", nums(&iy))]));
        if spatial {
            let n = match conv(&k.value) {
                Value::Vec2(_) => 2,
                _ => 3,
            };
            let (_, to) = spatial_tangents(keys, i);
            let (ti, _) = spatial_tangents(keys, i + 1);
            o.insert("to", nums(&to[..n]));
            o.insert("ti", nums(&ti[..n]));
        }
        out.push(Json::Object(o));
    }
    if keys.iter().any(|k| k.roving) {
        cx.warn("roving keys are written at the times they have now");
    }
    Json::Array(out)
}

/// The ease of segment `i` (to key `i + 1`) for each number: o.x, o.y, i.x, i.y.
fn segment(keys: &[Keyframe], i: usize, spatial: bool) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
    let (a, b) = (&keys[i], &keys[i + 1]);
    let dur = (b.time - a.time).seconds().max(1e-12);
    let dims = if spatial || a.value.dims() == 0 || a.value.dims() != b.value.dims() { 1 } else { a.value.dims() };
    let (mut ox, mut oy, mut ix, mut iy) = (vec![], vec![], vec![], vec![]);
    for d in 0..dims {
        let change = if spatial {
            segment_length(keys, i)
        } else if a.value.dims() == 0 {
            1.0
        } else {
            b.value.component(d) - a.value.component(d)
        };
        let (out, inn) = (side_ease(keys, i, d, true, spatial), side_ease(keys, i + 1, d, false, spatial));
        let (i0, i1) = (out.influence.clamp(Ease::MIN_INFLUENCE, 1.0), inn.influence.clamp(Ease::MIN_INFLUENCE, 1.0));
        let (y0, y1) = if change.abs() > 1e-12 { (out.speed * i0 * dur / change, 1.0 - inn.speed * i1 * dur / change) } else { (0.0, 1.0) };
        ox.push(round6(i0));
        oy.push(round6(y0));
        ix.push(round6(1.0 - i1));
        iy.push(round6(y1));
    }
    (ox, oy, ix, iy)
}

// ---------------------------------------------------------------- back

/// Whether a Lottie property has keys.
fn animated(j: &Json) -> bool {
    j.get("a").and_then(Json::as_f64) == Some(1.0) || j.get("k").and_then(Json::as_array).and_then(|a| a.first()).is_some_and(|f| f.get("t").is_some())
}

fn list(j: &Json) -> Vec<f64> {
    match j {
        Json::Number(n) => vec![*n],
        Json::Array(a) => a.iter().filter_map(Json::as_f64).collect(),
        _ => Vec::new(),
    }
}

/// A Lottie value (a key's `s` or a still `k`) as a value of `kind`; points and numbers take
/// what they need from a list.
pub fn value_of(j: &Json, kind: Kind) -> Option<Value> {
    // a key's value comes in a list of one
    let one = match j {
        Json::Array(a) if a.len() == 1 && !matches!(a[0], Json::Number(_)) => &a[0],
        _ => j,
    };
    let n = list(one);
    Some(match kind {
        Kind::Number => Value::Number(*n.first()?),
        Kind::Bool => Value::Bool(*n.first()? != 0.0),
        Kind::Choice(_) => Value::Choice(*n.first()? as u32),
        Kind::Vec2 => Value::Vec2([*n.first()?, n.get(1).copied().unwrap_or(0.0)]),
        Kind::Vec3 => Value::Vec3([*n.first()?, n.get(1).copied().unwrap_or(0.0), n.get(2).copied().unwrap_or(0.0)]),
        Kind::Color => color_of(one)?,
        Kind::Path => Value::Path(path_of(one)?),
        _ => return None,
    })
}

pub fn path_of(j: &Json) -> Option<ShapePath> {
    let pts = |key: &str| -> Option<Vec<[f64; 2]>> { j.get(key)?.as_array().map(|a| a.iter().map(|q| { let n = list(q); [n.first().copied().unwrap_or(0.0), n.get(1).copied().unwrap_or(0.0)] }).collect()) };
    let points = pts("v")?;
    let n = points.len();
    let fit = |mut v: Vec<[f64; 2]>| {
        v.resize(n, [0.0; 2]);
        v
    };
    Some(ShapePath { in_tangents: fit(pts("i").unwrap_or_default()), out_tangents: fit(pts("o").unwrap_or_default()), points, closed: j.get("c").and_then(Json::as_bool).unwrap_or(false), feather: veymelo_keyframe::Feathers::NONE })
}

/// Lottie's gradient stops (`count` colours, then the alphas) as a gradient.
pub fn gradient_of(stops: &[f64], count: usize) -> Gradient {
    let colors: Vec<(f64, [f64; 3])> = stops.as_chunks::<4>().0.iter().take(count).map(|c| (c[0], [c[1], c[2], c[3]])).collect();
    let rest = stops.get(count * 4..).unwrap_or(&[]);
    let opacities: Vec<(f64, f64)> = rest.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])).collect();
    Gradient { colors, opacities: if opacities.is_empty() { vec![(0.0, 1.0), (1.0, 1.0)] } else { opacities } }
}

/// A Lottie animated property into `p` (its value or keys, and its expression), each value
/// made by `conv` (most: `value_of` for the property's kind).
pub fn prop_from(cx: &mut Ctx, j: &Json, p: &mut Property, conv: &dyn Fn(&Json) -> Option<Value>, label: &str) {
    if !matches!(j, Json::Object(_)) {
        if let Some(v) = conv(j).filter(|v| p.def.check(v).is_ok()) {
            p.value = v;
        }
        return;
    }
    if let Some(x) = j.get("x").and_then(Json::as_str).filter(|x| !x.trim().is_empty()) {
        p.expression = Some(Expression { text: x.to_string(), enabled: true });
    }
    let Some(k) = j.get("k") else { return };
    if !animated(j) {
        match conv(k) {
            Some(v) if p.def.check(&v).is_ok() => p.value = v,
            Some(_) => cx.warn(format!("{label}: a value it cannot hold was left out")),
            None => {}
        }
        return;
    }
    let Some(lkeys) = k.as_array() else { return };
    let spatial = p.def.spatial;
    // the values first (a key without its own `s` ends where the one before it went: `e`)
    let mut keys: Vec<Keyframe> = Vec::new();
    for (n, lk) in lkeys.iter().enumerate() {
        let t = cx.tick(lk.get("t").and_then(Json::as_f64).unwrap_or(0.0));
        let v = lk.get("s").and_then(conv).or_else(|| n.checked_sub(1).and_then(|m| lkeys[m].get("e")).and_then(conv)).or_else(|| keys.last().map(|k: &Keyframe| k.value.clone()));
        let Some(v) = v.filter(|v| p.def.check(v).is_ok()) else {
            cx.warn(format!("{label}: a key it cannot hold was left out"));
            continue;
        };
        let mut key = Keyframe::new(t, v);
        if lk.get("h").and_then(Json::as_f64) == Some(1.0) {
            key.interp_out = Interp::Hold;
        }
        keys.push(key);
    }
    // then how each segment moves
    for i in 0..keys.len().saturating_sub(1) {
        let lk = &lkeys[i];
        if keys[i].interp_out == Interp::Hold {
            keys[i + 1].interp_in = Interp::Hold;
            continue;
        }
        let (Some(o), Some(inn)) = (lk.get("o"), lk.get("i")) else { continue };
        let dur = (keys[i + 1].time - keys[i].time).seconds().max(1e-12);
        let dims = keys[i].value.dims().max(1);
        let n = if spatial || keys[i].value.dims() == 0 { 1 } else { dims };
        if spatial {
            let (to, ti) = (list(lk.get("to").unwrap_or(&Json::Null)), list(lk.get("ti").unwrap_or(&Json::Null)));
            let three = |v: &[f64]| [v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0), v.get(2).copied().unwrap_or(0.0)];
            keys[i].spatial_out = three(&to);
            keys[i + 1].spatial_in = three(&ti);
            keys[i].spatial_auto = false;
            keys[i + 1].spatial_auto = false;
        }
        let change = |d: usize, keys: &[Keyframe]| -> f64 {
            if spatial {
                segment_length(keys, i)
            } else if keys[i].value.dims() == 0 {
                1.0
            } else {
                keys[i + 1].value.component(d) - keys[i].value.component(d)
            }
        };
        let (ox, oy, ix, iy) = (list(o.get("x").unwrap_or(&Json::Null)), list(o.get("y").unwrap_or(&Json::Null)), list(inn.get("x").unwrap_or(&Json::Null)), list(inn.get("y").unwrap_or(&Json::Null)));
        let pick = |v: &[f64], d: usize, def: f64| v.get(d).or(v.first()).copied().unwrap_or(def);
        let (mut outs, mut ins) = (Vec::new(), Vec::new());
        let mut straight = true;
        for d in 0..n {
            let dv = change(d, &keys);
            let (x0, y0, x1, y1) = (pick(&ox, d, 1.0 / 3.0), pick(&oy, d, 1.0 / 3.0), pick(&ix, d, 2.0 / 3.0), pick(&iy, d, 2.0 / 3.0));
            let (i0, i1) = (x0.clamp(Ease::MIN_INFLUENCE, 1.0), (1.0 - x1).clamp(Ease::MIN_INFLUENCE, 1.0));
            outs.push(Ease { speed: y0 * dv / (i0 * dur), influence: i0 });
            ins.push(Ease { speed: (1.0 - y1) * dv / (i1 * dur), influence: i1 });
            // a straight line in time and value is linear
            straight &= (x0 - y0).abs() < 1e-6 && (x1 - y1).abs() < 1e-6;
        }
        if !straight {
            keys[i].interp_out = Interp::Bezier;
            keys[i].ease_out = outs;
            keys[i + 1].interp_in = Interp::Bezier;
            keys[i + 1].ease_in = ins;
        }
    }
    if p.set_keys(keys).is_err() {
        cx.warn(format!("{label}: its keys could not be kept"));
    }
}

// ---------------------------------------------------------------- helpers both ways

/// A value as it is.
pub fn same(v: &Value) -> Value {
    v.clone()
}

/// A number times `k`.
pub fn times(k: f64) -> impl Fn(&Value) -> Value {
    move |v: &Value| match v {
        Value::Bool(b) => Value::Number(*b as u8 as f64),
        Value::Choice(c) => Value::Number(*c as f64 + 1.0),
        v if v.dims() == 1 => Value::Number(v.as_f64() * k),
        v => v.clone(),
    }
}

/// A choice as Lottie counts it, from 1.
pub fn from_one(v: &Value) -> Value {
    match v {
        Value::Choice(c) => Value::Number(*c as f64 + 1.0),
        v => v.clone(),
    }
}

/// The numbers of a Lottie value (one number, or a list; a key's list of one).
pub fn numbers(j: &Json) -> Vec<f64> {
    list(j)
}

pub fn number(j: &Json) -> Option<Value> {
    value_of(j, Kind::Number)
}

/// A Lottie number over `k`.
pub fn over(k: f64) -> impl Fn(&Json) -> Option<Value> {
    move |j: &Json| list(j).first().map(|n| Value::Number(n / k))
}

/// A Lottie number (from 1) as a choice.
pub fn choice_of(j: &Json) -> Option<Value> {
    list(j).first().map(|n| Value::Choice((n.round() - 1.0).max(0.0) as u32))
}

pub fn flag_of(j: &Json) -> Option<Value> {
    list(j).first().map(|n| Value::Bool(*n != 0.0))
}

pub fn vec2_of(j: &Json) -> Option<Value> {
    value_of(j, Kind::Vec2)
}

/// A point with a third number `z` when Lottie gives two.
pub fn vec3_or(z: f64) -> impl Fn(&Json) -> Option<Value> {
    move |j: &Json| {
        let n = list(j);
        Some(Value::Vec3([*n.first()?, n.get(1).copied().unwrap_or(0.0), n.get(2).copied().unwrap_or(z)]))
    }
}

/// A colour: three or four numbers 0 to 1 (or, in old files, 0 to 255).
pub fn color_of(j: &Json) -> Option<Value> {
    let n = list(j);
    if n.len() < 3 {
        return None;
    }
    let k = if n[..3].iter().any(|v| *v > 1.0) { 255.0 } else { 1.0 };
    let a = n.get(3).map_or(1.0, |a| if k > 1.0 { a / 255.0 } else { *a });
    Some(Value::Color(veymelo_color::rgba(n[0] / k, n[1] / k, n[2] / k, a.clamp(0.0, 1.0))))
}

pub fn path_value(j: &Json) -> Option<Value> {
    value_of(j, Kind::Path)
}
