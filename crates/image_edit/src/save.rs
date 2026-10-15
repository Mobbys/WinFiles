//! Salvataggio: JPEG, PNG, TIFF e WebP, con la risoluzione e il profilo ICC dentro al file.
//!
//! Il codificatore di `image` non scrive la risoluzione: i JPEG escono con la densità "nessuna
//! unità" e i PNG senza `pHYs`, e un file senza risoluzione in tipografia si stampa alla misura
//! sbagliata. Qui la si scrive: nel JPEG si corregge il blocco JFIF, nel PNG si aggiunge il
//! `pHYs`, nel TIFF si scrivono i campi `XResolution` e `YResolution`. Il profilo ICC lo scrive
//! `image` dove può (JPEG, PNG, WebP) e noi nel TIFF.
//!
//! La scrittura è atomica: prima un file temporaneo accanto, poi il rinomino. Se la corrente va
//! via a metà, l'originale è ancora lì.

use std::io::Cursor;
use std::path::Path;

use image::{DynamicImage, ImageEncoder};

use crate::{ImageEditError, Picture};

/// I formati in cui si può salvare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    Tiff,
    WebP,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Jpeg => "jpg",
            Format::Png => "png",
            Format::Tiff => "tif",
            Format::WebP => "webp",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Jpeg => "JPEG",
            Format::Png => "PNG",
            Format::Tiff => "TIFF",
            Format::WebP => "WebP",
        }
    }

    /// Il formato che l'estensione del nome suggerisce.
    pub fn from_extension(name: &str) -> Option<Self> {
        let ext = Path::new(name).extension()?.to_string_lossy().to_lowercase();
        match ext.as_str() {
            "jpg" | "jpeg" | "jpe" => Some(Format::Jpeg),
            "png" => Some(Format::Png),
            "tif" | "tiff" => Some(Format::Tiff),
            "webp" => Some(Format::WebP),
            _ => None,
        }
    }

    /// Vero se il formato tiene la trasparenza: nel JPEG il trasparente diventa bianco.
    pub fn keeps_alpha(self) -> bool {
        !matches!(self, Format::Jpeg)
    }

    /// Vero se nel file si può scrivere la risoluzione.
    pub fn keeps_dpi(self) -> bool {
        !matches!(self, Format::WebP)
    }
}

/// Le scelte del salvataggio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveOptions {
    pub format: Format,
    /// Qualità del JPEG, da 1 a 100.
    pub quality: u8,
    /// Scrive il profilo ICC di partenza nel file nuovo.
    pub keep_icc: bool,
}

impl Default for SaveOptions {
    fn default() -> Self {
        Self {
            format: Format::Jpeg,
            quality: 92,
            keep_icc: true,
        }
    }
}

/// Scrive l'immagine in `path`.
pub fn write(picture: &Picture, path: &Path, options: SaveOptions) -> Result<(), ImageEditError> {
    let data = encode(picture, options)?;
    write_atomic(path, &data)
}

/// L'immagine codificata in memoria (serve anche ai test e all'anteprima del peso).
pub fn encode(picture: &Picture, options: SaveOptions) -> Result<Vec<u8>, ImageEditError> {
    let icc = options.keep_icc.then(|| picture.icc()).flatten();
    let dpi = picture.dpi();
    match options.format {
        Format::Jpeg => {
            let image = flatten(picture.image());
            let mut out = Vec::new();
            let quality = options.quality.clamp(1, 100);
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
            if let Some(icc) = icc {
                // Un profilo troppo grande per i blocchi APP2 si lascia fuori: meglio senza che
                // un file rotto.
                let _ = encoder.set_icc_profile(icc.to_vec());
            }
            encoder.encode_image(&image)?;
            set_jpeg_dpi(&mut out, dpi);
            Ok(out)
        }
        Format::Png => {
            let image = picture.image();
            let mut out = Vec::new();
            let mut encoder = image::codecs::png::PngEncoder::new(&mut out);
            if let Some(icc) = icc {
                let _ = encoder.set_icc_profile(icc.to_vec());
            }
            encoder.write_image(
                image.as_bytes(),
                image.width(),
                image.height(),
                image.color().into(),
            )?;
            insert_png_phys(&mut out, dpi);
            Ok(out)
        }
        Format::WebP => {
            let image = DynamicImage::ImageRgba8(picture.image().to_rgba8());
            let mut out = Vec::new();
            let encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
            encoder.write_image(
                image.as_bytes(),
                image.width(),
                image.height(),
                image.color().into(),
            )?;
            Ok(out)
        }
        Format::Tiff => tiff_bytes(picture, icc, dpi),
    }
}

