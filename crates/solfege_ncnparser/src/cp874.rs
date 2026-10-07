//! Windows-874 (a superset of TIS-620) <-> Unicode. NCN lyric files are
//! TIS-620 Thai, occasionally with Windows-874 punctuation such as `…` (0x85)
//! or `’` (0x92).

/// Decode one byte; bytes undefined in Windows-874 map to U+FFFD.
pub fn decode_byte(b: u8) -> char {
    match b {
        0x00..=0x7F => b as char,
        0x80 => '\u{20AC}',
        0x85 => '\u{2026}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201C}',
        0x94 => '\u{201D}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0xA0 => '\u{00A0}',
        0xA1..=0xDA | 0xDF..=0xFB => char::from_u32(0x0E01 + (b as u32 - 0xA1)).unwrap_or('\u{FFFD}'),
        _ => '\u{FFFD}',
    }
}

pub fn decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| decode_byte(b)).collect()
}

/// Encode one char; `None` when Windows-874 cannot represent it.
pub fn encode_char(c: char) -> Option<u8> {
    let u = c as u32;
    Some(match u {
        0x00..=0x7F => u as u8,
        0x0E01..=0x0E3A | 0x0E3F..=0x0E5B => (u - 0x0E01 + 0xA1) as u8,
        0x20AC => 0x80,
        0x2026 => 0x85,
        0x2018 => 0x91,
        0x2019 => 0x92,
        0x201C => 0x93,
        0x201D => 0x94,
        0x2022 => 0x95,
        0x2013 => 0x96,
        0x2014 => 0x97,
        0x00A0 => 0xA0,
        _ => return None,
    })
}

/// Encode a string, replacing unrepresentable chars with `?`.
pub fn encode(s: &str) -> Vec<u8> {
    s.chars().map(|c| encode_char(c).unwrap_or(b'?')).collect()
}

/// Thai marks that sit above or below the previous letter and take no
/// horizontal space (MAI HAN-AKAT, upper/lower vowels, tone marks...).
pub fn is_combining(c: char) -> bool {
    matches!(c as u32, 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thai_round_trip() {
        let s = "น้องนอนไม่หลับ ลำไย ไหทองคำ…";
        let bytes = encode(s);
        assert_eq!(bytes[0], 0xB9);
        assert_eq!(*bytes.last().unwrap(), 0x85);
        assert_eq!(decode(&bytes), s);
    }

    #[test]
    fn undefined_bytes() {
        assert_eq!(decode_byte(0xDB), '\u{FFFD}');
        assert_eq!(decode_byte(0xFF), '\u{FFFD}');
        assert!(is_combining('้'));
        assert!(!is_combining('ก'));
        assert!(!is_combining('ำ'));
    }
}
