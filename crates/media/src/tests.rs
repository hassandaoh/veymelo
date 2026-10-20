use super::*;

fn scratch(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("veymelo-media-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name).to_string_lossy().into_owned()
}

/// A `w` × `h` TIFF of one colour (straight RGBA).
fn tiff(path: &str, w: usize, h: usize, rgba: [u8; 4]) {
    let px: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
    veymelo_os::write_picture(path, w, h, &px, "public.tiff", true, 1.0).unwrap();
}

#[test]
fn which_frame_shows_when() {
    let mut f = Footage::new("x.mov", FootageKind::Video);
    (f.frame_rate, f.duration) = (FrameRate::new(10, 1).unwrap(), Tick::from_seconds(1.0));
    assert_eq!(frame_at(&f, 0.0), Some((0, 0.0)));
    let (n, k) = frame_at(&f, 0.35).unwrap();
    assert!(n == 3 && (k - 0.5).abs() < 1e-3);
    assert_eq!(frame_at(&f, 0.999).map(|x| x.0), Some(9));
    assert_eq!(frame_at(&f, 1.0), None, "after it ends");
    assert_eq!(frame_at(&f, -0.1), None);
    f.loops = 3;
    assert_eq!(frame_at(&f, 2.5).map(|x| x.0), Some(5), "the third time through");
    assert_eq!(frame_at(&Footage::new("a.png", FootageKind::Image), 99.0), Some((0, 0.0)));
}

#[cfg(target_os = "macos")]
#[test]
fn stills_and_how_they_are_taken() {
    let path = scratch("half.tif");
    tiff(&path, 4, 2, [200, 100, 0, 128]);
    let f = probe(&path, false).unwrap();
    assert_eq!((f.kind, f.width, f.height, f.alpha), (FootageKind::Image, 4, 2, AlphaMode::Straight));
    let img = frame(&f, 0).unwrap();
    let a = 128.0 / 255.0;
    assert!((img.px[0][3] - a).abs() < 2e-3 && (img.px[0][0] - 200.0 / 255.0 * a).abs() < 3e-3, "premultiplied: {:?}", img.px[0]);
    // alpha left out: the colours as they are, solid
    let solid = Footage { alpha: AlphaMode::Ignore, ..f.clone() };
    let s = frame(&solid, 0).unwrap();
    assert!((s.px[0][3] - 1.0).abs() < 1e-6 && (s.px[0][0] - 200.0 / 255.0).abs() < 3e-3, "{:?}", s.px[0]);
    // pixels twice as wide as tall: twice as many square ones across
    let wide = Footage { pixel_aspect: 2.0, ..f.clone() };
    let w = frame(&wide, 0).unwrap();
    assert_eq!((w.width, w.height), (8, 2));
    // kept: asked again, not decoded again
    let before = stats().decoded;
    frame(&f, 0).unwrap();
    assert_eq!(stats().decoded, before);
    assert!(probe(&scratch("nothing.png"), false).is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn numbered_pictures_are_a_sequence() {
    let dir = scratch("seq");
    std::fs::create_dir_all(&dir).unwrap();
    for i in 7..12u8 {
        tiff(&format!("{dir}/shot_{i:03}.tif"), 2, 2, [i * 20, 0, 0, 255]);
    }
    std::fs::write(format!("{dir}/shot_notes.txt"), "not a frame").unwrap();
    let (pattern, start, count) = find_sequence(&format!("{dir}/shot_009.tif")).unwrap();
    assert_eq!((pattern, start, count), (format!("{dir}/shot_###.tif"), 7, 5));
    let f = probe(&format!("{dir}/shot_007.tif"), true).unwrap();
    assert_eq!((f.kind, f.start, f.duration), (FootageKind::Sequence, 7, SEQUENCE_RATE.tick_of(5)));
    assert_eq!(f.frame_path(2), format!("{dir}/shot_009.tif"));
    let third = frame(&f, 2).unwrap();
    assert!((third.px[0][0] - 180.0 / 255.0).abs() < 3e-3, "{:?}", third.px[0]);
    // one picture alone is not a sequence
    let lone = scratch("lone_5.tif");
    tiff(&lone, 2, 2, [0, 0, 0, 255]);
    assert_eq!(probe(&lone, true).unwrap().kind, FootageKind::Image);
}

#[cfg(target_os = "macos")]
#[test]
fn a_video_frame_by_frame_and_its_sound() {
    use veymelo_os::{Codec, Sound, Video};
    let (w, h, rate) = (32usize, 16usize, 8000u32);
    let path = scratch("count.mov");
    let mut v = Video::new(&path, w, h, 10, 1, Codec::ProRes(2), 1.0, 0, 0, Some(Sound { rate, channels: 2, aac_bitrate: 0 })).unwrap();
    for f in 0..20u8 {
        // each frame a grey as bright as its number
        let rgba: Vec<u8> = (0..w * h).flat_map(|_| [f * 12, f * 12, f * 12, 255]).collect();
        v.frame(&rgba).unwrap();
        let n = rate as usize / 10;
        // left: the frame's number in hundredths, right: silent
        let s: Vec<f32> = (0..n).flat_map(|_| [f as f32 / 100.0, 0.0]).collect();
        v.sound(&s).unwrap();
    }
    v.finish().unwrap();
    let f = probe(&path, false).unwrap();
    assert_eq!((f.kind, f.width, f.height, f.frame_rate, f.has_audio), (FootageKind::Video, 32, 16, FrameRate::new(10, 1).unwrap(), true));
    assert!((f.duration.seconds() - 2.0).abs() < 0.01, "{}", f.duration.seconds());
    let grey = |n: u64| frame(&f, n).unwrap().px[8 * 32 + 16][1] * 255.0;
    // in order, then back, then far ahead
    for n in [0u64, 1, 2, 3, 12, 5, 19] {
        assert!((grey(n) - n as f32 * 12.0).abs() < 2.5, "frame {n}: {}", grey(n));
    }
    let s = stats();
    assert!(s.readers >= 1 && s.decoded >= 7);
    // the sound where frame 13 is
    let heard = sound(&f, 1.3 + 0.05, 10, rate).unwrap();
    assert!((heard[0] - 0.13).abs() < 1e-3 && heard[1].abs() < 1e-6, "{:?}", &heard[..4]);
    let before = sound(&f, -1.0, 4, rate).unwrap();
    assert!(before.iter().all(|v| *v == 0.0), "silent before it");
    forget(Some(&path));
    assert!(frame(&f, 4).is_ok(), "read again after it was let go");
    // started fresh in the middle (the reader begins partway into the frame before), then on
    forget(Some(&path));
    for n in [7u64, 6, 8, 9, 10] {
        assert!((grey(n) - n as f32 * 12.0).abs() < 2.5, "frame {n} after starting at 7: {}", grey(n));
    }
}

#[cfg(target_os = "macos")]
#[test]
fn cuts_between_shots_found() {
    use veymelo_os::{Codec, Video};
    let (w, h) = (64usize, 36usize);
    let path = scratch("shots.mov");
    let mut v = Video::new(&path, w, h, 10, 1, Codec::ProRes(2), 1.0, 0, 0, None).unwrap();
    // three shots: a square sweeping fast over blue, a ball over orange, a slow gradient
    for f in 0..30usize {
        let mut px = vec![[0u8; 4]; w * h];
        for y in 0..h {
            for x in 0..w {
                px[y * w + x] = match f {
                    0..12 => {
                        let sx = f * 4;
                        if (sx..sx + 10).contains(&x) && (12..24).contains(&y) { [250, 250, 250, 255] } else { [20, 40, 160, 255] }
                    }
                    12..22 => {
                        let (cx, cy) = (10.0 + (f - 12) as f64 * 4.0, 18.0);
                        if ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt() < 7.0 { [30, 20, 10, 255] } else { [230, 140, 40, 255] }
                    }
                    _ => {
                        let g = (x * 3 + f) as u8;
                        [g / 2 + 60, g / 2 + 90, 70, 255]
                    }
                };
            }
        }
        let rgba: Vec<u8> = px.into_iter().flatten().collect();
        v.frame(&rgba).unwrap();
    }
    v.finish().unwrap();
    let f = probe(&path, false).unwrap();
    let mut seen = 0;
    let cuts = scene_cuts(&f, 50.0, &mut |n, _| {
        seen = n;
        true
    })
    .unwrap();
    let frames: Vec<u64> = cuts.iter().map(|c| c.frame).collect();
    assert_eq!(frames, vec![12, 22], "{cuts:?}");
    assert!((cuts[0].at - 1.2).abs() < 1e-9 && cuts[0].strength > 0.3);
    assert_eq!(seen, 30);
    // the fast square alone is one shot
    let one = scratch("one.mov");
    let mut v = Video::new(&one, w, h, 10, 1, Codec::ProRes(2), 1.0, 0, 0, None).unwrap();
    for f in 0..14usize {
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| if (f * 4..f * 4 + 10).contains(&(i % w)) { [250, 250, 250, 255] } else { [20, 40, 160, 255] }).collect();
        v.frame(&rgba).unwrap();
    }
    v.finish().unwrap();
    assert!(scene_cuts(&probe(&one, false).unwrap(), 50.0, &mut |_, _| true).unwrap().is_empty());
}

/// OpenEXR pictures read by our own decoder as the system reads them (the system's own files:
/// one with alpha, taken as straight as the system takes it; one in PXR24), and one in DWAA,
/// which the system does not read.
#[cfg(target_os = "macos")]
#[test]
fn openexr_pictures_as_the_system_reads_them() {
    let files = [
        "/System/Library/PrivateFrameworks/CoreRE.framework/Versions/A/Resources/twinkle.exr",
        "/System/Library/PrivateFrameworks/CoreRE.framework/Versions/A/Resources/spatial_media_4channel_map_sq.exr",
    ];
    for path in files.iter().filter(|p| std::path::Path::new(p).exists()) {
        let mut f = probe(path, false).unwrap();
        f.alpha = if f.alpha == AlphaMode::Ignore { f.alpha } else { AlphaMode::Straight };
        let ours = frame(&f, 0).unwrap();
        let sys = veymelo_os::read_picture(path, 0).unwrap();
        assert_eq!((ours.width, ours.height), (sys.width, sys.height));
        let most = ours.px.iter().zip(&sys.px).flat_map(|(a, b)| (0..4).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
        assert!(most < 1e-4, "{path}: {most}");
    }
    // a file of OpenEXR's own, DWAA: premultiplied, as the format has it
    let path = format!("{}/../exr/fixtures/render-dwaa.exr", env!("CARGO_MANIFEST_DIR"));
    let f = probe(&path, false).unwrap();
    assert_eq!((f.width, f.height, f.alpha), (48, 32, AlphaMode::Premultiplied));
    let img = frame(&f, 0).unwrap();
    assert!(img.px.iter().any(|p| p[3] == 0.0) && img.px.iter().any(|p| p[3] == 1.0 && p[2] > p[0]), "sky see-through, the ball blue");
    let p = passes(&path).unwrap();
    assert_eq!((p.width, p.height), (48, 32));
    assert!(p.depth().is_some() && p.object_id().is_some() && !p.crypto_ranks("ViewLayer.CryptoObject").is_empty());
}
