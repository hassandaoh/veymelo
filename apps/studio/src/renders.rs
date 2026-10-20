//! The renders going on, wherever they are made: read a few times a second on a thread of its
//! own, so the window knows when to draw again without looking at files itself.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use veymelo_engine::progress::{self, Render};

struct Watcher {
    now: Mutex<Vec<Render>>,
    /// Goes up whenever what is going on changes.
    count: AtomicU64,
}

fn watcher() -> &'static Watcher {
    static W: OnceLock<Watcher> = OnceLock::new();
    W.get_or_init(|| {
        std::thread::spawn(|| {
            loop {
                let seen = progress::renders();
                let w = watcher();
                let mut now = w.now.lock().unwrap_or_else(|e| e.into_inner());
                if *now != seen {
                    *now = seen;
                    w.count.fetch_add(1, Ordering::Relaxed);
                }
                drop(now);
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        });
        Watcher { now: Mutex::new(Vec::new()), count: AtomicU64::new(0) }
    })
}

/// The renders going on now, the oldest first.
pub fn now() -> Vec<Render> {
    watcher().now.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A count that goes up whenever the renders going on change.
pub fn count() -> u64 {
    watcher().count.load(Ordering::Relaxed)
}
