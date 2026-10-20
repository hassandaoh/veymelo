//! The value a property holds at a moment, and how each kind is written in JSON.

use veymelo_color::Rgba;
use veymelo_json::Value as Json;

/// What kind of value a property holds. A property's kind comes from its definition in code,
/// so a file only has to write the value itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Number,
    Vec2,
    Vec3,
    Color,
    Bool,
    /// One of a list of named options (written by name).
    Choice(&'static [&'static str]),
    Text,
    /// Another layer, by its id (or none).
    Layer,
    Path,
    Gradient,
    /// A text layer's document (its words and their look).
    Doc,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Number => "number",
            Kind::Vec2 => "vec2",
            Kind::Vec3 => "vec3",
            Kind::Color => "color",
            Kind::Bool => "bool",
            Kind::Choice(_) => "choice",
            Kind::Text => "text",
            Kind::Layer => "layer",
            Kind::Path => "path",
            Kind::Gradient => "gradient",
            Kind::Doc => "text document",
        }
    }

    /// How a value of this kind is written, for people and agents reading a command's help.
    pub fn shape(self) -> String {
        match self {
            Kind::Number => "a number".into(),
            Kind::Vec2 => "[x, y]".into(),
            Kind::Vec3 => "[x, y, z] (or [x, y], z = 0)".into(),
            Kind::Color => "\"#rrggbb\", \"#rrggbbaa\" or [r, g, b, a?] from 0 to 1".into(),
            Kind::Bool => "true or false".into(),
            Kind::Choice(options) => format!("one of {}", options.iter().map(|o| format!("\"{o}\"")).collect::<Vec<_>>().join(", ")),
            Kind::Text => "a string".into(),
            Kind::Layer => "a layer id, or null".into(),
            Kind::Path => "{\"points\": [[x, y]…], \"inTangents\"?: […], \"outTangents\"?: […], \"closed\"?: true, \"feather\"?: [{\"piece\", \"at\", \"width\", \"tension\"?}…]}".into(),
            Kind::Gradient => "{\"colors\": [[place, colour]…], \"opacities\"?: [[place, alpha]…]} (places 0 to 1), or a list of colours spread evenly".into(),
            Kind::Doc => "the words as a string, or {\"text\", \"font\"?, \"weight\"?, \"size\"?, \"fill\"?, \"stroke\"?, \"strokeWidth\"?, \"tracking\"?, \"leading\"?, \"justify\"?, \"box\"?…}".into(),
        }
    }
}

/// A Bezier path: points, each with tangents relative to it, open or closed; and, for a mask,
/// the points along it that soften its edge by different amounts (they go with its keys).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShapePath {
    pub points: Vec<[f64; 2]>,
    pub in_tangents: Vec<[f64; 2]>,
    pub out_tangents: Vec<[f64; 2]>,
    pub closed: bool,
    pub feather: Feathers,
}

/// A path's softening points: kept out of the way while there are none, as most paths have
/// (so every value stays small); otherwise a list like any other.
#[derive(Clone, Debug, Default)]
// one pointer where a list would be three words: every value of every property is a word
// smaller for it
#[allow(clippy::box_collection)]
pub struct Feathers(Option<Box<Vec<FeatherPoint>>>);

impl Feathers {
    pub const NONE: Feathers = Feathers(None);
}

impl std::ops::Deref for Feathers {
    type Target = Vec<FeatherPoint>;
    fn deref(&self) -> &Vec<FeatherPoint> {
        static EMPTY: Vec<FeatherPoint> = Vec::new();
        self.0.as_deref().unwrap_or(&EMPTY)
    }
}

impl std::ops::DerefMut for Feathers {
    fn deref_mut(&mut self) -> &mut Vec<FeatherPoint> {
        self.0.get_or_insert_with(Default::default)
    }
}

impl PartialEq for Feathers {
    fn eq(&self, o: &Feathers) -> bool {
        self[..] == o[..]
    }
}

impl FromIterator<FeatherPoint> for Feathers {
    fn from_iter<I: IntoIterator<Item = FeatherPoint>>(it: I) -> Feathers {
        let v: Vec<FeatherPoint> = it.into_iter().collect();
        Feathers(if v.is_empty() { None } else { Some(Box::new(v)) })
    }
}

impl<'a> IntoIterator for &'a Feathers {
    type Item = &'a FeatherPoint;
    type IntoIter = std::slice::Iter<'a, FeatherPoint>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut Feathers {
    type Item = &'a mut FeatherPoint;
    type IntoIter = std::slice::IterMut<'a, FeatherPoint>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl From<Vec<FeatherPoint>> for Feathers {
    fn from(v: Vec<FeatherPoint>) -> Feathers {
        v.into_iter().collect()
    }
}

