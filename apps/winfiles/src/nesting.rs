//! Il modulo Nesting (ADR 0027): dispone le copie dei PDF sagomati su lastre o su un rotolo e
//! salva un PDF per il RIP (Onyx), con la grafica ritagliata sulla fustella più l'abbondanza e le
//! tinte piatte com'erano. Il motore è `winfiles-nest` (dal pannello per Illustrator); qui ci sono
//! l'elenco dei file con le copie, le impostazioni, l'anteprima dei fogli e il salvataggio.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use egui::{Align, Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};
use winfiles_geom::{Size, Unit, format_area, format_length, format_number, format_size};
use winfiles_model::FileKind;
use winfiles_nest::{Item, Options, Outcome, Placement, Stock};
use winfiles_pdf_edit::{Contours, NestSheet, PdfFile, SheetPiece};

use crate::app::{card, segmented};
use crate::icons;
use crate::pdf::tracciati::{self, Finish, InkName, Role};
use crate::theme::{Tokens, semibold};

/// Lato lungo delle immagini delle pagine, in pixel.
const THUMB_SIDE: u32 = 512;
/// Pagine lette da un solo PDF.
const MAX_PAGES: usize = 200;
/// Altezza minima di una riga dell'elenco (la miniatura) e quella della riga dei campi.
const CELL_MIN: f32 = 76.0;
const FIELDS_H: f32 = 30.0;
/// Le scale dei file disegnati in piccolo: 1:N diventa N volte più grande sul foglio.
const SCALES: [u32; 9] = [1, 2, 4, 5, 10, 20, 25, 50, 100];
const BAR_ROW: f32 = 38.0;
/// Copie di una pagina.
const MAX_COPIES: usize = 999;

/// Lastre o rotolo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Support {
    #[default]
    Sheet,
    Roll,
}

/// Le rotazioni provate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Turn {
    None,
    #[default]
    Right,
    Deg45,
    Deg15,
}

impl Turn {
    fn step(self) -> f64 {
        match self {
            Turn::None => 0.0,
            Turn::Right => 90.0,
            Turn::Deg45 => 45.0,
            Turn::Deg15 => 15.0,
        }
    }
}

/// Le scelte del nesting, ricordate fra un'apertura e l'altra (misure in mm).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NestSettings {
    pub support: Support,
    pub sheet_w: f64,
    pub sheet_h: f64,
    pub roll_w: f64,
    pub roll_max: f64,
    pub margin: f64,
    pub spacing: f64,
    pub bleed: f64,
    pub turn: Turn,
    pub mirror: bool,
    pub holes: bool,
    pub seconds: u64,
}

impl Default for NestSettings {
    fn default() -> Self {
        Self {
            support: Support::Sheet,
            sheet_w: 3050.0,
            sheet_h: 2050.0,
            roll_w: 1600.0,
            roll_max: 5000.0,
            margin: 10.0,
            spacing: 6.0,
            bleed: 3.0,
            turn: Turn::Right,
            mirror: false,
            holes: false,
            seconds: 10,
        }
    }
}

impl NestSettings {
    /// Le misure dentro i limiti che il PDF e il motore reggono.
    pub fn sanitized(mut self) -> Self {
        let max = winfiles_pdf_edit::MAX_SHEET_MM;
        let fix = |v: f64, lo: f64, hi: f64, d: f64| if v.is_finite() { v.clamp(lo, hi) } else { d };
        let d = Self::default();
        self.sheet_w = fix(self.sheet_w, 50.0, max, d.sheet_w);
        self.sheet_h = fix(self.sheet_h, 50.0, max, d.sheet_h);
        self.roll_w = fix(self.roll_w, 50.0, max, d.roll_w);
        self.roll_max = fix(self.roll_max, 100.0, max, d.roll_max);
        self.margin = fix(self.margin, 0.0, 200.0, d.margin);
        self.spacing = fix(self.spacing, 0.0, 200.0, d.spacing);
        self.bleed = fix(self.bleed, 0.0, 50.0, d.bleed);
        self.seconds = self.seconds.clamp(1, 600);
        self
    }

    /// La distanza vera fra due sagome: almeno due abbondanze, che non si devono toccare.
    fn gap(&self) -> f64 {
        self.spacing.max(2.0 * self.bleed)
    }
}

/// Da dove viene la sagoma di una pagina.
#[derive(Clone, Debug, PartialEq)]
enum From {
    /// La fustella in tinta piatta, con il suo nome.
    Die(String),
    /// Un rettangolo: il formato finito trovato in questo modo.
    Rect(Finish),
    /// Un'immagine: il suo rettangolo.
    Image,
}

impl From {
    fn label(&self) -> String {
        match self {
            From::Die(name) => format!("Fustella «{name}»"),
            From::Rect(f) => f.label().to_string(),
            From::Image => "Immagine".to_string(),
        }
    }
}

/// Una pagina letta.
struct Shape {
    contours: Contours,
    from: From,
    /// La pagina come si vede, in mm.
    page: [f64; 2],
    /// Il rettangolo della sagoma, in mm.
    bounds: [f64; 4],
    /// L'abbondanza più stretta che il file ha già oltre la sagoma (mm), se si sa.
    bleed: Option<f64>,
}

/// Una pagina da disporre.
struct Entry {
    path: PathBuf,
    page: usize,
    pages: usize,
    copies: usize,
    /// La scala del disegno, 1:`scale`: sul foglio la pagina è `scale` volte più grande.
    scale: u32,
    shape: Shape,
    /// L'immagine della pagina intera e quella ritagliata sulla sagoma più l'abbondanza.
    thumb: winfiles_thumbs::Thumb,
    /// La miniatura ritagliata, con l'abbondanza e la scala per cui è stata fatta.
    texture: Option<(egui::TextureHandle, f64, u32)>,
}

impl Entry {
    fn name(&self) -> String {
        let name = file_name(&self.path);
        if self.pages > 1 {
            format!("{name} · pag. {}", self.page + 1)
        } else {
            name
        }
    }

    /// Di quanto si ingrandisce la pagina sul foglio.
    fn factor(&self) -> f64 {
        f64::from(self.scale.max(1))
    }

    /// La misura della sagoma sul foglio (con la scala).
    fn size(&self) -> Size {
        let b = self.shape.bounds;
        let k = self.factor();
        Size::new((b[2] - b[0]) * k, (b[3] - b[1]) * k)
    }

    /// I contorni della sagoma sul foglio, in mm (con la scala).
    fn contours(&self) -> Contours {
        let k = self.factor();
        scaled(&self.shape.contours, k)
    }

    /// L'abbondanza che il file ha già, sul foglio.
    fn file_bleed(&self) -> Option<f64> {
        self.shape.bleed.map(|b| b * self.factor())
    }

    /// Il ritaglio della grafica in mm della pagina: la sagoma più l'abbondanza `bleed` (mm sul
    /// foglio, quindi divisa per la scala).
    fn clip(&self, bleed: f64) -> Contours {
        clip_of(&self.shape, bleed, self.factor())
    }
}

/// I contorni ingranditi di `k`.
fn scaled(contours: &Contours, k: f64) -> Contours {
    contours
        .iter()
        .map(|c| c.iter().map(|p| [p[0] * k, p[1] * k]).collect())
        .collect()
}

