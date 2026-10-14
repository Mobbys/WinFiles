//! Stato dell'applicazione e cornice della finestra: barra dei menu, barra di controllo, pannelli
//! agganciabili e barra di stato, con l'aspetto di VectorCraft.

use std::collections::HashMap;
use std::path::PathBuf;

use craft_ui::docking::{Action, DockArea, DockContent, Layout, PanelLimits};
use egui::{Align, Color32, Rect, Sense, Stroke, Vec2, vec2};
use winfiles_geom::{Orientation, Paper, Size, Unit, format_area};
use winfiles_layout::{Item, LayoutError, MAX_COLUMNS, Sheet, grid};
use winfiles_model::{FileKind, Project, Totals};

use crate::dock::{self, PanelId};
use crate::icons;
use crate::measure::Measurer;
use crate::theme::{self, ThemeChoice, Tokens, semibold};
use crate::thumbs::Thumbnails;
use crate::{canvas, scan};

/// Punti schermo per millimetro al 100% (egui lavora a 96 punti per pollice).
pub const POINTS_PER_MM: f32 = 96.0 / 25.4;
const ZOOM_STOPS: [f32; 10] = [25.0, 33.0, 50.0, 66.7, 100.0, 150.0, 200.0, 300.0, 400.0, 800.0];

pub struct WinFilesApp {
    pub(crate) project: Project,
    pub(crate) selected: Option<usize>,
    pub(crate) sheet: usize,
    /// Ingrandimento in punti per millimetro; `None` adatta il foglio alla finestra.
    pub(crate) zoom: Option<f32>,
    /// L'ultimo ingrandimento usato, per mostrarlo nella barra di stato.
    pub(crate) shown_zoom: f32,
    /// Errori di lettura per file.
    pub(crate) errors: HashMap<PathBuf, String>,
    pub(crate) theme: ThemeChoice,
    applied: Option<Tokens>,
    dock: Layout<PanelId>,
    measurer: Measurer,
    pub(crate) thumbs: Thumbnails,
    status: String,
}

/// Proporzioni provvisorie finché un file non è stato misurato.
fn placeholder_size(kind: FileKind) -> Size {
    match kind {
        FileKind::Image => Size::new(4.0, 3.0),
        _ => Size::new(210.0, 297.0),
    }
}

/// Gli elementi da impaginare, una pagina per file per ora, con le proporzioni vere.
fn items(project: &Project) -> Vec<Item> {
    let labels = project.folder_labels();
    project
        .files
        .iter()
        .enumerate()
        .map(|(id, f)| Item {
            id,
            content: f
                .pages
                .first()
                .map(|p| p.finished())
                .unwrap_or_else(|| placeholder_size(f.kind)),
            folder: labels
                .get(f.folder())
                .cloned()
                .unwrap_or_else(|| f.folder().display().to_string()),
        })
        .collect()
}

impl WinFilesApp {
    pub fn new(ctx: &egui::Context, paths: Vec<PathBuf>) -> Self {
        let wake = ctx.clone();
        let wake2 = ctx.clone();
        let mut app = Self {
            project: Project::new("Senza titolo"),
            selected: None,
            sheet: 0,
            zoom: None,
            shown_zoom: 1.0,
            errors: HashMap::new(),
            theme: ThemeChoice::System,
            applied: None,
            dock: dock::default_layout(),
            measurer: Measurer::start(move || wake.request_repaint()),
            thumbs: Thumbnails::start(move || wake2.request_repaint()),
            status: String::new(),
        };
        if !paths.is_empty() {
            app.add_paths(paths);
        }
        app
    }

    fn add_paths(&mut self, paths: Vec<PathBuf>) {
        let before = self.project.files.len();
        let mut files = std::mem::take(&mut self.project.files);
        files.extend(scan::collect(&paths));
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files.dedup_by(|a, b| a.path == b.path);
        for f in files
            .iter()
            .filter(|f| f.pages.is_empty() && !self.errors.contains_key(&f.path))
        {
            self.measurer.request(f.path.clone(), f.kind);
        }
        let added = files.len().saturating_sub(before);
        self.project.files = files;
        self.selected = None;
        self.status = match added {
            0 => "Nessun file nuovo: immagini, PDF, AI ed EPS".into(),
            1 => "Aggiunto 1 file".into(),
            n => format!("Aggiunti {n} file"),
        };
    }

