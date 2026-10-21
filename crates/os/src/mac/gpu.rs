//! The GPU, through Metal (a part of macOS): programs in the Metal Shading Language, built from
//! their source the first time each is run, run over a grid of threads on buffers the processor
//! and the GPU share. Apple silicon has one memory for both, so a buffer filled here is read
//! there without being copied.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use super::{Id, NIL, Pool, Sel, error_text, msg, nsstring, rust_string, sel, send, send1};
use crate::gpu::{Arg, Kernel, Plain, Step};

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
/// MTLResourceStorageModePrivate: memory only the GPU uses.
const PRIVATE: usize = 2 << 4;

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

/// MTLAccelerationStructureSizes.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Sizes {
    structure: usize,
    build: usize,
    refit: usize,
}

/// Whether the GPU finds where rays meet triangles (Metal's ray tracing: in the hardware of the
/// M3 and the chips after it, done by its cores before them), and kernels can be handed what it
/// finds them in.
pub fn traces() -> bool {
    static TRACES: OnceLock<bool> = OnceLock::new();
    *TRACES.get_or_init(|| {
        let Some(g) = gpu() else { return false };
        unsafe {
            super::answers(g.device, "supportsRaytracing")
                && msg!(unsafe extern "C" fn(Id, Sel) -> bool)(g.device, sel("supportsRaytracing"))
                && super::answers(g.device, "accelerationStructureSizesWithDescriptor:")
                && class_answers("MTLComputeCommandEncoder", "setAccelerationStructure:atBufferIndex:")
        }
    })
}

/// Whether instances of the protocol a Metal object of class-ish name `what` answers `name` (asked
/// of a live one: an encoder made and ended).
unsafe fn class_answers(what: &str, name: &str) -> bool {
    let Some(g) = gpu() else { return false };
    unsafe {
        let _pool = Pool::new();
        let commands = send(g.queue, "commandBuffer");
        if commands.is_null() {
            return false;
        }
        let encoder = match what {
            "MTLComputeCommandEncoder" => send(commands, "computeCommandEncoder"),
            _ => NIL,
        };
        if encoder.is_null() {
            return false;
        }
        let yes = super::answers(encoder, name);
        send(encoder, "endEncoding");
        yes
    }
}

/// Triangles arranged by the GPU so a ray finds those it meets quickly (an acceleration
/// structure), for kernels that trace rays.
pub struct Shapes {
    id: Id,
}

// a Metal acceleration structure may be used from any thread
unsafe impl Send for Shapes {}
unsafe impl Sync for Shapes {}

impl Shapes {
    /// From triangles, three corners each one after another; none when there are none.
    pub fn new(corners: &[[f32; 3]]) -> Result<Option<Shapes>, String> {
        let g = gpu().ok_or("there is no GPU to draw with")?;
        if !traces() {
            return Err("the GPU does not trace rays".into());
        }
        let count = corners.len() / 3;
        if count == 0 {
            return Ok(None);
        }
        let vertices = Buffer::of(&corners[..count * 3])?;
        unsafe {
            let _pool = Pool::new();
            let geometry = send(super::class("MTLAccelerationStructureTriangleGeometryDescriptor"), "descriptor");
            let descriptor = send(super::class("MTLPrimitiveAccelerationStructureDescriptor"), "descriptor");
            if geometry.is_null() || descriptor.is_null() {
                return Err("the GPU cannot arrange triangles here".into());
            }
            send1(geometry, "setVertexBuffer:", vertices.id);
            msg!(unsafe extern "C" fn(Id, Sel, usize))(geometry, sel("setVertexStride:"), 12);
            msg!(unsafe extern "C" fn(Id, Sel, usize))(geometry, sel("setTriangleCount:"), count);
            msg!(unsafe extern "C" fn(Id, Sel, bool))(geometry, sel("setOpaque:"), true);
            let list = send1(super::class("NSArray"), "arrayWithObject:", geometry);
            send1(descriptor, "setGeometryDescriptors:", list);
            let sizes = msg!(unsafe extern "C" fn(Id, Sel, Id) -> Sizes)(g.device, sel("accelerationStructureSizesWithDescriptor:"), descriptor);
            let id = msg!(unsafe extern "C" fn(Id, Sel, usize) -> Id)(g.device, sel("newAccelerationStructureWithSize:"), sizes.structure);
            if id.is_null() {
                return Err(format!("the GPU has no room for {} triangles", count));
            }
            let shapes = Shapes { id };
            let scratch = msg!(unsafe extern "C" fn(Id, Sel, usize, usize) -> Id)(g.device, sel("newBufferWithLength:options:"), sizes.build.max(16), PRIVATE);
            if scratch.is_null() {
                return Err(format!("the GPU has no room to arrange {} triangles", count));
            }
            let scratch = Buffer { id: scratch, len: sizes.build };
            let commands = send(g.queue, "commandBuffer");
            let encoder = if commands.is_null() { NIL } else { send(commands, "accelerationStructureCommandEncoder") };
            if encoder.is_null() {
                return Err("the GPU took no work".into());
            }
            msg!(unsafe extern "C" fn(Id, Sel, Id, Id, Id, usize))(encoder, sel("buildAccelerationStructure:descriptor:scratchBuffer:scratchBufferOffset:"), shapes.id, descriptor, scratch.id, 0);
            send(encoder, "endEncoding");
            let _turn = Turn::take();
            send(commands, "commit");
            wait(commands, &[&vertices, &scratch], &[])?;
            Ok(Some(shapes))
        }
    }
}