type Loaded = (
    PathBuf,
    Result<Vec<(usize, usize, Shape, winfiles_thumbs::Thumb)>, String>,
);

/// Il calcolo in corso.
struct Running {
    stop: Arc<AtomicBool>,
    rx: mpsc::Receiver<RunMsg>,
    tries: usize,
}

enum RunMsg {
    Better(Outcome),
    Tries(usize),
    Done(Result<Outcome, String>),
}

/// Il risultato e per quali scelte è stato calcolato.
struct Result_ {
    outcome: Outcome,
    /// Le pagine dell'elenco al momento del calcolo (gli indici di `Placement::item`).
    entries: Vec<(PathBuf, usize)>,
    stamp: Stamp,
}

/// Quello da cui dipende un risultato: se cambia, va ricalcolato.
#[derive(Clone, Debug, PartialEq)]
struct Stamp {
    settings: NestSettings,
    copies: Vec<(PathBuf, usize, usize, u32)>,
}

pub struct NestingModule {
    entries: Vec<Entry>,
    pub settings: NestSettings,
    /// I nomi delle tinte e il loro ruolo (Taglio…), gli stessi di Modifica PDF.
    pub inks: Vec<InkName>,
    loading: mpsc::Receiver<Loaded>,
    load_tx: mpsc::Sender<Loaded>,
    pending: usize,
    running: Option<Running>,
    result: Option<Result_>,
    saving: Option<mpsc::Receiver<Result<PathBuf, String>>>,
    sheet: usize,
    chosen: Option<usize>,
    pub status: String,
    unit: Unit,
    wake: egui::Context,
    bar_height: f32,
    /// Ingrandimento dell'anteprima (1 = il foglio intero) e spostamento del foglio dal centro.
    zoom: f32,
    pan: Vec2,
}

impl NestingModule {
    pub fn new(ctx: &egui::Context) -> Self {
        let (load_tx, loading) = mpsc::channel();
        Self {
            entries: Vec::new(),
            settings: NestSettings::default(),
            inks: tracciati::default_inks(),
            loading,
            load_tx,
            pending: 0,
            running: None,
            result: None,
            saving: None,
            sheet: 0,
            chosen: None,
            status: String::new(),
            unit: Unit::Mm,
            wake: ctx.clone(),
            bar_height: 46.0,
            zoom: 1.0,
            pan: Vec2::ZERO,
        }
    }

    // --- File ------------------------------------------------------------------------------

    /// Aggiunge i PDF (e gli AI salvati con il PDF dentro), di cui ogni pagina diventa una sagoma,
    /// e le immagini, che sono rettangoli.
    pub fn add(&mut self, paths: &[PathBuf]) {
        let mut skipped = 0usize;
        for path in paths {
            if !matches!(
                FileKind::from_path(path),
                FileKind::Pdf | FileKind::Ai | FileKind::Image
            ) {
                skipped += 1;
                continue;
            }
            if self.entries.iter().any(|e| e.path == *path) {
                continue;
            }
            self.pending += 1;
            let tx = self.load_tx.clone();
            let wake = self.wake.clone();
            let inks = self.inks.clone();
            let path = path.clone();
            let spawned = std::thread::Builder::new()
                .name("nesting-file".into())
                .spawn(move || {
                    let result = load(&path, &inks);
                    let _ = tx.send((path, result));
                    wake.request_repaint();
                });
            if spawned.is_err() {
                self.pending -= 1;
            }
        }
        if skipped > 0 {
            self.status = if skipped == 1 {
                "Un file non va: nel nesting vanno PDF, AI e immagini".into()
            } else {
                format!("{skipped} file non vanno: nel nesting vanno PDF, AI e immagini")
            };
        }
    }

    pub fn dropped(&mut self, paths: Vec<PathBuf>) {
        self.add(&paths);
    }

    fn receive(&mut self) {
        while let Ok((path, result)) = self.loading.try_recv() {
            self.pending = self.pending.saturating_sub(1);
            match result {
                Ok(pages) => {
                    let copies = copies_in_name(&file_name(&path)).unwrap_or(1);
                    for (page, pages, shape, thumb) in pages {
                        self.entries.push(Entry {
                            path: path.clone(),
                            page,
                            pages,
                            copies,
                            scale: 1,
                            shape,
                            thumb,
                            texture: None,
                        });
                    }
                    self.status = format!("Aggiunto {}", file_name(&path));
                }
                Err(e) => self.status = format!("{}: {e}", file_name(&path)),
            }
        }
        if let Some(rx) = &self.saving
            && let Ok(r) = rx.try_recv()
        {
            self.saving = None;
            self.status = match r {
                Ok(p) => format!("Salvato in {}", file_name(&p)),
                Err(e) => format!("PDF non salvato: {e}"),
            };
        }
        let mut finished = false;
        if let Some(run) = &mut self.running {
            while let Ok(msg) = run.rx.try_recv() {
                match msg {
                    RunMsg::Better(o) => {
                        if let Some(r) = &mut self.result {
                            r.outcome = o;
                        }
                    }
                    RunMsg::Tries(n) => run.tries = n,
                    RunMsg::Done(r) => {
                        finished = true;
                        match r {
                            Ok(o) => {
                                if let Some(res) = &mut self.result {
                                    res.outcome = o;
                                }
                            }
                            Err(e) => {
                                self.result = None;
                                self.status = format!("Nesting non riuscito: {e}");
                            }
                        }
                    }
                }
            }
        }
        if finished {
            self.running = None;
            if self.result.is_some() {
                self.status = self.summary();
            }
        }
        if let Some(r) = &self.result {
            self.sheet = self.sheet.min(r.outcome.sheets.len().saturating_sub(1));
        }
    }

    fn remove(&mut self, i: usize) {
        if i < self.entries.len() {
            self.entries.remove(i);
            self.chosen = None;
        }
    }

    fn stamp(&self) -> Stamp {
        Stamp {
            settings: self.settings.clone(),
            copies: self
                .entries
                .iter()
                .map(|e| (e.path.clone(), e.page, e.copies, e.scale))
                .collect(),
        }
    }

    /// Il risultato vale ancora per l'elenco e le scelte di adesso.
    fn fresh(&self) -> bool {
        self.result.as_ref().is_some_and(|r| r.stamp == self.stamp())
    }

    fn total_copies(&self) -> usize {
        self.entries.iter().map(|e| e.copies).sum()
    }

    // --- Calcolo ---------------------------------------------------------------------------

