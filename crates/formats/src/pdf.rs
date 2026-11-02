//! PDF (e AI salvati con compatibilità PDF): numero di pagine, formato pagina e formato finito
//! (TrimBox) di ognuna, con la rotazione già applicata.

use hayro_syntax::object::Rect;
use hayro_syntax::page::Rotation;
use hayro_syntax::{LoadPdfError, Pdf};
use winfiles_geom::{Size, pt_to_mm};
use winfiles_model::PageInfo;

use crate::source::Source;
use crate::{MAX_PAGES, Probe, ProbeError};

/// Dimensione massima di un PDF da misurare (il file si legge tutto in memoria).
const MAX_PDF_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Lato massimo credibile di una pagina: 15.000 pollici, il limite di PDF 1.7 con UserUnit.
const MAX_SIDE_MM: f64 = 381_000.0;

/// `%PDF-` nei primi 1024 byte, come accettano i lettori PDF.
pub(crate) fn sniff(head: &[u8]) -> bool {
    head.windows(5).any(|w| w == b"%PDF-")
}

fn size_of(r: Rect, rotation: Rotation) -> Option<Size> {
    let w = pt_to_mm((r.x1 - r.x0).abs());
    let h = pt_to_mm((r.y1 - r.y0).abs());
    let ok = |v: f64| v.is_finite() && v > 0.0 && v <= MAX_SIDE_MM;
    if !ok(w) || !ok(h) {
        return None;
    }
    Some(match rotation {
        Rotation::Horizontal | Rotation::FlippedHorizontal => Size::new(h, w),
        Rotation::None | Rotation::Flipped => Size::new(w, h),
    })
}

pub(crate) fn probe(src: &mut dyn Source) -> Result<Probe, ProbeError> {
    let data = src.read_all(MAX_PDF_BYTES)?;
    let pdf = Pdf::new(data).map_err(|e| match e {
        LoadPdfError::Decryption(_) => ProbeError::Encrypted,
        LoadPdfError::Invalid => ProbeError::Damaged("struttura PDF non leggibile"),
    })?;
    let mut pages = Vec::new();
    for page in pdf.pages().iter().take(MAX_PAGES) {
        let rotation = page.rotation();
        // Il formato pagina è quello che mostrano i lettori: il CropBox dentro il MediaBox.
        let Some(size) = size_of(page.intersected_crop_box(), rotation) else {
            continue;
        };
        let trim = page
            .raw()
            .get::<Rect>(b"TrimBox")
            .and_then(|r| size_of(r, rotation))
            .filter(|t| (t.w - size.w).abs() > 0.01 || (t.h - size.h).abs() > 0.01);
        pages.push(PageInfo { size, trim });
    }
    if pages.is_empty() {
        return Err(ProbeError::NoSize);
    }
    Ok(Probe { pages, image: None })
}
