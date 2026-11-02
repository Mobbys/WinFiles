//! Foto HEIC/HEIF (iPhone, Mac): i pixel si leggono dal contenitore ISOBMFF con il lettore di
//! PhotoCraft, senza decodificare l'immagine. Le foto HEIC non dichiarano una risoluzione di
//! stampa: valgono i DPI predefiniti, come per ogni immagine che non li dice.

use crate::raster::{Raw, finish};
use crate::source::{Source, be32};
use crate::{Probe, ProbeError};

/// Marchi del contenitore che dicono "immagine HEIF con HEVC".
const BRANDS: [&[u8; 4]; 8] = [
    b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx", b"mif1", b"msf1",
];
/// Byte letti al massimo per trovare il riquadro `meta` all'inizio del file.
const MAX_HEAD: u64 = 16 * 1024 * 1024;
/// File letti per intero al massimo, quando le misure non stanno nel solo `meta`.
const MAX_BYTES: u64 = 512 * 1024 * 1024;
/// Riquadri di primo livello attraversati al massimo.
const MAX_BOXES: usize = 64;

pub(crate) fn sniff(head: &[u8]) -> bool {
    head.get(4..8) == Some(b"ftyp")
        && head
            .get(8..12)
            .is_some_and(|b| BRANDS.iter().any(|m| m.as_slice() == b))
}

pub(crate) fn probe(src: &mut dyn Source) -> Result<Probe, ProbeError> {
    // Prima solo l'inizio del file fino alla fine di `meta`: le foto lo hanno in testa e le
    // misure stanno lì. Se non basta (o `meta` è in fondo) si legge tutto, con un tetto.
    let info = match meta_end(src)? {
        Some(end) if end <= MAX_HEAD => {
            let head = src.read_prefix(usize::try_from(end).unwrap_or(usize::MAX))?;
            match photocraft_heif::probe(&head) {
                Ok(info) => info,
                Err(_) => whole(src)?,
            }
        }
        _ => whole(src)?,
    };
    finish(Raw {
        width: info.width,
        height: info.height,
        dpi: None,
        rotated: false,
    })
}

fn whole(src: &mut dyn Source) -> Result<photocraft_heif::Info, ProbeError> {
    let data = src.read_all(MAX_BYTES)?;
    photocraft_heif::probe(&data).map_err(|e| match e {
        photocraft_heif::Error::Unsupported(_) => {
            ProbeError::Damaged("HEIF senza un'immagine fissa (forse un video)")
        }
        _ => ProbeError::Damaged("HEIF non leggibile"),
    })
}

/// Dove finisce il riquadro `meta` di primo livello, se c'è.
fn meta_end(src: &mut dyn Source) -> Result<Option<u64>, ProbeError> {
    let len = src.len();
    let mut at = 0u64;
    for _ in 0..MAX_BOXES {
        if at.checked_add(8).is_none_or(|e| e > len) {
            return Ok(None);
        }
        let mut header = [0u8; 16];
        let n = if at + 16 <= len { 16 } else { 8 };
        src.read_exact_at(at, header.get_mut(..n).unwrap_or_default())?;
        let size = be32(&header, 0).ok_or(ProbeError::NoSize)?;
        let size = match size {
            0 => len - at,
            1 if n == 16 => u64::from_be_bytes(
                header
                    .get(8..16)
                    .and_then(|b| b.try_into().ok())
                    .unwrap_or_default(),
            ),
            1 => return Ok(None),
            s => u64::from(s),
        };
        if size < 8 {
            return Err(ProbeError::Damaged("riquadro HEIF di misura impossibile"));
        }
        let end = at
            .checked_add(size)
            .ok_or(ProbeError::Damaged("riquadro HEIF oltre la fine"))?;
        if header.get(4..8) == Some(b"meta") {
            return Ok(Some(end.min(len)));
        }
        at = end;
    }
    Ok(None)
}
