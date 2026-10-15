//! Modifica delle immagini (livello L2): apre un'immagine, ne dice le misure vere (pixel,
//! risoluzione, centimetri, area), la ruota, la specchia, la ritaglia, la ridimensiona e la
//! salva mantenendo il profilo ICC e la risoluzione. È il mestiere della scheda "Controllo
//! immagini" della 4.9, fatto senza perdere il colore.
//!
//! Le misure interne restano in millimetri (`winfiles-geom`); i pixel compaiono solo ai bordi,
//! dove si parla con il file.
//!
//! Niente qui può far chiudere l'app: i decodificatori leggono file ostili, quindi ogni
//! dimensione ha un tetto e ogni errore è un `Result`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::{DynamicImage, ImageDecoder, ImageReader, Limits};
use winfiles_geom::Size;

mod save;
#[cfg(test)]
mod tests;

pub use save::{Format, SaveOptions};

/// Pixel massimi di un'immagine aperta o creata da un ridimensionamento: a 4 byte l'uno sono
/// 1 GiB di memoria.
pub const MAX_PIXELS: u64 = 268_000_000;
/// Lato massimo, in pixel.
pub const MAX_SIDE: u32 = 200_000;
/// File più grande che si apre.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Foto HEIC più grande che si apre.
pub const MAX_HEIF_BYTES: u64 = 512 * 1024 * 1024;
/// Risoluzione usata quando il file non la dichiara: la stessa della 4.9.
pub const DEFAULT_DPI: f64 = 96.0;
/// Risoluzioni accettate.
pub const MIN_DPI: f64 = 1.0;
pub const MAX_DPI: f64 = 10_000.0;

#[derive(Debug, thiserror::Error)]
pub enum ImageEditError {
    #[error("{}", io_text(.0))]
    Io(#[from] std::io::Error),
    #[error("immagine non leggibile: {0}")]
    Decode(#[from] image::ImageError),
    #[error("foto HEIC non leggibile")]
    Heif,
    #[error("il file è troppo grande ({0} byte)")]
    TooLarge(u64),
    #[error("immagine troppo grande: {0} pixel")]
    TooManyPixels(u64),
    #[error("misura non valida")]
    BadSize,
    #[error("ritaglio fuori dall'immagine")]
    BadCrop,
    #[error("{0}")]
    Unsupported(&'static str),
    #[error("il decodificatore si è fermato su un errore interno")]
    Crashed,
}

fn io_text(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "file non trovato".into(),
        std::io::ErrorKind::PermissionDenied => "accesso al file negato".into(),
        _ => format!("impossibile leggere il file: {e}"),
    }
}

/// Il verso in cui si specchia.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Un ritaglio in pixel, dall'angolo in alto a sinistra.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CropPx {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Una modifica. Si tengono in fila: annullare vuol dire rifare la fila dall'originale, così
/// anche su un'immagine da 200 megapixel la memoria resta quella di due copie.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// Quarti di giro in senso orario (positivi) o antiorario (negativi).
    Rotate(i32),
    Flip(Axis),
    Crop(CropPx),
    Resize {
        width: u32,
        height: u32,
    },
    /// Cambia la risoluzione senza ricampionare: l'immagine resta quella, cambia quanto misura
    /// sulla carta.
    Dpi(f64),
}

impl Op {
    /// Il nome dell'operazione, per "Annulla" e "Ripeti".
    pub fn label(self) -> String {
        match self {
            Op::Rotate(q) if q.rem_euclid(4) == 3 => "Ruota a sinistra".into(),
            Op::Rotate(q) if q.rem_euclid(4) == 2 => "Capovolgi".into(),
            Op::Rotate(_) => "Ruota a destra".into(),
            Op::Flip(Axis::Horizontal) => "Specchia in orizzontale".into(),
            Op::Flip(Axis::Vertical) => "Specchia in verticale".into(),
            Op::Crop(_) => "Ritaglia".into(),
            Op::Resize { .. } => "Ridimensiona".into(),
            Op::Dpi(_) => "Cambia risoluzione".into(),
        }
    }
}

/// Come sono fatti i colori del file di partenza: si dice a chi guarda, perché il salvataggio
/// scrive sempre RGB o grigio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    Gray,
    GrayAlpha,
    Rgb,
    Rgba,
    Cmyk,
    Indexed,
}

impl ColorMode {
    pub fn label(self) -> &'static str {
        match self {
            ColorMode::Gray => "Scala di grigi",
            ColorMode::GrayAlpha => "Grigi con trasparenza",
            ColorMode::Rgb => "RGB",
            ColorMode::Rgba => "RGB con trasparenza",
            ColorMode::Cmyk => "CMYK (letta in RGB)",
            ColorMode::Indexed => "Colori indicizzati",
        }
    }
}

