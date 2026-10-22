//! Muoversi nell'anteprima ingrandita di Modifica PDF e Modifica immagini: Ctrl+rotella ingrandisce
//! verso il puntatore (il punto sotto il mouse resta fermo) e la barra spaziatrice tenuta premuta,
//! o il tasto centrale del mouse, fa da mano per trascinare la vista, come nei programmi di grafica.

use egui::{Pos2, Rect, Vec2};

/// Lo spostamento della vista se la mano è attiva: barra spaziatrice (senza un campo che scrive)
/// o tasto centrale, col puntatore sull'anteprima. `Some(ZERO)` = mano pronta ma ferma; mentre è
/// attiva gli strumenti (quote, taglierina) non devono reagire al trascinamento.
pub fn hand(ui: &egui::Ui, board: Rect) -> Option<Vec2> {
    let typing = ui.ctx().egui_wants_keyboard_input();
    let (space, middle, primary, delta) = ui.input(|i| {
        (
            i.key_down(egui::Key::Space),
            i.pointer.middle_down(),
            i.pointer.primary_down(),
            i.pointer.delta(),
        )
    });
    let held = (space && !typing) || middle;
    if !held || !ui.rect_contains_pointer(board) {
        return None;
    }
    let dragging = middle || primary;
    ui.ctx().set_cursor_icon(if dragging {
        egui::CursorIcon::Grabbing
    } else {
        egui::CursorIcon::Grab
    });
    Some(if dragging { delta } else { Vec2::ZERO })
}

/// Il punto della pagina da tenere sotto il puntatore durante un'ingrandita con la rotella. Si
/// ricorda quello del primo scatto finché il gesto continua (stesso punto, scatti ravvicinati):
/// finché la pagina sta nella finestra resta al centro e il punto scivola, ma appena è più grande
/// della finestra torna esattamente sotto il puntatore.
#[derive(Default)]
pub struct Anchor {
    held: Option<(Pos2, Vec2, f64)>,
}

impl Anchor {
    /// `current` dà il punto della pagina sotto `pointer` adesso, in unità della pagina.
    pub fn point(&mut self, pointer: Pos2, now: f64, current: impl FnOnce() -> Vec2) -> Vec2 {
        let point = match self.held {
            Some((at, point, when)) if (at - pointer).length() < 3.0 && now - when < 0.6 => point,
            _ => current(),
        };
        self.held = Some((pointer, point, now));
        point
    }
}

/// Il limite dello scostamento: una pagina più piccola della finestra resta al centro, una più
/// grande si sposta fino ai suoi bordi e un margine di `margin` punti.
pub fn clamp(pan: Vec2, shown: Vec2, board: Vec2, margin: f32) -> Vec2 {
    let limit = ((shown - board) / 2.0 + Vec2::splat(margin)).max(Vec2::ZERO);
    let limit = Vec2::new(
        if shown.x + 2.0 * margin > board.x {
            limit.x
        } else {
            0.0
        },
        if shown.y + 2.0 * margin > board.y {
            limit.y
        } else {
            0.0
        },
    );
    pan.clamp(-limit, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    #[test]
    fn il_punto_resta_quello_del_primo_scatto() {
        let mut a = Anchor::default();
        let p = pos2(100.0, 100.0);
        assert_eq!(a.point(p, 0.0, || vec2(1.0, 2.0)), vec2(1.0, 2.0));
        // Stesso gesto: il punto calcolato di nuovo (spostato dalla pagina al centro) non conta.
        assert_eq!(a.point(p, 0.2, || vec2(9.0, 9.0)), vec2(1.0, 2.0));
        // Fermi troppo a lungo, o il mouse altrove: si riparte da dove si è.
        assert_eq!(a.point(p, 2.0, || vec2(5.0, 5.0)), vec2(5.0, 5.0));
        assert_eq!(
            a.point(pos2(300.0, 100.0), 2.1, || vec2(7.0, 7.0)),
            vec2(7.0, 7.0)
        );
    }

    #[test]
    fn una_pagina_che_ci_sta_resta_al_centro() {
        let board = vec2(800.0, 600.0);
        assert_eq!(
            clamp(vec2(300.0, -200.0), vec2(400.0, 300.0), board, 40.0),
            Vec2::ZERO
        );
        // Più grande: si sposta solo fino al bordo più il margine.
        let big = clamp(vec2(5000.0, -5000.0), vec2(2000.0, 1000.0), board, 40.0);
        assert_eq!(big, vec2(640.0, -240.0));
    }
}
