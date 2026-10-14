//! Stato dell'applicazione e cornice della finestra: barra dei menu, barra di stampa, pannelli
//! agganciabili su schede arrotondate e barra di stato.

use std::collections::HashMap;
use std::path::PathBuf;

use craft_ui::docking::{Action, DockArea, DockContent, Layout, PanelLimits};
use egui::{Align, Color32, Rect, Sense, Stroke, Vec2, vec2};
use winfiles_geom::{Orientation, Paper, Unit, format_area};
use winfiles_layout::{LayoutError, MAX_COLUMNS, Sheet, grid_with_title};
use winfiles_model::{Project, Totals};

use crate::captions::{self, Slot};
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
    /// Le miniature sul foglio, una per pagina, ricalcolate a ogni fotogramma.
    slots: Vec<Slot>,
    /// Cartella di cui si sta rinominando l'intestazione nell'elenco, con il testo in modifica.
    pub(crate) renaming: Option<(PathBuf, String)>,
    status: String,
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
            slots: Vec::new(),
            renaming: None,
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

    /// La barra in alto: marchio, menu e scelte di aspetto (tema e unità).
    fn app_bar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE.fill(t.app_bar).inner_margin(egui::Margin {
            left: 14,
            right: 14,
            top: 8,
            bottom: 2,
        });
        egui::Panel::top("barra-menu")
            .exact_size(48.0)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    brand(ui);
                    ui.add_space(10.0);
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
                            dim_label(ui, "Trascina file o cartelle nella finestra per aggiungerli.");
                        });
                    });
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
                        ui.add_space(12.0);
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

    /// La barra delle scelte di stampa che si cambiano più spesso.
    fn control_bar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE.fill(t.app_bar).inner_margin(egui::Margin {
            left: 14,
            right: 14,
            top: 2,
            bottom: 6,
        });
        egui::Panel::top("barra-controllo")
            .exact_size(46.0)
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
                    ui.add_space(12.0);
                    dim_label(ui, "Colonne");
                    stepper(ui, &mut print.columns, 1, MAX_COLUMNS);
                    ui.add_space(12.0);
                    ui.checkbox(
                        &mut print.page_break_per_folder,
                        "Ogni cartella su un foglio nuovo",
                    );
                });
            });
    }

    /// La barra in basso: totali del lavoro e messaggi.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE
            .fill(t.app_bar)
            .inner_margin(egui::Margin::symmetric(16, 0));
        egui::Panel::bottom("barra-stato")
            .exact_size(32.0)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let totals = Totals::of(&self.project.files);
                    let text = format!(
                        "{} file  ·  {} pagine  ·  {}",
                        totals.files,
                        totals.pages,
                        format_area(totals.area_m2)
                    );
                    ui.label(egui::RichText::new(text).font(semibold(12.5)).color(t.text));
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        let pending = self.measurer.pending();
                        if pending > 0 {
                            ui.label(egui::RichText::new(format!("Misuro {pending} file…")).size(12.0));
                            ui.add(egui::Spinner::new().size(13.0).color(t.accent));
                        } else if !self.status.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&self.status).size(12.0).color(t.text_dim),
                                )
                                .truncate(),
                            );
                        }
                    });
                });
            });
    }

    /// L'anteprima: una scheda con titolo, navigazione tra i fogli e zoom, e il foglio sul fondo.
    pub(crate) fn canvas(&mut self, ui: &mut egui::Ui, sheets: &Result<Vec<Sheet>, LayoutError>) {
        let t = Tokens::get(ui.ctx());
        let full = ui.max_rect();
        card(ui, full);
        let count = sheets.as_ref().map_or(0, Vec::len);
        self.sheet = self.sheet.min(count.saturating_sub(1));

        // Intestazione della scheda.
        let header = Rect::from_min_size(full.min, vec2(full.width(), 48.0)).shrink2(vec2(14.0, 0.0));
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        let print = &self.project.print;
        let orientation = match print.orientation {
            Orientation::Portrait => "verticale",
            Orientation::Landscape => "orizzontale",
        };
        bar.label(
            egui::RichText::new("Anteprima di stampa")
                .font(semibold(14.0))
                .color(t.text_strong),
        );
        bar.label(egui::RichText::new(format!("{} {orientation}", print.paper.name())).color(t.text_dim));
        bar.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            let percent = self.shown_zoom / POINTS_PER_MM * 100.0;
            egui::ComboBox::from_id_salt("zoom")
                .selected_text(if self.zoom.is_none() {
                    format!("Adatta ({percent:.0}%)")
                } else {
                    format!("{percent:.0}%")
                })
                .width(118.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(self.zoom.is_none(), "Adatta alla finestra")
                        .clicked()
                    {
                        self.zoom = None;
                    }
                    for z in ZOOM_STOPS {
                        if ui.selectable_label(false, format!("{z:.0}%")).clicked() {
                            self.zoom = Some(z / 100.0 * POINTS_PER_MM);
                        }
                    }
                });
            ui.add_space(8.0);
            let last = count.saturating_sub(1);
            if icons::button(
                ui,
                "chevron-right",
                "Foglio successivo",
                false,
                self.sheet < last,
                28.0,
            )
            .clicked()
            {
                self.sheet = (self.sheet + 1).min(last);
            }
            let text = if count == 0 {
                "–".to_string()
            } else {
                format!("Foglio {} di {count}", self.sheet + 1)
            };
            ui.label(egui::RichText::new(text).color(t.text));
            if icons::button(
                ui,
                "chevron-left",
                "Foglio precedente",
                false,
                self.sheet > 0,
                28.0,
            )
            .clicked()
            {
                self.sheet = self.sheet.saturating_sub(1);
            }
        });

        // Il fondo su cui sta il foglio.
        let board = Rect::from_min_max(
            egui::pos2(full.left() + 8.0, full.top() + 48.0),
            full.max - vec2(8.0, 8.0),
        );
        if board.width() < 20.0 || board.height() < 20.0 {
            return;
        }
        ui.painter().rect_filled(board, t.radius, t.pasteboard);
        let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(board.shrink(2.0)));
        inner.set_clip_rect(board.shrink(2.0).intersect(ui.clip_rect()));
        let ui = &mut inner;
        let sheets = match sheets {
            Ok(s) => s,
            Err(e) => {
                ui.painter().text(
                    board.center(),
                    egui::Align2::CENTER_CENTER,
                    e.to_string(),
                    egui::FontId::proportional(14.0),
                    t.text_strong,
                );
                return;
            }
        };
        if self.project.files.is_empty() {
            empty_hint(ui, board);
        }
        let Some(sheet) = sheets.get(self.sheet) else {
            return;
        };
        let scale = self.zoom.unwrap_or_else(|| canvas::fit_zoom(sheet, board.size()));
        self.shown_zoom = scale;
        let out = egui::ScrollArea::both()
            .auto_shrink(false)
            .show(ui, |ui| {
                canvas::draw(
                    ui,
                    sheet,
                    &self.project,
                    &self.slots,
                    &mut self.thumbs,
                    self.selected,
                    scale,
                )
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

/// Il marchio in alto a sinistra: un quadratino blu con la W e il nome.
fn brand(ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
    ui.painter().rect_filled(r, 7.0, t.accent);
    ui.painter().text(
        r.center(),
        egui::Align2::CENTER_CENTER,
        "W",
        semibold(15.0),
        Color32::WHITE,
    );
    ui.label(
        egui::RichText::new("WinFiles")
            .font(semibold(15.0))
            .color(t.text_strong),
    );
}

/// Una scheda arrotondata che fa da fondo a un pannello.
pub(crate) fn card(ui: &egui::Ui, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(rect, t.card_radius, t.panel);
    ui.painter().rect_stroke(
        rect,
        t.card_radius,
        Stroke::new(1.0, t.border),
        egui::StrokeKind::Inside,
    );
}

/// Avviso al centro dell'anteprima vuota.
fn empty_hint(ui: &egui::Ui, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    let c = rect.center();
    icons::paint(
        ui,
        "folder-open",
        Rect::from_center_size(c - vec2(0.0, 48.0), Vec2::splat(48.0)),
        t.text_dim,
    );
    let p = ui.painter();
    p.text(
        c,
        egui::Align2::CENTER_CENTER,
        "Trascina qui file o cartelle",
        semibold(17.0),
        t.text_strong,
    );
    p.text(
        c + vec2(0.0, 26.0),
        egui::Align2::CENTER_CENTER,
        "Immagini, PDF, AI ed EPS, anche dentro le sottocartelle",
        egui::FontId::proportional(13.5),
        t.text_dim,
    );
}

pub(crate) fn dim_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(Tokens::get(ui.ctx()).text_dim));
}