    fn start(&mut self) {
        self.stop();
        if self.total_copies() == 0 {
            self.status = "Aggiungi dei file e scegli quante copie".into();
            return;
        }
        let s = self.settings.clone().sanitized();
        self.settings = s.clone();
        let items: Vec<Item> = self
            .entries
            .iter()
            .map(|e| Item {
                shape: e.contours(),
                copies: e.copies,
                right_angles_only: false,
            })
            .collect();
        let options = Options {
            stock: match s.support {
                Support::Sheet => Stock::Sheet {
                    width: s.sheet_w,
                    height: s.sheet_h,
                },
                Support::Roll => Stock::Roll {
                    width: s.roll_w,
                    max_length: s.roll_max,
                },
            },
            margin: s.margin,
            spacing: s.gap(),
            rotation_step: s.turn.step(),
            mirror: s.mirror,
            holes: s.holes,
            time_limit: Duration::from_secs(s.seconds),
            max_sheets: 200,
            seed: 1,
        };
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let wake = self.wake.clone();
        let spawned = std::thread::Builder::new().name("nesting".into()).spawn(move || {
            let better = tx.clone();
            let tries = tx.clone();
            let w1 = wake.clone();
            let w2 = wake.clone();
            let result = winfiles_nest::nest(
                &items,
                &options,
                &flag,
                |o| {
                    let _ = better.send(RunMsg::Better(o.clone()));
                    w1.request_repaint();
                },
                |p| {
                    let _ = tries.send(RunMsg::Tries(p.tries));
                    w2.request_repaint();
                },
            );
            let _ = tx.send(RunMsg::Done(result.map_err(|e| e.to_string())));
            wake.request_repaint();
        });
        if spawned.is_err() {
            self.status = "Impossibile avviare il calcolo".into();
            return;
        }
        self.result = Some(Result_ {
            outcome: Outcome {
                sheets: Vec::new(),
                unplaced: Vec::new(),
                placed: 0,
                total_length: None,
                utilization: 0.0,
                tries: 0,
            },
            entries: self.entries.iter().map(|e| (e.path.clone(), e.page)).collect(),
            stamp: self.stamp(),
        });
        self.sheet = 0;
        self.running = Some(Running { stop, rx, tries: 0 });
        self.status = "Calcolo in corso…".into();
    }

    /// Ferma il calcolo: resta la disposizione migliore trovata fin qui.
    fn stop(&mut self) {
        if let Some(r) = &self.running {
            r.stop.store(true, Ordering::Relaxed);
        }
    }

    fn summary(&self) -> String {
        let Some(r) = &self.result else {
            return String::new();
        };
        let o = &r.outcome;
        let total = o.placed + o.unplaced.len();
        let mut text = match o.total_length {
            Some(len) => format!(
                "{} su {} copie · rotolo lungo {}",
                o.placed,
                total,
                format_length(len, Unit::Cm)
            ),
            None => format!(
                "{} su {} copie · {} {}",
                o.placed,
                total,
                o.sheets.len(),
                if o.sheets.len() == 1 { "lastra" } else { "lastre" }
            ),
        };
        text.push_str(&format!(
            " · resa {}%",
            format_number(o.utilization * 100.0, 0, false)
        ));
        if !o.unplaced.is_empty() {
            text.push_str(&format!(" · {} fuori", o.unplaced.len()));
        }
        text
    }

    // --- Salvataggio -----------------------------------------------------------------------

    fn save(&mut self) {
        if self.running.is_some() || self.saving.is_some() {
            return;
        }
        let Some(r) = &self.result else {
            self.status = "Prima calcola il nesting".into();
            return;
        };
        if !self.fresh() {
            self.status = "Le scelte sono cambiate: ricalcola prima di salvare".into();
            return;
        }
        if r.outcome.sheets.is_empty() {
            self.status = "Nessuna copia sta sul foglio".into();
            return;
        }
        let start = self
            .entries
            .first()
            .and_then(|e| e.path.parent().map(Path::to_path_buf));
        let mut dialog = rfd::FileDialog::new()
            .set_title("Salva il nesting in PDF")
            .set_file_name("nesting.pdf")
            .add_filter("PDF", &["pdf"]);
        if let Some(dir) = start {
            dialog = dialog.set_directory(dir);
        }
        let Some(mut target) = dialog.save_file() else {
            return;
        };
        if target.extension().is_none_or(|e| !e.eq_ignore_ascii_case("pdf")) {
            target.set_extension("pdf");
        }
        // Tutto quello che serve al thread: i file (le immagini con la misura della loro pagina),
        // le sagome con l'abbondanza e la scala, i fogli.
        let mut files: Vec<(PathBuf, Option<Size>)> = Vec::new();
        let mut pieces: Vec<(usize, usize, Contours, f64)> = Vec::new();
        for e in &self.entries {
            let source = match files.iter().position(|(p, _)| *p == e.path) {
                Some(i) => i,
                None => {
                    let image =
                        (e.shape.from == From::Image).then(|| Size::new(e.shape.page[0], e.shape.page[1]));
                    files.push((e.path.clone(), image));
                    files.len() - 1
                }
            };
            pieces.push((source, e.page, e.clip(self.settings.bleed), e.factor()));
        }
        let sheets: Vec<NestSheet> = r
            .outcome
            .sheets
            .iter()
            .map(|s| NestSheet {
                width: s.width,
                height: s.height,
                pieces: s
                    .placements
                    .iter()
                    .filter_map(|p| {
                        let (source, page, clip, k) = pieces.get(p.item)?;
                        Some(SheetPiece {
                            source: *source,
                            page: *page,
                            matrix: matrix(p, *k),
                            clip: clip.clone(),
                        })
                    })
                    .collect(),
            })
            .collect();
        let (tx, rx) = mpsc::channel();
        let wake = self.wake.clone();
        let spawned = std::thread::Builder::new()
            .name("nesting-pdf".into())
            .spawn(move || {
                let result = (|| {
                    let opened = files
                        .iter()
                        .map(|(p, image)| {
                            let file = match image {
                                // L'immagine entra come un PDF di una pagina, alla sua risoluzione.
                                Some(size) => winfiles_pdf_out::image_page(p, *size)
                                    .map_err(|e| e.to_string())
                                    .and_then(|b| PdfFile::from_bytes(b).map_err(|e| e.to_string())),
                                None => PdfFile::open(p).map_err(|e| e.to_string()),
                            };
                            file.map_err(|e| format!("{}: {e}", file_name(p)))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let refs: Vec<&PdfFile> = opened.iter().collect();
                    let out = winfiles_pdf_edit::nest_document(&refs, &sheets).map_err(|e| e.to_string())?;
                    out.save(&target).map_err(|e| e.to_string())?;
                    Ok(target)
                })();
                let _ = tx.send(result);
                wake.request_repaint();
            });
        if spawned.is_ok() {
            self.saving = Some(rx);
            self.status = "Salvataggio del PDF…".into();
        }
    }

    // --- Tasti -----------------------------------------------------------------------------

    pub fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let pressed = |mods, key| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(mods, key)));
        if pressed(Modifiers::COMMAND, Key::S) {
            self.save();
        } else if pressed(Modifiers::COMMAND, Key::O) {
            let paths = pick_pdfs();
            self.add(&paths);
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if ctx.input(|i| i.key_pressed(Key::Enter)) && self.running.is_none() {
            self.start();
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.stop();
        }
        if ctx.input(|i| i.key_pressed(Key::Delete))
            && let Some(i) = self.chosen
        {
            self.remove(i);
        }
        let sheets = self.result.as_ref().map_or(0, |r| r.outcome.sheets.len());
        if ctx.input(|i| i.key_pressed(Key::PageDown)) && self.sheet + 1 < sheets {
            self.sheet += 1;
        }
        if ctx.input(|i| i.key_pressed(Key::PageUp)) {
            self.sheet = self.sheet.saturating_sub(1);
        }
    }

