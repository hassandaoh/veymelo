//! What a command takes, checked before it runs: each parameter has a name, a type and a line
//! of help. A parameter the command does not have, a missing one or one of the wrong type is
//! refused with what the command does take (and the name meant, for a near miss).

use veymelo_color::Rgba;
use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::describe;
use veymelo_time::Tick;

use crate::Error;

/// The type of a parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ty {
    Number,
    /// A whole number, not below zero.
    Whole,
    Bool,
    Text,
    /// Seconds.
    Time,
    /// `"#rrggbb"`, `"#rrggbbaa"` or `[r, g, b, a?]` from 0 to 1.
    Color,
    Choice(&'static [&'static str]),
    /// A composition: its id or its name.
    Comp,
    /// A layer: its id, its place from the top (`"#1"`) or its name.
    Layer,
    /// A frame rate: 30, 29.97 or `"30000/1001"`.
    Fps,
    /// Any JSON; the command checks it (a property's value, keyframes).
    Any,
    /// A JSON object the command reads.
    Object,
    /// A list of JSON values the command reads.
    List,
}

impl Ty {
    /// How it is written, for help and mistakes.
    pub fn shape(self) -> String {
        match self {
            Ty::Number => "a number".into(),
            Ty::Whole => "a whole number".into(),
            Ty::Bool => "true or false".into(),
            Ty::Text => "a string".into(),
            Ty::Time => "a time in seconds".into(),
            Ty::Color => "a colour, \"#rrggbb\" or [r, g, b] from 0 to 1".into(),
            Ty::Choice(options) => format!("one of {}", options.iter().map(|o| format!("\"{o}\"")).collect::<Vec<_>>().join(", ")),
            Ty::Comp => "a composition's id or name".into(),
            Ty::Layer => "a layer's id, its place from the top (\"#1\") or its name".into(),
            Ty::Fps => "frames a second: 30, 29.97 or \"30000/1001\"".into(),
            Ty::Any => "a value".into(),
            Ty::Object => "an object".into(),
            Ty::List => "a list".into(),
        }
    }

    fn fits(self, j: &Json) -> bool {
        match self {
            Ty::Number => j.as_f64().is_some_and(f64::is_finite),
            Ty::Whole => j.as_u64().is_some(),
            Ty::Bool => j.as_bool().is_some(),
            Ty::Text => j.as_str().is_some(),
            Ty::Time => j.as_f64().is_some_and(|t| t.is_finite() && t.abs() <= 1e9),
            Ty::Color => veymelo_keyframe::Value::from_json(veymelo_keyframe::Kind::Color, j).is_ok(),
            Ty::Choice(options) => j.as_str().is_some_and(|s| options.contains(&s)),
            Ty::Comp | Ty::Layer => j.as_u64().is_some() || j.as_str().is_some(),
            Ty::Fps => veymelo_project::fps_from_json(j).is_some(),
            Ty::Any => true,
            Ty::Object => j.as_object().is_some(),
            Ty::List => j.as_array().is_some(),
        }
    }

    /// As JSON Schema.
    pub fn schema(self) -> Json {
        let ty = |t: &str| Json::object([("type", t)]);
        let either = |a: Json, b: Json| Json::object([("anyOf", Json::array([a, b]))]);
        match self {
            Ty::Number | Ty::Time => ty("number"),
            Ty::Whole => Json::object([("type", Json::from("integer")), ("minimum", Json::Number(0.0))]),
            Ty::Bool => ty("boolean"),
            Ty::Text => ty("string"),
            Ty::Color => either(ty("string"), Json::object([("type", Json::from("array")), ("items", ty("number"))])),
            Ty::Choice(options) => Json::object([("enum", Json::array(options.iter().copied()))]),
            Ty::Comp | Ty::Layer => either(ty("integer"), ty("string")),
            Ty::Fps => either(ty("number"), ty("string")),
            Ty::Any => Json::Object(Map::new()),
            Ty::Object => ty("object"),
            Ty::List => ty("array"),
        }
    }
}

