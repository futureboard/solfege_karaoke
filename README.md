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

> [!IMPORTANT]
> **No support for proprietary or pirated karaoke formats.** This project
> does not support EMK, XMK, SIB, Sonic Karaoke or similar formats, nor
> the use of its code to read them or to play unlicensed songs. See
> [Acceptable use](#acceptable-use).
>
> **ไม่สนับสนุนฟอร์แมตคาราโอเกะเชิงพาณิชย์หรือเพลงละเมิดลิขสิทธิ์** โปรเจกต์นี้
> ไม่รองรับ EMK, XMK, SIB, Sonic Karaoke หรือฟอร์แมตลักษณะเดียวกัน และไม่สนับสนุน
> การนำโค้ดไปใช้อ่านไฟล์เหล่านั้นหรือเล่นเพลงที่ไม่ได้รับอนุญาต ดู
> [ข้อตกลงการใช้งาน](#acceptable-use)

![Lyrics on the stage](docs/screenshots/stage.png)

| | |
|---|---|
| ![Song search](docs/screenshots/songs.png) | ![Mixer](docs/screenshots/mixer.png) |
| Song search overlay (`/`) | Mixer panel (`M`) |
| ![SoundFonts per channel](docs/screenshots/sounds.png) | ![Sounds per instrument](docs/screenshots/instruments.png) |
| Sound settings window (`S`): SoundFont per channel | A sound for any GM instrument |
| ![Count-in](docs/screenshots/count-in.png) | ![Context menu](docs/screenshots/context-menu.png) |
| Four-beat count-in after a long rest | Right-click menu on the stage |
| ![Classic lyrics](docs/screenshots/classic.png) | ![About](docs/screenshots/about.png) |
| Classic layout: two lines that take turns (`L`) | About page |
| ![A kit per drum piece](docs/screenshots/drum-pieces.png) | |
| Kick, snare, ... each from its own SoundFont and kit | |

## What is in here

| Path | What it is |
|---|---|
| [`app/karaoke`](app/karaoke) | **Solfege Karaoke** (`solfege-karaoke`), the egui karaoke player |
| [`app/ncn2sfkar`](app/ncn2sfkar) | `ncn2sfkar`, converts an NCN library to `.sfkar` files |
| [`app/liveinst`](app/liveinst) | `simpletui`, a terminal instrument rack (WAV / SFZ / SF2) with MIDI I/O and a web UI |
| [`crates/solfege_ncnparser`](crates/solfege_ncnparser) | Parser for NCN libraries (`Song/*.mid`, `Lyrics/*.lyr`, `Cursor/*.cur`) |
| [`crates/solfege_sfkar`](crates/solfege_sfkar) | The `.sfkar` song file: read, write, convert from NCN |
| [`crates/solfege_songdb`](crates/solfege_songdb) | Song catalogue (SQLite) over NCN and `.sfkar` folders, with search, favourites and play history |
| [`crates/solfege_synth`](crates/solfege_synth) | Sample-based synth engine (WAV, SFZ, SF2), MIDI file player and audio output |

## The karaoke player

- **Lyric stage.** Lines fill with colour per syllable (Thai vowels and
  tone marks are shaped properly), in one of two layouts (`L`):
  *scroll*, where the line being sung sits in the middle and finished
  lines drift up, or *classic*, two fixed lines in the middle that take
  turns. A title card shows before the singing starts, four dots count
  in after a long rest, and the time of day sits in the corner.
- **Guide melody off** (`V`). Mutes MIDI channel 9, where NCN songs
  carry the vocal melody, in every song until turned back on.
- **Command overlay.** Songs, queue, commands and settings live in one
  panel over the stage.
- **Context menus.** Right click the stage or the bottom bar for
  playback, key and tempo, every panel and full screen; right click a
  song, a queued song, a mixer strip, a SoundFont or a channel for what
  applies to it. Each entry shows its keyboard shortcut.
- **Full screen** (`F`, `F11` or double-click the stage) fills the screen
  and keeps everything: bottom bar, mixer, overlays and windows. Type to filter, arrows to move, Enter to
  act.
- **Song catalogue.** Any number of NCN libraries and `.sfkar` folders in
  one searchable list, with favourites and play counts, kept in an SQLite
  database (`songs.dat`); songs can be queued.
- **Native file dialogs.** SoundFonts and song folders are picked with
  the system's own dialog: the common item dialog on Windows, NSOpenPanel
  on macOS and the XDG desktop portal on Linux (the desktop's file chooser
  on Wayland or X11, `zenity` when no portal runs).
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
- **A kit per drum piece.** Kick, snare, hi-hat, toms, cymbals and
  percussion can each play from a kit of their own, from any SoundFont;
  the pieces left alone play from channel 10's kit. Their mixer strips
  work the same either way.

### Keyboard

| Key | | Key | |
|---|---|---|---|
| `Space` | play / pause | `/` | search songs |
| `Shift Space` | stop (back to the start) | | |
| `←` `→` | back / forward 5 s | `Q` | queue |
| `[` `]` | key down / up | `M` | mixer panel |
| `,` `.` | slower / faster | `S` | sound settings window |
| `V` | guide melody (channel 9) on / off | `L` | lyric layout |
| `N` | next song in the queue | `Ctrl K` | all commands |
| `F` / `F11` | full screen (`Esc` leaves) | `Ctrl ,` | settings |
| right click | context menu | | |

In the overlay: `Enter` reserves a song, `Shift Enter` sings it now,
`Ctrl D` marks a favourite, `Tab` switches page and `Esc` closes.

## Getting started

### Requirements

- Rust **1.95** or newer (edition 2024), and a C compiler (SQLite is
  built from source by `rusqlite`).
- **Linux:** ALSA headers and `pkg-config` to build
  (`sudo apt install libasound2-dev pkg-config` on Debian / Ubuntu), and
  OpenGL plus `libxkbcommon-x11` to run. File dialogs need
  `xdg-desktop-portal` (any desktop has it) or, without one, `zenity`.
- **Windows:** nothing extra (audio goes through WASAPI).
- macOS is untested.
- Graphics go through **wgpu** (Vulkan, Metal or Direct3D 12, with
  OpenGL as a fallback), so any GPU from the last decade, or a software
  renderer such as llvmpipe, works.

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

Settings and the song catalogue are kept in the app's data folder
(`~/.local/share/solfege-karaoke` on Linux,
`%APPDATA%\solfege-karaoke\data` on Windows; both paths are also shown
under Settings):

| File | What it holds |
|---|---|
| `config.json` | Settings as readable JSON: SoundFont rack and routing, sounds per instrument, drum kit lock, reverb / chorus, audio device, lyric size and offset. Edit it while the player is closed; missing fields take their defaults. `--config <FILE>` uses another file. |
| `songs.dat` | The song catalogue, an SQLite database: song folders, songs, favourites and play history. |

Settings and catalogues of older versions (eframe's `app.ron`,
`songs.json`) are imported on the first start.

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

<a id="acceptable-use"></a>

## Acceptable use / ข้อตกลงการใช้งาน

**English.** Solfege Karaoke plays songs you have the right to use. We do
not support, and will not accept contributions for, using this source
code in any way to:

- read, convert, play or otherwise support proprietary, encrypted or
  copy-protected karaoke formats, including **EMK, XMK, SIB and Sonic
  Karaoke** files, or to break or bypass their encryption or protection;
- play, convert, copy or distribute song libraries obtained without the
  permission of their rights holders.

Issues and pull requests asking for such support will be closed. Anyone
who uses or modifies this code for these purposes does so entirely on
their own responsibility; the authors and contributors accept no
liability of any kind for it. The software is provided "as is", without
warranty, as stated in both licences below. This notice is not legal
advice; check the law where you live.

**ภาษาไทย** Solfege Karaoke มีไว้เล่นเพลงที่ผู้ใช้มีสิทธิ์ใช้งานเท่านั้น
เราไม่สนับสนุน และจะไม่รับการมีส่วนร่วม (contribution) ใด ๆ ที่นำซอร์สโค้ดนี้ไปใช้
ไม่ว่าในลักษณะใดก็ตาม เพื่อ

- อ่าน แปลง เล่น หรือรองรับฟอร์แมตคาราโอเกะเชิงพาณิชย์ที่เข้ารหัสหรือมีระบบป้องกัน
  การคัดลอก รวมถึงไฟล์ **EMK, XMK, SIB และ Sonic Karaoke** หรือเพื่อถอดรหัส
  หรือหลบเลี่ยงระบบป้องกันของไฟล์เหล่านั้น
- เล่น แปลง คัดลอก หรือเผยแพร่คลังเพลงที่ได้มาโดยไม่ได้รับอนุญาตจากเจ้าของลิขสิทธิ์

Issue และ pull request ที่ขอให้รองรับสิ่งเหล่านี้จะถูกปิด ผู้ที่นำโค้ดไปใช้หรือดัดแปลง
เพื่อวัตถุประสงค์ดังกล่าวต้องรับผิดชอบการกระทำของตนเองแต่เพียงผู้เดียว ผู้พัฒนาและ
ผู้มีส่วนร่วมจะไม่รับผิดชอบใด ๆ ทั้งสิ้น ซอฟต์แวร์นี้ให้ไว้ "ตามสภาพ" (as is) โดยไม่มี
การรับประกันตามสัญญาอนุญาตทั้งสองฉบับด้านล่าง ข้อความนี้ไม่ใช่คำแนะนำทางกฎหมาย
โปรดตรวจสอบกฎหมายในประเทศของคุณ

## AI policy

Contributions written with AI assistants are welcome under the rules in
[AI_POLICY.md](AI_POLICY.md) (English / ไทย): you review, test and answer
for every line, say which tools you used, and keep the licensing clean.

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
