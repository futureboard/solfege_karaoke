//! The lyric stage, in one of two layouts (`LyricMode`):
//!
//! - **Scroll**: the line being sung sits in the middle at full size and
//!   fills with colour syllable by syllable; finished lines drift up and
//!   fade, the next ones wait below.
//! - **Classic**: two fixed lines in the middle, top and bottom in turn;
//!   when a line is done, the one after next takes its place.
//!
//! Long rests get a four-dot count-in. The time of day sits top right.

use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, FontId, Galley, Painter, Pos2, Rect, Sense, Stroke, pos2, vec2};
use solfege_synth::engine::PlayState;

use crate::app::{KaraokeApp, NowPlaying};
use crate::config::LyricMode;
use crate::icons;
use crate::music::transpose_key;
use crate::style::{ACCENT, DIM, INK, SUNG, SUNG_HOT, TEXT, UNSUNG, lyrics_family};
use crate::timeline::{COUNT_IN_BEATS, Cue, Line};
use crate::ui::{menu, overlay};

/// Colours of the lyrics (from the settings), set at the start of a frame.
#[derive(Clone, Copy)]
struct Palette {
    unsung: Color32,
    sung: Color32,
    wipe: Color32,
    outline: Color32,
}

const PALETTE: &str = "lyric-palette";

fn palette(painter: &Painter) -> Palette {
    painter
        .ctx()
        .data(|d| d.get_temp(egui::Id::new(PALETTE)))
        .unwrap_or(Palette { unsung: UNSUNG, sung: SUNG, wipe: SUNG_HOT, outline: INK })
}

/// Once the current line is done, move the stage to the next line this
/// long before its count-in would begin.
const LOOK_AHEAD: f64 = 0.5;
/// Size of the lines around the focus line, relative to it.
const SIDE_SCALE: f32 = 0.66;

pub fn show(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    if resp.double_clicked() {
        let ctx = ui.ctx().clone();
        app.set_fullscreen(&ctx, !app.fullscreen);
    }
    resp.context_menu(|ui| menu::app_menu(app, ui));
    let painter = ui.painter_at(rect);
    backdrop(&painter, rect);
    let c = app.settings.lyric_colors;
    let rgb = |v: [u8; 3]| Color32::from_rgb(v[0], v[1], v[2]);
    let palette = Palette { unsung: rgb(c.unsung), sung: rgb(c.sung), wipe: rgb(c.wipe), outline: rgb(c.outline) };
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(PALETTE), palette));
    if app.settings.show_clock {
        wall_clock(&painter, rect);
        // Wake up for the next minute even when nothing plays.
        let secs = chrono::Timelike::second(&chrono::Local::now()) as u64;
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(60 - secs.min(59)));
    }

    let t = app.lyric_time();
    let scale = app.settings.lyric_scale;
    let mode = app.settings.lyric_mode;
    let key = app.synth.key();
    let state = app.synth.state();
    let Some(now) = app.now.as_mut() else {
        idle(&painter, rect, !app.synth.has_font());
        return;
    };
    header(&painter, rect, now);

    let base = (rect.height() * 0.085).clamp(26.0, 110.0) * scale;
    let max_w = rect.width() - 64.0;
    if now.finished && state != PlayState::Playing {
        let next = app.queue.front().map(|h| h.title.clone());
        finished(&painter, rect, now, base, next.as_deref());
        return;
    }
    let tl = &now.timeline;
    if tl.is_empty() {
        title_card(&painter, rect, now, base, key);
        return;
    }
    let cue = tl.cue(t);
    if cue == Cue::Intro {
        title_card(&painter, rect, now, base, key);
        return;
    }

    let target = stage_line(&tl.lines, tl.focus(t), t);
    if mode == LyricMode::Classic {
        let count_in = match cue {
            Cue::CountIn { line, beats } => Some((line, beats)),
            _ => None,
        };
        classic(&painter, now, rect, base, max_w, t, target, count_in);
        return;
    }
    let target = target as f32;
    let id = ui.id().with("stage-scroll").with(&now.entry.id);
    let scroll = ui.ctx().animate_value_with_time(id, target, 0.35);
    let spacing = base * 1.7;
    let center = rect.center().y + base * 0.15;

    let first = (scroll.floor() as i64 - 2).max(0) as usize;
    let last = ((scroll.ceil() as i64 + 3).max(0) as usize).min(tl.lines.len().saturating_sub(1));
    for i in first..=last {
        let d = i as f32 - scroll;
        let y = center + d * spacing - if d > 0.0 { base * (1.0 - SIDE_SCALE) * 0.5 } else { 0.0 };
        if y < rect.top() - base || y > rect.bottom() + base {
            continue;
        }
        let near = (1.0 - d.abs()).clamp(0.0, 1.0);
        let size = base * (SIDE_SCALE + (1.0 - SIDE_SCALE) * near);
        let alpha = (1.0 - 0.38 * d.abs()).clamp(0.12, 1.0) * if d < 0.0 { 0.75 } else { 1.0 };
        let focus = i == target as usize;
        let count_in = match cue {
            Cue::CountIn { line, beats } if line == i => Some(beats),
            _ => None,
        };
        draw_line(&painter, now, i, pos2(rect.center().x, y), size, max_w, alpha, t, focus, count_in);
    }
}

