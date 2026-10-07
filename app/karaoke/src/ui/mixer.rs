//! The mixer: a panel of its own, docked above the bottom bar so the
//! lyrics stay visible. Always 16 channel strips (channel 10 is the fader
//! for the whole drum kit), then each piece of the kit, the reverb and
//! chorus returns and the master; on the right, the master effect chain
//! (`ui::effects`). Strips are the engine's own (gain, pan,
//! mute, solo, reverb and chorus sends) with live meters; pan and sends
//! start from the song's own controllers (CC 10, 91, 93).

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Margin, Painter, Rect, Sense, Stroke, pos2, vec2};
use solfege_synth::engine::mixer::{FxParams, StripParams};

use crate::app::KaraokeApp;
use crate::icons;
use crate::ui::effects;
use crate::style::{self, ACCENT, DANGER, DIM, INK, LINE, RAISED, SUNG, TEXT};
use crate::synth::{DRUM_CH, KIT, MELODY_CH, StripId, kit_name, volume_db};

/// Panel height, strips included.
pub const HEIGHT: f32 = 384.0;
const H: f32 = 298.0;
/// Room under the strips for their scrollbar.
const SCROLLBAR: f32 = 14.0;
/// Strip width range: wider screens spread them out, narrower ones scroll.
const MIN_W: f32 = 56.0;
const MAX_W: f32 = 66.0;
const GAP: f32 = 3.0;
const GROUP_GAP: f32 = 10.0;
const STRIPS: f32 = 16.0 + KIT as f32 + 3.0;
/// Fader range in dB.
const MIN_DB: f32 = -60.0;
const MAX_DB: f32 = 12.0;
const REVERB: Color32 = Color32::from_rgb(0xa7, 0x8b, 0xfa);
const CHORUS: Color32 = Color32::from_rgb(0x60, 0xa5, 0xfa);
/// Where the send rows and the fader start, from the top of a strip.
const SENDS_Y: f32 = 94.0;
const FADER_Y: f32 = 134.0;

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

    egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 16, bottom: 10 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(egui::RichText::new(format!("{}  มิกเซอร์", icons::MIXER)).strong().color(TEXT));
            ui.label(egui::RichText::new("ลากเพื่อปรับ · ดับเบิลคลิกค่าเริ่มต้น · REV / CHO ส่งเข้าเอฟเฟกต์ · ช่อง 10 คุมกลองทั้งชุด").size(12.0).color(DIM));
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
        // Strips on the left, the master effect chain in a sidebar on the right.
        let strips_w = ui.available_width() - effects::SIDEBAR - GROUP_GAP;
        let body = vec2(ui.available_width(), H + 20.0 + SCROLLBAR);
        let (row, _) = ui.allocate_exact_size(body, Sense::hover());
        let strips_rect = Rect::from_min_size(row.min, vec2(strips_w, body.y));
        let side_rect = Rect::from_min_max(pos2(row.right() - effects::SIDEBAR, row.top()), row.max);
        ui.painter().vline(side_rect.left() - GROUP_GAP / 2.0, row.y_range(), Stroke::new(1.0, LINE));
        ui.scope_builder(egui::UiBuilder::new().max_rect(side_rect), |ui| effects::sidebar(app, ui));
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(strips_rect));
        let ui = &mut ui;
        // Fit every strip across; scroll only when it is narrow.
        // Strips keep a readable width; when they do not fit they scroll
        // sideways (the mouse wheel scrolls them too, no Shift needed).
        let w = ((strips_w - GROUP_GAP * 3.0) / STRIPS - GAP).clamp(MIN_W, MAX_W);
        let width = STRIPS * (w + GAP) + GROUP_GAP * 3.0;
        let overflow = width > strips_w + 0.5;
        ui.style_mut().always_scroll_the_only_direction = true;
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        let bar = if overflow { egui::scroll_area::ScrollBarVisibility::AlwaysVisible } else { egui::scroll_area::ScrollBarVisibility::AlwaysHidden };
        egui::ScrollArea::horizontal().auto_shrink([false, true]).scroll_bar_visibility(bar).show(ui, |ui| {
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
    if matches!(c.kind, Kind::Strip(StripId::Channel(MELODY_CH))) && app.synth.melody_off() {
        p.text(rect.right_top() + vec2(-6.0, 8.0), Align2::RIGHT_TOP, icons::MIC_OFF, FontId::proportional(11.0), DANGER);
    }
    let name_clip = Rect::from_min_max(rect.left_top() + vec2(6.0, 26.0), pos2(rect.right() - 4.0, rect.top() + 42.0));
    p.with_clip_rect(name_clip).text(name_clip.left_center(), Align2::LEFT_CENTER, &c.name, FontId::proportional(10.0), DIM);
    let header = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + 42.0));
    let name_hover = ui.interact(header, id.with("name"), Sense::click());
    name_hover.on_hover_text(&c.name).context_menu(|ui| column_menu(app, ui, &c.kind));

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

        // Reverb / chorus sends: the song's CC 91 / 93 plus the user's
        // offset, like the pan.
        let (rev_midi, cho_midi) = app.synth.midi_sends(match s {
            StripId::Channel(ch) => ch,
            StripId::Kit(_) => DRUM_CH,
        });
        let mut params = app.synth.strip(s);
        for (k, (label, color, midi, add)) in
            [("REV", REVERB, rev_midi, &mut params.reverb_add), ("CHO", CHORUS, cho_midi, &mut params.chorus_add)].into_iter().enumerate()
        {
            let r = row_rect(rect, SENDS_Y + k as f32 * 19.0);
            let heard = (midi + *add).clamp(0.0, 1.0);
            let marker = (add.abs() > 0.004).then_some(midi);
            let resp = bar(ui, &p, r, id.with(label), label, heard, marker, color, &format!("{:.0}", heard * 127.0));
            if resp.double_clicked() {
                *add = 0.0;
            } else if let Some(v) = drag_value(&resp, r) {
                *add = v - midi;
            }
            let name = if k == 0 { "ส่งเข้ารีเวิร์บ" } else { "ส่งเข้าคอรัส" };
            let cc = if k == 0 { 91 } else { 93 };
            let tip = if marker.is_some() {
                format!("{name} · เพลง {:.0} · ปรับ {:+.0} · ได้ {:.0}\nดับเบิลคลิกเพื่อกลับไปตามเพลง", midi * 127.0, *add * 127.0, heard * 127.0)
            } else {
                format!("{name} (MIDI CC{cc}): {:.0} จาก 127 · ลากเพื่อปรับ", midi * 127.0)
            };
            resp.on_hover_text(tip);
        }
        if params != app.synth.strip(s) {
            app.synth.set_strip(s, params);
        }
    }

    // Effect parameters on the return strips.
    if matches!(c.kind, Kind::Reverb | Kind::Chorus) {
        fx_controls(app, ui, &p, rect, id, matches!(c.kind, Kind::Reverb));
    }

    // Fader with meters.
    let fader = Rect::from_min_max(pos2(rect.left() + 8.0, rect.top() + FADER_Y), pos2(rect.right() - 8.0, rect.bottom() - 28.0));
    let (norm, peaks, label) = match c.kind {
        Kind::Strip(s) => {
            let params = app.synth.strip(s);
            (db_to_norm(params.gain_db), app.synth.strip_peak(s), db_text(params.gain_db))
        }
        Kind::Reverb => {
            let peak = app.synth.fx_peak().0;
            (app.synth.mixer().fx.reverb_return / 2.0, (peak, peak), format!("{:.0}%", app.synth.mixer().fx.reverb_return * 100.0))
        }
        Kind::Chorus => {
            let peak = app.synth.fx_peak().1;
            (app.synth.mixer().fx.chorus_return / 2.0, (peak, peak), format!("{:.0}%", app.synth.mixer().fx.chorus_return * 100.0))
        }
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
                app.synth.set_fx(FxParams { reverb_return: n * 2.0, ..fx });
            }
            Kind::Chorus => {
                let fx = app.synth.mixer().fx;
                app.synth.set_fx(FxParams { chorus_return: n * 2.0, ..fx });
            }
            Kind::Master => app.synth.set_volume(n),
        }
    }
    let muted = strip.is_some_and(|s| app.synth.strip(s).mute || (s == StripId::Channel(MELODY_CH) && app.synth.melody_off()));
    draw_fader(&p, fader, norm, peaks, resp.hovered() || resp.dragged(), muted, true);
    resp.on_hover_text(match c.kind {
        Kind::Reverb => "ระดับรีเวิร์บที่กลับเข้ามิกซ์ (มิเตอร์คือเสียงที่ส่งเข้า) · ดับเบิลคลิกเพื่อค่าเริ่มต้น",
        Kind::Chorus => "ระดับคอรัสที่กลับเข้ามิกซ์ (มิเตอร์คือเสียงที่ส่งเข้า) · ดับเบิลคลิกเพื่อค่าเริ่มต้น",
        _ => "ลากเพื่อปรับ · ดับเบิลคลิกเพื่อค่าเริ่มต้น · คลิกขวาดูเมนู",
    })
    .context_menu(|ui| column_menu(app, ui, &c.kind));

    p.text(pos2(rect.center().x, rect.bottom() - 14.0), Align2::CENTER_CENTER, label, FontId::monospace(11.0), if muted { DIM } else { TEXT });
}