impl Drop for Shapes {
    fn drop(&mut self) {
        unsafe { send(self.id, "release") };
    }
}

/// Waits for `commands` to finish (not for ever: a GPU that does not answer is given up on, what
/// it reads and writes kept for it).
unsafe fn wait(commands: Id, buffers: &[&Buffer], shapes: &[&Shapes]) -> Result<(), String> {
    unsafe {
        let start = std::time::Instant::now();
        while super::integer(commands, "status") < COMPLETED {
            if start.elapsed() > PATIENCE {
                for b in buffers {
                    send(b.id, "retain");
                }
                for s in shapes {
                    send(s.id, "retain");
                }
                send(commands, "retain");
                return Err(format!("the GPU did not answer in {} seconds", PATIENCE.as_secs()));
            }
            std::thread::sleep(std::time::Duration::from_micros(150));
        }
        if super::integer(commands, "status") != COMPLETED {
            return Err(format!("the GPU stopped part way: {}", error_text(send(commands, "error"))));
        }
        Ok(())
    }
}

/// Builds `kernel` now (else it is built the first time it runs), saying why it cannot be.
pub fn prepare(kernel: &Kernel) -> Result<(), String> {
    gpu().ok_or("there is no GPU to draw with")?.pipeline(kernel).map(|_| ())
}

/// Runs `kernel` once for each point of `grid` (across, down), its buffers and bytes given in
/// the order its parameters take them, and waits for it to end.
pub fn run(kernel: &Kernel, args: &[Arg], grid: [usize; 2]) -> Result<(), String> {
    run_each(&[(kernel, args, grid, None)])
}

/// Runs `steps` one after another as one piece of work (each sees what those before it wrote),
/// and waits for the last to end.
pub fn run_steps(steps: &[Step]) -> Result<(), String> {
    let each: Vec<OneStep> = steps.iter().map(|s| (s.kernel, s.args.as_slice(), s.grid, s.group)).collect();
    run_each(&each)
}

/// A kernel, what it takes, its grid and its groups.
type OneStep<'a> = (&'a Kernel, &'a [Arg<'a>], [usize; 2], Option<[usize; 2]>);

