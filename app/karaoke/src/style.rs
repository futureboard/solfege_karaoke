//! Fonts and colours. Thai text needs fonts egui does not ship, so the app
//! bundles Noto Sans Thai (UI fallback) and a bold pair for the lyrics.

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke};

/// Font family used for lyrics on the stage.
pub const LYRICS: &str = "lyrics";

pub fn lyrics_family() -> FontFamily {
    FontFamily::Name(LYRICS.into())
}

// Neutral greys so the lyric colours carry the stage.
/// Stage and bottom bar.
pub const INK: Color32 = Color32::from_rgb(0x15, 0x15, 0x15);
/// Overlay panel.
pub const PANEL: Color32 = Color32::from_rgb(0x1c, 0x1c, 0x1c);
/// Hovered / selected rows.
pub const RAISED: Color32 = Color32::from_rgb(0x27, 0x27, 0x27);
pub const LINE: Color32 = Color32::from_rgb(0x30, 0x30, 0x30);
pub const TEXT: Color32 = Color32::from_rgb(0xec, 0xec, 0xec);
pub const DIM: Color32 = Color32::from_rgb(0x8c, 0x8c, 0x8c);
/// Lyrics already sung.
pub const SUNG: Color32 = Color32::from_rgb(0xff, 0xb0, 0x3b);
pub const SUNG_HOT: Color32 = Color32::from_rgb(0xff, 0x6a, 0x3d);
/// Lyrics still to sing.
pub const UNSUNG: Color32 = Color32::from_rgb(0xf4, 0xf4, 0xf4);
pub const ACCENT: Color32 = Color32::from_rgb(0x4f, 0xd1, 0xc5);
pub const DANGER: Color32 = Color32::from_rgb(0xff, 0x5d, 0x73);

/// A CJK font installed with the system, if there is one (none is bundled:
/// they are tens of megabytes).
fn system_fallback() -> Option<FontData> {
    const CANDIDATES: &[&str] = &[
        // Windows
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\YuGothM.ttc",
        "C:\\Windows\\Fonts\\msgothic.ttc",
        "C:\\Windows\\Fonts\\malgun.ttf",
        // macOS
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        // Linux
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
    ];
    CANDIDATES.iter().find_map(|p| std::fs::read(p).ok()).map(FontData::from_owned)
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    };
    add(&mut fonts, "noto-thai", include_bytes!("../assets/fonts/NotoSansThai-Regular.ttf"));
    add(&mut fonts, "noto-thai-bold", include_bytes!("../assets/fonts/NotoSansThai-Bold.ttf"));
    add(&mut fonts, "noto-sans-bold", include_bytes!("../assets/fonts/NotoSans-Bold.ttf"));
    add(&mut fonts, "lucide", include_bytes!("../assets/fonts/lucide.ttf"));
    // Last resort for names in other scripts (Chinese, Japanese, Korean
    // SoundFont and song files): a font the system already has.
    let fallback = system_fallback().map(|data| {
        fonts.font_data.insert("system-fallback".into(), Arc::new(data));
        "system-fallback".to_string()
    });
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let list = fonts.families.entry(family).or_default();
        // Lucide first: its private-use code points would otherwise hit the
        // icon font egui ships. It has no other glyphs, so text falls through.
        list.insert(0, "lucide".into());
        list.push("noto-thai".into());
        list.extend(fallback.clone());
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
            (&mut w.hovered, Color32::from_rgb(0x2e, 0x2e, 0x2e)),
            (&mut w.active, Color32::from_rgb(0x38, 0x38, 0x38)),
            (&mut w.open, Color32::from_rgb(0x2e, 0x2e, 0x2e)),
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

/// Colour that marks a SoundFont of the rack wherever it appears.
pub fn font_color(index: usize) -> Color32 {
    const COLORS: [Color32; 8] = [
        ACCENT,
        SUNG,
        Color32::from_rgb(0xa7, 0x8b, 0xfa),
        Color32::from_rgb(0xf4, 0x72, 0xb6),
        Color32::from_rgb(0x60, 0xa5, 0xfa),
        Color32::from_rgb(0x4a, 0xde, 0x80),
        Color32::from_rgb(0xfb, 0x92, 0x3c),
        Color32::from_rgb(0xfa, 0xcc, 0x15),
    ];
    COLORS[index % COLORS.len()]
}

/// Linear blend of two colours.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}