/// A point on a mask's outline that softens its edge there: on the piece from point `piece` to
/// the next, at `at` on it (the curve's own measure, 0 at its start to 1 at its end), softening
/// `width` pixels outward (below zero: inward), the softness between such points going over
/// smoothly (`tension` 0) or straight (1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FeatherPoint {
    pub piece: usize,
    pub at: f64,
    pub width: f64,
    pub tension: f64,
}

impl ShapePath {
    /// Straight lines through `points`.
    pub fn polygon(points: &[[f64; 2]], closed: bool) -> ShapePath {
        ShapePath { points: points.to_vec(), in_tangents: vec![[0.0; 2]; points.len()], out_tangents: vec![[0.0; 2]; points.len()], closed, feather: Feathers::NONE }
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    fn lerp(&self, o: &ShapePath, t: f64) -> ShapePath {
        if self.len() != o.len() || self.closed != o.closed {
            return if t < 1.0 { self.clone() } else { o.clone() };
        }
        let mix = |a: &[[f64; 2]], b: &[[f64; 2]]| a.iter().zip(b).map(|(p, q)| [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]).collect();
        // the softening points go between too, where both have them on the same pieces
        let same = self.feather.len() == o.feather.len() && self.feather.iter().zip(&o.feather).all(|(a, b)| a.piece == b.piece);
        let feather = if same {
            let m = |a: f64, b: f64| a + (b - a) * t;
            self.feather.iter().zip(&o.feather).map(|(a, b)| FeatherPoint { piece: a.piece, at: m(a.at, b.at), width: m(a.width, b.width), tension: m(a.tension, b.tension) }).collect()
        } else if t < 1.0 {
            self.feather.clone()
        } else {
            o.feather.clone()
        };
        ShapePath { points: mix(&self.points, &o.points), in_tangents: mix(&self.in_tangents, &o.in_tangents), out_tangents: mix(&self.out_tangents, &o.out_tangents), closed: self.closed, feather }
    }

    /// How many pieces it has (one from each point to the next, and back to the first if closed).
    pub fn pieces(&self) -> usize {
        if self.closed { self.points.len() } else { self.points.len().saturating_sub(1) }
    }

    /// The tangents RotoBezier gives it: each point's along the line through the points either
    /// side of it, a sixth of the way between them (the ends of an open path none).
    pub fn roto_smoothed(&self) -> ShapePath {
        let n = self.points.len();
        let mut out = self.clone();
        out.in_tangents = vec![[0.0; 2]; n];
        out.out_tangents = vec![[0.0; 2]; n];
        for i in 0..n {
            let (before, after) = match (self.closed, i) {
                (true, _) => ((i + n - 1) % n, (i + 1) % n),
                (false, 0) => continue,
                (false, i) if i + 1 == n => continue,
                (false, i) => (i - 1, i + 1),
            };
            let (a, b) = (self.points[before], self.points[after]);
            let d = [(b[0] - a[0]) / 6.0, (b[1] - a[1]) / 6.0];
            out.out_tangents[i] = d;
            out.in_tangents[i] = [-d[0], -d[1]];
        }
        out
    }

    /// Its softening points kept where they were after a point was put in at `i` (splitting the
    /// piece before it at `split` along it, if it was split).
    pub fn feather_after_insert(&mut self, i: usize, split: Option<f64>) {
        for f in &mut self.feather {
            if i > 0 && f.piece == i - 1 {
                if let Some(s) = split.filter(|s| *s > 0.0 && *s < 1.0) {
                    if f.at < s {
                        f.at /= s;
                    } else {
                        f.piece += 1;
                        f.at = (f.at - s) / (1.0 - s);
                    }
                }
            } else if f.piece >= i {
                f.piece += 1;
            }
        }
    }

    /// Its softening points after point `i` was taken out, the path having had `pieces_before`
    /// pieces: the two pieces either side of it are one now (a point on the first goes to its
    /// first half, on the second to its second half); on an open path's end its piece goes.
    pub fn feather_after_remove(&mut self, i: usize, pieces_before: usize) {
        let closed = self.closed;
        // the piece the two become, and the two (none for an open path's end)
        let joined: Option<(usize, usize, usize)> = match (closed, i) {
            (true, 0) => Some((pieces_before - 1, 0, pieces_before.saturating_sub(2))),
            (true, i) => Some((i - 1, i, i - 1)),
            (false, 0) => None,
            (false, i) if i == pieces_before => None,
            (false, i) => Some((i - 1, i, i - 1)),
        };
        self.feather.retain_mut(|f| {
            match joined {
                Some((first, _, to)) if f.piece == first => {
                    f.piece = to;
                    f.at *= 0.5;
                }
                Some((_, second, to)) if f.piece == second => {
                    f.piece = to;
                    f.at = 0.5 + f.at * 0.5;
                }
                // an open path's end: the piece with it goes
                None if (i == 0 && f.piece == 0) || (i > 0 && f.piece + 1 == i) => return false,
                _ if f.piece > i || (i == 0 && f.piece > 0) => f.piece -= 1,
                _ => {}
            }
            true
        });
        let n = self.pieces();
        self.feather.retain(|f| f.piece < n);
    }
}

impl FeatherPoint {
    pub fn to_json(&self) -> Json {
        let mut o = veymelo_json::Map::new();
        o.insert("piece", self.piece as f64);
        o.insert("at", self.at);
        o.insert("width", self.width);
        if self.tension != 0.0 {
            o.insert("tension", self.tension);
        }
        Json::Object(o)
    }

