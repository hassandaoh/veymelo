//! Reading the parts of a project file strictly: every mistake says where it is and what was
//! expected, and a key the reader does not know is a mistake (so a typo is never ignored).

use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::describe;
use veymelo_time::Tick;

/// What is wrong in a project file, and where (`items[0].layers[2].props.transform.position`).
#[derive(Clone, Debug, PartialEq)]
pub struct LoadError {
    pub at: String,
    pub message: String,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.at.is_empty() { write!(f, "{}", self.message) } else { write!(f, "{}: {}", self.at, self.message) }
    }
}

impl std::error::Error for LoadError {}

pub(crate) fn fail<T>(at: &str, message: impl Into<String>) -> Result<T, LoadError> {
    Err(LoadError { at: at.to_string(), message: message.into() })
}

/// `at` and then `key`: `items[0]` and `name` make `items[0].name`.
pub(crate) fn join(at: &str, key: &str) -> String {
    if at.is_empty() { key.to_string() } else { format!("{at}.{key}") }
}

/// An object being read.
pub(crate) struct Obj<'a> {
    pub map: &'a Map,
    pub at: &'a str,
}

impl<'a> Obj<'a> {
    pub fn new(j: &'a Json, at: &'a str, what: &str) -> Result<Obj<'a>, LoadError> {
        match j.as_object() {
            Some(map) => Ok(Obj { map, at }),
            None => fail(at, format!("{what} is an object, got {}", describe(j))),
        }
    }

    /// Fails on any key not in `known`.
    pub fn only(&self, known: &[&str]) -> Result<(), LoadError> {
        match self.map.keys().find(|k| !known.contains(k)) {
            Some(k) => fail(self.at, format!("there is no \"{k}\" here (there is {})", known.join(", "))),
            None => Ok(()),
        }
    }

    pub fn at(&self, key: &str) -> String {
        join(self.at, key)
    }

    pub fn get(&self, key: &str) -> Option<&'a Json> {
        self.map.get(key)
    }

    fn wrong<T>(&self, key: &str, wanted: &str, got: &Json) -> Result<T, LoadError> {
        fail(&self.at(key), format!("expected {wanted}, got {}", describe(got)))
    }

    pub fn num(&self, key: &str) -> Result<Option<f64>, LoadError> {
        match self.get(key) {
            None => Ok(None),
            Some(j) => match j.as_f64().filter(|v| v.is_finite()) {
                Some(v) => Ok(Some(v)),
                None => self.wrong(key, "a number", j),
            },
        }
    }

    pub fn num_or(&self, key: &str, default: f64) -> Result<f64, LoadError> {
        Ok(self.num(key)?.unwrap_or(default))
    }

    /// A whole number from `min` to `max`.
    pub fn whole(&self, key: &str, min: u64, max: u64) -> Result<Option<u64>, LoadError> {
        match self.get(key) {
            None => Ok(None),
            Some(j) => match j.as_u64().filter(|v| (min..=max).contains(v)) {
                Some(v) => Ok(Some(v)),
                None => self.wrong(key, &format!("a whole number from {min} to {max}"), j),
            },
        }
    }

    pub fn need_whole(&self, key: &str, min: u64, max: u64) -> Result<u64, LoadError> {
        self.whole(key, min, max)?.ok_or_else(|| LoadError { at: self.at.to_string(), message: format!("\"{key}\" is missing") })
    }

    /// An id: a whole number above zero.
    pub fn id(&self, key: &str) -> Result<Option<u64>, LoadError> {
        self.whole(key, 1, 1 << 53)
    }

    pub fn text(&self, key: &str) -> Result<Option<&'a str>, LoadError> {
        match self.get(key) {
            None => Ok(None),
            Some(j) => match j.as_str() {
                Some(s) => Ok(Some(s)),
                None => self.wrong(key, "a string", j),
            },
        }
    }

    pub fn need_text(&self, key: &str) -> Result<&'a str, LoadError> {
        self.text(key)?.ok_or_else(|| LoadError { at: self.at.to_string(), message: format!("\"{key}\" is missing") })
    }

    pub fn flag(&self, key: &str, default: bool) -> Result<bool, LoadError> {
        match self.get(key) {
            None => Ok(default),
            Some(j) => match j.as_bool() {
                Some(b) => Ok(b),
                None => self.wrong(key, "true or false", j),
            },
        }
    }

    /// A time in seconds.
    pub fn time(&self, key: &str) -> Result<Option<Tick>, LoadError> {
        match self.get(key) {
            None => Ok(None),
            Some(j) => match j.as_f64().filter(|v| v.is_finite() && v.abs() <= 1e9) {
                Some(v) => Ok(Some(Tick::from_seconds(v))),
                None => self.wrong(key, "a time in seconds", j),
            },
        }
    }

    pub fn time_or(&self, key: &str, default: Tick) -> Result<Tick, LoadError> {
        Ok(self.time(key)?.unwrap_or(default))
    }

    /// One of a set of names: `ids` lists them, `pick` turns one into its value.
    pub fn choice<T>(&self, key: &str, ids: &[&str], pick: impl Fn(&str) -> Option<T>) -> Result<Option<T>, LoadError> {
        match self.get(key) {
            None => Ok(None),
            Some(j) => match j.as_str().and_then(&pick) {
                Some(v) => Ok(Some(v)),
                None => self.wrong(key, &format!("one of {}", ids.iter().map(|i| format!("\"{i}\"")).collect::<Vec<_>>().join(", ")), j),
            },
        }
    }

    pub fn list(&self, key: &str) -> Result<&'a [Json], LoadError> {
        match self.get(key) {
            None => Ok(&[]),
            Some(Json::Array(a)) => Ok(a),
            Some(j) => self.wrong(key, "a list", j),
        }
    }
}

/// Defines an enum whose variants have names in files and commands, with `ALL`, `IDS`,
/// `id()` and `from_id()`.
macro_rules! named {
    ($(#[$m:meta])* pub enum $name:ident { $($(#[$vm:meta])* $var:ident = $id:literal),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($(#[$vm])* $var),* }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),*];
            pub const IDS: &'static [&'static str] = &[$($id),*];

            pub fn id(self) -> &'static str {
                match self { $($name::$var => $id),* }
            }

            pub fn from_id(id: &str) -> Option<$name> {
                match id { $($id => Some($name::$var),)* _ => None }
            }
        }
    };
}

pub(crate) use named;
