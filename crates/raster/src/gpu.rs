//! Pictures worked on by the GPU, where there is one and the picture is big enough to be worth
//! handing over: the filters here, and the steps effects hand over ([`apply`]), the same as the
//! processor makes them (to a hundred-thousandth). Their programs start with what they share
//! ([`COMMON`]: pixels read, the picture between its pixels, the blend modes). A thread can keep
//! its drawing on the processor ([`on_processor`]); the GPU, once it fails, is not asked again.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};

use veymelo_os::gpu::{Arg, Buffer, Step, bytes_of};
pub use veymelo_os::gpu::{Kernel, Plain};

use crate::filter::Edge;
use crate::{Image, Pixels, Px};

/// What the GPU's picture programs share (put first in each).
pub const COMMON: &str = include_str!("common.metal");
const FILTERS: &str = include_str!("filter.metal");

/// A program made of `parts` one after another (made once for each list of parts).
pub fn program(parts: &[&'static str]) -> &'static str {
    type Made = std::sync::Mutex<std::collections::HashMap<Vec<usize>, &'static str>>;
    static MADE: std::sync::OnceLock<Made> = std::sync::OnceLock::new();
    let key: Vec<usize> = parts.iter().map(|p| p.as_ptr() as usize).collect();
    let mut made = MADE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    made.entry(key).or_insert_with(|| Box::leak(parts.concat().into_boxed_str()))
}

/// A kernel of the filters' program.
fn filter(name: &'static str) -> Kernel {
    Kernel { source: program(&[COMMON, FILTERS]), name }
}

/// The threads that make a stretch of a row together, the most a pass reaches either side of a
/// pixel for its stretch to be fetched whole, and how many pixels side by side a box's thread
/// makes (as filter.metal has them).
const STRETCH: usize = 256;
const REACH: i32 = 128;
const RUN: usize = 8;
const BOX_THREADS: usize = 64;
const BELL_RUN: usize = 4;

thread_local! {
    /// This thread draws on the processor alone.
    static PROCESSOR: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f` with what it draws on this thread drawn by the processor alone.
pub fn on_processor<R>(f: impl FnOnce() -> R) -> R {
    with_mode(true, f)
}

/// Whether this thread draws on the processor alone (to carry to threads it starts).
pub fn processor_only() -> bool {
    PROCESSOR.with(Cell::get)
}

/// Runs `f` drawing on the processor alone or not, as `processor` says.
pub fn with_mode<R>(processor: bool, f: impl FnOnce() -> R) -> R {
    let before = PROCESSOR.with(|p| p.replace(processor));
    let out = f();
    PROCESSOR.with(|p| p.set(before));
    out
}

static GIVEN_UP: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// How many pictures the GPU has worked on for this thread.
    static RAN: Cell<usize> = const { Cell::new(0) };
}

/// How many pictures the GPU has worked on for this thread (for tests to see it did).
pub fn ran() -> usize {
    RAN.with(Cell::get)
}

/// The fewest pixels a picture has for the GPU to filter it (`VEYMELO_GPU_FROM` sets another:
/// 0 hands every picture over, to test with).
pub(crate) fn fewest() -> usize {
    static FROM: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *FROM.get_or_init(|| std::env::var("VEYMELO_GPU_FROM").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(256 * 256))
}

/// Whether there is a GPU to work on pictures (and it has not been given up on).
pub fn available() -> bool {
    !GIVEN_UP.load(Ordering::Relaxed) && veymelo_os::gpu::available()
}

/// Whether a picture of `px` pixels is worked on by the GPU here.
pub fn wanted(px: usize) -> bool {
    px > 0 && px >= fewest() && !processor_only() && !GIVEN_UP.load(Ordering::Relaxed) && veymelo_os::gpu::available()
}

/// Says once that the GPU could not filter, and why; the processor does from then on.
fn failed(why: &str) {
    static SAID: std::sync::Once = std::sync::Once::new();
    GIVEN_UP.store(true, Ordering::Relaxed);
    SAID.call_once(|| eprintln!("the GPU could not filter a picture, so the processor does: {why}"));
}

/// A pass along the rows as the kernels take it.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Pass {
    w: u32,
    h: u32,
    clear: u32,
    taps: u32,
    width: f32,
    first: i32,
    last: i32,
    first_part: f32,
    last_part: f32,
}

unsafe impl Plain for Pass {}

impl Pass {
    /// A pass along rows `w` long, `h` of them.
    fn new(w: usize, h: usize, edge: Edge) -> Pass {
        Pass { w: w as u32, h: h as u32, clear: (edge == Edge::Clear) as u32, ..Pass::default() }
    }

    /// A box `width` wide round each pixel's middle: its ends from the pixel (they are the same
    /// for every pixel), and what they take of the pixels they cut, as filter.rs works them out.
    fn boxed(self, width: f64) -> Pass {
        let (a, b) = (0.5 - width / 2.0, 0.5 + width / 2.0);
        let (fa, fb) = (a.floor(), b.floor());
        Pass { width: width as f32, first: fa as i32, last: fb as i32, first_part: (fa + 1.0 - a) as f32, last_part: (b - fb) as f32, ..self }
    }
}

/// A step done to a picture on the GPU: a kernel run over it, its grid, and its groups. The
/// kernel takes the picture out, the picture in (what the step before made, or what a step put
/// in slot `input`), then its numbers (if it has some), the picture as it first was (if it asks:
/// `original`), the picture in slot `also` (if asked), and its settings. With `save`, what it
/// makes goes in that slot (the steps after it read it from there).
pub struct Op {
    pub kernel: Kernel,
    pub data: Option<Vec<u8>>,
    pub original: bool,
    pub settings: Vec<u8>,
    pub grid: [usize; 2],
    pub group: Option<[usize; 2]>,
    pub input: Option<usize>,
    pub also: Option<usize>,
    pub save: Option<usize>,
}

/// How many slots a run of steps has to keep pictures in.
pub const SLOTS: usize = 4;

impl Op {
    /// `kernel` with `settings`, over `grid`.
    pub fn new<T: Plain>(kernel: Kernel, settings: &T, grid: [usize; 2]) -> Op {
        Op { kernel, data: None, original: false, settings: bytes_of(settings).to_vec(), grid, group: None, input: None, also: None, save: None }
    }

    /// The step with `data` (numbers it reads) too.
    pub fn with<T: Plain>(self, data: &[T]) -> Op {
        Op { data: Some(veymelo_os::gpu::bytes_of_slice(data).to_vec()), ..self }
    }
}

/// The passes along the rows of a picture `w` × `h` that blur them by a Gaussian of deviation
/// `sigma`: an exact bell (its weights with noughts either side, for the pixels side by side a
/// thread makes), or three boxes.
fn bell_ops(w: usize, h: usize, sigma: f64, edge: Edge, bell: &dyn Fn(f64) -> Option<Vec<f32>>) -> Vec<Op> {
    let pass = Pass::new(w, h, edge);
    match bell(sigma) {
        Some(weights) => {
            let pad = [0f32; BELL_RUN - 1];
            let pass = Pass { taps: weights.len() as u32, ..pass };
            vec![Op { group: Some([STRETCH / BELL_RUN, 1]), ..Op::new(filter("bell_rows"), &pass, [w.div_ceil(BELL_RUN), h]).with(&[&pad[..], &weights, &pad[..]].concat()) }]
        }
        None => (0..3).map(|_| box_op(pass.boxed(2.0 * sigma))).collect(),
    }
}

/// A pass along the rows by a box: its stretch fetched whole, or (wider than that reaches)
/// each pixel's own.
fn box_op(pass: Pass) -> Op {
    let (w, h) = (pass.w as usize, pass.h as usize);
    if -pass.first <= REACH && pass.last <= REACH {
        Op { group: Some([BOX_THREADS, 1]), ..Op::new(filter("box_rows"), &pass, [w.div_ceil(RUN), h]) }
    } else {
        Op::new(filter("wide_box_rows"), &pass, [w, h])
    }
}

/// The picture `w` × `h` turned on its diagonal.
fn turn(w: usize, h: usize) -> Op {
    Op { group: Some([32, 8]), ..Op::new(filter("turn"), &[w as u32, h as u32], [w.div_ceil(32) * 32, h.div_ceil(32) * 8]) }
}

/// `across` (passes along the rows of a picture `w` × `h`) and then `down` (along its columns:
/// the picture turned, its rows passed along, and turned back).
fn both_ways(across: Vec<Op>, down: Vec<Op>, w: usize, h: usize) -> Vec<Op> {
    let mut ops = across;
    if !down.is_empty() {
        ops.push(turn(w, h));
        ops.extend(down);
        ops.push(turn(h, w));
    }
    ops
}

/// Buffers kept between pictures on this thread: the picture, two to pass it between, and the
/// slots.
struct Kept([Option<Buffer>; 3 + SLOTS]);

thread_local! {
    static KEPT: RefCell<Kept> = const { RefCell::new(Kept([const { None }; 3 + SLOTS])) };
}

/// The most bytes a thread keeps between pictures.
const MOST_KEPT: usize = 256 << 20;

/// A buffer of at least `len` bytes (what was in it is left), the one kept in `slot` if it fits.
fn take(slot: usize, len: usize) -> Result<Buffer, String> {
    let kept = KEPT.with(|k| k.borrow_mut().0[slot].take());
    match kept {
        Some(b) if b.len() >= len && b.len() <= len.max(1 << 20) * 4 => Ok(b),
        _ => Buffer::new(len + len / 4),
    }
}

/// Keeps `b` in `slot` for the next picture (unless this thread keeps too much already).
fn keep(slot: usize, b: Buffer) {
    KEPT.with(|k| {
        let mut k = k.borrow_mut();
        let others: usize = k.0.iter().enumerate().filter(|(i, _)| *i != slot).filter_map(|(_, b)| b.as_ref()).map(Buffer::len).sum();
        if others + b.len() <= MOST_KEPT {
            k.0[slot] = Some(b);
        }
    });
}

/// `ops` done to `img` on the GPU, one after another: the picture after them, or None when the
/// GPU could not (it is then given up on: the processor does the rest).
pub fn apply(img: &Image, ops: &[Op]) -> Option<Image> {
    if ops.is_empty() {
        return Some(img.clone());
    }
    let go = || -> Result<Image, String> {
        let n = img.px.len();
        let len = std::mem::size_of_val(&img.px[..]);
        // the picture where it is, if the GPU shares it; else copied over
        let copied = match img.px.buffer() {
            Some(_) => None,
            None => {
                let mut b = take(0, len)?;
                b.slice_mut::<Px>()[..n].copy_from_slice(&img.px);
                Some(b)
            }
        };
        let source: &Buffer = match &copied {
            Some(b) => b,
            None => img.px.buffer().expect("shared"),
        };
        let (a, b) = (take(1, len)?, take(2, len)?);
        // what the last step makes goes straight into the new picture's pixels (unless it puts
        // it in a slot)
        let made = if ops.last().is_some_and(|op| op.save.is_none()) { Pixels::shared_unfilled(n) } else { None };
        let used = |k: usize| ops.iter().any(|op| op.input == Some(k) || op.also == Some(k) || op.save == Some(k));
        let slots: Vec<Option<Buffer>> = (0..SLOTS).map(|k| used(k).then(|| take(3 + k, len)).transpose()).collect::<Result<_, _>>()?;
        let slot = |k: usize| -> Result<&Buffer, String> { slots.get(k).and_then(Option::as_ref).ok_or_else(|| format!("no slot {k}")) };
        let data: Vec<Option<Buffer>> = ops.iter().map(|op| op.data.as_deref().map(Buffer::of).transpose()).collect::<Result<_, _>>()?;
        let out_buffer = made.as_ref().and_then(|p| p.buffer());
        // where the last step's picture is: 0 the picture, 1 and 2 the two, 3 on the slots, and
        // the new picture's own (past them)
        const MADE: usize = 3 + SLOTS;
        let mut here = 0usize;
        let buffer = |w: usize| -> Result<&Buffer, String> {
            match w {
                0 => Ok(source),
                1 => Ok(&a),
                2 => Ok(&b),
                MADE => out_buffer.ok_or_else(|| "no picture to make".to_string()),
                k => slot(k - 3),
            }
        };
        let mut steps = Vec::with_capacity(ops.len());
        for (k, op) in ops.iter().enumerate() {
            let from = op.input.map_or(here, |s| 3 + s);
            // into a slot, the new picture (the last step), or back and forth between the two
            let to = match op.save {
                Some(s) => 3 + s,
                None if k + 1 == ops.len() && out_buffer.is_some() => MADE,
                None if from == 1 => 2,
                None => 1,
            };
            let mut args = vec![Arg::Pass(buffer(to)?), Arg::Pass(buffer(from)?)];
            if let Some(d) = &data[k] {
                args.push(Arg::Read(d));
            }
            if op.original {
                args.push(Arg::Pass(source));
            }
            if let Some(s) = op.also {
                args.push(Arg::Pass(slot(s)?));
            }
            args.push(Arg::Bytes(&op.settings));
            steps.push(Step { kernel: &op.kernel, args, grid: op.grid, group: op.group });
            here = to;
        }
        veymelo_os::gpu::run_steps(&steps)?;
        drop(steps);
        let elsewhere = if here == MADE { None } else { Some(buffer(here)?.slice::<Px>()[..n].to_vec()) };
        let px = match (elsewhere, made) {
            (None, Some(p)) => p,
            (Some(v), _) => Pixels::from(v),
            (None, None) => return Err("no picture made".into()),
        };
        if let Some(c) = copied {
            keep(0, c);
        }
        keep(1, a);
        keep(2, b);
        for (k, s) in slots.into_iter().enumerate() {
            if let Some(s) = s {
                keep(3 + k, s);
            }
        }
        Ok(Image { width: img.width, height: img.height, px })
    };
    match go() {
        Ok(out) => {
            RAN.with(|r| r.set(r.get() + 1));
            Some(out)
        }
        Err(why) => {
            failed(&why);
            None
        }
    }
}

/// [`crate::filter::gaussian`] on the GPU (None: leave it to the processor). `bell` gives the
/// weights of a deviation small enough to be drawn exactly; past that, three boxes.
pub(crate) fn gaussian(img: &Image, sx: f64, sy: f64, edge: Edge, bell: &dyn Fn(f64) -> Option<Vec<f32>>) -> Option<Image> {
    if !wanted(img.px.len()) {
        return None;
    }
    let (w, h) = (img.width, img.height);
    let across = if sx > 0.01 { bell_ops(w, h, sx, edge, bell) } else { Vec::new() };
    let down = if sy > 0.01 { bell_ops(h, w, sy, edge, bell) } else { Vec::new() };
    apply(img, &both_ways(across, down, w, h))
}

/// The steps that blur a picture `w` × `h` as [`crate::filter::gaussian`] does, for a run of
/// steps of an effect's own (the blur between them).
pub fn gaussian_ops(w: usize, h: usize, sx: f64, sy: f64, edge: Edge) -> Vec<Op> {
    let bell = |sigma: f64| (sigma <= crate::filter::EXACT_MOST).then(|| crate::filter::bell(sigma));
    let across = if sx > 0.01 { bell_ops(w, h, sx, edge, &bell) } else { Vec::new() };
    let down = if sy > 0.01 { bell_ops(h, w, sy, edge, &bell) } else { Vec::new() };
    both_ways(across, down, w, h)
}

/// A step that puts the picture (`w` × `h`) as it is in a slot (with `save`).
pub fn copy_op(w: usize, h: usize) -> Op {
    Op::new(filter("copy_picture"), &(w as u32), [w, h])
}

/// [`gaussian_ops`] reading the picture in slot `from` and putting the blur in slot `into` (or
/// leaving it as the run's picture), at least one step (a copy, where nothing blurs).
pub fn gaussian_between(w: usize, h: usize, s: f64, edge: Edge, from: usize, into: Option<usize>) -> Vec<Op> {
    let mut ops = gaussian_ops(w, h, s, s, edge);
    if ops.is_empty() {
        ops.push(copy_op(w, h));
    }
    ops[0].input = Some(from);
    if let Some(last) = ops.last_mut() {
        last.save = into;
    }
    ops
}

/// [`crate::filter::boxes`] on the GPU (None: leave it to the processor).
pub(crate) fn boxes(img: &Image, wx: f64, wy: f64, times: usize, edge: Edge) -> Option<Image> {
    if !wanted(img.px.len()) {
        return None;
    }
    let (w, h) = (img.width, img.height);
    // (a box no wider than a pixel leaves the line as it is)
    let ways = |width: f64, w: usize, h: usize| -> Vec<Op> { if width > 1.0 + 1e-9 { (0..times).map(|_| box_op(Pass::new(w, h, edge).boxed(width))).collect() } else { Vec::new() } };
    apply(img, &both_ways(ways(wx, w, h), ways(wy, h, w), w, h))
}
/// The size of a picture a kernel goes over, and how many samples each pixel takes.
#[repr(C)]
#[derive(Clone, Copy)]
struct Along {
    w: u32,
    h: u32,
    n: u32,
}

unsafe impl Plain for Along {}

/// [`crate::filter::along`] on the GPU (None: leave it to the processor). Where each sample
/// lies from its pixel is the same for every pixel: worked out here, as whole pixels and the
/// fraction past them, as the processor works them out.
pub(crate) fn along(img: &Image, dir: (f64, f64), length: f64, n: usize) -> Option<Image> {
    if !wanted(img.px.len()) {
        return None;
    }
    let mut steps = Vec::with_capacity(n * 4);
    for i in 0..n {
        let t = (i as f64 / (n - 1) as f64 - 0.5) * length;
        // (the pixel's middle, moved, less the half pixel the sampling takes off)
        let (ox, oy) = (dir.0 * t, dir.1 * t);
        let (fx, fy) = (ox.floor(), oy.floor());
        steps.extend([fx as f32, fy as f32, (ox - fx) as f32, (oy - fy) as f32]);
    }
    apply(img, &[Op::new(filter("along_blur"), &Along { w: img.width as u32, h: img.height as u32, n: n as u32 }, [img.width, img.height]).with(&steps)])
}

/// What a blur round a point takes: the picture's size, which (round or toward), the point,
/// how many pixels the farthest sample moves a pixel from the point, and how far round or in.
#[repr(C)]
#[derive(Clone, Copy)]
struct Round {
    w: u32,
    h: u32,
    zoom: u32,
    pad: u32,
    cx: [f32; 2],
    cy: [f32; 2],
    reach: [f32; 2],
    amount: f32,
}

unsafe impl Plain for Round {}

/// A double as two floats that add up to it, as the GPU holds numbers it must keep exactly
/// (common.metal's df).
pub fn split(v: f64) -> [f32; 2] {
    let hi = v as f32;
    [hi, (v - hi as f64) as f32]
}

/// [`crate::filter::round`] on the GPU (None: leave it to the processor).
pub(crate) fn round(img: &Image, centre: (f64, f64), amount: f64, zoom: bool) -> Option<Image> {
    if !wanted(img.px.len()) {
        return None;
    }
    let reach = if zoom { amount.abs() } else { amount.abs().to_radians() };
    let r = Round { w: img.width as u32, h: img.height as u32, zoom: zoom as u32, pad: 0, cx: split(centre.0), cy: split(centre.1), reach: split(reach), amount: amount as f32 };
    apply(img, &[Op::new(filter("round_blur"), &r, [img.width, img.height])])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter;

    /// A picture with edges, colours, clear parts and fine detail.
    fn picture(w: usize, h: usize) -> Image {
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
                let a = if (x / 37 + y / 23) % 3 == 0 { 0.0 } else if x % 7 == 0 { 0.5 } else { 1.0 };
                img.px[y * w + x] = [fx * a, fy * a, ((x * y) % 5) as f32 / 4.0 * a, a];
            }
        }
        img
    }

    fn most_apart(a: &Image, b: &Image) -> f32 {
        a.px.iter().zip(&b.px).flat_map(|(p, q)| (0..4).map(move |c| (p[c] - q[c]).abs())).fold(0.0, f32::max)
    }

    /// Every blend mode mixes as the processor's, pixels of every kind (clear, part see-through,
    /// brighter than white) onto every kind.
    #[test]
    fn the_gpu_blends_as_the_processor_does() {
        if !veymelo_os::gpu::available() {
            return;
        }
        const TEST: &str = "kernel void blend_test(device float4 *out [[buffer(0)]], const device float4 *under [[buffer(1)]], const device float4 *over [[buffer(2)]], constant uint &mode [[buffer(3)]], uint2 g [[thread_position_in_grid]], uint2 size [[threads_per_grid]]) {\n\
            uint i = g.y * size.x + g.x;\n\
            out[i] = blend(mode, under[i], over[i], 0.5);\n\
        }";
        let (w, h) = (64, 64);
        let mut under = Image::new(w, h);
        let mut over = Vec::with_capacity(w * h);
        for (i, p) in under.px.iter_mut().enumerate() {
            let f = |k: usize| ((i * 7919 + k * 104_729) % 1013) as f32 / 1012.0;
            let a = [0.0, 0.3, 1.0, f(1)][i % 4];
            *p = [f(2) * a * 1.4, f(3) * a, f(4) * a, a];
            let b = [1.0, 0.0, 0.55, f(5)][(i / 4) % 4];
            over.push([f(6) * b, f(7) * b * 1.6, f(8) * b, b]);
        }
        for m in veymelo_color::BlendMode::ALL {
            let kernel = Kernel { source: program(&[COMMON, TEST]), name: "blend_test" };
            let gpu = with_mode(false, || apply(&under, &[Op::new(kernel, &(m as u32), [w, h]).with(&over)])).expect("drawn by the GPU");
            for (k, (p, q)) in under.px.iter().zip(&over).enumerate() {
                let cpu = veymelo_color::blend(m, *p, *q, 0.5);
                for c in 0..4 {
                    let d = (gpu.px[k][c] - cpu[c]).abs();
                    assert!(d <= 2e-6 * cpu[c].abs().max(1.0), "{m:?} under {p:?} over {q:?}: GPU {:?}, processor {cpu:?}", gpu.px[k]);
                }
            }
        }
    }

    #[test]
    fn the_gpu_blurs_as_the_processor_does() {
        if !veymelo_os::gpu::available() {
            eprintln!("no GPU here: nothing to test");
            return;
        }
        let img = picture(301, 257);
        for edge in [Edge::Clear, Edge::Repeat] {
            // exact bells, three boxes, uneven, one way only, and boxes wider than a stretch
            // reaches
            for (sx, sy) in [(0.6, 0.6), (3.3, 7.9), (12.0, 12.0), (12.5, 30.25), (60.0, 0.0), (0.0, 4.0), (150.0, 140.0)] {
                let gpu = gaussian(&img, sx, sy, edge, &|s| (s <= 12.0).then(|| filter::bell_weights(s))).expect("drawn by the GPU");
                let cpu = on_processor(|| filter::gaussian(&img, sx, sy, edge));
                let d = most_apart(&gpu, &cpu);
                assert!(d <= 1e-5, "gaussian {sx} {sy} {edge:?}: {d}");
            }
            for (wx, wy, n) in [(2.5, 9.75, 1), (31.0, 1.0, 3), (4.0, 120.5, 2), (300.0, 2.0, 1)] {
                let gpu = boxes(&img, wx, wy, n, edge).expect("drawn by the GPU");
                let cpu = on_processor(|| filter::boxes(&img, wx, wy, n, edge));
                let d = most_apart(&gpu, &cpu);
                assert!(d <= 1e-5, "boxes {wx} {wy} {n} {edge:?}: {d}");
            }
        }
        // along a line, round a point and toward it: the samples lie where the processor's do
        for (dir, length) in [((1.0, 0.0), 6.0f64), ((0.6, -0.8), 37.5), ((-0.28, 0.96), 140.0)] {
            let n = (length.ceil() as usize * 2 + 1).clamp(3, 1025);
            let gpu = along(&img, dir, length, n).expect("drawn by the GPU");
            let d = most_apart(&gpu, &on_processor(|| filter::along(&img, dir, length)));
            assert!(d <= 1e-5, "along {dir:?} {length}: {d}");
        }
        for (centre, amount, zoom) in [((150.5, 128.25), 12.0, false), ((20.0, 240.0), 40.0, false), ((150.0, 100.0), 0.2, true), ((-30.5, 300.25), 0.35, true)] {
            let gpu = round(&img, centre, amount, zoom).expect("drawn by the GPU");
            let d = most_apart(&gpu, &on_processor(|| filter::round(&img, centre, amount, zoom)));
            assert!(d <= 1e-5, "round {centre:?} {amount} {zoom}: {d}");
        }
        // a picture too small is left to the processor, and so is everything on its threads
        assert!(gaussian(&picture(20, 20), 2.0, 2.0, Edge::Clear, &|_| None).is_none());
        assert!(on_processor(|| gaussian(&img, 2.0, 2.0, Edge::Clear, &|_| None)).is_none());
    }
}

#[cfg(test)]
mod timing {
    /// How long a big blur takes on each (run by hand: cargo test --release blur_takes --
    /// --ignored --nocapture).
    #[test]
    #[ignore]
    fn blur_takes() {
        let img = super::tests_picture(2000, 2000);
        for sigma in [4.0, 11.0, 18.0, 40.0] {
            let t = std::time::Instant::now();
            for _ in 0..5 {
                std::hint::black_box(crate::filter::gaussian(&img, sigma, sigma, crate::filter::Edge::Clear));
            }
            let gpu = t.elapsed().as_secs_f64() * 200.0;
            let t = std::time::Instant::now();
            super::on_processor(|| std::hint::black_box(crate::filter::gaussian(&img, sigma, sigma, crate::filter::Edge::Clear)));
            eprintln!("σ {sigma}: GPU {gpu:.1} ms, processor {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
        }
    }
}

#[cfg(test)]
fn tests_picture(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for (i, p) in img.px.iter_mut().enumerate() {
        *p = [(i % 7) as f32 / 7.0, 0.5, 0.25, 1.0];
    }
    img
}
