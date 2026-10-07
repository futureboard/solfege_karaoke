//! Print an NCN song with timestamps, or list a library.
//!
//! ```text
//! cargo run -p solfege_ncnparser --example dump -- shared/NCN            # list
//! cargo run -p solfege_ncnparser --example dump -- shared/NCN Z2608001   # timed lyrics
//! cargo run -p solfege_ncnparser --example dump -- shared/NCN Z2608001 --lrc
//! cargo run -p solfege_ncnparser --example dump -- shared/NCN --stats
//! ```

use solfege_ncnparser::NcnLibrary;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args.first().map(String::as_str).unwrap_or("shared/NCN");
    let lib = NcnLibrary::open(root)?;
    if args.get(1).map(String::as_str) == Some("--stats") {
        let (mut exact, mut longer, mut shorter, mut rejected, mut locked) = (0, 0, 0, 0, 0);
        for e in lib.complete() {
            let song = lib.load(&e.id)?;
            match song.alignment {
                solfege_ncnparser::Alignment::Exact => exact += 1,
                solfege_ncnparser::Alignment::CursorLonger { .. } => longer += 1,
                solfege_ncnparser::Alignment::CursorShorter { .. } => shorter += 1,
            }
            rejected += (song.rejected > 0) as usize;
            if let Some(m) = &song.midi_path {
                locked += solfege_ncnparser::MidiInfo::load(m)?.locked as usize;
            }
        }
        println!("exact {exact}, cursor longer {longer}, cursor shorter {shorter}");
        println!("songs with rejected cursor entries {rejected}, Lock-header MIDI {locked}");
        return Ok(());
    }
    let Some(id) = args.get(1) else {
        for h in lib.headers() {
            println!("{:<10} {:<4} {} — {}", h.id, h.key.unwrap_or_default(), h.title, h.artist);
        }
        println!("{} songs ({} complete)", lib.len(), lib.complete().count());
        return Ok(());
    };
    let song = lib.load(id)?;
    if args.iter().any(|a| a == "--lrc") {
        print!("{}", song.to_lrc(args.iter().any(|a| a == "--enhanced")));
        return Ok(());
    }
    println!("{} — {}  key {}  ppq {}", song.title, song.artist, song.key.as_deref().unwrap_or("-"), song.ppq);
    println!(
        "alignment {:?}, {} backsteps, {} rejected, {:.1}s",
        song.alignment,
        song.backsteps,
        song.rejected,
        song.duration()
    );
    for line in &song.lines {
        println!("{:>8.2}s  {}", song.seconds(line.start), line.text);
    }
    Ok(())
}
