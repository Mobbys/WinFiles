//! La pagina Stampa della scheda Visualizza: il file su A4 o A3 a misura vera, adattato, in
//! percentuale o (i disegni CAD) in scala 1:N, anche diviso in tasselli da unire; con l'anteprima
//! dei fogli come usciranno.
//!
//! Il PDF di stampa è vettoriale: le pagine del file messe sui fogli con [`nest_document`], come
//! il nesting, con la correzione della stampante calibrata nella matrice (ADR 0010).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use egui::{Align, Color32, Pos2, Rect, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};
use winfiles_geom::{Calibration, Paper, Size, Unit, format_length, format_number, format_size};
use winfiles_pdf_edit::{NestSheet, PdfFile, SheetPiece};

use super::annot::Notes;
use super::load::{CadChoice, Content, Facts, Opened, Renderer, ViewKind};
use crate::app::{segmented, stepper};
use crate::printing::{PrinterScan, pick};
use crate::theme::{Tokens, semibold};

/// Il numero massimo di fogli di una stampa: oltre, quasi certamente è un errore di scala.
pub const MAX_SHEETS: usize = 400;
/// Lo spazio in fondo al foglio per la didascalia, in mm.
const CAPTION_MM: f64 = 5.0;
/// Lo spazio in più per la riga del percorso, in mm.
const PATH_LINE_MM: f64 = 3.4;
/// Il corpo della didascalia, in mm.
const CAPTION_SIZE: f64 = 2.4;
/// Il corpo più piccolo a cui si stringe una riga lunga, in mm: sotto non si legge più.
const CAPTION_MIN_SIZE: f64 = 1.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Turn {
    /// Come serve al file: orizzontale se il file è più largo che alto.
    #[default]
    Auto,
    Portrait,
    Landscape,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// La misura vera (100%, o 1:1 per i disegni).
    #[default]
    Real,
    /// Il più grande che sta nell'area stampabile.
    Fit,
    /// Una percentuale della misura vera.
    Percent,
    /// Scala 1:N (i disegni CAD).
    Ratio,
}

/// Le scelte della pagina Stampa da ricordare (le copie no, come nella finestra Stampa).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Choice {
    pub printer: Option<String>,
    pub paper: Paper,
    pub turn: Turn,
    pub mode: Mode,
    pub percent: f64,
    /// Il N di 1:N.
    pub ratio: f64,
    pub tiles: bool,
    /// Sovrapposizione dei tasselli, in mm.
    pub overlap: f64,
    /// Il bordo che la stampante non stampa, in mm.
    pub margin: f64,
    pub caption: bool,
    /// Nella didascalia anche la cartella del file, su una seconda riga.
    pub path: bool,
    /// Le annotazioni della riga Annota sopra la pagina.
    pub annotations: bool,
    pub gray: bool,
    /// I disegni CAD tutti in nero.
    pub cad_black: bool,
}

impl Default for Choice {
    fn default() -> Self {
        Self {
            printer: None,
            paper: Paper::A4,
            turn: Turn::Auto,
            mode: Mode::Real,
            percent: 100.0,
            ratio: 50.0,
            tiles: false,
            overlap: 10.0,
            margin: 4.0,
            caption: true,
            path: false,
            annotations: true,
            gray: false,
            cad_black: true,
        }
    }
}

impl Choice {
    pub fn sanitized(mut self) -> Self {
        let ok = |v: f64, lo: f64, hi: f64, d: f64| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.percent = ok(self.percent, 1.0, 2000.0, 100.0);
        self.ratio = ok(self.ratio, 1.0, 100_000.0, 50.0);
        self.overlap = ok(self.overlap, 0.0, 50.0, 10.0);
        self.margin = ok(self.margin, 0.0, 30.0, 4.0);
        self
    }

    /// Lo spazio in fondo al foglio per la didascalia, in mm (0 senza).
    pub fn caption_mm(&self) -> f64 {
        match (self.caption, self.path) {
            (false, _) => 0.0,
            (true, false) => CAPTION_MM,
            (true, true) => CAPTION_MM + PATH_LINE_MM,
        }
    }
}

/// Una riga della didascalia: il testo, la sua linea di base dall'alto del foglio e il corpo, in mm.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionLine {
    pub text: String,
    pub y: f64,
    pub size: f64,
}

/// Il corpo con cui `text` sta in `width` mm: quello normale, o più piccolo per una riga lunga
/// (un percorso non si taglia mai). Larghezza media di una lettera in Helvetica: circa mezzo corpo.
fn fitted_size(text: &str, width: f64) -> f64 {
    let chars = text.chars().count().max(1) as f64;
    (width / (chars * 0.52)).clamp(CAPTION_MIN_SIZE, CAPTION_SIZE)
}

/// Le righe della didascalia di un foglio largo `w` e alto `h` mm con bordo `margin`: la prima
/// con nome, pagina, scala e tassello; con `path` sotto la cartella del file.
pub fn caption_lines(first: String, path: Option<String>, w: f64, h: f64, margin: f64) -> Vec<CaptionLine> {
    let width = (w - 2.0 * margin).max(1.0);
    let bottom = h - margin - 1.2;
    match path {
        None => vec![CaptionLine {
            size: fitted_size(&first, width),
            text: first,
            y: bottom,
        }],
        Some(p) => vec![
            CaptionLine {
                size: fitted_size(&first, width),
                text: first,
                y: bottom - PATH_LINE_MM,
            },
            CaptionLine {
                size: fitted_size(&p, width),
                text: p,
                y: bottom,
            },
        ],
    }
}

/// Un foglio della stampa: quale pagina, dove va e quale parte se ne vede.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetPlan {
    pub page: usize,
    pub size: Size,
    /// Dalla pagina (mm, y in giù) al foglio: x' = s·x + e, y' = s·y + f, come `[s, e, f]`.
    pub place: [f64; 3],
    /// La parte della pagina che va su questo foglio (mm): x, y, larghezza, altezza.
    pub clip: [f64; 4],
    /// Il tassello: riga e colonna da 0, righe e colonne in tutto.
    pub tile: Option<[usize; 4]>,
}

/// Il nome di un tassello: la riga in lettere e la colonna in numero, "B3".
pub fn tile_name(row: usize, col: usize) -> String {
    let mut r = row;
    let mut letters = Vec::new();
    loop {
        letters.push(b'A' + (r % 26) as u8);
        if r < 26 {
            break;
        }
        r = r / 26 - 1;
    }
    letters.reverse();
    format!("{}{}", String::from_utf8_lossy(&letters), col + 1)
}

fn sheet(paper: Paper, landscape: bool) -> Size {
    let p = paper.portrait();
    if landscape { Size::new(p.h, p.w) } else { p }
}

/// L'area stampabile del foglio: senza il bordo e, con la didascalia, la riga in fondo.
fn printable(s: Size, margin: f64, bottom: f64) -> Size {
    Size::new(
        (s.w - 2.0 * margin).max(1.0),
        (s.h - 2.0 * margin - bottom).max(1.0),
    )
}

/// La scala (mm sul foglio per mm della pagina) e l'orientamento per `content`.
/// `true_mm` sono i mm veri per mm della pagina (1 per PDF e immagini, la scala del disegno CAD).
pub fn scale_for(content: Size, true_mm: f64, c: &Choice) -> (f64, bool) {
    let wide = content.w > content.h;
    let fit = |landscape: bool| {
        let p = printable(sheet(c.paper, landscape), c.margin, c.caption_mm());
        (p.w / content.w.max(1e-6)).min(p.h / content.h.max(1e-6))
    };
    match c.mode {
        Mode::Fit => match c.turn {
            Turn::Portrait => (fit(false), false),
            Turn::Landscape => (fit(true), true),
            Turn::Auto => {
                let (a, b) = (fit(false), fit(true));
                if b > a { (b, true) } else { (a, false) }
            }
        },
        other => {
            let s = match other {
                Mode::Percent => true_mm * c.percent / 100.0,
                Mode::Ratio => true_mm / c.ratio.max(1.0),
                _ => true_mm,
            };
            let landscape = match c.turn {
                Turn::Portrait => false,
                Turn::Landscape => true,
                Turn::Auto => wide,
            };
            (s, landscape)
        }
    }
}

