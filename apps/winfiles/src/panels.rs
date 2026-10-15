//! Il contenuto dei pannelli: elenco dei file, proprietà del file scelto, note, impaginazione.

use std::path::Path;

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, Rect, Sense, Vec2, pos2, vec2};
use winfiles_geom::{Margins, Size, Unit, format_area, format_length, format_number, format_size};
use winfiles_model::{FileEntry, FileKind, Totals, View};

use crate::app::{WinFilesApp, dim_label};
use crate::icons;
use crate::theme::{Tokens, semibold};
use crate::thumbs::ThumbState;

const ROW: f32 = 38.0;

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

fn kind_icon(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Image => "image",
        _ => "layers",
    }
}

/// Due righe: il nome sopra, le misure sotto, tagliati con "…" se non c'è posto.
fn two_lines(
    ui: &egui::Ui,
    rect: Rect,
    top: &str,
    top_font: egui::FontId,
    top_color: Color32,
    bottom: &str,
    bottom_color: Color32,
) {
    let p = ui.painter();
    let width = (rect.width() - 8.0).max(1.0);
    let line = |text: &str, font: egui::FontId, color: Color32| {
        let mut job = LayoutJob::single_section(
            text.to_string(),
            TextFormat {
                font_id: font,
                color,
                ..Default::default()
            },
        );
        job.wrap = TextWrapping {
            max_width: width,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        p.layout_job(job)
    };
    let a = line(top, top_font, top_color);
    let b = line(bottom, egui::FontId::proportional(11.5), bottom_color);
    let total = a.size().y + b.size().y;
    let y = rect.center().y - total / 2.0;
    p.galley(pos2(rect.left(), y), a.clone(), top_color);
    p.galley(pos2(rect.left(), y + a.size().y), b, bottom_color);
}

/// Le misure di un file per l'elenco: formato finito, m² e pagine.
fn file_info(f: &FileEntry, unit: Unit, error: Option<&String>, assumed: bool) -> String {
    let Some(first) = f.pages.first() else {
        return if error.is_some_and(|e| e == crate::document::MISSING) {
            "file non trovato".into()
        } else if error.is_some() {
            "misure non leggibili".into()
        } else {
            "lettura delle misure…".into()
        };
    };
    let mut out = format!(
        "{}  ·  {}",
        format_size(first.size, unit),
        format_area(f.area_full_m2())
    );
    if let Some(trim) = first.trim.filter(|t| *t != first.size) {
        out.push_str(&format!("  ·  al vivo {}", format_size(trim, unit)));
    }
    if f.pages.len() > 1 {
        out.push_str(&format!("  ·  {} pagine", f.pages.len()));
    }
    if assumed {
        out.push_str("  ·  96 DPI presunti");
    }
    out
}

/// L'area di un gruppo di file, con quella al vivo quando è diversa.
pub(crate) fn areas(t: &Totals) -> String {
    crate::captions::area_text(t.area_full_m2, t.area_m2, true, true).unwrap_or_default()
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
            .color(Tokens::get(ui.ctx()).text_strong),
    );
    ui.add_space(2.0);
}

/// Cosa fare dei file dal menu del tasto destro.
enum FileAction {
    /// Toglie dall'elenco i file scelti.
    RemoveMarked,
    /// Toglie dall'elenco tutti i file di una cartella.
    RemoveFolder(std::path::PathBuf),
    Open(std::path::PathBuf),
    Reveal(std::path::PathBuf),
}

/// Cosa fare dell'intestazione di una cartella nell'elenco.
enum Rename {
    Start(std::path::PathBuf, String),
    Commit,
    Cancel,
    Reset(std::path::PathBuf),
}

