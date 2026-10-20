//! The part of YAML colour configurations are written in: mappings and sequences by indent,
//! `{}` and `[]` written in a line (or spread over lines), `!<Tag>`s on mappings, quoted and
//! plain words, `|` and `>` blocks, comments. Nesting past [`DEEPEST`] is cut off.

/// How deep collections may nest (configurations use about ten levels).
pub const DEEPEST: usize = 64;

/// A YAML value.
#[derive(Clone, Debug, PartialEq)]
pub enum Yaml {
    Null,
    Str(String),
    Seq(Vec<Yaml>),
    /// A mapping, in its order, with its tag (`!<MatrixTransform>` → "MatrixTransform").
    Map(String, Vec<(String, Yaml)>),
}

impl Yaml {
    pub fn get(&self, key: &str) -> Option<&Yaml> {
        match self {
            Yaml::Map(_, m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn tag(&self) -> &str {
        match self {
            Yaml::Map(t, _) => t,
            _ => "",
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Yaml::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The words at `key`, or nothing.
    pub fn text(&self, key: &str) -> &str {
        self.get(key).and_then(Yaml::str).unwrap_or("")
    }

    pub fn seq(&self) -> &[Yaml] {
        match self {
            Yaml::Seq(v) => v,
            _ => &[],
        }
    }

    pub fn map(&self) -> &[(String, Yaml)] {
        match self {
            Yaml::Map(_, m) => m,
            _ => &[],
        }
    }

    /// The numbers in it (a number, or a sequence of them).
    pub fn nums(&self) -> Vec<f64> {
        match self {
            Yaml::Str(s) => s.trim().parse().ok().into_iter().collect(),
            Yaml::Seq(v) => v.iter().filter_map(|y| y.str()?.trim().parse().ok()).collect(),
            _ => Vec::new(),
        }
    }

    /// A yes or no at `key` (`d` when it is not there).
    pub fn flag(&self, key: &str, d: bool) -> bool {
        match self.text(key).to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" => true,
            "false" | "no" | "off" => false,
            _ => d,
        }
    }
}

/// A line with comments taken off (a `#` outside quotes, at the start or after a space).
fn uncomment(line: &str) -> &str {
    let (mut single, mut double) = (false, false);
    let mut prev = ' ';
    for (i, c) in line.char_indices() {
        match c {
            '\'' if !double => single = !single,
            '"' if !single && prev != '\\' => double = !double,
            '#' if !single && !double && prev.is_whitespace() => return &line[..i],
            _ => {}
        }
        prev = c;
    }
    line
}

/// Words without their quotes.
fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        return s[1..s.len() - 1].replace("''", "'");
    }
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let mut out = String::new();
        let mut chars = s[1..s.len() - 1].chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(o) => out.push(o),
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    s.to_string()
}

/// How far `[` and `{` outnumber `]` and `}` outside quotes.
fn open_count(s: &str) -> i32 {
    let (mut n, mut single, mut double) = (0, false, false);
    for c in s.chars() {
        match c {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '[' | '{' if !single && !double => n += 1,
            ']' | '}' if !single && !double => n -= 1,
            _ => {}
        }
    }
    n
}

/// The key and the rest of a `key: value` line (the colon followed by a space or the end, and
/// outside quotes and brackets).
fn split_key(s: &str) -> Option<(String, &str)> {
    let (mut single, mut double, mut depth) = (false, false, 0);
    let b = s.as_bytes();
    for (i, c) in s.char_indices() {
        match c {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '[' | '{' if !single && !double => depth += 1,
            ']' | '}' if !single && !double => depth -= 1,
            ':' if !single && !double && depth == 0 && (i + 1 == b.len() || b[i + 1] == b' ') => {
                return Some((unquote(&s[..i]), s[i + 1..].trim()));
            }
            _ => {}
        }
    }
    None
}

/// A tag at the start (`!<Name>` or `!Name`) and what follows it.
fn take_tag(s: &str) -> (String, &str) {
    let s = s.trim_start();
    if let Some(r) = s.strip_prefix("!<")
        && let Some(e) = r.find('>')
    {
        return (r[..e].to_string(), r[e + 1..].trim_start());
    }
    if let Some(r) = s.strip_prefix('!') {
        let e = r.find(|c: char| c.is_whitespace() || c == '{' || c == '[').unwrap_or(r.len());
        return (r[..e].to_string(), r[e..].trim_start());
    }
    (String::new(), s)
}

/// A value written in a line: a flow collection (maybe tagged), or words.
fn inline(s: &str, depth: usize) -> Yaml {
    let mut p = Flow { s: s.as_bytes(), at: 0 };
    let v = p.value(depth);
    if p.rest().is_empty() || matches!(v, Yaml::Map(..) | Yaml::Seq(_)) { v } else { Yaml::Str(unquote(s)) }
}

/// Reading `{}` and `[]` collections.
struct Flow<'a> {
    s: &'a [u8],
    at: usize,
}

impl Flow<'_> {
    fn skip(&mut self) {
        while self.at < self.s.len() && self.s[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }

    fn rest(&self) -> &str {
        std::str::from_utf8(&self.s[self.at.min(self.s.len())..]).unwrap_or("").trim()
    }

    fn value(&mut self, depth: usize) -> Yaml {
        self.skip();
        let rest = std::str::from_utf8(&self.s[self.at..]).unwrap_or("");
        let (tag, after) = take_tag(rest);
        self.at = self.s.len() - after.len();
        self.skip();
        if depth > DEEPEST {
            self.at = self.s.len();
            return Yaml::Null;
        }
        match self.s.get(self.at) {
            Some(b'{') => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.skip();
                    match self.s.get(self.at) {
                        None => break,
                        Some(b'}') => {
                            self.at += 1;
                            break;
                        }
                        Some(b',') => {
                            self.at += 1;
                            continue;
                        }
                        _ => {}
                    }
                    let key = self.word(true);
                    self.skip();
                    let value = if self.s.get(self.at) == Some(&b':') {
                        self.at += 1;
                        self.value(depth + 1)
                    } else {
                        Yaml::Null
                    };
                    items.push((unquote(&key), value));
                }
                Yaml::Map(tag, items)
            }
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.skip();
                    match self.s.get(self.at) {
                        None => break,
                        Some(b']') => {
                            self.at += 1;
                            break;
                        }
                        Some(b',') => {
                            self.at += 1;
                            continue;
                        }
                        _ => items.push(self.value(depth + 1)),
                    }
                }
                Yaml::Seq(items)
            }
            _ => {
                let w = self.word(false);
                if w.is_empty() && !tag.is_empty() { Yaml::Map(tag, Vec::new()) } else if w.is_empty() { Yaml::Null } else { Yaml::Str(unquote(&w)) }
            }
        }
    }

    /// Words up to a `,` `}` `]` (or a `:` before a space, for a key), quotes kept whole.
    fn word(&mut self, key: bool) -> String {
        self.skip();
        let start = self.at;
        if let Some(&q @ (b'"' | b'\'')) = self.s.get(self.at) {
            self.at += 1;
            while self.at < self.s.len() && self.s[self.at] != q {
                if q == b'"' && self.s[self.at] == b'\\' {
                    self.at += 1;
                }
                self.at += 1;
            }
            self.at = (self.at + 1).min(self.s.len());
            return String::from_utf8_lossy(&self.s[start..self.at]).into_owned();
        }
        while self.at < self.s.len() {
            let c = self.s[self.at];
            if c == b',' || c == b'}' || c == b']' {
                break;
            }
            if key && c == b':' && self.s.get(self.at + 1).is_none_or(|n| n.is_ascii_whitespace() || *n == b',' || *n == b'}') {
                break;
            }
            self.at += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.at]).trim().to_string()
    }
}

