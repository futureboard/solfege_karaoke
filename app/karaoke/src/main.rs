//! Solfege Karaoke: sing along to NCN karaoke songs. The backing MIDI plays
//! through the `solfege_synth` engine with a SoundFont; lyrics come from
//! `solfege_ncnparser` and light up syllable by syllable.

// No console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
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

const USAGE: &str = "\
solfege-karaoke - NCN karaoke player

USAGE:
    solfege-karaoke [OPTIONS] [SONG_ID]

    SONG_ID starts that song once the library is loaded (e.g. Z2608001).

OPTIONS:
    -L, --library <DIR>     NCN library folder (holds Song, Lyrics, Cursor)
    -s, --soundfont <FILE>  SoundFont (.sf2) for the backing tracks
    -d, --device <NAME>     audio output device (substring match)
    -l, --list              list audio output devices, then exit
    -h, --help              show this help

Without options the last used library and SoundFont are reopened; on first
run `shared/NCN` and any .sf2 in `shared/` are picked up.
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
            "-s" | "--soundfont" => launch.soundfont = Some(PathBuf::from(value("--soundfont")?)),
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
    eframe::run_native("solfege-karaoke", options, Box::new(|cc| Ok(Box::new(KaraokeApp::new(cc, launch)))))
        .map_err(|e| anyhow!("{e}"))
}
