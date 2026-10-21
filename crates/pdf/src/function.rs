//! PDF functions, which give the colours of gradients and of colour spaces' tints: sampled
//! (type 0, read between its samples), exponential (type 2), stitched from others (type 3) and
//! PostScript calculator programs (type 4).

use crate::file::File;
use crate::object::{Lexer, Obj};

#[derive(Clone, Debug)]
pub enum Function {
    Sampled { domain: Vec<f64>, range: Vec<f64>, size: Vec<usize>, encode: Vec<f64>, decode: Vec<f64>, samples: Vec<f64> },
    Exponential { domain: Vec<f64>, c0: Vec<f64>, c1: Vec<f64>, n: f64, range: Vec<f64> },
    Stitched { domain: Vec<f64>, parts: Vec<Function>, bounds: Vec<f64>, encode: Vec<f64>, range: Vec<f64> },
    Calculator { domain: Vec<f64>, range: Vec<f64>, program: Vec<Op> },
    /// Several, one an output each.
    Each(Vec<Function>),
}

#[derive(Clone, Debug)]
pub enum Op {
    Num(f64),
    Word(String),
    /// `{…} if`, `{…} {…} ifelse`.
    If(Vec<Op>, Option<Vec<Op>>),
}

fn nums(o: Option<Obj>) -> Vec<f64> {
    o.and_then(|o| o.as_array().map(|a| a.iter().filter_map(Obj::as_f64).collect())).unwrap_or_default()
}

fn interpolate(x: f64, x0: f64, x1: f64, y0: f64, y1: f64) -> f64 {
    if (x1 - x0).abs() < 1e-12 { y0 } else { y0 + (x - x0) * (y1 - y0) / (x1 - x0) }
}

fn clip_to(v: &mut [f64], range: &[f64]) {
    for (i, x) in v.iter_mut().enumerate() {
        if let (Some(&lo), Some(&hi)) = (range.get(2 * i), range.get(2 * i + 1)) {
            *x = x.clamp(lo.min(hi), hi.max(lo));
        }
    }
}

/// Parse a calculator program's `{ … }` from `l` (its opening brace read).
fn program(l: &mut Lexer) -> Vec<Op> {
    let mut out: Vec<Op> = Vec::new();
    let mut blocks: Vec<Vec<Op>> = Vec::new();
    while let Some(o) = l.object() {
        match o {
            Obj::Word(w) if w == "{" => blocks.push(program(l)),
            Obj::Word(w) if w == "}" => break,
            Obj::Word(w) if w == "if" => {
                let a = blocks.pop().unwrap_or_default();
                out.push(Op::If(a, None));
            }
            Obj::Word(w) if w == "ifelse" => {
                let b = blocks.pop().unwrap_or_default();
                let a = blocks.pop().unwrap_or_default();
                out.push(Op::If(a, Some(b)));
            }
            Obj::Word(w) => out.push(Op::Word(w)),
            Obj::Bool(b) => out.push(Op::Word(if b { "true".into() } else { "false".into() })),
            o => out.push(Op::Num(o.as_f64().unwrap_or(0.0))),
        }
    }
    out
}

impl Function {
    /// The function `o` stands for (none when it is not one that can be read).
    pub fn read(file: &File, o: &Obj) -> Option<Function> {
        Function::read_depth(file, o, 0)
    }

