use super::*;

/// Two picture tracks and a sound track, at 25 frames a second.
fn timeline() -> Timeline {
    let media = |audio| Media { length: 20.0, width: 1280, height: 720, video: true, audio };
    let clip = |name: &str, path: &str, at: f64, length: f64, source_in: f64| Clip { name: name.into(), path: path.into(), at, length, source_in, media: media(true) };
    Timeline {
        name: "Cut & Edit".into(),
        rate: Rate { num: 25, den: 1 },
        width: 1280,
        height: 720,
        tracks: vec![
            Track { kind: Kind::Video, clips: vec![clip("Wide", "/shots/wide shot.mov", 0.0, 4.0, 2.0), clip("Close", "/shots/close.mov", 4.0, 3.0, 0.0)] },
            Track { kind: Kind::Video, clips: vec![clip("Title", "/gfx/title.mov", 1.0, 2.0, 0.0)] },
            Track { kind: Kind::Audio, clips: vec![clip("Music", "/audio/music.wav", 0.0, 7.0, 10.0)] },
        ],
    }
}

/// Each clip of each track: (kind, name, path, at, length, source in) in frames.
fn shape(t: &Timeline) -> Vec<(Kind, String, String, i64, i64, i64)> {
    t.tracks.iter().flat_map(|k| k.clips.iter().map(move |c| (k.kind, c.name.clone(), c.path.clone(), t.rate.frames(c.at), t.rate.frames(c.length), t.rate.frames(c.source_in)))).collect()
}

#[test]
fn every_format_reads_back_what_it_wrote() {
    let t = timeline();
    for format in [Format::Otio, Format::Fcp7, Format::Fcpxml] {
        let (text, notes) = write(&t, format);
        assert!(notes.is_empty(), "{format:?}: {notes:?}");
        assert_eq!(Format::of_text(&text), Some(format));
        let (back, notes) = read(&text, None, None).unwrap_or_else(|e| panic!("{format:?}: {e}\n{text}"));
        assert!(notes.is_empty(), "{format:?}: {notes:?}");
        assert_eq!(back.rate, t.rate, "{format:?}");
        assert_eq!((back.width, back.height), (1280, 720), "{format:?}");
        assert_eq!(shape(&back), shape(&t), "{format:?}\n{text}");
        assert_eq!(back.name, t.name, "{format:?}");
    }
    // an EDL keeps one picture track: the first, and the sound
    let (text, notes) = write(&t, Format::Edl);
    assert_eq!(notes.len(), 1, "{notes:?}");
    let (back, _) = read(&text, Some(Format::Edl), Some(Rate { num: 25, den: 1 })).unwrap();
    let want: Vec<_> = shape(&t).into_iter().filter(|c| c.1 != "Title").collect();
    assert_eq!(shape(&back), want, "{text}");
}

