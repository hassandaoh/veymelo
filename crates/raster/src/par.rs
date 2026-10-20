//! Work spread over the machine's cores, with the standard library alone. A thread that is one
//! of many already sharing the machine (each drawing its own frame) keeps its work to itself.

use std::cell::Cell;

thread_local! {
    /// This thread's work stays on it.
    static ALONE: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f` with the work it would spread over the cores kept on this thread (for threads that
/// already share the machine among them).
pub fn alone<R>(f: impl FnOnce() -> R) -> R {
    let before = ALONE.with(|a| a.replace(true));
    let out = f();
    ALONE.with(|a| a.set(before));
    out
}

/// How many threads to share `n` pieces of work among.
fn threads_for(n: usize) -> usize {
    if ALONE.with(Cell::get) {
        return 1;
    }
    std::thread::available_parallelism().map_or(1, |n| n.get()).min(n).max(1)
}

/// Run `f(y, row)` for every row of `px` (rows `width` long), the rows shared out among threads.
pub fn rows<T: Send>(px: &mut [T], width: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    if width == 0 || px.is_empty() {
        return;
    }
    let height = px.len() / width;
    let threads = threads_for(height);
    // small pictures are quicker on one thread than the cost of starting more
    if threads == 1 || px.len() < 32 * 1024 {
        for (y, row) in px.chunks_mut(width).enumerate() {
            f(y, row);
        }
        return;
    }
    let per = height.div_ceil(threads);
    std::thread::scope(|s| {
        for (k, block) in px.chunks_mut(per * width).enumerate() {
            let f = &f;
            s.spawn(move || {
                for (i, row) in block.chunks_mut(width).enumerate() {
                    f(k * per + i, row);
                }
            });
        }
    });
}

/// Run `f(k, chunk)` for every chunk of `px` `size` long (the last may be shorter), the chunks
/// shared out among threads.
pub fn chunks<T: Send>(px: &mut [T], size: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    if size == 0 || px.is_empty() {
        return;
    }
    let count = px.len().div_ceil(size);
    let threads = threads_for(count);
    if threads == 1 || px.len() < 32 * 1024 {
        for (k, c) in px.chunks_mut(size).enumerate() {
            f(k, c);
        }
        return;
    }
    let per = count.div_ceil(threads);
    std::thread::scope(|s| {
        for (t, block) in px.chunks_mut(per * size).enumerate() {
            let f = &f;
            s.spawn(move || {
                for (i, c) in block.chunks_mut(size).enumerate() {
                    f(t * per + i, c);
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_chunk_once() {
        let mut px = vec![0usize; 100_003];
        super::chunks(&mut px, 1000, |k, c| c.iter_mut().for_each(|p| *p = k + 1));
        assert!(px.iter().enumerate().all(|(i, p)| *p == i / 1000 + 1));
    }

    #[test]
    fn alone_keeps_the_work_on_the_thread() {
        let mut px = vec![0usize; 400 * 400];
        let here = std::thread::current().id();
        let seen = std::sync::Mutex::new(Vec::new());
        super::alone(|| super::rows(&mut px, 400, |_, _| seen.lock().unwrap().push(std::thread::current().id())));
        assert!(seen.lock().unwrap().iter().all(|t| *t == here));
    }

    #[test]
    fn every_row_once_with_its_number() {
        let (w, h) = (300, 500);
        let mut px = vec![0usize; w * h];
        super::rows(&mut px, w, |y, row| row.iter_mut().for_each(|p| *p += y + 1));
        for y in 0..h {
            assert!(px[y * w..(y + 1) * w].iter().all(|&p| p == y + 1));
        }
    }
}
