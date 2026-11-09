//! Le annotazioni disegnate sopra una miniatura: testi, linee e frecce, rettangoli, ovali, tratti
//! dell'evidenziatore e quote. Ognuna è attaccata a una pagina di un file e ha i punti in frazioni
//! della parte mostrata della pagina (0 = bordo sinistro o alto, 1 = bordo destro o basso): così
//! segue la miniatura quando cambiano colonne, foglio o ordine dei file. Spessori e corpi sono in
//! millimetri sulla carta, uguali a ogni grandezza della miniatura.

use serde::{Deserialize, Serialize};

/// Punti al più di un tratto dell'evidenziatore: oltre, il file è sicuramente rovinato.
pub const MAX_MARKER_POINTS: usize = 4000;
/// Annotazioni al più per pagina.
pub const MAX_PER_PAGE: usize = 500;
/// Lettere al più di un testo.
pub const MAX_TEXT_CHARS: usize = 4000;
/// Quanto lontano dalla miniatura può stare un punto, in frazioni della miniatura: anche una
/// miniatura piccola deve poter raggiungere ogni angolo del foglio.
const REACH: f64 = 100.0;

/// Un colore RGB con opacità.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Lo stesso colore con l'opacità `alpha` (0..1).
    pub fn with_alpha(self, alpha: f64) -> Self {
        let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self { a, ..self }
    }
}

/// Un punto in frazioni della miniatura.
pub type Point = (f64, f64);

/// Contorno e riempimento di una forma.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShapeStyle {
    /// Colore e spessore del contorno, in mm; `None` = senza contorno.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<(Rgba, f64)>,
    /// Colore del riempimento (con la sua trasparenza); `None` = vuota.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Rgba>,
}

/// Un'annotazione.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "lowercase")]
pub enum Annotation {
    /// Una linea, con la punta di freccia a un capo, all'altro o a tutti e due.
    Linea {
        a: Point,
        b: Point,
        color: Rgba,
        width: f64,
        #[serde(default)]
        start_arrow: bool,
        #[serde(default)]
        end_arrow: bool,
        /// Il capo `b` attaccato a un'altra miniatura, per una linea a cavallo di due: segue
        /// quella. Se quella miniatura non è sullo stesso foglio vale `b`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<Anchor>,
    },
    /// Un rettangolo tra due angoli opposti.
    Rettangolo {
        a: Point,
        b: Point,
        style: ShapeStyle,
        #[serde(default)]
        rounded: bool,
    },
    /// Un ovale (un cerchio, se il riquadro è quadrato sulla carta) dentro il riquadro.
    Ovale { a: Point, b: Point, style: ShapeStyle },
    /// Un testo con l'angolo in alto a sinistra in `at`; con `width` (mm) va a capo.
    Testo {
        at: Point,
        text: String,
        color: Rgba,
        /// Corpo in mm.
        size: f64,
        #[serde(default)]
        bold: bool,
        /// Un riquadro bianco sotto il testo, per leggerlo sopra un'immagine scura.
        #[serde(default)]
        background: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<f64>,
    },
    /// Un tratto a mano libera dell'evidenziatore, largo e trasparente.
    Evidenziatore {
        points: Vec<Point>,
        color: Rgba,
        width: f64,
    },
    /// Una quota: una linea con le stanghette e la misura vera della pagina scritta sopra.
    Quota {
        a: Point,
        b: Point,
        color: Rgba,
        width: f64,
        /// La misura scritta a mano, in mm: questa quota è la master e le altre della stessa
        /// pagina si leggono in proporzione.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<f64>,
        /// Corpo della misura scritta, in mm.
        #[serde(default = "quote_size")]
        size: f64,
    },
}

/// Il corpo di partenza della misura delle quote, in mm.
pub const QUOTE_SIZE: f64 = 2.8;

fn quote_size() -> f64 {
    QUOTE_SIZE
}

/// Un punto attaccato a un'altra miniatura: il file (per percorso, che non cambia riordinando),
/// la pagina e il punto in frazioni di quella miniatura.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub path: std::path::PathBuf,
    pub page: usize,
    pub at: Point,
}

impl Annotation {
    /// Tutti i punti, per spostarla o per sapere dove sta.
    pub fn points(&self) -> Vec<Point> {
        match self {
            Annotation::Linea { a, b, .. }
            | Annotation::Rettangolo { a, b, .. }
            | Annotation::Ovale { a, b, .. }
            | Annotation::Quota { a, b, .. } => vec![*a, *b],
            Annotation::Testo { at, .. } => vec![*at],
            Annotation::Evidenziatore { points, .. } => points.clone(),
        }
    }

