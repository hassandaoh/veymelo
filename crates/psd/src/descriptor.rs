//! Photoshop's descriptors ("action descriptors"): a class and a list of keyed values — numbers,
//! numbers with units, text, enumerations, flags, lists, other descriptors, references and raw
//! data — as layer effects, fill layers, type layers and smart objects keep their settings.

/// A value in a descriptor.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Desc(Desc),
    List(Vec<Item>),
    Double(f64),
    /// A number and its unit (`#Ang` degrees, `#Prc` percent, `#Pxl` pixels, `#Rlt` points,
    /// `#Nne` none).
    Unit(String, f64),
    Text(String),
    /// An enumeration's type and value.
    Enum(String, String),
    Int(i64),
    Bool(bool),
    Class(String),
    Data(Vec<u8>),
    /// A reference (kept as its items' names only).
    Reference(Vec<String>),
}

/// A descriptor: its class and its items in the order the file keeps them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Desc {
    pub class: String,
    pub items: Vec<(String, Item)>,
}

impl Desc {
    pub fn get(&self, key: &str) -> Option<&Item> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// A number (a double, a whole number, or a number with a unit).
    pub fn num(&self, key: &str) -> Option<f64> {
        match self.get(key)? {
            Item::Double(v) | Item::Unit(_, v) => Some(*v),
            Item::Int(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn flag(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            Item::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Item::Text(t) => Some(t),
            _ => None,
        }
    }

    /// An enumeration's value.
    pub fn enumeration(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Item::Enum(_, v) => Some(v),
            _ => None,
        }
    }

    pub fn desc(&self, key: &str) -> Option<&Desc> {
        match self.get(key)? {
            Item::Desc(d) => Some(d),
            _ => None,
        }
    }

    pub fn list(&self, key: &str) -> Option<&[Item]> {
        match self.get(key)? {
            Item::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn data(&self, key: &str) -> Option<&[u8]> {
        match self.get(key)? {
            Item::Data(d) => Some(d),
            _ => None,
        }
    }

    /// A colour descriptor (`Clr `) as straight sRGB from 0 to 1: RGB as it is (0 to 255),
    /// greys, HSB and CMYK made RGB.
    pub fn color(&self, key: &str) -> Option<[f64; 3]> {
        color_of(self.desc(key)?)
    }
}

/// A colour descriptor's RGB (0 to 1).
pub fn color_of(c: &Desc) -> Option<[f64; 3]> {
    let n = |k: &str| c.num(k).unwrap_or(0.0);
    Some(match c.class.as_str() {
        "RGBC" | "RGBColor" => {
            let r = c.num("Rd  ").or(c.num("redFloat").map(|v| v * 255.0)).unwrap_or(0.0);
            let g = c.num("Grn ").or(c.num("greenFloat").map(|v| v * 255.0)).unwrap_or(0.0);
            let b = c.num("Bl  ").or(c.num("blueFloat").map(|v| v * 255.0)).unwrap_or(0.0);
            [r / 255.0, g / 255.0, b / 255.0]
        }
        "Grsc" => {
            let v = 1.0 - n("Gry ") / 100.0;
            [v, v, v]
        }
        "HSBC" => {
            let (h, s, v) = (n("H   ") / 360.0, n("Strt") / 100.0, n("Brgh") / 100.0);
            let i = (h * 6.0).floor();
            let f = h * 6.0 - i;
            let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
            match i as i64 % 6 {
                0 => [v, t, p],
                1 => [q, v, p],
                2 => [p, v, t],
                3 => [p, q, v],
                4 => [t, p, v],
                _ => [v, p, q],
            }
        }
        "CMYC" => {
            let (cc, m, y, k) = (n("Cyn ") / 100.0, n("Mgnt") / 100.0, n("Ylw ") / 100.0, n("Blck") / 100.0);
            [(1.0 - cc) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)]
        }
        _ => return None,
    }
    .map(|v: f64| v.clamp(0.0, 1.0)))
}

/// Reads a descriptor's parts off bytes.
pub(crate) struct Reader<'a> {
    pub d: &'a [u8],
    pub at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.d.len()).ok_or("a descriptor cut short")?;
        let s = &self.d[self.at..end];
        self.at = end;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A Unicode string: its length in UTF-16 units, then those (a last zero dropped).
    pub fn unicode(&mut self) -> Result<String, String> {
        let n = self.u32()? as usize;
        if n > 1 << 24 {
            return Err("a descriptor string too long".into());
        }
        let b = self.take(2 * n)?;
        let units: Vec<u16> = b.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        Ok(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string())
    }

    /// A key or class id: its length then that many bytes, or (length 0) four bytes.
    fn id(&mut self) -> Result<String, String> {
        let n = self.u32()? as usize;
        let n = if n == 0 { 4 } else { n };
        if n > 1 << 16 {
            return Err("a descriptor id too long".into());
        }
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }

