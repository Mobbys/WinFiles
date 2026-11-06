//! Le annotazioni delle miniature (ADR 0030): testi, linee e frecce, rettangoli, ovali, tratti
//! dell'evidenziatore e quote disegnati sopra l'anteprima di Miniature. Si salvano nel lavoro
//! (`FileEntry::annotations`, in frazioni della miniatura) e vanno nella scena del foglio, quindi
//! anteprima, stampa e PDF le disegnano uguali.
//!
//! Mentre si traccia o si sposta, l'annotazione cambia nella copia mostrata del lavoro
//! (`WinFilesApp::shown`); a fine gesto passa nel lavoro vero, con un passo da annullare.

use std::path::PathBuf;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};
use winfiles_geom::{Rect as MmRect, Unit};
use winfiles_layout::Sheet;
use winfiles_model::{Anchor, Annotation, MAX_PER_PAGE, PageInfo, Point, Project, Rgba, ShapeStyle};
use winfiles_scene::Weight;

use crate::captions::Slot;
use crate::quotes::{GRIP, MIN_DRAWN, handle, length_text, segment_distance};
use crate::sketch::{self, PX_PER_MM, Sketch};
use crate::theme::Tokens;

/// I colori a portata di clic: quelli che si usano per segnare un errore, un'attenzione, un ok.
pub const PALETTE: [(Rgba, &str); 8] = [
    (Rgba::rgb(220, 38, 38), "Rosso"),
    (Rgba::rgb(234, 108, 0), "Arancio"),
    (Rgba::rgb(250, 204, 21), "Giallo"),
    (Rgba::rgb(22, 163, 74), "Verde"),
    (Rgba::rgb(37, 99, 235), "Blu"),
    (Rgba::rgb(147, 51, 234), "Viola"),
    (Rgba::rgb(20, 20, 20), "Nero"),
    (Rgba::rgb(255, 255, 255), "Bianco"),
];
/// Il giallo dell'evidenziatore.
pub const MARKER_YELLOW: Rgba = Rgba::rgb(255, 221, 0);
/// Opacità del riempimento e dell'evidenziatore di partenza.
pub const SOFT_ALPHA: f64 = 0.4;
/// Passi annullabili tenuti al più.
const MAX_UNDO: usize = 200;
/// Punti dell'ovale disegnato come poligono: liscio anche a tutta pagina.
const ELLIPSE_STEPS: usize = 96;

/// Lo strumento in uso.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tool {
    #[default]
    Select,
    Text,
    Arrow,
    Line,
    Rect,
    Ellipse,
    Marker,
    Quote,
}

impl Tool {
    pub const ALL: [(Tool, &'static str, &'static str); 8] = [
        (Tool::Select, "mouse-pointer-2", "Scegli e sposta (Esc)"),
        (
            Tool::Text,
            "type",
            "Testo: clic e scrivi, oppure trascina per un riquadro che va a capo",
        ),
        (Tool::Arrow, "move-up-right", "Freccia (Maiusc: dritta o a 45°)"),
        (Tool::Line, "minus", "Linea (Maiusc: dritta o a 45°)"),
        (Tool::Rect, "square", "Rettangolo (Maiusc: quadrato)"),
        (Tool::Ellipse, "circle", "Ovale (Maiusc: cerchio)"),
        (
            Tool::Marker,
            "highlighter",
            "Evidenziatore a mano libera (Maiusc: dritto)",
        ),
        (
            Tool::Quote,
            "ruler",
            "Quota: la misura vera del file tra due punti",
        ),
    ];
}

/// Contorno, riempimento o tutti e due.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Paint {
    #[default]
    Outline,
    Fill,
    Both,
}

/// Lo stile delle annotazioni nuove (e di quella scelta, quando lo si cambia). Si ricorda in
/// `impostazioni.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub color: Rgba,
    pub fill: Rgba,
    pub paint: Paint,
    /// Spessore di linee e contorni, in mm.
    pub width: f64,
    pub rounded: bool,
    pub start_arrow: bool,
    /// Corpo del testo, in mm.
    pub text_size: f64,
    pub bold: bool,
    pub background: bool,
    pub marker: Rgba,
    /// Larghezza del tratto dell'evidenziatore, in mm.
    pub marker_width: f64,
    /// Corpo della misura delle quote, in mm.
    pub quote_size: f64,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            color: PALETTE[0].0,
            fill: MARKER_YELLOW.with_alpha(SOFT_ALPHA),
            paint: Paint::Outline,
            width: 0.5,
            rounded: false,
            start_arrow: false,
            text_size: 4.0,
            bold: false,
            background: false,
            marker: MARKER_YELLOW.with_alpha(SOFT_ALPHA),
            marker_width: 4.0,
            quote_size: winfiles_model::QUOTE_SIZE,
        }
    }
}

impl Style {
    /// Riporta numeri assurdi letti dalle impostazioni a valori buoni.
    pub fn sanitized(mut self) -> Self {
        let d = Style::default();
        let fix = |v: f64, lo: f64, hi: f64, def: f64| if v.is_finite() { v.clamp(lo, hi) } else { def };
        self.width = fix(self.width, 0.1, 5.0, d.width);
        self.text_size = fix(self.text_size, 1.0, 40.0, d.text_size);
        self.marker_width = fix(self.marker_width, 1.0, 20.0, d.marker_width);
        self.quote_size = fix(self.quote_size, 1.0, 40.0, d.quote_size);
        self
    }

    fn shape_style(&self) -> ShapeStyle {
        ShapeStyle {
            stroke: (self.paint != Paint::Fill).then_some((self.color, self.width)),
            fill: (self.paint != Paint::Outline).then_some(self.fill),
        }
    }

    /// Lo stile preso da un'annotazione scelta, per far vedere i suoi valori nei comandi.
    fn take_from(&mut self, a: &Annotation) {
        match a {
            Annotation::Linea {
                color,
                width,
                start_arrow,
                ..
            } => {
                self.color = *color;
                self.width = *width;
                self.start_arrow = *start_arrow;
            }
            Annotation::Quota {
                color, width, size, ..
            } => {
                self.color = *color;
                self.width = *width;
                self.quote_size = *size;
            }
            Annotation::Rettangolo { style, rounded, .. } => {
                self.take_shape(style);
                self.rounded = *rounded;
            }
            Annotation::Ovale { style, .. } => self.take_shape(style),
            Annotation::Testo {
                color,
                size,
                bold,
                background,
                ..
            } => {
                self.color = *color;
                self.text_size = *size;
                self.bold = *bold;
                self.background = *background;
            }
            Annotation::Evidenziatore { color, width, .. } => {
                self.marker = *color;
                self.marker_width = *width;
            }
        }
    }

    fn take_shape(&mut self, s: &ShapeStyle) {
        self.paint = match (s.stroke.is_some(), s.fill.is_some()) {
            (true, true) => Paint::Both,
            (false, true) => Paint::Fill,
            _ => Paint::Outline,
        };
        if let Some((c, w)) = s.stroke {
            self.color = c;
            self.width = w;
        }
        if let Some(f) = s.fill {
            self.fill = f;
        }
    }

    /// `a` con lo stile scelto: forma e testo restano.
    fn apply_to(&self, a: &Annotation) -> Annotation {
        let mut out = a.clone();
        match &mut out {
            Annotation::Linea {
                color,
                width,
                start_arrow,
                ..
            } => {
                *color = self.color;
                *width = self.width;
                *start_arrow = self.start_arrow;
            }
            Annotation::Quota {
                color, width, size, ..
            } => {
                *color = self.color;
                *width = self.width;
                *size = self.quote_size;
            }
            Annotation::Rettangolo { style, rounded, .. } => {
                *style = self.shape_style();
                *rounded = self.rounded;
            }
            Annotation::Ovale { style, .. } => *style = self.shape_style(),
            Annotation::Testo {
                color,
                size,
                bold,
                background,
                ..
            } => {
                *color = self.color;
                *size = self.text_size;
                *bold = self.bold;
                *background = self.background;
            }
            Annotation::Evidenziatore { color, width, .. } => {
                *color = self.marker;
                *width = self.marker_width;
            }
        }
        out
    }
}

/// Colore della scena da quello salvato.
pub fn c32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

pub fn rgba(c: Color32) -> Rgba {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Rgba { r, g, b, a }
}

/// Dove sta sul foglio, in mm, un punto della miniatura `image`.
pub fn to_mm(image: MmRect, p: Point) -> (f64, f64) {
    (image.x + p.0 * image.w, image.y + p.1 * image.h)
}

/// Il punto della miniatura `image` che sta in `(x, y)` mm sul foglio.
pub fn to_frac(image: MmRect, (x, y): (f64, f64)) -> Point {
    let w = if image.w > 1e-9 { image.w } else { 1.0 };
    let h = if image.h > 1e-9 { image.h } else { 1.0 };
    ((x - image.x) / w, (y - image.y) / h)
}

