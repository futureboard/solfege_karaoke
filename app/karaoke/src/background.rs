//! The picture behind the lyrics: plain, a picture file, or a folder of
//! pictures shown one after another. Pictures load on a worker
//! thread and cross-fade in; a slow pan and zoom keeps a still picture
//! alive, and a dark veil keeps the lyrics readable on bright ones.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crossbeam_channel::Receiver;
use eframe::egui::{self, Color32, ColorImage, Painter, Pos2, Rect, TextureHandle, TextureOptions, Vec2, pos2, vec2};

use crate::config::{Background, BgFit, BgSource};
use crate::style::INK;

/// Picture files a background can be.
pub const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif"];

/// Larger pictures are scaled down to this many pixels on their long side.
const MAX_SIDE: u32 = 2560;
/// Seconds a new picture takes to fade in.
const FADE: f64 = 1.0;

/// A picture file, as what is shown or loading.
type Key = PathBuf;

struct Pic {
    key: Key,
    tex: TextureHandle,
    size: Vec2,
}

#[derive(Default)]
pub struct Backdrop {
    shown: Option<Pic>,
    shown_at: f64,
    /// The picture being replaced and when the change began.
    leaving: Option<(Pic, f64)>,
    loading: Option<(Key, Receiver<Result<ColorImage, String>>)>,
    /// Could not be loaded: not tried again until something else is chosen.
    failed: Option<Key>,
    /// The slideshow folder and its pictures.
    folder: Option<(PathBuf, Vec<PathBuf>)>,
    slide: usize,
    next_slide: f64,
}

impl Backdrop {
    /// Once a frame: follow the settings, take finished loads, run the
    /// slideshow. Returns a problem to report.
    pub fn update(&mut self, ctx: &egui::Context, bg: &Background) -> Option<String> {
        let now = ctx.input(|i| i.time);
        let mut problem = None;
        let want = match &bg.source {
            BgSource::Plain => None,
            BgSource::Image(p) => Some(p.clone()),
            BgSource::Folder(root) => {
                if self.folder.as_ref().is_none_or(|(r, _)| r != root) {
                    let list = pictures_in(root);
                    if list.is_empty() {
                        problem = Some(format!("{}: ไม่มีรูปในโฟลเดอร์นี้", root.display()));
                    }
                    self.folder = Some((root.clone(), list));
                    self.slide = 0;
                    self.next_slide = now + bg.slide_secs.max(3) as f64;
                }
                let list = &self.folder.as_ref().expect("set above").1;
                if now >= self.next_slide && self.loading.is_none() {
                    self.slide += 1;
                    self.next_slide = now + bg.slide_secs.max(3) as f64;
                }
                if list.len() > 1 {
                    ctx.request_repaint_after(Duration::from_secs_f64((self.next_slide - now).max(0.05)));
                }
                (!list.is_empty()).then(|| list[self.slide % list.len()].clone())
            }
        };
        if !matches!(bg.source, BgSource::Folder(_)) {
            self.folder = None;
        }

        // A finished load replaces the picture.
        if let Some((key, rx)) = &self.loading {
            match rx.try_recv() {
                Ok(Ok(img)) => {
                    let size = vec2(img.size[0] as f32, img.size[1] as f32);
                    let tex = ctx.load_texture("background", img, TextureOptions::LINEAR);
                    let pic = Pic { key: key.clone(), tex, size };
                    self.leaving = self.shown.replace(pic).map(|old| (old, now));
                    self.shown_at = now;
                    self.loading = None;
                }
                Ok(Err(e)) => {
                    problem = Some(format!("ใช้รูปพื้นหลังนี้ไม่ได้ — {e}"));
                    self.failed = Some(key.clone());
                    self.loading = None;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
                Err(crossbeam_channel::TryRecvError::Disconnected) => self.loading = None,
            }
        }

        let current = self.loading.as_ref().map(|(k, _)| k).or(self.shown.as_ref().map(|p| &p.key));
        match &want {
            Some(key) if Some(key) != current && Some(key) != self.failed.as_ref() => self.load(ctx, key.clone()),
            None if self.shown.is_some() => {
                // Fade back to the plain stage.
                self.leaving = self.shown.take().map(|old| (old, now));
                self.loading = None;
            }
            _ => {}
        }
        if want != self.failed {
            self.failed = None;
        }

        let fading = now - self.shown_at < FADE || self.leaving.is_some();
        if self.leaving.as_ref().is_some_and(|(_, since)| now - since >= FADE && (self.shown.is_none() || now - self.shown_at >= FADE)) {
            self.leaving = None;
        }
        if fading {
            ctx.request_repaint();
        } else if bg.motion && self.shown.is_some() && bg.fit != BgFit::Contain {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        problem
    }

    fn load(&mut self, ctx: &egui::Context, key: Key) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let job = key.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(load_picture(&job));
            ctx.request_repaint();
        });
        self.loading = Some((key, rx));
    }

    /// Show the next picture of the slideshow now.
    pub fn next_slide(&mut self) {
        self.next_slide = 0.0;
    }

    /// The picture on screen, for a preview.
    pub fn current(&self) -> Option<(egui::TextureId, Vec2)> {
        self.shown.as_ref().map(|p| (p.tex.id(), p.size))
    }

    /// Draw the background over `rect`.
    pub fn paint(&self, painter: &Painter, rect: Rect, bg: &Background, now: f64) {
        painter.rect_filled(rect, 0.0, INK);
        let t = |since: f64| ((now - since) / FADE).clamp(0.0, 1.0) as f32;
        if let Some((pic, since)) = &self.leaving {
            // Under a new picture it stays until covered; alone it fades out.
            let alpha = if self.shown.is_some() { 1.0 } else { 1.0 - t(*since) };
            draw(painter, rect, pic, bg, now, alpha);
        }
        if let Some(pic) = &self.shown {
            draw(painter, rect, pic, bg, now, t(self.shown_at));
        }
        if self.shown.is_none() && self.leaving.is_none() {
            return;
        }
        if bg.dim > 0.0 {
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha((bg.dim.clamp(0.0, 0.95) * 255.0) as u8));
        }
        // A soft shade under the song title and the clock.
        let band = Rect::from_min_size(rect.min, vec2(rect.width(), 90.0_f32.min(rect.height())));
        let (top, clear) = (Color32::from_black_alpha(130), Color32::TRANSPARENT);
        let mut mesh = egui::Mesh::default();
        for (pos, color) in [(band.left_top(), top), (band.right_top(), top), (band.right_bottom(), clear), (band.left_bottom(), clear)] {
            mesh.colored_vertex(pos, color);
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(mesh);
    }
}