    pub fn from_json(j: &Json) -> Result<FeatherPoint, String> {
        let o = j.as_object().ok_or("a softening point is {piece, at, width, tension?}")?;
        for key in o.keys() {
            if !matches!(key, "piece" | "at" | "width" | "tension") {
                return Err(format!("a softening point has no \"{key}\" (it has piece, at, width, tension)"));
            }
        }
        let num = |k: &str| o.get(k).and_then(Json::as_f64).filter(|v| v.is_finite());
        let piece = num("piece").filter(|p| *p >= 0.0 && p.fract() == 0.0).ok_or("a softening point's \"piece\" is a whole number from 0")?;
        let at = num("at").filter(|a| (0.0..=1.0).contains(a)).ok_or("a softening point's \"at\" is from 0 to 1")?;
        let width = num("width").ok_or("a softening point needs \"width\", pixels (below zero: inward)")?;
        let tension = match o.get("tension") {
            None => 0.0,
            Some(_) => num("tension").filter(|t| (0.0..=1.0).contains(t)).ok_or("a softening point's \"tension\" is from 0 to 1")?,
        };
        Ok(FeatherPoint { piece: piece as usize, at, width, tension })
    }
}

impl ShapePath {
}

/// A colour gradient: colour stops and, apart from them, opacity stops, each at a place from 0
/// (its start) to 1 (its end), in order. With no colour stops it is white to black.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub colors: Vec<(f64, [f64; 3])>,
    pub opacities: Vec<(f64, f64)>,
}

impl Default for Gradient {
    /// White to black, solid.
    fn default() -> Gradient {
        Gradient { colors: vec![(0.0, [1.0; 3]), (1.0, [0.0; 3])], opacities: vec![(0.0, 1.0), (1.0, 1.0)] }
    }
}

/// The value at `t` of stops in order (the end ones held past the ends).
fn stop_at<T: Copy>(stops: &[(f64, T)], t: f64, mix: impl Fn(T, T, f64) -> T) -> Option<T> {
    let first = stops.first()?;
    if t <= first.0 {
        return Some(first.1);
    }
    for w in stops.windows(2) {
        if t <= w[1].0 {
            let span = (w[1].0 - w[0].0).max(1e-12);
            return Some(mix(w[0].1, w[1].1, (t - w[0].0) / span));
        }
    }
    stops.last().map(|s| s.1)
}

impl Gradient {
    /// Its colour and alpha at `t` (0 to 1).
    pub fn at(&self, t: f64) -> [f64; 4] {
        if self.colors.is_empty() {
            return Gradient::default().at(t);
        }
        let c = stop_at(&self.colors, t, |a, b, f| [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * f)).unwrap_or([1.0; 3]);
        let a = stop_at(&self.opacities, t, |a, b, f| a + (b - a) * f).unwrap_or(1.0);
        [c[0], c[1], c[2], a]
    }

    fn lerp(&self, o: &Gradient, t: f64) -> Gradient {
        if self.colors.len() != o.colors.len() || self.opacities.len() != o.opacities.len() {
            return if t < 1.0 { self.clone() } else { o.clone() };
        }
        let colors = self.colors.iter().zip(&o.colors).map(|(a, b)| (a.0 + (b.0 - a.0) * t, [0, 1, 2].map(|k| a.1[k] + (b.1[k] - a.1[k]) * t))).collect();
        let opacities = self.opacities.iter().zip(&o.opacities).map(|(a, b)| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)).collect();
        Gradient { colors, opacities }
    }
}

/// A property's value at one moment.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Number(f64),
    Vec2([f64; 2]),
    Vec3([f64; 3]),
    Color(Rgba),
    Bool(bool),
    /// The index of an option in the property's list.
    Choice(u32),
    Text(String),
    Layer(Option<u64>),
    Path(ShapePath),
    Gradient(Gradient),
    Doc(Box<crate::text::TextDoc>),
}

