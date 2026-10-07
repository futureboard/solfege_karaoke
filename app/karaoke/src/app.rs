//! Application state and the actions the UI triggers.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use eframe::egui;
use solfege_sfkar::KarSong;
use solfege_songdb::Song;
use solfege_synth::engine::PlayState;

use crate::config::{self, ConfigFile, SavedInstrument, SavedPiece, Settings};
use crate::dialog::{Dialogs, Pick};
use crate::library::{self, Library};
use crate::synth::{InstrumentSound, Synth, SynthEvent};
use crate::timeline::Timeline;
use crate::ui::overlay::{Overlay, Page};
use crate::ui::sound::SoundPanel;

/// Command-line choices; they win over saved settings for this run.
#[derive(Default)]
pub struct Launch {
    /// Settings file instead of `config.json` in the data folder.
    pub config: Option<PathBuf>,
    pub library: Option<PathBuf>,
    /// SoundFonts for this run (repeatable option), replacing the saved rack.
    pub soundfonts: Vec<PathBuf>,
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
    config: ConfigFile,
    /// Native file / folder pickers.
    pub dialogs: Dialogs,
    pub synth: Synth,
    pub library: Library,
    pub queue: VecDeque<Song>,
    pub now: Option<NowPlaying>,
    pub toasts: Vec<Toast>,
    /// The window is full screen (bar, mixer and overlays stay available).
    pub fullscreen: bool,
    /// Song search, queue, commands, sounds and settings all live here.
    pub overlay: Option<Overlay>,
    /// The mixer panel, docked above the bottom bar.
    pub mixer_open: bool,
    /// The sound settings window (SoundFonts, channels, instruments, drums).
    pub sound: Option<SoundPanel>,
    pub devices: Vec<String>,
    /// Graphics backend and adapter, for the About page.
    pub renderer: String,
    /// Seek bar position while it is being dragged.
    pub scrub: Option<f64>,
    pending_song: Option<String>,
    clock: f64,
}