fn rect_of(a: (f64, f64), b: (f64, f64)) -> MmRect {
    MmRect::new(a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
}

/// Il testo di un'annotazione impostato in pixel della scena.
pub fn text_job(text: &str, size: f64, bold: bool, width: Option<f64>, color: Color32) -> LayoutJob {
    let weight = if bold { Weight::SemiBold } else { Weight::Regular };
    let mut job = LayoutJob::single_section(
        text.to_string(),
        TextFormat {
            font_id: sketch::font(size, weight),
            color,
            ..Default::default()
        },
    );
    job.wrap.max_width = width.map_or(f32::INFINITY, |w| (w as f32 * PX_PER_MM).max(1.0));
    job
}

/// Il margine attorno a un testo con lo sfondo, in mm.
fn text_pad(size: f64, background: bool) -> f64 {
    if background { size * 0.3 } else { 0.0 }
}

/// Il riquadro di un testo sul foglio, in mm: `w`, `h` sono quelli del testo impostato.
fn text_box(at: (f64, f64), w: f64, h: f64, size: f64, background: bool, width: Option<f64>) -> MmRect {
    let pad = text_pad(size, background);
    let w = width.map_or(w, |b| b.max(w));
    MmRect::new(at.0, at.1, w + 2.0 * pad, h + 2.0 * pad)
}

/// Dove sta sul foglio la miniatura di una pagina di un file (per percorso), se c'è.
pub type Placed<'a> = dyn Fn(&std::path::Path, usize) -> Option<MmRect> + 'a;

/// Disegna nella scena le annotazioni `list` della miniatura `image`; `page` dà la misura vera
/// alle quote, `placed` le altre miniature del foglio per le linee a cavallo di due.
pub fn draw(
    sk: &mut Sketch<'_>,
    image: MmRect,
    list: &[Annotation],
    page: Option<&PageInfo>,
    unit: Unit,
    placed: &Placed<'_>,
) {
    let scale = quote_scale(list, page);
    for a in list {
        draw_one(sk, image, &resolve(a, image, placed), page, unit, scale);
    }
}

/// `a` con il capo attaccato a un'altra miniatura portato nelle frazioni di `image`, come se
/// fosse suo: così si disegna, si sceglie e si tira come le altre linee.
pub fn resolve(a: &Annotation, image: MmRect, placed: &Placed<'_>) -> Annotation {
    let mut out = a.clone();
    if let Annotation::Linea { b, end, .. } = &mut out
        && let Some(e) = end.take()
        && let Some(other) = placed(&e.path, e.page)
    {
        *b = to_frac(image, to_mm(other, e.at));
    }
    out
}

/// La scala delle quote di una pagina: misura scritta diviso misura vera della master; 1 senza.
pub fn quote_scale(list: &[Annotation], page: Option<&PageInfo>) -> f64 {
    let Some(page) = page else { return 1.0 };
    list.iter()
        .find_map(|q| match q {
            Annotation::Quota {
                a, b, value: Some(v), ..
            } => {
                let t = true_length(page, *a, *b);
                (t > 1e-9).then(|| v / t)
            }
            _ => None,
        })
        .unwrap_or(1.0)
}

/// La misura che mostra una quota, in mm, con la scala `scale` della sua pagina.
pub fn quote_value(q: &Annotation, page: Option<&PageInfo>, scale: f64) -> Option<f64> {
    match q {
        Annotation::Quota { a, b, value, .. } => {
            value.or_else(|| page.map(|p| true_length(p, *a, *b) * scale))
        }
        _ => None,
    }
}

fn draw_one(
    sk: &mut Sketch<'_>,
    image: MmRect,
    a: &Annotation,
    page: Option<&PageInfo>,
    unit: Unit,
    scale: f64,
) {
    let m = |p: Point| to_mm(image, p);
    match a {
        Annotation::Linea {
            a,
            b,
            color,
            width,
            start_arrow,
            end_arrow,
            ..
        } => {
            // Un capo attaccato a una miniatura finita su un altro foglio non esce dal foglio.
            let size = sk.page.size;
            let keep = |(x, y): (f64, f64)| (x.clamp(0.0, size.w), y.clamp(0.0, size.h));
            arrow_line(
                sk,
                keep(m(*a)),
                keep(m(*b)),
                c32(*color),
                *width,
                *start_arrow,
                *end_arrow,
            );
        }
        Annotation::Rettangolo { a, b, style, rounded } => {
            let r = rect_of(m(*a), m(*b));
            if r.w <= 0.0 || r.h <= 0.0 {
                return;
            }
            let radius = if *rounded {
                (r.w.min(r.h) * 0.15).min(4.0)
            } else {
                0.0
            };
            sk.rect(
                r,
                radius,
                style.fill.map(c32),
                style.stroke.map(|(c, w)| (c32(c), w)),
            );
        }
        Annotation::Ovale { a, b, style } => {
            let r = rect_of(m(*a), m(*b));
            if r.w <= 0.0 || r.h <= 0.0 {
                return;
            }
            sk.path(
                ellipse_points(r),
                true,
                style.fill.map(c32),
                style.stroke.map(|(c, w)| (c32(c), w)),
            );
        }
        Annotation::Testo {
            at,
            text,
            color,
            size,
            bold,
            background,
            width,
        } => {
            if text.trim().is_empty() {
                return;
            }
            let galley = sk.layout(text_job(text, *size, *bold, *width, c32(*color)));
            let (w, h) = Sketch::size_mm(&galley);
            let at = m(*at);
            let pad = text_pad(*size, *background);
            if *background {
                let r = text_box(at, w, h, *size, true, *width);
                sk.rect(r, size * 0.2, Some(Color32::from_white_alpha(235)), None);
            }
            let weight = if *bold { Weight::SemiBold } else { Weight::Regular };
            sk.text(
                (at.0 + pad, at.1 + pad),
                &galley,
                *size,
                weight,
                c32(*color),
                None,
            );
        }
        Annotation::Evidenziatore { points, color, width } => {
            let pts: Vec<(f64, f64)> = points.iter().map(|p| m(*p)).collect();
            if pts.len() >= 2 {
                sk.path(pts, false, None, Some((c32(*color), *width)));
            }
        }
        Annotation::Quota {
            a: pa,
            b: pb,
            color,
            width,
            value,
            size,
        } => {
            let shown = quote_value(a, page, scale);
            quote(
                sk,
                m(*pa),
                m(*pb),
                c32(*color),
                *width,
                shown,
                value.is_some(),
                *size,
                unit,
            );
        }
    }
}

/// La lunghezza vera, in mm, tra due punti della pagina `page` mostrata nella miniatura.
pub fn true_length(page: &PageInfo, a: Point, b: Point) -> f64 {
    let s = page.finished();
    ((b.0 - a.0) * s.w).hypot((b.1 - a.1) * s.h)
}

fn ellipse_points(r: MmRect) -> Vec<(f64, f64)> {
    let (cx, cy, rx, ry) = (r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0, r.h / 2.0);
    (0..ELLIPSE_STEPS)
        .map(|i| {
            let t = i as f64 / ELLIPSE_STEPS as f64 * std::f64::consts::TAU;
            (cx + rx * t.cos(), cy + ry * t.sin())
        })
        .collect()
}

/// La punta di freccia in `tip` che arriva da `from`: lunghezza della punta e i tre vertici.
fn arrow_head(from: (f64, f64), tip: (f64, f64), width: f64) -> Option<(f64, [(f64, f64); 3])> {
    let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
    let len = dx.hypot(dy);
    if len <= 1e-6 {
        return None;
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = (width * 4.0 + 1.2).min(len * 0.6);
    let half = head * 0.45;
    let base = (tip.0 - ux * head, tip.1 - uy * head);
    Some((
        head,
        [
            tip,
            (base.0 - uy * half, base.1 + ux * half),
            (base.0 + uy * half, base.1 - ux * half),
        ],
    ))
}

fn arrow_line(
    sk: &mut Sketch<'_>,
    a: (f64, f64),
    b: (f64, f64),
    color: Color32,
    width: f64,
    start_arrow: bool,
    end_arrow: bool,
) {
    let len = (b.0 - a.0).hypot(b.1 - a.1);
    if len <= 1e-6 {
        return;
    }
    let (ux, uy) = ((b.0 - a.0) / len, (b.1 - a.1) / len);
    let (mut from, mut to) = (a, b);
    // La linea si ferma dentro la punta: il capo tondo non sporge oltre la freccia.
    if end_arrow && let Some((head, tri)) = arrow_head(a, b, width) {
        sk.path(tri.to_vec(), true, Some(color), None);
        to = (b.0 - ux * head * 0.7, b.1 - uy * head * 0.7);
    }
    if start_arrow && let Some((head, tri)) = arrow_head(b, a, width) {
        sk.path(tri.to_vec(), true, Some(color), None);
        from = (a.0 + ux * head * 0.7, a.1 + uy * head * 0.7);
    }
    sk.path(vec![from, to], false, None, Some((color, width)));
}

#[allow(clippy::too_many_arguments)]
fn quote(
    sk: &mut Sketch<'_>,
    a: (f64, f64),
    b: (f64, f64),
    color: Color32,
    width: f64,
    real: Option<f64>,
    master: bool,
    size: f64,
    unit: Unit,
) {
    let len = (b.0 - a.0).hypot(b.1 - a.1);
    if len <= 1e-6 {
        return;
    }
    let (ux, uy) = ((b.0 - a.0) / len, (b.1 - a.1) / len);
    // La normale verso l'alto del foglio (o verso sinistra per una quota verticale).
    let (nx, ny) = if -ux < 0.0 || (ux.abs() < 1e-9 && uy > 0.0) {
        (uy, -ux)
    } else {
        (-uy, ux)
    };
    let tick = 1.6 + width;
    sk.path(vec![a, b], false, None, Some((color, width)));
    for p in [a, b] {
        sk.path(
            vec![
                (p.0 - nx * tick, p.1 - ny * tick),
                (p.0 + nx * tick, p.1 + ny * tick),
            ],
            false,
            None,
            Some((color, width)),
        );
    }
    let Some(real) = real else { return };
    let label = length_text(Some(real), unit);
    let galley = sk.layout_line(&label, size, Weight::SemiBold);
    let (w, h) = Sketch::size_mm(&galley);
    let pad = size * 0.22;
    let mid = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let off = h / 2.0 + pad + width + size * 0.2;
    let c = (mid.0 + nx * off, mid.1 + ny * off);
    let r = MmRect::new(
        c.0 - w / 2.0 - pad,
        c.1 - h / 2.0 - pad * 0.6,
        w + 2.0 * pad,
        h + 1.2 * pad,
    );
    // La master, scritta a mano, ha l'etichetta piena: si vede quale dà la scala.
    let (fill, ink) = if master {
        (color, Color32::WHITE)
    } else {
        (Color32::from_white_alpha(235), color)
    };
    sk.rect(r, 0.8, Some(fill), Some((color, 0.2)));
    sk.text(
        (r.x + pad, r.y + pad * 0.6),
        &galley,
        size,
        Weight::SemiBold,
        ink,
        None,
    );
}

/// Una annotazione di una pagina: file e pagina come nel lavoro, e la posizione nell'elenco.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub file: usize,
    pub page: usize,
    pub index: usize,
}

/// Una maniglia dell'annotazione scelta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    /// Un capo di una linea o di una quota: `true` il secondo.
    End(bool),
    /// Un angolo o un lato di un rettangolo o di un ovale.
    Edge {
        left: bool,
        right: bool,
        top: bool,
        bottom: bool,
    },
    /// Il lato destro di un testo: la larghezza a cui va a capo.
    TextWidth,
}

