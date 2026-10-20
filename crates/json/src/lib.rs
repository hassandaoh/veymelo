//! JSON (RFC 8259), our own: a `Value`, read strictly (with the line and column of a mistake)
//! and written compact or pretty. Objects keep their keys in order, so files written from the
//! same project are the same, and a number reads back exactly as it was.
//!
//! JSON cannot hold NaN or infinity: such a number is written as `null`
//! (`Value::is_finite` tells whether a value would lose one).

use std::fmt::Write as _;

/// A JSON value.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(Map),
}

/// A JSON object: its keys in the order they were added, each once.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Map {
    entries: Vec<(String, Value)>,
}

impl Map {
    pub fn new() -> Map {
        Map::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Sets `key` to `value`: in its place if it is there, else at the end. Gives what it held.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) -> Option<Value> {
        let (key, value) = (key.into(), value.into());
        match self.entries.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => Some(std::mem::replace(v, value)),
            None => {
                self.entries.push((key, value));
                None
            }
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let i = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(i).1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }
}

impl<K: Into<String>, V: Into<Value>> FromIterator<(K, V)> for Map {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Map {
        let mut m = Map::new();
        for (k, v) in iter {
            m.insert(k, v);
        }
        m
    }
}

impl Value {
    /// An object of these keys and values, in this order.
    pub fn object<K: Into<String>, V: Into<Value>>(pairs: impl IntoIterator<Item = (K, V)>) -> Value {
        Value::Object(pairs.into_iter().collect())
    }

    /// An array of these values.
    pub fn array<V: Into<Value>>(items: impl IntoIterator<Item = V>) -> Value {
        Value::Array(items.into_iter().map(Into::into).collect())
    }

    /// The value under `key`, if this is an object that has it.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        match self {
            Value::Object(m) => m.get_mut(key),
            _ => None,
        }
    }

    /// The item at `index`, if this is an array long enough.
    pub fn at(&self, index: usize) -> Option<&Value> {
        self.as_array()?.get(index)
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// The number, if it is a whole one an `i64` can hold exactly.
    pub fn as_i64(&self) -> Option<i64> {
        let n = self.as_f64()?;
        (n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_992.0).then_some(n as i64)
    }

    /// The number, if it is a whole one, not below zero.
    pub fn as_u64(&self) -> Option<u64> {
        self.as_i64().and_then(|n| u64::try_from(n).ok())
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Map> {
        match self {
            Value::Object(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut Map> {
        match self {
            Value::Object(m) => Some(m),
            _ => None,
        }
    }

    /// What kind of value it is, in words (for messages).
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "a true or false",
            Value::Number(_) => "a number",
            Value::String(_) => "a string",
            Value::Array(_) => "an array",
            Value::Object(_) => "an object",
        }
    }

    /// Every number in it can be written (none is NaN or infinite).
    pub fn is_finite(&self) -> bool {
        match self {
            Value::Number(n) => n.is_finite(),
            Value::Array(a) => a.iter().all(Value::is_finite),
            Value::Object(m) => m.iter().all(|(_, v)| v.is_finite()),
            _ => true,
        }
    }

    /// The value as compact JSON text.
    pub fn to_json(&self) -> String {
        let mut s = String::new();
        write_value(&mut s, self, None, 0);
        s
    }

    /// The value as JSON text indented two spaces a level, with short lists of numbers on
    /// one line (`[100, 360]`), and a newline at the end.
    pub fn to_pretty(&self) -> String {
        let mut s = String::new();
        write_value(&mut s, self, Some(2), 0);
        s.push('\n');
        s
    }

    /// Reads JSON text.
    pub fn parse(text: &str) -> Result<Value, Error> {
        let mut r = Reader { text: text.as_bytes(), at: 0, depth: 0 };
        r.space();
        let v = r.value()?;
        r.space();
        if r.at < r.text.len() {
            return Err(r.fail("there is more after the value"));
        }
        Ok(v)
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_json())
    }
}

macro_rules! from_number {
    ($($t:ty),*) => {
        $(impl From<$t> for Value {
            fn from(n: $t) -> Value {
                Value::Number(n as f64)
            }
        })*
    };
}
from_number!(f64, f32, i8, i16, i32, i64, u8, u16, u32, u64, usize, isize);

impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::String(s.to_string())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::String(s)
    }
}

impl From<&String> for Value {
    fn from(s: &String) -> Value {
        Value::String(s.clone())
    }
}

