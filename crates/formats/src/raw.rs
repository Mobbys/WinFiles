//! Foto RAW delle fotocamere. Un RAW comincia quasi sempre come un TIFF, ma il primo riquadro è
//! solo una miniatura: le misure vere sono quelle dell'anteprima a piena misura che la
//! fotocamera mette nel file, trovata con il lettore di PhotoCraft. La risoluzione è quella
//! scritta in testa al TIFF, come per le foto JPEG della stessa macchina.
//!
//! Per trovare l'anteprima il file si legge tutto (con un tetto): le fotocamere la mettono dove
//! capita. Le sue misure possono differire di qualche pixel da quelle della foto sviluppata (i
//! bordi del sensore): nel modulo immagini valgono quelle della foto sviluppata.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use crate::raster::{Raw, finish};
use crate::source::Source;
use crate::{MAX_PIXELS, Probe, ProbeError, tiff};

/// RAW più grande che si misura.
const MAX_BYTES: u64 = 1024 * 1024 * 1024;

/// Il nome del file è quello di una foto RAW.
pub fn is_raw_path(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        photocraft_raw::EXTENSIONS
            .iter()
            .any(|r| r.eq_ignore_ascii_case(e))
    })
}

/// I RAW che si riconoscono dai primi byte, senza guardare il nome: CR2, CR3, ORF, RW2, RAF.
/// I NEF, gli ARW e i DNG sono TIFF a tutti gli effetti e si riconoscono dall'estensione.
pub(crate) fn sniff(head: &[u8]) -> bool {
    head.get(8..11) == Some(b"CR\x02")
        || head.get(4..12) == Some(b"ftypcrx ")
        || matches!(head.get(0..4), Some(b"IIRO" | b"IIRS" | b"MMOR" | b"IIU\0"))
        || head.starts_with(b"FUJIFILMCCD-RAW")
}

pub(crate) fn probe(src: &mut dyn Source) -> Result<Probe, ProbeError> {
    let data = src.read_all(MAX_BYTES)?;
    let found = catch_unwind(AssertUnwindSafe(|| {
        photocraft_raw::identify(&data)?;
        let orientation = photocraft_raw::orientation(&data).unwrap_or(1);
        if let Some(preview) = photocraft_raw::embedded_preview(&data) {
            return Some((preview.width, preview.height, orientation));
        }
        // Senza anteprima (succede con certi DNG) si leggono i dati del sensore: più lento,
        // ma la misura è quella esatta della foto sviluppata.
        let limits = photocraft_raw::Limits {
            max_width: MAX_PIXELS,
            max_height: MAX_PIXELS,
            ..photocraft_raw::Limits::default()
        };
        let sensor = photocraft_raw::decode(&data, &limits).ok()?;
        let width = u32::try_from(sensor.crop.width).ok()?;
        let height = u32::try_from(sensor.crop.height).ok()?;
        Some((width, height, sensor.orientation))
    }))
    .map_err(|_| ProbeError::Damaged("foto RAW illeggibile"))?;
    let (width, height, orientation) = found.ok_or(ProbeError::NoSize)?;
    let dpi = if tiff::sniff(&data) {
        tiff::read(src, 0).ok().and_then(|t| t.dpi)
    } else {
        None
    };
    finish(Raw {
        width,
        height,
        dpi,
        rotated: (5..=8).contains(&orientation),
    })
}
