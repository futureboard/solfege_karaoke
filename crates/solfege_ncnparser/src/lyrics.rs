//! `.lyr` files: a 4-line header (title, artist, key, blank) followed by
//! the lyric lines.

use std::path::Path;

use crate::cp874;

/// Number of header lines before the lyric body.
pub const HEADER_LINES: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct LyricLine {
    /// Original bytes; cursor entries index into these.
    pub raw: Vec<u8>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lyrics {
    pub title: String,
    pub artist: String,
    /// Musical key as written by the author (`Bm`, `C#`...), if any.
    pub key: Option<String>,
    pub lines: Vec<LyricLine>,
}

impl Lyrics {
    pub fn parse(bytes: &[u8]) -> Self {
        let mut lines: Vec<&[u8]> = split_lines(bytes);
        let header = |i: usize, lines: &[&[u8]]| lines.get(i).map(|l| cp874::decode(l).trim().to_string()).unwrap_or_default();
        let title = header(0, &lines);
        let artist = header(1, &lines);
        let key = Some(header(2, &lines)).filter(|k| !k.is_empty());
        let body: Vec<&[u8]> = if lines.len() > HEADER_LINES { lines.split_off(HEADER_LINES) } else { Vec::new() };
        let mut body: Vec<LyricLine> =
            body.into_iter().map(|raw| LyricLine { raw: raw.to_vec(), text: cp874::decode(raw) }).collect();
        // Trailing blank lines carry no cursor entries.
        while body.last().is_some_and(|l| l.raw.is_empty()) {
            body.pop();
        }
        Self { title, artist, key, lines: body }
    }

    pub fn load(path: &Path) -> crate::Result<Self> {
        Ok(Self::parse(&crate::error::read(path)?))
    }

    /// Cursor entries this text expects: every body byte plus one per line break.
    pub fn cursor_len(&self) -> usize {
        self.lines.iter().map(|l| l.raw.len() + 1).sum()
    }

    /// Read only the header of a `.lyr` file (title, artist, key).
    pub fn parse_header(bytes: &[u8]) -> (String, String, Option<String>) {
        let lines = split_lines(bytes);
        let get = |i: usize| lines.get(i).map(|l| cp874::decode(l).trim().to_string()).unwrap_or_default();
        let key = get(2);
        (get(0), get(1), Some(key).filter(|k| !k.is_empty()))
    }
}

/// Split on CRLF (or bare LF), keeping empty lines.
fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out: Vec<&[u8]> = bytes.split(|&b| b == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l)).collect();
    // A final newline produces one empty tail element that is not a line.
    if bytes.ends_with(b"\n") {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_body() {
        let text = "Title\r\nArtist\r\nBm\r\n\r\nline one\r\n\r\nline two\r\n\r\n\r\n";
        let l = Lyrics::parse(text.as_bytes());
        assert_eq!((l.title.as_str(), l.artist.as_str(), l.key.as_deref()), ("Title", "Artist", Some("Bm")));
        let texts: Vec<&str> = l.lines.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, ["line one", "", "line two"]);
        assert_eq!(l.cursor_len(), 9 + 1 + 9);
    }

    #[test]
    fn short_and_lf_files() {
        let l = Lyrics::parse(b"Only title");
        assert_eq!(l.title, "Only title");
        assert!(l.key.is_none() && l.lines.is_empty());
        let l = Lyrics::parse(b"T\nA\n\n\nla la\n");
        assert_eq!(l.key, None);
        assert_eq!(l.lines[0].text, "la la");
    }
}
