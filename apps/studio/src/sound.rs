//! The preview's sound: while it plays, the composition's sound is mixed a little ahead on a
//! thread of its own (the same mix an export makes) and handed to the system's audio output
//! (AudioToolbox's audio queue, a part of macOS, called straight from Rust). The time the sound
//! has reached is the preview's clock, so picture and sound stay together.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use veymelo_engine::render::audio;
use veymelo_engine::project::Project;
use veymelo_time::Tick;

/// The rate it plays at, the frames in each buffer handed to the system, how many buffers, and
/// how far ahead of the system the mix keeps (frames).
const RATE: u32 = audio::RATE;
const BUFFER: usize = 2048;
const BUFFERS: usize = 3;
const AHEAD: usize = RATE as usize / 2;
/// The frames mixed at a time.
const BLOCK: usize = 4800;

type Queue = *mut c_void;

#[repr(C)]
struct Format {
    rate: f64,
    id: u32,
    flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels: u32,
    bits: u32,
    reserved: u32,
}

#[repr(C)]
struct Buffer {
    capacity: u32,
    data: *mut c_void,
    size: u32,
    user: *mut c_void,
    packet_capacity: u32,
    packets: *mut c_void,
    packet_count: u32,
}

#[repr(C)]
#[derive(Default)]
struct Smpte {
    subframes: i16,
    divisor: i16,
    counter: u32,
    kind: u32,
    flags: u32,
    hours: i16,
    minutes: i16,
    seconds: i16,
    frames: i16,
}

#[repr(C)]
#[derive(Default)]
struct TimeStamp {
    sample_time: f64,
    host_time: u64,
    rate_scalar: f64,
    word_clock: u64,
    smpte: Smpte,
    flags: u32,
    reserved: u32,
}

type Callback = extern "C" fn(*mut c_void, Queue, *mut Buffer);

#[cfg(target_os = "macos")]
#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioQueueNewOutput(format: *const Format, callback: Callback, user: *mut c_void, run_loop: *mut c_void, mode: *const c_void, flags: u32, queue: *mut Queue) -> i32;
    fn AudioQueueAllocateBuffer(queue: Queue, bytes: u32, buffer: *mut *mut Buffer) -> i32;
    fn AudioQueueEnqueueBuffer(queue: Queue, buffer: *mut Buffer, packets: u32, descriptions: *const c_void) -> i32;
    fn AudioQueueStart(queue: Queue, at: *const c_void) -> i32;
    fn AudioQueueStop(queue: Queue, immediate: u8) -> i32;
    fn AudioQueueDispose(queue: Queue, immediate: u8) -> i32;
    fn AudioQueueGetCurrentTime(queue: Queue, timeline: *mut c_void, time: *mut TimeStamp, discontinuity: *mut u8) -> i32;
}

/// The sound mixed ahead, waiting for the system to take it (left, right, left…).
struct Feed {
    samples: Mutex<VecDeque<f32>>,
    stop: AtomicBool,
}

/// Sound playing: its queue, what feeds it, and the composition time it started from.
struct Player {
    queue: Queue,
    feed: Arc<Feed>,
    from: f64,
}

unsafe impl Send for Player {}

static PLAYER: Mutex<Option<Player>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The system wants a buffer filled: from what is mixed, silence where nothing is yet.
extern "C" fn refill(user: *mut c_void, queue: Queue, buffer: *mut Buffer) {
    let feed = unsafe { &*(user as *const Feed) };
    unsafe {
        let b = &mut *buffer;
        let out = std::slice::from_raw_parts_mut(b.data as *mut f32, BUFFER * 2);
        let mut s = lock(&feed.samples);
        for o in out.iter_mut() {
            *o = s.pop_front().unwrap_or(0.0);
        }
        drop(s);
        b.size = (BUFFER * 2 * 4) as u32;
        if !feed.stop.load(Ordering::Relaxed) {
            AudioQueueEnqueueBuffer(queue, buffer, 0, std::ptr::null());
        }
    }
}

/// Plays the sound of composition `comp` of `project` from `from` seconds (stopping what
/// played). Nothing plays when it has no sound.
pub fn play(project: Arc<Project>, comp: u64, from: f64) {
    stop();
    if !audio::heard(&project, comp) {
        return;
    }
    let length = project.comp(comp).map_or(0.0, |c| c.duration.seconds());
    let feed = Arc::new(Feed { samples: Mutex::new(VecDeque::with_capacity(AHEAD * 4)), stop: AtomicBool::new(false) });
    // the mix runs ahead of the system on its own thread
    let mixing = feed.clone();
    std::thread::spawn(move || {
        let mut at = (from * RATE as f64).round() as i64;
        let end = (length * RATE as f64).round() as i64;
        while !mixing.stop.load(Ordering::Relaxed) && at < end {
            if lock(&mixing.samples).len() / 2 >= AHEAD {
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue;
            }
            let n = BLOCK.min((end - at) as usize);
            let block = audio::mix(&project, comp, Tick::from_seconds(at as f64 / RATE as f64), n, RATE);
            lock(&mixing.samples).extend(block);
            at += n as i64;
        }
    });
    // the first of it mixed before the system starts asking
    let start = std::time::Instant::now();
    while lock(&feed.samples).len() / 2 < BUFFER * BUFFERS && start.elapsed().as_millis() < 300 {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let format = Format { rate: RATE as f64, id: u32::from_be_bytes(*b"lpcm"), flags: 1 | 8, bytes_per_packet: 8, frames_per_packet: 1, bytes_per_frame: 8, channels: 2, bits: 32, reserved: 0 };
    let mut queue: Queue = std::ptr::null_mut();
    unsafe {
        if AudioQueueNewOutput(&format, refill, Arc::as_ptr(&feed) as *mut c_void, std::ptr::null_mut(), std::ptr::null(), 0, &mut queue) != 0 || queue.is_null() {
            feed.stop.store(true, Ordering::Relaxed);
            return;
        }
        for _ in 0..BUFFERS {
            let mut b: *mut Buffer = std::ptr::null_mut();
            if AudioQueueAllocateBuffer(queue, (BUFFER * 2 * 4) as u32, &mut b) == 0 && !b.is_null() {
                refill(Arc::as_ptr(&feed) as *mut c_void, queue, b);
            }
        }
        AudioQueueStart(queue, std::ptr::null());
    }
    *lock(&PLAYER) = Some(Player { queue, feed, from });
}

/// Stops the sound, if it plays.
pub fn stop() {
    let Some(p) = lock(&PLAYER).take() else { return };
    p.feed.stop.store(true, Ordering::Relaxed);
    unsafe {
        AudioQueueStop(p.queue, 1);
        AudioQueueDispose(p.queue, 1);
    }
}

/// The composition time the sound has reached, while it plays.
pub fn clock() -> Option<f64> {
    let g = lock(&PLAYER);
    let p = g.as_ref()?;
    let mut ts = TimeStamp::default();
    let ok = unsafe { AudioQueueGetCurrentTime(p.queue, std::ptr::null_mut(), &mut ts, std::ptr::null_mut()) } == 0;
    (ok && ts.flags & 1 != 0).then(|| p.from + ts.sample_time.max(0.0) / RATE as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_structures_are_laid_out_as_it_reads_them() {
        assert_eq!(std::mem::size_of::<Format>(), 40);
        assert_eq!(std::mem::size_of::<TimeStamp>(), 64);
        assert_eq!(std::mem::size_of::<Buffer>(), 56);
        assert_eq!(u32::from_be_bytes(*b"lpcm"), 0x6c70_636d);
    }
}