impl Value {
    /// Whether this value is of kind `kind`.
    pub fn is(&self, kind: Kind) -> bool {
        match (self, kind) {
            (Value::Choice(i), Kind::Choice(options)) => (*i as usize) < options.len(),
            (Value::Number(_), Kind::Number)
            | (Value::Vec2(_), Kind::Vec2)
            | (Value::Vec3(_), Kind::Vec3)
            | (Value::Color(_), Kind::Color)
            | (Value::Bool(_), Kind::Bool)
            | (Value::Text(_), Kind::Text)
            | (Value::Layer(_), Kind::Layer)
            | (Value::Path(_), Kind::Path)
            | (Value::Gradient(_), Kind::Gradient)
            | (Value::Doc(_), Kind::Doc)
            | (Value::Text(_), Kind::Doc) => true,
            _ => false,
        }
    }

    /// How many numbers it is made of, each eased on its own (0 for values that are not numbers).
    pub fn dims(&self) -> usize {
        match self {
            Value::Number(_) => 1,
            Value::Vec2(_) => 2,
            Value::Vec3(_) => 3,
            Value::Color(_) => 4,
            _ => 0,
        }
    }

    /// Number `i` of it.
    pub fn component(&self, i: usize) -> f64 {
        match self {
            Value::Number(v) => *v,
            Value::Vec2(v) => v.get(i).copied().unwrap_or(0.0),
            Value::Vec3(v) => v.get(i).copied().unwrap_or(0.0),
            Value::Color(c) => [c.r, c.g, c.b, c.a].get(i).copied().unwrap_or(0.0),
            _ => 0.0,
        }
    }

    /// The same kind of value, number `i` taken from `f(i)`.
    pub fn with_components(&self, f: impl Fn(usize) -> f64) -> Value {
        match self {
            Value::Number(_) => Value::Number(f(0)),
            Value::Vec2(_) => Value::Vec2([f(0), f(1)]),
            Value::Vec3(_) => Value::Vec3([f(0), f(1), f(2)]),
            Value::Color(_) => Value::Color(Rgba { r: f(0), g: f(1), b: f(2), a: f(3) }),
            v => v.clone(),
        }
    }

    /// Whether it can move smoothly from one keyframe to the next (the others jump at the next key).
    pub fn interpolates(&self) -> bool {
        self.dims() > 0 || matches!(self, Value::Path(_) | Value::Gradient(_))
    }

    /// The value `t` of the way (0 to 1) toward `o`.
    pub fn lerp(&self, o: &Value, t: f64) -> Value {
        match (self, o) {
            (Value::Path(a), Value::Path(b)) => Value::Path(a.lerp(b, t)),
            (Value::Gradient(a), Value::Gradient(b)) => Value::Gradient(a.lerp(b, t)),
            _ if self.dims() > 0 && self.dims() == o.dims() => self.with_components(|i| {
                let (a, b) = (self.component(i), o.component(i));
                a + (b - a) * t
            }),
            _ => {
                if t < 1.0 {
                    self.clone()
                } else {
                    o.clone()
                }
            }
        }
    }

    pub fn as_f64(&self) -> f64 {
        self.component(0)
    }

    pub fn as_vec2(&self) -> [f64; 2] {
        [self.component(0), self.component(1)]
    }

    pub fn as_vec3(&self) -> [f64; 3] {
        [self.component(0), self.component(1), self.component(2)]
    }