/// Il JPEG non ha la trasparenza: il trasparente diventa bianco, come in stampa.
fn flatten(image: &DynamicImage) -> DynamicImage {
    if !image.color().has_alpha() {
        return image.clone();
    }
    let rgba = image.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = f32::from(p.0[3]) / 255.0;
        let mix = |c: u8| (f32::from(c) * a + 255.0 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
        rgb.put_pixel(x, y, image::Rgb([mix(p.0[0]), mix(p.0[1]), mix(p.0[2])]));
    }
    DynamicImage::ImageRgb8(rgb)
}

/// Corregge la densità nel blocco JFIF del JPEG appena scritto: unità "punti per pollice" e i
/// due valori. Senza questo il file dice "nessuna unità" e chi lo apre non sa quanto misura.
fn set_jpeg_dpi(data: &mut [u8], dpi: f64) {
    let value = dpi.round().clamp(1.0, 65535.0) as u16;
    let mut i = 2; // dopo SOI
    while i + 4 <= data.len() {
        let Some(marker) = data.get(i..i + 2) else { break };
        if marker.first() != Some(&0xFF) {
            break;
        }
        let Some(len) = data.get(i + 2..i + 4).map(|b| u16::from_be_bytes([b[0], b[1]])) else {
            break;
        };
        let len = usize::from(len);
        if marker.get(1) == Some(&0xE0) && data.get(i + 4..i + 9) == Some(b"JFIF\0") {
            // unità a +11, Xdensity a +12, Ydensity a +14 dall'inizio del marcatore.
            if let Some(slot) = data.get_mut(i + 11..i + 16) {
                slot[0] = 1;
                slot[1..3].copy_from_slice(&value.to_be_bytes());
                slot[3..5].copy_from_slice(&value.to_be_bytes());
            }
            return;
        }
        // SOS: da qui in poi ci sono i dati compressi, non altri blocchi.
        if marker.get(1) == Some(&0xDA) {
            return;
        }
        i = match i.checked_add(2).and_then(|i| i.checked_add(len)) {
            Some(next) if next > i => next,
            _ => return,
        };
    }
}

/// Aggiunge al PNG il riquadro `pHYs` con i pixel per metro, subito dopo `IHDR`.
fn insert_png_phys(data: &mut Vec<u8>, dpi: f64) {
    // 8 byte di firma, poi IHDR: 4 di lunghezza + 4 di nome + 13 di dati + 4 di CRC.
    const AFTER_IHDR: usize = 8 + 4 + 4 + 13 + 4;
    if data.len() < AFTER_IHDR || data.get(12..16) != Some(b"IHDR") {
        return;
    }
    let per_metre = (dpi / 0.0254).round().clamp(1.0, f64::from(u32::MAX)) as u32;
    let mut chunk = Vec::with_capacity(21);
    chunk.extend_from_slice(&9u32.to_be_bytes());
    let mut body = Vec::with_capacity(13);
    body.extend_from_slice(b"pHYs");
    body.extend_from_slice(&per_metre.to_be_bytes());
    body.extend_from_slice(&per_metre.to_be_bytes());
    body.push(1); // unità: metri
    chunk.extend_from_slice(&body);
    chunk.extend_from_slice(&crc32(&body).to_be_bytes());
    data.splice(AFTER_IHDR..AFTER_IHDR, chunk);
}

