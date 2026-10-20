//! Path data (`d`): moves, lines, horizontal and vertical lines, cubic and quadratic curves and
//! their smooth forms, elliptical arcs and closes, absolute and relative, as runs of points with
//! tangents (an arc made of cubic pieces of at most a quarter turn each).

use veymelo_path::{Sub, Vertex};

/// Reads numbers and command letters off path data.
struct Scan<'a> {
    b: &'a [u8],
    i: usize,
}

impl Scan<'_> {
    /// Past spaces and commas.
    fn skip(&mut self) {
        while self.i < self.b.len() && (self.b[self.i].is_ascii_whitespace() || self.b[self.i] == b',') {
            self.i += 1;
        }
    }

    fn done(&mut self) -> bool {
        self.skip();
        self.i >= self.b.len()
    }

    /// The command letter next, if one is.
    fn letter(&mut self) -> Option<u8> {
        self.skip();
        let c = *self.b.get(self.i)?;
        (c.is_ascii_alphabetic() && c != b'e' && c != b'E').then(|| {
            self.i += 1;
            c
        })
    }

    /// The number next: a sign, digits, a point, more digits, an exponent; a second point or a
    /// sign ends it.
    fn number(&mut self) -> Option<f64> {
        self.skip();
        let start = self.i;
        let b = self.b;
        let mut j = self.i;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let mut digits = false;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
            digits = true;
        }
        if j < b.len() && b[j] == b'.' {
            j += 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
                digits = true;
            }
        }
        if !digits {
            return None;
        }
        if j < b.len() && (b[j] == b'e' || b[j] == b'E') {
            let mut k = j + 1;
            if k < b.len() && (b[k] == b'+' || b[k] == b'-') {
                k += 1;
            }
            if k < b.len() && b[k].is_ascii_digit() {
                while k < b.len() && b[k].is_ascii_digit() {
                    k += 1;
                }
                j = k;
            }
        }
        self.i = j;
        std::str::from_utf8(&b[start..j]).ok()?.parse().ok()
    }

    /// An arc's flag: 0 or 1, written alone or run into what follows.
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let c = *self.b.get(self.i)?;
        (c == b'0' || c == b'1').then(|| {
            self.i += 1;
            c == b'1'
        })
    }
}

/// The numbers in a list (commas or spaces between them, or none where a sign or a second point
/// starts the next).
pub fn numbers(s: &str) -> Vec<f64> {
    let mut scan = Scan { b: s.as_bytes(), i: 0 };
    let mut out = Vec::new();
    while let Some(v) = scan.number() {
        out.push(v);
    }
    out
}

/// Runs of points being made.
#[derive(Default)]
struct Build {
    subs: Vec<Sub>,
    cur: Option<Sub>,
}

impl Build {
    fn finish(&mut self) {
        if let Some(s) = self.cur.take()
            && s.v.len() > 1
        {
            self.subs.push(s);
        }
    }

    fn move_to(&mut self, p: [f64; 2]) {
        self.finish();
        self.cur = Some(Sub { v: vec![Vertex::at(p[0], p[1])], closed: false });
    }

    /// The run being made, a new one from `from` if a close ended the last.
    fn run(&mut self, from: [f64; 2]) -> &mut Sub {
        if self.cur.as_ref().is_none_or(|s| s.closed) {
            self.finish();
            self.cur = Some(Sub { v: vec![Vertex::at(from[0], from[1])], closed: false });
        }
        self.cur.as_mut().expect("just made")
    }

    fn line_to(&mut self, from: [f64; 2], p: [f64; 2]) {
        self.run(from).v.push(Vertex::at(p[0], p[1]));
    }

    fn cubic_to(&mut self, from: [f64; 2], a: [f64; 2], b: [f64; 2], p: [f64; 2]) {
        let run = self.run(from);
        let last = run.v.last_mut().expect("a run has a point");
        last.o = [a[0] - last.p[0], a[1] - last.p[1]];
        run.v.push(Vertex { p, i: [b[0] - p[0], b[1] - p[1]], o: [0.0; 2] });
    }

    fn close(&mut self) {
        if let Some(s) = self.cur.as_mut() {
            // ending where it started: the two are one point
            if s.v.len() > 2 {
                let (first, last) = (s.v[0], s.v[s.v.len() - 1]);
                if (first.p[0] - last.p[0]).abs() < 1e-9 && (first.p[1] - last.p[1]).abs() < 1e-9 {
                    s.v[0].i = last.i;
                    s.v.pop();
                }
            }
            s.closed = true;
        }
    }
}

