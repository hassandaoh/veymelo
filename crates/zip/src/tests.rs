use super::*;

fn bytes(hex: &str) -> Vec<u8> {
    (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()
}

fn text() -> Vec<u8> {
    let mut t = "The quick brown fox jumps over the lazy dog. ".repeat(6);
    t.push_str(&"ตัวอักษรไทยก็มาด้วย 0123456789 ".repeat(3));
    t.into_bytes()
}

/// Streams made by another deflater (zlib, the one most files come from): stored, fixed codes
/// and codes made for the block.
const STORED: &str = "01dd0122fe54686520717569636b2062726f776e20666f78206a756d7073206f76657220746865206c617a7920646f672e2054686520717569636b2062726f776e20666f78206a756d7073206f76657220746865206c617a7920646f672e2054686520717569636b2062726f776e20666f78206a756d7073206f76657220746865206c617a7920646f672e2054686520717569636b2062726f776e20666f78206a756d7073206f76657220746865206c617a7920646f672e2054686520717569636b2062726f776e20666f78206a756d7073206f76657220746865206c617a7920646f672e2054686520717569636b2062726f776e20666f78206a756d7073206f76657220746865206c617a7920646f672e20e0b895e0b8b1e0b8a7e0b8ade0b8b1e0b881e0b8a9e0b8a3e0b984e0b897e0b8a2e0b881e0b987e0b8a1e0b8b2e0b894e0b989e0b8a7e0b8a2203031323334353637383920e0b895e0b8b1e0b8a7e0b8ade0b8b1e0b881e0b8a9e0b8a3e0b984e0b897e0b8a2e0b881e0b987e0b8a1e0b8b2e0b894e0b989e0b8a7e0b8a2203031323334353637383920e0b895e0b8b1e0b8a7e0b8ade0b8b1e0b881e0b8a9e0b8a3e0b984e0b897e0b8a2e0b881e0b987e0b8a1e0b8b2e0b894e0b989e0b8a7e0b8a2203031323334353637383920";
const FIXED: &str = "0bc94855282ccd4cce56482aca2fcf5348cbaf50c82acd2d2856c82f4b2d5228014ae72456552aa4e4a7eb29840c77c50f764c7db063e3831dcb1fec580b66343ed8b1f2c18ec50f76b63cd831fdc18e4520919ded0f762c7cb063d3831d531eecec042b5ea4606068646c626a666e6139880c0100";
const DYNAMIC: &str = "edd3d381e5000000c05662db7eb16da7ff2eae8bfbdae96100dcf9faeed681ede67bdfcdc1dcf507e1d941e31d46d02321d203c7e1b5e0f393a6f057c2e52f9c65e0426c609a619f4698005966d0416f5059e2b0c19a305955e8c15c68ddb088c3b908db56d8233e44dfb3b823ba38db0d14207d743f88442085943c0f2ca040cc3c8b74a4c7acbc2c1ca243fcbce95c66e49cbe2f226210d279ea6266e7a27e1c0ae512ea79d952e5d492fb5e3ae531dafbda6a15b6ba037e0c2e64cc07088d211a8d00026c2e125c000ded211ded00051c25973c044b9d255b9d84c042a5d042844cfda5dcc38cc622a3d622822aa3abb9a38aa152a3b1528aafd3abbfd38aa73267740a4ee8f3cff8cd3a41a8bd29a839b92f8009ac075568a235682465aea11d6e164de9a32de92573edb003ef175319b2339b0ceb1cb11b1f0ecb588ba75a0df79ca8879e2fdfdada6fb539debb523fbb7f02ef6cbeee7402f01001f10062ef1ac1ee8952f01661f10692e89d91f51b65e8a322ea8394c9b7e3cb979508a8e21a8c5525b8132798550462531642b52574d327d4f4146eb3164e752d7af35fbc1938c2e73c821b7be21740629839da173d5658660a96417a58242655124ed96716565066391531d5135139760ed77171bb0ca9340ac9b87799b468e97a0db5b40bd5febc0aebd2eafd71b4d6eb0cef7f75d6e3f5cfefd97dd8d93ef01b62c85b3014397334393104db72a2ba301af9733af9318e7a7aae7a3896057b76442541867a61863899a5677da4254d46766d462cd3646ff3642dc3a676e7e72cdf946fff942dcf17e1e817a23054fff035b324d7e114d7a23c34e882f4b32a37e91a37aab68de88e0cbba6f7e9568ebab94de441ccbba94ff55d4eb675ade44d2cafe3aef5d3aeb67b6d0cf8fbf3f7e7efcfdf9fbf3f7f7ffefefcfdf94f7ffe01";
const ZLIB: &str = "789ccb48cdc9c957c8402701680308b1";
/// An archive made by another zip writer: a folder, a stored file and a deflated one.
const ARCHIVE: &str = "504b0304140000000000f921535d00000000000000000000000007000000696d616765732f504b030414000000000000002100d1bb42170f0000000f0000000d0000006d616e69666573742e6a736f6e7b2276657273696f6e223a2231227d504b030414000000080000002100973029a61e0000006400000011000000616e696d6174696f6e732f612e6a736f6eab562a53b25232d533d73350d251ca49ac4c2d2a56b28a8eadada69e0400504b01021403140000000000f921535d000000000000000000000000070000000000000000001000fd4100000000696d616765732f504b0102140314000000000000002100d1bb42170f0000000f0000000d00000000000000000000008001250000006d616e69666573742e6a736f6e504b0102140314000000080000002100973029a61e0000006400000011000000000000000000000080015f000000616e696d6174696f6e732f612e6a736f6e504b05060000000003000300af000000ac0000000000";


fn noise(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

#[test]
fn the_check_value() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
}

#[test]
fn streams_from_another_deflater_read_back() {
    assert_eq!(inflate(&bytes(STORED)).unwrap(), text());
    assert_eq!(inflate(&bytes(FIXED)).unwrap(), text());
    let dynamic: Vec<u8> = (0..3000u32).map(|i| ((i * i * 7 + i / 13) % 64) as u8 + 32).collect();
    let d = bytes(DYNAMIC);
    assert_eq!((d[0] >> 1) & 3, 2, "codes made for the block");
    assert_eq!(inflate(&d).unwrap(), dynamic);
    assert_eq!(unzlib(&bytes(ZLIB)).unwrap(), b"hello hello hello hello");
}

#[test]
fn ours_read_back() {
    let mut samples: Vec<Vec<u8>> = vec![vec![], vec![7], b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_vec(), text()];
    samples.push(noise(100_000, 1));
    let mut mixed = noise(5000, 9);
    mixed.extend(std::iter::repeat_n(0u8, 70_000));
    mixed.extend(noise(70_000, 3));
    samples.push(mixed);
    for s in &samples {
        assert_eq!(inflate(&deflate(s)).as_deref(), Ok(&s[..]), "{} bytes", s.len());
        assert_eq!(unzlib(&zlib(s)).as_deref(), Ok(&s[..]));
    }
}

#[test]
fn broken_streams_are_errors() {
    let good = bytes(DYNAMIC);
    for cut in [0, 1, 5, 40, good.len() - 1] {
        assert!(inflate(&good[..cut]).is_err(), "cut at {cut}");
    }
    // anything at all gives a value or an error, never a crash
    for seed in 1..200 {
        let _ = inflate(&noise(300, seed));
    }
    assert_eq!(inflate(&[0x07]).unwrap_err(), "a block of an unknown kind");
    assert!(inflate_up_to(&deflate(&[0u8; 10_000]), 9_999).unwrap_err().contains("more than 9999"));
    let mut z = zlib(b"abc");
    let n = z.len();
    z[n - 1] ^= 1;
    assert!(unzlib(&z).unwrap_err().contains("check sum"));
}

#[test]
fn archives_both_ways() {
    let files = unzip(&bytes(ARCHIVE)).unwrap();
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["manifest.json", "animations/a.json"], "the folder is left out");
    assert_eq!(files[1].1, br#"{"v":"5.7.0","layers":[]}"#.repeat(4));
    let big = text().repeat(20);
    let ours = zip(&[("a.txt", &b"tiny"[..]), ("ไทย/b.txt", &big)]);
    let back = unzip(&ours).unwrap();
    assert_eq!(back, vec![("a.txt".to_string(), b"tiny".to_vec()), ("ไทย/b.txt".to_string(), big.clone())]);
    assert!(ours.len() < big.len() / 4, "the big one is deflated");
    let mut broken = ours.clone();
    let at = broken.windows(4).position(|w| w == b"tiny").unwrap();
    broken[at] = b'T';
    assert!(unzip(&broken).unwrap_err().contains("check sum"));
    assert!(unzip(b"not an archive at all").is_err());
}

/// How fast inflate is (run by hand: cargo test -p veymelo-zip --release inflate_speed -- --ignored --nocapture).
#[test]
#[ignore]
fn inflate_speed() {
    let mut data = noise(4_000_000, 5);
    for (i, b) in data.iter_mut().enumerate() {
        *b = if i % 7 < 4 { (i / 300 % 40) as u8 + b'a' } else { *b % 16 };
    }
    let packed = deflate(&data);
    let started = std::time::Instant::now();
    for _ in 0..5 {
        assert_eq!(inflate(&packed).unwrap().len(), data.len());
    }
    eprintln!("inflate: {:.1} MB/s", 5.0 * data.len() as f64 / 1e6 / started.elapsed().as_secs_f64());
}