    /// A descriptor: its name, class, and items.
    pub fn desc(&mut self, depth: usize) -> Result<Desc, String> {
        if depth > 64 {
            return Err("descriptors nested too deep".into());
        }
        self.unicode()?;
        let class = self.id()?;
        let n = self.u32()? as usize;
        let mut items = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            let key = self.id()?;
            let v = self.item(depth)?;
            items.push((key, v));
        }
        Ok(Desc { class, items })
    }

    fn item(&mut self, depth: usize) -> Result<Item, String> {
        let ty: [u8; 4] = self.take(4)?.try_into().expect("four bytes");
        Ok(match &ty {
            b"Objc" | b"GlbO" => Item::Desc(self.desc(depth + 1)?),
            b"VlLs" => {
                let n = self.u32()? as usize;
                let mut v = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    v.push(self.item(depth + 1)?);
                }
                Item::List(v)
            }
            b"doub" => Item::Double(f64::from_be_bytes(self.take(8)?.try_into().expect("eight bytes"))),
            b"UntF" => {
                let unit = String::from_utf8_lossy(self.take(4)?).into_owned();
                Item::Unit(unit, f64::from_be_bytes(self.take(8)?.try_into().expect("eight bytes")))
            }
            b"UnFl" => {
                let unit = String::from_utf8_lossy(self.take(4)?).into_owned();
                let n = self.u32()? as usize;
                let mut v = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    v.push(Item::Unit(unit.clone(), f64::from_be_bytes(self.take(8)?.try_into().expect("eight bytes"))));
                }
                Item::List(v)
            }
            b"TEXT" => Item::Text(self.unicode()?),
            b"enum" => {
                let t = self.id()?;
                Item::Enum(t, self.id()?)
            }
            b"long" => Item::Int(self.u32()? as i32 as i64),
            b"comp" => Item::Int(i64::from_be_bytes(self.take(8)?.try_into().expect("eight bytes"))),
            b"bool" => Item::Bool(self.take(1)?[0] != 0),
            b"type" | b"GlbC" => {
                self.unicode()?;
                Item::Class(self.id()?)
            }
            b"alis" | b"tdta" | b"Pth " => {
                let n = self.u32()? as usize;
                Item::Data(self.take(n)?.to_vec())
            }
            b"obj " => {
                let n = self.u32()? as usize;
                let mut names = Vec::new();
                for _ in 0..n {
                    let kind: [u8; 4] = self.take(4)?.try_into().expect("four bytes");
                    match &kind {
                        b"prop" => {
                            self.unicode()?;
                            self.id()?;
                            names.push(self.id()?);
                        }
                        b"Clss" => {
                            self.unicode()?;
                            names.push(self.id()?);
                        }
                        b"Enmr" => {
                            self.unicode()?;
                            self.id()?;
                            self.id()?;
                            names.push(self.id()?);
                        }
                        b"rele" => {
                            self.unicode()?;
                            names.push(self.id()?);
                            self.u32()?;
                        }
                        b"Idnt" | b"indx" => names.push(self.u32()?.to_string()),
                        b"name" => {
                            self.unicode()?;
                            self.id()?;
                            names.push(self.unicode()?);
                        }
                        other => return Err(format!("a reference of an unknown kind ({})", String::from_utf8_lossy(other))),
                    }
                }
                Item::Reference(names)
            }
            other => return Err(format!("a descriptor value of an unknown kind ({})", String::from_utf8_lossy(other))),
        })
    }
}

/// A descriptor from bytes (its version, four bytes, first when `versioned`).
pub fn read(d: &[u8], versioned: bool) -> Result<Desc, String> {
    let mut r = Reader { d, at: if versioned { 4 } else { 0 } };
    r.desc(0)
}

pub mod write {
    //! Descriptors written (for documents made to read back).

    use super::{Desc, Item};

    fn unicode(out: &mut Vec<u8>, s: &str) {
        let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        out.extend((units.len() as u32).to_be_bytes());
        for u in units {
            out.extend(u.to_be_bytes());
        }
    }

    fn id(out: &mut Vec<u8>, s: &str) {
        if s.len() == 4 {
            out.extend(0u32.to_be_bytes());
        } else {
            out.extend((s.len() as u32).to_be_bytes());
        }
        out.extend(s.as_bytes());
    }

    pub fn desc(out: &mut Vec<u8>, d: &Desc) {
        unicode(out, "");
        id(out, &d.class);
        out.extend((d.items.len() as u32).to_be_bytes());
        for (k, v) in &d.items {
            id(out, k);
            item(out, v);
        }
    }

    fn item(out: &mut Vec<u8>, v: &Item) {
        match v {
            Item::Desc(d) => {
                out.extend(b"Objc");
                desc(out, d);
            }
            Item::List(l) => {
                out.extend(b"VlLs");
                out.extend((l.len() as u32).to_be_bytes());
                for i in l {
                    item(out, i);
                }
            }
            Item::Double(x) => {
                out.extend(b"doub");
                out.extend(x.to_be_bytes());
            }
            Item::Unit(u, x) => {
                out.extend(b"UntF");
                out.extend(u.as_bytes());
                out.extend(x.to_be_bytes());
            }
            Item::Text(t) => {
                out.extend(b"TEXT");
                unicode(out, t);
            }
            Item::Enum(t, e) => {
                out.extend(b"enum");
                id(out, t);
                id(out, e);
            }
            Item::Int(n) => {
                out.extend(b"long");
                out.extend((*n as i32).to_be_bytes());
            }
            Item::Bool(b) => {
                out.extend(b"bool");
                out.push(*b as u8);
            }
            Item::Class(c) => {
                out.extend(b"type");
                unicode(out, "");
                id(out, c);
            }
            Item::Data(d) => {
                out.extend(b"tdta");
                out.extend((d.len() as u32).to_be_bytes());
                out.extend(d);
            }
            Item::Reference(_) => {
                out.extend(b"obj ");
                out.extend(0u32.to_be_bytes());
            }
        }
    }

    /// A descriptor's bytes, its version (16) first.
    pub fn versioned(d: &Desc) -> Vec<u8> {
        let mut out = 16u32.to_be_bytes().to_vec();
        desc(&mut out, d);
        out
    }
}