    fn clear(&mut self) {
        self.project.files.clear();
        self.errors.clear();
        self.thumbs.clear();
        self.selected = None;
        self.sheet = 0;
        self.status = "Elenco svuotato".into();
    }

    /// Porta nel progetto le misure arrivate dal thread di lettura.
    fn receive_measures(&mut self) {
        for m in self.measurer.drain() {
            // I file sono ordinati per percorso; uno tolto nel frattempo si ignora.
            let Ok(i) = self.project.files.binary_search_by(|f| f.path.cmp(&m.path)) else {
                continue;
            };
            let Some(file) = self.project.files.get_mut(i) else {
                continue;
            };
            match m.result {
                Ok(probe) => {
                    file.pages = probe.pages;
                    file.image = probe.image;
                    self.errors.remove(&m.path);
                }
                Err(e) => {
                    self.errors.insert(m.path, e.to_string());
                }
            }
        }
    }

    /// Applica il tema scelto (o quello di Windows) quando cambia.
    fn sync_theme(&mut self, ctx: &egui::Context) {
        let tokens = self.theme.resolve(ctx.system_theme());
        if self.applied != Some(tokens) {
            theme::apply(ctx, tokens);
            self.applied = Some(tokens);
        }
    }

    fn apply_dock(&mut self, action: Action<PanelId>) {
        if let Err(e) = self.dock.apply_with_permissions(action, |p| p.permissions()) {
            self.status = format!("Pannelli: {e}");
        }
    }

    fn toggle_panel(&mut self, panel: PanelId) {
        let action = if self.dock.contains(&panel) {
            Action::Close { panel }
        } else {
            Action::OpenAt {
                panel,
                anchor: PanelId::Canvas,
                placement: panel.reopen_at(),
            }
        };
        self.apply_dock(action);
    }

