//! Stato dell'applicazione e cornice della finestra: barra dei menu, barra di stampa, pannelli
//! agganciabili su schede arrotondate e barra di stato.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use craft_ui::docking::{Action, DockArea, DockContent, Layout, PanelLimits};
use egui::{Align, Color32, Rect, Sense, Stroke, Vec2, vec2};
use winfiles_geom::{Orientation, Paper, Unit};
use winfiles_layout::{LayoutError, MAX_COLUMNS, Sheet, grid_with_title};
use winfiles_model::{Project, Totals, View};

use crate::captions::{self, Slot};
use crate::dock::{self, PanelId};
use crate::document::{self, Then};
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
    /// Il file di cui le proprietà e le note mostrano i dati: l'ultimo scelto.
    pub(crate) selected: Option<usize>,
    /// Tutti i file scelti (Ctrl e Maiusc aggiungono), per eliminarli insieme.
    pub(crate) marked: BTreeSet<usize>,
    /// La pagina del file scelto di cui il pannello Note mostra la nota.
    pub(crate) selected_page: usize,
    /// Il file da cui parte una scelta con Maiusc.
    pub(crate) anchor: Option<usize>,
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
    pub(crate) measurer: Measurer,
    pub(crate) thumbs: Thumbnails,
    /// Le miniature sul foglio, una per pagina, ricalcolate a ogni fotogramma.
    slots: Vec<Slot>,
    /// Cartella di cui si sta rinominando l'intestazione nell'elenco, con il testo in modifica.
    pub(crate) renaming: Option<(PathBuf, String)>,
    /// Cosa mostra l'elenco dei file: cerca per nome e filtra per tipo.
    pub(crate) filter: crate::filter::FileFilter,
    /// Ctrl+F: il campo di ricerca prende il fuoco al prossimo disegno.
    pub(crate) focus_search: bool,
    pub(crate) status: String,
    /// I lavori aperti di recente, il più recente per primo.
    pub(crate) recent: Vec<PathBuf>,
    /// Ricerche di file in corso.
    scans: Vec<scan::Scan>,
    /// L'esportazione in PDF in corso.
    export: Option<crate::export::Export>,
    /// Esportare al prossimo fotogramma, quando i fogli sono pronti.
    want_export: bool,
    /// Aprire la finestra Stampa al prossimo fotogramma.
    want_print: bool,
    /// La finestra Stampa aperta.
    print_dialog: Option<crate::printing::PrintDialog>,
    /// Le scelte dell'ultima stampa, per la prossima.
    last_print: crate::printing::PrintDialog,
    /// La rotella sull'anteprima: il foglio era già al bordo alto/basso nel fotogramma prima, e
    /// quando ha cambiato foglio l'ultima volta.
    wheel_edges: (bool, bool),
    last_flip: f64,
    /// Dove portare lo scorrimento del foglio appena cambiato con la rotella.
    flip_to: Option<Edge>,
    /// `--pdf` sulla riga di comando: esportare qui appena i file sono letti, poi chiudere.
    batch_pdf: Option<PathBuf>,
    /// L'esportazione in corso è quella di `--pdf`.
    batch_running: bool,
    /// Per svegliare l'interfaccia dai thread.
    ctx: egui::Context,
    /// Il file del lavoro, se è stato salvato o aperto.
    pub(crate) path: Option<PathBuf>,
    /// Il lavoro com'era all'ultimo salvataggio, per sapere se ci sono modifiche.
    pub(crate) saved: Project,
    /// La domanda "salvare le modifiche?" aperta, con cosa fare dopo.
    pub(crate) asking: Option<Then>,
    /// La chiusura è già stata confermata.
    pub(crate) closing: bool,
    /// L'ultimo titolo dato alla finestra.
    shown_title: String,
    /// Le ultime scelte scritte in `impostazioni.json`, e quando si è controllato l'ultima volta.
    prefs: crate::prefs::Prefs,
    prefs_checked: f64,
    /// L'ultima impaginazione e il lavoro da cui viene: si rifà solo quando il lavoro cambia.
    layout_cache: Option<(Project, std::rc::Rc<Pages>)>,
    /// Nel nesting: la scala 1:N in uso (scelta o automatica).
    pub(crate) nesting_scale: Option<u32>,
}

