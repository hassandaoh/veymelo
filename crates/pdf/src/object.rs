//! A PDF's objects (numbers, strings, names, arrays, dictionaries, streams and references to
//! numbered objects) and the reader that makes them from bytes: the same reader serves the file
//! itself and the content of its pages.

use std::sync::Arc;

/// A dictionary: names and values, in the order the file gives them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dict(pub Vec<(String, Obj)>);

impl Dict {
    pub fn get(&self, key: &str) -> Option<&Obj> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn insert(&mut self, key: &str, v: Obj) {
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = v,
            None => self.0.push((key.to_string(), v)),
        }
    }
}

/// A stream: its dictionary and its bytes as the file has them (still encoded).
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub dict: Dict,
    pub data: Vec<u8>,
    /// Which object it is (for unlocking an encrypted file's streams).
    pub id: (u32, u16),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Obj {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Str(Vec<u8>),
    Name(String),
    Array(Vec<Obj>),
    Dict(Dict),
    Stream(Arc<Stream>),
    Ref(u32, u16),
    /// A word of a content stream (an operator), or one the file did not expect.
    Word(String),
}

impl Obj {
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Obj::Int(i) => Some(i as f64),
            Obj::Real(r) => Some(r),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Obj::Int(i) => Some(i),
            Obj::Real(r) => Some(r as i64),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&str> {
        match self {
            Obj::Name(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Obj]> {
        match self {
            Obj::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Obj::Dict(d) => Some(d),
            Obj::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&[u8]> {
        match self {
            Obj::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Obj::Bool(b) => Some(b),
            _ => None,
        }
    }
}

pub fn white(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

pub fn delimiter(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

/// Reads objects one after another from bytes.
pub struct Lexer<'a> {
    pub d: &'a [u8],
    pub p: usize,
}

/// What reading finds: an object, or the end of an array or dictionary.
#[derive(Debug, PartialEq)]
enum Token {
    Obj(Obj),
    ArrayEnd,
    DictEnd,
    DictStart,
    ArrayStart,
}

impl<'a> Lexer<'a> {
    pub fn new(d: &'a [u8], p: usize) -> Lexer<'a> {
        Lexer { d, p }
    }

    pub fn skip_white(&mut self) {
        let d = self.d;
        while self.p < d.len() {
            if white(d[self.p]) {
                self.p += 1;
            } else if d[self.p] == b'%' {
                while self.p < d.len() && d[self.p] != b'\n' && d[self.p] != b'\r' {
                    self.p += 1;
                }
            } else {
                break;
            }
        }
    }

    /// The next word (keyword, number) as its bytes, without reading it as an object.
    pub fn word(&mut self) -> Option<&'a [u8]> {
        self.skip_white();
        let start = self.p;
        while self.p < self.d.len() && !white(self.d[self.p]) && !delimiter(self.d[self.p]) {
            self.p += 1;
        }
        (self.p > start).then(|| &self.d[start..self.p])
    }

    fn literal(&mut self) -> Vec<u8> {
        let d = self.d;
        let mut out = Vec::new();
        let mut depth = 1;
        while self.p < d.len() {
            let b = d[self.p];
            self.p += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(b);
                }
                b'\\' => {
                    let Some(&e) = d.get(self.p) else { break };
                    self.p += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut v = (e - b'0') as u32;
                            for _ in 0..2 {
                                match d.get(self.p) {
                                    Some(&c @ b'0'..=b'7') => {
                                        v = v * 8 + (c - b'0') as u32;
                                        self.p += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(v as u8);
                        }
                        // a line break after a backslash is not part of the string
                        b'\r' => {
                            if d.get(self.p) == Some(&b'\n') {
                                self.p += 1;
                            }
                        }
                        b'\n' => {}
                        other => out.push(other),
                    }
                }
                // a line ending in a string is a newline
                b'\r' => {
                    if d.get(self.p) == Some(&b'\n') {
                        self.p += 1;
                    }
                    out.push(b'\n');
                }
                _ => out.push(b),
            }
        }
        out
    }

    fn hex(&mut self) -> Vec<u8> {
        let d = self.d;
        let mut digits = Vec::new();
        while self.p < d.len() && d[self.p] != b'>' {
            let b = d[self.p];
            if b.is_ascii_hexdigit() {
                digits.push(b);
            }
            self.p += 1;
        }
        self.p += 1;
        if digits.len() % 2 == 1 {
            digits.push(b'0');
        }
        digits.chunks(2).map(|c| (hexval(c[0]) << 4) | hexval(c[1])).collect()
    }

    fn name(&mut self) -> String {
        let d = self.d;
        let mut out = Vec::new();
        while self.p < d.len() && !white(d[self.p]) && !delimiter(d[self.p]) {
            if d[self.p] == b'#' && self.p + 2 < d.len() && d[self.p + 1].is_ascii_hexdigit() && d[self.p + 2].is_ascii_hexdigit() {
                out.push((hexval(d[self.p + 1]) << 4) | hexval(d[self.p + 2]));
                self.p += 3;
            } else {
                out.push(d[self.p]);
                self.p += 1;
            }
        }
        String::from_utf8(out).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect())
    }

    fn token(&mut self) -> Option<Token> {
        self.skip_white();
        let d = self.d;
        let b = *d.get(self.p)?;
        match b {
            b'(' => {
                self.p += 1;
                Some(Token::Obj(Obj::Str(self.literal())))
            }
            b'<' if d.get(self.p + 1) == Some(&b'<') => {
                self.p += 2;
                Some(Token::DictStart)
            }
            b'<' => {
                self.p += 1;
                Some(Token::Obj(Obj::Str(self.hex())))
            }
            b'>' if d.get(self.p + 1) == Some(&b'>') => {
                self.p += 2;
                Some(Token::DictEnd)
            }
            b'[' => {
                self.p += 1;
                Some(Token::ArrayStart)
            }
            b']' => {
                self.p += 1;
                Some(Token::ArrayEnd)
            }
            b'/' => {
                self.p += 1;
                Some(Token::Obj(Obj::Name(self.name())))
            }
            b'{' | b'}' | b')' | b'>' => {
                self.p += 1;
                Some(Token::Obj(Obj::Word((b as char).to_string())))
            }
            _ => {
                let w = self.word()?;
                Some(Token::Obj(match w {
                    b"true" => Obj::Bool(true),
                    b"false" => Obj::Bool(false),
                    b"null" => Obj::Null,
                    _ => number(w).unwrap_or_else(|| Obj::Word(String::from_utf8_lossy(w).into_owned())),
                }))
            }
        }
    }

    /// The next object (an array or dictionary read whole; `n g R` a reference); none at the end
    /// or at the end of an array or dictionary it is inside.
    pub fn object(&mut self) -> Option<Obj> {
        self.object_at(0)
    }

    fn object_at(&mut self, depth: usize) -> Option<Obj> {
        let t = self.token()?;
        match t {
            Token::Obj(Obj::Int(n)) => {
                // n g R
                let save = self.p;
                if let Some(Token::Obj(Obj::Int(g))) = self.token() {
                    let save2 = self.p;
                    if self.word() == Some(b"R") {
                        return Some(Obj::Ref(n.max(0) as u32, g.max(0) as u16));
                    }
                    self.p = save2;
                }
                self.p = save;
                Some(Obj::Int(n))
            }
            Token::Obj(o) => Some(o),
            Token::ArrayStart => {
                let mut items = Vec::new();
                loop {
                    self.skip_white();
                    if self.p >= self.d.len() {
                        break;
                    }
                    if self.d[self.p] == b']' {
                        self.p += 1;
                        break;
                    }
                    if depth > 200 {
                        break;
                    }
                    match self.object_at(depth + 1) {
                        Some(o) => items.push(o),
                        None => break,
                    }
                }
                Some(Obj::Array(items))
            }
            Token::DictStart => {
                let mut dict = Dict::default();
                loop {
                    self.skip_white();
                    if self.p >= self.d.len() {
                        break;
                    }
                    if self.d[self.p] == b'>' && self.d.get(self.p + 1) == Some(&b'>') {
                        self.p += 2;
                        break;
                    }
                    if depth > 200 {
                        break;
                    }
                    match self.object_at(depth + 1) {
                        Some(Obj::Name(k)) => {
                            let v = self.object_at(depth + 1).unwrap_or(Obj::Null);
                            dict.0.push((k, v));
                        }
                        // a key that is not a name: passed over
                        Some(_) => {}
                        None => break,
                    }
                }
                Some(Obj::Dict(dict))
            }
            Token::ArrayEnd | Token::DictEnd => None,
        }
    }
}

pub fn hexval(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

/// A number word: whole when it has no point.
pub fn number(w: &[u8]) -> Option<Obj> {
    let s = std::str::from_utf8(w).ok()?;
    let first = *w.first()?;
    if !(first.is_ascii_digit() || matches!(first, b'+' | b'-' | b'.')) {
        return None;
    }
    if !s.contains('.') && let Ok(i) = s.parse::<i64>() {
        return Some(Obj::Int(i));
    }
    // writers also put out "--5" and "5-": read as much as makes a number
    let cleaned: String = {
        let neg = s.starts_with('-');
        let body: String = s.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
        let mut seen_point = false;
        let body: String = body.chars().filter(|&c| c != '.' || !std::mem::replace(&mut seen_point, true)).collect();
        if neg { format!("-{body}") } else { body }
    };
    cleaned.parse::<f64>().ok().map(Obj::Real).or(Some(Obj::Real(0.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objects_are_read_as_the_file_writes_them() {
        let src = br"<< /Type /Page /Kids [1 0 R 2 0 R] /Name#20With#23 (a (nested) \(str\)\n\101) /H <48 65 6C6C 6F> /N -3.5 /I 12 /B true /Z null /E .5 >>";
        let o = Lexer::new(src, 0).object().unwrap();
        let d = o.as_dict().unwrap();
        assert_eq!(d.get("Type"), Some(&Obj::Name("Page".into())));
        assert_eq!(d.get("Kids"), Some(&Obj::Array(vec![Obj::Ref(1, 0), Obj::Ref(2, 0)])));
        assert_eq!(d.get("Name With#"), Some(&Obj::Str(b"a (nested) (str)\nA".to_vec())));
        assert_eq!(d.get("H"), Some(&Obj::Str(b"Hello".to_vec())));
        assert_eq!(d.get("N"), Some(&Obj::Real(-3.5)));
        assert_eq!(d.get("I"), Some(&Obj::Int(12)));
        assert_eq!(d.get("B"), Some(&Obj::Bool(true)));
        assert_eq!(d.get("Z"), Some(&Obj::Null));
        assert_eq!(d.get("E"), Some(&Obj::Real(0.5)));
        // content: numbers and operators
        let mut l = Lexer::new(b"1 0 0 1 72 720 cm % note\n/F1 12 Tf", 0);
        let all: Vec<Obj> = std::iter::from_fn(|| l.object()).collect();
        assert_eq!(all[6], Obj::Word("cm".into()));
        assert_eq!(all[7], Obj::Name("F1".into()));
        assert_eq!(all[9], Obj::Word("Tf".into()));
    }
}
