//! Misure dei file (livello L1): pagine e formati in millimetri, pixel e risoluzione delle
//! immagini. Legge solo le intestazioni, senza decodificare i contenuti, così anche migliaia di
//! file si misurano in pochi secondi.
//!
//! Ogni numero letto da un file è ostile: dimensioni e conteggi hanno un tetto, le letture sono
//! controllate e un file rovinato dà un errore, mai un crash.

use std::path::Path;

use winfiles_model::{FileKind, ImageInfo, PageInfo};

mod eps;
mod heif;
mod pdf;
mod psd;
mod raster;
mod raw;
mod source;
mod tiff;

#[cfg(test)]
mod tests;

pub use raw::is_raw_path;
pub use source::Source;

/// Risoluzione usata quando un'immagine non la dichiara (la stessa della versione 4.9).
pub const DEFAULT_DPI: f64 = 96.0;
/// Lato massimo accettato per un'immagine, in pixel.
pub const MAX_PIXELS: u32 = 1_000_000;
/// Pagine massime lette da un PDF.
pub const MAX_PAGES: usize = 100_000;

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("impossibile leggere il file: {0}")]
    Io(#[from] std::io::Error),
    #[error("formato non riconosciuto")]
    Unrecognized,
    #[error("file danneggiato: {0}")]
    Damaged(&'static str),
    #[error("il file è troppo grande da leggere ({0} byte)")]
    TooLarge(u64),
    #[error("il PDF è protetto da password")]
    Encrypted,
    #[error("il file non dichiara le sue misure")]
    NoSize,
}

/// Le misure di un file.
#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    pub pages: Vec<PageInfo>,
    pub image: Option<ImageInfo>,
}

/// Misura il file in `path`, che l'estensione dice essere di tipo `kind`. Il contenuto vince
/// sull'estensione: un `.ai` salvato con compatibilità PDF si legge come PDF.
pub fn probe(path: &Path, kind: FileKind) -> Result<Probe, ProbeError> {
    let mut src = source::FileSource::open(path)?;
    // I NEF, gli ARW, i DNG… hanno l'intestazione di un TIFF qualsiasi: li distingue il nome.
    if is_raw_path(path) {
        let head = src.read_prefix(16)?;
        if tiff::sniff(&head) || raw::sniff(&head) {
            return raw::probe(&mut src);
        }
    }
    probe_source(&mut src, kind)
}

/// Come [`probe`], su una sorgente qualsiasi (per esempio dati in memoria).
pub fn probe_source(src: &mut dyn Source, kind: FileKind) -> Result<Probe, ProbeError> {
    let head = src.read_prefix(1024)?;
    if raw::sniff(&head) {
        return raw::probe(src);
    }
    if psd::sniff(&head) {
        return psd::probe(src);
    }
    if raster::sniff(&head) {
        return raster::probe(src);
    }
    if heif::sniff(&head) {
        return heif::probe(src);
    }
    if pdf::sniff(&head) {
        return pdf::probe(src);
    }
    match kind {
        FileKind::Ai | FileKind::Eps if eps::sniff(&head) => eps::probe(src),
        _ => Err(ProbeError::Unrecognized),
    }
}
