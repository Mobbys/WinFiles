//! Le finestre del modulo PDF: margini, nuove misure, aggiungi pagine, estrai e dividi. Ognuna
//! restituisce cosa fare quando si conferma; il modulo poi lo fa sul documento.

use std::path::PathBuf;

use egui::vec2;
use winfiles_geom::{Margins, Size, format_number};

use crate::app::dim_label;
use crate::fluent;
use crate::panels::mm_field;
use crate::theme::{Tokens, semibold};

/// Le pagine su cui agire: quelle scelte nell'elenco o tutte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Chosen,
    All,
}

pub enum Dialog {
    Margins {
        margins: Margins,
        same: bool,
        color: egui::Color32,
        scope: Scope,
    },
    Resize {
        size: Size,
        keep: bool,
        scope: Scope,
    },
    Insert(Box<Insert>),
    Extract {
        one_per_page: bool,
    },
    Split {
        every: bool,
        count: u32,
        before: String,
    },
}

/// Le pagine da aggiungere da un altro PDF.
pub struct Insert {
    pub file: winfiles_pdf_edit::PdfFile,
    pub path: PathBuf,
    pub pages: String,
    pub at: Position,
}

/// Dove vanno le pagine aggiunte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Start,
    AfterCurrent,
    End,
}

/// Cosa fare, confermata una finestra.
pub enum Action {
    Margins(Scope, Margins, [u8; 3]),
    Resize(Scope, Size),
    Insert(winfiles_pdf_edit::PdfFile, Vec<usize>, Position),
    Extract {
        one_per_page: bool,
    },
    /// Dividere prima di queste pagine (da 0).
    Split(Vec<usize>),
}

/// Dati del documento che le finestre mostrano.
pub struct Context {
    pub chosen: usize,
    pub total: usize,
    pub current: usize,
    /// Misura della pagina corrente in mm, per proporre le nuove misure.
    pub current_size: Size,
}

impl Dialog {
    pub fn margins() -> Self {
        Dialog::Margins {
            margins: Margins::uniform(10.0),
            same: true,
            color: egui::Color32::WHITE,
            scope: Scope::Chosen,
        }
    }

    pub fn resize(size: Size) -> Self {
        Dialog::Resize {
            size,
            keep: true,
            scope: Scope::Chosen,
        }
    }

    /// Mostra la finestra. `Some(None)` vuol dire chiusa senza fare nulla.
    pub fn show(&mut self, ctx: &egui::Context, doc: &Context) -> Option<Option<Action>> {
        let t = Tokens::get(ctx);
        let mut out = None;
        let title = match self {
            Dialog::Margins { .. } => "Aggiungi margini",
            Dialog::Resize { .. } => "Ridimensiona pagine",
            Dialog::Insert { .. } => "Aggiungi pagine",
            Dialog::Extract { .. } => "Estrai pagine",
            Dialog::Split { .. } => "Dividi il PDF",
        };
        let modal = egui::Modal::new(egui::Id::new("pdf-finestra")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(
                egui::RichText::new(title)
                    .font(semibold(17.0))
                    .color(t.text_strong),
            );
            ui.add_space(12.0);
            let (ok_label, ready) = match self {
                Dialog::Margins {
                    margins,
                    same,
                    color,
                    scope,
                } => {
                    margins_body(ui, margins, same, color);
                    scope_choice(ui, scope, doc);
                    ("Applica", true)
                }
                Dialog::Resize { size, keep, scope } => {
                    resize_body(ui, size, keep, doc.current_size);
                    scope_choice(ui, scope, doc);
                    ("Applica", true)
                }
                Dialog::Insert(i) => {
                    let ok = insert_body(ui, &i.file, &i.path, &mut i.pages, &mut i.at, doc);
                    ("Aggiungi", ok)
                }
                Dialog::Extract { one_per_page } => {
                    extract_body(ui, one_per_page, doc);
                    ("Estrai…", true)
                }
                Dialog::Split { every, count, before } => {
                    let ok = split_body(ui, every, count, before, doc);
                    ("Dividi…", ok)
                }
            };
            ui.add_space(16.0);
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), 36.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    let ok = egui::Button::new(
                        egui::RichText::new(ok_label)
                            .font(semibold(13.5))
                            .color(t.on_primary),
                    )
                    .fill(t.primary)
                    .min_size(vec2(100.0, 32.0));
                    if ui.add_enabled(ready, ok).clicked() {
                        out = Some(Some(()));
                    }
                    if ui
                        .add(egui::Button::new("Annulla").min_size(vec2(84.0, 32.0)))
                        .clicked()
                    {
                        out = Some(None);
                    }
                },
            );
        });
        if modal.should_close() && out.is_none() {
            out = Some(None);
        }
        match out? {
            None => Some(None),
            Some(()) => Some(self.action(doc)),
        }
    }

    fn action(&mut self, doc: &Context) -> Option<Action> {
        match self {
            Dialog::Margins {
                margins,
                color,
                scope,
                ..
            } => Some(Action::Margins(
                *scope,
                *margins,
                [color.r(), color.g(), color.b()],
            )),
            Dialog::Resize { size, scope, .. } => Some(Action::Resize(*scope, *size)),
            Dialog::Insert(i) => {
                let list = parse_pages(&i.pages, i.file.page_count())?;
                Some(Action::Insert(i.file.clone(), list, i.at))
            }
            Dialog::Extract { one_per_page } => Some(Action::Extract {
                one_per_page: *one_per_page,
            }),
            Dialog::Split { every, count, before } => {
                let cuts = split_cuts(*every, *count, before, doc.total)?;
                Some(Action::Split(cuts))
            }
        }
    }
}