/// Two fixed rows; line `i` always uses row `i % 2`. The row of the line
/// being sung shows it, the other row the line after it.
#[allow(clippy::too_many_arguments)]
fn classic(painter: &Painter, now: &mut NowPlaying, rect: Rect, base: f32, max_w: f32, t: f64, target: usize, count_in: Option<(usize, f64)>) {
    let spacing = base * 1.6;
    let center = rect.center().y + base * 0.15;
    let rows = [center - spacing / 2.0, center + spacing / 2.0];
    let n = now.timeline.lines.len();
    for (i, focus) in [(target, true), (target + 1, false)] {
        if i >= n {
            continue;
        }
        let beats = count_in.filter(|(line, _)| *line == i).map(|(_, b)| b);
        let alpha = if focus { 1.0 } else { 0.8 };
        draw_line(painter, now, i, pos2(rect.center().x, rows[i % 2]), base * 0.92, max_w, alpha, t, focus, beats);
    }
}

/// `HH:MM`, local time, top right.
fn wall_clock(painter: &Painter, rect: Rect) {
    let now = chrono::Local::now().format("%H:%M").to_string();
    let pos = pos2(rect.right() - 18.0, rect.top() + 18.0);
    let r = painter.text(pos, Align2::RIGHT_TOP, now, FontId::proportional(17.0), TEXT.gamma_multiply(0.8));
    painter.text(pos2(r.left() - 8.0, r.center().y), Align2::RIGHT_CENTER, icons::CLOCK, FontId::proportional(14.0), DIM);
}

/// The line to centre: the focus line, or the next one once the focus line
/// is finished and the next is close.
fn stage_line(lines: &[Line], focus: usize, t: f64) -> usize {
    match (lines.get(focus), lines.get(focus + 1)) {
        (Some(cur), Some(next)) if t >= cur.start && t >= cur.end && next.start - t <= next.count_in() + LOOK_AHEAD => {
            focus + 1
        }
        _ => focus,
    }
}

/// Syllable edges of a line as fractions of its width. Measured once at a
/// reference size; text advances scale with the font size.
fn edges(painter: &Painter, line: &Line) -> Vec<f32> {
    let font = FontId::new(64.0, lyrics_family());
    let total = painter.layout_no_wrap(joined(line), font.clone(), TEXT).size().x.max(1.0);
    let mut out = Vec::with_capacity(line.syllables.len() + 1);
    out.push(0.0);
    let mut prefix = String::new();
    for s in &line.syllables[..line.syllables.len().saturating_sub(1)] {
        prefix.push_str(&s.text);
        let w = painter.layout_no_wrap(prefix.clone(), font.clone(), TEXT).size().x;
        out.push((w / total).clamp(0.0, 1.0));
    }
    out.push(1.0);
    // Shaping can make a prefix a hair wider than the next one.
    for i in 1..out.len() {
        out[i] = out[i].max(out[i - 1]);
    }
    out
}

fn joined(line: &Line) -> String {
    if line.syllables.is_empty() { line.text.clone() } else { line.syllables.iter().map(|s| s.text.as_str()).collect() }
}

