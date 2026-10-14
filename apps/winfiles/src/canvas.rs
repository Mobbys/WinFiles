//! L'anteprima: il foglio impaginato sul fondo grigio, come un documento in VectorCraft. Le misure
//! arrivano in millimetri dal motore di layout e qui vengono solo scalate: nessun calcolo di
//! impaginazione in questo file. Il foglio è carta bianca e resta bianco anche nel tema scuro.

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2};
use std::collections::HashMap;
use std::path::Path;
use winfiles_geom::{area_m2, format_area, format_size};

use winfiles_layout::{Header, Sheet};
use winfiles_model::{PageInfo, Project, Totals};

use crate::captions::{
    CAPTION_PAD, FOLDER_NOTE_INSET, INFO_RATIO, MM_PER_PT, NOTE_GAP, NOTE_INK, Slot, TITLE_PT, line_mm,
    note_job, title_job,
};
use crate::theme::{Tokens, semibold};
use crate::thumbs::{ThumbState, Thumbnails};

/// Margine attorno al foglio quando l'anteprima lo adatta alla finestra, in punti schermo.
const FIT_MARGIN: f32 = 28.0;
pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 8.0;

/// Colori della carta stampata (non dell'interfaccia): testo quasi nero, grigi di servizio.
const INK: Color32 = Color32::from_rgb(20, 20, 20);
const INK_SOFT: Color32 = Color32::from_rgb(85, 85, 85);
/// Colori delle etichette delle cartelle, dal livello più alto al più basso, come le "briciole"
/// colorate della 4.9 ma scuri abbastanza da leggerci il bianco anche stampati.
const CRUMB_COLORS: [Color32; 5] = [
    Color32::from_rgb(0x2f, 0x6f, 0xd0),
    Color32::from_rgb(0x0e, 0x80, 0x6c),
    Color32::from_rgb(0xb8, 0x5c, 0x00),
    Color32::from_rgb(0x8e, 0x24, 0xaa),
    Color32::from_rgb(0x55, 0x2b, 0xc4),
];
const FRAME: Color32 = Color32::from_rgb(190, 190, 190);

/// Ingrandimento che fa stare il foglio intero nello spazio `avail` (punti per millimetro).
pub fn fit_zoom(sheet: &Sheet, avail: Vec2) -> f32 {
    let (w, h) = (sheet.size.w as f32, sheet.size.h as f32);
    ((avail.x - 2.0 * FIT_MARGIN) / w)
        .min((avail.y - 2.0 * FIT_MARGIN) / h)
        .clamp(MIN_ZOOM, MAX_ZOOM)
}

/// Cosa è successo nell'anteprima in questo fotogramma.
#[derive(Default)]
pub struct CanvasOutput {
    pub clicked: Option<usize>,
    /// Ctrl + rotella: fattore di ingrandimento richiesto.
    pub zoom_by: Option<f32>,
}