    fn read_depth(file: &File, o: &Obj, depth: usize) -> Option<Function> {
        if depth > 16 {
            return None;
        }
        let o = file.resolve(o);
        if let Obj::Array(a) = &o {
            return Some(Function::Each(a.iter().filter_map(|f| Function::read_depth(file, f, depth + 1)).collect()));
        }
        let d = o.as_dict()?;
        let get = |k: &str| file.at(d, k);
        let domain = nums(get("Domain"));
        let range = nums(get("Range"));
        match get("FunctionType")?.as_i64()? {
            0 => {
                let Obj::Stream(s) = &o else { return None };
                let size: Vec<usize> = nums(get("Size")).iter().map(|&v| v.max(1.0) as usize).collect();
                let bps = get("BitsPerSample").and_then(|v| v.as_i64()).unwrap_or(8).clamp(1, 32) as usize;
                let mut encode = nums(get("Encode"));
                if encode.is_empty() {
                    encode = size.iter().flat_map(|&s| [0.0, (s - 1) as f64]).collect();
                }
                let decode = {
                    let d = nums(get("Decode"));
                    if d.is_empty() { range.clone() } else { d }
                };
                let data = file.decoded(s).ok()?.data;
                let outputs = range.len() / 2;
                let count = size.iter().product::<usize>() * outputs;
                let max = ((1u64 << bps) - 1) as f64;
                let mut samples = Vec::with_capacity(count);
                let mut bit = 0usize;
                for _ in 0..count {
                    let mut v = 0u64;
                    for _ in 0..bps {
                        let byte = *data.get(bit / 8).unwrap_or(&0);
                        v = (v << 1) | ((byte >> (7 - bit % 8)) & 1) as u64;
                        bit += 1;
                    }
                    samples.push(v as f64 / max);
                }
                Some(Function::Sampled { domain, range, size, encode, decode, samples })
            }
            2 => {
                let c0 = nums(get("C0"));
                let c1 = nums(get("C1"));
                Some(Function::Exponential {
                    domain,
                    c0: if c0.is_empty() { vec![0.0] } else { c0 },
                    c1: if c1.is_empty() { vec![1.0] } else { c1 },
                    n: get("N").and_then(|v| v.as_f64()).unwrap_or(1.0),
                    range,
                })
            }
            3 => {
                let parts = get("Functions")?.as_array()?.iter().filter_map(|f| Function::read_depth(file, f, depth + 1)).collect();
                Some(Function::Stitched { domain, parts, bounds: nums(get("Bounds")), encode: nums(get("Encode")), range })
            }
            4 => {
                let Obj::Stream(s) = &o else { return None };
                let data = file.decoded(s).ok()?.data;
                let mut l = Lexer::new(&data, 0);
                // to the first brace
                while let Some(o) = l.object() {
                    if o == Obj::Word("{".into()) {
                        break;
                    }
                }
                Some(Function::Calculator { domain, range, program: program(&mut l) })
            }
            _ => None,
        }
    }

    /// Whether its outputs change in a straight line with its input (one input), so a
    /// gradient's stops need only its ends and joins.
    pub fn straight(&self) -> bool {
        match self {
            Function::Exponential { n, .. } => (*n - 1.0).abs() < 1e-9,
            Function::Stitched { parts, .. } => parts.iter().all(Function::straight),
            Function::Each(fs) => fs.iter().all(Function::straight),
            _ => false,
        }
    }

    /// Where its input changes how it works (a stitched function's bounds), inside `t0`..`t1`.
    pub fn joins(&self) -> Vec<f64> {
        match self {
            Function::Stitched { bounds, .. } => bounds.clone(),
            Function::Each(fs) => fs.iter().flat_map(Function::joins).collect(),
            _ => Vec::new(),
        }
    }

    pub fn eval(&self, x: &[f64]) -> Vec<f64> {
        self.eval_depth(x, 0)
    }