fn draw(painter: &Painter, rect: Rect, pic: &Pic, bg: &Background, now: f64, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let (dest, uv) = placement(rect, pic.size, bg.fit, bg.motion.then_some(now));
    painter.image(pic.tex.id(), dest, uv, Color32::WHITE.gamma_multiply(alpha));
}

/// Where a picture of `size` goes on `rect` and which part of it shows.
/// `motion` (the time) adds a slow pan and zoom.
pub fn placement(rect: Rect, size: Vec2, fit: BgFit, motion: Option<f64>) -> (Rect, Rect) {
    let full = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    if size.x < 1.0 || size.y < 1.0 || rect.width() < 1.0 || rect.height() < 1.0 {
        return (rect, full);
    }
    let (pic, stage) = (size.x / size.y, rect.width() / rect.height());
    let window = match fit {
        BgFit::Contain => {
            let s = (rect.width() / size.x).min(rect.height() / size.y);
            return (Rect::from_center_size(rect.center(), size * s), full);
        }
        BgFit::Stretch => vec2(1.0, 1.0),
        // The part of the picture with the stage's shape.
        BgFit::Cover if pic > stage => vec2(stage / pic, 1.0),
        BgFit::Cover => vec2(1.0, pic / stage),
    };
    let (zoom, pan) = match motion {
        Some(t) => {
            let wave = |period: f64, phase: f64| (0.5 + 0.5 * (t * std::f64::consts::TAU / period + phase).sin()) as f32;
            (1.06 + 0.04 * (wave(47.0, 0.0) * 2.0 - 1.0), vec2(wave(61.0, 0.3), wave(53.0, 1.9)))
        }
        None => (1.0, vec2(0.5, 0.5)),
    };
    let w = window / zoom;
    let min = pos2((1.0 - w.x) * pan.x, (1.0 - w.y) * pan.y);
    (rect, Rect::from_min_size(min, w))
}

/// Pictures in a folder and its sub-folders, by name.
fn pictures_in(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if is_picture(&p) {
                out.push(p);
            }
            if out.len() >= 5000 {
                break;
            }
        }
    }
    out.sort();
    out
}

pub fn is_picture(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Read a picture file, scaled down to `MAX_SIDE` on its long side.
fn load_picture(path: &Path) -> Result<ColorImage, String> {
    let err = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let img = image::ImageReader::open(path).map_err(|e| err(&e))?.with_guessed_format().map_err(|e| err(&e))?.decode().map_err(|e| err(&e))?;
    let img = if img.width().max(img.height()) > MAX_SIDE { img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle) } else { img };
    let rgba = img.to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_covers_contains_and_stretches() {
        let stage = Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0));
        // A square picture covering a 16:9 stage shows its middle band.
        let (dest, uv) = placement(stage, vec2(1000.0, 1000.0), BgFit::Cover, None);
        assert_eq!(dest, stage);
        assert!((uv.width() - 1.0).abs() < 1e-6 && (uv.height() - 0.5625).abs() < 1e-6);
        assert!((uv.center().y - 0.5).abs() < 1e-6);
        // Contained, it is centred at full height.
        let (dest, uv) = placement(stage, vec2(1000.0, 1000.0), BgFit::Contain, Some(3.0));
        assert_eq!(dest, Rect::from_center_size(stage.center(), vec2(900.0, 900.0)));
        assert_eq!(uv, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)));
        // Moving, the window stays inside the picture.
        for t in [0.0, 10.0, 33.3, 100.0] {
            let (_, uv) = placement(stage, vec2(4000.0, 1000.0), BgFit::Cover, Some(t));
            assert!(uv.min.x >= -1e-6 && uv.max.x <= 1.0 + 1e-6 && uv.min.y >= -1e-6 && uv.max.y <= 1.0 + 1e-6, "{uv:?}");
        }
        let (_, uv) = placement(stage, vec2(1000.0, 1000.0), BgFit::Stretch, None);
        assert_eq!(uv, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)));
    }

    #[test]
    fn folders_list_pictures_only() {
        let dir = std::env::temp_dir().join(format!("karaoke-bg-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["b.JPG", "a.png", "sub/c.webp", "notes.txt"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let names: Vec<String> = pictures_in(&dir).iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(names, ["a.png", "b.JPG", "sub/c.webp"]);
        // A file that is not a picture is reported, not a crash.
        assert!(load_picture(&dir.join("a.png")).is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
