//! Icone Lucide (ISC), disegnate da SVG e colorate con i colori del tema.

use std::sync::OnceLock;

use craft_ui::icons::{SvgIconSet, SvgStyle};
use egui::{Color32, Rect, Response, Sense, Ui, Vec2};

use crate::theme::Tokens;

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../assets/icons/", $name, ".svg")))),*]
    };
}

static ICONS: &[(&str, &[u8])] = icons![
    "chevron-down",
    "chevron-left",
    "chevron-right",
    "chevrons-left",
    "chevrons-right",
    "file-plus",
    "folder-open",
    "image",
    "info",
    "layers",
    "layout-grid",
    "maximize-2",
    "minus",
    "pencil",
    "plus",
    "printer",
    "ruler",
    "square-dashed",
    "trash-2",
    "x",
    "zoom-in",
];

fn table() -> &'static SvgIconSet {
    static T: OnceLock<SvgIconSet> = OnceLock::new();
    T.get_or_init(|| {
        SvgIconSet::new(
            ICONS,
            SvgStyle {
                current_color: "white",
                stroke_width: Some("1.6"),
            },
            "bytes://icon/",
            "square-dashed",
        )
    })
}

/// Disegna l'icona `name` dentro `rect`.
pub fn paint(ui: &Ui, name: &str, rect: Rect, tint: Color32) {
    egui::Image::new(table().source(name))
        .tint(tint)
        .fit_to_exact_size(rect.size())
        .paint_at(ui, rect);
}

/// Pulsante quadrato con icona, come nelle barre di VectorCraft.
pub fn button(ui: &mut Ui, name: &str, tip: &str, selected: bool, enabled: bool, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::splat(size),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    let t = Tokens::get(ui.ctx());
    if selected {
        ui.painter().rect_filled(rect, t.radius, t.pressed);
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, t.radius, t.hover);
    }
    let tint = if !enabled {
        t.text_disabled
    } else if resp.hovered() || selected {
        t.text_strong
    } else {
        t.icon
    };
    let icon = (size * 0.6).round();
    paint(
        ui,
        name,
        Rect::from_center_size(rect.center(), Vec2::splat(icon)),
        tint,
    );
    if tip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tip)
    }
}
