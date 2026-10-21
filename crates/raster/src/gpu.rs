//! Filters drawn by the GPU, where there is one and the picture is big enough to be worth
//! handing over: the same as the processor makes them (to a hundred-thousandth). A thread can
//! keep its drawing on the processor ([`on_processor`]); the GPU, once it fails, is not asked
//! again.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};

use veymelo_os::gpu::{Arg, Buffer, Kernel, Plain, Step, bytes_of};

use crate::filter::Edge;
use crate::{Image, Px};

const SOURCE: &str = include_str!("filter.metal");
const BELL: Kernel = Kernel { source: SOURCE, name: "bell_rows" };
const BOX: Kernel = Kernel { source: SOURCE, name: "box_rows" };
const WIDE_BOX: Kernel = Kernel { source: SOURCE, name: "wide_box_rows" };
const TURN: Kernel = Kernel { source: SOURCE, name: "turn" };
const ALONG: Kernel = Kernel { source: SOURCE, name: "along_blur" };
const ROUND: Kernel = Kernel { source: SOURCE, name: "round_blur" };

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

/// The fewest pixels a picture has for the GPU to filter it (`VEYMELO_GPU_FROM` sets another:
/// 0 hands every picture over, to test with).
fn fewest() -> usize {
    static FROM: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *FROM.get_or_init(|| std::env::var("VEYMELO_GPU_FROM").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(256 * 256))
}

/// Whether a picture of `px` pixels is filtered by the GPU here.
fn wanted(px: usize) -> bool {
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

/// A step done to a picture on the GPU: a kernel run over it (taking the picture out, the
/// picture in, perhaps a list of numbers, and its settings), its grid, and its groups.
struct Op {
    kernel: &'static Kernel,
    numbers: Option<Vec<f32>>,
    settings: Vec<u8>,
    grid: [usize; 2],
    group: Option<[usize; 2]>,
}

impl Op {
    fn new<T: Plain>(kernel: &'static Kernel, settings: &T, grid: [usize; 2]) -> Op {
        Op { kernel, numbers: None, settings: bytes_of(settings).to_vec(), grid, group: None }
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
            vec![Op { numbers: Some([&pad[..], &weights, &pad[..]].concat()), group: Some([STRETCH / BELL_RUN, 1]), ..Op::new(&BELL, &pass, [w.div_ceil(BELL_RUN), h]) }]
        }
        None => (0..3).map(|_| box_op(pass.boxed(2.0 * sigma))).collect(),
    }
}

/// A pass along the rows by a box: its stretch fetched whole, or (wider than that reaches)
/// each pixel's own.
fn box_op(pass: Pass) -> Op {
    let (w, h) = (pass.w as usize, pass.h as usize);
    if -pass.first <= REACH && pass.last <= REACH {
        Op { group: Some([BOX_THREADS, 1]), ..Op::new(&BOX, &pass, [w.div_ceil(RUN), h]) }
    } else {
        Op::new(&WIDE_BOX, &pass, [w, h])
    }
}

/// The picture `w` × `h` turned on its diagonal.
fn turn(w: usize, h: usize) -> Op {
    Op { group: Some([32, 8]), ..Op::new(&TURN, &[w as u32, h as u32], [w.div_ceil(32) * 32, h.div_ceil(32) * 8]) }
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

/// Buffers kept between pictures on this thread: the picture, and two to pass it between.
struct Kept([Option<Buffer>; 3]);

thread_local! {
    static KEPT: RefCell<Kept> = const { RefCell::new(Kept([None, None, None])) };
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

/// `ops` done to `img` on the GPU: the picture after them, or None when the GPU could not.
fn run(img: &Image, ops: &[Op]) -> Option<Image> {
    if ops.is_empty() {
        return Some(img.clone());
    }
    let go = || -> Result<Image, String> {
        let len = std::mem::size_of_val(img.px.as_slice());
        let mut source = take(0, len)?;
        source.slice_mut::<Px>()[..img.px.len()].copy_from_slice(&img.px);
        let (a, b) = (take(1, len)?, take(2, len)?);
        let numbers: Vec<Option<Buffer>> = ops.iter().map(|op| op.numbers.as_deref().map(Buffer::of).transpose()).collect::<Result<_, _>>()?;
        let mut steps = Vec::with_capacity(ops.len());
        for (k, op) in ops.iter().enumerate() {
            // from the picture, then back and forth between the two
            let from = if k == 0 { &source } else if k % 2 == 1 { &a } else { &b };
            let to = if k % 2 == 0 { &a } else { &b };
            let mut args = vec![Arg::Pass(to), Arg::Pass(from)];
            if let Some(n) = &numbers[k] {
                args.push(Arg::Read(n));
            }
            args.push(Arg::Bytes(&op.settings));
            steps.push(Step { kernel: op.kernel, args, grid: op.grid, group: op.group });
        }
        veymelo_os::gpu::run_steps(&steps)?;
        drop(steps);
        let last = if ops.len() % 2 == 1 { &a } else { &b };
        let out = Image { width: img.width, height: img.height, px: last.slice::<Px>()[..img.px.len()].to_vec() };
        keep(0, source);
        keep(1, a);
        keep(2, b);
        Ok(out)
    };
    match go() {
        Ok(out) => Some(out),
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
    run(img, &both_ways(across, down, w, h))
}

/// [`crate::filter::boxes`] on the GPU (None: leave it to the processor).
pub(crate) fn boxes(img: &Image, wx: f64, wy: f64, times: usize, edge: Edge) -> Option<Image> {
    if !wanted(img.px.len()) {
        return None;
    }
    let (w, h) = (img.width, img.height);
    // (a box no wider than a pixel leaves the line as it is)
    let ways = |width: f64, w: usize, h: usize| -> Vec<Op> { if width > 1.0 + 1e-9 { (0..times).map(|_| box_op(Pass::new(w, h, edge).boxed(width))).collect() } else { Vec::new() } };
    run(img, &both_ways(ways(wx, w, h), ways(wy, h, w), w, h))
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
    let op = Op { numbers: Some(steps), ..Op::new(&ALONG, &Along { w: img.width as u32, h: img.height as u32, n: n as u32 }, [img.width, img.height]) };
    run(img, &[op])
}

/// What a blur round a point takes: the picture's size, the point (its whole pixels and the
/// fraction past them), how far round or in, and which.
#[repr(C)]
#[derive(Clone, Copy)]
struct Round {
    w: u32,
    h: u32,
    cx: i32,
    cy: i32,
    fx: f32,
    fy: f32,
    amount: f32,
    zoom: u32,
}

unsafe impl Plain for Round {}

/// [`crate::filter::round`] on the GPU (None: leave it to the processor).
pub(crate) fn round(img: &Image, centre: (f64, f64), amount: f64, zoom: bool) -> Option<Image> {
    if !wanted(img.px.len()) {
        return None;
    }
    let (cx, cy) = (centre.0.floor(), centre.1.floor());
    let r = Round { w: img.width as u32, h: img.height as u32, cx: cx as i32, cy: cy as i32, fx: (centre.0 - cx) as f32, fy: (centre.1 - cy) as f32, amount: amount as f32, zoom: zoom as u32 };
    run(img, &[Op::new(&ROUND, &r, [img.width, img.height])])
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
