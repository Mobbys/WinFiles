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

use super::load::{CadChoice, Content, Facts, Opened, Renderer, ViewKind};
use crate::app::{segmented, stepper};
use crate::printing::{PrinterScan, pick};
use crate::theme::{Tokens, semibold};

/// Il numero massimo di fogli di una stampa: oltre, quasi certamente è un errore di scala.
pub const MAX_SHEETS: usize = 400;
/// Lo spazio in fondo al foglio per la didascalia, in mm.
const CAPTION_MM: f64 = 5.0;

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
fn printable(s: Size, margin: f64, caption: bool) -> Size {
    let bottom = if caption { CAPTION_MM } else { 0.0 };
    Size::new((s.w - 2.0 * margin).max(1.0), (s.h - 2.0 * margin - bottom).max(1.0))
}

/// La scala (mm sul foglio per mm della pagina) e l'orientamento per `content`.
/// `true_mm` sono i mm veri per mm della pagina (1 per PDF e immagini, la scala del disegno CAD).
pub fn scale_for(content: Size, true_mm: f64, c: &Choice) -> (f64, bool) {
    let wide = content.w > content.h;
    let fit = |landscape: bool| {
        let p = printable(sheet(c.paper, landscape), c.margin, c.caption);
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
        let p = printable(size, c.margin, c.caption);
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
        let p = printable(size, c.margin, c.caption);
        let step_w = (p.w - c.overlap).max(p.w * 0.5);
        let step_h = (p.h - c.overlap).max(p.h * 0.5);
        let count = |len: f64, area: f64, step: f64| -> usize {
            if len <= area + 1e-6 {
                1
            } else {
                (((len - area) / step).ceil() as usize).saturating_add(1)
            }
        };
        (size, p, step_w, step_h, count(shown.w, p.w, step_w), count(shown.h, p.h, step_h))
    };
    let p_now = printable(sheet(c.paper, landscape), c.margin, c.caption);
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
    all_pages: bool,
    current_page: usize,
    sheet: usize,
    printers: PrinterScan,
    sending: Option<Receiver<Result<usize, String>>>,
    ctx: egui::Context,
    /// La scala vera del disegno CAD: mm veri per unità.
    cad_unit_mm: f64,
}

