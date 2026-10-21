//! Glyph names: the ones a CFF font may name by number (its standard strings), the 258 a
//! TrueType font's `post` table may name by number (the Macintosh order), the codes of the Latin
//! encodings PostScript and PDF fonts use (Standard, WinAnsi, MacRoman), and the characters the
//! names stand for (Adobe's glyph list for the names those hold, and the `uniXXXX`/`uXXXX` forms).

use std::collections::HashMap;
use std::sync::OnceLock;

/// The strings a CFF font names by numbers below 391.
const CFF_STRINGS: &str = ".notdef space exclam quotedbl numbersign dollar percent ampersand quoteright parenleft parenright asterisk plus comma hyphen period slash zero one two three four five six seven eight nine colon semicolon less equal greater question at A B C D E F G H I J K L M N O P Q R S T U V W X Y Z bracketleft backslash bracketright asciicircum underscore quoteleft a b c d e f g h i j k l m n o p q r s t u v w x y z braceleft bar braceright asciitilde exclamdown cent sterling fraction yen florin section currency quotesingle quotedblleft guillemotleft guilsinglleft guilsinglright fi fl endash dagger daggerdbl periodcentered paragraph bullet quotesinglbase quotedblbase quotedblright guillemotright ellipsis perthousand questiondown grave acute circumflex tilde macron breve dotaccent dieresis ring cedilla hungarumlaut ogonek caron emdash AE ordfeminine Lslash Oslash OE ordmasculine ae dotlessi lslash oslash oe germandbls onesuperior logicalnot mu trademark Eth onehalf plusminus Thorn onequarter divide brokenbar degree thorn threequarters twosuperior registered minus eth multiply threesuperior copyright Aacute Acircumflex Adieresis Agrave Aring Atilde Ccedilla Eacute Ecircumflex Edieresis Egrave Iacute Icircumflex Idieresis Igrave Ntilde Oacute Ocircumflex Odieresis Ograve Otilde Scaron Uacute Ucircumflex Udieresis Ugrave Yacute Ydieresis Zcaron aacute acircumflex adieresis agrave aring atilde ccedilla eacute ecircumflex edieresis egrave iacute icircumflex idieresis igrave ntilde oacute ocircumflex odieresis ograve otilde scaron uacute ucircumflex udieresis ugrave yacute ydieresis zcaron exclamsmall Hungarumlautsmall dollaroldstyle dollarsuperior ampersandsmall Acutesmall parenleftsuperior parenrightsuperior twodotenleader onedotenleader zerooldstyle oneoldstyle twooldstyle threeoldstyle fouroldstyle fiveoldstyle sixoldstyle sevenoldstyle eightoldstyle nineoldstyle commasuperior threequartersemdash periodsuperior questionsmall asuperior bsuperior centsuperior dsuperior esuperior isuperior lsuperior msuperior nsuperior osuperior rsuperior ssuperior tsuperior ff ffi ffl parenleftinferior parenrightinferior Circumflexsmall hyphensuperior Gravesmall Asmall Bsmall Csmall Dsmall Esmall Fsmall Gsmall Hsmall Ismall Jsmall Ksmall Lsmall Msmall Nsmall Osmall Psmall Qsmall Rsmall Ssmall Tsmall Usmall Vsmall Wsmall Xsmall Ysmall Zsmall colonmonetary onefitted rupiah Tildesmall exclamdownsmall centoldstyle Lslashsmall Scaronsmall Zcaronsmall Dieresissmall Brevesmall Caronsmall Dotaccentsmall Macronsmall figuredash hypheninferior Ogoneksmall Ringsmall Cedillasmall questiondownsmall oneeighth threeeighths fiveeighths seveneighths onethird twothirds zerosuperior foursuperior fivesuperior sixsuperior sevensuperior eightsuperior ninesuperior zeroinferior oneinferior twoinferior threeinferior fourinferior fiveinferior sixinferior seveninferior eightinferior nineinferior centinferior dollarinferior periodinferior commainferior Agravesmall Aacutesmall Acircumflexsmall Atildesmall Adieresissmall Aringsmall AEsmall Ccedillasmall Egravesmall Eacutesmall Ecircumflexsmall Edieresissmall Igravesmall Iacutesmall Icircumflexsmall Idieresissmall Ethsmall Ntildesmall Ogravesmall Oacutesmall Ocircumflexsmall Otildesmall Odieresissmall OEsmall Oslashsmall Ugravesmall Uacutesmall Ucircumflexsmall Udieresissmall Yacutesmall Thornsmall Ydieresissmall 001.000 001.001 001.002 001.003 Black Bold Book Light Medium Regular Roman Semibold";