fn scope_choice(ui: &mut egui::Ui, scope: &mut Scope, doc: &Context) {
    ui.add_space(12.0);
    dim_label(ui, "Applica a");
    let chosen = if doc.chosen > 1 {
        format!("Le {} pagine scelte", doc.chosen)
    } else {
        format!("La pagina {}", doc.current + 1)
    };
    if fluent::radio(ui, *scope == Scope::Chosen, chosen).clicked() {
        *scope = Scope::Chosen;
    }
    if fluent::radio(
        ui,
        *scope == Scope::All,
        format!("Tutte le pagine ({})", doc.total),
    )
    .clicked()
    {
        *scope = Scope::All;
    }
}

fn margins_body(ui: &mut egui::Ui, m: &mut Margins, same: &mut bool, color: &mut egui::Color32) {
    egui::Grid::new("pdf-margini")
        .num_columns(4)
        .spacing([12.0, 10.0])
        .show(ui, |ui| {
            if *same {
                dim_label(ui, "Tutti i lati");
                let before = m.top;
                fluent::number(ui, 104.0, mm_field(&mut m.top, 2000.0));
                if m.top != before {
                    *m = Margins::uniform(m.top);
                }
                ui.end_row();
            } else {
                dim_label(ui, "Sopra");
                fluent::number(ui, 104.0, mm_field(&mut m.top, 2000.0));
                dim_label(ui, "Sotto");
                fluent::number(ui, 104.0, mm_field(&mut m.bottom, 2000.0));
                ui.end_row();
                dim_label(ui, "Sinistra");
                fluent::number(ui, 104.0, mm_field(&mut m.left, 2000.0));
                dim_label(ui, "Destra");
                fluent::number(ui, 104.0, mm_field(&mut m.right, 2000.0));
                ui.end_row();
            }
        });
    ui.add_space(6.0);
    if ui.add(fluent::checkbox(same, "Uguali su tutti i lati")).changed() && *same {
        *m = Margins::uniform(m.top);
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        dim_label(ui, "Colore");
        ui.color_edit_button_srgba(color);
        for (c, name) in [(egui::Color32::WHITE, "Bianco"), (egui::Color32::BLACK, "Nero")] {
            if ui.small_button(name).clicked() {
                *color = c;
            }
        }
    });
}

