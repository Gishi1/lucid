//! Text decoding for topic parts.
//!
//! Real-world packages contain UTF-8 (sometimes with a BOM), UTF-16 with a BOM, and
//! Windows-1252 topics (what you get when a tool writes "smart quotes" without declaring a
//! charset). We decode all of those without pulling in a full encoding library:
//! Windows-1252 is Latin-1 except for the 0x80..=0x9F range, which we map explicitly.

/// The encoding a part was decoded from (shown in `lucid info`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Windows1252,
    Latin1,
}

impl Encoding {
    pub fn name(self) -> &'static str {
        match self {
            Encoding::Utf8 => "utf-8",
            Encoding::Utf8Bom => "utf-8 (BOM)",
            Encoding::Utf16Le => "utf-16le",
            Encoding::Utf16Be => "utf-16be",
            Encoding::Windows1252 => "windows-1252",
            Encoding::Latin1 => "iso-8859-1",
        }
    }
}

/// Decoded text plus the encoding that was used.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub text: String,
    pub encoding: Encoding,
    /// True when the bytes were not valid UTF-8 and had to be lossily repaired.
    pub lossy: bool,
}

/// Decode a topic part.
///
/// `declared` is the charset advertised by the document (`<meta charset>`,
/// `http-equiv="Content-Type"`), if any; it is only consulted when there is no BOM and the
/// bytes are not valid UTF-8.
pub fn decode_bytes(bytes: &[u8], declared: Option<&str>) -> Decoded {
    // 1. BOM sniffing wins over everything.
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return match std::str::from_utf8(rest) {
            Ok(s) => Decoded {
                text: s.to_owned(),
                encoding: Encoding::Utf8Bom,
                lossy: false,
            },
            Err(_) => lossy_utf8(rest, Encoding::Utf8Bom),
        };
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return match decode_utf16(rest, true) {
            Some(text) => Decoded {
                text,
                encoding: Encoding::Utf16Le,
                lossy: false,
            },
            None => lossy_utf8(bytes, Encoding::Utf8),
        };
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return match decode_utf16(rest, false) {
            Some(text) => Decoded {
                text,
                encoding: Encoding::Utf16Be,
                lossy: false,
            },
            None => lossy_utf8(bytes, Encoding::Utf8),
        };
    }

    // 2. Plain UTF-8 (the overwhelmingly common case).
    if let Ok(s) = std::str::from_utf8(bytes) {
        // Honour a declared non-UTF-8 charset only if the content is pure ASCII anyway —
        // there is nothing to re-encode in that case, so we keep the cheap path.
        return Decoded {
            text: s.to_owned(),
            encoding: Encoding::Utf8,
            lossy: false,
        };
    }

    // 3. Declared legacy charset, else Windows-1252 (superset of used Latin-1 pages).
    match declared.map(str::trim).map(str::to_ascii_lowercase) {
        Some(c) if is_latin1ish(&c) => Decoded {
            text: bytes.iter().map(|b| *b as char).collect(),
            encoding: Encoding::Latin1,
            lossy: false,
        },
        Some(c) if c == "windows-1252" || c == "cp1252" || c == "ansi" => Decoded {
            text: decode_cp1252(bytes),
            encoding: Encoding::Windows1252,
            lossy: false,
        },
        _ => Decoded {
            text: decode_cp1252(bytes),
            encoding: Encoding::Windows1252,
            lossy: false,
        },
    }
}

fn is_latin1ish(charset: &str) -> bool {
    matches!(
        charset,
        "iso-8859-1" | "iso8859-1" | "latin1" | "latin-1" | "iso_8859-1"
    )
}

fn lossy_utf8(bytes: &[u8], encoding: Encoding) -> Decoded {
    Decoded {
        text: String::from_utf8_lossy(bytes).into_owned(),
        encoding,
        lossy: true,
    }
}

/// Minimal UTF-16 decoder (only used for BOM-marked parts).
fn decode_utf16(bytes: &[u8], little_endian: bool) -> Option<String> {
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| {
            if little_endian {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    Some(String::from_utf16_lossy(&units))
}

/// Windows-1252 with the 0x80..=0x9F block mapped to its "smart typography" characters
/// (the rest is identical to Latin-1).
pub fn decode_cp1252(bytes: &[u8]) -> String {
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{8D}',
        '\u{017D}', '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}',
        '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}',
        '\u{0153}', '\u{9D}', '\u{017E}', '\u{0178}',
    ];
    let mut out = String::with_capacity(bytes.len());
    for b in bytes {
        match *b {
            0x00..=0x7F => out.push(*b as char),
            0x80..=0x9F => out.push(HIGH[(*b - 0x80) as usize]),
            _ => out.push(*b as char),
        }
    }
    out
}

/// Find a declared charset in the first chunk of a document (`<meta charset=…>` or
/// `http-equiv="Content-Type" content="…charset=…"`). Cheap on purpose: this runs before
/// decoding, on raw bytes.
pub fn sniff_declared_charset(bytes: &[u8], limit: usize) -> Option<String> {
    let hay = &bytes[..bytes.len().min(limit)];
    let text: String = hay
        .iter()
        .map(|b| if b.is_ascii() { *b as char } else { ' ' })
        .collect();
    let lower = text.to_ascii_lowercase();

    // <meta charset="x">
    if let Some(idx) = lower.find("charset") {
        let after = &lower[idx + "charset".len()..];
        let after = after.trim_start();
        let after = after.strip_prefix('=').unwrap_or(after).trim_start();
        let after = after.trim_start_matches(['"', '\'']);
        let end = after
            .find(|c: char| c == '"' || c == '\'' || c == ';' || c.is_whitespace() || c == '>')
            .unwrap_or(after.len());
        let value = after[..end].trim();
        if !value.is_empty() && value.len() < 40 {
            return Some(value.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_utf8() {
        let d = decode_bytes("héllo".as_bytes(), None);
        assert_eq!(d.text, "héllo");
        assert_eq!(d.encoding, Encoding::Utf8);
        assert!(!d.lossy);
    }

    #[test]
    fn utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"hi");
        let d = decode_bytes(&bytes, None);
        assert_eq!(d.text, "hi");
        assert_eq!(d.encoding, Encoding::Utf8Bom);
    }

    #[test]
    fn cp1252_smart_quotes() {
        let bytes = b"caf\xe9 \x93quoted\x94";
        let d = decode_bytes(bytes, None);
        assert_eq!(d.text, "café “quoted”");
        assert_eq!(d.encoding, Encoding::Windows1252);
    }

    #[test]
    fn declared_latin1() {
        let bytes = b"caf\xe9";
        let d = decode_bytes(bytes, Some("ISO-8859-1"));
        assert_eq!(d.text, "café");
        assert_eq!(d.encoding, Encoding::Latin1);
    }

    #[test]
    fn utf16_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "hi".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let d = decode_bytes(&bytes, None);
        assert_eq!(d.text, "hi");
        assert_eq!(d.encoding, Encoding::Utf16Le);
    }

    #[test]
    fn charset_sniffing() {
        let html = br#"<html><head><meta http-equiv="Content-Type" content="text/html; charset=windows-1252"></head></html>"#;
        assert_eq!(
            sniff_declared_charset(html, 4096).as_deref(),
            Some("windows-1252")
        );
        let html2 = br#"<meta charset='UTF-8'>"#;
        assert_eq!(sniff_declared_charset(html2, 4096).as_deref(), Some("utf-8"));
        let html3 = br#"<html><head><title>x</title></head></html>"#;
        assert_eq!(sniff_declared_charset(html3, 4096), None);
    }
}