/// The 258 glyph names of the Macintosh order (a `post` table's names below 258).
const MAC_NAMES: &str = ".notdef .null nonmarkingreturn space exclam quotedbl numbersign dollar percent ampersand quotesingle parenleft parenright asterisk plus comma hyphen period slash zero one two three four five six seven eight nine colon semicolon less equal greater question at A B C D E F G H I J K L M N O P Q R S T U V W X Y Z bracketleft backslash bracketright asciicircum underscore grave a b c d e f g h i j k l m n o p q r s t u v w x y z braceleft bar braceright asciitilde Adieresis Aring Ccedilla Eacute Ntilde Odieresis Udieresis aacute agrave acircumflex adieresis atilde aring ccedilla eacute egrave ecircumflex edieresis iacute igrave icircumflex idieresis ntilde oacute ograve ocircumflex odieresis otilde uacute ugrave ucircumflex udieresis dagger degree cent sterling section bullet paragraph germandbls registered copyright trademark acute dieresis notequal AE Oslash infinity plusminus lessequal greaterequal yen mu partialdiff summation product pi integral ordfeminine ordmasculine Omega ae oslash questiondown exclamdown logicalnot radical florin approxequal Delta guillemotleft guillemotright ellipsis nonbreakingspace Agrave Atilde Otilde OE oe endash emdash quotedblleft quotedblright quoteleft quoteright divide lozenge ydieresis Ydieresis fraction currency guilsinglleft guilsinglright fi fl daggerdbl periodcentered quotesinglbase quotedblbase perthousand Acircumflex Ecircumflex Aacute Edieresis Egrave Iacute Icircumflex Idieresis Igrave Oacute Ocircumflex apple Ograve Uacute Ucircumflex Ugrave dotlessi circumflex tilde macron breve dotaccent ring cedilla hungarumlaut ogonek caron Lslash lslash Scaron scaron Zcaron zcaron brokenbar Eth eth Yacute yacute Thorn thorn minus multiply onesuperior twosuperior threesuperior onehalf onequarter threequarters franc Gbreve gbreve Idotaccent Scedilla scedilla Cacute cacute Ccaron ccaron dcroat";

/// The printable ASCII names from 32 to 126, quotes as Standard has them.
const ASCII: &str = "space exclam quotedbl numbersign dollar percent ampersand quoteright parenleft parenright asterisk plus comma hyphen period slash zero one two three four five six seven eight nine colon semicolon less equal greater question at A B C D E F G H I J K L M N O P Q R S T U V W X Y Z bracketleft backslash bracketright asciicircum underscore quoteleft a b c d e f g h i j k l m n o p q r s t u v w x y z braceleft bar braceright asciitilde";

/// Standard's codes from 161 (`.` none).
const STANDARD_HIGH: &str = "exclamdown cent sterling fraction yen florin section currency quotesingle quotedblleft guillemotleft guilsinglleft guilsinglright fi fl . endash dagger daggerdbl periodcentered . paragraph bullet quotesinglbase quotedblbase quotedblright guillemotright ellipsis perthousand . questiondown . grave acute circumflex tilde macron breve dotaccent dieresis . ring cedilla . hungarumlaut ogonek caron emdash . . . . . . . . . . . . . . . . AE . ordfeminine . . . . Lslash Oslash OE ordmasculine . . . . . ae . . . dotlessi . . lslash oslash oe germandbls . . . .";

