//! Miniature (livello L1): un'immagine RGBA piccola per l'anteprima a schermo. Le immagini si
//! decodificano con `image`, le foto HEIC con il lettore di PhotoCraft, PDF e AI con `hayro`
//! (prima pagina), gli EPS mostrano l'anteprima TIFF che hanno dentro, se c'è. Gli AI in
//! PostScript ("legacy") e gli EPS di Illustrator senza anteprima si disegnano traducendo le loro
//! forme in PDF (`legacy_ai`).
//!
//! I decodificatori sono codice di terzi che legge file ostili: ogni chiamata ha tetti di memoria
//! e passa da `catch_unwind`, così un file rovinato dà un errore e non chiude l'app.

use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{LoadPdfError, Pdf};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use image::{DynamicImage, ImageDecoder, ImageReader, Limits};
use winfiles_model::FileKind;

mod legacy_ai;

/// Memoria massima che un decodificatore di immagini può chiedere (1 GiB).
const MAX_DECODE_BYTES: u64 = 1024 * 1024 * 1024;
/// File PDF più grande che si apre per la miniatura.
const MAX_PDF_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Foto HEIC più grande che si apre per la miniatura.
const MAX_HEIF_BYTES: u64 = 512 * 1024 * 1024;
/// Pixel massimi di una foto HEIC: a 16 bit con trasparenza sono 8 byte l'uno, sotto
/// [`MAX_DECODE_BYTES`].
const MAX_HEIF_PIXELS: u64 = 120_000_000;
/// Lato massimo di una miniatura.
pub const MAX_SIDE: u32 = 4096;

#[derive(Debug, thiserror::Error)]
pub enum ThumbError {
    #[error("{}", io_text(.0))]
    Io(#[from] std::io::Error),
    #[error("immagine non leggibile: {0}")]
    Image(#[from] image::ImageError),
    #[error("{}", heif_text(.0))]
    Heif(photocraft_heif::Error),
    #[error("PDF non leggibile")]
    Pdf,
    #[error("il PDF è protetto da password")]
    Encrypted,
    #[error("il file è troppo grande ({0} byte)")]
    TooLarge(u64),
    #[error("il file non contiene un'anteprima")]
    NoPreview,
    #[error("il decodificatore si è fermato su un errore interno")]
    Crashed,
    #[error("il file non ha la pagina {0}")]
    NoPage(usize),
}

/// Gli errori più comuni detti in italiano: quelli del sistema arrivano nella lingua di Windows.
fn io_text(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "file non trovato".into(),
        std::io::ErrorKind::PermissionDenied => "accesso al file negato".into(),
        _ => format!("impossibile leggere il file: {e}"),
    }
}

fn heif_text(e: &photocraft_heif::Error) -> &'static str {
    match e {
        photocraft_heif::Error::Unsupported(_) => {
            "foto HEIC senza un'immagine fissa (forse una sequenza o un video)"
        }
        photocraft_heif::Error::Limit(_) => "foto HEIC troppo grande",
        photocraft_heif::Error::Malformed(_) => "foto HEIC danneggiata",
    }
}

/// Una miniatura: pixel RGBA non premoltiplicati, riga per riga.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumb {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Miniatura della pagina `page` (da 0) del file, con il lato lungo di al massimo `max_side`
/// pixel. Immagini ed EPS hanno solo la pagina 0.
pub fn render(path: &Path, kind: FileKind, page: usize, max_side: u32) -> Result<Thumb, ThumbError> {
    let max_side = max_side.clamp(16, MAX_SIDE);
    catch_unwind(AssertUnwindSafe(|| render_inner(path, kind, page, max_side)))
        .unwrap_or(Err(ThumbError::Crashed))
}

