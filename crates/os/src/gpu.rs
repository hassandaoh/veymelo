//! The GPU, where there is one to draw with (Metal on a Mac with Apple silicon): kernels in the
//! Metal Shading Language run over a grid of threads on buffers it shares with the processor.
//! `VEYMELO_GPU=0` leaves it unused, so everything is drawn by the processor.

#[cfg(target_os = "macos")]
pub use crate::mac::gpu::{Buffer, name, prepare, run};

/// A GPU program: its source and the kernel in it to run.
#[derive(Clone, Copy, Debug)]
pub struct Kernel {
    pub source: &'static str,
    pub name: &'static str,
}

/// What a kernel takes, in the order of its parameters: a buffer it reads, one it writes (or
/// reads and writes), or a few bytes (a struct of settings).
pub enum Arg<'a> {
    Read(&'a Buffer),
    Write(&'a mut Buffer),
    Bytes(&'a [u8]),
}

/// Plain data, the same bytes on the processor and the GPU (numbers, and `#[repr(C)]` structs
/// of them laid out as the kernel's are).
///
/// # Safety
/// Every bit pattern of its size must be a valid value, and it must hold no pointers.
pub unsafe trait Plain: Copy {}

unsafe impl Plain for u8 {}
unsafe impl Plain for u32 {}
unsafe impl Plain for i32 {}
unsafe impl Plain for f32 {}
unsafe impl<T: Plain, const N: usize> Plain for [T; N] {}

/// The bytes of `v`, to hand a kernel.
pub fn bytes_of<T: Plain>(v: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), std::mem::size_of::<T>()) }
}

/// The bytes of `items`.
pub fn bytes_of_slice<T: Plain>(items: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(items.as_ptr().cast::<u8>(), std::mem::size_of_val(items)) }
}

/// Whether the GPU draws (there is one, and it is not turned off).
pub fn available() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let off = std::env::var("VEYMELO_GPU").is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "off" | "no" | "false"));
        !off && there()
    })
}

#[cfg(target_os = "macos")]
fn there() -> bool {
    name().is_some()
}

#[cfg(not(target_os = "macos"))]
fn there() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
mod none {
    use super::{Arg, Kernel, Plain};

    /// No GPU here: nothing is ever made.
    pub struct Buffer(());

    impl Buffer {
        pub fn new(_len: usize) -> Result<Buffer, String> {
            Err("there is no GPU to draw with here".into())
        }
        pub fn of<T: Plain>(_items: &[T]) -> Result<Buffer, String> {
            Buffer::new(0)
        }
        pub fn len(&self) -> usize {
            0
        }
        pub fn is_empty(&self) -> bool {
            true
        }
        pub fn bytes(&self) -> &[u8] {
            &[]
        }
        pub fn bytes_mut(&mut self) -> &mut [u8] {
            &mut []
        }
        pub fn slice<T: Plain>(&self) -> &[T] {
            &[]
        }
        pub fn slice_mut<T: Plain>(&mut self) -> &mut [T] {
            &mut []
        }
    }

    pub fn name() -> Option<String> {
        None
    }

    pub fn run(_kernel: &Kernel, _args: &[Arg], _grid: [usize; 2]) -> Result<(), String> {
        Err("there is no GPU to draw with here".into())
    }

    pub fn prepare(_kernel: &Kernel) -> Result<(), String> {
        Err("there is no GPU to draw with here".into())
    }
}

#[cfg(not(target_os = "macos"))]
pub use none::{Buffer, name, prepare, run};
