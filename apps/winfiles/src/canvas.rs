//! L'anteprima: il foglio impaginato sul fondo grigio, come un documento in VectorCraft. Le misure
//! arrivano in millimetri dal motore di layout e qui vengono solo scalate: nessun calcolo di
//! impaginazione in questo file.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use winfiles_geom::{format_area, format_size};
use winfiles_layout::Sheet;
use winfiles_model::Project;

use crate::theme::{TOKENS, semibold};

/// Margine attorno al foglio quando l'anteprima lo adatta alla finestra, in punti schermo.
const FIT_MARGIN: f32 = 28.0;
pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 8.0;

/// Ingrandimento che fa stare il foglio intero nello spazio `avail` (1.0 = 1 mm per punto).
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

/// Disegna `sheet` a `scale` punti per millimetro nello spazio disponibile.
pub fn draw(
    ui: &mut egui::Ui,
    sheet: &Sheet,
    project: &Project,
    selected: Option<usize>,
    scale: f32,
) -> CanvasOutput {
    let t = TOKENS;
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
        page.translate(Vec2::new(0.0, 2.0)).expand(1.0),
        0.0,
        Color32::from_black_alpha(70),
    );
    painter.rect_filled(page, 0.0, Color32::WHITE);
    painter.rect_stroke(
        map(sheet.area),
        0.0,
        Stroke::new(0.5, Color32::from_rgb(170, 200, 240)),
        StrokeKind::Inside,
    );

    // Corpo del testo in millimetri, come sarà stampato; sotto i 4 punti schermo non si legge.
    let header_text = mm(3.2);
    let name_text = mm(2.5);
    let info_text = mm(2.1);
    for header in &sheet.headers {
        let r = map(header.rect);
        painter.rect_filled(r, 0.0, Color32::from_gray(236));
        painter.line_segment(
            [r.left_bottom(), r.right_bottom()],
            Stroke::new(1.0, Color32::from_gray(160)),
        );
        if header_text >= 4.0 {
            let label = if header.continued {
                format!("{} (continua)", header.folder)
            } else {
                header.folder.clone()
            };
            let galley = painter.layout_no_wrap(label, semibold(header_text), Color32::from_gray(30));
            let clip = painter.with_clip_rect(r);
            clip.galley(
                Pos2::new(r.left() + mm(2.0), r.center().y - galley.size().y / 2.0),
                galley,
                Color32::from_gray(30),
            );
        }
    }

    let pointer = response.interact_pointer_pos();
    for item in &sheet.items {
        let cell = map(item.cell);
        let image = map(item.image);
        let file = project.files.get(item.id);
        painter.rect_filled(image, 0.0, Color32::from_gray(225));
        painter.rect_stroke(
            image,
            0.0,
            Stroke::new(0.5, Color32::from_gray(150)),
            StrokeKind::Inside,
        );
        if name_text >= 4.0 {
            let kind = file.map(|f| f.kind.label()).unwrap_or("");
            painter.text(
                image.center(),
                Align2::CENTER_CENTER,
                kind,
                FontId::proportional(name_text),
                Color32::from_gray(130),
            );
        }
        let caption = map(item.caption);
        let clip = painter.with_clip_rect(caption);
        if let Some(f) = file {
            let name = f
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut y = caption.top() + mm(1.0);
            if name_text >= 4.0 {
                let galley = painter.layout(
                    name,
                    FontId::proportional(name_text),
                    Color32::from_gray(25),
                    caption.width(),
                );
                let h = galley.rows.first().map_or(galley.size().y, |r| r.height());
                // Una riga sola: i nomi lunghi si tagliano.
                clip.with_clip_rect(Rect::from_min_size(
                    Pos2::new(caption.left(), y),
                    Vec2::new(caption.width(), h),
                ))
                .galley(
                    Pos2::new(caption.center().x - galley.size().x.min(caption.width()) / 2.0, y),
                    galley,
                    Color32::from_gray(25),
                );
                y += h + mm(0.4);
            }
            if info_text >= 4.0
                && let Some(first) = f.pages.first()
            {
                let mut info = format!(
                    "{} · {}",
                    format_size(first.finished(), project.unit),
                    format_area(f.area_m2())
                );
                if f.pages.len() > 1 {
                    info = format!("{info} · {} pag.", f.pages.len());
                }
                clip.text(
                    Pos2::new(caption.center().x, y),
                    Align2::CENTER_TOP,
                    info,
                    FontId::proportional(info_text),
                    Color32::from_gray(90),
                );
            }
        }
        if selected == Some(item.id) {
            painter.rect_stroke(
                cell.expand(2.0),
                2.0,
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
