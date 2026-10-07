//! Native file and folder pickers, through `rfd`: the common item dialog
//! (IFileOpenDialog) on Windows, NSOpenPanel on macOS and the XDG desktop
//! portal on Linux, which shows the desktop's own file manager dialog on
//! Wayland and X11 alike (zenity when no portal is running).
//!
//! The UI asks for a dialog with [`Dialogs::ask`]; the app opens it once a
//! frame, parented to the main window ([`Dialogs::launch`]), and waits for
//! the answer on a worker thread so the window keeps drawing. The chosen
//! paths come back through [`Dialogs::poll`].

use std::path::{Path, PathBuf};

use crossbeam_channel::Receiver;
use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// SoundFonts (`.sf2`) or SFZ instruments, several at once.
    SoundFonts,
    /// A song folder: an NCN library or a folder of `.sfkar` files.
    SongFolder,
}

impl Pick {
    pub fn title(self) -> &'static str {
        match self {
            Pick::SoundFonts => "เลือก SoundFont / SFZ",
            Pick::SongFolder => "เลือกโฟลเดอร์เพลง (NCN หรือ .sfkar)",
        }
    }
}

#[derive(Default)]
pub struct Dialogs {
    /// Asked for, not opened yet (it needs the window to sit on).
    request: Option<(Pick, Option<PathBuf>)>,
    /// Open on screen; the worker sends the paths (empty if cancelled).
    open: Option<(Pick, Receiver<Vec<PathBuf>>)>,
}

impl Dialogs {
    /// Ask for a dialog, starting in `start` (a folder, or a file whose
    /// folder is used). Ignored while one is already open.
    pub fn ask(&mut self, pick: Pick, start: Option<PathBuf>) {
        if self.open.is_none() {
            self.request = Some((pick, start));
        }
    }

    /// A dialog is asked for or on screen.
    pub fn busy(&self) -> bool {
        self.request.is_some() || self.open.is_some()
    }

    /// Open the requested dialog over the main window.
    pub fn launch(&mut self, frame: &eframe::Frame, ctx: &egui::Context) {
        let Some((pick, start)) = self.request.take() else { return };
        let mut dialog = rfd::AsyncFileDialog::new().set_title(pick.title()).set_parent(frame);
        if let Some(dir) = start.as_deref().and_then(start_dir) {
            dialog = dialog.set_directory(dir);
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = ctx.clone();
        match pick {
            Pick::SoundFonts => {
                let picked = dialog.add_filter("SoundFont / SFZ", &["sf2", "sfz"]).pick_files();
                std::thread::spawn(move || {
                    let paths = pollster::block_on(picked).unwrap_or_default().iter().map(|f| f.path().to_path_buf()).collect();
                    let _ = tx.send(paths);
                    ctx.request_repaint();
                });
            }
            Pick::SongFolder => {
                let picked = dialog.pick_folder();
                std::thread::spawn(move || {
                    let paths = pollster::block_on(picked).map(|f| f.path().to_path_buf()).into_iter().collect();
                    let _ = tx.send(paths);
                    ctx.request_repaint();
                });
            }
        }
        self.open = Some((pick, rx));
    }

    /// The answer of a closed dialog: what was picked (empty = cancelled).
    pub fn poll(&mut self) -> Option<(Pick, Vec<PathBuf>)> {
        let (pick, rx) = self.open.as_ref()?;
        let paths = match rx.try_recv() {
            Ok(paths) => paths,
            Err(crossbeam_channel::TryRecvError::Empty) => return None,
            // The worker died (no dialog backend): treat it as cancelled.
            Err(crossbeam_channel::TryRecvError::Disconnected) => Vec::new(),
        };
        let pick = *pick;
        self.open = None;
        Some((pick, paths))
    }
}

/// The folder a dialog opens in: `start` itself, or the folder of a file.
fn start_dir(start: &Path) -> Option<&Path> {
    if start.is_dir() { Some(start) } else { start.parent().filter(|p| p.is_dir()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_wait_for_the_window_and_one_dialog_at_a_time() {
        let mut d = Dialogs::default();
        assert!(!d.busy());
        d.ask(Pick::SongFolder, None);
        assert!(d.busy());
        assert!(d.poll().is_none(), "nothing open yet");
        // An answer from the worker comes back once.
        let (tx, rx) = crossbeam_channel::bounded(1);
        d.request = None;
        d.open = Some((Pick::SoundFonts, rx));
        d.ask(Pick::SongFolder, None);
        assert!(d.request.is_none(), "ignored while a dialog is open");
        tx.send(vec![PathBuf::from("a.sf2")]).unwrap();
        assert_eq!(d.poll(), Some((Pick::SoundFonts, vec![PathBuf::from("a.sf2")])));
        assert!(!d.busy());
        // A worker that died counts as cancelled.
        let (tx, rx) = crossbeam_channel::bounded::<Vec<PathBuf>>(1);
        d.open = Some((Pick::SongFolder, rx));
        drop(tx);
        assert_eq!(d.poll(), Some((Pick::SongFolder, vec![])));
    }

    #[test]
    fn starts_in_the_folder_of_a_file() {
        let dir = std::env::temp_dir();
        assert_eq!(start_dir(&dir), Some(dir.as_path()));
        assert_eq!(start_dir(&dir.join("no-such-file.sf2")), Some(dir.as_path()));
        assert_eq!(start_dir(Path::new("/no/such/dir/file.sf2")), None);
    }
}
