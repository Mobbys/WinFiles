//! La scheda Visualizza (ADR 0031): guardare un file senza cambiarlo, con le misure vere, lo zoom
//! e, per i vettoriali, le tinte piatte; poi passarlo alla scheda che serve.
//!
//! Si apre con il doppio clic in Esplora file (se WinFiles è il programma predefinito di quel
//! tipo), con il doppio clic su una miniatura e trascinando i file sulla scheda. Ogni file aperto
//! ha la sua linguetta; le frecce scorrono i file della stessa cartella, o del lavoro delle
//! miniature se il file arriva da lì.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Align, Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use winfiles_geom::{Unit, area_m2, format_area, format_length, format_number, format_size};

use crate::app::{POINTS_PER_MM, card};
use crate::icons;
use crate::theme::{Tokens, semibold};

mod info;
pub mod load;
pub mod print;

use load::{CadChoice, Facts, Opened, Opening, PageView, Renderer, Tile, ViewKind};

/// Lati delle pagine intere disegnate: si sceglie il primo che basta per lo schermo.
const SIDES: [u32; 4] = [512, 1024, 2048, 4096];
/// Lato delle pagine nella striscia a sinistra.
const STRIP_SIDE: u32 = 256;
/// Zoom (punti dello schermo per mm) al minimo e al massimo.
const MIN_SCALE: f32 = 0.02;
const MAX_SCALE: f32 = 400.0;
/// Le scale del menu, in percentuale della misura vera.
const ZOOM_STOPS: [f32; 10] = [10.0, 25.0, 50.0, 75.0, 100.0, 150.0, 200.0, 400.0, 800.0, 1600.0];

/// Una richiesta della scheda all'app: passare il file a un'altra scheda, tornare alle miniature.
pub enum Request {
    /// Aggiungere i file all'elenco delle miniature.
    ToThumbnails(Vec<PathBuf>),
    /// Aprire il file in Modifica PDF; con `paths` il pannello Tracciati aperto.
    ToPdf {
        path: PathBuf,
        paths: bool,
    },
    /// Aprire il file in Modifica immagini; con `die` la fustella aperta.
    ToImages {
        path: PathBuf,
        die: bool,
    },
    ToNesting(Vec<PathBuf>),
    /// Tornare all'anteprima delle miniature, dove si era.
    BackToThumbnails,
}

/// Le schede del pannello a destra.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideTab {
    Info,
    Spots,
    Layers,
    Quotes,
}

/// Come si è arrivati al file, per le frecce.
#[derive(Clone, Debug, Default)]
struct Browse {
    files: Vec<PathBuf>,
    /// I file vengono dall'elenco delle miniature, non dalla cartella.
    from_job: bool,
}

enum State {
    Loading(Opening),
    Ready(Box<Opened>),
    Failed(String),
}

struct Doc {
    path: PathBuf,
    kind: ViewKind,
    page: usize,
    state: State,
    renderer: Option<Renderer>,
    browse: Browse,
    cad: CadChoice,
    /// Per le quote: cambia a ogni file e pagina.
    id: u64,
}

impl Doc {
    fn opened(&self) -> Option<&Opened> {
        match &self.state {
            State::Ready(o) => Some(o),
            _ => None,
        }
    }

    fn page_count(&self) -> usize {
        self.opened().map_or(1, |o| o.pages.len().max(1))
    }
}

/// Lo zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Zoom {
    /// La pagina intera nella finestra.
    Fit,
    /// La larghezza della pagina nella finestra.
    Width,
    /// Punti dello schermo per mm.
    Scale(f32),
}

pub struct ViewerModule {
    docs: Vec<Doc>,
    current: usize,
    pub status: String,
    pub requests: Vec<Request>,
    zoom: Zoom,
    /// La scala in uso adesso (punti per mm), anche quando è "Adatta".
    shown_scale: f32,
    /// Lo spostamento della pagina dal centro dell'anteprima, in punti.
    pan: Vec2,
    anchor: crate::pan::Anchor,
    side_open: bool,
    side_tab: SideTab,
    pub show_boxes: bool,
    pub show_rulers: bool,
    /// La tinta piatta evidenziata (nome com'è nel file).
    highlight: Option<Vec<u8>>,
    quoting: bool,
    quotes: crate::quotes::Quotes,
    pub print: Option<print::PrintPage>,
    /// Le scelte della pagina Stampa, ricordate nelle impostazioni.
    pub print_choice: print::Choice,
    /// La correzione delle stampanti calibrate (quelle dell'app).
    pub calibrations: std::collections::BTreeMap<String, winfiles_geom::Calibration>,
    /// Le scelte per i disegni CAD (fondo nero, unità quando mancano).
    pub cad_dark: bool,
    /// I nomi delle tinte di taglio conosciuti (quelli di Modifica PDF).
    pub inks: Vec<crate::pdf::tracciati::InkName>,
    /// Il punto del puntatore sulla pagina, in mm.
    pointer_mm: Option<[f64; 2]>,
    /// Ultimo cambio di pagina con la rotella, per non saltarne dieci con un colpo solo.
    last_flip: f64,
    next_id: u64,
    ctx: egui::Context,
    bar_height: f32,
}

impl ViewerModule {
    pub fn new(ctx: &egui::Context) -> Self {
        Self {
            docs: Vec::new(),
            current: 0,
            status: String::new(),
            requests: Vec::new(),
            zoom: Zoom::Fit,
            shown_scale: 1.0,
            pan: Vec2::ZERO,
            anchor: crate::pan::Anchor::default(),
            side_open: true,
            side_tab: SideTab::Info,
            show_boxes: true,
            show_rulers: true,
            highlight: None,
            quoting: false,
            quotes: crate::quotes::Quotes::default(),
            print: None,
            print_choice: print::Choice::default(),
            calibrations: std::collections::BTreeMap::new(),
            cad_dark: true,
            inks: Vec::new(),
            pointer_mm: None,
            last_flip: 0.0,
            next_id: 1,
            ctx: ctx.clone(),
            bar_height: 46.0,
        }
    }

    /// Il titolo della finestra con il file aperto.
    pub fn title(&self) -> Option<String> {
        self.doc()
            .map(|d| format!("{} — Visualizza — WinFiles", file_name(&d.path)))
    }

    fn doc(&self) -> Option<&Doc> {
        self.docs.get(self.current)
    }

    fn doc_mut(&mut self) -> Option<&mut Doc> {
        self.docs.get_mut(self.current)
    }

    /// Il file aperto adesso.
    pub fn current_path(&self) -> Option<&Path> {
        self.doc().map(|d| d.path.as_path())
    }

    /// Apre i file (uno per linguetta) e mostra il primo. Le frecce scorrono la cartella del
    /// primo file.
    pub fn open(&mut self, paths: &[PathBuf]) {
        let mut first = None;
        for p in paths {
            // Un percorso relativo (dalla riga di comando) diventa completo, per la cartella e le
            // frecce.
            let p = &std::path::absolute(p).unwrap_or_else(|_| p.clone());
            if p.is_dir() {
                continue;
            }
            let browse = Browse {
                files: folder_files(p),
                from_job: false,
            };
            let at = self.open_one(p, browse);
            first.get_or_insert(at);
        }
        if let Some(at) = first {
            self.show(at);
        } else if !paths.is_empty() {
            self.status = "Le cartelle non si visualizzano: trascina i file".into();
        }
    }

