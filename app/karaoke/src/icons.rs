//! [Lucide](https://lucide.dev) icons (ISC licence), drawn from the bundled
//! `assets/fonts/lucide.ttf`. That file is the lucide-static 1.52.0 font cut
//! down to the code points below; after adding an icon here, regenerate it with
//!
//! ```text
//! pyftsubset lucide.ttf --output-file=app/karaoke/assets/fonts/lucide.ttf \
//!     --unicodes=$(grep -o 'u{e[0-9a-f]*}' app/karaoke/src/icons.rs | tr -d 'u{}' | paste -sd,)
//! ```

pub const PLAY: &str = "\u{e13c}"; // play
pub const PAUSE: &str = "\u{e12e}"; // pause
pub const STOP: &str = "\u{e167}"; // square
pub const NEXT: &str = "\u{e160}"; // skip-forward
pub const SETTINGS: &str = "\u{e154}"; // settings
pub const FULLSCREEN: &str = "\u{e112}"; // maximize
pub const SEARCH: &str = "\u{e151}"; // search
pub const PLUS: &str = "\u{e13d}"; // plus
pub const MINUS: &str = "\u{e11c}"; // minus
pub const QUEUE_ADD: &str = "\u{e23f}"; // list-plus
pub const QUEUE: &str = "\u{e2e0}"; // list-music
pub const CLOSE: &str = "\u{e1b2}"; // x
pub const MOVE_UP: &str = "\u{e04a}"; // arrow-up
pub const TRASH: &str = "\u{e18e}"; // trash-2
pub const MUSIC: &str = "\u{e122}"; // music
pub const MIC: &str = "\u{e349}"; // mic-vocal
pub const DRUM: &str = "\u{e55d}"; // drum
pub const METRONOME: &str = "\u{e6bc}"; // metronome
pub const KEY: &str = "\u{e560}"; // keyboard-music
pub const GAUGE: &str = "\u{e1bf}"; // gauge
pub const VOLUME: &str = "\u{e1ab}"; // volume-2
pub const MUTE: &str = "\u{e1ac}"; // volume-x
pub const SLIDERS: &str = "\u{e29a}"; // sliders-horizontal
pub const FOLDER: &str = "\u{e0d7}"; // folder
pub const FOLDER_OPEN: &str = "\u{e247}"; // folder-open
pub const FOLDER_UP: &str = "\u{e33d}"; // folder-up
pub const FILE_MUSIC: &str = "\u{e55e}"; // file-music
pub const REFRESH: &str = "\u{e145}"; // refresh-cw
pub const CHECK: &str = "\u{e06c}"; // check
pub const ALERT: &str = "\u{e193}"; // triangle-alert
pub const LOADER: &str = "\u{e10a}"; // loader-circle
pub const KEYBOARD: &str = "\u{e284}"; // keyboard
pub const HEADPHONES: &str = "\u{e0f1}"; // headphones
pub const TYPE: &str = "\u{e198}"; // type
pub const TIMER: &str = "\u{e1e0}"; // timer