/// Right click on a strip: mute / solo, back to the song's values, and
/// the sound settings of that channel.
fn column_menu(app: &mut KaraokeApp, ui: &mut egui::Ui, kind: &Kind) {
    use crate::ui::menu::{heading, item, item_if, toggle};
    use crate::ui::sound::{SoundPanel, Tab};
    match *kind {
        Kind::Strip(s) => {
            let p = app.synth.strip(s);
            let title = match s {
                StripId::Channel(ch) => format!("แชนแนล {}  ·  {}", ch + 1, app.synth.channel_sound(ch).unwrap_or("ไม่ได้ใช้")),
                StripId::Kit(g) => format!("ชุดกลอง  ·  {}", kit_name(g)),
            };
            heading(ui, &title);
            if toggle(ui, p.mute, icons::VOLUME, "ปิดเสียง", "") {
                app.synth.set_strip(s, StripParams { mute: !p.mute, ..p });
            }
            if toggle(ui, p.solo, icons::HEADPHONES, "โซโล่", "") {
                app.synth.set_strip(s, StripParams { solo: !p.solo, ..p });
            }
            if s == StripId::Channel(MELODY_CH) && toggle(ui, app.synth.melody_off(), icons::MIC_OFF, "ปิดเมโลดี้ร้องนำ (ทุกเพลง)", "V") {
                app.toggle_melody();
            }
            ui.separator();
            if item_if(ui, p.gain_db != 0.0, icons::UNDO, "ความดัง 0 dB", "") {
                app.synth.set_strip(s, StripParams { gain_db: 0.0, ..p });
            }
            if item_if(ui, p.pan != 0.0, icons::UNDO, "แพนตามเพลง", "") {
                app.synth.set_strip(s, StripParams { pan: 0.0, ..p });
            }
            if item_if(ui, p.reverb_add != 0.0 || p.chorus_add != 0.0, icons::UNDO, "รีเวิร์บ / คอรัสตามเพลง", "") {
                app.synth.set_strip(s, StripParams { reverb_add: 0.0, chorus_add: 0.0, ..p });
            }
            if item_if(ui, p != StripParams::default(), icons::RESTART, "รีเซ็ตช่องนี้ทั้งหมด", "") {
                app.synth.set_strip(s, StripParams::default());
            }
            ui.separator();
            let drums = matches!(s, StripId::Kit(_) | StripId::Channel(DRUM_CH));
            let (icon, label, tab) =
                if drums { (icons::DRUM, "ชุดกลองและ SoundFont กลอง…", Tab::Drums) } else { (icons::FILE_MUSIC, "เสียงและ SoundFont ของแชนแนล…", Tab::Channels) };
            if item(ui, icon, label, "") {
                app.open_sound();
                app.sound = Some(SoundPanel::at(tab));
            }
        }
        Kind::Reverb | Kind::Chorus => {
            let reverb = matches!(kind, Kind::Reverb);
            heading(ui, if reverb { "รีเวิร์บ" } else { "คอรัส" });
            let fx = app.synth.mixer().fx;
            let d = FxParams::default();
            let changed = if reverb {
                (fx.reverb_return, fx.reverb_room, fx.reverb_damp, fx.reverb_width) != (d.reverb_return, d.reverb_room, d.reverb_damp, d.reverb_width)
            } else {
                (fx.chorus_return, fx.chorus_rate, fx.chorus_depth, fx.chorus_delay) != (d.chorus_return, d.chorus_rate, d.chorus_depth, d.chorus_delay)
            };
            let off = if reverb { fx.reverb_return == 0.0 } else { fx.chorus_return == 0.0 };
            if toggle(ui, off, icons::VOLUME, "ปิดเอฟเฟกต์นี้", "") {
                let level = if off { 0.5 } else { 0.0 };
                app.synth.set_fx(if reverb { FxParams { reverb_return: level, ..fx } } else { FxParams { chorus_return: level, ..fx } });
            }
            if item_if(ui, changed, icons::RESTART, "ค่าเริ่มต้น", "") {
                app.synth.set_fx(if reverb {
                    FxParams { reverb_return: d.reverb_return, reverb_room: d.reverb_room, reverb_damp: d.reverb_damp, reverb_width: d.reverb_width, ..fx }
                } else {
                    FxParams { chorus_return: d.chorus_return, chorus_rate: d.chorus_rate, chorus_depth: d.chorus_depth, chorus_delay: d.chorus_delay, ..fx }
                });
            }
        }
        Kind::Master => {
            heading(ui, "รวม");
            if item(ui, icons::UNDO, "ระดับเสียง 80%", "") {
                app.synth.set_volume(0.8);
            }
        }
    }
    ui.separator();
    if item_if(ui, app.synth.mixer_touched(), icons::VOLUME, "เปิดเสียงทุกช่อง", "") {
        for id in strip_ids() {
            let p = app.synth.strip(id);
            app.synth.set_strip(id, StripParams { mute: false, solo: false, ..p });
        }
    }
    if item(ui, icons::RESTART, "รีเซ็ตแชนแนลของเพลงนี้", "") {
        app.synth.reset_channels();
    }
    if item(ui, icons::REMOVE, "ปิดมิกเซอร์", "M") {
        app.mixer_open = false;
    }
}

