//! A picture's pixels: on the heap, or (what the GPU made) in memory the processor and the GPU
//! share, so the GPU works on them again where they are, with nothing copied there or back.
//! Either way they are read and written as a slice. Pictures start on the heap (its memory costs
//! nothing until it is drawn on); shared memory comes from a pool: a picture let go gives its
//! memory back for the next the GPU makes.

use std::ops::{Deref, DerefMut};
use std::sync::Mutex;

use veymelo_os::gpu::Buffer;

use crate::Px;

/// A picture's pixels, row after row.
pub struct Pixels(Store);

enum Store {
    Heap(Vec<Px>),
    /// The memory, where it starts (a buffer's memory stays where it is while the buffer
    /// lasts, so it is asked for once), and how many pixels.
    Shared { buf: Option<Buffer>, at: *mut Px, len: usize },
}

// the shared memory is owned by the buffer held with it, reached only through `&`/`&mut` of the
// pixels, as a Vec's is
unsafe impl Send for Pixels {}
unsafe impl Sync for Pixels {}

impl Pixels {
    /// `n` clear pixels.
    pub fn zeroed(n: usize) -> Pixels {
        Pixels::filled(n, [0.0; 4])
    }

    /// `n` pixels of `p`.
    pub fn filled(n: usize, p: Px) -> Pixels {
        Pixels(Store::Heap(vec![p; n]))
    }

    /// `n` pixels shared with the GPU whatever they hold (for the GPU to fill), if there is one.
    pub fn shared_unfilled(n: usize) -> Option<Pixels> {
        take(n).map(|mut buf| {
            let at = buf.slice_mut::<Px>().as_mut_ptr();
            Pixels(Store::Shared { buf: Some(buf), at, len: n })
        })
    }

    /// The memory the GPU can work on them in, if they are shared.
    pub fn buffer(&self) -> Option<&Buffer> {
        match &self.0 {
            Store::Shared { buf, .. } => buf.as_ref(),
            Store::Heap(_) => None,
        }
    }

    /// As a list of their own.
    pub fn into_vec(mut self) -> Vec<Px> {
        match &mut self.0 {
            Store::Heap(v) => std::mem::take(v),
            Store::Shared { .. } => self.to_vec(),
        }
    }
}

impl Deref for Pixels {
    type Target = [Px];

    fn deref(&self) -> &[Px] {
        match &self.0 {
            Store::Heap(v) => v,
            Store::Shared { at, len, .. } => unsafe { std::slice::from_raw_parts(*at, *len) },
        }
    }
}

impl DerefMut for Pixels {
    fn deref_mut(&mut self) -> &mut [Px] {
        match &mut self.0 {
            Store::Heap(v) => v,
            Store::Shared { at, len, .. } => unsafe { std::slice::from_raw_parts_mut(*at, *len) },
        }
    }
}

impl Drop for Pixels {
    fn drop(&mut self) {
        if let Store::Shared { buf, .. } = &mut self.0
            && let Some(b) = buf.take()
        {
            give_back(b);
        }
    }
}

impl Clone for Pixels {
    /// A copy on the heap (to be worked on by the processor, as copies mostly are).
    fn clone(&self) -> Pixels {
        Pixels(Store::Heap(self.to_vec()))
    }
}

impl PartialEq for Pixels {
    fn eq(&self, other: &Pixels) -> bool {
        **self == **other
    }
}

impl std::fmt::Debug for Pixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl From<Vec<Px>> for Pixels {
    fn from(v: Vec<Px>) -> Pixels {
        Pixels(Store::Heap(v))
    }
}

impl FromIterator<Px> for Pixels {
    fn from_iter<I: IntoIterator<Item = Px>>(iter: I) -> Pixels {
        Pixels(Store::Heap(iter.into_iter().collect()))
    }
}

impl IntoIterator for Pixels {
    type Item = Px;
    type IntoIter = std::vec::IntoIter<Px>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
    }
}

impl<'a> IntoIterator for &'a Pixels {
    type Item = &'a Px;
    type IntoIter = std::slice::Iter<'a, Px>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut Pixels {
    type Item = &'a mut Px;
    type IntoIter = std::slice::IterMut<'a, Px>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// Shared memory let go, by size (bytes), to be taken again; and how much of it there is.
struct Pool {
    free: Vec<Buffer>,
    bytes: usize,
}

static POOL: Mutex<Pool> = Mutex::new(Pool { free: Vec::new(), bytes: 0 });

/// The most shared memory kept for pictures to come.
const MOST_KEPT: usize = 1 << 30;

/// Shared memory for `n` pixels: one let go that fits (not far bigger), or new (None: no GPU).
fn take(n: usize) -> Option<Buffer> {
    if !crate::gpu::available() {
        return None;
    }
    let len = n * std::mem::size_of::<Px>();
    {
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        let best = pool.free.iter().enumerate().filter(|(_, b)| b.len() >= len && b.len() <= len + len / 4 + (1 << 20)).min_by_key(|(_, b)| b.len()).map(|(i, _)| i);
        // (a few dozen at most: looked through whole)
        if let Some(i) = best {
            let b = pool.free.swap_remove(i);
            pool.bytes -= b.len();
            return Some(b);
        }
    }
    Buffer::new_unfilled(len).ok()
}

/// Shared memory back in the pool (unless it holds too much already: then the oldest go).
fn give_back(b: Buffer) {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    pool.bytes += b.len();
    pool.free.push(b);
    while pool.bytes > MOST_KEPT && !pool.free.is_empty() {
        let old = pool.free.remove(0);
        pool.bytes -= old.len();
    }
}
