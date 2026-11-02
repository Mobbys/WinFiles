//! La correzione di una stampante: quanto allungare o accorciare il foglio e di quanto spostarlo
//! perché le misure stampate tornino esatte. Si ricava dalla pagina di calibrazione stampata e
//! misurata con un righello.
//!
//! Il modello è lo stesso per i due assi: la stampante mette in `s · p + o` ciò che le si manda in
//! `p` (millimetri dal bordo in alto a sinistra del foglio). La correzione manda `scale · p +
//! offset`, così che stampato finisca in `p`.

use serde::{Deserialize, Serialize};

/// Allungamento massimo che si accetta come correzione: oltre, la misura è sbagliata o la
/// stampante sta riducendo il foglio di suo ("Adatta alla pagina" nel driver).
pub const MAX_STRETCH: f64 = 0.05;
/// Spostamento massimo accettato, in mm.
pub const MAX_SHIFT: f64 = 15.0;

/// La correzione per una stampante, per asse.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Calibration {
    pub scale_x: f64,
    pub scale_y: f64,
    /// Spostamento in mm, positivo verso destra.
    pub offset_x: f64,
    /// Spostamento in mm, positivo verso il basso.
    pub offset_y: f64,
}

impl Default for Calibration {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Le quattro misure della pagina di calibrazione, in mm: le due linee lunghe e le distanze dai
/// bordi sinistro e superiore del foglio alla cornice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measures {
    pub width: f64,
    pub height: f64,
    pub left: f64,
    pub top: f64,
}

/// Quale misura non si può usare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadMeasure {
    Width,
    Height,
    Left,
    Top,
}

impl std::fmt::Display for BadMeasure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            BadMeasure::Width => "la linea orizzontale è troppo diversa dal previsto: ricontrolla la misura",
            BadMeasure::Height => "la linea verticale è troppo diversa dal previsto: ricontrolla la misura",
            BadMeasure::Left => "la distanza dal bordo sinistro è troppo diversa dal previsto",
            BadMeasure::Top => "la distanza dal bordo superiore è troppo diversa dal previsto",
        })
    }
}

impl Calibration {
    /// Nessuna correzione.
    pub const IDENTITY: Calibration = Calibration {
        scale_x: 1.0,
        scale_y: 1.0,
        offset_x: 0.0,
        offset_y: 0.0,
    };

    /// Vero se non cambia nulla, a meno di un millesimo di millimetro su un A3.
    pub fn is_identity(&self) -> bool {
        (self.scale_x - 1.0).abs() < 2e-6
            && (self.scale_y - 1.0).abs() < 2e-6
            && self.offset_x.abs() < 1e-3
            && self.offset_y.abs() < 1e-3
    }

    /// Dove mandare il punto `p` (mm) perché la stampante lo stampi in `p`.
    pub fn apply(&self, p: (f64, f64)) -> (f64, f64) {
        (
            self.scale_x * p.0 + self.offset_x,
            self.scale_y * p.1 + self.offset_y,
        )
    }

    /// La correzione dentro i limiti, o nessuna se i numeri letti sono assurdi.
    pub fn sanitized(self) -> Self {
        let ok = [self.scale_x, self.scale_y, self.offset_x, self.offset_y]
            .iter()
            .all(|v| v.is_finite())
            && ((1.0 - MAX_STRETCH)..=(1.0 + MAX_STRETCH)).contains(&self.scale_x)
            && ((1.0 - MAX_STRETCH)..=(1.0 + MAX_STRETCH)).contains(&self.scale_y)
            && self.offset_x.abs() <= MAX_SHIFT
            && self.offset_y.abs() <= MAX_SHIFT;
        if ok { self } else { Self::IDENTITY }
    }

    /// La correzione nuova da ciò che si è misurato (`measured`) sulla pagina stampata con la
    /// correzione `used`, che doveva dare `expected`.
    pub fn from_measures(
        used: Calibration,
        expected: Measures,
        measured: Measures,
    ) -> Result<Calibration, BadMeasure> {
        let x = axis(
            (used.scale_x, used.offset_x),
            (expected.width, expected.left),
            (measured.width, measured.left),
        );
        let y = axis(
            (used.scale_y, used.offset_y),
            (expected.height, expected.top),
            (measured.height, measured.top),
        );
        let (scale_x, offset_x) =
            x.map_err(|long| if long { BadMeasure::Width } else { BadMeasure::Left })?;
        let (scale_y, offset_y) =
            y.map_err(|long| if long { BadMeasure::Height } else { BadMeasure::Top })?;
        Ok(Calibration {
            scale_x,
            scale_y,
            offset_x,
            offset_y,
        })
    }
}

