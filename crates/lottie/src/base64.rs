//! Base64 (RFC 4648), for the pictures a Lottie file carries inside it as `data:` addresses.

const LETTERS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = u32::from(c[0]) << 16 | u32::from(*c.get(1).unwrap_or(&0)) << 8 | u32::from(*c.get(2).unwrap_or(&0));
        for k in 0..4 {
            if k <= c.len() {
                out.push(LETTERS[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The bytes base64 text stands for (spaces and line breaks in it are passed over; the
/// URL-safe letters `-` and `_` are read too).
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = acc << 6 | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// A `data:` address's type and bytes (`data:image/png;base64,…`).
pub fn data_uri(uri: &str) -> Option<(String, Vec<u8>)> {
    let rest = uri.strip_prefix("data:")?;
    let (head, body) = rest.split_once(',')?;
    let mime = head.split(';').next().unwrap_or("").to_ascii_lowercase();
    let bytes = if head.ends_with(";base64") { decode(body)? } else { body.as_bytes().to_vec() };
    Some((mime, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_ways() {
        for (raw, text) in [(&b""[..], ""), (b"f", "Zg=="), (b"fo", "Zm8="), (b"foo", "Zm9v"), (b"foobar", "Zm9vYmFy")] {
            assert_eq!(encode(raw), text);
            assert_eq!(decode(text).as_deref(), Some(raw));
        }
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&all)), Some(all));
        assert_eq!(decode("Zm9v\nYmFy"), Some(b"foobar".to_vec()));
        assert_eq!(decode("Zm9v*"), None);
        assert_eq!(data_uri("data:image/png;base64,Zm9v"), Some(("image/png".into(), b"foo".to_vec())));
        assert_eq!(data_uri("images/a.png"), None);
    }
}
