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
mod diag;
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
    // Lo stesso tipo e la stessa strada delle Miniature: anche i disegni (DXF, DWG, PLT, SVG)
    // passano dalla loro pagina alla misura vera.
    let kind = FileKind::from_path(path);
    if kind == FileKind::Unsupported {
        return Err("tipo di file non previsto".into());
    }
    winfiles_thumbs::render(path, kind, 0, side).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;

/// Il tipo del file dai primi byte, quando Windows non dà il nome (il flusso senza nome).
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    let head = bytes.get(..bytes.len().min(4096)).unwrap_or_default();
    let starts = |m: &[u8]| head.starts_with(m);
    if starts(b"%PDF") {
        return Some("pdf");
    }
    if starts(b"%!PS") || starts(&[0xC5, 0xD0, 0xD3, 0xC6]) {
        return Some("eps");
    }
    if starts(b"8BPS") {
        return Some(if head.get(4..6) == Some(&[0, 2]) {
            "psb"
        } else {
            "psd"
        });
    }
    if starts(b"AC10") || starts(b"AC1.") || starts(b"AC2.") {
        return Some("dwg");
    }
    if starts(&[0x1f, 0x8b]) {
        return Some("svgz");
    }
    let text = String::from_utf8_lossy(head);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with("<svg") || (trimmed.starts_with("<?xml") && text.contains("<svg")) {
        return Some("svg");
    }
    if trimmed.starts_with("0\r\nSECTION") || trimmed.starts_with("0\nSECTION") || trimmed.starts_with("999")
    {
        return Some("dxf");
    }
    if head.get(4..12) == Some(b"ftypheic")
        || head.get(4..12) == Some(b"ftypmif1")
        || head.get(4..12) == Some(b"ftypheix")
    {
        return Some("heic");
    }
    if starts(b"RIFF") && head.get(8..12) == Some(b"WEBP") {
        return Some("webp");
    }
    if winfiles_cad::hpgl::looks_like_hpgl(head) {
        return Some("plt");
    }
    None
}

/// File piccoli per le prove (i test e la prova della CI su Windows).
#[doc(hidden)]
pub mod prova {
    /// Un DXF con un rettangolo di 100 × 50.
    pub const DXF: &str = "0\nSECTION\n2\nENTITIES\n0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n10\n0\n20\n0\n10\n100\n20\n0\n10\n100\n20\n50\n10\n0\n20\n50\n0\nENDSEC\n0\nEOF\n";

    /// Un PLT con un rettangolo e un cerchio.
    pub const PLT: &str = "IN;SP1;PU0,0;PD4000,0,4000,2000,0,2000,0,0;PU;SP2;PU2000,1000;CI800;";

    /// Un SVG rosso con una scritta.
    pub const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect width="200" height="100" fill="red"/><text x="10" y="60" font-size="40">Prova</text></svg>"#;

    /// Un PDF di una pagina (il PLT a 200 mm).
    pub fn pdf() -> Result<Vec<u8>, String> {
        let d = winfiles_cad::hpgl::parse(PLT.as_bytes()).map_err(|e| e.to_string())?;
        d.to_pdf(&winfiles_cad::PdfOptions {
            mm_per_unit: d.fit_scale(200.0),
            ..winfiles_cad::PdfOptions::default()
        })
    }

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