/// Un asse: con la correzione `(a, b)` la linea lunga `length` è venuta `measured.0` e il bordo
/// della cornice, a `edge` dal bordo del foglio, a `measured.1`. L'errore dice quale misura non
/// va: vero per la linea lunga.
fn axis(used: (f64, f64), expected: (f64, f64), measured: (f64, f64)) -> Result<(f64, f64), bool> {
    let (a, b) = used;
    let (length, edge) = expected;
    let (m_length, m_edge) = measured;
    let usable = |v: f64| v.is_finite() && v > 0.0;
    if !(usable(a) && usable(length) && usable(m_length)) {
        return Err(true);
    }
    // La stampante allunga di `s` e sposta di `o`.
    let s = m_length / (a * length);
    let new_scale = 1.0 / s;
    if (new_scale - 1.0).abs() > MAX_STRETCH {
        return Err(true);
    }
    if !(m_edge.is_finite() && m_edge >= 0.0) {
        return Err(false);
    }
    let o = m_edge - s * (a * edge + b);
    let new_offset = -o / s;
    if new_offset.abs() > MAX_SHIFT {
        return Err(false);
    }
    Ok((new_scale, new_offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED: Measures = Measures {
        width: 180.0,
        height: 240.0,
        left: 10.0,
        top: 10.0,
    };

    /// Dove la stampante `(s, o)` per asse mette ciò che riceve con la correzione `c`.
    fn printed(c: &Calibration, dev: (f64, f64, f64, f64), p: (f64, f64)) -> (f64, f64) {
        let q = c.apply(p);
        (dev.0 * q.0 + dev.2, dev.1 * q.1 + dev.3)
    }

    #[test]
    fn la_correzione_rimette_a_posto_le_misure() {
        // Una stampante che allunga dell'1% in larghezza, accorcia dello 0,5% in altezza e sposta
        // di 0,8 mm a destra e 1,2 mm in su.
        let dev = (1.01, 0.995, 0.8, -1.2);
        let id = Calibration::IDENTITY;
        let measured = Measures {
            width: 180.0 * 1.01,
            height: 240.0 * 0.995,
            left: printed(&id, dev, (10.0, 10.0)).0,
            top: printed(&id, dev, (10.0, 10.0)).1,
        };
        let c = Calibration::from_measures(id, EXPECTED, measured).unwrap();
        for p in [(0.0, 0.0), (10.0, 10.0), (105.0, 148.5), (210.0, 297.0)] {
            let got = printed(&c, dev, p);
            assert!(
                (got.0 - p.0).abs() < 1e-9 && (got.1 - p.1).abs() < 1e-9,
                "{p:?} → {got:?}"
            );
        }
        // Rimisurata con la correzione, la pagina dà le misure giuste e la correzione non cambia.
        let again = Measures {
            width: printed(&c, dev, (190.0, 0.0)).0 - printed(&c, dev, (10.0, 0.0)).0,
            height: printed(&c, dev, (0.0, 250.0)).1 - printed(&c, dev, (0.0, 10.0)).1,
            left: printed(&c, dev, (10.0, 10.0)).0,
            top: printed(&c, dev, (10.0, 10.0)).1,
        };
        let same = Calibration::from_measures(c, EXPECTED, again).unwrap();
        assert!((same.scale_x - c.scale_x).abs() < 1e-12);
        assert!((same.offset_y - c.offset_y).abs() < 1e-9);
    }

    #[test]
    fn le_misure_giuste_non_cambiano_nulla() {
        let c = Calibration::from_measures(Calibration::IDENTITY, EXPECTED, EXPECTED).unwrap();
        assert!(c.is_identity());
        assert!(Calibration::default().is_identity());
    }

    #[test]
    fn le_misure_assurde_si_rifiutano() {
        let id = Calibration::IDENTITY;
        let with = |f: fn(&mut Measures)| {
            let mut m = EXPECTED;
            f(&mut m);
            Calibration::from_measures(id, EXPECTED, m)
        };
        // 18 cm al posto di 180 mm, una linea mancante, un bordo negativo o lontanissimo.
        assert_eq!(with(|m| m.width = 18.0), Err(BadMeasure::Width));
        assert_eq!(with(|m| m.height = 0.0), Err(BadMeasure::Height));
        assert_eq!(with(|m| m.height = f64::NAN), Err(BadMeasure::Height));
        assert_eq!(with(|m| m.left = -1.0), Err(BadMeasure::Left));
        assert_eq!(with(|m| m.top = 40.0), Err(BadMeasure::Top));
        // Un 2% si accetta.
        assert!(with(|m| m.width = 180.0 * 1.02).is_ok());
    }

    #[test]
    fn una_correzione_letta_male_torna_nessuna() {
        let bad = Calibration {
            scale_x: 3.0,
            ..Calibration::IDENTITY
        };
        assert!(bad.sanitized().is_identity());
        let nan = Calibration {
            offset_y: f64::NAN,
            ..Calibration::IDENTITY
        };
        assert!(nan.sanitized().is_identity());
        let ok = Calibration {
            scale_x: 1.01,
            offset_y: -2.0,
            ..Calibration::IDENTITY
        };
        assert_eq!(ok.sanitized(), ok);
    }
}
