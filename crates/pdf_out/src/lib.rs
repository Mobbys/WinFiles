//! PDF in uscita (livello L2): le scene dei fogli diventano un PDF con un foglio per pagina, alla
//! misura vera (A4 o A3). I testi usano Inter incorporato; le miniature (immagini, pagine di PDF
//! e AI, anteprime degli EPS) entrano come JPEG a 150 DPI della loro misura sul foglio: nitide
//! in stampa e leggere, qualunque sia la risoluzione del file originale.
//!
//! Un file che non si riesce a leggere non ferma l'esportazione: al suo posto resta un riquadro
//! grigio, e il file finisce nell'elenco degli avvisi.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Size as KSize, Transform};
use krilla::image::Image;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule, Stroke};
use krilla::surface::Surface;
use krilla::text::{Font, TextDirection};
use krilla::{Data, Document};
use winfiles_geom::{Calibration, Rect, mm_to_pt};
use winfiles_scene::{Color, Item, Page, Picture, Source, Text, Weight};

/// Risoluzione delle miniature nel PDF, rispetto alla loro misura sul foglio.
pub const IMAGE_DPI: f64 = 150.0;
/// Lato massimo in pixel di una miniatura nel PDF (un A3 intero a 150 DPI ne chiede 2480).
const MAX_IMAGE_SIDE: u32 = 2600;
/// Qualità dei JPEG delle miniature.
const JPEG_QUALITY: u8 = 85;

#[derive(Debug, thiserror::Error)]
pub enum PdfOutError {
    #[error("il font incorporato non è leggibile")]
    Font,
    #[error("non riesco a scrivere il PDF: {0}")]
    Write(String),
    #[error("esportazione annullata")]
    Cancelled,
    #[error("non riesco a salvare il file: {0}")]
    Io(#[from] std::io::Error),
}

/// I caratteri da incorporare: gli stessi dell'anteprima.
pub struct Fonts {
    pub regular: &'static [u8],
    pub semibold: &'static [u8],
}

/// Com'è andata l'esportazione.
#[derive(Debug, Default)]
pub struct Report {
    pub pages: usize,
    /// File che non si sono potuti disegnare, con il motivo.
    pub warnings: Vec<(PathBuf, String)>,
}

struct Writer {
    regular: Font,
    semibold: Font,
    rasters: HashMap<(Source, u32), Option<Image>>,
    report: Report,
}

/// Scrive `pages` in un PDF. `progress` riceve il numero di fogli fatti; `cancel` ferma il lavoro.
pub fn write(
    pages: &[Page],
    fonts: &Fonts,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize),
) -> Result<(Vec<u8>, Report), PdfOutError> {
    let regular = Font::new(Data::from(fonts.regular), 0).ok_or(PdfOutError::Font)?;
    let semibold = Font::new(Data::from(fonts.semibold), 0).ok_or(PdfOutError::Font)?;
    let mut w = Writer {
        regular,
        semibold,
        rasters: HashMap::new(),
        report: Report::default(),
    };
    let mut doc = Document::new();
    for (i, page) in pages.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(PdfOutError::Cancelled);
        }
        let size = KSize::from_wh(pt(page.size.w), pt(page.size.h))
            .ok_or_else(|| PdfOutError::Write("formato del foglio non valido".into()))?;
        let mut p = doc.start_page_with(PageSettings::new(size));
        let mut surface = p.surface();
        let c = page.calibration;
        let calibrated = !c.is_identity();
        if calibrated {
            surface.push_transform(&calibration(&c));
        }
        for item in &page.items {
            w.item(&mut surface, item);
        }
        if calibrated {
            surface.pop();
        }
        surface.finish();
        p.finish();
        w.report.pages += 1;
        progress(i + 1);
    }
    let bytes = doc.finish().map_err(|e| PdfOutError::Write(format!("{e:?}")))?;
    Ok((bytes, w.report))
}