/// A parameter of a command.
#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: &'static str,
    pub ty: Ty,
    pub required: bool,
    pub doc: &'static str,
}

impl Param {
    /// A parameter that must be given.
    pub const fn need(name: &'static str, ty: Ty, doc: &'static str) -> Param {
        Param { name, ty, required: true, doc }
    }

    /// A parameter that may be left out.
    pub const fn opt(name: &'static str, ty: Ty, doc: &'static str) -> Param {
        Param { name, ty, required: false, doc }
    }
}

/// The parameters of one run of a command, checked against what it takes.
#[derive(Debug)]
pub struct Args<'a> {
    map: Map,
    command: &'a str,
}

/// How far apart two names are: letters added, taken away, changed or swapped with the next.
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.to_lowercase().chars().collect(), b.to_lowercase().chars().collect());
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// The name in `names` closest to `name`, if one is close.
pub fn nearest<'n>(name: &str, names: impl IntoIterator<Item = &'n str>) -> Option<&'n str> {
    names.into_iter().map(|n| (distance(name, n), n)).filter(|(d, n)| *d <= (n.chars().count() / 3).max(1)).min_by_key(|(d, _)| *d).map(|(_, n)| n)
}

impl<'a> Args<'a> {
    /// Check `params` (an object, or null for none) against `takes`.
    pub fn check(command: &'a str, takes: &[Param], params: &Json) -> Result<Args<'a>, Error> {
        let map = match params {
            Json::Null => Map::new(),
            Json::Object(m) => m.clone(),
            other => return Err(Error::params(format!("{command} takes an object of parameters, got {}", describe(other)))),
        };
        let names = || takes.iter().map(|p| p.name).collect::<Vec<_>>().join(", ");
        for (key, v) in map.iter() {
            let Some(p) = takes.iter().find(|p| p.name == key) else {
                let hint = nearest(key, takes.iter().map(|p| p.name)).map(|n| format!(" Did you mean `{n}`?")).unwrap_or_default();
                let takes_what = if takes.is_empty() { "no parameters".to_string() } else { format!("only {}", names()) };
                return Err(Error::params(format!("{command} has no parameter `{key}`.{hint} (It takes {takes_what}.)")));
            };
            if !p.ty.fits(v) {
                return Err(Error::params(format!("{command}: `{key}` is {}, got {}", p.ty.shape(), describe(v))));
            }
        }
        if let Some(p) = takes.iter().find(|p| p.required && !map.contains(p.name)) {
            return Err(Error::params(format!("{command} needs `{}`: {} ({})", p.name, p.doc, p.ty.shape())));
        }
        Ok(Args { map, command })
    }

    pub fn command(&self) -> &str {
        self.command
    }

    pub fn has(&self, name: &str) -> bool {
        self.map.contains(name)
    }

    pub fn json(&self, name: &str) -> Option<&Json> {
        self.map.get(name)
    }

    pub fn num(&self, name: &str) -> Option<f64> {
        self.map.get(name)?.as_f64()
    }

    pub fn whole(&self, name: &str) -> Option<u64> {
        self.map.get(name)?.as_u64()
    }

    pub fn bool(&self, name: &str) -> Option<bool> {
        self.map.get(name)?.as_bool()
    }

    pub fn text(&self, name: &str) -> Option<&str> {
        self.map.get(name)?.as_str()
    }

    pub fn time(&self, name: &str) -> Option<Tick> {
        self.num(name).map(Tick::from_seconds)
    }

    pub fn color(&self, name: &str) -> Option<Rgba> {
        veymelo_keyframe::Value::from_json(veymelo_keyframe::Kind::Color, self.map.get(name)?).ok().map(|v| v.as_color())
    }

    pub fn fps(&self, name: &str) -> Option<veymelo_time::FrameRate> {
        veymelo_project::fps_from_json(self.map.get(name)?)
    }

    /// A parameter that must be there (the check made sure of it).
    pub fn need(&self, name: &str) -> &Json {
        self.map.get(name).unwrap_or(&Json::Null)
    }
}

/// The JSON Schema of what a command takes.
pub fn schema(takes: &[Param]) -> Json {
    let mut props = Map::new();
    for p in takes {
        let mut s = p.ty.schema();
        if let Json::Object(o) = &mut s {
            o.insert("description", format!("{} ({})", p.doc, p.ty.shape()));
        }
        props.insert(p.name, s);
    }
    Json::object([
        ("type", Json::from("object")),
        ("properties", Json::Object(props)),
        ("required", Json::array(takes.iter().filter(|p| p.required).map(|p| p.name))),
        ("additionalProperties", Json::Bool(false)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAKES: &[Param] = &[
        Param::need("name", Ty::Text, "what to call it"),
        Param::opt("width", Ty::Whole, "how wide"),
        Param::opt("fps", Ty::Fps, "how fast"),
        Param::opt("blend", Ty::Choice(&["normal", "add"]), "how it mixes"),
    ];

    fn check(text: &str) -> Result<Args<'static>, String> {
        Args::check("comp.new", TAKES, &Json::parse(text).unwrap()).map_err(|e| e.message)
    }

    #[test]
    fn good_parameters_pass() {
        let a = check(r#"{"name": "Main", "width": 1920, "fps": 29.97}"#).unwrap();
        assert_eq!(a.text("name"), Some("Main"));
        assert_eq!(a.whole("width"), Some(1920));
        assert_eq!(a.fps("fps"), Some(veymelo_time::FrameRate::FPS_29_97));
        assert_eq!(a.num("missing"), None);
    }

    #[test]
    fn mistakes_say_what_is_taken() {
        assert_eq!(check(r#"{"name": "A", "widht": 5}"#).unwrap_err(), "comp.new has no parameter `widht`. Did you mean `width`? (It takes only name, width, fps, blend.)");
        assert_eq!(check(r#"{"name": "A", "colour": 5}"#).unwrap_err(), "comp.new has no parameter `colour`. (It takes only name, width, fps, blend.)");
        assert_eq!(check(r#"{"width": 5}"#).unwrap_err(), "comp.new needs `name`: what to call it (a string)");
        assert_eq!(check(r#"{"name": "A", "width": -5}"#).unwrap_err(), "comp.new: `width` is a whole number, got -5");
        assert_eq!(check(r#"{"name": "A", "blend": "plus"}"#).unwrap_err(), "comp.new: `blend` is one of \"normal\", \"add\", got \"plus\"");
        assert_eq!(check("[1]").unwrap_err(), "comp.new takes an object of parameters, got an array of 1");
        assert_eq!(Args::check("edit.undo", &[], &Json::parse(r#"{"steps": 2}"#).unwrap()).unwrap_err().message, "edit.undo has no parameter `steps`. (It takes no parameters.)");
        assert!(Args::check("edit.undo", &[], &Json::Null).is_ok());
    }

    #[test]
    fn near_names() {
        assert_eq!(nearest("positon", ["anchor", "position", "scale"]), Some("position"));
        assert_eq!(nearest("Opacity", ["opacity"]), Some("opacity"));
        assert_eq!(nearest("zz", ["anchor", "position"]), None);
        assert_eq!(nearest("widht", ["width", "height"]), Some("width"));
    }

    #[test]
    fn a_schema_lists_every_parameter() {
        let s = schema(TAKES);
        assert_eq!(s.get("required").unwrap().to_json(), r#"["name"]"#);
        assert_eq!(s.get("properties").unwrap().get("blend").unwrap().get("enum").unwrap().to_json(), r#"["normal","add"]"#);
        assert_eq!(s.get("properties").unwrap().get("width").unwrap().get("description").unwrap().as_str(), Some("how wide (a whole number)"));
    }
}
