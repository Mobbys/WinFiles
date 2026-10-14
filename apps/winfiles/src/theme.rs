//! Colori, font e stile di egui, con un tema scuro e uno chiaro sul modello di VectorCraft.
//! Nessun widget scrive un colore a mano: li prende tutti da [`Tokens::get`]. Un test controlla il
//! contrasto di ogni testo sui suoi sfondi (WCAG), così un testo illeggibile non passa la CI.

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals,
};

/// Tema scelto dall'utente.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    /// Segue il tema di Windows.
    #[default]
    System,
    Dark,
    Light,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light];

    pub const fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "Come Windows",
            ThemeChoice::Dark => "Scuro",
            ThemeChoice::Light => "Chiaro",
        }
    }

    /// Il tema da usare davvero, dato quello del sistema (se noto).
    pub fn resolve(self, system: Option<egui::Theme>) -> Tokens {
        match (self, system) {
            (ThemeChoice::Light, _) | (ThemeChoice::System, Some(egui::Theme::Light)) => LIGHT,
            _ => DARK,
        }
    }
}

/// Tutti i colori dell'interfaccia.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
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

/// Tema scuro (il "Scuro" di VectorCraft, con testi più chiari per il contrasto).
pub const DARK: Tokens = Tokens {
    dark: true,
    app_bar: hex(0x2b2b2b),
    panel: hex(0x323232),
    panel_darker: hex(0x282828),
    tab_strip: hex(0x262626),
    input: hex(0x1f1f1f),
    input_border: hex(0x4a4a4a),
    border: hex(0x1b1b1b),
    divider: hex(0x3f3f3f),
    text: hex(0xe6e6e6),
    text_strong: hex(0xffffff),
    text_dim: hex(0xb4b4b4),
    text_disabled: hex(0x8a8a8a),
    icon: hex(0xd0d0d0),
    hover: hex(0x454545),
    pressed: hex(0x1d1d1d),
    accent: hex(0x378ef0),
    row_selected: hex(0x2d4f73),
    pasteboard: hex(0x3a3a3a),
    button: hex(0x3a3a3a),
    button_border: hex(0x6a6a6a),
    warning: hex(0xffb84d),
    error: hex(0xff6b6b),
    radius: 2,
};

/// Tema chiaro (il "Chiaro" di VectorCraft).
pub const LIGHT: Tokens = Tokens {
    dark: false,
    app_bar: hex(0xe8e8e8),
    panel: hex(0xf3f3f3),
    panel_darker: hex(0xe6e6e6),
    tab_strip: hex(0xdedede),
    input: hex(0xffffff),
    input_border: hex(0xb0b0b0),
    border: hex(0xc8c8c8),
    divider: hex(0xd6d6d6),
    text: hex(0x1f1f1f),
    text_strong: hex(0x000000),
    text_dim: hex(0x555555),
    text_disabled: hex(0x7a7a7a),
    icon: hex(0x333333),
    hover: hex(0xdcdcdc),
    pressed: hex(0xc8c8c8),
    accent: hex(0x1473e6),
    row_selected: hex(0xc8daf7),
    pasteboard: hex(0xc4c4c4),
    button: hex(0xfafafa),
    button_border: hex(0x9a9a9a),
    warning: hex(0x8a4b00),
    error: hex(0xb3261e),
    radius: 2,
};

impl Tokens {
    /// I colori in uso, salvati nel contesto da [`apply`].
    pub fn get(ctx: &egui::Context) -> Tokens {
        ctx.data(|d| d.get_temp::<Tokens>(egui::Id::NULL)).unwrap_or(DARK)
    }
}

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
pub fn apply(ctx: &egui::Context, t: Tokens) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, t));
    let cr = CornerRadius::same(t.radius);
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.panel;
    v.window_fill = t.panel;
    v.extreme_bg_color = t.input;
    v.text_edit_bg_color = Some(t.input);
    v.faint_bg_color = t.panel_darker;
    v.code_bg_color = t.panel_darker;
    v.window_stroke = Stroke::new(1.0, t.border);
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(6);
    v.selection.bg_fill = t.accent.gamma_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0, t.text_strong);
    v.hyperlink_color = t.accent;
    v.override_text_color = Some(t.text);
    v.weak_text_color = Some(t.text_dim);
    v.warn_fg_color = t.warning;
    v.error_fg_color = t.error;
    let alpha = if t.dark { 120 } else { 50 };
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha(alpha),
    };
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(alpha),
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
    // Tutti gli stati dei widget scrivono il testo con colori leggibili sul loro sfondo.
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    w.inactive.fg_stroke = Stroke::new(1.0, t.text);
    w.hovered.fg_stroke = Stroke::new(1.5, t.text_strong);
    w.active.fg_stroke = Stroke::new(1.5, t.text_strong);
    w.open.fg_stroke = Stroke::new(1.0, t.text_strong);
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.divider);
    w.inactive.bg_stroke = Stroke::new(1.0, t.button_border);
    w.inactive.bg_fill = t.button;
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
        s.spacing.slider_rail_height = 4.0;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Rapporto di contrasto WCAG 2 tra due colori.
    fn contrast(a: Color32, b: Color32) -> f64 {
        let lum = |c: Color32| {
            let ch = |v: u8| {
                let v = f64::from(v) / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// Ogni testo deve leggersi su ogni sfondo su cui compare: 4,5:1 per testi e avvisi
    /// (WCAG AA), 3:1 per i comandi disattivati.
    #[test]
    fn every_text_is_readable_on_its_backgrounds() {
        for t in [DARK, LIGHT] {
            let surfaces = [
                t.app_bar,
                t.panel,
                t.panel_darker,
                t.tab_strip,
                t.input,
                t.hover,
                t.row_selected,
                t.button,
            ];
            for bg in surfaces {
                for (name, fg) in [("text", t.text), ("text_strong", t.text_strong)] {
                    assert!(
                        contrast(fg, bg) >= 4.5,
                        "{name} {fg:?} su {bg:?} (scuro: {})",
                        t.dark
                    );
                }
            }
            for bg in [t.app_bar, t.panel, t.panel_darker, t.tab_strip, t.input] {
                for (name, fg, min) in [
                    ("text_dim", t.text_dim, 4.5),
                    ("warning", t.warning, 4.5),
                    ("error", t.error, 4.5),
                    ("text_disabled", t.text_disabled, 3.0),
                    ("icon", t.icon, 4.5),
                ] {
                    assert!(
                        contrast(fg, bg) >= min,
                        "{name} {fg:?} su {bg:?} (scuro: {})",
                        t.dark
                    );
                }
            }
            // La guida del cursore e i bordi dei campi si distinguono dal pannello.
            assert!(
                contrast(t.button_border, t.panel) >= 2.0,
                "button_border (scuro: {})",
                t.dark
            );
        }
    }

    #[test]
    fn system_choice_follows_windows() {
        assert_eq!(ThemeChoice::System.resolve(Some(egui::Theme::Light)), LIGHT);
        assert_eq!(ThemeChoice::System.resolve(Some(egui::Theme::Dark)), DARK);
        assert_eq!(ThemeChoice::System.resolve(None), DARK);
        assert_eq!(ThemeChoice::Light.resolve(Some(egui::Theme::Dark)), LIGHT);
    }
}
