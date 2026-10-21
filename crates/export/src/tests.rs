use super::*;
use veymelo_color::Rgba;
use veymelo_project::queue::{OutputModule, ProRes};
use veymelo_project::{Layer, Source, Value};

/// A 64 × 36 composition, 1 second at 10 frames a second, a white square moving across a
/// see-through background.
fn project() -> (Project, u64) {
    let mut p = Project::new();
    let mut comp = Comp::new(64, 36, FrameRate::new(10, 1).unwrap(), Tick::from_seconds(1.0));
    comp.background = Rgba::BLACK;
    let id = p.add_comp("Main", comp);
    let mut l = Layer::new(p.alloc(), "Square", Source::Solid { color: Rgba::WHITE, width: 10, height: 10 }, Tick::from_seconds(1.0));
    l.props.prop_mut("transform/anchor").unwrap().set_value(Value::Vec3([5.0, 5.0, 0.0])).unwrap();
    let keys = vec![veymelo_project::Keyframe::new(Tick::ZERO, Value::Vec3([5.0, 18.0, 0.0])), veymelo_project::Keyframe::new(Tick::from_seconds(1.0), Value::Vec3([59.0, 18.0, 0.0]))];
    l.props.prop_mut("transform/position").unwrap().set_keys(keys).unwrap();
    p.comp_mut(id).unwrap().layers.push(l);
    (p, id)
}

