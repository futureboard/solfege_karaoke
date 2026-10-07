//! The mixer: a panel of its own, docked above the bottom bar so the
//! lyrics stay visible. Always 16 channel strips (channel 10 is the fader
//! for the whole drum kit), then each piece of the kit, the reverb and
//! chorus returns and the master. Strips are the engine's own (gain, pan,
//! mute, solo) with live meters.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Margin, Painter, Rect, Sense, Stroke, pos2, vec2};
use solfege_synth::engine::mixer::StripParams;

use crate::app::KaraokeApp;
use crate::icons;
use crate::style::{self, ACCENT, DANGER, DIM, INK, LINE, RAISED, SUNG, TEXT};
use crate::synth::{DRUM_CH, KIT, StripId, kit_name, volume_db};

/// Panel height, strips included.
pub const HEIGHT: f32 = 336.0;
const H: f32 = 270.0;
const GAP: f32 = 3.0;
const GROUP_GAP: f32 = 14.0;
const STRIPS: f32 = 16.0 + KIT as f32 + 3.0;
/// Fader range in dB.
const MIN_DB: f32 = -60.0;
const MAX_DB: f32 = 12.0;

/// Fader travel (0..1) for a gain, squared so the useful range near 0 dB
/// gets most of the length.
pub fn db_to_norm(db: f32) -> f32 {
    if db <= MIN_DB { 0.0 } else { ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0).powi(2) }
}

pub fn norm_to_db(n: f32) -> f32 {
    let db = n.clamp(0.0, 1.0).sqrt() * (MAX_DB - MIN_DB) + MIN_DB;
    if db <= MIN_DB + 0.5 { MIN_DB } else { (db * 10.0).round() / 10.0 }
}

/// Meter height (0..1) for a linear peak, -48..+6 dB.
fn meter(peak: f32) -> f32 {
    if peak <= 1e-5 { 0.0 } else { ((20.0 * peak.log10() + 48.0) / 54.0).clamp(0.0, 1.0) }
}

enum Kind {
    Strip(StripId),
    Reverb,
    Chorus,
    Master,
}

struct Column {
    kind: Kind,
    number: String,
    name: String,
    /// The song plays on this strip (others are drawn dimmed).
    used: bool,
}

pub fn show(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let used = app.synth.channels_used();
    let channels: Vec<Column> = (0..16)
        .map(|ch| Column {
            kind: Kind::Strip(StripId::Channel(ch)),
            number: if ch == DRUM_CH { format!("{} 10", icons::DRUM) } else { (ch + 1).to_string() },
            name: app.synth.channel_sound(ch).unwrap_or(if ch == DRUM_CH { "Drums" } else { "—" }).to_string(),
            used: used & (1 << ch) != 0,
        })
        .collect();
    let drums = used & (1 << DRUM_CH) != 0;
    let kit: Vec<Column> = (0..KIT)
        .map(|g| Column { kind: Kind::Strip(StripId::Kit(g)), number: icons::DRUM.into(), name: kit_name(g).into(), used: drums })
        .collect();
    let fx = vec![
        Column { kind: Kind::Reverb, number: "FX".into(), name: "Reverb".into(), used: true },
        Column { kind: Kind::Chorus, number: "FX".into(), name: "Chorus".into(), used: true },
    ];
    let master = vec![Column { kind: Kind::Master, number: icons::VOLUME.into(), name: "Master".into(), used: true }];
    let groups: [(&str, Vec<Column>); 4] = [("แชนแนล 1–16", channels), ("ชุดกลอง (ช่อง 10)", kit), ("เอฟเฟกต์", fx), ("รวม", master)];

    egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 10, bottom: 10 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(egui::RichText::new(format!("{}  มิกเซอร์", icons::MIXER)).strong().color(TEXT));
            ui.label(egui::RichText::new("ลากเพื่อปรับ · ดับเบิลคลิกค่าเริ่มต้น · ช่อง 10 คุมกลองทั้งชุด").size(12.0).color(DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(icons::REMOVE).on_hover_text("ปิดมิกเซอร์ (M)").clicked() {
                    app.mixer_open = false;
                }
                if ui.small_button(format!("{}  รีเซ็ตแชนแนลของเพลงนี้", icons::RESTART)).clicked() {
                    app.synth.reset_channels();
                }
                if ui.add_enabled(app.synth.mixer_touched(), egui::Button::new(format!("{}  เปิดเสียงทุกช่อง", icons::VOLUME)).small()).clicked() {
                    for id in strip_ids() {
                        let p = app.synth.strip(id);
                        app.synth.set_strip(id, StripParams { mute: false, solo: false, ..p });
                    }
                }
            });
        });
        ui.add_space(6.0);
        // Fit every strip across the window; scroll only when it is narrow.
        let w = ((ui.available_width() - GROUP_GAP * 3.0) / STRIPS - GAP).clamp(40.0, 60.0);
        let width = STRIPS * (w + GAP) + GROUP_GAP * 3.0;
        egui::ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
            let (area, _) = ui.allocate_exact_size(vec2(width.max(ui.available_width()), H + 20.0), Sense::hover());
            let mut x = area.left();
            for (title, cols) in &groups {
                ui.painter().text(pos2(x, area.top()), Align2::LEFT_TOP, *title, FontId::proportional(11.0), DIM);
                let top = area.top() + 20.0;
                for c in cols {
                    column(app, ui, c, Rect::from_min_size(pos2(x, top), vec2(w, H)));
                    x += w + GAP;
                }
                x += GROUP_GAP - GAP;
            }
        });
    });
    // Meters move with the music.
    ui.ctx().request_repaint();
}

