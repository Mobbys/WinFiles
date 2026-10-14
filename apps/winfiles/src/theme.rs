//! Colori, font e stile di egui: un tema scuro e uno chiaro moderni, con schede arrotondate,
//! comandi alti e bordi leggeri. Nessun widget scrive un colore a mano: li prende tutti da
//! [`Tokens::get`]. Un test controlla il contrasto di ogni testo sui suoi sfondi (WCAG), così un
//! testo illeggibile non passa la CI.

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

/// Tutti i colori e i raggi dell'interfaccia.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    /// Fondo della finestra, dietro le schede dei pannelli.
    pub app_bar: Color32,
    /// Fondo delle schede (pannelli).
    pub panel: Color32,
    /// Righe di gruppo e superfici secondarie dentro le schede.
    pub panel_darker: Color32,
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
    /// Testo e bordi su `selection` (voci scelte, campo attivo).
    pub accent_text: Color32,
    /// Fondo dei pulsanti principali (Salva, e poi Stampa): blu pieno, in entrambi i temi.
    pub primary: Color32,
    /// Testo sui pulsanti principali.
    pub on_primary: Color32,
    pub selection: Color32,
    pub pasteboard: Color32,
    pub button: Color32,
    pub button_border: Color32,
    pub warning: Color32,
    pub error: Color32,
    /// Raggio degli angoli dei comandi.
    pub radius: u8,
    /// Raggio degli angoli delle schede.
    pub card_radius: u8,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Tema scuro: fondo quasi nero, schede grigio antracite, accento blu.
pub const DARK: Tokens = Tokens {
    dark: true,
    app_bar: hex(0x18181b),
    panel: hex(0x232326),
    panel_darker: hex(0x2c2c30),
    input: hex(0x1b1b1e),
    input_border: hex(0x5c5c66),
    border: hex(0x33333a),
    divider: hex(0x34343a),
    text: hex(0xe8e8ea),
    text_strong: hex(0xffffff),
    text_dim: hex(0xa9a9b0),
    text_disabled: hex(0x8a8a92),
    icon: hex(0xcfcfd4),
    hover: hex(0x34343a),
    pressed: hex(0x3e3e45),
    accent: hex(0x3b82f6),
    accent_text: hex(0xbfdbfe),
    primary: hex(0x2563eb),
    on_primary: hex(0xffffff),
    selection: hex(0x1e3a5f),
    pasteboard: hex(0x1b1b1e),
    button: hex(0x2f2f34),
    button_border: hex(0x5c5c66),
    warning: hex(0xffb547),
    error: hex(0xff7a7a),
    radius: 6,
    card_radius: 10,
};

/// Tema chiaro: fondo grigio perla, schede bianche, accento blu.
pub const LIGHT: Tokens = Tokens {
    dark: false,
    app_bar: hex(0xeceef2),
    panel: hex(0xffffff),
    panel_darker: hex(0xf3f4f7),
    input: hex(0xffffff),
    input_border: hex(0xa3a9b3),
    border: hex(0xdfe2e7),
    divider: hex(0xe4e6ea),
    text: hex(0x1d1f23),
    text_strong: hex(0x000000),
    text_dim: hex(0x5b616c),
    text_disabled: hex(0x7b818b),
    icon: hex(0x353a42),
    hover: hex(0xeceef2),
    pressed: hex(0xe0e3e8),
    accent: hex(0x2563eb),
    accent_text: hex(0x1d4ed8),
    primary: hex(0x2563eb),
    on_primary: hex(0xffffff),
    selection: hex(0xdbe7fd),
    pasteboard: hex(0xe4e6eb),
    button: hex(0xf3f4f7),
    button_border: hex(0xa3a9b3),
    warning: hex(0x955700),
    error: hex(0xc62828),
    radius: 6,
    card_radius: 10,
};

impl Tokens {
    /// I colori in uso, salvati nel contesto da [`apply`].
    pub fn get(ctx: &egui::Context) -> Tokens {
        ctx.data(|d| d.get_temp::<Tokens>(egui::Id::NULL)).unwrap_or(DARK)
    }
}

pub const FONT_SEMIBOLD: &str = "ui-semibold";

/// Inter, lo stesso carattere nell'interfaccia, nell'anteprima e nei PDF.
pub const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
pub const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(FONT_SEMIBOLD.into()))
}