fn scratch(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("veymelo-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name).to_string_lossy().into_owned()
}

fn item(comp: u64, format: Format, path: &str) -> RenderItem {
    let mut it = RenderItem::new(1, comp);
    it.output = OutputModule { format, path: path.into(), ..OutputModule::default() };
    it
}

#[test]
fn the_frames_of_a_span() {
    let (p, c) = project();
    let comp = p.comp(c).unwrap();
    let (rate, times) = frame_times(comp, &RenderSettings { span: Span::Comp, ..RenderSettings::default() }).unwrap();
    assert_eq!((rate.as_f64(), times.len()), (10.0, 10));
    assert_eq!(times[3], Tick::from_seconds(0.3));
    let (_, half) = frame_times(comp, &RenderSettings { span: Span::Custom, start: Tick::from_seconds(0.25), end: Tick::from_seconds(0.6), ..RenderSettings::default() }).unwrap();
    assert_eq!(half, vec![Tick::from_seconds(0.3), Tick::from_seconds(0.4), Tick::from_seconds(0.5)], "the frames that start in it");
    let (slow, fewer) = frame_times(comp, &RenderSettings { span: Span::Comp, frame_rate: Some(5.0), ..RenderSettings::default() }).unwrap();
    assert_eq!((slow.as_f64(), fewer.len()), (5.0, 5));
    assert!(frame_times(comp, &RenderSettings { span: Span::Custom, start: Tick::from_seconds(0.5), end: Tick::from_seconds(0.5), ..RenderSettings::default() }).is_err());
}

#[test]
fn cut_sized_and_channelled() {
    let mut img = Image::new(4, 2);
    img.px[0] = [1.0, 0.0, 0.0, 1.0];
    let c = crop(&img, [0, 1, 0, 1]);
    assert_eq!((c.width, c.height), (2, 2));
    let wider = crop(&img, [0, -1, 0, 0]);
    assert_eq!((wider.width, wider.px[1]), (5, [1.0, 0.0, 0.0, 1.0]), "added on the left, clear");
    // halved: the mean of each two by two
    let mut big = Image::new(4, 4);
    for (i, p) in big.px.iter_mut().enumerate() {
        let v = if (i % 4) % 2 == 0 { 1.0 } else { 0.0 };
        *p = [v, v, v, 1.0];
    }
    let small = resize(&big, 2, 2);
    assert!(small.px.iter().all(|p| (p[0] - 0.5).abs() < 1e-6));
    // half see-through red: straight, multiplied, or its alpha as grey
    let half = Image { width: 1, height: 1, px: vec![[0.5, 0.0, 0.0, 0.5]].into() };
    assert_eq!(channels(&half, Channels::Rgba, false), vec![255, 0, 0, 128]);
    assert_eq!(channels(&half, Channels::Rgba, true), vec![128, 0, 0, 128]);
    assert_eq!(channels(&half, Channels::Alpha, false), vec![128, 128, 128, 255]);
    assert_eq!(channels(&half, Channels::Rgb, false)[3], 255);
}

#[test]
fn a_png_sequence() {
    let (p, c) = project();
    let path = scratch("seq_###.png");
    let mut seen = Vec::new();
    let made = make(&p, &item(c, Format::PngSequence, &path), &mut |done, total| {
        seen.push((done, total));
        true
    })
    .unwrap();
    assert_eq!((made.frames, made.files.len(), made.size), (10, 10, [64, 36]), "the work area: all of it at first");
    assert_eq!(seen.last(), Some(&(10, 10)));
    assert!(made.files[2].ends_with("seq_002.png"));
    let first = std::fs::read(&made.files[0]).unwrap();
    assert_eq!(&first[1..4], b"PNG");
}

#[test]
fn a_gif_reads_back() {
    let (p, c) = project();
    let path = scratch("moving.gif");
    let mut it = item(c, Format::Gif, &path);
    it.settings.span = Span::Comp;
    it.output.resize = Some([32, 18]);
    let made = make(&p, &it, &mut |_, _| true).unwrap();
    assert_eq!((made.frames, made.size), (10, [32, 18]));
    let file = std::fs::read(&path).unwrap();
    assert_eq!(&file[..6], b"GIF89a");
    assert_eq!(u16::from_le_bytes([file[6], file[7]]), 32);
    let frames = file.windows(3).filter(|w| w[0] == 0x21 && w[1] == 0xF9 && w[2] == 4).count();
    assert_eq!(frames, 10);
}

#[test]
fn stopping_leaves_no_half_file() {
    let (p, c) = project();
    let path = scratch("stopped.gif");
    let err = make(&p, &item(c, Format::Gif, &path), &mut |done, _| done < 2).unwrap_err();
    assert!(err.contains("stopped after 2"), "{err}");
    assert!(!std::path::Path::new(&path).exists());
}

#[cfg(target_os = "macos")]
#[test]
fn videos_by_the_system() {
    let (p, c) = project();
    for (format, name) in [(Format::Mp4, "clip.mp4"), (Format::Hevc, "clip_hevc.mp4")] {
        let path = scratch(name);
        let made = make(&p, &item(c, format, &path), &mut |_, _| true).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(made.files, vec![path.clone()]);
        let file = std::fs::read(&path).unwrap();
        assert!(file.len() > 200 && &file[4..8] == b"ftyp", "{name}: an MP4 file");
    }
    // ProRes 4444 keeps the see-through parts
    let path = scratch("clip.mov");
    let mut it = item(c, Format::ProRes, &path);
    it.output.prores = ProRes::P4444;
    it.output.channels = Channels::Rgba;
    let made = make(&p, &it, &mut |_, _| true).unwrap();
    assert!(made.notes.iter().all(|n| !n.contains("alpha")), "{:?}", made.notes);
    let file = std::fs::read(&path).unwrap();
    assert!(file.windows(4).any(|w| w == b"ap4h"), "ProRes 4444 inside");
    // an odd size is made even
    let mut odd = Project::new();
    let oc = odd.add_comp("Odd", Comp::new(33, 21, FrameRate::new(10, 1).unwrap(), Tick::from_seconds(0.3)));
    let made = make(&odd, &item(oc, Format::Mp4, &scratch("odd.mp4")), &mut |_, _| true).unwrap();
    assert_eq!(made.size, [32, 20]);
}

#[cfg(target_os = "macos")]
#[test]
fn jpeg_and_tiff_by_the_system() {
    let (p, c) = project();
    for (format, name, magic) in [(Format::JpegSequence, "f_##.jpg", &[0xFFu8, 0xD8][..]), (Format::TiffSequence, "f_##.tif", &b"MM"[..])] {
        let made = make(&p, &item(c, format, &scratch(name)), &mut |_, _| true).unwrap();
        let file = std::fs::read(&made.files[0]).unwrap();
        assert!(file.starts_with(magic) || (format == Format::TiffSequence && file.starts_with(b"II")), "{name}");
    }
}

/// The test composition with a 1 kHz tone (a solid with Tone).
fn with_tone() -> (Project, u64) {
    let (mut p, id) = project();
    let mut l = Layer::new(p.alloc(), "Beep", Source::Solid { color: Rgba::BLACK, width: 2, height: 2 }, Tick::from_seconds(1.0));
    let mut g = veymelo_project::Group::instance(veymelo_effects::find("tone").unwrap().def, p.alloc(), "Tone");
    g.prop_mut("frequency1").unwrap().set_value(Value::Number(1000.0)).unwrap();
    for i in 2..=5 {
        g.prop_mut(&format!("frequency{i}")).unwrap().set_value(Value::Number(0.0)).unwrap();
    }
    l.props.group_mut("effects").unwrap().children.push(veymelo_project::Node::Group(g));
    p.comp_mut(id).unwrap().layers.push(l);
    (p, id)
}

#[test]
fn a_wav_of_the_sound() {
    let (p, c) = with_tone();
    let path = scratch("tone.wav");
    let mut it = item(c, Format::Wav, &path);
    it.settings.span = Span::Comp;
    it.output.audio_bits = 16;
    let made = make(&p, &it, &mut |_, _| true).unwrap();
    assert_eq!(made.frames, 48_000, "a second at 48 kHz");
    let file = std::fs::read(&path).unwrap();
    assert_eq!((&file[..4], &file[8..16]), (&b"RIFF"[..], &b"WAVEfmt "[..]));
    assert_eq!(u32::from_le_bytes(file[24..28].try_into().unwrap()), 48_000);
    assert_eq!(file.len(), 44 + 48_000 * 2 * 2);
    let left: Vec<f32> = file[44..].chunks(4).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32767.0).collect();
    let ups = left.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    let peak = left.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!((ups as i64 - 1000).abs() <= 1 && (peak - 0.2).abs() < 1e-3, "{ups} {peak}");
    // mono, floating point
    it.output.audio_channels = 1;
    it.output.audio_bits = 32;
    make(&p, &it, &mut |_, _| true).unwrap();
    assert_eq!(std::fs::read(&path).unwrap().len(), 44 + 48_000 * 4);
    // a composition with nothing heard: silent, and said so
    let (q, d) = project();
    let made = make(&q, &item(d, Format::Wav, &scratch("silent.wav")), &mut |_, _| true).unwrap();
    assert!(made.notes.iter().any(|n| n.contains("no sound")));
}