/// Una riga di testo che si tronca con "…" se non sta nella larghezza.
fn one_line(text: String, font: FontId, color: Color32, width: f32) -> LayoutJob {
    let mut job = LayoutJob::single_section(
        text,
        TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping {
        max_width: width.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    job.halign = egui::Align::Center;
    job
}

/// La parte della miniatura da mostrare: per i PDF con TrimBox solo il formato finito, senza
/// l'abbondanza, come il riquadro calcolato dal layout.
fn trim_uv(page: Option<&PageInfo>) -> Rect {
    let full = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    let Some(page) = page else {
        return full;
    };
    let Some(trim) = page.trim else { return full };
    let fx = (trim.w / page.size.w).clamp(0.0, 1.0) as f32;
    let fy = (trim.h / page.size.h).clamp(0.0, 1.0) as f32;
    if !(fx > 0.0 && fy > 0.0) {
        return full;
    }
    Rect::from_center_size(pos2(0.5, 0.5), Vec2::new(fx, fy))
}

/// Disegna `sheet` a `scale` punti per millimetro nello spazio disponibile. `slots` dice a quale
/// pagina di quale file corrisponde ogni elemento; `selected` è il file scelto.
pub fn draw(
    ui: &mut egui::Ui,
    sheet: &Sheet,
    project: &Project,
    slots: &[Slot],
    thumbs: &mut Thumbnails,
    selected: Option<usize>,
    scale: f32,
) -> CanvasOutput {
    let t = Tokens::get(ui.ctx());
    let mut out = CanvasOutput::default();
    let page_size = Vec2::new(sheet.size.w as f32, sheet.size.h as f32) * scale;
    let avail = ui.available_size();
    let size = Vec2::new(
        avail.x.max(page_size.x + 2.0 * FIT_MARGIN),
        avail.y.max(page_size.y + 2.0 * FIT_MARGIN),
    );
    let (response, painter) = ui.allocate_painter(size, Sense::click());
    let origin = response.rect.center() - page_size / 2.0;
    let map = |r: winfiles_geom::Rect| {
        Rect::from_min_size(
            origin + Vec2::new(r.x as f32, r.y as f32) * scale,
            Vec2::new(r.w as f32, r.h as f32) * scale,
        )
    };
    let mm = |v: f64| v as f32 * scale;

    let page = Rect::from_min_size(origin, page_size);
    painter.rect_filled(
        page.translate(Vec2::new(0.0, 3.0)).expand(1.0),
        0.0,
        Color32::from_black_alpha(90),
    );
    painter.rect_filled(page, 0.0, Color32::WHITE);

    // Corpo del testo come sarà stampato (punti tipografici convertiti in mm e poi in punti
    // schermo); sotto i 4 punti schermo non si legge e non si disegna.
    let print = &project.print;
    let font = |pt: f64| (pt * MM_PER_PT) as f32 * scale;
    let (name_pt, info_pt) = (print.caption_text, print.caption_text * INFO_RATIO);
    // Per ogni intestazione: totali e nota della sua cartella.
    let labels = project.folder_labels();
    let totals = project.folder_totals();
    let folders: HashMap<&str, (&Path, Totals)> = labels
        .iter()
        .filter_map(|(path, label)| Some((label.as_str(), (*path, *totals.get(path)?))))
        .collect();
    if let Some(r) = sheet.title
        && font(TITLE_PT) >= 4.0
    {
        let r = map(r);
        let galley = painter.layout_job(title_job(&print.title, r.width() / scale, scale));
        painter.with_clip_rect(r).galley(r.min, galley, INK);
    }
    for header in &sheet.headers {
        let folder = folders.get(header.folder.as_str());
        let r = map(header.rect);
        let title = Rect::from_min_size(r.min, Vec2::new(r.width(), mm(print.header)));
        let custom = folder
            .and_then(|f| project.folder_titles.get(f.0))
            .map(|t| t.trim())
            .filter(|t| !t.is_empty());
        draw_header(&painter, title, header, custom, folder.map(|f| &f.1), scale);
        if let Some(note) = header.note
            && font(print.note_text) >= 4.0
            && let Some(text) = folder.and_then(|f| project.folder_notes.get(f.0))
        {
            let note_rect = map(note);
            let width = note.w - 2.0 * FOLDER_NOTE_INSET;
            let galley = painter.layout_job(note_job(text, print.note_text, width, scale));
            painter.with_clip_rect(note_rect).galley(
                pos2(
                    note_rect.left() + mm(FOLDER_NOTE_INSET),
                    note_rect.top() + mm(CAPTION_PAD),
                ),
                galley,
                NOTE_INK,
            );
        }
    }

    let pointer = response.interact_pointer_pos();
    for item in &sheet.items {
        let cell = map(item.cell);
        let image = map(item.image);
        let Some(slot) = slots.get(item.id) else {
            continue;
        };
        let Some(file) = project.files.get(slot.file) else {
            continue;
        };
        let page_info = file.pages.get(slot.page);
        let visible = ui.clip_rect().intersects(cell);
        match visible.then(|| thumbs.get(&file.path, file.kind, slot.page)) {
            Some(ThumbState::Ready(tex)) => {
                painter.image(tex.id(), image, trim_uv(page_info), Color32::WHITE);
                painter.rect_stroke(image, 0.0, Stroke::new(0.5, FRAME), StrokeKind::Outside);
            }
            state => {
                painter.rect_filled(image, 0.0, Color32::from_gray(242));
                painter.rect_stroke(image, 0.0, Stroke::new(0.5, FRAME), StrokeKind::Inside);
                if font(info_pt) >= 4.0 {
                    let text = match state {
                        Some(ThumbState::Failed(_)) => {
                            format!("{}\nanteprima non disponibile", file.kind.label())
                        }
                        _ => "…".to_string(),
                    };
                    let mut job = LayoutJob::single_section(
                        text,
                        TextFormat {
                            font_id: FontId::proportional(font(info_pt)),
                            color: INK_SOFT,
                            ..Default::default()
                        },
                    );
                    job.wrap.max_width = image.width();
                    job.halign = egui::Align::Center;
                    let galley = painter.layout_job(job);
                    painter.galley(
                        pos2(image.center().x, image.center().y - galley.size().y / 2.0),
                        galley,
                        INK_SOFT,
                    );
                }
            }
        }

        // Le pagine di uno stesso file: cornice ed etichetta "Pag. 2/4" nel colore del file.
        if let Some(color) = slot.color {
            painter.rect_stroke(
                image,
                0.0,
                Stroke::new(mm(0.5).max(1.0), color),
                StrokeKind::Outside,
            );
            let px = font(name_pt * 0.9);
            if px >= 4.0 {
                let label = format!("Pag. {}/{}", slot.page + 1, slot.pages);
                let galley = painter.layout_no_wrap(label, semibold(px), Color32::WHITE);
                let pad = Vec2::new(px * 0.45, px * 0.2);
                let badge = Rect::from_min_size(image.min + Vec2::splat(mm(1.0)), galley.size() + pad * 2.0);
                painter.rect_filled(badge, px * 0.3, color);
                painter.galley(badge.min + pad, galley, Color32::WHITE);
            }
        }

        let caption = map(item.caption);
        let clip = painter.with_clip_rect(caption.intersect(painter.clip_rect()));
        let mut y = caption.top() + mm(CAPTION_PAD);
        if font(name_pt) >= 4.0 {
            let name = file
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let galley = clip.layout_job(one_line(name, semibold(font(name_pt)), INK, caption.width()));
            clip.galley(pos2(caption.center().x, y), galley, INK);
        }
        y += mm(line_mm(name_pt));
        if let Some(page) = page_info {
            let finished = page.finished();
            let mut area = format_area(area_m2(finished));
            if slot.pages > 1 {
                area = format!("{area} · pagina {} di {}", slot.page + 1, slot.pages);
            }
            for line in [format_size(finished, project.unit), area] {
                if font(info_pt) >= 4.0 {
                    let galley = clip.layout_job(one_line(
                        line,
                        FontId::proportional(font(info_pt)),
                        INK_SOFT,
                        caption.width(),
                    ));
                    clip.galley(pos2(caption.center().x, y), galley, INK_SOFT);
                }
                y += mm(line_mm(info_pt));
            }
        } else {
            y += 2.0 * mm(line_mm(info_pt));
        }
        if !file.note.trim().is_empty() && font(print.note_text) >= 4.0 {
            y += mm(NOTE_GAP);
            let mut job = note_job(&file.note, print.note_text, item.caption.w, scale);
            job.halign = egui::Align::Center;
            let galley = clip.layout_job(job);
            clip.galley(pos2(caption.center().x, y), galley, NOTE_INK);
        }

        if selected == Some(slot.file) {
            painter.rect_stroke(
                cell.expand(3.0),
                3.0,
                Stroke::new(2.0, t.accent),
                StrokeKind::Outside,
            );
        }
        if response.clicked() && pointer.is_some_and(|p| cell.contains(p)) {
            out.clicked = Some(slot.file);
        }
    }

    if response.hovered() {
        // Ctrl + rotella (o il pizzico sul touchpad) arriva da egui come fattore di zoom.
        let zoom = ui.input(|i| i.zoom_delta());
        if zoom != 1.0 {
            out.zoom_by = Some(zoom);
        }
    }
    out
}

/// L'intestazione di una cartella: le cartelle come etichette colorate, una linea sottile e i
/// totali a destra. Nessun riquadro grigio.
fn draw_header(
    painter: &egui::Painter,
    title: Rect,
    header: &Header,
    custom: Option<&str>,
    totals: Option<&Totals>,
    scale: f32,
) {
    let mm = |v: f32| v * scale;
    let px = mm(3.2);
    if px < 4.0 {
        painter.line_segment(
            [title.left_center(), title.right_center()],
            Stroke::new(1.0, FRAME),
        );
        return;
    }
    let cy = title.center().y;
    let pad = Vec2::new(mm(1.6), mm(0.7));
    let mut x = title.left();
    // Un'intestazione scelta dall'utente è un'unica etichetta; altrimenti una per cartella.
    let crumbs: Vec<&str> = match custom {
        Some(text) => vec![text],
        None => header.folder.split(" › ").collect(),
    };
    for (i, crumb) in crumbs.iter().enumerate() {
        let color = CRUMB_COLORS
            .get(i % CRUMB_COLORS.len())
            .copied()
            .unwrap_or(INK_SOFT);
        let galley = painter.layout_no_wrap((*crumb).to_string(), semibold(px), Color32::WHITE);
        let size = galley.size() + pad * 2.0;
        // Le cartelle che non stanno si fermano prima dei totali.
        if x + size.x > title.right() - mm(30.0) && i > 0 {
            break;
        }
        let pill = Rect::from_min_size(pos2(x, cy - size.y / 2.0), size);
        painter.rect_filled(pill, mm(1.2), color);
        painter.galley(pill.min + pad, galley, Color32::WHITE);
        x = pill.right();
        if i + 1 < crumbs.len() {
            let sep = painter.layout_no_wrap("›".to_string(), semibold(px), INK_SOFT);
            painter.galley(pos2(x + mm(1.0), cy - sep.size().y / 2.0), sep.clone(), INK_SOFT);
            x += sep.size().x + mm(2.0);
        }
    }
    if header.continued {
        let more = painter.layout_no_wrap(
            "(continua)".to_string(),
            FontId::proportional(px * 0.85),
            INK_SOFT,
        );
        painter.galley(
            pos2(x + mm(2.0), cy - more.size().y / 2.0),
            more.clone(),
            INK_SOFT,
        );
        x += more.size().x + mm(2.0);
    }
    let mut line_end = title.right();
    if let Some(t) = totals {
        let mut text = format!(
            "{} file · {} pagine · {}",
            t.files,
            t.pages,
            format_area(t.area_m2)
        );
        if t.unmeasured > 0 {
            text = format!("{text} (da misurare: {})", t.unmeasured);
        }
        let galley = painter.layout_no_wrap(text, FontId::proportional(px * 0.82), INK_SOFT);
        let pos = pos2(title.right() - galley.size().x, cy - galley.size().y / 2.0);
        if pos.x > x + mm(6.0) {
            line_end = pos.x - mm(2.5);
            painter.galley(pos, galley, INK_SOFT);
        }
    }
    if line_end > x + mm(4.0) {
        painter.line_segment(
            [pos2(x + mm(2.5), cy), pos2(line_end, cy)],
            Stroke::new(mm(0.25).max(1.0), FRAME),
        );
    }
}
