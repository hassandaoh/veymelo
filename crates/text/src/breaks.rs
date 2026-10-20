//! Where a line may break: after spaces, after a hyphen, between ideographs, and between Thai
//! words (Thai leaves no spaces between words; the system's dictionary finds them).

/// The places in `text` (character indices) a line may break before.
pub fn opportunities(text: &[char]) -> Vec<usize> {
    let mut out = Vec::new();
    let space = |c: char| c == ' ' || c == '\u{3000}' || c == '\t';
    let closing = |c: char| ".,!?:;)]}»”’%、。ๆฯ".contains(c);
    for i in 1..text.len() {
        let (a, b) = (text[i - 1], text[i]);
        let after_space = space(a) && !space(b);
        let after_hyphen = (a == '-' || a == '\u{2010}') && b.is_alphanumeric();
        let ideographs = is_ideograph(a) || is_ideograph(b);
        if (after_space || after_hyphen || ideographs) && !closing(b) {
            out.push(i);
        }
    }
    // Thai words, from the system's word breaker
    for (start, end) in thai_runs(text) {
        for k in words(&text[start..end]) {
            if k > 0 && start + k < text.len() && !closing(text[start + k]) {
                out.push(start + k);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn is_ideograph(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xAC00..=0xD7AF)
}

fn thai_runs(text: &[char]) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut start = None;
    for (i, &c) in text.iter().enumerate() {
        let thai = (0x0E00..=0x0E7F).contains(&(c as u32));
        match (thai, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                runs.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        runs.push((s, text.len()));
    }
    runs
}

/// Where words start in a run of Thai (character indices).
#[cfg(target_os = "macos")]
fn words(run: &[char]) -> Vec<usize> {
    use std::ffi::c_void;
    type Ref = *const c_void;
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Range {
        start: isize,
        len: isize,
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCharacters(a: Ref, chars: *const u16, len: isize) -> Ref;
        fn CFLocaleCreate(a: Ref, id: Ref) -> Ref;
        fn CFStringTokenizerCreate(a: Ref, s: Ref, range: Range, unit: usize, locale: Ref) -> Ref;
        fn CFStringTokenizerAdvanceToNextToken(t: Ref) -> usize;
        fn CFStringTokenizerGetCurrentTokenRange(t: Ref) -> Range;
        fn CFRelease(r: Ref);
    }
    const WORD_BOUNDARY: usize = 4;
    let utf16: Vec<u16> = run.iter().collect::<String>().encode_utf16().collect();
    // UTF-16 offsets back to character indices (Thai is all in the first plane, but be sure)
    let mut char_of = Vec::with_capacity(utf16.len() + 1);
    for (i, c) in run.iter().enumerate() {
        for _ in 0..c.len_utf16() {
            char_of.push(i);
        }
    }
    char_of.push(run.len());
    let mut out = Vec::new();
    unsafe {
        let s = CFStringCreateWithCharacters(std::ptr::null(), utf16.as_ptr(), utf16.len() as isize);
        let th: Vec<u16> = "th".encode_utf16().collect();
        let id = CFStringCreateWithCharacters(std::ptr::null(), th.as_ptr(), th.len() as isize);
        let locale = CFLocaleCreate(std::ptr::null(), id);
        let tok = CFStringTokenizerCreate(std::ptr::null(), s, Range { start: 0, len: utf16.len() as isize }, WORD_BOUNDARY, locale);
        while CFStringTokenizerAdvanceToNextToken(tok) != 0 {
            let r = CFStringTokenizerGetCurrentTokenRange(tok);
            if r.start >= 0 && (r.start as usize) < char_of.len() {
                out.push(char_of[r.start as usize]);
            }
        }
        for r in [tok, locale, id, s] {
            CFRelease(r);
        }
    }
    out
}

/// Without the system's dictionary, a Thai run breaks only where it has spaces.
#[cfg(not(target_os = "macos"))]
fn words(_run: &[char]) -> Vec<usize> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Vec<usize> {
        opportunities(&text.chars().collect::<Vec<_>>())
    }

    #[test]
    fn spaces_and_hyphens() {
        assert_eq!(at("one two  three"), vec![4, 9]);
        assert_eq!(at("well-known"), vec![5]);
        assert_eq!(at("end ."), vec![], "never before a full stop");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn thai_words_are_found() {
        let text: Vec<char> = "สวัสดีครับวันนี้อากาศดี".chars().collect();
        let breaks = opportunities(&text);
        // สวัสดี|ครับ|วันนี้|อากาศ|ดี
        assert!(breaks.contains(&6) && breaks.contains(&10), "{breaks:?}");
        assert!(!breaks.contains(&3), "not inside a word: {breaks:?}");
    }
}
