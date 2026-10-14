//! Stato e pannelli dell'applicazione. Disposizione provvisoria a pannelli fissi: il docking di
//! `craft-ui` arriva con il prototipo della Fase 0 (vedi `docs/adr/0002-interfaccia-a-pannelli.md`).

use std::path::PathBuf;

use winfiles_geom::{Margins, Orientation, Paper, Size};
use winfiles_layout::{Item, LayoutError, Sheet, grid};
use winfiles_model::{FileKind, Project};

use crate::{preview, scan};

#[derive(Default)]
pub struct WinFilesApp {
    project: Project,
    selected: Option<usize>,
    sheet: usize,
}

/// Proporzioni provvisorie finché i decodificatori (Fase 2) non leggono le misure vere.
fn placeholder_size(kind: FileKind) -> Size {
    match kind {
        FileKind::Image => Size::new(4.0, 3.0),
        _ => Size::new(210.0, 297.0),
    }
}

/// Gli elementi da impaginare, una pagina per file per ora.
fn items(project: &Project) -> Vec<Item> {
    project
        .files
        .iter()
        .enumerate()
        .map(|(id, f)| Item {
            id,
            content: f
                .pages
                .first()
                .map(|p| p.size)
                .unwrap_or_else(|| placeholder_size(f.kind)),
            folder: f.folder().display().to_string(),
        })
        .collect()
}

impl WinFilesApp {
    pub fn with_paths(paths: Vec<PathBuf>) -> Self {
        let mut app = Self::default();
        if !paths.is_empty() {
            app.add_paths(paths);
        }
        app
    }

    fn add_paths(&mut self, paths: Vec<PathBuf>) {
        let mut files = std::mem::take(&mut self.project.files);
        files.extend(scan::collect(&paths));
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files.dedup_by(|a, b| a.path == b.path);
        self.project.files = files;
        self.selected = None;
    }

    fn files_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("File");
        ui.label(format!("{} file", self.project.files.len()));
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let mut last_folder = None;
            for (i, file) in self.project.files.iter().enumerate() {
                let folder = file.folder();
                if last_folder != Some(folder) {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(folder.display().to_string()).strong().small());
                    last_folder = Some(folder);
                }
                let name = file
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let text = if file.note.is_empty() {
                    name
                } else {
                    format!("{name}  ✎")
                };
                if ui.selectable_label(self.selected == Some(i), text).clicked() {
                    self.selected = Some(i);
                }
            }
        });
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Impaginazione");
        let print = &mut self.project.print;
        egui::Grid::new("impaginazione")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Formato");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut print.paper, Paper::A4, "A4");
                    ui.selectable_value(&mut print.paper, Paper::A3, "A3");
                });
                ui.end_row();
                ui.label("Orientamento");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut print.orientation, Orientation::Portrait, "Verticale");
                    ui.selectable_value(&mut print.orientation, Orientation::Landscape, "Orizzontale");
                });
                ui.end_row();
                ui.label("Colonne");
                ui.add(egui::Slider::new(&mut print.columns, 1..=10));
                ui.end_row();
                ui.label("Margini");
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
                ui.label("Spazio tra miniature");
                ui.add(
                    egui::DragValue::new(&mut print.gap)
                        .range(0.0..=20.0)
                        .speed(0.5)
                        .suffix(" mm"),
                );
                ui.end_row();
                ui.label("Didascalia");
                ui.add(
                    egui::DragValue::new(&mut print.caption)
                        .range(0.0..=40.0)
                        .speed(0.5)
                        .suffix(" mm"),
                );
                ui.end_row();
            });
        ui.checkbox(
            &mut print.page_break_per_folder,
            "Ogni cartella su un foglio nuovo",
        );

        ui.add_space(12.0);
        ui.heading("Note");
        match self.selected.and_then(|i| self.project.files.get_mut(i)) {
            Some(file) => {
                ui.label("Nota del file");
                ui.add(
                    egui::TextEdit::multiline(&mut file.note)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
                let folder = file.folder().to_path_buf();
                ui.label("Nota della cartella");
                let note = self.project.folder_notes.entry(folder).or_default();
                ui.add(
                    egui::TextEdit::multiline(note)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
            }
            None => {
                ui.label("Seleziona un file per scrivere le sue note e quelle della sua cartella.");
            }
        }
    }

    fn preview_panel(&mut self, ui: &mut egui::Ui, sheets: &Result<Vec<Sheet>, LayoutError>) {
        let sheets = match sheets {
            Ok(s) => s,
            Err(e) => {
                ui.colored_label(ui.visuals().error_fg_color, e.to_string());
                return;
            }
        };
        let count = sheets.len();
        self.sheet = self.sheet.min(count.saturating_sub(1));
        ui.horizontal(|ui| {
            if ui.add_enabled(self.sheet > 0, egui::Button::new("◀")).clicked() {
                self.sheet -= 1;
            }
            ui.label(format!("Foglio {} di {}", self.sheet + 1, count));
            if ui
                .add_enabled(self.sheet + 1 < count, egui::Button::new("▶"))
                .clicked()
            {
                self.sheet += 1;
            }
            if self.project.files.is_empty() {
                ui.label("Trascina qui file o cartelle: immagini, PDF, AI, EPS.");
            }
        });
        if let Some(sheet) = sheets.get(self.sheet) {
            let clicked = preview::draw_sheet(ui, sheet, &self.project, self.selected);
            if let Some(id) = clicked {
                self.selected = Some(id);
            }
        }
    }
}

impl eframe::App for WinFilesApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let dropped: Vec<PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if !dropped.is_empty() {
            self.add_paths(dropped);
        }

        egui::Panel::top("barra").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("WinFiles");
                ui.separator();
                if ui.button("Svuota elenco").clicked() {
                    self.project.files.clear();
                    self.selected = None;
                }
            });
        });
        egui::Panel::left("file")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| self.files_panel(ui));
        egui::Panel::right("impostazioni")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| self.settings_panel(ui));
        let sheets = grid(&self.project.print, &items(&self.project));
        egui::CentralPanel::default().show(ui, |ui| self.preview_panel(ui, &sheets));
    }
}