/// I fogli di una pagina di `content` mm.
pub fn plan(page: usize, content: Size, true_mm: f64, c: &Choice) -> Vec<SheetPlan> {
    let (s, landscape) = scale_for(content, true_mm, c);
    let shown = Size::new(content.w * s, content.h * s);
    let full = [0.0, 0.0, content.w, content.h];
    let one = |landscape: bool| {
        let size = sheet(c.paper, landscape);
        let p = printable(size, c.margin, c.caption_mm());
        // Al centro dell'area stampabile; più grande del foglio si centra lo stesso (e si taglia).
        let e = c.margin + (p.w - shown.w) / 2.0;
        let f = c.margin + (p.h - shown.h) / 2.0;
        SheetPlan {
            page,
            size,
            place: [s, e, f],
            clip: full,
            tile: None,
        }
    };
    let grid = |landscape: bool| {
        let size = sheet(c.paper, landscape);
        let p = printable(size, c.margin, c.caption_mm());
        let step_w = (p.w - c.overlap).max(p.w * 0.5);
        let step_h = (p.h - c.overlap).max(p.h * 0.5);
        let count = |len: f64, area: f64, step: f64| -> usize {
            if len <= area + 1e-6 {
                1
            } else {
                (((len - area) / step).ceil() as usize).saturating_add(1)
            }
        };
        (
            size,
            p,
            step_w,
            step_h,
            count(shown.w, p.w, step_w),
            count(shown.h, p.h, step_h),
        )
    };
    let p_now = printable(sheet(c.paper, landscape), c.margin, c.caption_mm());
    let fits = shown.w <= p_now.w + 0.05 && shown.h <= p_now.h + 0.05;
    if fits || !c.tiles || c.mode == Mode::Fit {
        return vec![one(landscape)];
    }
    // In tasselli, con "Automatico" l'orientamento che ne fa meno.
    let landscape = match c.turn {
        Turn::Auto => {
            let a = grid(false);
            let b = grid(true);
            a.4 * a.5 > b.4 * b.5
        }
        Turn::Portrait => false,
        Turn::Landscape => true,
    };
    let (size, p, step_w, step_h, cols, rows) = grid(landscape);
    if cols.saturating_mul(rows) > MAX_SHEETS {
        return Vec::new();
    }
    let mut out = Vec::new();
    for r in 0..rows {
        for col in 0..cols {
            let x0 = col as f64 * step_w;
            let y0 = r as f64 * step_h;
            let w = p.w.min(shown.w - x0);
            let h = p.h.min(shown.h - y0);
            out.push(SheetPlan {
                page,
                size,
                place: [s, c.margin - x0, c.margin - y0],
                clip: [x0 / s, y0 / s, w / s, h / s],
                tile: Some([r, col, rows, cols]),
            });
        }
    }
    out
}

/// Dove va il file nel PDF di stampa.
#[derive(Clone)]
enum Source {
    Pdf(Arc<Vec<u8>>),
    Image(Arc<image::DynamicImage>, Size),
    Cad {
        drawing: Arc<winfiles_cad::Drawing>,
        options: winfiles_cad::PdfOptions,
    },
}

/// Cosa ha deciso chi usa la pagina.
pub enum Outcome {
    Keep,
    Close,
    Status(String),
}

/// Quali pagine stampare di un file a più pagine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PagesMode {
    Current,
    All,
    /// Quelle scritte nel campo "Pagine".
    Range,
}

/// Le pagine scritte a mano ("2-6, 8", "2 5 8", "3-", "-4"), numerate da 1, in indici da 0
/// ordinati e senza doppioni; `count` è il numero di pagine del file.
pub fn parse_pages(text: &str, count: usize) -> Result<Vec<usize>, String> {
    let mut set = std::collections::BTreeSet::new();
    let mut text = text.replace(['–', '—'], "-");
    // "6 - 4" è un intervallo, non tre pezzi.
    while text.contains(" -") || text.contains("- ") {
        text = text.replace(" -", "-").replace("- ", "-");
    }
    let number = |s: &str| -> Result<usize, String> {
        let n: usize = s
            .trim()
            .parse()
            .map_err(|_| format!("Non capisco «{}»: scrivi per esempio 2-6, 8", s.trim()))?;
        if n == 0 || n > count {
            return Err(format!("La pagina {n} non c'è: il file va da 1 a {count}"));
        }
        Ok(n - 1)
    };
    for part in text
        .split([',', ';', ' '])
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        match part.split_once('-') {
            Some((a, b)) => {
                let a = if a.trim().is_empty() { 0 } else { number(a)? };
                let b = if b.trim().is_empty() {
                    count.saturating_sub(1)
                } else {
                    number(b)?
                };
                set.extend(a.min(b)..=a.max(b));
            }
            None => {
                set.insert(number(part)?);
            }
        }
    }
    if set.is_empty() {
        return Err("Scrivi le pagine, per esempio 2-6 oppure 2, 5, 8".into());
    }
    Ok(set.into_iter().collect())
}

/// Le annotazioni di una pagina da stampare sopra di lei.
#[derive(Clone, Debug)]
pub struct Overlay {
    /// La scena delle annotazioni sul foglio virtuale (`super::annot`).
    pub scene: winfiles_scene::Page,
    /// mm del foglio virtuale per mm della pagina mostrata.
    pub factor: f64,
    /// Di quanto la pagina mostrata sporge da quella stampata, a sinistra e in alto (mm): i
    /// disegni CAD si guardano con un po' di carta attorno e si stampano senza.
    pub offset: (f64, f64),
    /// Dove sta l'angolo della pagina nella scena (mm del foglio virtuale): la scena si allarga
    /// per tenere anche le annotazioni fuori dalla pagina.
    pub pad: (f64, f64),
}

impl Overlay {
    /// [s, e, f] che portano le annotazioni dove va la pagina messa con `place`.
    fn place(&self, place: [f64; 3]) -> [f64; 3] {
        let [s, e, f] = place;
        let k = self.factor.max(1e-9);
        [
            s / k,
            e - s * (self.offset.0 + self.pad.0 / k),
            f - s * (self.offset.1 + self.pad.1 / k),
        ]
    }

    /// Il ritaglio `clip` della pagina stampata nelle misure della scena.
    fn clip(&self, clip: [f64; 4]) -> [f64; 4] {
        let k = self.factor;
        [
            (clip[0] + self.offset.0) * k + self.pad.0,
            (clip[1] + self.offset.1) * k + self.pad.1,
            clip[2] * k,
            clip[3] * k,
        ]
    }
}

pub struct PrintPage {
    path: PathBuf,
    kind: ViewKind,
    /// Le pagine della sorgente di stampa (mm).
    pages: Vec<Size>,
    /// mm veri per mm della pagina.
    true_mm: f64,
    source: Source,
    renderer: Renderer,
    pub choice: Choice,
    copies: u32,
    pages_mode: PagesMode,
    /// Le pagine scritte a mano: "2-6, 8".
    range_text: String,
    current_page: usize,
    sheet: usize,
    printers: PrinterScan,
    sending: Option<Receiver<Result<usize, String>>>,
    ctx: egui::Context,
    /// La scala vera del disegno CAD: mm veri per unità.
    cad_unit_mm: f64,
    /// Le pagine come si vedono nel Visualizza (mm): le annotazioni sono fatte su queste.
    shown: Vec<Size>,
    /// L'altezza della riga Annota sopra l'anteprima: resta la più alta vista.
    annot_row: f32,
}