#[allow(clippy::too_many_arguments)]
fn draw_line(
    painter: &Painter,
    now: &mut NowPlaying,
    i: usize,
    center: Pos2,
    size: f32,
    max_w: f32,
    alpha: f32,
    t: f64,
    focus: bool,
    count_in: Option<f64>,
) {
    let line = &now.timeline.lines[i];
    let joined_text = joined(line);
    let mut font = FontId::new(size.round(), lyrics_family());
    let mut galley = painter.layout_no_wrap(joined_text.clone(), font.clone(), UNSUNG);
    if galley.size().x > max_w {
        font.size = (size * max_w / galley.size().x).floor().max(10.0);
        galley = painter.layout_no_wrap(joined_text.clone(), font.clone(), UNSUNG);
    }
    let edges = now.edges.entry(i).or_insert_with(|| edges(painter, line));
    let w = galley.size().x;
    // Centre the visible text; padding spaces still take their timing.
    let width_of = |s: &str| if s.is_empty() { 0.0 } else { painter.layout_no_wrap(s.to_string(), font.clone(), UNSUNG).size().x };
    let lead = width_of(&joined_text[..joined_text.len() - joined_text.trim_start().len()]);
    let visible = width_of(joined_text.trim());
    let pos = pos2(center.x - lead - visible / 2.0, center.y - galley.size().y / 2.0);
    let left = pos.x;

    let (done, frac) = line.progress(t);
    let edge = |k: usize| left + w * edges.get(k).copied().unwrap_or(1.0);
    let done_x = edge(done);
    let wipe_x = done_x + (edge(done + 1) - done_x) * frac;

    let a = |c: Color32| c.gamma_multiply(alpha);
    let pal = palette(painter);
    outline(painter, &galley, pos, size, a(pal.outline.gamma_multiply(0.85)));
    painter.galley_with_override_text_color(pos, galley.clone(), a(pal.unsung));
    if wipe_x > left {
        let tall = |x0: f32, x1: f32| Rect::from_min_max(pos2(x0, pos.y - size), pos2(x1, pos.y + galley.size().y + size));
        painter.with_clip_rect(tall(left - 2.0, done_x)).galley_with_override_text_color(pos, galley.clone(), a(pal.sung));
        if wipe_x > done_x {
            painter.with_clip_rect(tall(done_x, wipe_x)).galley_with_override_text_color(pos, galley.clone(), a(pal.wipe));
        }
    }

    // A glowing bead rides the wipe under the line being sung.
    let singing = focus && t >= line.start && t < line.end && done < line.syllables.len();
    if singing {
        let bead = pos2(wipe_x, pos.y + galley.size().y + size * 0.12);
        for (r, o) in [(size * 0.16, 0.12), (size * 0.10, 0.25), (size * 0.055, 1.0)] {
            painter.circle_filled(bead, r, pal.wipe.gamma_multiply(o));
        }
    }

    // Four dots, one going out on every beat until the line starts.
    if let Some(beats) = count_in {
        let n = COUNT_IN_BEATS;
        let dots = (beats.ceil().max(0.0) as usize).min(n);
        let r = (size * 0.11).max(4.0);
        let y = pos.y - r * 2.6;
        let pulse = (beats.fract() as f32).clamp(0.0, 1.0);
        for k in 0..n {
            let c = pos2(left + lead + r + k as f32 * r * 3.0, y);
            if k < dots {
                let grow = if k + 1 == dots { 0.8 + 0.2 * pulse } else { 1.0 };
                painter.circle_filled(c, r * grow, pal.sung);
            } else {
                painter.circle_stroke(c, r * 0.7, Stroke::new(1.5, DIM.gamma_multiply(0.5)));
            }
        }
    }
}

/// Dark rim around the letters so they read on any backdrop.
fn outline(painter: &Painter, galley: &Arc<Galley>, pos: Pos2, size: f32, color: Color32) {
    let o = (size * 0.035).clamp(1.0, 3.5);
    for d in [vec2(-o, 0.0), vec2(o, 0.0), vec2(0.0, -o), vec2(0.0, o), vec2(o * 0.8, o * 1.6)] {
        painter.galley_with_override_text_color(pos + d, galley.clone(), color);
    }
}

fn backdrop(painter: &Painter, rect: Rect) {
    painter.rect_filled(rect, 0.0, INK);
}

fn header(painter: &Painter, rect: Rect, now: &NowPlaying) {
    let pad = 18.0;
    let title = if now.entry.artist.is_empty() {
        now.entry.title.clone()
    } else {
        format!("{}  —  {}", now.entry.title, now.entry.artist)
    };
    painter.text(rect.left_top() + vec2(pad, pad), Align2::LEFT_TOP, title, FontId::proportional(15.0), DIM);
}

fn title_card(painter: &Painter, rect: Rect, now: &NowPlaying, base: f32, key: i32) {
    let c = rect.center();
    let max_w = rect.width() - 80.0;
    let mut size = base * 1.05;
    let pal = palette(painter);
    let mut g = painter.layout_no_wrap(now.song.meta.title.clone(), FontId::new(size, lyrics_family()), pal.sung);
    if g.size().x > max_w {
        size *= max_w / g.size().x;
        g = painter.layout_no_wrap(now.song.meta.title.clone(), FontId::new(size, lyrics_family()), pal.sung);
    }
    let pos = c - vec2(g.size().x / 2.0, g.size().y + base * 0.2);
    outline(painter, &g, pos, size, pal.outline);
    painter.galley(pos, g, pal.sung);
    painter.text(c + vec2(0.0, base * 0.15), Align2::CENTER_TOP, &now.song.meta.artist, FontId::new(base * 0.5, lyrics_family()), pal.unsung);
    if let Some(k) = &now.song.meta.key {
        let shown = transpose_key(k, key).unwrap_or_else(|| k.clone());
        painter.text(c + vec2(0.0, base * 0.95), Align2::CENTER_TOP, format!("{}  คีย์ {shown}", icons::KEY), FontId::proportional(base * 0.32), ACCENT);
    }
}