impl PrintPage {
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
                (Source::Image(Arc::clone(img), size), opened.content.clone(), vec![size])
            }
        };
        let wake = ctx.clone();
        let renderer = Renderer::start(content, "stampa-visualizza".into(), move || wake.request_repaint());
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
            all_pages: false,
            current_page: page,
            sheet: 0,
            printers,
            sending: None,
            ctx: ctx.clone(),
            cad_unit_mm,
        }
    }

    fn cad(&self) -> bool {
        self.kind == ViewKind::Cad
    }

    /// I fogli di tutta la stampa.
    fn sheets(&self) -> Vec<SheetPlan> {
        let pages: Vec<usize> = if self.all_pages {
            (0..self.pages.len()).collect()
        } else {
            vec![self.current_page.min(self.pages.len().saturating_sub(1))]
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
        self.preview(ui, view, &sheets, unit);
        crate::app::card(ui, side);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(side.shrink2(vec2(16.0, 12.0)))
                .layout(egui::Layout::top_down(Align::Min)),
        );
        egui::ScrollArea::vertical()
            .id_salt("visualizza-stampa")
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.label(egui::RichText::new("Stampa").font(semibold(18.0)).color(t.text_strong));
                ui.add(
                    egui::Label::new(egui::RichText::new(super::file_name(&self.path)).color(t.text_dim)).wrap(),
                );
                ui.add_space(8.0);
                if let Some(o) = self.settings(ui, unit, calibrations, &sheets) {
                    out = o;
                }
            });
        if matches!(out, Outcome::Keep) && self.sending.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
        out
    }

    fn settings(
        &mut self,
        ui: &mut egui::Ui,
        unit: Unit,
        calibrations: &BTreeMap<String, Calibration>,
        sheets: &[SheetPlan],
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
                    if ui.selectable_label(self.choice.printer.as_deref() == Some(&p.name), text).clicked() {
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

        label(ui, "Foglio");
        ui.horizontal(|ui| {
            segmented(ui, "visualizza-stampa-carta", &mut self.choice.paper, &[(Paper::A4, "A4"), (Paper::A3, "A3")]);
        });
        ui.add_space(4.0);
        segmented(
            ui,
            "visualizza-stampa-verso",
            &mut self.choice.turn,
            &[(Turn::Auto, "Automatico"), (Turn::Portrait, "Verticale"), (Turn::Landscape, "Orizzontale")],
        );

        label(ui, "Scala");
        let cad = self.cad();
        let modes: Vec<(Mode, &str)> = if cad {
            vec![(Mode::Real, "1:1"), (Mode::Ratio, "1:N"), (Mode::Fit, "Adatta al foglio")]
        } else {
            vec![(Mode::Real, "Misura vera"), (Mode::Percent, "Percentuale"), (Mode::Fit, "Adatta al foglio")]
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
                        egui::DragValue::new(&mut self.choice.ratio).range(1.0..=100_000.0).speed(1.0),
                    );
                    for n in [20.0, 50.0, 100.0, 200.0] {
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
                let one = plan(page, *size, self.true_mm, &Choice { tiles: false, ..self.choice.clone() });
                let too_big = one.first().is_some_and(|p| {
                    let area = printable(p.size, self.choice.margin, self.choice.caption);
                    printed.w > area.w + 0.05 || printed.h > area.h + 0.05
                });
                if too_big && !self.choice.tiles {
                    ui.label(
                        egui::RichText::new("Più grande del foglio: attiva i tasselli o scegli Adatta al foglio.")
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
            .on_hover_text("Il bordo del foglio che la stampante lascia bianco: Adatta e tasselli ne stanno dentro");
        });

        label(ui, "Opzioni");
        if self.pages.len() > 1 {
            ui.horizontal(|ui| {
                if crate::fluent::radio(ui, !self.all_pages, format!("Pagina {}", self.current_page + 1)).clicked() {
                    self.all_pages = false;
                    self.sheet = 0;
                }
                if crate::fluent::radio(ui, self.all_pages, format!("Tutte le {} pagine", self.pages.len())).clicked() {
                    self.all_pages = true;
                    self.sheet = 0;
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Copie").color(t.text));
            stepper(ui, &mut self.copies, 1, 999);
        });
        ui.add(crate::fluent::checkbox(&mut self.choice.caption, "Didascalia in fondo al foglio"))
            .on_hover_text("Nome del file, pagina, scala e tassello, in piccolo sotto il disegno");
        ui.add(crate::fluent::checkbox(&mut self.choice.gray, "Bianco e nero"));
        if cad {
            let before = self.choice.cad_black;
            ui.add(crate::fluent::checkbox(&mut self.choice.cad_black, "Disegno tutto in nero"));
            if before != self.choice.cad_black {
                self.remake_cad();
            }
        }

        ui.add_space(14.0);
        let count = sheets.len();
        let summary = if count == 0 {
            format!("Troppi fogli (oltre {MAX_SHEETS}): scegli una scala più piccola")
        } else {
            let total = count.saturating_mul(self.copies as usize);
            format!(
                "{count} {} {}{}",
                if count == 1 { "foglio" } else { "fogli" },
                self.choice.paper.name(),
                if self.copies > 1 { format!(" × {} copie = {total}", self.copies) } else { String::new() }
            )
        };
        ui.label(egui::RichText::new(summary).font(semibold(13.5)).color(t.text_strong));
        ui.add_space(8.0);
        let mut result = None;
        let busy = self.sending.is_some();
        ui.horizontal(|ui| {
            let can = count > 0 && !busy;
            let (fill, ink) = if can { (t.primary, t.on_primary) } else { (t.button, t.text_disabled) };
            let b = egui::Button::image_and_text(
                crate::icons::image("printer", 18.0, ink),
                egui::RichText::new(if busy { "Stampo…" } else { "Stampa" })
                    .font(semibold(15.0))
                    .color(ink),
            )
            .fill(fill)
            .min_size(vec2(150.0, 40.0));
            if ui.add_enabled(can, b).clicked() {
                self.start(sheets, calibration, unit);
            }
            if busy {
                ui.add(egui::Spinner::new().color(t.accent));
            }
            if ui.add(egui::Button::new("Chiudi").min_size(vec2(90.0, 40.0))).clicked() {
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

    fn preview(&mut self, ui: &mut egui::Ui, rect: Rect, sheets: &[SheetPlan], unit: Unit) {
        let t = Tokens::get(ui.ctx());
        crate::app::card(ui, rect);
        let bar = Rect::from_min_size(rect.min + vec2(12.0, 8.0), vec2(rect.width() - 24.0, 34.0));
        let board = Rect::from_min_max(pos2(rect.left() + 12.0, bar.bottom() + 6.0), rect.max - vec2(12.0, 12.0));
        ui.painter().rect_filled(board, t.radius, t.pasteboard);
        let mut bar_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bar)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        let n = sheets.len();
        if crate::icons::button(&mut bar_ui, "chevron-left", "Foglio precedente", false, self.sheet > 0, 30.0).clicked() {
            self.sheet = self.sheet.saturating_sub(1);
        }
        let current = sheets.get(self.sheet);
        let mut text = if n == 0 { "Nessun foglio".to_string() } else { format!("Foglio {} di {n}", self.sheet + 1) };
        if let Some([r, c, rows, cols]) = current.and_then(|s| s.tile) {
            text.push_str(&format!("  ·  tassello {} ({rows} righe × {cols} colonne)", tile_name(r, c)));
        }
        bar_ui.label(egui::RichText::new(text).font(semibold(13.0)).color(t.text_strong));
        if crate::icons::button(&mut bar_ui, "chevron-right", "Foglio seguente", false, self.sheet + 1 < n, 30.0).clicked() {
            self.sheet = (self.sheet + 1).min(n.saturating_sub(1));
        }
        let Some(sp) = current.cloned() else {
            return;
        };
        let k = ((board.width() - 60.0) / sp.size.w as f32).min((board.height() - 60.0) / sp.size.h as f32).max(0.05);
        let paper = Rect::from_center_size(board.center(), vec2(sp.size.w as f32 * k, sp.size.h as f32 * k));
        let painter = ui.painter_at(board);
        painter.rect_filled(paper.translate(vec2(0.0, 3.0)).expand(1.0), 2.0, Color32::from_black_alpha(60));
        painter.rect_filled(paper, 0.0, Color32::WHITE);
        let to = |x: f64, y: f64| pos2(paper.left() + x as f32 * k, paper.top() + y as f32 * k);
        // L'area stampabile, tratteggiata.
        let m = self.choice.margin;
        let area = printable(sp.size, m, self.choice.caption);
        let ar = Rect::from_min_max(to(m, m), to(m + area.w, m + area.h));
        let dashes = [ar.left_top(), ar.right_top(), ar.right_bottom(), ar.left_bottom(), ar.left_top()];
        for w in dashes.windows(2) {
            if let [a, b] = w {
                painter.extend(egui::Shape::dashed_line(&[*a, *b], Stroke::new(1.0, Color32::from_gray(190)), 5.0, 4.0));
            }
        }
        // La pagina (o il suo pezzo) dove va sul foglio.
        let [s, e, f] = sp.place;
        let page_rect = Rect::from_min_max(
            to(e, f),
            to(e + s * self.pages.get(sp.page).map_or(0.0, |p| p.w), f + s * self.pages.get(sp.page).map_or(0.0, |p| p.h)),
        );
        let c = sp.clip;
        let clip_rect = Rect::from_min_max(to(e + s * c[0], f + s * c[1]), to(e + s * (c[0] + c[2]), f + s * (c[1] + c[3])))
            .intersect(paper);
        self.renderer.want_base(sp.page, 2048);
        let clipped = ui.painter_at(clip_rect.intersect(board));
        match self.renderer.base.get(&sp.page) {
            Some((_, tex)) => {
                clipped.image(tex.id(), page_rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                clipped.rect_filled(page_rect, 0.0, Color32::from_gray(235));
            }
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
            painter.text(
                to(m, sp.size.h - m - 1.2),
                egui::Align2::LEFT_BOTTOM,
                self.caption(&sp, unit),
                egui::FontId::proportional((2.4 * k).clamp(6.0, 13.0)),
                Color32::from_gray(80),
            );
        }
    }

    fn caption(&self, sp: &SheetPlan, unit: Unit) -> String {
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
    fn start(&mut self, sheets: &[SheetPlan], calibration: Option<Calibration>, unit: Unit) {
        let sheets = sheets.to_vec();
        let captions: Vec<Option<String>> = sheets
            .iter()
            .map(|sp| self.choice.caption.then(|| self.caption(sp, unit)))
            .collect();
        let source = self.source.clone();
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
        let spawned = std::thread::Builder::new().name("stampa-visualizza".into()).spawn(move || {
            let r = build(&source, &sheets, &captions, calibration, true_mm, cad_unit_mm, margin).and_then(|pdf| {
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
fn build(
    source: &Source,
    sheets: &[SheetPlan],
    captions: &[Option<String>],
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
        [cal.scale_x * s, 0.0, 0.0, cal.scale_y * s, cal.scale_x * e + cal.offset_x, cal.scale_y * f + cal.offset_y]
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
            .map(|t| winfiles_cad::TextLine {
                x: margin,
                y: sp.size.h - margin - 1.2,
                size: 2.4,
                gray: 0.3,
                text: t.clone(),
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
    let mut sources: Vec<&PdfFile> = vec![&file];
    let mut nest = Vec::new();
    for (sp, cap) in sheets.iter().zip(&caption_files) {
        let [s, e, f] = sp.place;
        let c = sp.clip;
        let clip = vec![vec![[c[0], c[1]], [c[0] + c[2], c[1]], [c[0] + c[2], c[1] + c[3]], [c[0], c[1] + c[3]]]];
        let mut pieces = vec![SheetPiece {
            source: 0,
            page: sp.page,
            matrix: matrix(s, e, f),
            clip,
            hide: Vec::new(),
        }];
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
        let c = Choice { mode: Mode::Fit, ..a4() };
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
        let caps: Vec<Option<String>> = sheets.iter().map(|_| Some("prova".into())).collect();
        let pdf = build(&Source::Pdf(Arc::new(page)), &sheets, &caps, None, 1.0, 1.0, 4.0).unwrap();
        let pages = winfiles_thumbs::PdfPages::open(pdf).unwrap();
        assert_eq!(pages.len(), sheets.len());
    }
}