impl WinFilesApp {
    pub fn new(
        ctx: &egui::Context,
        paths: Vec<PathBuf>,
        batch_pdf: Option<PathBuf>,
        prefs: crate::prefs::Prefs,
    ) -> Self {
        let blank = blank_project(&prefs);
        let last_print = crate::printing::PrintDialog {
            printer: prefs.printing.printer.clone(),
            duplex: prefs.printing.sides.duplex(),
            gray: prefs.printing.gray,
            fit: prefs.printing.fit,
            ..crate::printing::PrintDialog::default()
        };
        let wake = ctx.clone();
        let wake2 = ctx.clone();
        let mut app = Self {
            project: blank.clone(),
            saved: blank,
            path: None,
            asking: None,
            closing: false,
            shown_title: String::new(),
            selected: None,
            marked: BTreeSet::new(),
            selected_page: 0,
            anchor: None,
            sheet: 0,
            zoom: None,
            shown_zoom: 1.0,
            errors: HashMap::new(),
            theme: prefs.theme,
            applied: None,
            dock: prefs.dock.clone().unwrap_or_else(dock::default_layout),
            measurer: Measurer::start(move || wake.request_repaint()),
            thumbs: Thumbnails::start(move || wake2.request_repaint()),
            slots: Vec::new(),
            renaming: None,
            filter: Default::default(),
            focus_search: false,
            status: String::new(),
            scans: Vec::new(),
            export: None,
            want_export: false,
            want_print: false,
            print_dialog: None,
            last_print,
            batch_pdf,
            batch_running: false,
            wheel_edges: (true, true),
            last_flip: 0.0,
            flip_to: None,
            recent: crate::recent::load(),
            ctx: ctx.clone(),
            prefs,
            prefs_checked: 0.0,
            nesting_scale: None,
            layout_cache: None,
        };
        // Un file .winfiles sulla riga di comando (doppio clic in Esplora file) apre il lavoro;
        // gli altri file e cartelle si aggiungono all'elenco.
        let (projects, files): (Vec<PathBuf>, Vec<PathBuf>) =
            paths.into_iter().partition(|p| document::is_project(p));
        if let Some(p) = projects.first() {
            app.open_path(p);
        }
        if !files.is_empty() {
            app.add_paths(files);
        }
        app
    }

    /// Chiede dove salvare e scrive tutti i fogli in un PDF, in un thread a parte; la fine arriva
    /// con [`Self::receive_export`].
    fn export_pdf(&mut self, ctx: &egui::Context, sheets: &Pages, to: Option<PathBuf>) {
        if self.export.is_some() {
            return;
        }
        if self.project.files.is_empty() {
            self.status = "Non ci sono file da esportare".into();
            return;
        }
        // Con file ancora da misurare il PDF avrebbe miniature provvisorie.
        if !self.scans.is_empty() || self.measurer.pending() > 0 {
            self.status = "Aspetta che finisca la lettura dei file, poi esporta".into();
            return;
        }
        let sheets = match sheets {
            Ok(s) => s,
            Err(e) => {
                self.status = format!("Non riesco a impaginare: {e}");
                return;
            }
        };
        let mut dialog = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_title("Esporta i fogli in PDF")
            .set_file_name(format!("{}.pdf", self.document_name()));
        if let Some(dir) = self.path.as_deref().and_then(std::path::Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        let Some(mut path) = to.or_else(|| dialog.save_file()) else {
            return;
        };
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            path.as_mut_os_string().push(".pdf");
        }
        let pages = self.compose(ctx, sheets, 0..=sheets.len().saturating_sub(1));
        self.start_job(pages, crate::export::Target::File(path));
    }

    /// Le scene dei fogli `range` (da 0): prima il riepilogo, poi le miniature.
    fn compose(
        &self,
        ctx: &egui::Context,
        sheets: &Sheets,
        range: std::ops::RangeInclusive<usize>,
    ) -> Vec<winfiles_scene::Page> {
        let summary_sheets = sheets.summary.as_ref().map_or(0, |s| s.sheets.len());
        range
            .filter_map(|i| match &sheets.summary {
                Some(summary) if i < summary_sheets => summary
                    .sheets
                    .get(i)
                    .map(|s| crate::summary::compose(ctx, s, summary, &self.project)),
                _ => sheets
                    .grid
                    .get(i - summary_sheets)
                    .map(|s| canvas::compose(ctx, s, &self.project, &self.slots)),
            })
            .collect()
    }

    fn start_job(&mut self, pages: Vec<winfiles_scene::Page>, target: crate::export::Target) {
        let wake = self.ctx.clone();
        match crate::export::Export::start(pages, target, move || wake.request_repaint()) {
            Ok(job) => self.export = Some(job),
            Err(e) => self.status = format!("Non riesco ad avviare il lavoro: {e}"),
        }
    }

    /// Apre la finestra Stampa, se si può stampare.
    fn open_print(&mut self) {
        if self.export.is_some() || self.print_dialog.is_some() {
            return;
        }
        if self.project.files.is_empty() {
            self.status = "Non ci sono file da stampare".into();
            return;
        }
        if !self.scans.is_empty() || self.measurer.pending() > 0 {
            self.status = "Aspetta che finisca la lettura dei file, poi stampa".into();
            return;
        }
        let mut dialog = self.last_print.clone();
        dialog.refresh();
        self.print_dialog = Some(dialog);
    }

    /// La finestra Stampa aperta; alla conferma i fogli scelti vanno alla stampante.
    fn print_window(&mut self, ctx: &egui::Context, sheets: &Pages) {
        let count = sheets.as_ref().map_or(0, Sheets::len);
        let title = self.document_name();
        let current = self.sheet;
        let Some(dialog) = self.print_dialog.as_mut() else {
            return;
        };
        let Some(answer) = dialog.show(ctx, &title, count, current) else {
            return;
        };
        if let Some(d) = self.print_dialog.take() {
            self.last_print = d;
        }
        let crate::printing::Answer::Print(job, range) = answer else {
            return;
        };
        let Ok(sheets) = sheets else {
            return;
        };
        let pages = self.compose(ctx, sheets, range);
        self.start_job(pages, crate::export::Target::Printer(job));
    }