    pub fn file_menu(&mut self, ui: &mut egui::Ui) {
        let item = |ui: &mut egui::Ui, text: &str, keys: &str, enabled: bool| {
            ui.add_enabled(enabled, egui::Button::new(text).shortcut_text(keys))
                .clicked()
        };
        if item(ui, "Aggiungi file…", "Ctrl+O", true) {
            ui.close();
            let paths = pick_pdfs();
            self.add(&paths);
        }
        ui.separator();
        let can_save = self.fresh() && self.running.is_none();
        if item(ui, "Salva il PDF dei fogli…", "Ctrl+S", can_save) {
            ui.close();
            self.save();
        }
        ui.separator();
        if item(ui, "Svuota l'elenco", "", !self.entries.is_empty()) {
            ui.close();
            self.stop();
            self.entries.clear();
            self.result = None;
        }
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
        egui::Panel::top("barra-nesting")
            .exact_size(self.bar_height)
            .frame(frame)
            .show(ui, |ui| {
                let area = ui.available_rect_before_wrap();
                let area = area.with_max_y(area.min.y + BAR_ROW);
                let mut right = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(area)
                        .layout(egui::Layout::right_to_left(Align::Center)),
                );
                {
                    let ui = &mut right;
                    let can_save = self.fresh() && self.running.is_none() && self.saving.is_none();
                    let (fill, ink) = if can_save {
                        (t.primary, t.on_primary)
                    } else {
                        (t.button, t.text_disabled)
                    };
                    let save = egui::Button::image_and_text(
                        icons::image("save", 18.0, ink),
                        egui::RichText::new("Salva PDF").font(semibold(14.0)).color(ink),
                    )
                    .fill(fill)
                    .min_size(vec2(124.0, 34.0));
                    if ui
                        .add_enabled(can_save, save)
                        .on_hover_text("Salva i fogli in un PDF per il RIP, una pagina per foglio (Ctrl+S)")
                        .clicked()
                    {
                        self.save();
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
                    if text_button(ui, "file-plus", "Aggiungi file", true)
                        .on_hover_text("Aggiungi PDF, AI e immagini: ogni pagina è una sagoma (Ctrl+O)")
                        .clicked()
                    {
                        let paths = pick_pdfs();
                        self.add(&paths);
                    }
                    separator(ui);
                    if self.running.is_some() {
                        if text_button(ui, "x", "Ferma", true)
                            .on_hover_text("Tiene la disposizione migliore trovata fin qui (Esc)")
                            .clicked()
                        {
                            self.stop();
                        }
                    } else {
                        let can = self.total_copies() > 0 && self.pending == 0;
                        let (fill, ink) = if can && !self.fresh() {
                            (t.primary, t.on_primary)
                        } else {
                            (t.button, if can { t.text_strong } else { t.text_disabled })
                        };
                        let go = egui::Button::image_and_text(
                            icons::image("layout-grid", 17.0, ink),
                            egui::RichText::new("Calcola").font(semibold(13.5)).color(ink),
                        )
                        .fill(fill)
                        .min_size(vec2(0.0, 32.0));
                        if ui
                            .add_enabled(can, go)
                            .on_hover_text("Dispone le copie sui fogli (Invio)")
                            .clicked()
                        {
                            self.start();
                        }
                    }
                }
                let height = left.min_rect().height().max(BAR_ROW) + 8.0;
                if (height - self.bar_height).abs() > 0.5 {
                    self.bar_height = height;
                    ui.ctx().request_repaint();
                }
            });
    }

