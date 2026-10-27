//! HTML character reference decoding (named + numeric).
//!
//! Help topics are XHTML-ish, so references are mostly `&amp;`, `&lt;`, `&gt;`, `&quot;`,
//! `&nbsp;` and numeric escapes. We cover the common named set, everything numeric, and
//! leave unknown references untouched (better than dropping text).

/// Decode character references in `s`.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < s.len() {
        let c = bytes[i];
        if c != b'&' {
            // copy one whole UTF-8 char
            let ch_len = utf8_len(c);
            let end = (i + ch_len).min(s.len());
            out.push_str(&s[i..end]);
            i = end;
            continue;
        }
        // find the terminating ';' within a sane distance
        let rest = &s[i + 1..];
        let semi = rest
            .char_indices()
            .take(34)
            .find(|(_, ch)| *ch == ';' || *ch == '&' || *ch == '<' || *ch == ' ')
            .map(|(idx, ch)| (idx, ch));
        match semi {
            Some((idx, ';')) => {
                let name = &rest[..idx];
                match decode_one(name) {
                    Some(ch) => {
                        out.push(ch);
                        i += 1 + idx + 1;
                    }
                    None => {
                        out.push('&');
                        i += 1;
                    }
                }
            }
            _ => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// Decode a single entity body (without the `&` and `;`).
pub fn decode_one(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let (radix, digits) = match num.strip_prefix(['x', 'X']) {
            Some(hex) => (16, hex),
            None => (10, num),
        };
        if digits.is_empty() || digits.len() > 8 {
            return None;
        }
        let value = u32::from_str_radix(digits, radix).ok()?;
        return char::from_u32(value).or(Some('\u{FFFD}'));
    }
    NAMED
        .binary_search_by(|(n, _)| (*n).cmp(name))
        .ok()
        .map(|idx| NAMED[idx].1)
}