/// Label, tooltip, value, default, min, max and how to show the value.
type FxRow<'a> = (&'a str, &'a str, &'a mut f32, f32, f32, f32, fn(f32) -> String);

/// Reverb room / damping / width, or chorus rate / depth / delay, as bars
/// where the channel strips have pan and mute / solo.
fn fx_controls(app: &mut KaraokeApp, ui: &mut egui::Ui, p: &Painter, rect: Rect, id: egui::Id, reverb: bool) {
    let mut fx = app.synth.mixer().fx;
    let defaults = FxParams::default();
    let color = if reverb { REVERB } else { CHORUS };
    let rows: [FxRow; 3] = if reverb {
        [
            ("ROOM", "ขนาดห้อง", &mut fx.reverb_room, defaults.reverb_room, 0.0, 1.0, |v| format!("{:.0}", v * 100.0)),
            ("DAMP", "ความอับของเสียงสะท้อน (ตัดเสียงแหลม)", &mut fx.reverb_damp, defaults.reverb_damp, 0.0, 1.0, |v| format!("{:.0}", v * 100.0)),
            ("WIDE", "ความกว้างสเตอริโอ", &mut fx.reverb_width, defaults.reverb_width, 0.0, 1.0, |v| format!("{:.0}", v * 100.0)),
        ]
    } else {
        [
            ("RATE", "ความเร็วการสั่น (Hz)", &mut fx.chorus_rate, defaults.chorus_rate, 0.05, 8.0, |v| format!("{v:.1}")),
            ("DPTH", "ความลึก (ms)", &mut fx.chorus_depth, defaults.chorus_depth, 0.0, 15.0, |v| format!("{v:.1}")),
            ("DLY", "ดีเลย์ (ms)", &mut fx.chorus_delay, defaults.chorus_delay, 2.0, 30.0, |v| format!("{v:.0}")),
        ]
    };
    for (k, (label, tip, value, default, min, max, text)) in rows.into_iter().enumerate() {
        let r = row_rect(rect, 46.0 + k as f32 * 19.0);
        let norm = (*value - min) / (max - min);
        let resp = bar(ui, p, r, id.with(label), label, norm, None, color, &text(*value));
        if resp.double_clicked() {
            *value = default;
        } else if let Some(v) = drag_value(&resp, r) {
            *value = min + v * (max - min);
        }
        resp.on_hover_text(format!("{tip}: {} · ดับเบิลคลิกเพื่อค่าเริ่มต้น", text(*value)));
    }
    if fx != app.synth.mixer().fx {
        app.synth.set_fx(fx);
    }
}

