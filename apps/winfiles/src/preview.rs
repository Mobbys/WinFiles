//! Disegna un foglio impaginato. Le misure arrivano in millimetri dal motore di layout e qui
//! vengono solo scalate allo spazio disponibile: nessun calcolo di impaginazione in questo file.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use winfiles_layout::Sheet;
use winfiles_model::Project;

/// Disegna `sheet` centrato nello spazio disponibile. Restituisce l'id della miniatura cliccata.
pub fn draw_sheet(
    ui: &mut egui::Ui,
    sheet: &Sheet,
    project: &Project,
    selected: Option<usize>,
) -> Option<usize> {
    let avail = ui.available_size();
    let (w, h) = (sheet.size.w as f32, sheet.size.h as f32);
    let scale = ((avail.x - 24.0) / w).min((avail.y - 24.0) / h).max(0.1);
    let (response, painter) = ui.allocate_painter(avail, Sense::click());
    let origin = response.rect.center() - Vec2::new(w * scale, h * scale) / 2.0;
    let map = |r: winfiles_geom::Rect| {
        Rect::from_min_size(
            origin + Vec2::new(r.x as f32, r.y as f32) * scale,
            Vec2::new(r.w as f32, r.h as f32) * scale,
        )
    };

    let page = Rect::from_min_size(origin, Vec2::new(w, h) * scale);
    painter.rect_filled(
        page.translate(Vec2::splat(3.0)),
        0.0,
        Color32::from_black_alpha(40),
    );
    painter.rect_filled(page, 0.0, Color32::WHITE);
    painter.rect_stroke(
        map(sheet.area),
        0.0,
        Stroke::new(0.5, Color32::from_gray(220)),
        StrokeKind::Inside,
    );

    let text_size = (3.0 * scale).clamp(7.0, 14.0);
    for header in &sheet.headers {
        let r = map(header.rect);
        painter.rect_filled(r, 2.0, Color32::from_rgb(226, 232, 240));
        let label = if header.continued {
            format!("{} (continua)", header.folder)
        } else {
            header.folder.clone()
        };
        painter.text(
            r.left_center() + Vec2::new(6.0, 0.0),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(text_size),
            Color32::from_gray(40),
        );
    }

    let mut clicked = None;
    let pointer = response.interact_pointer_pos();
    for item in &sheet.items {
        let cell = map(item.cell);
        let image = map(item.image);
        let file = project.files.get(item.id);
        let kind = file.map(|f| f.kind.label()).unwrap_or("");
        painter.rect_filled(image, 0.0, Color32::from_gray(232));
        painter.rect_stroke(
            image,
            0.0,
            Stroke::new(0.5, Color32::from_gray(150)),
            StrokeKind::Inside,
        );
        painter.text(
            image.center(),
            Align2::CENTER_CENTER,
            kind,
            FontId::proportional(text_size),
            Color32::from_gray(120),
        );
        let name = file
            .and_then(|f| f.path.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let caption = map(item.caption);
        painter.text(
            Pos2::new(caption.center().x, caption.top() + 2.0),
            Align2::CENTER_TOP,
            name,
            FontId::proportional(text_size),
            Color32::from_gray(30),
        );
        if selected == Some(item.id) {
            painter.rect_stroke(
                cell.expand(2.0),
                2.0,
                Stroke::new(2.0, Color32::from_rgb(37, 99, 235)),
                StrokeKind::Outside,
            );
        }
        if response.clicked() && pointer.is_some_and(|p| cell.contains(p)) {
            clicked = Some(item.id);
        }
    }
    clicked
}
