//! La scala dei file in Miniature: i disegni fatti in piccolo (1:10, 1:20, 1:30…) si segnano
//! con un clic, e le misure diventano quelle vere.
//!
//! La riga "Scala" sotto l'intestazione dell'anteprima ha una pillola per ogni scala: un clic la
//! mette ai file scelti. Con il Timbro acceso ogni clic su una miniatura mette la scala scelta
//! alla sua pagina, finché si preme Esc o lo si spegne.

use egui::{Color32, Rect, Sense, Stroke, vec2};
use winfiles_model::{MAX_SCALE, Project};

use crate::theme::{Tokens, semibold};

/// Le scale delle pillole, le più usate in ditta; le altre con "Altra".
pub const PILLS: [u32; 6] = [1, 10, 20, 30, 50, 100];

/// Lo stato della riga Scala.
pub struct ScaleBar {
    /// La riga è aperta.
    pub on: bool,
    /// Il timbro acceso, con la sua scala: un clic su una miniatura la mette a quella pagina.
    pub stamp: Option<u32>,
    /// La scala scritta nel campo "Altra".
    pub custom: u32,
}

impl Default for ScaleBar {
    fn default() -> Self {
        Self {
            on: false,
            stamp: None,
            custom: 25,
        }
    }
}

/// Cosa chiede la riga Scala in questo fotogramma.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarAction {
    /// Mette la scala a tutte le pagine dei file scelti.
    Apply(u32),
}

impl ScaleBar {
    pub fn close(&mut self) {
        self.on = false;
        self.stamp = None;
    }

    /// La riga: pillole, campo "Altra", timbro e il suggerimento. `current` è la scala comune
    /// dei file scelti (evidenziata), `marked` quanti sono.
    pub fn bar(&mut self, ui: &mut egui::Ui, current: Option<u32>, marked: usize) -> Option<BarAction> {
        let t = Tokens::get(ui.ctx());
        let mut action = None;
        crate::app::dim_label(ui, "Scala");
        let shown = self.stamp.or(current);
        if let Some(n) = pills(ui, shown, self.stamp.is_some()) {
            if self.stamp.is_some() {
                // Con il timbro acceso una pillola cambia la scala del timbro.
                self.stamp = Some(n);
            } else {
                action = Some(BarAction::Apply(n));
            }
        }
        ui.add_space(8.0);
        crate::app::dim_label(ui, "Altra 1:");
        crate::fluent::number(
            ui,
            56.0,
            egui::DragValue::new(&mut self.custom)
                .range(2..=MAX_SCALE)
                .speed(0.2),
        )
        .on_hover_text("Una scala che non è fra le pillole, per esempio 1:25 o 1:200");
        if ui
            .button("Metti")
            .on_hover_text("Mette la scala scritta qui accanto")
            .clicked()
        {
            if self.stamp.is_some() {
                self.stamp = Some(self.custom);
            } else {
                action = Some(BarAction::Apply(self.custom));
            }
        }
        ui.add_space(8.0);
        let mut stamp = self.stamp.is_some();
        if crate::fluent::toggle(ui, stamp, "Timbro")
            .on_hover_text("Acceso, ogni clic su una miniatura le mette la scala scelta. Esc lo spegne.")
            .clicked()
        {
            stamp = !stamp;
            self.stamp = stamp.then(|| current.filter(|n| *n > 1).unwrap_or(20));
        }
        ui.add_space(8.0);
        let hint = match (self.stamp, marked) {
            (Some(n), _) => format!("Timbro 1:{n}: clic sulle miniature · Esc per finire"),
            (None, 0) => "Scegli le miniature e premi una scala, o accendi il Timbro".into(),
            (None, 1) => "Un clic su una scala la mette al file scelto".into(),
            (None, n) => format!("Un clic su una scala la mette ai {n} file scelti"),
        };
        ui.label(egui::RichText::new(hint).size(12.5).color(t.text_dim));
        action
    }
}