    /// La stessa annotazione spostata di `(dx, dy)` in frazioni della miniatura.
    pub fn moved(&self, dx: f64, dy: f64) -> Self {
        let m = |p: &Point| (p.0 + dx, p.1 + dy);
        let mut out = self.clone();
        match &mut out {
            Annotation::Linea { a, b, .. }
            | Annotation::Rettangolo { a, b, .. }
            | Annotation::Ovale { a, b, .. }
            | Annotation::Quota { a, b, .. } => {
                *a = m(a);
                *b = m(b);
            }
            Annotation::Testo { at, .. } => *at = m(at),
            Annotation::Evidenziatore { points, .. } => {
                for p in points.iter_mut() {
                    *p = m(p);
                }
            }
        }
        out
    }

    /// Riporta numeri assurdi letti da un file a valori che non fanno danni; `false` se non
    /// resta niente da disegnare.
    pub fn sanitize(&mut self) -> bool {
        let point = |p: &mut Point| {
            p.0 = clean(p.0, 0.5).clamp(-REACH, 1.0 + REACH);
            p.1 = clean(p.1, 0.5).clamp(-REACH, 1.0 + REACH);
        };
        let width = |w: &mut f64| *w = clean(*w, 0.5).clamp(0.05, 20.0);
        match self {
            Annotation::Linea {
                a, b, width: w, end, ..
            } => {
                point(a);
                point(b);
                width(w);
                if let Some(e) = end {
                    point(&mut e.at);
                }
            }
            Annotation::Quota {
                a,
                b,
                width: w,
                value,
                size,
                ..
            } => {
                point(a);
                point(b);
                width(w);
                *size = clean(*size, QUOTE_SIZE).clamp(1.0, 40.0);
                *value = value.filter(|v| v.is_finite() && *v > 0.0 && *v <= 1_000_000.0);
            }
            Annotation::Rettangolo { a, b, style, .. } | Annotation::Ovale { a, b, style } => {
                point(a);
                point(b);
                if let Some((_, w)) = &mut style.stroke {
                    width(w);
                }
                return style.stroke.is_some() || style.fill.is_some();
            }
            Annotation::Testo {
                at,
                text,
                size,
                width: w,
                ..
            } => {
                point(at);
                *size = clean(*size, 4.0).clamp(0.5, 100.0);
                if let Some(w) = w {
                    *w = clean(*w, 40.0).clamp(2.0, 2000.0);
                }
                if text.chars().count() > MAX_TEXT_CHARS {
                    *text = text.chars().take(MAX_TEXT_CHARS).collect();
                }
                return !text.trim().is_empty();
            }
            Annotation::Evidenziatore { points, width: w, .. } => {
                points.truncate(MAX_MARKER_POINTS);
                points.iter_mut().for_each(point);
                width(w);
                return points.len() >= 2;
            }
        }
        true
    }
}

fn clean(v: f64, fallback: f64) -> f64 {
    if v.is_finite() { v } else { fallback }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i_numeri_assurdi_si_riportano_a_posto() {
        let mut a = Annotation::Linea {
            a: (f64::NAN, 1e9),
            b: (0.5, 0.5),
            color: Rgba::rgb(255, 0, 0),
            width: f64::INFINITY,
            start_arrow: false,
            end_arrow: true,
            end: None,
        };
        assert!(a.sanitize());
        let Annotation::Linea { a: p, width, .. } = a else {
            panic!("tipo cambiato");
        };
        assert_eq!(p, (0.5, 101.0));
        assert_eq!(width, 0.5);
    }

    #[test]
    fn un_testo_vuoto_o_un_tratto_di_un_punto_si_tolgono() {
        let mut t = Annotation::Testo {
            at: (0.1, 0.1),
            text: "  ".into(),
            color: Rgba::rgb(0, 0, 0),
            size: 4.0,
            bold: false,
            background: false,
            width: None,
        };
        assert!(!t.sanitize());
        let mut m = Annotation::Evidenziatore {
            points: vec![(0.1, 0.1)],
            color: Rgba::rgb(255, 230, 0),
            width: 4.0,
        };
        assert!(!m.sanitize());
    }

    #[test]
    fn spostata_si_muove_tutta() {
        let r = Annotation::Ovale {
            a: (0.1, 0.2),
            b: (0.3, 0.4),
            style: ShapeStyle {
                stroke: None,
                fill: Some(Rgba::rgb(1, 2, 3)),
            },
        };
        let m = r.moved(0.1, -0.1);
        let pts = m.points();
        assert!((pts[0].0 - 0.2).abs() < 1e-9 && (pts[1].1 - 0.3).abs() < 1e-9);
    }
}
