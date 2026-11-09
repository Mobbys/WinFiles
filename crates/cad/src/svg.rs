//! I disegni SVG: diventano un PDF vettoriale con [svg2pdf], alla loro misura vera (le unità del
//! file, con i pixel a 96 per pollice come nei browser), così la scheda Visualizza li mostra e li
//! stampa come un PDF.

use std::sync::{Arc, OnceLock};

use svg2pdf::usvg;

use crate::CadError;

/// Byte massimi di un SVG (anche compresso, SVGZ).
const MAX_BYTES: usize = 256 * 1024 * 1024;
/// Lato massimo del disegno in mm (100 m).
const MAX_SIDE_MM: f64 = 100_000.0;

/// I caratteri del sistema, letti una volta sola (servono ai testi degli SVG).
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

/// La misura del disegno in mm.
pub fn size_mm(tree: &usvg::Tree) -> (f64, f64) {
    let s = tree.size();
    (
        f64::from(s.width()) * 25.4 / 96.0,
        f64::from(s.height()) * 25.4 / 96.0,
    )
}

/// Un SVG (o SVGZ) in PDF, con la sua misura in mm.
pub fn to_pdf(bytes: &[u8]) -> Result<(Vec<u8>, (f64, f64)), CadError> {
    if bytes.len() > MAX_BYTES {
        return Err(CadError::TooLarge(bytes.len() as u64));
    }
    let options = usvg::Options {
        fontdb: fonts(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options).map_err(|e| CadError::Svg(e.to_string()))?;
    let (w, h) = size_mm(&tree);
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return Err(CadError::Empty);
    }
    if w > MAX_SIDE_MM || h > MAX_SIDE_MM {
        return Err(CadError::Svg(format!("misura fuori scala ({w:.0} × {h:.0} mm)")));
    }
    let page = svg2pdf::PageOptions { dpi: 96.0 };
    let pdf = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        svg2pdf::to_pdf(&tree, svg2pdf::ConversionOptions::default(), page)
    }))
    .map_err(|_| CadError::Svg("il convertitore si è fermato su un errore interno".into()))?
    .map_err(|e| CadError::Svg(e.to_string()))?;
    Ok((pdf, (w, h)))
}

/// Un SVG dal disco.
pub fn open(path: &std::path::Path) -> Result<(Vec<u8>, (f64, f64)), CadError> {
    let len = std::fs::metadata(path)?.len();
    if len > MAX_BYTES as u64 {
        return Err(CadError::TooLarge(len));
    }
    to_pdf(&std::fs::read(path)?)
}