    pub fn as_color(&self) -> Rgba {
        match self {
            Value::Color(c) => *c,
            _ => Rgba::BLACK,
        }
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            v => v.as_f64() != 0.0,
        }
    }

    pub fn as_choice(&self) -> u32 {
        match self {
            Value::Choice(i) => *i,
            _ => 0,
        }
    }

    pub fn as_text(&self) -> &str {
        match self {
            Value::Text(s) => s,
            _ => "",
        }
    }

    pub fn as_layer(&self) -> Option<u64> {
        match self {
            Value::Layer(l) => *l,
            _ => None,
        }
    }

    /// A text document: a document's, or plain words with the look a document starts with.
    pub fn doc(&self) -> Option<crate::text::TextDoc> {
        match self {
            Value::Doc(d) => Some((**d).clone()),
            Value::Text(t) => Some(crate::text::TextDoc::of(t)),
            _ => None,
        }
    }

    pub fn as_gradient(&self) -> Option<&Gradient> {
        match self {
            Value::Gradient(g) => Some(g),
            _ => None,
        }
    }

    pub fn as_path(&self) -> Option<&ShapePath> {
        match self {
            Value::Path(p) => Some(p),
            _ => None,
        }
    }

    /// As JSON, the way `kind` writes it (a choice by its option's name).
    pub fn to_json(&self, kind: Kind) -> Json {
        match self {
            Value::Number(v) => Json::Number(*v),
            Value::Vec2(v) => Json::from(*v),
            Value::Vec3(v) => Json::from(*v),
            Value::Color(c) => color_json(*c),
            Value::Bool(b) => Json::Bool(*b),
            Value::Choice(i) => match kind {
                Kind::Choice(options) => options.get(*i as usize).map_or(Json::Number(*i as f64), |o| Json::from(*o)),
                _ => Json::Number(*i as f64),
            },
            Value::Text(s) => Json::from(s),
            Value::Layer(l) => l.map_or(Json::Null, |id| Json::Number(id as f64)),
            Value::Path(p) => path_json(p),
            Value::Gradient(g) => gradient_json(g),
            Value::Doc(d) => d.to_json(),
        }
    }

    /// The value of kind `kind` that `j` writes, or what is wrong with it.
    pub fn from_json(kind: Kind, j: &Json) -> Result<Value, String> {
        let wrong = || format!("expected {}, got {}", kind.shape(), describe(j));
        let num = |j: &Json| j.as_f64().filter(|v| v.is_finite());
        let nums = |j: &Json, n: usize| -> Option<Vec<f64>> {
            let a = j.as_array()?;
            if a.len() != n {
                return None;
            }
            a.iter().map(num).collect()
        };
        Ok(match kind {
            Kind::Number => Value::Number(num(j).ok_or_else(wrong)?),
            Kind::Vec2 => {
                let v = nums(j, 2).ok_or_else(wrong)?;
                Value::Vec2([v[0], v[1]])
            }
            Kind::Vec3 => {
                let v = nums(j, 3).or_else(|| nums(j, 2).map(|v| vec![v[0], v[1], 0.0])).ok_or_else(wrong)?;
                Value::Vec3([v[0], v[1], v[2]])
            }
            Kind::Color => Value::Color(color_from_json(j).ok_or_else(wrong)?),
            Kind::Bool => Value::Bool(j.as_bool().ok_or_else(wrong)?),
            Kind::Choice(options) => {
                let name = j.as_str().ok_or_else(wrong)?;
                Value::Choice(options.iter().position(|o| *o == name).ok_or_else(|| format!("\"{name}\" is not {}", kind.shape()))? as u32)
            }
            Kind::Text => Value::Text(j.as_str().ok_or_else(wrong)?.to_string()),
            Kind::Layer => match j {
                Json::Null => Value::Layer(None),
                _ => Value::Layer(Some(j.as_u64().ok_or_else(wrong)?)),
            },
            Kind::Path => Value::Path(path_from_json(j).map_err(|e| if e.is_empty() { wrong() } else { e })?),
            Kind::Gradient => Value::Gradient(gradient_from_json(j).map_err(|e| if e.is_empty() { wrong() } else { e })?),
            Kind::Doc => {
                // a document with nothing but words is kept as its words
                let d = crate::text::TextDoc::from_json(j)?;
                if d == crate::text::TextDoc::of(&d.text) { Value::Text(d.text) } else { Value::Doc(Box::new(d)) }
            }
        })
    }

    /// The value every property of `kind` starts from when its definition gives none.
    pub fn zero(kind: Kind) -> Value {
        match kind {
            Kind::Number => Value::Number(0.0),
            Kind::Vec2 => Value::Vec2([0.0; 2]),
            Kind::Vec3 => Value::Vec3([0.0; 3]),
            Kind::Color => Value::Color(Rgba::BLACK),
            Kind::Bool => Value::Bool(false),
            Kind::Choice(_) => Value::Choice(0),
            Kind::Text => Value::Text(String::new()),
            Kind::Layer => Value::Layer(None),
            Kind::Path => Value::Path(ShapePath::default()),
            Kind::Gradient => Value::Gradient(Gradient::default()),
            Kind::Doc => Value::Text(String::new()),
        }
    }
}

/// A few words on what a JSON value is, for error messages.
pub fn describe(j: &Json) -> String {
    match j {
        Json::Null => "null".into(),
        Json::Bool(b) => b.to_string(),
        Json::Number(n) => n.to_string(),
        Json::String(s) if s.chars().count() <= 24 => format!("\"{s}\""),
        Json::String(_) => "a string".into(),
        Json::Array(a) => format!("an array of {}", a.len()),
        Json::Object(_) => "an object".into(),
    }
}