fn strip_ids() -> impl Iterator<Item = StripId> {
    (0..16).map(StripId::Channel).chain((0..KIT).map(StripId::Kit))
}

fn column(app: &mut KaraokeApp, ui: &mut egui::Ui, c: &Column, rect: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(rect, CornerRadius::same(8), if c.used { RAISED } else { style::mix(INK, RAISED, 0.5) });
    let id = ui.id().with(("mix", &c.name, &c.number));

    // Header: number and the sound's name.
    let head = if c.used { TEXT } else { DIM };
    p.text(rect.left_top() + vec2(6.0, 7.0), Align2::LEFT_TOP, &c.number, FontId::proportional(12.0), head);
    if matches!(c.kind, Kind::Strip(StripId::Channel(DRUM_CH))) && app.synth.drum_lock().is_some() {
        p.text(rect.right_top() + vec2(-6.0, 8.0), Align2::RIGHT_TOP, icons::LOCK, FontId::proportional(11.0), SUNG);
    }
    let name_clip = Rect::from_min_max(rect.left_top() + vec2(6.0, 26.0), pos2(rect.right() - 4.0, rect.top() + 42.0));
    p.with_clip_rect(name_clip).text(name_clip.left_center(), Align2::LEFT_CENTER, &c.name, FontId::proportional(10.0), DIM);
    let name_hover = ui.interact(name_clip, id.with("name"), Sense::hover());
    name_hover.on_hover_text(&c.name);

    let strip = match c.kind {
        Kind::Strip(s) => Some(s),
        _ => None,
    };

    // Pan (strips only): the song's own pan (MIDI CC 10) plus the user's
    // offset. The hollow mark is where the song puts it, the knob where it
    // ends up; dragging moves the knob, double-click returns to the song.
    let pan_r = Rect::from_min_size(pos2(rect.left() + 7.0, rect.top() + 43.0), vec2(rect.width() - 14.0, 24.0));
    if let Some(s) = strip {
        let mut params = app.synth.strip(s);
        let midi = match s {
            StripId::Channel(ch) => app.synth.midi_pan(ch),
            StripId::Kit(_) => app.synth.midi_pan(DRUM_CH),
        };
        let resp = ui.interact(pan_r, id.with("pan"), Sense::click_and_drag());
        if resp.double_clicked() {
            params.pan = 0.0;
        } else if let Some(pos) = resp.interact_pointer_pos()
            && resp.dragged()
        {
            let want = ((pos.x - pan_r.left()) / pan_r.width() * 2.0 - 1.0).clamp(-1.0, 1.0);
            params.pan = (want - midi).clamp(-1.0, 1.0);
        }
        let heard = (midi + params.pan).clamp(-1.0, 1.0);
        let track = Rect::from_center_size(pos2(pan_r.center().x, pan_r.bottom() - 4.0), vec2(pan_r.width(), 3.0));
        p.rect_filled(track, 1.5, LINE);
        let x_of = |v: f32| pan_r.center().x + v * pan_r.width() / 2.0;
        let cx = pan_r.center().x;
        p.vline(cx, (track.top() - 2.0)..=(track.bottom() + 2.0), Stroke::new(1.0, DIM.gamma_multiply(0.6)));
        let hx = x_of(heard);
        p.rect_filled(Rect::from_min_max(pos2(cx.min(hx), track.top()), pos2(cx.max(hx), track.bottom())), 1.5, ACCENT);
        if params.pan.abs() > 0.005 {
            p.circle_stroke(pos2(x_of(midi), track.center().y), 3.5, Stroke::new(1.0, DIM));
        }
        let hot = resp.hovered() || resp.dragged();
        let knob = if params.pan.abs() > 0.005 { SUNG } else { TEXT };
        let knob_r = Rect::from_center_size(pos2(hx, track.center().y), vec2(if hot { 4.0 } else { 3.0 }, 10.0));
        p.rect_filled(knob_r, 1.5, knob);
        p.text(pos2(pan_r.center().x, pan_r.top()), Align2::CENTER_TOP, pan_text(heard), FontId::monospace(9.0), if params.pan.abs() > 0.005 { SUNG } else { DIM });
        let tip = if params.pan.abs() > 0.005 {
            format!("เพลง {} · ปรับ {:+.0} · ได้ {}\nดับเบิลคลิกเพื่อกลับไปตามเพลง", pan_text(midi), params.pan * 100.0, pan_text(heard))
        } else {
            format!("แพนจากเพลง (MIDI CC10): {}", pan_text(midi))
        };
        resp.on_hover_text(tip);
        if params != app.synth.strip(s) {
            app.synth.set_strip(s, params);
        }

        // Mute / solo.
        let tw = ((rect.width() - 14.0) / 2.0).min(22.0);
        let m_r = Rect::from_min_size(pos2(rect.left() + 5.0, rect.top() + 70.0), vec2(tw, 18.0));
        let s_r = Rect::from_min_size(pos2(rect.right() - 5.0 - tw, rect.top() + 70.0), vec2(tw, 18.0));
        let mut params = app.synth.strip(s);
        if toggle(ui, &p, m_r, id.with("m"), "M", params.mute, DANGER).on_hover_text("ปิดเสียง").clicked() {
            params.mute = !params.mute;
        }
        if toggle(ui, &p, s_r, id.with("s"), "S", params.solo, SUNG).on_hover_text("โซโล่").clicked() {
            params.solo = !params.solo;
        }
        if params != app.synth.strip(s) {
            app.synth.set_strip(s, params);
        }
    }

    // Fader with meters.
    let fader = Rect::from_min_max(pos2(rect.left() + 8.0, rect.top() + 98.0), pos2(rect.right() - 8.0, rect.bottom() - 28.0));
    let (norm, peaks, label) = match c.kind {
        Kind::Strip(s) => {
            let params = app.synth.strip(s);
            (db_to_norm(params.gain_db), app.synth.strip_peak(s), db_text(params.gain_db))
        }
        Kind::Reverb => (app.synth.mixer().fx.reverb_return / 2.0, (0.0, 0.0), format!("{:.0}%", app.synth.mixer().fx.reverb_return * 100.0)),
        Kind::Chorus => (app.synth.mixer().fx.chorus_return / 2.0, (0.0, 0.0), format!("{:.0}%", app.synth.mixer().fx.chorus_return * 100.0)),
        Kind::Master => (app.synth.volume(), app.synth.master_peak(), db_text(volume_db(app.synth.volume()))),
    };
    let resp = ui.interact(fader, id.with("fader"), Sense::click_and_drag());
    let new_norm = if resp.double_clicked() {
        Some(match c.kind {
            Kind::Strip(_) => db_to_norm(0.0),
            Kind::Reverb | Kind::Chorus => 0.25,
            Kind::Master => 0.8,
        })
    } else if resp.dragged()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        Some(((fader.bottom() - pos.y) / fader.height()).clamp(0.0, 1.0))
    } else {
        None
    };
    if let Some(n) = new_norm {
        match c.kind {
            Kind::Strip(s) => {
                let params = app.synth.strip(s);
                app.synth.set_strip(s, StripParams { gain_db: norm_to_db(n), ..params });
            }
            Kind::Reverb => {
                let fx = app.synth.mixer().fx;
                app.synth.set_fx(solfege_synth::engine::mixer::FxParams { reverb_return: n * 2.0, ..fx });
            }
            Kind::Chorus => {
                let fx = app.synth.mixer().fx;
                app.synth.set_fx(solfege_synth::engine::mixer::FxParams { chorus_return: n * 2.0, ..fx });
            }
            Kind::Master => app.synth.set_volume(n),
        }
    }
    let muted = strip.is_some_and(|s| app.synth.strip(s).mute);
    draw_fader(&p, fader, norm, peaks, resp.hovered() || resp.dragged(), muted, strip.is_some() || matches!(c.kind, Kind::Master));
    resp.on_hover_text("ลากเพื่อปรับ · ดับเบิลคลิกเพื่อค่าเริ่มต้น");

    p.text(pos2(rect.center().x, rect.bottom() - 14.0), Align2::CENTER_CENTER, label, FontId::monospace(11.0), if muted { DIM } else { TEXT });
}