impl WinFilesApp {
    pub(crate) fn files_panel(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
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
                let label = self
                    .project
                    .folder_titles
                    .get(folder)
                    .map(|t| t.trim())
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .or_else(|| labels.get(folder).cloned())
                    .unwrap_or_else(|| folder_name(folder));
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
        let mut menu_on = None;
        let mut action = None;
        let marked = self.marked.len();
        let mods = ui.input(|i| i.modifiers);
        let mut rename = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show_rows(ui, ROW, rows.len(), |ui, range| {
                for row in rows.get(range).unwrap_or_default() {
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                    match row {
                        Row::Folder(path, label, tot) => {
                            ui.painter()
                                .rect_filled(rect.shrink2(vec2(0.0, 1.0)), t.radius, t.panel_darker);
                            let icon = Rect::from_center_size(
                                rect.left_center() + vec2(12.0, 0.0),
                                Vec2::splat(16.0),
                            );
                            icons::paint(ui, "folder-open", icon, t.icon);
                            let mut info = format!("{} file · {}", tot.files, areas(tot));
                            if tot.pages > tot.files {
                                info = format!("{} file · {} pagine · {}", tot.files, tot.pages, areas(tot));
                            }
                            let editing = matches!(&self.renaming, Some((p, _)) if p.as_path() == *path);
                            if editing {
                                // Rinomina sul posto: Invio o clic fuori conferma, Esc annulla.
                                let field = Rect::from_min_size(
                                    rect.min + vec2(24.0, 1.0),
                                    vec2(rect.width() - 28.0, 20.0),
                                );
                                if let Some((_, text)) = &mut self.renaming {
                                    // In una Ui figlia, così il campo non sposta le righe sotto.
                                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field));
                                    let edit = child.add_sized(
                                        field.size(),
                                        egui::TextEdit::singleline(text)
                                            .font(semibold(12.5))
                                            .margin(egui::Margin::symmetric(6, 1))
                                            .hint_text(labels.get(*path).map_or("", String::as_str)),
                                    );
                                    if !edit.has_focus() && !edit.lost_focus() {
                                        edit.request_focus();
                                    }
                                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                        rename = Some(Rename::Cancel);
                                    } else if edit.lost_focus() {
                                        rename = Some(Rename::Commit);
                                    }
                                }
                                ui.painter().text(
                                    rect.left_bottom() + vec2(30.0, -2.0),
                                    egui::Align2::LEFT_BOTTOM,
                                    &info,
                                    egui::FontId::proportional(11.5),
                                    t.text_dim,
                                );
                            } else {
                                two_lines(
                                    ui,
                                    rect.with_min_x(rect.left() + 26.0)
                                        .with_max_x(rect.right() - 26.0),
                                    label,
                                    semibold(12.5),
                                    t.text_strong,
                                    &info,
                                    t.text_dim,
                                );
                                if resp.hovered() {
                                    let r = Rect::from_center_size(
                                        rect.right_center() - vec2(14.0, 0.0),
                                        Vec2::splat(14.0),
                                    );
                                    icons::paint(ui, "pencil", r, t.text_dim);
                                }
                                if resp.double_clicked() {
                                    rename = Some(Rename::Start(path.to_path_buf(), label.clone()));
                                }
                                let resp = resp.on_hover_text(format!(
                                    "{}\nDoppio clic per rinominare l'intestazione",
                                    path.display()
                                ));
                                resp.context_menu(|ui| {
                                    if ui.button("Rinomina l'intestazione").clicked() {
                                        rename = Some(Rename::Start(path.to_path_buf(), label.clone()));
                                        ui.close();
                                    }
                                    if ui.button("Torna al nome della cartella").clicked() {
                                        rename = Some(Rename::Reset(path.to_path_buf()));
                                        ui.close();
                                    }
                                    ui.separator();
                                    if ui.button("Mostra la cartella").clicked() {
                                        action = Some(FileAction::Open(path.to_path_buf()));
                                        ui.close();
                                    }
                                    if ui
                                        .button(format!(
                                            "Togli dall'elenco i {} file della cartella",
                                            tot.files
                                        ))
                                        .clicked()
                                    {
                                        action = Some(FileAction::RemoveFolder(path.to_path_buf()));
                                        ui.close();
                                    }
                                });
                            }
                        }
                        Row::File(i) => {
                            let Some(f) = self.project.files.get(*i) else {
                                continue;
                            };
                            let selected = self.marked.contains(i);
                            if selected {
                                ui.painter().rect_filled(rect, t.radius, t.selection);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, t.radius, t.hover);
                            }
                            let error = self.errors.get(&f.path);
                            let tint = if error.is_some() { t.warning } else { t.icon };
                            let icon = Rect::from_center_size(
                                rect.left_center() + vec2(26.0, 0.0),
                                Vec2::splat(15.0),
                            );
                            icons::paint(ui, kind_icon(f.kind), icon, tint);
                            let name = file_name(f);
                            // Una matita a destra per i file con una nota (Inter non ha il simbolo ✎).
                            let note_icon = f.has_notes();
                            if note_icon {
                                let r = Rect::from_center_size(
                                    rect.right_center() - vec2(16.0, 0.0),
                                    Vec2::splat(14.0),
                                );
                                icons::paint(
                                    ui,
                                    "pencil",
                                    r,
                                    if selected { t.text_strong } else { t.accent_text },
                                );
                            }
                            // Misure calcolate con la risoluzione presunta: in arancione, da verificare.
                            let assumed = f.image.is_some_and(|i| !i.dpi_from_file);
                            let info_color = if (assumed || error.is_some()) && !selected {
                                t.warning
                            } else if selected {
                                t.text_strong
                            } else {
                                t.text_dim
                            };
                            let name_color = if selected { t.text_strong } else { t.text };
                            let info = file_info(f, unit, error, assumed);
                            two_lines(
                                ui,
                                rect.with_min_x(rect.left() + 42.0)
                                    .with_max_x(rect.right() - if note_icon { 30.0 } else { 0.0 }),
                                &name,
                                egui::FontId::proportional(12.5),
                                name_color,
                                &info,
                                info_color,
                            );
                            let resp = match error {
                                Some(e) => resp.on_hover_text(e.as_str()),
                                None => resp.on_hover_text(f.path.display().to_string()),
                            };
                            if resp.clicked() {
                                clicked = Some(*i);
                            }
                            if resp.secondary_clicked() {
                                menu_on = Some(*i);
                            }
                            let path = &f.path;
                            resp.context_menu(|ui| {
                                // Il clic destro su un file non scelto lo sceglie da solo.
                                let count = if selected { marked.max(1) } else { 1 };
                                if ui.button("Apri con il programma predefinito").clicked() {
                                    action = Some(FileAction::Open(path.clone()));
                                    ui.close();
                                }
                                if ui.button("Mostra nella cartella").clicked() {
                                    action = Some(FileAction::Reveal(path.clone()));
                                    ui.close();
                                }
                                ui.separator();
                                let label = match count {
                                    1 => "Togli dall'elenco".to_string(),
                                    n => format!("Togli dall'elenco i {n} file scelti"),
                                };
                                if ui
                                    .button(label)
                                    .on_hover_text("Il file resta sul disco. Tasto Canc.")
                                    .clicked()
                                {
                                    action = Some(FileAction::RemoveMarked);
                                    ui.close();
                                }
                            });
                        }
                    }
                }
            });
        if let Some(i) = clicked {
            self.pick(i, mods);
        }
        if let Some(i) = menu_on {
            self.pick_for_menu(i);
        }
        match action {
            Some(FileAction::RemoveMarked) => {
                let marked = std::mem::take(&mut self.marked);
                self.remove_files(&marked);
            }
            Some(FileAction::RemoveFolder(folder)) => {
                let set = self
                    .project
                    .files
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| f.folder() == folder)
                    .map(|(i, _)| i)
                    .collect();
                self.remove_files(&set);
            }
            Some(FileAction::Open(path)) => {
                if let Err(e) = crate::shell::open(&path) {
                    self.status = format!("Non riesco ad aprire {}: {e}", path.display());
                }
            }
            Some(FileAction::Reveal(path)) => {
                if let Err(e) = crate::shell::reveal(&path) {
                    self.status = format!("Non riesco a mostrare {}: {e}", path.display());
                }
            }
            None => {}
        }
        match rename {
            Some(Rename::Start(path, label)) => self.renaming = Some((path, label)),
            Some(Rename::Cancel) => self.renaming = None,
            Some(Rename::Reset(path)) => {
                self.project.folder_titles.remove(&path);
            }
            Some(Rename::Commit) => {
                if let Some((path, text)) = self.renaming.take() {
                    let text = text.trim();
                    // Vuoto o uguale al percorso: si torna al nome della cartella.
                    let default = self.project.folder_labels().get(path.as_path()).cloned();
                    if text.is_empty() || Some(text) == default.as_deref() {
                        self.project.folder_titles.remove(&path);
                    } else {
                        self.project.folder_titles.insert(path, text.to_string());
                    }
                }
            }
            None => {}
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
                .color(Tokens::get(ui.ctx()).text_strong),
        );
        ui.label(
            egui::RichText::new(f.folder().display().to_string())
                .size(11.0)
                .color(Tokens::get(ui.ctx()).text_dim),
        );
        ui.add_space(4.0);
        let thumbs = &mut self.thumbs;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            // L'anteprima grande del file scelto.
            let t = Tokens::get(ui.ctx());
            let box_size = vec2(ui.available_width(), 170.0);
            let (area, _) = ui.allocate_exact_size(box_size, Sense::hover());
            ui.painter().rect_filled(area, t.radius, t.pasteboard);
            match thumbs.get(&f.path, f.kind, 0) {
                ThumbState::Ready(tex) => {
                    let size = tex.size_vec2();
                    let k = ((area.width() - 16.0) / size.x).min((area.height() - 16.0) / size.y).min(1.0);
                    let r = Rect::from_center_size(area.center(), size * k);
                    ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                ThumbState::Loading => {
                    ui.painter().text(area.center(), egui::Align2::CENTER_CENTER, "Preparo l'anteprima…", egui::FontId::proportional(12.0), t.text);
                }
                ThumbState::Failed(e) => {
                    let mut job = LayoutJob::single_section(format!("Anteprima non disponibile: {e}"), TextFormat { font_id: egui::FontId::proportional(12.0), color: t.text, ..Default::default() });
                    job.wrap.max_width = area.width() - 24.0;
                    job.halign = egui::Align::Center;
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(area.center().x, area.center().y - g.size().y / 2.0), g, t.text);
                }
            }
            ui.add_space(6.0);
            egui::Grid::new("proprieta").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                prop(ui, "Tipo", f.kind.label());
                if let Some(e) = self.errors.get(&f.path) {
                    dim_label(ui, "Misure");
                    ui.colored_label(Tokens::get(ui.ctx()).warning, e.as_str());
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
                        prop(ui, "Formato pagina", format_size(first.size, unit));
                        if let Some(trim) = first.trim.filter(|t| *t != first.size) {
                            prop(ui, "Al vivo", format_size(trim, unit));
                            let bleed = (first.size.w - trim.w) / 2.0;
                            if bleed > 0.05 {
                                prop(ui, "Abbondanza", format!("{} per lato", format_length(bleed, unit)));
                            }
                        }
                    }
                    prop(ui, "Area", format_area(f.area_full_m2()));
                    if (f.area_full_m2() - f.area_m2()).abs() > 1e-6 {
                        prop(ui, "Area al vivo", format_area(f.area_m2()));
                    }
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
                        ui.colored_label(Tokens::get(ui.ctx()).warning, format!("{dpi} (non dichiarata)"))
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
                            Some(trim) => format!("{} (al vivo {})", format_size(p.size, unit), format_size(trim, unit)),
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
        let t = Tokens::get(ui.ctx());
        let labels = self.project.folder_labels();
        let unit_info = |tot: &Totals| {
            let pages = if tot.pages > tot.files {
                format!(" · {} pagine", tot.pages)
            } else {
                String::new()
            };
            format!("{} file{pages} · {}", tot.files, areas(tot))
        };
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for (folder, tot) in self.project.folder_totals() {
                let label = self
                    .project
                    .folder_titles
                    .get(folder)
                    .map(|t| t.trim())
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .or_else(|| labels.get(folder).cloned())
                    .unwrap_or_else(|| folder_name(folder));
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                two_lines(
                    ui,
                    rect,
                    &label,
                    egui::FontId::proportional(12.5),
                    t.text,
                    &unit_info(&tot),
                    t.text_dim,
                );
                resp.on_hover_text(folder.display().to_string());
            }
            let all = Totals::of(&self.project.files);
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
            ui.painter().line_segment(
                [rect.left_top(), rect.right_top()],
                egui::Stroke::new(1.0, t.divider),
            );
            two_lines(
                ui,
                rect,
                "Totale",
                semibold(12.5),
                t.text_strong,
                &unit_info(&all),
                t.text,
            );
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
                "Seleziona un file o una sua pagina nell'anteprima per scrivere le note.",
            );
            return;
        };
        let folder = file.folder().to_path_buf();
        let pages = file.pages.len().max(1);
        let mut page = self.selected_page.min(pages - 1);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if pages > 1 {
                // Ogni pagina ha la sua nota: si sceglie qui o cliccando la miniatura.
                ui.horizontal(|ui| {
                    section(ui, "Nota della pagina");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Da destra a sinistra: si legge "‹ 2 › di 4".
                        dim_label(ui, &format!("di {pages}"));
                        let mut n = (page + 1) as u32;
                        crate::app::stepper(ui, &mut n, 1, pages as u32);
                        page = (n as usize).clamp(1, pages) - 1;
                    });
                });
            } else {
                section(ui, "Nota del file");
            }
            let mut text = file.note(page).to_string();
            let edit = ui.add(
                egui::TextEdit::multiline(&mut text)
                    .id_salt(("nota-pagina", page))
                    .desired_rows(3)
                    .desired_width(ui.available_width() - 8.0)
                    .margin(egui::Margin::symmetric(10, 8))
                    .hint_text("Per esempio: verificare il colore"),
            );
            if edit.changed() {
                file.set_note(page, text);
            }
            if pages > 1 {
                let others: Vec<String> = file
                    .notes
                    .iter()
                    .filter(|(p, n)| **p != page && **p < pages && !n.trim().is_empty())
                    .map(|(p, _)| (p + 1).to_string())
                    .collect();
                if !others.is_empty() {
                    dim_label(ui, &format!("Note anche a pagina {}", others.join(", ")));
                }
            }
            section(ui, &format!("Nota della cartella {}", folder_name(&folder)));
            // Si scrive su una copia: aprire il pannello non deve creare note vuote nel lavoro.
            let mut note = self
                .project
                .folder_notes
                .get(&folder)
                .cloned()
                .unwrap_or_default();
            let edit = ui.add(
                egui::TextEdit::multiline(&mut note)
                    .id_salt("nota-cartella")
                    .desired_rows(3)
                    .desired_width(ui.available_width() - 8.0)
                    .margin(egui::Margin::symmetric(10, 8))
                    .hint_text("Vale per tutti i file della cartella"),
            );
            if edit.changed() {
                if note.trim().is_empty() {
                    self.project.folder_notes.remove(&folder);
                } else {
                    self.project.folder_notes.insert(folder, note);
                }
            }
        });
        self.selected_page = page;
    }

    pub(crate) fn layout_panel(&mut self, ui: &mut egui::Ui) {
        let shown_scale = self.nesting_scale;
        let print = &mut self.project.print;
        let mm = |ui: &mut egui::Ui, value: &mut f64, max: f64| ui.add(mm_field(value, max));
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            dim_label(ui, "Titolo in cima ai fogli");
            ui.add(
                egui::TextEdit::singleline(&mut print.title)
                    .desired_width(ui.available_width() - 8.0)
                    .margin(egui::Margin::symmetric(10, 7))
                    .hint_text("Per esempio: Cliente Rossi, ordine 124"),
            );
            ui.add_space(8.0);
            dim_label(ui, "Testo del titolo");
            ui.add(text_slider(&mut print.title_text, MIN_TITLE_PT..=MAX_TITLE_PT));
            ui.add_space(8.0);
            // Il cursore occupa la larghezza del pannello, lasciando posto al numero.
            ui.spacing_mut().slider_width = (ui.available_width() - 64.0).max(60.0);
            match print.view {
                View::Grid => {
                    dim_label(ui, "Colonne");
                    ui.add(egui::Slider::new(&mut print.columns, 1..=10));
                }
                View::List => {
                    dim_label(ui, "Larghezza delle miniature");
                    ui.spacing_mut().slider_width = (ui.available_width() - 84.0).max(60.0);
                    ui.add(
                        egui::Slider::new(&mut print.list_thumb, winfiles_layout::LIST_THUMB_MM)
                            .step_by(1.0)
                            .fixed_decimals(0)
                            .suffix(" mm"),
                    );
                }
                View::Nesting => {
                    dim_label(ui, "Scala");
                    ui.horizontal(|ui| {
                        crate::app::scale_stepper(ui, &mut print.nesting_scale, shown_scale);
                    });
                }
            }
            ui.add_space(8.0);
            dim_label(ui, "Testo di nome e misure");
            ui.add(text_slider(
                &mut print.caption_text,
                MIN_CAPTION_PT..=MAX_CAPTION_PT,
            ));
            ui.add_space(8.0);
            dim_label(ui, "Testo delle intestazioni");
            ui.add(text_slider(&mut print.header_text, MIN_HEADER_PT..=MAX_HEADER_PT));
            ui.add_space(8.0);
            dim_label(ui, "Testo delle note");
            ui.add(text_slider(&mut print.note_text, MIN_NOTE_PT..=MAX_NOTE_PT));
            ui.add_space(10.0);
            egui::Grid::new("impaginazione")
                .num_columns(2)
                .spacing([12.0, 10.0])
                .show(ui, |ui| {
                    dim_label(ui, "Margini");
                    let mut margin = print.margins.top;
                    if mm(ui, &mut margin, 50.0).changed() {
                        print.margins = Margins::uniform(margin);
                    }
                    ui.end_row();
                    dim_label(ui, "Spazio tra le colonne");
                    mm(ui, &mut print.gap, 20.0);
                    ui.end_row();
                    dim_label(ui, "Spazio tra le righe");
                    mm(ui, &mut print.row_gap, 20.0);

                    ui.end_row();
                });
            ui.add_space(6.0);
            ui.checkbox(&mut print.show_sizes, "Misure con abbondanza e crocini")
                .on_hover_text("Il formato pagina del file e la sua area, sotto ogni miniatura");
            ui.checkbox(&mut print.show_trim, "Misure al vivo")
                .on_hover_text("In rosso, il formato finito (TrimBox) quando è diverso dalla pagina");
            ui.checkbox(&mut print.show_summary, "Foglio di riepilogo in cima")
                .on_hover_text("Un foglio con l'elenco di cartelle e file, pagine, misure, m² e note");
            ui.checkbox(
                &mut print.page_break_per_folder,
                "Ogni cartella su un foglio nuovo",
            );
            ui.checkbox(&mut print.show_notes, "Stampa le note")
                .on_hover_text("Spento: le note restano nel lavoro ma non vanno sul foglio");
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

/// Un campo numerico in millimetri con la virgola decimale, come si scrive in italiano.
fn mm_field(value: &mut f64, max: f64) -> egui::DragValue<'_> {
    egui::DragValue::new(value)
        .range(0.0..=max)
        .speed(0.5)
        .custom_formatter(|v, _| format!("{} mm", format_number(v, 1, false)))
        .custom_parser(|text| {
            let clean = text.trim().trim_end_matches("mm").trim().replace(',', ".");
            clean.parse::<f64>().ok()
        })
}

