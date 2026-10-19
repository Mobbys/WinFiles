//! Stato dell'applicazione e cornice della finestra: barra dei menu, barra di stampa, pannelli
//! agganciabili su schede arrotondate e barra di stato.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

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
    /// I file non supportati restano fuori dall'elenco (la casella spenta).
    pub(crate) hide_unsupported: bool,
    /// La vecchia WinFile 4.9 sullo stesso PC: la ricerca in corso, poi quella trovata.
    legacy_search: Option<std::sync::mpsc::Receiver<Option<crate::legacy::Legacy>>>,
    legacy: Option<crate::legacy::Legacy>,
    keep_legacy: bool,
    /// I file non supportati tolti con la casella, da rimettere se la si riaccende.
    pub(crate) parked: Vec<winfiles_model::FileEntry>,
    pub(crate) theme: ThemeChoice,
    applied: Option<Tokens>,
    dock: Layout<PanelId>,
    pub(crate) measurer: Measurer,
    pub(crate) thumbs: Thumbnails,
    /// Le miniature sul foglio, una per pagina, ricalcolate a ogni fotogramma.
    slots: Vec<Slot>,
    /// Cartella di cui si sta rinominando l'intestazione nell'elenco, con il testo in modifica.
    pub(crate) renaming: Option<(PathBuf, String)>,
    /// Il file su cui si è aperto il menu del tasto destro nell'anteprima del foglio.
    pub(crate) sheet_menu: Option<usize>,
    /// Cosa mostra l'elenco dei file: cerca per nome e filtra per tipo.
    pub(crate) filter: crate::filter::FileFilter,
    /// Ctrl+F: il campo di ricerca prende il fuoco al prossimo disegno.
    pub(crate) focus_search: bool,
    /// File o cartella che si sta trascinando nell'elenco per cambiarne l'ordine.
    pub(crate) drag: Option<crate::reorder::Drag>,
    /// File con più pagine aperti nell'elenco, una riga per pagina.
    pub(crate) expanded: BTreeSet<PathBuf>,
    /// Pagina da mostrare nell'anteprima al prossimo disegno: si va al suo foglio.
    pub(crate) reveal: Option<(usize, usize)>,
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
    /// Le pagine da stampare chieste da Modifica PDF o Modifica immagini, con il nome del lavoro e
    /// quella in anteprima: finché ci sono, la finestra Stampa stampa queste e non i fogli.
    module_print: Option<(Vec<winfiles_scene::Page>, String, usize)>,
    /// Le scelte dell'ultima stampa, per la prossima.
    last_print: crate::printing::PrintDialog,
    /// Le stampanti del sistema, lette in un thread a parte.
    printer_scan: crate::printing::PrinterScan,
    /// Il controllo degli aggiornamenti e l'installer scaricato.
    updater: crate::update::Updater,
    /// La finestra con le novità della versione nuova è aperta.
    show_notes: bool,
    /// La finestra "Calibra la stampante" aperta.
    calibrate: Option<crate::calibrate::CalibrateDialog>,
    /// Aprire la finestra della calibrazione al prossimo fotogramma.
    want_calibrate: bool,
    /// La correzione di ogni stampante calibrata, per nome.
    calibrations: std::collections::BTreeMap<String, winfiles_geom::Calibration>,
    /// I modelli di impaginazione salvati.
    pub(crate) presets: Vec<crate::presets::Preset>,
    /// Il nome del modello che si sta scrivendo per salvarlo.
    pub(crate) preset_name: Option<String>,
    /// `--calibrazione A4 uscita.pdf`: la pagina di calibrazione in PDF, poi chiudere.
    batch_calibration: Option<(Paper, PathBuf)>,
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
    /// Il modulo mostrato: miniature da stampare o modifica dei PDF.
    pub(crate) module: Module,
    /// Il modulo PDF (Fase 9), con il suo documento aperto.
    pub(crate) pdf: crate::pdf::PdfModule,
    /// Il modulo immagini (Fase 8), con le immagini aperte.
    pub(crate) images: crate::images::ImagesModule,
}

/// I moduli di WinFiles, come le schede della 4.9.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Module {
    #[default]
    Thumbnails,
    Pdf,
    Images,
}

