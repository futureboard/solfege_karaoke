//! Key names under transposition.

const MAJOR: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];
const MINOR: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "G#", "A", "Bb", "B"];

/// Pitch class of a note name such as `C`, `F#`, `Bb` (also `♯` / `♭`),
/// and the rest of the string.
fn root(key: &str) -> Option<(i32, &str)> {
    let letter = key.chars().next()?;
    let mut pc: i32 = match letter.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let mut rest = &key[letter.len_utf8()..];
    while let Some(c) = rest.chars().next() {
        match c {
            '#' | '♯' => pc += 1,
            'b' | '♭' => pc -= 1,
            _ => break,
        }
        rest = &rest[c.len_utf8()..];
    }
    Some((pc.rem_euclid(12), rest))
}

/// `key` moved by `semitones`, spelled the way karaoke charts usually
/// write it (`Bm` + 2 = `C#m`, `G` - 1 = `F#`). `None` when the key is not
/// a note name.
pub fn transpose_key(key: &str, semitones: i32) -> Option<String> {
    let key = key.trim();
    let (pc, rest) = root(key)?;
    let minor = rest.starts_with('m') && !rest.starts_with("maj");
    let names = if minor { &MINOR } else { &MAJOR };
    Some(format!("{}{rest}", names[(pc + semitones).rem_euclid(12) as usize]))
}

/// `+2`, `-1`, `0`.
pub fn signed(n: i32) -> String {
    if n > 0 { format!("+{n}") } else { n.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transposes_major_and_minor_keys() {
        assert_eq!(transpose_key("Bm", 2).as_deref(), Some("C#m"));
        assert_eq!(transpose_key("Bm", 0).as_deref(), Some("Bm"));
        assert_eq!(transpose_key("G", -1).as_deref(), Some("F#"));
        assert_eq!(transpose_key("C", 1).as_deref(), Some("Db"));
        assert_eq!(transpose_key("Ebm", 12).as_deref(), Some("Ebm"));
        assert_eq!(transpose_key("F#m", -2).as_deref(), Some("Em"));
        assert_eq!(transpose_key("Bbmaj7", 2).as_deref(), Some("Cmaj7"));
        assert_eq!(transpose_key(" a ", 3).as_deref(), Some("C"));
        assert_eq!(transpose_key("?", 1), None);
        assert_eq!(transpose_key("", 1), None);
    }

    #[test]
    fn signed_numbers() {
        assert_eq!(signed(2), "+2");
        assert_eq!(signed(0), "0");
        assert_eq!(signed(-3), "-3");
    }
}