    pub fn status_bar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE
            .fill(t.app_bar)
            .inner_margin(egui::Margin::symmetric(16, 0));
        egui::Panel::bottom("barra-stato-nesting")
            .exact_size(32.0)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let text = format!(
                        "{} {}  ·  {} {}",
                        self.entries.len(),
                        if self.entries.len() == 1 {
                            "sagoma"
                        } else {
                            "sagome"
                        },
                        self.total_copies(),
                        if self.total_copies() == 1 {
                            "copia"
                        } else {
                            "copie"
                        },
                    );
                    ui.label(egui::RichText::new(text).font(semibold(12.5)).color(t.text));
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
        self.unit = unit;
        self.receive();
        self.textures(ui.ctx());
        let full = ui.max_rect();
        let list_w = (full.width() * 0.22).clamp(200.0, 300.0);
        let side_w = (full.width() * 0.24).clamp(250.0, 320.0);
        let list = Rect::from_min_size(full.min, vec2(list_w, full.height()));
        let side = Rect::from_min_max(pos2(full.right() - side_w, full.top()), full.max);
        let view = Rect::from_min_max(
            pos2(list.right() + 10.0, full.top()),
            pos2(side.left() - 10.0, full.bottom()),
        );
        self.file_list(ui, list);
        self.preview(ui, view);
        self.settings_panel(ui, side);
    }

    /// Le immagini delle pagine, ritagliate sulla sagoma più l'abbondanza di adesso.
    fn textures(&mut self, ctx: &egui::Context) {
        let bleed = self.settings.bleed;
        for (i, e) in self.entries.iter_mut().enumerate() {
            if e.texture
                .as_ref()
                .is_some_and(|(_, b, k)| (*b - bleed).abs() < 1e-9 && *k == e.scale)
            {
                continue;
            }
            let clip = e.clip(bleed);
            let img = masked(&e.thumb, &clip, e.shape.page);
            let tex = ctx.load_texture(
                format!("nesting-{i}-{}", e.path.display()),
                img,
                egui::TextureOptions::LINEAR,
            );
            e.texture = Some((tex, bleed, e.scale));
        }
    }

    fn file_list(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 40.0)).shrink2(vec2(14.0, 0.0));
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        bar.label(
            egui::RichText::new("Sagome")
                .font(semibold(14.0))
                .color(t.text_strong),
        );
        bar.label(egui::RichText::new(self.entries.len().to_string()).color(t.text_dim));
        if self.pending > 0 {
            bar.add(egui::Spinner::new().size(14.0).color(t.primary));
        }
        let body = Rect::from_min_max(
            pos2(rect.left() + 6.0, header.bottom()),
            rect.max - vec2(6.0, 8.0),
        );
        if self.entries.is_empty() {
            let c = body.center();
            icons::paint(
                ui,
                "files",
                Rect::from_center_size(c - vec2(0.0, 40.0), Vec2::splat(36.0)),
                t.text_dim,
            );
            let g = ui.painter().layout(
                "Trascina qui i PDF o le immagini da disporre. La sagoma è la fustella di taglio, o il formato \
                 finito se non c'è."
                    .into(),
                egui::FontId::proportional(13.0),
                t.text_dim,
                body.width() - 28.0,
            );
            let at = pos2(c.x - g.size().x / 2.0, c.y - 8.0);
            ui.painter().galley(at, g, t.text_dim);
            return;
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
        let mut remove = None;
        let unit = self.unit;
        let want = self.settings.bleed;
        egui::ScrollArea::vertical()
            .id_salt("elenco-nesting")
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                for i in 0..self.entries.len() {
                    let Some(e) = self.entries.get_mut(i) else {
                        continue;
                    };
                    let chosen = self.chosen == Some(i);
                    // Prima i testi, che vanno a capo: l'altezza della riga dipende da loro, e i
                    // campi stanno sotto, a tutta larghezza.
                    let width = ui.available_width();
                    let x_off = 8.0 + 56.0 + 10.0;
                    let text_w = (width - x_off - 8.0).max(20.0);
                    let name = ui.painter().layout(
                        e.name(),
                        semibold(12.5),
                        if chosen { t.text_strong } else { t.text },
                        text_w,
                    );
                    let mut info = format!("{} · {}", format_size(e.size(), unit), e.shape.from.label());
                    if e.scale > 1 {
                        info = format!("{info} · scala 1:{}", e.scale);
                    }
                    let short = e.file_bleed().is_some_and(|b| b + 0.05 < want);
                    if let (true, Some(b)) = (short, e.file_bleed()) {
                        info = format!("{info}\nAbbondanza nel file: {}", format_length(b, Unit::Mm));
                    }
                    let galley = ui.painter().layout(
                        info,
                        egui::FontId::proportional(11.5),
                        if short { t.warning } else { t.text_dim },
                        text_w,
                    );
                    let text_h = 8.0 + name.size().y + 2.0 + galley.size().y;
                    let top_h = text_h.max(CELL_MIN);
                    let cell_h = top_h + FIELDS_H + 8.0;
                    let (cell, resp) = ui.allocate_exact_size(vec2(width, cell_h), Sense::click());
                    if !ui.is_rect_visible(cell) {
                        continue;
                    }
                    let fill = if chosen {
                        t.selection
                    } else if resp.hovered() {
                        t.hover
                    } else {
                        Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(cell.shrink(2.0), t.radius + 2, fill);
                    let thumb_rect = Rect::from_min_size(cell.min + vec2(8.0, 10.0), Vec2::splat(56.0));
                    if let Some((tex, _, _)) = &e.texture {
                        let at = fit_rect(thumb_rect, Size::new(e.shape.page[0], e.shape.page[1]));
                        ui.painter().image(
                            tex.id(),
                            at,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    let x = cell.left() + x_off;
                    let name_h = name.size().y;
                    ui.painter().galley(pos2(x, cell.top() + 8.0), name, t.text);
                    ui.painter()
                        .galley(pos2(x, cell.top() + 10.0 + name_h), galley, t.text_dim);
                    // Copie e scala, sotto i testi.
                    let row = Rect::from_min_size(
                        pos2(cell.left() + 8.0, cell.top() + top_h + 2.0),
                        vec2(width - 16.0, FIELDS_H),
                    );
                    let mut c = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(row)
                            .layout(egui::Layout::left_to_right(Align::Center)),
                    );
                    c.spacing_mut().item_spacing.x = 4.0;
                    c.label(egui::RichText::new("Copie").size(11.5).color(t.text_dim));
                    crate::fluent::number(
                        &mut c,
                        40.0,
                        egui::DragValue::new(&mut e.copies)
                            .range(0..=MAX_COPIES)
                            .speed(0.2),
                    )
                    .on_hover_text("Quante copie di questa sagoma (0 la lascia fuori)");
                    c.add_space(8.0);
                    c.label(egui::RichText::new("Scala").size(11.5).color(t.text_dim));
                    egui::ComboBox::from_id_salt(("scala-nesting", i))
                        .icon(crate::fluent::combo_icon)
                        .selected_text(format!("1:{}", e.scale))
                        .width(62.0)
                        .height(400.0)
                        .show_ui(&mut c, |ui| {
                            for n in SCALES {
                                ui.selectable_value(&mut e.scale, n, format!("1:{n}"));
                            }
                        })
                        .response
                        .on_hover_text(
                            "La scala del disegno nel file: con 1:10 sul foglio va 10 volte più \
                             grande, abbondanza compresa",
                        );
                    if resp.clicked() {
                        self.chosen = Some(i);
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Togli dall'elenco").clicked() {
                            remove = Some(i);
                            ui.close();
                        }
                    });
                }
            });
        if let Some(i) = remove {
            self.remove(i);
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 44.0)).shrink2(vec2(14.0, 0.0));
        let sheets = self.result.as_ref().map_or(0, |r| r.outcome.sheets.len());
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        let label = match self.settings.support {
            Support::Sheet if sheets > 0 => format!("Lastra {} di {sheets}", self.sheet + 1),
            Support::Roll if sheets > 1 => format!("Rotolo, pezzo {} di {sheets}", self.sheet + 1),
            Support::Roll => "Rotolo".into(),
            Support::Sheet => "Lastra".into(),
        };
        bar.label(
            egui::RichText::new(label)
                .font(semibold(14.0))
                .color(t.text_strong),
        );
        if sheets > 1 {
            if icons::button(
                &mut bar,
                "chevron-left",
                "Foglio prima (Pag Su)",
                false,
                self.sheet > 0,
                28.0,
            )
            .clicked()
            {
                self.sheet = self.sheet.saturating_sub(1);
            }
            if icons::button(
                &mut bar,
                "chevron-right",
                "Foglio dopo (Pag Giù)",
                false,
                self.sheet + 1 < sheets,
                28.0,
            )
            .clicked()
            {
                self.sheet += 1;
            }
        }
        if let Some(run) = &self.running {
            bar.add(egui::Spinner::new().size(16.0).color(t.primary));
            bar.label(
                egui::RichText::new(format!("Cerco disposizioni migliori… {} provate", run.tries))
                    .color(t.text_dim),
            );
        } else if self.result.is_some() {
            if self.fresh() {
                bar.label(egui::RichText::new(self.summary()).color(t.text_dim));
            } else {
                bar.label(egui::RichText::new("Le scelte sono cambiate: premi Calcola").color(t.warning));
            }
        }
        let area = Rect::from_min_max(
            pos2(rect.left() + 14.0, header.bottom() + 4.0),
            rect.max - vec2(14.0, 14.0),
        );
        let Some(r) = &self.result else {
            let text = if self.entries.is_empty() {
                "Aggiungi i file, scegli le copie e premi Calcola"
            } else {
                "Premi Calcola per disporre le copie"
            };
            ui.painter().text(
                area.center(),
                egui::Align2::CENTER_CENTER,
                text,
                semibold(15.0),
                t.text_dim,
            );
            return;
        };
        let Some(sheet) = r.outcome.sheets.get(self.sheet) else {
            if self.running.is_some() {
                ui.painter().text(
                    area.center(),
                    egui::Align2::CENTER_CENTER,
                    "Calcolo della prima disposizione…",
                    semibold(15.0),
                    t.text_dim,
                );
            } else if !r.outcome.unplaced.is_empty() {
                ui.painter().text(
                    area.center(),
                    egui::Align2::CENTER_CENTER,
                    "Le sagome non stanno sul foglio: controlla misure e margine",
                    semibold(15.0),
                    t.warning,
                );
            }
            return;
        };
        // Il foglio intero nell'anteprima; il rotolo con la lunghezza usata. Ctrl+rotella
        // ingrandisce verso il puntatore, trascinare sposta, il doppio clic torna al foglio intero.
        let (sw, sh) = (sheet.width, sheet.height.max(1.0));
        let fit = ((area.width() / sw as f32).min(area.height() / sh as f32)).max(0.0001);
        let resp = ui.interact(area, ui.id().with("foglio-nesting"), Sense::click_and_drag());
        if let Some(p) = resp.hover_pos()
            && let Some(z) = crate::app::wheel_zoom(ui, area)
        {
            {
                let old = fit * self.zoom;
                let min = area.center() + self.pan - vec2(sw as f32, sh as f32) * old / 2.0;
                let q = (p - min) / old;
                self.zoom = (self.zoom * z).clamp(1.0, 400.0);
                let new = fit * self.zoom;
                let new_min = p - q * new;
                self.pan = new_min + vec2(sw as f32, sh as f32) * new / 2.0 - area.center();
            }
        }
        if resp.dragged() {
            self.pan += resp.drag_delta();
        }
        if resp.double_clicked() {
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
        }
        let scale = fit * self.zoom;
        let size = vec2(sw as f32 * scale, sh as f32 * scale);
        self.pan = crate::pan::clamp(self.pan, size, area.size(), 24.0);
        let paper = Rect::from_center_size(area.center() + self.pan, size);
        let to = |p: [f64; 2]| -> Pos2 {
            pos2(
                paper.left() + p[0] as f32 * scale,
                paper.top() + p[1] as f32 * scale,
            )
        };
        let painter = ui.painter_at(area.expand(2.0));
        painter.rect_filled(paper, 0.0, Color32::WHITE);
        painter.rect_stroke(paper, 0.0, Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
        // Il margine, tratteggiato.
        let m = self.settings.margin;
        if m > 0.0 {
            let inner = Rect::from_min_max(to([m, m]), to([sw - m, sh - m]));
            let corners = [
                inner.left_top(),
                inner.right_top(),
                inner.right_bottom(),
                inner.left_bottom(),
                inner.left_top(),
            ];
            for w in corners.windows(2) {
                if let [a, b] = w {
                    painter.extend(egui::Shape::dashed_line(
                        &[*a, *b],
                        Stroke::new(1.0, t.divider),
                        4.0,
                        4.0,
                    ));
                }
            }
        }
        let hover = ui.input(|i| i.pointer.hover_pos());
        let mut tip: Option<String> = None;
        for p in &sheet.placements {
            let Some((path, page)) = r.entries.get(p.item) else {
                continue;
            };
            let Some(e) = self.entries.iter().find(|e| e.path == *path && e.page == *page) else {
                continue;
            };
            let k = e.factor();
            let [pw, ph] = e.shape.page.map(|v| v * k);
            if let Some((tex, _, _)) = &e.texture {
                let corners = [[0.0, 0.0], [pw, 0.0], [pw, ph], [0.0, ph]].map(|c| to(p.apply(c)));
                let uvs = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
                let mut mesh = egui::Mesh::with_texture(tex.id());
                for (c, uv) in corners.iter().zip(uvs) {
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos: *c,
                        uv,
                        color: Color32::WHITE,
                    });
                }
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                painter.add(egui::Shape::mesh(mesh));
            }
            // La linea di taglio, sopra.
            let mut inside = false;
            for c in &e.shape.contours {
                let pts: Vec<Pos2> = c.iter().map(|q| to(p.apply([q[0] * k, q[1] * k]))).collect();
                if let Some(h) = hover
                    && contains(&pts, h)
                {
                    inside = !inside;
                }
                painter.add(egui::Shape::closed_line(pts, Stroke::new(1.0, t.quote)));
            }
            if inside {
                tip = Some(format!("{} · copia {}", e.name(), p.copy + 1));
            }
        }
        if let Some(tip) = tip {
            resp.on_hover_text(tip);
        } else if self.zoom <= 1.0 {
            resp.on_hover_text("Ctrl+rotella per ingrandire, trascina per spostare");
        }
        // La misura del foglio sotto.
        let caption = format_size(Size::new(sw, sh), Unit::Cm);
        painter.text(
            pos2(paper.center().x, (paper.bottom() + 4.0).min(area.bottom() - 2.0)),
            egui::Align2::CENTER_TOP,
            caption,
            egui::FontId::proportional(11.5),
            t.text_dim,
        );
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        let inner = rect.shrink2(vec2(16.0, 12.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        let unit = self.unit;
        egui::ScrollArea::vertical()
            .id_salt("impostazioni-nesting")
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                let s = &mut self.settings;
                heading(ui, "Supporto");
                segmented(ui, "supporto-nesting", &mut s.support, &[(Support::Sheet, "Lastre"), (Support::Roll, "Rotolo")]);
                ui.add_space(8.0);
                egui::Grid::new("misure-nesting")
                    .num_columns(2)
                    .spacing(vec2(10.0, 8.0))
                    .show(ui, |ui| {
                        match s.support {
                            Support::Sheet => {
                                length_row(ui, "Larghezza", &mut s.sheet_w, unit, 50.0..=winfiles_pdf_edit::MAX_SHEET_MM);
                                length_row(ui, "Altezza", &mut s.sheet_h, unit, 50.0..=winfiles_pdf_edit::MAX_SHEET_MM);
                            }
                            Support::Roll => {
                                length_row(ui, "Larghezza", &mut s.roll_w, unit, 50.0..=winfiles_pdf_edit::MAX_SHEET_MM);
                                length_row(ui, "Lunghezza max", &mut s.roll_max, unit, 100.0..=winfiles_pdf_edit::MAX_SHEET_MM);
                            }
                        }
                        length_row(ui, "Margine", &mut s.margin, Unit::Mm, 0.0..=200.0);
                    });
                if s.support == Support::Roll {
                    hint(
                        ui,
                        "Il rotolo si taglia alla lunghezza usata; oltre la lunghezza massima si passa a un altro \
                         pezzo (un PDF non va oltre 5,08 m).",
                    );
                }
                ui.add_space(12.0);
                heading(ui, "Sagome");
                egui::Grid::new("distanze-nesting")
                    .num_columns(2)
                    .spacing(vec2(10.0, 8.0))
                    .show(ui, |ui| {
                        length_row(ui, "Distanza", &mut s.spacing, Unit::Mm, 0.0..=200.0);
                        length_row(ui, "Abbondanza", &mut s.bleed, Unit::Mm, 0.0..=50.0);
                    });
                if 2.0 * s.bleed > s.spacing {
                    hint(
                        ui,
                        &format!(
                            "Con {} di abbondanza le fustelle restano ad almeno {}, così le abbondanze non si \
                             toccano.",
                            format_length(s.bleed, Unit::Mm),
                            format_length(2.0 * s.bleed, Unit::Mm)
                        ),
                    );
                }
                ui.add_space(12.0);
                heading(ui, "Rotazione");
                segmented(
                    ui,
                    "rotazione-nesting",
                    &mut s.turn,
                    &[(Turn::None, "No"), (Turn::Right, "90°"), (Turn::Deg45, "45°"), (Turn::Deg15, "15°")],
                );
                ui.add_space(8.0);
                ui.add(crate::fluent::checkbox(&mut s.mirror, "Anche specchiate"))
                    .on_hover_text("Solo per i materiali senza dritto e rovescio");
                ui.add(crate::fluent::checkbox(&mut s.holes, "Sagome piccole nei fori"))
                    .on_hover_text("Usa i fori interni (il centro di una O) per le sagome più piccole");
                ui.add_space(12.0);
                heading(ui, "Tempo di ricerca");
                segmented(
                    ui,
                    "tempo-nesting",
                    &mut s.seconds,
                    &[(5, "5 s"), (10, "10 s"), (30, "30 s"), (60, "1 min")],
                );
                hint(
                    ui,
                    "Dopo la prima disposizione prova ordini diversi per questo tempo e tiene la migliore; \
                     Ferma la tiene subito.",
                );
                if let Some(r) = &self.result
                    && !r.outcome.sheets.is_empty()
                {
                    ui.add_space(14.0);
                    heading(ui, "Risultato");
                    let o = &r.outcome;
                    let area: f64 = o.sheets.iter().map(|s| s.width * s.height).sum::<f64>() / 1_000_000.0;
                    let pieces: f64 = o.sheets.iter().map(|s| s.area).sum::<f64>() / 1_000_000.0;
                    egui::Grid::new("risultato-nesting")
                        .num_columns(2)
                        .spacing(vec2(10.0, 4.0))
                        .show(ui, |ui| {
                            let row = |ui: &mut egui::Ui, k: &str, v: String| {
                                ui.label(egui::RichText::new(k).color(t.text_dim));
                                ui.label(egui::RichText::new(v).color(t.text_strong));
                                ui.end_row();
                            };
                            row(ui, "Copie", format!("{} su {}", o.placed, o.placed + o.unplaced.len()));
                            match o.total_length {
                                Some(len) => row(ui, "Rotolo", format_length(len, Unit::Cm)),
                                None => {
                                    row(ui, "Lastre", o.sheets.len().to_string());
                                    if let Some(last) = o.sheets.last() {
                                        row(ui, "Ultima usata", format!("fino a {}", format_length(last.used, Unit::Cm)));
                                    }
                                }
                            }
                            row(ui, "Materiale", format_area(area));
                            row(ui, "Sagome", format_area(pieces));
                            row(ui, "Resa", format!("{}%", format_number(o.utilization * 100.0, 0, false)));
                        });
                    if !o.unplaced.is_empty() {
                        hint_color(
                            ui,
                            &format!(
                                "{} copie non stanno: sono più grandi del foglio o le lastre sono finite.",
                                o.unplaced.len()
                            ),
                            t.warning,
                        );
                    }
                }
            });
    }
}

