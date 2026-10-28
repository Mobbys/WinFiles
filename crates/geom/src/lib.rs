//! Misure, formati carta e rettangoli. Livello L0: nessuna dipendenza interna.
//!
//! Tutte le misure interne sono in **millimetri** (`f64`). Le conversioni verso punti PDF
//! (1/72 di pollice) e pixel avvengono solo ai bordi: lettura dei file e scrittura del PDF.

use serde::{Deserialize, Serialize};

mod calibration;
mod units;
pub use calibration::{BadMeasure, Calibration, MAX_SHIFT, MAX_STRETCH, Measures};
pub use units::{Unit, area_m2, format_area, format_length, format_number, format_size};

/// Millimetri in un pollice.
pub const MM_PER_INCH: f64 = 25.4;
/// Punti PDF in un pollice.
pub const PT_PER_INCH: f64 = 72.0;

/// Converte punti PDF in millimetri.
pub fn pt_to_mm(pt: f64) -> f64 {
    pt * MM_PER_INCH / PT_PER_INCH
}

/// Converte millimetri in punti PDF.
pub fn mm_to_pt(mm: f64) -> f64 {
    mm * PT_PER_INCH / MM_PER_INCH
}

/// Converte pixel in millimetri alla risoluzione data. Una risoluzione non valida (zero,
/// negativa o non finita) restituisce `None`: il chiamante decide il ripiego.
pub fn px_to_mm(px: f64, dpi: f64) -> Option<f64> {
    (dpi.is_finite() && dpi > 0.0).then(|| px * MM_PER_INCH / dpi)
}

/// Pixel necessari per stampare `mm` millimetri alla risoluzione data, arrotondati per eccesso.
pub fn mm_to_px(mm: f64, dpi: f64) -> u32 {
    let px = (mm.max(0.0) * dpi.max(0.0) / MM_PER_INCH).ceil();
    if px.is_finite() {
        px.min(f64::from(u32::MAX)) as u32
    } else {
        0
    }
}

/// Larghezza e altezza in millimetri.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

impl Size {
    pub const fn new(w: f64, h: f64) -> Self {
        Self { w, h }
    }

    /// Rapporto larghezza/altezza; `None` se una delle due misure non è positiva.
    pub fn aspect(self) -> Option<f64> {
        (self.w > 0.0 && self.h > 0.0 && self.w.is_finite() && self.h.is_finite()).then(|| self.w / self.h)
    }

    /// La dimensione più grande con le stesse proporzioni di `self` che sta dentro `bounds`.
    /// Restituisce `None` se `self` non ha proporzioni valide.
    pub fn fit_within(self, bounds: Size) -> Option<Size> {
        let aspect = self.aspect()?;
        let w = bounds.w.max(0.0);
        let h = bounds.h.max(0.0);
        Some(if w / h.max(f64::MIN_POSITIVE) > aspect {
            Size::new(h * aspect, h)
        } else {
            Size::new(w, w / aspect)
        })
    }
}

/// Rettangolo in millimetri, origine in alto a sinistra del foglio, y verso il basso.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(self) -> f64 {
        self.y + self.h
    }

    pub fn size(self) -> Size {
        Size::new(self.w, self.h)
    }

    /// Vero se `other` sta interamente dentro `self`, con una tolleranza di `eps` mm.
    pub fn contains(self, other: Rect, eps: f64) -> bool {
        other.x >= self.x - eps
            && other.y >= self.y - eps
            && other.right() <= self.right() + eps
            && other.bottom() <= self.bottom() + eps
    }

    /// Vero se i due rettangoli si sovrappongono per più di `eps` mm (i bordi a contatto non contano).
    pub fn overlaps(self, other: Rect, eps: f64) -> bool {
        self.x + eps < other.right()
            && other.x + eps < self.right()
            && self.y + eps < other.bottom()
            && other.y + eps < self.bottom()
    }

    /// Rettangolo ristretto di `m` su ogni lato (mai negativo).
    pub fn inset(self, m: Margins) -> Rect {
        Rect::new(
            self.x + m.left,
            self.y + m.top,
            (self.w - m.left - m.right).max(0.0),
            (self.h - m.top - m.bottom).max(0.0),
        )
    }
}

/// Margini in millimetri.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Margins {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl Margins {
    pub const fn uniform(mm: f64) -> Self {
        Self {
            top: mm,
            right: mm,
            bottom: mm,
            left: mm,
        }
    }
}

/// Formati carta supportati.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Paper {
    A4,
    A3,
}

impl Paper {
    /// Dimensione in verticale, in millimetri (ISO 216).
    pub const fn portrait(self) -> Size {
        match self {
            Paper::A4 => Size::new(210.0, 297.0),
            Paper::A3 => Size::new(297.0, 420.0),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Paper::A4 => "A4",
            Paper::A3 => "A3",
        }
    }
}

/// Orientamento del foglio.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Orientation {
    Portrait,
    Landscape,
}

/// Dimensione del foglio nell'orientamento scelto.
pub fn sheet_size(paper: Paper, orientation: Orientation) -> Size {
    let p = paper.portrait();
    match orientation {
        Orientation::Portrait => p,
        Orientation::Landscape => Size::new(p.h, p.w),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn formati_iso() {
        assert_eq!(
            sheet_size(Paper::A4, Orientation::Portrait),
            Size::new(210.0, 297.0)
        );
        assert_eq!(
            sheet_size(Paper::A3, Orientation::Landscape),
            Size::new(420.0, 297.0)
        );
    }

    #[test]
    fn conversioni() {
        assert!((pt_to_mm(72.0) - 25.4).abs() < 1e-9);
        assert!((mm_to_pt(210.0) - 595.275_590_551).abs() < 1e-6);
        assert_eq!(px_to_mm(300.0, 0.0), None);
        // Una miniatura di 60 mm a 300 dpi richiede 709 px (708,66 arrotondato per eccesso).
        assert_eq!(mm_to_px(60.0, 300.0), 709);
        assert_eq!(mm_to_px(f64::NAN, 300.0), 0);
    }

    #[test]
    fn fit_mantiene_le_proporzioni() {
        let fitted = Size::new(400.0, 200.0).fit_within(Size::new(50.0, 50.0));
        assert_eq!(fitted, Some(Size::new(50.0, 25.0)));
        assert_eq!(Size::new(0.0, 10.0).fit_within(Size::new(50.0, 50.0)), None);
    }

    proptest! {
        #[test]
        fn fit_sta_dentro_e_conserva_il_rapporto(
            w in 0.01f64..10_000.0, h in 0.01f64..10_000.0,
            bw in 0.0f64..500.0, bh in 0.0f64..500.0,
        ) {
            let fitted = Size::new(w, h).fit_within(Size::new(bw, bh));
            let fitted = fitted.ok_or_else(|| TestCaseError::fail("proporzioni valide"))?;
            prop_assert!(fitted.w <= bw + 1e-9 && fitted.h <= bh + 1e-9);
            if fitted.w > 1e-6 && fitted.h > 1e-6 {
                prop_assert!(((fitted.w / fitted.h) - (w / h)).abs() / (w / h) < 1e-6);
            }
        }
    }
}