    /// A esportazione finita: il messaggio nella barra di stato e il PDF aperto.
    fn receive_export(&mut self) {
        let Some(job) = &self.export else {
            return;
        };
        let Some(result) = job.poll() else {
            return;
        };
        let path = match &job.target {
            crate::export::Target::File(path) => path.clone(),
            crate::export::Target::Printer(print) => {
                self.status = crate::export::print_outcome(print, job.total, &result);
                self.export = None;
                return;
            }
        };
        self.status = crate::export::outcome(&path, &result);
        if self.batch_running {
            eprintln!("{}", self.status);
            self.export = None;
            self.closing = true;
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if result.is_ok()
            && let Err(e) = crate::shell::open(&path)
        {
            self.status = format!("{}; non riesco ad aprirlo: {e}", self.status);
        }
        self.export = None;
    }

    /// Cerca i file nei percorsi dati in un thread a parte; arrivano con [`Self::receive_scans`].
    fn add_paths(&mut self, paths: Vec<PathBuf>) {
        let wake = self.ctx.clone();
        match scan::Scan::start(paths.clone(), move || wake.request_repaint()) {
            Ok(s) => self.scans.push(s),
            // Senza thread si cerca qui, bloccando un momento la finestra.
            Err(_) => self.merge(scan::collect(&paths), false, false),
        }
    }

    /// Porta nel lavoro i file delle ricerche finite.
    fn receive_scans(&mut self) {
        let mut done = Vec::new();
        self.scans.retain(|s| match s.poll() {
            Some(found) => {
                done.push(found);
                false
            }
            None => true,
        });
        for found in done {
            self.merge(found.files, found.cancelled, found.truncated);
        }
    }

    /// Ferma le ricerche in corso: i loro risultati non arriveranno.
    pub(crate) fn cancel_scans(&mut self) {
        for s in self.scans.drain(..) {
            s.cancel();
        }
    }

    fn merge(&mut self, found: Vec<winfiles_model::FileEntry>, cancelled: bool, truncated: bool) {
        let before = self.project.files.len();
        let mut files = std::mem::take(&mut self.project.files);
        files.extend(found);
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
        self.unselect();
        self.status = match added {
            0 => "Nessun file nuovo: immagini, PDF, AI ed EPS".into(),
            1 => "Aggiunto 1 file".into(),
            n => format!("Aggiunti {n} file"),
        };
        if cancelled {
            self.status = "Ricerca annullata: nessun file aggiunto".into();
        } else if truncated {
            self.status = format!(
                "{}: fermata a {} file, aggiungi le cartelle una alla volta",
                self.status,
                scan::MAX_FILES
            );
        }
    }

    fn clear(&mut self) {
        self.cancel_scans();
        self.project.files.clear();
        self.errors.clear();
        self.thumbs.clear();
        self.unselect();
        self.sheet = 0;
        self.status = "Elenco svuotato".into();
    }

    pub(crate) fn unselect(&mut self) {
        self.selected = None;
        self.marked.clear();
        self.anchor = None;
    }

    /// Ctrl+N, Ctrl+O, Ctrl+S e Ctrl+Maiusc+S, come in ogni programma; Ctrl+F cerca nell'elenco.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let pressed = |mods, key| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(mods, key)));
        if pressed(Modifiers::COMMAND | Modifiers::SHIFT, Key::S) {
            self.save(true);
        } else if pressed(Modifiers::COMMAND, Key::S) {
            self.save(false);
        } else if pressed(Modifiers::COMMAND, Key::E) {
            self.want_export = true;
        } else if pressed(Modifiers::COMMAND, Key::P) {
            self.want_print = true;
        } else if pressed(Modifiers::COMMAND, Key::O) {
            self.guard(ctx, Then::Open(None));
        } else if pressed(Modifiers::COMMAND, Key::N) {
            self.guard(ctx, Then::New);
        } else if pressed(Modifiers::COMMAND, Key::F) {
            self.focus_search = true;
        }
    }

    /// Sceglie un file con un clic: da solo, oppure con Ctrl lo aggiunge o lo toglie e con Maiusc
    /// prende tutti quelli tra il primo scelto e questo, come in Esplora file.
    pub(crate) fn pick(&mut self, i: usize, mods: egui::Modifiers) {
        self.selected_page = 0;
        if mods.shift
            && let Some(a) = self.anchor
        {
            if !mods.command {
                self.marked.clear();
            }
            self.marked.extend(a.min(i)..=a.max(i));
            self.selected = Some(i);
        } else if mods.command {
            if !self.marked.remove(&i) {
                self.marked.insert(i);
            }
            self.selected = self
                .marked
                .contains(&i)
                .then_some(i)
                .or_else(|| self.marked.last().copied());
            self.anchor = Some(i);
        } else {
            self.marked.clear();
            self.marked.insert(i);
            self.selected = Some(i);
            self.anchor = Some(i);
        }
    }

