//! Un disegno (DXF, DWG, PLT, SVG) come pagina PDF bianca alla sua misura vera: la strada unica
//! con cui lo leggono le Miniature (misure, miniature, stampa, PDF) e le anteprime di Esplora
//! file. La scheda Visualizza usa lo stesso lettore ([`crate::open`]) e in più sceglie layer,
//! fondo e unità.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::{CadError, Drawing, MAX_PAGE_MM, PdfOptions};

/// Le estensioni dei disegni che diventano una pagina: quelle del CAD e gli SVG.
pub const DRAWING_EXTENSIONS: &[&str] = &["dxf", "dwg", "plt", "hpgl", "hpg", "svg", "svgz"];

/// Vero se il nome è di un disegno (DXF, DWG, PLT, SVG).
pub fn is_drawing_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| DRAWING_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Il disegno pronto da disegnare come un PDF.
#[derive(Clone, Debug, PartialEq)]
pub struct Paper {
    /// Una pagina PDF vettoriale su fondo bianco, alla misura vera (ridotta solo se supera il
    /// limite dei PDF: le proporzioni restano quelle).
    pub pdf: Arc<Vec<u8>>,
    /// La misura vera del disegno in mm (per un DXF senza unità dichiarate, 1 unità = 1 mm,
    /// come in Visualizza).
    pub size_mm: (f64, f64),
}

/// Un disegno letto, tenuto in memoria.
#[derive(Clone)]
enum Read {
    Cad(Arc<Drawing>),
    /// Gli SVG diventano subito un PDF: hanno già i loro spessori.
    Svg(Arc<Vec<u8>>, (f64, f64)),
}

/// La misura vera in mm del disegno in `path`.
pub fn size_mm(path: &Path) -> Result<(f64, f64), CadError> {
    match read(path)? {
        Read::Cad(d) => Ok(geometry(&d, 1.0)?.0),
        Read::Svg(_, size) => Ok(size),
    }
}

/// Il disegno in `path` come pagina da disegnare con `side_px` pixel sul lato lungo: le linee
/// sono spesse almeno un pixel, perché quelle sottilissime di Visualizza, rimpicciolite,
/// diventano puntini grigi. Lo spessore del file vale se è più grande.
pub fn paper(path: &Path, side_px: u32) -> Result<Paper, CadError> {
    match read(path)? {
        Read::Cad(d) => drawing_paper(&d, 1.0, side_px),
        Read::Svg(pdf, size_mm) => Ok(Paper { pdf, size_mm }),
    }
}

/// La pagina di un DXF o DWG già letto. `unit_mm` vale quando il file non dichiara le unità.
pub fn drawing_paper(drawing: &Drawing, unit_mm: f64, side_px: u32) -> Result<Paper, CadError> {
    let ((w, h), scale, unit_mm) = geometry(drawing, unit_mm)?;
    let page_long = w.max(h) * scale / unit_mm;
    let line = page_long / f64::from(side_px.max(16)) * LINE_PX;
    // Un margine di uno spessore tiene intere le linee sul bordo.
    let options = PdfOptions {
        mm_per_unit: scale,
        line_mm: Some(line),
        margin_mm: line,
        ..PdfOptions::default()
    };
    let pdf = drawing.to_pdf(&options).map_err(CadError::Page)?;
    Ok(Paper {
        pdf: Arc::new(pdf),
        size_mm: (w.max(MIN_SIDE_MM), h.max(MIN_SIDE_MM)),
    })
}

/// Misura vera in mm, scala della pagina (mm per unità) e mm per unità del disegno.
fn geometry(drawing: &Drawing, unit_mm: f64) -> Result<((f64, f64), f64, f64), CadError> {
    let unit_mm = drawing.units.mm.unwrap_or(unit_mm);
    let (w, h) = drawing.size_mm(unit_mm);
    if !(w.is_finite() && h.is_finite() && unit_mm > 0.0) {
        return Err(CadError::Empty);
    }
    // A misura vera; oltre i 5 m della pagina PDF si riduce, la misura resta quella vera.
    let scale = if w.max(h) <= MAX_PAGE_MM / 2.0 {
        unit_mm
    } else {
        drawing.fit_scale(MAX_PAGE_MM / 2.0)
    };
    Ok(((w.max(MIN_SIDE_MM), h.max(MIN_SIDE_MM)), scale, unit_mm))
}

/// Lo spessore minimo delle linee, in pixel della miniatura.
const LINE_PX: f64 = 1.2;

/// Lato minimo di un disegno (una linea dritta ha altezza zero).
const MIN_SIDE_MM: f64 = 1.0;

/// Legge il disegno; gli ultimi letti restano in memoria, così misura, miniature a più
/// grandezze e PDF non rileggono il file.
fn read(path: &Path) -> Result<Read, CadError> {
    let meta = std::fs::metadata(path)?;
    let key = (path.to_path_buf(), meta.len(), meta.modified().ok());
    if let Some(r) = cached(&key) {
        return Ok(r);
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let read = if ext == "svg" || ext == "svgz" {
        let (pdf, size) = crate::svg::open(path)?;
        Read::Svg(Arc::new(pdf), size)
    } else {
        Read::Cad(Arc::new(crate::open(path)?))
    };
    remember(key, &read);
    Ok(read)
}

type Key = (PathBuf, u64, Option<SystemTime>);

/// Disegni tenuti in memoria.
const CACHED: usize = 6;

fn cache() -> &'static Mutex<Vec<(Key, Read)>> {
    static CACHE: Mutex<Vec<(Key, Read)>> = Mutex::new(Vec::new());
    &CACHE
}

fn cached(key: &Key) -> Option<Read> {
    let list = cache().lock().ok()?;
    list.iter().find(|(k, _)| k == key).map(|(_, r)| r.clone())
}

fn remember(key: Key, read: &Read) {
    if let Ok(mut list) = cache().lock() {
        list.retain(|(k, _)| k.0 != key.0);
        if list.len() >= CACHED {
            list.remove(0);
        }
        list.push((key, read.clone()));
    }
}