/// WinAnsi's codes from 127.
const WIN_HIGH: &str = "bullet Euro bullet quotesinglbase florin quotedblbase ellipsis dagger daggerdbl circumflex perthousand Scaron guilsinglleft OE bullet Zcaron bullet bullet quoteleft quoteright quotedblleft quotedblright bullet endash emdash tilde trademark scaron guilsinglright oe bullet zcaron Ydieresis space exclamdown cent sterling currency yen brokenbar section dieresis copyright ordfeminine guillemotleft logicalnot hyphen registered macron degree plusminus twosuperior threesuperior acute mu paragraph periodcentered cedilla onesuperior ordmasculine guillemotright onequarter onehalf threequarters questiondown Agrave Aacute Acircumflex Atilde Adieresis Aring AE Ccedilla Egrave Eacute Ecircumflex Edieresis Igrave Iacute Icircumflex Idieresis Eth Ntilde Ograve Oacute Ocircumflex Otilde Odieresis multiply Oslash Ugrave Uacute Ucircumflex Udieresis Yacute Thorn germandbls agrave aacute acircumflex atilde adieresis aring ae ccedilla egrave eacute ecircumflex edieresis igrave iacute icircumflex idieresis eth ntilde ograve oacute ocircumflex otilde odieresis divide oslash ugrave uacute ucircumflex udieresis yacute thorn ydieresis";

/// MacRoman's codes from 128.
const MAC_HIGH: &str = "Adieresis Aring Ccedilla Eacute Ntilde Odieresis Udieresis aacute agrave acircumflex adieresis atilde aring ccedilla eacute egrave ecircumflex edieresis iacute igrave icircumflex idieresis ntilde oacute ograve ocircumflex odieresis otilde uacute ugrave ucircumflex udieresis dagger degree cent sterling section bullet paragraph germandbls registered copyright trademark acute dieresis notequal AE Oslash infinity plusminus lessequal greaterequal yen mu partialdiff summation product pi integral ordfeminine ordmasculine Omega ae oslash questiondown exclamdown logicalnot radical florin approxequal Delta guillemotleft guillemotright ellipsis space Agrave Atilde Otilde OE oe endash emdash quotedblleft quotedblright quoteleft quoteright divide lozenge ydieresis Ydieresis fraction currency guilsinglleft guilsinglright fi fl daggerdbl periodcentered quotesinglbase quotedblbase perthousand Acircumflex Ecircumflex Aacute Edieresis Egrave Iacute Icircumflex Idieresis Igrave Oacute Ocircumflex apple Ograve Uacute Ucircumflex Ugrave dotlessi circumflex tilde macron breve dotaccent ring cedilla hungarumlaut ogonek caron";

/// Characters of the names the encodings and the Macintosh order hold beyond ASCII and Latin-1.
const MORE: &[(&str, u32)] = &[
    ("Euro", 0x20AC), ("quotesinglbase", 0x201A), ("florin", 0x0192), ("quotedblbase", 0x201E), ("ellipsis", 0x2026), ("dagger", 0x2020),
    ("daggerdbl", 0x2021), ("circumflex", 0x02C6), ("perthousand", 0x2030), ("Scaron", 0x0160), ("guilsinglleft", 0x2039), ("OE", 0x0152),
    ("Zcaron", 0x017D), ("quoteleft", 0x2018), ("quoteright", 0x2019), ("quotedblleft", 0x201C), ("quotedblright", 0x201D), ("bullet", 0x2022),
    ("endash", 0x2013), ("emdash", 0x2014), ("tilde", 0x02DC), ("trademark", 0x2122), ("scaron", 0x0161), ("guilsinglright", 0x203A),
    ("oe", 0x0153), ("zcaron", 0x017E), ("Ydieresis", 0x0178), ("fraction", 0x2044), ("fi", 0xFB01), ("fl", 0xFB02), ("ff", 0xFB00),
    ("ffi", 0xFB03), ("ffl", 0xFB04), ("dotlessi", 0x0131), ("breve", 0x02D8), ("dotaccent", 0x02D9), ("ring", 0x02DA),
    ("hungarumlaut", 0x02DD), ("ogonek", 0x02DB), ("caron", 0x02C7), ("Lslash", 0x0141), ("lslash", 0x0142), ("notequal", 0x2260),
    ("infinity", 0x221E), ("lessequal", 0x2264), ("greaterequal", 0x2265), ("partialdiff", 0x2202), ("summation", 0x2211),
    ("product", 0x220F), ("pi", 0x03C0), ("integral", 0x222B), ("Omega", 0x2126), ("radical", 0x221A), ("approxequal", 0x2248),
    ("Delta", 0x2206), ("lozenge", 0x25CA), ("apple", 0xF8FF), ("minus", 0x2212), ("nonbreakingspace", 0x00A0), ("nbspace", 0x00A0),
    ("sfthyphen", 0x00AD), ("franc", 0x20A3), ("Gbreve", 0x011E), ("gbreve", 0x011F), ("Idotaccent", 0x0130), ("Scedilla", 0x015E),
    ("scedilla", 0x015F), ("Cacute", 0x0106), ("cacute", 0x0107), ("Ccaron", 0x010C), ("ccaron", 0x010D), ("dcroat", 0x0111),
    ("quotesingle", 0x0027), ("grave", 0x0060), ("nonmarkingreturn", 0x000D), ("middot", 0x00B7), ("overscore", 0x00AF),
];

