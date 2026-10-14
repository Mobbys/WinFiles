//! Immagini raster: JPEG, PNG, TIFF, GIF, BMP e WebP. Si leggono pixel e risoluzione dalle
//! intestazioni; le misure in millimetri vengono da pixel ÷ DPI.

use winfiles_geom::{Size, px_to_mm};
use winfiles_model::{ImageInfo, PageInfo};

use crate::source::{Source, be16, be32, le16, le32};
use crate::{DEFAULT_DPI, MAX_PIXELS, Probe, ProbeError, tiff};

/// Segmenti massimi attraversati in un JPEG o blocchi in un PNG prima di arrendersi.
const MAX_SEGMENTS: usize = 10_000;
/// Risoluzione massima credibile: oltre è un valore rovinato e si ignora.
const MAX_DPI: f64 = 100_000.0;

pub(crate) fn sniff(head: &[u8]) -> bool {
    head.starts_with(&[0xFF, 0xD8, 0xFF])
        || head.starts_with(b"\x89PNG\r\n\x1a\n")
        || tiff::sniff(head)
        || head.starts_with(b"GIF87a")
        || head.starts_with(b"GIF89a")
        || head.starts_with(b"BM")
        || (head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP"))
}

/// Pixel letti e risoluzione dichiarata (se c'è).
struct Raw {
    width: u32,
    height: u32,
    dpi: Option<(f64, f64)>,
    /// L'immagine si mostra ruotata di 90° (orientamento EXIF da 5 a 8).
    rotated: bool,
}

pub(crate) fn probe(src: &mut dyn Source) -> Result<Probe, ProbeError> {
    let head = src.read_prefix(64)?;
    let raw = if head.starts_with(&[0xFF, 0xD8]) {
        jpeg(src)?
    } else if head.starts_with(b"\x89PNG") {
        png(src)?
    } else if tiff::sniff(&head) {
        let t = tiff::read(src, 0)?;
        Raw {
            width: t.width.ok_or(ProbeError::NoSize)?,
            height: t.height.ok_or(ProbeError::NoSize)?,
            dpi: t.dpi,
            rotated: (5..=8).contains(&t.orientation),
        }
    } else if head.starts_with(b"GIF") {
        Raw {
            width: le16(&head, 6).map(u32::from).ok_or(ProbeError::NoSize)?,
            height: le16(&head, 8).map(u32::from).ok_or(ProbeError::NoSize)?,
            dpi: None,
            rotated: false,
        }
    } else if head.starts_with(b"BM") {
        bmp(&head)?
    } else {
        webp(&head)?
    };
    finish(raw)
}

/// Controlla i numeri e calcola le misure.
fn finish(raw: Raw) -> Result<Probe, ProbeError> {
    let Raw {
        mut width,
        mut height,
        dpi,
        rotated,
    } = raw;
    if width == 0 || height == 0 {
        return Err(ProbeError::NoSize);
    }
    if width > MAX_PIXELS || height > MAX_PIXELS {
        return Err(ProbeError::Damaged("dimensioni in pixel non credibili"));
    }
    let valid = |d: f64| (1.0..=MAX_DPI).contains(&d);
    let dpi = dpi.filter(|(x, y)| valid(*x) && valid(*y));
    let (mut dpi_x, mut dpi_y) = dpi.unwrap_or((DEFAULT_DPI, DEFAULT_DPI));
    if rotated {
        std::mem::swap(&mut width, &mut height);
        std::mem::swap(&mut dpi_x, &mut dpi_y);
    }
    let w = px_to_mm(f64::from(width), dpi_x).ok_or(ProbeError::NoSize)?;
    let h = px_to_mm(f64::from(height), dpi_y).ok_or(ProbeError::NoSize)?;
    Ok(Probe {
        pages: vec![PageInfo {
            size: Size::new(w, h),
            trim: None,
        }],
        image: Some(ImageInfo {
            width_px: width,
            height_px: height,
            dpi_x,
            dpi_y,
            dpi_from_file: dpi.is_some(),
        }),
    })
}

/// Risoluzione da un'unità JFIF/PNG: 1 = per pollice, 2 = per centimetro.
fn density(unit: u8, x: f64, y: f64) -> Option<(f64, f64)> {
    match unit {
        1 => Some((x, y)),
        2 => Some((x * 2.54, y * 2.54)),
        _ => None,
    }
}

fn jpeg(src: &mut dyn Source) -> Result<Raw, ProbeError> {
    let mut pos: u64 = 2;
    let mut jfif = None;
    let mut exif: Option<tiff::TiffInfo> = None;
    for _ in 0..MAX_SEGMENTS {
        let mut m = [0u8; 2];
        src.read_exact_at(pos, &mut m)?;
        if m[0] != 0xFF {
            return Err(ProbeError::Damaged("marcatore JPEG mancante"));
        }
        let marker = m[1];
        // Byte di riempimento 0xFF ripetuti: si avanza di uno.
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        // Marcatori senza lunghezza.
        if marker == 0x01 || (0xD0..=0xD8).contains(&marker) {
            pos += 2;
            continue;
        }
        if marker == 0xD9 || marker == 0xDA {
            break;
        }
        let mut l = [0u8; 2];
        src.read_exact_at(pos + 2, &mut l)?;
        let len = u16::from_be_bytes(l);
        if len < 2 {
            return Err(ProbeError::Damaged("segmento JPEG troppo corto"));
        }
        let body = pos + 4;
        let body_len = usize::from(len - 2);
        let is_sof = matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF);
        if is_sof {
            let b = src.read_up_to(body, body_len.min(5))?;
            let height = be16(&b, 1).ok_or(ProbeError::Damaged("intestazione JPEG corta"))?;
            let width = be16(&b, 3).ok_or(ProbeError::Damaged("intestazione JPEG corta"))?;
            // Il JFIF vince sull'EXIF, che spesso resta quello della fotocamera (72 DPI).
            let dpi = jfif.or(exif.and_then(|e| e.dpi));
            let rotated = exif.is_some_and(|e| (5..=8).contains(&e.orientation));
            return Ok(Raw {
                width: u32::from(width),
                height: u32::from(height),
                dpi,
                rotated,
            });
        }
        if marker == 0xE0 && jfif.is_none() {
            let b = src.read_up_to(body, body_len.min(14))?;
            if b.starts_with(b"JFIF\0")
                && let (Some(&unit), Some(x), Some(y)) = (b.get(7), be16(&b, 8), be16(&b, 10))
            {
                jfif = density(unit, f64::from(x), f64::from(y));
            }
        }
        if marker == 0xE1 && exif.is_none() {
            let b = src.read_up_to(body, body_len)?;
            if let Some(mut tiff_bytes) = b.strip_prefix(b"Exif\0\0") {
                // Un EXIF rovinato non impedisce di leggere i pixel.
                exif = tiff::read(&mut tiff_bytes, 0).ok();
            }
        }
        pos = body + u64::from(len - 2);
    }
    Err(ProbeError::NoSize)
}

