//! Il contenuto dei pannelli: elenco dei file, proprietà del file scelto, note, impaginazione.

use std::path::Path;

use egui::{Rect, Sense, Vec2, pos2, vec2};
use winfiles_geom::{Margins, Size, Unit, format_area, format_length, format_number, format_size};
use winfiles_model::{FileEntry, FileKind, Totals};

use crate::app::{WinFilesApp, dim_label};
use crate::icons;
use crate::theme::{TOKENS, semibold};

const ROW: f32 = 22.0;

enum Row<'a> {
    Folder(&'a Path, String, Totals),
    File(usize),
}

fn file_name(f: &FileEntry) -> String {
    f.path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn folder_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// La misura breve di un file per l'elenco: formato finito della prima pagina.
fn short_size(f: &FileEntry, unit: Unit, error: bool) -> String {
    match f.pages.first() {
        Some(p) => format_size(p.finished(), unit),
        None if error => "illeggibile".into(),
        None => "…".into(),
    }
}

fn kind_icon(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Image => "image",
        _ => "layers",
    }
}

/// Testo a sinistra e a destra in una riga, con il sinistro tagliato se non c'è posto.
fn row_text(
    ui: &egui::Ui,
    rect: Rect,
    left: &str,
    left_font: egui::FontId,
    left_color: egui::Color32,
    right: &str,
    right_color: egui::Color32,
) {
    let p = ui.painter();
    let right_galley = p.layout_no_wrap(right.to_string(), egui::FontId::proportional(11.5), right_color);
    let right_w = right_galley.size().x;
    p.galley(
        pos2(
            rect.right() - right_w,
            rect.center().y - right_galley.size().y / 2.0,
        ),
        right_galley,
        right_color,
    );
    let clip = Rect::from_min_max(rect.min, pos2(rect.right() - right_w - 8.0, rect.bottom()));
    let galley = p.layout_no_wrap(left.to_string(), left_font, left_color);
    p.with_clip_rect(clip).galley(
        pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        left_color,
    );
}

/// Una riga etichetta/valore nelle proprietà.
fn prop(ui: &mut egui::Ui, label: &str, value: impl Into<egui::WidgetText>) {
    dim_label(ui, label);
    ui.label(value);
    ui.end_row();
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(title)
            .font(semibold(12.5))
            .color(TOKENS.text_strong),
    );
    ui.add_space(2.0);
}

impl WinFilesApp {
    pub(crate) fn files_panel(&mut self, ui: &mut egui::Ui) {
        let t = TOKENS;
        if self.project.files.is_empty() {
            dim_label(ui, "Nessun file. Trascina file o cartelle nella finestra.");
            return;
        }
        let totals = self.project.folder_totals();
        let labels = self.project.folder_labels();
        let mut rows = Vec::with_capacity(self.project.files.len() + totals.len());
        let mut last: Option<&Path> = None;
        for (i, f) in self.project.files.iter().enumerate() {
            let folder = f.folder();
            if last != Some(folder) {
                let label = labels.get(folder).cloned().unwrap_or_else(|| folder_name(folder));
                rows.push(Row::Folder(
                    folder,
                    label,
                    totals.get(folder).copied().unwrap_or_default(),
                ));
                last = Some(folder);
            }
            rows.push(Row::File(i));
        }
        let unit = self.project.unit;
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show_rows(ui, ROW, rows.len(), |ui, range| {
                for row in rows.get(range).unwrap_or_default() {
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                    match row {
                        Row::Folder(path, label, tot) => {
                            ui.painter().rect_filled(rect, 0.0, t.panel_darker);
                            icons::paint(
                                ui,
                                "folder-open",
                                Rect::from_center_size(
                                    rect.left_center() + vec2(10.0, 0.0),
                                    Vec2::splat(14.0),
                                ),
                                t.icon,
                            );
                            let right = format!("{} file · {}", tot.files, format_area(tot.area_m2));
                            row_text(
                                ui,
                                rect.with_min_x(rect.left() + 22.0).with_max_x(rect.right() - 4.0),
                                label,
                                semibold(12.0),
                                t.text_strong,
                                &right,
                                t.text_dim,
                            );
                            resp.on_hover_text(path.display().to_string());
                        }
                        Row::File(i) => {
                            let Some(f) = self.project.files.get(*i) else {
                                continue;
                            };
                            let selected = self.selected == Some(*i);
                            if selected {
                                ui.painter().rect_filled(rect, 0.0, t.row_selected);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
                            }
                            let error = self.errors.get(&f.path);
                            let tint = if error.is_some() { t.warning } else { t.icon };
                            icons::paint(
                                ui,
                                kind_icon(f.kind),
                                Rect::from_center_size(
                                    rect.left_center() + vec2(24.0, 0.0),
                                    Vec2::splat(13.0),
                                ),
                                tint,
                            );
                            let mut name = file_name(f);
                            if !f.note.is_empty() {
                                name.push_str("  ✎");
                            }
                            let color = if selected { t.text_strong } else { t.text };
                            // Misure calcolate con la risoluzione presunta: in arancione, da verificare.
                            let assumed = f.image.is_some_and(|i| !i.dpi_from_file);
                            let size_color = if assumed || error.is_some() {
                                t.warning
                            } else {
                                t.text_dim
                            };
                            row_text(
                                ui,
                                rect.with_min_x(rect.left() + 36.0).with_max_x(rect.right() - 4.0),
                                &name,
                                egui::FontId::proportional(12.0),
                                color,
                                &short_size(f, unit, error.is_some()),
                                size_color,
                            );
                            let resp = match error {
                                Some(e) => resp.on_hover_text(e.as_str()),
                                None => resp,
                            };
                            if resp.clicked() {
                                clicked = Some(*i);
                            }
                        }
                    }
                }
            });
        if clicked.is_some() {
            self.selected = clicked;
        }
    }