/// A colour as `#rrggbb(aa)` when it is exactly 8-bit, otherwise as numbers so nothing is lost.
fn color_json(c: Rgba) -> Json {
    let byte = |v: f64| {
        let b = v * 255.0;
        (b.round() - b).abs() < 1e-9 && (0.0..=255.0).contains(&b.round())
    };
    if byte(c.r) && byte(c.g) && byte(c.b) && byte(c.a) {
        Json::from(c.to_hex())
    } else if c.a == 1.0 {
        Json::from([c.r, c.g, c.b])
    } else {
        Json::from([c.r, c.g, c.b, c.a])
    }
}

fn color_from_json(j: &Json) -> Option<Rgba> {
    if let Some(s) = j.as_str() {
        return Rgba::from_hex(s);
    }
    let a: Vec<f64> = j.as_array()?.iter().map(|v| v.as_f64().filter(|v| v.is_finite())).collect::<Option<_>>()?;
    match a.len() {
        3 => Some(Rgba { r: a[0], g: a[1], b: a[2], a: 1.0 }),
        4 => Some(Rgba { r: a[0], g: a[1], b: a[2], a: a[3] }),
        _ => None,
    }
}

fn path_json(p: &ShapePath) -> Json {
    let list = |v: &[[f64; 2]]| Json::array(v.iter().map(|q| Json::from(*q)));
    let mut o = veymelo_json::Map::new();
    o.insert("points", list(&p.points));
    if p.in_tangents.iter().any(|t| *t != [0.0; 2]) {
        o.insert("inTangents", list(&p.in_tangents));
    }
    if p.out_tangents.iter().any(|t| *t != [0.0; 2]) {
        o.insert("outTangents", list(&p.out_tangents));
    }
    if p.closed {
        o.insert("closed", true);
    }
    if !p.feather.is_empty() {
        o.insert("feather", Json::array(p.feather.iter().map(FeatherPoint::to_json)));
    }
    Json::Object(o)
}

fn gradient_json(g: &Gradient) -> Json {
    let colors = Json::array(g.colors.iter().map(|(at, c)| Json::array([Json::Number(*at), color_json(Rgba { r: c[0], g: c[1], b: c[2], a: 1.0 })])));
    let mut o = veymelo_json::Map::new();
    o.insert("colors", colors);
    let solid = g.opacities.iter().all(|(_, a)| *a == 1.0);
    if !solid {
        o.insert("opacities", Json::array(g.opacities.iter().map(|(at, a)| Json::array([*at, *a]))));
    }
    Json::Object(o)
}