/// Un'immagine in memoria con ciò che serve per stamparla: pixel, risoluzione e profilo colore.
#[derive(Clone)]
pub struct Picture {
    /// Dietro un `Arc`: copiare un'immagine da cento megapixel per ogni "Annulla" non si può,
    /// e le operazioni ne costruiscono comunque una nuova.
    image: Arc<DynamicImage>,
    dpi: f64,
    icc: Option<Vec<u8>>,
    color: ColorMode,
    bits: u8,
}

impl Picture {
    pub fn width(&self) -> u32 {
        self.image.width()
    }

    pub fn height(&self) -> u32 {
        self.image.height()
    }

    pub fn dpi(&self) -> f64 {
        self.dpi
    }

    pub fn icc(&self) -> Option<&[u8]> {
        self.icc.as_deref()
    }

    pub fn color(&self) -> ColorMode {
        self.color
    }

    /// Bit per canale del file di partenza (8 o 16).
    pub fn bits(&self) -> u8 {
        self.bits
    }

    pub fn image(&self) -> &DynamicImage {
        &self.image
    }

    /// L'immagine condivisa, da passare a chi la disegna senza copiarla.
    pub fn shared(&self) -> Arc<DynamicImage> {
        Arc::clone(&self.image)
    }

    /// Quanto misura sulla carta, in millimetri.
    pub fn size_mm(&self) -> Size {
        Size {
            w: mm(self.image.width(), self.dpi),
            h: mm(self.image.height(), self.dpi),
        }
    }

    /// L'area in metri quadrati.
    pub fn area_m2(&self) -> f64 {
        let s = self.size_mm();
        s.w * s.h / 1_000_000.0
    }

    /// Il rapporto fra i lati ridotto ai minimi termini, come "3:2".
    pub fn ratio(&self) -> String {
        let (w, h) = (self.image.width(), self.image.height());
        let d = gcd(w, h).max(1);
        format!("{}:{}", w / d, h / d)
    }

    /// La distanza da cui l'immagine si vede bene, in metri: la regola della 4.9 (80 diviso i
    /// punti per pollice), quella delle insegne e dei grandi formati.
    pub fn viewing_distance_m(&self) -> Option<f64> {
        (self.dpi > 0.0).then(|| 80.0 / self.dpi)
    }
}

