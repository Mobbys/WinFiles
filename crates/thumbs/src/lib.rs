//! Miniature (livello L1): un'immagine RGBA piccola per l'anteprima a schermo. Le immagini si
//! decodificano con `image`, PDF e AI con `hayro` (prima pagina), gli EPS mostrano l'anteprima
//! TIFF che hanno dentro, se c'è.
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

/// Memoria massima che un decodificatore di immagini può chiedere (1 GiB).
const MAX_DECODE_BYTES: u64 = 1024 * 1024 * 1024;
/// File PDF più grande che si apre per la miniatura.
const MAX_PDF_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Lato massimo di una miniatura.
pub const MAX_SIDE: u32 = 4096;

#[derive(Debug, thiserror::Error)]
pub enum ThumbError {
    #[error("impossibile leggere il file: {0}")]
    Io(#[from] std::io::Error),
    #[error("immagine non leggibile: {0}")]
    Image(#[from] image::ImageError),
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
}

/// Una miniatura: pixel RGBA non premoltiplicati, riga per riga.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumb {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Miniatura del file con il lato lungo di al massimo `max_side` pixel.
pub fn render(path: &Path, kind: FileKind, max_side: u32) -> Result<Thumb, ThumbError> {
    let max_side = max_side.clamp(16, MAX_SIDE);
    catch_unwind(AssertUnwindSafe(|| render_inner(path, kind, max_side))).unwrap_or(Err(ThumbError::Crashed))
}

fn render_inner(path: &Path, kind: FileKind, max_side: u32) -> Result<Thumb, ThumbError> {
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
        return pdf(data, max_side);
    }
    if head.starts_with(&[0xC5, 0xD0, 0xD3, 0xC6]) {
        return eps_preview(&mut file, head, max_side);
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
    let small = if img.width() > max_side || img.height() > max_side {
        img.thumbnail(max_side, max_side)
    } else {
        img
    };
    let rgba = small.to_rgba8();
    Ok(Thumb {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

fn pdf(data: Vec<u8>, max_side: u32) -> Result<Thumb, ThumbError> {
    let pdf = Pdf::new(data).map_err(|e| match e {
        LoadPdfError::Decryption(_) => ThumbError::Encrypted,
        LoadPdfError::Invalid => ThumbError::Pdf,
    })?;
    let page = pdf.pages().first().ok_or(ThumbError::Pdf)?;
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