/// Scrive il PDF su disco passando da un file temporaneo, come il salvataggio del lavoro.
pub fn save(path: &Path, bytes: &[u8]) -> Result<(), PdfOutError> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// La correzione della stampante come trasformazione del foglio, in punti PDF.
fn calibration(c: &Calibration) -> Transform {
    Transform::from_row(
        c.scale_x as f32,
        0.0,
        0.0,
        c.scale_y as f32,
        pt(c.offset_x),
        pt(c.offset_y),
    )
}

fn pt(mm: f64) -> f32 {
    mm_to_pt(mm) as f32
}

fn color(c: Color) -> rgb::Color {
    rgb::Color::new(c.r, c.g, c.b)
}

fn opacity(c: Color) -> NormalizedF32 {
    NormalizedF32::new(f32::from(c.a) / 255.0).unwrap_or(NormalizedF32::ONE)
}

fn fill(c: Color) -> Fill {
    Fill {
        paint: color(c).into(),
        opacity: opacity(c),
        rule: FillRule::NonZero,
    }
}

/// Il contorno di un rettangolo con gli angoli arrotondati, in punti PDF.
fn rounded(r: Rect, radius: f64) -> Option<krilla::geom::Path> {
    let (x, y, w, h) = (pt(r.x), pt(r.y), pt(r.w), pt(r.h));
    if !(w > 0.0 && h > 0.0) {
        return None;
    }
    let rad = pt(radius).clamp(0.0, w.min(h) / 2.0);
    let mut b = PathBuilder::new();
    if rad <= 0.01 {
        b.push_rect(krilla::geom::Rect::from_xywh(x, y, w, h)?);
        return b.finish();
    }
    // Arco approssimato con una cubica (0,5523 = 4/3·(√2−1)).
    let k = rad * 0.552_284_8;
    b.move_to(x + rad, y);
    b.line_to(x + w - rad, y);
    b.cubic_to(x + w - rad + k, y, x + w, y + rad - k, x + w, y + rad);
    b.line_to(x + w, y + h - rad);
    b.cubic_to(x + w, y + h - rad + k, x + w - rad + k, y + h, x + w - rad, y + h);
    b.line_to(x + rad, y + h);
    b.cubic_to(x + rad - k, y + h, x, y + h - rad + k, x, y + h - rad);
    b.line_to(x, y + rad);
    b.cubic_to(x, y + rad - k, x + rad - k, y, x + rad, y);
    b.close();
    b.finish()
}

impl Writer {
    fn item(&mut self, s: &mut Surface<'_>, item: &Item) {
        match item {
            Item::Rect {
                rect,
                radius,
                fill: f,
                stroke,
            } => {
                if let Some(c) = f
                    && let Some(path) = rounded(*rect, *radius)
                {
                    s.set_stroke(None);
                    s.set_fill(Some(fill(*c)));
                    s.draw_path(&path);
                }
                if let Some((c, width)) = stroke {
                    // Il bordo sta dentro il rettangolo, come nell'anteprima.
                    let half = width / 2.0;
                    let inner = Rect::new(rect.x + half, rect.y + half, rect.w - width, rect.h - width);
                    if let Some(path) = rounded(inner, (radius - half).max(0.0)) {
                        s.set_fill(None);
                        s.set_stroke(Some(Stroke {
                            paint: color(*c).into(),
                            width: pt(*width),
                            opacity: opacity(*c),
                            ..Stroke::default()
                        }));
                        s.draw_path(&path);
                        s.set_stroke(None);
                    }
                }
            }
            Item::Line {
                from,
                to,
                color: c,
                width,
            } => {
                let mut b = PathBuilder::new();
                b.move_to(pt(from.0), pt(from.1));
                b.line_to(pt(to.0), pt(to.1));
                if let Some(path) = b.finish() {
                    s.set_fill(None);
                    s.set_stroke(Some(Stroke {
                        paint: color(*c).into(),
                        width: pt(*width),
                        opacity: opacity(*c),
                        ..Stroke::default()
                    }));
                    s.draw_path(&path);
                    s.set_stroke(None);
                }
            }
            Item::Text(t) => self.text(s, t),
            Item::Picture(p) => self.picture(s, p),
        }
    }

