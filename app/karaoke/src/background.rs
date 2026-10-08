//! The picture behind the lyrics: plain, one of the built-in pictures
//! (drawn here, so the app ships no image files), a picture file, or a
//! folder of pictures shown one after another. Pictures load on a worker
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

/// Names of the built-in pictures, in `BgSource::Preset` order.
pub const PRESETS: [&str; 6] = ["ราตรีแสงไฟ", "ออโรร่า", "พระอาทิตย์ตก", "ใต้ทะเล", "นีออน", "ไฟเวที"];

/// Larger pictures are scaled down to this many pixels on their long side.
const MAX_SIDE: u32 = 2560;
/// Size the built-in pictures are drawn at (the GPU scales them smoothly).
const PRESET_SIZE: [usize; 2] = [1280, 720];
/// Seconds a new picture takes to fade in.
const FADE: f64 = 1.0;

#[derive(Clone, Debug, PartialEq)]
enum Key {
    Preset(usize),
    File(PathBuf),
}

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
    thumbs: Vec<TextureHandle>,
}

impl Backdrop {
    /// Once a frame: follow the settings, take finished loads, run the
    /// slideshow. Returns a problem to report.
    pub fn update(&mut self, ctx: &egui::Context, bg: &Background) -> Option<String> {
        let now = ctx.input(|i| i.time);
        let mut problem = None;
        let want = match &bg.source {
            BgSource::Plain => None,
            BgSource::Preset(i) => Some(Key::Preset((*i).min(PRESETS.len() - 1))),
            BgSource::Image(p) => Some(Key::File(p.clone())),
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
                (!list.is_empty()).then(|| Key::File(list[self.slide % list.len()].clone()))
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
            let img = match &job {
                Key::Preset(i) => Ok(preset(*i, PRESET_SIZE)),
                Key::File(p) => load_picture(p),
            };
            let _ = tx.send(img);
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

    /// Small picture of built-in background `i`.
    pub fn thumb(&mut self, ctx: &egui::Context, i: usize) -> egui::TextureId {
        if self.thumbs.is_empty() {
            self.thumbs = (0..PRESETS.len()).map(|k| ctx.load_texture(format!("bg-thumb-{k}"), preset(k, [240, 135]), TextureOptions::LINEAR)).collect();
        }
        self.thumbs[i.min(PRESETS.len() - 1)].id()
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

// ------------------------------------------------------------ built-in pictures

type Rgb = [f32; 3];

fn hex(c: u32) -> Rgb {
    [((c >> 16) & 0xFF) as f32 / 255.0, ((c >> 8) & 0xFF) as f32 / 255.0, (c & 0xFF) as f32 / 255.0]
}

fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn add(a: Rgb, b: Rgb, k: f32) -> Rgb {
    [a[0] + b[0] * k, a[1] + b[1] * k, a[2] + b[2] * k]
}

/// Gradient through `stops` (position, colour) at `t`.
fn ramp(stops: &[(f32, u32)], t: f32) -> Rgb {
    let i = stops.iter().position(|s| s.0 >= t).unwrap_or(stops.len() - 1);
    if i == 0 {
        return hex(stops[0].1);
    }
    let (a, b) = (stops[i - 1], stops[i]);
    lerp(hex(a.1), hex(b.1), (t - a.0) / (b.0 - a.0).max(1e-6))
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Small deterministic random numbers (the pictures look the same every run).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / (1u64 << 24) as f32
    }
}

/// Hash noise in 0..1 for dithering (avoids banding in smooth gradients).
fn noise(x: usize, y: usize) -> f32 {
    let mut h = (x as u32).wrapping_mul(374761393) ^ (y as u32).wrapping_mul(668265263);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    ((h ^ (h >> 16)) & 0xFFFF) as f32 / 65535.0
}

/// Built-in picture `index` drawn at `size` (width, height).
pub fn preset(index: usize, size: [usize; 2]) -> ColorImage {
    let [w, h] = size;
    let aspect = w as f32 / h as f32;
    let mut rng = Rng(0x5eed + index as u64 * 7919);
    // Glowing discs, as (x, y, radius, colour, strength); x in 0..aspect.
    let mut discs = Vec::new();
    let mut stars = Vec::new();
    match index {
        0 => {
            let colors = [0xffb03b, 0xff5d8f, 0x4fd1c5, 0xffd27a, 0xb388ff];
            for _ in 0..46 {
                let r = 0.02 + rng.next().powi(2) * 0.09;
                discs.push((rng.next() * aspect, 0.15 + rng.next() * 0.85, r, hex(colors[(rng.next() * 5.0) as usize % 5]), 0.18 + rng.next() * 0.35));
            }
        }
        1 | 3 => {
            for _ in 0..if index == 1 { 140 } else { 50 } {
                stars.push((rng.next() * aspect, rng.next() * if index == 1 { 0.6 } else { 1.0 }, 0.3 + rng.next() * 0.7));
            }
        }
        _ => {}
    }
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let px = u * aspect;
            let c = match index {
                // Night city bokeh.
                0 => {
                    let mut c = ramp(&[(0.0, 0x0b1026), (0.6, 0x1d1240), (1.0, 0x2c1238)], v);
                    c = add(c, hex(0xff8a3d), 0.10 * smooth(0.55, 1.0, v));
                    for &(cx, cy, r, col, k) in &discs {
                        let d = ((px - cx).powi(2) + (v - cy).powi(2)).sqrt();
                        if d < r * 1.3 {
                            let disc = 1.0 - smooth(r * 0.82, r, d);
                            let halo = (1.0 - d / (r * 1.3)).max(0.0).powi(2) * 0.25;
                            c = add(c, col, (disc + halo) * k);
                        }
                    }
                    c
                }
                // Aurora over a dark sky.
                1 => {
                    let mut c = ramp(&[(0.0, 0x03060f), (0.7, 0x07202a), (1.0, 0x0b2a2a)], v);
                    for (k, col) in [(0.0, 0x2bd99a), (1.0, 0x29b6c9), (2.0, 0x8a5cf6)] {
                        let center = 0.30 + 0.09 * k + 0.07 * (u * 3.1 + k * 1.7).sin() + 0.03 * (u * 7.3 + k).sin();
                        let width = 0.05 + 0.02 * (u * 2.0 + k).cos().abs();
                        let band = (-((v - center) / width).powi(2)).exp();
                        let curtain = 0.55 + 0.45 * (u * 70.0 + (u * 9.0 + k).sin() * 4.0).sin().powi(2);
                        let tail = smooth(center + 0.25, center, v);
                        c = add(c, hex(col), band * curtain * 0.55 + tail * band.sqrt() * 0.12);
                    }
                    for &(sx, sy, b) in &stars {
                        let d2 = (px - sx).powi(2) + (v - sy).powi(2);
                        c = add(c, [1.0, 1.0, 1.0], (-(d2 / 2.0e-6)).exp() * b * 0.8);
                    }
                    c
                }
                // Sunset over hills.
                2 => {
                    let horizon = 0.64;
                    let mut c = ramp(&[(0.0, 0x1a0b33), (0.35, 0x6a1f6b), (0.55, 0xd14a6a), (horizon, 0xffa04d)], v.min(horizon));
                    let d = ((px - aspect * 0.5).powi(2) + (v - 0.6).powi(2)).sqrt();
                    c = add(c, hex(0xffd27a), (1.0 - smooth(0.055, 0.06, d)) * 0.9 + (-d * 6.0).exp() * 0.45);
                    let far = 0.67 + 0.035 * (u * 6.0 + 1.0).sin() + 0.02 * (u * 13.0).sin();
                    let near = 0.76 + 0.05 * (u * 4.0 + 2.5).sin() + 0.025 * (u * 11.0 + 0.7).sin();
                    if v > near {
                        c = ramp(&[(0.0, 0x1b0b24), (1.0, 0x0c0612)], (v - near) * 3.0);
                    } else if v > far {
                        c = lerp(hex(0x4a1d4f), hex(0x2a1032), (v - far) * 5.0);
                    } else if v > horizon {
                        c = lerp(hex(0xff8c50), hex(0x8a3060), (v - horizon) * 8.0);
                    }
                    c
                }
                // Light from above, under the sea.
                3 => {
                    let mut c = ramp(&[(0.0, 0x0f6a8a), (0.45, 0x0a3d62), (1.0, 0x02101f)], v);
                    let (sx, sy) = (aspect * 0.35, -0.25);
                    let angle = (px - sx).atan2(v - sy);
                    let rays = (0.5 + 0.5 * (angle * 26.0 + (angle * 7.0).sin() * 2.0).sin()).powi(6);
                    c = add(c, hex(0x9fe8ff), rays * (1.0 - v).powi(2) * 0.28);
                    for &(bx, by, b) in &stars {
                        let r = 0.004 + b * 0.008;
                        let d = ((px - bx).powi(2) + (v - by).powi(2)).sqrt();
                        let ring = (1.0 - ((d - r).abs() / 0.0025).min(1.0)) * 0.35;
                        c = add(c, hex(0xc8f4ff), ring * (1.0 - v * 0.5));
                    }
                    c
                }
                // Neon sun and grid.
                4 => {
                    let horizon = 0.58;
                    if v < horizon {
                        let mut c = ramp(&[(0.0, 0x0d0221), (0.7, 0x2a0845), (1.0, 0x5a0f5e)], v / horizon);
                        let (cx, cy, r) = (aspect * 0.5, 0.40, 0.17);
                        let d = ((px - cx).powi(2) + (v - cy).powi(2)).sqrt();
                        let gap = v > cy && ((v - cy) * 60.0).fract() < 0.35 * (v - cy) / r;
                        if d < r && !gap {
                            c = ramp(&[(0.0, 0xffe066), (0.5, 0xff8a5c), (1.0, 0xff3d9a)], (v - (cy - r)) / (2.0 * r));
                        }
                        c = add(c, hex(0xff3d9a), (-(d - r).max(0.0) * 9.0).exp() * 0.25 + smooth(horizon - 0.12, horizon, v) * 0.25);
                        c
                    } else {
                        let depth = 1.0 / (v - horizon + 0.02);
                        let gx = ((px - aspect * 0.5) * depth * 1.6).fract().abs();
                        let gz = (depth * 0.9).fract();
                        let line = |f: f32, wdt: f32| (1.0 - (f.min(1.0 - f) / wdt).min(1.0)).powi(2);
                        let fade = smooth(horizon, horizon + 0.25, v);
                        let mut c = lerp(hex(0x14032b), hex(0x07010f), (v - horizon) * 2.0);
                        c = add(c, hex(0xff3db5), (line(gx, 0.035 * depth.sqrt()) + line(gz, 0.06)) * (0.25 + 0.65 * fade));
                        add(c, hex(0xff3d9a), (1.0 - smooth(horizon, horizon + 0.08, v)) * 0.35)
                    }
                }
                // Stage lights.
                _ => {
                    let mut c = ramp(&[(0.0, 0x05050a), (1.0, 0x0e0b16)], v);
                    for (k, col) in [(0.12, 0xff3db5), (0.37, 0x3dd9ff), (0.63, 0xffb03b), (0.88, 0x9b6bff)] {
                        let (sx, sy) = (k * aspect, -0.08);
                        let aim = (k - 0.5) * 0.5;
                        let angle = (px - sx).atan2(v - sy) - aim;
                        let cone = 1.0 - smooth(0.10, 0.17, angle.abs());
                        let dist = ((px - sx).powi(2) + (v - sy).powi(2)).sqrt();
                        c = add(c, hex(col), cone * (0.45 - dist * 0.3).max(0.0));
                    }
                    add(c, hex(0x6a4c9c), smooth(0.75, 1.0, v) * 0.25)
                }
            };
            let n = (noise(x, y) - 0.5) / 255.0;
            let to8 = |f: f32| ((f + n).clamp(0.0, 1.0) * 255.0).round() as u8;
            pixels.push(Color32::from_rgb(to8(c[0]), to8(c[1]), to8(c[2])));
        }
    }
    ColorImage::new(size, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_draw_and_stay_dark_enough_for_lyrics() {
        for i in 0..PRESETS.len() {
            let img = preset(i, [160, 90]);
            assert_eq!(img.pixels.len(), 160 * 90);
            let mean = img.pixels.iter().map(|c| (c.r() as u32 + c.g() as u32 + c.b() as u32) / 3).sum::<u32>() / img.pixels.len() as u32;
            assert!((8..110).contains(&mean), "preset {i}: mean brightness {mean}");
            assert_eq!(img.pixels, preset(i, [160, 90]).pixels, "the same every time");
        }
    }

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