#[cfg(target_os = "macos")]
#[test]
fn videos_carry_the_sound() {
    let (p, c) = with_tone();
    for (format, name, codec) in [(Format::Mp4, "beep.mp4", "aac"), (Format::ProRes, "beep.mov", "lpcm")] {
        let path = scratch(name);
        make(&p, &item(c, format, &path), &mut |_, _| true).unwrap();
        let info = veymelo_os::media_info(&path).unwrap();
        assert_eq!(info.sound.as_ref().map(|s| s.codec.as_str()), Some(codec), "{name}");
        let s = veymelo_os::read_sound(&path, 48_000, 2).unwrap();
        let left: Vec<f32> = s.iter().step_by(2).copied().collect();
        assert!((left.len() as i64 - 48_000).abs() < 2400, "{name}: {}", left.len());
        let mid = &left[12_000..36_000];
        let ups = mid.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!((ups as i64 - 500).abs() <= 2, "{name}: {ups}");
    }
    // the sound left out when asked
    let path = scratch("quiet.mp4");
    let mut it = item(c, Format::Mp4, &path);
    it.output.audio = false;
    make(&p, &it, &mut |_, _| true).unwrap();
    assert!(veymelo_os::media_info(&path).unwrap().sound.is_none());
}

#[test]
fn frames_drawn_at_once_are_written_in_their_order() {
    // drawn out of order (later frames quicker), written in order, each once
    let draw = |k: usize| -> Result<usize, String> {
        std::thread::sleep(std::time::Duration::from_micros(((40 - k % 40) * 50) as u64));
        Ok(k * 10)
    };
    let mut seen = Vec::new();
    assert_eq!(in_order(200, 8, &draw, &mut |k, d| {
        seen.push((k, d));
        Ok(true)
    }), Ok(None));
    assert_eq!(seen, (0..200).map(|k| (k, k * 10)).collect::<Vec<_>>());
    // stopped: how many were written
    let mut n = 0;
    assert_eq!(in_order(200, 8, &draw, &mut |_, _| {
        n += 1;
        Ok(n < 37)
    }), Ok(Some(37)));
    // a frame that cannot be drawn ends it, with why
    let bad = |k: usize| if k == 50 { Err("no".to_string()) } else { Ok(k) };
    let mut last = 0;
    assert_eq!(in_order(200, 8, &bad, &mut |k, _| {
        last = k;
        Ok(true)
    }), Err("no".to_string()));
    assert_eq!(last, 49, "every frame before it was written");
}
