//! L'abbondanza fatta con i pixel (ADR 0028): l'immagine si allarga di qualche millimetro per
//! lato e la fascia nuova si riempie con l'immagine stessa, ribaltata a specchio sul bordo o con
//! l'ultima riga di pixel allungata. Lavora su ogni tipo di pixel (8 e 16 bit, grigi, con o senza
//! trasparenza), così i colori restano quelli del file.

use image::{DynamicImage, ImageBuffer, Pixel};

use crate::CropPx;

/// Come si riempie la fascia.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BleedMethod {
    /// L'immagine ribaltata sul bordo: continua le linee e le sfumature.
    #[default]
    Mirror,
    /// L'ultima riga (o colonna) di pixel ripetuta fino al bordo nuovo.
    Stretch,
}

impl BleedMethod {
    pub fn label(self) -> &'static str {
        match self {
            BleedMethod::Mirror => "A specchio",
            BleedMethod::Stretch => "Allunga il bordo",
        }
    }
}

/// Il pixel di partenza per la colonna (o la riga) `i` dell'immagine nuova, larga `n` pixel
/// all'origine con `before` pixel aggiunti prima.
fn source(i: u32, before: u32, n: u32, method: BleedMethod) -> u32 {
    let last = n.saturating_sub(1);
    let v = i64::from(i) - i64::from(before);
    let n64 = i64::from(n.max(1));
    let picked = match method {
        BleedMethod::Stretch => v,
        // Ribaltata sul bordo, e di nuovo se la fascia è più larga dell'immagine.
        BleedMethod::Mirror => {
            let period = 2 * n64;
            let m = v.rem_euclid(period);
            if m < n64 { m } else { period - 1 - m }
        }
    };
    u32::try_from(picked.clamp(0, i64::from(last))).unwrap_or(0)
}

fn extend<P: Pixel>(
    img: &ImageBuffer<P, Vec<P::Subpixel>>,
    sides: [u32; 4],
    method: BleedMethod,
) -> ImageBuffer<P, Vec<P::Subpixel>> {
    let (w, h) = img.dimensions();
    let [left, top, right, bottom] = sides;
    let nw = w.saturating_add(left).saturating_add(right);
    let nh = h.saturating_add(top).saturating_add(bottom);
    let xs: Vec<u32> = (0..nw).map(|x| source(x, left, w, method)).collect();
    ImageBuffer::from_fn(nw, nh, |x, y| {
        let sx = xs.get(x as usize).copied().unwrap_or(0);
        let sy = source(y, top, h, method);
        *img.get_pixel(sx, sy)
    })
}

/// L'immagine con `sides` pixel in più a sinistra, in alto, a destra e in basso.
pub(crate) fn add(img: &DynamicImage, sides: [u32; 4], method: BleedMethod) -> DynamicImage {
    match img {
        DynamicImage::ImageLuma8(b) => DynamicImage::ImageLuma8(extend(b, sides, method)),
        DynamicImage::ImageLumaA8(b) => DynamicImage::ImageLumaA8(extend(b, sides, method)),
        DynamicImage::ImageRgb8(b) => DynamicImage::ImageRgb8(extend(b, sides, method)),
        DynamicImage::ImageRgba8(b) => DynamicImage::ImageRgba8(extend(b, sides, method)),
        DynamicImage::ImageLuma16(b) => DynamicImage::ImageLuma16(extend(b, sides, method)),
        DynamicImage::ImageLumaA16(b) => DynamicImage::ImageLumaA16(extend(b, sides, method)),
        DynamicImage::ImageRgb16(b) => DynamicImage::ImageRgb16(extend(b, sides, method)),
        DynamicImage::ImageRgba16(b) => DynamicImage::ImageRgba16(extend(b, sides, method)),
        DynamicImage::ImageRgb32F(b) => DynamicImage::ImageRgb32F(extend(b, sides, method)),
        DynamicImage::ImageRgba32F(b) => DynamicImage::ImageRgba32F(extend(b, sides, method)),
        other => DynamicImage::ImageRgba16(extend(&other.to_rgba16(), sides, method)),
    }
}

/// Dove finisce il formato finito dopo una modifica dell'immagine larga `w` × `h` (prima della
/// modifica). `None` se la modifica lo fa sparire (un ritaglio che lo lascia fuori).
pub(crate) fn rotate_trim(t: CropPx, w: u32, h: u32, quarters: i32) -> CropPx {
    match quarters.rem_euclid(4) {
        1 => CropPx {
            x: h.saturating_sub(t.y.saturating_add(t.height)),
            y: t.x,
            width: t.height,
            height: t.width,
        },
        2 => CropPx {
            x: w.saturating_sub(t.x.saturating_add(t.width)),
            y: h.saturating_sub(t.y.saturating_add(t.height)),
            ..t
        },
        3 => CropPx {
            x: t.y,
            y: w.saturating_sub(t.x.saturating_add(t.width)),
            width: t.height,
            height: t.width,
        },
        _ => t,
    }
}

/// Il formato finito dopo un ritaglio `c`: la parte che resta dentro, spostata.
pub(crate) fn crop_trim(t: CropPx, c: CropPx) -> Option<CropPx> {
    let x0 = t.x.max(c.x);
    let y0 = t.y.max(c.y);
    let x1 = t.x.saturating_add(t.width).min(c.x.saturating_add(c.width));
    let y1 = t.y.saturating_add(t.height).min(c.y.saturating_add(c.height));
    (x1 > x0 && y1 > y0).then(|| CropPx {
        x: x0 - c.x,
        y: y0 - c.y,
        width: x1 - x0,
        height: y1 - y0,
    })
}

/// Il formato finito dopo un ridimensionamento da `from` a `to` pixel.
pub(crate) fn resize_trim(t: CropPx, from: (u32, u32), to: (u32, u32)) -> Option<CropPx> {
    let kx = f64::from(to.0) / f64::from(from.0.max(1));
    let ky = f64::from(to.1) / f64::from(from.1.max(1));
    let r = |v: u32, k: f64| (f64::from(v) * k).round().max(0.0) as u32;
    let out = CropPx {
        x: r(t.x, kx),
        y: r(t.y, ky),
        width: r(t.width, kx).min(to.0),
        height: r(t.height, ky).min(to.1),
    };
    (out.width > 0 && out.height > 0).then_some(out)
}
