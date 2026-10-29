//! Lo strumento "Seleziona tracciato" del modulo PDF: si clicca una linea sulla pagina e a destra
//! se ne vedono colore, spessore, sovrastampa, ingombro e lunghezza; si cambiano colore (anche in
//! una tinta piatta con il nome che si vuole: taglio, cordonatura, bianco…), spessore e
//! sovrastampa. In cima al pannello il controllo della pagina: fustella sì o no, filetti in
//! quadricromia che sembrano una fustella, abbondanza per lato.
//!
//! La lettura della pagina gira in un altro thread; i cambi li fa `winfiles-pdf-edit`.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use egui::{Align, Pos2, Rect, Sense, Stroke, Vec2, vec2};
use serde::{Deserialize, Serialize};
use winfiles_geom::{Unit, format_length, format_number};
use winfiles_pdf_edit::{
    BleedMode, BleedSpec, Ink, NewInk, PageScan, Part, PathId, PathInfo, Restyle, Snapshot,
};

use crate::icons;
use crate::theme::{Tokens, semibold};

/// Tolleranza del clic, in punti dello schermo.
const HIT_TOLERANCE: f32 = 5.0;
/// Sotto questa abbondanza (in mm) il lato si segnala.
const MIN_BLEED: f64 = 2.0;
/// Nomi di tinte ricordati al massimo.
const MAX_INKS: usize = 64;
/// Un filetto più spesso di così (mm) non sembra una fustella.
const HAIRLINE_MAX: f64 = 0.5;

/// Il ruolo di una tinta per chi stampa e taglia.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    /// Linea di taglio (fustella).
    Cut,
    /// Cordonatura, mezzo taglio, piega.
    Crease,
    /// Bianco di stampa.
    White,
    /// Vernice, lucido, rilievo.
    Varnish,
    #[default]
    Other,
}

impl Role {
    pub const ALL: [Role; 5] = [Role::Cut, Role::Crease, Role::White, Role::Varnish, Role::Other];

    pub fn label(self) -> &'static str {
        match self {
            Role::Cut => "Taglio",
            Role::Crease => "Cordonatura",
            Role::White => "Bianco",
            Role::Varnish => "Vernice",
            Role::Other => "Altro",
        }
    }

    /// Il ruolo che fa pensare il nome (le stesse parole del pannello di Illustrator).
    pub fn guess(name: &str) -> Role {
        let n = name.to_lowercase();
        let any = |words: &[&str]| words.iter().any(|w| n.contains(w));
        if any(&[
            "cordon",
            "crease",
            "rill",
            "piega",
            "fold",
            "score",
            "groove",
            "mezzo taglio",
            "mezzotaglio",
            "half",
            "kiss",
        ]) {
            Role::Crease
        } else if any(&[
            "fustell", "taglio", "tagli", "cut", "thru", "contour", "stanz", "die", "trim", "outline",
            "contorno",
        ]) {
            Role::Cut
        } else if any(&["bianc", "white", "wht", "blanc", "weiss"]) {
            Role::White
        } else if any(&[
            "vernic", "varnish", "lucid", "gloss", "uv", "lacca", "lack", "primer", "rilievo", "emboss",
        ]) {
            Role::Varnish
        } else {
            Role::Other
        }
    }

    /// Il colore di prova di una tinta nuova con questo ruolo.
    fn cmyk(self) -> [f64; 4] {
        match self {
            Role::Cut => [0.0, 1.0, 0.0, 0.0],
            Role::Crease => [1.0, 0.0, 0.0, 0.0],
            Role::White => [0.0, 0.0, 0.0, 0.15],
            Role::Varnish => [0.0, 0.0, 0.6, 0.0],
            Role::Other => [0.0, 0.0, 0.0, 1.0],
        }
    }
}

/// Un nome di tinta ricordato, con il suo ruolo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InkName {
    pub name: String,
    pub role: Role,
    /// Il colore con cui la tinta si vede a schermo e nelle prove (C, M, Y, K da 0 a 1); senza,
    /// quello del ruolo.
    #[serde(default)]
    pub cmyk: Option<[f64; 4]>,
}

impl InkName {
    pub fn color(&self) -> [f64; 4] {
        self.cmyk.unwrap_or(self.role.cmyk())
    }
}

/// I nomi di partenza: quelli che legge Onyx in ditta (Michel, 29 ottobre 2026).
pub fn default_inks() -> Vec<InkName> {
    [
        ("Taglio", Role::Cut),
        ("Cordone", Role::Crease),
        ("Spot1", Role::White),
        ("Spot2", Role::Varnish),
    ]
    .into_iter()
    .map(|(n, r)| InkName {
        name: n.into(),
        role: r,
        cmyk: Some(r.cmyk()),
    })
    .collect()
}

/// I nomi di partenza della prima prova: chi ha solo quelli riceve i nuovi.
const OLD_DEFAULTS: [&str; 4] = ["CutContour", "Cordonatura", "Bianco", "Vernice"];

/// Toglie i nomi vuoti, troppo lunghi o doppi, e i colori impossibili.
pub fn sanitized(inks: Vec<InkName>) -> Vec<InkName> {
    if !inks.is_empty() && inks.iter().all(|i| OLD_DEFAULTS.contains(&i.name.as_str())) {
        return default_inks();
    }
    let mut out: Vec<InkName> = Vec::new();
    for mut i in inks {
        i.name = i.name.trim().to_string();
        i.cmyk = i
            .cmyk
            .filter(|c| c.iter().all(|v| v.is_finite()))
            .map(|c| c.map(|v| v.clamp(0.0, 1.0)));
        if i.name.is_empty() || i.name.len() > 127 || out.iter().any(|o| o.name == i.name) {
            continue;
        }
        out.push(i);
        if out.len() >= MAX_INKS {
            break;
        }
    }
    out
}

/// Il ruolo di una tinta: quello ricordato, o quello che fa pensare il nome.
pub fn role_of(inks: &[InkName], name: &str) -> Role {
    inks.iter()
        .find(|i| i.name == name)
        .map_or_else(|| Role::guess(name), |i| i.role)
}

/// La pagina letta.
struct Scanned {
    key: (u64, usize),
    result: Result<Arc<PageScan>, String>,
}

/// La lettura in corso: per quale versione e pagina, e da dove arriva.
type Pending = ((u64, usize), Receiver<Result<PageScan, String>>);

/// Il pannello: lo stato dello strumento.
pub struct PathTool {
    /// Lo strumento è acceso: clic sulla pagina sceglie i tracciati.
    pub on: bool,
    scanned: Option<Scanned>,
    pending: Option<Pending>,
    pub selected: BTreeSet<PathId>,
    /// Dove è cominciato il trascinamento per scegliere con un riquadro (millimetri).
    marquee: Option<[f64; 2]>,
    part: Part,
    /// Il nome della tinta da dare.
    ink_name: String,
    role: Role,
    /// Tinta piatta o quadricromia.
    mode: InkMode,
    /// Il colore da dare (C, M, Y, K da 0 a 1): della tinta a schermo, o in quadricromia.
    cmyk: [f64; 4],
    /// Il primo tracciato scelto e la parte quando i valori sono stati proposti dalla scelta.
    prefilled: Option<(PathId, Part)>,
    /// Lo spessore da dare, in millimetri.
    width: f64,
    /// Lo spessore l'ha cambiato chi usa il pannello (altrimenti non si tocca).
    width_touched: bool,
    /// La sovrastampa da dare.
    overprint: bool,
    /// La sovrastampa l'ha cambiata chi usa il pannello (o il ruolo della tinta).
    overprint_touched: bool,
    /// Tinta e saturazione della ruota, tenute anche quando il colore diventa grigio.
    hue: f64,
    sat: f64,
    /// Dopo un cambio: i tracciati da scegliere quando arriva la pagina riletta.
    after_scan: Option<Vec<PathId>>,
    /// Altezza del contenuto del pannello, tenuta mentre la pagina si rilegge (lo scorrimento resta).
    content_height: f32,
    /// L'abbondanza da creare, in mm.
    bleed_amount: f64,
    /// I lati dove crearla: sopra, destra, sotto, sinistra.
    bleed_sides: [bool; 4],
    bleed_mode: BleedMode,
    /// La lettura (versione, pagina) per cui i lati sono stati proposti.
    bleed_for: Option<(u64, usize)>,
}