/// Latin-1's names from 160 (none at 173 is soft hyphen, `hyphen` in the encodings).
const LATIN1: &str = "space exclamdown cent sterling currency yen brokenbar section dieresis copyright ordfeminine guillemotleft logicalnot hyphen registered macron degree plusminus twosuperior threesuperior acute mu paragraph periodcentered cedilla onesuperior ordmasculine guillemotright onequarter onehalf threequarters questiondown Agrave Aacute Acircumflex Atilde Adieresis Aring AE Ccedilla Egrave Eacute Ecircumflex Edieresis Igrave Iacute Icircumflex Idieresis Eth Ntilde Ograve Oacute Ocircumflex Otilde Odieresis multiply Oslash Ugrave Uacute Ucircumflex Udieresis Yacute Thorn germandbls agrave aacute acircumflex atilde adieresis aring ae ccedilla egrave eacute ecircumflex edieresis igrave iacute icircumflex idieresis eth ntilde ograve oacute ocircumflex otilde odieresis divide oslash ugrave uacute ucircumflex udieresis yacute thorn ydieresis";

/// The name a CFF font gives number `sid` from its standard strings.
pub fn cff_string(sid: usize) -> Option<&'static str> {
    static ALL: OnceLock<Vec<&str>> = OnceLock::new();
    ALL.get_or_init(|| CFF_STRINGS.split(' ').collect()).get(sid).copied()
}

/// Glyph `i`'s name in the Macintosh order.
pub fn mac_name(i: usize) -> Option<&'static str> {
    static ALL: OnceLock<Vec<&str>> = OnceLock::new();
    ALL.get_or_init(|| MAC_NAMES.split(' ').collect()).get(i).copied()
}

/// An encoding's 256 names, from its printable ASCII (`quotes`: the plain quote and grave, else
/// Standard's curly ones) and its codes from `from` on.
fn table(quotes: bool, from: usize, high: &'static str) -> [Option<&'static str>; 256] {
    let mut t = [None; 256];
    for (k, n) in ASCII.split(' ').enumerate() {
        t[32 + k] = Some(n);
    }
    if quotes {
        t[39] = Some("quotesingle");
        t[96] = Some("grave");
    }
    for (k, n) in high.split(' ').enumerate() {
        t[from + k] = (n != ".").then_some(n);
    }
    t
}

/// The Latin encodings PostScript and PDF fonts use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Standard,
    WinAnsi,
    MacRoman,
}

/// The name an encoding gives `code`.
pub fn encoded(e: Encoding, code: u8) -> Option<&'static str> {
    static TABLES: OnceLock<[[Option<&str>; 256]; 3]> = OnceLock::new();
    let t = TABLES.get_or_init(|| [table(false, 161, STANDARD_HIGH), table(true, 127, WIN_HIGH), table(true, 128, MAC_HIGH)]);
    t[match e {
        Encoding::Standard => 0,
        Encoding::WinAnsi => 1,
        Encoding::MacRoman => 2,
    }][code as usize]
}

