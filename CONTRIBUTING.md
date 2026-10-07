# Contributing to Solfege Karaoke

Thanks for wanting to help. This is an **experimental project**: things
move quickly and nothing is stable yet, so small, focused changes and an
issue first for anything large work best.

## Setting up

1. Install Rust 1.95 or newer (`rustup`).
2. On Linux, install the ALSA headers and `pkg-config`
   (`sudo apt install libasound2-dev pkg-config`).
3. Unpack the NCN sample pack into `shared/` (it is git-ignored):

   ```sh
   curl -LO https://cdn.futureboard.studio/NCN.zip
   unzip NCN.zip -d shared
   ```

4. For the synth tests and for hearing anything, install a General MIDI
   SoundFont (`sudo apt install timgm6mb-soundfont`) or put a `.sf2` in
   `shared/`.

Then:

```sh
cargo build --workspace
cargo run -p karaoke
```

## Before you open a pull request

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
```

- **Tests pass.** Tests that need `shared/NCN` or a SoundFont skip
  themselves when it is missing, except the NCN parser tests, which
  require the sample pack. Say in the pull request if you could not run
  some of them.
- **No new clippy warnings** in the code you touched.
- **Behaviour changes come with a test** where one is practical: parsers,
  formats, timing and the synth have unit or integration tests next to
  the code (`#[cfg(test)] mod tests`, or `tests/` in a crate).
- **UI changes come with a screenshot** in the pull request. Screenshots
  for the README live in `docs/screenshots/`.

## Code style

- Write code that reads like the code around it: naming, comment density
  and structure. Comments explain *why*, not *what*.
- There is no project `rustfmt.toml`, and the existing code is not
  rustfmt-formatted. Do not reformat files you are not otherwise
  changing; keep diffs about the change.
- Keep the audio thread free of allocation, locks and I/O
  (`crates/solfege_synth/src/engine`). Hand data over through the command
  channel and free it through the garbage channel.
- The karaoke UI is in Thai. Keep user-facing text Thai and short; code,
  comments, commit messages and docs are in English.
- Icons come from [Lucide](https://lucide.dev) through a subset of its
  font. To add one, add a constant to `app/karaoke/src/icons.rs` and
  regenerate `app/karaoke/assets/fonts/lucide.ttf` with the `pyftsubset`
  command at the top of that file (from `fonttools` and the
  `lucide-static` npm package). Glyphs outside the bundled fonts show up
  as empty boxes.

## Project layout

| Path | Notes |
|---|---|
| `app/karaoke` | The player. `synth.rs` drives the engine, `timeline.rs` times the lyrics, `library.rs` wraps the catalogue, `config.rs` reads and writes `config.json`, `dialog.rs` opens the native file dialogs, `ui/` draws the stage, the bottom bar and the overlay pages. |
| `app/ncn2sfkar` | Converter command line tool. |
| `app/liveinst` | The terminal instrument rack and its web UI (`webui/`, React + Vite). |
| `crates/solfege_ncnparser` | NCN reader. Keep it dependency-free. |
| `crates/solfege_sfkar` | `.sfkar` reader / writer. Format changes need a version bump and a note in the crate docs. |
| `crates/solfege_songdb` | Song catalogue (SQLite, `songs.dat`). |
| `crates/solfege_synth` | Synth engine, SMF player, audio output. |

## Commits and pull requests

- One logical change per commit, with a message that says what changed
  and why (an imperative summary line, then a short body).
- Open the pull request against `main`, describe the change and how you
  tested it, and link the issue if there is one.
- Expect review comments; small follow-up commits are fine.

## Song data and SoundFonts

Do not commit song files, NCN packs or SoundFonts. They have their own
licences and are kept out of the repository on purpose (`shared/` is
ignored).

## Out of scope

Support for proprietary, encrypted or copy-protected karaoke formats
(EMK, XMK, SIB, Sonic Karaoke and the like), code that breaks or bypasses
their protection, and anything aimed at unlicensed song libraries will
not be accepted. See
[Acceptable use](README.md#acceptable-use).

## AI-assisted contributions

Using AI tools is fine; follow [AI_POLICY.md](AI_POLICY.md). In short:
review and test everything yourself, say in the pull request which tools
you used, and do not submit what you could not explain in review.

## Licence

By contributing you agree that your contributions are dual licensed under
the MIT license and the Apache License 2.0, as described in
[README.md](README.md#license), without any additional terms or
conditions.