fn resize_body(ui: &mut egui::Ui, size: &mut Size, keep: &mut bool, current: Size) {
    let ratio = current.aspect().unwrap_or(1.0);
    egui::Grid::new("pdf-misura")
        .num_columns(4)
        .spacing([12.0, 10.0])
        .show(ui, |ui| {
            dim_label(ui, "Base");
            let w = fluent::number(ui, 110.0, mm_field(&mut size.w, 381_000.0));
            dim_label(ui, "Altezza");
            let h = fluent::number(ui, 110.0, mm_field(&mut size.h, 381_000.0));
            if *keep && w.changed() {
                size.h = size.w / ratio;
            } else if *keep && h.changed() {
                size.w = size.h * ratio;
            }
            ui.end_row();
        });
    ui.add_space(6.0);
    if ui
        .add(fluent::checkbox(keep, "Mantieni le proporzioni"))
        .changed()
        && *keep
    {
        size.h = size.w / ratio;
    }
    ui.add_space(4.0);
    let t = Tokens::get(ui.ctx());
    ui.label(
        egui::RichText::new(
            "Il contenuto si ingrandisce o rimpicciolisce senza deformarsi e resta al centro.",
        )
        .size(12.0)
        .color(t.text_dim),
    );
}

fn insert_body(
    ui: &mut egui::Ui,
    file: &winfiles_pdf_edit::PdfFile,
    path: &std::path::Path,
    pages: &mut String,
    at: &mut Position,
    doc: &Context,
) -> bool {
    let t = Tokens::get(ui.ctx());
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let count = file.page_count();
    ui.label(egui::RichText::new(format!("{name}: {}", crate::canvas::pages_text(count))).color(t.text));
    ui.add_space(10.0);
    dim_label(
        ui,
        "Pagine da aggiungere (vuoto per tutte, oppure per esempio 1-3, 5)",
    );
    fluent::text_field(
        ui,
        egui::TextEdit::singleline(pages)
            .hint_text("tutte")
            .desired_width(240.0)
            .margin(fluent::MARGIN),
    );
    let parsed = parse_pages(pages, count);
    if parsed.is_none() {
        ui.label(
            egui::RichText::new(format!("Pagine non valide: il file va da 1 a {count}"))
                .size(12.0)
                .color(t.error),
        );
    }
    ui.add_space(10.0);
    dim_label(ui, "Dove");
    for (pos, label) in [
        (
            Position::AfterCurrent,
            format!("Dopo la pagina {}", doc.current + 1),
        ),
        (Position::Start, "All'inizio".to_string()),
        (Position::End, "Alla fine".to_string()),
    ] {
        if fluent::radio(ui, *at == pos, label).clicked() {
            *at = pos;
        }
    }
    parsed.is_some()
}

fn extract_body(ui: &mut egui::Ui, one_per_page: &mut bool, doc: &Context) {
    let t = Tokens::get(ui.ctx());
    let which = if doc.chosen > 1 {
        format!("Le {} pagine scelte", doc.chosen)
    } else {
        format!("La pagina {}", doc.current + 1)
    };
    ui.label(egui::RichText::new(which).color(t.text));
    ui.add_space(8.0);
    if fluent::radio(ui, !*one_per_page, "In un solo PDF").clicked() {
        *one_per_page = false;
    }
    if fluent::radio(ui, *one_per_page, "Un PDF per pagina, in una cartella").clicked() {
        *one_per_page = true;
    }
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("Per scegliere tutte le pagine: Ctrl+A nell'elenco.")
            .size(12.0)
            .color(t.text_dim),
    );
}