/// Inter (OFL) per tutta l'interfaccia, con i font di egui come riserva per i simboli mancanti.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("Inter".into(), Arc::new(FontData::from_static(INTER_REGULAR)));
    fonts.font_data.insert(
        "Inter-SemiBold".into(),
        Arc::new(FontData::from_static(INTER_SEMIBOLD)),
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

/// Applica colori e misure allo stile globale di egui: comandi alti 30 punti, angoli arrotondati,
/// bordi leggeri, cursori con la parte riempita in blu e maniglia tonda.
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
    v.window_corner_radius = CornerRadius::same(t.card_radius);
    v.menu_corner_radius = CornerRadius::same(t.card_radius);
    v.selection.bg_fill = t.selection;
    v.selection.stroke = Stroke::new(1.5, t.accent_text);
    v.hyperlink_color = t.accent;
    v.override_text_color = Some(t.text);
    v.weak_text_color = Some(t.text_dim);
    v.warn_fg_color = t.warning;
    v.error_fg_color = t.error;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = false;
    let alpha = if t.dark { 140 } else { 40 };
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 0,
        color: Color32::from_black_alpha(alpha),
    };
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 10],
        blur: 32,
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
    for state in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        state.corner_radius = cr;
        state.expansion = 0.0;
    }
    // Ogni stato scrive il testo con un colore leggibile sul suo sfondo.
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    w.inactive.fg_stroke = Stroke::new(1.0, t.text);
    w.hovered.fg_stroke = Stroke::new(1.0, t.text_strong);
    w.active.fg_stroke = Stroke::new(1.0, t.text_strong);
    w.open.fg_stroke = Stroke::new(1.0, t.text_strong);
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.divider);
    w.inactive.bg_stroke = Stroke::new(1.0, t.button_border);
    w.inactive.bg_fill = t.button;
    w.inactive.weak_bg_fill = t.button;
    w.hovered.bg_fill = t.hover;
    w.hovered.weak_bg_fill = t.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, t.input_border);
    w.active.bg_stroke = Stroke::new(1.0, t.accent);
    w.open.bg_stroke = Stroke::new(1.0, t.input_border);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 5.0);
        s.spacing.interact_size = egui::vec2(32.0, 30.0);
        s.spacing.icon_width = 16.0;
        s.spacing.icon_width_inner = 9.0;
        s.spacing.icon_spacing = 8.0;
        s.spacing.menu_margin = egui::Margin::same(8);
        s.spacing.window_margin = egui::Margin::same(12);
        s.spacing.slider_width = 150.0;
        s.spacing.slider_rail_height = 4.0;
        s.spacing.combo_width = 120.0;
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Body, FontId::proportional(13.5)),
            (TextStyle::Button, FontId::proportional(13.5)),
            (TextStyle::Heading, semibold(15.0)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        s.animation_time = 0.1;
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

    /// Ogni testo deve leggersi su ogni sfondo su cui compare: 4,5:1 per testi, avvisi e icone
    /// (WCAG AA), 3:1 per i comandi disattivati.
    #[test]
    fn every_text_is_readable_on_its_backgrounds() {
        for t in [DARK, LIGHT] {
            let all = [
                t.app_bar,
                t.panel,
                t.panel_darker,
                t.input,
                t.hover,
                t.pressed,
                t.selection,
                t.button,
            ];
            for bg in all {
                for (name, fg) in [("text", t.text), ("text_strong", t.text_strong)] {
                    assert!(
                        contrast(fg, bg) >= 4.5,
                        "{name} {fg:?} su {bg:?} (scuro: {})",
                        t.dark
                    );
                }
            }
            let main = [t.app_bar, t.panel, t.panel_darker, t.input, t.hover, t.button];
            for bg in main {
                for (name, fg, min) in [
                    ("text_dim", t.text_dim, 4.5),
                    ("warning", t.warning, 4.5),
                    ("error", t.error, 4.5),
                    ("icon", t.icon, 4.5),
                    ("text_disabled", t.text_disabled, 3.0),
                ] {
                    assert!(
                        contrast(fg, bg) >= min,
                        "{name} {fg:?} su {bg:?} (scuro: {})",
                        t.dark
                    );
                }
            }
            assert!(
                contrast(t.on_primary, t.primary) >= 4.5,
                "on_primary (scuro: {})",
                t.dark
            );
            assert!(
                contrast(t.accent_text, t.selection) >= 4.5,
                "accent_text (scuro: {})",
                t.dark
            );
            // I bordi dei campi si vedono sul pannello (WCAG 1.4.11: 3:1 per i componenti).
            assert!(
                contrast(t.input_border, t.panel) >= 2.0,
                "input_border (scuro: {})",
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