/// Un numero con i pulsanti − e + ai lati, in un unico contenitore arrotondato.
pub(crate) fn stepper(ui: &mut egui::Ui, value: &mut u32, min: u32, max: u32) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(96.0, 30.0), Sense::hover());
    ui.painter().rect_filled(rect, t.radius, t.input);
    ui.painter().rect_stroke(
        rect,
        t.radius,
        Stroke::new(1.0, t.input_border),
        egui::StrokeKind::Inside,
    );
    let minus = Rect::from_min_size(rect.min, vec2(30.0, 30.0));
    let plus = Rect::from_min_size(egui::pos2(rect.right() - 30.0, rect.top()), vec2(30.0, 30.0));
    for (r, icon, enabled, delta) in [
        (minus, "minus", *value > min, -1i64),
        (plus, "plus", *value < max, 1),
    ] {
        let resp = ui.interact(
            r,
            ui.id().with(("stepper", icon)),
            if enabled { Sense::click() } else { Sense::hover() },
        );
        if enabled && resp.hovered() {
            ui.painter().rect_filled(r.shrink(2.0), t.radius, t.hover);
        }
        let tint = if enabled { t.icon } else { t.text_disabled };
        icons::paint(
            ui,
            icon,
            Rect::from_center_size(r.center(), Vec2::splat(15.0)),
            tint,
        );
        if resp.clicked() {
            *value = u32::try_from((i64::from(*value) + delta).clamp(i64::from(min), i64::from(max)))
                .unwrap_or(min);
        }
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        value.to_string(),
        semibold(13.5),
        t.text_strong,
    );
}