/// The code an encoding gives the glyph named `name`.
pub fn code_of(e: Encoding, name: &str) -> Option<u8> {
    (0..=255u8).find(|&c| encoded(e, c) == Some(name))
}

/// The character a glyph name stands for: from Adobe's list for the Latin names, `uniXXXX` (the
/// first of them), `uXXXX` to `uXXXXXX`, or a name of one character.
pub fn unicode_of(name: &str) -> Option<char> {
    static KNOWN: OnceLock<HashMap<&str, u32>> = OnceLock::new();
    let known = KNOWN.get_or_init(|| {
        let mut m = HashMap::new();
        for (k, n) in ASCII.split(' ').enumerate() {
            m.insert(n, 32 + k as u32);
        }
        m.insert("quoteright", 0x2019);
        m.insert("quoteleft", 0x2018);
        for (k, n) in LATIN1.split(' ').enumerate() {
            m.entry(n).or_insert(160 + k as u32);
        }
        for &(n, c) in MORE {
            m.insert(n, c);
        }
        m.insert("space", 32);
        m.insert("hyphen", 0x2D);
        m
    });
    // the part before a dot or underscore names it (a.sc, f_i aside)
    let base = name.split('.').next().unwrap_or(name);
    if let Some(&c) = known.get(base) {
        return char::from_u32(c);
    }
    let hex = |s: &str| (!s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit())).then(|| u32::from_str_radix(s, 16).ok()).flatten();
    if let Some(rest) = base.strip_prefix("uni")
        && rest.len() >= 4
        && let Some(c) = hex(&rest[..4])
    {
        return char::from_u32(c);
    }
    if let Some(rest) = base.strip_prefix('u')
        && (4..=6).contains(&rest.len())
        && let Some(c) = hex(rest)
    {
        return char::from_u32(c);
    }
    let mut chars = base.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_have_their_sizes_and_places() {
        assert_eq!(CFF_STRINGS.split(' ').count(), 391);
        assert_eq!(cff_string(34), Some("A"));
        assert_eq!(cff_string(390), Some("Semibold"));
        assert_eq!(MAC_NAMES.split(' ').count(), 258);
        assert_eq!(mac_name(36), Some("A"));
        assert_eq!(mac_name(257), Some("dcroat"));
        assert_eq!(STANDARD_HIGH.split(' ').count(), 256 - 161);
        assert_eq!(WIN_HIGH.split(' ').count(), 256 - 127);
        assert_eq!(MAC_HIGH.split(' ').count(), 256 - 128);
        assert_eq!(LATIN1.split(' ').count(), 96);
        for (e, code, name) in [
            (Encoding::Standard, 39, "quoteright"),
            (Encoding::Standard, 174, "fi"),
            (Encoding::Standard, 225, "AE"),
            (Encoding::Standard, 251, "germandbls"),
            (Encoding::WinAnsi, 39, "quotesingle"),
            (Encoding::WinAnsi, 128, "Euro"),
            (Encoding::WinAnsi, 159, "Ydieresis"),
            (Encoding::WinAnsi, 233, "eacute"),
            (Encoding::MacRoman, 128, "Adieresis"),
            (Encoding::MacRoman, 202, "space"),
            (Encoding::MacRoman, 255, "caron"),
        ] {
            assert_eq!(encoded(e, code), Some(name), "{e:?} {code}");
        }
        assert_eq!(encoded(Encoding::Standard, 176), None);
    }

    #[test]
    fn names_stand_for_their_characters() {
        for (name, c) in [("A", 'A'), ("quoteright", '’'), ("eacute", 'é'), ("Euro", '€'), ("uni0E01", 'ก'), ("u1F600", '😀'), ("fi", 'ﬁ'), ("a.sc", 'a'), ("hyphen", '-'), ("space", ' ')] {
            assert_eq!(unicode_of(name), Some(c), "{name}");
        }
        assert_eq!(unicode_of("g123"), None);
    }
}