impl KaraokeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        crate::style::install(&cc.egui_ctx);
        let data = eframe::storage_dir(crate::APP_ID);
        let mut config = ConfigFile::new(launch.config.clone().or_else(|| data.as_ref().map(|d| d.join(config::FILE_NAME))));
        let legacy = cc.storage.and_then(|s| eframe::get_value::<Settings>(s, config::LEGACY_KEY));
        let (mut settings, config_error) = config.load(legacy);
        if !launch.soundfonts.is_empty() {
            settings.soundfonts = launch.soundfonts.clone();
            settings.routing = [0; 16];
        }
        if launch.device.is_some() {
            settings.device = launch.device;
        }
        let (library, library_error) = Library::open(data.as_deref());
        let mut app = Self {
            settings,
            config,
            dialogs: Dialogs::default(),
            synth: Synth::new(),
            library,
            queue: VecDeque::new(),
            now: None,
            toasts: Vec::new(),
            fullscreen: false,
            overlay: None,
            mixer_open: false,
            sound: None,
            devices: Vec::new(),
            renderer: cc.wgpu_render_state.as_ref().map_or_else(
                || "—".to_string(),
                |rs| {
                    let info = rs.adapter.get_info();
                    format!("wgpu · {} · {}", info.backend, info.name)
                },
            ),
            scrub: None,
            pending_song: launch.song,
            clock: 0.0,
        };
        if let Some(e) = config_error {
            app.toast_error(format!("อ่านไฟล์ตั้งค่าไม่ได้ ใช้ค่าเริ่มต้น (เก็บไฟล์เดิมเป็น .bak): {e}"));
        }
        app.synth.set_volume(app.settings.volume);
        app.synth.start_output(app.settings.device.as_deref());
        if let Some(e) = app.synth.output_error.clone() {
            app.toast_error(format!("ไม่มีเสียงออก: {e}"));
        }
        // The saved rack, else an older single font, else (first run only)
        // one found on disk. A rack emptied on purpose stays empty.
        let mut fonts = std::mem::take(&mut app.settings.soundfonts);
        if fonts.is_empty() && !app.config.existed() {
            fonts.extend(app.settings.soundfont.take().filter(|p| p.is_file()).or_else(library::find_soundfont));
        }
        for path in fonts {
            if !path.is_file() {
                app.toast_error(format!("ไม่พบ SoundFont: {}", path.display()));
                continue;
            }
            if let Err(e) = app.synth.add_font(path) {
                app.toast_error(e);
            }
        }
        app.synth.set_routing(app.settings.routing);
        app.synth.set_drum_lock(app.settings.drum_lock);
        app.synth.set_fx(app.settings.fx);
        app.synth.set_melody_off(app.settings.melody_off);
        for saved in app.settings.instruments.clone() {
            if let Some(font) = app.synth.fonts().iter().position(|f| f.path == saved.font) {
                let sound = InstrumentSound { font, bank: saved.bank, program: saved.program };
                app.synth.set_instrument(saved.instrument, Some(sound));
            }
        }
        for saved in app.settings.pieces.clone() {
            if let Some(font) = app.synth.fonts().iter().position(|f| f.path == saved.font) {
                app.synth.set_piece(saved.piece, Some(InstrumentSound { font, bank: saved.bank, program: saved.program }));
            }
        }
        if app.synth.fonts().is_empty() {
            app.toast_error("ยังไม่มี SoundFont (.sf2) — เพิ่มได้ที่แท็บ เสียง (S)".into());
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

    /// Stop and go back to the start of the song; it stays loaded, so
    /// Play starts it again. The queue does not move on.
    pub fn stop(&mut self) {
        self.synth.stop();
        if let Some(n) = &mut self.now {
            n.finished = false;
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

    /// Add a SoundFont (or SFZ) to the rack.
    pub fn add_soundfont(&mut self, path: PathBuf) {
        if let Err(e) = self.synth.add_font(path) {
            self.toast_error(e);
        }
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
        if let Some((pick, paths)) = self.dialogs.poll() {
            for path in paths {
                match pick {
                    Pick::SoundFonts => self.add_soundfont(path),
                    Pick::SongFolder => self.add_source(path),
                }
            }
        }
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
        self.sound = None;
        self.overlay = Some(Overlay::new(page));
    }

    /// Open the sound settings window (closes the overlay).
    pub fn open_sound(&mut self) {
        self.overlay = None;
        self.sound = Some(SoundPanel::new());
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        // The overlay handles its own keys.
        if self.overlay.is_some() || self.sound.is_some() || ctx.egui_wants_keyboard_input() {
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
            self.mixer_open = !self.mixer_open;
            return;
        }
        if pressed(Key::S) {
            return self.open_sound();
        }
        if pressed(Key::V) {
            self.toggle_melody();
        }
        if pressed(Key::L) {
            self.toggle_lyric_mode();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Space)) {
            self.stop();
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
            self.set_fullscreen(ctx, !self.fullscreen);
        }
        if self.fullscreen && pressed(Key::Escape) {
            self.set_fullscreen(ctx, false);
        } else if self.mixer_open && pressed(Key::Escape) {
            self.mixer_open = false;
        }
    }

    /// The settings file (`None` when there is no data folder).
    pub fn config_path(&self) -> Option<&std::path::Path> {
        self.config.path.as_deref()
    }

    /// Mute or bring back the guide melody (channel 9), as a saved setting.
    pub fn toggle_melody(&mut self) {
        let off = !self.synth.melody_off();
        self.synth.set_melody_off(off);
        self.settings.melody_off = off;
        self.toast(if off { "ปิดเมโลดี้ร้องนำ (ช่อง 9)".into() } else { "เปิดเมโลดี้ร้องนำ (ช่อง 9)".into() });
    }

    /// Switch between the two lyric layouts.
    pub fn toggle_lyric_mode(&mut self) {
        self.settings.lyric_mode = self.settings.lyric_mode.other();
        self.toast(format!("เนื้อร้อง: {}", self.settings.lyric_mode.label()));
    }

    pub fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        self.fullscreen = on;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }
}

impl eframe::App for KaraokeApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        // Follow the window: full screen can also be left from the system.
        if let Some(full) = ctx.input(|i| i.viewport().fullscreen) {
            self.fullscreen = full;
        }
        self.shortcuts(ctx);
        self.settings.volume = self.synth.volume();
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        crate::ui::show(self, ui);
        // Dialogs asked for this frame open over the window.
        self.dialogs.launch(frame, ui.ctx());
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        // The SoundFont rack lives in the synth; copy it out to save.
        self.settings.soundfonts = self.synth.font_paths();
        self.settings.routing = self.synth.routing();
        self.settings.drum_lock = self.synth.drum_lock();
        self.settings.fx = self.synth.mixer().fx;
        let paths = self.synth.font_paths();
        self.settings.instruments = self
            .synth
            .instruments()
            .iter()
            .filter_map(|(&instrument, s)| {
                Some(SavedInstrument { instrument, font: paths.get(s.font)?.clone(), bank: s.bank, program: s.program })
            })
            .collect();
        self.settings.pieces = self
            .synth
            .pieces()
            .iter()
            .enumerate()
            .filter_map(|(piece, s)| {
                let s = s.as_ref()?;
                Some(SavedPiece { piece, font: paths.get(s.font)?.clone(), bank: s.bank, program: s.program })
            })
            .collect();
        if let Err(e) = self.config.save(&self.settings) {
            self.toast_error(format!("บันทึกการตั้งค่าไม่ได้: {e}"));
        }
        if let Err(e) = self.library.save() {
            self.toast_error(format!("บันทึกฐานข้อมูลเพลงไม่ได้: {e}"));
        }
    }
}
