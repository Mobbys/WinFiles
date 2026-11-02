//! Foto RAW delle fotocamere e file di Photoshop (livello L1): li trasforma in immagini da
//! mostrare, misurare e modificare, con le librerie di PhotoCraft (ADR 0017).
//!
//! - **RAW** (DNG, CR2, NEF, ARW, PEF, ORF, RW2, RAF): si sviluppano dai dati del sensore in
//!   ProPhoto a 16 bit e si convertono in sRGB, che è quello che si vede a schermo e si salva.
//!   Dove lo sviluppo non c'è ancora (i CR3 di Canon) si usa l'anteprima a piena misura che la
//!   fotocamera mette nel file. Per le miniature basta sempre l'anteprima: è molto più veloce.
//! - **PSD e PSB**: si legge l'immagine unita che Photoshop salva accanto ai livelli, con il suo
//!   profilo ICC e la sua risoluzione. I CMYK si convertono in sRGB con il profilo del file.
//!
//! I file sono ostili: ogni misura ha un tetto, le librerie non usano `unsafe`, e un errore
//! interno di un lettore diventa un `Result`, mai un crash.

use std::panic::{AssertUnwindSafe, catch_unwind};

use image::metadata::Orientation;
use image::{DynamicImage, ImageBuffer, ImageDecoder, ImageFormat, ImageReader, Limits};
use photocraft_cms::builtin::{self, Builtin};
use photocraft_cms::{ColorSpace, Intent, Profile, Transform};
use photocraft_psd::{ColorMode as PsdMode, PsdFile};
use photocraft_raw::{DevelopOptions, RawError};

#[cfg(test)]
mod tests;

/// Pixel massimi di un'immagine letta: a 16 bit per tre canali sono 1,5 GiB.
pub const MAX_PIXELS: u64 = 268_000_000;
/// Lato massimo, in pixel.
pub const MAX_SIDE: u32 = 200_000;

/// Estensioni delle foto RAW riconosciute.
pub const RAW_EXTENSIONS: &[&str] = photocraft_raw::EXTENSIONS;
/// Estensioni dei file di Photoshop.
pub const PSD_EXTENSIONS: &[&str] = &["psd", "psb"];

#[derive(Debug, thiserror::Error)]
pub enum PhotoError {
    #[error("{0}")]
    Raw(&'static str),
    #[error("file di Photoshop non leggibile: {0}")]
    Psd(String),
    #[error(
        "il file di Photoshop è stato salvato senza «Massimizza compatibilità»: manca l'immagine \
         unita da mostrare. Riaprilo in Photoshop e salvalo con quell'opzione"
    )]
    NoComposite,
    #[error("{0}")]
    Unsupported(&'static str),
    #[error("immagine troppo grande: {0} pixel")]
    TooManyPixels(u64),
    #[error("conversione dei colori non riuscita")]
    Color,
    #[error("anteprima della fotocamera non leggibile: {0}")]
    Preview(#[from] image::ImageError),
    #[error("il lettore si è fermato su un errore interno")]
    Crashed,
}

/// Che file è.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Raw,
    Psd,
}

/// Riconosce una foto RAW o un file di Photoshop dai byte, non dall'estensione. Un RAW comincia
/// come un TIFF qualsiasi: va riconosciuto prima di passarlo a un lettore di TIFF, che
/// mostrerebbe solo la piccola miniatura che la fotocamera mette in testa.
pub fn sniff(data: &[u8]) -> Option<Kind> {
    if is_psd(data) {
        return Some(Kind::Psd);
    }
    let raw = catch_unwind(|| photocraft_raw::identify(data)).ok().flatten();
    raw.map(|_| Kind::Raw)
}

/// L'intestazione di un PSD (versione 1) o di un PSB (versione 2).
pub fn is_psd(head: &[u8]) -> bool {
    head.starts_with(b"8BPS") && matches!(head.get(4..6), Some([0, 1] | [0, 2]))
}

/// I colori del file di partenza.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Gray,
    Rgb,
    Cmyk,
    Indexed,
}

