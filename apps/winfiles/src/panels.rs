//! Il contenuto dei pannelli: elenco dei file, proprietà del file scelto, note, impaginazione.

use std::path::Path;

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, Rect, Sense, Vec2, pos2, vec2};
use winfiles_geom::{Margins, Size, Unit, format_area, format_length, format_number, format_size};
use winfiles_model::{FileEntry, FileKind, PrintState, Totals, View};

use crate::app::{WinFilesApp, dim_label};
use crate::filter::Found;
use crate::fluent;
use crate::icons;
use crate::reorder;
use crate::theme::{Tokens, semibold};
use crate::thumbs::ThumbState;

const ROW: f32 = 38.0;

enum Row<'a> {
    /// Una cartella con i totali, lo stato di stampa e i file che l'elenco mostra.
    Folder(&'a Path, String, Totals, PrintState, Vec<usize>),
    /// Un file, e se ha più pagine se sono aperte sotto di lui.
    File(usize, Option<bool>),
    /// Una pagina di un file con più pagine.
    Page(usize, usize),
}

/// La casella "in stampa" a sinistra di file e cartelle: piena con la spunta, piena con un
/// trattino se va in stampa solo una parte, vuota se resta fuori.
fn paint_print_check(ui: &egui::Ui, rect: Rect, state: PrintState, hovered: bool) {
    let t = Tokens::get(ui.ctx());
    let painter = ui.painter();
    let fill = if hovered { t.accent } else { t.primary };
    let stroke = egui::Stroke::new(1.8, t.on_primary);
    let (l, tp, s) = (rect.left(), rect.top(), rect.width() / 16.0);
    match state {
        PrintState::All => {
            painter.rect_filled(rect, 3.0, fill);
            painter.line(
                vec![
                    pos2(l + 4.0 * s, tp + 8.5 * s),
                    pos2(l + 6.8 * s, tp + 11.3 * s),
                    pos2(l + 12.0 * s, tp + 5.0 * s),
                ],
                stroke,
            );
        }
        PrintState::Some => {
            painter.rect_filled(rect, 3.0, fill);
            painter.line_segment(
                [
                    pos2(l + 4.0 * s, rect.center().y),
                    pos2(l + 12.0 * s, rect.center().y),
                ],
                stroke,
            );
        }
        PrintState::None => {
            let fill = if hovered { t.hover } else { t.input };
            painter.rect(
                rect,
                3.0,
                fill,
                egui::Stroke::new(1.0, t.input_border),
                egui::StrokeKind::Inside,
            );
        }
    }
}

/// Lo stato di un gruppo di file: tutti in stampa, nessuno, o una parte.
fn group_state(states: impl IntoIterator<Item = PrintState>) -> PrintState {
    let (mut all, mut none) = (true, true);
    for s in states {
        all &= s == PrintState::All;
        none &= s == PrintState::None;
    }
    match (all, none) {
        (true, _) => PrintState::All,
        (_, true) => PrintState::None,
        _ => PrintState::Some,
    }
}

/// Il riquadro della casella "in stampa" in una riga dell'elenco.
fn check_rect(row: Rect) -> Rect {
    Rect::from_center_size(row.left_center() + vec2(11.0, 0.0), Vec2::splat(14.0))
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

/// Quali pagine del file vanno in stampa: una casella per un file di una pagina, un numero da
/// accendere o spegnere per ogni pagina degli altri. Restituisce la pagina toccata (None = tutte).
fn print_section(ui: &mut egui::Ui, f: &FileEntry) -> Option<(Option<usize>, bool)> {
    let mut out = None;
    let pages = f.sheet_pages();
    if pages == 1 {
        let mut on = f.printed(0);
        if ui
            .add(fluent::checkbox(&mut on, "In stampa"))
            .on_hover_text("Tolto dalla stampa, il file resta nell'elenco con le sue note. Tasto Spazio.")
            .changed()
        {
            out = Some((None, on));
        }
        return out;
    }
    let printed = f.printed_pages().count();
    ui.horizontal(|ui| {
        section(ui, "Pagine in stampa");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(printed > 0, egui::Button::new("Nessuna").small())
                .clicked()
            {
                out = Some((None, false));
            }
            if ui
                .add_enabled(printed < pages, egui::Button::new("Tutte").small())
                .clicked()
            {
                out = Some((None, true));
            }
        });
    });
    dim_label(
        ui,
        &format!("{printed} di {pages}: clic su un numero per togliere o rimettere la pagina"),
    );
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for p in 0..pages.min(500) {
            let on = f.printed(p);
            if fluent::toggle(ui, on, format!("{}", p + 1)).clicked() {
                out = Some((Some(p), !on));
            }
        }
    });
    out
}

