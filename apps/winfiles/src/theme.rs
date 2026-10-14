//! Colori, font e stile di egui. I valori vengono dal tema "Medio scuro" di VectorCraft (a sua
//! volta misurato sulle app Adobe): nessun widget scrive un colore a mano, li prende tutti da qui.

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals,
};

/// Tutti i colori dell'interfaccia.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub app_bar: Color32,
    pub panel: Color32,
    pub panel_darker: Color32,
    pub tab_strip: Color32,
    pub input: Color32,
    pub input_border: Color32,
    pub border: Color32,
    pub divider: Color32,
    pub text: Color32,
    pub text_strong: Color32,
    pub text_dim: Color32,
    pub text_disabled: Color32,
    pub icon: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub accent: Color32,
    pub row_selected: Color32,
    pub pasteboard: Color32,
    pub button: Color32,
    pub button_border: Color32,
    pub warning: Color32,
    pub error: Color32,
    pub radius: u8,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Il tema "Medio scuro".
pub const TOKENS: Tokens = Tokens {
    app_bar: hex(0x535353),
    panel: hex(0x535353),
    panel_darker: hex(0x424242),
    tab_strip: hex(0x424242),
    input: hex(0x454545),
    input_border: hex(0x5f5f5f),
    border: hex(0x383838),
    divider: hex(0x4b4b4b),
    text: hex(0xd1d1d1),
    text_strong: hex(0xffffff),
    text_dim: hex(0xb0b0b0),
    text_disabled: hex(0x7a7a7a),
    icon: hex(0xc2c2c2),
    hover: hex(0x606060),
    pressed: hex(0x303030),
    accent: hex(0x378ef0),
    row_selected: hex(0x52677c),
    pasteboard: hex(0x606060),
    button: hex(0x535353),
    button_border: hex(0x747474),
    warning: hex(0xf0a330),
    error: hex(0xe34850),
    radius: 2,
};

pub const FONT_SEMIBOLD: &str = "ui-semibold";

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(FONT_SEMIBOLD.into()))
}

/// Inter (OFL) per tutta l'interfaccia, con i font di egui come riserva per i simboli mancanti.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "Inter-SemiBold".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-SemiBold.ttf"
        ))),
    );
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut regular = vec!["Inter".to_string()];
    regular.extend(fallback.iter().cloned());
    fonts.families.insert(FontFamily::Proportional, regular);
    let mut semi = vec!["Inter-SemiBold".to_string()];
    semi.extend(fallback);
    fonts
        .families
        .insert(FontFamily::Name(FONT_SEMIBOLD.into()), semi);
    ctx.set_fonts(fonts);
}

/// Applica i colori e le spaziature allo stile globale di egui.
pub fn apply(ctx: &egui::Context) {
    let t = TOKENS;
    let cr = CornerRadius::same(t.radius);
    let mut v = Visuals::dark();
    v.panel_fill = t.panel;
    v.window_fill = t.panel;
    v.extreme_bg_color = t.input;
    v.faint_bg_color = t.panel_darker;
    v.window_stroke = Stroke::new(1.0, t.border);
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(6);
    v.selection.bg_fill = t.accent.gamma_multiply(0.55);
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.hyperlink_color = t.accent;
    v.override_text_color = Some(t.text);
    v.warn_fg_color = t.warning;
    v.error_fg_color = t.error;
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(140),
    };
    let w = &mut v.widgets;
    craft_ui::theme::WidgetPalette {
        noninteractive: t.panel,
        inactive: t.button,
        hovered: t.hover,
        active: t.pressed,
        open: t.hover,
    }
    .apply_to(w, cr, Stroke::new(1.0, t.text));
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.divider);
    w.inactive.bg_stroke = Stroke::new(1.0, t.button_border);
    w.inactive.weak_bg_fill = t.button;
    w.hovered.bg_stroke = Stroke::new(1.0, t.input_border);
    w.active.bg_stroke = Stroke::new(1.0, t.accent);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
        s.spacing.button_padding = egui::vec2(8.0, 2.0);
        s.spacing.interact_size = egui::vec2(24.0, 22.0);
        s.spacing.menu_margin = egui::Margin::same(6);
        s.spacing.slider_width = 110.0;
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(10.5)),
            (TextStyle::Body, FontId::proportional(12.5)),
            (TextStyle::Button, FontId::proportional(12.5)),
            (TextStyle::Heading, semibold(14.0)),
            (TextStyle::Monospace, FontId::monospace(11.5)),
        ]
        .into();
        s.animation_time = 0.08;
    });
}
