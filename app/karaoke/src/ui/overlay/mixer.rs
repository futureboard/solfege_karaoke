//! Mixer: a strip for every MIDI part the song uses, one for each piece of
//! the drum kit, the reverb and chorus returns, and the master. Strips are
//! the engine's own (gain, pan, mute, solo) with live meters.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Margin, Painter, Rect, Sense, Stroke, pos2, vec2};
use solfege_synth::engine::mixer::StripParams;

use crate::app::KaraokeApp;
use crate::icons;
use crate::style::{self, ACCENT, DANGER, DIM, INK, LINE, RAISED, SUNG, TEXT};
use crate::synth::{KIT, StripId, kit_name, volume_db};

const W: f32 = 54.0;
const H: f32 = 300.0;
const GAP: f32 = 4.0;
const GROUP_GAP: f32 = 18.0;
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
}

pub fn show(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let used = app.synth.channels_used();
    let channels: Vec<Column> = (0..16)
        .filter(|&ch| ch != 9 && used & (1 << ch) != 0)
        .map(|ch| Column {
            kind: Kind::Strip(StripId::Channel(ch)),
            number: (ch + 1).to_string(),
            name: app.synth.channel_sound(ch).unwrap_or("—").to_string(),
        })
        .collect();
    let kit: Vec<Column> =
        (0..KIT).map(|g| Column { kind: Kind::Strip(StripId::Kit(g)), number: icons::DRUM.into(), name: kit_name(g).into() }).collect();
    let fx = vec![
        Column { kind: Kind::Reverb, number: "FX".into(), name: "Reverb".into() },
        Column { kind: Kind::Chorus, number: "FX".into(), name: "Chorus".into() },
    ];
    let master = vec![Column { kind: Kind::Master, number: icons::VOLUME.into(), name: "Master".into() }];
    let groups: [(&str, Vec<Column>); 4] = [("แชนแนล", channels), ("กลอง", kit), ("เอฟเฟกต์", fx), ("รวม", master)];

    egui::Frame::new().inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
        let width: f32 = groups.iter().map(|(_, c)| c.len().max(1) as f32 * (W + GAP) - GAP).sum::<f32>() + GROUP_GAP * 3.0;
        egui::ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
            let (area, _) = ui.allocate_exact_size(vec2(width.max(ui.available_width()), H + 22.0), Sense::hover());
            let mut x = area.left();
            for (title, cols) in &groups {
                ui.painter().text(pos2(x, area.top()), Align2::LEFT_TOP, *title, FontId::proportional(12.0), DIM);
                let top = area.top() + 22.0;
                if cols.is_empty() {
                    let r = Rect::from_min_size(pos2(x, top), vec2(W, H));
                    ui.painter().rect_stroke(r, CornerRadius::same(10), Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, "ยังไม่มีเพลง", FontId::proportional(11.0), DIM);
                    x += W + GROUP_GAP;
                    continue;
                }
                for c in cols {
                    column(app, ui, c, Rect::from_min_size(pos2(x, top), vec2(W, H)));
                    x += W + GAP;
                }
                x += GROUP_GAP - GAP;
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if ui.add_enabled(app.synth.mixer_touched(), egui::Button::new(format!("{}  เปิดเสียงทุกช่อง", icons::VOLUME))).clicked() {
                for id in strip_ids() {
                    let p = app.synth.strip(id);
                    app.synth.set_strip(id, StripParams { mute: false, solo: false, ..p });
                }
            }
            if ui.button(format!("{}  รีเซ็ตแชนแนลของเพลงนี้", icons::RESTART)).clicked() {
                app.synth.reset_channels();
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
    p.rect_filled(rect, CornerRadius::same(10), RAISED);
    let id = ui.id().with(("mix", &c.name, &c.number));

    // Header: number and the sound's name.
    p.text(rect.left_top() + vec2(8.0, 8.0), Align2::LEFT_TOP, &c.number, FontId::proportional(13.0), TEXT);
    let name_clip = Rect::from_min_max(rect.left_top() + vec2(6.0, 26.0), pos2(rect.right() - 4.0, rect.top() + 42.0));
    p.with_clip_rect(name_clip).text(name_clip.left_center(), Align2::LEFT_CENTER, &c.name, FontId::proportional(10.0), DIM);
    let name_hover = ui.interact(name_clip, id.with("name"), Sense::hover());
    name_hover.on_hover_text(&c.name);

    let strip = match c.kind {
        Kind::Strip(s) => Some(s),
        _ => None,
    };

    // Pan (strips only).
    let pan_r = Rect::from_min_size(pos2(rect.left() + 9.0, rect.top() + 48.0), vec2(rect.width() - 18.0, 14.0));
    if let Some(s) = strip {
        let mut params = app.synth.strip(s);
        let resp = ui.interact(pan_r, id.with("pan"), Sense::click_and_drag());
        if resp.double_clicked() {
            params.pan = 0.0;
        } else if let Some(pos) = resp.interact_pointer_pos()
            && resp.dragged()
        {
            params.pan = ((pos.x - pan_r.left()) / pan_r.width() * 2.0 - 1.0).clamp(-1.0, 1.0);
        }
        let track = Rect::from_center_size(pan_r.center(), vec2(pan_r.width(), 3.0));
        p.rect_filled(track, 1.5, LINE);
        let cx = pan_r.center().x;
        let px = cx + params.pan * pan_r.width() / 2.0;
        p.rect_filled(Rect::from_min_max(pos2(cx.min(px), track.top()), pos2(cx.max(px), track.bottom())), 1.5, ACCENT);
        p.circle_filled(pos2(px, track.center().y), if resp.hovered() || resp.dragged() { 5.0 } else { 4.0 }, TEXT);
        resp.on_hover_text(pan_text(params.pan));
        if params != app.synth.strip(s) {
            app.synth.set_strip(s, params);
        }

        // Mute / solo.
        let m_r = Rect::from_min_size(pos2(rect.left() + 7.0, rect.top() + 70.0), vec2(22.0, 18.0));
        let s_r = Rect::from_min_size(pos2(rect.right() - 29.0, rect.top() + 70.0), vec2(22.0, 18.0));
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

fn pan_text(pan: f32) -> String {
    let v = (pan * 100.0).round() as i32;
    match v {
        0 => "กลาง".into(),
        v if v < 0 => format!("ซ้าย {}", -v),
        v => format!("ขวา {v}"),
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
        assert_eq!(pan_text(-0.5), "ซ้าย 50");
    }
}