/// Mentre si trascina: la linea di arrivo, il cartellino sotto il puntatore e lo scorrimento
/// quando il puntatore arriva al bordo dell'elenco.
fn paint_drag(
    ui: &egui::Ui,
    drawn: &[(usize, Rect)],
    drop: Option<reorder::Drop>,
    pointer: Option<egui::Pos2>,
    text: &str,
) {
    let t = Tokens::get(ui.ctx());
    if let Some(d) = drop
        && let Some((_, rect)) = drawn.iter().find(|(r, _)| *r == d.row)
    {
        // Un pixel dentro la riga, così la linea non sparisce sotto il bordo dell'elenco.
        let y = if d.top {
            rect.top() + 1.0
        } else {
            rect.bottom() - 1.0
        };
        let stroke = egui::Stroke::new(2.0, t.primary);
        ui.painter()
            .hline(rect.left() + 8.0..=rect.right() - 4.0, y, stroke);
        ui.painter()
            .circle_stroke(pos2(rect.left() + 5.0, y), 3.0, stroke);
    }
    ui.ctx().set_cursor_icon(if drop.is_some() {
        egui::CursorIcon::Grabbing
    } else {
        egui::CursorIcon::NoDrop
    });
    let Some(p) = pointer else {
        return;
    };
    let clip = ui.clip_rect();
    if p.y < clip.top() + 24.0 {
        ui.scroll_with_delta(vec2(0.0, 8.0));
    } else if p.y > clip.bottom() - 24.0 {
        ui.scroll_with_delta(vec2(0.0, -8.0));
    }
    ui.ctx().request_repaint();
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("trascina-file"),
    ));
    let galley = painter.layout_no_wrap(text.to_string(), egui::FontId::proportional(12.0), t.on_primary);
    let r = Rect::from_min_size(p + vec2(14.0, 10.0), galley.size() + vec2(16.0, 8.0));
    painter.rect_filled(r, t.radius, t.primary);
    painter.galley(r.min + vec2(8.0, 4.0), galley, t.on_primary);
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
    /// Toglie dall'elenco i file di una cartella che l'elenco mostra.
    RemoveSet(std::collections::BTreeSet<usize>),
    Open(std::path::PathBuf),
    Reveal(std::path::PathBuf),
    /// Mette in stampa (o ne toglie) i file scelti.
    PrintMarked(bool),
    /// Mette in stampa (o ne toglie) un file solo, dalla sua casella.
    PrintFile(usize, bool),
    /// Mette in stampa (o ne toglie) i file di una cartella che l'elenco mostra.
    PrintSet(std::collections::BTreeSet<usize>, bool),
    /// Mette in stampa (o ne toglie) una pagina.
    PrintPage(usize, usize, bool),
    /// Apre o chiude le pagine di un file nell'elenco.
    Expand(std::path::PathBuf),
    /// Copia negli appunti la tabella dei file scelti.
    CopyMarked,
}

/// Cosa fare dell'intestazione di una cartella nell'elenco.
enum Rename {
    Start(std::path::PathBuf, String),
    Commit,
    Cancel,
    Reset(std::path::PathBuf),
}