    fn eval_depth(&self, x: &[f64], depth: usize) -> Vec<f64> {
        if depth > 16 {
            return Vec::new();
        }
        let clamp_in = |x: &[f64], domain: &[f64]| -> Vec<f64> {
            x.iter().enumerate().map(|(i, &v)| match (domain.get(2 * i), domain.get(2 * i + 1)) {
                (Some(&a), Some(&b)) => v.clamp(a.min(b), b.max(a)),
                _ => v,
            }).collect()
        };
        match self {
            Function::Each(fs) => fs.iter().flat_map(|f| f.eval_depth(x, depth + 1).into_iter().take(1)).collect(),
            Function::Exponential { domain, c0, c1, n, range } => {
                let t = clamp_in(x, domain).first().copied().unwrap_or(0.0);
                let tn = if *n == 1.0 { t } else { t.max(0.0).powf(*n) };
                let mut out: Vec<f64> = c0.iter().zip(c1).map(|(a, b)| a + tn * (b - a)).collect();
                clip_to(&mut out, range);
                out
            }
            Function::Stitched { domain, parts, bounds, encode, range } => {
                let t = clamp_in(x, domain).first().copied().unwrap_or(0.0);
                let (d0, d1) = (domain.first().copied().unwrap_or(0.0), domain.get(1).copied().unwrap_or(1.0));
                let k = bounds.iter().position(|&b| t < b).unwrap_or(bounds.len()).min(parts.len().saturating_sub(1));
                let lo = if k == 0 { d0 } else { bounds[k - 1] };
                let hi = if k < bounds.len() { bounds[k] } else { d1 };
                let (e0, e1) = (encode.get(2 * k).copied().unwrap_or(0.0), encode.get(2 * k + 1).copied().unwrap_or(1.0));
                let mut out = parts.get(k).map(|p| p.eval_depth(&[interpolate(t, lo, hi, e0, e1)], depth + 1)).unwrap_or_default();
                clip_to(&mut out, range);
                out
            }
            Function::Sampled { domain, range, size, encode, decode, samples } => {
                let x = clamp_in(x, domain);
                let m = size.len();
                let outputs = range.len() / 2;
                // each input's place among its samples, and the corners round it
                let mut base = vec![0usize; m];
                let mut frac = vec![0.0; m];
                for i in 0..m {
                    let e = interpolate(x.get(i).copied().unwrap_or(0.0), domain[2 * i], domain[2 * i + 1], encode[2 * i], encode[2 * i + 1]).clamp(0.0, (size[i] - 1) as f64);
                    base[i] = (e.floor() as usize).min(size[i].saturating_sub(2));
                    frac[i] = if size[i] < 2 { 0.0 } else { e - base[i] as f64 };
                }
                let mut out = vec![0.0; outputs];
                for corner in 0..(1usize << m) {
                    let mut w = 1.0;
                    let mut index = 0;
                    let mut stride = 1;
                    for i in 0..m {
                        let up = corner >> i & 1 == 1;
                        if up && size[i] < 2 {
                            w = 0.0;
                        }
                        w *= if up { frac[i] } else { 1.0 - frac[i] };
                        index += (base[i] + usize::from(up)) * stride;
                        stride *= size[i];
                    }
                    if w == 0.0 {
                        continue;
                    }
                    for (j, o) in out.iter_mut().enumerate() {
                        *o += w * samples.get(index * outputs + j).copied().unwrap_or(0.0);
                    }
                }
                for (j, o) in out.iter_mut().enumerate() {
                    *o = decode.get(2 * j).copied().unwrap_or(0.0) + *o * (decode.get(2 * j + 1).copied().unwrap_or(1.0) - decode.get(2 * j).copied().unwrap_or(0.0));
                }
                clip_to(&mut out, range);
                out
            }
            Function::Calculator { domain, range, program } => {
                let mut stack: Vec<f64> = clamp_in(x, domain);
                run(program, &mut stack, 0);
                let n = range.len() / 2;
                let mut out: Vec<f64> = stack.iter().rev().take(n).rev().copied().collect();
                clip_to(&mut out, range);
                out
            }
        }
    }
}

fn pop(s: &mut Vec<f64>) -> f64 {
    s.pop().unwrap_or(0.0)
}