impl From<Map> for Value {
    fn from(m: Map) -> Value {
        Value::Object(m)
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Value {
        Value::Array(v.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(o: Option<T>) -> Value {
        o.map_or(Value::Null, Into::into)
    }
}

impl<T: Into<Value> + Copy, const N: usize> From<[T; N]> for Value {
    fn from(a: [T; N]) -> Value {
        Value::Array(a.iter().map(|&x| x.into()).collect())
    }
}

/// A mistake in JSON text: what, and where (line and column, from 1).
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}, column {}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for Error {}

/// How deeply arrays and objects may nest.
const MAX_DEPTH: usize = 512;

struct Reader<'a> {
    text: &'a [u8],
    at: usize,
    depth: usize,
}

impl Reader<'_> {
    fn fail(&self, message: impl Into<String>) -> Error {
        let before = &self.text[..self.at.min(self.text.len())];
        let line = before.iter().filter(|&&b| b == b'\n').count() + 1;
        let start = before.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        // the column counts characters, not bytes
        let column = String::from_utf8_lossy(&before[start..]).chars().count() + 1;
        Error { message: message.into(), line, column }
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn word(&mut self, word: &str, value: Value) -> Result<Value, Error> {
        if self.text[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(self.fail(format!("expected {word}")))
        }
    }

    fn value(&mut self) -> Result<Value, Error> {
        match self.peek() {
            None => Err(self.fail("the text ends where a value should be")),
            Some(b'n') => self.word("null", Value::Null),
            Some(b't') => self.word("true", Value::Bool(true)),
            Some(b'f') => self.word("false", Value::Bool(false)),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(c) => Err(self.fail(format!("a value cannot start with {}", describe(c)))),
        }
    }

    fn deeper(&mut self) -> Result<(), Error> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.fail(format!("arrays and objects nest more than {MAX_DEPTH} deep")));
        }
        Ok(())
    }

    fn array(&mut self) -> Result<Value, Error> {
        self.deeper()?;
        self.at += 1;
        let mut items = vec![];
        self.space();
        if self.peek() == Some(b']') {
            self.at += 1;
            self.depth -= 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.space();
            items.push(self.value()?);
            self.space();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    self.depth -= 1;
                    return Ok(Value::Array(items));
                }
                Some(c) => return Err(self.fail(format!("expected , or ] in an array, not {}", describe(c)))),
                None => return Err(self.fail("the text ends inside an array")),
            }
        }
    }

    fn object(&mut self) -> Result<Value, Error> {
        self.deeper()?;
        self.at += 1;
        let mut map = Map::new();
        self.space();
        if self.peek() == Some(b'}') {
            self.at += 1;
            self.depth -= 1;
            return Ok(Value::Object(map));
        }
        loop {
            self.space();
            if self.peek() != Some(b'"') {
                return Err(self.fail("expected a key in quotes"));
            }
            let key_at = self.at;
            let key = self.string()?;
            if map.contains(&key) {
                self.at = key_at;
                return Err(self.fail(format!("the key \"{key}\" appears twice")));
            }
            self.space();
            if self.peek() != Some(b':') {
                return Err(self.fail("expected : after a key"));
            }
            self.at += 1;
            self.space();
            let v = self.value()?;
            map.insert(key, v);
            self.space();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    self.depth -= 1;
                    return Ok(Value::Object(map));
                }
                Some(c) => return Err(self.fail(format!("expected , or }} in an object, not {}", describe(c)))),
                None => return Err(self.fail("the text ends inside an object")),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, Error> {
        let digits = self.text.get(self.at..self.at + 4).ok_or_else(|| self.fail("a \\u escape needs four hex digits"))?;
        let s = std::str::from_utf8(digits).map_err(|_| self.fail("a \\u escape needs four hex digits"))?;
        let v = u32::from_str_radix(s, 16).map_err(|_| self.fail("a \\u escape needs four hex digits"))?;
        self.at += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, Error> {
        self.at += 1;
        let mut out = String::new();
        loop {
            // the run of plain characters up to the next quote, backslash or control character
            let start = self.at;
            while let Some(c) = self.peek() {
                if c == b'"' || c == b'\\' || c < 0x20 {
                    break;
                }
                self.at += 1;
            }
            // the text is UTF-8 and runs end at ASCII bytes, so a run is whole characters
            out.push_str(std::str::from_utf8(&self.text[start..self.at]).unwrap_or_default());
            match self.peek() {
                None => return Err(self.fail("the text ends inside a string")),
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let c = self.peek().ok_or_else(|| self.fail("the text ends inside a string"))?;
                    self.at += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xD800..0xDC00).contains(&code) {
                                // a surrogate pair: the low half must follow
                                if !self.text[self.at..].starts_with(b"\\u") {
                                    return Err(self.fail("a \\u escape of a high surrogate needs its low surrogate after it"));
                                }
                                self.at += 2;
                                let low = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return Err(self.fail("a high surrogate is followed by something other than a low surrogate"));
                                }
                                code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                            } else if (0xDC00..0xE000).contains(&code) {
                                return Err(self.fail("a low surrogate without a high one before it"));
                            }
                            out.push(char::from_u32(code).ok_or_else(|| self.fail("not a character"))?);
                        }
                        other => {
                            self.at -= 1;
                            return Err(self.fail(format!("\\{} is not an escape", other as char)));
                        }
                    }
                }
                Some(c) => return Err(self.fail(format!("a string cannot hold {} as it is: write it escaped", describe(c)))),
            }
        }
    }

    fn number(&mut self) -> Result<Value, Error> {
        let start = self.at;
        let digits = |r: &mut Reader| -> usize {
            let s = r.at;
            while matches!(r.peek(), Some(b'0'..=b'9')) {
                r.at += 1;
            }
            r.at - s
        };
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.at += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(self.fail("a number cannot start with 0 and go on with digits"));
                }
            }
            Some(b'1'..=b'9') => {
                digits(self);
            }
            _ => return Err(self.fail("expected a digit")),
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if digits(self) == 0 {
                return Err(self.fail("expected a digit after the decimal point"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if digits(self) == 0 {
                return Err(self.fail("expected a digit in the exponent"));
            }
        }
        let s = std::str::from_utf8(&self.text[start..self.at]).unwrap_or_default();
        let n: f64 = s.parse().map_err(|_| self.fail("not a number"))?;
        if !n.is_finite() {
            self.at = start;
            return Err(self.fail("the number is too large"));
        }
        Ok(Value::Number(n))
    }
}

