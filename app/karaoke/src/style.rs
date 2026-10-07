//! Fonts and colours. Thai text needs fonts egui does not ship, so the app
//! bundles Noto Sans Thai (UI fallback) and a bold pair for the lyrics.

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke};

/// Font family used for lyrics on the stage.
pub const LYRICS: &str = "lyrics";

pub fn lyrics_family() -> FontFamily {
    FontFamily::Name(LYRICS.into())
}

// A night-blue room lit by a warm "sung" colour and a cool accent.
pub const INK: Color32 = Color32::from_rgb(0x0d, 0x10, 0x1c);
pub const PANEL: Color32 = Color32::from_rgb(0x13, 0x17, 0x27);
pub const RAISED: Color32 = Color32::from_rgb(0x1b, 0x20, 0x35);
pub const LINE: Color32 = Color32::from_rgb(0x2a, 0x30, 0x4a);
pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xea, 0xf2);
pub const DIM: Color32 = Color32::from_rgb(0x8a, 0x90, 0xab);
/// Lyrics already sung.
pub const SUNG: Color32 = Color32::from_rgb(0xff, 0xb0, 0x3b);
pub const SUNG_HOT: Color32 = Color32::from_rgb(0xff, 0x6a, 0x3d);
/// Lyrics still to sing.
pub const UNSUNG: Color32 = Color32::from_rgb(0xf4, 0xf5, 0xfa);
pub const ACCENT: Color32 = Color32::from_rgb(0x4f, 0xd1, 0xc5);
pub const DANGER: Color32 = Color32::from_rgb(0xff, 0x5d, 0x73);

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    };
    add(&mut fonts, "noto-thai", include_bytes!("../assets/fonts/NotoSansThai-Regular.ttf"));
    add(&mut fonts, "noto-thai-bold", include_bytes!("../assets/fonts/NotoSansThai-Bold.ttf"));
    add(&mut fonts, "noto-sans-bold", include_bytes!("../assets/fonts/NotoSans-Bold.ttf"));
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("noto-thai".into());
    }
    // Thai first so a Thai phrase and its spaces shape as one run; Latin
    // letters fall through to Noto Sans.
    let mut lyrics = vec!["noto-thai-bold".to_string(), "noto-sans-bold".to_string()];
    lyrics.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts.families.insert(lyrics_family(), lyrics);
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = PANEL;
        v.window_fill = RAISED;
        v.window_stroke = Stroke::new(1.0, LINE);
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(10);
        v.extreme_bg_color = INK;
        v.faint_bg_color = RAISED;
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = SUNG_HOT.gamma_multiply(0.55);
        v.selection.stroke = Stroke::new(1.0, SUNG);
        v.slider_trailing_fill = true;
        let w = &mut v.widgets;
        for (state, fill) in [
            (&mut w.noninteractive, PANEL),
            (&mut w.inactive, RAISED),
            (&mut w.hovered, Color32::from_rgb(0x26, 0x2d, 0x4a)),
            (&mut w.active, Color32::from_rgb(0x30, 0x38, 0x5c)),
            (&mut w.open, Color32::from_rgb(0x26, 0x2d, 0x4a)),
        ] {
            state.corner_radius = CornerRadius::same(8);
            if fill != PANEL {
                state.bg_fill = fill;
                state.weak_bg_fill = fill;
            }
        }
        w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.6));
    });
}

/// Linear blend of two colours.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}
