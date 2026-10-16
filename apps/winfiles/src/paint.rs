//! L'anteprima disegna la scena del foglio (vedi `sketch`): la stessa che va nel PDF. Le
//! miniature arrivano dalla cache dell'interfaccia; finché non sono pronte resta un riquadro
//! grigio con un avviso, che nel PDF non c'è.

use egui::{Color32, FontId, Pos2, Rect, Stroke, StrokeKind, Vec2, pos2};
use winfiles_scene::{Color, Item, Page, Picture, Text, Weight};

use crate::canvas::{FRAME, INK_SOFT, to_screen};
use crate::thumbs::{ThumbState, Thumbnails};

/// Sotto questa altezza in punti schermo il testo non si legge e non si disegna.
const MIN_TEXT_PX: f32 = 3.5;

fn c32(c: Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// Disegna `page` con l'angolo in alto a sinistra in `origin`, a `scale` punti per mm.
pub fn page(painter: &egui::Painter, origin: Pos2, scale: f32, page: &Page, thumbs: &mut Thumbnails) {
    let mm = |v: f64| v as f32 * scale;
    let map = |r: winfiles_geom::Rect| to_screen(origin, r, scale);
    let at = |(x, y): (f64, f64)| origin + Vec2::new(mm(x), mm(y));
    let visible = painter.clip_rect();
    for item in &page.items {
        match item {
            Item::Rect {
                rect,
                radius,
                fill,
                stroke,
            } => {
                let r = map(*rect);
                if !visible.intersects(r.expand(2.0)) {
                    continue;
                }
                let stroke = stroke.map_or(Stroke::NONE, |(c, w)| Stroke::new(mm(w).max(0.75), c32(c)));
                painter.rect(
                    r,
                    mm(*radius),
                    fill.map_or(Color32::TRANSPARENT, c32),
                    stroke,
                    StrokeKind::Inside,
                );
            }
            Item::Line {
                from,
                to,
                color,
                width,
            } => {
                painter.line_segment(
                    [at(*from), at(*to)],
                    Stroke::new(mm(*width).max(0.75), c32(*color)),
                );
            }
            Item::Text(text) => draw_text(painter, origin, scale, text),
            Item::Picture(p) => picture(painter, map(p.rect), p, thumbs, scale),
        }
    }
}

fn draw_text(painter: &egui::Painter, origin: Pos2, scale: f32, t: &Text) {
    let px = t.size as f32 * scale;
    if px < MIN_TEXT_PX {
        return;
    }
    let x = origin.x + t.x as f32 * scale;
    let baseline = origin.y + t.baseline as f32 * scale;
    // Scarta i testi fuori vista prima di impostarli.
    let visible = painter.clip_rect();
    if baseline < visible.top() - px * 2.0 || baseline > visible.bottom() + px * 2.0 {
        return;
    }
    let font = match t.weight {
        Weight::Regular => FontId::proportional(px),
        Weight::SemiBold => crate::theme::semibold(px),
    };
    let color = c32(t.color);
    let galley = painter.layout_no_wrap(t.text.clone(), font, color);
    let ascent = galley
        .rows
        .first()
        .and_then(|r| r.row.glyphs.first().map(|g| r.pos.y + g.pos.y))
        .unwrap_or(px * 0.8);
    painter.galley(pos2(x, baseline - ascent), galley, color);
}

fn picture(painter: &egui::Painter, r: Rect, p: &Picture, thumbs: &mut Thumbnails, scale: f32) {
    if !painter.clip_rect().intersects(r) {
        return;
    }
    let s = &p.source;
    let crop = Rect::from_min_size(
        pos2(p.crop.x as f32, p.crop.y as f32),
        Vec2::new(p.crop.w as f32, p.crop.h as f32),
    );
    match thumbs.get(&s.path, s.kind, s.page) {
        ThumbState::Ready(tex) => {
            painter.image(tex.id(), r, crop, Color32::WHITE);
        }
        state => {
            painter.rect_filled(r, 0.0, Color32::from_gray(242));
            painter.rect_stroke(r, 0.0, Stroke::new(0.5, FRAME), StrokeKind::Inside);
            let px = (2.6 * scale).min(r.width() / 10.0);
            if px >= 4.0 {
                let text = match state {
                    ThumbState::Failed(_) => format!("{}\nanteprima non disponibile", s.kind.label()),
                    _ => "…".to_string(),
                };
                let mut job = egui::text::LayoutJob::single_section(
                    text,
                    egui::TextFormat {
                        font_id: FontId::proportional(px),
                        color: INK_SOFT,
                        ..Default::default()
                    },
                );
                job.wrap.max_width = r.width();
                job.halign = egui::Align::Center;
                let galley = painter.layout_job(job);
                painter.galley(
                    pos2(r.center().x, r.center().y - galley.size().y / 2.0),
                    galley,
                    INK_SOFT,
                );
            }
        }
    }
}