impl Default for PathTool {
    fn default() -> Self {
        Self {
            on: false,
            scanned: None,
            pending: None,
            selected: BTreeSet::new(),
            marquee: None,
            part: Part::Line,
            ink_name: "Taglio".into(),
            role: Role::Cut,
            mode: InkMode::Spot,
            cmyk: Role::Cut.cmyk(),
            prefilled: None,
            width: 0.25,
            width_touched: false,
            overprint: true,
            overprint_touched: false,
            hue: 0.0,
            sat: 0.0,
            after_scan: None,
            content_height: 0.0,
            bleed_amount: 3.0,
            bleed_sides: [true; 4],
            bleed_mode: BleedMode::Mirror,
            bleed_for: None,
        }
    }
}

/// Il colore nuovo: com'è, una tinta piatta o un colore di quadricromia.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InkMode {
    Keep,
    Spot,
    Process,
}

/// Il colore di quadricromia per lo schermo.
fn cmyk_screen(c: [f64; 4]) -> egui::Color32 {
    let rgb = Ink::Cmyk(c).screen().unwrap_or([0, 0, 0]);
    egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// Da RGB (0–1) a tinta (0–1), saturazione e luminosità.
fn rgb_to_hsv([r, g, b]: [f64; 3]) -> [f64; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 1e-9 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    let s = if max <= 1e-9 { 0.0 } else { d / max };
    [h, s, max]
}

/// Da tinta, saturazione e luminosità (0–1) a RGB (0–1).
fn hsv_to_rgb([h, s, v]: [f64; 3]) -> [f64; 3] {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - (h6.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h6 {
        h if h < 1.0 => (c, x, 0.0),
        h if h < 2.0 => (x, c, 0.0),
        h if h < 3.0 => (0.0, c, x),
        h if h < 4.0 => (0.0, x, c),
        h if h < 5.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

/// Il colore di quadricromia visto a schermo, in RGB da 0 a 1.
fn cmyk_rgb([c, m, y, k]: [f64; 4]) -> [f64; 3] {
    let ink = |v: f64| (1.0 - v.clamp(0.0, 1.0)) * (1.0 - k.clamp(0.0, 1.0));
    [ink(c), ink(m), ink(y)]
}

fn rgb32([r, g, b]: [f64; 3]) -> egui::Color32 {
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgb(byte(r), byte(g), byte(b))
}

/// I colori a portata di clic sotto la ruota.
const QUICK: [(&str, [f64; 4]); 8] = [
    ("Ciano", [1.0, 0.0, 0.0, 0.0]),
    ("Magenta", [0.0, 1.0, 0.0, 0.0]),
    ("Giallo", [0.0, 0.0, 1.0, 0.0]),
    ("Nero", [0.0, 0.0, 0.0, 1.0]),
    ("Rosso", [0.0, 1.0, 1.0, 0.0]),
    ("Verde", [1.0, 0.0, 1.0, 0.0]),
    ("Blu", [1.0, 1.0, 0.0, 0.0]),
    ("Grigio chiaro", [0.0, 0.0, 0.0, 0.15]),
];

/// La ruota dei colori: giro = tinta, distanza dal centro = saturazione; sotto la barra della
/// luminosità, i colori rapidi e i valori C, M, Y, K. `hue`/`sat` tengono la tinta quando il
/// colore passa dal grigio.
fn colour_wheel(ui: &mut egui::Ui, c: &mut [f64; 4], hue: &mut f64, sat: &mut f64) {
    let t = Tokens::get(ui.ctx());
    let [h0, s0, v0] = rgb_to_hsv(cmyk_rgb(*c));
    if s0 > 0.02 && v0 > 0.02 {
        *hue = h0;
        *sat = s0;
    } else if v0 > 0.02 {
        *sat = s0;
    }
    let mut v = v0;
    let size = ui.available_width().min(168.0);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - size) / 2.0).max(0.0));
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
        let centre = rect.center();
        let radius = size / 2.0 - 2.0;
        // La ruota: anelli di triangoli con il colore nei vertici.
        let mut mesh = egui::Mesh::default();
        const SEGMENTS: u32 = 72;
        const RINGS: u32 = 6;
        let at = |seg: u32, ring: u32| {
            let a = seg as f64 / f64::from(SEGMENTS) * std::f64::consts::TAU;
            let r = f64::from(ring) / f64::from(RINGS);
            let pos = centre + vec2((a.cos() * r) as f32, -(a.sin() * r) as f32) * radius;
            let col = rgb32(hsv_to_rgb([seg as f64 / f64::from(SEGMENTS), r, v.max(0.15)]));
            (pos, col)
        };
        for ring in 0..RINGS {
            for seg in 0..SEGMENTS {
                let base = mesh.vertices.len() as u32;
                for (sg, rg) in [(seg, ring), (seg + 1, ring), (seg + 1, ring + 1), (seg, ring + 1)] {
                    let (pos, col) = at(sg, rg);
                    mesh.colored_vertex(pos, col);
                }
                mesh.add_triangle(base, base + 1, base + 2);
                mesh.add_triangle(base, base + 2, base + 3);
            }
        }
        ui.painter().add(egui::Shape::mesh(mesh));
        ui.painter()
            .circle_stroke(centre, radius, Stroke::new(1.0, t.border));
        if let Some(p) = resp.interact_pointer_pos()
            && (resp.clicked() || resp.dragged())
        {
            let d = p - centre;
            let a = f64::from(-d.y).atan2(f64::from(d.x));
            *hue = (a / std::f64::consts::TAU).rem_euclid(1.0);
            *sat = (f64::from(d.length() / radius)).clamp(0.0, 1.0);
            if v < 0.05 {
                v = 1.0;
            }
            changed = true;
        }
        let a = *hue * std::f64::consts::TAU;
        let mark = centre + vec2((a.cos() * *sat) as f32, -(a.sin() * *sat) as f32) * radius;
        ui.painter()
            .circle_stroke(mark, 6.0, Stroke::new(3.0, egui::Color32::BLACK));
        ui.painter()
            .circle_stroke(mark, 6.0, Stroke::new(1.5, egui::Color32::WHITE));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        resp.on_hover_text("Clicca o trascina per scegliere il colore");
    });
    // La luminosità: dal nero al colore pieno.
    ui.add_space(4.0);
    let (bar, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::click_and_drag());
    let mut mesh = egui::Mesh::default();
    const STEPS: u32 = 16;
    for i in 0..=STEPS {
        let f = f64::from(i) / f64::from(STEPS);
        let x = bar.left() + bar.width() * f as f32;
        let col = rgb32(hsv_to_rgb([*hue, *sat, f]));
        mesh.colored_vertex(egui::pos2(x, bar.top()), col);
        mesh.colored_vertex(egui::pos2(x, bar.bottom()), col);
        if i > 0 {
            let b = i * 2;
            mesh.add_triangle(b - 2, b - 1, b);
            mesh.add_triangle(b - 1, b, b + 1);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    ui.painter()
        .rect_stroke(bar, 3.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    if let Some(p) = resp.interact_pointer_pos()
        && (resp.clicked() || resp.dragged())
    {
        v = f64::from((p.x - bar.left()) / bar.width().max(1.0)).clamp(0.0, 1.0);
        changed = true;
    }
    let x = bar.left() + bar.width() * v as f32;
    ui.painter().rect_stroke(
        Rect::from_center_size(egui::pos2(x, bar.center().y), vec2(6.0, 20.0)),
        2.0,
        Stroke::new(2.0, t.text_strong),
        egui::StrokeKind::Middle,
    );
    resp.on_hover_text("Più chiaro o più scuro");
    if changed {
        let [r, g, b] = hsv_to_rgb([*hue, *sat, v]);
        if let Some(new) = as_cmyk(&Ink::Rgb([r, g, b])) {
            *c = new.map(|x| (x * 100.0).round() / 100.0);
        }
    }
    // I colori rapidi.
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        let side = ((ui.available_width() - 7.0 * 4.0) / 8.0).clamp(14.0, 22.0);
        for (name, q) in QUICK {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
            let on = c.iter().zip(q.iter()).all(|(a, b)| (a - b).abs() < 0.005);
            ui.painter().rect(
                r,
                4.0,
                cmyk_screen(q),
                Stroke::new(if on { 2.0 } else { 1.0 }, if on { t.accent } else { t.border }),
                egui::StrokeKind::Inside,
            );
            if resp.on_hover_text(name).clicked() {
                *c = q;
            }
        }
    });
    ui.add_space(4.0);
    egui::CollapsingHeader::new(
        egui::RichText::new(format!(
            "C {} M {} Y {} K {}",
            (c[0] * 100.0).round(),
            (c[1] * 100.0).round(),
            (c[2] * 100.0).round(),
            (c[3] * 100.0).round()
        ))
        .color(t.text_dim),
    )
    .id_salt("tracciati-cmyk")
    .show(ui, |ui| cmyk_fields(ui, c));
}

