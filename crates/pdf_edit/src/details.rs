//! Le informazioni di un PDF per la scheda Visualizza (ADR 0031): i riquadri di ogni pagina
//! (formato, al vivo, abbondanza), chi l'ha creato, se dichiara PDF/X e se porta la copia di
//! Illustrator. Si legge e basta: il file non cambia.

use pdfcraft_cos::{Document, Object};
use pdfcraft_organize as organize;
use winfiles_geom::{Size, pt_to_mm};

/// I riquadri di una pagina come la mostrano i lettori (rotazione applicata), in millimetri,
/// misurati dall'angolo in alto a sinistra del formato visibile (CropBox): `[x, y, larghezza,
/// altezza]`, con la y in giù.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageBoxes {
    /// Il formato visibile (CropBox dentro il MediaBox).
    pub size: Size,
    /// Il formato finito (TrimBox), se diverso dal formato visibile.
    pub trim: Option<[f64; 4]>,
    /// L'abbondanza (BleedBox), se diversa dal formato visibile.
    pub bleed: Option<[f64; 4]>,
    /// Rotazione della pagina in gradi, in senso orario.
    pub rotation: i64,
}

impl PageBoxes {
    /// Il formato finito: il TrimBox se c'è, altrimenti il formato visibile.
    pub fn finished(&self) -> Size {
        self.trim.map_or(self.size, |t| Size::new(t[2], t[3]))
    }

    /// L'abbondanza per lato attorno al formato finito, in mm (sinistra, sopra, destra, sotto):
    /// dal BleedBox se c'è, altrimenti dal formato visibile.
    pub fn bleed_sides(&self) -> Option<[f64; 4]> {
        let t = self.trim?;
        let b = self.bleed.unwrap_or([0.0, 0.0, self.size.w, self.size.h]);
        let sides = [t[0] - b[0], t[1] - b[1], b[0] + b[2] - t[0] - t[2], b[1] + b[3] - t[1] - t[3]];
        sides.iter().any(|v| *v > 0.05).then_some(sides.map(|v| v.max(0.0)))
    }
}

/// Le informazioni del documento.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    pub pages: Vec<PageBoxes>,
    /// Il programma che ha scritto il PDF (`/Producer`).
    pub producer: Option<String>,
    /// Il programma con cui è stato fatto (`/Creator`).
    pub creator: Option<String>,
    pub title: Option<String>,
    /// La versione PDF/X dichiarata (`/GTS_PDFXVersion`), se c'è.
    pub pdfx: Option<String>,
    /// Il profilo di uscita dichiarato (`/OutputConditionIdentifier`), se c'è.
    pub output_intent: Option<String>,
    /// La versione del formato (`1.6`, `1.7`, `2.0`).
    pub version: String,
    /// Il PDF porta la copia del file di Illustrator (ADR 0020).
    pub illustrator_data: bool,
}

/// Legge le informazioni.
pub(crate) fn read(doc: &Document) -> Details {
    let info = doc
        .trailer()
        .get(b"Info")
        .and_then(|o| doc.dict(o))
        .unwrap_or_default();
    let text = |k: &[u8]| {
        info.get(k)
            .map(|o| doc.resolve(o))
            .and_then(|o| o.as_string().map(|s| s.to_text()))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let catalog = doc
        .root()
        .and_then(|r| doc.get(r).as_dict().cloned())
        .unwrap_or_default();
    let intent = catalog
        .get(b"OutputIntents")
        .map(|o| doc.resolve(o))
        .and_then(|o| match &*o {
            Object::Array(items) => items.first().and_then(|i| doc.dict(i)),
            _ => None,
        });
    let output_intent = intent.and_then(|d| {
        [b"OutputConditionIdentifier".as_slice(), b"Info", b"OutputCondition"]
            .iter()
            .find_map(|k| {
                d.get(k)
                    .map(|o| doc.resolve(o))
                    .and_then(|o| o.as_string().map(|s| s.to_text()))
                    .filter(|s| !s.trim().is_empty())
            })
    });
    Details {
        pages: pages(doc),
        producer: text(b"Producer"),
        creator: text(b"Creator"),
        title: text(b"Title"),
        pdfx: text(b"GTS_PDFXVersion"),
        output_intent,
        version: doc.version().to_string(),
        illustrator_data: crate::illustrator::has(doc),
    }
}

fn pages(doc: &Document) -> Vec<PageBoxes> {
    let Ok(boxes) = organize::page_boxes(doc) else {
        return Vec::new();
    };
    boxes
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let rotation = organize::page_rotation(doc, i).unwrap_or(0).rem_euclid(360);
            boxes_of(b[1], b[2], b[3], rotation)
        })
        .collect()
}

