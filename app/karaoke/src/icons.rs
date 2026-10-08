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
pub const QUEUE: &str = "\u{e2e0}"; // list-music
pub const MOVE_UP: &str = "\u{e04a}"; // arrow-up
pub const TRASH: &str = "\u{e18e}"; // trash-2
pub const MIC: &str = "\u{e349}"; // mic-vocal
pub const DRUM: &str = "\u{e55d}"; // drum
pub const METRONOME: &str = "\u{e6bc}"; // metronome
pub const KEY: &str = "\u{e560}"; // keyboard-music
pub const VOLUME: &str = "\u{e1ab}"; // volume-2
pub const FOLDER_OPEN: &str = "\u{e247}"; // folder-open
pub const FILE_MUSIC: &str = "\u{e55e}"; // file-music
pub const REFRESH: &str = "\u{e145}"; // refresh-cw
pub const CHECK: &str = "\u{e06c}"; // check
pub const ALERT: &str = "\u{e193}"; // triangle-alert
pub const LOADER: &str = "\u{e10a}"; // loader-circle
pub const KEYBOARD: &str = "\u{e284}"; // keyboard
pub const HEADPHONES: &str = "\u{e0f1}"; // headphones
pub const TYPE: &str = "\u{e198}"; // type
pub const MOVE_DOWN: &str = "\u{e042}"; // arrow-down
pub const ENTER: &str = "\u{e0a1}"; // corner-down-left
pub const COMMAND: &str = "\u{e09a}"; // command
pub const RESTART: &str = "\u{e148}"; // rotate-ccw
pub const STAR: &str = "\u{e176}"; // star
pub const MIXER: &str = "\u{e162}"; // sliders-vertical
pub const FOLDER_PLUS: &str = "\u{e0d9}"; // folder-plus
pub const DATABASE: &str = "\u{e0ad}"; // database
pub const REMOVE: &str = "\u{e1b2}"; // x
pub const LOCK: &str = "\u{e10b}"; // lock
pub const GUITAR: &str = "\u{e55f}"; // guitar
pub const EXIT_FULLSCREEN: &str = "\u{e11a}"; // minimize
pub const COPY: &str = "\u{e09e}"; // copy
pub const ENQUEUE: &str = "\u{e23f}"; // list-plus
pub const PLAY_NEXT: &str = "\u{e2e1}"; // list-start
pub const CLEAR_QUEUE: &str = "\u{e240}"; // list-x
pub const STAR_OFF: &str = "\u{e2b0}"; // star-off
pub const UNDO: &str = "\u{e2a1}"; // undo-2
pub const MIC_OFF: &str = "\u{e119}"; // mic-off
pub const CLOCK: &str = "\u{e087}"; // clock
pub const INFO: &str = "\u{e0f9}"; // info
pub const LYRICS_SCROLL: &str = "\u{e4d1}"; // gallery-vertical
pub const LYRICS_CLASSIC: &str = "\u{e439}"; // rows-2
pub const MONITOR: &str = "\u{e11d}"; // monitor
pub const SECOND_SCREEN: &str = "\u{e195}"; // tv
pub const SECOND_SCREEN_OFF: &str = "\u{e1dc}"; // monitor-off
pub const IMAGE: &str = "\u{e0f6}"; // image
pub const IMAGES: &str = "\u{e5c4}"; // images
pub const SHUFFLE: &str = "\u{e15e}"; // shuffle
pub const LINK: &str = "\u{e0b9}"; // external-link
pub const EFFECTS: &str = "\u{e412}"; // sparkles
pub const POWER: &str = "\u{e140}"; // power
pub const COLLAPSED: &str = "\u{e06f}"; // chevron-right
pub const PREVIOUS: &str = "\u{e06e}"; // chevron-left
