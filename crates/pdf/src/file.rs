//! A PDF file opened: where each numbered object is (the cross-reference tables and streams, the
//! newest update first, objects packed in object streams too; when they are missing or wrong,
//! found again by reading the whole file), each object read when it is asked for and kept, a
//! stream's bytes undone through its filters (and unlocked, in a file locked for no password),
//! and its pages with what they take from the page tree above them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use crate::crypt::Crypt;
use crate::filters::{Decoded, decode, filters_of};
use crate::object::{Dict, Lexer, Obj, Stream};

#[derive(Clone, Copy, Debug)]
enum Entry {
    /// At a place in the file, of a generation.
    At(usize),
    /// The `index`th object of an object stream.
    In(u32, usize),
}

pub struct File {
    data: Vec<u8>,
    xref: HashMap<u32, Entry>,
    pub trailer: Dict,
    kept: RefCell<HashMap<u32, Obj>>,
    /// Objects being read (one that holds itself is not read again).
    reading: RefCell<Vec<u32>>,
    crypt: Option<Crypt>,
    /// ICC profiles read, by their streams' objects.
    profiles: RefCell<HashMap<u32, Option<Arc<veymelo_color::icc::Profile>>>>,
}

/// A page: its dictionary with what it takes from above (resources, boxes, turn).
pub struct Page {
    pub dict: Dict,
    pub resources: Dict,
    pub media: [f64; 4],
    pub crop: [f64; 4],
    pub rotate: i64,
}

/// Where `what` is last in `d`.
fn rfind(d: &[u8], what: &[u8]) -> Option<usize> {
    d.windows(what.len()).rposition(|w| w == what)
}

fn find_from(d: &[u8], what: &[u8], from: usize) -> Option<usize> {
    d.get(from..)?.windows(what.len()).position(|w| w == what).map(|p| p + from)
}