/// Quattro campi C, M, Y, K in percentuale.
fn cmyk_fields(ui: &mut egui::Ui, c: &mut [f64; 4]) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        // Quattro campi nella larghezza che c'è (il pannello è stretto).
        let field = ((ui.available_width() - 4.0 * 14.0 - 12.0) / 4.0).clamp(28.0, 48.0);
        for (v, label) in c.iter_mut().zip(["C", "M", "Y", "K"]) {
            ui.label(egui::RichText::new(label).color(t.text_dim));
            let mut pc = (*v * 100.0).round();
            let resp = crate::fluent::number(
                ui,
                field,
                egui::DragValue::new(&mut pc)
                    .range(0.0..=100.0)
                    .speed(1.0)
                    .max_decimals(0),
            )
            .on_hover_text(format!("{label} in percentuale"));
            if resp.changed() {
                *v = (pc / 100.0).clamp(0.0, 1.0);
            }
        }
    });
}

/// Cosa chiede il pannello al modulo.
pub enum Request {
    Restyle(Vec<PathId>, Restyle),
    /// Crea l'abbondanza; i tracciati sono la fustella, da tenere fuori dalla grafica ribaltata.
    Bleed(BleedSpec, Vec<PathId>),
}

/// Distanza dal punto `p` al segmento `a`–`b`.
fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}

fn near_line(path: &PathInfo, at: [f64; 2], tol: f64) -> bool {
    path.parts.iter().any(|part| {
        let pts = &part.points;
        let closing = match (part.closed, pts.first(), pts.last()) {
            (true, Some(f), Some(l)) => Some((*l, *f)),
            _ => None,
        };
        pts.windows(2)
            .map(|w| (w[0], w[1]))
            .chain(closing)
            .any(|(a, b)| segment_distance(at, a, b) <= tol)
    })
}

/// Il punto è dentro il tracciato (regola pari-dispari, i pezzi aperti si chiudono da soli).
fn inside(path: &PathInfo, at: [f64; 2]) -> bool {
    let mut odd = false;
    for part in &path.parts {
        let p = &part.points;
        let n = p.len();
        if n < 3 {
            continue;
        }
        let mut j = n - 1;
        for i in 0..n {
            let (a, b) = (p[i], p[j]);
            if (a[1] > at[1]) != (b[1] > at[1])
                && at[0] < (b[0] - a[0]) * (at[1] - a[1]) / (b[1] - a[1]) + a[0]
            {
                odd = !odd;
            }
            j = i;
        }
    }
    odd
}

fn within(b: [f64; 4], at: [f64; 2], pad: f64) -> bool {
    at[0] >= b[0] - pad && at[0] <= b[2] + pad && at[1] >= b[1] - pad && at[1] <= b[3] + pad
}

/// Il tracciato sotto il punto: prima le linee (quella disegnata più sopra), poi i riempimenti.
pub fn hit(scan: &PageScan, at: [f64; 2], tol: f64) -> Option<usize> {
    let lines = scan.paths.iter().enumerate().rev().find(|(_, p)| {
        let Some(l) = &p.line else { return false };
        let pad = tol.max(l.width / 2.0);
        within(p.bounds, at, pad) && near_line(p, at, pad)
    });
    if let Some((i, _)) = lines {
        return Some(i);
    }
    scan.paths
        .iter()
        .enumerate()
        .rev()
        .find(|(_, p)| p.fill.is_some() && within(p.bounds, at, 0.0) && inside(p, at))
        .map(|(i, _)| i)
}

/// I tracciati come quello scelto: stesso colore e spessore del contorno, stesso riempimento.
pub fn similar(scan: &PageScan, model: &PathInfo) -> Vec<PathId> {
    let same_line = |p: &PathInfo| match (&p.line, &model.line) {
        (Some(a), Some(b)) => a.ink.same(&b.ink) && (a.width - b.width).abs() < 0.01,
        (None, None) => true,
        _ => false,
    };
    let same_fill = |p: &PathInfo| match (&p.fill, &model.fill) {
        (Some(a), Some(b)) => a.ink.same(&b.ink),
        (None, None) => true,
        _ => false,
    };
    scan.paths
        .iter()
        .filter(|p| same_line(p) && same_fill(p))
        .map(|p| p.id)
        .collect()
}

/// Un gruppo di filetti dello stesso colore e spessore.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub ink: Ink,
    pub width: f64,
    pub ids: Vec<PathId>,
    pub length: f64,
    pub bounds: [f64; 4],
}

fn union(a: Option<[f64; 4]>, b: [f64; 4]) -> [f64; 4] {
    match a {
        Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
        None => b,
    }
}

/// Il controllo della pagina.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Check {
    /// Le tinte di taglio e cordonatura usate dai contorni: nome, ruolo, gruppo.
    pub dies: Vec<(String, Role, Group)>,
    /// Filetti sottili in quadricromia, chiusi o lunghi: forse una fustella fatta male.
    pub suspects: Vec<Group>,
    /// Il formato finito: dal TrimBox o dalla fustella.
    pub finished: Option<([f64; 4], &'static str)>,
    /// Abbondanza sopra, destra, sotto, sinistra in mm (None senza formato finito o senza grafica).
    pub bleed: Option<[f64; 4]>,
}

