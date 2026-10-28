//! La pagina di calibrazione: dove vanno la cornice, le due linee lunghe da misurare, il quadrato
//! di controllo, le tacche dei margini e il testo con le istruzioni. Le misure da controllare sono
//! numeri tondi, così si leggono sul righello senza conti.

use winfiles_geom::{Measures, Paper, Rect, Size};

/// Distanza della cornice dai bordi del foglio, in mm.
pub const INSET: f64 = 10.0;
/// Fin dove arrivano le tacche dei margini, in mm dal bordo.
pub const EDGE_MARKS: u32 = 10;
/// Ascissa della linea verticale, in mm dal bordo sinistro.
const VERTICAL_X: f64 = 22.0;
/// Il testo e il resto stanno a destra della linea verticale, da qui.
const CONTENT_X: f64 = 34.0;
/// Altezza del riquadro del testo, in mm.
const TEXT_H: f64 = 78.0;

/// Un lato del foglio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

/// Una linea da misurare: parte da `from` e va verso destra o verso il basso.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ruler {
    pub from: (f64, f64),
    /// Lunghezza in mm, sempre un numero tondo.
    pub length: f64,
    pub vertical: bool,
}

impl Ruler {
    pub fn to(&self) -> (f64, f64) {
        if self.vertical {
            (self.from.0, self.from.1 + self.length)
        } else {
            (self.from.0 + self.length, self.from.1)
        }
    }
}

/// Le tacche di un lato: una ogni millimetro dal bordo fino a [`EDGE_MARKS`], centrate in `at`
/// lungo il lato. La prima che si vede stampata dice quanto margine la stampante lascia bianco.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeMarks {
    pub side: Side,
    pub at: f64,
}

/// La pagina di calibrazione impaginata.
#[derive(Clone, Debug, PartialEq)]
pub struct CalibrationSheet {
    pub paper: Paper,
    pub size: Size,
    /// La cornice, a [`INSET`] mm da ogni bordo.
    pub frame: Rect,
    pub horizontal: Ruler,
    pub vertical: Ruler,
    /// Il quadrato di controllo: deve venire con i lati uguali.
    pub square: Rect,
    /// Dove scrivere titolo e istruzioni.
    pub text: Rect,
    pub edges: [EdgeMarks; 4],
}

/// Il numero tondo (multiplo di `step`) più grande che non supera `v`.
fn round_down(v: f64, step: f64) -> f64 {
    (v / step).floor() * step
}

impl CalibrationSheet {
    /// La pagina per un foglio verticale del formato `paper`.
    pub fn new(paper: Paper) -> Self {
        let size = paper.portrait();
        let (w, h) = (size.w, size.h);
        let frame = Rect::new(INSET, INSET, w - 2.0 * INSET, h - 2.0 * INSET);
        // La linea verticale il più lunga possibile lasciando spazio alle tacche dei margini.
        let v_len = round_down(h - 5.0 * INSET, 10.0);
        let vertical = Ruler {
            from: (VERTICAL_X, (h - v_len) / 2.0),
            length: v_len,
            vertical: true,
        };
        let right = w - INSET - 4.0;
        let text = Rect::new(CONTENT_X, INSET + 8.0, right - CONTENT_X, TEXT_H);
        // Quella orizzontale a destra della verticale, senza incrociarla.
        let h_len = round_down(right - CONTENT_X, 10.0);
        let horizontal = Ruler {
            from: (
                CONTENT_X + (right - CONTENT_X - h_len) / 2.0,
                text.bottom() + 12.0,
            ),
            length: h_len,
            vertical: false,
        };
        // Il quadrato più grande, a passi di 5 cm, nello spazio sotto la linea orizzontale.
        let top = horizontal.from.1 + 22.0;
        let room = (right - CONTENT_X).min(vertical.to().1 - top);
        let side = round_down(room, 50.0).clamp(50.0, 200.0);
        let square = Rect::new(CONTENT_X + (right - CONTENT_X - side) / 2.0, top, side, side);
        let edges = [
            EdgeMarks {
                side: Side::Top,
                at: w / 2.0,
            },
            EdgeMarks {
                side: Side::Right,
                at: h / 2.0,
            },
            EdgeMarks {
                side: Side::Bottom,
                at: w / 2.0,
            },
            EdgeMarks {
                side: Side::Left,
                at: h / 2.0,
            },
        ];
        Self {
            paper,
            size,
            frame,
            horizontal,
            vertical,
            square,
            text,
            edges,
        }
    }

    /// Le misure che una stampante esatta dà su questa pagina.
    pub fn expected(&self) -> Measures {
        Measures {
            width: self.horizontal.length,
            height: self.vertical.length,
            left: INSET,
            top: INSET,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_misure_sono_tonde_e_tutto_sta_nella_cornice() {
        for paper in [Paper::A4, Paper::A3] {
            let s = CalibrationSheet::new(paper);
            let inner = Rect::new(s.frame.x + 1.0, s.frame.y + 1.0, s.frame.w - 2.0, s.frame.h - 2.0);
            for ruler in [s.horizontal, s.vertical] {
                assert_eq!(ruler.length % 10.0, 0.0, "{paper:?}");
                let (a, b) = (ruler.from, ruler.to());
                assert!(
                    inner.contains(Rect::new(a.0, a.1, b.0 - a.0, b.1 - a.1), 1e-9),
                    "{paper:?}"
                );
            }
            assert!(inner.contains(s.square, 1e-9), "{paper:?}");
            assert!(inner.contains(s.text, 1e-9), "{paper:?}");
            assert_eq!(s.square.w, s.square.h);
            // Nulla si tocca: il testo sopra la linea orizzontale, il quadrato sotto con lo spazio
            // per i numeri, tutto a destra della linea verticale con le sue tacche.
            assert!(s.text.bottom() + 8.0 <= s.horizontal.from.1);
            assert!(s.horizontal.from.1 + 15.0 <= s.square.y);
            assert!(s.vertical.from.0 + 10.0 <= s.text.x.min(s.square.x).min(s.horizontal.from.0));
            // Le tacche dei margini stanno fuori dalla cornice e lontano dalle linee.
            assert!(s.vertical.from.1 > INSET + 5.0 && s.vertical.to().1 < s.size.h - INSET - 5.0);
            assert_eq!(s.expected().left, INSET);
        }
        let a4 = CalibrationSheet::new(Paper::A4);
        assert_eq!((a4.horizontal.length, a4.vertical.length), (160.0, 240.0));
        assert_eq!(a4.square.w, 100.0);
        let a3 = CalibrationSheet::new(Paper::A3);
        assert_eq!((a3.horizontal.length, a3.vertical.length), (240.0, 370.0));
        assert_eq!(a3.square.w, 200.0);
    }
}