    fn app_bar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE
            .fill(t.app_bar)
            .inner_margin(egui::Margin::symmetric(10, 0))
            .stroke(Stroke::new(1.0, t.border));
        egui::Panel::top("barra-menu")
            .exact_size(34.0)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(
                        egui::RichText::new("WinFiles")
                            .font(semibold(13.5))
                            .color(t.text_strong),
                    );
                    ui.add_space(8.0);
                    egui::MenuBar::new().ui(ui, |ui| {
                        ui.menu_button("File", |ui| {
                            if ui
                                .add_enabled(
                                    !self.project.files.is_empty(),
                                    egui::Button::new("Svuota elenco"),
                                )
                                .clicked()
                            {
                                self.clear();
                                ui.close();
                            }
                        });
                        ui.menu_button("Visualizza", |ui| {
                            if ui.button("Adatta alla finestra").clicked() {
                                self.zoom = None;
                                ui.close();
                            }
                            if ui.button("Dimensioni reali (100%)").clicked() {
                                self.zoom = Some(POINTS_PER_MM);
                                ui.close();
                            }
                            ui.separator();
                            for choice in ThemeChoice::ALL {
                                let label = format!("Tema: {}", choice.label().to_lowercase());
                                if ui.radio(self.theme == choice, label).clicked() {
                                    self.theme = choice;
                                    ui.close();
                                }
                            }
                            ui.separator();
                            for unit in Unit::ALL {
                                let label = match unit {
                                    Unit::Mm => "Misure in millimetri",
                                    Unit::Cm => "Misure in centimetri",
                                };
                                if ui.radio(self.project.unit == unit, label).clicked() {
                                    self.project.unit = unit;
                                    ui.close();
                                }
                            }
                        });
                        ui.menu_button("Finestra", |ui| {
                            for panel in PanelId::TOOLS {
                                if ui
                                    .checkbox(&mut self.dock.contains(&panel), panel.label())
                                    .clicked()
                                {
                                    self.toggle_panel(panel);
                                    ui.close();
                                }
                            }
                            ui.separator();
                            if ui.button("Ripristina la disposizione").clicked() {
                                self.dock = dock::default_layout();
                                ui.close();
                            }
                        });
                        ui.menu_button("Aiuto", |ui| {
                            ui.label(concat!("WinFiles ", env!("CARGO_PKG_VERSION")));
                            ui.label(
                                egui::RichText::new(
                                    "Trascina file o cartelle nella finestra per aggiungerli.",
                                )
                                .color(t.text_dim),
                            );
                        });
                    });
                });
            });
    }

    /// La barra di controllo sotto i menu: le scelte di stampa che si cambiano più spesso.
    fn control_bar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE
            .fill(t.panel)
            .inner_margin(egui::Margin::symmetric(10, 0))
            .stroke(Stroke::new(1.0, t.border));
        egui::Panel::top("barra-controllo")
            .exact_size(36.0)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let print = &mut self.project.print;
                    dim_label(ui, "Foglio");
                    segmented(
                        ui,
                        "carta",
                        &mut print.paper,
                        &[(Paper::A4, "A4"), (Paper::A3, "A3")],
                    );
                    segmented(
                        ui,
                        "orientamento",
                        &mut print.orientation,
                        &[
                            (Orientation::Portrait, "Verticale"),
                            (Orientation::Landscape, "Orizzontale"),
                        ],
                    );
                    bar_divider(ui);
                    dim_label(ui, "Colonne");
                    if icons::button(ui, "minus", "Una colonna in meno", false, print.columns > 1, 22.0)
                        .clicked()
                    {
                        print.columns = print.columns.saturating_sub(1).max(1);
                    }
                    ui.label(egui::RichText::new(print.columns.to_string()).color(t.text_strong));
                    if icons::button(
                        ui,
                        "plus",
                        "Una colonna in più",
                        false,
                        print.columns < MAX_COLUMNS,
                        22.0,
                    )
                    .clicked()
                    {
                        print.columns = (print.columns + 1).min(MAX_COLUMNS);
                    }
                    bar_divider(ui);
                    ui.checkbox(
                        &mut print.page_break_per_folder,
                        "Ogni cartella su un foglio nuovo",
                    );
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        segmented(
                            ui,
                            "tema",
                            &mut self.theme,
                            &[
                                (ThemeChoice::Light, "Chiaro"),
                                (ThemeChoice::Dark, "Scuro"),
                                (ThemeChoice::System, "Auto"),
                            ],
                        );
                        dim_label(ui, "Tema");
                        bar_divider(ui);
                        segmented(
                            ui,
                            "unita",
                            &mut self.project.unit,
                            &[(Unit::Cm, "cm"), (Unit::Mm, "mm")],
                        );
                        dim_label(ui, "Misure");
                    });
                });
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, sheets: usize) {
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE
            .fill(t.panel)
            .inner_margin(egui::Margin::symmetric(8, 0))
            .stroke(Stroke::new(1.0, t.border));
        egui::Panel::bottom("barra-stato")
            .exact_size(26.0)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let percent = self.shown_zoom / POINTS_PER_MM * 100.0;
                    egui::ComboBox::from_id_salt("zoom")
                        .selected_text(egui::RichText::new(format!("{percent:.0}%")).size(11.5))
                        .width(70.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(self.zoom.is_none(), "Adatta").clicked() {
                                self.zoom = None;
                            }
                            for z in ZOOM_STOPS {
                                if ui.selectable_label(false, format!("{z:.0}%")).clicked() {
                                    self.zoom = Some(z / 100.0 * POINTS_PER_MM);
                                }
                            }
                        });
                    bar_divider(ui);
                    let last = sheets.saturating_sub(1);
                    if icons::button(ui, "chevrons-left", "Primo foglio", false, self.sheet > 0, 20.0)
                        .clicked()
                    {
                        self.sheet = 0;
                    }
                    if icons::button(
                        ui,
                        "chevron-left",
                        "Foglio precedente",
                        false,
                        self.sheet > 0,
                        20.0,
                    )
                    .clicked()
                    {
                        self.sheet = self.sheet.saturating_sub(1);
                    }
                    let text = if sheets == 0 {
                        "–".to_string()
                    } else {
                        format!("Foglio {} di {sheets}", self.sheet + 1)
                    };
                    ui.label(egui::RichText::new(text).size(11.5));
                    if icons::button(
                        ui,
                        "chevron-right",
                        "Foglio successivo",
                        false,
                        self.sheet < last,
                        20.0,
                    )
                    .clicked()
                    {
                        self.sheet = (self.sheet + 1).min(last);
                    }
                    if icons::button(
                        ui,
                        "chevrons-right",
                        "Ultimo foglio",
                        false,
                        self.sheet < last,
                        20.0,
                    )
                    .clicked()
                    {
                        self.sheet = last;
                    }
                    bar_divider(ui);
                    let totals = Totals::of(&self.project.files);
                    ui.label(
                        egui::RichText::new(format!(
                            "{} file · {} pagine · {}",
                            totals.files,
                            totals.pages,
                            format_area(totals.area_m2)
                        ))
                        .size(11.5)
                        .color(t.text_strong),
                    );
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        let pending = self.measurer.pending();
                        if pending > 0 {
                            ui.label(egui::RichText::new(format!("Misuro {pending} file…")).size(11.0));
                            ui.add(egui::Spinner::new().size(12.0).color(t.accent));
                        } else if !self.status.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&self.status).size(11.0).color(t.text_dim),
                                )
                                .truncate(),
                            );
                        }
                    });
                });
            });
    }

    fn canvas_title(&self, sheets: usize) -> String {
        let print = &self.project.print;
        let orientation = match print.orientation {
            Orientation::Portrait => "verticale",
            Orientation::Landscape => "orizzontale",
        };
        let percent = self.shown_zoom / POINTS_PER_MM * 100.0;
        let fogli = if sheets == 1 { "foglio" } else { "fogli" };
        format!(
            "{} · {} {orientation} · {sheets} {fogli} @ {percent:.0}%",
            self.project.title,
            print.paper.name()
        )
    }

    /// L'anteprima, sul fondo grigio.
    pub(crate) fn canvas(
        &mut self,
        ui: &mut egui::Ui,
        sheets: &Result<Vec<Sheet>, LayoutError>,
        title: &str,
    ) {
        let t = Tokens::get(ui.ctx());
        doc_tab(ui, title);
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, t.pasteboard);
        let sheets = match sheets {
            Ok(s) => s,
            Err(e) => {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    e.to_string(),
                    egui::FontId::proportional(13.0),
                    t.text_strong,
                );
                return;
            }
        };
        if self.project.files.is_empty() {
            empty_hint(ui, rect);
        }
        self.sheet = self.sheet.min(sheets.len().saturating_sub(1));
        let Some(sheet) = sheets.get(self.sheet) else {
            return;
        };
        let scale = self.zoom.unwrap_or_else(|| canvas::fit_zoom(sheet, rect.size()));
        self.shown_zoom = scale;
        let out = egui::ScrollArea::both()
            .auto_shrink(false)
            .show(ui, |ui| {
                canvas::draw(ui, sheet, &self.project, &mut self.thumbs, self.selected, scale)
            })
            .inner;
        if let Some(id) = out.clicked {
            self.selected = Some(id);
        }
        if let Some(by) = out.zoom_by {
            self.zoom = Some((scale * by).clamp(canvas::MIN_ZOOM, canvas::MAX_ZOOM));
        }
    }
}