impl WinFilesApp {
    pub fn new(
        ctx: &egui::Context,
        paths: Vec<PathBuf>,
        batch_pdf: Option<PathBuf>,
        batch_calibration: Option<(Paper, PathBuf)>,
        edit_pdf: Option<PathBuf>,
        edit_images: Vec<PathBuf>,
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
            hide_unsupported: prefs.hide_unsupported,
            legacy_search: (!prefs.keep_legacy).then(crate::legacy::find),
            legacy: None,
            keep_legacy: prefs.keep_legacy,
            parked: Vec::new(),
            theme: prefs.theme,
            applied: None,
            dock: prefs.dock.clone().unwrap_or_else(dock::default_layout),
            measurer: Measurer::start(move || wake.request_repaint()),
            thumbs: Thumbnails::start(move || wake2.request_repaint()),
            slots: Vec::new(),
            renaming: None,
            sheet_menu: None,
            filter: Default::default(),
            focus_search: false,
            drag: None,
            expanded: BTreeSet::new(),
            reveal: None,
            status: String::new(),
            scans: Vec::new(),
            export: None,
            want_export: false,
            want_print: false,
            print_dialog: None,
            module_print: None,
            last_print,
            printer_scan: crate::printing::PrinterScan::default(),
            updater: crate::update::Updater::default(),
            show_notes: false,
            batch_pdf,
            batch_running: false,
            calibrate: None,
            want_calibrate: false,
            calibrations: prefs.calibrations.clone(),
            presets: prefs.presets.clone(),
            preset_name: None,
            batch_calibration,
            wheel_edges: (true, true),
            last_flip: 0.0,
            flip_to: None,
            recent: crate::recent::load(),
            ctx: ctx.clone(),
            prefs,
            prefs_checked: 0.0,
            nesting_scale: None,
            layout_cache: None,
            module: Module::Thumbnails,
            pdf: crate::pdf::PdfModule::new(ctx),
            images: crate::images::ImagesModule::new(ctx),
        };
        // Le stampanti si cercano subito, così Stampa le trova già pronte.
        if app.batch_pdf.is_none() && app.batch_calibration.is_none() {
            app.scan_printers();
            // Come la 4.9: all'avvio si chiede a GitHub se c'è una versione nuova, senza aspettare.
            app.check_updates(false);
        }
        // `--modifica-pdf file.pdf` apre quel PDF nel modulo PDF ("Apri con" di Windows).
        if let Some(path) = edit_pdf {
            app.module = Module::Pdf;
            app.pdf.open(&path);
        }
        // `--modifica-immagini foto.jpg …` apre quelle immagini nel modulo immagini.
        if !edit_images.is_empty() {
            app.module = Module::Images;
            app.images.open(&edit_images);
        }
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

    /// Rilegge le stampanti in un thread a parte; arrivano con [`Self::receive_printers`].
    fn scan_printers(&mut self) {
        let wake = self.ctx.clone();
        self.printer_scan.start(move || wake.request_repaint());
    }

    /// Chiede l'ultima versione pubblicata; `asked` se l'ha chiesto chi usa WinFiles.
    fn check_updates(&mut self, asked: bool) {
        let wake = self.ctx.clone();
        self.updater.check(asked, move || wake.request_repaint());
    }