/// I riquadri in mm dal formato visibile `crop`, girati come la pagina.
fn boxes_of(crop: [f64; 4], bleed: [f64; 4], trim: [f64; 4], rotation: i64) -> PageBoxes {
    let (cw, ch) = (crop[2] - crop[0], crop[3] - crop[1]);
    // Un riquadro in punti PDF (y in su) → [x, y, w, h] in mm dall'alto a sinistra del crop,
    // dopo la rotazione in senso orario.
    let place = |r: [f64; 4]| -> [f64; 4] {
        let (x0, y0) = (r[0] - crop[0], crop[3] - r[3]);
        let (w, h) = (r[2] - r[0], r[3] - r[1]);
        let out = match rotation {
            90 => [ch - y0 - h, x0, h, w],
            180 => [cw - x0 - w, ch - y0 - h, w, h],
            270 => [y0, cw - x0 - w, h, w],
            _ => [x0, y0, w, h],
        };
        out.map(pt_to_mm)
    };
    let turned = rotation == 90 || rotation == 270;
    let size = if turned {
        Size::new(pt_to_mm(ch), pt_to_mm(cw))
    } else {
        Size::new(pt_to_mm(cw), pt_to_mm(ch))
    };
    let differs = |r: [f64; 4]| {
        let p = place(r);
        (p[0].abs() > 0.01 || p[1].abs() > 0.01 || (p[2] - size.w).abs() > 0.01 || (p[3] - size.h).abs() > 0.01)
            .then_some(p)
    };
    PageBoxes {
        size,
        trim: differs(trim),
        bleed: differs(bleed),
        rotation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn il_rifilo_si_misura_dall_alto_a_sinistra() {
        // A4 con 3 mm di abbondanza: pagina 216×303, al vivo 210×297.
        let k = 72.0 / 25.4;
        let crop = [0.0, 0.0, 216.0 * k, 303.0 * k];
        let trim = [3.0 * k, 3.0 * k, 213.0 * k, 300.0 * k];
        let b = boxes_of(crop, crop, trim, 0);
        let t = b.trim.unwrap();
        assert!((t[0] - 3.0).abs() < 1e-6 && (t[1] - 3.0).abs() < 1e-6);
        assert!((t[2] - 210.0).abs() < 1e-6 && (t[3] - 297.0).abs() < 1e-6);
        assert!(b.bleed.is_none());
        let sides = b.bleed_sides().unwrap();
        assert!(sides.iter().all(|s| (s - 3.0).abs() < 1e-6));
    }

    #[test]
    fn la_pagina_girata_scambia_i_lati() {
        let k = 72.0 / 25.4;
        let crop = [0.0, 0.0, 100.0 * k, 50.0 * k];
        // Al vivo spostato a destra: 10 mm a sinistra, 0 a destra.
        let trim = [10.0 * k, 0.0, 100.0 * k, 50.0 * k];
        let b = boxes_of(crop, crop, trim, 90);
        assert!((b.size.w - 50.0).abs() < 1e-6 && (b.size.h - 100.0).abs() < 1e-6);
        let t = b.trim.unwrap();
        // Girata di 90° in senso orario, la sinistra va in alto.
        assert!((t[1] - 10.0).abs() < 1e-6, "{t:?}");
        assert!((t[3] - 90.0).abs() < 1e-6, "{t:?}");
    }

    #[test]
    fn senza_rifilo_nessuna_abbondanza() {
        let crop = [0.0, 0.0, 100.0, 100.0];
        let b = boxes_of(crop, crop, crop, 0);
        assert!(b.trim.is_none() && b.bleed_sides().is_none());
    }
}