/// Un'immagine letta, con quello che il file dice di sé.
pub struct Photo {
    pub image: DynamicImage,
    /// Il profilo ICC dei pixel di `image` (sRGB per i RAW e per i CMYK convertiti).
    pub icc: Option<Vec<u8>>,
    /// La risoluzione scritta nel file, se c'è e se il file la porta qui (i PSD); per i RAW la
    /// legge `winfiles-formats` dalla testa del TIFF.
    pub dpi: Option<f64>,
    /// I colori del file di partenza.
    pub mode: Mode,
    /// Bit per canale del file di partenza.
    pub bits: u16,
    /// Da dove vengono i pixel, da mostrare a chi guarda: "RAW NEF sviluppato", "PSD, 12 livelli".
    pub origin: String,
}

/// Quanto lavoro fare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    /// Tutti i pixel: i RAW si sviluppano. Per modificare.
    Full,
    /// Quanto basta per una miniatura: dei RAW si prende l'anteprima della fotocamera.
    Preview,
}

/// Legge una foto RAW o un file di Photoshop.
pub fn decode(data: &[u8], quality: Quality) -> Result<Photo, PhotoError> {
    catch_unwind(AssertUnwindSafe(|| match sniff(data) {
        Some(Kind::Psd) => psd(data),
        Some(Kind::Raw) => raw(data, quality),
        None => Err(PhotoError::Unsupported(
            "non è una foto RAW né un file di Photoshop",
        )),
    }))
    .unwrap_or(Err(PhotoError::Crashed))
}

// ---------------------------------------------------------------------------------------------
// RAW

fn raw(data: &[u8], quality: Quality) -> Result<Photo, PhotoError> {
    let name = photocraft_raw::identify(data).map(|f| f.name()).unwrap_or("RAW");
    if quality == Quality::Preview
        && let Ok(photo) = raw_preview(data, name)
    {
        return Ok(photo);
    }
    let limits = photocraft_raw::Limits {
        max_width: MAX_SIDE,
        max_height: MAX_SIDE,
        max_pixels: MAX_PIXELS,
        ..photocraft_raw::Limits::default()
    };
    let sensor = match photocraft_raw::decode(data, &limits) {
        Ok(sensor) => sensor,
        // Formati riconosciuti ma non ancora sviluppati (i CR3): l'anteprima della fotocamera.
        Err(e @ RawError::Unsupported(_)) => return raw_preview(data, name).map_err(|_| raw_error(&e)),
        Err(e) => return Err(raw_error(&e)),
    };
    let mut options = DevelopOptions {
        limits,
        ..DevelopOptions::default()
    };
    // I RAW senza descrizione del colore (tutti tranne i DNG) prendono i colori e la curva
    // dalla foto JPEG che la fotocamera stessa ha messo nel file: così la foto sviluppata
    // somiglia a quella vista sul display della macchina, non a un negativo spento.
    if sensor.color.calibrations.is_empty() && !sensor.is_monochrome() {
        options.camera_look = camera_look(data, &sensor, &options);
    }
    let developed = photocraft_raw::develop_sensor(&sensor, &options).map_err(|e| raw_error(&e))?;
    drop(sensor);
    developed_photo(developed, name)
}

/// I colori della fotocamera, ricavati dalla sua anteprima JPEG (`photocraft_raw::fit_look`).
/// Basta un'anteprima piccola: si sceglie la più piccola che abbia almeno il doppio dei
/// riquadri su cui si calcola.
fn camera_look(
    data: &[u8],
    sensor: &photocraft_raw::Sensor,
    options: &DevelopOptions,
) -> Option<photocraft_raw::CameraLook> {
    let previews = photocraft_raw::embedded_previews(data);
    let long = |p: &photocraft_raw::Preview| p.width.max(p.height);
    let edge = 2 * photocraft_raw::PROXY_EDGE as u32;
    let pick = previews
        .iter()
        .filter(|p| long(p) >= edge)
        .min_by_key(|p| long(p))
        .or_else(|| previews.iter().max_by_key(|p| long(p)))?;
    check_size(pick.width, pick.height).ok()?;
    let (image, icc) = upright(data, pick.jpeg).ok()?;
    let mut rgb = image.to_rgb8();
    let adobe = photocraft_raw::embedded_preview_color_space(data)
        == Some(photocraft_raw::PreviewColorSpace::AdobeRgb);
    let profile = match icc {
        Some(bytes) => Some(std::borrow::Cow::Owned(Profile::parse(&bytes).ok()?)),
        None if adobe => Some(std::borrow::Cow::Borrowed(Builtin::AdobeRgbCompat.profile())),
        None => None,
    };
    if let Some(profile) = profile.filter(|p| !p.same_colors(builtin::srgb())) {
        let t = Transform::new(&profile, builtin::srgb(), Intent::RelativeColorimetric, true).ok()?;
        if t.inputs() != 3 || t.outputs() != 3 {
            return None;
        }
        let mut out = vec![0u8; rgb.as_raw().len()];
        t.convert_u8(rgb.as_raw(), 3, &mut out, 3, false);
        rgb = ImageBuffer::from_raw(rgb.width(), rgb.height(), out)?;
    }
    let reference = photocraft_raw::Reference {
        width: rgb.width(),
        height: rgb.height(),
        rgb: rgb.as_raw(),
    };
    photocraft_raw::fit_look(sensor, options, &reference).ok()
}