/// Controlla la pagina: fustelle in tinta, filetti sospetti, abbondanza.
pub fn check(scan: &PageScan, inks: &[InkName]) -> Check {
    let mut dies: Vec<(String, Role, Group)> = Vec::new();
    let mut suspects: Vec<Group> = Vec::new();
    let mut content: Option<[f64; 4]> = None;
    let page = [0.0, 0.0, scan.width, scan.height];
    let clip = |b: [f64; 4]| {
        let r = [
            b[0].max(page[0]),
            b[1].max(page[1]),
            b[2].min(page[2]),
            b[3].min(page[3]),
        ];
        (r[0] < r[2] && r[1] < r[3]).then_some(r)
    };
    for p in &scan.paths {
        let die = p.line.as_ref().and_then(|l| match &l.ink {
            Ink::Spot { name, .. } => {
                let role = role_of(inks, name);
                matches!(role, Role::Cut | Role::Crease).then(|| (name.clone(), role, l))
            }
            _ => None,
        });
        if let Some((name, role, l)) = die {
            let g = match dies.iter_mut().find(|(n, _, _)| *n == name) {
                Some((_, _, g)) => g,
                None => {
                    dies.push((
                        name,
                        role,
                        Group {
                            ink: l.ink.clone(),
                            width: l.width,
                            ids: Vec::new(),
                            length: 0.0,
                            bounds: p.bounds,
                        },
                    ));
                    match dies.last_mut() {
                        Some((_, _, g)) => g,
                        None => continue,
                    }
                }
            };
            g.ids.push(p.id);
            g.length += p.length;
            g.bounds = union(Some(g.bounds), p.bounds);
            continue;
        }
        if let Some(b) = clip(p.bounds) {
            content = Some(union(content, b));
        }
        if let (Some(l), None) = (&p.line, &p.fill)
            && l.ink.is_process()
            && l.width <= HAIRLINE_MAX
        {
            match suspects
                .iter_mut()
                .find(|g| g.ink.same(&l.ink) && (g.width - l.width).abs() < 0.01)
            {
                Some(g) => {
                    g.ids.push(p.id);
                    g.length += p.length;
                    g.bounds = union(Some(g.bounds), p.bounds);
                }
                None => suspects.push(Group {
                    ink: l.ink.clone(),
                    width: l.width,
                    ids: vec![p.id],
                    length: p.length,
                    bounds: p.bounds,
                }),
            }
        }
    }
    for o in &scan.others {
        if let Some(b) = clip(*o) {
            content = Some(union(content, b));
        }
    }
    // Un filetto sospetto è chiuso o gira attorno a un bel pezzo di pagina.
    let area = |b: [f64; 4]| (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0);
    let page_area = area(page).max(1.0);
    suspects.retain(|g| {
        let closed = g
            .ids
            .iter()
            .any(|id| scan.paths.iter().any(|p| p.id == *id && p.closed()));
        closed || area(g.bounds) > page_area * 0.2
    });
    suspects.sort_by(|a, b| area(b.bounds).total_cmp(&area(a.bounds)));
    suspects.truncate(3);
    let die_bounds = dies
        .iter()
        .filter(|(_, r, _)| *r == Role::Cut)
        .fold(None, |acc, (_, _, g)| Some(union(acc, g.bounds)));
    let finished = match (scan.trim, die_bounds, suspects.first()) {
        (Some(t), _, _) => Some((t, "TrimBox")),
        (None, Some(d), _) => Some((d, "fustella")),
        (None, None, Some(g)) => Some((g.bounds, "filetto")),
        // Senza TrimBox né fustella il formato finito è la pagina: il caso più comune dei file
        // esportati senza abbondanza.
        (None, None, None) => Some((page, "pagina")),
    };
    let bleed = match (finished, content) {
        (Some((f, _)), Some(c)) => Some([
            (f[1] - c[1]).max(0.0),
            (c[2] - f[2]).max(0.0),
            (c[3] - f[3]).max(0.0),
            (f[0] - c[0]).max(0.0),
        ]),
        _ => None,
    };
    Check {
        dies,
        suspects,
        finished,
        bleed,
    }
}

/// "0,25 pt (0,09 mm)" o "il più sottile".
fn width_text(mm: f64) -> String {
    if mm <= 0.0001 {
        return "il più sottile possibile".into();
    }
    format!(
        "{} pt · {} mm",
        format_number(winfiles_geom::mm_to_pt(mm), 2, true),
        format_number(mm, 2, true)
    )
}

/// Lunghezza in metri sopra il metro, in millimetri sotto.
fn length_text(mm: f64, unit: Unit) -> String {
    if mm >= 1000.0 {
        format!("{} m", format_number(mm / 1000.0, 2, false))
    } else {
        format_length(mm, unit)
    }
}

