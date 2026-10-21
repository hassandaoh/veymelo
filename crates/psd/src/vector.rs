//! Vector masks (`vmsk`, `vsms`): paths that say which of a layer is seen, as Photoshop's path
//! records keep them — 26 bytes each: a selector, then for a subpath's first record its number
//! of knots and how it joins the shapes before it, or for a knot its three points (the control
//! point before it, the knot, the control point after it), each down then across as fixed point
//! numbers with 24 bits after the point, of the document's height and width.

use crate::Psd;

/// A knot of a path: the control point before it, itself, the control point after it (document
/// pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knot {
    pub before: [f64; 2],
    pub at: [f64; 2],
    pub after: [f64; 2],
}

/// How a shape joins those before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    /// Shown where either is (combine).
    Add,
    /// Taken away (subtract front shape).
    Subtract,
    /// Shown where both are (intersect).
    Intersect,
    /// Shown where one or the other is but not both (exclude overlapping).
    Exclude,
}

/// A subpath: its knots, whether it closes, how it joins the shapes before it, and whether it is
/// part of the shape before it (merged into it, holes made where they overlap).
#[derive(Clone, Debug, PartialEq)]
pub struct Subpath {
    pub knots: Vec<Knot>,
    pub closed: bool,
    pub join: Join,
    pub merged: bool,
}

/// A layer's vector mask: its subpaths, whether it is inverted or switched off, and whether what
/// is outside every shape starts shown.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorMask {
    pub paths: Vec<Subpath>,
    pub inverted: bool,
    pub disabled: bool,
    pub fill_starts_all: bool,
}

/// Paths from path records (for a document `w` × `h`).
pub fn paths(d: &[u8], w: f64, h: f64) -> (Vec<Subpath>, bool) {
    let fixed = |b: &[u8]| i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64 / (1 << 24) as f64;
    let point = |b: &[u8]| [fixed(&b[4..8]) * w, fixed(&b[0..4]) * h];
    let mut out: Vec<Subpath> = Vec::new();
    let mut all = false;
    for rec in d.as_chunks::<26>().0 {
        let sel = u16::from_be_bytes([rec[0], rec[1]]);
        let body = &rec[2..];
        match sel {
            0 | 3 => {
                // how it joins: −1 merged into the shape before, 0 exclude, 1 combine, 2
                // subtract, 3 intersect (files before Photoshop CS6 keep 0 here: combine)
                let op = i16::from_be_bytes([body[2], body[3]]);
                let newer = body[2..].iter().any(|&b| b != 0);
                let (join, merged) = match (op, newer) {
                    (-1, _) => (out.last().map_or(Join::Add, |p| p.join), true),
                    (0, true) => (Join::Exclude, false),
                    (2, _) => (Join::Subtract, false),
                    (3, _) => (Join::Intersect, false),
                    _ => (Join::Add, false),
                };
                out.push(Subpath { knots: Vec::new(), closed: sel == 0, join, merged });
            }
            1 | 2 | 4 | 5 => {
                if let Some(p) = out.last_mut() {
                    p.knots.push(Knot { before: point(&body[0..8]), at: point(&body[8..16]), after: point(&body[16..24]) });
                }
            }
            8 => all = u16::from_be_bytes([body[0], body[1]]) != 0,
            _ => {}
        }
    }
    out.retain(|p| !p.knots.is_empty());
    (out, all)
}

impl Psd {
    /// Layer `i`'s vector mask.
    pub fn vector_mask(&self, i: usize) -> Option<VectorMask> {
        let d = self.block(i, b"vmsk").or_else(|| self.block(i, b"vsms"))?;
        if d.len() < 8 {
            return None;
        }
        let flags = u32::from_be_bytes([d[4], d[5], d[6], d[7]]);
        let (paths, fill_starts_all) = paths(&d[8..], self.width as f64, self.height as f64);
        Some(VectorMask { paths, inverted: flags & 1 != 0, disabled: flags & 4 != 0, fill_starts_all })
    }
}
