//! La taglierina: il rettangolo che si disegna sull'anteprima, si sposta e si tira per i lati e
//! gli angoli, come nella 4.9. La usano il modulo immagini (in pixel) e il modulo PDF (in
//! millimetri): qui le misure sono nell'unità di chi la usa, larga `w` e alta `h`.

use egui::{Rect, Sense, Stroke, StrokeKind, Vec2, pos2};

use crate::theme::Tokens;

/// Quanto vicino bisogna essere a una maniglia perché risponda, in punti.
const GRIP: f32 = 9.0;

/// Come si sta trascinando il rettangolo.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Grab {
    /// Si sta disegnando un rettangolo nuovo.
    Draw,
    /// Si sposta tutto il rettangolo.
    Move,
    /// Si tira un lato o un angolo: sinistra, destra, sopra, sotto (uno o due veri).
    Edge {
        left: bool,
        right: bool,
        top: bool,
        bottom: bool,
    },
}

/// Il rettangolo da tenere, con gli angoli in qualsiasi ordine finché si trascina.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crop {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Crop {
    pub fn normalized(self) -> Self {
        Self {
            x0: self.x0.min(self.x1),
            y0: self.y0.min(self.y1),
            x1: self.x0.max(self.x1),
            y1: self.y0.max(self.y1),
        }
    }

    pub fn width(self) -> f32 {
        (self.x1 - self.x0).abs()
    }

    pub fn height(self) -> f32 {
        (self.y1 - self.y0).abs()
    }
}

/// Lo stato della taglierina su un'anteprima.
#[derive(Default)]
pub struct Cutter {
    /// Il rettangolo, se c'è.
    pub crop: Option<Crop>,
    grab: Option<(Grab, egui::Pos2, Crop)>,
    /// Dove tenere la lente (a schermo), mentre si trascina.
    pub lens: Option<egui::Pos2>,
}

impl Cutter {
    /// Toglie il rettangolo.
    pub fn clear(&mut self) {
        self.crop = None;
        self.grab = None;
        self.lens = None;
    }

    /// Il rettangolo si disegna, si sposta e si tira per i lati su `paper`, l'anteprima di una
    /// pagina o immagine larga `w` e alta `h` nell'unità di chi chiama.
    pub fn interact(&mut self, ui: &mut egui::Ui, id: egui::Id, paper: Rect, w: f32, h: f32) {
        // Senza prendere la tastiera: Invio ed Esc restano alla taglierina.
        let resp = ui.interact(paper, id, Sense::CLICK | Sense::DRAG);
        let scale = paper.width() / w.max(f32::EPSILON);
        let to_unit = |p: egui::Pos2| -> egui::Pos2 {
            pos2(
                ((p.x - paper.left()) / scale).clamp(0.0, w),
                ((p.y - paper.top()) / scale).clamp(0.0, h),
            )
        };
        if resp.drag_started() || resp.clicked() {
            // Chi aveva la tastiera (una casella appena cliccata) la lascia: Invio deve tagliare,
            // non spuntare di nuovo la casella.
            ui.memory_mut(|m| {
                if let Some(f) = m.focused() {
                    m.surrender_focus(f);
                }
            });
        }
        if resp.drag_started() {
            // Dove il pulsante è stato premuto, non dove si trova ora: il primo movimento può
            // essere già lontano, e si deve capire che cosa si è afferrato.
            let origin = ui
                .ctx()
                .input(|i| i.pointer.press_origin())
                .or_else(|| resp.interact_pointer_pos());
            if let Some(pos) = origin {
                let at = to_unit(pos);
                let grab = self.hit(at, scale);
                let here = Crop {
                    x0: at.x,
                    y0: at.y,
                    x1: at.x,
                    y1: at.y,
                };
                // Disegnando un rettangolo nuovo si parte da dove si è premuto, anche se ce
                // n'era già uno.
                let start = match grab {
                    Grab::Draw => {
                        self.crop = Some(here);
                        here
                    }
                    _ => self.crop.unwrap_or(here),
                };
                self.grab = Some((grab, at, start));
            }
        }
        if let (true, Some((grab, from, start)), Some(pos)) =
            (resp.dragged(), self.grab, resp.interact_pointer_pos())
        {
            let at = to_unit(pos);
            let (dx, dy) = (at.x - from.x, at.y - from.y);
            let c = match grab {
                Grab::Draw => Crop {
                    x0: start.x0,
                    y0: start.y0,
                    x1: at.x,
                    y1: at.y,
                },
                Grab::Move => {
                    let n = start.normalized();
                    let dx = dx.clamp(-n.x0, w - n.x1);
                    let dy = dy.clamp(-n.y0, h - n.y1);
                    Crop {
                        x0: n.x0 + dx,
                        y0: n.y0 + dy,
                        x1: n.x1 + dx,
                        y1: n.y1 + dy,
                    }
                }
                Grab::Edge {
                    left,
                    right,
                    top,
                    bottom,
                } => {
                    let n = start.normalized();
                    Crop {
                        x0: if left { at.x } else { n.x0 },
                        x1: if right { at.x } else { n.x1 },
                        y0: if top { at.y } else { n.y0 },
                        y1: if bottom { at.y } else { n.y1 },
                    }
                }
            };
            self.crop = Some(c);
            self.lens = resp.interact_pointer_pos();
        }
        if resp.drag_stopped() {
            self.grab = None;
            self.lens = None;
            self.crop = self.crop.map(Crop::normalized).filter(|c| {
                // Un clic senza trascinare toglie il rettangolo invece di lasciarne uno vuoto.
                c.width() * scale > 4.0 && c.height() * scale > 4.0
            });
        }
        // Il cursore dice cosa si sta per fare.
        if let Some(pos) = ui.ctx().pointer_latest_pos().filter(|p| paper.contains(*p)) {
            let cursor = match self.hit(to_unit(pos), scale) {
                Grab::Draw => egui::CursorIcon::Crosshair,
                Grab::Move => egui::CursorIcon::Move,
                Grab::Edge { left, right, .. } if !left && !right => egui::CursorIcon::ResizeVertical,
                Grab::Edge { top, bottom, .. } if !top && !bottom => egui::CursorIcon::ResizeHorizontal,
                Grab::Edge { .. } => egui::CursorIcon::ResizeNwSe,
            };
            ui.ctx().set_cursor_icon(cursor);
        }
    }

