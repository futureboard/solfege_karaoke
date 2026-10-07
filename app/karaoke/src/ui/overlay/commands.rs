//! Everything the app can do, as commands to search and run.

use eframe::egui;
use solfege_synth::engine::PlayState;

use super::{Outcome, Overlay, Page, Target};
use crate::app::KaraokeApp;
use crate::icons;
use crate::music::{signed, transpose_key};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cmd {
    PlayPause,
    Restart,
    Stop,
    Next,
    KeyUp,
    KeyDown,
    KeyReset,
    Faster,
    Slower,
    SpeedReset,
    Fullscreen,
    Songs,
    Queue,
    ClearQueue,
    Mixer,
    ResetMixer,
    Favorite,
    Settings,
    OpenLibrary,
    Rescan,
    ChooseSoundFont,
    Sounds,
}

pub const ALL: [Cmd; 22] = [
    Cmd::PlayPause,
    Cmd::Restart,
    Cmd::Stop,
    Cmd::Next,
    Cmd::KeyUp,
    Cmd::KeyDown,
    Cmd::KeyReset,
    Cmd::Faster,
    Cmd::Slower,
    Cmd::SpeedReset,
    Cmd::Fullscreen,
    Cmd::Songs,
    Cmd::Queue,
    Cmd::ClearQueue,
    Cmd::Mixer,
    Cmd::ResetMixer,
    Cmd::Favorite,
    Cmd::Settings,
    Cmd::OpenLibrary,
    Cmd::Rescan,
    Cmd::ChooseSoundFont,
    Cmd::Sounds,
];

impl Cmd {
    pub fn icon(self, app: &KaraokeApp) -> &'static str {
        match self {
            Cmd::PlayPause if app.synth.state() == PlayState::Playing => icons::PAUSE,
            Cmd::PlayPause => icons::PLAY,
            Cmd::Restart => icons::RESTART,
            Cmd::Stop => icons::STOP,
            Cmd::Next => icons::NEXT,
            Cmd::KeyUp | Cmd::KeyDown | Cmd::KeyReset => icons::KEY,
            Cmd::Faster | Cmd::Slower | Cmd::SpeedReset => icons::METRONOME,
            Cmd::Fullscreen => icons::FULLSCREEN,
            Cmd::Songs => icons::SEARCH,
            Cmd::Queue => icons::QUEUE,
            Cmd::ClearQueue => icons::TRASH,
            Cmd::Mixer | Cmd::ResetMixer => icons::MIXER,
            Cmd::Favorite => icons::STAR,
            Cmd::Settings => icons::SETTINGS,
            Cmd::OpenLibrary | Cmd::Rescan => icons::FOLDER_OPEN,
            Cmd::ChooseSoundFont | Cmd::Sounds => icons::FILE_MUSIC,
        }
    }

    pub fn label(self, app: &KaraokeApp) -> String {
        let key = app.synth.key();
        let key_name = |k: i32| {
            let song_key = app.now.as_ref().and_then(|n| n.song.meta.key.as_deref());
            match song_key.and_then(|s| transpose_key(s, k)) {
                Some(name) => format!("{name} ({})", signed(k)),
                None => signed(k),
            }
        };
        let speed = |s: f64| format!("{:.0}%", s * 100.0);
        match self {
            Cmd::PlayPause if app.synth.state() == PlayState::Playing => "พักเพลง".into(),
            Cmd::PlayPause => "เล่นเพลง".into(),
            Cmd::Restart => "ร้องใหม่ตั้งแต่ต้น".into(),
            Cmd::Stop => "หยุด".into(),
            Cmd::Next => match app.queue.front() {
                Some(h) => format!("เพลงถัดไป: {}", h.title),
                None => "เพลงถัดไป (คิวว่าง)".into(),
            },
            Cmd::KeyUp => format!("เพิ่มคีย์  ·  {}", key_name(key + 1)),
            Cmd::KeyDown => format!("ลดคีย์  ·  {}", key_name(key - 1)),
            Cmd::KeyReset => format!("คีย์เดิม  ·  {}", key_name(0)),
            Cmd::Faster => format!("เร็วขึ้น  ·  {}", speed(app.synth.speed() + 0.05)),
            Cmd::Slower => format!("ช้าลง  ·  {}", speed(app.synth.speed() - 0.05)),
            Cmd::SpeedReset => "ความเร็วปกติ (100%)".into(),
            Cmd::Fullscreen if app.stage_only => "ออกจากเต็มจอ".into(),
            Cmd::Fullscreen => "เต็มจอ (เฉพาะเนื้อร้อง)".into(),
            Cmd::Songs => "ค้นหาเพลง".into(),
            Cmd::Queue => format!("ดูคิวเพลง ({})", app.queue.len()),
            Cmd::ClearQueue => "ล้างคิว".into(),
            Cmd::Mixer => "มิกเซอร์".into(),
            Cmd::ResetMixer => "รีเซ็ตมิกเซอร์แชนแนลของเพลงนี้".into(),
            Cmd::Favorite => match &app.now {
                Some(n) if app.library.is_favorite(&n.entry.uid) => format!("เอาออกจากเพลงโปรด: {}", n.entry.title),
                Some(n) => format!("เพิ่มในเพลงโปรด: {}", n.entry.title),
                None => "เพลงโปรด".into(),
            },
            Cmd::Settings => "ตั้งค่า".into(),
            Cmd::OpenLibrary => "เพิ่มโฟลเดอร์เพลง (NCN / .sfkar)…".into(),
            Cmd::Rescan => "สแกนคลังเพลงใหม่".into(),
            Cmd::ChooseSoundFont => "เพิ่ม SoundFont / SFZ…".into(),
            Cmd::Sounds => "เสียง: SoundFont ของแต่ละแชนแนล".into(),
        }
    }

    /// English words that also find the command.
    fn aliases(self) -> &'static str {
        match self {
            Cmd::PlayPause => "play pause",
            Cmd::Restart => "restart again",
            Cmd::Stop => "stop",
            Cmd::Next => "next skip",
            Cmd::KeyUp | Cmd::KeyDown | Cmd::KeyReset => "key transpose pitch",
            Cmd::Faster | Cmd::Slower | Cmd::SpeedReset => "tempo speed bpm",
            Cmd::Fullscreen => "fullscreen stage",
            Cmd::Songs => "song search find",
            Cmd::Queue | Cmd::ClearQueue => "queue",
            Cmd::Mixer | Cmd::ResetMixer => "mixer tracks channels mute solo volume",
            Cmd::Favorite => "favorite favourite star",
            Cmd::Settings => "settings preferences",
            Cmd::OpenLibrary | Cmd::Rescan => "library ncn sfkar folder scan database",
            Cmd::ChooseSoundFont | Cmd::Sounds => "soundfont sf2 sfz sounds instrument routing",
        }
    }

    pub fn keys(self) -> &'static [&'static str] {
        match self {
            Cmd::PlayPause => &["Space"],
            Cmd::Next => &["N"],
            Cmd::KeyUp => &["]"],
            Cmd::KeyDown => &["["],
            Cmd::Faster => &["."],
            Cmd::Slower => &[","],
            Cmd::Fullscreen => &["F"],
            Cmd::Songs => &["/"],
            Cmd::Queue => &["Q"],
            Cmd::Mixer => &["M"],
            Cmd::Sounds => &["S"],
            Cmd::Settings => &["Ctrl", ","],
            _ => &[],
        }
    }

    /// Hidden when it would do nothing.
    fn available(self, app: &KaraokeApp) -> bool {
        let song = app.now.is_some();
        match self {
            Cmd::PlayPause | Cmd::Restart | Cmd::Stop | Cmd::KeyUp | Cmd::KeyDown | Cmd::Faster | Cmd::Slower => song,
            Cmd::KeyReset => song && app.synth.key() != 0,
            Cmd::SpeedReset => song && (app.synth.speed() - 1.0).abs() > 1e-3,
            Cmd::Next | Cmd::ClearQueue => !app.queue.is_empty(),
            Cmd::ResetMixer | Cmd::Favorite => song,
            Cmd::Rescan => !app.library.db.sources.is_empty(),
            _ => true,
        }
    }

    pub fn matches(self, app: &KaraokeApp, query: &str) -> bool {
        self.available(app)
            && query.split_whitespace().all(|w| self.label(app).to_lowercase().contains(w) || self.aliases().contains(w))
    }
}