/// A calculator program on `stack` (booleans as 1 and 0).
fn run(program: &[Op], stack: &mut Vec<f64>, depth: usize) {
    if depth > 32 {
        return;
    }
    for op in program {
        if stack.len() > 1000 {
            return;
        }
        match op {
            Op::Num(v) => stack.push(*v),
            Op::If(a, b) => {
                let c = pop(stack) != 0.0;
                if c {
                    run(a, stack, depth + 1);
                } else if let Some(b) = b {
                    run(b, stack, depth + 1);
                }
            }
            Op::Word(w) => {
                let b2 = |v: bool| if v { 1.0 } else { 0.0 };
                let v = match w.as_str() {
                    "abs" => pop(stack).abs(),
                    "neg" => -pop(stack),
                    "ceiling" => pop(stack).ceil(),
                    "floor" => pop(stack).floor(),
                    "round" => (pop(stack) + 0.5).floor(),
                    "truncate" | "cvi" => pop(stack).trunc(),
                    "cvr" => pop(stack),
                    "sqrt" => pop(stack).max(0.0).sqrt(),
                    "sin" => pop(stack).to_radians().sin(),
                    "cos" => pop(stack).to_radians().cos(),
                    "ln" => pop(stack).ln(),
                    "log" => pop(stack).log10(),
                    "not" => {
                        let v = pop(stack);
                        if v == 0.0 || v == 1.0 { 1.0 - v } else { !(v as i64) as f64 }
                    }
                    "true" => 1.0,
                    "false" => 0.0,
                    "dup" => {
                        let v = pop(stack);
                        stack.push(v);
                        v
                    }
                    "pop" => {
                        pop(stack);
                        continue;
                    }
                    "exch" => {
                        let (b, a) = (pop(stack), pop(stack));
                        stack.push(b);
                        a
                    }
                    "copy" => {
                        let n = pop(stack).max(0.0) as usize;
                        let from = stack.len().saturating_sub(n);
                        let top: Vec<f64> = stack[from..].to_vec();
                        stack.extend(top);
                        continue;
                    }
                    "index" => {
                        let n = pop(stack).max(0.0) as usize;
                        stack.get(stack.len().wrapping_sub(1 + n)).copied().unwrap_or(0.0)
                    }
                    "roll" => {
                        let (j, n) = (pop(stack) as i64, pop(stack).max(0.0) as usize);
                        if n > 0 && n <= stack.len() {
                            let from = stack.len() - n;
                            let part = &mut stack[from..];
                            let j = j.rem_euclid(n as i64) as usize;
                            part.rotate_right(j);
                        }
                        continue;
                    }
                    two => {
                        let (b, a) = (pop(stack), pop(stack));
                        match two {
                            "add" => a + b,
                            "sub" => a - b,
                            "mul" => a * b,
                            "div" => {
                                if b == 0.0 {
                                    0.0
                                } else {
                                    a / b
                                }
                            }
                            "idiv" => {
                                if b as i64 == 0 {
                                    0.0
                                } else {
                                    (a as i64 / b as i64) as f64
                                }
                            }
                            "mod" => {
                                if b as i64 == 0 {
                                    0.0
                                } else {
                                    (a as i64 % b as i64) as f64
                                }
                            }
                            "exp" => a.powf(b),
                            "atan" => {
                                let d = a.atan2(b).to_degrees();
                                if d < 0.0 { d + 360.0 } else { d }
                            }
                            "eq" => b2(a == b),
                            "ne" => b2(a != b),
                            "gt" => b2(a > b),
                            "ge" => b2(a >= b),
                            "lt" => b2(a < b),
                            "le" => b2(a <= b),
                            "and" => ((a as i64) & (b as i64)) as f64,
                            "or" => ((a as i64) | (b as i64)) as f64,
                            "xor" => ((a as i64) ^ (b as i64)) as f64,
                            "bitshift" => {
                                if b >= 0.0 {
                                    ((a as i64) << (b as i64).min(63)) as f64
                                } else {
                                    ((a as i64) >> (-b as i64).min(63)) as f64
                                }
                            }
                            _ => {
                                stack.push(a);
                                b
                            }
                        }
                    }
                };
                stack.push(v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_calculator_program_runs() {
        let src = b"{ 360 mul sin 2 div exch 360 mul sin 2 div add 0.5 gt { 1 } { 0 } ifelse }";
        let mut l = Lexer::new(src, 1);
        let p = program(&mut l);
        let f = Function::Calculator { domain: vec![0.0, 1.0, 0.0, 1.0], range: vec![0.0, 1.0], program: p };
        assert_eq!(f.eval(&[0.25, 0.25]), vec![1.0]);
        assert_eq!(f.eval(&[0.0, 0.0]), vec![0.0]);
        let e = Function::Exponential { domain: vec![0.0, 1.0], c0: vec![0.0, 1.0], c1: vec![1.0, 0.0], n: 2.0, range: vec![] };
        assert_eq!(e.eval(&[0.5]), vec![0.25, 0.75]);
        let s = Function::Stitched {
            domain: vec![0.0, 1.0],
            parts: vec![Function::Exponential { domain: vec![0.0, 1.0], c0: vec![0.0], c1: vec![1.0], n: 1.0, range: vec![] }; 2],
            bounds: vec![0.5],
            encode: vec![0.0, 1.0, 1.0, 0.0],
            range: vec![],
        };
        assert_eq!(s.eval(&[0.25]), vec![0.5]);
        assert_eq!(s.eval(&[0.75]), vec![0.5]);
        assert!(s.straight());
        let sampled = Function::Sampled { domain: vec![0.0, 1.0], range: vec![0.0, 1.0], size: vec![3], encode: vec![0.0, 2.0], decode: vec![0.0, 1.0], samples: vec![0.0, 1.0, 0.0] };
        assert!((sampled.eval(&[0.25])[0] - 0.5).abs() < 1e-9 && (sampled.eval(&[0.5])[0] - 1.0).abs() < 1e-9);
    }
}
