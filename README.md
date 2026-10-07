# Solfege Karaoke

![status: experimental](https://img.shields.io/badge/status-experimental-orange)
![not for production](https://img.shields.io/badge/production-not%20ready-red)
![license: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)
![rust 1.95+](https://img.shields.io/badge/rust-1.95%2B-brown)

A karaoke player for Thai karaoke songs, written in Rust: NCN song packs
and its own `.sfkar` format, a SoundFont synth for the backing tracks, and
lyrics that light up syllable by syllable.

โปรแกรมคาราโอเกะสำหรับเพลงไทย เล่นเพลง NCN และไฟล์ `.sfkar` ด้วยเสียงจาก
SoundFont พร้อมเนื้อร้องที่ไล่สีทีละพยางค์

> [!WARNING]
> **Experimental project — not for production use.**
> This is a work in progress. The `.sfkar` format, the song catalogue,
> saved settings and every API may change without notice or migration.
> Expect rough edges and missing features; do not rely on it for events,
> venues or anything you cannot afford to have break.

![Lyrics on the stage](docs/screenshots/stage.png)

| | |
|---|---|
| ![Song search](docs/screenshots/songs.png) | ![Mixer](docs/screenshots/mixer.png) |
| Song search overlay (`/`) | Mixer panel (`M`) |
| ![SoundFonts per channel](docs/screenshots/sounds.png) | ![Sounds per instrument](docs/screenshots/instruments.png) |
| Sound settings window (`S`): SoundFont per channel | A sound for any GM instrument |
| ![Count-in](docs/screenshots/count-in.png) | |
| Four-beat count-in after a long rest | |

## What is in here

| Path | What it is |
|---|---|
| [`app/karaoke`](app/karaoke) | **Solfege Karaoke** (`solfege-karaoke`), the egui karaoke player |
| [`app/ncn2sfkar`](app/ncn2sfkar) | `ncn2sfkar`, converts an NCN library to `.sfkar` files |
| [`app/liveinst`](app/liveinst) | `simpletui`, a terminal instrument rack (WAV / SFZ / SF2) with MIDI I/O and a web UI |
| [`crates/solfege_ncnparser`](crates/solfege_ncnparser) | Parser for NCN libraries (`Song/*.mid`, `Lyrics/*.lyr`, `Cursor/*.cur`) |
| [`crates/solfege_sfkar`](crates/solfege_sfkar) | The `.sfkar` song file: read, write, convert from NCN |
| [`crates/solfege_songdb`](crates/solfege_songdb) | Song catalogue over NCN and `.sfkar` folders, with search, favourites and play history |
| [`crates/solfege_synth`](crates/solfege_synth) | Sample-based synth engine (WAV, SFZ, SF2), MIDI file player and audio output |

## The karaoke player

- **Lyric stage.** The line being sung sits in the middle and fills with
  colour per syllable (Thai vowels and tone marks are shaped properly);
  finished lines drift up, the next ones wait below. A title card shows
  before the singing starts and four dots count in, one per beat, after
  a long rest.
- **Command overlay.** Songs, queue, commands and settings live in one
  panel over the stage. Type to filter, arrows to move, Enter to
  act.
- **Song catalogue.** Any number of NCN libraries and `.sfkar` folders in
  one searchable list, with favourites and play counts; songs can be
  queued.
- **Key and tempo.** Change key by semitones (the drums stay put) and
  tempo from 50 % to 150 %; BPM and key are shown live.
- **Mixer.** Its own panel, docked under the lyrics: all 16 MIDI channels
  (channel 10 is the fader for the whole drum kit), each drum-kit piece,
  reverb and chorus returns and master, with gain, pan, mute, solo,
  reverb / chorus sends and meters. Pan and sends start from the song's
  own values (MIDI CC 10, 91, 93) and your adjustment goes on top;
  double-click returns to the song's value. The reverb (room, damping,
  width) and chorus (rate, depth, delay) are adjustable on their return
  strips and remembered between runs.
- **Sound settings window.** SoundFonts, channels, instruments and the
  drum kit have their own window (`S` or the bar button), separate from
  the command overlay.
- **SoundFont rack.** Up to eight SoundFonts (or SFZ instruments). Route
  each MIDI channel to any of them, pin a different sound on a channel,
  or choose the sound of any of the 128 General MIDI instruments, from
  any font, for whichever channel plays it.
- **Drum kit lock.** Lock channel 10 to one kit; songs cannot change it.

### Keyboard

| Key | | Key | |
|---|---|---|---|
| `Space` | play / pause | `/` | search songs |
| `←` `→` | back / forward 5 s | `Q` | queue |
| `[` `]` | key down / up | `M` | mixer panel |
| `,` `.` | slower / faster | `S` | sound settings window |
| `N` | next song in the queue | `Ctrl K` | all commands |
| `F` / `F11` | full screen (`Esc` leaves) | `Ctrl ,` | settings |

In the overlay: `Enter` reserves a song, `Shift Enter` sings it now,
`Ctrl D` marks a favourite, `Tab` switches page and `Esc` closes.

## Getting started

### Requirements

- Rust **1.95** or newer (edition 2024).
- **Linux:** ALSA headers and `pkg-config` to build
  (`sudo apt install libasound2-dev pkg-config` on Debian / Ubuntu), and
  OpenGL plus `libxkbcommon-x11` to run.
- **Windows:** nothing extra (audio goes through WASAPI).
- macOS is untested.

### Songs and sounds

Neither songs nor SoundFonts are part of the repository.

- **Sample songs.** An NCN sample pack is used for development and tests.
  Unpack it into `shared/` so that `shared/NCN/Song`, `shared/NCN/Lyrics`
  and `shared/NCN/Cursor` exist:

  ```sh
  curl -LO https://cdn.futureboard.studio/NCN.zip
  unzip NCN.zip -d shared
  ```

- **SoundFont.** Any General MIDI `.sf2` works (for example the
  `timgm6mb-soundfont` or `fluid-soundfont-gm` packages on Debian /
  Ubuntu). The player looks in `shared/`, next to the executable and in
  `/usr/share/sounds/sf2`, or you can add one in the sound settings window.

### Run

```sh
cargo run --release -p karaoke                     # the karaoke player
cargo run --release -p karaoke -- Z2608001         # start a song by its code
cargo run --release -p karaoke -- song.sfkar       # play a .sfkar file directly
cargo run --release -p karaoke -- -s gm.sf2 -s piano.sfz -L /path/to/NCN
cargo run --release -p karaoke -- --help
```

The song catalogue and settings are kept in the app's data folder
(`~/.local/share/solfege-karaoke` on Linux,
`%APPDATA%\solfege-karaoke\data` on Windows).

### Convert NCN to `.sfkar`

```sh
cargo run --release -p ncn2sfkar -- shared/NCN                 # all songs -> shared/NCN-sfkar/
cargo run --release -p ncn2sfkar -- shared/NCN -o out Z2608001 # some songs
cargo run --release -p ncn2sfkar -- shared/NCN --dry-run       # check only
```

Existing files are kept unless `--force` is given; the exit code is
non-zero if any song failed.

## The `.sfkar` format

One file per song, so a song no longer needs three parallel folders and
its text is UTF-8 instead of Windows-874:

```text
"SFKR"  u16 version (1)  u16 flags (0)
chunks: 4-byte id, u32 length (little endian), data
  META  JSON: title, artist, key, source id, duration
  MIDI  a Standard MIDI File (the backing track)
  LYRC  JSON: lines of [tick, text] segments with an end tick
```

Lyric times are ticks of the MIDI chunk, so they follow its tempo map and
any playback speed. Readers skip chunks they do not know. The full
description is in [`crates/solfege_sfkar`](crates/solfege_sfkar/src/lib.rs).
As the project is experimental, the format may still change.

## Development

```sh
cargo test --workspace      # needs shared/NCN; a few tests also use a .sf2
cargo clippy --workspace --all-targets
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the details.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.

### Third-party assets

The karaoke player bundles fonts under their own licences:

- Noto Sans Thai and Noto Sans, SIL Open Font License 1.1
  ([`app/karaoke/assets/fonts/OFL.txt`](app/karaoke/assets/fonts/OFL.txt))
- Lucide icons, ISC License
  ([`app/karaoke/assets/fonts/LUCIDE-LICENSE.txt`](app/karaoke/assets/fonts/LUCIDE-LICENSE.txt))

Song data, SoundFonts and the NCN sample pack are not covered by this
repository's licence; use them according to their own terms.