/// Un'anteprima JPEG della fotocamera, dritta: con il suo orientamento se ne ha uno, se no con
/// quello del RAW. Dà anche il suo profilo ICC.
fn upright(data: &[u8], jpeg: &[u8]) -> Result<(DynamicImage, Option<Vec<u8>>), PhotoError> {
    let mut reader = ImageReader::with_format(std::io::Cursor::new(jpeg), ImageFormat::Jpeg);
    reader.limits(limits());
    let mut decoder = reader.into_decoder()?;
    let own = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let icc = decoder.icc_profile().ok().flatten().filter(|p| !p.is_empty());
    let mut image = DynamicImage::from_decoder(decoder)?;
    let orientation = if own == Orientation::NoTransforms {
        photocraft_raw::orientation(data)
            .and_then(|o| u8::try_from(o).ok())
            .and_then(Orientation::from_exif)
            .unwrap_or(Orientation::NoTransforms)
    } else {
        own
    };
    image.apply_orientation(orientation);
    Ok((image, icc))
}

fn raw_error(e: &RawError) -> PhotoError {
    PhotoError::Raw(match e {
        RawError::NotRaw => "non è una foto RAW",
        RawError::Unsupported(_) => "questo tipo di foto RAW non si legge ancora",
        RawError::Malformed(_) => "foto RAW danneggiata",
        RawError::LimitExceeded(_) => "foto RAW troppo grande",
    })
}

/// Lo sviluppo esce in ProPhoto RGB a 16 bit: si porta in sRGB (intento colorimetrico relativo
/// con compensazione del punto nero), che a schermo si vede giusto e in stampa è quello che ci
/// si aspetta da una foto senza profilo.
fn developed_photo(d: photocraft_raw::Developed, name: &str) -> Result<Photo, PhotoError> {
    check_size(d.width, d.height)?;
    let transform = Transform::new(
        Builtin::ProPhotoCompat.profile(),
        builtin::srgb(),
        Intent::RelativeColorimetric,
        true,
    )
    .map_err(|_| PhotoError::Color)?;
    if transform.inputs() != 3 || transform.outputs() != 3 {
        return Err(PhotoError::Color);
    }
    let expected = (d.width as usize)
        .checked_mul(d.height as usize)
        .and_then(|n| n.checked_mul(3))
        .ok_or(PhotoError::Raw("foto RAW troppo grande"))?;
    if d.rgb.len() != expected {
        return Err(PhotoError::Raw("foto RAW danneggiata"));
    }
    let mut rgb = vec![0u16; expected];
    transform.convert_u16(&d.rgb, 3, &mut rgb, 3, false);
    let image = ImageBuffer::<image::Rgb<u16>, _>::from_raw(d.width, d.height, rgb)
        .ok_or(PhotoError::Raw("foto RAW danneggiata"))?;
    let camera = match (d.info.make.as_deref(), d.info.model.as_deref()) {
        (_, Some(model)) => format!(", {}", model.trim()),
        (Some(make), None) => format!(", {}", make.trim()),
        _ => String::new(),
    };
    Ok(Photo {
        image: DynamicImage::ImageRgb16(image),
        icc: Some(srgb_icc()),
        dpi: None,
        mode: Mode::Rgb,
        bits: 16,
        origin: format!("RAW {name} sviluppato{camera}"),
    })
}

/// L'anteprima JPEG più grande che la fotocamera ha messo nel file, girata come la foto.
fn raw_preview(data: &[u8], name: &str) -> Result<Photo, PhotoError> {
    let preview =
        photocraft_raw::embedded_preview(data).ok_or(PhotoError::Raw("la foto RAW non ha un'anteprima"))?;
    check_size(preview.width, preview.height)?;
    let (image, icc) = upright(data, preview.jpeg)?;
    Ok(Photo {
        image,
        icc,
        dpi: None,
        mode: Mode::Rgb,
        bits: 8,
        origin: format!("RAW {name}, anteprima della fotocamera"),
    })
}