#[derive(Clone, Debug)]
enum Gesture {
    /// Si traccia un'annotazione nuova, partita da `from` (mm).
    New { from: (f64, f64) },
    /// Si tira una maniglia.
    Handle { handle: Handle, start: Annotation },
    /// Si sposta tutta, partita da `from` (mm).
    Move { from: (f64, f64), start: Annotation },
}

#[derive(Clone, Debug)]
struct Drag {
    target: Target,
    image: MmRect,
    gesture: Gesture,
}

/// Un passo annullabile: le annotazioni di una pagina prima e dopo.
#[derive(Clone, Debug)]
struct Change {
    path: PathBuf,
    page: usize,
    before: Vec<Annotation>,
    after: Vec<Annotation>,
}

/// Una miniatura del foglio: dove sta e di chi è.
#[derive(Clone, Copy)]
struct Spot {
    file: usize,
    page: usize,
    image: MmRect,
    cell: MmRect,
}

fn spots(sheet: &Sheet, slots: &[Slot]) -> Vec<Spot> {
    sheet
        .items
        .iter()
        .filter_map(|item| {
            let s = slots.get(item.id)?;
            Some(Spot {
                file: s.file,
                page: s.page,
                image: item.image,
                cell: item.cell,
            })
        })
        .collect()
}

/// Le annotazioni di Miniature: strumento, stile, scelta, gesti in corso e passi da annullare.
#[derive(Default)]
pub struct Annotator {
    /// La riga degli strumenti è aperta e l'anteprima traccia annotazioni.
    pub on: bool,
    pub tool: Tool,
    pub style: Style,
    pub selected: Option<Target>,
    drag: Option<Drag>,
    /// Il testo che si sta scrivendo, con il testo com'è adesso.
    editing: Option<(Target, String)>,
    /// Il campo del testo deve prendere la tastiera al prossimo disegno.
    focus_text: bool,
    undo: Vec<Change>,
    redo: Vec<Change>,
    clipboard: Option<Annotation>,
    /// La pagina sotto il puntatore all'ultimo disegno, per incollare lì.
    hovered: Option<(usize, usize, MmRect)>,
    /// Le pagine da riportare nel lavoro alla fine del disegno.
    pending: Vec<(usize, usize)>,
    /// La misura di una quota che si sta scrivendo, com'è battuta.
    typing_value: Option<(Target, String)>,
    /// Le miniature del foglio all'ultimo disegno, con il loro file: per attaccare i capi delle
    /// linee alla miniatura su cui finiscono.
    placed: Vec<(Spot, PathBuf)>,
}

/// Il riquadro di un'annotazione a schermo, per sceglierla e per le maniglie.
fn screen_bounds(ctx: &egui::Context, a: &Annotation, image: MmRect, origin: Pos2, scale: f32) -> Rect {
    let s = |p: (f64, f64)| origin + vec2(p.0 as f32, p.1 as f32) * scale;
    match a {
        Annotation::Testo {
            at,
            text,
            size,
            bold,
            background,
            width,
            ..
        } => {
            let shown = if text.is_empty() { " " } else { text.as_str() };
            let galley =
                ctx.fonts_mut(|f| f.layout_job(text_job(shown, *size, *bold, *width, Color32::BLACK)));
            let (w, h) = Sketch::size_mm(&galley);
            let r = text_box(to_mm(image, *at), w, h, *size, *background, *width);
            Rect::from_min_max(s((r.x, r.y)), s((r.right(), r.bottom())))
        }
        _ => {
            let pts = a.points();
            let mut r = Rect::NOTHING;
            for p in pts {
                r.extend_with(s(to_mm(image, p)));
            }
            r
        }
    }
}

/// Quanto è lontano `q` (a schermo) dall'annotazione: 0 sopra, `None` se troppo lontano.
fn hit_distance(
    ctx: &egui::Context,
    a: &Annotation,
    image: MmRect,
    origin: Pos2,
    scale: f32,
    q: Pos2,
) -> Option<f32> {
    let s = |p: Point| {
        let m = to_mm(image, p);
        origin + vec2(m.0 as f32, m.1 as f32) * scale
    };
    let near = GRIP;
    let d = match a {
        Annotation::Linea { a, b, width, .. } | Annotation::Quota { a, b, width, .. } => {
            (segment_distance(q, s(*a), s(*b)) - *width as f32 * scale / 2.0).max(0.0)
        }
        Annotation::Evidenziatore { points, width, .. } => {
            let pts: Vec<Pos2> = points.iter().map(|p| s(*p)).collect();
            let d = pts
                .windows(2)
                .map(|w| segment_distance(q, w[0], w[1]))
                .fold(f32::INFINITY, f32::min);
            (d - *width as f32 * scale / 2.0).max(0.0)
        }
        Annotation::Rettangolo { a, b, style, .. } => {
            let r = Rect::from_two_pos(s(*a), s(*b));
            if style.fill.is_some() && r.contains(q) {
                0.0
            } else {
                let outside = r.distance_to_pos(q);
                let inside = (q.x - r.left())
                    .min(r.right() - q.x)
                    .min(q.y - r.top())
                    .min(r.bottom() - q.y);
                if r.contains(q) { inside } else { outside }
            }
        }
        Annotation::Ovale { a, b, style } => {
            let r = Rect::from_two_pos(s(*a), s(*b));
            let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
            if rx < 0.5 || ry < 0.5 {
                q.distance(r.center())
            } else {
                let v = q - r.center();
                let k = ((v.x / rx).powi(2) + (v.y / ry).powi(2)).sqrt();
                if style.fill.is_some() && k <= 1.0 {
                    0.0
                } else {
                    (k - 1.0).abs() * rx.min(ry)
                }
            }
        }
        Annotation::Testo { .. } => {
            let r = screen_bounds(ctx, a, image, origin, scale);
            r.distance_to_pos(q)
        }
    };
    (d <= near).then_some(d)
}

/// Le maniglie dell'annotazione, a schermo.
fn handles(
    ctx: &egui::Context,
    a: &Annotation,
    image: MmRect,
    origin: Pos2,
    scale: f32,
) -> Vec<(Handle, Pos2)> {
    let s = |p: Point| {
        let m = to_mm(image, p);
        origin + vec2(m.0 as f32, m.1 as f32) * scale
    };
    match a {
        Annotation::Linea { a, b, .. } | Annotation::Quota { a, b, .. } => {
            vec![(Handle::End(false), s(*a)), (Handle::End(true), s(*b))]
        }
        Annotation::Rettangolo { a, b, .. } | Annotation::Ovale { a, b, .. } => {
            let r = Rect::from_two_pos(s(*a), s(*b));
            let e = |left, right, top, bottom| Handle::Edge {
                left,
                right,
                top,
                bottom,
            };
            vec![
                (e(true, false, true, false), r.left_top()),
                (e(false, true, true, false), r.right_top()),
                (e(true, false, false, true), r.left_bottom()),
                (e(false, true, false, true), r.right_bottom()),
                (e(false, false, true, false), r.center_top()),
                (e(false, false, false, true), r.center_bottom()),
                (e(true, false, false, false), r.left_center()),
                (e(false, true, false, false), r.right_center()),
            ]
        }
        Annotation::Testo { .. } => {
            let r = screen_bounds(ctx, a, image, origin, scale);
            vec![(Handle::TextWidth, r.right_center())]
        }
        Annotation::Evidenziatore { .. } => Vec::new(),
    }
}

/// Il punto `to` (mm) portato sulla direzione a 0°, 45° o 90° da `from` più vicina.
fn snap45(from: (f64, f64), to: (f64, f64)) -> (f64, f64) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = dx.hypot(dy);
    if len <= 1e-9 {
        return to;
    }
    let step = std::f64::consts::FRAC_PI_4;
    let angle = (dy.atan2(dx) / step).round() * step;
    // Sulla diagonale la lunghezza è quella proiettata, così il capo segue il puntatore.
    let proj = dx * angle.cos() + dy * angle.sin();
    (from.0 + proj * angle.cos(), from.1 + proj * angle.sin())
}

