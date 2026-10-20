//! A small XML reader, enough for SVG: elements with their attributes (entities decoded), the
//! text directly inside each (CDATA kept as text); comments, processing instructions and the
//! document type passed over, but for the entities its internal part declares. Names lose their
//! namespace prefixes (`svg:rect` is `rect`, `xlink:href` is `href`).

use std::collections::HashMap;

/// An element: its name, attributes in the order written, its children (indices into
/// [`Xml::elements`]) and the text directly inside it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<usize>,
    pub text: String,
}

impl Element {
    /// The attribute called `name`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// A document: its elements, the root first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Xml {
    pub elements: Vec<Element>,
}

impl Xml {
    pub fn root(&self) -> &Element {
        &self.elements[0]
    }

    /// Element `i` and every element inside it, depth first, in document order.
    pub fn descendants(&self, i: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = vec![i];
        while let Some(at) = stack.pop() {
            out.push(at);
            stack.extend(self.elements[at].children.iter().rev());
        }
        out
    }
}

/// The deepest elements may nest.
const MAX_DEPTH: usize = 256;

/// A name without its namespace prefix.
fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Text with its entities and character references turned into what they stand for.
fn decode(s: &str, entities: &HashMap<String, String>) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(end) = after.find(';').filter(|&e| e <= 32) else {
            out.push('&');
            rest = after;
            continue;
        };
        let name = &after[..end];
        let found = match name {
            "lt" => Some("<".to_string()),
            "gt" => Some(">".to_string()),
            "amp" => Some("&".to_string()),
            "quot" => Some("\"".to_string()),
            "apos" => Some("'".to_string()),
            _ if name.starts_with("#x") || name.starts_with("#X") => u32::from_str_radix(&name[2..], 16).ok().and_then(char::from_u32).map(String::from),
            _ if name.starts_with('#') => name[1..].parse::<u32>().ok().and_then(char::from_u32).map(String::from),
            _ => entities.get(name).cloned(),
        };
        match found {
            Some(v) => out.push_str(&v),
            None => {
                out.push('&');
                out.push_str(name);
                out.push(';');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The entities a document type's internal part declares (`<!ENTITY name "value">`).
fn entities_of(doctype: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = doctype;
    while let Some(i) = rest.find("<!ENTITY") {
        rest = &rest[i + 8..];
        let mut words = rest.trim_start().splitn(2, char::is_whitespace);
        let (Some(name), Some(tail)) = (words.next(), words.next()) else { break };
        let tail = tail.trim_start();
        let Some(q) = tail.chars().next().filter(|c| *c == '"' || *c == '\'') else { continue };
        if let Some(end) = tail[1..].find(q) {
            out.insert(name.to_string(), tail[1..1 + end].to_string());
        }
    }
    out
}

/// A document from its text.
pub fn parse(text: &str) -> Result<Xml, String> {
    let b = text.as_bytes();
    let mut xml = Xml::default();
    let mut stack: Vec<usize> = Vec::new();
    let mut entities = HashMap::new();
    let mut i = 0;
    let find = |from: usize, what: &str| text.get(from..).and_then(|r| r.find(what)).map(|p| from + p);
    while i < b.len() {
        if b[i] != b'<' {
            let end = find(i, "<").unwrap_or(b.len());
            if let Some(&top) = stack.last() {
                let t = decode(&text[i..end], &entities);
                xml.elements[top].text.push_str(&t);
            }
            i = end;
            continue;
        }
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            i = find(i + 4, "-->").map_or(b.len(), |e| e + 3);
        } else if rest.starts_with("<![CDATA[") {
            let end = find(i + 9, "]]>").unwrap_or(b.len());
            if let Some(&top) = stack.last() {
                xml.elements[top].text.push_str(&text[i + 9..end.min(b.len())]);
            }
            i = (end + 3).min(b.len());
        } else if rest.starts_with("<?") {
            i = find(i + 2, "?>").map_or(b.len(), |e| e + 2);
        } else if rest.starts_with("<!") {
            // the document type, with an internal part in brackets perhaps
            let open = find(i, "[");
            let close = find(i, ">").unwrap_or(b.len());
            if let Some(o) = open.filter(|&o| o < close) {
                let end = find(o, "]").unwrap_or(b.len());
                entities = entities_of(&text[o..end]);
                i = find(end, ">").map_or(b.len(), |e| e + 1);
            } else {
                i = close + 1;
            }
        } else if rest.starts_with("</") {
            let end = find(i, ">").ok_or("a closing tag is not finished")?;
            let name = local(text[i + 2..end].trim());
            match stack.pop() {
                Some(top) if xml.elements[top].name == name => {}
                Some(top) => return Err(format!("<{}> is closed by </{name}>", xml.elements[top].name)),
                None => return Err(format!("</{name}> closes nothing")),
            }
            i = end + 1;
        } else {
            // a start tag: its name, then attributes, up to > or />
            let mut j = i + 1;
            while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
                j += 1;
            }
            let name = local(&text[i + 1..j]).to_string();
            if name.is_empty() {
                return Err("an element has no name".into());
            }
            let mut attrs = Vec::new();
            let closed;
            loop {
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j >= b.len() {
                    return Err(format!("<{name}> is not finished"));
                }
                if b[j] == b'>' {
                    closed = false;
                    j += 1;
                    break;
                }
                if b[j] == b'/' {
                    closed = true;
                    j = find(j, ">").map_or(b.len(), |e| e + 1);
                    break;
                }
                let k0 = j;
                while j < b.len() && b[j] != b'=' && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
                    j += 1;
                }
                let key = local(&text[k0..j]).to_string();
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < b.len() && b[j] == b'=' {
                    j += 1;
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    let q = *b.get(j).ok_or_else(|| format!("<{name}> is not finished"))?;
                    if q == b'"' || q == b'\'' {
                        let end = text[j + 1..].find(q as char).map(|e| j + 1 + e).ok_or_else(|| format!("an attribute of <{name}> is not finished"))?;
                        attrs.push((key, decode(&text[j + 1..end], &entities)));
                        j = end + 1;
                    } else {
                        let v0 = j;
                        while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                            j += 1;
                        }
                        attrs.push((key, decode(&text[v0..j], &entities)));
                    }
                } else if !key.is_empty() {
                    attrs.push((key, String::new()));
                }
            }
            let at = xml.elements.len();
            xml.elements.push(Element { name, attrs, children: Vec::new(), text: String::new() });
            match stack.last() {
                Some(&top) => xml.elements[top].children.push(at),
                None if at != 0 => return Err("there is more than one root element".into()),
                None => {}
            }
            if !closed {
                stack.push(at);
                if stack.len() > MAX_DEPTH {
                    return Err(format!("elements nested more than {MAX_DEPTH} deep"));
                }
            }
            i = j;
        }
    }
    if let Some(&top) = stack.last() {
        return Err(format!("<{}> is not closed", xml.elements[top].name));
    }
    if xml.elements.is_empty() {
        return Err("there is no element".into());
    }
    Ok(xml)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_attributes_and_text() {
        let x = parse(
            r##"<?xml version="1.0"?>
<!DOCTYPE svg [ <!ENTITY ns "http://www.w3.org/2000/svg"> ]>
<!-- a comment -->
<svg:svg xmlns="&ns;" width='10' xlink:href="#a&amp;b">
  <style><![CDATA[ .a { fill: red } ]]></style>
  <g><rect x="1"/><circle r=3 /></g>
  <text>&lt;hi&#33;&#x21;&gt;</text>
</svg:svg>"##,
        )
        .unwrap();
        let root = x.root();
        assert_eq!((root.name.as_str(), root.attr("xmlns"), root.attr("width"), root.attr("href")), ("svg", Some("http://www.w3.org/2000/svg"), Some("10"), Some("#a&b")));
        let names: Vec<&str> = x.descendants(0).iter().map(|&i| x.elements[i].name.as_str()).collect();
        assert_eq!(names, ["svg", "style", "g", "rect", "circle", "text"]);
        assert_eq!(x.elements[1].text.trim(), ".a { fill: red }");
        assert_eq!(x.elements[5].text, "<hi!!>");
        assert_eq!(x.elements[4].attr("r"), Some("3"));
        assert!(parse("<a><b></a>").unwrap_err().contains("closed by"));
        assert!(parse("<a>").unwrap_err().contains("not closed"));
    }
}