    /// Apre un file che arriva dalle miniature: le frecce scorrono `job` (i file del lavoro
    /// nell'ordine dell'elenco).
    pub fn open_from_job(&mut self, path: &Path, page: usize, job: Vec<PathBuf>) {
        let path = &std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let job: Vec<PathBuf> = job
            .into_iter()
            .map(|p| std::path::absolute(&p).unwrap_or(p))
            .collect();
        let at = self.open_one(
            path,
            Browse {
                files: job,
                from_job: true,
            },
        );
        if let Some(d) = self.docs.get_mut(at) {
            d.page = page;
            d.browse.from_job = true;
        }
        self.show(at);
    }

    /// Apre il file in una linguetta nuova, o ritrova quella dove è già aperto.
    fn open_one(&mut self, path: &Path, browse: Browse) -> usize {
        if let Some(i) = self.docs.iter().position(|d| d.path == path) {
            if let Some(d) = self.docs.get_mut(i)
                && (browse.from_job || d.browse.files.is_empty())
            {
                d.browse = browse;
            }
            return i;
        }
        let kind = ViewKind::of(path);
        let cad = CadChoice {
            dark: self.cad_dark,
            ..CadChoice::default()
        };
        let wake = self.ctx.clone();
        let opening = Opening::start(path.to_path_buf(), kind, cad.clone(), move || {
            wake.request_repaint()
        });
        self.next_id += 1;
        self.docs.push(Doc {
            path: path.to_path_buf(),
            kind,
            page: 0,
            state: State::Loading(opening),
            renderer: None,
            browse,
            cad,
            id: self.next_id,
        });
        self.docs.len() - 1
    }

    fn show(&mut self, at: usize) {
        if at != self.current {
            self.highlight = None;
        }
        self.current = at.min(self.docs.len().saturating_sub(1));
        self.zoom = Zoom::Fit;
        self.pan = Vec2::ZERO;
    }

    fn close(&mut self, at: usize) {
        if at < self.docs.len() {
            self.docs.remove(at);
        }
        if self.current >= self.docs.len() {
            self.current = self.docs.len().saturating_sub(1);
        } else if at < self.current {
            self.current -= 1;
        }
        self.highlight = None;
        self.zoom = Zoom::Fit;
        self.pan = Vec2::ZERO;
    }

    /// I file trascinati sulla scheda.
    pub fn dropped(&mut self, paths: Vec<PathBuf>) {
        let mut files = Vec::new();
        for p in paths {
            if p.is_dir() {
                files.extend(folder_files(&p.join("x")).into_iter().take(1));
            } else {
                files.push(p);
            }
        }
        self.open(&files);
    }

    /// Il file dopo o prima nella cartella (o nel lavoro).
    fn step_file(&mut self, forward: bool) {
        let Some(d) = self.doc() else {
            return;
        };
        let files = &d.browse.files;
        let Some(at) = files.iter().position(|p| *p == d.path) else {
            return;
        };
        let next = if forward {
            at.checked_add(1).filter(|i| *i < files.len())
        } else {
            at.checked_sub(1)
        };
        let Some(path) = next.and_then(|i| files.get(i)).cloned() else {
            self.status = if forward {
                "È l'ultimo file".into()
            } else {
                "È il primo file".into()
            };
            return;
        };
        let browse = d.browse.clone();
        // Si cambia il file nella stessa linguetta, come nei visualizzatori di foto.
        let kind = ViewKind::of(&path);
        let wake = self.ctx.clone();
        let cad = CadChoice {
            dark: self.cad_dark,
            ..CadChoice::default()
        };
        let opening = Opening::start(path.clone(), kind, cad.clone(), move || wake.request_repaint());
        self.next_id += 1;
        let id = self.next_id;
        if let Some(already) = self.docs.iter().position(|d| d.path == path) {
            self.show(already);
            return;
        }
        if let Some(d) = self.doc_mut() {
            *d = Doc {
                path,
                kind,
                page: 0,
                state: State::Loading(opening),
                renderer: None,
                browse,
                cad,
                id,
            };
        }
        self.highlight = None;
        self.zoom = Zoom::Fit;
        self.pan = Vec2::ZERO;
    }

    fn step_page(&mut self, forward: bool) -> bool {
        let Some(d) = self.doc_mut() else {
            return false;
        };
        let n = d.page_count();
        let next = if forward {
            (d.page + 1 < n).then_some(d.page + 1)
        } else {
            d.page.checked_sub(1)
        };
        match next {
            Some(p) => {
                d.page = p;
                self.pan = Vec2::ZERO;
                true
            }
            None => false,
        }
    }

    /// Le aperture finite e i disegni arrivati.
    fn receive(&mut self) {
        let ctx = self.ctx.clone();
        for d in &mut self.docs {
            if let State::Loading(o) = &d.state
                && let Some(r) = o.poll()
            {
                match r {
                    Ok(opened) => {
                        let wake = ctx.clone();
                        d.renderer = Some(Renderer::start(
                            opened.content.clone(),
                            format!("visualizza-{}", d.id),
                            move || wake.request_repaint(),
                        ));
                        d.page = d.page.min(opened.pages.len().saturating_sub(1));
                        d.state = State::Ready(Box::new(opened));
                    }
                    Err(e) => d.state = State::Failed(e),
                }
            }
            if let Some(r) = &mut d.renderer {
                r.receive(&ctx);
            }
        }
    }

    /// Rifà la pagina di un disegno CAD con le scelte nuove (layer, fondo, unità).
    fn redo_cad(&mut self) {
        let wake = self.ctx.clone();
        let Some(d) = self.doc_mut() else {
            return;
        };
        let Some(Facts::Cad(f)) = d.opened().map(|o| &o.facts) else {
            return;
        };
        let drawing = Arc::clone(&f.drawing);
        let opening = Opening::cad(drawing, d.cad.clone(), move || wake.request_repaint());
        d.state = State::Loading(opening);
        d.renderer = None;
    }

    // --- Tastiera e menu ------------------------------------------------------------------

