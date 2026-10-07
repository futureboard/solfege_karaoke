//! The master effect chain: ten slots run in order 1 → 10 on the main
//! output. They sit in a sidebar on the right of the mixer panel; each
//! slot picks an effect, can be switched off (bypassed), and opens its
//! parameters in a popup editor over the stage.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Frame, Key, Margin, Modifiers, RichText, Sense, Shadow, Stroke, vec2};
use solfege_synth::engine::inserts::{INSERT_SLOTS, InsertKind, InsertParams};

use crate::app::KaraokeApp;
use crate::icons;
use crate::style::{ACCENT, DIM, INK, LINE, PANEL, RAISED, SUNG, TEXT};

/// Width of the sidebar in the mixer panel.
pub const SIDEBAR: f32 = 200.0;
const ROW: f32 = 24.0;

/// The ten slots, for the right side of the mixer.
pub fn sidebar(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{}  เอฟเฟกต์รวม", icons::EFFECTS)).size(11.0).color(DIM));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let used = app.synth.inserts().iter().flatten().filter(|p| !p.bypass).count();
            ui.label(RichText::new(format!("{used}/{INSERT_SLOTS}")).size(11.0).color(if used > 0 { ACCENT } else { DIM }));
        });
    });
    ui.add_space(2.0);
    for slot in 0..INSERT_SLOTS {
        slot_row(app, ui, slot);
    }
}

fn slot_row(app: &mut KaraokeApp, ui: &mut egui::Ui, slot: usize) {
    let params = app.synth.inserts()[slot];
    let on = params.is_some_and(|p| !p.bypass);
    let editing = app.effect_editor == Some(slot);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let p = ui.painter();
    let fill = if editing {
        RAISED
    } else if resp.hovered() {
        crate::style::mix(INK, RAISED, 0.6)
    } else {
        INK
    };
    p.rect_filled(rect, CornerRadius::same(6), fill);
    if editing {
        p.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, SUNG.gamma_multiply(0.6)), egui::StrokeKind::Inside);
    }
    let y = rect.center().y;
    p.text(egui::pos2(rect.left() + 13.0, y), Align2::CENTER_CENTER, (slot + 1).to_string(), FontId::monospace(10.0), if on { SUNG } else { DIM });

    // Power (bypass), only for a filled slot.
    let power = egui::Rect::from_center_size(egui::pos2(rect.left() + 36.0, y), vec2(20.0, 20.0));
    if let Some(prm) = params {
        let pr = ui.interact(power, ui.id().with(("fx-power", slot)), Sense::click());
        if pr.hovered() {
            ui.painter().rect_filled(power, CornerRadius::same(5), PANEL);
        }
        ui.painter().text(power.center(), Align2::CENTER_CENTER, icons::POWER, FontId::proportional(12.0), if prm.bypass { DIM } else { ACCENT });
        if pr.on_hover_text(if prm.bypass { "เปิดเอฟเฟกต์นี้" } else { "ปิดชั่วคราว (bypass)" }).clicked() {
            app.synth.set_insert(slot, Some(InsertParams { bypass: !prm.bypass, ..prm }));
        }
    }
    // Name; the row opens the editor.
    let name_left = rect.left() + 52.0;
    let clip = egui::Rect::from_min_max(egui::pos2(name_left, rect.top()), egui::pos2(rect.right() - 22.0, rect.bottom()));
    let (name, color) = match params {
        Some(prm) => (prm.kind.name(), if prm.bypass { DIM } else { TEXT }),
        None => ("— ว่าง —", DIM.gamma_multiply(0.7)),
    };
    ui.painter().with_clip_rect(clip).text(egui::pos2(name_left, y), Align2::LEFT_CENTER, name, FontId::proportional(12.0), color);
    let icon = if params.is_some() { icons::COLLAPSED } else { icons::PLUS };
    ui.painter().text(egui::pos2(rect.right() - 10.0, y), Align2::CENTER_CENTER, icon, FontId::proportional(11.0), DIM);
    let tip = if params.is_some() { "คลิกเพื่อปรับค่า · คลิกขวาดูเมนู" } else { "คลิกเพื่อใส่เอฟเฟกต์" };
    let resp = resp.on_hover_text(tip);
    if resp.clicked() && !power.contains(resp.interact_pointer_pos().unwrap_or_default()) {
        app.effect_editor = if editing { None } else { Some(slot) };
    }
    resp.context_menu(|ui| slot_menu(app, ui, slot));
}

/// The popup editor over the stage for the slot being edited.
pub fn editor(app: &mut KaraokeApp, ctx: &egui::Context) {
    let Some(slot) = app.effect_editor.filter(|&s| s < INSERT_SLOTS) else { return };
    if !crate::ui::popup_open(ctx) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        app.effect_editor = None;
        return;
    }
    let screen = ctx.content_rect();
    let outside = egui::Area::new(egui::Id::new("fx-editor-backdrop"))
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(110));
            resp.clicked()
        })
        .inner;
    let mut close = false;
    egui::Area::new(egui::Id::new("fx-editor"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, -40.0))
        .show(ctx, |ui| {
            Frame::new()
                .fill(PANEL)
                .stroke(Stroke::new(1.0, LINE))
                .corner_radius(CornerRadius::same(14))
                .shadow(Shadow { offset: [0, 16], blur: 48, spread: 0, color: Color32::from_black_alpha(170) })
                .inner_margin(Margin::same(18))
                .show(ui, |ui| {
                    ui.set_width(420.0);
                    close = editor_body(app, ui, slot);
                });
        });
    if close || (outside && !crate::ui::popup_open(ctx)) {
        app.effect_editor = None;
    }
}