/// A line of the document: how far it is indented and what is on it.
struct Line {
    indent: usize,
    text: String,
}

/// Lines with comments and blanks gone, flow collections spread over lines joined, `|` and `>`
/// blocks gathered into their line.
fn lines(text: &str) -> Vec<Line> {
    let raw: Vec<&str> = text.lines().collect();
    let mut out: Vec<Line> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let line = raw[i].trim_end_matches('\r');
        i += 1;
        let body = uncomment(line);
        if body.trim().is_empty() || body.trim() == "---" || body.trim_start().starts_with('%') {
            continue;
        }
        let indent = body.len() - body.trim_start().len();
        let mut t = body.trim().to_string();
        // a block of words: the lines indented further, kept (|) or folded (>)
        if t.ends_with(": |") || t.ends_with(": >") || t.ends_with(": |-") || t.ends_with(": >-") {
            let fold = t.contains(": >");
            let head = t[..t.rfind(':').unwrap_or(0) + 1].to_string();
            let mut block = Vec::new();
            while i < raw.len() && (raw[i].trim().is_empty() || raw[i].len() - raw[i].trim_start().len() > indent) {
                block.push(raw[i].trim().to_string());
                i += 1;
            }
            let joined = block.join(if fold { " " } else { "\n" });
            t = format!("{head} \"{}\"", joined.trim().replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"));
        }
        // an unclosed [ or {: the lines after it until it closes
        while open_count(&t) > 0 && i < raw.len() {
            t.push(' ');
            t.push_str(uncomment(raw[i]).trim());
            i += 1;
        }
        out.push(Line { indent, text: t });
    }
    out
}