    pub fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if self.print.is_some() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.print = None;
            }
            return;
        }
        let k = |m: egui::Modifiers, key: egui::Key| ctx.input_mut(|i| i.consume_key(m, key));
        use egui::{Key, Modifiers as M};
        if k(M::CTRL, Key::O) {
            self.pick_and_open();
        }
        if k(M::CTRL, Key::P) {
            self.open_print();
        }
        if k(M::CTRL, Key::W) && !self.docs.is_empty() {
            self.close(self.current);
        }
        if k(M::NONE, Key::ArrowRight) {
            self.step_file(true);
        }
        if k(M::NONE, Key::ArrowLeft) {
            self.step_file(false);
        }
        if k(M::NONE, Key::PageDown) {
            self.step_page(true);
        }
        if k(M::NONE, Key::PageUp) {
            self.step_page(false);
        }
        if k(M::CTRL, Key::Num0) {
            self.zoom = Zoom::Fit;
            self.pan = Vec2::ZERO;
        }
        if k(M::CTRL, Key::Plus) || k(M::CTRL, Key::Equals) {
            self.zoom_by(1.25);
        }
        if k(M::CTRL, Key::Minus) {
            self.zoom_by(0.8);
        }
        if k(M::NONE, Key::Escape) && self.highlight.is_some() {
            self.highlight = None;
        }
        // Passa a: Ctrl+1 … Ctrl+6, nell'ordine del menu.
        for (n, key) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6]
            .into_iter()
            .enumerate()
        {
            if k(M::CTRL, key) {
                self.send_to(n);
            }
        }
    }

    fn zoom_by(&mut self, k: f32) {
        self.zoom = Zoom::Scale((self.shown_scale * k).clamp(MIN_SCALE, MAX_SCALE));
    }

    pub fn file_menu(&mut self, ui: &mut egui::Ui) {
        let item = |ui: &mut egui::Ui, text: &str, keys: &str| {
            ui.add(egui::Button::new(text).shortcut_text(keys)).clicked()
        };
        if item(ui, "Apri…", "Ctrl+O") {
            ui.close();
            self.pick_and_open();
        }
        let has = self.doc().is_some();
        ui.add_enabled_ui(has, |ui| {
            if item(ui, "Stampa…", "Ctrl+P") {
                ui.close();
                self.open_print();
            }
            if item(ui, "Chiudi il file", "Ctrl+W") {
                ui.close();
                self.close(self.current);
            }
            ui.separator();
            if ui.button("Mostra nella cartella").clicked() {
                ui.close();
                self.reveal();
            }
            if ui.button("Apri con il programma di prima").clicked() {
                ui.close();
                self.open_external();
            }
        });
    }

    fn pick_and_open(&mut self) {
        let files = rfd::FileDialog::new()
            .set_title("Apri un file da visualizzare")
            .add_filter(
                "File che WinFiles mostra",
                &[
                    "pdf", "ai", "eps", "jpg", "jpeg", "png", "tif", "tiff", "webp", "bmp", "gif", "heic",
                    "heif", "psd", "psb", "dng", "cr2", "cr3", "nef", "nrw", "arw", "pef", "orf", "rw2",
                    "raf", "dxf", "dwg", "plt", "hpgl", "hpg", "svg", "svgz", "tga", "ico",
                ],
            )
            .pick_files()
            .unwrap_or_default();
        self.open(&files);
    }

    fn reveal(&mut self) {
        if let Some(p) = self.current_path().map(Path::to_path_buf)
            && let Err(e) = crate::shell::reveal(&p)
        {
            self.status = format!("Non riesco ad aprire la cartella: {e}");
        }
    }

    fn open_external(&mut self) {
        if let Some(p) = self.current_path().map(Path::to_path_buf) {
            match crate::defaults::open_with_previous(&p) {
                Ok(name) => self.status = format!("Aperto con {name}"),
                Err(e) => self.status = e,
            }
        }
    }

    fn open_print(&mut self) {
        let Some(d) = self.doc() else {
            return;
        };
        let Some(o) = d.opened() else {
            self.status = "Aspetta che il file sia aperto".into();
            return;
        };
        self.print = Some(print::PrintPage::new(
            &self.ctx,
            &d.path,
            d.kind,
            o,
            d.page,
            &d.cad,
            self.print_choice.clone(),
        ));
    }

    /// Le voci di "Passa a", nell'ordine: nome, se vale per questo file, la richiesta.
    fn send_options(&self) -> Vec<(&'static str, &'static str, bool)> {
        let kind = self.doc().map_or(ViewKind::Unsupported, |d| d.kind);
        let pdf_inside = self
            .doc()
            .is_some_and(|d| matches!(kind, ViewKind::Pdf) || (kind == ViewKind::Ai && is_pdf_file(&d.path)));
        let image = kind == ViewKind::Image;
        let nestable = matches!(
            kind,
            ViewKind::Pdf | ViewKind::Ai | ViewKind::Eps | ViewKind::Image
        );
        let listed = kind != ViewKind::Unsupported && kind != ViewKind::Cad;
        vec![
            (
                "Miniature",
                "Aggiunge il file all'elenco delle miniature (Ctrl+1)",
                listed,
            ),
            (
                "Modifica PDF",
                "Apre il file in Modifica PDF (Ctrl+2)",
                pdf_inside,
            ),
            (
                "Tracciati",
                "Apre Modifica PDF con lo strumento Tracciati (Ctrl+3)",
                pdf_inside,
            ),
            (
                "Modifica immagini",
                "Apre l'immagine in Modifica immagini (Ctrl+4)",
                image,
            ),
            (
                "Fustella",
                "Apre Modifica immagini con la fustella (Ctrl+5)",
                image,
            ),
            ("Nesting", "Aggiunge il file al nesting (Ctrl+6)", nestable),
        ]
    }

    fn send_to(&mut self, n: usize) {
        let Some(path) = self.current_path().map(Path::to_path_buf) else {
            return;
        };
        let Some((name, _, ok)) = self.send_options().get(n).copied() else {
            return;
        };
        if !ok {
            self.status = format!("Questo file non va in {name}");
            return;
        }
        self.requests.push(match n {
            0 => Request::ToThumbnails(vec![path]),
            1 => Request::ToPdf { path, paths: false },
            2 => Request::ToPdf { path, paths: true },
            3 => Request::ToImages { path, die: false },
            4 => Request::ToImages { path, die: true },
            _ => Request::ToNesting(vec![path]),
        });
    }

    // --- Barre -----------------------------------------------------------------------------

    pub fn control_bar(&mut self, ui: &mut egui::Ui) {
        self.receive();
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE.fill(t.app_bar).inner_margin(egui::Margin {
            left: 14,
            right: 14,
            top: 2,
            bottom: 6,
        });
        egui::Panel::top("barra-visualizza")
            .exact_size(self.bar_height)
            .frame(frame)
            .show(ui, |ui| {
                let area = ui.available_rect_before_wrap();
                let row = area.with_max_y(area.min.y + 38.0);
                let mut right = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(row)
                        .layout(egui::Layout::right_to_left(Align::Center)),
                );
                {
                    let ui = &mut right;
                    let can = self.doc().is_some_and(|d| d.opened().is_some());
                    let (fill, ink) = if can {
                        (t.primary, t.on_primary)
                    } else {
                        (t.button, t.text_disabled)
                    };
                    let b = egui::Button::image_and_text(
                        icons::image("printer", 18.0, ink),
                        egui::RichText::new("Stampa").font(semibold(14.0)).color(ink),
                    )
                    .fill(fill)
                    .min_size(vec2(112.0, 34.0));
                    if ui
                        .add_enabled(can, b)
                        .on_hover_text(
                            "Stampa il file a misura vera, adattato o in scala, con l'anteprima (Ctrl+P)",
                        )
                        .clicked()
                    {
                        self.open_print();
                    }
                }
                let right_w = right.min_rect().width();
                let mut left = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(area.with_max_x((area.max.x - right_w - 16.0).max(area.min.x)))
                        .layout(egui::Layout::left_to_right(Align::Center).with_main_wrap(true)),
                );
                {
                    let ui = &mut left;
                    ui.spacing_mut().interact_size.y = 34.0;
                    if self.doc().is_some_and(|d| d.browse.from_job)
                        && text_button(ui, "arrow-left", "Miniature", true)
                            .on_hover_text("Torna all'anteprima delle miniature, dove eri")
                            .clicked()
                    {
                        self.requests.push(Request::BackToThumbnails);
                    }
                    if text_button(ui, "folder-open", "Apri", true)
                        .on_hover_text("Apri un file da guardare (Ctrl+O)")
                        .clicked()
                    {
                        self.pick_and_open();
                    }
                    separator(ui);
                    self.nav_group(ui);
                    separator(ui);
                    self.zoom_group(ui);
                    separator(ui);
                    self.send_menu(ui);
                    let has = self.doc().is_some();
                    if text_button(ui, "external-link", "Apri con…", has)
                        .on_hover_text("Apri il file con il programma che lo apriva prima di WinFiles")
                        .clicked()
                    {
                        self.open_external();
                    }
                    if icons::button(ui, "folder-open", "Mostra nella cartella", false, has, 32.0).clicked() {
                        self.reveal();
                    }
                    separator(ui);
                    if icons::button(
                        ui,
                        "info",
                        "Misure e informazioni",
                        self.side_open && self.side_tab == SideTab::Info,
                        has,
                        32.0,
                    )
                    .clicked()
                    {
                        self.toggle_side(SideTab::Info);
                    }
                    let kind = self.doc().map(|d| d.kind);
                    if matches!(kind, Some(ViewKind::Pdf | ViewKind::Ai | ViewKind::Eps))
                        && icons::button(
                            ui,
                            "palette",
                            "Tinte piatte",
                            self.side_open && self.side_tab == SideTab::Spots,
                            true,
                            32.0,
                        )
                        .clicked()
                    {
                        self.toggle_side(SideTab::Spots);
                    }
                    if kind == Some(ViewKind::Cad)
                        && icons::button(
                            ui,
                            "layers",
                            "Layer del disegno",
                            self.side_open && self.side_tab == SideTab::Layers,
                            true,
                            32.0,
                        )
                        .clicked()
                    {
                        self.toggle_side(SideTab::Layers);
                    }
                    if icons::button(
                        ui,
                        "ruler",
                        "Quote: misura sul file",
                        self.side_open && self.side_tab == SideTab::Quotes,
                        has,
                        32.0,
                    )
                    .clicked()
                    {
                        self.toggle_side(SideTab::Quotes);
                    }
                }
                let height = left.min_rect().height().max(38.0) + 8.0;
                if (height - self.bar_height).abs() > 0.5 {
                    self.bar_height = height;
                    ui.ctx().request_repaint();
                }
            });
    }

    fn nav_group(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let (pos, total) = self
            .doc()
            .and_then(|d| {
                let at = d.browse.files.iter().position(|p| *p == d.path)?;
                Some((at, d.browse.files.len()))
            })
            .unwrap_or((0, 0));
        let tip_prev = if self.doc().is_some_and(|d| d.browse.from_job) {
            "File precedente del lavoro (←)"
        } else {
            "File precedente nella cartella (←)"
        };
        if icons::button(ui, "chevron-left", tip_prev, false, total > 0 && pos > 0, 32.0).clicked() {
            self.step_file(false);
        }
        let text = if total > 0 {
            format!("{} di {total}", pos + 1)
        } else {
            "–".into()
        };
        ui.label(
            egui::RichText::new(text)
                .font(semibold(13.0))
                .color(t.text_strong),
        );
        let tip_next = if self.doc().is_some_and(|d| d.browse.from_job) {
            "File seguente del lavoro (→)"
        } else {
            "File seguente nella cartella (→)"
        };
        if icons::button(ui, "chevron-right", tip_next, false, pos + 1 < total, 32.0).clicked() {
            self.step_file(true);
        }
    }

    fn zoom_group(&mut self, ui: &mut egui::Ui) {
        if icons::button(ui, "zoom-out", "Rimpicciolisci (Ctrl+−)", false, true, 32.0).clicked() {
            self.zoom_by(0.8);
        }
        let label = match self.zoom {
            Zoom::Fit => format!("Adatta ({:.0}%)", self.shown_scale / POINTS_PER_MM * 100.0),
            Zoom::Width => format!("Larghezza ({:.0}%)", self.shown_scale / POINTS_PER_MM * 100.0),
            Zoom::Scale(s) => format!("{:.0}%", s / POINTS_PER_MM * 100.0),
        };
        egui::ComboBox::from_id_salt("visualizza-zoom")
            .icon(crate::fluent::combo_icon)
            .selected_text(label)
            .width(140.0)
            .show_ui(ui, |ui| {
                if ui.selectable_label(self.zoom == Zoom::Fit, "Adatta alla finestra (Ctrl+0)").clicked() {
                    self.zoom = Zoom::Fit;
                    self.pan = Vec2::ZERO;
                }
                if ui.selectable_label(self.zoom == Zoom::Width, "Adatta alla larghezza").clicked() {
                    self.zoom = Zoom::Width;
                    self.pan = Vec2::ZERO;
                }
                ui.separator();
                for z in ZOOM_STOPS {
                    let s = z / 100.0 * POINTS_PER_MM;
                    let label = if (z - 100.0).abs() < 0.1 {
                        "100% (misura vera)".to_string()
                    } else {
                        format!("{z:.0}%")
                    };
                    if ui.selectable_label(self.zoom == Zoom::Scale(s), label).clicked() {
                        self.zoom = Zoom::Scale(s);
                    }
                }
            })
            .response
            .on_hover_text("Ctrl+rotella ingrandisce verso il puntatore; barra spaziatrice o tasto centrale per spostarti");
        if icons::button(ui, "zoom-in", "Ingrandisci (Ctrl++)", false, true, 32.0).clicked() {
            self.zoom_by(1.25);
        }
    }

    fn send_menu(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let has = self.doc().is_some();
        let options = self.send_options();
        let mut chosen = None;
        ui.add_enabled_ui(has, |ui| {
            egui::containers::menu::MenuButton::from_button(
                egui::Button::image_and_text(
                    icons::image("send", 16.0, if has { t.text_strong } else { t.text_disabled }),
                    egui::RichText::new("Passa a…").color(if has { t.text_strong } else { t.text_disabled }),
                )
                .min_size(vec2(0.0, 32.0)),
            )
            .ui(ui, |ui| {
                for (i, (name, tip, ok)) in options.iter().enumerate() {
                    let b = egui::Button::new(*name).shortcut_text(format!("Ctrl+{}", i + 1));
                    if ui.add_enabled(*ok, b).on_hover_text(*tip).clicked() {
                        chosen = Some(i);
                        ui.close();
                    }
                }
            })
            .0
            .on_hover_text("Manda il file a una delle altre schede");
        });
        if let Some(i) = chosen {
            self.send_to(i);
        }
    }

    pub fn status_bar(&mut self, ui: &mut egui::Ui, unit: Unit) {
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE
            .fill(t.app_bar)
            .inner_margin(egui::Margin::symmetric(16, 0));
        let text = self.doc().map(|d| {
            let mut s = format!("{}  ·  {}", file_name(&d.path), d.kind.label_for(&d.path));
            let n = d.page_count();
            if n > 1 {
                s.push_str(&format!("  ·  pagina {} di {n}", d.page + 1));
            }
            if let Some(p) = self.pointer_mm {
                s.push_str(&format!(
                    "  ·  x {}  y {}",
                    format_length(p[0], unit),
                    format_length(p[1], unit)
                ));
            }
            s
        });
        egui::Panel::bottom("barra-stato-visualizza")
            .exact_size(32.0)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(
                        egui::RichText::new(text.unwrap_or_else(|| "Nessun file aperto".into()))
                            .font(semibold(12.5))
                            .color(t.text),
                    );
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if !self.status.is_empty() {
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

    // --- Corpo -----------------------------------------------------------------------------

    pub fn body(&mut self, ui: &mut egui::Ui, unit: Unit) {
        self.receive();
        if self.print.is_some() {
            self.print_body(ui, unit);
            return;
        }
        let full = ui.max_rect();
        if self.docs.is_empty() {
            self.empty(ui, full);
            return;
        }
        let tabs_h = 34.0;
        let tabs = Rect::from_min_size(full.min, vec2(full.width(), tabs_h));
        self.tabs(ui, tabs);
        let rest = full.with_min_y(tabs.bottom() + 6.0);
        let side_w = if self.side_open {
            (rest.width() * 0.26).clamp(260.0, 340.0)
        } else {
            0.0
        };
        let strip_w = if self.doc().is_some_and(|d| d.page_count() > 1) {
            120.0
        } else {
            0.0
        };
        let strip = Rect::from_min_size(rest.min, vec2(strip_w, rest.height()));
        let side = Rect::from_min_max(pos2(rest.right() - side_w, rest.top()), rest.max);
        let view = Rect::from_min_max(
            pos2(
                if strip_w > 0.0 {
                    strip.right() + 10.0
                } else {
                    rest.left()
                },
                rest.top(),
            ),
            pos2(
                if side_w > 0.0 {
                    side.left() - 10.0
                } else {
                    rest.right()
                },
                rest.bottom(),
            ),
        );
        if strip_w > 0.0 {
            self.page_strip(ui, strip);
        }
        self.preview(ui, view, unit);
        if side_w > 0.0 {
            self.side_panel(ui, side, unit);
        }
    }

    fn empty(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink(40.0))
                .layout(egui::Layout::top_down(Align::Center)),
        );
        child.add_space((rect.height() * 0.3).max(20.0));
        child.add(icons::image("eye", 48.0, t.text_dim));
        child.add_space(12.0);
        child.label(
            egui::RichText::new("Trascina qui un file da guardare")
                .font(semibold(18.0))
                .color(t.text_strong),
        );
        child.add_space(6.0);
        child.label(
            egui::RichText::new(
                "PDF, Illustrator, EPS, immagini, DXF e DWG. Si vedono le misure vere e le tinte piatte; il file non cambia.",
            )
            .color(t.text_dim),
        );
        child.add_space(14.0);
        if child
            .add(
                egui::Button::image_and_text(
                    icons::image("folder-open", 17.0, t.on_primary),
                    egui::RichText::new("Apri un file…")
                        .font(semibold(13.5))
                        .color(t.on_primary),
                )
                .fill(t.primary)
                .min_size(vec2(150.0, 34.0)),
            )
            .clicked()
        {
            self.pick_and_open();
        }
    }

    /// Le linguette dei file aperti, come in un browser.
    fn tabs(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let t = Tokens::get(ui.ctx());
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        child.spacing_mut().item_spacing.x = 4.0;
        let mut pick = None;
        let mut close = None;
        egui::ScrollArea::horizontal()
            .id_salt("visualizza-linguette")
            .show(&mut child, |ui| {
                ui.horizontal(|ui| {
                    for (i, d) in self.docs.iter().enumerate() {
                        let active = i == self.current;
                        let name = file_name(&d.path);
                        let short: String = if name.chars().count() > 34 {
                            format!("{}…", name.chars().take(33).collect::<String>())
                        } else {
                            name.clone()
                        };
                        let font = if active {
                            semibold(13.0)
                        } else {
                            egui::FontId::proportional(13.0)
                        };
                        let color = if active { t.text_strong } else { t.text };
                        let galley = ui.painter().layout_no_wrap(short, font, color);
                        let w = galley.size().x + 18.0 + 26.0;
                        let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                        let fill = if active {
                            t.panel
                        } else if resp.hovered() {
                            t.hover
                        } else {
                            t.app_bar
                        };
                        ui.painter().rect_filled(r, t.radius, fill);
                        if active {
                            ui.painter().rect_stroke(
                                r,
                                t.radius,
                                Stroke::new(1.0, t.border),
                                egui::StrokeKind::Inside,
                            );
                            ui.painter().hline(
                                r.x_range().shrink(6.0),
                                r.bottom() - 1.5,
                                Stroke::new(2.0, t.accent),
                            );
                        }
                        ui.painter().galley(
                            pos2(r.left() + 10.0, r.center().y - galley.size().y / 2.0),
                            galley,
                            color,
                        );
                        let x =
                            Rect::from_center_size(pos2(r.right() - 14.0, r.center().y), Vec2::splat(18.0));
                        let x_resp = ui.interact(x, ui.id().with(("chiudi", i)), Sense::click());
                        if x_resp.hovered() {
                            ui.painter().rect_filled(x, 4.0, t.pressed);
                        }
                        icons::paint(
                            ui,
                            "x",
                            x.shrink(3.0),
                            if x_resp.hovered() {
                                t.text_strong
                            } else {
                                t.text_dim
                            },
                        );
                        if x_resp.clicked() {
                            close = Some(i);
                        } else if resp.clicked() {
                            pick = Some(i);
                        }
                        if resp.middle_clicked() {
                            close = Some(i);
                        }
                        resp.on_hover_text(d.path.display().to_string());
                    }
                });
            });
        if let Some(i) = close {
            self.close(i);
        } else if let Some(i) = pick {
            self.show(i);
        }
    }

    fn page_strip(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        let Some(d) = self.docs.get_mut(self.current) else {
            return;
        };
        let Some(pages) = d.opened().map(|o| o.pages.clone()) else {
            return;
        };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(8.0)));
        let mut pick = None;
        egui::ScrollArea::vertical()
            .id_salt(("visualizza-pagine", d.id))
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                let w = ui.available_width();
                for (i, p) in pages.iter().enumerate() {
                    let aspect = (p.size.h / p.size.w.max(1e-6)) as f32;
                    let tw = w - 16.0;
                    let th = (tw * aspect).min(tw * 1.6);
                    let tw = th / aspect;
                    let (r, resp) = ui.allocate_exact_size(vec2(w, th + 26.0), Sense::click());
                    let paper =
                        Rect::from_center_size(pos2(r.center().x, r.top() + 4.0 + th / 2.0), vec2(tw, th));
                    if ui.is_rect_visible(r)
                        && let Some(rend) = &mut d.renderer
                    {
                        rend.want_base(i, STRIP_SIDE);
                        ui.painter().rect_filled(paper, 0.0, Color32::WHITE);
                        if let Some((_, tex)) = rend.base.get(&i) {
                            ui.painter().image(tex.id(), paper, unit_rect(), Color32::WHITE);
                        }
                    }
                    let selected = i == d.page;
                    ui.painter().rect_stroke(
                        paper.expand(if selected { 2.0 } else { 0.5 }),
                        2.0,
                        Stroke::new(
                            if selected { 2.0 } else { 1.0 },
                            if selected { t.accent } else { t.border },
                        ),
                        egui::StrokeKind::Outside,
                    );
                    ui.painter().text(
                        pos2(r.center().x, paper.bottom() + 12.0),
                        egui::Align2::CENTER_CENTER,
                        (i + 1).to_string(),
                        egui::FontId::proportional(12.0),
                        if selected { t.text_strong } else { t.text_dim },
                    );
                    if resp.clicked() {
                        pick = Some(i);
                    }
                }
            });
        if let Some(i) = pick {
            d.page = i;
            self.pan = Vec2::ZERO;
        }
    }

    /// L'anteprima: la pagina con zoom, spostamento, riquadri, righelli, tinte e quote.
    fn preview(&mut self, ui: &mut egui::Ui, rect: Rect, unit: Unit) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        let (ruler_top, ruler_left) = if self.show_rulers {
            (22.0, 34.0)
        } else {
            (0.0, 0.0)
        };
        let board = Rect::from_min_max(
            rect.min + vec2(6.0 + ruler_left, 6.0 + ruler_top),
            rect.max - vec2(6.0, 6.0),
        );
        let pasteboard = if self.doc().is_some_and(|d| d.kind == ViewKind::Cad && d.cad.dark) {
            Color32::from_rgb(16, 16, 18)
        } else {
            t.pasteboard
        };
        ui.painter().rect_filled(board, t.radius, pasteboard);
        let Some(d) = self.docs.get(self.current) else {
            return;
        };
        let view = match &d.state {
            State::Loading(_) => {
                ui.put(
                    Rect::from_center_size(board.center(), vec2(220.0, 40.0)),
                    egui::Spinner::new().size(28.0).color(t.accent),
                );
                return;
            }
            State::Failed(e) => {
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(Rect::from_center_size(
                            board.center(),
                            vec2(board.width().min(520.0), 160.0),
                        ))
                        .layout(egui::Layout::top_down(Align::Center)),
                );
                child.add(icons::image("triangle-alert", 32.0, t.warning));
                child.add_space(8.0);
                child.label(
                    egui::RichText::new(format!("Non riesco a mostrare {}", file_name(&d.path)))
                        .font(semibold(15.0))
                        .color(t.text_strong),
                );
                child.label(egui::RichText::new(e).color(t.text_dim));
                child.add_space(10.0);
                if child.button("Apri con il programma di prima").clicked() {
                    self.open_external();
                }
                return;
            }
            State::Ready(o) => o.pages.get(d.page).copied(),
        };
        let Some(page) = view else {
            return;
        };
        let size = page.size;
        let (pw, ph) = (size.w.max(1e-3) as f32, size.h.max(1e-3) as f32);
        let fit = ((board.width() - 40.0) / pw)
            .min((board.height() - 40.0) / ph)
            .max(MIN_SCALE);
        let width_fit = ((board.width() - 40.0) / pw).max(MIN_SCALE);
        let mut scale = match self.zoom {
            Zoom::Fit => fit,
            Zoom::Width => width_fit,
            Zoom::Scale(s) => s,
        };
        let resp = ui.interact(board, ui.id().with("visualizza-tavola"), Sense::click_and_drag());
        let hand = crate::pan::hand(ui, board);
        // Ctrl+rotella: ingrandisce verso il puntatore.
        if let Some(z) = crate::app::wheel_zoom(ui, board) {
            let new = (scale * z).clamp(MIN_SCALE, MAX_SCALE);
            if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
                let paper_min = board.center() + self.pan - vec2(pw, ph) * scale / 2.0;
                let now = ui.input(|i| i.time);
                let point = self.anchor.point(p, now, || (p - paper_min) / scale);
                // Il punto della pagina sotto il puntatore resta lì.
                let new_min = p - point * new;
                self.pan = new_min + vec2(pw, ph) * new / 2.0 - board.center();
            }
            scale = new;
            self.zoom = Zoom::Scale(new);
        }
        // Trascinare sposta la pagina (le quote, se attive, usano il trascinamento).
        if let Some(dv) = hand {
            self.pan += dv;
        } else if !self.quoting && resp.dragged() {
            self.pan += resp.drag_delta();
        }
        // La rotella scorre la pagina; arrivati in fondo (o in cima) passa alla pagina dopo.
        let wheel = if ui.rect_contains_pointer(board) && !ui.input(|i| i.modifiers.command) {
            ui.input(|i| i.smooth_scroll_delta)
        } else {
            Vec2::ZERO
        };
        let shown = vec2(pw, ph) * scale;
        let before = crate::pan::clamp(self.pan, shown, board.size(), 20.0);
        self.pan = crate::pan::clamp(self.pan + wheel, shown, board.size(), 20.0);
        let now = ui.input(|i| i.time);
        if wheel.y.abs() > 0.5 && (self.pan - before).length() < 0.01 && now - self.last_flip > 0.45 {
            let forward = wheel.y < 0.0;
            if self.step_page(forward) {
                self.last_flip = now;
                self.pan = vec2(0.0, if forward { 1e6 } else { -1e6 });
                self.pan = crate::pan::clamp(self.pan, shown, board.size(), 20.0);
            }
        }
        self.shown_scale = scale;
        let paper = Rect::from_center_size(board.center() + self.pan, shown);
        let Some(d) = self.docs.get_mut(self.current) else {
            return;
        };
        let painter = ui.painter_at(board);
        painter.rect_filled(
            paper.translate(vec2(0.0, 2.0)).expand(1.0),
            2.0,
            Color32::from_black_alpha(50),
        );
        painter.rect_filled(paper, 0.0, Color32::WHITE);
        let ppp = ui.ctx().pixels_per_point();
        let needed = (shown.x.max(shown.y) * ppp).ceil() as u32;
        let side = SIDES.into_iter().find(|s| *s >= needed).unwrap_or(4096);
        let page_no = d.page;
        if let Some(rend) = &mut d.renderer {
            rend.want_base(page_no, side);
            match rend.base.get(&page_no) {
                Some((_, tex)) => {
                    painter.image(tex.id(), paper, unit_rect(), Color32::WHITE);
                }
                None => {
                    if let Some(e) = rend.failed.get(&page_no) {
                        painter.text(
                            paper.center(),
                            egui::Align2::CENTER_CENTER,
                            format!("Pagina non disegnabile: {e}"),
                            egui::FontId::proportional(13.0),
                            t.error,
                        );
                    }
                }
            }
            // Ingrandita oltre la pagina intera: il pezzo che si vede, nitido.
            let base_side = rend.base.get(&page_no).map_or(0, |(s, _)| *s);
            if needed > base_side.max(1) + base_side / 5 {
                let visible = paper.intersect(board);
                if visible.is_positive() {
                    let to_mm = |p: Pos2| {
                        [
                            f64::from((p.x - paper.left()) / scale),
                            f64::from((p.y - paper.top()) / scale),
                        ]
                    };
                    let a = to_mm(visible.min);
                    let b = to_mm(visible.max);
                    let px_per_mm = f64::from(scale * ppp);
                    let region = [a[0], a[1], b[0] - a[0], b[1] - a[1]];
                    let covered = rend
                        .tile
                        .as_ref()
                        .is_some_and(|(tl, _, _)| tl.covers(page_no, region, px_per_mm));
                    let still =
                        !resp.dragged() && hand.is_none_or(|v| v == Vec2::ZERO) && wheel == Vec2::ZERO;
                    if !covered && still {
                        // Un po' di margine attorno, entro il lato massimo.
                        let grow_w = (region[2] * 0.25)
                            .min(((f64::from(load::MAX_SIDE) / px_per_mm) - region[2]).max(0.0) / 2.0);
                        let grow_h = (region[3] * 0.25)
                            .min(((f64::from(load::MAX_SIDE) / px_per_mm) - region[3]).max(0.0) / 2.0);
                        let x0 = (region[0] - grow_w).max(0.0);
                        let y0 = (region[1] - grow_h).max(0.0);
                        let x1 = (region[0] + region[2] + grow_w).min(size.w);
                        let y1 = (region[1] + region[3] + grow_h).min(size.h);
                        let max_mm = f64::from(load::MAX_SIDE) / px_per_mm;
                        let tile = Tile {
                            page: page_no,
                            region: [x0, y0, (x1 - x0).min(max_mm), (y1 - y0).min(max_mm)],
                            px_per_mm,
                        };
                        rend.want_tile(tile);
                    }
                    if let Some((tl, actual, tex)) = &rend.tile
                        && tl.page == page_no
                        && (tl.px_per_mm / px_per_mm - 1.0).abs() < 0.6
                    {
                        let r = Rect::from_min_size(
                            pos2(
                                paper.left() + actual[0] as f32 * scale,
                                paper.top() + actual[1] as f32 * scale,
                            ),
                            vec2(actual[2] as f32 * scale, actual[3] as f32 * scale),
                        );
                        painter.image(tex.id(), r, unit_rect(), Color32::WHITE);
                    }
                    if rend.tile_pending() {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(60));
                    }
                }
            }
        }
        let mm_to = |x: f64, y: f64| pos2(paper.left() + x as f32 * scale, paper.top() + y as f32 * scale);
        // La tinta evidenziata: il resto sbiadito, i tracciati della tinta nel loro colore.
        if let Some(name) = self.highlight.clone() {
            if let Some(rend) = &mut d.renderer {
                rend.want_scan(page_no);
            }
            painter.rect_filled(paper, 0.0, Color32::from_rgba_unmultiplied(255, 255, 255, 205));
            let swatch = spot_swatch(d, &name);
            if let Some(Ok(scan)) = d.renderer.as_ref().and_then(|r| r.scans.get(&page_no)) {
                for p in &scan.paths {
                    let fill = p.fill.as_ref().is_some_and(|f| ink_is(&f.ink, &name));
                    let line = p.line.as_ref().filter(|l| ink_is(&l.ink, &name));
                    if !fill && line.is_none() {
                        continue;
                    }
                    let w = line.map_or(1.5, |l| (l.width as f32 * scale).max(1.5));
                    for part in &p.parts {
                        let pts: Vec<Pos2> = part.points.iter().map(|q| mm_to(q[0], q[1])).collect();
                        if pts.len() < 2 {
                            continue;
                        }
                        if fill {
                            painter.add(egui::Shape::closed_line(
                                pts.clone(),
                                Stroke::new(w.max(2.0), swatch),
                            ));
                        }
                        if line.is_some() {
                            if part.closed {
                                painter.add(egui::Shape::closed_line(pts, Stroke::new(w, swatch)));
                            } else {
                                painter.add(egui::Shape::line(pts, Stroke::new(w, swatch)));
                            }
                        }
                    }
                }
            }
        }
        // I riquadri: al vivo (verde) e abbondanza (blu).
        if self.show_boxes {
            let dash = |r: [f64; 4], c: Color32| {
                let a = mm_to(r[0], r[1]);
                let b = mm_to(r[0] + r[2], r[1] + r[3]);
                let rr = Rect::from_two_pos(a, b);
                let pts = [
                    rr.left_top(),
                    rr.right_top(),
                    rr.right_bottom(),
                    rr.left_bottom(),
                    rr.left_top(),
                ];
                for w in pts.windows(2) {
                    if let [p, q] = w {
                        painter.extend(egui::Shape::dashed_line(&[*p, *q], Stroke::new(1.2, c), 6.0, 4.0));
                    }
                }
            };
            if let Some(b) = page.bleed {
                dash(b, Color32::from_rgb(37, 99, 235));
            }
            if let Some(tr) = page.trim {
                dash(tr, Color32::from_rgb(22, 163, 74));
            }
        }
        // Il puntatore in mm, dall'angolo in alto a sinistra della pagina.
        // I disegni CAD grandi si guardano ridotti: righelli, puntatore e quote dicono i mm veri.
        let real_k = match d.opened().map(|o| &o.facts) {
            Some(Facts::Cad(f)) => f.unit_mm / f.page_scale,
            _ => 1.0,
        };
        self.pointer_mm = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|p| board.contains(*p))
            .map(|p| {
                [
                    f64::from((p.x - paper.left()) / scale) * real_k,
                    f64::from((p.y - paper.top()) / scale) * real_k,
                ]
            });
        // Le quote.
        if self.quoting {
            let mm_per_unit = real_k;
            self.quotes.prepare(d.id * 10_000 + page_no as u64, mm_per_unit);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(board));
            child.set_clip_rect(board);
            if hand.is_none() {
                let qid = child.id().with("quote-visualizza");
                self.quotes.interact(&mut child, qid, paper, pw, ph);
            }
            self.quotes.paint(&child, paper, pw, unit);
        }
        if self.show_rulers {
            rulers(
                ui,
                rect,
                board,
                paper,
                scale / real_k as f32,
                unit,
                self.pointer_mm,
            );
        }
        // Il doppio clic rimette la pagina intera.
        if resp.double_clicked() && !self.quoting {
            self.zoom = Zoom::Fit;
            self.pan = Vec2::ZERO;
        }
    }

    fn side_panel(&mut self, ui: &mut egui::Ui, rect: Rect, unit: Unit) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        let kind = self.doc().map_or(ViewKind::Unsupported, |d| d.kind);
        let mut tabs = vec![(SideTab::Info, "Misure")];
        match kind {
            ViewKind::Pdf | ViewKind::Ai | ViewKind::Eps => tabs.push((SideTab::Spots, "Tinte")),
            ViewKind::Cad => tabs.push((SideTab::Layers, "Layer")),
            _ => {}
        }
        tabs.push((SideTab::Quotes, "Quote"));
        if !tabs.iter().any(|(s, _)| *s == self.side_tab) {
            self.side_tab = SideTab::Info;
        }
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 44.0)).shrink2(vec2(10.0, 6.0));
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        let mut tab = self.side_tab;
        crate::app::segmented(&mut bar, "visualizza-pannello", &mut tab, &tabs);
        if tab != self.side_tab {
            self.side_tab = tab;
        }
        self.quoting = self.side_tab == SideTab::Quotes;
        bar.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if icons::button(ui, "x", "Chiudi il pannello", false, true, 26.0).clicked() {
                self.side_open = false;
                self.quoting = false;
            }
        });
        let body = Rect::from_min_max(
            pos2(rect.left() + 14.0, header.bottom() + 6.0),
            rect.max - vec2(14.0, 10.0),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(body)
                .layout(egui::Layout::top_down(Align::Min)),
        );
        egui::ScrollArea::vertical()
            .id_salt(("visualizza-lato", self.side_tab as u8))
            .auto_shrink([false, false])
            .show(&mut child, |ui| match self.side_tab {
                SideTab::Info => self.info_panel(ui, unit),
                SideTab::Spots => self.spots_panel(ui, unit),
                SideTab::Layers => self.layers_panel(ui, unit),
                SideTab::Quotes => {
                    if let Some(a) = self.quotes.panel(ui, unit) {
                        match a {
                            crate::quotes::PanelAction::Print => self.open_print(),
                            crate::quotes::PanelAction::Save => {
                                self.status =
                                    "Per salvare le quote apri il file in Modifica PDF o Modifica immagini"
                                        .into();
                            }
                        }
                    }
                }
            });
        let _ = t;
    }

    fn print_body(&mut self, ui: &mut egui::Ui, unit: Unit) {
        let Some(mut page) = self.print.take() else {
            return;
        };
        let outcome = page.show(ui, unit, &self.calibrations);
        self.print_choice = page.choice.clone();
        match outcome {
            print::Outcome::Keep => self.print = Some(page),
            print::Outcome::Close => {}
            print::Outcome::Status(s) => self.status = s,
        }
    }

    /// Apre il pannello a destra su `tab`, o lo chiude se è già lì.
    fn toggle_side(&mut self, tab: SideTab) {
        if self.side_open && self.side_tab == tab {
            self.side_open = false;
            self.quoting = false;
        } else {
            self.side_open = true;
            self.side_tab = tab;
        }
    }
}

