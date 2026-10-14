//! Il foglio come scena: i testi si impostano con i caratteri di egui una volta sola, a una
//! risoluzione fissa, e diventano righe già posizionate in millimetri. L'anteprima e il PDF
//! disegnano la stessa scena, quindi gli a capo, i "…" e le posizioni sono gli stessi.

use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{Color32, Galley};
use winfiles_geom::{Rect, Size};
use winfiles_scene::{Color, Item, Page, Picture, Text, Weight};

use crate::theme::semibold;

/// Pixel per millimetro con cui si impostano i testi della scena.
pub const PX_PER_MM: f32 = 10.0;

/// Il colore di stampa dalla tavolozza di egui.
pub fn color(c: Color32) -> Color {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Color { r, g, b, a }
}

/// Il carattere da usare, in pixel della scena, per un corpo in mm.
pub fn font(size_mm: f64, weight: Weight) -> egui::FontId {
    let px = (size_mm as f32 * PX_PER_MM).max(0.5);
    match weight {
        Weight::Regular => egui::FontId::proportional(px),
        Weight::SemiBold => semibold(px),
    }
}

/// Chi scrive la scena di un foglio.
pub struct Sketch<'a> {
    ctx: &'a egui::Context,
    pub page: Page,
}

impl<'a> Sketch<'a> {
    pub fn new(ctx: &'a egui::Context, size: Size) -> Self {
        Self {
            ctx,
            page: Page::new(size),
        }
    }

    pub fn rect(&mut self, rect: Rect, radius: f64, fill: Option<Color32>, stroke: Option<(Color32, f64)>) {
        self.page.push(Item::Rect {
            rect,
            radius,
            fill: fill.map(color),
            stroke: stroke.map(|(c, w)| (color(c), w)),
        });
    }

    pub fn line(&mut self, from: (f64, f64), to: (f64, f64), c: Color32, width: f64) {
        self.page.push(Item::Line {
            from,
            to,
            color: color(c),
            width,
        });
    }

    pub fn picture(&mut self, picture: Picture) {
        self.page.push(Item::Picture(picture));
    }

    /// Imposta un testo (misure in pixel della scena, vedi [`PX_PER_MM`]).
    pub fn layout(&self, job: LayoutJob) -> Arc<Galley> {
        self.ctx.fonts_mut(|f| f.layout_job(job))
    }

    /// Una riga senza a capo con il corpo `size` mm.
    pub fn layout_line(&self, text: &str, size: f64, weight: Weight) -> Arc<Galley> {
        self.layout(LayoutJob::simple_singleline(
            text.to_string(),
            font(size, weight),
            Color32::BLACK,
        ))
    }

    /// Larghezza e altezza di un testo impostato, in mm.
    pub fn size_mm(galley: &Galley) -> (f64, f64) {
        let s = galley.size();
        (f64::from(s.x / PX_PER_MM), f64::from(s.y / PX_PER_MM))
    }

    /// Mette sul foglio le righe di `galley` con l'angolo del testo in `at` (mm). Le righe che
    /// cominciano sotto `bottom` restano fuori, come in un riquadro che taglia.
    pub fn text(
        &mut self,
        at: (f64, f64),
        galley: &Galley,
        size: f64,
        weight: Weight,
        c: Color32,
        bottom: Option<f64>,
    ) {
        let k = f64::from(PX_PER_MM);
        for placed in &galley.rows {
            let row = &placed.row;
            let Some(first) = row.glyphs.first() else {
                continue;
            };
            let top = at.1 + f64::from(placed.pos.y) / k;
            if bottom.is_some_and(|b| top >= b - 1e-6) {
                break;
            }
            let text = row.text();
            if text.trim().is_empty() {
                continue;
            }
            self.page.push(Item::Text(Text {
                x: at.0 + f64::from(placed.pos.x + first.pos.x) / k,
                baseline: top + f64::from(first.pos.y) / k,
                size,
                weight,
                color: color(c),
                text,
                width: f64::from(row.size.x - first.pos.x) / k,
            }));
        }
    }
}

/// `r` allargato di `d` mm per lato.
pub fn grow(r: Rect, d: f64) -> Rect {
    Rect::new(r.x - d, r.y - d, r.w + 2.0 * d, r.h + 2.0 * d)
}
