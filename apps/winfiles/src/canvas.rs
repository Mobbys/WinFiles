//! L'anteprima: il foglio impaginato sul fondo grigio, come un documento in VectorCraft. Le misure
//! arrivano in millimetri dal motore di layout e qui vengono solo scalate: nessun calcolo di
//! impaginazione in questo file. Il foglio è carta bianca e resta bianco anche nel tema scuro.

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2};
use winfiles_geom::{area_m2, format_area, format_size};
use winfiles_layout::Sheet;
use winfiles_model::{PageInfo, Project};

use crate::captions::{
    CAPTION_PAD, FOLDER_NOTE_INSET, INFO_RATIO, MM_PER_PT, NOTE_GAP, NOTE_INK, Slot, line_mm, note_job,
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
const HEADER_FILL: Color32 = Color32::from_rgb(234, 234, 234);
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
    let header_px = mm(3.4);
    for header in &sheet.headers {
        let r = map(header.rect);
        let title = Rect::from_min_size(r.min, Vec2::new(r.width(), mm(print.header)));
        painter.rect_filled(title, 0.0, HEADER_FILL);
        painter.line_segment(
            [title.left_bottom(), title.right_bottom()],
            Stroke::new(mm(0.3).max(0.5), INK_SOFT),
        );
        if header_px >= 4.0 {
            let label = if header.continued {
                format!("{} (continua)", header.folder)
            } else {
                header.folder.clone()
            };
            let mut job = one_line(label, semibold(header_px), INK, title.width() - mm(4.0));
            job.halign = egui::Align::LEFT;
            let galley = painter.layout_job(job);
            painter.galley(
                pos2(title.left() + mm(2.0), title.center().y - galley.size().y / 2.0),
                galley,
                INK,
            );
        }
        if let Some(note) = header.note
            && font(print.note_text) >= 4.0
        {
            let note_rect = map(note);
            let folder_note = slots
                .iter()
                .filter_map(|s| project.files.get(s.file))
                .find(|f| project.folder_labels().get(f.folder()) == Some(&header.folder))
                .and_then(|f| project.folder_notes.get(f.folder()));
            if let Some(text) = folder_note {
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