fn split_body(
    ui: &mut egui::Ui,
    every: &mut bool,
    count: &mut u32,
    before: &mut String,
    doc: &Context,
) -> bool {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        if fluent::radio(ui, *every, "Ogni").clicked() {
            *every = true;
        }
        let max = u32::try_from(doc.total.max(1)).unwrap_or(u32::MAX);
        if fluent::number(ui, 64.0, egui::DragValue::new(count).range(1..=max)).changed() {
            *every = true;
        }
        ui.label(if *count == 1 { "pagina" } else { "pagine" });
    });
    ui.add_space(6.0);
    if fluent::radio(ui, !*every, "Prima delle pagine (per esempio 4, 9)").clicked() {
        *every = false;
    }
    let resp = fluent::text_field(
        ui,
        egui::TextEdit::singleline(before)
            .hint_text("4, 9")
            .desired_width(240.0)
            .margin(fluent::MARGIN),
    );
    if resp.changed() {
        *every = false;
    }
    ui.add_space(10.0);
    let cuts = split_cuts(*every, *count, before, doc.total);
    match &cuts {
        Some(c) => {
            let ranges = winfiles_pdf_edit::split_ranges(doc.total, c);
            let parts: Vec<String> = ranges
                .iter()
                .take(6)
                .map(|r| {
                    if r.len() == 1 {
                        format!("{}", r.start + 1)
                    } else {
                        format!("{}–{}", r.start + 1, r.end)
                    }
                })
                .collect();
            let more = if ranges.len() > 6 { ", …" } else { "" };
            ui.label(
                egui::RichText::new(format!(
                    "{} file: pagine {}{more}",
                    format_number(ranges.len() as f64, 0, true),
                    parts.join(", ")
                ))
                .color(t.text),
            );
        }
        None => {
            ui.label(
                egui::RichText::new(format!("Pagine non valide: il PDF va da 1 a {}", doc.total))
                    .size(12.0)
                    .color(t.error),
            );
        }
    }
    cuts.is_some_and(|c| winfiles_pdf_edit::split_ranges(doc.total, &c).len() > 1)
}

/// "1-3, 5" in pagine da 0; vuoto o "tutte" vuol dire tutte. `None` se non va.
pub fn parse_pages(text: &str, count: usize) -> Option<Vec<usize>> {
    let text = text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("tutte") {
        return Some((0..count).collect());
    }
    let mut out = Vec::new();
    for part in text.split([',', ';', ' ']).filter(|p| !p.is_empty()) {
        let (a, b) = match part.split_once(['-', '–']) {
            Some((a, b)) => (a.trim().parse::<usize>().ok()?, b.trim().parse::<usize>().ok()?),
            None => {
                let n = part.trim().parse::<usize>().ok()?;
                (n, n)
            }
        };
        if a == 0 || b == 0 || a > count || b > count {
            return None;
        }
        if a <= b {
            out.extend(a - 1..b);
        } else {
            out.extend((b - 1..a).rev());
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Dove dividere: ogni `count` pagine, o prima delle pagine scritte in `before`.
fn split_cuts(every: bool, count: u32, before: &str, total: usize) -> Option<Vec<usize>> {
    if every {
        let n = usize::try_from(count.max(1)).unwrap_or(1);
        return Some((1..).map(|i| i * n).take_while(|c| *c < total).collect());
    }
    let list = parse_pages(before, total)?;
    if before.trim().is_empty() {
        return None;
    }
    Some(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagine_scritte_a_mano() {
        assert_eq!(parse_pages("", 3), Some(vec![0, 1, 2]));
        assert_eq!(parse_pages("tutte", 2), Some(vec![0, 1]));
        assert_eq!(parse_pages("1-3, 5", 5), Some(vec![0, 1, 2, 4]));
        assert_eq!(parse_pages("3–1", 3), Some(vec![2, 1, 0]));
        assert_eq!(parse_pages("0", 3), None);
        assert_eq!(parse_pages("4", 3), None);
        assert_eq!(parse_pages("due", 3), None);
    }

    #[test]
    fn dove_dividere() {
        assert_eq!(split_cuts(true, 2, "", 5), Some(vec![2, 4]));
        assert_eq!(split_cuts(true, 1, "", 3), Some(vec![1, 2]));
        assert_eq!(split_cuts(false, 1, "4, 9", 10), Some(vec![3, 8]));
        assert_eq!(split_cuts(false, 1, "", 10), None);
    }
}