/// L'angolo opposto a `from` per un quadrato (sulla carta) verso `to`.
fn square(from: (f64, f64), to: (f64, f64)) -> (f64, f64) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let side = dx.abs().max(dy.abs());
    (from.0 + side.copysign(dx), from.1 + side.copysign(dy))
}

/// La miniatura che contiene `q` (mm), o la più vicina.
fn nearest(spots: &[Spot], q: (f64, f64)) -> Option<Spot> {
    let dist = |r: MmRect| {
        let dx = (r.x - q.0).max(q.0 - r.right()).max(0.0);
        let dy = (r.y - q.1).max(q.1 - r.bottom()).max(0.0);
        dx.hypot(dy)
    };
    spots
        .iter()
        .copied()
        .min_by(|a, b| dist(a.cell).total_cmp(&dist(b.cell)))
}

/// Attacca il capo `b` delle linee alla miniatura (diversa da `own`) dentro la cui cella
/// finisce; un capo sulla propria miniatura o fuori da ogni cella resta suo.
fn anchor_lines(list: &mut [Annotation], own: Spot, placed: &[(Spot, PathBuf)]) {
    for a in list.iter_mut() {
        let Annotation::Linea { b, end, .. } = a else {
            continue;
        };
        *end = None;
        let at = to_mm(own.image, *b);
        let inside = |r: MmRect| at.0 >= r.x && at.0 <= r.right() && at.1 >= r.y && at.1 <= r.bottom();
        if inside(own.cell) {
            continue;
        }
        if let Some((s, path)) = placed.iter().find(|(s, _)| inside(s.cell)) {
            *end = Some(Anchor {
                path: path.clone(),
                page: s.page,
                at: to_frac(s.image, at),
            });
        }
    }
}

fn clamp_to(cell: MmRect, p: (f64, f64)) -> (f64, f64) {
    (p.0.clamp(cell.x, cell.right()), p.1.clamp(cell.y, cell.bottom()))
}

impl Annotator {
    /// Lo stile ricordato dalle impostazioni.
    pub fn with_style(style: Style) -> Self {
        Self {
            style: style.sanitized(),
            ..Self::default()
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Chiude la riga e lascia ciò che era a metà: un testo in corso resta com'è.
    pub fn close(&mut self) {
        self.on = false;
        self.drag = None;
        self.selected = None;
    }

    /// L'annotazione scelta, se c'è ancora.
    pub fn selected_annotation<'a>(&self, shown: &'a Project) -> Option<&'a Annotation> {
        let t = self.selected?;
        shown.files.get(t.file)?.annotations(t.page).get(t.index)
    }

    /// Riporta nel lavoro le annotazioni di `file`/`page` come sono nella copia mostrata,
    /// con un passo da annullare.
    pub fn commit(&mut self, project: &mut Project, shown: &Project, file: usize, page: usize) {
        let mut after = shown
            .files
            .get(file)
            .map(|f| f.annotations(page).to_vec())
            .unwrap_or_default();
        if let Some((own, _)) = self.placed.iter().find(|(s, _)| s.file == file && s.page == page) {
            anchor_lines(&mut after, *own, &self.placed);
        }
        self.set(project, file, page, after);
    }

    /// Tutte le quote della pagina della quota scelta tornano alla misura vera del file.
    pub fn true_sizes(&mut self, project: &mut Project) {
        let Some(t) = self.selected else { return };
        self.edit(project, t.file, t.page, |list| {
            for a in list.iter_mut() {
                if let Annotation::Quota { value, .. } = a {
                    *value = None;
                }
            }
        });
    }

    /// La misura scritta a mano (mm) della quota `t`: diventa la master della sua pagina.
    fn set_quote_value(shown: &mut Project, t: Target, mm: Option<f64>) {
        let Some(f) = shown.files.get_mut(t.file) else {
            return;
        };
        let mut list = f.annotations(t.page).to_vec();
        for (i, a) in list.iter_mut().enumerate() {
            if let Annotation::Quota { value, .. } = a {
                if i == t.index {
                    *value = mm;
                } else if mm.is_some() {
                    *value = None;
                }
            }
        }
        f.set_annotations(t.page, list);
    }

    /// Le pagine cambiate in questo disegno, da riportare nel lavoro.
    pub fn take_pending(&mut self) -> Vec<(usize, usize)> {
        std::mem::take(&mut self.pending)
    }

    /// Mette `after` come annotazioni di `file`/`page` nel lavoro, con un passo da annullare.
    fn set(&mut self, project: &mut Project, file: usize, page: usize, mut after: Vec<Annotation>) {
        let Some(f) = project.files.get_mut(file) else {
            return;
        };
        after.retain_mut(Annotation::sanitize);
        after.truncate(MAX_PER_PAGE);
        let before = f.annotations(page).to_vec();
        if before == after {
            return;
        }
        self.undo.push(Change {
            path: f.path.clone(),
            page,
            before,
            after: after.clone(),
        });
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
        f.set_annotations(page, after);
    }

    /// Cambia le annotazioni di una pagina del lavoro.
    fn edit(
        &mut self,
        project: &mut Project,
        file: usize,
        page: usize,
        f: impl FnOnce(&mut Vec<Annotation>),
    ) {
        let Some(entry) = project.files.get(file) else {
            return;
        };
        let mut list = entry.annotations(page).to_vec();
        f(&mut list);
        self.set(project, file, page, list);
    }

    pub fn undo(&mut self, project: &mut Project) -> bool {
        self.step(project, true)
    }

    pub fn redo(&mut self, project: &mut Project) -> bool {
        self.step(project, false)
    }

    fn step(&mut self, project: &mut Project, back: bool) -> bool {
        let (from, to) = if back {
            (&mut self.undo, &mut self.redo)
        } else {
            (&mut self.redo, &mut self.undo)
        };
        let Some(change) = from.pop() else {
            return false;
        };
        // Il file si cerca per percorso: nel frattempo l'elenco può essere stato riordinato.
        let Some(f) = project.files.iter_mut().find(|f| f.path == change.path) else {
            return false;
        };
        let list = if back { &change.before } else { &change.after };
        f.set_annotations(change.page, list.clone());
        to.push(change);
        self.selected = None;
        self.editing = None;
        self.drag = None;
        true
    }

    /// Toglie l'annotazione scelta.
    pub fn delete_selected(&mut self, project: &mut Project) -> bool {
        let Some(t) = self.selected.take() else {
            return false;
        };
        self.edit(project, t.file, t.page, |list| {
            if t.index < list.len() {
                list.remove(t.index);
            }
        });
        true
    }

    /// Toglie tutte le annotazioni di una pagina.
    pub fn clear_page(&mut self, project: &mut Project, file: usize, page: usize) {
        self.selected = None;
        self.editing = None;
        self.edit(project, file, page, Vec::clear);
    }

    /// Copia l'annotazione scelta.
    pub fn copy(&mut self, shown: &Project) -> bool {
        match self.selected_annotation(shown) {
            Some(a) => {
                self.clipboard = Some(a.clone());
                true
            }
            None => false,
        }
    }

    /// Incolla l'annotazione copiata sulla miniatura sotto il puntatore (o su quella della
    /// scelta), nella stessa posizione; sulla stessa pagina un po' spostata, per vederla.
    pub fn paste(&mut self, project: &mut Project) -> bool {
        let Some(mut a) = self.clipboard.clone() else {
            return false;
        };
        let target = self
            .hovered
            .map(|(f, p, _)| (f, p))
            .or_else(|| self.selected.map(|t| (t.file, t.page)));
        let Some((file, page)) = target else {
            return false;
        };
        let same = project
            .files
            .get(file)
            .is_some_and(|f| f.annotations(page).contains(&a));
        if same {
            a = a.moved(0.03, 0.03);
        }
        // Sull'altra miniatura il capo attaccato non avrebbe senso: resta suo.
        if let Annotation::Linea { end, .. } = &mut a {
            *end = None;
        }
        let mut index = 0;
        self.edit(project, file, page, |list| {
            list.push(a.clone());
            index = list.len() - 1;
        });
        self.selected = Some(Target { file, page, index });
        true
    }

    /// Lo stile cambiato dai comandi va anche sull'annotazione scelta.
    pub fn restyle_selected(&mut self, project: &mut Project) {
        let Some(t) = self.selected else { return };
        let style = self.style.clone();
        self.edit(project, t.file, t.page, |list| {
            if let Some(a) = list.get_mut(t.index) {
                *a = style.apply_to(a);
            }
        });
    }

    /// Le punte della linea scelta.
    pub fn set_arrows(&mut self, project: &mut Project, start: bool, end: bool) {
        let Some(t) = self.selected else { return };
        self.edit(project, t.file, t.page, |list| {
            if let Some(Annotation::Linea {
                start_arrow,
                end_arrow,
                ..
            }) = list.get_mut(t.index)
            {
                *start_arrow = start;
                *end_arrow = end;
            }
        });
    }

