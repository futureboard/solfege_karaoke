//! Parser tests against the real NCN sample library in `shared/NCN`.

use std::path::PathBuf;

use solfege_ncnparser::{Alignment, Cursor, Lyrics, NcnLibrary};

fn root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shared/NCN");
    assert!(p.is_dir(), "sample library missing: {}", p.display());
    p
}

fn library() -> NcnLibrary {
    NcnLibrary::open(root()).expect("open shared/NCN")
}

#[test]
fn scans_all_three_folders() {
    let lib = library();
    assert_eq!(lib.len(), 139);
    assert_eq!(lib.complete().count(), 139, "every song has .mid, .lyr and .cur");
    // Upper-case extensions (.Cur, .Lyr, .MID) are picked up too.
    let e = lib.get("z2608020").expect("lookup ignores case");
    assert!(e.cursor.as_ref().unwrap().to_string_lossy().ends_with(".Cur"));
}

#[test]
fn known_song_header_and_timing() {
    let song = library().load("Z2608001").unwrap();
    assert_eq!(song.title, "น้องนอนไม่หลับ cover");
    assert_eq!(song.artist, "ลำไย ไหทองคำ");
    assert_eq!(song.key.as_deref(), Some("Bm"));
    assert_eq!(song.ppq, 480);
    assert_eq!(song.lines[0].text, "น้องนอนไม่หลับ-ลำไย ไหทองคำ");
    // First cursor value is 16 -> 16 * 480 / 24 ticks.
    assert_eq!(song.lines[0].clusters[0].tick, 320);
    assert_eq!(song.lines[0].clusters[0].text, "น้");
    assert_eq!(song.alignment, Alignment::CursorLonger { extra: 25 });
    assert_eq!(song.backsteps, 0);
    let last = song.lines.last().unwrap();
    assert!(last.text.contains("จบเพลง"));
    assert!(song.seconds(last.end) <= song.duration() + 1.0);
}

#[test]
fn every_song_parses_with_sane_timing() {
    let lib = library();
    let mut exact = 0;
    for e in lib.complete() {
        let song = lib.load(&e.id).unwrap_or_else(|err| panic!("{}: {err}", e.id));
        assert!(!song.title.is_empty(), "{}: empty title", e.id);
        assert!(song.sung_lines().count() > 3, "{}: too few lyric lines", e.id);
        assert!(song.ppq > 0);
        // Highlight times never go backwards across the whole song.
        let ticks: Vec<u32> = song.lines.iter().flat_map(|l| l.clusters.iter().map(|c| c.tick)).collect();
        assert!(ticks.windows(2).all(|w| w[0] <= w[1]), "{}: non-monotonic", e.id);
        for l in &song.lines {
            assert!(l.start <= l.end, "{}: line ends before it starts", e.id);
        }
        // Lyrics finish inside the backing track (a little slack for the outro).
        let last = song.lines.last().unwrap().end;
        assert!(
            song.seconds(last) <= song.duration() + 5.0,
            "{}: lyrics end {:.1}s after a {:.1}s song",
            e.id,
            song.seconds(last),
            song.duration()
        );
        if song.alignment == Alignment::Exact {
            exact += 1;
        }
    }
    // 115 of the 139 sample songs line up byte-for-byte with their cursor.
    assert!(exact >= 110, "only {exact} exact alignments");
}

#[test]
fn cursor_length_rule_holds_for_most_files() {
    let lib = library();
    let mut matched = 0;
    let mut total = 0;
    for e in lib.complete() {
        let lyr = Lyrics::load(e.lyrics.as_ref().unwrap()).unwrap();
        let cur = Cursor::load(e.cursor.as_ref().unwrap()).unwrap();
        total += 1;
        if cur.len() == lyr.cursor_len() {
            matched += 1;
        }
    }
    assert_eq!(total, 139);
    assert!(matched * 100 / total >= 80, "{matched}/{total}");
}

#[test]
fn odd_length_cursor_is_tolerated() {
    let lib = library();
    let odd: Vec<_> = lib
        .complete()
        .filter(|e| std::fs::metadata(e.cursor.as_ref().unwrap()).unwrap().len() % 2 == 1)
        .collect();
    assert!(!odd.is_empty(), "sample set includes an odd-length cursor");
    for e in odd {
        let cur = Cursor::load(e.cursor.as_ref().unwrap()).unwrap();
        assert!(cur.truncated_byte);
        lib.load(&e.id).unwrap();
    }
}

#[test]
fn search_by_artist_and_title() {
    let lib = library();
    let hits = lib.search("ลำไย");
    assert!(hits.iter().any(|h| h.id == "Z2608001"));
    let headers = lib.headers();
    assert_eq!(headers.len(), 139);
    assert!(headers.iter().filter(|h| h.key.is_some()).count() > 100);
}

#[test]
fn progress_follows_the_song() {
    let song = library().load("Z2608001").unwrap();
    let (i, line) = song.sung_lines().nth(3).unwrap();
    let mid = line.clusters[line.clusters.len() / 2].tick;
    let p = song.progress(mid);
    assert_eq!(p.line, i);
    assert!(p.clusters > 0 && p.clusters <= line.clusters.len());
    let lrc = song.to_lrc(false);
    assert!(lrc.starts_with("[ti:น้องนอนไม่หลับ cover]"));
}