/// Sorted so lookups can binary-search.
static NAMED: &[(&str, char)] = &[
    ("AElig", 'Æ'),
    ("Aacute", 'Á'),
    ("Acirc", 'Â'),
    ("Agrave", 'À'),
    ("Alpha", 'Α'),
    ("Aring", 'Å'),
    ("Atilde", 'Ã'),
    ("Auml", 'Ä'),
    ("Beta", 'Β'),
    ("Ccedil", 'Ç'),
    ("Chi", 'Χ'),
    ("Dagger", '‡'),
    ("Delta", 'Δ'),
    ("ETH", 'Ð'),
    ("Eacute", 'É'),
    ("Ecirc", 'Ê'),
    ("Egrave", 'È'),
    ("Epsilon", 'Ε'),
    ("Eta", 'Η'),
    ("Euml", 'Ë'),
    ("Gamma", 'Γ'),
    ("Iacute", 'Í'),
    ("Icirc", 'Î'),
    ("Igrave", 'Ì'),
    ("Iota", 'Ι'),
    ("Iuml", 'Ï'),
    ("Kappa", 'Κ'),
    ("Lambda", 'Λ'),
    ("Mu", 'Μ'),
    ("Ntilde", 'Ñ'),
    ("Nu", 'Ν'),
    ("OElig", 'Œ'),
    ("Oacute", 'Ó'),
    ("Ocirc", 'Ô'),
    ("Ograve", 'Ò'),
    ("Omega", 'Ω'),
    ("Omicron", 'Ο'),
    ("Oslash", 'Ø'),
    ("Otilde", 'Õ'),
    ("Ouml", 'Ö'),
    ("Phi", 'Φ'),
    ("Pi", 'Π'),
    ("Prime", '″'),
    ("Psi", 'Ψ'),
    ("Rho", 'Ρ'),
    ("Sigma", 'Σ'),
    ("THORN", 'Þ'),
    ("Tau", 'Τ'),
    ("Theta", 'Θ'),
    ("Uacute", 'Ú'),
    ("Ucirc", 'Û'),
    ("Ugrave", 'Ù'),
    ("Upsilon", 'Υ'),
    ("Uuml", 'Ü'),
    ("Xi", 'Ξ'),
    ("Yacute", 'Ý'),
    ("Yuml", 'Ÿ'),
    ("Zeta", 'Ζ'),
    ("aacute", 'á'),
    ("acirc", 'â'),
    ("acute", '´'),
    ("aelig", 'æ'),
    ("agrave", 'à'),
    ("alefsym", 'ℵ'),
    ("alpha", 'α'),
    ("amp", '&'),
    ("and", '∧'),
    ("ang", '∠'),
    ("apos", '\''),
    ("aring", 'å'),
    ("asymp", '≈'),
    ("atilde", 'ã'),
    ("auml", 'ä'),
    ("bdquo", '„'),
    ("beta", 'β'),
    ("brvbar", '¦'),
    ("bull", '•'),
    ("cap", '∩'),
    ("ccedil", 'ç'),
    ("cedil", '¸'),
    ("cent", '¢'),
    ("chi", 'χ'),
    ("circ", 'ˆ'),
    ("clubs", '♣'),
    ("cong", '≅'),
    ("copy", '©'),
    ("crarr", '↵'),
    ("cup", '∪'),
    ("curren", '¤'),
    ("dArr", '⇓'),
    ("dagger", '†'),
    ("darr", '↓'),
    ("deg", '°'),
    ("delta", 'δ'),
    ("diams", '♦'),
    ("divide", '÷'),
    ("eacute", 'é'),
    ("ecirc", 'ê'),
    ("egrave", 'è'),
    ("empty", '∅'),
    ("emsp", '\u{2003}'),
    ("ensp", '\u{2002}'),
    ("epsilon", 'ε'),
    ("equiv", '≡'),
    ("eta", 'η'),
    ("eth", 'ð'),
    ("euml", 'ë'),
    ("euro", '€'),
    ("exist", '∃'),
    ("fnof", 'ƒ'),
    ("forall", '∀'),
    ("frac12", '½'),
    ("frac14", '¼'),
    ("frac34", '¾'),
    ("frasl", '⁄'),
    ("gamma", 'γ'),
    ("ge", '≥'),
    ("gt", '>'),
    ("hArr", '⇔'),
    ("harr", '↔'),
    ("hearts", '♥'),
    ("hellip", '…'),
    ("iacute", 'í'),
    ("icirc", 'î'),
    ("iexcl", '¡'),
    ("igrave", 'ì'),
    ("image", 'ℑ'),
    ("infin", '∞'),
    ("int", '∫'),
    ("iota", 'ι'),
    ("iquest", '¿'),
    ("isin", '∈'),
    ("iuml", 'ï'),
    ("kappa", 'κ'),
    ("lArr", '⇐'),
    ("lambda", 'λ'),
    ("lang", '〈'),
    ("laquo", '«'),
    ("larr", '←'),
    ("lceil", '⌈'),
    ("ldquo", '“'),
    ("le", '≤'),
    ("lfloor", '⌊'),
    ("lowast", '∗'),
    ("loz", '◊'),
    ("lrm", '\u{200E}'),
    ("lsaquo", '‹'),
    ("lsquo", '‘'),
    ("lt", '<'),
    ("macr", '¯'),
    ("mdash", '—'),
    ("micro", 'µ'),
    ("middot", '·'),
    ("minus", '−'),
    ("mu", 'μ'),
    ("nabla", '∇'),
    ("nbsp", '\u{A0}'),
    ("ndash", '–'),
    ("ne", '≠'),
    ("ni", '∋'),
    ("not", '¬'),
    ("notin", '∉'),
    ("nsub", '⊄'),
    ("ntilde", 'ñ'),
    ("nu", 'ν'),
    ("oacute", 'ó'),
    ("ocirc", 'ô'),
    ("oelig", 'œ'),
    ("ograve", 'ò'),
    ("oline", '‾'),
    ("omega", 'ω'),
    ("omicron", 'ο'),
    ("oplus", '⊕'),
    ("or", '∨'),
    ("ordf", 'ª'),
    ("ordm", 'º'),
    ("oslash", 'ø'),
    ("otilde", 'õ'),
    ("otimes", '⊗'),
    ("ouml", 'ö'),
    ("para", '¶'),
    ("part", '∂'),
    ("permil", '‰'),
    ("perp", '⊥'),
    ("phi", 'φ'),
    ("pi", 'π'),
    ("piv", 'ϖ'),
    ("plusmn", '±'),
    ("pound", '£'),
    ("prime", '′'),
    ("prod", '∏'),
    ("prop", '∝'),
    ("psi", 'ψ'),
    ("quot", '"'),
    ("rArr", '⇒'),
    ("radic", '√'),
    ("rang", '〉'),
    ("raquo", '»'),
    ("rarr", '→'),
    ("rceil", '⌉'),
    ("rdquo", '”'),
    ("reg", '®'),
    ("rfloor", '⌋'),
    ("rho", 'ρ'),
    ("rlm", '\u{200F}'),
    ("rsaquo", '›'),
    ("rsquo", '’'),
    ("sbquo", '‚'),
    ("scaron", 'š'),
    ("sdot", '⋅'),
    ("sect", '§'),
    ("shy", '\u{AD}'),
    ("sigma", 'σ'),
    ("sigmaf", 'ς'),
    ("sim", '∼'),
    ("spades", '♠'),
    ("sub", '⊂'),
    ("sube", '⊆'),
    ("sum", '∑'),
    ("sup", '⊃'),
    ("sup1", '¹'),
    ("sup2", '²'),
    ("sup3", '³'),
    ("supe", '⊇'),
    ("szlig", 'ß'),
    ("tau", 'τ'),
    ("there4", '∴'),
    ("theta", 'θ'),
    ("thetasym", 'ϑ'),
    ("thinsp", '\u{2009}'),
    ("thorn", 'þ'),
    ("tilde", '˜'),
    ("times", '×'),
    ("trade", '™'),
    ("uArr", '⇑'),
    ("uacute", 'ú'),
    ("uarr", '↑'),
    ("ucirc", 'û'),
    ("ugrave", 'ù'),
    ("uml", '¨'),
    ("upsih", 'ϒ'),
    ("upsilon", 'υ'),
    ("uuml", 'ü'),
    ("weierp", '℘'),
    ("xi", 'ξ'),
    ("yacute", 'ý'),
    ("yen", '¥'),
    ("yuml", 'ÿ'),
    ("zeta", 'ζ'),
    ("zwj", '\u{200D}'),
    ("zwnj", '\u{200C}'),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        assert_eq!(decode_entities("a &amp; b"), "a & b");
        assert_eq!(decode_entities("&lt;tag&gt;"), "<tag>");
        assert_eq!(decode_entities("caf&#233;"), "café");
        assert_eq!(decode_entities("caf&#xE9;"), "café");
        assert_eq!(decode_entities("&nbsp;x"), "\u{A0}x");
        assert_eq!(decode_entities("&mdash;&hellip;"), "—…");
    }

    #[test]
    fn unknown_and_malformed_are_kept() {
        assert_eq!(decode_entities("&unknown;"), "&unknown;");
        assert_eq!(decode_entities("a & b"), "a & b");
        assert_eq!(decode_entities("&amp"), "&amp");
        assert_eq!(decode_entities("100% &"), "100% &");
    }

    #[test]
    fn does_not_split_utf8() {
        assert_eq!(decode_entities("héllo &amp; wörld"), "héllo & wörld");
        assert_eq!(decode_entities("日本語&amp;test"), "日本語&test");
    }

    #[test]
    fn name_table_is_sorted() {
        let mut sorted: Vec<&str> = NAMED.iter().map(|(n, _)| *n).collect();
        let original = sorted.clone();
        sorted.sort_unstable();
        assert_eq!(original, sorted, "NAMED must stay sorted for binary search");
    }
}