    fn text(&mut self, s: &mut Surface<'_>, t: &Text) {
        if t.text.trim().is_empty() || !t.size.is_finite() || t.size <= 0.0 {
            return;
        }
        let font = match t.weight {
            Weight::Regular => self.regular.clone(),
            Weight::SemiBold => self.semibold.clone(),
        };
        s.set_stroke(None);
        s.set_fill(Some(fill(t.color)));
        s.draw_text(
            Point::from_xy(pt(t.x), pt(t.baseline)),
            font,
            pt(t.size),
            &t.text,
            false,
            TextDirection::LeftToRight,
        );
    }

    fn picture(&mut self, s: &mut Surface<'_>, p: &Picture) {
        let crop = p.crop;
        if !(crop.w > 0.0 && crop.h > 0.0) {
            return;
        }
        // La pagina intera, scalata in modo che la parte `crop` riempia `rect`.
        let full_w = p.rect.w / crop.w;
        let full_h = p.rect.h / crop.h;
        let left = p.rect.x - crop.x * full_w;
        let top = p.rect.y - crop.y * full_h;
        let Some(clip) = rounded(p.rect, 0.0) else {
            return;
        };
        let prepared = self.prepare(&p.source, full_w.max(full_h));
        s.push_clip_path(&clip, &FillRule::NonZero);
        s.push_transform(&Transform::from_translate(pt(left), pt(top)));
        let size = KSize::from_wh(pt(full_w), pt(full_h));
        match (prepared, size) {
            (Some(img), Some(size)) => s.draw_image(img, size),
            _ => {
                if let Some(path) = rounded(Rect::new(0.0, 0.0, full_w, full_h), 0.0) {
                    s.set_stroke(None);
                    s.set_fill(Some(fill(Color::rgb(235, 235, 235))));
                    s.draw_path(&path);
                }
            }
        }
        s.pop();
        s.pop();
    }

    /// La miniatura come JPEG per un lato lungo di `side_mm` sul foglio, a [`IMAGE_DPI`].
    fn prepare(&mut self, source: &Source, side_mm: f64) -> Option<Image> {
        let px = ((side_mm / 25.4 * IMAGE_DPI).ceil() as u32).clamp(64, MAX_IMAGE_SIDE);
        // Le misure vicine (le pagine dello stesso file) condividono la stessa immagine.
        let px = px.div_ceil(64).saturating_mul(64).min(MAX_IMAGE_SIDE);
        let key = (source.clone(), px);
        if let Some(cached) = self.rasters.get(&key) {
            return cached.clone();
        }
        let image = match raster(source, px) {
            Ok(img) => Some(img),
            Err(e) => {
                self.warn(&source.path, e);
                None
            }
        };
        self.rasters.insert(key, image.clone());
        image
    }

    fn warn(&mut self, path: &Path, why: String) {
        if !self.report.warnings.iter().any(|(p, _)| p == path) {
            self.report.warnings.push((path.to_path_buf(), why));
        }
    }
}

/// La pagina come JPEG.
fn raster(source: &Source, side: u32) -> Result<Image, String> {
    let thumb =
        winfiles_thumbs::render(&source.path, source.kind, source.page, side).map_err(|e| e.to_string())?;
    let rgba = image::RgbaImage::from_raw(thumb.width, thumb.height, thumb.rgba)
        .ok_or_else(|| "immagine non valida".to_string())?;
    // Il JPEG non ha trasparenza: si appoggia la miniatura sul bianco della carta.
    let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(|e| e.to_string())?;
    Image::from_jpeg(Data::from(out), true)
}

#[cfg(test)]
mod tests;
