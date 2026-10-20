//! The GPU, through Metal (a part of macOS): programs in the Metal Shading Language, built from
//! their source the first time each is run, run over a grid of threads on buffers the processor
//! and the GPU share. Apple silicon has one memory for both, so a buffer filled here is read
//! there without being copied.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use super::{Id, NIL, Pool, Sel, error_text, msg, nsstring, rust_string, sel, send, send1};
use crate::gpu::{Arg, Kernel, Plain};

#[link(name = "Metal", kind = "framework")]
unsafe extern "C" {
    fn MTLCreateSystemDefaultDevice() -> Id;
}

// a program run outside a window finds the GPU only with CoreGraphics in it
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {}

/// MTLSize.
#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
    width: usize,
    height: usize,
    depth: usize,
}

/// MTLGPUFamilyApple7: the GPU of the M1 and every Apple silicon chip after it.
const APPLE7: i64 = 1007;
/// MTLCommandBufferStatusCompleted.
const COMPLETED: i64 = 4;
/// Bytes handed straight to a kernel (more go in a buffer).
const INLINE: usize = 4096;

struct Gpu {
    device: Id,
    queue: Id,
    name: String,
    /// Each kernel built, by its source and name: its pipeline, and its threads in a group.
    built: Mutex<HashMap<(usize, &'static str), Result<(usize, usize, usize), String>>>,
}

// Metal's device, queue and pipelines may be used from any thread
unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
    GPU.get_or_init(|| {
        let _pool = Pool::new();
        let device = unsafe { MTLCreateSystemDefaultDevice() };
        if device.is_null() {
            return None;
        }
        // only the GPUs of Apple silicon (each has what the kernels here use)
        let family = unsafe { msg!(unsafe extern "C" fn(Id, Sel, i64) -> bool)(device, sel("supportsFamily:"), APPLE7) };
        if !family {
            return None;
        }
        let queue = unsafe { send(device, "newCommandQueue") };
        if queue.is_null() {
            return None;
        }
        let name = rust_string(unsafe { send(device, "name") });
        Some(Gpu { device, queue, name, built: Mutex::new(HashMap::new()) })
    })
    .as_ref()
}

/// The GPU's name, when there is one to draw with.
pub fn name() -> Option<String> {
    gpu().map(|g| g.name.clone())
}

impl Gpu {
    /// The kernel's pipeline (built the first time), and how many of its threads go in a group
    /// across and down.
    fn pipeline(&self, k: &Kernel) -> Result<(Id, usize, usize), String> {
        let key = (k.source.as_ptr() as usize, k.name);
        let mut built = self.built.lock().unwrap_or_else(|e| e.into_inner());
        let made = built.entry(key).or_insert_with(|| unsafe { self.build(k) });
        made.clone().map(|(p, w, h)| (p as Id, w, h))
    }

    unsafe fn build(&self, k: &Kernel) -> Result<(usize, usize, usize), String> {
        unsafe {
            let _pool = Pool::new();
            let options = send(send(super::class("MTLCompileOptions"), "alloc"), "init");
            // exact arithmetic and functions: the pictures as the processor draws them
            if super::answers(options, "setMathMode:") {
                msg!(unsafe extern "C" fn(Id, Sel, i64))(options, sel("setMathMode:"), 0);
                msg!(unsafe extern "C" fn(Id, Sel, i64))(options, sel("setMathFloatingPointFunctions:"), 1);
            } else {
                msg!(unsafe extern "C" fn(Id, Sel, bool))(options, sel("setFastMathEnabled:"), false);
            }
            let mut err: Id = NIL;
            let library = msg!(unsafe extern "C" fn(Id, Sel, Id, Id, *mut Id) -> Id)(self.device, sel("newLibraryWithSource:options:error:"), nsstring(k.source), options, &mut err);
            send(options, "release");
            if library.is_null() {
                return Err(format!("the GPU program does not build: {}", error_text(err)));
            }
            let function = send1(library, "newFunctionWithName:", nsstring(k.name));
            send(library, "release");
            if function.is_null() {
                return Err(format!("the GPU program has no kernel {}", k.name));
            }
            let mut err: Id = NIL;
            let pipeline = msg!(unsafe extern "C" fn(Id, Sel, Id, *mut Id) -> Id)(self.device, sel("newComputePipelineStateWithFunction:error:"), function, &mut err);
            send(function, "release");
            if pipeline.is_null() {
                return Err(format!("the GPU cannot run {}: {}", k.name, error_text(err)));
            }
            let across = (super::integer(pipeline, "threadExecutionWidth") as usize).max(1);
            let most = (super::integer(pipeline, "maxTotalThreadsPerThreadgroup") as usize).max(across);
            Ok((pipeline as usize, across, (most / across).clamp(1, 8)))
        }
    }
}

