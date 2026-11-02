//! L'anteprima: il foglio impaginato sul fondo grigio, come un documento in VectorCraft. Le misure
//! arrivano in millimetri dal motore di layout; qui si compone la scena del foglio (la stessa del
//! PDF) e la si disegna: nessun calcolo di impaginazione in questo file.
//! Il foglio è carta bianca e resta bianco anche nel tema scuro.

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::collections::HashMap;
use std::path::Path;
use winfiles_geom::Rect as MmRect;
use winfiles_layout::{Header, Sheet};
use winfiles_model::{FileKind, PageInfo, PrintSettings, Project, Totals};
use winfiles_scene::{Page, Picture, Source, Weight};

use crate::captions::{
    CAPTION_PAD, FOLDER_NOTE_INSET, INFO_RATIO, MM_PER_PT, NOTE_GAP, NOTE_INK, Slot, TITLE_PAD_X,
    TITLE_PAD_Y, area_text, info_lines, line_mm, name_job, note_job, title_job,
};
use crate::sketch::{self, PX_PER_MM, Sketch, grow};
use crate::theme::Tokens;
use crate::thumbs::Thumbnails;

/// Margine attorno al foglio quando l'anteprima lo adatta alla finestra, in punti schermo.
const FIT_MARGIN: f32 = 28.0;
pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 8.0;

/// Colori della carta stampata (non dell'interfaccia): testo quasi nero, grigi di servizio.
pub(crate) const INK: Color32 = Color32::from_rgb(20, 20, 20);
pub(crate) const INK_SOFT: Color32 = Color32::from_rgb(85, 85, 85);
/// Colori delle etichette delle cartelle, dal livello più alto al più basso, come le "briciole"
/// colorate della 4.9 ma scuri abbastanza da leggerci il bianco anche stampati.
const CRUMB_COLORS: [Color32; 5] = [
    Color32::from_rgb(0x2f, 0x6f, 0xd0),
    Color32::from_rgb(0x0e, 0x80, 0x6c),
    Color32::from_rgb(0xb8, 0x5c, 0x00),
    Color32::from_rgb(0x8e, 0x24, 0xaa),
    Color32::from_rgb(0x55, 0x2b, 0xc4),
];
const TITLE_FILL: Color32 = Color32::from_rgb(246, 247, 249);
pub(crate) const FRAME: Color32 = Color32::from_rgb(190, 190, 190);

/// Ingrandimento che fa stare il foglio intero nello spazio `avail` (punti per millimetro).
pub fn fit_zoom(size: winfiles_geom::Size, avail: Vec2) -> f32 {
    let (w, h) = (size.w as f32, size.h as f32);
    ((avail.x - 2.0 * FIT_MARGIN) / w)
        .min((avail.y - 2.0 * FIT_MARGIN) / h)
        .clamp(MIN_ZOOM, MAX_ZOOM)
}

/// I file scelti e la pagina in primo piano (quella del pannello Note).
#[derive(Clone, Copy)]
pub struct Selection<'a> {
    pub marked: &'a std::collections::BTreeSet<usize>,
    pub focus: Option<(usize, usize)>,
}

/// Cosa è successo nell'anteprima in questo fotogramma.
#[derive(Default)]
pub struct CanvasOutput {
    /// File e pagina della miniatura cliccata.
    pub clicked: Option<(usize, usize)>,
    /// Il tasto destro: file e pagina sotto il puntatore, o niente fuori dalle miniature.
    pub secondary: Option<Option<(usize, usize)>>,
    /// Il foglio, per aprirci sopra il menu del tasto destro.
    pub response: Option<egui::Response>,
}