    /// I gesti sull'anteprima: `resp` è il foglio (che deve sentire anche il trascinamento),
    /// `origin` il suo angolo a schermo, `scale` i punti per mm. Le modifiche in corso vanno in
    /// `shown`; le pagine da riportare nel lavoro si prendono con [`Annotator::take_pending`].
    #[allow(clippy::too_many_arguments)]
    pub fn interact(
        &mut self,
        ui: &mut egui::Ui,
        resp: &egui::Response,
        origin: Pos2,
        scale: f32,
        sheet: &Sheet,
        slots: &[Slot],
        shown: &mut Project,
    ) {
        let ctx = ui.ctx().clone();
        let spots = spots(sheet, slots);
        let to_mm_at = |p: Pos2| {
            (
                f64::from((p.x - origin.x) / scale),
                f64::from((p.y - origin.y) / scale),
            )
        };
        // La miniatura di un punto: quella che lo contiene, o la più vicina (testi e forme
        // si possono mettere anche nei margini e sulle intestazioni).
        let spot_at = |q: (f64, f64)| nearest(&spots, q);
        let paper = MmRect::new(0.0, 0.0, sheet.size.w, sheet.size.h);
        self.placed = spots
            .iter()
            .filter_map(|s| Some((*s, shown.files.get(s.file)?.path.clone())))
            .collect();
        // Le linee con un capo attaccato a un'altra miniatura si portano nelle frazioni della
        // loro: si scelgono e si tirano come le altre. Si riattaccano quando si riportano nel
        // lavoro (`commit`).
        let placed_rect = |path: &std::path::Path, page: usize| {
            self.placed
                .iter()
                .find(|(s, p)| p == path && s.page == page)
                .map(|(s, _)| s.image)
        };
        for s in &spots {
            let Some(f) = shown.files.get_mut(s.file) else {
                continue;
            };
            let list = f.annotations(s.page);
            if list
                .iter()
                .any(|a| matches!(a, Annotation::Linea { end: Some(_), .. }))
            {
                let resolved = list.iter().map(|a| resolve(a, s.image, &placed_rect)).collect();
                f.set_annotations(s.page, resolved);
            }
        }
        let find_spot =
            |file: usize, page: usize| spots.iter().copied().find(|s| s.file == file && s.page == page);
        let shift = ui.input(|i| i.modifiers.shift);
        let pointer = ctx.pointer_latest_pos().filter(|p| resp.rect.contains(*p));
        self.hovered = pointer.and_then(|p| spot_at(to_mm_at(p)).map(|s| (s.file, s.page, s.image)));

        // Una scelta che non c'è più (file tolto, pagina fuori stampa) si lascia.
        if let Some(t) = self.selected
            && (find_spot(t.file, t.page).is_none()
                || shown
                    .files
                    .get(t.file)
                    .is_none_or(|f| t.index >= f.annotations(t.page).len()))
        {
            self.selected = None;
        }

        // Che cosa c'è sotto un punto a schermo: prima la scelta, poi le ultime disegnate.
        let hit = |shown: &Project, q: Pos2, sel: Option<Target>| -> Option<Target> {
            let mut best: Option<(f32, Target)> = None;
            for s in spots.iter().rev() {
                let Some(f) = shown.files.get(s.file) else {
                    continue;
                };
                for (index, a) in f.annotations(s.page).iter().enumerate().rev() {
                    if let Some(d) = hit_distance(&ctx, a, s.image, origin, scale, q) {
                        let t = Target {
                            file: s.file,
                            page: s.page,
                            index,
                        };
                        let d = if Some(t) == sel { d - GRIP } else { d };
                        if best.is_none_or(|(bd, _)| d < bd) {
                            best = Some((d, t));
                        }
                    }
                }
            }
            best.map(|(_, t)| t)
        };
        let annotation = |shown: &Project, t: Target| -> Option<Annotation> {
            shown.files.get(t.file)?.annotations(t.page).get(t.index).cloned()
        };
        let handle_at =
            |shown: &Project, q: Pos2, sel: Option<Target>| -> Option<(Target, Handle, Annotation, Spot)> {
                let t = sel?;
                let a = annotation(shown, t)?;
                let s = find_spot(t.file, t.page)?;
                handles(&ctx, &a, s.image, origin, scale)
                    .into_iter()
                    .find(|(_, p)| p.distance(q) <= GRIP)
                    .map(|(h, _)| (t, h, a.clone(), s))
            };

        if resp.drag_started() || resp.clicked() {
            // Un testo in corso si chiude toccando l'anteprima.
            self.finish_text(shown);
        }
        if resp.double_clicked()
            && let Some(q) = resp.interact_pointer_pos()
            && let Some(t) = hit(shown, q, self.selected)
            && let Some(Annotation::Testo { text, .. }) = annotation(shown, t)
        {
            self.selected = Some(t);
            self.editing = Some((t, text));
            self.focus_text = true;
        } else if resp.double_clicked()
            && let Some(q) = resp.interact_pointer_pos()
            && let Some(t) = hit(shown, q, self.selected)
            && let Some(a @ Annotation::Quota { .. }) = annotation(shown, t)
        {
            // Doppio clic su una quota: si scrive la misura vera, e diventa la master.
            let page = shown.files.get(t.file).and_then(|f| f.pages.get(t.page));
            let list = shown
                .files
                .get(t.file)
                .map(|f| f.annotations(t.page))
                .unwrap_or(&[]);
            let unit = shown.unit;
            let text = quote_value(&a, page, quote_scale(list, page))
                .map(|v| winfiles_geom::format_number(v / unit.mm(), unit.decimals(), true))
                .unwrap_or_default();
            self.selected = Some(t);
            self.typing_value = Some((t, text));
            self.focus_text = true;
        } else if resp.clicked()
            && let Some(q) = resp.interact_pointer_pos()
        {
            let found = hit(shown, q, self.selected);
            if self.tool == Tool::Text && found.is_none() {
                if let Some(s) = spot_at(to_mm_at(q)) {
                    self.new_text(shown, s, to_mm_at(q), None);
                }
            } else {
                self.selected = found;
                if let Some(a) = found.and_then(|t| annotation(shown, t)) {
                    self.style.take_from(&a);
                }
            }
        }

        if resp.drag_started() {
            let start = ctx
                .input(|i| i.pointer.press_origin())
                .or_else(|| resp.interact_pointer_pos());
            if let Some(q) = start {
                let at = to_mm_at(q);
                if let Some((target, handle, start, s)) = handle_at(shown, q, self.selected) {
                    self.drag = Some(Drag {
                        target,
                        image: s.image,
                        gesture: Gesture::Handle { handle, start },
                    });
                } else if let Some(t) = hit(shown, q, self.selected)
                    .filter(|t| self.tool == Tool::Select || Some(*t) == self.selected)
                    && let (Some(a), Some(s)) = (annotation(shown, t), find_spot(t.file, t.page))
                {
                    self.selected = Some(t);
                    self.style.take_from(&a);
                    self.drag = Some(Drag {
                        target: t,
                        image: s.image,
                        gesture: Gesture::Move { from: at, start: a },
                    });
                } else if self.tool != Tool::Select
                    && let Some(s) = spot_at(at)
                    && let Some(f) = shown.files.get_mut(s.file)
                    && f.annotations(s.page).len() < MAX_PER_PAGE
                {
                    let p = to_frac(s.image, at);
                    let first = self.new_shape(p, p);
                    let mut list = f.annotations(s.page).to_vec();
                    list.push(first);
                    let index = list.len() - 1;
                    f.set_annotations(s.page, list);
                    let target = Target {
                        file: s.file,
                        page: s.page,
                        index,
                    };
                    self.selected = Some(target);
                    self.drag = Some(Drag {
                        target,
                        image: s.image,
                        gesture: Gesture::New { from: at },
                    });
                }
            }
        }

        if resp.dragged()
            && let Some(drag) = self.drag.clone()
            && let Some(q) = resp.interact_pointer_pos()
        {
            let at = clamp_to(paper, to_mm_at(q));
            let t = drag.target;
            let current = annotation(shown, t);
            let changed = match (&drag.gesture, current) {
                (Gesture::New { from }, Some(cur)) => Some(self.grow_new(&cur, drag.image, *from, at, shift)),
                (Gesture::Move { from, start }, _) => {
                    let (dx, dy) = (at.0 - from.0, at.1 - from.1);
                    let (fx, fy) = (dx / drag.image.w.max(1e-9), dy / drag.image.h.max(1e-9));
                    Some(start.moved(fx, fy))
                }
                (Gesture::Handle { handle, start }, _) => {
                    Some(pull(&ctx, start, *handle, drag.image, at, shift))
                }
                _ => None,
            };
            if let Some(a) = changed
                && let Some(f) = shown.files.get_mut(t.file)
            {
                let mut list = f.annotations(t.page).to_vec();
                if let Some(slot) = list.get_mut(t.index) {
                    *slot = a;
                    f.set_annotations(t.page, list);
                }
            }
        }

        if resp.drag_stopped()
            && let Some(drag) = self.drag.take()
        {
            let t = drag.target;
            if let Gesture::New { .. } = drag.gesture
                && let Some(a) = annotation(shown, t)
            {
                let r = screen_bounds(&ctx, &a, drag.image, origin, scale);
                let small = match a {
                    Annotation::Testo { .. } => false,
                    Annotation::Linea { .. }
                    | Annotation::Quota { .. }
                    | Annotation::Evidenziatore { .. } => r.width().max(r.height()) < MIN_DRAWN,
                    _ => r.width().min(r.height()) < MIN_DRAWN,
                };
                if small && let Some(f) = shown.files.get_mut(t.file) {
                    // Un clic senza trascinare non lascia una forma vuota.
                    let mut list = f.annotations(t.page).to_vec();
                    if t.index < list.len() {
                        list.remove(t.index);
                    }
                    f.set_annotations(t.page, list);
                    self.selected = None;
                } else if let Annotation::Testo { text, .. } = a {
                    self.editing = Some((t, text));
                    self.focus_text = true;
                }
            }
            if self.editing.is_none() {
                self.pending.push((t.file, t.page));
            }
        }

        // Il cursore dice cosa si sta per fare.
        if let Some(p) = pointer {
            let cursor = if self.drag.is_some() {
                egui::CursorIcon::Grabbing
            } else if handle_at(shown, p, self.selected).is_some() {
                egui::CursorIcon::Crosshair
            } else if hit(shown, p, self.selected)
                .is_some_and(|t| self.tool == Tool::Select || Some(t) == self.selected)
            {
                egui::CursorIcon::Move
            } else if self.tool == Tool::Text {
                egui::CursorIcon::Text
            } else if self.tool == Tool::Select {
                egui::CursorIcon::Default
            } else {
                egui::CursorIcon::Crosshair
            };
            ctx.set_cursor_icon(cursor);
        }

        self.paint_selection(ui, origin, scale, &spots, shown);
        self.text_field(ui, origin, scale, &spots, shown);
        self.value_field(ui, origin, scale, &spots, shown);
    }