/// Il colore di un inchiostro per il quadratino del pannello.
fn swatch(ink: &Ink, spots: &[winfiles_pdf_edit::Spot]) -> Option<egui::Color32> {
    let rgb = match ink {
        Ink::Spot { name, .. } => spots.iter().find(|s| s.name == *name).and_then(|s| s.swatch),
        other => other.screen(),
    }?;
    Some(egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
}

/// Un colore di quadricromia per la tinta nuova, dal colore di partenza.
fn as_cmyk(ink: &Ink) -> Option<[f64; 4]> {
    match ink {
        Ink::Cmyk(v) => Some(*v),
        Ink::Gray(g) => Some([0.0, 0.0, 0.0, 1.0 - g]),
        Ink::Rgb([r, g, b]) => {
            let k = 1.0 - r.max(*g).max(*b);
            if k >= 1.0 {
                return Some([0.0, 0.0, 0.0, 1.0]);
            }
            let f = |v: f64| (1.0 - v - k) / (1.0 - k);
            Some([f(*r), f(*g), f(*b), k])
        }
        _ => None,
    }
}

impl PathTool {
    /// La pagina letta, se pronta per `key` (versione del documento, pagina).
    pub fn scan(&self) -> Option<&Arc<PageScan>> {
        self.scanned.as_ref().and_then(|s| s.result.as_ref().ok())
    }

    /// I tracciati da scegliere quando arriva la pagina riletta dopo un cambio.
    pub fn select_after_scan(&mut self, ids: Vec<PathId>) {
        self.after_scan = Some(ids);
    }

    /// Chiede la lettura della pagina se serve e raccoglie quella finita.
    pub fn update(&mut self, ctx: &egui::Context, key: (u64, usize), doc: impl FnOnce() -> Snapshot) {
        if let Some((k, rx)) = &self.pending
            && let Ok(result) = rx.try_recv()
        {
            let k = *k;
            self.pending = None;
            let result = result.map(Arc::new);
            if let Some(ids) = self.after_scan.take()
                && k.1 == key.1
            {
                self.selected = ids.into_iter().collect();
            }
            if let Ok(scan) = &result {
                // Le scelte che non ci sono più (pagina cambiata, annullato) si lasciano.
                self.selected.retain(|id| scan.paths.iter().any(|p| p.id == *id));
            }
            self.scanned = Some(Scanned { key: k, result });
        }
        let fresh = self.scanned.as_ref().is_some_and(|s| s.key == key);
        let asked = self.pending.as_ref().is_some_and(|(k, _)| *k == key);
        if fresh || asked {
            return;
        }
        if self.scanned.as_ref().is_some_and(|s| s.key.1 != key.1) {
            self.selected.clear();
        }
        let (tx, rx) = channel();
        let snap = doc();
        let wake = ctx.clone();
        let page = key.1;
        std::thread::spawn(move || {
            let r = snap.scan_page(page).map_err(|e| e.to_string());
            let _ = tx.send(r);
            wake.request_repaint();
        });
        self.pending = Some((key, rx));
    }

    /// Clic e riquadro sulla pagina, e i tracciati scelti evidenziati sopra.
    pub fn interact(&mut self, ui: &mut egui::Ui, id: egui::Id, paper: Rect, w: f32) {
        let t = Tokens::get(ui.ctx());
        let resp = ui.interact(paper, id, Sense::click_and_drag());
        let per_mm = paper.width() / w.max(0.001);
        let to_mm = |p: Pos2| -> [f64; 2] {
            [
                f64::from((p.x - paper.left()) / per_mm),
                f64::from((p.y - paper.top()) / per_mm),
            ]
        };
        let to_screen = |p: [f64; 2]| -> Pos2 {
            egui::pos2(
                paper.left() + p[0] as f32 * per_mm,
                paper.top() + p[1] as f32 * per_mm,
            )
        };
        let tol = f64::from(HIT_TOLERANCE / per_mm);
        let shift = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        let scan = self.scan().cloned();
        if let Some(scan) = &scan {
            if resp.clicked()
                && let Some(p) = resp.interact_pointer_pos()
            {
                match hit(scan, to_mm(p), tol).and_then(|i| scan.paths.get(i)) {
                    Some(path) if shift => {
                        if !self.selected.remove(&path.id) {
                            self.selected.insert(path.id);
                        }
                    }
                    Some(path) => {
                        self.selected.clear();
                        self.selected.insert(path.id);
                    }
                    None if !shift => self.selected.clear(),
                    None => {}
                }
            }
            if resp.drag_started()
                && let Some(p) = resp.interact_pointer_pos()
            {
                self.marquee = Some(to_mm(p));
            }
            if let (Some(start), Some(now)) = (self.marquee, ui.input(|i| i.pointer.interact_pos())) {
                let end = to_mm(now);
                let r = [
                    start[0].min(end[0]),
                    start[1].min(end[1]),
                    start[0].max(end[0]),
                    start[1].max(end[1]),
                ];
                ui.painter().rect(
                    Rect::from_two_pos(to_screen([r[0], r[1]]), to_screen([r[2], r[3]])),
                    0.0,
                    t.selection.gamma_multiply(0.35),
                    Stroke::new(1.0, t.accent),
                    egui::StrokeKind::Inside,
                );
                if resp.drag_stopped() {
                    self.marquee = None;
                    if !shift {
                        self.selected.clear();
                    }
                    for p in &scan.paths {
                        let b = p.bounds;
                        if b[0] >= r[0] && b[1] >= r[1] && b[2] <= r[2] && b[3] <= r[3] {
                            self.selected.insert(p.id);
                        }
                    }
                }
            }
            // Il tracciato sotto il puntatore, leggero; quelli scelti, pieni.
            let hover = resp
                .hover_pos()
                .filter(|_| self.marquee.is_none())
                .and_then(|p| hit(scan, to_mm(p), tol));
            let draw = |path: &PathInfo, color: egui::Color32, width: f32| {
                for part in &path.parts {
                    let mut pts: Vec<Pos2> = part.points.iter().map(|p| to_screen(*p)).collect();
                    if part.closed
                        && let Some(f) = pts.first().copied()
                    {
                        pts.push(f);
                    }
                    if pts.len() == 1 {
                        ui.painter().circle_filled(pts[0], width, color);
                    } else {
                        ui.painter()
                            .add(egui::Shape::line(pts, Stroke::new(width, color)));
                    }
                }
            };
            if let Some(path) = hover.and_then(|i| scan.paths.get(i))
                && !self.selected.contains(&path.id)
            {
                draw(path, t.accent.gamma_multiply(0.6), 2.0);
            }
            for path in scan.paths.iter().filter(|p| self.selected.contains(&p.id)) {
                draw(path, egui::Color32::WHITE, 4.0);
                draw(path, t.accent, 2.0);
            }
            if hover.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        if !resp.dragged() && !resp.drag_stopped() {
            self.marquee = None;
        }
    }

    /// Il pannello a destra: controllo della pagina, tracciati scelti, cambi.
    pub fn panel(
        &mut self,
        ui: &mut egui::Ui,
        unit: Unit,
        inks: &mut Vec<InkName>,
        spots: &[winfiles_pdf_edit::Spot],
    ) -> Option<Request> {
        let t = Tokens::get(ui.ctx());
        let scan = match &self.scanned {
            Some(Scanned { result: Ok(s), .. }) => s.clone(),
            Some(Scanned { result: Err(e), .. }) => {
                ui.colored_label(t.error, format!("Pagina non leggibile: {e}"));
                return None;
            }
            None => {
                ui.label(egui::RichText::new("Lettura della pagina…").color(t.text_dim));
                return None;
            }
        };
        let mut request = None;
        // Mentre la pagina si rilegge dopo un cambio si mostra la lettura di prima, ferma, con la
        // stessa altezza: lo scorrimento non torna in cima.
        let busy = self.pending.is_some();
        egui::ScrollArea::vertical()
            .id_salt("pdf-tracciati")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if busy {
                    ui.set_min_height(self.content_height);
                    ui.disable();
                }
                let top = ui.cursor().top();
                let c = check(&scan, inks);
                self.check_card(ui, &scan, &c, unit, spots);
                ui.add_space(8.0);
                let bleed = self.bleed_card(ui, &c, unit);
                if c.finished.is_some() {
                    ui.add_space(8.0);
                }
                request = self.selection_card(ui, &scan, inks, unit, spots).or(bleed);
                if !busy {
                    self.content_height = ui.cursor().top() - top;
                }
            });
        if busy { None } else { request }
    }

    fn check_card(
        &mut self,
        ui: &mut egui::Ui,
        scan: &PageScan,
        c: &Check,
        unit: Unit,
        spots: &[winfiles_pdf_edit::Spot],
    ) {
        let t = Tokens::get(ui.ctx());
        card_frame(ui, |ui| {
            ui.label(
                egui::RichText::new("Controllo della pagina")
                    .font(semibold(13.0))
                    .color(t.text_strong),
            );
            ui.add_space(4.0);
            let status = |ui: &mut egui::Ui, ok: bool, text: String| {
                ui.horizontal_top(|ui| {
                    let (icon, color) = if ok {
                        ("circle-check", t.accent_text)
                    } else {
                        ("triangle-alert", t.warning)
                    };
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                    icons::paint(ui, icon, r, color);
                    ui.add(egui::Label::new(egui::RichText::new(text).color(t.text)).wrap());
                });
                ui.add_space(2.0);
            };
            if c.dies.is_empty() {
                status(ui, false, "Nessuna fustella in tinta piatta".into());
            }
            for (name, role, g) in &c.dies {
                {
                    status(
                        ui,
                        true,
                        format!(
                            "{} «{name}»: {} {}, {}",
                            role.label(),
                            g.ids.len(),
                            if g.ids.len() == 1 {
                                "tracciato"
                            } else {
                                "tracciati"
                            },
                            length_text(g.length, unit)
                        ),
                    );
                }
                if ui
                    .button("Seleziona")
                    .on_hover_text("Sceglie tutti i tracciati di questa tinta")
                    .clicked()
                {
                    self.selected = g.ids.iter().copied().collect();
                }
            }
            for g in &c.suspects {
                ui.add_space(4.0);
                ui.horizontal_top(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                    match swatch(&g.ink, spots) {
                        Some(col) => {
                            ui.painter().rect(
                                r.shrink(1.0),
                                3.0,
                                col,
                                Stroke::new(1.0, t.border),
                                egui::StrokeKind::Inside,
                            );
                        }
                        None => icons::paint(ui, "triangle-alert", r, t.warning),
                    }
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!(
                                "Filetto in quadricromia, forse una fustella: {}, {}, {} {}",
                                g.ink.label(),
                                width_text(g.width),
                                g.ids.len(),
                                if g.ids.len() == 1 {
                                    "tracciato"
                                } else {
                                    "tracciati"
                                }
                            ))
                            .color(t.text),
                        )
                        .wrap(),
                    );
                });
                if ui
                    .button("Seleziona questi filetti")
                    .on_hover_text("Poi sotto si fanno diventare una tinta, per esempio Taglio")
                    .clicked()
                {
                    self.selected = g.ids.iter().copied().collect();
                    self.part = Part::Line;
                }
            }
            ui.add_space(4.0);
            match (c.finished, c.bleed) {
                (Some((_, "pagina")), _) => status(
                    ui,
                    false,
                    "Formato finito non indicato (niente TrimBox né fustella): si prende la pagina intera, \
                     senza abbondanza"
                        .into(),
                ),
                (Some((_, from)), Some(b)) => {
                    let ok = b.iter().all(|v| *v >= MIN_BLEED);
                    let fmt = |v: f64| format_number(v / unit.mm(), unit.decimals(), true);
                    status(
                        ui,
                        ok,
                        format!(
                            "Abbondanza oltre il formato finito ({from}): sopra {}, destra {}, sotto {}, sinistra {} {}",
                            fmt(b[0]),
                            fmt(b[1]),
                            fmt(b[2]),
                            fmt(b[3]),
                            unit.symbol()
                        ),
                    );
                }
                (Some(_), None) => status(ui, false, "Nessuna grafica sulla pagina".into()),
                (None, _) => status(
                    ui,
                    false,
                    "Formato finito non indicato (niente TrimBox né fustella): l'abbondanza non si può misurare".into(),
                ),
            }
            if scan.truncated {
                ui.label(
                    egui::RichText::new("Pagina molto complessa: letti solo i primi tracciati.")
                        .size(12.0)
                        .color(t.warning),
                );
            }
        });
    }

    /// Crea l'abbondanza che manca attorno al formato finito.
    fn bleed_card(&mut self, ui: &mut egui::Ui, c: &Check, unit: Unit) -> Option<Request> {
        let t = Tokens::get(ui.ctx());
        let (finished, from) = c.finished?;
        // I lati si propongono a ogni lettura nuova: quelli con meno abbondanza di quella voluta.
        let key = self.scanned.as_ref().map(|s| s.key);
        let propose = |amount: f64| -> [bool; 4] {
            match c.bleed {
                Some(b) => b.map(|v| v + 0.05 < amount),
                None => [true; 4],
            }
        };
        if self.bleed_for != key {
            self.bleed_for = key;
            self.bleed_sides = propose(self.bleed_amount);
        }
        let mut request = None;
        card_frame(ui, |ui| {
            ui.label(
                egui::RichText::new("Crea l'abbondanza")
                    .font(semibold(13.0))
                    .color(t.text_strong),
            );
            ui.add_space(2.0);
            let size = winfiles_geom::format_size(
                winfiles_geom::Size::new(finished[2] - finished[0], finished[3] - finished[1]),
                unit,
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "Formato finito {size} ({})",
                        match from {
                            "fustella" => "dalla fustella",
                            "filetto" => "dal filetto",
                            "pagina" => "la pagina intera",
                            _ => "dal TrimBox",
                        }
                    ))
                    .color(t.text_dim),
                )
                .wrap(),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Abbondanza").color(t.text_dim));
                let resp = crate::fluent::number(
                    ui,
                    90.0,
                    egui::DragValue::new(&mut self.bleed_amount)
                        .range(0.5..=50.0)
                        .speed(0.1)
                        .custom_formatter(|v, _| format!("{} mm", format_number(v, 1, true)))
                        .custom_parser(|text| {
                            let clean = text.trim().trim_end_matches("mm").trim().replace(',', ".");
                            clean.parse::<f64>().ok()
                        }),
                );
                if resp.changed() {
                    self.bleed_sides = propose(self.bleed_amount);
                }
            });
            ui.add_space(4.0);
            let names = ["Sopra", "Destra", "Sotto", "Sinistra"];
            egui::Grid::new("abbondanza-lati")
                .num_columns(2)
                .spacing(vec2(16.0, 4.0))
                .show(ui, |ui| {
                    for (i, name) in names.iter().enumerate() {
                        if let Some(side) = self.bleed_sides.get_mut(i) {
                            let have = c
                                .bleed
                                .and_then(|b| b.get(i).copied())
                                .map(|v| format!("c'è {} mm", format_number(v, 1, true)))
                                .unwrap_or_default();
                            ui.add(crate::fluent::checkbox(side, *name)).on_hover_text(have);
                        }
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
            ui.add_space(6.0);
            crate::app::segmented(
                ui,
                "abbondanza-modo",
                &mut self.bleed_mode,
                &[
                    (BleedMode::Mirror, "A specchio"),
                    (BleedMode::Stretch, "Allunga il bordo"),
                ],
            );
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(match self.bleed_mode {
                        BleedMode::Mirror => {
                            "La grafica vicino al bordo si ribalta fuori dal formato finito: va bene per foto e sfondi."
                        }
                        BleedMode::Stretch => {
                            "L'ultimo mezzo millimetro di grafica si allunga fino al bordo: va bene per tinte unite e sfumature."
                        }
                    })
                    .size(12.0)
                    .color(t.text_dim),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            let any = self.bleed_sides.iter().any(|s| *s);
            let go = egui::Button::new(
                egui::RichText::new("Crea abbondanza")
                    .font(semibold(13.5))
                    .color(if any { t.on_primary } else { t.text_disabled }),
            )
            .fill(if any { t.primary } else { t.button })
            .min_size(vec2(ui.available_width(), 32.0));
            if ui
                .add_enabled(any, go)
                .on_hover_text(
                    "La pagina diventa formato finito più l'abbondanza sui lati scelti; la fustella \
                     non si ribalta. Formato finito e abbondanza si scrivono nel PDF (TrimBox e BleedBox).",
                )
                .on_disabled_hover_text("Scegli almeno un lato")
                .clicked()
            {
                // La fustella (e il filetto da cui si è preso il formato finito) resta fuori.
                let mut die: Vec<PathId> = c
                    .dies
                    .iter()
                    .flat_map(|(_, _, g)| g.ids.iter().copied())
                    .collect();
                if from == "filetto"
                    && let Some(g) = c.suspects.first()
                {
                    die.extend(g.ids.iter().copied());
                }
                request = Some(Request::Bleed(
                    BleedSpec {
                        finished,
                        amount: self.bleed_amount,
                        sides: self.bleed_sides,
                        mode: self.bleed_mode,
                    },
                    die,
                ));
            }
        });
        request
    }

    fn selection_card(
        &mut self,
        ui: &mut egui::Ui,
        scan: &PageScan,
        inks: &mut Vec<InkName>,
        unit: Unit,
        spots: &[winfiles_pdf_edit::Spot],
    ) -> Option<Request> {
        let t = Tokens::get(ui.ctx());
        let chosen: Vec<&PathInfo> = scan
            .paths
            .iter()
            .filter(|p| self.selected.contains(&p.id))
            .collect();
        let mut request = None;
        card_frame(ui, |ui| {
            ui.horizontal(|ui| {
                let n = chosen.len();
                ui.label(
                    egui::RichText::new(match n {
                        0 => "Nessun tracciato scelto".to_string(),
                        1 => "1 tracciato scelto".to_string(),
                        n => format!("{n} tracciati scelti"),
                    })
                    .font(semibold(13.0))
                    .color(t.text_strong),
                );
                if n > 0 {
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if icons::button(ui, "x", "Togli la scelta (Esc)", false, true, 22.0).clicked() {
                            self.selected.clear();
                        }
                    });
                }
            });
            if chosen.is_empty() {
                ui.label(
                    egui::RichText::new(
                        "Clicca una linea sulla pagina. Maiusc+clic aggiunge o toglie; \
                         trascinando si prendono i tracciati dentro il riquadro.",
                    )
                    .color(t.text_dim),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "Sulla pagina: {} tracciati.",
                        scan.paths
                            .iter()
                            .filter(|p| p.line.is_some() || p.fill.is_some())
                            .count()
                    ))
                    .size(12.0)
                    .color(t.text_dim),
                );
                return;
            }
            let first = chosen[0];
            ui.horizontal(|ui| {
                if ui
                    .button("Seleziona simili")
                    .on_hover_text(
                        "Tutti i tracciati della pagina con lo stesso colore e spessore del primo scelto",
                    )
                    .clicked()
                {
                    self.selected = similar(scan, first).into_iter().collect();
                }
            });
            ui.add_space(4.0);
            // Le caratteristiche: quelle comuni, o "diversi".
            let lines: Vec<_> = chosen.iter().filter_map(|p| p.line.as_ref()).collect();
            let fills: Vec<_> = chosen.iter().filter_map(|p| p.fill.as_ref()).collect();
            let row = |ui: &mut egui::Ui, label: &str, color: Option<egui::Color32>, value: String| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().interact_size.y = 0.0;
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.allocate_ui_with_layout(
                        vec2(88.0, 16.0),
                        egui::Layout::left_to_right(Align::Min),
                        |ui| {
                            ui.set_min_width(88.0);
                            ui.label(egui::RichText::new(label).color(t.text_dim));
                        },
                    );
                    if let Some(c) = color {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                        ui.painter().rect(
                            r.translate(vec2(0.0, 1.0)),
                            3.0,
                            c,
                            Stroke::new(1.0, t.border),
                            egui::StrokeKind::Inside,
                        );
                    }
                    ui.add(egui::Label::new(egui::RichText::new(value).color(t.text_strong)).wrap());
                });
                ui.add_space(2.0);
            };
            {
                {
                    match lines.first() {
                        None => row(ui, "Contorno", None, "nessuno".into()),
                        Some(l) => {
                            let same_ink = lines.iter().all(|x| x.ink.same(&l.ink));
                            let same_w = lines.iter().all(|x| (x.width - l.width).abs() < 0.005);
                            let all_op = lines.iter().all(|x| x.overprint);
                            let none_op = lines.iter().all(|x| !x.overprint);
                            let extra = if lines.len() < chosen.len() {
                                " (non tutti)"
                            } else {
                                ""
                            };
                            row(
                                ui,
                                "Contorno",
                                same_ink.then(|| swatch(&l.ink, spots)).flatten(),
                                if same_ink {
                                    format!("{}{extra}", l.ink.label())
                                } else {
                                    "colori diversi".into()
                                },
                            );
                            row(
                                ui,
                                "Spessore",
                                None,
                                if same_w {
                                    width_text(l.width)
                                } else {
                                    "diversi".into()
                                },
                            );
                            row(
                                ui,
                                "Sovrastampa",
                                None,
                                match (all_op, none_op) {
                                    (true, _) => "sì".into(),
                                    (_, true) => "no".into(),
                                    _ => "solo in parte".into(),
                                },
                            );
                        }
                    }
                    match fills.first() {
                        None => row(ui, "Riempimento", None, "nessuno".into()),
                        Some(f) => {
                            let same = fills.iter().all(|x| x.ink.same(&f.ink));
                            row(
                                ui,
                                "Riempimento",
                                same.then(|| swatch(&f.ink, spots)).flatten(),
                                if same {
                                    f.ink.label()
                                } else {
                                    "colori diversi".into()
                                },
                            );
                        }
                    }
                    let closed = chosen.iter().filter(|p| p.closed()).count();
                    row(
                        ui,
                        "Forma",
                        None,
                        match closed {
                            0 => "aperta".into(),
                            c if c == chosen.len() => "chiusa".into(),
                            c => format!("{c} chiusi, {} aperti", chosen.len() - c),
                        },
                    );
                    let b = chosen
                        .iter()
                        .fold(None, |acc, p| Some(union(acc, p.bounds)))
                        .unwrap_or_default();
                    row(
                        ui,
                        "Ingombro",
                        None,
                        winfiles_geom::format_size(winfiles_geom::Size::new(b[2] - b[0], b[3] - b[1]), unit),
                    );
                    let length: f64 = chosen.iter().map(|p| p.length).sum();
                    row(ui, "Lunghezza", None, length_text(length, unit));
                }
            }
            let forms = chosen.iter().filter(|p| p.in_form).count();
            if forms > 0 {
                ui.label(
                    egui::RichText::new(
                        "Alcuni stanno in un gruppo ripetuto del PDF: cambiandoli cambiano tutte le sue copie.",
                    )
                    .size(12.0)
                    .color(t.text_dim),
                );
            }
            let clips = chosen.iter().filter(|p| p.clips).count();
            if clips > 0 {
                ui.label(
                    egui::RichText::new(format!(
                        "{clips} sono anche tracciati di ritaglio e non si possono cambiare."
                    ))
                    .size(12.0)
                    .color(t.warning),
                );
            }
        });
        if chosen.is_empty() {
            return None;
        }
        ui.add_space(8.0);
        let has_line = chosen.iter().any(|p| p.line.is_some());
        let has_fill = chosen.iter().any(|p| p.fill.is_some());
        if !has_line && self.part == Part::Line {
            self.part = Part::Fill;
        }
        if !has_fill && self.part == Part::Fill {
            self.part = Part::Line;
        }
        let ids: Vec<PathId> = chosen.iter().map(|p| p.id).collect();
        let current_ink = |part: Part| {
            chosen.iter().find_map(|p| match part {
                Part::Line => p.line.as_ref().map(|l| l.ink.clone()),
                Part::Fill => p.fill.as_ref().map(|f| f.ink.clone()),
            })
        };
        let current_overprint = |part: Part| {
            let v: Vec<bool> = chosen
                .iter()
                .filter_map(|p| match part {
                    Part::Line => p.line.as_ref().map(|l| l.overprint),
                    Part::Fill => p.fill.as_ref().map(|f| f.overprint),
                })
                .collect();
            !v.is_empty() && v.iter().all(|x| *x)
        };
        // Taglio, cordonatura, bianco e vernice sui contorni vanno in sovrastampa, come vogliono i RIP.
        let die_role = |r: Role| matches!(r, Role::Cut | Role::Crease | Role::Varnish | Role::White);
        // I valori si propongono da quelli del primo tracciato scelto: colore, spessore, sovrastampa.
        if self.prefilled != Some((ids[0], self.part)) {
            self.prefilled = Some((ids[0], self.part));
            if let Some(ink) = current_ink(self.part) {
                let from_swatch = swatch(&ink, spots).map(|c| {
                    let f = |v: u8| f64::from(v) / 255.0;
                    Ink::Rgb([f(c.r()), f(c.g()), f(c.b())])
                });
                if let Some(c) = as_cmyk(&ink).or_else(|| from_swatch.as_ref().and_then(as_cmyk)) {
                    self.cmyk = c.map(|x| (x * 100.0).round() / 100.0);
                }
            }
            if let Some(l) = chosen.iter().find_map(|p| p.line.as_ref()) {
                self.width = (l.width * 100.0).round() / 100.0;
            }
            self.width_touched = false;
            self.overprint = current_overprint(self.part);
            self.overprint_touched = false;
            if self.mode == InkMode::Spot && self.part == Part::Line && die_role(self.role) {
                self.overprint = true;
                self.overprint_touched = true;
            }
        }
        card_frame(ui, |ui| {
            ui.label(
                egui::RichText::new("Cambia")
                    .font(semibold(13.0))
                    .color(t.text_strong),
            );
            ui.add_space(4.0);
            if has_line && has_fill {
                crate::app::segmented(
                    ui,
                    "tracciati-parte",
                    &mut self.part,
                    &[(Part::Line, "Contorno"), (Part::Fill, "Riempimento")],
                );
                ui.add_space(6.0);
            }
            ui.label(egui::RichText::new("Colore").color(t.text_dim));
            let mode_before = self.mode;
            crate::app::segmented(
                ui,
                "tracciati-modo",
                &mut self.mode,
                &[
                    (InkMode::Keep, "Com'è"),
                    (InkMode::Spot, "Tinta piatta"),
                    (InkMode::Process, "CMYK"),
                ],
            );
            if self.mode != mode_before
                && self.mode == InkMode::Spot
                && self.part == Part::Line
                && die_role(self.role)
            {
                self.overprint = true;
                self.overprint_touched = true;
            }
            ui.add_space(6.0);
            if self.mode == InkMode::Spot {
                ui.label(egui::RichText::new("Nome della tinta").color(t.text_dim));
                let mut picked_role = None;
                ui.horizontal(|ui| {
                    let edit = egui::TextEdit::singleline(&mut self.ink_name)
                        .hint_text("Nome della tinta")
                        .desired_width((ui.available_width() - 56.0).max(80.0));
                    let resp = crate::fluent::text_field(ui, edit);
                    if resp.changed() {
                        let name = self.ink_name.trim();
                        picked_role = Some(role_of(inks, name));
                        if let Some(i) = inks.iter().find(|i| i.name == name) {
                            self.cmyk = i.color();
                        }
                    }
                    egui::ComboBox::from_id_salt("tracciati-nomi")
                        .icon(crate::fluent::combo_icon)
                        .selected_text("")
                        .width(28.0)
                        .show_ui(ui, |ui| {
                            for i in inks.iter() {
                                let text = format!("{}  ·  {}", i.name, i.role.label());
                                if ui.selectable_label(self.ink_name == i.name, text).clicked() {
                                    self.ink_name = i.name.clone();
                                    picked_role = Some(i.role);
                                    self.cmyk = i.color();
                                }
                            }
                        })
                        .response
                        .on_hover_text("I nomi usati di recente");
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Ruolo").color(t.text_dim));
                    egui::ComboBox::from_id_salt("tracciati-ruolo")
                        .icon(crate::fluent::combo_icon)
                        .selected_text(self.role.label())
                        .width(140.0)
                        .show_ui(ui, |ui| {
                            for r in Role::ALL {
                                if ui.selectable_label(self.role == r, r.label()).clicked() {
                                    picked_role = Some(r);
                                }
                            }
                        });
                });
                if let Some(r) = picked_role {
                    self.role = r;
                    if self.part == Part::Line && die_role(r) {
                        self.overprint = true;
                        self.overprint_touched = true;
                    }
                }
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Colore a schermo e nelle prove").color(t.text_dim))
                    .on_hover_text("Il RIP stampa la tinta per nome; questo è il colore con cui si vede");
                colour_wheel(ui, &mut self.cmyk, &mut self.hue, &mut self.sat);
            } else if self.mode == InkMode::Process {
                colour_wheel(ui, &mut self.cmyk, &mut self.hue, &mut self.sat);
            }
            if self.part == Part::Line {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Spessore").color(t.text_dim));
                    let resp = crate::fluent::number(
                        ui,
                        90.0,
                        egui::DragValue::new(&mut self.width)
                            .range(0.0..=20.0)
                            .speed(0.01)
                            .custom_formatter(|v, _| format!("{} mm", format_number(v, 2, true)))
                            .custom_parser(|text| {
                                let clean = text.trim().trim_end_matches("mm").trim().replace(',', ".");
                                clean.parse::<f64>().ok()
                            }),
                    );
                    if resp.changed() {
                        self.width_touched = true;
                    }
                });
            }
            ui.add_space(6.0);
            if ui
                .add(crate::fluent::checkbox(&mut self.overprint, "Sovrastampa"))
                .on_hover_text(
                    "Si stampa sopra gli altri colori senza togliere quello che c'è sotto: \
                     giusto per taglio e cordonatura, non per la grafica",
                )
                .changed()
            {
                self.overprint_touched = true;
            }
            let name = self.ink_name.trim().to_string();
            let ink = match self.mode {
                InkMode::Keep => None,
                InkMode::Spot => Some(NewInk::Spot {
                    name: name.clone(),
                    cmyk: self.cmyk,
                }),
                InkMode::Process => Some(NewInk::Cmyk(self.cmyk)),
            };
            let width = (self.part == Part::Line && self.width_touched).then_some(self.width);
            let overprint = (self.overprint_touched || self.overprint != current_overprint(self.part))
                .then_some(self.overprint);
            let something = ink.is_some() || width.is_some() || overprint.is_some();
            let valid = !(self.mode == InkMode::Spot && name.is_empty());
            ui.add_space(8.0);
            let n = ids.len();
            let go = egui::Button::new(
                egui::RichText::new(if n == 1 {
                    "Applica".to_string()
                } else {
                    format!("Applica ai {n} tracciati")
                })
                .font(semibold(13.5))
                .color(if something && valid {
                    t.on_primary
                } else {
                    t.text_disabled
                }),
            )
            .fill(if something && valid { t.primary } else { t.button })
            .min_size(vec2(ui.available_width(), 32.0));
            let target = if self.part == Part::Line {
                "il contorno"
            } else {
                "il riempimento"
            };
            let tip = match self.mode {
                InkMode::Keep => format!(
                    "Cambia spessore e sovrastampa del{} scelto",
                    if n == 1 { " tracciato" } else { "i tracciati" }
                ),
                InkMode::Spot => format!(
                    "Colora {target} con la tinta «{name}» al 100%, con spessore e sovrastampa scelti qui"
                ),
                InkMode::Process => {
                    format!("Colora {target} in quadricromia, con spessore e sovrastampa scelti qui")
                }
            };
            if ui
                .add_enabled(something && valid, go)
                .on_hover_text(tip)
                .on_disabled_hover_text(if valid {
                    "Niente da cambiare: scegli un colore, uno spessore o la sovrastampa"
                } else {
                    "Scrivi il nome della tinta"
                })
                .clicked()
            {
                if self.mode == InkMode::Spot {
                    remember(inks, &name, self.role, self.cmyk);
                }
                request = Some(Request::Restyle(
                    ids.clone(),
                    Restyle {
                        part: self.part,
                        ink,
                        width,
                        overprint,
                    },
                ));
                // Dopo il cambio i valori si ripropongono dai tracciati cambiati.
                self.prefilled = None;
            }
        });
        request
    }
}