/// Pulsanti uniti a scelta singola: la scelta attiva è evidenziata in blu tenue.
pub(crate) fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut T,
    options: &[(T, &str)],
) {
    let t = Tokens::get(ui.ctx());
    let font = egui::FontId::proportional(13.0);
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, label)| {
            ui.painter()
                .layout_no_wrap((*label).to_string(), font.clone(), Color32::WHITE)
                .size()
                .x
                + 24.0
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 30.0), Sense::hover());
    ui.painter().rect_filled(rect, t.radius + 2, t.input);
    ui.painter().rect_stroke(
        rect,
        t.radius + 2,
        Stroke::new(1.0, t.input_border),
        egui::StrokeKind::Inside,
    );
    let mut x = rect.left() + 2.0;
    for (i, ((option, label), w)) in options.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(egui::pos2(x, rect.top() + 2.0), vec2(*w, rect.height() - 4.0));
        let resp = ui.interact(r, ui.id().with((id, i)), Sense::click());
        let on = *value == *option;
        if on {
            ui.painter().rect_filled(r, t.radius, t.selection);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, t.radius, t.hover);
        }
        let (color, f) = if on {
            (t.accent_text, semibold(13.0))
        } else {
            (t.text, font.clone())
        };
        ui.painter()
            .text(r.center(), egui::Align2::CENTER_CENTER, *label, f, color);
        if resp.clicked() {
            *value = *option;
        }
        x += w;
    }
}

/// Il titolo di una linguetta, con lo spazio per la maniglia.
fn tab_text(label: &str) -> String {
    format!("{TAB_GRIP_SPACE}{label}")
}
const TAB_GRIP_SPACE: &str = "     ";

