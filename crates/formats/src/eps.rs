//! EPS e AI in PostScript: il riquadro dai commenti DSC (`%%HiResBoundingBox`, altrimenti
//! `%%BoundingBox`), anche nei file con l'intestazione binaria DOS e anteprima TIFF.

use winfiles_geom::{Size, pt_to_mm};
use winfiles_model::PageInfo;

use crate::source::{Source, le32};
use crate::{Probe, ProbeError};

/// Quanto si legge in testa e in coda per cercare i commenti.
const WINDOW: usize = 256 * 1024;
const DOS_MAGIC: [u8; 4] = [0xC5, 0xD0, 0xD3, 0xC6];

pub(crate) fn sniff(head: &[u8]) -> bool {
    head.starts_with(b"%!PS") || head.starts_with(&DOS_MAGIC)
}

pub(crate) fn probe(src: &mut dyn Source) -> Result<Probe, ProbeError> {
    let head = src.read_prefix(32)?;
    // Intestazione DOS: inizio e lunghezza della parte PostScript.
    let (start, len) = if head.starts_with(&DOS_MAGIC) {
        let start = le32(&head, 4).ok_or(ProbeError::Damaged("intestazione EPS corta"))?;
        let len = le32(&head, 8).ok_or(ProbeError::Damaged("intestazione EPS corta"))?;
        (u64::from(start), u64::from(len))
    } else {
        (0, src.len())
    };
    let end = start
        .checked_add(len)
        .filter(|e| *e <= src.len())
        .ok_or(ProbeError::Damaged("sezione PostScript fuori dal file"))?;
    let window = usize::try_from(len).unwrap_or(usize::MAX).min(WINDOW);
    let first = src.read_up_to(start, window)?;
    let mut bbox = find(&first, false);
    if bbox.is_none_or(|b| b.atend) {
        let tail_start = end.saturating_sub(window as u64).max(start);
        let tail = src.read_up_to(tail_start, window)?;
        bbox = find(&tail, true).filter(|b| !b.atend).or(bbox);
    }
    let bbox = bbox.filter(|b| !b.atend).ok_or(ProbeError::NoSize)?;
    let w = pt_to_mm(bbox.urx - bbox.llx);
    let h = pt_to_mm(bbox.ury - bbox.lly);
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 && w < 400_000.0 && h < 400_000.0) {
        return Err(ProbeError::Damaged("BoundingBox non valido"));
    }
    Ok(Probe {
        pages: vec![PageInfo {
            size: Size::new(w, h),
            trim: None,
        }],
        image: None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct BBox {
    llx: f64,
    lly: f64,
    urx: f64,
    ury: f64,
    /// `(atend)`: il valore vero è in coda al file.
    atend: bool,
}

/// Cerca i riquadri nelle righe di `text`. In testa vale il primo trovato, in coda l'ultimo.
/// L'alta risoluzione vince sempre su quella intera.
fn find(text: &[u8], last: bool) -> Option<BBox> {
    let mut hires = None;
    let mut plain = None;
    for line in text.split(|b| *b == b'\n' || *b == b'\r') {
        let (slot, rest) = if let Some(rest) = line.strip_prefix(b"%%HiResBoundingBox:") {
            (&mut hires, rest)
        } else if let Some(rest) = line.strip_prefix(b"%%BoundingBox:") {
            (&mut plain, rest)
        } else {
            if !last && line.starts_with(b"%%EndComments") && (hires.is_some() || plain.is_some()) {
                break;
            }
            continue;
        };
        if slot.is_some() && !last {
            continue;
        }
        *slot = parse(rest);
    }
    match (hires, plain) {
        (Some(h), _) if !h.atend => Some(h),
        (_, Some(p)) => Some(p),
        (h, None) => h,
    }
}

fn parse(rest: &[u8]) -> Option<BBox> {
    let text = std::str::from_utf8(rest).ok()?.trim();
    if text.starts_with("(atend)") {
        return Some(BBox {
            llx: 0.0,
            lly: 0.0,
            urx: 0.0,
            ury: 0.0,
            atend: true,
        });
    }
    let mut n = text
        .split_ascii_whitespace()
        .map(|v| v.parse::<f64>().ok().filter(|v| v.is_finite()));
    Some(BBox {
        llx: n.next()??,
        lly: n.next()??,
        urx: n.next()??,
        ury: n.next()??,
        atend: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hires_wins_and_atend_is_resolved() {
        let b = find(b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 596 842\n%%HiResBoundingBox: 0 0 595.28 841.89\n%%EndComments\n", false);
        assert_eq!(b.map(|b| b.urx), Some(595.28));
        let b = find(b"%%BoundingBox: (atend)\n%%EndComments\n", false);
        assert!(b.is_some_and(|b| b.atend));
        let b = find(b"showpage\n%%Trailer\n%%BoundingBox: 10 10 110 60\n", true);
        assert_eq!(b.map(|b| (b.urx - b.llx, b.ury - b.lly)), Some((100.0, 50.0)));
        assert_eq!(find(b"%%BoundingBox: 1 2 x 4\n", false), None);
    }
}