/// Memory the processor and the GPU share.
pub struct Buffer {
    id: Id,
    len: usize,
}

// a Metal buffer may be used from any thread; its bytes are reached only through `&`/`&mut`
unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}

impl Buffer {
    /// `len` bytes, all zero.
    pub fn new(len: usize) -> Result<Buffer, String> {
        let g = gpu().ok_or("there is no GPU to draw with")?;
        // Metal makes no empty buffers
        let size = len.max(16);
        let id = unsafe { msg!(unsafe extern "C" fn(Id, Sel, usize, usize) -> Id)(g.device, sel("newBufferWithLength:options:"), size, 0) };
        if id.is_null() {
            return Err(format!("the GPU has no room for {} MB", len / 1_000_000));
        }
        let mut b = Buffer { id, len };
        b.bytes_mut().fill(0);
        Ok(b)
    }

    /// A buffer holding `items`.
    pub fn of<T: Plain>(items: &[T]) -> Result<Buffer, String> {
        let mut b = Buffer::new(std::mem::size_of_val(items))?;
        b.slice_mut::<T>().copy_from_slice(items);
        Ok(b)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn contents(&self) -> *mut u8 {
        unsafe { msg!(unsafe extern "C" fn(Id, Sel) -> *mut c_void)(self.id, sel("contents")) as *mut u8 }
    }

    pub fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.contents(), self.len) }
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.contents(), self.len) }
    }

    /// Its bytes as `T`s (as many whole ones as fit; a buffer starts on a page, so any `T` is
    /// aligned).
    pub fn slice<T: Plain>(&self) -> &[T] {
        unsafe { std::slice::from_raw_parts(self.contents() as *const T, self.len / std::mem::size_of::<T>().max(1)) }
    }

    pub fn slice_mut<T: Plain>(&mut self) -> &mut [T] {
        unsafe { std::slice::from_raw_parts_mut(self.contents() as *mut T, self.len / std::mem::size_of::<T>().max(1)) }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe { send(self.id, "release") };
    }
}

/// Builds `kernel` now (else it is built the first time it runs), saying why it cannot be.
pub fn prepare(kernel: &Kernel) -> Result<(), String> {
    gpu().ok_or("there is no GPU to draw with")?.pipeline(kernel).map(|_| ())
}

