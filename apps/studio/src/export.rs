//! The app's own render: the composition shown, made into a file on a thread of its own from
//! the project as it was when Render was pressed (so the window and the AI go on meanwhile),
//! telling how far it has come as every render does, and stopped if asked.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use veymelo_engine::progress::Reporter;
use veymelo_engine::project::Project;
use veymelo_engine::project::queue::RenderItem;

/// A render the app started: what of, where to, when, how far (a fraction's bits), and how it
/// ended.
struct Job {
    project: Option<PathBuf>,
    out: PathBuf,
    started: Instant,
    stop: Arc<AtomicBool>,
    fraction: Arc<AtomicU64>,
    ended: Arc<Mutex<Option<(Result<(), String>, Instant)>>>,
}

static JOB: Mutex<Option<Job>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// How the app's render stands, for the project in `file`.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Going: how far (0 to 1), and about how many seconds it has left (once that can be told).
    Going(f64, Option<f64>),
    /// Made `out`, so many seconds ago, in so many seconds.
    Made(PathBuf, f64, f64),
    /// Stopped or failed: why, so many seconds ago.
    Failed(String, f64),
}

/// The app's render of the project in `file`, if it has one going or just ended.
pub fn state(file: &Path) -> Option<State> {
    let g = lock(&JOB);
    let j = g.as_ref().filter(|j| j.project.as_deref() == Some(file))?;
    let ended = lock(&j.ended);
    Some(match &*ended {
        None => {
            let k = f64::from_bits(j.fraction.load(Ordering::Relaxed));
            let spent = j.started.elapsed().as_secs_f64();
            // as fast on as so far (told once a little of it is made)
            State::Going(k, (k > 0.03 && spent > 1.0).then(|| spent * (1.0 - k) / k))
        }
        Some((Ok(()), at)) => State::Made(j.out.clone(), at.elapsed().as_secs_f64(), at.duration_since(j.started).as_secs_f64()),
        Some((Err(why), at)) => State::Failed(why.clone(), at.elapsed().as_secs_f64()),
    })
}

/// How long something took, in words: `6.2 s`, `1 min 5 s`.
pub fn took(secs: f64) -> String {
    if secs < 60.0 { format!("{secs:.1} s") } else { format!("{} min {} s", secs as u64 / 60, secs as u64 % 60) }
}

/// A time left in words: `12 s left`, `1 min 5 s left`.
pub fn left(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s < 60 { format!("{s} s left") } else { format!("{} min {} s left", s / 60, s % 60) }
}

/// Whether a render of the app's is going (one at a time).
pub fn going() -> bool {
    lock(&JOB).as_ref().is_some_and(|j| lock(&j.ended).is_none())
}

/// Starts making composition `comp` (called `name`) of `project` (from `file`) into `out`.
pub fn start(project: Arc<Project>, file: Option<PathBuf>, comp: u64, name: &str, out: PathBuf) -> Result<(), String> {
    if going() {
        return Err("a render is going already: it can be stopped first".into());
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    }
    let mut item = RenderItem::new(0, comp);
    item.output.path = out.display().to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let fraction = Arc::new(AtomicU64::new(0f64.to_bits()));
    let ended = Arc::new(Mutex::new(None));
    let (s, f, e, n, o) = (stop.clone(), fraction.clone(), ended.clone(), name.to_string(), item.output.path.clone());
    let owner = file.clone();
    std::thread::spawn(move || {
        let mut told = Reporter::begin(owner.as_deref(), &n, &o);
        let made = veymelo_export::make(&project, &item, &mut |done, total| {
            told.tick(done, total);
            f.store((done as f64 / total.max(1) as f64).to_bits(), Ordering::Relaxed);
            !s.load(Ordering::Relaxed)
        });
        drop(told);
        let result = match made {
            Ok(_) => Ok(()),
            Err(_) if s.load(Ordering::Relaxed) => Err("stopped".into()),
            Err(why) => Err(why),
        };
        *lock(&e) = Some((result, Instant::now()));
    });
    *lock(&JOB) = Some(Job { project: file, out, started: Instant::now(), stop, fraction, ended });
    Ok(())
}

/// Stops the app's render (its half-made file goes).
pub fn stop() {
    if let Some(j) = lock(&JOB).as_ref() {
        j.stop.store(true, Ordering::Relaxed);
    }
}