fn toggle(ui: &mut egui::Ui, p: &Painter, r: Rect, id: egui::Id, text: &str, on: bool, color: Color32) -> egui::Response {
    let resp = ui.interact(r, id, Sense::click());
    let fill = if on { color } else if resp.hovered() { style::mix(INK, TEXT, 0.08) } else { INK };
    p.rect_filled(r, CornerRadius::same(5), fill);
    p.text(r.center(), Align2::CENTER_CENTER, text, FontId::proportional(11.0), if on { INK } else { DIM });
    resp
}

fn draw_fader(p: &Painter, r: Rect, norm: f32, peaks: (f32, f32), hot: bool, muted: bool, metered: bool) {
    // Track on the left, stereo meter on the right.
    let track_x = r.left() + 12.0;
    let track = Rect::from_center_size(pos2(track_x, r.center().y), vec2(4.0, r.height()));
    p.rect_filled(track, 2.0, INK);
    // 0 dB mark.
    let zero_y = r.bottom() - db_to_norm(0.0) * r.height();
    p.hline((track_x - 8.0)..=(track_x + 8.0), zero_y, Stroke::new(1.0, LINE));
    let cap_y = r.bottom() - norm * r.height();
    let mut fill = track;
    fill.set_top(cap_y);
    p.rect_filled(fill, 2.0, if muted { DIM.gamma_multiply(0.5) } else { ACCENT.gamma_multiply(0.7) });
    let cap = Rect::from_center_size(pos2(track_x, cap_y), vec2(22.0, 10.0));
    p.rect_filled(cap, CornerRadius::same(3), if hot { SUNG } else { TEXT });
    p.hline((cap.left() + 4.0)..=(cap.right() - 4.0), cap_y, Stroke::new(1.0, INK));

    if metered {
        for (k, peak) in [peaks.0, peaks.1].into_iter().enumerate() {
            let x = r.right() - 14.0 + k as f32 * 6.0;
            let bar = Rect::from_min_max(pos2(x, r.top()), pos2(x + 4.0, r.bottom()));
            p.rect_filled(bar, 1.0, INK);
            let m = if muted { 0.0 } else { meter(peak) };
            if m > 0.0 {
                let lit = Rect::from_min_max(pos2(bar.left(), bar.bottom() - m * bar.height()), bar.max);
                let color = if peak >= 1.0 { DANGER } else if peak >= 0.5 { SUNG } else { ACCENT };
                p.rect_filled(lit, 1.0, color);
            }
        }
    }
}

fn db_text(db: f32) -> String {
    if db <= MIN_DB { "-∞".into() } else { format!("{db:+.1}") }
}

/// `L32`, `C`, `R20` (MIDI-style, out of 64).
fn pan_text(pan: f32) -> String {
    let v = (pan * 64.0).round() as i32;
    match v {
        0 => "C".into(),
        v if v < 0 => format!("L{}", -v),
        v => format!("R{v}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fader_curve_round_trips() {
        assert_eq!(db_to_norm(MIN_DB), 0.0);
        assert_eq!(db_to_norm(MAX_DB), 1.0);
        for db in [-40.0, -12.0, -3.5, 0.0, 6.0] {
            assert!((norm_to_db(db_to_norm(db)) - db).abs() < 0.11, "{db}");
        }
        assert!(db_to_norm(0.0) > 0.6, "0 dB sits high on the fader");
        assert_eq!(norm_to_db(0.0), MIN_DB);
        assert_eq!(db_text(-60.0), "-∞");
        assert_eq!(pan_text(-0.5), "L32");
        assert_eq!(pan_text(0.0), "C");
        assert_eq!(pan_text(1.0), "R64");
    }
}