    /// Che cosa c'è sotto il cursore: un lato, un angolo, il mezzo o niente.
    fn hit(&self, at: egui::Pos2, scale: f32) -> Grab {
        let Some(c) = self.crop.map(Crop::normalized) else {
            return Grab::Draw;
        };
        let near = GRIP / scale.max(0.0001);
        let left = (at.x - c.x0).abs() <= near;
        let right = (at.x - c.x1).abs() <= near;
        let top = (at.y - c.y0).abs() <= near;
        let bottom = (at.y - c.y1).abs() <= near;
        let inside_x = at.x >= c.x0 - near && at.x <= c.x1 + near;
        let inside_y = at.y >= c.y0 - near && at.y <= c.y1 + near;
        if (left || right) && inside_y || (top || bottom) && inside_x {
            return Grab::Edge {
                left: left && inside_y,
                right: right && inside_y,
                top: top && inside_x,
                bottom: bottom && inside_x,
            };
        }
        if at.x > c.x0 && at.x < c.x1 && at.y > c.y0 && at.y < c.y1 {
            return Grab::Move;
        }
        Grab::Draw
    }

    /// Il rettangolo con le maniglie, e fuori l'anteprima scurita: si vede subito cosa resta.
    pub fn paint(&self, ui: &egui::Ui, paper: Rect, w: f32) {
        let Some(c) = self.crop.map(Crop::normalized) else {
            return;
        };
        let t = Tokens::get(ui.ctx());
        let scale = paper.width() / w.max(f32::EPSILON);
        let r = Rect::from_min_max(
            pos2(paper.left() + c.x0 * scale, paper.top() + c.y0 * scale),
            pos2(paper.left() + c.x1 * scale, paper.top() + c.y1 * scale),
        );
        let p = ui.painter().with_clip_rect(paper);
        let shade = egui::Color32::from_black_alpha(110);
        p.rect_filled(
            Rect::from_min_max(paper.min, pos2(paper.right(), r.top())),
            0.0,
            shade,
        );
        p.rect_filled(
            Rect::from_min_max(pos2(paper.left(), r.bottom()), paper.max),
            0.0,
            shade,
        );
        p.rect_filled(
            Rect::from_min_max(pos2(paper.left(), r.top()), pos2(r.left(), r.bottom())),
            0.0,
            shade,
        );
        p.rect_filled(
            Rect::from_min_max(pos2(r.right(), r.top()), pos2(paper.right(), r.bottom())),
            0.0,
            shade,
        );
        p.rect_stroke(r, 0.0, Stroke::new(1.5, t.accent), StrokeKind::Middle);
        // Le maniglie: quattro angoli e quattro lati.
        let handles = [
            r.left_top(),
            r.center_top(),
            r.right_top(),
            r.right_center(),
            r.right_bottom(),
            r.center_bottom(),
            r.left_bottom(),
            r.left_center(),
        ];
        for h in handles {
            p.rect(
                Rect::from_center_size(h, Vec2::splat(8.0)),
                1.0,
                t.accent,
                Stroke::new(1.0, egui::Color32::WHITE),
                StrokeKind::Middle,
            );
        }
    }
}