/// The document's value.
pub fn parse(text: &str) -> Yaml {
    let ls = lines(text);
    let mut i = 0;
    if ls.is_empty() {
        return Yaml::Null;
    }
    block(&ls, &mut i, ls[0].indent, 0)
}

/// The block starting at line `i`, its lines indented `indent`.
fn block(ls: &[Line], i: &mut usize, indent: usize, depth: usize) -> Yaml {
    if depth > DEEPEST || *i >= ls.len() {
        return Yaml::Null;
    }
    let first = &ls[*i].text;
    if first == "-" || first.starts_with("- ") {
        let mut items = Vec::new();
        while *i < ls.len() && ls[*i].indent == indent && (ls[*i].text == "-" || ls[*i].text.starts_with("- ")) {
            let rest = ls[*i].text[1..].trim_start().to_string();
            *i += 1;
            items.push(item(ls, i, indent, &rest, depth));
        }
        return Yaml::Seq(items);
    }
    if split_key(first).is_some() {
        let mut map = Vec::new();
        let mut tag = String::new();
        while *i < ls.len() && ls[*i].indent == indent {
            let Some((k, v)) = split_key(&ls[*i].text) else { break };
            let v = v.to_string();
            *i += 1;
            map.push((k, value(ls, i, indent, &v, depth)));
        }
        if map.is_empty() {
            tag.clear();
        }
        return Yaml::Map(tag, map);
    }
    let v = inline(first, depth);
    *i += 1;
    v
}

/// The value after `key:` on a line indented `indent`: written there, or the block below it.
fn value(ls: &[Line], i: &mut usize, indent: usize, v: &str, depth: usize) -> Yaml {
    let (tag, rest) = take_tag(v);
    if rest.is_empty() {
        // a block below: indented further, or a sequence at the key's own indent
        let below = *i < ls.len() && (ls[*i].indent > indent || (ls[*i].indent == indent && (ls[*i].text == "-" || ls[*i].text.starts_with("- "))));
        if below {
            let inner = ls[*i].indent;
            return match block(ls, i, inner, depth + 1) {
                Yaml::Map(_, m) => Yaml::Map(tag, m),
                other if tag.is_empty() => other,
                other => other,
            };
        }
        return if tag.is_empty() { Yaml::Null } else { Yaml::Map(tag, Vec::new()) };
    }
    inline(v, depth + 1)
}

/// A sequence item: words, a flow collection, or a mapping whose first key is on the dash's
/// line (its keys indented past the dash).
fn item(ls: &[Line], i: &mut usize, indent: usize, rest: &str, depth: usize) -> Yaml {
    if rest.is_empty() {
        if *i < ls.len() && ls[*i].indent > indent {
            let inner = ls[*i].indent;
            return block(ls, i, inner, depth + 1);
        }
        return Yaml::Null;
    }
    let (tag, after) = take_tag(rest);
    if after.is_empty() {
        // a tagged mapping below the dash
        if *i < ls.len() && ls[*i].indent > indent {
            let inner = ls[*i].indent;
            if let Yaml::Map(_, m) = block(ls, i, inner, depth + 1) {
                return Yaml::Map(tag, m);
            }
        }
        return Yaml::Map(tag, Vec::new());
    }
    if after.starts_with('{') || after.starts_with('[') || split_key(after).is_none() {
        return inline(rest, depth + 1);
    }
    // `- key: value` and its other keys on the lines below, as far in as the first key
    let inner = indent + (ls[*i - 1].text.len() - after.len());
    let mut map = Vec::new();
    if let Some((k, v)) = split_key(after) {
        let v = v.to_string();
        map.push((k, value(ls, i, inner, &v, depth + 1)));
    }
    while *i < ls.len() && ls[*i].indent == inner {
        let Some((k, v)) = split_key(&ls[*i].text) else { break };
        let v = v.to_string();
        *i += 1;
        map.push((k, value(ls, i, inner, &v, depth + 1)));
    }
    Yaml::Map(tag, map)
}