    /// Un testo nuovo in `at` (mm) della miniatura `s`, da scrivere subito.
    fn new_text(&mut self, shown: &mut Project, s: Spot, at: (f64, f64), width: Option<f64>) {
        let Some(f) = shown.files.get_mut(s.file) else {
            return;
        };
        if f.annotations(s.page).len() >= MAX_PER_PAGE {
            return;
        }
        let mut list = f.annotations(s.page).to_vec();
        list.push(Annotation::Testo {
            at: to_frac(s.image, at),
            text: String::new(),
            color: self.style.color,
            size: self.style.text_size,
            bold: self.style.bold,
            background: self.style.background,
            width,
        });
        let t = Target {
            file: s.file,
            page: s.page,
            index: list.len() - 1,
        };
        f.set_annotations(s.page, list);
        self.selected = Some(t);
        self.editing = Some((t, String::new()));
        self.focus_text = true;
    }

    /// L'annotazione appena iniziata in `p` (frazioni), con lo strumento e lo stile in uso.
    fn new_shape(&self, a: Point, b: Point) -> Annotation {
        let st = &self.style;
        match self.tool {
            Tool::Arrow | Tool::Line | Tool::Select => Annotation::Linea {
                a,
                b,
                color: st.color,
                width: st.width,
                start_arrow: self.tool == Tool::Arrow && st.start_arrow,
                end_arrow: self.tool == Tool::Arrow,
                end: None,
            },
            Tool::Rect => Annotation::Rettangolo {
                a,
                b,
                style: st.shape_style(),
                rounded: st.rounded,
            },
            Tool::Ellipse => Annotation::Ovale {
                a,
                b,
                style: st.shape_style(),
            },
            Tool::Marker => Annotation::Evidenziatore {
                points: vec![a, b],
                color: st.marker,
                width: st.marker_width,
            },
            Tool::Quote => Annotation::Quota {
                a,
                b,
                color: st.color,
                width: st.width.min(0.5),
                value: None,
                size: st.quote_size,
            },
            Tool::Text => Annotation::Testo {
                at: a,
                text: String::new(),
                color: st.color,
                size: st.text_size,
                bold: st.bold,
                background: st.background,
                width: None,
            },
        }
    }

    /// L'annotazione nuova mentre la si traccia da `from` a `at` (mm).
    fn grow_new(
        &self,
        cur: &Annotation,
        image: MmRect,
        from: (f64, f64),
        at: (f64, f64),
        shift: bool,
    ) -> Annotation {
        let f = |p| to_frac(image, p);
        match cur {
            Annotation::Linea { .. } | Annotation::Quota { .. } => {
                let to = if shift { snap45(from, at) } else { at };
                with_ends(cur, f(from), f(to))
            }
            Annotation::Rettangolo { .. } | Annotation::Ovale { .. } => {
                let to = if shift { square(from, at) } else { at };
                with_ends(cur, f(from), f(to))
            }
            Annotation::Evidenziatore { points, color, width } => {
                let mut points = points.clone();
                if shift {
                    points.truncate(1);
                    points.push(f(snap45(from, at)));
                } else {
                    // Un punto nuovo solo quando ci si è mossi abbastanza: tratti leggeri.
                    let last = points.last().map(|p| to_mm(image, *p)).unwrap_or(from);
                    if (at.0 - last.0).hypot(at.1 - last.1) >= 0.4
                        && points.len() < winfiles_model::MAX_PER_PAGE * 8
                    {
                        points.push(f(at));
                    }
                }
                Annotation::Evidenziatore {
                    points,
                    color: *color,
                    width: *width,
                }
            }
            Annotation::Testo { .. } => {
                // Trascinando con lo strumento Testo si sceglie la larghezza a cui va a capo.
                let w = (at.0 - from.0).abs();
                let mut out = cur.clone();
                if let Annotation::Testo { at: p, width, .. } = &mut out {
                    *p = f((from.0.min(at.0), from.1.min(at.1)));
                    *width = (w >= 5.0).then_some(w);
                }
                out
            }
        }
    }

    /// Chiude il testo che si stava scrivendo: vuoto si toglie.
    fn finish_text(&mut self, shown: &mut Project) {
        let Some((t, text)) = self.editing.take() else {
            return;
        };
        if let Some(f) = shown.files.get_mut(t.file) {
            let mut list = f.annotations(t.page).to_vec();
            if text.trim().is_empty() {
                if t.index < list.len() {
                    list.remove(t.index);
                }
                self.selected = None;
            } else if let Some(Annotation::Testo { text: old, .. }) = list.get_mut(t.index) {
                *old = text;
            }
            f.set_annotations(t.page, list);
        }
        self.pending.push((t.file, t.page));
    }

    /// Esc: chiude il testo, poi lascia la scelta, poi torna a Scegli.
    pub fn escape(&mut self, shown: &mut Project) {
        if self.editing.is_some() {
            self.finish_text(shown);
        } else if self.selected.is_some() {
            self.selected = None;
        } else {
            self.tool = Tool::Select;
        }
    }

    fn paint_selection(&self, ui: &egui::Ui, origin: Pos2, scale: f32, spots: &[Spot], shown: &Project) {
        let Some(t) = self.selected else { return };
        let Some(s) = spots.iter().find(|s| s.file == t.file && s.page == t.page) else {
            return;
        };
        let Some(a) = shown
            .files
            .get(t.file)
            .and_then(|f| f.annotations(t.page).get(t.index))
        else {
            return;
        };
        let tokens = Tokens::get(ui.ctx());
        let p = ui.painter();
        let r = screen_bounds(ui.ctx(), a, s.image, origin, scale).expand(4.0);
        let dashes = egui::Shape::dashed_line(
            &[
                r.left_top(),
                r.right_top(),
                r.right_bottom(),
                r.left_bottom(),
                r.left_top(),
            ],
            Stroke::new(1.0, tokens.accent),
            4.0,
            3.0,
        );
        p.extend(dashes);
        for (_, at) in handles(ui.ctx(), a, s.image, origin, scale) {
            handle(p, at, tokens.accent);
        }
    }

    /// Il campo dove si scrive il testo, sotto il testo sul foglio. Il foglio mostra il testo
    /// mentre lo si batte.
    fn text_field(
        &mut self,
        ui: &mut egui::Ui,
        origin: Pos2,
        scale: f32,
        spots: &[Spot],
        shown: &mut Project,
    ) {
        let Some((t, mut text)) = self.editing.clone() else {
            return;
        };
        let Some(s) = spots
            .iter()
            .find(|s| s.file == t.file && s.page == t.page)
            .copied()
        else {
            self.finish_text(shown);
            return;
        };
        let Some(a) = shown
            .files
            .get(t.file)
            .and_then(|f| f.annotations(t.page).get(t.index))
            .cloned()
        else {
            self.editing = None;
            return;
        };
        let r = screen_bounds(ui.ctx(), &a, s.image, origin, scale);
        let at = pos2(r.left(), r.bottom() + 6.0);
        let mut done = false;
        let mut lost = false;
        egui::Area::new(egui::Id::new("testo-annotazione"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(260.0);
                    let edit = egui::TextEdit::multiline(&mut text)
                        .desired_width(250.0)
                        .desired_rows(2)
                        .hint_text("Scrivi il testo…");
                    let resp = crate::fluent::text_field(ui, edit);
                    if self.focus_text {
                        resp.request_focus();
                        self.focus_text = false;
                    } else if resp.lost_focus() {
                        lost = true;
                    }
                    ui.horizontal(|ui| {
                        crate::app::dim_label(ui, "Ctrl+Invio per chiudere");
                        if ui.button("Fatto").clicked() {
                            done = true;
                        }
                    });
                    if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter)) {
                        done = true;
                    }
                });
            });
        // Il testo battuto si vede subito sul foglio.
        if let Some(f) = shown.files.get_mut(t.file) {
            let mut list = f.annotations(t.page).to_vec();
            if let Some(Annotation::Testo { text: old, .. }) = list.get_mut(t.index)
                && *old != text
            {
                old.clone_from(&text);
                f.set_annotations(t.page, list);
            }
        }
        self.editing = Some((t, text));
        if done || lost && !ui.input(|i| i.pointer.any_down()) {
            self.finish_text(shown);
        }
    }
}