/// Ricorda il nome (in cima all'elenco) con il suo ruolo.
pub fn remember(inks: &mut Vec<InkName>, name: &str, role: Role, cmyk: [f64; 4]) {
    inks.retain(|i| i.name != name);
    inks.insert(
        0,
        InkName {
            name: name.to_string(),
            role,
            cmyk: Some(cmyk),
        },
    );
    inks.truncate(MAX_INKS);
}

fn card_frame(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE
        .fill(t.panel_darker)
        .corner_radius(t.radius)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn il_ruolo_si_indovina_dal_nome() {
        assert_eq!(Role::guess("CutContour"), Role::Cut);
        assert_eq!(Role::guess("Fustella"), Role::Cut);
        assert_eq!(Role::guess("Thru-cut"), Role::Cut);
        assert_eq!(Role::guess("Cordonatura"), Role::Crease);
        assert_eq!(Role::guess("Mezzo taglio"), Role::Crease);
        assert_eq!(Role::guess("White"), Role::White);
        assert_eq!(Role::guess("Vernice UV"), Role::Varnish);
        assert_eq!(Role::guess("PANTONE 185 C"), Role::Other);
    }

    #[test]
    fn i_nomi_ricordati_vincono_sulle_parole() {
        let mut inks = default_inks();
        remember(&mut inks, "Linea 2", Role::Crease, [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(role_of(&inks, "Linea 2"), Role::Crease);
        assert_eq!(inks[0].name, "Linea 2");
        remember(&mut inks, "Taglio", Role::Cut, [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(inks[0].name, "Taglio");
        assert_eq!(inks.iter().filter(|i| i.name == "Taglio").count(), 1);
        assert_eq!(inks[1].color(), [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn nomi_vuoti_o_doppi_si_tolgono() {
        let ink = |n: &str, r: Role, c: Option<[f64; 4]>| InkName {
            name: n.into(),
            role: r,
            cmyk: c,
        };
        let inks = sanitized(vec![
            ink(" Taglio ", Role::Cut, Some([0.0, 2.0, f64::NAN, 0.0])),
            ink("Taglio", Role::Other, None),
            ink("", Role::Other, None),
            ink("Spot1", Role::White, Some([0.0, 1.5, 0.0, 0.0])),
        ]);
        assert_eq!(
            inks,
            [
                ink("Taglio", Role::Cut, None),
                ink("Spot1", Role::White, Some([0.0, 1.0, 0.0, 0.0]))
            ]
        );
        // Chi aveva solo i nomi della prima prova riceve quelli della ditta.
        let old = sanitized(vec![
            ink("CutContour", Role::Cut, None),
            ink("Bianco", Role::White, None),
        ]);
        assert_eq!(old, default_inks());
        assert_eq!(old[0].name, "Taglio");
    }

    #[test]
    fn distanza_dal_segmento() {
        assert!((segment_distance([5.0, 3.0], [0.0, 0.0], [10.0, 0.0]) - 3.0).abs() < 1e-9);
        assert!((segment_distance([13.0, 4.0], [0.0, 0.0], [10.0, 0.0]) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn la_ruota_torna_al_colore_di_partenza() {
        for c in [
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 0.4],
        ] {
            let back = as_cmyk(&Ink::Rgb(hsv_to_rgb(rgb_to_hsv(cmyk_rgb(c))))).unwrap();
            for (a, b) in back.iter().zip(c.iter()) {
                assert!((a - b).abs() < 1e-9, "{c:?} -> {back:?}");
            }
        }
        // Il giallo sta a un sesto del giro.
        let [h, s, v] = rgb_to_hsv(cmyk_rgb([0.0, 0.0, 1.0, 0.0]));
        assert!((h - 1.0 / 6.0).abs() < 1e-9 && (s - 1.0).abs() < 1e-9 && (v - 1.0).abs() < 1e-9);
    }

    #[test]
    fn da_rgb_a_cmyk() {
        assert_eq!(as_cmyk(&Ink::Rgb([1.0, 0.0, 1.0])), Some([0.0, 1.0, 0.0, 0.0]));
        assert_eq!(as_cmyk(&Ink::Gray(0.0)), Some([0.0, 0.0, 0.0, 1.0]));
    }
}