impl PrintPage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ctx: &egui::Context,
        path: &Path,
        kind: ViewKind,
        opened: &Opened,
        page: usize,
        cad: &CadChoice,
        choice: Choice,
    ) -> Self {
        let mut choice = choice.sanitized();
        let mut true_mm = 1.0;
        let mut cad_unit_mm = 1.0;
        let (source, content, pages) = match (&opened.facts, &opened.content) {
            (Facts::Cad(f), _) => {
                // La pagina della sorgente a una scala che sta sotto il limite dei PDF, senza
                // margine; la matrice la porta alla scala scelta.
                cad_unit_mm = f.unit_mm;
                let options = winfiles_cad::PdfOptions {
                    mm_per_unit: f.page_scale,
                    hidden: cad.hidden.clone(),
                    dark: false,
                    black: choice.cad_black,
                    line_mm: Some(0.12),
                    margin_mm: 0.0,
                };
                true_mm = f.unit_mm / f.page_scale;
                let (w, h) = f.drawing.page_mm(&options);
                let pdf = f.drawing.to_pdf(&options).unwrap_or_default();
                if choice.mode == Mode::Percent {
                    choice.mode = Mode::Ratio;
                }
                (
                    Source::Cad {
                        drawing: Arc::clone(&f.drawing),
                        options,
                    },
                    Content::Pdf(Arc::new(pdf)),
                    vec![Size::new(w, h)],
                )
            }
            (_, Content::Pdf(bytes)) => {
                if choice.mode == Mode::Ratio {
                    choice.mode = Mode::Real;
                }
                (
                    Source::Pdf(Arc::clone(bytes)),
                    opened.content.clone(),
                    opened.pages.iter().map(|p| p.size).collect(),
                )
            }
            (_, Content::Image(img, _)) => {
                if choice.mode == Mode::Ratio {
                    choice.mode = Mode::Real;
                }
                let size = opened.pages.first().map_or(Size::new(100.0, 100.0), |p| p.size);
                (
                    Source::Image(Arc::clone(img), size),
                    opened.content.clone(),
                    vec![size],
                )
            }
        };
        // Un disegno più grande del foglio a 1:1 (una casa, un capannone) parte adattato.
        if cad_unit_mm > 0.0
            && matches!(opened.facts, Facts::Cad(_))
            && choice.mode == Mode::Real
            && let Some(size) = pages.first()
        {
            let one = plan(
                0,
                *size,
                true_mm,
                &Choice {
                    tiles: false,
                    ..choice.clone()
                },
            );
            let (s, _) = scale_for(*size, true_mm, &choice);
            let fits = one.first().is_some_and(|p| {
                let area = printable(p.size, choice.margin, choice.caption_mm());
                size.w * s <= area.w + 0.05 && size.h * s <= area.h + 0.05
            });
            if !fits {
                choice.mode = Mode::Fit;
            }
        }
        let wake = ctx.clone();
        let renderer = Renderer::start(content, "stampa-visualizza".into(), move || {
            wake.request_repaint()
        });
        let mut printers = PrinterScan::default();
        let wake = ctx.clone();
        printers.start(move || wake.request_repaint());
        Self {
            path: path.to_path_buf(),
            kind,
            pages,
            true_mm,
            source,
            renderer,
            choice,
            copies: 1,
            pages_mode: PagesMode::Current,
            range_text: String::new(),
            current_page: page,
            sheet: 0,
            printers,
            sending: None,
            ctx: ctx.clone(),
            cad_unit_mm,
            shown: opened.pages.iter().map(|p| p.size).collect(),
            annot_row: 0.0,
        }
    }

    /// La pagina come si vede nel Visualizza.
    fn shown_size(&self, page: usize) -> Size {
        self.shown
            .get(page)
            .or_else(|| self.pages.get(page))
            .copied()
            .unwrap_or(Size::new(100.0, 100.0))
    }

    /// Lo scarto fra la pagina mostrata e quella stampata (il margine dei disegni CAD).
    fn offset(&self, page: usize) -> (f64, f64) {
        match (self.shown.get(page), self.pages.get(page)) {
            (Some(shown), Some(printed)) => (
                ((shown.w - printed.w) / 2.0).max(0.0),
                ((shown.h - printed.h) / 2.0).max(0.0),
            ),
            _ => (0.0, 0.0),
        }
    }

    /// Le annotazioni vanno in stampa.
    fn notes_on(&self, notes: &Notes) -> bool {
        notes.annot.on || (self.choice.annotations && notes.show)
    }

    /// Il file ha annotazioni (o si stanno facendo): c'è la casella "Annotazioni".
    fn has_notes(&self, notes: &Notes) -> bool {
        notes.annot.on || (notes.show && notes.any(&self.path))
    }

    /// Le annotazioni di una pagina da stampare, se ci sono e vanno stampate.
    fn overlay(&self, notes: &Notes, page: usize) -> Option<Overlay> {
        if !self.notes_on(notes) {
            return None;
        }
        let list = notes.list(&self.path, page);
        if list.is_empty() {
            return None;
        }
        let size = self.shown_size(page);
        let (scene, pad) = super::annot::scene(&self.ctx, list, size, Unit::Mm);
        Some(Overlay {
            scene,
            factor: super::annot::factor(size),
            offset: self.offset(page),
            pad,
        })
    }

    /// Le annotazioni che escono dal foglio (o, con i tasselli, dalla pagina) e non si stampano
    /// intere.
    fn outside(&self, notes: &Notes, sheets: &[SheetPlan]) -> usize {
        if !self.notes_on(notes) {
            return 0;
        }
        let mut pages: Vec<usize> = sheets.iter().map(|sp| sp.page).collect();
        pages.dedup();
        let mut n = 0;
        for page in pages {
            let Some(sp) = sheets.iter().find(|sp| sp.page == page) else {
                continue;
            };
            let off = self.offset(page);
            let printed = self.pages.get(page).copied().unwrap_or(Size::new(0.0, 0.0));
            let [s, e, f] = sp.place;
            let s = s.max(1e-9);
            // Dove le annotazioni si stampano, in mm della pagina stampata.
            let (x0, y0, x1, y1) = if sp.tile.is_some() {
                (0.0, 0.0, printed.w, printed.h)
            } else {
                (-e / s, -f / s, (sp.size.w - e) / s, (sp.size.h - f) / s)
            };
            let tol = 0.5 / s;
            n += notes
                .bounds(&self.ctx, &self.path, page, self.shown_size(page))
                .iter()
                .filter(|b| {
                    let (bx, by) = (b.x - off.0, b.y - off.1);
                    bx < x0 - tol || by < y0 - tol || bx + b.w > x1 + tol || by + b.h > y1 + tol
                })
                .count();
        }
        n
    }

    fn cad(&self) -> bool {
        self.kind == ViewKind::Cad
    }

    /// I fogli di tutta la stampa.
    fn sheets(&self) -> Vec<SheetPlan> {
        let pages: Vec<usize> = match self.pages_mode {
            PagesMode::All => (0..self.pages.len()).collect(),
            PagesMode::Current => vec![self.current_page.min(self.pages.len().saturating_sub(1))],
            PagesMode::Range => parse_pages(&self.range_text, self.pages.len()).unwrap_or_default(),
        };
        let mut out = Vec::new();
        for p in pages {
            if let Some(size) = self.pages.get(p) {
                out.extend(plan(p, *size, self.true_mm, &self.choice));
                if out.len() > MAX_SHEETS {
                    return Vec::new();
                }
            }
        }
        out
    }

    /// La scala in parole: "100%", "1:50", "adattato al 37%".
    fn scale_text(&self, page: usize) -> String {
        let Some(size) = self.pages.get(page) else {
            return String::new();
        };
        let (s, _) = scale_for(*size, self.true_mm, &self.choice);
        let real = s / self.true_mm;
        if self.cad() {
            let n = 1.0 / real.max(1e-9);
            if (n - 1.0).abs() < 1e-6 {
                "1:1".into()
            } else {
                format!("1:{}", format_number(n, 1, true))
            }
        } else {
            format!("{}%", format_number(real * 100.0, 1, true))
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        unit: Unit,
        calibrations: &BTreeMap<String, Calibration>,
        notes: &mut Notes,
    ) -> Outcome {
        let t = Tokens::get(ui.ctx());
        self.renderer.receive(ui.ctx());
        if let Some(list) = self.printers.poll() {
            self.choice.printer = pick(&list, self.choice.printer.take());
        }
        if let Some(rx) = &self.sending {
            match rx.try_recv() {
                Ok(Ok(n)) => {
                    self.sending = None;
                    return Outcome::Status(format!(
                        "Mandato alla stampante: {n} {}",
                        if n == 1 { "foglio" } else { "fogli" }
                    ));
                }
                Ok(Err(e)) => {
                    self.sending = None;
                    return Outcome::Status(format!("Stampa non riuscita: {e}"));
                }
                Err(_) => {}
            }
        }
        let full = ui.max_rect();
        let side_w = 340.0_f32.min(full.width() * 0.45);
        let side = Rect::from_min_max(pos2(full.right() - side_w, full.top()), full.max);
        let view = Rect::from_min_max(full.min, pos2(side.left() - 10.0, full.bottom()));
        let sheets = self.sheets();
        self.sheet = self.sheet.min(sheets.len().saturating_sub(1));
        let mut out = Outcome::Keep;
        self.preview(ui, view, &sheets, unit, notes);
        crate::app::card(ui, side);
        let inner = side.shrink2(vec2(16.0, 12.0));
        let foot = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 82.0), inner.max);
        ui.painter()
            .hline(side.x_range(), foot.top() - 6.0, Stroke::new(1.0, t.divider));
        let mut foot_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(foot)
                .layout(egui::Layout::top_down(Align::Min)),
        );
        if let Some(o) = self.footer(&mut foot_ui, calibrations, &sheets, unit, notes) {
            out = o;
        }
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner.with_max_y(foot.top() - 12.0))
                .layout(egui::Layout::top_down(Align::Min)),
        );
        egui::ScrollArea::vertical()
            .id_salt("visualizza-stampa")
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.label(
                    egui::RichText::new("Stampa")
                        .font(semibold(18.0))
                        .color(t.text_strong),
                );
                ui.add(
                    egui::Label::new(egui::RichText::new(super::file_name(&self.path)).color(t.text_dim))
                        .wrap(),
                );
                ui.add_space(8.0);
                if let Some(o) = self.settings(ui, unit, calibrations, &sheets, notes) {
                    out = o;
                }
            });
        if matches!(out, Outcome::Keep) && self.sending.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
        out
    }

    fn settings(
        &mut self,
        ui: &mut egui::Ui,
        unit: Unit,
        calibrations: &BTreeMap<String, Calibration>,
        sheets: &[SheetPlan],
        notes: &Notes,
    ) -> Option<Outcome> {
        let t = Tokens::get(ui.ctx());
        let label = |ui: &mut egui::Ui, s: &str| {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(s).font(semibold(13.0)).color(t.text_strong));
        };
        label(ui, "Stampante");
        let list = self.printers.list();
        let shown = self.choice.printer.clone().unwrap_or_else(|| {
            if self.printers.searching() {
                "Cerco le stampanti…".into()
            } else {
                "Predefinita del sistema".into()
            }
        });
        egui::ComboBox::from_id_salt("visualizza-stampante")
            .icon(crate::fluent::combo_icon)
            .selected_text(shown)
            .width(ui.available_width() - 4.0)
            .show_ui(ui, |ui| {
                for p in &list {
                    let text = if p.default {
                        format!("{} (predefinita)", p.name)
                    } else {
                        p.name.clone()
                    };
                    if ui
                        .selectable_label(self.choice.printer.as_deref() == Some(&p.name), text)
                        .clicked()
                    {
                        self.choice.printer = Some(p.name.clone());
                    }
                }
            });
        let calibration = self.calibration(calibrations);
        let cal_text = match &calibration {
            Some(c) => format!("Calibrata: {}", crate::calibrate::describe(c)),
            None => "Stampante non calibrata".into(),
        };
        ui.label(egui::RichText::new(cal_text).size(12.0).color(t.text_dim));

        if self.pages.len() > 1 {
            label(ui, "Pagine");
            let before = (self.pages_mode, self.range_text.clone());
            ui.horizontal_wrapped(|ui| {
                if crate::fluent::radio(
                    ui,
                    self.pages_mode == PagesMode::Current,
                    format!("Pagina {}", self.current_page + 1),
                )
                .clicked()
                {
                    self.pages_mode = PagesMode::Current;
                }
                if crate::fluent::radio(
                    ui,
                    self.pages_mode == PagesMode::All,
                    format!("Tutte ({})", self.pages.len()),
                )
                .clicked()
                {
                    self.pages_mode = PagesMode::All;
                }
            });
            ui.horizontal(|ui| {
                if crate::fluent::radio(ui, self.pages_mode == PagesMode::Range, "Solo").clicked() {
                    self.pages_mode = PagesMode::Range;
                }
                let field = crate::fluent::text_field(
                    ui,
                    egui::TextEdit::singleline(&mut self.range_text)
                        .hint_text("es. 2-6, 8")
                        .vertical_align(Align::Center)
                        .desired_width(150.0),
                )
                .on_hover_text("Pagine e intervalli separati da virgole: 2-6 oppure 2, 5, 8 oppure 3-");
                if field.gained_focus() || field.changed() {
                    self.pages_mode = PagesMode::Range;
                }
            });
            if self.pages_mode == PagesMode::Range {
                match parse_pages(&self.range_text, self.pages.len()) {
                    Ok(list) => {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} {} su {}",
                                list.len(),
                                if list.len() == 1 { "pagina" } else { "pagine" },
                                self.pages.len()
                            ))
                            .size(12.5)
                            .color(t.text_dim),
                        );
                    }
                    Err(e) => {
                        ui.label(egui::RichText::new(e).size(12.5).color(t.warning));
                    }
                }
            }
            if before != (self.pages_mode, self.range_text.clone()) {
                self.sheet = 0;
            }
        }
        label(ui, "Foglio");
        ui.horizontal(|ui| {
            segmented(
                ui,
                "visualizza-stampa-carta",
                &mut self.choice.paper,
                &[(Paper::A4, "A4"), (Paper::A3, "A3")],
            );
        });
        ui.add_space(4.0);
        segmented(
            ui,
            "visualizza-stampa-verso",
            &mut self.choice.turn,
            &[
                (Turn::Auto, "Automatico"),
                (Turn::Portrait, "Verticale"),
                (Turn::Landscape, "Orizzontale"),
            ],
        );

        label(ui, "Scala");
        let cad = self.cad();
        let modes: Vec<(Mode, &str)> = if cad {
            vec![
                (Mode::Real, "1:1"),
                (Mode::Ratio, "1:N"),
                (Mode::Fit, "Adatta al foglio"),
            ]
        } else {
            vec![
                (Mode::Real, "Misura vera"),
                (Mode::Percent, "Percentuale"),
                (Mode::Fit, "Adatta al foglio"),
            ]
        };
        segmented(ui, "visualizza-stampa-scala", &mut self.choice.mode, &modes);
        ui.add_space(4.0);
        match self.choice.mode {
            Mode::Percent => {
                ui.horizontal(|ui| {
                    crate::fluent::number(
                        ui,
                        80.0,
                        egui::DragValue::new(&mut self.choice.percent)
                            .range(1.0..=2000.0)
                            .speed(1.0)
                            .suffix(" %"),
                    );
                    for p in [50.0, 200.0] {
                        if ui.small_button(format!("{p:.0}%")).clicked() {
                            self.choice.percent = p;
                        }
                    }
                });
            }
            Mode::Ratio => {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("1 :").color(t.text_strong));
                    crate::fluent::number(
                        ui,
                        80.0,
                        egui::DragValue::new(&mut self.choice.ratio)
                            .range(1.0..=100_000.0)
                            .speed(1.0),
                    );
                    for n in [20.0, 50.0, 100.0] {
                        if ui.small_button(format!("1:{n:.0}")).clicked() {
                            self.choice.ratio = n;
                        }
                    }
                });
            }
            _ => {}
        }
        let page = sheets.get(self.sheet).map_or(self.current_page, |s| s.page);
        if let Some(size) = self.pages.get(page) {
            let (s, _) = scale_for(*size, self.true_mm, &self.choice);
            let printed = Size::new(size.w * s, size.h * s);
            ui.label(
                egui::RichText::new(format!(
                    "Sulla carta: {} ({})",
                    format_size(printed, unit),
                    self.scale_text(page)
                ))
                .size(12.5)
                .color(t.text),
            );
            if self.choice.mode != Mode::Fit {
                let one = plan(
                    page,
                    *size,
                    self.true_mm,
                    &Choice {
                        tiles: false,
                        ..self.choice.clone()
                    },
                );
                let too_big = one.first().is_some_and(|p| {
                    let area = printable(p.size, self.choice.margin, self.choice.caption_mm());
                    printed.w > area.w + 0.05 || printed.h > area.h + 0.05
                });
                if too_big && !self.choice.tiles {
                    ui.label(
                        egui::RichText::new(
                            "Più grande del foglio: attiva i tasselli o scegli Adatta al foglio.",
                        )
                        .size(12.5)
                        .color(t.warning),
                    );
                }
            }
        }
        if self.choice.mode != Mode::Fit {
            ui.add_space(6.0);
            ui.add(crate::fluent::checkbox(&mut self.choice.tiles, "Dividi in tasselli da unire"))
                .on_hover_text("Il file a misura vera su più fogli, ognuno con un pezzo; i pezzi si sovrappongono per incollarli");
            if self.choice.tiles {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Sovrapposizione").color(t.text));
                    crate::fluent::number(
                        ui,
                        70.0,
                        egui::DragValue::new(&mut self.choice.overlap)
                            .range(0.0..=50.0)
                            .speed(0.5)
                            .suffix(" mm"),
                    );
                });
            }
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Bordo non stampabile").color(t.text));
            crate::fluent::number(
                ui,
                70.0,
                egui::DragValue::new(&mut self.choice.margin)
                    .range(0.0..=30.0)
                    .speed(0.5)
                    .suffix(" mm"),
            )
            .on_hover_text(
                "Il bordo del foglio che la stampante lascia bianco: Adatta e tasselli ne stanno dentro",
            );
        });

        label(ui, "Opzioni");
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Copie").color(t.text));
            stepper(ui, &mut self.copies, 1, 999);
        });
        ui.add(crate::fluent::checkbox(
            &mut self.choice.caption,
            "Didascalia in fondo al foglio",
        ))
        .on_hover_text("Nome del file, pagina, scala e tassello, in piccolo sotto il disegno");
        ui.add_enabled_ui(self.choice.caption, |ui| {
            ui.add(crate::fluent::checkbox(
                &mut self.choice.path,
                "Percorso del file",
            ))
            .on_hover_text("La cartella del file su una seconda riga della didascalia");
        });
        if self.has_notes(notes) {
            ui.add_enabled_ui(!notes.annot.on, |ui| {
                ui.add(crate::fluent::checkbox(
                    &mut self.choice.annotations,
                    "Annotazioni",
                ))
                .on_hover_text("Testi, frecce e forme disegnati con Annota sopra la pagina e sul foglio");
            });
            let out = self.outside(notes, sheets);
            if out > 0 {
                let text = if out == 1 {
                    "1 annotazione esce dal foglio: si stampa solo la parte dentro".to_string()
                } else {
                    format!("{out} annotazioni escono dal foglio: si stampa solo la parte dentro")
                };
                ui.add(egui::Label::new(egui::RichText::new(text).color(t.warning)).wrap());
            }
        }
        ui.add(crate::fluent::checkbox(&mut self.choice.gray, "Bianco e nero"));
        if cad {
            let before = self.choice.cad_black;
            ui.add(crate::fluent::checkbox(
                &mut self.choice.cad_black,
                "Disegno tutto in nero",
            ));
            if before != self.choice.cad_black {
                self.remake_cad();
            }
        }

        None
    }

    /// Il riepilogo e i pulsanti, sempre in vista in fondo al pannello.
    fn footer(
        &mut self,
        ui: &mut egui::Ui,
        calibrations: &BTreeMap<String, Calibration>,
        sheets: &[SheetPlan],
        unit: Unit,
        notes: &Notes,
    ) -> Option<Outcome> {
        let t = Tokens::get(ui.ctx());
        let calibration = self.calibration(calibrations);
        let count = sheets.len();
        let summary = if count == 0
            && self.pages_mode == PagesMode::Range
            && parse_pages(&self.range_text, self.pages.len()).is_err()
        {
            "Scrivi quali pagine stampare".to_string()
        } else if count == 0 {
            format!("Troppi fogli (oltre {MAX_SHEETS}): scegli una scala più piccola")
        } else {
            let total = count.saturating_mul(self.copies as usize);
            format!(
                "{count} {} {}{}",
                if count == 1 { "foglio" } else { "fogli" },
                self.choice.paper.name(),
                if self.copies > 1 {
                    format!(" × {} copie = {total}", self.copies)
                } else {
                    String::new()
                }
            )
        };
        ui.label(
            egui::RichText::new(summary)
                .font(semibold(13.5))
                .color(t.text_strong),
        );
        ui.add_space(8.0);
        let mut result = None;
        let busy = self.sending.is_some();
        ui.horizontal(|ui| {
            let can = count > 0 && !busy;
            let (fill, ink) = if can {
                (t.primary, t.on_primary)
            } else {
                (t.button, t.text_disabled)
            };
            let b = egui::Button::image_and_text(
                crate::icons::image("printer", 18.0, ink),
                egui::RichText::new(if busy { "Stampo…" } else { "Stampa" })
                    .font(semibold(15.0))
                    .color(ink),
            )
            .fill(fill)
            .min_size(vec2(150.0, 40.0));
            if ui.add_enabled(can, b).clicked() {
                self.start(sheets, calibration, unit, notes);
            }
            if busy {
                ui.add(egui::Spinner::new().color(t.accent));
            }
            if ui
                .add(egui::Button::new("Chiudi").min_size(vec2(90.0, 40.0)))
                .clicked()
            {
                result = Some(Outcome::Close);
            }
        });
        result
    }

    fn calibration(&self, calibrations: &BTreeMap<String, Calibration>) -> Option<Calibration> {
        let name = self.choice.printer.as_ref()?;
        calibrations.get(name).copied().filter(|c| !c.is_identity())
    }

    /// Rifà la sorgente del disegno CAD (nero o a colori) e la sua anteprima.
    fn remake_cad(&mut self) {
        let Source::Cad { drawing, options } = &mut self.source else {
            return;
        };
        options.black = self.choice.cad_black;
        let pdf = drawing.to_pdf(options).unwrap_or_default();
        let wake = self.ctx.clone();
        self.renderer = Renderer::start(
            Content::Pdf(Arc::new(pdf)),
            format!("stampa-visualizza-{}", u8::from(self.choice.cad_black)),
            move || wake.request_repaint(),
        );
    }

    fn preview(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        sheets: &[SheetPlan],
        unit: Unit,
        notes: &mut Notes,
    ) {
        let t = Tokens::get(ui.ctx());
        crate::app::card(ui, rect);
        let bar = Rect::from_min_size(rect.min + vec2(12.0, 8.0), vec2(rect.width() - 24.0, 34.0));
        // Qui spessori e corpi delle annotazioni si scrivono in mm della carta.
        notes.paper = sheets
            .get(self.sheet)
            .map(|sp| super::annot::factor(self.shown_size(sp.page)) / sp.place[0].max(1e-9));
        // Il tasto Annota, a destra nella barra dei fogli.
        let mut annota_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bar)
                .layout(egui::Layout::right_to_left(Align::Center)),
        );
        let on = notes.annot.on;
        let tint = if on { t.on_primary } else { t.text_strong };
        let mut b = egui::Button::image_and_text(
            crate::icons::image("pencil", 17.0, tint),
            egui::RichText::new("Annota").color(tint),
        )
        .min_size(vec2(0.0, 30.0));
        if on {
            b = b.fill(t.primary);
        }
        if annota_ui
            .add(b)
            .on_hover_text(
                "Testo, frecce, forme ed evidenziatore sul foglio, anche nel bianco attorno al \
                 file: sono le stesse annotazioni del Visualizza",
            )
            .clicked()
        {
            if on {
                notes.annot.close();
            } else {
                notes.annot.on = true;
                notes.show = true;
                self.choice.annotations = true;
            }
        }
        let bar = bar.with_max_x(annota_ui.min_rect().left() - 8.0);
        let mut top = bar.bottom() + 6.0;
        // La riga degli strumenti: va a capo quando la finestra è stretta.
        if notes.annot.on {
            let area = Rect::from_min_max(
                pos2(rect.left() + 12.0, top),
                pos2(rect.right() - 12.0, rect.bottom() - 100.0),
            );
            let mut row = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(area)
                    .layout(egui::Layout::top_down(Align::Min)),
            );
            row.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.spacing_mut().interact_size.y = 32.0;
                notes.bar(ui);
            });
            let h = row.min_rect().height().max(32.0);
            if h > self.annot_row + 0.5 {
                self.annot_row = h;
                ui.ctx().request_repaint();
            }
            top += self.annot_row + 6.0;
        } else {
            self.annot_row = 0.0;
        }
        let board = Rect::from_min_max(pos2(rect.left() + 12.0, top), rect.max - vec2(12.0, 12.0));
        ui.painter().rect_filled(board, t.radius, t.pasteboard);
        let mut bar_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bar)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        let n = sheets.len();
        if crate::icons::button(
            &mut bar_ui,
            "chevron-left",
            "Foglio precedente",
            false,
            self.sheet > 0,
            30.0,
        )
        .clicked()
        {
            self.sheet = self.sheet.saturating_sub(1);
        }
        let current = sheets.get(self.sheet);
        let mut text = if n == 0 {
            "Nessun foglio".to_string()
        } else {
            format!("Foglio {} di {n}", self.sheet + 1)
        };
        if let Some([r, c, rows, cols]) = current.and_then(|s| s.tile) {
            text.push_str(&format!(
                "  ·  tassello {} ({rows} righe × {cols} colonne)",
                tile_name(r, c)
            ));
        }
        bar_ui.label(
            egui::RichText::new(text)
                .font(semibold(13.0))
                .color(t.text_strong),
        );
        if crate::icons::button(
            &mut bar_ui,
            "chevron-right",
            "Foglio seguente",
            false,
            self.sheet + 1 < n,
            30.0,
        )
        .clicked()
        {
            self.sheet = (self.sheet + 1).min(n.saturating_sub(1));
        }
        let Some(sp) = current.cloned() else {
            return;
        };
        let k = ((board.width() - 60.0) / sp.size.w as f32)
            .min((board.height() - 60.0) / sp.size.h as f32)
            .max(0.05);
        let paper = Rect::from_center_size(board.center(), vec2(sp.size.w as f32 * k, sp.size.h as f32 * k));
        let painter = ui.painter_at(board);
        painter.rect_filled(
            paper.translate(vec2(0.0, 3.0)).expand(1.0),
            2.0,
            Color32::from_black_alpha(60),
        );
        painter.rect_filled(paper, 0.0, Color32::WHITE);
        let to = |x: f64, y: f64| pos2(paper.left() + x as f32 * k, paper.top() + y as f32 * k);
        // L'area stampabile, tratteggiata.
        let m = self.choice.margin;
        let area = printable(sp.size, m, self.choice.caption_mm());
        let ar = Rect::from_min_max(to(m, m), to(m + area.w, m + area.h));
        let dashes = [
            ar.left_top(),
            ar.right_top(),
            ar.right_bottom(),
            ar.left_bottom(),
            ar.left_top(),
        ];
        for w in dashes.windows(2) {
            if let [a, b] = w {
                painter.extend(egui::Shape::dashed_line(
                    &[*a, *b],
                    Stroke::new(1.0, Color32::from_gray(190)),
                    5.0,
                    4.0,
                ));
            }
        }
        // La pagina (o il suo pezzo) dove va sul foglio.
        let [s, e, f] = sp.place;
        let page_rect = Rect::from_min_max(
            to(e, f),
            to(
                e + s * self.pages.get(sp.page).map_or(0.0, |p| p.w),
                f + s * self.pages.get(sp.page).map_or(0.0, |p| p.h),
            ),
        );
        let c = sp.clip;
        let clip_rect = Rect::from_min_max(
            to(e + s * c[0], f + s * c[1]),
            to(e + s * (c[0] + c[2]), f + s * (c[1] + c[3])),
        )
        .intersect(paper);
        self.renderer.want_base(sp.page, 2048);
        let clipped = ui.painter_at(clip_rect.intersect(board));
        match self.renderer.base.get(&sp.page) {
            Some((_, tex)) => {
                clipped.image(
                    tex.id(),
                    page_rect,
                    Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                clipped.rect_filled(page_rect, 0.0, Color32::from_gray(235));
            }
        }
        // Le annotazioni: su un foglio intero anche nel bianco attorno alla pagina, con i
        // tasselli solo nel pezzo. Con Annota aperta si tracciano qui, sul foglio.
        if self.notes_on(notes) {
            let frame = if sp.tile.is_some() { clip_rect } else { paper };
            let sense = if notes.annot.on {
                egui::Sense::click_and_drag()
            } else {
                egui::Sense::hover()
            };
            let resp = ui.interact(board, ui.id().with("stampa-annota"), sense);
            let off = self.offset(sp.page);
            let size = self.shown_size(sp.page);
            let ks = k * s as f32;
            let shown_rect = Rect::from_min_size(
                page_rect.min - vec2(off.0 as f32, off.1 as f32) * ks,
                vec2(size.w as f32, size.h as f32) * ks,
            );
            let path = self.path.clone();
            notes.view(
                ui,
                &resp,
                frame.intersect(board),
                frame,
                shown_rect,
                ks,
                size,
                &path,
                sp.page,
                self.pages.len(),
                unit,
            );
        }
        // I bordi della sovrapposizione dei tasselli.
        if sp.tile.is_some() && self.choice.overlap > 0.0 {
            let o = self.choice.overlap as f32 * k;
            let st = Stroke::new(1.0, Color32::from_rgb(37, 99, 235));
            let [r, col, rows, cols] = sp.tile.unwrap_or([0, 0, 1, 1]);
            if col + 1 < cols {
                painter.vline(clip_rect.right() - o, clip_rect.y_range(), st);
            }
            if r + 1 < rows {
                painter.hline(clip_rect.x_range(), clip_rect.bottom() - o, st);
            }
        }
        if self.choice.caption {
            for line in self.caption(&sp, unit) {
                painter.text(
                    to(m, line.y),
                    egui::Align2::LEFT_BOTTOM,
                    line.text,
                    egui::FontId::proportional((line.size as f32 * k).clamp(4.0, 13.0)),
                    Color32::from_gray(80),
                );
            }
        }
    }

    fn caption(&self, sp: &SheetPlan, unit: Unit) -> Vec<CaptionLine> {
        let folder = self
            .choice
            .path
            .then(|| self.path.parent().map(|d| d.display().to_string()))
            .flatten();
        caption_lines(
            self.caption_text(sp, unit),
            folder,
            sp.size.w,
            sp.size.h,
            self.choice.margin,
        )
    }

    fn caption_text(&self, sp: &SheetPlan, unit: Unit) -> String {
        let mut s = super::file_name(&self.path);
        if self.pages.len() > 1 {
            s.push_str(&format!(" · pagina {}", sp.page + 1));
        }
        s.push_str(&format!(" · scala {}", self.scale_text(sp.page)));
        if let Some(size) = self.pages.get(sp.page) {
            let real = Size::new(size.w * self.true_mm, size.h * self.true_mm);
            s.push_str(&format!(" · misura vera {}", format_size(real, unit)));
        }
        if let Some([r, c, rows, cols]) = sp.tile {
            s.push_str(&format!(" · tassello {} di {}×{}", tile_name(r, c), rows, cols));
        }
        let _ = format_length;
        s
    }

    /// Compone il PDF dei fogli e lo manda alla stampante, in un thread a parte.
    fn start(&mut self, sheets: &[SheetPlan], calibration: Option<Calibration>, unit: Unit, notes: &Notes) {
        let sheets = sheets.to_vec();
        let captions: Vec<Option<Vec<CaptionLine>>> = sheets
            .iter()
            .map(|sp| self.choice.caption.then(|| self.caption(sp, unit)))
            .collect();
        let source = self.source.clone();
        let mut overlays: BTreeMap<usize, Overlay> = BTreeMap::new();
        for sp in &sheets {
            if !overlays.contains_key(&sp.page)
                && let Some(o) = self.overlay(notes, sp.page)
            {
                overlays.insert(sp.page, o);
            }
        }
        let true_mm = self.true_mm;
        let cad_unit_mm = self.cad_unit_mm;
        let margin = self.choice.margin;
        let job = craft_print::Job {
            printer: self.choice.printer.clone(),
            copies: self.copies.max(1),
            collate: true,
            duplex: Some(craft_print::Duplex::Off),
            scaling: craft_print::Scaling::Exact,
            grayscale: self.choice.gray,
            title: super::file_name(&self.path),
            options: Vec::new(),
        };
        let (tx, rx) = channel();
        let wake = self.ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("stampa-visualizza".into())
            .spawn(move || {
                let r = build(
                    &source,
                    &sheets,
                    &captions,
                    &overlays,
                    calibration,
                    true_mm,
                    cad_unit_mm,
                    margin,
                )
                .and_then(|pdf| {
                    craft_print::submit_with_progress(&pdf, &job, &mut |_, _| {})
                        .map(|_| sheets.len())
                        .map_err(|e| e.to_string())
                });
                let _ = tx.send(r);
                wake.request_repaint();
            });
        if spawned.is_ok() {
            self.sending = Some(rx);
        }
    }
}