fn finished(painter: &Painter, rect: Rect, now: &NowPlaying, base: f32, next: Option<&str>) {
    let c = rect.center();
    painter.text(c - vec2(0.0, base * 0.3), Align2::CENTER_BOTTOM, "จบเพลง", FontId::new(base, lyrics_family()), palette(painter).sung);
    painter.text(c, Align2::CENTER_TOP, &now.song.meta.title, FontId::new(base * 0.45, lyrics_family()), DIM);
    let hint = match next {
        Some(t) => format!("ถัดไป: {t}"),
        None => "กด / เพื่อเลือกเพลงต่อไป หรือ Space เพื่อร้องซ้ำ".to_string(),
    };
    painter.text(c + vec2(0.0, base * 1.0), Align2::CENTER_TOP, hint, FontId::proportional(16.0), TEXT);
}

/// Centred row of "[keys] what" pairs.
fn key_hints(painter: &Painter, center: Pos2, hints: &[(&[&str], &str)]) {
    let font = FontId::proportional(15.0);
    let gap = 28.0;
    let width_of = |keys: &[&str], what: &str| {
        keys.iter().map(|k| overlay::keycap_width(painter, k) + 4.0).sum::<f32>()
            + 6.0
            + painter.layout_no_wrap(what.to_string(), font.clone(), TEXT).size().x
    };
    let total: f32 = hints.iter().map(|(k, w)| width_of(k, w)).sum::<f32>() + gap * (hints.len() as f32 - 1.0);
    let mut x = center.x - total / 2.0;
    for (keys, what) in hints {
        for k in *keys {
            x = overlay::keycap(painter, pos2(x + overlay::keycap_width(painter, k), center.y), k).right() + 4.0;
        }
        x = painter.text(pos2(x + 6.0, center.y), Align2::LEFT_CENTER, *what, font.clone(), TEXT).right() + gap;
    }
}

fn idle(painter: &Painter, rect: Rect, no_font: bool) {
    let c = rect.center();
    let size = (rect.height() * 0.1).clamp(28.0, 84.0);
    painter.text(c - vec2(0.0, size * 1.45), Align2::CENTER_BOTTOM, icons::MIC, FontId::proportional(size * 0.9), ACCENT);
    painter.text(c - vec2(0.0, size * 0.2), Align2::CENTER_BOTTOM, "พร้อมร้อง", FontId::new(size, lyrics_family()), palette(painter).sung);
    key_hints(painter, c + vec2(0.0, size * 0.1 + 12.0), &[(&["/"], "ค้นหาเพลง"), (&["Ctrl", "K"], "คำสั่งทั้งหมด")]);
    if no_font {
        painter.text(
            c + vec2(0.0, size * 0.1 + 30.0),
            Align2::CENTER_TOP,
            "ยังไม่มี SoundFont — เนื้อร้องจะเลื่อนตามเพลงแต่ไม่มีเสียงดนตรี (แท็บ เสียง: S)",
            FontId::proportional(13.0),
            DIM,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::Syllable;

    fn line(start: f64, end: f64) -> Line {
        Line { text: "x".into(), syllables: vec![Syllable { text: "x".into(), start, end }], start, end, beat: 0.5 }
    }

    #[test]
    fn stage_moves_on_before_the_next_line() {
        // 120 BPM: a two second count-in.
        let lines = [line(10.0, 12.0), line(13.0, 15.0), line(30.0, 32.0)];
        assert_eq!(stage_line(&lines, 0, 11.0), 0);
        // Line 0 done, line 1 starts within the look-ahead.
        assert_eq!(stage_line(&lines, 0, 12.2), 1);
        // Long rest after line 1: stay until just before the count-in.
        assert_eq!(stage_line(&lines, 1, 20.0), 1);
        assert_eq!(stage_line(&lines, 1, 27.4), 1);
        assert_eq!(stage_line(&lines, 1, 27.6), 2);
        assert_eq!(stage_line(&lines, 2, 40.0), 2);
        // Before the first line starts nothing moves.
        assert_eq!(stage_line(&lines, 0, 5.0), 0);
    }
}