fn heading(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(
        egui::RichText::new(text)
            .font(semibold(13.0))
            .color(t.text_strong),
    );
    ui.add_space(4.0);
}

fn hint(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    hint_color(ui, text, t.text_dim);
}

fn hint_color(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.add_space(4.0);
    ui.add(egui::Label::new(egui::RichText::new(text).size(11.5).color(color)).wrap());
}

/// Una riga del modulo: etichetta e lunghezza in `unit`, salvata in mm.
fn length_row(
    ui: &mut egui::Ui,
    label: &str,
    mm: &mut f64,
    unit: Unit,
    range: std::ops::RangeInclusive<f64>,
) {
    let t = Tokens::get(ui.ctx());
    ui.allocate_ui_with_layout(
        vec2(104.0, crate::fluent::HEIGHT),
        egui::Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_width(104.0);
            ui.label(egui::RichText::new(label).color(t.text));
        },
    );
    ui.horizontal(|ui| {
        let k = unit.mm();
        let mut v = *mm / k;
        let resp = crate::fluent::number(
            ui,
            76.0,
            egui::DragValue::new(&mut v)
                .range(range.start() / k..=range.end() / k)
                .speed(if unit == Unit::Cm { 0.5 } else { 1.0 })
                .max_decimals(unit.decimals()),
        );
        if resp.changed() {
            *mm = v * k;
        }
        ui.label(egui::RichText::new(unit.symbol()).color(t.text_dim));
    });
    ui.end_row();
}

