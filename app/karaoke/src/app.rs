//! Application state and the actions the UI triggers.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};
use solfege_ncnparser::{NcnSong, SongHeader};
use solfege_synth::engine::PlayState;

use crate::library::{self, Library};
use crate::synth::{Synth, SynthEvent};
use crate::timeline::Timeline;
use crate::ui::files::FilePicker;

const SETTINGS_KEY: &str = "settings";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
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
    pub song: Option<String>,
}

pub struct NowPlaying {
    pub header: SongHeader,
    pub song: NcnSong,
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

pub enum Picking {
    Library,
    SoundFont,
}

pub struct KaraokeApp {
    pub settings: Settings,
    pub synth: Synth,
    pub library: Library,
    pub queue: VecDeque<SongHeader>,
    pub now: Option<NowPlaying>,
    pub selected: Option<usize>,
    pub toasts: Vec<Toast>,
    /// Hide the side panels and show only the lyrics.
    pub stage_only: bool,
    pub show_settings: bool,
    pub picker: Option<(Picking, FilePicker)>,
    pub devices: Vec<String>,
    /// Seek bar position while it is being dragged.
    pub scrub: Option<f64>,
    pub focus_search: bool,
    pending_song: Option<String>,
    clock: f64,
}

impl KaraokeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        crate::style::install(&cc.egui_ctx);
        let mut settings: Settings = cc.storage.and_then(|s| eframe::get_value(s, SETTINGS_KEY)).unwrap_or_default();
        if launch.library.is_some() {
            settings.library = launch.library;
        }
        if launch.soundfont.is_some() {
            settings.soundfont = launch.soundfont;
        }
        if launch.device.is_some() {
            settings.device = launch.device;
        }
        let mut app = Self {
            settings,
            synth: Synth::new(),
            library: Library::new(),
            queue: VecDeque::new(),
            now: None,
            selected: None,
            toasts: Vec::new(),
            stage_only: false,
            show_settings: false,
            picker: None,
            devices: Vec::new(),
            scrub: None,
            focus_search: true,
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
            None => app.toast_error("ยังไม่มี SoundFont (.sf2) — เลือกได้ที่ ตั้งค่า".into()),
        }
        match app.settings.library.clone().filter(|p| p.is_dir()).or_else(library::find_default_root) {
            Some(root) => app.library.open(root),
            None => app.toast_error("ยังไม่ได้เลือกคลังเพลง NCN — เลือกได้ที่ ตั้งค่า".into()),
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

    pub fn play_now(&mut self, header: SongHeader) {
        let song = match self.library.load(&header.id) {
            Ok(s) => s,
            Err(e) => return self.toast_error(e),
        };
        let Some(midi) = song.midi_path.clone() else { return };
        if let Err(e) = self.synth.load_song(&midi) {
            return self.toast_error(format!("{}: {e:#}", header.id));
        }
        // Every song starts in its own key and tempo with all parts on.
        self.synth.set_key(0);
        self.synth.set_speed(1.0);
        self.synth.set_mutes(0);
        self.synth.play();
        let timeline = Timeline::new(&song);
        self.now = Some(NowPlaying { header, song, timeline, edges: HashMap::new(), finished: false });
    }

    /// Add to the queue, or start right away when nothing is on.
    pub fn enqueue(&mut self, header: SongHeader) {
        let idle = self.now.as_ref().is_none_or(|n| n.finished) || self.synth.state() == PlayState::Empty;
        if idle && self.queue.is_empty() {
            self.play_now(header);
        } else {
            self.toast(format!("เพิ่มในคิว: {}", header.title));
            self.queue.push_back(header);
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

    pub fn open_library(&mut self, root: PathBuf) {
        self.settings.library = Some(root.clone());
        self.selected = None;
        self.library.open(root);
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
        if self.library.poll() {
            match &self.library.error {
                Some(e) => self.toast_error(format!("คลังเพลง: {e}")),
                None => {
                    self.toast(format!("คลังเพลง: {} เพลง", self.library.songs.len()));
                    if let Some(id) = self.pending_song.take() {
                        match self.library.songs.iter().find(|h| h.id.eq_ignore_ascii_case(&id)) {
                            Some(h) => self.play_now(h.clone()),
                            None => self.toast_error(format!("ไม่พบเพลง {id}")),
                        }
                    }
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

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::Key;
        let pressed = |k: Key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, k));
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
        if pressed(Key::Slash) {
            self.focus_search = true;
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
    }
}