#[test]
fn files_made_elsewhere() {
    // an EDL that starts at an hour, with a dissolve and a clip with no file named
    let edl = "TITLE: Reel 1\nFCM: NON-DROP FRAME\n\n001  A001     V     C        00:00:05:00 00:00:07:00 01:00:00:00 01:00:02:00\n* FROM CLIP NAME: a.mov\n002  A002     V     D    012 00:00:01:00 00:00:03:00 01:00:02:00 01:00:04:00\n";
    let (t, notes) = read(edl, None, None).unwrap();
    assert_eq!(t.name, "Reel 1");
    assert_eq!(shape(&t).iter().map(|c| (c.1.clone(), c.3, c.4, c.5)).collect::<Vec<_>>(), [("a.mov".to_string(), 0, 60, 150), ("A002".to_string(), 60, 60, 30)]);
    assert!(notes.iter().any(|n| n.contains("dissolve")));
    // OTIO's older clips, with a gap
    let otio = r#"{"OTIO_SCHEMA": "Timeline.1", "name": "Old", "tracks": {"OTIO_SCHEMA": "Stack.1", "children": [{"OTIO_SCHEMA": "Track.1", "kind": "Video", "children": [
        {"OTIO_SCHEMA": "Gap.1", "source_range": {"start_time": {"rate": 24, "value": 0}, "duration": {"rate": 24, "value": 48}}},
        {"OTIO_SCHEMA": "Clip.1", "name": "b", "source_range": {"start_time": {"rate": 24, "value": 100}, "duration": {"rate": 24, "value": 24}},
         "media_reference": {"OTIO_SCHEMA": "ExternalReference.1", "target_url": "file:///media/b%20take.mov", "available_range": {"start_time": {"rate": 24, "value": 86400}, "duration": {"rate": 24, "value": 2400}}}}]}]}}"#;
    let (t, _) = read(otio, None, None).unwrap();
    assert_eq!(t.rate, Rate { num: 24, den: 1 });
    let c = &t.tracks[0].clips[0];
    assert_eq!((c.path.as_str(), t.rate.frames(c.at), t.rate.frames(c.length)), ("/media/b take.mov", 48, 24));
    assert_eq!(t.rate.frames(c.source_in), 0, "the range is of the media's own time, which starts at its available range");
    // FCPXML with a clip whose picture is inside it, and a clip hanging on it above
    let fcpxml = r#"<?xml version="1.0"?><fcpxml version="1.8"><resources>
        <format id="f1" frameDuration="1001/30000s" width="1920" height="1080"/>
        <asset id="a1" name="c" src="file:///m/c.mov" start="0s" duration="100s" hasVideo="1" hasAudio="1" format="f1"/>
        <asset id="a2" name="d" src="file:///m/d.png" start="0s" duration="0s" hasVideo="1" format="f1"/></resources>
        <library><event name="E"><project name="P"><sequence format="f1" duration="20s"><spine>
          <clip name="C" offset="0s" start="10s" duration="5s"><video ref="a1" offset="0s" start="10s" duration="5s"/>
            <video ref="a2" lane="1" offset="12s" start="0s" duration="2s" name="D"/></clip>
          <transition offset="5s" duration="1s"/>
          <asset-clip ref="a1" offset="5s" start="30s" duration="3s" name="C2"/>
        </spine></sequence></project></event></library></fcpxml>"#;
    let (t, notes) = read(fcpxml, None, None).unwrap();
    assert_eq!(t.rate, Rate::of(30, true));
    assert_eq!(t.name, "P");
    let s: Vec<(String, f64, f64)> = t.tracks.iter().flat_map(|k| k.clips.iter().map(|c| (c.name.clone(), (c.at * 100.0).round() / 100.0, (c.source_in * 100.0).round() / 100.0))).collect();
    assert_eq!(s, [("C".to_string(), 0.0, 10.0), ("C2".to_string(), 5.0, 30.0), ("D".to_string(), 2.0, 0.0)], "the one above at 12 s of C's own time: 2 s in");
    assert!(notes.iter().any(|n| n.contains("transition")));
    // Premiere's XML: an NTSC sequence, its file given in full once and then by id
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE xmeml><xmeml version="5"><sequence id="s"><name>Seq</name>
        <rate><timebase>24</timebase><ntsc>TRUE</ntsc></rate><media><video><track>
          <clipitem id="c1"><name>e</name><rate><timebase>24</timebase><ntsc>TRUE</ntsc></rate><start>0</start><end>48</end><in>24</in><out>72</out>
            <file id="f1"><name>e.mov</name><pathurl>file://localhost/Volumes/Media/e.mov</pathurl><duration>480</duration>
              <media><video><samplecharacteristics><width>3840</width><height>2160</height></samplecharacteristics></video></media></file></clipitem>
          <clipitem id="c2"><name>e again</name><start>48</start><end>96</end><in>0</in><out>48</out><file id="f1"/></clipitem>
        </track></video></media></sequence></xmeml>"#;
    let (t, _) = read(xml, None, None).unwrap();
    assert_eq!(t.rate, Rate::of(24, true));
    let c = &t.tracks[0].clips;
    assert_eq!((c[1].path.as_str(), c[1].media.width), ("/Volumes/Media/e.mov", 3840), "the file by its id");
    assert_eq!(t.rate.frames(c[0].source_in), 24);
}

#[test]
fn urls() {
    assert_eq!(file_url("/a b/c#1.mov"), "file:///a%20b/c%231.mov");
    assert_eq!(path_of("file:///a%20b/c%231.mov"), "/a b/c#1.mov");
    assert_eq!(path_of("file://localhost/x.mov"), "/x.mov");
}
