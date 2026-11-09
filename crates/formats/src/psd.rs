//! File di Photoshop (PSD e PSB): pixel dall'intestazione, risoluzione dalla risorsa 1005.
//! Si leggono solo l'intestazione e l'elenco delle risorse, mai i livelli né l'immagine.

use crate::raster::{Raw, finish};
use crate::source::{Source, be16, be32};
use crate::{Probe, ProbeError};

/// Risorse attraversate al massimo prima di arrendersi.
const MAX_RESOURCES: usize = 10_000;
/// La risorsa con la risoluzione.
const RESOLUTION_INFO: u16 = 1005;

pub(crate) fn sniff(head: &[u8]) -> bool {
    head.starts_with(b"8BPS") && matches!(head.get(4..6), Some([0, 1] | [0, 2]))
}

pub(crate) fn probe(src: &mut dyn Source) -> Result<Probe, ProbeError> {
    let head = src.read_prefix(30)?;
    let height = be32(&head, 14).ok_or(ProbeError::NoSize)?;
    let width = be32(&head, 18).ok_or(ProbeError::NoSize)?;
    let color_data = be32(&head, 26).ok_or(ProbeError::Damaged("intestazione PSD troppo corta"))?;
    let section = 30u64.saturating_add(u64::from(color_data));
    let dpi = resolution(src, section).unwrap_or(None);
    finish(Raw {
        width,
        height,
        dpi,
        rotated: false,
    })
}

/// La risoluzione dalla sezione delle risorse che comincia a `section`. Una sezione rovinata
/// vuol dire solo "risoluzione non dichiarata".
fn resolution(src: &mut dyn Source, section: u64) -> Result<Option<(f64, f64)>, ProbeError> {
    let mut len = [0u8; 4];
    src.read_exact_at(section, &mut len)?;
    let mut pos = section + 4;
    let end = pos
        .saturating_add(u64::from(u32::from_be_bytes(len)))
        .min(src.len());
    for _ in 0..MAX_RESOURCES {
        if pos.saturating_add(12) > end {
            break;
        }
        // Firma (4), numero (2), nome Pascal pari, lunghezza (4), dati pari.
        let top = src.read_up_to(pos, 7)?;
        let id = be16(&top, 4).ok_or(ProbeError::Damaged("risorsa PSD troncata"))?;
        let name_len = u64::from(top.get(6).copied().unwrap_or(0));
        let name = (1 + name_len).next_multiple_of(2);
        let size_at = pos + 6 + name;
        let size_bytes = src.read_up_to(size_at, 4)?;
        let size = u64::from(be32(&size_bytes, 0).ok_or(ProbeError::Damaged("risorsa PSD troncata"))?);
        let data = size_at + 4;
        if id == RESOLUTION_INFO && size >= 16 {
            let info = src.read_up_to(data, 16)?;
            // 16.16: 300 DPI si legge 300,0000153; si arrotonda al millesimo.
            let fixed = |at| be32(&info, at).map(|v| (f64::from(v) / 65536.0 * 1000.0).round() / 1000.0);
            if let (Some(x), Some(y)) = (fixed(0), fixed(8)) {
                return Ok(Some((x, y)));
            }
        }
        pos = data.saturating_add(size.next_multiple_of(2));
    }
    Ok(None)
}