/// Le tinte con lo stesso nome nel file.
fn ink_is(ink: &winfiles_pdf_edit::Ink, raw: &[u8]) -> bool {
    matches!(ink, winfiles_pdf_edit::Ink::Spot { name, .. } if name.as_bytes() == raw || String::from_utf8_lossy(raw) == *name)
}

/// Il colore di una tinta per evidenziarla: quello del file, o un magenta acceso.
fn spot_swatch(d: &Doc, raw: &[u8]) -> Color32 {
    if let Some(Facts::Pdf { spots: Ok(list), .. }) = d.opened().map(|o| &o.facts)
        && let Some(s) = list.iter().find(|s| s.raw == raw)
        && let Some([r, g, b]) = s.swatch
    {
        // Le tinte quasi bianche (Bianco, vernici) non si vedrebbero sul bianco.
        if u32::from(r) + u32::from(g) + u32::from(b) < 690 {
            return Color32::from_rgb(r, g, b);
        }
    }
    Color32::from_rgb(219, 39, 119)
}

/// I righelli in alto e a sinistra, in mm (o cm) dall'angolo della pagina.
fn rulers(
    ui: &egui::Ui,
    rect: Rect,
    board: Rect,
    paper: Rect,
    scale: f32,
    unit: Unit,
    pointer: Option<[f64; 2]>,
) {
    let t = Tokens::get(ui.ctx());
    let top = Rect::from_min_max(
        pos2(board.left(), rect.top() + 6.0),
        pos2(board.right(), board.top() - 2.0),
    );
    let left = Rect::from_min_max(
        pos2(rect.left() + 6.0, board.top()),
        pos2(board.left() - 2.0, board.bottom()),
    );
    let painter = ui.painter();
    painter.rect_filled(top, 3.0, t.panel_darker);
    painter.rect_filled(left, 3.0, t.panel_darker);
    // Il passo delle tacche: il primo che lascia almeno 60 punti fra due numeri.
    let steps = [
        1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0, 20_000.0,
        50_000.0, 100_000.0,
    ];
    let step = steps
        .iter()
        .copied()
        .find(|s| s * scale >= 60.0)
        .unwrap_or(100_000.0);
    let minor = step
        / if step >= 10.0 && (step / 10.0).fract() == 0.0 {
            10.0
        } else {
            5.0
        };
    let font = egui::FontId::proportional(10.0);
    // Dal metro in su i numeri sono metri: "12 m" sta nel righello, "12.000" no.
    let label = |v: f32| {
        let mm = f64::from(v);
        if step >= 1000.0 {
            format!("{} m", format_number(mm / 1000.0, 0, true))
        } else {
            format_number(
                mm / unit.mm(),
                if unit == Unit::Cm && step < 10.0 { 1 } else { 0 },
                true,
            )
        }
    };
    let tp = ui.painter_at(top);
    let start = ((board.left() - paper.left()) / scale / minor).floor() * minor;
    let mut v = start;
    while paper.left() + v * scale <= board.right() {
        let x = paper.left() + v * scale;
        let major = (v / step).round() * step == v || ((v / step) - (v / step).round()).abs() < 1e-4;
        let h = if major { 9.0 } else { 4.0 };
        tp.vline(x, (top.bottom() - h)..=top.bottom(), Stroke::new(1.0, t.text_dim));
        if major {
            tp.text(
                pos2(x + 2.0, top.top() + 1.0),
                egui::Align2::LEFT_TOP,
                label(v),
                font.clone(),
                t.text_dim,
            );
        }
        v += minor;
    }
    let lp = ui.painter_at(left);
    let start = ((board.top() - paper.top()) / scale / minor).floor() * minor;
    let mut v = start;
    while paper.top() + v * scale <= board.bottom() {
        let y = paper.top() + v * scale;
        let major = ((v / step) - (v / step).round()).abs() < 1e-4;
        let w = if major { 9.0 } else { 4.0 };
        lp.hline((left.right() - w)..=left.right(), y, Stroke::new(1.0, t.text_dim));
        if major {
            lp.text(
                pos2(left.left() + 2.0, y + 2.0),
                egui::Align2::LEFT_TOP,
                label(v),
                font.clone(),
                t.text_dim,
            );
        }
        v += minor;
    }
    if let Some([x, y]) = pointer {
        let px = paper.left() + x as f32 * scale;
        let py = paper.top() + y as f32 * scale;
        tp.vline(px, top.y_range(), Stroke::new(1.0, t.accent));
        lp.hline(left.x_range(), py, Stroke::new(1.0, t.accent));
    }
}