fn mm(px: u32, dpi: f64) -> f64 {
    winfiles_geom::px_to_mm(f64::from(px), dpi).unwrap_or(0.0)
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Un'immagine aperta, con la fila delle modifiche e il suo "Annulla".
pub struct Editor {
    path: PathBuf,
    original: Picture,
    /// La fila delle modifiche; `cursor` dice quante se ne applicano.
    steps: Vec<Op>,
    cursor: usize,
    current: Picture,
    /// Quante modifiche erano applicate all'ultimo salvataggio.
    saved_at: Option<usize>,
    bytes_on_disk: u64,
}

impl Editor {
    /// Apre il file in `path`.
    pub fn open(path: &Path) -> Result<Self, ImageEditError> {
        let bytes_on_disk = std::fs::metadata(path)?.len();
        if bytes_on_disk > MAX_BYTES {
            return Err(ImageEditError::TooLarge(bytes_on_disk));
        }
        let original = load(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            current: original.clone(),
            original,
            steps: Vec::new(),
            cursor: 0,
            saved_at: Some(0),
            bytes_on_disk,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn bytes_on_disk(&self) -> u64 {
        self.bytes_on_disk
    }

    /// L'immagine come si vede adesso, con le modifiche applicate.
    pub fn picture(&self) -> &Picture {
        &self.current
    }

    /// L'immagine com'era nel file.
    pub fn original(&self) -> &Picture {
        &self.original
    }

    pub fn is_modified(&self) -> bool {
        self.saved_at != Some(self.cursor)
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor < self.steps.len()
    }

    pub fn undo_label(&self) -> Option<String> {
        self.cursor
            .checked_sub(1)
            .and_then(|i| self.steps.get(i))
            .map(|op| op.label())
    }

    pub fn redo_label(&self) -> Option<String> {
        self.steps.get(self.cursor).map(|op| op.label())
    }

    /// Applica una modifica. Se non si può fare (un ritaglio vuoto, una misura assurda) non
    /// cambia niente e lo dice.
    pub fn apply(&mut self, op: Op) -> Result<(), ImageEditError> {
        let next = apply_one(&self.current, op)?;
        self.steps.truncate(self.cursor);
        self.steps.push(op);
        self.cursor = self.steps.len();
        if self.saved_at > Some(self.cursor) {
            // Le modifiche salvate sono state buttate via: non si torna più a quel punto.
            self.saved_at = None;
        }
        self.current = next;
        Ok(())
    }

    pub fn undo(&mut self) -> Result<(), ImageEditError> {
        if self.cursor == 0 {
            return Ok(());
        }
        self.cursor -= 1;
        self.replay()
    }

    pub fn redo(&mut self) -> Result<(), ImageEditError> {
        if self.cursor >= self.steps.len() {
            return Ok(());
        }
        self.cursor += 1;
        self.replay()
    }

    /// Torna all'immagine del file, buttando via tutte le modifiche.
    pub fn reset(&mut self) {
        self.steps.clear();
        self.cursor = 0;
        self.current = self.original.clone();
    }

    /// Rifà la fila delle modifiche dall'originale: così "Annulla" non costa memoria.
    fn replay(&mut self) -> Result<(), ImageEditError> {
        let mut picture = self.original.clone();
        let steps = self.steps.get(..self.cursor).unwrap_or_default().to_vec();
        for op in steps {
            picture = apply_one(&picture, op)?;
        }
        self.current = picture;
        Ok(())
    }

    /// Salva in `path` con le opzioni date; la scrittura è atomica.
    pub fn save(&mut self, path: &Path, options: SaveOptions) -> Result<(), ImageEditError> {
        save::write(&self.current, path, options)?;
        if path == self.path {
            self.saved_at = Some(self.cursor);
            self.bytes_on_disk = std::fs::metadata(path)
                .map(|m| m.len())
                .unwrap_or(self.bytes_on_disk);
        }
        Ok(())
    }

    /// Il nome proposto per "Salva con nome": `foto_modificata.jpg`.
    pub fn suggested_name(&self, format: Format) -> String {
        let stem = self
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "immagine".into());
        format!("{stem}_modificata.{}", format.extension())
    }
}

fn apply_one(picture: &Picture, op: Op) -> Result<Picture, ImageEditError> {
    let mut next = picture.clone();
    match op {
        Op::Rotate(quarters) => {
            next.image = Arc::new(match quarters.rem_euclid(4) {
                1 => picture.image.rotate90(),
                2 => picture.image.rotate180(),
                3 => picture.image.rotate270(),
                _ => return Ok(next),
            });
        }
        Op::Flip(Axis::Horizontal) => next.image = Arc::new(picture.image.fliph()),
        Op::Flip(Axis::Vertical) => next.image = Arc::new(picture.image.flipv()),
        Op::Crop(crop) => {
            let (w, h) = (picture.image.width(), picture.image.height());
            let right = crop.x.checked_add(crop.width).ok_or(ImageEditError::BadCrop)?;
            let bottom = crop.y.checked_add(crop.height).ok_or(ImageEditError::BadCrop)?;
            if crop.width == 0 || crop.height == 0 || right > w || bottom > h {
                return Err(ImageEditError::BadCrop);
            }
            next.image = Arc::new(picture.image.crop_imm(crop.x, crop.y, crop.width, crop.height));
        }
        Op::Resize { width, height } => {
            check_size(width, height)?;
            if width == picture.image.width() && height == picture.image.height() {
                return Ok(next);
            }
            next.image = Arc::new(picture.image.resize_exact(
                width,
                height,
                image::imageops::FilterType::Lanczos3,
            ));
        }
        Op::Dpi(dpi) => {
            if !(MIN_DPI..=MAX_DPI).contains(&dpi) {
                return Err(ImageEditError::BadSize);
            }
            next.dpi = dpi;
        }
    }
    Ok(next)
}

fn check_size(width: u32, height: u32) -> Result<(), ImageEditError> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(ImageEditError::BadSize);
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_PIXELS {
        return Err(ImageEditError::TooManyPixels(pixels));
    }
    Ok(())
}

/// I pixel che servono per avere `mm` millimetri a `dpi` punti per pollice.
pub fn pixels_for_mm(mm: f64, dpi: f64) -> u32 {
    winfiles_geom::mm_to_px(mm, dpi).clamp(1, MAX_SIDE)
}

/// La risoluzione che fa misurare `mm` millimetri a un lato di `px` pixel.
pub fn dpi_for_mm(px: u32, mm: f64) -> Option<f64> {
    (mm > 0.0 && px > 0).then(|| f64::from(px) * 25.4 / mm)
}

/// Legge il file: immagini con `image`, foto HEIC con il lettore di PhotoCraft. L'orientamento
/// EXIF si applica subito, così quello che si vede è quello che è.
fn load(path: &Path) -> Result<Picture, ImageEditError> {
    let data = std::fs::read(path)?;
    if is_heif(&data) {
        if data.len() as u64 > MAX_HEIF_BYTES {
            return Err(ImageEditError::TooLarge(data.len() as u64));
        }
        return heif(&data);
    }
    let reader = ImageReader::new(std::io::Cursor::new(&data)).with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_PIXELS * 4);
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    let mut reader = reader;
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let (w, h) = decoder.dimensions();
    check_size(w, h)?;
    let orientation = decoder.orientation()?;
    let icc = decoder
        .icc_profile()
        .unwrap_or(None)
        .or_else(|| tiff_icc(&data))
        .filter(|p| !p.is_empty());
    let color_type = decoder.original_color_type();
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let dpi = file_dpi(path);
    Ok(Picture {
        color: color_mode(color_type, &image),
        bits: bits_per_channel(&image),
        image: Arc::new(image),
        dpi,
        icc,
    })
}