impl Annotator {
    /// Il campo dove si scrive la misura di una quota, sotto la quota.
    fn value_field(
        &mut self,
        ui: &mut egui::Ui,
        origin: Pos2,
        scale: f32,
        spots: &[Spot],
        shown: &mut Project,
    ) {
        let Some((t, mut text)) = self.typing_value.clone() else {
            return;
        };
        let Some(s) = spots
            .iter()
            .find(|s| s.file == t.file && s.page == t.page)
            .copied()
        else {
            self.typing_value = None;
            return;
        };
        let Some(a) = shown
            .files
            .get(t.file)
            .and_then(|f| f.annotations(t.page).get(t.index))
            .cloned()
        else {
            self.typing_value = None;
            return;
        };
        let unit = shown.unit;
        let r = screen_bounds(ui.ctx(), &a, s.image, origin, scale);
        let mut done = false;
        let mut cancel = false;
        egui::Area::new(egui::Id::new("misura-quota"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(r.center().x - 110.0, r.bottom() + 10.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        crate::app::dim_label(ui, &format!("Misura vera ({})", unit.symbol()));
                        let edit = egui::TextEdit::singleline(&mut text).desired_width(90.0);
                        let resp = crate::fluent::text_field(ui, edit);
                        if self.focus_text {
                            resp.request_focus();
                            self.focus_text = false;
                        }
                        if resp.lost_focus() {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                cancel = true;
                            } else {
                                done = true;
                            }
                        }
                        if ui.button("Fatto").clicked() {
                            done = true;
                        }
                    });
                    crate::app::dim_label(ui, "Le altre quote della miniatura si adeguano");
                });
            });
        self.typing_value = Some((t, text.clone()));
        if cancel {
            self.typing_value = None;
        } else if done {
            self.typing_value = None;
            let clean = text
                .trim()
                .trim_end_matches(unit.symbol())
                .trim()
                .replace('.', "")
                .replace(',', ".");
            let mm = if clean.is_empty() {
                None
            } else {
                clean.parse::<f64>().ok().map(|v| v * unit.mm())
            };
            if clean.is_empty() || mm.is_some_and(|v| v.is_finite() && v > 0.0 && v <= 1_000_000.0) {
                Self::set_quote_value(shown, t, mm);
                self.pending.push((t.file, t.page));
            }
        }
    }
}

/// `a` con i due punti cambiati (linee, quote, rettangoli, ovali).
fn with_ends(a: &Annotation, pa: Point, pb: Point) -> Annotation {
    let mut out = a.clone();
    match &mut out {
        Annotation::Linea { a, b, .. }
        | Annotation::Quota { a, b, .. }
        | Annotation::Rettangolo { a, b, .. }
        | Annotation::Ovale { a, b, .. } => {
            *a = pa;
            *b = pb;
        }
        _ => {}
    }
    out
}

/// `start` con la maniglia `h` portata in `at` (mm).
fn pull(
    ctx: &egui::Context,
    start: &Annotation,
    h: Handle,
    image: MmRect,
    at: (f64, f64),
    shift: bool,
) -> Annotation {
    let m = |p: Point| to_mm(image, p);
    let f = |p| to_frac(image, p);
    match (start, h) {
        (Annotation::Linea { a, b, .. } | Annotation::Quota { a, b, .. }, Handle::End(second)) => {
            let fixed = if second { m(*a) } else { m(*b) };
            let p = if shift { snap45(fixed, at) } else { at };
            if second {
                with_ends(start, *a, f(p))
            } else {
                with_ends(start, f(p), *b)
            }
        }
        (
            Annotation::Rettangolo { a, b, .. } | Annotation::Ovale { a, b, .. },
            Handle::Edge {
                left,
                right,
                top,
                bottom,
            },
        ) => {
            let r = rect_of(m(*a), m(*b));
            let (mut x0, mut y0, mut x1, mut y1) = (r.x, r.y, r.right(), r.bottom());
            if left {
                x0 = at.0;
            }
            if right {
                x1 = at.0;
            }
            if top {
                y0 = at.1;
            }
            if bottom {
                y1 = at.1;
            }
            if shift && (left || right) && (top || bottom) {
                // Con Maiusc un angolo tiene il quadrato, dall'angolo opposto.
                let fixed = (if left { x1 } else { x0 }, if top { y1 } else { y0 });
                let p = square(fixed, at);
                return with_ends(start, f(fixed), f(p));
            }
            with_ends(start, f((x0, y0)), f((x1, y1)))
        }
        (
            Annotation::Testo {
                at: p,
                text,
                size,
                bold,
                background,
                ..
            },
            Handle::TextWidth,
        ) => {
            let x = m(*p).0;
            let pad = text_pad(*size, *background);
            let w = (at.0 - x - 2.0 * pad).max(3.0);
            // Un testo corto non si allarga oltre il bisogno se non lo si chiede.
            let _ = (ctx, text, bold);
            let mut out = start.clone();
            if let Annotation::Testo { width, .. } = &mut out {
                *width = Some(w);
            }
            out
        }
        _ => start.clone(),
    }
}

/// La riga degli strumenti sopra l'anteprima. Restituisce cosa fare nel lavoro.
#[derive(Default)]
pub struct BarOutput {
    pub undo: bool,
    pub redo: bool,
    pub delete: bool,
    pub restyle: bool,
    pub arrows: Option<(bool, bool)>,
    pub true_sizes: bool,
}

impl Annotator {
    /// La riga Annota: strumenti, colori, contorno e riempimento, spessore, testo, annulla.
    /// `kind` è l'annotazione scelta, se c'è: i comandi mostrano il suo stile.
    pub fn bar(&mut self, ui: &mut egui::Ui, selected: Option<&Annotation>, show: &mut bool) -> BarOutput {
        let mut out = BarOutput::default();
        let t = Tokens::get(ui.ctx());
        ui.spacing_mut().item_spacing.x = 4.0;
        for (tool, icon, tip) in Tool::ALL {
            if crate::icons::button(ui, icon, tip, self.tool == tool, true, 30.0).clicked() {
                self.tool = tool;
                if tool != Tool::Select {
                    self.selected = None;
                }
            }
        }
        separator(ui);
        // Quale stile si mostra: quello dell'annotazione scelta o dello strumento.
        let what = match selected {
            Some(Annotation::Linea { .. }) => Tool::Arrow,
            Some(Annotation::Quota { .. }) => Tool::Quote,
            Some(Annotation::Rettangolo { .. }) => Tool::Rect,
            Some(Annotation::Ovale { .. }) => Tool::Ellipse,
            Some(Annotation::Testo { .. }) => Tool::Text,
            Some(Annotation::Evidenziatore { .. }) => Tool::Marker,
            None => self.tool,
        };
        let before = self.style.clone();
        match what {
            Tool::Select => {
                crate::app::dim_label(ui, "Scegli un'annotazione per cambiarla");
            }
            Tool::Marker => {
                crate::app::dim_label(ui, "Colore");
                swatches(ui, "evid", &mut self.style.marker, true);
                crate::app::dim_label(ui, "Larghezza");
                mm_field(ui, &mut self.style.marker_width, 1.0, 20.0);
            }
            Tool::Text => {
                crate::app::dim_label(ui, "Colore");
                swatches(ui, "testo", &mut self.style.color, false);
                crate::app::dim_label(ui, "Corpo");
                mm_field(ui, &mut self.style.text_size, 1.0, 40.0);
                ui.add(crate::fluent::checkbox(&mut self.style.bold, "Grassetto"));
                ui.add(crate::fluent::checkbox(
                    &mut self.style.background,
                    "Sfondo bianco",
                ))
                .on_hover_text("Un riquadro bianco sotto il testo, per leggerlo sopra le immagini scure");
            }
            Tool::Rect | Tool::Ellipse => {
                crate::app::segmented(
                    ui,
                    "riempimento",
                    &mut self.style.paint,
                    &[
                        (Paint::Outline, "Contorno"),
                        (Paint::Fill, "Riempimento"),
                        (Paint::Both, "Tutti e due"),
                    ],
                );
                if self.style.paint != Paint::Fill {
                    crate::app::dim_label(ui, "Colore");
                    swatches(ui, "contorno", &mut self.style.color, false);
                    mm_field(ui, &mut self.style.width, 0.1, 5.0);
                }
                if self.style.paint != Paint::Outline {
                    crate::app::dim_label(ui, "Fondo");
                    swatches(ui, "riempimento", &mut self.style.fill, true);
                }
                if what == Tool::Rect {
                    ui.add(crate::fluent::checkbox(&mut self.style.rounded, "Angoli tondi"));
                }
            }
            Tool::Arrow | Tool::Line | Tool::Quote => {
                crate::app::dim_label(ui, "Colore");
                swatches(ui, "linea", &mut self.style.color, false);
                crate::app::dim_label(ui, "Spessore");
                mm_field(ui, &mut self.style.width, 0.1, 5.0);
                if let Some(Annotation::Linea {
                    start_arrow,
                    end_arrow,
                    ..
                }) = selected
                {
                    let (mut s, mut e) = (*start_arrow, *end_arrow);
                    let a = ui
                        .add(crate::fluent::checkbox(&mut s, "Punta all'inizio"))
                        .changed();
                    let b = ui
                        .add(crate::fluent::checkbox(&mut e, "Punta alla fine"))
                        .changed();
                    if a || b {
                        out.arrows = Some((s, e));
                    }
                }
                if what == Tool::Quote {
                    crate::app::dim_label(ui, "Corpo");
                    mm_field(ui, &mut self.style.quote_size, 1.0, 40.0);
                }
                if matches!(selected, Some(Annotation::Linea { .. })) {
                    // Le punte della linea scelta sono già sopra.
                } else if let Some(Annotation::Quota { value, .. }) = selected {
                    if crate::icons::button(
                        ui,
                        "rotate-ccw",
                        "Torna alle misure vere del file",
                        false,
                        value.is_some(),
                        30.0,
                    )
                    .clicked()
                    {
                        out.true_sizes = true;
                    }
                    crate::app::dim_label(ui, "Doppio clic sulla quota per scrivere la misura");
                } else if what == Tool::Quote {
                    crate::app::dim_label(ui, "Doppio clic su una quota per scrivere la misura");
                } else if what == Tool::Arrow && self.tool == Tool::Arrow {
                    ui.add(crate::fluent::checkbox(
                        &mut self.style.start_arrow,
                        "Punta anche all'inizio",
                    ));
                }
            }
        }
        if self.style != before && selected.is_some() {
            out.restyle = true;
        }
        separator(ui);
        if crate::icons::button(ui, "undo-2", "Annulla (Ctrl+Z)", false, self.can_undo(), 30.0).clicked() {
            out.undo = true;
        }
        if crate::icons::button(ui, "redo-2", "Ripeti (Ctrl+Y)", false, self.can_redo(), 30.0).clicked() {
            out.redo = true;
        }
        if crate::icons::button(
            ui,
            "trash-2",
            "Togli l'annotazione scelta (Canc)",
            false,
            selected.is_some(),
            30.0,
        )
        .clicked()
        {
            out.delete = true;
        }
        separator(ui);
        ui.add(crate::fluent::checkbox(show, "Mostra annotazioni"))
            .on_hover_text(
                "Spento, le annotazioni restano nel lavoro ma non vanno su anteprima, stampa e PDF",
            );
        let _ = t;
        out
    }
}