fn row_rect(strip: Rect, y: f32) -> Rect {
    Rect::from_min_size(pos2(strip.left() + 5.0, strip.top() + y), vec2(strip.width() - 10.0, 16.0))
}

/// A small horizontal bar: label on the left, value on the right, filled
/// to `value` (0..1). `marker` shows where the song had it.
#[allow(clippy::too_many_arguments)]
fn bar(ui: &mut egui::Ui, p: &Painter, r: Rect, id: egui::Id, label: &str, value: f32, marker: Option<f32>, color: Color32, text: &str) -> egui::Response {
    let resp = ui.interact(r, id, Sense::click_and_drag());
    let hot = resp.hovered() || resp.dragged();
    p.rect_filled(r, CornerRadius::same(4), INK);
    let fill = Rect::from_min_max(r.min, pos2(r.left() + r.width() * value.clamp(0.0, 1.0), r.bottom()));
    p.rect_filled(fill, CornerRadius::same(4), color.gamma_multiply(if hot { 0.55 } else { 0.38 }));
    if let Some(m) = marker {
        let x = r.left() + r.width() * m.clamp(0.0, 1.0);
        p.vline(x, (r.top() + 2.0)..=(r.bottom() - 2.0), Stroke::new(1.0, TEXT.gamma_multiply(0.7)));
    }
    if hot {
        p.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, color), egui::StrokeKind::Inside);
    }
    // Full label when it fits beside the value, else its first letter.
    let font = FontId::monospace(8.5);
    let room = r.width() - 9.0 - p.layout_no_wrap(text.to_string(), font.clone(), TEXT).size().x;
    let label = if p.layout_no_wrap(label.to_string(), font.clone(), TEXT).size().x <= room { label } else { &label[..1] };
    p.text(pos2(r.left() + 3.0, r.center().y), Align2::LEFT_CENTER, label, font.clone(), if marker.is_some() { SUNG } else { DIM });
    p.text(pos2(r.right() - 3.0, r.center().y), Align2::RIGHT_CENTER, text, font, TEXT);
    resp
}

/// The 0..1 position a drag on a bar asks for.
fn drag_value(resp: &egui::Response, r: Rect) -> Option<f32> {
    let pos = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked())?;
    Some(((pos.x - r.left()) / r.width()).clamp(0.0, 1.0))
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