/// Le pillole delle scale: restituisce la scala premuta. `on` è quella evidenziata; `stamp` la
/// colora come il timbro acceso. (Niente doppio clic per il timbro: il primo clic metterebbe già
/// la scala ai file scelti.)
fn pills(ui: &mut egui::Ui, on: Option<u32>, stamp: bool) -> Option<u32> {
    let t = Tokens::get(ui.ctx());
    let font = egui::FontId::proportional(13.0);
    let labels: Vec<String> = PILLS.iter().map(|n| format!("1:{n}")).collect();
    let widths: Vec<f32> = labels
        .iter()
        .map(|l| {
            ui.painter()
                .layout_no_wrap(l.clone(), font.clone(), Color32::WHITE)
                .size()
                .x
                + 22.0
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, crate::fluent::HEIGHT), Sense::hover());
    ui.painter().rect(
        rect,
        t.radius + 2,
        t.field,
        Stroke::new(1.0, t.field_border),
        egui::StrokeKind::Inside,
    );
    let mut out = None;
    let mut x = rect.left() + 2.0;
    for (i, (n, (label, w))) in PILLS.iter().zip(labels.iter().zip(&widths)).enumerate() {
        let r = Rect::from_min_size(egui::pos2(x, rect.top() + 2.0), vec2(*w, rect.height() - 4.0));
        let resp = ui.interact(r, ui.id().with(("scala", i)), Sense::click());
        let selected = on == Some(*n);
        if selected && stamp {
            ui.painter().rect_filled(r, t.radius, t.primary);
        } else if selected {
            ui.painter().rect_filled(r, t.radius, t.selection);
            let bar = Rect::from_center_size(
                egui::pos2(r.center().x, r.bottom() - 2.5),
                vec2(16.0_f32.min(r.width() - 8.0), 3.0),
            );
            ui.painter().rect_filled(bar, 1.5, t.accent);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, t.radius, t.field_hover);
        }
        let (color, f) = match (selected, stamp) {
            (true, true) => (t.on_primary, semibold(13.0)),
            (true, false) => (t.accent_text, semibold(13.0)),
            _ => (t.text, font.clone()),
        };
        ui.painter()
            .text(r.center(), egui::Align2::CENTER_CENTER, label, f, color);
        let tip = if *n == 1 {
            "1:1 toglie la scala: misure del file".to_string()
        } else {
            format!("1:{n} ai file scelti (con il Timbro acceso: la scala del timbro)")
        };
        let resp = resp.on_hover_text(tip);
        if resp.clicked() {
            out = Some(*n);
        }
        x += w;
    }
    out
}

/// La scala comune ai file `files` (tutte le pagine uguali), `None` se è diversa o non ce ne sono.
pub fn common(project: &Project, files: impl IntoIterator<Item = usize>) -> Option<u32> {
    let mut out = None;
    for i in files {
        let s = project.files.get(i)?.common_scale()?;
        match out {
            None => out = Some(s),
            Some(o) if o != s => return None,
            Some(_) => {}
        }
    }
    out
}

/// Mette la scala 1:`scale` a tutte le pagine dei file `files`.
pub fn apply(project: &mut Project, files: impl IntoIterator<Item = usize>, scale: u32) {
    for i in files {
        if let Some(f) = project.files.get_mut(i) {
            f.set_all_scales(scale);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use winfiles_geom::Size;
    use winfiles_model::{FileEntry, FileKind, PageInfo};

    use super::*;

    fn pdf(name: &str, pages: usize) -> FileEntry {
        let mut f = FileEntry::new(PathBuf::from(name), FileKind::Pdf);
        f.pages = vec![
            PageInfo {
                size: Size::new(594.0, 420.0),
                trim: None,
            };
            pages
        ];
        f
    }

    #[test]
    fn la_scala_va_a_tutte_le_pagine_dei_file_scelti() {
        let mut p = Project::new("prova");
        p.files = vec![pdf("/a/uno.pdf", 2), pdf("/a/due.pdf", 1), pdf("/a/tre.pdf", 1)];
        assert_eq!(common(&p, [0, 1]), Some(1));
        apply(&mut p, [0, 1], 20);
        assert_eq!(p.files[0].scale(1), 20);
        assert_eq!(common(&p, [0, 1]), Some(20));
        assert_eq!(common(&p, [0, 2]), None);
        assert_eq!(p.files[2].scale(0), 1);
        // Una pagina diversa: la scala comune non c'è più.
        p.files[0].set_scale(1, 50);
        assert_eq!(common(&p, [0]), None);
        apply(&mut p, [0], 1);
        assert!(p.files[0].scales.is_empty());
        assert_eq!(common(&p, std::iter::empty()), None);
    }
}