fn unit_rect() -> Rect {
    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))
}

fn separator(ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(13.0, 24.0), Sense::hover());
    ui.painter()
        .vline(r.center().x, r.y_range(), Stroke::new(1.0, t.divider));
}

fn text_button(ui: &mut egui::Ui, icon: &str, text: &str, enabled: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let tint = if enabled { t.text_strong } else { t.text_disabled };
    ui.add_enabled(
        enabled,
        egui::Button::image_and_text(
            icons::image(icon, 17.0, tint),
            egui::RichText::new(text).color(tint),
        )
        .min_size(vec2(0.0, 32.0)),
    )
}

pub(crate) fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// Il file ha l'intestazione di un PDF (gli AI salvati con il PDF dentro).
fn is_pdf_file(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 1024];
    std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .is_ok_and(|n| head.get(..n).is_some_and(|h| h.windows(5).any(|w| w == b"%PDF-")))
}

/// I file della cartella di `path` che la scheda sa mostrare, in ordine di nome come Esplora file.
fn folder_files(path: &Path) -> Vec<PathBuf> {
    let Some(dir) = path.parent() else {
        return vec![path.to_path_buf()];
    };
    let Ok(read) = std::fs::read_dir(dir) else {
        return vec![path.to_path_buf()];
    };
    let mut files: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && ViewKind::of(p) != ViewKind::Unsupported)
        .take(20_000)
        .collect();
    files.sort_by_cached_key(|p| natural_key(&file_name(p)));
    if !files.iter().any(|p| p == path) && path.is_file() {
        files.insert(0, path.to_path_buf());
    }
    files
}