    /// La barra sotto i comandi che annuncia la versione nuova.
    /// La barra che propone di togliere la vecchia WinFile 4.9, se c'è ancora.
    fn legacy_bar(&mut self, ui: &mut egui::Ui) {
        if let Some(rx) = &self.legacy_search
            && let Ok(found) = rx.try_recv()
        {
            self.legacy = found;
            self.legacy_search = None;
        }
        let Some(old) = self.legacy.clone() else {
            return;
        };
        let t = Tokens::get(ui.ctx());
        let name = match old.version.as_str() {
            "" => "WinFile".to_string(),
            v => format!("WinFile {v}"),
        };
        let frame = egui::Frame::NONE
            .fill(t.panel)
            .stroke(egui::Stroke::new(1.0, t.border))
            .corner_radius(t.card_radius)
            .inner_margin(egui::Margin::symmetric(12, 4))
            .outer_margin(egui::Margin {
                left: 10,
                right: 10,
                top: 0,
                bottom: 6,
            });
        egui::Panel::top("barra-vecchia-winfile")
            .frame(egui::Frame::NONE.fill(t.app_bar))
            .show(ui, |ui| {
                frame.show(ui, |ui| {
                    ui.set_min_height(30.0);
                    ui.horizontal_centered(|ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "Su questo PC c'è ancora {name}: WinFiles la sostituisce, puoi toglierla."
                            ))
                            .font(semibold(13.0))
                            .color(t.text_strong),
                        );
                        ui.label(
                            egui::RichText::new("Tienila solo se usi la Simulazione quote.")
                                .size(12.5)
                                .color(t.text_dim),
                        );
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Tienila").clicked() {
                                self.legacy = None;
                                self.keep_legacy = true;
                            }
                            let remove = egui::Button::new(
                                egui::RichText::new(format!("Togli {name}"))
                                    .font(semibold(13.0))
                                    .color(t.on_primary),
                            )
                            .fill(t.primary)
                            .min_size(vec2(96.0, 26.0));
                            if ui.add(remove).clicked() {
                                match crate::legacy::remove(&old) {
                                    Ok(()) => self.status = format!("Avviata la disinstallazione di {name}"),
                                    Err(e) => self.status = format!("Non riesco a togliere {name}: {e}"),
                                }
                                self.legacy = None;
                            }
                        });
                    });
                });
            });
    }

    fn update_bar(&mut self, ui: &mut egui::Ui) {
        self.legacy_bar(ui);
        use crate::update::State;
        self.updater.poll();
        let installed = crate::update::installed();
        let (text, busy) = match &self.updater.state {
            State::Available(r) if !self.updater.dismissed => (
                format!(
                    "È disponibile WinFiles {} (questa è la {}).",
                    r.version,
                    crate::update::Version::current()
                ),
                false,
            ),
            State::Downloading(r) => (format!("Scarico WinFiles {}…", r.version), true),
            State::Ready(r, _) if !self.updater.dismissed => (
                format!(
                    "WinFiles {} è pronto: si installa quando chiudi WinFiles.",
                    r.version
                ),
                false,
            ),
            State::Checking if self.updater.asked => ("Controllo gli aggiornamenti…".to_string(), true),
            State::UpToDate if self.updater.asked && !self.updater.dismissed => (
                format!("WinFiles {} è aggiornato.", crate::update::Version::current()),
                false,
            ),
            State::Failed(e) if self.updater.asked && !self.updater.dismissed => (e.clone(), false),
            _ => return,
        };
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE
            .fill(t.panel)
            .stroke(egui::Stroke::new(1.0, t.border))
            .corner_radius(t.card_radius)
            .inner_margin(egui::Margin::symmetric(12, 4))
            .outer_margin(egui::Margin {
                left: 10,
                right: 10,
                top: 0,
                bottom: 6,
            });
        egui::Panel::top("barra-aggiornamenti")
            .frame(egui::Frame::NONE.fill(t.app_bar))
            .show(ui, |ui| {
                frame.show(ui, |ui| {
                    ui.set_min_height(30.0);
                    ui.horizontal_centered(|ui| {
                        if busy {
                            ui.add(egui::Spinner::new().size(13.0).color(t.accent));
                        }
                        ui.label(
                            egui::RichText::new(text)
                                .font(semibold(13.0))
                                .color(t.text_strong),
                        );
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if !busy
                                && crate::icons::button(ui, "x", "Più tardi", false, true, 26.0).clicked()
                            {
                                self.updater.dismissed = true;
                            }
                            let primary = |label: &str| {
                                egui::Button::new(
                                    egui::RichText::new(label)
                                        .font(semibold(13.0))
                                        .color(t.on_primary),
                                )
                                .fill(t.primary)
                                .min_size(vec2(96.0, 26.0))
                            };
                            match self.updater.state.clone() {
                                State::Available(r) => {
                                    let label = if installed && r.msi.is_some() {
                                        "Aggiorna"
                                    } else {
                                        "Scarica"
                                    };
                                    if ui.add(primary(label)).clicked() {
                                        if installed && r.msi.is_some() {
                                            let wake = self.ctx.clone();
                                            self.updater.download(move || wake.request_repaint());
                                        } else if let Err(e) = crate::shell::open(Path::new(&r.page)) {
                                            self.status =
                                                format!("Non riesco ad aprire la pagina del rilascio: {e}");
                                        }
                                    }
                                    if !r.notes.trim().is_empty() && ui.button("Novità").clicked() {
                                        self.show_notes = true;
                                    }
                                }
                                State::Ready(..) => {
                                    if ui.add(primary("Riavvia ora")).clicked() {
                                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                                    }
                                    if ui.button("Novità").clicked() {
                                        self.show_notes = true;
                                    }
                                }
                                _ => {}
                            }
                        });
                    });
                });
            });
    }

    /// Le novità della versione annunciata.
    fn notes_window(&mut self, ctx: &egui::Context) {
        use crate::update::State;
        let (State::Available(r) | State::Downloading(r) | State::Ready(r, _)) = &self.updater.state else {
            self.show_notes = false;
            return;
        };
        if !self.show_notes {
            return;
        }
        let t = Tokens::get(ctx);
        let title = format!("Novità di WinFiles {}", r.version);
        let notes = r.notes.clone();
        let mut open = true;
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    // Le note sono il Markdown del rilascio: titoli in grassetto, elenchi con il pallino.
                    for line in notes.lines() {
                        let line = line.trim_end().replace("**", "");
                        let text = if let Some(title) = line.strip_prefix('#') {
                            egui::RichText::new(title.trim_start_matches('#').trim())
                                .font(semibold(14.0))
                                .color(t.text_strong)
                        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* "))
                        {
                            egui::RichText::new(format!("•  {item}")).size(13.0).color(t.text)
                        } else {
                            egui::RichText::new(line).size(13.0).color(t.text)
                        };
                        ui.label(text);
                    }
                });
            });
        self.show_notes = open;
    }

    /// Porta le stampanti appena lette nelle finestre aperte.
    fn receive_printers(&mut self) {
        let Some(list) = self.printer_scan.poll() else {
            return;
        };
        if let Some(d) = self.print_dialog.as_mut() {
            d.set_printers(list.clone());
        }
        if let Some(d) = self.calibrate.as_mut() {
            d.set_printers(list);
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
        dialog.set_printers(self.printer_scan.list());
        self.print_dialog = Some(dialog);
        self.scan_printers();
    }

    /// Prende la stampa o il PDF con le quote chiesti da Modifica PDF o Modifica immagini.
    fn module_output(&mut self) {
        let output = match self.module {
            Module::Pdf => self.pdf.output.take(),
            Module::Images => self.images.output.take(),
            _ => None,
        };
        let Some(output) = output else {
            return;
        };
        if self.export.is_some() || self.print_dialog.is_some() {
            self.set_module_status("Aspetta che finisca la stampa in corso".into());
            return;
        }
        match output {
            crate::quotes::Output::Print {
                pages,
                title,
                current,
            } => {
                if pages.is_empty() {
                    return;
                }
                self.module_print = Some((pages, title, current));
                let mut dialog = self.last_print.clone();
                dialog.set_printers(self.printer_scan.list());
                self.print_dialog = Some(dialog);
                self.scan_printers();
            }
            crate::quotes::Output::Save { pages, path } => {
                self.start_job(pages, crate::export::Target::File(path));
                if self.export.is_none() {
                    let status = self.status.clone();
                    self.set_module_status(status);
                }
            }
        }
    }

    /// Scrive nella barra di stato del modulo aperto (Modifica PDF o Modifica immagini).
    fn set_module_status(&mut self, text: String) {
        match self.module {
            Module::Pdf => self.pdf.status = text,
            Module::Images => self.images.status = text,
            _ => {}
        }
    }

    /// La finestra Stampa aperta; alla conferma i fogli scelti vanno alla stampante.
    fn print_window(&mut self, ctx: &egui::Context, sheets: &Pages) {
        let (count, title, current) = match &self.module_print {
            Some((pages, title, current)) => (pages.len(), title.clone(), *current),
            None => (
                sheets.as_ref().map_or(0, Sheets::len),
                self.document_name(),
                self.sheet,
            ),
        };
        let Some(dialog) = self.print_dialog.as_mut() else {
            return;
        };
        let searching = self.printer_scan.searching();
        let Some(answer) = dialog.show(ctx, &title, count, current, &self.calibrations, searching) else {
            return;
        };
        if let Some(d) = self.print_dialog.take() {
            self.last_print = d;
        }
        let module_pages = self.module_print.take();
        let (job, range) = match answer {
            crate::printing::Answer::Print(job, range) => (job, range),
            crate::printing::Answer::Calibrate => {
                self.want_calibrate = true;
                return;
            }
            crate::printing::Answer::Cancel => return,
        };
        if let Some((pages, _, _)) = module_pages {
            let mut pages: Vec<_> = pages
                .into_iter()
                .enumerate()
                .filter(|(i, _)| range.contains(i))
                .map(|(_, p)| p)
                .collect();
            crate::calibrate::apply(&mut pages, &job, &self.calibrations);
            self.start_job(pages, crate::export::Target::Printer(job));
            return;
        }
        let Ok(sheets) = sheets else {
            return;
        };
        let mut pages = self.compose(ctx, sheets, range);
        crate::calibrate::apply(&mut pages, &job, &self.calibrations);
        self.start_job(pages, crate::export::Target::Printer(job));
    }

    /// Apre la finestra della calibrazione sulla stampante dell'ultima stampa.
    fn open_calibrate(&mut self) {
        if self.calibrate.is_some() || self.print_dialog.is_some() {
            return;
        }
        let paper = self.project.print.paper;
        self.calibrate = Some(crate::calibrate::CalibrateDialog::new(
            self.printer_scan.list(),
            self.last_print.printer.clone(),
            paper,
        ));
        self.scan_printers();
    }

    /// La finestra della calibrazione aperta.
    fn calibrate_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.calibrate.as_mut() else {
            return;
        };
        let searching = self.printer_scan.searching();
        let Some(action) = dialog.show(ctx, &self.calibrations, &mut self.thumbs, searching) else {
            return;
        };
        use crate::calibrate::Action;
        match action {
            Action::Print(page, job) => {
                if self.export.is_some() {
                    self.status = "Aspetta che finisca la stampa in corso".into();
                } else {
                    self.start_job(vec![*page], crate::export::Target::Printer(job));
                }
            }
            Action::Pdf(page) => {
                let Some(mut path) = rfd::FileDialog::new()
                    .add_filter("PDF", &["pdf"])
                    .set_title("Salva la pagina di calibrazione")
                    .set_file_name("Calibrazione.pdf")
                    .save_file()
                else {
                    return;
                };
                if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
                    path.as_mut_os_string().push(".pdf");
                }
                if self.export.is_none() {
                    self.start_job(vec![*page], crate::export::Target::File(path));
                }
            }
            Action::Save(name, c) => {
                self.status = format!(
                    "Correzione salvata per {name}: {}",
                    crate::calibrate::describe(&c)
                );
                if c.is_identity() {
                    self.calibrations.remove(&name);
                } else {
                    self.calibrations.insert(name, c);
                }
            }
            Action::Remove(name) => {
                self.status = format!("Tolta la correzione di {name}");
                self.calibrations.remove(&name);
            }
            Action::Close => self.calibrate = None,
        }
        // Una correzione nuova si scrive subito nelle impostazioni.
        self.remember_prefs(ctx, true);
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
        let mut hidden = 0;
        if self.hide_unsupported {
            let (keep, parked) = crate::unsupported::split(found);
            let known = |p: &std::path::Path| {
                files.iter().any(|f: &winfiles_model::FileEntry| f.path == p)
                    || self.parked.iter().any(|f| f.path == p)
            };
            let parked: Vec<_> = parked.into_iter().filter(|f| !known(&f.path)).collect();
            hidden = parked.len();
            self.parked.extend(parked);
            files.extend(keep);
        } else {
            files.extend(found);
        }
        files.sort_by_cached_key(|f| winfiles_model::file_order_key(&f.path));
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
            0 => "Nessun file nuovo".into(),
            1 => "Aggiunto 1 file".into(),
            n => format!("Aggiunti {n} file"),
        };
        match hidden {
            0 => {}
            1 => self.status.push_str(", 1 non supportato lasciato fuori"),
            n => self
                .status
                .push_str(&format!(", {n} non supportati lasciati fuori")),
        }
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
        self.parked.clear();
        self.errors.clear();
        self.thumbs.clear();
        self.unselect();
        self.sheet = 0;
        self.status = "Elenco svuotato".into();
    }

    /// La casella "Includi i file non supportati": spenta li toglie tutti dall'elenco e dai
    /// fogli, riaccesa li rimette al loro posto.
    pub(crate) fn include_unsupported(&mut self, on: bool) {
        self.hide_unsupported = !on;
        self.unselect();
        if on {
            let parked = std::mem::take(&mut self.parked);
            let n = parked.len();
            crate::unsupported::restore(&mut self.project.files, parked);
            self.status = match n {
                0 => "Nessun file non supportato da rimettere".into(),
                1 => "Rimesso 1 file non supportato".into(),
                n => format!("Rimessi {n} file non supportati"),
            };
        } else {
            let n = self.park_unsupported();
            self.status = match n {
                1 => "Tolto 1 file non supportato".into(),
                n => format!("Tolti {n} file non supportati"),
            };
        }
    }

    /// Mette da parte i file non supportati del lavoro; quanti erano.
    pub(crate) fn park_unsupported(&mut self) -> usize {
        let files = std::mem::take(&mut self.project.files);
        let (keep, parked) = crate::unsupported::split(files);
        self.project.files = keep;
        let n = parked.len();
        self.parked.extend(parked);
        n
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
        // Ctrl+C arriva come evento Copia (con o senza Maiusc). Fuori dai campi di testo copia la
        // tabella: dei file scelti, o di tutto il lavoro con Maiusc.
        let copy = ctx.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Copy))
                .then_some(i.modifiers.shift)
        });
        if let Some(all) = copy
            && !ctx.egui_wants_keyboard_input()
        {
            if all || self.marked.is_empty() {
                self.copy_table(None);
            } else {
                let marked = self.marked.clone();
                self.copy_table(Some(&marked));
            }
        }
    }

    /// Copia negli appunti la tabella delle pagine in stampa (dei file `only`, se c'è): HTML per
    /// Excel, Word e la posta, testo a tabulazioni per tutto il resto.
    pub(crate) fn copy_table(&mut self, only: Option<&BTreeSet<usize>>) {
        let unit = self.project.unit;
        let lines = crate::table::lines(&self.project, only);
        if lines.is_empty() {
            self.status = "Nessuna pagina in stampa da copiare".into();
            return;
        }
        let text = crate::table::tsv(&lines, unit);
        let html = crate::table::html(&lines, unit);
        let rich = arboard::Clipboard::new().and_then(|mut c| c.set_html(html, Some(text.clone())));
        if rich.is_err() {
            // Senza gli appunti di sistema restano quelli di egui, solo testo.
            self.ctx.copy_text(text);
        }
        let n = lines.len();
        self.status = format!(
            "Copiata negli appunti la tabella di {n} {}: si incolla in Excel, Word o in una mail",
            if n == 1 { "pagina" } else { "pagine" }
        );
    }

    /// Salva la tabella delle pagine in stampa in un CSV per Excel.
    fn export_csv(&mut self) {
        let lines = crate::table::lines(&self.project, None);
        if lines.is_empty() {
            self.status = "Nessuna pagina in stampa da esportare".into();
            return;
        }
        let mut dialog = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_title("Esporta la tabella in CSV")
            .set_file_name(format!("{}.csv", self.document_name()));
        if let Some(dir) = self.path.as_deref().and_then(std::path::Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        let Some(mut path) = dialog.save_file() else {
            return;
        };
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv")) {
            path.as_mut_os_string().push(".csv");
        }
        let text = crate::table::csv(&lines, self.project.unit);
        let mut tmp = path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let written = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &path));
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.status = match written {
            Ok(()) => format!("Esportata la tabella in {name}: {} righe", lines.len()),
            Err(e) => format!("Non riesco a scrivere {name}: {e}"),
        };
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

    /// Sposta i file indicati prima del file `before`, tenendo scelti gli stessi file.
    pub(crate) fn move_files(&mut self, moving: &BTreeSet<usize>, before: usize) {
        let path = |i: usize| self.project.files.get(i).map(|f| f.path.clone());
        let marked: Vec<PathBuf> = self.marked.iter().filter_map(|i| path(*i)).collect();
        let selected = self.selected.and_then(path);
        let anchor = self.anchor.and_then(path);
        self.project.move_files(moving, before);
        let index: HashMap<&std::path::Path, usize> = self
            .project
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.path.as_path(), i))
            .collect();
        let find = |p: &PathBuf| index.get(p.as_path()).copied();
        self.marked = marked.iter().filter_map(find).collect();
        self.selected = selected.as_ref().and_then(find);
        self.anchor = anchor.as_ref().and_then(find);
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
        let arrived = self.measurer.drain();
        if arrived.is_empty() {
            return;
        }
        // L'ordine dei file può essere quello scelto trascinando: si cercano per percorso. Uno
        // tolto nel frattempo si ignora.
        let index: HashMap<PathBuf, usize> = self
            .project
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.path.clone(), i))
            .collect();
        for m in arrived {
            let Some(&i) = index.get(&m.path) else {
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
                    ui.add_space(12.0);
                    // I moduli, come le schede della 4.9: miniature da stampare e modifica dei PDF.
                    segmented(
                        ui,
                        "modulo",
                        &mut self.module,
                        &[
                            (Module::Thumbnails, "Miniature"),
                            (Module::Pdf, "Modifica PDF"),
                            (Module::Images, "Modifica immagini"),
                        ],
                    );
                    ui.add_space(12.0);
                    egui::MenuBar::new().ui(ui, |ui| {
                        ui.menu_button("File", |ui| {
                            if self.module == Module::Pdf {
                                self.pdf.file_menu(ui);
                                return;
                            }
                            if self.module == Module::Images {
                                self.images.file_menu(ui);
                                return;
                            }
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
                            let has_files = !self.project.files.is_empty();
                            if ui
                                .add_enabled(has_files, egui::Button::new("Esporta tabella CSV…"))
                                .on_hover_text(
                                    "Una riga per pagina in stampa, con misure e m², da aprire in Excel",
                                )
                                .clicked()
                            {
                                ui.close();
                                self.export_csv();
                            }
                            if ui
                                .add_enabled(
                                    has_files,
                                    egui::Button::new("Copia tabella").shortcut_text("Ctrl+Maiusc+C"),
                                )
                                .on_hover_text(
                                    "Da incollare in Excel, Word o in una mail, con cartelle e totali",
                                )
                                .clicked()
                            {
                                ui.close();
                                self.copy_table(None);
                            }
                            ui.separator();
                            if ui
                                .add_enabled(
                                    self.export.is_none(),
                                    egui::Button::new("Calibra la stampante…"),
                                )
                                .on_hover_text(
                                    "Una pagina da misurare con il righello, perché le stampe vengano esatte",
                                )
                                .clicked()
                            {
                                ui.close();
                                self.want_calibrate = true;
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
                                if crate::fluent::radio(ui, self.theme == choice, label).clicked() {
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
                                if crate::fluent::radio(ui, self.project.unit == unit, label).clicked() {
                                    self.project.unit = unit;
                                    ui.close();
                                }
                            }
                        });
                        if self.module == Module::Thumbnails {
                            ui.menu_button("Finestra", |ui| {
                                for panel in PanelId::TOOLS {
                                    if ui
                                        .add(crate::fluent::checkbox(
                                            &mut self.dock.contains(&panel),
                                            panel.label(),
                                        ))
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
                        }
                        ui.menu_button("Aiuto", |ui| {
                            ui.label(concat!("WinFiles ", env!("CARGO_PKG_VERSION")));
                            if ui.button("Controlla aggiornamenti").clicked() {
                                ui.close();
                                self.check_updates(true);
                            }
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
                        "{} file  ·  {}  ·  {}",
                        totals.files,
                        crate::canvas::pages_text(totals.pages),
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
        // Una pagina scelta nell'elenco: si va al foglio che la contiene.
        if let Some(target) = self.reveal.take()
            && let Ok(sheets) = pages
            && let Some(n) = sheet_of(sheets, &self.slots, target)
        {
            self.sheet = n;
        }

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
                .icon(crate::fluent::combo_icon)
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
        // Il tasto destro su una miniatura apre lo stesso menu dell'elenco dei file.
        if let Some(hit) = out.secondary {
            self.sheet_menu = hit.map(|(file, _)| file);
            if let Some((file, page)) = hit {
                self.pick_for_menu(file);
                self.selected_page = page;
            }
        }
        if let (Some(resp), Some(i)) = (out.response, self.sheet_menu) {
            let count = if self.marked.contains(&i) {
                self.marked.len().max(1)
            } else {
                1
            };
            let mut action = None;
            if let Some(f) = self.project.files.get(i) {
                let state = f.print_state();
                resp.context_menu(|ui| action = crate::panels::file_menu(ui, f, count, state));
            }
            if let Some(a) = action {
                self.file_action(ui.ctx(), a);
            }
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
        .add(crate::fluent::checkbox(&mut auto, "Auto"))
        .on_hover_text("Il file più grande largo un terzo del foglio")
        .changed()
    {
        *scale = if auto { 0 } else { current };
    }
}

/// Un campo con − e + ai lati e `text` al centro; dice di quanto cambiare (−1, 0 o +1).
fn stepper_with(ui: &mut egui::Ui, id: &str, text: &str, can_dec: bool, can_inc: bool) -> i32 {
    let t = Tokens::get(ui.ctx());
    // Come il campo numerico di Windows 11: fondo e bordi dei campi, − e + dentro.
    let (rect, _) = ui.allocate_exact_size(vec2(100.0, crate::fluent::HEIGHT), Sense::hover());
    ui.painter().rect(
        rect,
        crate::fluent::RADIUS,
        t.field,
        Stroke::new(1.0, t.field_border),
        egui::StrokeKind::Inside,
    );
    crate::fluent::field_edge(ui, rect, false);
    let side = vec2(30.0, rect.height());
    let minus = Rect::from_min_size(rect.min, side);
    let plus = Rect::from_min_size(egui::pos2(rect.right() - side.x, rect.top()), side);
    let mut out = 0;
    for (r, icon, enabled, delta) in [(minus, "minus", can_dec, -1), (plus, "plus", can_inc, 1)] {
        let resp = ui.interact(
            r,
            ui.id().with(("stepper", id, icon)),
            if enabled { Sense::click() } else { Sense::hover() },
        );
        if enabled && resp.hovered() {
            ui.painter().rect_filled(r.shrink(3.0), 3.0, t.field_hover);
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

/// Pulsanti uniti a scelta singola: la scelta attiva è evidenziata in blu tenue, con la piccola
/// barra d'accento sotto come le schede di Windows 11.
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
    let (rect, _) = ui.allocate_exact_size(vec2(total, crate::fluent::HEIGHT), Sense::hover());
    ui.painter().rect(
        rect,
        t.radius + 2,
        t.field,
        Stroke::new(1.0, t.field_border),
        egui::StrokeKind::Inside,
    );
    let mut x = rect.left() + 2.0;
    for (i, ((option, label), w)) in options.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(egui::pos2(x, rect.top() + 2.0), vec2(*w, rect.height() - 4.0));
        let resp = ui.interact(r, ui.id().with((id, i)), Sense::click());
        let on = *value == *option;
        if on {
            ui.painter().rect_filled(r, t.radius, t.selection);
            let bar = Rect::from_center_size(
                egui::pos2(r.center().x, r.bottom() - 2.5),
                vec2(16.0_f32.min(r.width() - 8.0), 3.0),
            );
            ui.painter().rect_filled(bar, 1.5, t.accent);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, t.radius, t.field_hover);
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

/// Il foglio (contando il riepilogo) dove sta la miniatura della pagina `page` del file `file`.
fn sheet_of(sheets: &Sheets, slots: &[Slot], (file, page): (usize, usize)) -> Option<usize> {
    let before = sheets.summary.as_ref().map_or(0, |s| s.sheets.len());
    let n = sheets.grid.iter().position(|sheet| {
        sheet.items.iter().any(|item| {
            slots
                .get(item.id)
                .is_some_and(|s| s.file == file && s.page == page)
        })
    })?;
    Some(before + n)
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
            calibrations: self.calibrations.clone(),
            presets: self.presets.clone(),
            window,
            maximized,
            hide_unsupported: self.hide_unsupported,
            keep_legacy: self.keep_legacy,
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
        if self.batch_pdf.is_some() || self.batch_calibration.is_some() || self.batch_running {
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
        let pdf_mode = self.module == Module::Pdf;
        let images_mode = self.module == Module::Images;
        if pdf_mode {
            if !dropped.is_empty() {
                self.pdf.dropped(&ctx, dropped);
            }
            self.pdf.shortcuts(&ctx);
        } else if images_mode {
            if !dropped.is_empty() {
                self.images.dropped(dropped);
            }
            self.images.shortcuts(&ctx);
        } else {
            let (projects, dropped): (Vec<PathBuf>, Vec<PathBuf>) =
                dropped.into_iter().partition(|p| document::is_project(p));
            if !dropped.is_empty() {
                self.add_paths(dropped);
            }
            if let Some(p) = projects.into_iter().next() {
                self.guard(&ctx, Then::Open(Some(p)));
            }
            self.shortcuts(&ctx);
        }
        self.remember_prefs(&ctx, false);
        // Chiudere la finestra con modifiche non salvate chiede prima di perderle: prima il PDF in
        // modifica, poi il lavoro delle miniature.
        if ctx.input(|i| i.viewport().close_requested()) {
            // Alla chiusura le scelte si scrivono subito, anche se è passato meno di un secondo.
            self.remember_prefs(&ctx, true);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.closing && !self.pdf.can_quit() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.module = Module::Pdf;
        } else if ctx.input(|i| i.viewport().close_requested()) && !self.closing && !self.images.can_quit() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.module = Module::Images;
        } else if ctx.input(|i| i.viewport().close_requested()) && !self.closing && self.is_dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.asking = Some(Then::Close);
            self.module = Module::Thumbnails;
        }
        let title = match self.module {
            Module::Pdf => self
                .pdf
                .title()
                .unwrap_or_else(|| "Modifica PDF — WinFiles".into()),
            Module::Images => self
                .images
                .title()
                .unwrap_or_else(|| "Modifica immagini — WinFiles".into()),
            Module::Thumbnails => self.window_title(),
        };
        if title != self.shown_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.shown_title = title;
        }
        // Canc toglie dall'elenco i file scelti, se non si sta scrivendo in un campo.
        if !pdf_mode
            && !images_mode
            && !self.marked.is_empty()
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| i.key_pressed(egui::Key::Delete))
        {
            let marked = std::mem::take(&mut self.marked);
            self.remove_files(&marked);
        }
        // Spazio mette o toglie dalla stampa i file scelti, come la spunta delle miniature della 4.9.
        if !pdf_mode
            && !images_mode
            && !self.marked.is_empty()
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
        if std::mem::take(&mut self.want_calibrate) {
            self.open_calibrate();
        }
        if let Some((paper, path)) = self.batch_calibration.take() {
            let sheet = winfiles_layout::calibration::CalibrationSheet::new(paper);
            let page = crate::calibrate::compose(&ctx, &sheet, None, &winfiles_geom::Calibration::IDENTITY);
            self.batch_running = true;
            self.start_job(vec![page], crate::export::Target::File(path));
            if self.export.is_none() {
                eprintln!("{}", self.status);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        let exporting = self.export.is_some();
        self.receive_export();
        if exporting && self.export.is_none() {
            let status = self.status.clone();
            self.set_module_status(status);
        }
        if let Some(job) = &self.export {
            let progress = job.progress_text();
            self.set_module_status(progress);
        }
        self.receive_printers();
        self.print_window(&ctx, sheets);
        self.calibrate_window(&ctx);
        self.app_bar(ui);
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE.fill(t.app_bar).inner_margin(egui::Margin {
            left: 10,
            right: 10,
            top: 0,
            bottom: 0,
        });
        if self.module == Module::Pdf {
            self.pdf.control_bar(ui);
            self.update_bar(ui);
            self.pdf.status_bar(ui);
            let unit = self.project.unit;
            egui::CentralPanel::default()
                .frame(frame.inner_margin(egui::Margin {
                    left: 10,
                    right: 10,
                    top: 0,
                    bottom: 2,
                }))
                .show(ui, |ui| self.pdf.body(ui, unit));
            self.module_output();
            self.pdf.windows(&ctx);
            self.notes_window(&ctx);
            self.unsaved_dialog(&ctx);
            return;
        }
        if self.module == Module::Images {
            self.images.control_bar(ui);
            self.update_bar(ui);
            self.images.status_bar(ui);
            let unit = self.project.unit;
            egui::CentralPanel::default()
                .frame(frame.inner_margin(egui::Margin {
                    left: 10,
                    right: 10,
                    top: 0,
                    bottom: 2,
                }))
                .show(ui, |ui| self.images.body(ui, unit));
            self.module_output();
            self.images.windows(&ctx);
            self.unsaved_dialog(&ctx);
            return;
        }
        self.control_bar(ui);
        self.update_bar(ui);
        self.status_bar(ui);
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
        self.notes_window(&ctx);
        self.unsaved_dialog(&ctx);
    }

    // L'installer scaricato si avvia quando WinFiles si è chiuso (ADR 0014).
    #[cfg(feature = "opengl")]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.install_update();
    }

    #[cfg(not(feature = "opengl"))]
    fn on_exit(&mut self) {
        self.install_update();
    }
}

impl WinFilesApp {
    /// Avvia l'installer già scaricato, se c'è.
    fn install_update(&mut self) {
        if let crate::update::State::Ready(_, msi) = &self.updater.state
            && let Err(e) = crate::update::install_after_exit(msi)
        {
            eprintln!("Non riesco ad avviare l'aggiornamento: {e}");
        }
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