/// Le linguette di un gruppo di pannelli, ridisegnate sopra quelle di craft-ui: una maniglia a
/// puntini che dice che si possono trascinare, la mano al passaggio del mouse, e la linguetta
/// attiva unita alla scheda con una barra blu in alto. Le posizioni seguono le regole di craft-ui
/// (larghezza del titolo + 24 punti, da sinistra; 12 punti di presa nelle finestre mobili): se le
/// linguette non stanno tutte, craft-ui ne nasconde alcune e qui non si disegna nulla.
fn paint_tabs(ui: &egui::Ui, panels: &[PanelId], active: usize, floating: bool, body: Rect) {
    let t = Tokens::get(ui.ctx());
    let style = dock::style(ui);
    let strip = Rect::from_min_max(
        egui::pos2(body.left(), body.top() - style.tab_height),
        egui::pos2(body.right(), body.top()),
    );
    let mut x = strip.left() + if floating { 12.0 } else { 0.0 };
    let widths: Vec<f32> = panels
        .iter()
        .map(|p| {
            ui.painter()
                .layout_no_wrap(tab_text(p.label()), style.font.clone(), Color32::WHITE)
                .size()
                .x
                + 24.0
        })
        .collect();
    if x + widths.iter().sum::<f32>() > strip.right() {
        return;
    }
    let painter = ui.ctx().layer_painter(ui.layer_id());
    let pointer = ui.ctx().pointer_hover_pos();
    for (i, (panel, w)) in panels.iter().zip(&widths).enumerate() {
        let tab = Rect::from_min_size(egui::pos2(x, strip.top()), vec2(*w, strip.height()));
        x += w;
        let hovered = pointer.is_some_and(|p| tab.contains(p));
        if i == active {
            painter.rect_filled(
                tab,
                egui::CornerRadius {
                    nw: t.radius,
                    ne: t.radius,
                    sw: 0,
                    se: 0,
                },
                t.panel,
            );
            painter.rect_filled(
                Rect::from_min_size(tab.min + vec2(6.0, 0.0), vec2(tab.width() - 12.0, 2.5)),
                1.0,
                t.accent,
            );
            let label = painter.layout_no_wrap(tab_text(panel.label()), style.font.clone(), t.text_strong);
            painter.galley(tab.center() - label.size() * 0.5, label, t.text_strong);
        } else if hovered {
            painter.rect_filled(tab.shrink2(vec2(2.0, 4.0)), t.radius, t.hover);
            let label = painter.layout_no_wrap(tab_text(panel.label()), style.font.clone(), t.text);
            painter.galley(tab.center() - label.size() * 0.5, label, t.text);
        }
        // Maniglia: sei puntini a sinistra del titolo.
        let grip = egui::pos2(tab.left() + 15.0, tab.center().y);
        let color = if i == active || hovered {
            t.text_dim
        } else {
            t.text_disabled
        };
        for dx in [0.0, 4.0] {
            for dy in [-4.0, 0.0, 4.0] {
                painter.circle_filled(grip + vec2(dx, dy), 1.2, color);
            }
        }
        if hovered && *panel != PanelId::Canvas {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
}

/// Collega i pannelli agganciabili al loro contenuto.
struct Content<'a> {
    app: &'a mut WinFilesApp,
    sheets: &'a Result<Vec<Sheet>, LayoutError>,
}

impl DockContent<PanelId> for Content<'_> {
    /// Spazio a sinistra del titolo per la maniglia di trascinamento (vedi `paint_tabs`).
    fn label(
        &mut self,
        _ui: &egui::Ui,
        _panel: &PanelId,
        logical: &str,
        _font: &egui::FontId,
        _color: Color32,
    ) -> egui::WidgetText {
        tab_text(logical).into()
    }

    fn body(&mut self, ui: &mut egui::Ui, panel: &PanelId) {
        if *panel == PanelId::Canvas {
            self.app.canvas(ui, self.sheets);
            return;
        }
        let body = ui.max_rect();
        let t = Tokens::get(ui.ctx());
        // Il fondo della scheda: l'angolo in alto a sinistra è dritto perché ci si appoggia la
        // linguetta del primo pannello.
        ui.painter().rect_filled(
            body,
            egui::CornerRadius {
                nw: 0,
                ne: t.card_radius,
                sw: t.card_radius,
                se: t.card_radius,
            },
            t.panel,
        );
        if let Some((panels, active, floating)) = dock::group_of(&self.app.dock, *panel) {
            paint_tabs(ui, &panels, active, floating, body);
        }
        egui::Frame::NONE
            .inner_margin(egui::Margin::same(14))
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

        self.slots = captions::slots(&self.project);
        let print = &self.project.print;
        let width = winfiles_geom::sheet_size(print.paper, print.orientation).w
            - print.margins.left
            - print.margins.right;
        let title = captions::title_height(ui.ctx(), &print.title, width);
        let sheets = grid_with_title(
            print,
            &captions::items(ui.ctx(), &self.project, &self.slots),
            title,
        );
        self.app_bar(ui);
        self.control_bar(ui);
        self.status_bar(ui);
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE.fill(t.app_bar).inner_margin(egui::Margin {
            left: 10,
            right: 10,
            top: 0,
            bottom: 0,
        });
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
                        min: vec2(320.0, 240.0),
                        ..Default::default()
                    },
                    _ => PanelLimits {
                        min: vec2(220.0, 110.0),
                        ..Default::default()
                    },
                },
                &mut Content {
                    app: self,
                    sheets: &sheets,
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