/// Il nome per l'ordine "naturale": senza maiuscole, i numeri come numeri (2 prima di 10).
fn natural_key(name: &str) -> Vec<(u8, u64, String)> {
    let mut out = Vec::new();
    let mut digits = String::new();
    let mut text = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_digit() {
            if !text.is_empty() {
                out.push((1, 0, std::mem::take(&mut text)));
            }
            digits.push(c);
        } else {
            if !digits.is_empty() {
                out.push((0, digits.parse().unwrap_or(u64::MAX), std::mem::take(&mut digits)));
            }
            text.push(c);
        }
    }
    if !digits.is_empty() {
        out.push((0, digits.parse().unwrap_or(u64::MAX), digits));
    }
    if !text.is_empty() {
        out.push((1, 0, text));
    }
    out
}

/// Il testo delle misure da copiare: nome, formato, al vivo, m².
fn measures_text(name: &str, page: &PageView, unit: Unit) -> String {
    let mut s = format!("{name}\nFormato: {}", format_size(page.size, unit));
    if page.trim.is_some() {
        s.push_str(&format!("\nAl vivo: {}", format_size(page.finished(), unit)));
    }
    s.push_str(&format!("\nArea: {}", format_area(area_m2(page.finished()))));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_ordine_dei_numeri_e_naturale() {
        let mut names = vec!["foto10.jpg", "foto2.jpg", "Foto1.jpg", "alfa.pdf"];
        names.sort_by_cached_key(|n| natural_key(n));
        assert_eq!(names, vec!["alfa.pdf", "Foto1.jpg", "foto2.jpg", "foto10.jpg"]);
    }

    #[test]
    fn i_tipi_si_riconoscono_dal_nome() {
        assert_eq!(ViewKind::of(Path::new("a.DXF")), ViewKind::Cad);
        assert_eq!(ViewKind::of(Path::new("a.dwg")), ViewKind::Cad);
        assert_eq!(ViewKind::of(Path::new("a.pdf")), ViewKind::Pdf);
        assert_eq!(ViewKind::of(Path::new("a.nef")), ViewKind::Image);
        assert_eq!(ViewKind::of(Path::new("a.docx")), ViewKind::Unsupported);
    }

    #[test]
    fn il_pezzo_copre_solo_alla_sua_scala() {
        let t = Tile {
            page: 0,
            region: [10.0, 10.0, 100.0, 100.0],
            px_per_mm: 20.0,
        };
        assert!(t.covers(0, [20.0, 20.0, 50.0, 50.0], 20.0));
        assert!(!t.covers(1, [20.0, 20.0, 50.0, 50.0], 20.0));
        assert!(!t.covers(0, [5.0, 20.0, 50.0, 50.0], 20.0));
        assert!(!t.covers(0, [20.0, 20.0, 50.0, 50.0], 40.0));
    }
}