/// Runs `kernel` once for each point of `grid` (across, down), its buffers and bytes given in
/// the order its parameters take them, and waits for it to end.
pub fn run(kernel: &Kernel, args: &[Arg], grid: [usize; 2]) -> Result<(), String> {
    let g = gpu().ok_or("there is no GPU to draw with")?;
    if grid[0] == 0 || grid[1] == 0 {
        return Ok(());
    }
    let (pipeline, across, down) = g.pipeline(kernel)?;
    let _pool = Pool::new();
    // bytes too many to hand straight over go in a buffer of their own
    let mut held = Vec::new();
    for a in args.iter() {
        if let Arg::Bytes(b) = a
            && b.len() > INLINE
        {
            held.push(Buffer::of(b)?);
        }
    }
    unsafe {
        let commands = send(g.queue, "commandBuffer");
        let encoder = send(commands, "computeCommandEncoder");
        if commands.is_null() || encoder.is_null() {
            return Err("the GPU took no work".into());
        }
        send1(encoder, "setComputePipelineState:", pipeline);
        let mut extra = held.iter();
        for (i, a) in args.iter().enumerate() {
            let buffer = |b: &Buffer| msg!(unsafe extern "C" fn(Id, Sel, Id, usize, usize))(encoder, sel("setBuffer:offset:atIndex:"), b.id, 0, i);
            match a {
                Arg::Read(b) => buffer(b),
                Arg::Write(b) => buffer(b),
                Arg::Bytes(bytes) if bytes.len() > INLINE => buffer(extra.next().expect("held")),
                Arg::Bytes(bytes) => {
                    // a parameter is never empty
                    let pad = [0u8; 16];
                    let b: &[u8] = if bytes.is_empty() { &pad } else { bytes };
                    msg!(unsafe extern "C" fn(Id, Sel, *const c_void, usize, usize))(encoder, sel("setBytes:length:atIndex:"), b.as_ptr().cast(), b.len(), i);
                }
            }
        }
        let whole = Size { width: grid[0], height: grid[1], depth: 1 };
        let group = Size { width: across, height: down, depth: 1 };
        msg!(unsafe extern "C" fn(Id, Sel, Size, Size))(encoder, sel("dispatchThreads:threadsPerThreadgroup:"), whole, group);
        send(encoder, "endEncoding");
        // a few pieces of work on the GPU at a time (many threads drawing frames would otherwise
        // pile theirs up together), and not waited for for ever: a GPU that does not answer
        // (the computer short of memory) is given up on, and the work is done without it
        let _turn = Turn::take();
        send(commands, "commit");
        let start = std::time::Instant::now();
        while super::integer(commands, "status") < COMPLETED {
            if start.elapsed() > PATIENCE {
                // it may still finish later: what it reads and writes is kept for it
                for a in args {
                    match a {
                        Arg::Read(b) => {
                            send(b.id, "retain");
                        }
                        Arg::Write(b) => {
                            send(b.id, "retain");
                        }
                        Arg::Bytes(_) => {}
                    }
                }
                for b in &held {
                    send(b.id, "retain");
                }
                send(commands, "retain");
                return Err(format!("the GPU did not answer in {} seconds", PATIENCE.as_secs()));
            }
            std::thread::sleep(std::time::Duration::from_micros(150));
        }
        if super::integer(commands, "status") != COMPLETED {
            return Err(format!("the GPU stopped part way: {}", error_text(send(commands, "error"))));
        }
    }
    Ok(())
}

/// A turn on the GPU: a few pieces of work at a time, no more (taken until it is dropped).
struct Turn;

static TURNS: Mutex<usize> = Mutex::new(0);
static TURN_FREE: std::sync::Condvar = std::sync::Condvar::new();
/// How many pieces of work the GPU is given at once.
const AT_ONCE: usize = 3;

impl Turn {
    fn take() -> Turn {
        let mut n = TURNS.lock().unwrap_or_else(|e| e.into_inner());
        while *n >= AT_ONCE {
            n = TURN_FREE.wait(n).unwrap_or_else(|e| e.into_inner());
        }
        *n += 1;
        Turn
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        *TURNS.lock().unwrap_or_else(|e| e.into_inner()) -= 1;
        TURN_FREE.notify_one();
    }
}
/// How long a piece of work is waited for before the GPU is given up on.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(15);

#[cfg(test)]
mod tests {
    use super::*;

    const ADD: Kernel = Kernel {
        source: "#include <metal_stdlib>\nusing namespace metal;\n\
                 kernel void add(device float *out [[buffer(0)]], const device float *a [[buffer(1)]], constant float &k [[buffer(2)]], uint2 at [[thread_position_in_grid]], uint2 size [[threads_per_grid]]) {\n\
                     uint i = at.y * size.x + at.x;\n\
                     out[i] = a[i] * k + float(at.x);\n\
                 }",
        name: "add",
    };

    #[test]
    fn a_kernel_runs_over_its_grid() {
        if gpu().is_none() {
            eprintln!("no GPU here: nothing to test");
            return;
        }
        let a = Buffer::of(&(0..12).map(|i| i as f32).collect::<Vec<f32>>()).unwrap();
        let mut out = Buffer::new(12 * 4).unwrap();
        let k = 2.0f32;
        run(&ADD, &[Arg::Write(&mut out), Arg::Read(&a), Arg::Bytes(crate::gpu::bytes_of(&k))], [4, 3]).unwrap();
        let got = out.slice::<f32>();
        for (i, v) in got.iter().enumerate() {
            assert_eq!(*v, i as f32 * 2.0 + (i % 4) as f32);
        }
        assert!(name().is_some_and(|n| !n.is_empty()));
    }

    #[test]
    fn a_program_that_does_not_build_says_why() {
        if gpu().is_none() {
            return;
        }
        let bad = Kernel { source: "kernel void nothing( {", name: "nothing" };
        let mut out = Buffer::new(4).unwrap();
        let e = run(&bad, &[Arg::Write(&mut out)], [1, 1]).unwrap_err();
        assert!(e.contains("does not build"), "{e}");
    }
}