fn render_inner(path: &Path, kind: FileKind, page: usize, max_side: u32) -> Result<Thumb, ThumbError> {
    let mut file = std::fs::File::open(path)?;
    let mut head = [0u8; 1024];
    let n = read_up_to(&mut file, &mut head)?;
    let head = head.get(..n).unwrap_or_default();
    if head.windows(5).any(|w| w == b"%PDF-") {
        let len = file.metadata()?.len();
        if len > MAX_PDF_BYTES {
            return Err(ThumbError::TooLarge(len));
        }
        file.seek(SeekFrom::Start(0))?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        return pdf(data, page, max_side);
    }
    if page > 0 {
        return Err(ThumbError::NoPage(page + 1));
    }
    if head.starts_with(&[0xC5, 0xD0, 0xD3, 0xC6]) {
        return match eps_preview(&mut file, head, max_side) {
            Err(ThumbError::NoPreview) => illustrator(&mut file, dos_section(head), max_side),
            other => other,
        };
    }
    if head.starts_with(b"%!") && legacy_ai::is_illustrator(head) {
        let len = file.metadata()?.len();
        return illustrator(&mut file, Some((0, len)), max_side);
    }
    if is_heif(head) {
        let len = file.metadata()?.len();
        if len > MAX_HEIF_BYTES {
            return Err(ThumbError::TooLarge(len));
        }
        file.seek(SeekFrom::Start(0))?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        return heif(&data, max_side);
    }
    match kind {
        FileKind::Image => {
            file.seek(SeekFrom::Start(0))?;
            image_thumb(
                ImageReader::new(BufReader::new(file)).with_guessed_format()?,
                max_side,
            )
        }
        _ => Err(ThumbError::NoPreview),
    }
}

/// Il contenitore di una foto HEIC/HEIF (marchi HEVC nel riquadro `ftyp`).
fn is_heif(head: &[u8]) -> bool {
    const BRANDS: [&[u8; 4]; 8] = [
        b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx", b"mif1", b"msf1",
    ];
    head.get(4..8) == Some(b"ftyp")
        && head
            .get(8..12)
            .is_some_and(|b| BRANDS.iter().any(|m| m.as_slice() == b))
}

/// Miniatura di una foto HEIC, già girata come la mostra il telefono.
fn heif(data: &[u8], max_side: u32) -> Result<Thumb, ThumbError> {
    let options = photocraft_heif::Options {
        max_pixels: MAX_HEIF_PIXELS,
        apply_transforms: true,
    };
    let decoded = photocraft_heif::decode(data, &options).map_err(ThumbError::Heif)?;
    let samples = if decoded.sixteen_bit {
        decoded
            .data
            .chunks_exact(2)
            .map(|b| {
                let v = <[u8; 2]>::try_from(b).map(u16::from_ne_bytes).unwrap_or_default();
                (v >> 8) as u8
            })
            .collect()
    } else {
        decoded.data
    };
    let broken = || ThumbError::Heif(photocraft_heif::Error::Malformed("pixel mancanti".into()));
    let img = if decoded.has_alpha {
        DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(decoded.width, decoded.height, samples).ok_or_else(broken)?,
        )
    } else {
        DynamicImage::ImageRgb8(
            image::RgbImage::from_raw(decoded.width, decoded.height, samples).ok_or_else(broken)?,
        )
    };
    Ok(shrink(img, max_side))
}