/// An elliptical arc from `p0` to `p1` (radii, turn in degrees, the large and sweep flags) as
/// cubic pieces: each piece's two handles and its end.
fn arc(p0: [f64; 2], r: [f64; 2], phi: f64, large: bool, sweep: bool, p1: [f64; 2]) -> Vec<[[f64; 2]; 3]> {
    let (mut rx, mut ry) = (r[0].abs(), r[1].abs());
    if (p0[0] - p1[0]).abs() < 1e-12 && (p0[1] - p1[1]).abs() < 1e-12 {
        return Vec::new();
    }
    if rx < 1e-12 || ry < 1e-12 {
        return vec![[p0, p1, p1]];
    }
    let (s, c) = phi.to_radians().sin_cos();
    let (dx, dy) = ((p0[0] - p1[0]) / 2.0, (p0[1] - p1[1]) / 2.0);
    let (x1, y1) = (c * dx + s * dy, -s * dx + c * dy);
    // radii too small to reach: grown just enough
    let lambda = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let k = (num / den).max(0.0).sqrt() * if large == sweep { -1.0 } else { 1.0 };
    let (cx1, cy1) = (k * rx * y1 / ry, -k * ry * x1 / rx);
    let centre = [c * cx1 - s * cy1 + (p0[0] + p1[0]) / 2.0, s * cx1 + c * cy1 + (p0[1] + p1[1]) / 2.0];
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let (ux, uy) = ((x1 - cx1) / rx, (y1 - cy1) / ry);
    let (vx, vy) = ((-x1 - cx1) / rx, (-y1 - cy1) / ry);
    let start = angle(1.0, 0.0, ux, uy);
    let mut turn = angle(ux, uy, vx, vy);
    if !sweep && turn > 0.0 {
        turn -= std::f64::consts::TAU;
    } else if sweep && turn < 0.0 {
        turn += std::f64::consts::TAU;
    }
    let n = (turn.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = turn / n as f64;
    let handle = 4.0 / 3.0 * (step / 4.0).tan();
    let at = |t: f64| {
        let (st, ct) = t.sin_cos();
        [centre[0] + c * rx * ct - s * ry * st, centre[1] + s * rx * ct + c * ry * st]
    };
    let along = |t: f64| {
        let (st, ct) = t.sin_cos();
        [-c * rx * st - s * ry * ct, -s * rx * st + c * ry * ct]
    };
    (0..n)
        .map(|k| {
            let (t0, t1) = (start + step * k as f64, start + step * (k + 1) as f64);
            let (a, b) = (at(t0), if k + 1 == n { p1 } else { at(t1) });
            let (da, db) = (along(t0), along(t1));
            [[a[0] + handle * da[0], a[1] + handle * da[1]], [b[0] - handle * db[0], b[1] - handle * db[1]], b]
        })
        .collect()
}

/// The runs of points path data `d` draws (what it says up to its first mistake).
pub fn parse(d: &str) -> Vec<Sub> {
    let mut scan = Scan { b: d.as_bytes(), i: 0 };
    let mut out = Build::default();
    let (mut here, mut start) = ([0.0f64; 2], [0.0f64; 2]);
    // the last cubic's second handle and the last quadratic's handle, for the smooth forms
    let (mut last_cubic, mut last_quad): (Option<[f64; 2]>, Option<[f64; 2]>) = (None, None);
    let mut command = 0u8;
    while !scan.done() {
        let c = match scan.letter() {
            Some(c) => c,
            // more numbers: the command again (after a move, lines)
            None if command == b'M' => b'L',
            None if command == b'm' => b'l',
            None if command != 0 && command != b'Z' && command != b'z' => command,
            None => break,
        };
        command = c;
        let rel = c.is_ascii_lowercase();
        let base = if rel { here } else { [0.0, 0.0] };
        let pt = |scan: &mut Scan| -> Option<[f64; 2]> { Some([base[0] + scan.number()?, base[1] + scan.number()?]) };
        let ok = match c.to_ascii_uppercase() {
            b'M' => pt(&mut scan).map(|p| {
                out.move_to(p);
                here = p;
                start = p;
            }),
            b'L' => pt(&mut scan).map(|p| {
                out.line_to(here, p);
                here = p;
            }),
            b'H' => scan.number().map(|x| {
                let p = [if rel { here[0] + x } else { x }, here[1]];
                out.line_to(here, p);
                here = p;
            }),
            b'V' => scan.number().map(|y| {
                let p = [here[0], if rel { here[1] + y } else { y }];
                out.line_to(here, p);
                here = p;
            }),
            b'C' => (|| {
                let (a, b, p) = (pt(&mut scan)?, pt(&mut scan)?, pt(&mut scan)?);
                out.cubic_to(here, a, b, p);
                last_cubic = Some(b);
                here = p;
                Some(())
            })(),
            b'S' => (|| {
                let a = last_cubic.map_or(here, |b| [2.0 * here[0] - b[0], 2.0 * here[1] - b[1]]);
                let (b, p) = (pt(&mut scan)?, pt(&mut scan)?);
                out.cubic_to(here, a, b, p);
                last_cubic = Some(b);
                here = p;
                Some(())
            })(),
            b'Q' | b'T' => (|| {
                let q = if c.eq_ignore_ascii_case(&b'Q') { pt(&mut scan)? } else { last_quad.map_or(here, |q| [2.0 * here[0] - q[0], 2.0 * here[1] - q[1]]) };
                let p = pt(&mut scan)?;
                let third = |a: f64, b: f64| a + 2.0 / 3.0 * (b - a);
                out.cubic_to(here, [third(here[0], q[0]), third(here[1], q[1])], [third(p[0], q[0]), third(p[1], q[1])], p);
                last_quad = Some(q);
                here = p;
                Some(())
            })(),
            b'A' => (|| {
                let r = [scan.number()?, scan.number()?];
                let phi = scan.number()?;
                let (large, sweep) = (scan.flag()?, scan.flag()?);
                let p = pt(&mut scan)?;
                for [a, b, e] in arc(here, r, phi, large, sweep, p) {
                    out.cubic_to(here, a, b, e);
                    here = e;
                }
                here = p;
                Some(())
            })(),
            b'Z' => {
                out.close();
                here = start;
                Some(())
            }
            _ => None,
        };
        if ok.is_none() {
            break;
        }
        if !matches!(c.to_ascii_uppercase(), b'C' | b'S') {
            last_cubic = None;
        }
        if !matches!(c.to_ascii_uppercase(), b'Q' | b'T') {
            last_quad = None;
        }
    }
    out.finish();
    out.subs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(s: &Sub) -> Vec<[f64; 2]> {
        s.v.iter().map(|v| v.p).collect()
    }

    #[test]
    fn every_command() {
        assert_eq!(numbers("1.5.5-2e1,3 -.25"), vec![1.5, 0.5, -20.0, 3.0, -0.25]);
        // lines, absolute and relative, a close ending where it started
        let s = parse("M10 10 h20 v20 H10 z m5 5 l1 1");
        assert_eq!(s.len(), 2);
        assert_eq!(points(&s[0]), vec![[10.0, 10.0], [30.0, 10.0], [30.0, 30.0], [10.0, 30.0]]);
        assert!(s[0].closed && !s[1].closed);
        assert_eq!(points(&s[1]), vec![[15.0, 15.0], [16.0, 16.0]], "after a close, relative to where it started");
        // a cubic and its smooth form reflecting the handle
        let s = parse("M0 0 C 10 0 20 10 20 20 S 30 40 40 40");
        assert_eq!(s[0].v[1].i, [0.0, -10.0]);
        assert_eq!(s[0].v[1].o, [0.0, 10.0], "the smooth curve's first handle is the last one reflected");
        // a quadratic as a cubic: handles two thirds of the way to its one
        let s = parse("M0 0 Q 30 30 60 0");
        assert_eq!((s[0].v[0].o, s[0].v[1].i), ([20.0, 20.0], [-20.0, 20.0]));
        // a half circle by an arc, run-together flags
        let s = parse("M0 0 A25 25 0 0150 0");
        let last = s[0].v.last().unwrap().p;
        assert!((last[0] - 50.0).abs() < 1e-9 && last[1].abs() < 1e-9);
        assert_eq!(s[0].v.len(), 3, "two quarter turns");
        let mid = s[0].v[1].p;
        assert!((mid[0] - 25.0).abs() < 1e-6 && (mid[1].abs() - 25.0).abs() < 1e-6, "{mid:?}");
        // a mistake stops it where it is
        assert_eq!(parse("M0 0 L10 10 L 5").len(), 1);
    }
}
