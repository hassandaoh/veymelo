use super::*;

fn scratch(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("veymelo-os-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name).to_string_lossy().into_owned()
}

/// A 3 × 2 picture whose pixels are numbered 0 to 5 in their red.
fn numbered() -> Picture {
    Picture { width: 3, height: 2, px: (0..6).map(|i| [i as f32, 0.0, 0.0, 1.0]).collect(), alpha: false }
}

fn reds(p: &Picture) -> Vec<u8> {
    p.px.iter().map(|q| q[0] as u8).collect()
}

#[test]
fn every_turn_a_camera_writes() {
    // 0 1 2
    // 3 4 5
    let want: [(u8, (usize, usize), [u8; 6]); 8] = [
        (1, (3, 2), [0, 1, 2, 3, 4, 5]),
        (2, (3, 2), [2, 1, 0, 5, 4, 3]),
        (3, (3, 2), [5, 4, 3, 2, 1, 0]),
        (4, (3, 2), [3, 4, 5, 0, 1, 2]),
        (5, (2, 3), [0, 3, 1, 4, 2, 5]),
        (6, (2, 3), [3, 0, 4, 1, 5, 2]),
        (7, (2, 3), [5, 2, 4, 1, 3, 0]),
        (8, (2, 3), [2, 5, 1, 4, 0, 3]),
    ];
    for (o, size, px) in want {
        let t = turned(numbered(), o);
        assert_eq!(((t.width, t.height), reds(&t)), (size, px.to_vec()), "orientation {o}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn pictures_through_the_system() {
    // straight in, premultiplied out
    let (w, h) = (4, 3);
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            rgba.extend([(x * 80) as u8, (y * 100) as u8, 200, if x == 0 { 128 } else { 255 }]);
        }
    }
    let tif = scratch("p.tif");
    write_picture(&tif, w, h, &rgba, "public.tiff", true, 1.0).unwrap();
    let info = picture_info(&tif).unwrap();
    assert_eq!((info.width, info.height, info.frames, info.alpha), (4, 3, 1, true));
    let p = read_picture(&tif, 0).unwrap();
    assert_eq!((p.width, p.height), (4, 3));
    let at = |x: usize, y: usize| p.px[y * 4 + x];
    let q = at(2, 1);
    assert!((q[0] - 160.0 / 255.0).abs() < 2e-3 && (q[1] - 100.0 / 255.0).abs() < 2e-3 && (q[3] - 1.0).abs() < 1e-6, "{q:?}");
    let half = at(0, 2);
    assert!((half[3] - 128.0 / 255.0).abs() < 2e-3 && (half[2] - 200.0 / 255.0 * half[3]).abs() < 3e-3, "premultiplied: {half:?}");
    let jpg = scratch("p.jpg");
    write_picture(&jpg, w, h, &rgba, "public.jpeg", false, 0.9).unwrap();
    let j = picture_info(&jpg).unwrap();
    assert_eq!((j.width, j.alpha, j.kind.as_str()), (4, false, "public.jpeg"));
    assert!(read_picture(&scratch("none.png"), 0).unwrap_err().contains("no file"));
    std::fs::write(scratch("junk.png"), b"not a picture").unwrap();
    assert!(read_picture(&scratch("junk.png"), 0).is_err());
    // the same from its bytes in memory
    let b = read_picture_bytes(&std::fs::read(&tif).unwrap()).unwrap();
    assert_eq!(b, p);
    assert!(read_picture_bytes(b"not a picture").is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn video_and_sound_both_ways() {
    // a second of colour bars that move, with a 440 Hz tone
    let (w, h, fps, rate) = (64usize, 48usize, 10, 48_000u32);
    let path = scratch("av.mp4");
    say_exceptions();
    let mut v = Video::new(&path, w, h, fps, 1, Codec::H264, 0.9, 0, 0, Some(Sound { rate, channels: 2, aac_bitrate: 192_000 })).unwrap();
    for f in 0..10 {
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let red = if x < 32 { 255 } else { 0 };
                let i = (y * w + x) * 4;
                rgba[i..i + 4].copy_from_slice(&[red, (f * 25) as u8, 0, 255]);
            }
        }
        v.frame(&rgba).unwrap();
        let n = rate as usize / fps as usize;
        let tone: Vec<f32> = (0..n).flat_map(|k| {
            let t = (f as usize * n + k) as f64 / rate as f64;
            let s = (std::f64::consts::TAU * 440.0 * t).sin() as f32 * 0.5;
            [s, s]
        }).collect();
        v.sound(&tone).unwrap();
    }
    v.finish().unwrap();
    let info = media_info(&path).unwrap();
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height, video.codec.as_str()), (64, 48, "avc1"));
    assert!((video.fps - 10.0).abs() < 0.01 && (info.duration - 1.0).abs() < 0.06, "{info:?}");
    let sound = info.sound.as_ref().unwrap();
    assert_eq!((sound.rate, sound.channels, sound.codec.as_str()), (48_000.0, 2, "aac"));
    // the frames, in order, from the middle
    let mut frames = Frames::open(&path, 0.5).unwrap();
    let mut seen = Vec::new();
    while let Some(f) = frames.next_frame() {
        let (t, p) = f.unwrap();
        seen.push((t, p.px[24 * 64 + 10], p.px[24 * 64 + 50]));
    }
    assert!((seen[0].0 - 0.5).abs() < 0.051, "the frame showing at 0.5 s first: {:?}", seen.iter().map(|s| s.0).collect::<Vec<_>>());
    assert_eq!(seen.len(), 5);
    let (_, left, right) = seen[0];
    assert!(left[0] > 0.9 && right[0] < 0.1 && (left[1] - 125.0 / 255.0).abs() < 0.05, "{left:?} {right:?}");
    // the sound, at another rate
    let stereo = read_sound(&path, 16_000, 2).unwrap();
    let s: Vec<f32> = stereo.iter().step_by(2).copied().collect();
    assert!((s.len() as f64 / 16_000.0 - 1.0).abs() < 0.06, "{} samples", s.len());
    let mid = &s[4000..12000];
    let crossings = mid.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    assert!((crossings as i64 - 220).abs() <= 2, "440 Hz over half a second: {crossings}");
    let peak = mid.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!((peak - 0.5).abs() < 0.05, "{peak}");
    // mono: the system mixes the two channels a third quieter each (equal power)
    let mono = read_sound(&path, 16_000, 1).unwrap();
    let peak = mono[4000..12000].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!((mono.len() as i64 - stereo.len() as i64 / 2).abs() < 2 && (peak - 0.5 * std::f32::consts::SQRT_2).abs() < 0.05, "{peak}");
    assert!(media_info(&scratch("none.mov")).unwrap_err().contains("no file"));
}

#[cfg(target_os = "macos")]
#[test]
fn sound_in_every_kind_of_file() {
    // QuickTime with ProRes and AAC or PCM, MP4 with H.264 and AAC; pictures and sound given
    // one after the other, as an exporter gives them
    for (name, codec, aac, kind) in [("prores_aac.mov", Codec::ProRes(2), 128_000u32, "aac"), ("prores_pcm.mov", Codec::ProRes(2), 0, "lpcm"), ("h264.mp4", Codec::H264, 0, "aac")] {
        let path = scratch(name);
        let mut v = Video::new(&path, 32, 16, 10, 1, codec, 1.0, 0, 0, Some(Sound { rate: 48_000, channels: 2, aac_bitrate: aac })).unwrap();
        for _ in 0..20 {
            v.frame(&vec![100u8; 32 * 16 * 4]).unwrap();
            v.sound(&vec![0.1f32; 9600]).unwrap();
        }
        v.finish().unwrap_or_else(|e| panic!("{name}: {e}"));
        let info = media_info(&path).unwrap();
        assert_eq!(info.sound.map(|s| s.codec), Some(kind.to_string()), "{name}");
        assert!((info.duration - 2.0).abs() < 0.05, "{name}: {}", info.duration);
    }
}