    /// Il tasto destro su un file non scelto lo sceglie da solo; su uno già scelto tiene la scelta.
    pub(crate) fn pick_for_menu(&mut self, i: usize) {
        if !self.marked.contains(&i) {
            self.pick(i, egui::Modifiers::NONE);
        }
    }

    /// Mette in stampa (o ne toglie) tutte le pagine dei file indicati: restano nell'elenco con
    /// le loro note.
    pub(crate) fn set_printed(&mut self, files: &BTreeSet<usize>, on: bool) {
        for i in files {
            if let Some(f) = self.project.files.get_mut(*i) {
                f.set_all_printed(on);
            }
        }
    }

    /// Spazio sui file scelti: se ce n'è uno in stampa li toglie tutti, altrimenti li rimette.
    pub(crate) fn toggle_marked_print(&mut self) {
        let any_on = self
            .marked
            .iter()
            .filter_map(|i| self.project.files.get(*i))
            .any(|f| f.printed_pages().next().is_some());
        let marked = self.marked.clone();
        self.set_printed(&marked, !any_on);
    }

    /// Toglie dall'elenco i file indicati (solo dal lavoro, mai dal disco).
    pub(crate) fn remove_files(&mut self, remove: &BTreeSet<usize>) {
        if remove.is_empty() {
            return;
        }
        let before = self.project.files.len();
        let mut i = 0;
        self.project.files.retain(|_| {
            let keep = !remove.contains(&i);
            i += 1;
            keep
        });
        let gone = before - self.project.files.len();
        self.unselect();
        self.status = match gone {
            1 => "Tolto 1 file dall'elenco".into(),
            n => format!("Tolti {n} file dall'elenco"),
        };
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
                            let ctx = ui.ctx().clone();
                            let item = |ui: &mut egui::Ui, text: &str, keys: &str| {
                                ui.add(egui::Button::new(text).shortcut_text(keys)).clicked()
                            };
                            if item(ui, "Nuovo lavoro", "Ctrl+N") {
                                ui.close();
                                self.guard(&ctx, Then::New);
                            }
                            if item(ui, "Apri…", "Ctrl+O") {
                                ui.close();
                                self.guard(&ctx, Then::Open(None));
                            }
                            let recent = self.recent.clone();
                            ui.add_enabled_ui(!recent.is_empty(), |ui| {
                                ui.menu_button("Lavori recenti", |ui| {
                                    for path in &recent {
                                        let name = path
                                            .file_stem()
                                            .map(|n| n.to_string_lossy().into_owned())
                                            .unwrap_or_default();
                                        let there = path.is_file();
                                        let resp = ui
                                            .add_enabled(there, egui::Button::new(name))
                                            .on_hover_text(path.display().to_string())
                                            .on_disabled_hover_text(format!(
                                                "{}\nNon c'è più: è stato spostato o cancellato",
                                                path.display()
                                            ));
                                        if resp.clicked() {
                                            ui.close();
                                            self.guard(&ctx, Then::Open(Some(path.clone())));
                                        }
                                    }
                                    ui.separator();
                                    if ui.button("Svuota l'elenco").clicked() {
                                        crate::recent::forget(&mut self.recent);
                                        ui.close();
                                    }
                                });
                            });
                            ui.separator();
                            if item(ui, "Salva", "Ctrl+S") {
                                ui.close();
                                self.save(false);
                            }
                            if item(ui, "Salva con nome…", "Ctrl+Maiusc+S") {
                                ui.close();
                                self.save(true);
                            }
                            ui.separator();
                            let can_export = !self.project.files.is_empty() && self.export.is_none();
                            if ui
                                .add_enabled(
                                    can_export,
                                    egui::Button::new("Esporta PDF…").shortcut_text("Ctrl+E"),
                                )
                                .clicked()
                            {
                                ui.close();
                                self.want_export = true;
                            }
                            if ui
                                .add_enabled(can_export, egui::Button::new("Stampa…").shortcut_text("Ctrl+P"))
                                .clicked()
                            {
                                ui.close();
                                self.want_print = true;
                            }
                            ui.separator();
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
                    dim_label(ui, "Vista");
                    segmented(
                        ui,
                        "vista",
                        &mut print.view,
                        &[
                            (View::Grid, "Griglia"),
                            (View::List, "Elenco"),
                            (View::Nesting, "Nesting"),
                        ],
                    );
                    ui.add_space(12.0);
                    // Il controllo che conta nella vista scelta: colonne, miniatura o scala.
                    match print.view {
                        View::Grid => {
                            dim_label(ui, "Colonne");
                            stepper(ui, &mut print.columns, 1, MAX_COLUMNS);
                        }
                        View::List => {
                            dim_label(ui, "Miniatura");
                            thumb_stepper(ui, &mut print.list_thumb);
                        }
                        View::Nesting => {
                            dim_label(ui, "Scala");
                            scale_stepper(ui, &mut print.nesting_scale, self.nesting_scale);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        let enabled = !self.project.files.is_empty() && self.export.is_none();
                        // Stampa: il pulsante principale dell'app, grande e blu.
                        let (fill, ink) = if enabled {
                            (t.primary, t.on_primary)
                        } else {
                            (t.button, t.text_disabled)
                        };
                        let print = egui::Button::image_and_text(
                            icons::image("printer", 18.0, ink),
                            egui::RichText::new("Stampa").font(semibold(14.0)).color(ink),
                        )
                        .fill(fill)
                        .min_size(vec2(118.0, 34.0));
                        if ui
                            .add_enabled(enabled, print)
                            .on_hover_text("Stampa i fogli alla misura vera (Ctrl+P)")
                            .clicked()
                        {
                            self.want_print = true;
                        }
                        ui.add_space(4.0);
                        let tint = if enabled { t.text_strong } else { t.text_disabled };
                        let button = egui::Button::image_and_text(
                            icons::image("file-down", 17.0, tint),
                            egui::RichText::new("Esporta PDF").color(tint),
                        )
                        .min_size(vec2(0.0, 32.0));
                        if ui
                            .add_enabled(enabled, button)
                            .on_hover_text("Salva tutti i fogli in un PDF, alla misura vera (Ctrl+E)")
                            .clicked()
                        {
                            self.want_export = true;
                        }
                    });
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
                    let mut text = format!(
                        "{} file  ·  {} pagine  ·  {}",
                        totals.files,
                        totals.pages,
                        crate::panels::areas(&totals)
                    );
                    if totals.excluded > 0 {
                        text.push_str(&format!("  ·  {} fuori stampa", totals.excluded));
                    }
                    ui.label(egui::RichText::new(text).font(semibold(12.5)).color(t.text));
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        let pending = self.measurer.pending();
                        if let Some(job) = &self.export {
                            if job.can_cancel() && ui.button("Annulla").clicked() {
                                job.cancel();
                            }
                            let text = job.progress_text();
                            ui.label(egui::RichText::new(text).size(12.0));
                            ui.add(egui::Spinner::new().size(13.0).color(t.accent));
                        } else if !self.scans.is_empty() {
                            let found: usize = self.scans.iter().map(scan::Scan::found).sum();
                            if ui.button("Annulla").clicked() {
                                for s in &self.scans {
                                    s.cancel();
                                }
                            }
                            ui.label(
                                egui::RichText::new(format!("Cerco i file… {found} trovati")).size(12.0),
                            );
                            ui.add(egui::Spinner::new().size(13.0).color(t.accent));
                            // Il conteggio sale mentre il thread lavora.
                            ui.ctx()
                                .request_repaint_after(std::time::Duration::from_millis(150));
                        } else if pending > 0 {
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
    pub(crate) fn canvas(&mut self, ui: &mut egui::Ui, pages: &Pages) {
        let t = Tokens::get(ui.ctx());
        let full = ui.max_rect();
        card(ui, full);
        let count = pages.as_ref().map_or(0, Sheets::len);
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
        // Nel nesting la scala comune, come si scrive in tipografia: 1:20.
        let scale = self
            .nesting_scale
            .map(|n| format!(" · scala 1:{n}"))
            .unwrap_or_default();
        bar.label(
            egui::RichText::new(format!("{} {orientation}{scale}", print.paper.name())).color(t.text_dim),
        );
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
        let pages = match pages {
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
        let summary_sheets = pages.summary.as_ref().map_or(0, |s| s.sheets.len());
        // I fogli di riepilogo vengono prima di quelli delle miniature.
        if let Some(summary) = &pages.summary
            && let Some(sheet) = summary.sheets.get(self.sheet)
        {
            let scale = self
                .zoom
                .unwrap_or_else(|| canvas::fit_zoom(sheet.size, board.size()));
            self.shown_zoom = scale;
            let scene = crate::summary::compose(ui.ctx(), sheet, summary, &self.project);
            let offset = self.flip_offset(ui, sheet.size, scale);
            let scrolled = scroll_area(offset).show(ui, |ui| {
                crate::summary::draw(ui, sheet, &scene, summary, &mut self.thumbs, &self.marked, scale)
            });
            self.wheel(ui, board, count, &scrolled);
            self.zoom_with_wheel(ui, board, scale);
            if let Some(file) = scrolled.inner {
                let mods = ui.input(|i| i.modifiers);
                self.pick(file, mods);
            }
            return;
        }
        let Some(sheet) = pages.grid.get(self.sheet.saturating_sub(summary_sheets)) else {
            return;
        };
        let scale = self
            .zoom
            .unwrap_or_else(|| canvas::fit_zoom(sheet.size, board.size()));
        self.shown_zoom = scale;
        let scene = canvas::compose(ui.ctx(), sheet, &self.project, &self.slots);
        let offset = self.flip_offset(ui, sheet.size, scale);
        let scrolled = scroll_area(offset).show(ui, |ui| {
            canvas::draw(
                ui,
                sheet,
                &scene,
                &self.slots,
                &mut self.thumbs,
                canvas::Selection {
                    marked: &self.marked,
                    focus: self.selected.map(|f| (f, self.selected_page)),
                },
                scale,
            )
        });
        self.wheel(ui, board, count, &scrolled);
        self.zoom_with_wheel(ui, board, scale);
        let out = scrolled.inner;
        if let Some((file, page)) = out.clicked {
            let mods = ui.input(|i| i.modifiers);
            self.pick(file, mods);
            self.selected_page = page;
        }
    }
}

/// Ctrl + rotella (o il pizzico sul touchpad) con il puntatore sull'anteprima: il fattore di
/// ingrandimento chiesto, uguale per ogni foglio, miniature o riepilogo.
pub(crate) fn wheel_zoom(ui: &egui::Ui, board: Rect) -> Option<f32> {
    let zoom = ui.input(|i| i.zoom_delta());
    (zoom != 1.0 && ui.rect_contains_pointer(board)).then_some(zoom)
}

impl WinFilesApp {
    fn zoom_with_wheel(&mut self, ui: &egui::Ui, board: Rect, scale: f32) {
        if let Some(by) = wheel_zoom(ui, board) {
            self.zoom = Some((scale * by).clamp(canvas::MIN_ZOOM, canvas::MAX_ZOOM));
        }
    }

    /// La rotella sull'anteprima scorre il foglio e, arrivati al bordo, passa al foglio dopo o a
    /// quello prima. Serve una girata di rotella in più quando si arriva al bordo, così chi
    /// scorre un foglio ingrandito non salta al successivo senza volerlo.
    fn wheel<R>(
        &mut self,
        ui: &egui::Ui,
        board: Rect,
        count: usize,
        out: &egui::scroll_area::ScrollAreaOutput<R>,
    ) {
        let (offset, view, content) = (out.state.offset.y, out.inner_rect.height(), out.content_size.y);
        // Qualche punto di tolleranza: il foglio intero sporge appena dalla finestra (l'ombra e il
        // margine), e quei pochi punti non devono costare una girata di rotella.
        const SLACK: f32 = 16.0;
        let edges = (offset <= SLACK, offset + view >= content - SLACK);
        let (was_top, was_bottom) = std::mem::replace(&mut self.wheel_edges, edges);
        let (delta, ctrl, now) = ui.input(|i| {
            let delta: f32 = i
                .raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                    _ => None,
                })
                .sum();
            (delta, i.modifiers.command, i.time)
        });
        if delta == 0.0 || ctrl || !ui.rect_contains_pointer(board) || now - self.last_flip < 0.35 {
            return;
        }
        // Rotella in giù: delta negativo.
        if delta < 0.0 && was_bottom && self.sheet + 1 < count {
            self.sheet += 1;
            self.flip_to = Some(Edge::Top);
        } else if delta > 0.0 && was_top && self.sheet > 0 {
            self.sheet -= 1;
            self.flip_to = Some(Edge::Bottom);
        } else {
            return;
        }
        self.last_flip = now;
        self.wheel_edges = (true, true);
        ui.ctx().request_repaint();
    }
}

/// Il bordo del foglio da mostrare dopo un cambio di foglio con la rotella.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Top,
    Bottom,
}