/// La linguetta del documento sopra l'anteprima, come in VectorCraft.
fn doc_tab(ui: &mut egui::Ui, title: &str) {
    let t = Tokens::get(ui.ctx());
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    ui.painter().rect_filled(strip, 0.0, t.tab_strip);
    ui.painter().line_segment(
        [strip.left_bottom(), strip.right_bottom()],
        Stroke::new(1.0, t.border),
    );
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_string(), semibold(12.0), t.text_strong);
    let w = (galley.size().x + 28.0).min(strip.width());
    let tab = Rect::from_min_size(strip.min, vec2(w, strip.height() - 1.0));
    ui.painter().rect_filled(tab, 0.0, t.panel);
    ui.painter()
        .line_segment([tab.right_top(), tab.right_bottom()], Stroke::new(1.0, t.border));
    ui.painter().with_clip_rect(tab).galley(
        egui::pos2(tab.left() + 14.0, tab.center().y - galley.size().y / 2.0),
        galley,
        t.text_strong,
    );
}

/// Avviso al centro dell'anteprima vuota.
fn empty_hint(ui: &egui::Ui, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    let c = rect.center();
    icons::paint(
        ui,
        "folder-open",
        Rect::from_center_size(c - vec2(0.0, 44.0), Vec2::splat(48.0)),
        t.text,
    );
    let p = ui.painter();
    p.text(
        c,
        egui::Align2::CENTER_CENTER,
        "Trascina qui file o cartelle",
        semibold(16.0),
        t.text_strong,
    );
    p.text(
        c + vec2(0.0, 24.0),
        egui::Align2::CENTER_CENTER,
        "Immagini, PDF, AI ed EPS, anche dentro le sottocartelle",
        egui::FontId::proportional(12.5),
        t.text,
    );
}

