//! Application state and the actions the UI triggers.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};
use solfege_sfkar::KarSong;
use solfege_songdb::Song;
use solfege_synth::engine::PlayState;

use crate::library::{self, Library};
use crate::synth::{Synth, SynthEvent};
use crate::timeline::Timeline;
use crate::ui::overlay::{Overlay, Page};

const SETTINGS_KEY: &str = "settings";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Library folder of older versions; moved into the song catalogue.
    #[serde(skip_serializing)]
    pub library: Option<PathBuf>,
    pub soundfont: Option<PathBuf>,
    pub device: Option<String>,
    pub volume: f32,
    /// Lyric size relative to the stage height.
    pub lyric_scale: f32,
    /// Shift the lyrics against the music (positive = lyrics later).
    pub lyric_offset_ms: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { library: None, soundfont: None, device: None, volume: 0.8, lyric_scale: 1.0, lyric_offset_ms: 0 }
    }
}

/// Command-line choices; they win over saved settings for this run.
#[derive(Default)]
pub struct Launch {
    pub library: Option<PathBuf>,
    pub soundfont: Option<PathBuf>,
    pub device: Option<String>,
    /// A song code from the catalogue, or a `.sfkar` file.
    pub song: Option<String>,
}

pub struct NowPlaying {
    /// The catalogue entry (or a loose `.sfkar` file).
    pub entry: Song,
    pub song: KarSong,
    pub timeline: Timeline,
    /// Syllable edges per line as fractions of the line width, measured once.
    pub edges: HashMap<usize, Vec<f32>>,
    /// Played to the end on its own.
    pub finished: bool,
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub at: f64,
}

pub struct KaraokeApp {
    pub settings: Settings,
    pub synth: Synth,
    pub library: Library,
    pub queue: VecDeque<Song>,
    pub now: Option<NowPlaying>,
    pub toasts: Vec<Toast>,
    /// Full screen with the bottom bar hidden: only the lyrics.
    pub stage_only: bool,
    /// Song search, queue, commands, mixer and settings all live here.
    pub overlay: Option<Overlay>,
    pub devices: Vec<String>,
    /// Seek bar position while it is being dragged.
    pub scrub: Option<f64>,
    pending_song: Option<String>,
    clock: f64,
}