/// Corpo minimo e massimo del testo delle didascalie, in punti.
pub(crate) const MIN_CAPTION_PT: f64 = 6.0;
pub(crate) const MAX_CAPTION_PT: f64 = 24.0;
/// Corpo minimo e massimo del testo delle note, in punti (come gli 8-40 px della 4.9).
pub(crate) const MIN_NOTE_PT: f64 = 6.0;
/// Corpo minimo e massimo delle intestazioni di cartella e del titolo, in punti.
pub(crate) const MIN_HEADER_PT: f64 = 6.0;
pub(crate) const MAX_HEADER_PT: f64 = 24.0;
pub(crate) const MIN_TITLE_PT: f64 = 8.0;
pub(crate) const MAX_TITLE_PT: f64 = 40.0;
pub(crate) const MAX_NOTE_PT: f64 = 30.0;

/// Un cursore per il corpo di un testo, in punti tipografici con la virgola decimale.
pub(crate) fn text_slider(value: &mut f64, range: std::ops::RangeInclusive<f64>) -> egui::Slider<'_> {
    egui::Slider::new(value, range)
        .step_by(0.5)
        .custom_formatter(|v, _| format!("{} pt", format_number(v, 1, true)))
        .custom_parser(|text| {
            let clean = text.trim().trim_end_matches("pt").trim().replace(',', ".");
            clean.parse::<f64>().ok()
        })
}
