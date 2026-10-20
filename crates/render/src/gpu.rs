//! What the GPU draws, laid out as its kernels read it: each struct here is the one of the same
//! name in the kernel's source, field for field (only 16-byte fields, so both lay them out alike).

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use veymelo_os::gpu::{Buffer, Kernel, Plain};

/// A run of 3D layers put together (three.metal).
pub(crate) const SCENE: Kernel = Kernel { source: include_str!("three.metal"), name: "scene" };

/// The most layers of a run the GPU puts together (more are drawn by the processor).
pub(crate) const MOST_PLANES: usize = 64;

/// What the kernel takes for "no matte".
pub(crate) const NONE: u32 = u32::MAX;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Plane {
    pub m: [f32; 4],
    pub w: [f32; 4],
    pub at: [f32; 4],
    pub dz: [f32; 4],
    pub tex: [f32; 4],
    pub own: [f32; 4],
    pub reach: [i32; 4],
    pub img: [u32; 4],
    pub ownimg: [u32; 4],
    pub origin: [f32; 4],
    pub across: [f32; 4],
    pub down: [f32; 4],
    pub normal: [f32; 4],
    pub unworld: [[f32; 4]; 4],
    pub mat0: [f32; 4],
    pub mat1: [f32; 4],
    pub flags: [u32; 4],
    pub more: [u32; 4],
    pub order: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Light {
    pub at: [f32; 4],
    pub dir: [f32; 4],
    pub color: [f32; 4],
    pub fall: [f32; 4],
    pub kind: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Params {
    pub a: [u32; 4],
    pub b: [u32; 4],
    pub eye: [f32; 4],
}

unsafe impl Plain for Plane {}
unsafe impl Plain for Light {}
unsafe impl Plain for Params {}

static DRAWN: AtomicU64 = AtomicU64::new(0);

/// How many runs of 3D layers the GPU has put together in this process.
pub fn scenes_drawn() -> u64 {
    DRAWN.load(Ordering::Relaxed)
}

pub(crate) fn drew_scene() {
    DRAWN.fetch_add(1, Ordering::Relaxed);
}

/// The buffers a drawing thread keeps from one frame to the next, by what they hold (each made
/// again only when a bigger one is needed, so the GPU's memory is not asked for every frame).
pub(crate) const TEXELS: usize = 0;
pub(crate) const ROWS: usize = 1;
pub(crate) const PLANES: usize = 2;
pub(crate) const MATTES: usize = 3;
pub(crate) const LIGHTS: usize = 4;
pub(crate) const CASTERS: usize = 5;

thread_local! {
    static KEPT: RefCell<[Option<Buffer>; 6]> = const { RefCell::new([None, None, None, None, None, None]) };
}

/// A buffer of at least `len` bytes for what `slot` holds (what was in it is left: fill what is
/// read). Give it back with `keep`.
pub(crate) fn take(slot: usize, len: usize) -> Result<Buffer, String> {
    // the one kept, if it is big enough and not far bigger than needed (else it goes)
    if let Some(b) = KEPT.with(|k| k.borrow_mut()[slot].take())
        && b.len() >= len
        && b.len() <= len.max(1 << 20) * 4
    {
        return Ok(b);
    }
    // a little to spare, so a frame a little bigger than the last does not make it again
    Buffer::new(len + len / 4)
}

/// `items` in the buffer for `slot`.
pub(crate) fn take_with<T: Plain>(slot: usize, items: &[T]) -> Result<Buffer, String> {
    let mut b = take(slot, std::mem::size_of_val(items))?;
    b.slice_mut::<T>()[..items.len()].copy_from_slice(items);
    Ok(b)
}

/// The buffer back, for the next frame drawn on this thread (one too big to hold on to goes).
pub(crate) fn keep(slot: usize, b: Buffer) {
    if b.len() <= MOST_KEPT {
        KEPT.with(|k| k.borrow_mut()[slot] = Some(b));
    }
}

/// The most a buffer kept between frames holds (bytes): bigger ones are made when needed and
/// let go after.
const MOST_KEPT: usize = 96 << 20;

/// Lets go of the buffers kept on this thread (a thread that stops drawing for a while).
pub fn let_go() {
    KEPT.with(|k| *k.borrow_mut() = [None, None, None, None, None, None]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_structs_are_as_big_as_the_kernel_reads_them() {
        assert_eq!(std::mem::size_of::<Plane>(), 22 * 16);
        assert_eq!(std::mem::size_of::<Light>(), 5 * 16);
        assert_eq!(std::mem::size_of::<Params>(), 3 * 16);
    }

    #[test]
    fn the_kernels_build() {
        if !veymelo_os::gpu::available() {
            return;
        }
        veymelo_os::gpu::prepare(&SCENE).unwrap();
    }
}