/// A byte, in words (for messages).
fn describe(c: u8) -> String {
    match c {
        b'\n' => "a line break".into(),
        b'\t' => "a tab".into(),
        0..0x20 => format!("the control character {c:#04x}"),
        0x80.. => "a character outside ASCII".into(),
        _ => format!("'{}'", c as char),
    }
}

fn write_number(out: &mut String, n: f64) {
    if !n.is_finite() {
        out.push_str("null");
    } else if n.fract() == 0.0 && n.abs() < 1e16 {
        // a whole number, without ".0" (and -0 as 0)
        let _ = write!(out, "{}", n as i64);
    } else {
        // the shortest digits that read back as the same number
        let _ = write!(out, "{n:?}");
    }
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn newline(out: &mut String, indent: Option<usize>, level: usize) {
    if let Some(n) = indent {
        out.push('\n');
        out.extend(std::iter::repeat_n(' ', n * level));
    }
}

/// A short list of plain values is written on one line when pretty.
fn inline(items: &[Value]) -> bool {
    items.len() <= 4 && items.iter().all(|v| matches!(v, Value::Number(_) | Value::Bool(_) | Value::Null))
}

fn write_value(out: &mut String, v: &Value, indent: Option<usize>, level: usize) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(out, *n),
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
            } else if indent.is_some() && inline(items) {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    write_value(out, item, None, 0);
                }
                out.push(']');
            } else {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    newline(out, indent, level + 1);
                    write_value(out, item, indent, level + 1);
                }
                newline(out, indent, level);
                out.push(']');
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, indent, level + 1);
                write_string(out, k);
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                write_value(out, item, indent, level + 1);
            }
            newline(out, indent, level);
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Value {
        Value::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    fn err(s: &str) -> Error {
        Value::parse(s).expect_err(s)
    }

    #[test]
    fn values_read_and_write_back() {
        for s in ["null", "true", "false", "0", "-1", "3.5", "\"hi\"", "[]", "{}", "[1,[2,[3]]]", "{\"a\":1,\"b\":[true,null]}"] {
            assert_eq!(p(s).to_json(), s);
        }
        // order of keys is kept
        let v = p("{\"z\":1,\"a\":2,\"m\":3}");
        assert_eq!(v.as_object().unwrap().keys().collect::<Vec<_>>(), ["z", "a", "m"]);
    }

    #[test]
    fn numbers_read_back_exactly() {
        for n in [0.1, 0.1 + 0.2, 1e300, -2.5e-320, 123456789.12345679, 9007199254740993.0, f64::MAX, f64::MIN_POSITIVE, 1.0 / 3.0] {
            let text = Value::from(n).to_json();
            assert_eq!(p(&text).as_f64(), Some(n), "{n} → {text}");
        }
        assert_eq!(Value::from(42).to_json(), "42");
        assert_eq!(Value::from(-0.0).to_json(), "0");
        assert_eq!(Value::from(1e20).to_json(), "1e20");
        assert_eq!(Value::from(f64::NAN).to_json(), "null");
        assert!(!Value::array([1.0, f64::INFINITY]).is_finite());
        assert_eq!(p("1E3").as_i64(), Some(1000));
        assert_eq!(p("2.5").as_i64(), None);
        assert_eq!(p("-3").as_u64(), None);
    }

    #[test]
    fn strings_escape_and_unescape() {
        let s = "line\nbreak \"quote\" back\\slash tab\t ctrl\u{1} ไทย 😀";
        let text = Value::from(s).to_json();
        assert_eq!(text, "\"line\\nbreak \\\"quote\\\" back\\\\slash tab\\t ctrl\\u0001 ไทย 😀\"");
        assert_eq!(p(&text).as_str(), Some(s));
        assert_eq!(p("\"\\u0e01\\ud83d\\ude00\\/\"").as_str(), Some("ก😀/"));
    }

    #[test]
    fn mistakes_say_where_they_are() {
        let e = err("{\n  \"a\": 1,\n  \"b\" 2\n}");
        assert_eq!((e.line, e.column), (3, 7), "{e}");
        assert!(e.message.contains("expected :"));
        assert!(err("[1, 2").message.contains("ends inside an array"));
        assert!(err("[1 2]").message.contains("expected , or ]"));
        assert!(err("{\"a\":1,\"a\":2}").message.contains("appears twice"));
        assert!(err("01").message.contains("cannot start with 0"));
        assert!(err("1.").message.contains("after the decimal point"));
        assert!(err("1e400").message.contains("too large"));
        assert!(err("\"a\nb\"").message.contains("a line break"));
        assert!(err("\"\\x\"").message.contains("not an escape"));
        assert!(err("\"\\ud83d\"").message.contains("low surrogate"));
        assert!(err("\"\\ude00\"").message.contains("without a high one"));
        assert!(err("tru").message.contains("expected true"));
        assert!(err("1 2").message.contains("more after the value"));
        assert!(err("").message.contains("ends where a value should be"));
        // the column counts characters
        let e = err("\"ไทย\" x");
        assert_eq!(e.column, 7);
    }

    #[test]
    fn nesting_too_deep_is_refused_not_a_crash() {
        let deep = "[".repeat(100_000);
        assert!(err(&deep).message.contains("nest more than"));
        let ok = format!("{}{}", "[".repeat(500), "]".repeat(500));
        assert!(Value::parse(&ok).is_ok());
    }

    #[test]
    fn pretty_text_is_easy_to_read() {
        let v = Value::object([
            ("name", Value::from("Main")),
            ("size", Value::array([1920, 1080])),
            ("layers", Value::array([Value::object([("id", 1)]), Value::object([("id", 2)])])),
            ("empty", Value::Array(vec![])),
        ]);
        let want = "{\n  \"name\": \"Main\",\n  \"size\": [1920, 1080],\n  \"layers\": [\n    {\n      \"id\": 1\n    },\n    {\n      \"id\": 2\n    }\n  ],\n  \"empty\": []\n}\n";
        assert_eq!(v.to_pretty(), want);
        assert_eq!(p(&v.to_pretty()), v);
    }

    #[test]
    fn maps_set_in_place_and_remove() {
        let mut m = Map::new();
        m.insert("a", 1);
        m.insert("b", 2);
        assert_eq!(m.insert("a", 3), Some(Value::from(1)));
        assert_eq!(m.keys().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(m.remove("a"), Some(Value::from(3)));
        assert_eq!(m.len(), 1);
        let v = Value::from(m);
        assert_eq!(v.get("b").and_then(Value::as_i64), Some(2));
        assert!(v.get("a").is_none());
    }
}