/// Il profilo ICC di un TIFF. `image` 0.25 lo perde non appena gli si danno dei limiti di
/// memoria (li applica ricostruendo il lettore), e i limiti ci servono: qui si rilegge la sola
/// intestazione del TIFF, che costa niente. Sugli altri formati il lettore fallisce subito.
fn tiff_icc(data: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = image::codecs::tiff::TiffDecoder::new(std::io::Cursor::new(data)).ok()?;
    decoder.icc_profile().ok().flatten()
}

/// La risoluzione dichiarata dal file, letta dalle intestazioni come per le misure
/// (`winfiles-formats`): JFIF ed EXIF nei JPEG, `pHYs` nei PNG, i campi dei TIFF.
fn file_dpi(path: &Path) -> f64 {
    winfiles_formats::probe(path, winfiles_model::FileKind::Image)
        .ok()
        .and_then(|p| p.image)
        .map(|i| i.dpi_x)
        .filter(|d| (MIN_DPI..=MAX_DPI).contains(d))
        .unwrap_or(DEFAULT_DPI)
}

fn color_mode(original: image::ExtendedColorType, image: &DynamicImage) -> ColorMode {
    use image::ExtendedColorType as E;
    match original {
        E::Cmyk8 | E::Cmyk16 => ColorMode::Cmyk,
        E::L1 | E::L8 | E::L16 => ColorMode::Gray,
        E::La1 | E::La8 | E::La16 => ColorMode::GrayAlpha,
        E::Unknown(_) => match image {
            DynamicImage::ImageLuma8(_) | DynamicImage::ImageLuma16(_) => ColorMode::Gray,
            DynamicImage::ImageLumaA8(_) | DynamicImage::ImageLumaA16(_) => ColorMode::GrayAlpha,
            _ if image.color().has_alpha() => ColorMode::Rgba,
            _ => ColorMode::Rgb,
        },
        _ if image.color().has_alpha() => ColorMode::Rgba,
        _ => ColorMode::Rgb,
    }
}

fn bits_per_channel(image: &DynamicImage) -> u8 {
    match image {
        DynamicImage::ImageLuma16(_)
        | DynamicImage::ImageLumaA16(_)
        | DynamicImage::ImageRgb16(_)
        | DynamicImage::ImageRgba16(_) => 16,
        DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_) => 32,
        _ => 8,
    }
}

/// Il contenitore di una foto HEIC/HEIF (gli stessi marchi delle miniature).
fn is_heif(data: &[u8]) -> bool {
    const BRANDS: [&[u8; 4]; 8] = [
        b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx", b"mif1", b"msf1",
    ];
    data.get(4..8) == Some(b"ftyp")
        && data
            .get(8..12)
            .is_some_and(|b| BRANDS.iter().any(|m| m.as_slice() == b))
}

fn heif(data: &[u8]) -> Result<Picture, ImageEditError> {
    let options = photocraft_heif::Options {
        max_pixels: MAX_PIXELS,
        apply_transforms: true,
    };
    let decoded = photocraft_heif::decode(data, &options).map_err(|_| ImageEditError::Heif)?;
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
    let image = if decoded.has_alpha {
        DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(decoded.width, decoded.height, samples).ok_or(ImageEditError::Heif)?,
        )
    } else {
        DynamicImage::ImageRgb8(
            image::RgbImage::from_raw(decoded.width, decoded.height, samples).ok_or(ImageEditError::Heif)?,
        )
    };
    Ok(Picture {
        color: if decoded.has_alpha {
            ColorMode::Rgba
        } else {
            ColorMode::Rgb
        },
        bits: if decoded.sixteen_bit { 16 } else { 8 },
        image: Arc::new(image),
        dpi: DEFAULT_DPI,
        icc: None,
    })
}