/// Il CRC-32 dei PNG (polinomio 0xEDB88320), venti righe invece di una dipendenza.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Il TIFF, scritto con la risoluzione e il profilo ICC. `image` non scrive né l'una né l'altro,
/// quindi qui si usa direttamente la libreria `tiff` (la stessa che `image` usa dentro).
fn tiff_bytes(picture: &Picture, icc: Option<&[u8]>, dpi: f64) -> Result<Vec<u8>, ImageEditError> {
    use tiff::encoder::{TiffEncoder, colortype};
    use tiff::tags::{ResolutionUnit, Tag};

    let image = picture.image();
    let (w, h) = (image.width(), image.height());
    let mut out = Cursor::new(Vec::new());
    let mut encoder =
        TiffEncoder::new(&mut out).map_err(|_| ImageEditError::Unsupported("TIFF non scritto"))?;

    /// I campi uguali per tutti i tipi di pixel: risoluzione e profilo colore.
    fn common<W, C, K>(
        image: &mut tiff::encoder::ImageEncoder<'_, W, C, K>,
        dpi: f64,
        icc: Option<&[u8]>,
    ) -> Result<(), ImageEditError>
    where
        W: std::io::Write + std::io::Seek,
        C: tiff::encoder::colortype::ColorType,
        K: tiff::encoder::TiffKind,
    {
        let fail = |_| ImageEditError::Unsupported("TIFF non scritto");
        let value = || tiff::encoder::Rational {
            n: (dpi * 1000.0).round().clamp(1.0, f64::from(u32::MAX)) as u32,
            d: 1000,
        };
        image.resolution_unit(ResolutionUnit::Inch);
        image.x_resolution(value());
        image.y_resolution(value());
        if let Some(icc) = icc {
            // 34675 è il campo del profilo ICC (ICC Profile, TIFF 6.0 supplemento 1).
            image
                .encoder()
                .write_tag(Tag::Unknown(34675), icc)
                .map_err(fail)?;
        }
        Ok(())
    }

    let fail = |_| ImageEditError::Unsupported("TIFF non scritto");
    match image {
        DynamicImage::ImageLuma8(buf) => {
            let mut img = encoder.new_image::<colortype::Gray8>(w, h).map_err(fail)?;
            common(&mut img, dpi, icc)?;
            img.write_data(buf.as_raw()).map_err(fail)?;
        }
        DynamicImage::ImageLuma16(buf) => {
            let mut img = encoder.new_image::<colortype::Gray16>(w, h).map_err(fail)?;
            common(&mut img, dpi, icc)?;
            img.write_data(buf.as_raw()).map_err(fail)?;
        }
        DynamicImage::ImageRgba8(buf) => {
            let mut img = encoder.new_image::<colortype::RGBA8>(w, h).map_err(fail)?;
            common(&mut img, dpi, icc)?;
            img.write_data(buf.as_raw()).map_err(fail)?;
        }
        DynamicImage::ImageRgba16(buf) => {
            let mut img = encoder.new_image::<colortype::RGBA16>(w, h).map_err(fail)?;
            common(&mut img, dpi, icc)?;
            img.write_data(buf.as_raw()).map_err(fail)?;
        }
        DynamicImage::ImageRgb16(buf) => {
            let mut img = encoder.new_image::<colortype::RGB16>(w, h).map_err(fail)?;
            common(&mut img, dpi, icc)?;
            img.write_data(buf.as_raw()).map_err(fail)?;
        }
        other => {
            // Tutto il resto (compresi i grigi con trasparenza e i 32 bit) va in RGB a 8 bit.
            let buf = other.to_rgb8();
            let mut img = encoder.new_image::<colortype::RGB8>(w, h).map_err(fail)?;
            common(&mut img, dpi, icc)?;
            img.write_data(buf.as_raw()).map_err(fail)?;
        }
    }
    Ok(out.into_inner())
}

/// Scrive `data` in `path` passando da un file temporaneo: o c'è il file nuovo o c'è il vecchio,
/// mai mezzo file.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<(), ImageEditError> {
    let tmp = path.with_extension("winfiles-tmp");
    std::fs::write(&tmp, data)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e.into())
        }
    }
}
