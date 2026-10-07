//! Solfege Karaoke: sing along to NCN and .sfkar karaoke songs. The backing MIDI plays
//! through the `solfege_synth` engine with a SoundFont; lyrics come from
//! `solfege_ncnparser` and light up syllable by syllable.

// No console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod gm;
mod icons;
mod library;
mod music;
mod style;
mod synth;
mod timeline;
mod ui;

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use eframe::egui;

use app::{KaraokeApp, Launch};

/// Name of the settings and song catalogue folder.
pub const APP_ID: &str = "solfege-karaoke";

const USAGE: &str = "\
solfege-karaoke - karaoke player for NCN and .sfkar songs

USAGE:
    solfege-karaoke [OPTIONS] [SONG]

    SONG is a song code from the catalogue (e.g. Z2608001), started once
    the library is scanned, or a .sfkar file to play directly.

OPTIONS:
    -L, --library <DIR>     add a song folder: an NCN library (Song, Lyrics,
                            Cursor) or a folder of .sfkar files
    -s, --soundfont <FILE>  SoundFont (.sf2) or SFZ for the backing tracks;
                            repeat for a rack (the first plays every channel
                            until routed otherwise)
    -d, --device <NAME>     audio output device (substring match)
    -l, --list              list audio output devices, then exit
    -h, --help              show this help

The song catalogue, SoundFont rack and settings persist between runs. On
first run `shared/NCN` and the first .sf2 in `shared/` are picked up.
";

fn parse_args() -> Result<Option<Launch>> {
    let mut launch = Launch::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| anyhow!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-l" | "--list" => {
                println!("Audio host: {}", solfege_synth::audio::host_name());
                let default = solfege_synth::audio::default_device();
                for d in solfege_synth::audio::output_devices() {
                    let mark = if Some(&d) == default.as_ref() { "*" } else { " " };
                    println!("  {mark} {d}");
                }
                return Ok(None);
            }
            "-L" | "--library" => launch.library = Some(PathBuf::from(value("--library")?)),
            "-s" | "--soundfont" => launch.soundfonts.push(PathBuf::from(value("--soundfont")?)),
            "-d" | "--device" => launch.device = Some(value("--device")?),
            s if s.starts_with('-') => bail!("unknown option {s}\n\n{USAGE}"),
            _ => launch.song = Some(arg),
        }
    }
    Ok(Some(launch))
}

fn main() -> Result<()> {
    let Some(launch) = parse_args()? else { return Ok(()) };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Solfege Karaoke")
            .with_inner_size([1280.0, 780.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(APP_ID, options, Box::new(|cc| Ok(Box::new(KaraokeApp::new(cc, launch)))))
        .map_err(|e| anyhow!("{e}"))
}