impl KaraokeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        crate::style::install(&cc.egui_ctx);
        let mut settings: Settings = cc.storage.and_then(|s| eframe::get_value(s, SETTINGS_KEY)).unwrap_or_default();
        if launch.soundfont.is_some() {
            settings.soundfont = launch.soundfont;
        }
        if launch.device.is_some() {
            settings.device = launch.device;
        }
        let catalogue = eframe::storage_dir(crate::APP_ID).map(|d| d.join("songs.json"));
        let (library, library_error) = Library::open(catalogue);
        let mut app = Self {
            settings,
            synth: Synth::new(),
            library,
            queue: VecDeque::new(),
            now: None,
            toasts: Vec::new(),
            stage_only: false,
            overlay: None,
            devices: Vec::new(),
            scrub: None,
            pending_song: launch.song,
            clock: 0.0,
        };
        app.synth.set_volume(app.settings.volume);
        app.synth.start_output(app.settings.device.as_deref());
        if let Some(e) = app.synth.output_error.clone() {
            app.toast_error(format!("ไม่มีเสียงออก: {e}"));
        }
        match app.settings.soundfont.clone().filter(|p| p.is_file()).or_else(library::find_soundfont) {
            Some(sf) => app.synth.load_soundfont(sf),
            None => app.toast_error("ยังไม่มี SoundFont (.sf2) — เลือกได้ที่ ตั้งค่า (Ctrl+,)".into()),
        }
        if let Some(e) = library_error {
            app.toast_error(format!("ฐานข้อมูลเพลงเสียหาย สร้างใหม่: {e}"));
        }
        // First run (or a library from an older version): seed the catalogue.
        let seed = launch.library.or_else(|| {
            let old = app.settings.library.take().filter(|p| p.is_dir());
            if app.library.db.sources.is_empty() { old.or_else(library::find_default_root) } else { None }
        });
        if let Some(root) = seed {
            app.add_source(root);
        } else if app.library.db.sources.is_empty() {
            app.toast_error("ยังไม่มีคลังเพลง — เพิ่มโฟลเดอร์ NCN หรือ .sfkar ได้ที่ ตั้งค่า (Ctrl+,)".into());
        } else {
            app.library.rescan();
        }
        // A .sfkar file given on the command line plays straight away.
        if let Some(path) = app.pending_song.clone().map(PathBuf::from).filter(|p| p.is_file()) {
            app.pending_song = None;
            match library::loose_song(&path) {
                Ok(song) => app.play_now(song),
                Err(e) => app.toast_error(e),
            }
        }
        app
    }

    // ------------------------------------------------------------ toasts

    pub fn toast(&mut self, text: String) {
        self.toasts.push(Toast { text, error: false, at: self.clock });
    }

    pub fn toast_error(&mut self, text: String) {
        self.toasts.push(Toast { text, error: true, at: self.clock });
    }

    // ------------------------------------------------------------- songs

    /// Lyric clock: the song position shifted by the user's offset.
    pub fn lyric_time(&self) -> f64 {
        let t = self.scrub.unwrap_or_else(|| self.synth.time());
        t - self.settings.lyric_offset_ms as f64 / 1000.0
    }

    /// Tempo being heard now: the song's tempo at this point times the
    /// speed setting.
    pub fn bpm(&self) -> Option<f64> {
        let now = self.now.as_ref()?;
        Some(now.timeline.tempo.bpm(self.synth.time()) * self.synth.speed())
    }

    pub fn play_now(&mut self, entry: Song) {
        let loaded = library::load(&entry).and_then(|song| {
            let timing = song.timing().map_err(|e| e.to_string())?;
            self.synth.load_midi(&song.midi, &entry.id).map_err(|e| format!("{e:#}"))?;
            Ok((song, timing))
        });
        let (song, timing) = match loaded {
            Ok(x) => x,
            Err(e) => return self.toast_error(format!("{}: {e}", entry.id)),
        };
        // Every song starts in its own key and tempo with all parts on.
        self.synth.set_key(0);
        self.synth.set_speed(1.0);
        self.synth.reset_channels();
        self.synth.play();
        if let Err(e) = self.library.record_play(&entry.uid) {
            self.toast_error(format!("บันทึกประวัติเพลงไม่ได้: {e}"));
        }
        let timeline = Timeline::new(&song, &timing);
        self.now = Some(NowPlaying { entry, song, timeline, edges: HashMap::new(), finished: false });
    }

    /// Add to the queue, or start right away when nothing is on.
    pub fn enqueue(&mut self, entry: Song) {
        let idle = self.now.as_ref().is_none_or(|n| n.finished) || self.synth.state() == PlayState::Empty;
        if idle && self.queue.is_empty() {
            self.play_now(entry);
        } else {
            self.toast(format!("เพิ่มในคิว: {}", entry.title));
            self.queue.push_back(entry);
        }
    }

    pub fn play_next(&mut self) {
        match self.queue.pop_front() {
            Some(h) => self.play_now(h),
            None => {
                self.synth.stop();
                if let Some(n) = &mut self.now {
                    n.finished = true;
                }
            }
        }
    }

    /// Add a folder of NCN or .sfkar songs to the catalogue.
    pub fn add_source(&mut self, root: PathBuf) {
        match self.library.add_source(root.clone()) {
            Ok(kind) => {
                let kind = match kind {
                    solfege_songdb::SourceKind::Ncn => "NCN",
                    solfege_songdb::SourceKind::Sfkar => ".sfkar",
                };
                self.toast(format!("เพิ่มคลังเพลง {kind}: {}", root.display()));
            }
            Err(e) => self.toast_error(e),
        }
    }

    pub fn open_soundfont(&mut self, path: PathBuf) {
        self.settings.soundfont = Some(path.clone());
        self.synth.load_soundfont(path);
    }

    pub fn reopen_output(&mut self) {
        self.synth.start_output(self.settings.device.as_deref());
        match self.synth.output_error.clone() {
            Some(e) => self.toast_error(format!("ไม่มีเสียงออก: {e}")),
            None => self.toast(format!("เสียงออก: {}", self.synth.output_info)),
        }
        // The new engine starts stopped at the top of the song.
        if let Some(n) = &mut self.now {
            n.finished = true;
        }
    }

    // ------------------------------------------------------------- frame

    fn poll(&mut self, ctx: &egui::Context) {
        self.clock = ctx.input(|i| i.time);
        if let Some(report) = self.library.poll() {
            self.toast(format!("คลังเพลง: {} เพลง", report.songs));
            if let Some(e) = report.errors.first() {
                let more = if report.errors.len() > 1 { format!(" (และอีก {} รายการ)", report.errors.len() - 1) } else { String::new() };
                self.toast_error(format!("อ่านไม่ได้: {e}{more}"));
            }
            if let Err(e) = self.library.save() {
                self.toast_error(format!("บันทึกฐานข้อมูลเพลงไม่ได้: {e}"));
            }
            if let Some(code) = self.pending_song.take() {
                match self.library.find(&code).cloned() {
                    Some(song) => self.play_now(song),
                    None => self.toast_error(format!("ไม่พบเพลง {code}")),
                }
            }
        }
        for ev in self.synth.poll() {
            match ev {
                SynthEvent::SoundFontLoaded { name, presets } => self.toast(format!("SoundFont: {name} ({presets} เสียง)")),
                SynthEvent::SoundFontFailed(e) => self.toast_error(format!("SoundFont: {e}")),
                SynthEvent::Ended => {
                    if let Some(n) = &mut self.now {
                        n.finished = true;
                    }
                    if !self.queue.is_empty() {
                        self.play_next();
                    }
                }
            }
        }
        let now = self.clock;
        self.toasts.retain(|t| now - t.at < if t.error { 7.0 } else { 3.5 });
        if self.synth.state() == PlayState::Playing || self.library.scanning() || self.synth.loading_soundfont() {
            ctx.request_repaint();
        } else if !self.toasts.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    pub fn open(&mut self, page: Page) {
        self.overlay = Some(Overlay::new(page));
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        // The overlay handles its own keys.
        if self.overlay.is_some() || ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::{Key, Modifiers};
        let command = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, k));
        if command(Key::K) {
            return self.open(Page::Commands);
        }
        if command(Key::Comma) {
            return self.open(Page::Settings);
        }
        let pressed = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
        if pressed(Key::Slash) {
            return self.open(Page::Songs);
        }
        if pressed(Key::Q) {
            return self.open(Page::Queue);
        }
        if pressed(Key::M) {
            return self.open(Page::Mixer);
        }
        if pressed(Key::Space) {
            self.synth.toggle();
        }
        if pressed(Key::ArrowLeft) {
            self.synth.seek(self.synth.time() - 5.0);
        }
        if pressed(Key::ArrowRight) {
            self.synth.seek(self.synth.time() + 5.0);
        }
        if pressed(Key::OpenBracket) {
            self.synth.set_key(self.synth.key() - 1);
        }
        if pressed(Key::CloseBracket) {
            self.synth.set_key(self.synth.key() + 1);
        }
        if pressed(Key::Comma) {
            self.synth.set_speed(self.synth.speed() - 0.05);
        }
        if pressed(Key::Period) {
            self.synth.set_speed(self.synth.speed() + 0.05);
        }
        if pressed(Key::N) {
            self.play_next();
        }
        if pressed(Key::F) || pressed(Key::F11) {
            self.set_stage_only(ctx, !self.stage_only);
        }
        if self.stage_only && pressed(Key::Escape) {
            self.set_stage_only(ctx, false);
        }
    }

    pub fn set_stage_only(&mut self, ctx: &egui::Context, on: bool) {
        self.stage_only = on;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }
}

impl eframe::App for KaraokeApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.shortcuts(ctx);
        self.settings.volume = self.synth.volume();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        crate::ui::show(self, ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
        if let Err(e) = self.library.save() {
            self.toast_error(format!("บันทึกฐานข้อมูลเพลงไม่ได้: {e}"));
        }
    }
}