impl WinFilesApp {
    /// In cima all'elenco: il campo di ricerca (Ctrl+F, Esc lo svuota) e i tipi da mostrare.
    fn search_bar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let edit = egui::TextEdit::singleline(&mut self.filter.text)
            .hint_text("Cerca per nome o cartella")
            .margin(egui::Margin {
                left: 28,
                right: 26,
                top: 7,
                bottom: 7,
            })
            .desired_width(f32::INFINITY);
        let resp = fluent::text_field(ui, edit).on_hover_text("Ctrl+F. Esc svuota la ricerca.");
        if std::mem::take(&mut self.focus_search) {
            resp.request_focus();
        }
        let icon = Rect::from_center_size(resp.rect.left_center() + vec2(15.0, 0.0), Vec2::splat(14.0));
        icons::paint(ui, "search", icon, t.text_dim);
        if !self.filter.text.is_empty() {
            let x = Rect::from_center_size(resp.rect.right_center() - vec2(14.0, 0.0), Vec2::splat(18.0));
            let clear = ui
                .interact(x, ui.id().with("svuota-ricerca"), Sense::click())
                .on_hover_text("Svuota la ricerca");
            if clear.hovered() {
                ui.painter().rect_filled(x, 3.0, t.hover);
            }
            icons::paint(ui, "x", x.shrink(3.0), t.text_dim);
            // Esc toglie il fuoco al campo prima che lo si veda qui: conta anche il fuoco appena perso.
            let esc =
                (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Escape));
            if clear.clicked() || esc {
                self.filter.text.clear();
            }
        }
        // I tipi, solo se il lavoro ne ha più di uno.
        let present: Vec<_> = crate::filter::KINDS
            .iter()
            .filter(|(k, _)| self.project.files.iter().any(|f| f.kind == *k))
            .collect();
        if present.len() > 1 {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                if fluent::toggle(ui, self.filter.kinds.is_empty(), "Tutti").clicked() {
                    self.filter.kinds.clear();
                }
                for (kind, label) in present {
                    let on = self.filter.kinds.contains(kind);
                    if fluent::toggle(ui, on, *label)
                        .on_hover_text("Clic per mostrare anche questo tipo; \"Tutti\" toglie il filtro")
                        .clicked()
                    {
                        self.filter.toggle(*kind);
                    }
                }
            });
        }
        ui.add_space(4.0);
    }

    pub(crate) fn files_panel(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        if self.project.files.is_empty() {
            dim_label(ui, "Nessun file. Trascina file o cartelle nella finestra.");
            return;
        }
        self.search_bar(ui);
        let filter = &self.filter;
        let files = &self.project.files;
        let found: Vec<(usize, Found)> = files
            .iter()
            .enumerate()
            .map(|(i, f)| (i, filter.find(f)))
            .filter(|(_, m)| *m != Found::Nothing)
            .collect();
        let visible: Vec<usize> = found.iter().map(|(i, _)| *i).collect();
        if filter.is_active() {
            // Canc e Spazio non devono toccare file che l'elenco non mostra.
            self.marked.retain(|i| visible.binary_search(i).is_ok());
            ui.horizontal(|ui| {
                dim_label(ui, &format!("{} di {} file", visible.len(), files.len()));
                if ui.small_button("Mostra tutti").clicked() {
                    self.filter = Default::default();
                }
            });
            if visible.is_empty() {
                dim_label(ui, "Nessun file corrisponde alla ricerca.");
                return;
            }
        }
        let files = &self.project.files;
        // I file visibili raggruppati per cartella, nell'ordine del lavoro, ognuno con le pagine
        // da mostrare sotto: tutte se è aperto, quelle trovate se la ricerca ha trovato pagine.
        let mut groups: Vec<(&Path, Vec<usize>)> = Vec::new();
        let mut shown_pages: Vec<Vec<Vec<usize>>> = Vec::new();
        for (i, m) in found {
            let Some(f) = files.get(i) else {
                continue;
            };
            let pages = match m {
                Found::Pages(pages) => pages,
                _ if f.sheet_pages() > 1 && self.expanded.contains(&f.path) => (0..f.sheet_pages()).collect(),
                _ => Vec::new(),
            };
            match (groups.last_mut(), shown_pages.last_mut()) {
                (Some((last, list)), Some(p)) if *last == f.folder() => {
                    list.push(i);
                    p.push(pages);
                }
                _ => {
                    groups.push((f.folder(), vec![i]));
                    shown_pages.push(vec![pages]);
                }
            }
        }
        let labels = self.project.folder_labels();
        let group_files: Vec<Vec<usize>> = groups.iter().map(|(_, l)| l.clone()).collect();
        let page_rows: Vec<Vec<usize>> = shown_pages
            .iter()
            .map(|g| g.iter().map(Vec::len).collect())
            .collect();
        let layout = reorder::Rows {
            groups: &group_files,
            pages: &page_rows,
        };
        // Con un filtro attivo l'elenco ha dei buchi: l'ordine si cambia solo vedendo tutto.
        let can_drag = !self.filter.is_active() && self.renaming.is_none();
        if !can_drag {
            self.drag = None;
        }
        let pointer = ui.ctx().pointer_latest_pos();
        let released = ui.input(|i| !i.pointer.any_down());
        let mut drop = None;
        let mut start_drag = None;
        let mut mode = Sense::click();
        if can_drag {
            mode = Sense::click_and_drag();
        }
        // Il cartellino sotto il puntatore mentre si trascina.
        let drag_label = self.drag.as_ref().map(|d| match d {
            reorder::Drag::Files { files, .. } if files.len() == 1 => files
                .first()
                .and_then(|i| self.project.files.get(*i))
                .map(file_name)
                .unwrap_or_default(),
            reorder::Drag::Files { files, .. } => format!("{} file", files.len()),
            reorder::Drag::Group(_) => "Cartella intera".to_string(),
        });
        let mut rows = Vec::with_capacity(visible.len() + groups.len());
        for ((folder, list), pages) in groups.into_iter().zip(shown_pages) {
            let label = self
                .project
                .folder_titles
                .get(folder)
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .or_else(|| labels.get(folder).cloned())
                .unwrap_or_else(|| folder_name(folder));
            let members = || list.iter().filter_map(|i| files.get(*i));
            let totals = Totals::of(members());
            let state = group_state(members().map(FileEntry::print_state));
            rows.push(Row::Folder(folder, label, totals, state, list.clone()));
            for (i, pages) in list.iter().zip(pages) {
                let multi = files.get(*i).is_some_and(|f| f.sheet_pages() > 1);
                rows.push(Row::File(*i, multi.then_some(!pages.is_empty())));
                rows.extend(pages.into_iter().map(|p| Row::Page(*i, p)));
            }
        }
        let unit = self.project.unit;
        let mut clicked = None;
        let mut menu_on = None;
        let mut action = None;
        let marked = self.marked.len();
        let mods = ui.input(|i| i.modifiers);
        let mut rename = None;
        let mut page_click = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show_rows(ui, ROW, rows.len(), |ui, range| {
                let mut drawn = Vec::new();
                for (r, row) in rows.iter().enumerate().skip(range.start).take(range.len()) {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), mode);
                    drawn.push((r, rect));
                    if let (Some(drag), Some(p)) = (&self.drag, pointer)
                        && (rect.top()..rect.bottom()).contains(&p.y)
                    {
                        drop = layout.drop_at(drag, r, p.y < rect.center().y);
                    }
                    let spot = layout.spot(r);
                    if resp.drag_started()
                        && let Some(spot) = spot.filter(|s| !s.page)
                    {
                        let file = spot
                            .file
                            .and_then(|k| group_files.get(spot.group)?.get(k).copied());
                        start_drag = Some((spot.group, file));
                    }
                    let dragged = match (&self.drag, row) {
                        (Some(reorder::Drag::Group(g)), _) => spot.is_some_and(|s| s.group == *g),
                        (Some(reorder::Drag::Files { files, .. }), Row::File(i, _) | Row::Page(i, _)) => {
                            files.contains(i)
                        }
                        _ => false,
                    };
                    match row {
                        Row::Folder(path, label, tot, state, members) => {
                            ui.painter()
                                .rect_filled(rect.shrink2(vec2(0.0, 1.0)), t.radius, t.panel_darker);
                            let check = check_rect(rect);
                            let check_resp = ui
                                .interact(
                                    check.expand(4.0),
                                    ui.id().with(("stampa-cartella", path)),
                                    Sense::click(),
                                )
                                .on_hover_text("In stampa: togli o rimetti tutta la cartella");
                            paint_print_check(ui, check, *state, check_resp.hovered());
                            if check_resp.clicked() {
                                action = Some(FileAction::PrintSet(
                                    members.iter().copied().collect(),
                                    *state != PrintState::All,
                                ));
                            }
                            let icon = Rect::from_center_size(
                                rect.left_center() + vec2(32.0, 0.0),
                                Vec2::splat(16.0),
                            );
                            icons::paint(ui, "folder-open", icon, t.icon);
                            let mut info = if tot.files == 0 {
                                "fuori stampa".to_string()
                            } else if tot.pages > tot.files {
                                format!("{} file · {} pagine · {}", tot.files, tot.pages, areas(tot))
                            } else {
                                format!("{} file · {}", tot.files, areas(tot))
                            };
                            if tot.files > 0 && tot.excluded > 0 {
                                info.push_str(&format!(" · {} fuori stampa", tot.excluded));
                            }
                            let editing = matches!(&self.renaming, Some((p, _)) if p.as_path() == *path);
                            if editing {
                                // Rinomina sul posto: Invio o clic fuori conferma, Esc annulla.
                                let field = Rect::from_min_size(
                                    rect.min + vec2(44.0, 1.0),
                                    vec2(rect.width() - 48.0, 20.0),
                                );
                                if let Some((_, text)) = &mut self.renaming {
                                    // In una Ui figlia, così il campo non sposta le righe sotto.
                                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field));
                                    let edit = fluent::field_scope(&mut child, |ui| {
                                        ui.add_sized(
                                            field.size(),
                                            egui::TextEdit::singleline(text)
                                                .font(semibold(12.5))
                                                .margin(egui::Margin::symmetric(6, 1))
                                                .hint_text(labels.get(*path).map_or("", String::as_str)),
                                        )
                                    });
                                    fluent::field_edge(&child, edit.rect, edit.has_focus());
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
                                    rect.left_bottom() + vec2(50.0, -2.0),
                                    egui::Align2::LEFT_BOTTOM,
                                    &info,
                                    egui::FontId::proportional(11.5),
                                    t.text_dim,
                                );
                            } else {
                                two_lines(
                                    ui,
                                    rect.with_min_x(rect.left() + 46.0)
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
                                    let on = *state != PrintState::All;
                                    let text = if on {
                                        "Metti in stampa tutta la cartella"
                                    } else {
                                        "Togli dalla stampa tutta la cartella"
                                    };
                                    if ui.button(text).clicked() {
                                        action =
                                            Some(FileAction::PrintSet(members.iter().copied().collect(), on));
                                        ui.close();
                                    }
                                    if ui.button("Mostra la cartella").clicked() {
                                        action = Some(FileAction::Open(path.to_path_buf()));
                                        ui.close();
                                    }
                                    if ui
                                        .button(format!(
                                            "Togli dall'elenco i {} file della cartella",
                                            members.len()
                                        ))
                                        .clicked()
                                    {
                                        action =
                                            Some(FileAction::RemoveSet(members.iter().copied().collect()));
                                        ui.close();
                                    }
                                });
                            }
                        }
                        Row::File(i, open) => {
                            let Some(f) = self.project.files.get(*i) else {
                                continue;
                            };
                            let selected = self.marked.contains(i);
                            if selected {
                                ui.painter().rect_filled(rect, t.radius, t.selection);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, t.radius, t.hover);
                            }
                            // La freccia che apre le pagine dei file con più pagine, sopra il fondo della riga.
                            if let Some(open) = open {
                                let r = Rect::from_center_size(
                                    rect.left_center() + vec2(11.0, 0.0),
                                    Vec2::splat(18.0),
                                );
                                let arrow = ui
                                    .interact(r, ui.id().with(("pagine", *i)), Sense::click())
                                    .on_hover_text(if *open {
                                        "Chiudi le pagine"
                                    } else {
                                        "Mostra le pagine una per una"
                                    });
                                if arrow.hovered() {
                                    ui.painter().rect_filled(r, 3.0, t.pressed);
                                }
                                let icon = if *open { "chevron-down" } else { "chevron-right" };
                                icons::paint(
                                    ui,
                                    icon,
                                    r.shrink(2.0),
                                    if selected { t.text_strong } else { t.text_dim },
                                );
                                if arrow.clicked() {
                                    action = Some(FileAction::Expand(f.path.clone()));
                                }
                            }
                            let error = self.errors.get(&f.path);
                            let state = f.print_state();
                            let out = state == PrintState::None;
                            let check = check_rect(rect).translate(vec2(20.0, 0.0));
                            let check_resp = ui
                                .interact(
                                    check.expand(4.0),
                                    ui.id().with(("stampa-file", *i)),
                                    Sense::click(),
                                )
                                .on_hover_text(match state {
                                    PrintState::All => "In stampa. Clic per lasciarlo fuori (Spazio)",
                                    PrintState::Some => {
                                        "In stampa solo alcune pagine: scegli quali in Proprietà"
                                    }
                                    PrintState::None => "Fuori stampa. Clic per rimetterlo (Spazio)",
                                });
                            paint_print_check(ui, check, state, check_resp.hovered());
                            if check_resp.clicked() {
                                action = Some(FileAction::PrintFile(*i, state != PrintState::All));
                            }
                            let tint = if error.is_some() {
                                t.warning
                            } else if out {
                                t.text_disabled
                            } else {
                                t.icon
                            };
                            let icon = Rect::from_center_size(
                                rect.left_center() + vec2(50.0, 0.0),
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
                            } else if out && !selected {
                                t.text_disabled
                            } else if selected {
                                t.text_strong
                            } else {
                                t.text_dim
                            };
                            let name_color = if selected {
                                t.text_strong
                            } else if out {
                                t.text_dim
                            } else {
                                t.text
                            };
                            let mut info = file_info(f, unit, error, assumed);
                            match state {
                                PrintState::All => {}
                                PrintState::Some => {
                                    info = format!(
                                        "{} di {} pagine in stampa  ·  {info}",
                                        f.printed_pages().count(),
                                        f.sheet_pages()
                                    )
                                }
                                PrintState::None => info = format!("fuori stampa  ·  {info}"),
                            }
                            two_lines(
                                ui,
                                rect.with_min_x(rect.left() + 66.0)
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
                                let label = match count {
                                    1 => "Copia negli appunti".to_string(),
                                    n => format!("Copia negli appunti i {n} file scelti"),
                                };
                                if ui
                                    .button(label)
                                    .on_hover_text("Nome, pagine, misure, m² e note, da incollare in Excel o in una mail")
                                    .clicked()
                                {
                                    action = Some(FileAction::CopyMarked);
                                    ui.close();
                                }
                                ui.separator();
                                let on = state != PrintState::All;
                                let label = match (count, on) {
                                    (1, true) => "Metti in stampa".to_string(),
                                    (1, false) => "Togli dalla stampa".to_string(),
                                    (n, true) => format!("Metti in stampa i {n} file scelti"),
                                    (n, false) => format!("Togli dalla stampa i {n} file scelti"),
                                };
                                if ui
                                    .button(label)
                                    .on_hover_text("Il file resta nell'elenco con le sue note. Tasto Spazio.")
                                    .clicked()
                                {
                                    action = Some(FileAction::PrintMarked(on));
                                    ui.close();
                                }
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
                        Row::Page(i, p) => {
                            let Some(f) = self.project.files.get(*i) else {
                                continue;
                            };
                            let selected = self.selected == Some(*i) && self.selected_page == *p;
                            if selected {
                                ui.painter().rect_filled(rect, t.radius, t.selection);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, t.radius, t.hover);
                            }
                            let on = f.printed(*p);
                            let check = check_rect(rect).translate(vec2(40.0, 0.0));
                            let check_resp = ui
                                .interact(
                                    check.expand(4.0),
                                    ui.id().with(("stampa-pagina", *i, *p)),
                                    Sense::click(),
                                )
                                .on_hover_text(if on {
                                    "Pagina in stampa. Clic per lasciarla fuori"
                                } else {
                                    "Pagina fuori stampa. Clic per rimetterla"
                                });
                            paint_print_check(
                                ui,
                                check,
                                if on { PrintState::All } else { PrintState::None },
                                check_resp.hovered(),
                            );
                            if check_resp.clicked() {
                                action = Some(FileAction::PrintPage(*i, *p, !on));
                            }
                            let note = f.note(*p).trim();
                            if !note.is_empty() {
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
                            let mut info = match f.pages.get(*p) {
                                Some(page) => format_size(page.size, unit),
                                None => "lettura delle misure…".to_string(),
                            };
                            if let Some(trim) =
                                f.pages.get(*p).and_then(|pg| pg.trim.filter(|t| *t != pg.size))
                            {
                                info.push_str(&format!("  ·  al vivo {}", format_size(trim, unit)));
                            }
                            if !on {
                                info = format!("fuori stampa  ·  {info}");
                            }
                            let first_line = note.lines().next().unwrap_or("");
                            let name = if first_line.is_empty() {
                                format!("Pagina {} di {}", p + 1, f.sheet_pages())
                            } else {
                                format!("Pagina {}  ·  {first_line}", p + 1)
                            };
                            two_lines(
                                ui,
                                rect.with_min_x(rect.left() + 64.0)
                                    .with_max_x(rect.right() - if note.is_empty() { 0.0 } else { 30.0 }),
                                &name,
                                egui::FontId::proportional(12.0),
                                if selected {
                                    t.text_strong
                                } else if on {
                                    t.text
                                } else {
                                    t.text_dim
                                },
                                &info,
                                if selected {
                                    t.text_strong
                                } else if on {
                                    t.text_dim
                                } else {
                                    t.text_disabled
                                },
                            );
                            let resp = resp.on_hover_text(if note.is_empty() {
                                "Clic per sceglierla: la vedi nell'anteprima e ne scrivi la nota".to_string()
                            } else {
                                note.to_string()
                            });
                            if resp.clicked() || resp.secondary_clicked() {
                                page_click = Some((*i, *p));
                            }
                        }
                    }
                    if dragged {
                        // Le righe che si stanno spostando restano al loro posto, velate.
                        ui.painter()
                            .rect_filled(rect, t.radius, t.panel.gamma_multiply(0.6));
                    }
                }
                if let Some(text) = &drag_label {
                    paint_drag(ui, &drawn, drop, pointer, text);
                }
            });
        if let Some((g, file)) = start_drag {
            self.drag = match file {
                Some(i) => {
                    // Si trascina la scelta se il file ne fa parte e sta tutta nella sua cartella.
                    let same = group_files.get(g).is_some_and(|list| {
                        self.marked.contains(&i) && self.marked.iter().all(|m| list.contains(m))
                    });
                    if !same {
                        self.pick(i, egui::Modifiers::NONE);
                    }
                    Some(reorder::Drag::Files {
                        group: g,
                        files: self.marked.clone(),
                    })
                }
                None => Some(reorder::Drag::Group(g)),
            };
        } else if released
            && let Some(drag) = self.drag.take()
            && let Some(d) = drop
        {
            // Lasciato fuori da un punto valido, il trascinamento finisce senza spostare nulla.
            let moving = match drag {
                reorder::Drag::Files { files, .. } => files,
                reorder::Drag::Group(g) => group_files.get(g).into_iter().flatten().copied().collect(),
            };
            self.move_files(&moving, d.before);
        }
        if let Some(i) = clicked {
            self.pick(i, mods);
            if !mods.command && !mods.shift {
                self.reveal = Some((i, 0));
            }
        }
        if let Some((i, p)) = page_click {
            self.pick(i, egui::Modifiers::NONE);
            self.selected_page = p;
            self.reveal = Some((i, p));
        }
        if let Some(i) = menu_on {
            self.pick_for_menu(i);
        }
        match action {
            Some(FileAction::RemoveMarked) => {
                let marked = std::mem::take(&mut self.marked);
                self.remove_files(&marked);
            }
            Some(FileAction::RemoveSet(set)) => self.remove_files(&set),
            Some(FileAction::PrintMarked(on)) => {
                let marked = self.marked.clone();
                self.set_printed(&marked, on);
            }
            Some(FileAction::PrintFile(i, on)) => {
                // La casella di un file scelto insieme ad altri vale per tutti, come in Esplora file.
                let set = if self.marked.contains(&i) {
                    self.marked.clone()
                } else {
                    std::collections::BTreeSet::from([i])
                };
                self.set_printed(&set, on);
            }
            Some(FileAction::PrintSet(set, on)) => self.set_printed(&set, on),
            Some(FileAction::PrintPage(i, p, on)) => {
                if let Some(f) = self.project.files.get_mut(i) {
                    f.set_printed(p, on);
                }
            }
            Some(FileAction::Expand(path)) => {
                if !self.expanded.remove(&path) {
                    self.expanded.insert(path);
                }
            }
            Some(FileAction::Open(path)) => {
                if let Err(e) = crate::shell::open(&path) {
                    self.status = format!("Non riesco ad aprire {}: {e}", path.display());
                }
            }
            Some(FileAction::CopyMarked) => {
                let marked = self.marked.clone();
                self.copy_table(Some(&marked));
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
        // Pagina (o tutto il file, con None) da mettere in stampa o togliere.
        let mut print: Option<(Option<usize>, bool)> = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            // Prima cosa che si vede: che cosa del file va in stampa.
            print = print_section(ui, f);
            ui.add_space(6.0);
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
        if let Some((page, on)) = print
            && let Some(f) = self.selected.and_then(|i| self.project.files.get_mut(i))
        {
            match page {
                Some(p) => f.set_printed(p, on),
                None => f.set_all_printed(on),
            }
        }
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
            let edit = fluent::text_field(
                ui,
                egui::TextEdit::multiline(&mut text)
                    .id_salt(("nota-pagina", page))
                    .desired_rows(3)
                    .desired_width(ui.available_width() - 8.0)
                    .margin(fluent::MARGIN)
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
            let edit = fluent::text_field(
                ui,
                egui::TextEdit::multiline(&mut note)
                    .id_salt("nota-cartella")
                    .desired_rows(3)
                    .desired_width(ui.available_width() - 8.0)
                    .margin(fluent::MARGIN)
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

    /// In cima al pannello Impaginazione: il modello in uso, per sceglierne un altro, salvare le
    /// impostazioni come modello o eliminarlo.
    fn presets_section(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let current = crate::presets::matching(&self.presets, &self.project.print).map(|p| p.name.clone());
        dim_label(ui, "Modello");
        let mut chosen = None;
        let mut remove = false;
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 118.0).max(80.0);
            let shown = current.clone().unwrap_or_else(|| {
                if self.presets.is_empty() {
                    "Nessun modello".into()
                } else {
                    "A mano".into()
                }
            });
            ui.add_enabled_ui(!self.presets.is_empty(), |ui| {
                // Il nome lungo si tronca: senza un tetto la casella allargherebbe il pannello.
                ui.set_max_width(width);
                egui::ComboBox::from_id_salt("modello")
                    .icon(fluent::combo_icon)
                    .selected_text(shown)
                    .width(width)
                    .truncate()
                    .show_ui(ui, |ui| {
                        for (k, p) in self.presets.iter().enumerate() {
                            if ui
                                .selectable_label(current.as_deref() == Some(p.name.as_str()), &p.name)
                                .clicked()
                            {
                                chosen = Some(k);
                            }
                        }
                    })
                    .response
                    .on_hover_text(current.as_deref().unwrap_or("Scegli un modello salvato"));
            });
            if ui
                .button("Salva…")
                .on_hover_text("Salva l'impaginazione di adesso come modello (il titolo resta del lavoro)")
                .clicked()
            {
                self.preset_name = Some(current.clone().unwrap_or_default());
            }
            if crate::icons::button(
                ui,
                "trash-2",
                "Elimina questo modello",
                false,
                current.is_some(),
                16.0,
            )
            .clicked()
            {
                remove = true;
            }
        });
        if let Some(p) = chosen.and_then(|k| self.presets.get(k)) {
            crate::presets::apply(p, &mut self.project.print);
            self.status = format!("Impaginazione dal modello «{}»", p.name);
        }
        if remove && let Some(name) = &current {
            self.presets.retain(|p| &p.name != name);
            self.status = format!("Eliminato il modello «{name}»");
        }
        let mut done = None;
        if let Some(name) = self.preset_name.as_mut() {
            let edit = fluent::text_field(
                ui,
                egui::TextEdit::singleline(name)
                    .desired_width(ui.available_width() - 8.0)
                    .margin(fluent::MARGIN)
                    .hint_text("Nome del modello"),
            );
            if !edit.lost_focus() && !edit.has_focus() && name.is_empty() {
                edit.request_focus();
            }
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ok = crate::presets::clean_name(name).is_some();
            ui.horizontal(|ui| {
                if ui.add_enabled(ok, egui::Button::new("Salva")).clicked() || (enter && ok) {
                    done = Some(true);
                }
                if ui.button("Annulla").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    done = Some(false);
                }
            });
            if self.presets.iter().any(|p| {
                crate::presets::clean_name(name).is_some_and(|n| n.to_lowercase() == p.name.to_lowercase())
            }) {
                ui.label(
                    egui::RichText::new("Sostituisce il modello con lo stesso nome")
                        .size(12.0)
                        .color(t.text_dim),
                );
            }
        }
        match done {
            Some(true) => {
                if let Some(name) = self.preset_name.take() {
                    self.status = match crate::presets::save(&mut self.presets, &name, &self.project.print) {
                        Some(n) => format!("Salvato il modello «{n}»"),
                        None => format!(
                            "Troppi modelli: ne tengo al massimo {}",
                            crate::presets::MAX_PRESETS
                        ),
                    };
                }
            }
            Some(false) => self.preset_name = None,
            None => {}
        }
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);
    }

    pub(crate) fn layout_panel(&mut self, ui: &mut egui::Ui) {
        self.presets_section(ui);
        let shown_scale = self.nesting_scale;
        let print = &mut self.project.print;
        let mm =
            |ui: &mut egui::Ui, value: &mut f64, max: f64| fluent::number(ui, 96.0, mm_field(value, max));
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            dim_label(ui, "Titolo in cima ai fogli");
            fluent::text_field(
                ui,
                egui::TextEdit::singleline(&mut print.title)
                    .desired_width(ui.available_width() - 8.0)
                    .margin(fluent::MARGIN)
                    .hint_text("Per esempio: Cliente Rossi, ordine 124"),
            );
            ui.add_space(8.0);
            dim_label(ui, "Testo del titolo");
            ui.add(text_slider(&mut print.title_text, MIN_TITLE_PT..=MAX_TITLE_PT));
            ui.add_space(8.0);
            match print.view {
                View::Grid => {
                    dim_label(ui, "Colonne");
                    ui.add(fluent::Slider::new(&mut print.columns, 1..=10));
                }
                View::List => {
                    dim_label(ui, "Larghezza delle miniature");
                    ui.add(
                        fluent::Slider::new(&mut print.list_thumb, winfiles_layout::LIST_THUMB_MM)
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
            ui.add(fluent::checkbox(
                &mut print.show_sizes,
                "Misure con abbondanza e crocini",
            ))
            .on_hover_text("Il formato pagina del file e la sua area, sotto ogni miniatura");
            ui.add(fluent::checkbox(&mut print.show_trim, "Misure al vivo"))
                .on_hover_text("In rosso, il formato finito (TrimBox) quando è diverso dalla pagina");
            ui.add(fluent::checkbox(
                &mut print.show_summary,
                "Foglio di riepilogo in cima",
            ))
            .on_hover_text("Un foglio con l'elenco di cartelle e file, pagine, misure, m² e note");
            ui.add(fluent::checkbox(
                &mut print.page_break_per_folder,
                "Ogni cartella su un foglio nuovo",
            ));
            ui.add(fluent::checkbox(&mut print.show_notes, "Stampa le note"))
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
pub(crate) fn mm_field(value: &mut f64, max: f64) -> egui::DragValue<'_> {
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
pub(crate) fn text_slider(value: &mut f64, range: std::ops::RangeInclusive<f64>) -> fluent::Slider<'_> {
    fluent::Slider::new(value, range)
        .step_by(0.5)
        .custom_formatter(|v| format!("{} pt", format_number(v, 1, true)))
        .custom_parser(|text| {
            let clean = text.trim().trim_end_matches("pt").trim().replace(',', ".");
            clean.parse::<f64>().ok()
        })
}