/// Il PDF di stampa: per ogni foglio la pagina messa al suo posto, ritagliata al tassello, con la
/// didascalia; tutto passato per la correzione della stampante.
#[allow(clippy::too_many_arguments)]
fn build(
    source: &Source,
    sheets: &[SheetPlan],
    captions: &[Option<Vec<CaptionLine>>],
    overlays: &BTreeMap<usize, Overlay>,
    calibration: Option<Calibration>,
    true_mm: f64,
    cad_unit_mm: f64,
    margin: f64,
) -> Result<Vec<u8>, String> {
    let _ = cad_unit_mm;
    // La sorgente: per i disegni la si rifà con le linee dello spessore giusto sulla carta.
    let file = match source {
        Source::Pdf(bytes) => PdfFile::from_bytes(bytes.as_ref().clone()).map_err(|e| e.to_string())?,
        Source::Image(img, size) => {
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            let pdf = winfiles_pdf_out::image_page_bytes(png, *size).map_err(|e| e.to_string())?;
            PdfFile::from_bytes(pdf).map_err(|e| e.to_string())?
        }
        Source::Cad { drawing, options } => {
            // Lo spessore è in mm sulla pagina della sorgente: la matrice lo scala di `s`.
            let s = sheets.first().map_or(1.0, |sp| sp.place[0]);
            let o = winfiles_cad::PdfOptions {
                line_mm: Some(0.12 / s.max(1e-6)),
                ..options.clone()
            };
            PdfFile::from_bytes(drawing.to_pdf(&o)?).map_err(|e| e.to_string())?
        }
    };
    let _ = true_mm;
    let cal = calibration.unwrap_or(Calibration::IDENTITY);
    // x' = sx·(s·x + e) + ox
    let matrix = |s: f64, e: f64, f: f64| {
        [
            cal.scale_x * s,
            0.0,
            0.0,
            cal.scale_y * s,
            cal.scale_x * e + cal.offset_x,
            cal.scale_y * f + cal.offset_y,
        ]
    };
    let mut caption_files = Vec::new();
    for (sp, text) in sheets.iter().zip(captions) {
        let mut lines = Vec::new();
        // I segni dove incollare: le linee della sovrapposizione dei tasselli.
        if let Some([r, c, rows, cols]) = sp.tile {
            let [s, e, f] = sp.place;
            let right = e + s * (sp.clip[0] + sp.clip[2]);
            let bottom = f + s * (sp.clip[1] + sp.clip[3]);
            let left = e + s * sp.clip[0];
            let top = f + s * sp.clip[1];
            let _ = (r, rows);
            let mark = 3.0;
            // Le crocette agli angoli del pezzo.
            for (x, y) in [(left, top), (right, top), (left, bottom), (right, bottom)] {
                lines.push([x - mark, y, x + mark, y]);
                lines.push([x, y - mark, x, y + mark]);
            }
            let _ = (c, cols);
        }
        let texts: Vec<winfiles_cad::TextLine> = text
            .iter()
            .flatten()
            .map(|t| winfiles_cad::TextLine {
                x: margin,
                y: t.y,
                size: t.size,
                gray: 0.3,
                text: t.text.clone(),
            })
            .collect();
        let pdf = if texts.is_empty() && lines.is_empty() {
            None
        } else {
            let bytes = winfiles_cad::text_page(sp.size.w, sp.size.h, &texts, &lines)?;
            Some(PdfFile::from_bytes(bytes).map_err(|e| e.to_string())?)
        };
        caption_files.push(pdf);
    }
    // Le annotazioni: un PDF con una pagina per ogni pagina annotata, nell'ordine delle pagine.
    let notes = if overlays.is_empty() {
        None
    } else {
        let scenes: Vec<winfiles_scene::Page> = overlays.values().map(|o| o.scene.clone()).collect();
        let never = std::sync::atomic::AtomicBool::new(false);
        let (bytes, _) = winfiles_pdf_out::write(&scenes, &crate::export::FONTS, &never, |_| {})
            .map_err(|e| e.to_string())?;
        Some(PdfFile::from_bytes(bytes).map_err(|e| e.to_string())?)
    };
    let mut sources: Vec<&PdfFile> = vec![&file];
    if let Some(n) = &notes {
        sources.push(n);
    }
    let mut nest = Vec::new();
    for (sp, cap) in sheets.iter().zip(&caption_files) {
        let [s, e, f] = sp.place;
        let c = sp.clip;
        let clip = vec![vec![
            [c[0], c[1]],
            [c[0] + c[2], c[1]],
            [c[0] + c[2], c[1] + c[3]],
            [c[0], c[1] + c[3]],
        ]];
        let mut pieces = vec![SheetPiece {
            source: 0,
            page: sp.page,
            matrix: matrix(s, e, f),
            clip,
            hide: Vec::new(),
        }];
        if let Some(n) = overlays.keys().position(|p| *p == sp.page)
            && let Some(o) = overlays.get(&sp.page)
        {
            let [os, oe, of] = o.place(sp.place);
            // Su un foglio intero le annotazioni vanno anche nel bianco attorno alla pagina; con
            // i tasselli ognuno tiene il suo pezzo.
            let clip = if sp.tile.is_some() {
                let c = o.clip(sp.clip);
                vec![vec![
                    [c[0], c[1]],
                    [c[0] + c[2], c[1]],
                    [c[0] + c[2], c[1] + c[3]],
                    [c[0], c[1] + c[3]],
                ]]
            } else {
                Vec::new()
            };
            pieces.push(SheetPiece {
                source: 1,
                page: n,
                matrix: matrix(os, oe, of),
                clip,
                hide: Vec::new(),
            });
        }
        if let Some(cf) = cap {
            sources.push(cf);
            pieces.push(SheetPiece {
                source: sources.len() - 1,
                page: 0,
                matrix: matrix(1.0, 0.0, 0.0),
                clip: Vec::new(),
                hide: Vec::new(),
            });
        }
        nest.push(NestSheet {
            width: sp.size.w,
            height: sp.size.h,
            pieces,
            lines: Vec::new(),
        });
    }
    let doc = winfiles_pdf_edit::nest_document(&sources, &nest, None).map_err(|e| e.to_string())?;
    doc.to_bytes().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_pagine_scritte_a_mano() {
        assert_eq!(parse_pages("2-6", 10), Ok(vec![1, 2, 3, 4, 5]));
        assert_eq!(parse_pages("2, 5, 8", 10), Ok(vec![1, 4, 7]));
        assert_eq!(parse_pages("8 2;5 2", 10), Ok(vec![1, 4, 7]));
        assert_eq!(parse_pages("2-4, 3, 9-", 10), Ok(vec![1, 2, 3, 8, 9]));
        assert_eq!(parse_pages("-2", 10), Ok(vec![0, 1]));
        assert_eq!(parse_pages("6 – 4", 10), Ok(vec![3, 4, 5]));
        assert!(parse_pages("12", 10).is_err());
        assert!(parse_pages("0", 10).is_err());
        assert!(parse_pages("due", 10).is_err());
        assert!(parse_pages("  ", 10).is_err());
    }

    #[test]
    fn le_annotazioni_fuori_dalla_pagina_vanno_sul_foglio() {
        // Un A5 a misura vera al centro di un A4; un rettangolo rosso a sinistra, fuori dalla
        // pagina (nella scena, a 10 mm dal bordo: la pagina comincia a 40 mm).
        let page = winfiles_cad::text_page(148.0, 210.0, &[], &[]).unwrap();
        let mut scene = winfiles_scene::Page::new(Size::new(188.0, 210.0));
        scene.push(winfiles_scene::Item::Rect {
            rect: winfiles_geom::Rect::new(5.0, 100.0, 20.0, 20.0),
            radius: 0.0,
            fill: Some(winfiles_scene::Color::rgb(255, 0, 0)),
            stroke: None,
        });
        let overlays = BTreeMap::from([(
            0,
            Overlay {
                scene,
                factor: 1.0,
                offset: (0.0, 0.0),
                pad: (40.0, 0.0),
            },
        )]);
        let c = Choice {
            caption: false,
            mode: Mode::Real,
            ..Choice::default()
        };
        let sheets = plan(0, Size::new(148.0, 210.0), 1.0, &c);
        let pdf = build(
            &Source::Pdf(Arc::new(page)),
            &sheets,
            &[None],
            &overlays,
            None,
            1.0,
            1.0,
            4.0,
        )
        .unwrap();
        let pages = winfiles_thumbs::PdfPages::open(pdf).unwrap();
        let img = pages.render(0, 400).unwrap();
        let [s, e, f] = sheets[0].place;
        assert!((s - 1.0).abs() < 1e-9);
        let k = img.width as f64 / sheets[0].size.w;
        // Il centro del rettangolo: 15 mm nella scena = 25 mm a sinistra della pagina.
        let (x, y) = (((e - 25.0) * k) as u32, ((f + 110.0) * k) as u32);
        assert!(e > 25.0, "la pagina deve lasciare posto a sinistra");
        let i = ((y * img.width + x) * 4) as usize;
        let (r, g, b) = (img.rgba[i], img.rgba[i + 1], img.rgba[i + 2]);
        assert!(r > 200 && g < 60 && b < 60, "{r} {g} {b}");
    }

    #[test]
    fn le_annotazioni_vanno_sulla_pagina_stampata() {
        // Una pagina A4 bianca e un rettangolo rosso pieno in alto a sinistra, sul foglio virtuale.
        let page = winfiles_cad::text_page(210.0, 297.0, &[], &[]).unwrap();
        let mut scene = winfiles_scene::Page::new(Size::new(210.0, 297.0));
        scene.push(winfiles_scene::Item::Rect {
            rect: winfiles_geom::Rect::new(20.0, 20.0, 40.0, 40.0),
            radius: 0.0,
            fill: Some(winfiles_scene::Color::rgb(255, 0, 0)),
            stroke: None,
        });
        let overlays = BTreeMap::from([(
            0,
            Overlay {
                scene,
                factor: 1.0,
                offset: (0.0, 0.0),
                pad: (0.0, 0.0),
            },
        )]);
        let c = Choice {
            caption: false,
            mode: Mode::Fit,
            ..Choice::default()
        };
        let sheets = plan(0, Size::new(210.0, 297.0), 1.0, &c);
        let pdf = build(
            &Source::Pdf(Arc::new(page)),
            &sheets,
            &[None],
            &overlays,
            None,
            1.0,
            1.0,
            4.0,
        )
        .unwrap();
        let pages = winfiles_thumbs::PdfPages::open(pdf).unwrap();
        let img = pages.render(0, 297).unwrap();
        let px = |x: u32, y: u32| {
            let i = ((y * img.width + x) * 4) as usize;
            (img.rgba[i], img.rgba[i + 1], img.rgba[i + 2])
        };
        // Al centro del rettangolo (40 mm, 40 mm della pagina, ridotta ad Adatta) è rosso; fuori no.
        let [s, e, f] = sheets[0].place;
        let k = img.width as f64 / sheets[0].size.w;
        let (x, y) = (((e + s * 40.0) * k) as u32, ((f + s * 40.0) * k) as u32);
        let (r, g, b) = px(x, y);
        assert!(r > 200 && g < 60 && b < 60, "{r} {g} {b}");
        let (r, g, b) = px(((e + s * 120.0) * k) as u32, ((f + s * 150.0) * k) as u32);
        assert!(r > 200 && g > 200 && b > 200, "{r} {g} {b}");
    }

    #[test]
    fn il_percorso_va_su_una_seconda_riga_intera() {
        let lines = caption_lines(
            "a.pdf · scala 1:1".into(),
            Some("C:\\Lavori".into()),
            210.0,
            297.0,
            4.0,
        );
        assert_eq!(lines.len(), 2);
        assert!(lines[0].y < lines[1].y);
        assert!((lines[1].size - CAPTION_SIZE).abs() < 1e-9);
        // Un percorso lunghissimo si stringe ma resta intero.
        let long = format!("C:\\{}", "cartella\\".repeat(30));
        let lines = caption_lines("a.pdf".into(), Some(long.clone()), 210.0, 297.0, 4.0);
        assert_eq!(lines[1].text, long);
        assert!(lines[1].size < CAPTION_SIZE);
        assert!(lines[1].size >= CAPTION_MIN_SIZE);
    }

    #[test]
    fn con_il_percorso_la_didascalia_prende_piu_spazio() {
        let mut c = Choice::default();
        let one = c.caption_mm();
        c.path = true;
        assert!(c.caption_mm() > one);
        c.caption = false;
        assert_eq!(c.caption_mm(), 0.0);
    }

    fn a4() -> Choice {
        Choice {
            caption: false,
            ..Choice::default()
        }
    }

    #[test]
    fn un_a5_a_misura_vera_sta_al_centro_di_un_a4() {
        let p = plan(0, Size::new(148.0, 210.0), 1.0, &a4());
        assert_eq!(p.len(), 1);
        let sp = &p[0];
        assert_eq!(sp.size, Size::new(210.0, 297.0));
        assert!((sp.place[0] - 1.0).abs() < 1e-9);
        assert!((sp.place[1] - 31.0).abs() < 1e-9);
        assert!((sp.place[2] - 43.5).abs() < 1e-9);
    }

    #[test]
    fn adatta_sceglie_il_verso_che_ingrandisce_di_piu() {
        let c = Choice {
            mode: Mode::Fit,
            ..a4()
        };
        let (s, landscape) = scale_for(Size::new(1000.0, 500.0), 1.0, &c);
        assert!(landscape);
        assert!((s - (297.0 - 8.0) / 1000.0).abs() < 1e-9);
    }

    #[test]
    fn i_tasselli_coprono_tutto_con_la_sovrapposizione() {
        let c = Choice {
            tiles: true,
            overlap: 10.0,
            turn: Turn::Portrait,
            ..a4()
        };
        // 500 × 700 mm su A4 verticale: area 202 × 289, passo 192 × 279.
        let p = plan(0, Size::new(500.0, 700.0), 1.0, &c);
        assert_eq!(p.len(), 3 * 3);
        let last = p.last().unwrap();
        assert_eq!(last.tile, Some([2, 2, 3, 3]));
        assert!((last.clip[0] + last.clip[2] - 500.0).abs() < 1e-9);
        assert!((last.clip[1] + last.clip[3] - 700.0).abs() < 1e-9);
        // Ogni pezzo comincia dentro il bordo non stampabile.
        for sp in &p {
            let x = sp.place[1] + sp.place[0] * sp.clip[0];
            assert!((x - 4.0).abs() < 1e-9);
        }
    }

    #[test]
    fn la_scala_dei_disegni_e_uno_a_n() {
        let c = Choice {
            mode: Mode::Ratio,
            ratio: 50.0,
            ..a4()
        };
        // Pagina sorgente ridotta di 10 (1 mm della pagina = 10 mm veri): a 1:50 si stampa a 1/5.
        let (s, _) = scale_for(Size::new(100.0, 100.0), 10.0, &c);
        assert!((s - 0.2).abs() < 1e-9);
    }

    #[test]
    fn i_nomi_dei_tasselli() {
        assert_eq!(tile_name(0, 0), "A1");
        assert_eq!(tile_name(1, 2), "B3");
        assert_eq!(tile_name(26, 0), "AA1");
    }

    #[test]
    fn troppi_fogli_non_si_stampano() {
        let c = Choice { tiles: true, ..a4() };
        assert!(plan(0, Size::new(20_000.0, 20_000.0), 1.0, &c).is_empty());
    }

    #[test]
    fn il_pdf_di_stampa_ha_un_foglio_per_tassello() {
        let page = winfiles_cad::text_page(500.0, 700.0, &[], &[[0.0, 0.0, 500.0, 700.0]]).unwrap();
        let c = Choice {
            tiles: true,
            turn: Turn::Portrait,
            ..Choice::default()
        };
        let sheets = plan(0, Size::new(500.0, 700.0), 1.0, &c);
        let caps: Vec<Option<Vec<CaptionLine>>> = sheets
            .iter()
            .map(|sp| {
                Some(caption_lines(
                    "prova".into(),
                    Some("C:\\Lavori".into()),
                    sp.size.w,
                    sp.size.h,
                    4.0,
                ))
            })
            .collect();
        let pdf = build(
            &Source::Pdf(Arc::new(page)),
            &sheets,
            &caps,
            &BTreeMap::new(),
            None,
            1.0,
            1.0,
            4.0,
        )
        .unwrap();
        let pages = winfiles_thumbs::PdfPages::open(pdf).unwrap();
        assert_eq!(pages.len(), sheets.len());
    }
}