/// A gradient from JSON; an empty error means "not shaped like a gradient at all".
fn gradient_from_json(j: &Json) -> Result<Gradient, String> {
    let stops = |list: &[Json], what: &str| -> Result<Vec<(f64, [f64; 3], f64)>, String> {
        let n = list.len();
        if n < 2 {
            return Err(format!("a gradient needs at least two {what}"));
        }
        list.iter()
            .enumerate()
            .map(|(i, s)| {
                // [place, colour], or a colour alone spread evenly
                let (at, c) = match s.as_array() {
                    Some(p) if p.len() == 2 && p[0].as_f64().is_some() && (p[1].as_str().is_some() || p[1].as_array().is_some()) => (p[0].as_f64().unwrap_or(0.0), &p[1]),
                    _ => (i as f64 / (n - 1) as f64, s),
                };
                let c = color_from_json(c).ok_or_else(|| format!("{what} are colours (\"#rrggbb\" or [r, g, b]), found {}", describe(c)))?;
                if !(0.0..=1.0).contains(&at) {
                    return Err(format!("a stop's place is from 0 to 1, not {at}"));
                }
                Ok((at, [c.r, c.g, c.b], c.a))
            })
            .collect()
    };
    let (raw, opacities) = match j {
        Json::Array(list) => (stops(list, "colours")?, None),
        Json::Object(o) => {
            if let Some(k) = o.keys().find(|k| *k != "colors" && *k != "opacities") {
                return Err(format!("a gradient has no \"{k}\" (it has colors, opacities)"));
            }
            let list = o.get("colors").and_then(Json::as_array).ok_or("a gradient needs \"colors\": [[place, colour]…]")?;
            let op = match o.get("opacities") {
                None => None,
                Some(v) => Some(
                    v.as_array()
                        .ok_or("\"opacities\" is a list of [place, alpha]")?
                        .iter()
                        .map(|p| match p.as_array().map(|p| p.iter().map(Json::as_f64).collect::<Option<Vec<_>>>()) {
                            Some(Some(p)) if p.len() == 2 && (0.0..=1.0).contains(&p[0]) && (0.0..=1.0).contains(&p[1]) => Ok((p[0], p[1])),
                            _ => Err(format!("an opacity stop is [place, alpha], both 0 to 1, found {}", describe(p))),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                ),
            };
            (stops(list, "colours")?, op)
        }
        _ => return Err(String::new()),
    };
    let mut colors: Vec<(f64, [f64; 3])> = raw.iter().map(|(at, c, _)| (*at, *c)).collect();
    colors.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut opacities = opacities.unwrap_or_else(|| raw.iter().map(|(at, _, a)| (*at, *a)).collect());
    opacities.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(Gradient { colors, opacities })
}

/// A path from JSON; an empty error means "not shaped like a path at all".
fn path_from_json(j: &Json) -> Result<ShapePath, String> {
    let o = j.as_object().ok_or_else(String::new)?;
    for key in o.keys() {
        if !matches!(key, "points" | "inTangents" | "outTangents" | "closed" | "feather") {
            return Err(format!("a path has no \"{key}\" (it has points, inTangents, outTangents, closed, feather)"));
        }
    }
    let list = |key: &str, n: Option<usize>| -> Result<Vec<[f64; 2]>, String> {
        let Some(v) = o.get(key) else { return Ok(vec![[0.0; 2]; n.unwrap_or(0)]) };
        let a = v.as_array().ok_or_else(|| format!("\"{key}\" should be a list of [x, y]"))?;
        if let Some(n) = n
            && a.len() != n
        {
            return Err(format!("\"{key}\" has {} entries for {n} points", a.len()));
        }
        a.iter()
            .map(|q| match q.as_array().map(|q| q.iter().map(|v| v.as_f64().filter(|v| v.is_finite())).collect::<Option<Vec<_>>>()) {
                Some(Some(q)) if q.len() == 2 => Ok([q[0], q[1]]),
                _ => Err(format!("\"{key}\" should be a list of [x, y], found {}", describe(q))),
            })
            .collect()
    };
    if !o.contains("points") {
        return Err("a path needs \"points\"".into());
    }
    let points = list("points", None)?;
    let n = Some(points.len());
    let closed = match o.get("closed") {
        None => false,
        Some(c) => c.as_bool().ok_or("\"closed\" should be true or false")?,
    };
    let mut path = ShapePath { in_tangents: list("inTangents", n)?, out_tangents: list("outTangents", n)?, points, closed, feather: Feathers::NONE };
    if let Some(f) = o.get("feather") {
        let list = f.as_array().ok_or("\"feather\" is a list of softening points, [{piece, at, width, tension?}…]")?;
        path.feather = list.iter().map(FeatherPoint::from_json).collect::<Result<_, _>>()?;
        if let Some(bad) = path.feather.iter().find(|f| f.piece >= path.pieces()) {
            return Err(format!("a softening point is on piece {} of a path with {} pieces", bad.piece, path.pieces()));
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLENDS: &[&str] = &["normal", "multiply", "screen"];

    fn round(kind: Kind, v: Value) -> Json {
        let j = v.to_json(kind);
        assert_eq!(Value::from_json(kind, &j), Ok(v), "{}", j.to_json());
        j
    }

    #[test]
    fn every_kind_reads_back() {
        assert_eq!(round(Kind::Number, Value::Number(12.5)).to_json(), "12.5");
        assert_eq!(round(Kind::Vec2, Value::Vec2([960.0, 540.0])).to_json(), "[960,540]");
        assert_eq!(round(Kind::Vec3, Value::Vec3([1.0, 2.0, -3.0])).to_json(), "[1,2,-3]");
        assert_eq!(round(Kind::Color, Value::Color(Rgba::from_hex("#ff8000").unwrap())).to_json(), "\"#ff8000\"");
        assert_eq!(round(Kind::Color, Value::Color(Rgba { r: 0.5, g: 0.25, b: 0.0, a: 1.0 })).to_json(), "[0.5,0.25,0]");
        assert_eq!(round(Kind::Color, Value::Color(Rgba { r: 1.0, g: 1.0, b: 1.0, a: 0.3 })).to_json(), "[1,1,1,0.3]");
        assert_eq!(round(Kind::Bool, Value::Bool(true)).to_json(), "true");
        assert_eq!(round(Kind::Choice(BLENDS), Value::Choice(2)).to_json(), "\"screen\"");
        assert_eq!(round(Kind::Text, Value::Text("สวัสดี".into())).to_json(), "\"สวัสดี\"");
        assert_eq!(round(Kind::Layer, Value::Layer(Some(7))).to_json(), "7");
        assert_eq!(round(Kind::Layer, Value::Layer(None)).to_json(), "null");
        let mut p = ShapePath::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], true);
        assert_eq!(round(Kind::Path, Value::Path(p.clone())).to_json(), r#"{"points":[[0,0],[10,0],[10,10]],"closed":true}"#);
        p.out_tangents[0] = [3.0, -1.0];
        round(Kind::Path, Value::Path(p));
        // a 2D point is a 3D one on the plane z = 0
        assert_eq!(Value::from_json(Kind::Vec3, &Json::parse("[4, 5]").unwrap()), Ok(Value::Vec3([4.0, 5.0, 0.0])));
    }

    #[test]
    fn mistakes_say_what_was_wanted() {
        let read = |kind: Kind, text: &str| Value::from_json(kind, &Json::parse(text).unwrap()).unwrap_err();
        assert_eq!(read(Kind::Vec2, "[1, 2, 3]"), "expected [x, y], got an array of 3");
        assert_eq!(read(Kind::Number, "\"ten\""), "expected a number, got \"ten\"");
        assert_eq!(read(Kind::Choice(BLENDS), "\"plus\""), "\"plus\" is not one of \"normal\", \"multiply\", \"screen\"");
        assert_eq!(read(Kind::Color, "\"#12\""), "expected \"#rrggbb\", \"#rrggbbaa\" or [r, g, b, a?] from 0 to 1, got \"#12\"");
        assert_eq!(read(Kind::Layer, "-1"), "expected a layer id, or null, got -1");
        assert_eq!(read(Kind::Path, r#"{"points": [[0, 0]], "inTangents": []}"#), "\"inTangents\" has 0 entries for 1 points");
        assert_eq!(read(Kind::Path, r#"{"vertices": []}"#), "a path has no \"vertices\" (it has points, inTangents, outTangents, closed, feather)");
        assert!(read(Kind::Path, "[]").starts_with("expected {\"points\""));
    }

    #[test]
    fn gradients() {
        let g = Value::from_json(Kind::Gradient, &Json::parse(r##"["#ff0000", "#0000ff"]"##).unwrap()).unwrap();
        let gr = g.as_gradient().unwrap();
        assert_eq!(gr.at(0.5), [0.5, 0.0, 0.5, 1.0]);
        assert_eq!(g.to_json(Kind::Gradient).to_json(), r##"{"colors":[[0,"#ff0000"],[1,"#0000ff"]]}"##);
        let faded = r##"{"colors": [[0, "#ffffff"], [1, "#000000"]], "opacities": [[0, 1], [1, 0]]}"##;
        let f = Value::from_json(Kind::Gradient, &Json::parse(faded).unwrap()).unwrap();
        assert_eq!(f.as_gradient().unwrap().at(1.0)[3], 0.0);
        assert_eq!(Value::from_json(Kind::Gradient, &f.to_json(Kind::Gradient)), Ok(f.clone()), "reads back");
        let half = g.lerp(&Value::Gradient(Gradient::default()), 0.5);
        assert_eq!(half.as_gradient().unwrap().at(0.0), [1.0, 0.5, 0.5, 1.0]);
        let bad = |t: &str| Value::from_json(Kind::Gradient, &Json::parse(t).unwrap()).unwrap_err();
        assert_eq!(bad(r##"["#ff0000"]"##), "a gradient needs at least two colours");
        assert_eq!(bad(r##"{"colors": [[0, "#fff"], [2, "#000"]]}"##), "a stop's place is from 0 to 1, not 2");
    }

    #[test]
    fn values_mix() {
        assert_eq!(Value::Vec2([0.0, 10.0]).lerp(&Value::Vec2([10.0, 20.0]), 0.5), Value::Vec2([5.0, 15.0]));
        let c = Value::Color(Rgba::BLACK).lerp(&Value::Color(Rgba { r: 1.0, g: 0.5, b: 0.0, a: 1.0 }), 0.5);
        assert_eq!(c.as_color(), Rgba { r: 0.5, g: 0.25, b: 0.0, a: 1.0 });
        // values that are not numbers jump at the end
        assert_eq!(Value::Bool(false).lerp(&Value::Bool(true), 0.99), Value::Bool(false));
        assert_eq!(Value::Bool(false).lerp(&Value::Bool(true), 1.0), Value::Bool(true));
        // paths with different numbers of points cannot morph
        let a = Value::Path(ShapePath::polygon(&[[0.0, 0.0], [10.0, 0.0]], false));
        let b = Value::Path(ShapePath::polygon(&[[0.0, 10.0], [10.0, 10.0]], false));
        assert_eq!(a.lerp(&b, 0.5).as_path().unwrap().points, vec![[0.0, 5.0], [10.0, 5.0]]);
        let c = Value::Path(ShapePath::polygon(&[[0.0, 0.0]], false));
        assert_eq!(a.lerp(&c, 0.5), a);
    }
}