/// Returns true when the editor should close.
fn editor_body(app: &mut KaraokeApp, ui: &mut egui::Ui, slot: usize) -> bool {
    let mut close = false;
    let params = app.synth.inserts()[slot];
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{}  เอฟเฟกต์ช่อง {}", icons::EFFECTS, slot + 1)).size(16.0).strong().color(TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close = ui.button(icons::REMOVE).on_hover_text("ปิด (Esc)").clicked();
            if ui.add_enabled(slot + 1 < INSERT_SLOTS, egui::Button::new(icons::COLLAPSED)).on_hover_text("ช่องถัดไป").clicked() {
                app.effect_editor = Some(slot + 1);
            }
            if ui.add_enabled(slot > 0, egui::Button::new(icons::PREVIOUS)).on_hover_text("ช่องก่อนหน้า").clicked() {
                app.effect_editor = Some(slot - 1);
            }
        });
    });
    ui.label(RichText::new("ทำงานกับเสียงรวมทั้งหมด ตามลำดับช่อง 1 → 10").size(11.0).color(DIM));
    ui.add_space(10.0);

    ui.horizontal(|ui| {
        ui.label(RichText::new("เอฟเฟกต์").color(DIM));
        let shown = match params {
            Some(p) => RichText::new(p.kind.name()).color(TEXT),
            None => RichText::new("— ว่าง —").color(DIM),
        };
        let mut pick = params.map(|p| p.kind);
        crate::ui::fixed_width(ui, 220.0, |ui| {
            egui::ComboBox::from_id_salt(("fx-kind", slot)).truncate().selected_text(shown).width(220.0).height(360.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut pick, None, RichText::new("— ว่าง —").color(DIM));
                for k in InsertKind::ALL {
                    ui.selectable_value(&mut pick, Some(k), k.name());
                }
            })
        });
        if pick != params.map(|p| p.kind) {
            app.synth.set_insert(slot, pick.map(InsertParams::new));
        }
        if let Some(p) = params {
            let label = if p.bypass { format!("{}  ปิดอยู่", icons::POWER) } else { format!("{}  ทำงาน", icons::POWER) };
            let button = egui::Button::new(RichText::new(label).color(if p.bypass { DIM } else { INK })).fill(if p.bypass { INK } else { ACCENT });
            if ui.add(button).on_hover_text("เปิด / ปิดชั่วคราว (bypass)").clicked() {
                app.synth.set_insert(slot, Some(InsertParams { bypass: !p.bypass, ..p }));
            }
        }
    });
    ui.add_space(10.0);

    let Some(p) = app.synth.inserts()[slot] else {
        ui.label(RichText::new("เลือกเอฟเฟกต์จากรายการด้านบน").color(DIM));
        return close;
    };
    let mut next = p;
    egui::Grid::new(("fx-params", slot)).num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
        ui.spacing_mut().slider_width = 260.0;
        for (i, spec) in p.kind.params().iter().enumerate() {
            ui.label(RichText::new(spec.name).color(TEXT));
            let decimals = if spec.max - spec.min <= 20.0 { 1 } else { 0 };
            let log = (spec.unit == "Hz" || spec.unit == "ms") && spec.min > 0.0;
            let slider = egui::Slider::new(&mut next.values[i], spec.min..=spec.max)
                .suffix(format!(" {}", spec.unit))
                .fixed_decimals(decimals)
                .logarithmic(log);
            ui.add(slider).on_hover_text(format!("ค่าเริ่มต้น {} {}", spec.default, spec.unit));
            ui.end_row();
        }
    });
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if ui.button(format!("{}  ค่าเริ่มต้น", icons::UNDO)).clicked() {
            next = InsertParams { bypass: p.bypass, ..InsertParams::new(p.kind) };
        }
        if ui.button(format!("{}  เอาออก", icons::TRASH)).clicked() {
            app.synth.set_insert(slot, None);
        }
    });
    if app.synth.inserts()[slot].is_some() && next != p {
        app.synth.set_insert(slot, Some(next));
    }
    close
}

fn slot_menu(app: &mut KaraokeApp, ui: &mut egui::Ui, slot: usize) {
    use crate::ui::menu::{heading, item, item_if, toggle};
    let params = app.synth.inserts()[slot];
    heading(ui, &format!("ช่อง {}  ·  {}", slot + 1, params.map_or("ว่าง", |p| p.kind.name())));
    if item(ui, icons::EFFECTS, if params.is_some() { "ปรับค่า…" } else { "ใส่เอฟเฟกต์…" }, "") {
        app.effect_editor = Some(slot);
    }
    if let Some(p) = params {
        if toggle(ui, p.bypass, icons::POWER, "ปิดชั่วคราว (bypass)", "") {
            app.synth.set_insert(slot, Some(InsertParams { bypass: !p.bypass, ..p }));
        }
        if item_if(ui, p != InsertParams::new(p.kind), icons::UNDO, "ค่าเริ่มต้น", "") {
            app.synth.set_insert(slot, Some(InsertParams::new(p.kind)));
        }
    }
    if item_if(ui, slot > 0, icons::MOVE_UP, "เลื่อนขึ้น (ทำก่อน)", "") {
        app.synth.swap_inserts(slot, slot - 1);
    }
    if item_if(ui, slot + 1 < INSERT_SLOTS, icons::MOVE_DOWN, "เลื่อนลง (ทำทีหลัง)", "") {
        app.synth.swap_inserts(slot, slot + 1);
    }
    ui.separator();
    if item_if(ui, params.is_some(), icons::REMOVE, "เอาออก", "") {
        app.synth.set_insert(slot, None);
    }
    if item_if(ui, app.synth.inserts().iter().any(Option::is_some), icons::TRASH, "ล้างทุกช่อง", "") {
        for s in 0..INSERT_SLOTS {
            app.synth.set_insert(s, None);
        }
    }
}
