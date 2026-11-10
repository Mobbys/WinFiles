//! Le anteprime che Esplora file di Windows mostra nelle cartelle con le icone grandi (ADR 0036).
//!
//! Windows chiede l'anteprima di un file alla DLL registrata per la sua estensione, in un processo
//! a parte (dllhost): se un file rovinato la fa cadere, Esplora file resta in piedi. Qui c'è la
//! parte che disegna, uguale su ogni sistema e provata con i test; in `com.rs` quella che parla
//! con Windows.

use std::path::Path;

use winfiles_model::FileKind;
use winfiles_thumbs::Thumb;

#[cfg(windows)]
mod com;
#[cfg(windows)]
mod reg;

/// Il CLSID della classe delle anteprime (uguale nell'installer).
pub const CLSID: &str = "{8B3C7E21-5D4A-4F96-9C1B-2E7A6D0F4B53}";

/// Le estensioni per cui WinFiles dà l'anteprima. JPEG, PNG, BMP, GIF e TIFF restano a Windows,
/// che le fa già; HEIC, WebP e RAW le fa solo con le estensioni dello Store.
pub const EXTENSIONS: &[&str] = &[
    "pdf", "ai", "eps", "psd", "psb", "heic", "heif", "webp", "tga", "dng", "cr2", "cr3", "nef", "nrw",
    "arw", "orf", "rw2", "raf", "pef", "svg", "svgz", "dxf", "dwg", "plt", "hpgl",
];

/// Byte massimi di un file di cui si fa l'anteprima.
pub const MAX_BYTES: u64 = 1024 * 1024 * 1024;

/// L'anteprima del file, con il lato lungo al massimo `side` pixel.
pub fn thumbnail(path: &Path, side: u32) -> Result<Thumb, String> {
    let side = side.clamp(16, 2560);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "svg" | "svgz" => {
            let (pdf, _) = winfiles_cad::svg::open(path).map_err(|e| e.to_string())?;
            pdf_thumb(pdf, side)
        }
        "dxf" | "dwg" | "plt" | "hpgl" | "hpg" => {
            let drawing = winfiles_cad::open(path).map_err(|e| e.to_string())?;
            // Il disegno in 200 mm sul lato lungo, fondo bianco, linee visibili anche in piccolo.
            let options = winfiles_cad::PdfOptions {
                mm_per_unit: drawing.fit_scale(200.0),
                line_mm: Some(0.8),
                margin_mm: 4.0,
                ..winfiles_cad::PdfOptions::default()
            };
            pdf_thumb(drawing.to_pdf(&options)?, side)
        }
        _ => {
            let kind = FileKind::from_path(path);
            if kind == FileKind::Unsupported {
                return Err("tipo di file non previsto".into());
            }
            winfiles_thumbs::render(path, kind, 0, side).map_err(|e| e.to_string())
        }
    }
}

fn pdf_thumb(pdf: Vec<u8>, side: u32) -> Result<Thumb, String> {
    winfiles_thumbs::PdfPages::open(pdf)
        .and_then(|p| p.render(0, side))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;

/// File piccoli per le prove (i test e la prova della CI su Windows).
#[doc(hidden)]
pub mod prova {
    /// Un DXF con un rettangolo di 100 × 50.
    pub const DXF: &str = "0\nSECTION\n2\nENTITIES\n0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n10\n0\n20\n0\n10\n100\n20\n0\n10\n100\n20\n50\n10\n0\n20\n50\n0\nENDSEC\n0\nEOF\n";

    /// Un PSD RGB rosso di `w` × `h` pixel, senza livelli né compressione.
    pub fn psd(w: u32, h: u32) -> Vec<u8> {
        let mut d = b"8BPS".to_vec();
        d.extend_from_slice(&1u16.to_be_bytes());
        d.extend_from_slice(&[0; 6]);
        d.extend_from_slice(&3u16.to_be_bytes());
        d.extend_from_slice(&h.to_be_bytes());
        d.extend_from_slice(&w.to_be_bytes());
        d.extend_from_slice(&8u16.to_be_bytes());
        d.extend_from_slice(&3u16.to_be_bytes());
        // Modo colore, risorse, livelli: vuoti; poi i pixel senza compressione.
        d.extend_from_slice(&[0; 12]);
        d.extend_from_slice(&0u16.to_be_bytes());
        let n = (w as usize).saturating_mul(h as usize);
        for v in [255u8, 0, 0] {
            d.extend(std::iter::repeat_n(v, n));
        }
        d
    }
}