/// Il ritaglio di una sagoma: la sagoma più l'abbondanza, almeno mezzo millimetro (lo spessore
/// della linea di taglio deve restare intero).
/// Con la scala 1:`k` la pagina si ingrandisce, quindi sulla pagina l'abbondanza è `k` volte più
/// stretta.
fn clip_of(shape: &Shape, bleed: f64, k: f64) -> Contours {
    winfiles_pdf_edit::grow_shape(&shape.contours, bleed.max(0.5) / k.max(1.0))
}

/// La matrice dalla pagina (mm) al foglio (mm) di una copia: prima la scala `k` e lo specchio,
/// poi la rotazione e lo spostamento, come nel motore.
fn matrix(p: &Placement, k: f64) -> [f64; 6] {
    let (s, c) = p.rotation.to_radians().sin_cos();
    let f = if p.mirror { -1.0 } else { 1.0 };
    [c * f * k, s * f * k, -s * k, c * k, p.x, p.y]
}

/// Il punto `p` è dentro il poligono (pari-dispari).
fn contains(poly: &[Pos2], p: Pos2) -> bool {
    let n = poly.len();
    let mut inside = false;
    for i in 0..n {
        let (Some(a), Some(b)) = (poly.get(i), poly.get((i + n - 1) % n)) else {
            continue;
        };
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

/// L'immagine della pagina con trasparente tutto quello che sta fuori da `clip` (mm sulla pagina
/// `page`), riga per riga: dove una riga incrocia i contorni si entra e si esce (pari-dispari).
fn masked(thumb: &winfiles_thumbs::Thumb, clip: &[Vec<[f64; 2]>], page: [f64; 2]) -> egui::ColorImage {
    let (w, h) = (thumb.width as usize, thumb.height as usize);
    let mut rgba = thumb.rgba.clone();
    if rgba.len() < w * h * 4 || page[0] <= 0.0 || page[1] <= 0.0 {
        return egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
    }
    let (kx, ky) = (w as f64 / page[0], h as f64 / page[1]);
    let mut xs: Vec<f64> = Vec::new();
    for y in 0..h {
        let fy = (y as f64 + 0.5) / ky;
        xs.clear();
        for c in clip {
            let n = c.len();
            for i in 0..n {
                let (Some(a), Some(b)) = (c.get(i), c.get((i + 1) % n)) else {
                    continue;
                };
                if (a[1] > fy) != (b[1] > fy) {
                    xs.push(a[0] + (fy - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
                }
            }
        }
        xs.sort_by(f64::total_cmp);
        let mut x = 0usize;
        let mut inside_from: Option<usize> = None;
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for (k, v) in xs.iter().enumerate() {
            let px = ((v * kx).round().max(0.0) as usize).min(w);
            if k % 2 == 0 {
                inside_from = Some(px);
            } else if let Some(s) = inside_from.take() {
                spans.push((s, px));
            }
        }
        for (s, e) in spans {
            for px in x..s.max(x) {
                if let Some(a) = rgba.get_mut((y * w + px) * 4 + 3) {
                    *a = 0;
                }
            }
            x = x.max(e);
        }
        for px in x..w {
            if let Some(a) = rgba.get_mut((y * w + px) * 4 + 3) {
                *a = 0;
            }
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba)
}

/// Legge un PDF: per ogni pagina la sagoma (la fustella di taglio, se c'è, o il formato finito)
/// e l'immagine.
fn load(path: &Path, inks: &[InkName]) -> Result<Vec<(usize, usize, Shape, winfiles_thumbs::Thumb)>, String> {
    if FileKind::from_path(path) == FileKind::Image {
        return load_image(path).map(|one| vec![one]);
    }
    let file = PdfFile::open(path).map_err(|e| e.to_string())?;
    let count = file.page_count();
    if count == 0 {
        return Err("il PDF non ha pagine".into());
    }
    let kind = FileKind::from_path(path);
    let mut out = Vec::new();
    for page in 0..count.min(MAX_PAGES) {
        let scan = file.scan_page(page).map_err(|e| e.to_string())?;
        let check = tracciati::check(&scan, inks);
        let die = check.dies.iter().find(|(_, role, _)| *role == Role::Cut);
        let page_rect = [0.0, 0.0, scan.width, scan.height];
        let (contours, from, bounds) = match die.and_then(|(name, _, g)| {
            let c = winfiles_pdf_edit::die_shape(&scan, &g.ids).ok()?;
            let b = winfiles_pdf_edit::shape_bounds(&c)?;
            Some((c, From::Die(name.clone()), b))
        }) {
            Some(found) => found,
            None => {
                let (r, f) = check
                    .finished
                    .filter(|(_, f)| !matches!(f, Finish::Die))
                    .unwrap_or((page_rect, Finish::Page));
                (rect_contour(r), From::Rect(f), r)
            }
        };
        // L'abbondanza che il file ha già: la grafica oltre la sagoma, fin dove arriva la pagina.
        let bleed = check.content.map(|c| {
            let room = tracciati::bleed_beyond(
                bounds,
                [
                    c[0].max(0.0),
                    c[1].max(0.0),
                    c[2].min(scan.width),
                    c[3].min(scan.height),
                ],
            );
            room.iter().copied().fold(f64::MAX, f64::min)
        });
        let thumb = winfiles_thumbs::render(path, kind, page, THUMB_SIDE).map_err(|e| e.to_string())?;
        out.push((
            page,
            count,
            Shape {
                contours,
                from,
                page: [scan.width, scan.height],
                bounds,
                bleed,
            },
            thumb,
        ));
    }
    Ok(out)
}

/// Legge un'immagine: la sagoma è il suo rettangolo, alla misura che dicono i pixel e i DPI. Oltre
/// il bordo non c'è grafica, quindi l'abbondanza nel file è zero.
fn load_image(path: &Path) -> Result<(usize, usize, Shape, winfiles_thumbs::Thumb), String> {
    let probe = winfiles_formats::probe(path, FileKind::Image).map_err(|e| e.to_string())?;
    let size = probe
        .pages
        .first()
        .map(|p| p.size)
        .filter(|s| s.w > 0.0 && s.h > 0.0)
        .ok_or_else(|| "l'immagine non dichiara le sue misure".to_string())?;
    let thumb = winfiles_thumbs::render(path, FileKind::Image, 0, THUMB_SIDE).map_err(|e| e.to_string())?;
    let bounds = [0.0, 0.0, size.w, size.h];
    Ok((
        0,
        1,
        Shape {
            contours: rect_contour(bounds),
            from: From::Image,
            page: [size.w, size.h],
            bounds,
            bleed: Some(0.0),
        },
        thumb,
    ))
}

/// Le copie scritte nel nome del file, come nel pannello per Illustrator: «x20» o «20pz».
fn copies_in_name(name: &str) -> Option<usize> {
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s).to_lowercase();
    let words = stem.split(|c: char| c.is_whitespace() || c == '_' || c == '-');
    for w in words {
        let digits = if let Some(n) = w.strip_prefix('x') {
            n
        } else if let Some(n) = w.strip_suffix("pz").or_else(|| w.strip_suffix("pezzi")) {
            n
        } else {
            continue;
        };
        if let Ok(n) = digits.parse::<usize>()
            && (1..=MAX_COPIES).contains(&n)
        {
            return Some(n);
        }
    }
    None
}

fn rect_contour(r: [f64; 4]) -> Contours {
    vec![vec![[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]]]
}

fn fit_rect(bounds: Rect, size: Size) -> Rect {
    let (w, h) = (size.w.max(1e-6) as f32, size.h.max(1e-6) as f32);
    let k = (bounds.width() / w).min(bounds.height() / h);
    Rect::from_center_size(bounds.center(), vec2(w * k, h * k))
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

fn pick_pdfs() -> Vec<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Aggiungi file al nesting")
        .add_filter(
            "PDF, Illustrator e immagini",
            &[
                "pdf", "PDF", "ai", "AI", "jpg", "jpeg", "png", "tif", "tiff", "webp", "bmp", "psd",
            ],
        )
        .pick_files()
        .unwrap_or_default()
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_matrice_fa_come_il_motore() {
        let p = Placement {
            item: 0,
            copy: 0,
            rotation: 90.0,
            mirror: true,
            x: 10.0,
            y: 5.0,
        };
        let m = matrix(&p, 1.0);
        let q = [3.0, 7.0];
        let a = p.apply(q);
        let b = [m[0] * q[0] + m[2] * q[1] + m[4], m[1] * q[0] + m[3] * q[1] + m[5]];
        assert!((a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9);
    }

    #[test]
    fn la_scala_ingrandisce_la_pagina_ma_non_l_abbondanza() {
        let p = Placement {
            item: 0,
            copy: 0,
            rotation: 0.0,
            mirror: false,
            x: 10.0,
            y: 5.0,
        };
        // In scala 1:10 un punto a 3 mm dall'angolo della pagina sta a 30 mm sul foglio.
        let m = matrix(&p, 10.0);
        assert_eq!([m[0] * 3.0 + m[4], m[3] * 3.0 + m[5]], [40.0, 35.0]);
        // L'abbondanza di 3 mm sul foglio è 0,3 mm sulla pagina.
        let shape = Shape {
            contours: rect_contour([0.0, 0.0, 20.0, 10.0]),
            from: From::Image,
            page: [20.0, 10.0],
            bounds: [0.0, 0.0, 20.0, 10.0],
            bleed: Some(0.0),
        };
        let b = winfiles_pdf_edit::shape_bounds(&clip_of(&shape, 3.0, 10.0)).unwrap();
        assert!((b[0] + 0.3).abs() < 0.02 && (b[2] - 20.3).abs() < 0.02, "{b:?}");
    }

    #[test]
    fn fuori_dalla_sagoma_diventa_trasparente() {
        let thumb = winfiles_thumbs::Thumb {
            width: 10,
            height: 10,
            rgba: vec![255; 400],
        };
        // Pagina 100 × 100 mm, sagoma nella metà sinistra.
        let clip = rect_contour([0.0, 0.0, 50.0, 100.0]);
        let img = masked(&thumb, &clip, [100.0, 100.0]);
        assert_eq!(img.pixels[0].a(), 255);
        assert_eq!(img.pixels[9].a(), 0);
        assert_eq!(img.pixels[95].a(), 0);
        assert_eq!(img.pixels[94].a(), 255);
    }

    #[test]
    fn le_copie_dal_nome() {
        assert_eq!(copies_in_name("stella x20.pdf"), Some(20));
        assert_eq!(copies_in_name("cerchio_15pz.pdf"), Some(15));
        assert_eq!(copies_in_name("box-xl.pdf"), None);
        assert_eq!(copies_in_name("x0.pdf"), None);
    }

    #[test]
    fn le_scelte_si_aggiustano() {
        let s = NestSettings {
            sheet_w: f64::NAN,
            bleed: 500.0,
            seconds: 0,
            ..NestSettings::default()
        }
        .sanitized();
        assert_eq!(s.sheet_w, NestSettings::default().sheet_w);
        assert_eq!(s.bleed, 50.0);
        assert_eq!(s.seconds, 1);
    }
}
