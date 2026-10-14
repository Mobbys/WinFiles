//! L'anteprima: il foglio impaginato sul fondo grigio, come un documento in VectorCraft. Le misure
//! arrivano in millimetri dal motore di layout e qui vengono solo scalate: nessun calcolo di
//! impaginazione in questo file. Il foglio è carta bianca e resta bianco anche nel tema scuro.

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2};
use winfiles_geom::{format_area, format_size};
use winfiles_layout::Sheet;
use winfiles_model::{FileEntry, Project};

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
fn trim_uv(file: &FileEntry) -> Rect {
    let full = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    let Some(page) = file.pages.first() else {
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

/// Disegna `sheet` a `scale` punti per millimetro nello spazio disponibile.
pub fn draw(
    ui: &mut egui::Ui,
    sheet: &Sheet,
    project: &Project,
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
    let mm = |v: f32| v * scale;

    let page = Rect::from_min_size(origin, page_size);
    painter.rect_filled(
        page.translate(Vec2::new(0.0, 3.0)).expand(1.0),
        0.0,
        Color32::from_black_alpha(90),
    );
    painter.rect_filled(page, 0.0, Color32::WHITE);

    // Corpo del testo in millimetri, come sarà stampato; sotto i 4 punti schermo non si legge
    // e non si disegna.
    let (header_pt, name_pt, info_pt) = (mm(3.4), mm(2.8), mm(2.3));
    for header in &sheet.headers {
        let r = map(header.rect);
        painter.rect_filled(r, 0.0, HEADER_FILL);
        painter.line_segment(
            [r.left_bottom(), r.right_bottom()],
            Stroke::new(mm(0.3).max(0.5), INK_SOFT),
        );
        if header_pt >= 4.0 {
            let label = if header.continued {
                format!("{} (continua)", header.folder)
            } else {
                header.folder.clone()
            };
            let mut job = one_line(label, semibold(header_pt), INK, r.width() - mm(4.0));
            job.halign = egui::Align::LEFT;
            let galley = painter.layout_job(job);
            painter.galley(
                pos2(r.left() + mm(2.0), r.center().y - galley.size().y / 2.0),
                galley,
                INK,
            );
        }
    }

    let pointer = response.interact_pointer_pos();
    for item in &sheet.items {
        let cell = map(item.cell);
        let image = map(item.image);
        let Some(file) = project.files.get(item.id) else {
            continue;
        };
        let visible = ui.clip_rect().intersects(cell);
        match visible.then(|| thumbs.get(&file.path, file.kind)) {
            Some(ThumbState::Ready(tex)) => {
                painter.image(tex.id(), image, trim_uv(file), Color32::WHITE);
                painter.rect_stroke(image, 0.0, Stroke::new(0.5, FRAME), StrokeKind::Outside);
            }
            state => {
                painter.rect_filled(image, 0.0, Color32::from_gray(242));
                painter.rect_stroke(image, 0.0, Stroke::new(0.5, FRAME), StrokeKind::Inside);
                if name_pt >= 4.0 {
                    let text = match state {
                        Some(ThumbState::Failed(_)) => {
                            format!("{}\nanteprima non disponibile", file.kind.label())
                        }
                        _ => "…".to_string(),
                    };
                    let mut job = LayoutJob::single_section(
                        text,
                        TextFormat {
                            font_id: FontId::proportional(info_pt),
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

        let caption = map(item.caption);
        let mut y = caption.top() + mm(1.2);
        if name_pt >= 4.0 {
            let name = file
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let galley = painter.layout_job(one_line(name, semibold(name_pt), INK, caption.width()));
            painter.galley(pos2(caption.center().x, y), galley.clone(), INK);
            y += galley.size().y + mm(0.3);
        }
        if info_pt >= 4.0
            && let Some(first) = file.pages.first()
        {
            let size = format_size(first.finished(), project.unit);
            let mut extra = format_area(file.area_m2());
            if file.pages.len() > 1 {
                extra = format!("{extra} · {} pagine", file.pages.len());
            }
            for line in [size, extra] {
                let galley = painter.layout_job(one_line(
                    line,
                    FontId::proportional(info_pt),
                    INK_SOFT,
                    caption.width(),
                ));
                painter.galley(pos2(caption.center().x, y), galley.clone(), INK_SOFT);
                y += galley.size().y;
            }
        }
        if selected == Some(item.id) {
            painter.rect_stroke(
                cell.expand(3.0),
                3.0,
                Stroke::new(2.0, t.accent),
                StrokeKind::Outside,
            );
        }
        if response.clicked() && pointer.is_some_and(|p| cell.contains(p)) {
            out.clicked = Some(item.id);
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