/// A rectangle `[x0 y0 x1 y1]`, its corners in order.
pub fn rect(o: Option<&Obj>) -> Option<[f64; 4]> {
    let a = o?.as_array()?;
    let v: Vec<f64> = a.iter().filter_map(Obj::as_f64).collect();
    if v.len() < 4 {
        return None;
    }
    Some([v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

impl File {
    pub fn open(data: Vec<u8>) -> Result<File, String> {
        let head = find_from(&data, b"%PDF-", 0).filter(|&p| p < 1024).ok_or("it is not a PDF file (an Illustrator file from before version 9, or saved without PDF content, cannot be read)")?;
        let _ = head;
        let mut f = File { data, xref: HashMap::new(), trailer: Dict::default(), kept: RefCell::new(HashMap::new()), reading: RefCell::new(Vec::new()), crypt: None, profiles: RefCell::new(HashMap::new()) };
        let read = f.read_xrefs();
        if read.is_err() || f.trailer.get("Root").is_none() || f.xref.is_empty() || !f.places_hold() {
            f.find_again();
        }
        if f.trailer.get("Root").is_none() {
            return Err("its catalogue (the root of its pages) cannot be found".into());
        }
        if let Some(enc) = f.trailer.get("Encrypt").cloned() {
            let enc = f.resolve(&enc);
            let id = f.trailer.get("ID").and_then(Obj::as_array).and_then(|a| a.first()).and_then(Obj::as_str).map(<[u8]>::to_vec).unwrap_or_default();
            f.crypt = Some(Crypt::new(enc.as_dict().ok_or("its encryption cannot be read")?, &id)?);
            f.kept.borrow_mut().clear();
        }
        Ok(f)
    }

    /// Whether every place the tables give holds the object they say.
    fn places_hold(&self) -> bool {
        self.xref.iter().all(|(&n, e)| match *e {
            Entry::At(at) => {
                let mut l = Lexer::new(&self.data, at);
                matches!(l.object(), Some(Obj::Int(k)) if k as u32 == n) && matches!(l.object(), Some(Obj::Int(_))) && l.word() == Some(b"obj")
            }
            Entry::In(..) => true,
        })
    }

    /// The cross-reference sections from the last one back.
    fn read_xrefs(&mut self) -> Result<(), String> {
        let tail = self.data.len().saturating_sub(2048);
        let at = rfind(&self.data[tail..], b"startxref").ok_or("no startxref")? + tail;
        let mut l = Lexer::new(&self.data, at + 9);
        let mut next = l.object().and_then(|o| o.as_i64()).map(|v| v as usize);
        let mut seen = Vec::new();
        while let Some(at) = next {
            if seen.contains(&at) || at >= self.data.len() || seen.len() > 100 {
                break;
            }
            seen.push(at);
            next = self.read_section(at)?;
        }
        Ok(())
    }

    /// One cross-reference section (a table and its trailer, or a stream): the one before it.
    fn read_section(&mut self, at: usize) -> Result<Option<usize>, String> {
        let mut l = Lexer::new(&self.data, at);
        if l.word() == Some(b"xref") {
            loop {
                let save = l.p;
                match l.word() {
                    Some(b"trailer") => break,
                    Some(w) => {
                        let start: u32 = std::str::from_utf8(w).ok().and_then(|s| s.parse().ok()).ok_or("a bad cross-reference table")?;
                        let count: u32 = l.word().and_then(|w| std::str::from_utf8(w).ok()?.parse().ok()).ok_or("a bad cross-reference table")?;
                        for k in 0..count {
                            let (Some(off), Some(_gen), Some(kind)) = (l.word(), l.word(), l.word()) else { return Err("a cross-reference table cut off".into()) };
                            if kind == b"n" {
                                let off: usize = std::str::from_utf8(off).ok().and_then(|s| s.parse().ok()).unwrap_or(0);
                                self.xref.entry(start + k).or_insert(Entry::At(off));
                            }
                        }
                    }
                    None => {
                        l.p = save;
                        break;
                    }
                }
            }
            let trailer = l.object().and_then(|o| o.as_dict().cloned()).ok_or("its trailer cannot be read")?;
            // a file updated in both ways keeps a stream of the rest
            if let Some(stm) = trailer.get("XRefStm").and_then(Obj::as_i64) {
                let _ = self.read_section(stm as usize);
            }
            let prev = trailer.get("Prev").and_then(Obj::as_i64).map(|v| v as usize);
            self.merge_trailer(trailer);
            return Ok(prev);
        }
        // a cross-reference stream
        let (_, obj) = self.parse_at(at).ok_or("a cross-reference that is neither a table nor a stream")?;
        let Obj::Stream(s) = obj else { return Err("a cross-reference that is not a stream".into()) };
        let data = decode(&s.data, &filters_of(&s.dict, false))?.data;
        let w: Vec<usize> = s.dict.get("W").and_then(Obj::as_array).map(|a| a.iter().filter_map(|o| o.as_i64().map(|v| v.max(0) as usize)).collect()).unwrap_or_default();
        if w.len() < 3 {
            return Err("a cross-reference stream without its widths".into());
        }
        let size = s.dict.get("Size").and_then(Obj::as_i64).unwrap_or(0);
        let index: Vec<i64> = s.dict.get("Index").and_then(Obj::as_array).map(|a| a.iter().filter_map(Obj::as_i64).collect()).unwrap_or_else(|| vec![0, size]);
        let field = |p: &mut usize, n: usize| {
            let v = (0..n).fold(0usize, |v, k| (v << 8) | *data.get(*p + k).unwrap_or(&0) as usize);
            *p += n;
            v
        };
        let mut p = 0;
        for pair in index.chunks(2) {
            let (start, count) = (pair[0].max(0) as u32, pair.get(1).copied().unwrap_or(0).max(0) as u32);
            for k in 0..count {
                if p >= data.len() {
                    break;
                }
                let kind = if w[0] == 0 { 1 } else { field(&mut p, w[0]) };
                let (a, b) = (field(&mut p, w[1]), field(&mut p, w[2]));
                let e = match kind {
                    1 => Entry::At(a),
                    2 => Entry::In(a as u32, b),
                    _ => continue,
                };
                self.xref.entry(start + k).or_insert(e);
            }
        }
        let prev = s.dict.get("Prev").and_then(Obj::as_i64).map(|v| v as usize);
        self.merge_trailer(s.dict.clone());
        Ok(prev)
    }

    /// A trailer's keys the newer ones do not have.
    fn merge_trailer(&mut self, t: Dict) {
        for (k, v) in t.0 {
            if self.trailer.get(&k).is_none() {
                self.trailer.0.push((k, v));
            }
        }
    }

    /// Every `n g obj` found by reading the whole file, the last of each number kept, and the
    /// trailer (or the catalogue) found so.
    fn find_again(&mut self) {
        let d = &self.data;
        let mut found = HashMap::new();
        let mut p = 0;
        while let Some(o) = find_from(d, b"obj", p) {
            p = o + 3;
            // "n g obj" with the line or a space before n
            let mut q = o;
            let mut nums = Vec::new();
            for _ in 0..2 {
                while q > 0 && crate::object::white(d[q - 1]) {
                    q -= 1;
                }
                let end = q;
                while q > 0 && d[q - 1].is_ascii_digit() {
                    q -= 1;
                }
                if q == end {
                    break;
                }
                nums.push(std::str::from_utf8(&d[q..end]).ok().and_then(|s| s.parse::<u32>().ok()));
            }
            if let [Some(_gen), Some(num)] = nums[..]
                && (q == 0 || crate::object::white(d[q - 1]) || d[q - 1] == b'>')
                && d.get(o + 3).is_none_or(|&b| crate::object::white(b) || crate::object::delimiter(b))
            {
                found.insert(num, Entry::At(q));
            }
        }
        let mut trailer = None;
        let mut p = 0;
        while let Some(t) = find_from(d, b"trailer", p) {
            p = t + 7;
            if let Some(Obj::Dict(dict)) = Lexer::new(d, t + 7).object()
                && dict.get("Root").is_some()
            {
                trailer = Some(dict);
            }
        }
        self.xref = found;
        if let Some(t) = trailer {
            self.trailer = t;
        }
        // objects of object streams, and the catalogue when no trailer names it
        let nums: Vec<u32> = self.xref.keys().copied().collect();
        let mut packed = Vec::new();
        for n in nums {
            if let Some((_, Obj::Stream(s))) = self.xref.get(&n).and_then(|e| if let Entry::At(at) = e { self.parse_at(*at) } else { None }) {
                match s.dict.get("Type").and_then(Obj::as_name) {
                    Some("ObjStm") => packed.push((n, s)),
                    Some("XRef") if self.trailer.get("Root").is_none() => self.trailer = s.dict.clone(),
                    _ => {}
                }
            }
        }
        for (n, s) in packed {
            let count = s.dict.get("N").and_then(Obj::as_i64).unwrap_or(0).max(0) as usize;
            if let Ok(dec) = decode(&s.data, &filters_of(&s.dict, false)) {
                let mut l = Lexer::new(&dec.data, 0);
                for k in 0..count {
                    if let (Some(Obj::Int(num)), Some(_)) = (l.object(), l.object()) {
                        self.xref.entry(num as u32).or_insert(Entry::In(n, k));
                    }
                }
            }
        }
        if self.trailer.get("Root").is_none() {
            let nums: Vec<u32> = self.xref.keys().copied().collect();
            for n in nums {
                if self.get(n).as_dict().and_then(|d| d.get("Type")).and_then(Obj::as_name) == Some("Catalog") {
                    self.trailer.insert("Root", Obj::Ref(n, 0));
                    break;
                }
            }
        }
        self.kept.borrow_mut().clear();
    }

    /// The object `n g obj …` at `at`: its number and the object (a stream with its bytes).
    fn parse_at(&self, at: usize) -> Option<(u32, Obj)> {
        let mut l = Lexer::new(&self.data, at);
        let num = match l.object()? {
            Obj::Int(n) => n as u32,
            _ => return None,
        };
        let generation = match l.object()? {
            Obj::Int(g) => g as u16,
            _ => return None,
        };
        if l.word() != Some(b"obj") {
            return None;
        }
        let obj = l.object().unwrap_or(Obj::Null);
        if l.word() == Some(b"stream")
            && let Obj::Dict(dict) = &obj
        {
            let d = &self.data;
            let mut start = l.p;
            if d.get(start) == Some(&b'\r') {
                start += 1;
            }
            if d.get(start) == Some(&b'\n') {
                start += 1;
            }
            let len = match dict.get("Length") {
                Some(Obj::Ref(n, _)) if !self.reading.borrow().contains(n) => self.get(*n).as_i64(),
                Some(o) => o.as_i64(),
                None => None,
            };
            // the length it gives, when "endstream" follows it; else up to "endstream"
            let fits = |end: usize| {
                let mut q = end;
                while q < d.len() && crate::object::white(d[q]) {
                    q += 1;
                }
                d.get(q..q + 9) == Some(b"endstream")
            };
            let end = match len {
                Some(n) if n >= 0 && start + (n as usize) <= d.len() && fits(start + n as usize) => start + n as usize,
                _ => {
                    let mut e = find_from(d, b"endstream", start).unwrap_or(d.len());
                    // the line end before it is not data
                    if e > start && d[e - 1] == b'\n' {
                        e -= 1;
                    }
                    if e > start && d[e - 1] == b'\r' {
                        e -= 1;
                    }
                    e
                }
            };
            return Some((num, Obj::Stream(Arc::new(Stream { dict: dict.clone(), data: d[start..end.max(start)].to_vec(), id: (num, generation) }))));
        }
        Some((num, obj))
    }

    /// Object number `n` (null when there is none).
    pub fn get(&self, n: u32) -> Obj {
        if let Some(o) = self.kept.borrow().get(&n) {
            return o.clone();
        }
        if self.reading.borrow().contains(&n) || self.reading.borrow().len() > 64 {
            return Obj::Null;
        }
        self.reading.borrow_mut().push(n);
        let mut obj = match self.xref.get(&n).copied() {
            Some(Entry::At(at)) => match self.parse_at(at) {
                Some((num, o)) if num == n => Some(o),
                _ => None,
            },
            Some(Entry::In(stream, index)) => self.packed(stream, index),
            None => None,
        };
        let mut o = obj.take().unwrap_or(Obj::Null);
        if let Some(c) = &self.crypt
            && !matches!(self.xref.get(&n), Some(Entry::In(..)))
        {
            o = c.strings(o, (n, 0));
        }
        self.reading.borrow_mut().pop();
        self.kept.borrow_mut().insert(n, o.clone());
        o
    }

    /// Object `index` of object stream `stream`.
    fn packed(&self, stream: u32, index: usize) -> Option<Obj> {
        let Obj::Stream(s) = self.get(stream) else { return None };
        let dec = self.decoded(&s).ok()?;
        let count = s.dict.get("N").and_then(Obj::as_i64).unwrap_or(0).max(0) as usize;
        let first = s.dict.get("First").and_then(Obj::as_i64).unwrap_or(0).max(0) as usize;
        let mut l = Lexer::new(&dec.data, 0);
        let mut offsets = Vec::with_capacity(count);
        for _ in 0..count {
            let (_, off) = (l.object()?.as_i64()?, l.object()?.as_i64()?);
            offsets.push(off as usize);
        }
        let mut l = Lexer::new(&dec.data, first + *offsets.get(index)?);
        l.object()
    }

    /// `o`, or the object it refers to.
    pub fn resolve(&self, o: &Obj) -> Obj {
        match o {
            Obj::Ref(n, _) => self.get(*n),
            other => other.clone(),
        }
    }

    /// `dict[key]`, the object it refers to when it is a reference.
    pub fn at(&self, dict: &Dict, key: &str) -> Option<Obj> {
        dict.get(key).map(|o| self.resolve(o)).filter(|o| *o != Obj::Null)
    }

    /// The ICC profile in stream `s` (`at` how it was named: a reference keeps it read once).
    pub fn profile(&self, s: &Stream, at: Option<&Obj>) -> Option<Arc<veymelo_color::icc::Profile>> {
        let key = match at {
            Some(Obj::Ref(n, _)) => Some(*n),
            _ => None,
        };
        if let Some(k) = key
            && let Some(p) = self.profiles.borrow().get(&k)
        {
            return p.clone();
        }
        let p = self.decoded(s).ok().and_then(|d| veymelo_color::icc::Profile::read(&d.data)).map(Arc::new);
        if let Some(k) = key {
            self.profiles.borrow_mut().insert(k, p.clone());
        }
        p
    }

    /// A stream's bytes as the file has them, unlocked.
    pub fn unlocked(&self, s: &Stream) -> Option<Vec<u8>> {
        Some(match &self.crypt {
            Some(c) => c.stream(&s.data, s.id),
            None => s.data.clone(),
        })
    }

    /// A stream's bytes, unlocked and undone through its filters.
    pub fn decoded(&self, s: &Stream) -> Result<Decoded, String> {
        let raw;
        let data = match &self.crypt {
            Some(c) if s.dict.get("Type").and_then(Obj::as_name) != Some("XRef") => {
                raw = c.stream(&s.data, s.id);
                &raw[..]
            }
            _ => &s.data[..],
        };
        // its filters and their parameters may be references
        let mut dict = s.dict.clone();
        for k in ["Filter", "DecodeParms"] {
            if let Some(v) = dict.get(k).cloned() {
                let v = match self.resolve(&v) {
                    Obj::Array(a) => Obj::Array(a.iter().map(|o| self.resolve(o)).collect()),
                    other => other,
                };
                dict.insert(k, v);
            }
        }
        decode(data, &filters_of(&dict, false))
    }

    /// The pages, in order.
    pub fn pages(&self) -> Vec<Page> {
        let mut out = Vec::new();
        let root = self.trailer.get("Root").map(|r| self.resolve(r)).unwrap_or(Obj::Null);
        let Some(top) = root.as_dict().and_then(|d| d.get("Pages")).map(|p| self.resolve(p)) else { return out };
        self.walk(&top, &Dict::default(), &mut out, 0, &mut Vec::new());
        out
    }

    fn walk(&self, node: &Obj, inherited: &Dict, out: &mut Vec<Page>, depth: usize, seen: &mut Vec<usize>) {
        let Some(d) = node.as_dict() else { return };
        if depth > 64 {
            return;
        }
        let mut inh = inherited.clone();
        for k in ["Resources", "MediaBox", "CropBox", "Rotate"] {
            if let Some(v) = d.get(k) {
                inh.insert(k, v.clone());
            }
        }
        let kids = d.get("Kids").map(|k| self.resolve(k));
        let is_pages = d.get("Type").and_then(Obj::as_name) == Some("Pages") || kids.as_ref().and_then(Obj::as_array).is_some();
        if is_pages {
            for k in kids.as_ref().and_then(Obj::as_array).unwrap_or(&[]) {
                if let Obj::Ref(n, _) = k {
                    if seen.contains(&(*n as usize)) {
                        continue;
                    }
                    seen.push(*n as usize);
                }
                self.walk(&self.resolve(k), &inh, out, depth + 1, seen);
            }
            return;
        }
        let resources = inh.get("Resources").map(|r| self.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
        let media = rect(inh.get("MediaBox").map(|m| self.resolve(m)).as_ref()).unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let crop = rect(inh.get("CropBox").map(|m| self.resolve(m)).as_ref()).map(|c| [c[0].max(media[0]), c[1].max(media[1]), c[2].min(media[2]), c[3].min(media[3])]).filter(|c| c[2] > c[0] && c[3] > c[1]).unwrap_or(media);
        let rotate = inh.get("Rotate").map(|r| self.resolve(r)).and_then(|r| r.as_i64()).unwrap_or(0).rem_euclid(360) / 90 * 90;
        out.push(Page { dict: d.clone(), resources, media, crop, rotate });
    }
}