fn png(src: &mut dyn Source) -> Result<Raw, ProbeError> {
    let ihdr = src.read_up_to(8, 25)?;
    if ihdr.get(4..8) != Some(b"IHDR") {
        return Err(ProbeError::Damaged("PNG senza IHDR"));
    }
    let width = be32(&ihdr, 8).ok_or(ProbeError::NoSize)?;
    let height = be32(&ihdr, 12).ok_or(ProbeError::NoSize)?;
    let mut dpi = None;
    let mut pos: u64 = 8;
    for _ in 0..MAX_SEGMENTS {
        let h = src.read_up_to(pos, 8)?;
        let (Some(len), Some(kind)) = (be32(&h, 0), h.get(4..8)) else {
            break;
        };
        if kind == b"IDAT" || kind == b"IEND" {
            break;
        }
        if kind == b"pHYs" {
            let b = src.read_up_to(pos + 8, 9)?;
            if let (Some(x), Some(y), Some(&1)) = (be32(&b, 0), be32(&b, 4), b.get(8)) {
                // Pixel per metro → per pollice.
                dpi = Some((f64::from(x) * 0.0254, f64::from(y) * 0.0254));
            }
            break;
        }
        pos = pos.saturating_add(12).saturating_add(u64::from(len));
    }
    Ok(Raw {
        width,
        height,
        dpi,
        rotated: false,
    })
}

fn bmp(head: &[u8]) -> Result<Raw, ProbeError> {
    let dib = le32(head, 14).ok_or(ProbeError::Damaged("intestazione BMP corta"))?;
    if dib == 12 {
        return Ok(Raw {
            width: le16(head, 18).map(u32::from).ok_or(ProbeError::NoSize)?,
            height: le16(head, 20).map(u32::from).ok_or(ProbeError::NoSize)?,
            dpi: None,
            rotated: false,
        });
    }
    // Nei BMP l'altezza è negativa quando le righe vanno dall'alto in basso.
    let signed = |at| le32(head, at).map(|v| i32::from_le_bytes(v.to_le_bytes()).unsigned_abs());
    let width = signed(18).ok_or(ProbeError::NoSize)?;
    let height = signed(22).ok_or(ProbeError::NoSize)?;
    let dpi = match (le32(head, 38), le32(head, 42)) {
        (Some(x), Some(y)) if x > 0 && y > 0 => Some((f64::from(x) * 0.0254, f64::from(y) * 0.0254)),
        _ => None,
    };
    Ok(Raw {
        width,
        height,
        dpi,
        rotated: false,
    })
}

fn webp(head: &[u8]) -> Result<Raw, ProbeError> {
    let u24 = |at: usize| -> Option<u32> {
        let b = head.get(at..at + 3)?;
        Some(u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16)
    };
    let (width, height) = match head.get(12..16) {
        Some(b"VP8X") => (u24(24).map(|v| v + 1), u24(27).map(|v| v + 1)),
        Some(b"VP8L") => {
            let bits = le32(head, 21);
            (
                bits.map(|b| (b & 0x3FFF) + 1),
                bits.map(|b| ((b >> 14) & 0x3FFF) + 1),
            )
        }
        Some(b"VP8 ") => (
            le16(head, 26).map(|v| u32::from(v & 0x3FFF)),
            le16(head, 28).map(|v| u32::from(v & 0x3FFF)),
        ),
        _ => (None, None),
    };
    Ok(Raw {
        width: width.ok_or(ProbeError::NoSize)?,
        height: height.ok_or(ProbeError::NoSize)?,
        dpi: None,
        rotated: false,
    })
}
