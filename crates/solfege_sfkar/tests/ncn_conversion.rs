//! Convert the whole sample NCN library in `shared/NCN` and read it back.

use std::path::PathBuf;

use solfege_ncnparser::NcnLibrary;
use solfege_sfkar::{KarSong, convert};

#[test]
fn every_sample_song_converts_and_round_trips() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shared/NCN");
    let Ok(lib) = NcnLibrary::open(&root) else {
        eprintln!("skipped: {} missing", root.display());
        return;
    };
    let mut count = 0;
    for e in lib.complete() {
        let kar = convert(&lib, &e.id).unwrap_or_else(|err| panic!("{}: {err}", e.id));
        let back = KarSong::parse(&kar.to_bytes()).unwrap();
        assert_eq!(back, kar, "{}", e.id);

        let ncn = lib.load(&e.id).unwrap();
        assert_eq!(kar.meta.title, ncn.title);
        assert_eq!(kar.meta.id.as_deref(), Some(e.id.as_str()));
        assert!(kar.midi.starts_with(b"MThd"), "{}: MIDI header", e.id);
        assert_eq!(kar.lyrics.lines.len(), ncn.sung_lines().count(), "{}", e.id);
        // The text survives intact and the timing still never goes back.
        for (l, (_, n)) in kar.lyrics.lines.iter().zip(ncn.sung_lines()) {
            assert_eq!(l.text(), n.text);
            assert!(l.segments.windows(2).all(|w| w[0].tick() < w[1].tick()), "{}: {:?}", e.id, l);
            assert!(l.start() <= l.end);
        }
        let timing = kar.timing().unwrap();
        assert_eq!(timing.ppq, ncn.ppq);
        assert!((timing.duration() - kar.meta.duration).abs() < 0.01);
        count += 1;
    }
    assert_eq!(count, 139);
}

#[test]
fn reading_details_only() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shared/NCN");
    let Ok(lib) = NcnLibrary::open(&root) else { return };
    let kar = convert(&lib, "Z2608001").unwrap();
    let path = std::env::temp_dir().join(format!("sfkar-meta-{}.sfkar", std::process::id()));
    kar.save(&path).unwrap();
    assert_eq!(KarSong::read_meta(&path).unwrap(), kar.meta);
    assert_eq!(KarSong::load(&path).unwrap(), kar);
    std::fs::remove_file(path).ok();
}