fn separator(ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(9.0, 24.0), Sense::hover());
    ui.painter()
        .vline(r.center().x, r.y_range(), Stroke::new(1.0, t.divider));
}

/// Un campo in mm con una cifra decimale.
fn mm_field(ui: &mut egui::Ui, value: &mut f64, min: f64, max: f64) {
    let drag = egui::DragValue::new(value)
        .range(min..=max)
        .speed(0.05)
        .custom_formatter(|v, _| format!("{} mm", winfiles_geom::format_number(v, 1, false)))
        .custom_parser(|text| {
            let clean = text.trim().trim_end_matches("mm").trim().replace(',', ".");
            clean.parse::<f64>().ok()
        });
    crate::fluent::number(ui, 76.0, drag);
}

/// I colori a portata di clic e uno a scelta. `soft` tiene la trasparenza del colore in uso
/// (riempimento, evidenziatore).
fn swatches(ui: &mut egui::Ui, salt: &str, color: &mut Rgba, soft: bool) {
    let t = Tokens::get(ui.ctx());
    let alpha = if soft { f64::from(color.a) / 255.0 } else { 1.0 };
    for (c, name) in PALETTE {
        let c = if soft { c.with_alpha(alpha) } else { c };
        let chosen = Rgba { a: 255, ..*color } == Rgba { a: 255, ..c };
        let (r, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
        let p = ui.painter();
        let ring = if chosen { t.accent } else { t.field_border };
        p.circle_filled(r.center(), 8.0, Color32::WHITE);
        p.circle_filled(r.center(), 8.0, c32(c));
        p.circle_stroke(r.center(), 8.0, Stroke::new(if chosen { 2.0 } else { 1.0 }, ring));
        if resp.on_hover_text(name).clicked() {
            *color = c;
        }
    }
    let mut c = c32(*color);
    let id = ui.id().with(("colore-annotazione", salt));
    ui.push_id(id, |ui| {
        ui.spacing_mut().interact_size = vec2(26.0, 20.0);
        if egui::widgets::color_picker::color_edit_button_srgba(
            ui,
            &mut c,
            egui::color_picker::Alpha::OnlyBlend,
        )
        .on_hover_text("Un altro colore, con la trasparenza")
        .changed()
        {
            *color = rgba(c);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_coordinate_seguono_la_miniatura() {
        let small = MmRect::new(10.0, 20.0, 50.0, 100.0);
        let big = MmRect::new(5.0, 5.0, 200.0, 400.0);
        let p = to_frac(small, (35.0, 70.0));
        assert_eq!(p, (0.5, 0.5));
        // La stessa annotazione sulla miniatura grande sta nello stesso punto del disegno.
        assert_eq!(to_mm(big, p), (105.0, 205.0));
    }

    #[test]
    fn maiusc_tiene_la_linea_a_45_gradi() {
        let p = snap45((0.0, 0.0), (10.0, 9.0));
        assert!((p.0 - p.1).abs() < 1e-9);
        let p = snap45((0.0, 0.0), (10.0, 1.0));
        assert!(p.1.abs() < 1e-9 && (p.0 - 10.0).abs() < 1e-9);
    }

    #[test]
    fn maiusc_fa_il_quadrato() {
        assert_eq!(square((0.0, 0.0), (10.0, -4.0)), (10.0, -10.0));
    }

    #[test]
    fn la_quota_misura_la_pagina_vera() {
        let page = PageInfo {
            size: winfiles_geom::Size::new(1000.0, 500.0),
            trim: None,
        };
        // Metà larghezza = 500 mm veri, qualunque sia la grandezza della miniatura.
        assert!((true_length(&page, (0.0, 0.5), (0.5, 0.5)) - 500.0).abs() < 1e-9);
    }

    #[test]
    fn la_punta_della_freccia_sta_sul_capo() {
        let (head, tri) = arrow_head((0.0, 0.0), (20.0, 0.0), 0.5).unwrap();
        assert_eq!(tri[0], (20.0, 0.0));
        assert!(head > 0.0 && tri[1].0 < 20.0 && tri[1].1 > 0.0 && tri[2].1 < 0.0);
    }

    #[test]
    fn annulla_e_ripeti() {
        let mut project = Project::new("prova");
        project.files.push(winfiles_model::FileEntry::new(
            "/a.pdf".into(),
            winfiles_model::FileKind::Pdf,
        ));
        let mut ann = Annotator::default();
        let line = ann.new_shape((0.1, 0.1), (0.5, 0.5));
        ann.edit(&mut project, 0, 0, |l| l.push(line.clone()));
        assert_eq!(project.files[0].annotations(0).len(), 1);
        ann.selected = Some(Target {
            file: 0,
            page: 0,
            index: 0,
        });
        assert!(ann.delete_selected(&mut project));
        assert!(project.files[0].annotations(0).is_empty());
        assert!(ann.undo(&mut project));
        assert_eq!(project.files[0].annotations(0), std::slice::from_ref(&line));
        assert!(ann.redo(&mut project));
        assert!(project.files[0].annotations(0).is_empty());
    }

    #[test]
    fn lo_stile_va_sull_annotazione_scelta() {
        let st = Style {
            paint: Paint::Both,
            color: PALETTE[4].0,
            ..Style::default()
        };
        let a = Annotation::Ovale {
            a: (0.0, 0.0),
            b: (1.0, 1.0),
            style: ShapeStyle {
                stroke: None,
                fill: None,
            },
        };
        let Annotation::Ovale { style, .. } = st.apply_to(&a) else {
            panic!("tipo cambiato");
        };
        assert_eq!(style.stroke.map(|s| s.0), Some(PALETTE[4].0));
        assert!(style.fill.is_some());
    }

    fn spot(file: usize, x: f64) -> Spot {
        Spot {
            file,
            page: 0,
            image: MmRect::new(x, 10.0, 50.0, 50.0),
            cell: MmRect::new(x, 10.0, 50.0, 60.0),
        }
    }

    #[test]
    fn il_capo_che_finisce_su_un_altra_miniatura_si_attacca_a_lei() {
        let own = spot(0, 0.0);
        let other = spot(1, 100.0);
        let placed = vec![(own, PathBuf::from("/a.pdf")), (other, PathBuf::from("/b.pdf"))];
        let mut list = vec![Annotation::Linea {
            a: (0.5, 0.5),
            // 125 mm: dentro la seconda miniatura, a metà.
            b: (2.5, 0.5),
            color: PALETTE[0].0,
            width: 0.5,
            start_arrow: false,
            end_arrow: true,
            end: None,
        }];
        anchor_lines(&mut list, own, &placed);
        let Annotation::Linea { end: Some(e), .. } = &list[0] else {
            panic!("capo non attaccato");
        };
        assert_eq!(e.path, PathBuf::from("/b.pdf"));
        assert!((e.at.0 - 0.5).abs() < 1e-9 && (e.at.1 - 0.5).abs() < 1e-9);
        // Se la seconda miniatura si sposta, il capo la segue.
        let moved = MmRect::new(0.0, 200.0, 50.0, 50.0);
        let lookup = |_: &std::path::Path, _: usize| Some(moved);
        let Annotation::Linea { b, .. } = resolve(&list[0], own.image, &lookup) else {
            panic!("tipo cambiato");
        };
        assert_eq!(to_mm(own.image, b), (25.0, 225.0));
    }

    #[test]
    fn la_quota_scritta_a_mano_fa_la_scala_delle_altre() {
        let page = PageInfo {
            size: winfiles_geom::Size::new(200.0, 100.0),
            trim: None,
        };
        let q = |b: f64, value: Option<f64>| Annotation::Quota {
            size: 2.8,
            a: (0.0, 0.5),
            b: (b, 0.5),
            color: PALETTE[3].0,
            width: 0.3,
            value,
        };
        // La master misura 100 mm nel file ma è scritta 1000: scala 10.
        let list = vec![q(0.5, Some(1000.0)), q(0.25, None)];
        let scale = quote_scale(&list, Some(&page));
        assert!((scale - 10.0).abs() < 1e-9);
        assert_eq!(quote_value(&list[0], Some(&page), scale), Some(1000.0));
        assert!((quote_value(&list[1], Some(&page), scale).unwrap() - 500.0).abs() < 1e-9);
        assert_eq!(quote_scale(&[q(0.5, None)], Some(&page)), 1.0);
    }
}
