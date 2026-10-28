//! Il primo IFD di un TIFF: pixel, risoluzione e orientamento. Serve sia per i file `.tif` sia per
//! il blocco EXIF dei JPEG, che ha la stessa struttura.

use crate::ProbeError;
use crate::source::Source;

/// Voci massime lette da un IFD (il formato ne permette 65535, i file veri poche decine).
const MAX_ENTRIES: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TiffInfo {
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Risoluzione in punti per pollice, già convertita dall'unità del file.
    pub dpi: Option<(f64, f64)>,
    /// Orientamento EXIF (1-8); da 5 a 8 l'immagine si mostra ruotata di 90°.
    pub orientation: u16,
}

pub(crate) fn sniff(head: &[u8]) -> bool {
    head.starts_with(b"II*\0") || head.starts_with(b"MM\0*")
}

struct Reader<'a> {
    src: &'a mut dyn Source,
    base: u64,
    little: bool,
}

impl Reader<'_> {
    fn bytes<const N: usize>(&mut self, offset: u64) -> Result<[u8; N], ProbeError> {
        let mut buf = [0; N];
        let at = self
            .base
            .checked_add(offset)
            .ok_or(ProbeError::Damaged("posizione TIFF non valida"))?;
        self.src.read_exact_at(at, &mut buf)?;
        Ok(buf)
    }

    fn u16_of(&self, b: [u8; 2]) -> u16 {
        if self.little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        }
    }

    fn u32_of(&self, b: [u8; 4]) -> u32 {
        if self.little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        }
    }

    fn u16_at(&mut self, offset: u64) -> Result<u16, ProbeError> {
        let b = self.bytes::<2>(offset)?;
        Ok(self.u16_of(b))
    }

    fn u32_at(&mut self, offset: u64) -> Result<u32, ProbeError> {
        let b = self.bytes::<4>(offset)?;
        Ok(self.u32_of(b))
    }

    /// Un intero SHORT o LONG dal campo valore di una voce.
    fn integer(&self, kind: u16, value: [u8; 4]) -> Option<u32> {
        match kind {
            3 => Some(u32::from(self.u16_of([value[0], value[1]]))),
            4 => Some(self.u32_of(value)),
            _ => None,
        }
    }

    /// Un RATIONAL, che sta sempre fuori dalla voce, all'offset indicato.
    fn rational(&mut self, kind: u16, value: [u8; 4]) -> Result<Option<f64>, ProbeError> {
        if kind != 5 {
            return Ok(None);
        }
        let at = u64::from(self.u32_of(value));
        let num = self.u32_at(at)?;
        let den = self.u32_at(at.saturating_add(4))?;
        Ok((den != 0).then(|| f64::from(num) / f64::from(den)))
    }
}

/// Legge il primo IFD del TIFF che inizia a `base`.
pub(crate) fn read(src: &mut dyn Source, base: u64) -> Result<TiffInfo, ProbeError> {
    let mut head = [0; 4];
    src.read_exact_at(base, &mut head)?;
    let little = match &head {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return Err(ProbeError::Unrecognized),
    };
    let mut r = Reader { src, base, little };
    let ifd = u64::from(r.u32_at(4)?);
    let count = usize::from(r.u16_at(ifd)?).min(MAX_ENTRIES);
    let (mut xres, mut yres, mut unit) = (None, None, 2u32);
    let mut info = TiffInfo {
        orientation: 1,
        ..TiffInfo::default()
    };
    for i in 0..count {
        let at = ifd.saturating_add(2).saturating_add(12 * i as u64);
        let entry = r.bytes::<12>(at)?;
        let tag = r.u16_of([entry[0], entry[1]]);
        let kind = r.u16_of([entry[2], entry[3]]);
        let value = [entry[8], entry[9], entry[10], entry[11]];
        match tag {
            256 => info.width = r.integer(kind, value),
            257 => info.height = r.integer(kind, value),
            274 => {
                info.orientation = r
                    .integer(kind, value)
                    .and_then(|v| u16::try_from(v).ok())
                    .unwrap_or(1)
            }
            282 => xres = r.rational(kind, value)?,
            283 => yres = r.rational(kind, value)?,
            296 => unit = r.integer(kind, value).unwrap_or(2),
            _ => {}
        }
    }
    let per_inch = match unit {
        2 => Some(1.0),
        3 => Some(2.54),
        _ => None,
    };
    info.dpi = match (xres, yres, per_inch) {
        (Some(x), Some(y), Some(k)) => Some((x * k, y * k)),
        (Some(x), None, Some(k)) => Some((x * k, x * k)),
        _ => None,
    };
    Ok(info)
}