    pub(crate) fn properties_panel(&mut self, ui: &mut egui::Ui) {
        let unit = self.project.unit;
        let Some(f) = self.selected.and_then(|i| self.project.files.get(i)) else {
            self.project_summary(ui);
            return;
        };
        ui.label(
            egui::RichText::new(file_name(f))
                .font(semibold(13.0))
                .color(TOKENS.text_strong),
        );
        ui.label(
            egui::RichText::new(f.folder().display().to_string())
                .size(11.0)
                .color(TOKENS.text_dim),
        );
        ui.add_space(4.0);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            egui::Grid::new("proprieta").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                prop(ui, "Tipo", f.kind.label());
                if let Some(e) = self.errors.get(&f.path) {
                    dim_label(ui, "Misure");
                    ui.colored_label(TOKENS.warning, e.as_str());
                    ui.end_row();
                }
                if f.pages.is_empty() && !self.errors.contains_key(&f.path) {
                    prop(ui, "Misure", "in lettura…");
                }
                if let Some(first) = f.pages.first() {
                    let same = f.pages.iter().all(|p| p == first);
                    if f.pages.len() > 1 {
                        prop(ui, "Pagine", f.pages.len().to_string());
                    }
                    if same {
                        prop(ui, "Formato", format_size(first.size, unit));
                        if let Some(trim) = first.trim {
                            prop(ui, "Formato finito", format_size(trim, unit));
                            let bleed = (first.size.w - trim.w) / 2.0;
                            if bleed > 0.05 {
                                prop(ui, "Abbondanza", format!("{} per lato", format_length(bleed, unit)));
                            }
                        }
                    }
                    prop(ui, "Area", format_area(f.area_m2()));
                }
                if let Some(img) = f.image {
                    prop(ui, "Pixel", format!("{} × {} px", format_number(f64::from(img.width_px), 0, true), format_number(f64::from(img.height_px), 0, true)));
                    let dpi = if (img.dpi_x - img.dpi_y).abs() < 0.5 {
                        format!("{} DPI", format_number(img.dpi_x, 0, true))
                    } else {
                        format!("{} × {} DPI", format_number(img.dpi_x, 0, true), format_number(img.dpi_y, 0, true))
                    };
                    if img.dpi_from_file {
                        prop(ui, "Risoluzione", dpi);
                    } else {
                        dim_label(ui, "Risoluzione");
                        ui.colored_label(TOKENS.warning, format!("{dpi} (non dichiarata)"))
                            .on_hover_text("Il file non dice la sua risoluzione: le misure in mm e m² usano 96 DPI, come la versione 4.9.");
                        ui.end_row();
                    }
                }
            });
            if let Some(first) = f.pages.first()
                && f.pages.iter().any(|p| p != first)
            {
                section(ui, "Pagine");
                egui::Grid::new("pagine").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                    for (n, p) in f.pages.iter().enumerate().take(500) {
                        dim_label(ui, &format!("{}", n + 1));
                        let text = match p.trim {
                            Some(trim) => format!("{} (finito {})", format_size(p.size, unit), format_size(trim, unit)),
                            None => format_size(p.size, unit),
                        };
                        ui.label(text);
                        ui.end_row();
                    }
                });
            }
        });
    }

    /// Senza un file scelto: le somme per cartella e del progetto.
    fn project_summary(&self, ui: &mut egui::Ui) {
        if self.project.files.is_empty() {
            dim_label(ui, "Seleziona un file per vederne misure, pagine e risoluzione.");
            return;
        }
        section(ui, "Riepilogo per cartella");
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            egui::Grid::new("riepilogo")
                .num_columns(3)
                .spacing([10.0, 5.0])
                .striped(true)
                .show(ui, |ui| {
                    let labels = self.project.folder_labels();
                    for (folder, tot) in self.project.folder_totals() {
                        let label = labels.get(folder).cloned().unwrap_or_else(|| folder_name(folder));
                        ui.label(label).on_hover_text(folder.display().to_string());
                        ui.label(format!("{} file", tot.files));
                        ui.label(format_area(tot.area_m2));
                        ui.end_row();
                    }
                    let all = Totals::of(&self.project.files);
                    ui.label(egui::RichText::new("Totale").font(semibold(12.5)));
                    ui.label(egui::RichText::new(format!("{} file", all.files)).font(semibold(12.5)));
                    ui.label(egui::RichText::new(format_area(all.area_m2)).font(semibold(12.5)));
                    ui.end_row();
                });
            ui.add_space(6.0);
            dim_label(
                ui,
                "Le aree usano il formato finito (TrimBox) quando il PDF lo dichiara.",
            );
        });
    }

    pub(crate) fn notes_panel(&mut self, ui: &mut egui::Ui) {
        let Some(file) = self.selected.and_then(|i| self.project.files.get_mut(i)) else {
            dim_label(
                ui,
                "Seleziona un file per scrivere le sue note e quelle della sua cartella.",
            );
            return;
        };
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            section(ui, "Nota del file");
            ui.add(
                egui::TextEdit::multiline(&mut file.note)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text("Per esempio: verificare il colore"),
            );
            let folder = file.folder().to_path_buf();
            section(ui, &format!("Nota della cartella {}", folder_name(&folder)));
            let note = self.project.folder_notes.entry(folder).or_default();
            ui.add(
                egui::TextEdit::multiline(note)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text("Vale per tutti i file della cartella"),
            );
        });
    }

    pub(crate) fn layout_panel(&mut self, ui: &mut egui::Ui) {
        let print = &mut self.project.print;
        let mm = |ui: &mut egui::Ui, value: &mut f64, max: f64| {
            ui.add(
                egui::DragValue::new(value)
                    .range(0.0..=max)
                    .speed(0.5)
                    .suffix(" mm"),
            );
        };
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            egui::Grid::new("impaginazione")
                .num_columns(2)
                .spacing([12.0, 7.0])
                .show(ui, |ui| {
                    dim_label(ui, "Colonne");
                    ui.add(egui::Slider::new(&mut print.columns, 1..=10));
                    ui.end_row();
                    dim_label(ui, "Margini");
                    let mut margin = print.margins.top;
                    if ui
                        .add(
                            egui::DragValue::new(&mut margin)
                                .range(0.0..=50.0)
                                .speed(0.5)
                                .suffix(" mm"),
                        )
                        .changed()
                    {
                        print.margins = Margins::uniform(margin);
                    }
                    ui.end_row();
                    dim_label(ui, "Spazio tra miniature");
                    mm(ui, &mut print.gap, 20.0);
                    ui.end_row();
                    dim_label(ui, "Didascalia");
                    mm(ui, &mut print.caption, 40.0);
                    ui.end_row();
                    dim_label(ui, "Intestazione cartella");
                    mm(ui, &mut print.header, 30.0);
                    ui.end_row();
                });
            ui.add_space(6.0);
            ui.checkbox(
                &mut print.page_break_per_folder,
                "Ogni cartella su un foglio nuovo",
            );
            ui.add_space(8.0);
            let sheet = winfiles_geom::sheet_size(print.paper, print.orientation);
            let area = Size::new(
                sheet.w - print.margins.left - print.margins.right,
                sheet.h - print.margins.top - print.margins.bottom,
            );
            dim_label(
                ui,
                &format!("Area stampabile {}", format_size(area, self.project.unit)),
            );
        });
    }
}