fn read_up_to(file: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match file.read(buf.get_mut(n..).unwrap_or_default())? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

fn image_thumb<R: std::io::BufRead + Seek>(
    mut reader: ImageReader<R>,
    max_side: u32,
) -> Result<Thumb, ThumbError> {
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    limits.max_image_width = Some(1_000_000);
    limits.max_image_height = Some(1_000_000);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    // L'orientamento EXIF: le foto girate in verticale si mostrano dritte, come le misure.
    let orientation = decoder.orientation()?;
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    Ok(shrink(img, max_side))
}

/// L'immagine ridotta a `max_side` pixel sul lato lungo, in RGBA.
fn shrink(img: DynamicImage, max_side: u32) -> Thumb {
    let small = if img.width() > max_side || img.height() > max_side {
        img.thumbnail(max_side, max_side)
    } else {
        img
    };
    let rgba = small.to_rgba8();
    Thumb {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    }
}

fn pdf(data: Vec<u8>, page: usize, max_side: u32) -> Result<Thumb, ThumbError> {
    PdfPages::open(data)?.render_inner(page, max_side)
}

/// Un PDF già letto, per disegnarne più pagine senza rileggerlo (modulo PDF).
pub struct PdfPages(Pdf);

impl PdfPages {
    pub fn open(data: Vec<u8>) -> Result<Self, ThumbError> {
        catch_unwind(AssertUnwindSafe(|| {
            Pdf::new(data).map(Self).map_err(|e| match e {
                LoadPdfError::Decryption(_) => ThumbError::Encrypted,
                LoadPdfError::Invalid => ThumbError::Pdf,
            })
        }))
        .unwrap_or(Err(ThumbError::Crashed))
    }

    pub fn len(&self) -> usize {
        self.0.pages().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// La pagina `page` (da 0) con il lato lungo di al massimo `max_side` pixel.
    pub fn render(&self, page: usize, max_side: u32) -> Result<Thumb, ThumbError> {
        let max_side = max_side.clamp(16, MAX_SIDE);
        catch_unwind(AssertUnwindSafe(|| self.render_inner(page, max_side)))
            .unwrap_or(Err(ThumbError::Crashed))
    }

    fn render_inner(&self, page: usize, max_side: u32) -> Result<Thumb, ThumbError> {
        let page = self.0.pages().get(page).ok_or(ThumbError::NoPage(page + 1))?;
        let (w, h) = page.render_dimensions();
        let long = w.max(h);
        if !(long.is_finite() && long > 0.0) {
            return Err(ThumbError::Pdf);
        }
        let scale = (max_side as f32 / long).min(8.0);
        let cache = RenderCache::new();
        let pixmap = hayro::render(
            page,
            &cache,
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
            },
        );
        let (width, height) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
        if width == 0 || height == 0 {
            return Err(ThumbError::Pdf);
        }
        // Sfondo bianco opaco: i pixel premoltiplicati coincidono con quelli normali.
        Ok(Thumb {
            width,
            height,
            rgba: pixmap.data_as_u8_slice().to_vec(),
        })
    }
}

/// Inizio e lunghezza della parte PostScript di un EPS con intestazione DOS.
fn dos_section(head: &[u8]) -> Option<(u64, u64)> {
    let field = |at: usize| {
        head.get(at..at + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
    };
    Some((u64::from(field(4)?), u64::from(field(8)?)))
}

/// Il disegno di un file PostScript di Illustrator (`section`: inizio e lunghezza), tradotto in
/// PDF; gli altri PostScript non hanno anteprima.
fn illustrator(
    file: &mut std::fs::File,
    section: Option<(u64, u64)>,
    max_side: u32,
) -> Result<Thumb, ThumbError> {
    let (start, len) = section.ok_or(ThumbError::NoPreview)?;
    if len > MAX_PDF_BYTES {
        return Err(ThumbError::TooLarge(len));
    }
    file.seek(SeekFrom::Start(start))?;
    let mut ps = Vec::new();
    file.take(len).read_to_end(&mut ps)?;
    if !legacy_ai::is_illustrator(&ps) {
        return Err(ThumbError::NoPreview);
    }
    let pdf_data = legacy_ai::to_pdf(&ps).ok_or(ThumbError::NoPreview)?;
    pdf(pdf_data, 0, max_side)
}

/// L'anteprima TIFF di un EPS con intestazione DOS (offset a 20, lunghezza a 24).
fn eps_preview(file: &mut std::fs::File, head: &[u8], max_side: u32) -> Result<Thumb, ThumbError> {
    let field = |at: usize| {
        head.get(at..at + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
    };
    let (Some(start), Some(len)) = (field(20), field(24)) else {
        return Err(ThumbError::NoPreview);
    };
    if start == 0 || len == 0 {
        return Err(ThumbError::NoPreview);
    }
    if u64::from(len) > MAX_DECODE_BYTES {
        return Err(ThumbError::TooLarge(u64::from(len)));
    }
    file.seek(SeekFrom::Start(u64::from(start)))?;
    let mut data = Vec::new();
    file.take(u64::from(len)).read_to_end(&mut data)?;
    image_thumb(
        ImageReader::new(Cursor::new(data)).with_guessed_format()?,
        max_side,
    )
}

#[cfg(test)]
mod tests;