// ---------------------------------------------------------------------------------------------
// PSD e PSB

fn psd(data: &[u8]) -> Result<Photo, PhotoError> {
    let file = PsdFile::from_bytes(data).map_err(|e| PhotoError::Psd(e.to_string()))?;
    let header = &file.header;
    check_size(header.width, header.height)?;
    if file.has_real_merged_data() == Some(false) {
        return Err(PhotoError::NoComposite);
    }
    let kind = if header.version.is_psb() { "PSB" } else { "PSD" };
    let layers = file.layers().len();
    let mut origin = match layers {
        0 => kind.to_string(),
        1 => format!("{kind}, 1 livello"),
        n => format!("{kind}, {n} livelli"),
    };
    let dpi = file
        .resolution()
        // 16.16: 300 DPI si legge 300,0000153; si arrotonda al millesimo.
        .map(|r| (f64::from(r.h_res_fixed) / 65536.0 * 1000.0).round() / 1000.0)
        .filter(|d| d.is_finite() && *d >= 1.0);
    let bits = header.depth;
    let (image, icc, mode) = match header.color_mode {
        PsdMode::Cmyk => {
            let (image, icc, profile) = psd_cmyk(&file)?;
            origin = format!("{origin}; CMYK convertito in RGB con {profile}");
            (image, Some(icc), Mode::Cmyk)
        }
        PsdMode::Rgb | PsdMode::Indexed | PsdMode::Grayscale | PsdMode::Bitmap | PsdMode::Duotone => {
            psd_composite(&file)?
        }
        PsdMode::Lab => {
            return Err(PhotoError::Unsupported(
                "i file di Photoshop in Lab non si leggono ancora",
            ));
        }
        PsdMode::Multichannel | PsdMode::Unknown(_) => {
            return Err(PhotoError::Unsupported(
                "i file di Photoshop multicanale non si leggono",
            ));
        }
    };
    Ok(Photo {
        image,
        icc,
        dpi,
        mode,
        bits,
        origin,
    })
}

/// L'immagine unita in RGB o in grigio, come la dà la libreria (a 8 bit), con il profilo del
/// file dove vale per quei pixel.
fn psd_composite(file: &PsdFile) -> Result<(DynamicImage, Option<Vec<u8>>, Mode), PhotoError> {
    let rgba = file
        .composite_rgba8()
        .map_err(|e| PhotoError::Psd(e.to_string()))?;
    let alpha = file.merged_has_alpha();
    let (w, h) = (rgba.width, rgba.height);
    let broken = || PhotoError::Psd("pixel mancanti".into());
    let mode = file.header.color_mode;
    let gray = matches!(mode, PsdMode::Grayscale | PsdMode::Bitmap | PsdMode::Duotone);
    let image = match (gray, alpha) {
        (true, false) => DynamicImage::ImageLuma8(
            ImageBuffer::from_raw(
                w,
                h,
                rgba.data
                    .chunks_exact(4)
                    .map(|p| p.first().copied().unwrap_or(0))
                    .collect(),
            )
            .ok_or_else(broken)?,
        ),
        (true, true) => DynamicImage::ImageLumaA8(
            ImageBuffer::from_raw(
                w,
                h,
                rgba.data
                    .chunks_exact(4)
                    .flat_map(|p| [p.first().copied().unwrap_or(0), p.get(3).copied().unwrap_or(255)])
                    .collect(),
            )
            .ok_or_else(broken)?,
        ),
        (false, false) => DynamicImage::ImageRgb8(
            ImageBuffer::from_raw(
                w,
                h,
                rgba.data
                    .chunks_exact(4)
                    .flat_map(|p| p.iter().take(3).copied())
                    .collect(),
            )
            .ok_or_else(broken)?,
        ),
        (false, true) => DynamicImage::ImageRgba8(ImageBuffer::from_raw(w, h, rgba.data).ok_or_else(broken)?),
    };
    // Il profilo del file vale per i pixel solo in RGB e in grigio; i bicromi e i colori
    // indicizzati diventano pixel senza profilo (sRGB per tutti).
    let icc = match mode {
        PsdMode::Rgb | PsdMode::Grayscale => file.icc_profile().map(<[u8]>::to_vec).filter(|p| !p.is_empty()),
        _ => None,
    };
    let mode = match mode {
        PsdMode::Indexed => Mode::Indexed,
        _ if gray => Mode::Gray,
        _ => Mode::Rgb,
    };
    Ok((image, icc, mode))
}

