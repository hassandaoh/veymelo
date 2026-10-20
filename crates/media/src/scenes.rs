//! Where one shot of a video ends and the next begins (scene edits). Each frame is boiled down
//! to a small grid of brightness and a histogram of each colour; the change from one frame to
//! the next is how far both moved. A cut is a change well above the changes between the frames
//! around it, so steady motion (a pan, someone walking) is not taken for one.

use veymelo_project::{Footage, FootageKind};

/// A cut: the first frame of the new shot (the file's own frame), when it shows (seconds into
/// the file) and how sharp the change is, 0 to 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    pub frame: u64,
    pub at: f64,
    pub strength: f64,
}

const GW: usize = 32;
const GH: usize = 18;
const BINS: usize = 16;
/// Frames on each side whose changes a change is weighed against.
const AROUND: usize = 8;

/// What a frame is boiled down to.
struct Seen {
    grid: Vec<f32>,
    hist: [[f32; BINS]; 3],
}

fn seen(w: usize, h: usize, px: &[[f32; 4]]) -> Seen {
    let mut grid = vec![0.0f32; GW * GH];
    let mut count = vec![0u32; GW * GH];
    let mut hist = [[0.0f32; BINS]; 3];
    // about 40 000 pixels looked at, whatever the size
    let step = ((w * h) as f64 / 40_000.0).sqrt().floor().max(1.0) as usize;
    let mut n = 0.0f32;
    for y in (0..h).step_by(step) {
        let gy = y * GH / h;
        for x in (0..w).step_by(step) {
            let p = px[y * w + x];
            let c = [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0), p[2].clamp(0.0, 1.0)];
            let cell = gy * GW + x * GW / w;
            grid[cell] += 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
            count[cell] += 1;
            for k in 0..3 {
                hist[k][((c[k] * BINS as f32) as usize).min(BINS - 1)] += 1.0;
            }
            n += 1.0;
        }
    }
    for (g, c) in grid.iter_mut().zip(&count) {
        if *c > 0 {
            *g /= *c as f32;
        }
    }
    for ch in &mut hist {
        ch.iter_mut().for_each(|v| *v /= n.max(1.0));
    }
    Seen { grid, hist }
}

/// How much changed between two frames, 0 to 1.
fn change(a: &Seen, b: &Seen) -> f64 {
    let colours: f32 = (0..3).map(|k| a.hist[k].iter().zip(&b.hist[k]).map(|(p, q)| (p - q).abs()).sum::<f32>() / 2.0).sum::<f32>() / 3.0;
    let shapes: f32 = a.grid.iter().zip(&b.grid).map(|(p, q)| (p - q).abs()).sum::<f32>() / a.grid.len() as f32;
    ((colours + shapes) / 2.0) as f64
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let m = v.len() / 2;
    if v.len() % 2 == 1 { v[m] } else { (v[m - 1] + v[m]) / 2.0 }
}

/// The cuts in `changes` (`changes[i]`: from frame i - 1 to frame i; `changes[0]` unused):
/// sensitivity 0 (only the starkest) to 100 (the faintest), shots at least `shortest` frames.
fn cuts_in(changes: &[f64], sensitivity: f64, shortest: usize) -> Vec<(usize, f64)> {
    let s = sensitivity.clamp(0.0, 100.0);
    let least = 0.35 - 0.003 * s;
    let above = 4.0 - 0.02 * s;
    let mut out: Vec<(usize, f64)> = Vec::new();
    for i in 1..changes.len() {
        let d = changes[i];
        if d < least {
            continue;
        }
        let around: Vec<f64> = (i.saturating_sub(AROUND).max(1)..(i + AROUND + 1).min(changes.len())).filter(|&j| j != i).map(|j| changes[j]).collect();
        if d < above * median(around) + 0.02 {
            continue;
        }
        match out.last_mut() {
            // too soon after the last: the sharper of the two
            Some(last) if i - last.0 < shortest => {
                if d > last.1 {
                    *last = (i, d);
                }
            }
            _ => out.push((i, d)),
        }
    }
    out
}

/// The scene edits in footage `f`: every cut between shots, sensitivity 0 to 100 (50 is
/// usual). `going` hears frames looked at of how many, and stops it by saying false.
pub fn scene_cuts(f: &Footage, sensitivity: f64, going: &mut dyn FnMut(u64, u64) -> bool) -> Result<Vec<Cut>, String> {
    let fps = f.native_rate.unwrap_or(f.frame_rate).as_f64();
    let total = (f.duration.seconds() * f.frame_rate.as_f64()).round().max(0.0) as u64;
    let mut looks: Vec<(u64, Seen)> = Vec::new();
    match f.kind {
        FootageKind::Image | FootageKind::Audio | FootageKind::Placeholder | FootageKind::Model => return Ok(Vec::new()),
        FootageKind::Sequence => {
            for n in 0..total {
                let p = crate::read_picture(&f.frame_path(n), 0)?;
                looks.push((n, seen(p.width, p.height, &p.px)));
                if !going(n + 1, total) {
                    return Err("stopped".into());
                }
            }
        }
        FootageKind::Video => {
            if crate::PICTURES.contains(&crate::ext_of(&f.path).as_str()) {
                // an animated picture: its frames as they are
                let info = crate::picture_info(&f.path)?;
                for n in 0..info.frames {
                    let p = crate::read_picture(&f.path, n)?;
                    looks.push((n as u64, seen(p.width, p.height, &p.px)));
                }
            } else {
                read_video(f, fps, total, &mut looks, going)?;
            }
        }
    }
    let changes: Vec<f64> = (0..looks.len()).map(|i| if i == 0 { 0.0 } else { change(&looks[i - 1].1, &looks[i].1) }).collect();
    let shortest = ((fps * 0.25).round() as usize).max(2);
    Ok(cuts_in(&changes, sensitivity, shortest).into_iter().map(|(i, d)| Cut { frame: looks[i].0, at: looks[i].0 as f64 / fps, strength: (d * 1000.0).round() / 1000.0 }).collect())
}

#[cfg(target_os = "macos")]
fn read_video(f: &Footage, fps: f64, total: u64, looks: &mut Vec<(u64, Seen)>, going: &mut dyn FnMut(u64, u64) -> bool) -> Result<(), String> {
    let mut frames = veymelo_os::Frames::open(&f.path, 0.0)?;
    while let Some(next) = frames.next_frame() {
        let (at, p) = next?;
        let n = (at * fps + 1e-3).floor().max(0.0) as u64;
        if looks.last().is_some_and(|(m, _)| *m >= n) {
            continue;
        }
        looks.push((n, seen(p.width, p.height, &p.px)));
        if !going(n + 1, total) {
            return Err("stopped".into());
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn read_video(f: &Footage, _: f64, _: u64, _: &mut Vec<(u64, Seen)>, _: &mut dyn FnMut(u64, u64) -> bool) -> Result<(), String> {
    Err(format!("{}: video is only read on macOS so far", f.path))
}
