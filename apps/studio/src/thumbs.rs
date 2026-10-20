//! Each project's picture for its plate on the page of projects: a frame of the composition it
//! opens on, a third of the way in, drawn small on a thread of its own and kept; drawn again
//! when its file changes (at most every few seconds, as an AI may change it often).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Instant, SystemTime};

/// The longest side a picture is drawn at (pixels), and how soon after one is drawn the next of
/// the same project may be.
const LONGEST: f64 = 240.0;
const AGAIN_AFTER: f64 = 4.0;

/// A project's picture: straight RGB pixels, and its composition's size and length.
pub struct Thumb {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<u8>,
    pub duration: f64,
}

impl Thumb {
    /// Its colour at (`x`, `y`) (0 to 1 across and down), between its pixels.
    pub fn at(&self, x: f64, y: f64) -> [f32; 3] {
        let (fx, fy) = ((x * self.w as f64 - 0.5).clamp(0.0, (self.w - 1) as f64), (y * self.h as f64 - 0.5).clamp(0.0, (self.h - 1) as f64));
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let get = |x: usize, y: usize, c: usize| self.rgb[(y * self.w + x) * 3 + c] as f32;
        std::array::from_fn(|c| {
            let top = get(x0, y0, c) + (get(x1, y0, c) - get(x0, y0, c)) * tx;
            let bottom = get(x0, y1, c) + (get(x1, y1, c) - get(x0, y1, c)) * tx;
            top + (bottom - top) * ty
        })
    }
}

/// What is kept of a project: its picture (none if it has nothing to show), what its file's
/// time was when drawn, and when.
struct Kept {
    thumb: Option<Arc<Thumb>>,
    for_time: Option<SystemTime>,
    made: Instant,
}

struct Thumbs {
    kept: Mutex<HashMap<PathBuf, Kept>>,
    /// The projects waiting to be drawn, and the drawing thread's wake-up.
    queue: Mutex<Vec<(PathBuf, Option<SystemTime>)>>,
    wake: Condvar,
    /// Goes up whenever a picture is drawn.
    count: AtomicU64,
}

fn thumbs() -> &'static Thumbs {
    static T: OnceLock<Thumbs> = OnceLock::new();
    T.get_or_init(|| {
        std::thread::spawn(draw_loop);
        Thumbs { kept: Mutex::new(HashMap::new()), queue: Mutex::new(Vec::new()), wake: Condvar::new(), count: AtomicU64::new(0) }
    })
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The picture of the project in `file` (whose file last changed at `modified`), if one is
/// drawn; asks for it (again) when there is none yet or the file changed since.
pub fn get(file: &Path, modified: Option<SystemTime>) -> Option<Arc<Thumb>> {
    let t = thumbs();
    let kept = lock(&t.kept);
    let (thumb, stale) = match kept.get(file) {
        Some(k) => (k.thumb.clone(), k.for_time != modified && k.made.elapsed().as_secs_f64() > AGAIN_AFTER),
        None => (None, true),
    };
    drop(kept);
    if stale {
        let mut q = lock(&t.queue);
        if !q.iter().any(|(f, _)| f == file) {
            q.push((file.to_path_buf(), modified));
            t.wake.notify_one();
        }
    }
    thumb
}

/// A count that goes up whenever a picture is drawn.
pub fn count() -> u64 {
    thumbs().count.load(Ordering::Relaxed)
}

fn draw_loop() {
    let t = thumbs();
    loop {
        let (file, modified) = {
            let mut q = lock(&t.queue);
            while q.is_empty() {
                q = t.wake.wait(q).unwrap_or_else(|e| e.into_inner());
            }
            q.remove(0)
        };
        let thumb = draw(&file).map(Arc::new);
        lock(&t.kept).insert(file, Kept { thumb, for_time: modified, made: Instant::now() });
        t.count.fetch_add(1, Ordering::Relaxed);
    }
}

/// The project's picture: the composition it opens on, a third of the way in.
fn draw(file: &Path) -> Option<Thumb> {
    let session = veymelo_engine::Session::open(file).ok()?;
    let comp_id = session.view.comp?;
    let project = session.project();
    let comp = project.comp(comp_id)?;
    let (w, h) = (comp.width as f64, comp.height as f64);
    let scale = (LONGEST / w.max(h)).min(1.0);
    let duration = comp.duration.seconds();
    let options = veymelo_engine::render::Options { scale, motion_blur: false, ..Default::default() };
    let r = veymelo_engine::render::render(project, comp_id, veymelo_time::Tick::from_seconds(duration / 3.0), &options).ok()?;
    let img = r.image;
    let mut rgb = Vec::with_capacity(img.width * img.height * 3);
    for p in &img.px {
        // (over white where it is clear)
        let k = 1.0 - p[3].clamp(0.0, 1.0);
        rgb.extend([0, 1, 2].map(|c| ((p[c] + k).clamp(0.0, 1.0) * 255.0).round() as u8));
    }
    Some(Thumb { w: img.width, h: img.height, rgb, duration })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_read_between_its_pixels() {
        let t = Thumb { w: 2, h: 1, rgb: vec![0, 0, 0, 255, 255, 255], duration: 1.0 };
        assert_eq!(t.at(0.0, 0.5), [0.0; 3]);
        assert_eq!(t.at(1.0, 0.5), [255.0; 3]);
        assert!((t.at(0.5, 0.5)[0] - 127.5).abs() < 1e-3);
    }
}
