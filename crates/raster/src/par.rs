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
    // the threads draw where this one does (on the processor alone, or not)
    let mode = crate::gpu::processor_only();
    std::thread::scope(|s| {
        for (k, block) in px.chunks_mut(per * width).enumerate() {
            let f = &f;
            s.spawn(move || {
                crate::gpu::with_mode(mode, || {
                    for (i, row) in block.chunks_mut(width).enumerate() {
                        f(k * per + i, row);
                    }
                })
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
    let mode = crate::gpu::processor_only();
    std::thread::scope(|s| {
        for (t, block) in px.chunks_mut(per * size).enumerate() {
            let f = &f;
            s.spawn(move || {
                crate::gpu::with_mode(mode, || {
                    for (i, c) in block.chunks_mut(size).enumerate() {
                        f(t * per + i, c);
                    }
                })
            });
        }
    });
}

/// `f(k)` for every `k` below `n`, worked out on the cores (each takes the next not yet taken),
/// in the order of `k`.
pub fn map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = threads_for(n);
    if threads == 1 || n < 2 {
        return (0..n).map(f).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mode = crate::gpu::processor_only();
    let mut done: Vec<(usize, T)> = std::thread::scope(|s| {
        let work: Vec<_> = (0..threads)
            .map(|_| {
                let (f, next) = (&f, &next);
                s.spawn(move || {
                    crate::gpu::with_mode(mode, || {
                        let mut mine = Vec::new();
                        loop {
                            let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if k >= n {
                                break mine;
                            }
                            mine.push((k, f(k)));
                        }
                    })
                })
            })
            .collect();
        work.into_iter().flat_map(|w| w.join().expect("a piece of work")).collect()
    });
    done.sort_unstable_by_key(|(k, _)| *k);
    done.into_iter().map(|(_, t)| t).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn map_keeps_the_order() {
        let got = super::map(1000, |k| k * k);
        assert!(got.iter().enumerate().all(|(k, v)| *v == k * k));
    }

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
