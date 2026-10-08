//! # solfege_ncnparser
//!
//! Parser for **NCN karaoke** libraries, the format used by Thai karaoke
//! software (NCN song packs). A library is
//! three parallel folder trees keyed by song id:
//!
//! ```text
//! NCN/
//!   Song/Z/Z2608001.mid     standard MIDI file (the backing track)
//!   Lyrics/Z/Z2608001.lyr   Windows-874 / TIS-620 text, CRLF lines
//!   Cursor/Z/Z2608001.cur   little-endian u16 timing per lyric byte
//! ```
//!
//! **Lyrics** (`.lyr`): line 1 title, line 2 artist, line 3 key (may be
//! empty), line 4 blank, then one lyric line per display line.
//!
//! **Cursor** (`.cur`): one `u16` per byte of the lyric body, plus one entry
//! for each line break. Values are in 1/24 of a quarter note, so
//! `tick = value * ppq / 24` with `ppq` from the MIDI header. Real files are
//! a little sloppy: values can step backwards inside a word, and the array
//! may be a few entries longer or shorter than the text. Mapping is
//! sequential from the start; extra entries are ignored, missing ones reuse
//! the last time, and highlight times are clamped to never go backwards.
//!
//! ```no_run
//! use solfege_ncnparser::NcnLibrary;
//!
//! let lib = NcnLibrary::open("shared/NCN")?;
//! let song = lib.load("Z2608001")?;
//! println!("{} - {} ({})", song.title, song.artist, song.key.as_deref().unwrap_or("?"));
//! for line in &song.lines {
//!     println!("[{:>7.2}s] {}", song.seconds(line.start), line.text);
//! }
//! # Ok::<(), solfege_ncnparser::Error>(())
//! ```

pub mod cp874;
pub mod cursor;
mod error;
pub mod library;
pub mod lyrics;
pub mod midi;
pub mod song;

pub use cursor::{CURSOR_RESOLUTION, Cursor};
pub use error::Error;
pub use library::{Entry, NcnLibrary, SongHeader};
pub use lyrics::{LyricLine, Lyrics};
pub use midi::{Meter, MidiInfo, MidiText, TempoMap};
pub use song::{Alignment, Cluster, NcnSong, Progress, TimedLine};

pub type Result<T> = std::result::Result<T, Error>;