pub(crate) fn dim_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(Tokens::get(ui.ctx()).text_dim));
}

fn bar_divider(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    let (r, _) = ui.allocate_exact_size(vec2(1.0, 18.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, Tokens::get(ui.ctx()).divider);
    ui.add_space(4.0);
}

/// Pulsanti uniti a scelta singola, come i selettori di VectorCraft.
pub(crate) fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut T,
    options: &[(T, &str)],
) {
    let t = Tokens::get(ui.ctx());
    let font = egui::FontId::proportional(12.0);
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, label)| {
            ui.painter()
                .layout_no_wrap((*label).to_string(), font.clone(), Color32::WHITE)
                .size()
                .x
                + 18.0
        })
        .collect();
    let total: f32 = widths.iter().sum();
    let (rect, _) = ui.allocate_exact_size(vec2(total, 22.0), Sense::hover());
    ui.painter().rect_filled(rect, t.radius, t.input);
    let mut x = rect.left();
    for (i, ((option, label), w)) in options.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(egui::pos2(x, rect.top()), vec2(*w, rect.height()));
        let resp = ui.interact(r, ui.id().with((id, i)), Sense::click());
        let on = *value == *option;
        if on {
            ui.painter().rect_filled(r.shrink(1.0), t.radius, t.hover);
        } else if resp.hovered() {
            ui.painter().rect_filled(r.shrink(1.0), t.radius, t.panel);
        }
        let color = if on { t.text_strong } else { t.text };
        ui.painter().text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            font.clone(),
            color,
        );
        if resp.clicked() {
            *value = *option;
        }
        x += w;
    }
    ui.painter().rect_stroke(
        rect,
        t.radius,
        Stroke::new(1.0, t.input_border),
        egui::StrokeKind::Inside,
    );
}

/// Collega i pannelli agganciabili al loro contenuto.
struct Content<'a> {
    app: &'a mut WinFilesApp,
    sheets: &'a Result<Vec<Sheet>, LayoutError>,
    title: &'a str,
}

impl DockContent<PanelId> for Content<'_> {
    fn body(&mut self, ui: &mut egui::Ui, panel: &PanelId) {
        if *panel == PanelId::Canvas {
            self.app.canvas(ui, self.sheets, self.title);
            return;
        }
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                match panel {
                    PanelId::Files => self.app.files_panel(ui),
                    PanelId::Properties => self.app.properties_panel(ui),
                    PanelId::Notes => self.app.notes_panel(ui),
                    PanelId::Layout => self.app.layout_panel(ui),
                    PanelId::Canvas => {}
                }
            });
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
        self.sync_theme(ui.ctx());
        self.receive_measures();
        self.thumbs.receive(ui.ctx());

        let sheets = grid(&self.project.print, &items(&self.project));
        let count = sheets.as_ref().map_or(0, Vec::len);
        self.app_bar(ui);
        self.control_bar(ui);
        self.status_bar(ui, count);
        let title = self.canvas_title(count);
        let frame = egui::Frame::NONE.fill(Tokens::get(ui.ctx()).border);
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            let layout = self.dock.clone();
            let style = dock::style(ui);
            let output = DockArea::new(egui::Id::new("winfiles-pannelli")).show_customized(
                ui,
                &layout,
                &style,
                |p| p.label().to_string(),
                |p| p.permissions(),
                |p| match p {
                    PanelId::Canvas => PanelLimits {
                        min: vec2(240.0, 200.0),
                        ..Default::default()
                    },
                    _ => PanelLimits {
                        min: vec2(200.0, 90.0),
                        ..Default::default()
                    },
                },
                &mut Content {
                    app: self,
                    sheets: &sheets,
                    title: &title,
                },
            );
            if let Some(e) = output.error {
                self.status = format!("Disposizione dei pannelli non valida ({e}): ripristinata");
                self.dock = dock::default_layout();
            }
            for action in output.actions {
                self.apply_dock(action);
            }
        });
    }
}