/// I PSD in CMYK: i quattro canali si convertono in sRGB con il profilo del file (o un CMYK
/// patinato generico se non ce l'ha), non con la formula ingenua che sbaglia neri e blu.
fn psd_cmyk(file: &PsdFile) -> Result<(DynamicImage, Vec<u8>, String), PhotoError> {
    let header = &file.header;
    let (w, h) = (header.width, header.height);
    let pixels = (w as usize)
        .checked_mul(h as usize)
        .ok_or(PhotoError::TooManyPixels(u64::MAX))?;
    let all = file.decode_merged().map_err(|e| PhotoError::Psd(e.to_string()))?;
    let plane = header
        .row_bytes()
        .checked_mul(h as usize)
        .ok_or(PhotoError::TooManyPixels(u64::MAX))?;
    let mut planes = Vec::with_capacity(4);
    for i in 0..4 {
        let bytes = all
            .get(i * plane..(i + 1) * plane)
            .ok_or_else(|| PhotoError::Psd("canali mancanti".into()))?;
        let bytes = if header.depth == 8 {
            bytes.to_vec()
        } else {
            photocraft_psd::pixels::plane_to_u8(bytes, header.depth, w as usize, h as usize)
                .map_err(|e| PhotoError::Psd(e.to_string()))?
        };
        if bytes.len() < pixels {
            return Err(PhotoError::Psd("canali mancanti".into()));
        }
        planes.push(bytes);
    }
    drop(all);
    // Photoshop salva il CMYK rovesciato (255 = niente inchiostro); il profilo vuole l'inchiostro.
    let mut cmyk = Vec::with_capacity(pixels.saturating_mul(4));
    for i in 0..pixels {
        for p in &planes {
            cmyk.push(255 - p.get(i).copied().unwrap_or(255));
        }
    }
    drop(planes);
    let embedded = file
        .icc_profile()
        .and_then(|b| Profile::parse(b).ok())
        .filter(|p| p.color_space == ColorSpace::Cmyk);
    let source = embedded.as_ref().unwrap_or(Builtin::CoatedCmyk.profile());
    let profile = match &embedded {
        Some(p) if !p.description.trim().is_empty() => format!("il suo profilo ({})", p.description.trim()),
        Some(_) => "il suo profilo".to_string(),
        None => "un profilo CMYK patinato generico".to_string(),
    };
    let transform = Transform::new(source, builtin::srgb(), Intent::RelativeColorimetric, true)
        .map_err(|_| PhotoError::Color)?;
    if transform.inputs() != 4 || transform.outputs() != 3 {
        return Err(PhotoError::Color);
    }
    let mut rgb = vec![0u8; pixels.saturating_mul(3)];
    transform.convert_u8(&cmyk, 4, &mut rgb, 3, false);
    let image = ImageBuffer::<image::Rgb<u8>, _>::from_raw(w, h, rgb).ok_or(PhotoError::Color)?;
    Ok((DynamicImage::ImageRgb8(image), srgb_icc(), profile))
}

// ---------------------------------------------------------------------------------------------

/// Il nome scritto dentro un profilo ICC ("Adobe RGB (1998)", "Coated FOGRA39…").
pub fn icc_name(bytes: &[u8]) -> Option<String> {
    let profile = catch_unwind(|| Profile::parse(bytes)).ok()?.ok()?;
    let name = profile.description.trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// Il profilo sRGB di PhotoCraft (pubblico dominio), da scrivere nei file salvati.
pub fn srgb_icc() -> Vec<u8> {
    builtin::srgb().to_bytes().as_ref().clone()
}

fn check_size(width: u32, height: u32) -> Result<(), PhotoError> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 {
        return Err(PhotoError::Unsupported("immagine vuota"));
    }
    if width > MAX_SIDE || height > MAX_SIDE || pixels > MAX_PIXELS {
        return Err(PhotoError::TooManyPixels(pixels));
    }
    Ok(())
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_PIXELS * 4);
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits
}