impl WinFilesApp {
    /// Lo scorrimento che mostra il bordo chiesto del foglio appena cambiato con la rotella,
    /// calcolato dall'altezza vera del foglio: un valore enorme lascerebbe la vista fuori dal
    /// foglio e lo scorrimento animato non finirebbe mai.
    fn flip_offset(&mut self, ui: &egui::Ui, size: winfiles_geom::Size, scale: f32) -> Option<f32> {
        let view = ui.available_height();
        Some(match self.flip_to.take()? {
            Edge::Top => 0.0,
            Edge::Bottom => (canvas::content_height(size, scale, view) - view).max(0.0),
        })
    }
}

/// Lo spazio in cui scorre il foglio, portato a `offset` dopo un cambio di foglio con la rotella.
fn scroll_area(offset: Option<f32>) -> egui::ScrollArea {
    let area = egui::ScrollArea::both().auto_shrink(false).id_salt("foglio");
    match offset {
        Some(y) => area.vertical_scroll_offset(y),
        None => area,
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
    let delta = stepper_with(ui, "colonne", &value.to_string(), *value > min, *value < max);
    *value = u32::try_from((i64::from(*value) + i64::from(delta)).clamp(i64::from(min), i64::from(max)))
        .unwrap_or(min);
}

/// La larghezza delle miniature dell'elenco, a passi di 5 mm.
pub(crate) fn thumb_stepper(ui: &mut egui::Ui, mm: &mut f64) {
    let (lo, hi) = (
        *winfiles_layout::LIST_THUMB_MM.start(),
        *winfiles_layout::LIST_THUMB_MM.end(),
    );
    let v = if mm.is_finite() { mm.clamp(lo, hi) } else { lo };
    let text = format!("{} mm", winfiles_geom::format_number(v, 0, true));
    let delta = stepper_with(ui, "miniatura", &text, v > lo, v < hi);
    if delta != 0 {
        // Si torna su un multiplo di 5, poi si sale o si scende di 5.
        let base = (v / 5.0).round() * 5.0;
        *mm = (base + 5.0 * f64::from(delta)).clamp(lo, hi);
    }
}

/// Scale "tonde" per il nesting, come 1:N.
const SCALES: [u32; 30] = [
    1, 2, 3, 4, 5, 6, 8, 10, 12, 15, 20, 25, 30, 40, 50, 60, 75, 100, 125, 150, 200, 250, 300, 400, 500, 750,
    1000, 2000, 5000, 10_000,
];

/// La scala del nesting: i pulsanti passano alla scala tonda vicina (− ingrandisce, + rimpicciolisce)
/// e la fissano; "Auto" torna alla scala automatica. `shown` è la scala in uso adesso.
pub(crate) fn scale_stepper(ui: &mut egui::Ui, scale: &mut u32, shown: Option<u32>) {
    let current = if *scale > 0 { *scale } else { shown.unwrap_or(1) };
    let text = format!("1:{current}");
    let delta = stepper_with(ui, "scala", &text, current > 1, current < 10_000);
    if delta < 0 {
        *scale = SCALES.iter().rev().copied().find(|&n| n < current).unwrap_or(1);
    } else if delta > 0 {
        *scale = SCALES.iter().copied().find(|&n| n > current).unwrap_or(10_000);
    }
    let mut auto = *scale == 0;
    if ui
        .checkbox(&mut auto, "Auto")
        .on_hover_text("Il file più grande largo un terzo del foglio")
        .changed()
    {
        *scale = if auto { 0 } else { current };
    }
}

/// Un campo con − e + ai lati e `text` al centro; dice di quanto cambiare (−1, 0 o +1).
fn stepper_with(ui: &mut egui::Ui, id: &str, text: &str, can_dec: bool, can_inc: bool) -> i32 {
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
    let mut out = 0;
    for (r, icon, enabled, delta) in [(minus, "minus", can_dec, -1), (plus, "plus", can_inc, 1)] {
        let resp = ui.interact(
            r,
            ui.id().with(("stepper", id, icon)),
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
            out = delta;
        }
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        semibold(13.5),
        t.text_strong,
    );
    out
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

impl WinFilesApp {
    /// I fogli del lavoro. Si impaginano di nuovo solo quando il lavoro cambia (file, misure,
    /// note, impostazioni): con migliaia di file misurare tutti i testi a ogni fotogramma
    /// rallenterebbe l'anteprima.
    fn pages(&mut self, ctx: &egui::Context) -> std::rc::Rc<Pages> {
        if let Some((key, pages)) = &self.layout_cache
            && *key == self.project
        {
            return pages.clone();
        }
        self.slots = captions::slots(&self.project);
        // L'altezza delle intestazioni segue il corpo del loro testo.
        let mut print = self.project.print.clone();
        print.header = captions::header_height(print.header_text);
        let width = winfiles_geom::sheet_size(print.paper, print.orientation).w
            - print.margins.left
            - print.margins.right;
        let title = captions::title_height(ctx, &print.title, print.title_text, width);
        let items = captions::items(ctx, &self.project, &self.slots);
        self.nesting_scale = (print.view == View::Nesting && !items.is_empty())
            .then(|| winfiles_layout::nesting_denominator(&print, &items));
        let pages = grid_with_title(&print, &items, title).and_then(|grid| {
            let summary = if print.show_summary && !self.project.files.is_empty() {
                Some(crate::summary::layout(ctx, &self.project, title)?)
            } else {
                None
            };
            Ok(Sheets { summary, grid })
        });
        let pages = std::rc::Rc::new(pages);
        self.layout_cache = Some((self.project.clone(), pages.clone()));
        pages
    }
}

/// Tutti i fogli del lavoro: il riepilogo (se richiesto) e poi le miniature.
pub(crate) struct Sheets {
    pub summary: Option<crate::summary::Summary>,
    pub grid: Vec<Sheet>,
}

impl Sheets {
    pub fn len(&self) -> usize {
        self.summary.as_ref().map_or(0, |s| s.sheets.len()) + self.grid.len()
    }
}

pub(crate) type Pages = Result<Sheets, LayoutError>;

/// Collega i pannelli agganciabili al loro contenuto.
struct Content<'a> {
    app: &'a mut WinFilesApp,
    sheets: &'a Pages,
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

/// Un lavoro nuovo, con l'impaginazione e l'unità delle ultime scelte.
pub(crate) fn blank_project(prefs: &crate::prefs::Prefs) -> Project {
    let mut p = Project::new("Senza titolo");
    p.print = prefs.print.clone();
    p.print.title.clear();
    p.unit = prefs.unit;
    p
}

impl WinFilesApp {
    /// Un lavoro nuovo con le ultime scelte.
    pub(crate) fn blank(&self) -> Project {
        blank_project(&self.prefs)
    }

    /// Le scelte di adesso, da ricordare alla prossima apertura.
    fn current_prefs(&self, ctx: &egui::Context) -> crate::prefs::Prefs {
        let (inner, maximized) = ctx.input(|i| {
            let v = i.viewport();
            (v.inner_rect, v.maximized.unwrap_or(false))
        });
        let mut print = self.project.print.clone();
        print.title.clear();
        // Da massimizzata la finestra tiene la misura di prima, per quando si rimpicciolisce.
        let window = match inner {
            Some(r) if !maximized => Some([r.width(), r.height()]),
            _ => self.prefs.window,
        };
        crate::prefs::Prefs {
            dock: Some(self.dock.clone()),
            theme: self.theme,
            unit: self.project.unit,
            print,
            printing: crate::prefs::PrintChoices {
                printer: self.last_print.printer.clone(),
                sides: crate::prefs::Sides::from_duplex(self.last_print.duplex),
                gray: self.last_print.gray,
                fit: self.last_print.fit,
            },
            window,
            maximized,
        }
    }

    /// Scrive le scelte se sono cambiate; si controlla al più una volta al secondo, così anche
    /// una chiusura improvvisa le lascia salvate.
    fn remember_prefs(&mut self, ctx: &egui::Context, now: bool) {
        let time = ctx.input(|i| i.time);
        if !now && time - self.prefs_checked < 1.0 {
            return;
        }
        self.prefs_checked = time;
        if self.batch_pdf.is_some() || self.batch_running {
            return;
        }
        let current = self.current_prefs(ctx);
        if current != self.prefs {
            crate::prefs::save(&current);
            self.prefs = current;
        }
    }
}

impl eframe::App for WinFilesApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dropped: Vec<PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        let (projects, dropped): (Vec<PathBuf>, Vec<PathBuf>) =
            dropped.into_iter().partition(|p| document::is_project(p));
        if !dropped.is_empty() {
            self.add_paths(dropped);
        }
        if let Some(p) = projects.into_iter().next() {
            self.guard(&ctx, Then::Open(Some(p)));
        }
        self.shortcuts(&ctx);
        self.remember_prefs(&ctx, false);
        // Chiudere la finestra con modifiche non salvate chiede prima di perderle.
        if ctx.input(|i| i.viewport().close_requested()) {
            // Alla chiusura le scelte si scrivono subito, anche se è passato meno di un secondo.
            self.remember_prefs(&ctx, true);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.closing && self.is_dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.asking = Some(Then::Close);
        }
        let title = self.window_title();
        if title != self.shown_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.shown_title = title;
        }
        // Canc toglie dall'elenco i file scelti, se non si sta scrivendo in un campo.
        if !self.marked.is_empty()
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| i.key_pressed(egui::Key::Delete))
        {
            let marked = std::mem::take(&mut self.marked);
            self.remove_files(&marked);
        }
        // Spazio mette o toglie dalla stampa i file scelti, come la spunta delle miniature della 4.9.
        if !self.marked.is_empty()
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| i.key_pressed(egui::Key::Space))
        {
            self.toggle_marked_print();
        }
        self.sync_theme(ui.ctx());
        self.receive_scans();
        self.receive_measures();
        self.thumbs.receive(ui.ctx());

        let sheets = self.pages(ui.ctx());
        let sheets: &Pages = &sheets;
        if std::mem::take(&mut self.want_export) {
            self.export_pdf(&ctx, sheets, None);
        }
        if self.batch_pdf.is_some() && self.scans.is_empty() && self.measurer.pending() == 0 {
            let path = self.batch_pdf.take();
            self.batch_running = path.is_some();
            self.export_pdf(&ctx, sheets, path);
            if self.export.is_none() {
                eprintln!("{}", self.status);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if std::mem::take(&mut self.want_print) {
            self.open_print();
        }
        self.receive_export();
        self.print_window(&ctx, sheets);
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
                &mut Content { app: self, sheets },
            );
            if let Some(e) = output.error {
                self.status = format!("Disposizione dei pannelli non valida ({e}): ripristinata");
                self.dock = dock::default_layout();
            }
            for action in output.actions {
                self.apply_dock(action);
            }
        });
        self.unsaved_dialog(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::wheel_zoom;

    /// Un fotogramma con Ctrl + rotella (per egui un evento di zoom) e il puntatore in `at`.
    fn frame(at: egui::Pos2) -> Option<f32> {
        let ctx = egui::Context::default();
        let board = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(400.0, 300.0));
        let mut seen = None;
        for events in [vec![egui::Event::PointerMoved(at)], vec![egui::Event::Zoom(1.25)]] {
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                seen = wheel_zoom(ui, board);
            });
            out.textures_delta.clear();
        }
        seen
    }

    #[test]
    fn ctrl_rotella_ingrandisce_sopra_l_anteprima() {
        // Prima funzionava solo sui fogli di miniature, non sul foglio di riepilogo: ora la
        // richiesta si legge sull'anteprima, qualunque foglio mostri.
        assert_eq!(frame(egui::pos2(300.0, 250.0)), Some(1.25));
        assert_eq!(frame(egui::pos2(20.0, 20.0)), None);
    }
}