/// Una riga di testo che si tronca con "…" se non sta nella larghezza (`width` in mm), centrata.
pub(crate) fn one_line(text: String, size: f64, weight: Weight, width: f64) -> LayoutJob {
    let mut job = LayoutJob::single_section(
        text,
        TextFormat {
            font_id: sketch::font(size, weight),
            color: INK,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping {
        max_width: (width as f32 * PX_PER_MM).max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    job.halign = egui::Align::Center;
    job
}

/// La parte della miniatura da mostrare: per i PDF con TrimBox solo il formato finito, senza
/// l'abbondanza, come il riquadro calcolato dal layout. In frazioni della pagina intera.
fn trim_crop(page: Option<&PageInfo>) -> MmRect {
    let full = MmRect::new(0.0, 0.0, 1.0, 1.0);
    let Some(page) = page else {
        return full;
    };
    let Some(trim) = page.trim else { return full };
    let fx = (trim.w / page.size.w).clamp(0.0, 1.0);
    let fy = (trim.h / page.size.h).clamp(0.0, 1.0);
    if !(fx > 0.0 && fy > 0.0) {
        return full;
    }
    MmRect::new((1.0 - fx) / 2.0, (1.0 - fy) / 2.0, fx, fy)
}

/// La scena di un foglio di miniature: intestazioni, miniature, didascalie e note, in mm.
/// `slots` dice a quale pagina di quale file corrisponde ogni elemento.
pub fn compose(ctx: &egui::Context, sheet: &Sheet, project: &Project, slots: &[Slot]) -> Page {
    let mut sk = Sketch::new(ctx, sheet.size);
    let print = &project.print;
    let em = |pt: f64| pt * MM_PER_PT;
    let (name_pt, info_pt) = (print.caption_text, print.caption_text * INFO_RATIO);
    // Per ogni intestazione: totali e nota della sua cartella.
    let labels = project.folder_labels();
    let totals = project.folder_totals();
    let folders: HashMap<&str, (&Path, Totals)> = labels
        .iter()
        .filter_map(|(path, label)| Some((label.as_str(), (*path, *totals.get(path)?))))
        .collect();
    if let Some(title) = sheet.title {
        draw_title(&mut sk, title, print);
    }
    for header in &sheet.headers {
        let folder = folders.get(header.folder.as_str());
        let r = header.rect;
        let note_h = header.note.map_or(0.0, |n| n.h);
        let title = MmRect::new(r.x, r.y, r.w, r.h - note_h);
        let custom = folder
            .and_then(|f| project.folder_titles.get(f.0))
            .map(|t| t.trim())
            .filter(|t| !t.is_empty());
        draw_header(
            &mut sk,
            title,
            header,
            custom,
            folder.map(|f| totals_text(&f.1, print)).as_deref(),
            folder.and_then(|f| folder_path(project, f.0)).as_deref(),
            em(print.header_text),
        );
        if let Some(note) = header.note
            && let Some(text) = folder
                .map(|f| project.printed_folder_note(f.0))
                .filter(|n| !n.trim().is_empty())
        {
            let width = note.w - 2.0 * FOLDER_NOTE_INSET;
            let galley = sk.layout(note_job(text, print.note_text, width, PX_PER_MM));
            sk.text(
                (note.x + FOLDER_NOTE_INSET, note.y + CAPTION_PAD),
                &galley,
                em(print.note_text),
                Weight::Regular,
                NOTE_INK,
                Some(note.bottom()),
            );
        }
    }

    let list = print.view == winfiles_model::View::List;
    // Nell'elenco: per ogni riga, dove inizia la riga successiva sullo stesso foglio (se c'è e
    // senza un'intestazione in mezzo), per il filo che le separa.
    let rows_below: HashMap<usize, f64> = if list {
        sheet
            .items
            .iter()
            .zip(sheet.items.iter().skip(1))
            .filter(|(a, b)| {
                !sheet
                    .headers
                    .iter()
                    .any(|h| h.rect.y >= a.cell.bottom() && h.rect.y < b.cell.y)
            })
            .map(|(a, b)| (a.id, b.cell.y))
            .collect()
    } else {
        HashMap::new()
    };
    for item in &sheet.items {
        let Some(slot) = slots.get(item.id) else {
            continue;
        };
        let Some(file) = project.files.get(slot.file) else {
            continue;
        };
        let page_info = file.pages.get(slot.page);
        let image = item.image;
        let unsupported = file.kind == FileKind::Unsupported;
        if unsupported {
            unsupported_tile(&mut sk, image, &file.path);
        } else {
            sk.picture(Picture {
                rect: image,
                source: Source {
                    path: file.path.clone(),
                    kind: file.kind,
                    page: slot.page,
                },
                crop: trim_crop(page_info),
                dpi: None,
            });
            // Un filo sottile attorno alla miniatura: le immagini chiare non si perdono nel foglio.
            sk.rect(grow(image, 0.12), 0.0, None, Some((FRAME, 0.12)));
        }

        // Le pagine di uno stesso file: cornice ed etichetta "Pag. 2/4" nel colore del file.
        if let Some(color) = slot.color {
            sk.rect(grow(image, 0.5), 0.0, None, Some((color, 0.5)));
            let size = em(name_pt * 0.9);
            let label = format!("Pag. {}/{}", slot.page + 1, slot.pages);
            let galley = sk.layout_line(&label, size, Weight::SemiBold);
            let (w, h) = Sketch::size_mm(&galley);
            let (px, py) = (size * 0.45, size * 0.2);
            let badge = MmRect::new(image.x + 1.0, image.y + 1.0, w + 2.0 * px, h + 2.0 * py);
            sk.rect(badge, size * 0.3, Some(color), None);
            sk.text(
                (badge.x + px, badge.y + py),
                &galley,
                size,
                Weight::SemiBold,
                Color32::WHITE,
                None,
            );
        }

        let caption = item.caption;
        // Nella griglia il testo è centrato sotto la miniatura, nell'elenco allineato a sinistra
        // accanto alla miniatura.
        let (tx, align) = if list {
            (caption.x, egui::Align::LEFT)
        } else {
            (caption.x + caption.w / 2.0, egui::Align::Center)
        };
        let bottom = Some(caption.bottom());
        let mut y = caption.y + CAPTION_PAD;
        let name = crate::captions::file_name(&file.path);
        let mut job = name_job(&name, name_pt, caption.w, PX_PER_MM);
        job.halign = align;
        let galley = sk.layout(job);
        sk.text((tx, y), &galley, em(name_pt), Weight::SemiBold, INK, bottom);
        y += galley.rows.len().max(1) as f64 * line_mm(name_pt);
        match page_info {
            Some(page) => {
                for line in info_lines(page, project.unit, print) {
                    let ink = if line.trim { NOTE_INK } else { INK_SOFT };
                    let mut job = one_line(line.text, em(info_pt), Weight::Regular, caption.w);
                    job.halign = align;
                    let galley = sk.layout(job);
                    sk.text((tx, y), &galley, em(info_pt), Weight::Regular, ink, bottom);
                    y += line_mm(info_pt);
                }
            }
            None if unsupported => {}
            None => y += line_mm(info_pt),
        }
        let note = project.printed_note(file, slot.page);
        if !note.trim().is_empty() {
            y += NOTE_GAP;
            let mut job = note_job(note, print.note_text, caption.w, PX_PER_MM);
            job.halign = align;
            let galley = sk.layout(job);
            sk.text(
                (tx, y),
                &galley,
                em(print.note_text),
                Weight::Regular,
                NOTE_INK,
                bottom,
            );
        }
        // Nell'elenco un filo sottile separa una riga dalla successiva, a metà dello spazio.
        if list && let Some(next) = rows_below.get(&item.id) {
            let ly = (item.cell.bottom() + next) / 2.0;
            sk.line((item.cell.x, ly), (item.cell.right(), ly), FRAME, 0.15);
        }
    }
    sk.page
}

/// Disegna `scene`, la scena di `sheet`, a `scale` punti per millimetro nello spazio disponibile,
/// con sopra la scelta dei file. `selected` sono i file scelti.
pub fn draw(
    ui: &mut egui::Ui,
    sheet: &Sheet,
    scene: &Page,
    slots: &[Slot],
    thumbs: &mut Thumbnails,
    selected: Selection<'_>,
    scale: f32,
) -> CanvasOutput {
    let t = Tokens::get(ui.ctx());
    let mut out = CanvasOutput::default();
    let (response, painter, origin) = paper(ui, sheet.size, scale);
    crate::paint::page(&painter, origin, scale, scene, thumbs);

    let pointer = response.interact_pointer_pos();
    if response.secondary_clicked() {
        out.secondary = Some(None);
    }
    for item in &sheet.items {
        let Some(slot) = slots.get(item.id) else {
            continue;
        };
        let cell = to_screen(origin, item.cell, scale);
        if selected.marked.contains(&slot.file) {
            // La pagina di cui si scrive la nota ha il bordo pieno, le altre scelte uno sottile.
            let width = if selected.focus == Some((slot.file, slot.page)) {
                2.5
            } else {
                1.0
            };
            painter.rect_stroke(
                cell.expand(3.0),
                3.0,
                Stroke::new(width, t.accent),
                StrokeKind::Outside,
            );
        }
        if response.clicked() && pointer.is_some_and(|p| cell.contains(p)) {
            out.clicked = Some((slot.file, slot.page));
        }
        if response.secondary_clicked() && pointer.is_some_and(|p| cell.contains(p)) {
            out.secondary = Some(Some((slot.file, slot.page)));
        }
    }
    out.response = Some(response);
    out
}

/// Il posto di un file non supportato: un riquadro grigio chiaro con l'estensione in grande
/// ("DOCX") e sotto "non supportato", se c'è spazio. Fa parte della scena: anteprima, PDF e
/// stampa lo disegnano uguale, senza cercare una miniatura che non esiste.
fn unsupported_tile(sk: &mut Sketch<'_>, r: MmRect, path: &Path) {
    sk.rect(
        r,
        (r.w.min(r.h) * 0.04).min(1.5),
        Some(TITLE_FILL),
        Some((INK_SOFT, 0.25)),
    );
    let ext = crate::unsupported::extension(path);
    // L'estensione larga al più tre quarti del riquadro e alta al più un quarto.
    let probe = 10.0;
    let (w, h) = Sketch::size_mm(&sk.layout_line(&ext, probe, Weight::SemiBold));
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let size = (probe * (r.w * 0.75 / w).min(r.h * 0.25 / h)).max(0.5);
    let big = sk.layout_line(&ext, size, Weight::SemiBold);
    let (bw, bh) = Sketch::size_mm(&big);
    let small_size = (size * 0.32).clamp(0.5, 3.0);
    let small = sk.layout_line("non supportato", small_size, Weight::Regular);
    let (sw, sh) = Sketch::size_mm(&small);
    let gap = bh * 0.1;
    let with_small = sw <= r.w * 0.9 && bh + gap + sh <= r.h * 0.9;
    let total = if with_small { bh + gap + sh } else { bh };
    let top = r.y + (r.h - total) / 2.0;
    let cx = r.x + r.w / 2.0;
    sk.text(
        (cx - bw / 2.0, top),
        &big,
        size,
        Weight::SemiBold,
        INK_SOFT,
        Some(r.bottom()),
    );
    if with_small {
        sk.text(
            (cx - sw / 2.0, top + bh + gap),
            &small,
            small_size,
            Weight::Regular,
            INK_SOFT,
            Some(r.bottom()),
        );
    }
}

/// Il titolo in cima al foglio, in un riquadro dagli angoli arrotondati con un fondo appena
/// grigio.
pub(crate) fn draw_title(sk: &mut Sketch<'_>, r: MmRect, print: &PrintSettings) {
    sk.rect(r, 1.8, Some(TITLE_FILL), Some((INK_SOFT, 0.35)));
    let inner = (r.w - 2.0 * TITLE_PAD_X) as f32;
    let galley = sk.layout(title_job(&print.title, print.title_text, inner, PX_PER_MM));
    sk.text(
        (r.x + TITLE_PAD_X, r.y + TITLE_PAD_Y),
        &galley,
        print.title_text * MM_PER_PT,
        Weight::SemiBold,
        INK,
        Some(r.bottom()),
    );
}

/// Altezza dello spazio che occupa un foglio alto `size.h` mm a `scale`, con i margini attorno,
/// in uno spazio visibile alto `view` punti.
pub fn content_height(size: winfiles_geom::Size, scale: f32, view: f32) -> f32 {
    view.max(size.h as f32 * scale + 2.0 * FIT_MARGIN)
}

/// Il foglio bianco con la sua ombra, centrato nello spazio disponibile. Restituisce la risposta
/// ai clic, il pennello e l'angolo in alto a sinistra del foglio sullo schermo.
pub(crate) fn paper(
    ui: &mut egui::Ui,
    size: winfiles_geom::Size,
    scale: f32,
) -> (egui::Response, egui::Painter, Pos2) {
    let page_size = Vec2::new(size.w as f32, size.h as f32) * scale;
    let avail = ui.available_size();
    let total = Vec2::new(
        avail.x.max(page_size.x + 2.0 * FIT_MARGIN),
        content_height(size, scale, avail.y),
    );
    let (response, painter) = ui.allocate_painter(total, Sense::click());
    let origin = response.rect.center() - page_size / 2.0;
    let page = Rect::from_min_size(origin, page_size);
    painter.rect_filled(
        page.translate(Vec2::new(0.0, 3.0)).expand(1.0),
        0.0,
        Color32::from_black_alpha(90),
    );
    painter.rect_filled(page, 0.0, Color32::WHITE);
    (response, painter, origin)
}

/// Il riquadro in mm sul foglio portato in punti schermo.
pub(crate) fn to_screen(origin: Pos2, r: winfiles_geom::Rect, scale: f32) -> Rect {
    Rect::from_min_size(
        origin + Vec2::new(r.x as f32, r.y as f32) * scale,
        Vec2::new(r.w as f32, r.h as f32) * scale,
    )
}

/// "1 pagina", "3 pagine".
pub(crate) fn pages_text(n: usize) -> String {
    if n == 1 {
        "1 pagina".into()
    } else {
        format!("{n} pagine")
    }
}

/// I totali di una cartella a destra della sua intestazione: file, pagine e area, con le misure
/// che le caselle dell'impaginazione lasciano vedere.
pub(crate) fn totals_text(t: &Totals, print: &PrintSettings) -> String {
    let mut text = format!("{} file · {}", t.files, pages_text(t.pages));
    if let Some(area) = area_text(t.area_full_m2, t.area_m2, print.show_sizes, print.show_trim) {
        text = format!("{text} · {area}");
    }
    if t.unmeasured > 0 {
        text = format!("{text} (da misurare: {})", t.unmeasured);
    }
    text
}

/// L'intestazione di una cartella: le cartelle come etichette colorate, una linea sottile e i
/// totali a destra. Nessun riquadro grigio. `size` è il corpo del testo in mm. Quando non sta in
/// una riga va a capo (una cartella o un titolo più lungo della riga va a capo anche dentro la
/// sua etichetta): niente esce dal foglio e niente si perde.
pub(crate) fn draw_header(
    sk: &mut Sketch<'_>,
    title: MmRect,
    header: &Header,
    custom: Option<&str>,
    totals: Option<&str>,
    path: Option<&str>,
    size: f64,
) {
    let flow = header_flow(
        sk,
        title.w,
        &crumbs(header, custom),
        header.continued,
        totals,
        path,
        size,
    );
    let at = |(x, y): (f64, f64)| (title.x + x, title.y + y);
    for part in &flow.parts {
        match part {
            Part::Pill { rect, galley, color } => {
                let r = MmRect::new(title.x + rect.x, title.y + rect.y, rect.w, rect.h);
                sk.rect(r, 1.2, Some(*color), None);
                let pad_x = size * 0.5;
                let pad_y = size * 0.22;
                sk.text(
                    (r.x + pad_x, r.y + pad_y),
                    galley,
                    size,
                    Weight::SemiBold,
                    Color32::WHITE,
                    None,
                );
            }
            Part::Text {
                at: pos,
                galley,
                size,
                weight,
            } => sk.text(at(*pos), galley, *size, *weight, INK_SOFT, None),
        }
    }
    if let Some((from, to)) = flow.line {
        sk.line(at(from), at(to), FRAME, 0.25);
    }
}

/// Le cartelle da mostrare: il titolo scelto dall'utente, se c'è, è un'unica etichetta.
fn crumbs<'a>(header: &'a Header, custom: Option<&'a str>) -> Vec<&'a str> {
    match custom {
        Some(text) => vec![text],
        None => header.folder.split(" › ").collect(),
    }
}

/// Un pezzo di intestazione già al suo posto, rispetto all'angolo in alto a sinistra.
enum Part {
    Pill {
        rect: MmRect,
        galley: std::sync::Arc<egui::Galley>,
        color: Color32,
    },
    Text {
        at: (f64, f64),
        galley: std::sync::Arc<egui::Galley>,
        size: f64,
        weight: Weight,
    },
}

/// Un'intestazione sistemata in righe larghe al massimo `width` mm.
struct Flow {
    parts: Vec<Part>,
    /// La linea sottile dopo le cartelle, se c'è posto.
    line: Option<((f64, f64), (f64, f64))>,
    height: f64,
}

/// Un elemento da mettere in riga: un'etichetta (con il "›" che la segue) o un testo.
struct Piece {
    galley: std::sync::Arc<egui::Galley>,
    /// Larghezza e altezza della parte colorata (o del testo), in mm.
    w: f64,
    h: f64,
    pill: Option<Color32>,
    /// Il "›" dopo l'etichetta, con la sua larghezza.
    sep: Option<(std::sync::Arc<egui::Galley>, f64, f64)>,
    size: f64,
    weight: Weight,
}

fn header_flow(
    sk: &Sketch<'_>,
    width: f64,
    crumbs: &[&str],
    continued: bool,
    totals: Option<&str>,
    path: Option<&str>,
    size: f64,
) -> Flow {
    let width = width.max(1.0);
    let (pad_x, pad_y) = (size * 0.5, size * 0.22);
    // Una riga com'era prima: l'intestazione di una riga resta identica.
    let row_h = crate::captions::header_height(size / MM_PER_PT);
    let single = Sketch::size_mm(&sk.layout_line("Ag", size, Weight::SemiBold)).1 + 2.0 * pad_y;
    let extra = (row_h - single).max(0.0);

    let mut pieces = Vec::with_capacity(crumbs.len() + 1);
    for (i, crumb) in crumbs.iter().enumerate() {
        let color = CRUMB_COLORS
            .get(i % CRUMB_COLORS.len())
            .copied()
            .unwrap_or(INK_SOFT);
        let mut galley = sk.layout_line(crumb, size, Weight::SemiBold);
        if Sketch::size_mm(&galley).0 + 2.0 * pad_x > width {
            // Più lunga della riga intera: va a capo dentro l'etichetta.
            galley = sk.layout(LayoutJob::simple(
                (*crumb).to_string(),
                sketch::font(size, Weight::SemiBold),
                Color32::BLACK,
                ((width - 2.0 * pad_x).max(1.0) as f32) * PX_PER_MM,
            ));
        }
        let (w, h) = Sketch::size_mm(&galley);
        let sep = (i + 1 < crumbs.len()).then(|| {
            let g = sk.layout_line("›", size, Weight::SemiBold);
            let (sw, sh) = Sketch::size_mm(&g);
            (g, sw, sh)
        });
        pieces.push(Piece {
            galley,
            w: w + 2.0 * pad_x,
            h: h + 2.0 * pad_y,
            pill: Some(color),
            sep,
            size,
            weight: Weight::SemiBold,
        });
    }
    if continued {
        let small = size * 0.85;
        let galley = sk.layout_line("(continua)", small, Weight::Regular);
        let (w, h) = Sketch::size_mm(&galley);
        pieces.push(Piece {
            galley,
            w: w + 2.0,
            h,
            pill: None,
            sep: None,
            size: small,
            weight: Weight::Regular,
        });
    }

    // In righe: un pezzo che non sta va a capo (il primo di una riga ci va comunque).
    let mut rows: Vec<Vec<(f64, Piece)>> = vec![Vec::new()];
    let mut x = 0.0;
    for piece in pieces {
        let sep_w = piece.sep.as_ref().map_or(0.0, |s| s.1 + 3.0);
        if x > 0.0 && x + piece.w > width {
            rows.push(Vec::new());
            x = 0.0;
        }
        let at = x;
        x += piece.w + sep_w;
        if let Some(row) = rows.last_mut() {
            row.push((at, piece));
        }
    }
    let end_x = rows.last().and_then(|r| r.last()).map_or(0.0, |(at, p)| at + p.w);

    let mut parts = Vec::new();
    let mut y = 0.0;
    let mut last_cy = row_h / 2.0;
    for row in &rows {
        let tallest = row.iter().map(|(_, p)| p.h).fold(single, f64::max);
        let h = (tallest + extra).max(row_h);
        let cy = y + h / 2.0;
        for (at, p) in row {
            let top = cy - p.h / 2.0;
            match p.pill {
                Some(color) => parts.push(Part::Pill {
                    rect: MmRect::new(*at, top, p.w, p.h),
                    galley: p.galley.clone(),
                    color,
                }),
                None => parts.push(Part::Text {
                    at: (at + 2.0, top),
                    galley: p.galley.clone(),
                    size: p.size,
                    weight: p.weight,
                }),
            }
            if let Some((g, _, sh)) = &p.sep {
                parts.push(Part::Text {
                    at: (at + p.w + 1.0, cy - sh / 2.0),
                    galley: g.clone(),
                    size: p.size,
                    weight: p.weight,
                });
            }
        }
        last_cy = cy;
        y += h;
    }

    // I totali a destra dell'ultima riga, o in una riga tutta loro se lì non c'è posto.
    let mut line_end = width;
    if let Some(text) = totals {
        let small = size * 0.82;
        let galley = sk.layout_line(text, small, Weight::Regular);
        let (w, h) = Sketch::size_mm(&galley);
        let tx = (width - w).max(0.0);
        if tx > end_x + 6.0 {
            line_end = tx - 2.5;
            parts.push(Part::Text {
                at: (tx, last_cy - h / 2.0),
                galley,
                size: small,
                weight: Weight::Regular,
            });
        } else {
            parts.push(Part::Text {
                at: (tx, y + (row_h - h) / 2.0),
                galley,
                size: small,
                weight: Weight::Regular,
            });
            y += row_h;
        }
    }
    let line = (line_end > end_x + 4.0).then_some(((end_x + 2.5, last_cy), (line_end, last_cy)));
    // Il percorso della cartella sotto le etichette, piccolo; un percorso lungo va a capo.
    if let Some(text) = path.filter(|p| !p.is_empty()) {
        let small = size * 0.78;
        let galley = sk.layout(LayoutJob::simple(
            text.to_string(),
            sketch::font(small, Weight::Regular),
            Color32::BLACK,
            ((width - 2.0).max(1.0) as f32) * PX_PER_MM,
        ));
        let h = Sketch::size_mm(&galley).1;
        parts.push(Part::Text {
            at: (2.0, y + 0.3),
            galley,
            size: small,
            weight: Weight::Regular,
        });
        y += h + 1.2;
    }
    Flow {
        parts,
        line,
        height: y,
    }
}

/// Quanto è alta, in mm, l'intestazione della cartella `folder` (con l'etichetta `label`) su una
/// riga larga `width` mm: più di una riga quando le cartelle vanno a capo.
pub(crate) fn header_block(
    ctx: &egui::Context,
    project: &Project,
    folder: &Path,
    label: &str,
    width: f64,
    continued: bool,
) -> f64 {
    let print = &project.print;
    let sk = Sketch::new(ctx, winfiles_geom::Size::new(width.max(1.0), 1.0));
    let custom = project
        .folder_titles
        .get(folder)
        .map(|t| t.trim())
        .filter(|t| !t.is_empty());
    let header = Header {
        folder: label.to_string(),
        rect: MmRect::new(0.0, 0.0, width, 0.0),
        note: None,
        continued,
    };
    let totals = project.folder_totals().get(folder).map(|t| totals_text(t, print));
    let path = folder_path(project, folder);
    header_flow(
        &sk,
        width,
        &crumbs(&header, custom),
        continued,
        totals.as_deref(),
        path.as_deref(),
        print.header_text * MM_PER_PT,
    )
    .height
}

/// Il percorso da scrivere sotto l'intestazione, se la casella è accesa.
pub(crate) fn folder_path(project: &Project, folder: &Path) -> Option<String> {
    project
        .print
        .show_path
        .then(|| folder.display().to_string())
        .filter(|p| !p.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La segnalazione di Michel: con il testo dell'intestazione grande le cartelle uscivano dal
    /// foglio. Vanno a capo, restano tutte e nessuna esce dalla riga.
    #[test]
    fn l_intestazione_grande_va_a_capo_e_non_esce() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let sk = Sketch::new(ui.ctx(), winfiles_geom::Size::new(190.0, 277.0));
            let size = 24.0 * MM_PER_PT;
            let crumbs = [
                "Ordini 2026",
                "Cliente Rossi Arredamenti Srl",
                "Campagna Primavera Estate",
            ];
            let flow = header_flow(
                &sk,
                190.0,
                &crumbs,
                true,
                Some("2 file · 2 pagine · 0,090 m²"),
                None,
                size,
            );
            let one = crate::captions::header_height(24.0);
            assert!(flow.height > 2.0 * one, "deve andare a capo: {}", flow.height);
            let pills = flow
                .parts
                .iter()
                .filter_map(|p| match p {
                    Part::Pill { rect, .. } => Some(*rect),
                    Part::Text { .. } => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(pills.len(), crumbs.len());
            for r in &pills {
                assert!(r.x >= 0.0 && r.right() <= 190.0 + 1e-6 && r.bottom() <= flow.height + 1e-6);
            }
            // Un titolo più lungo della riga va a capo dentro la sua etichetta.
            let long = ["un titolo scelto a mano ".repeat(6)];
            let long: Vec<&str> = long.iter().map(String::as_str).collect();
            let flow = header_flow(&sk, 190.0, &long, false, None, None, size);
            assert!(flow.height > one);
            // Corto: una riga, alta come sempre.
            let short = header_flow(&sk, 190.0, &["Foto"], false, Some("1 file"), None, size);
            assert!((short.height - one).abs() < 1e-9);
        });
        out.textures_delta.clear();
    }

    /// Richiesta di Michel: il percorso della cartella sotto l'intestazione. Aggiunge altezza,
    /// sta sotto le etichette e un percorso lunghissimo va a capo senza uscire dalla riga.
    #[test]
    fn il_percorso_sta_sotto_l_intestazione_e_va_a_capo() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let sk = Sketch::new(ui.ctx(), winfiles_geom::Size::new(190.0, 277.0));
            let size = 9.0 * MM_PER_PT;
            let crumbs = ["Lavori", "Cliente Rossi"];
            let plain = header_flow(&sk, 190.0, &crumbs, false, Some("1 file"), None, size);
            let short = "C:\\Lavori\\Cliente Rossi";
            let with = header_flow(&sk, 190.0, &crumbs, false, Some("1 file"), Some(short), size);
            assert!(with.height > plain.height);
            let long = format!("\\\\server\\{}", "CartellaConUnNomeMoltoLungo\\".repeat(12));
            let wrapped = header_flow(&sk, 190.0, &crumbs, false, None, Some(&long), size);
            assert!(wrapped.height > with.height, "il percorso lungo va a capo");
            for p in &wrapped.parts {
                if let Part::Text { at, galley, .. } = p {
                    let (w, _) = Sketch::size_mm(galley);
                    assert!(at.0 + w <= 190.0 + 1e-6, "esce dalla riga: {} + {w}", at.0);
                }
            }
        });
        out.textures_delta.clear();
    }

    /// Il blocco segnalato da Michel: dopo un cambio di foglio con la rotella verso l'alto lo
    /// scorrimento andava a un valore enorme e il foglio spariva dalla vista. L'altezza dello
    /// spazio è sempre finita e il fondo del foglio sta dentro.
    #[test]
    fn il_fondo_del_foglio_e_raggiungibile() {
        let a4 = winfiles_geom::Size::new(210.0, 297.0);
        for (scale, view) in [
            (0.1, 600.0),
            (1.3 * 3.78, 600.0),
            (MAX_ZOOM, 600.0),
            (2.0, 5000.0),
        ] {
            let h = content_height(a4, scale, view);
            assert!(h.is_finite() && h >= view);
            let bottom = (h - view).max(0.0);
            assert!(bottom + view >= 297.0 * scale + FIT_MARGIN - 1e-3);
        }
    }
}