pub fn run(app: &mut KaraokeApp, ov: &mut Overlay, cmd: Cmd, ctx: &egui::Context) -> Outcome {
    match cmd {
        Cmd::PlayPause => {
            if let Some(n) = &mut app.now {
                n.finished = false;
            }
            app.synth.toggle();
            Outcome::Close
        }
        Cmd::Restart => {
            if let Some(n) = &mut app.now {
                n.finished = false;
            }
            app.synth.seek(0.0);
            app.synth.play();
            Outcome::Close
        }
        Cmd::Stop => {
            app.synth.stop();
            Outcome::Close
        }
        Cmd::Next => {
            app.play_next();
            Outcome::Close
        }
        // Adjustments keep the overlay open so Enter can repeat them.
        Cmd::KeyUp => {
            app.synth.set_key(app.synth.key() + 1);
            Outcome::Stay
        }
        Cmd::KeyDown => {
            app.synth.set_key(app.synth.key() - 1);
            Outcome::Stay
        }
        Cmd::KeyReset => {
            app.synth.set_key(0);
            Outcome::Stay
        }
        Cmd::Faster => {
            app.synth.set_speed(app.synth.speed() + 0.05);
            Outcome::Stay
        }
        Cmd::Slower => {
            app.synth.set_speed(app.synth.speed() - 0.05);
            Outcome::Stay
        }
        Cmd::SpeedReset => {
            app.synth.set_speed(1.0);
            Outcome::Stay
        }
        Cmd::Fullscreen => {
            app.set_stage_only(ctx, !app.stage_only);
            Outcome::Close
        }
        Cmd::Songs => Outcome::Goto(Page::Songs),
        Cmd::Queue => Outcome::Goto(Page::Queue),
        Cmd::ClearQueue => {
            app.queue.clear();
            Outcome::Stay
        }
        Cmd::Mixer => Outcome::Goto(Page::Mixer),
        Cmd::ResetMixer => {
            app.synth.reset_channels();
            Outcome::Stay
        }
        Cmd::Favorite => {
            if let Some(uid) = app.now.as_ref().map(|n| n.entry.uid.clone())
                && let Err(e) = app.library.toggle_favorite(&uid)
            {
                app.toast_error(e);
            }
            Outcome::Stay
        }
        Cmd::Settings => Outcome::Goto(Page::Settings),
        Cmd::OpenLibrary => {
            let start = app.library.db.sources.last().map(|s| s.path.clone());
            ov.browse(Target::Library, start);
            Outcome::Stay
        }
        Cmd::Rescan => {
            app.library.rescan();
            Outcome::Close
        }
        Cmd::ChooseSoundFont => {
            ov.browse(Target::SoundFont, app.synth.fonts().last().map(|f| f.path.clone()));
            Outcome::Stay
        }
        Cmd::Sounds => Outcome::Goto(Page::Sounds),
    }
}