fn run_each(steps: &[OneStep]) -> Result<(), String> {
    let g = gpu().ok_or("there is no GPU to draw with")?;
    let steps: Vec<_> = steps.iter().filter(|s| s.2[0] > 0 && s.2[1] > 0).collect();
    if steps.is_empty() {
        return Ok(());
    }
    let pipelines = steps.iter().map(|s| g.pipeline(s.0)).collect::<Result<Vec<_>, String>>()?;
    let _pool = Pool::new();
    // bytes too many to hand straight over go in a buffer of their own
    let mut held = Vec::new();
    for a in steps.iter().flat_map(|s| s.1.iter()) {
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
        let mut extra = held.iter();
        for (step, (pipeline, across, down)) in steps.iter().zip(&pipelines) {
            send1(encoder, "setComputePipelineState:", *pipeline);
            for (i, a) in step.1.iter().enumerate() {
                let buffer = |b: &Buffer| msg!(unsafe extern "C" fn(Id, Sel, Id, usize, usize))(encoder, sel("setBuffer:offset:atIndex:"), b.id, 0, i);
                match a {
                    Arg::Read(b) | Arg::Pass(b) => buffer(b),
                    Arg::Write(b) => buffer(b),
                    Arg::Shapes(s) => msg!(unsafe extern "C" fn(Id, Sel, Id, usize))(encoder, sel("setAccelerationStructure:atBufferIndex:"), s.id, i),
                    Arg::Bytes(bytes) if bytes.len() > INLINE => buffer(extra.next().expect("held")),
                    Arg::Bytes(bytes) => {
                        // a parameter is never empty
                        let pad = [0u8; 16];
                        let b: &[u8] = if bytes.is_empty() { &pad } else { bytes };
                        msg!(unsafe extern "C" fn(Id, Sel, *const c_void, usize, usize))(encoder, sel("setBytes:length:atIndex:"), b.as_ptr().cast(), b.len(), i);
                    }
                }
            }
            let whole = Size { width: step.2[0], height: step.2[1], depth: 1 };
            let [across, down] = step.3.unwrap_or([*across, *down]);
            let group = Size { width: across, height: down, depth: 1 };
            msg!(unsafe extern "C" fn(Id, Sel, Size, Size))(encoder, sel("dispatchThreads:threadsPerThreadgroup:"), whole, group);
        }
        send(encoder, "endEncoding");
        // a few pieces of work on the GPU at a time (many threads drawing frames would otherwise
        // pile theirs up together), and not waited for for ever: a GPU that does not answer
        // (the computer short of memory) is given up on, and the work is done without it
        let _turn = Turn::take();
        send(commands, "commit");
        let mut buffers: Vec<&Buffer> = held.iter().collect();
        let mut shapes = Vec::new();
        for a in steps.iter().flat_map(|s| s.1.iter()) {
            match a {
                Arg::Read(b) | Arg::Pass(b) => buffers.push(b),
                Arg::Write(b) => buffers.push(b),
                Arg::Shapes(s) => shapes.push(*s),
                Arg::Bytes(_) => {}
            }
        }
        wait(commands, &buffers, &shapes)?;
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
    fn rays_meet_the_triangles_they_go_through() {
        if gpu().is_none() || !traces() {
            eprintln!("no GPU that traces rays here: nothing to test");
            return;
        }
        const CAST: Kernel = Kernel {
            source: "#include <metal_stdlib>\n#include <metal_raytracing>\nusing namespace metal;\nusing namespace metal::raytracing;\n\
                     kernel void cast(device float *out [[buffer(0)]], primitive_acceleration_structure shapes [[buffer(1)]], uint2 at [[thread_position_in_grid]]) {\n\
                         ray r;\n\
                         r.origin = float3(float(at.x) - 1.5, float(at.y) - 1.5, -5.0);\n\
                         r.direction = float3(0.0, 0.0, 1.0);\n\
                         r.min_distance = 0.0;\n\
                         r.max_distance = 100.0;\n\
                         intersector<> it;\n\
                         intersection_result<> h = it.intersect(r, shapes);\n\
                         out[at.y * 4 + at.x] = h.type == intersection_type::triangle ? h.distance : -1.0;\n\
                     }",
            name: "cast",
        };
        // a triangle at z = 2, its long side from (1, -1) to (-1, 1)
        let shapes = Shapes::new(&[[-1.0, -1.0, 2.0], [1.0, -1.0, 2.0], [-1.0, 1.0, 2.0]]).unwrap().expect("a triangle");
        let mut out = Buffer::new(16 * 4).unwrap();
        run(&CAST, &[Arg::Write(&mut out), Arg::Shapes(&shapes)], [4, 4]).unwrap();
        let got = out.slice::<f32>();
        // (-0.5, -0.5) is inside, 7 away; (1.5, 1.5) and (-1.5, -0.5) are not
        assert_eq!(got[4 + 1], 7.0);
        assert_eq!(got[3 * 4 + 3], -1.0);
        assert_eq!(got[4], -1.0);
        assert!(Shapes::new(&[]).unwrap().is_none());
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
