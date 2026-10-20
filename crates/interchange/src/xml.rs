//! Enough XML for the timeline files: elements with their attributes, the text in them, and
//! the character entities; read into a tree, and written out with what must be escaped.

/// An element: its name, attributes in order, what is in it, and its text (all of it, joined).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<Element>,
    pub text: String,
}

impl Element {
    /// The value of attribute `name`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// The first element in it named `name`.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Every element in it named `name`.
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// The text of the first element in it named `name`, trimmed.
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.trim())
    }

    /// The element at a path of names from here (`media/video/format`).
    pub fn at(&self, path: &str) -> Option<&Element> {
        path.split('/').try_fold(self, |e, name| e.child(name))
    }

    /// Every element under it (itself first), in document order.
    pub fn descendants(&self) -> Vec<&Element> {
        let mut out = vec![self];
        for c in &self.children {
            out.extend(c.descendants());
        }
        out
    }
}

/// Text with the five special characters made entities.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// Entities made characters again.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        match tail.find(';') {
            Some(j) => {
                let name = &tail[1..j];
                let c = match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
                    n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32),
                    _ => None,
                };
                match c {
                    Some(c) => {
                        out.push(c);
                        rest = &tail[j + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = &tail[1..];
                    }
                }
            }
            None => {
                out.push_str(tail);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// The document's root element, or where it is not written right.
pub fn parse(text: &str) -> Result<Element, String> {
    let b = text.as_bytes();
    let mut i = 0;
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    while i < b.len() {
        if b[i] != b'<' {
            let end = text[i..].find('<').map_or(b.len(), |k| i + k);
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&unescape(&text[i..end]));
            }
            i = end;
            continue;
        }
        let rest = &text[i..];
        if rest.starts_with("<?") {
            i += rest.find("?>").ok_or("a <? is not closed")? + 2;
        } else if rest.starts_with("<!--") {
            i += rest.find("-->").ok_or("a comment is not closed")? + 3;
        } else if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>").ok_or("a CDATA section is not closed")?;
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&rest[9..end]);
            }
            i += end + 3;
        } else if rest.starts_with("<!") {
            // a document type, perhaps with a bracketed part
            let mut depth = 0;
            let mut k = 2;
            let rb = rest.as_bytes();
            while k < rb.len() {
                match rb[k] {
                    b'[' => depth += 1,
                    b']' => depth -= 1,
                    b'>' if depth <= 0 => break,
                    _ => {}
                }
                k += 1;
            }
            i += k + 1;
        } else if rest.starts_with("</") {
            let end = rest.find('>').ok_or("a closing tag is not closed")?;
            let name = rest[2..end].trim();
            let done = stack.pop().ok_or_else(|| format!("</{name}> closes nothing"))?;
            if done.name != name {
                return Err(format!("</{name}> closes <{}>", done.name));
            }
            match stack.last_mut() {
                Some(parent) => parent.children.push(done),
                None => root = Some(done),
            }
            i += end + 1;
        } else {
            // an opening tag: its name, attributes, and whether it closes itself
            let mut k = 1;
            let rb = rest.as_bytes();
            let mut quote: Option<u8> = None;
            while k < rb.len() {
                match (quote, rb[k]) {
                    (None, b'"' | b'\'') => quote = Some(rb[k]),
                    (Some(q), c) if c == q => quote = None,
                    (None, b'>') => break,
                    _ => {}
                }
                k += 1;
            }
            if k >= rb.len() {
                return Err("a tag is not closed".into());
            }
            let inner = &rest[1..k];
            let closes = inner.ends_with('/');
            let inner = inner.trim_end_matches('/');
            let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
            let mut el = Element { name: inner[..name_end].to_string(), ..Element::default() };
            let mut a = inner[name_end..].trim_start();
            while !a.is_empty() {
                let eq = a.find('=').ok_or_else(|| format!("an attribute of <{}> has no value", el.name))?;
                let key = a[..eq].trim().to_string();
                let after = a[eq + 1..].trim_start();
                let q = after.chars().next().filter(|c| *c == '"' || *c == '\'').ok_or_else(|| format!("the attribute {key} of <{}> is not quoted", el.name))?;
                let close = after[1..].find(q).ok_or_else(|| format!("the attribute {key} of <{}> is not closed", el.name))?;
                el.attributes.push((key, unescape(&after[1..1 + close])));
                a = after[close + 2..].trim_start();
            }
            if closes {
                match stack.last_mut() {
                    Some(parent) => parent.children.push(el),
                    None => root = Some(el),
                }
            } else {
                stack.push(el);
            }
            i += k + 1;
        }
    }
    if let Some(open) = stack.last() {
        return Err(format!("<{}> is not closed", open.name));
    }
    root.ok_or_else(|| "there is no element in it".into())
}

/// An XML document being written, indented.
#[derive(Default)]
pub struct Writer {
    pub out: String,
    depth: usize,
}

impl Writer {
    pub fn new() -> Writer {
        Writer { out: "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n".into(), depth: 0 }
    }

    fn attrs(attributes: &[(&str, String)]) -> String {
        attributes.iter().map(|(k, v)| format!(" {k}=\"{}\"", escape(v))).collect()
    }

    /// `<name …>`, what follows inside it until [`Writer::close`].
    pub fn open(&mut self, name: &str, attributes: &[(&str, String)]) {
        self.out.push_str(&format!("{}<{name}{}>\n", "  ".repeat(self.depth), Self::attrs(attributes)));
        self.depth += 1;
    }

    pub fn close(&mut self, name: &str) {
        self.depth -= 1;
        self.out.push_str(&format!("{}</{name}>\n", "  ".repeat(self.depth)));
    }

    /// `<name …/>`.
    pub fn empty(&mut self, name: &str, attributes: &[(&str, String)]) {
        self.out.push_str(&format!("{}<{name}{}/>\n", "  ".repeat(self.depth), Self::attrs(attributes)));
    }

    /// `<name>text</name>`.
    pub fn text(&mut self, name: &str, text: &str) {
        self.out.push_str(&format!("{}<{name}>{}</{name}>\n", "  ".repeat(self.depth), escape(text)));
    }

    /// A line as it is (a document type).
    pub fn raw(&mut self, line: &str) {
        self.out.push_str(line);
        self.out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_back() {
        let mut w = Writer::new();
        w.raw("<!DOCTYPE xmeml>");
        w.open("a", &[("x", "1 & <2>".into())]);
        w.text("b", "it's \"so\"");
        w.empty("c", &[("y", "z".into())]);
        w.close("a");
        let e = parse(&w.out).unwrap();
        assert_eq!(e.attr("x"), Some("1 & <2>"));
        assert_eq!(e.text_of("b"), Some("it's \"so\""));
        assert_eq!(e.child("c").and_then(|c| c.attr("y")), Some("z"));
        assert!(parse("<a><b></a>").is_err());
        let doc = parse("<!DOCTYPE x [<!ENTITY y \"z\">]><!-- hi --><r a='1'><![CDATA[<raw>]]>&#65;&#x42;</r>").unwrap();
        assert_eq!(doc.text, "<raw>AB");
        assert_eq!(doc.attr("a"), Some("1"));
    }
}
