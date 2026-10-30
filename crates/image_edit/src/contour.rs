//! La fustella dal contorno del soggetto (ADR 0028, fase 2): quali pixel sono sfondo (un colore,
//! cercato a partire dai bordi, o la trasparenza), il contorno di tutto il resto, pulito dagli
//! scalini dei pixel e allargato (o stretto) di qualche millimetro. Il risultato sono contorni
//! chiusi in mm dall'angolo in alto a sinistra dell'immagine, pronti per la tinta del taglio.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};
use image::{DynamicImage, GenericImageView};

use crate::{ImageEditError, Picture};

/// Lato massimo dell'immagine su cui si cerca il contorno: abbastanza per una linea di taglio
/// precisa (su un metro, mezzo millimetro), e il lavoro resta di un attimo.
const WORK_SIDE: u32 = 2000;

/// Che cosa è sfondo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Background {
    /// Un colore RGB: i pixel simili collegati ai bordi dell'immagine (il bianco dentro il
    /// soggetto resta soggetto).
    Color([u8; 3]),
    /// I pixel trasparenti.
    Transparent,
}

/// Le scelte del contorno.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContourSpec {
    pub background: Background,
    /// Quanto un pixel può essere diverso dal colore di sfondo ed esserlo ancora, da 0 a 100.
    pub tolerance: f64,
    /// Di quanto la fustella si allontana dal soggetto, in mm: positivo fuori (il bordo bianco
    /// dell'adesivo), negativo dentro.
    pub offset: f64,
    /// Lisciatura, in mm: le rientranze e le punte più strette di questo raggio si arrotondano
    /// (0: il contorno segue il soggetto da vicino).
    pub smooth: f64,
    /// I pezzi (e i fori) più piccoli di questa area si ignorano, in mm².
    pub min_area: f64,
    /// Taglia anche i fori dentro il soggetto (il centro di una O): ogni zona del colore di
    /// sfondo, anche chiusa dentro il soggetto, è sfondo.
    pub holes: bool,
}

type Contour = Vec<[f64; 2]>;

/// I contorni della fustella in mm, o un errore se non c'è un soggetto.
pub fn trace(picture: &Picture, spec: &ContourSpec) -> Result<Vec<Contour>, ImageEditError> {
    trace_image(picture.image(), 25.4 / picture.dpi(), spec)
}

/// Come [`trace`] su un'immagine qualunque (una pagina di PDF disegnata, per esempio) di cui
/// un pixel misura `mm_per_px` millimetri.
pub fn trace_image(
    img: &DynamicImage,
    mm_per_px: f64,
    spec: &ContourSpec,
) -> Result<Vec<Contour>, ImageEditError> {
    if !(mm_per_px.is_finite() && mm_per_px > 0.0) {
        return Err(ImageEditError::BadSize);
    }
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err(ImageEditError::BadSize);
    }
    // Sull'immagine ridotta; `k` porta i suoi pixel in mm.
    let shrink = (f64::from(w.max(h)) / f64::from(WORK_SIDE)).max(1.0);
    let small: DynamicImage = if shrink > 1.0 {
        let nw = ((f64::from(w) / shrink).round() as u32).max(1);
        let nh = ((f64::from(h) / shrink).round() as u32).max(1);
        img.resize_exact(nw, nh, image::imageops::FilterType::Triangle)
    } else {
        img.clone()
    };
    let (sw, sh) = small.dimensions();
    let mm_per_px = mm_per_px * f64::from(w) / f64::from(sw);
    let subject = mask(&small, spec);
    if !subject.iter().any(|v| *v) {
        return Err(ImageEditError::Unsupported(
            "non trovo un soggetto: è tutto sfondo",
        ));
    }
    if subject.iter().all(|v| *v) {
        return Err(ImageEditError::Unsupported(
            "non trovo lo sfondo: controlla il colore o alza la tolleranza",
        ));
    }
    // Ogni fila di pixel del soggetto è un rettangolo; uniti danno il contorno a scalini.
    let mut rects: Vec<Contour> = Vec::new();
    for y in 0..sh {
        let row = (y as usize) * (sw as usize);
        let mut x = 0u32;
        while x < sw {
            if subject.get(row + x as usize).copied().unwrap_or(false) {
                let start = x;
                while x < sw && subject.get(row + x as usize).copied().unwrap_or(false) {
                    x += 1;
                }
                let (x0, x1, y0, y1) = (f64::from(start), f64::from(x), f64::from(y), f64::from(y + 1));
                rects.push(vec![[x0, y0], [x0, y1], [x1, y1], [x1, y0]]);
            } else {
                x += 1;
            }
        }
    }
    let shapes = rects.simplify_shape(FillRule::NonZero);
    let min_px = spec.min_area.max(0.0) / (mm_per_px * mm_per_px);
    // Gli scalini via: semplificati a un pixel e arrotondati, poi in mm.
    let mut clean: Vec<Vec<Contour>> = Vec::new();
    for shape in shapes {
        let mut out: Vec<Contour> = Vec::new();
        for (i, c) in shape.into_iter().enumerate() {
            let hole = i > 0;
            if hole && !spec.holes {
                continue;
            }
            if area(&c).abs() < min_px {
                continue;
            }
            let smooth = chaikin(&simplify(&c, 0.9), 2);
            if smooth.len() < 3 {
                continue;
            }
            out.push(
                smooth
                    .iter()
                    .map(|p| [p[0] * mm_per_px, p[1] * mm_per_px])
                    .collect(),
            );
        }
        if !out.is_empty() {
            clean.push(out);
        }
    }
    if clean.is_empty() {
        return Err(ImageEditError::Unsupported(
            "il soggetto è più piccolo dei pezzi da ignorare",
        ));
    }
    // Ripulita (gli arrotondamenti possono incrociarsi) e allargata con gli angoli tondi.
    let flat: Vec<Contour> = clean.into_iter().flatten().collect();
    let tidy = flat.simplify_shape(FillRule::EvenOdd);
    let offset = if spec.offset.is_finite() {
        spec.offset.clamp(-50.0, 100.0)
    } else {
        0.0
    };
    let grow = |shapes: Vec<Vec<Contour>>, d: f64| {
        let style = OutlineStyle::new(d).line_join(LineJoin::Round(0.1));
        shapes.outline(&style)
    };
    let mut shapes = if offset.abs() < 0.01 {
        tidy
    } else {
        grow(tidy, offset)
    };
    // Lisciatura, come una moneta di raggio `r` fatta girare attorno: prima la chiusura (fuori e
    // dentro: riempie le rientranze strette), poi l'apertura (dentro e fuori: arrotonda le
    // punte) con un raggio non più grande della distanza, così la lama non entra mai nel
    // soggetto: ogni suo punto ha attorno un cerchio di quel raggio tutto dentro la fustella.
    let r = if spec.smooth.is_finite() {
        spec.smooth.clamp(0.0, 50.0)
    } else {
        0.0
    };
    if r >= 0.05 {
        shapes = grow(grow(shapes, r), -r);
        let open = r.min(offset);
        if open >= 0.05 {
            shapes = grow(grow(shapes, -open), open);
        }
    }
    let result: Vec<Contour> = shapes.into_iter().flatten().collect();
    let result: Vec<Contour> = result.into_iter().filter(|c| c.len() >= 3).collect();
    if result.is_empty() {
        return Err(ImageEditError::Unsupported(
            "con questa distanza dentro, il soggetto sparisce",
        ));
    }
    Ok(result)
}

/// I pixel del soggetto, riga per riga.
fn mask(img: &DynamicImage, spec: &ContourSpec) -> Vec<bool> {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let (wu, hu) = (w as usize, h as usize);
    match spec.background {
        Background::Transparent => rgba.pixels().map(|p| p.0[3] >= 128).collect(),
        Background::Color(c) => {
            // Distanza massima nel cubo RGB (0–441) per la tolleranza in percento.
            let limit = (spec.tolerance.clamp(0.0, 100.0) / 100.0 * 441.0).max(1.0);
            let limit2 = limit * limit;
            let near = |x: usize, y: usize| -> bool {
                rgba.get_pixel_checked(x as u32, y as u32).is_some_and(|p| {
                    let [r, g, b, a] = p.0;
                    if a < 128 {
                        return true;
                    }
                    let d = |v: u8, t: u8| f64::from(v) - f64::from(t);
                    let (dr, dg, db) = (d(r, c[0]), d(g, c[1]), d(b, c[2]));
                    dr * dr + dg * dg + db * db <= limit2
                })
            };
            // Con i fori, ogni zona del colore di sfondo è sfondo (il centro bianco di una O);
            // senza, lo sfondo parte dai bordi e si allarga ai vicini simili, così il bianco
            // dentro il soggetto resta soggetto.
            if spec.holes {
                return (0..hu)
                    .flat_map(|y| (0..wu).map(move |x| (x, y)))
                    .map(|(x, y)| !near(x, y))
                    .collect();
            }
            let mut background = vec![false; wu * hu];
            let mut stack: Vec<(usize, usize)> = Vec::new();
            for x in 0..wu {
                stack.push((x, 0));
                stack.push((x, hu - 1));
            }
            for y in 0..hu {
                stack.push((0, y));
                stack.push((wu - 1, y));
            }
            while let Some((x, y)) = stack.pop() {
                let i = y * wu + x;
                if background.get(i).copied().unwrap_or(true) || !near(x, y) {
                    continue;
                }
                if let Some(v) = background.get_mut(i) {
                    *v = true;
                }
                if x > 0 {
                    stack.push((x - 1, y));
                }
                if x + 1 < wu {
                    stack.push((x + 1, y));
                }
                if y > 0 {
                    stack.push((x, y - 1));
                }
                if y + 1 < hu {
                    stack.push((x, y + 1));
                }
            }
            background.into_iter().map(|b| !b).collect()
        }
    }
}

fn area(c: &[[f64; 2]]) -> f64 {
    let n = c.len();
    (0..n)
        .map(|i| {
            let (Some(a), Some(b)) = (c.get(i), c.get((i + 1) % n)) else {
                return 0.0;
            };
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        / 2.0
}

/// Ramer–Douglas–Peucker su un contorno chiuso: i punti più lontani di `eps` dalla corda restano.
fn simplify(c: &[[f64; 2]], eps: f64) -> Contour {
    if c.len() < 4 {
        return c.to_vec();
    }
    // Si parte dai due punti più lontani tra loro (circa): il primo e quello più lontano da lui.
    let first = c.first().copied().unwrap_or([0.0, 0.0]);
    let far = (0..c.len())
        .max_by(|a, b| {
            let d = |i: &usize| {
                c.get(*i)
                    .map_or(0.0, |p| (p[0] - first[0]).hypot(p[1] - first[1]))
            };
            d(a).total_cmp(&d(b))
        })
        .unwrap_or(0);
    let mut keep = vec![false; c.len()];
    if let Some(k) = keep.get_mut(0) {
        *k = true;
    }
    if let Some(k) = keep.get_mut(far) {
        *k = true;
    }
    let closed: Vec<[f64; 2]> = c.iter().copied().chain(std::iter::once(first)).collect();
    let mut stack = vec![(0usize, far), (far, c.len())];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (Some(pa), Some(pb)) = (closed.get(a), closed.get(b)) else {
            continue;
        };
        let (dx, dy) = (pb[0] - pa[0], pb[1] - pa[1]);
        let len = dx.hypot(dy).max(1e-12);
        let mut best = (0.0, a);
        for i in a + 1..b {
            if let Some(p) = closed.get(i) {
                let d = ((p[0] - pa[0]) * dy - (p[1] - pa[1]) * dx).abs() / len;
                if d > best.0 {
                    best = (d, i);
                }
            }
        }
        if best.0 > eps {
            if let Some(k) = keep.get_mut(best.1) {
                *k = true;
            }
            stack.push((a, best.1));
            stack.push((best.1, b));
        }
    }
    c.iter().zip(keep).filter_map(|(p, k)| k.then_some(*p)).collect()
}

/// Quanto Chaikin può tagliare da uno spigolo, in pixel: basta a togliere gli scalini, mentre
/// sui lati lunghi (un quarto di 500 pixel!) mangerebbe la punta di un triangolo.
const SMOOTH_CUT: f64 = 1.5;

/// Arrotonda gli angoli tagliando ogni lato a un quarto e tre quarti (al più `SMOOTH_CUT`
/// pixel dallo spigolo), `rounds` volte.
fn chaikin(c: &[[f64; 2]], rounds: usize) -> Contour {
    let mut pts = c.to_vec();
    for _ in 0..rounds {
        let n = pts.len();
        if n < 3 {
            break;
        }
        let mut next = Vec::with_capacity(n * 2);
        for i in 0..n {
            let (Some(a), Some(b)) = (pts.get(i), pts.get((i + 1) % n)) else {
                continue;
            };
            let len = (b[0] - a[0]).hypot(b[1] - a[1]);
            let t = if len > 0.0 {
                (SMOOTH_CUT / len).min(0.25)
            } else {
                0.25
            };
            next.push([(1.0 - t) * a[0] + t * b[0], (1.0 - t) * a[1] + t * b[1]]);
            next.push([t * a[0] + (1.0 - t) * b[0], t * a[1] + (1.0 - t) * b[1]]);
        }
        pts = next;
    }
    pts
}

/// L'area racchiusa da contorni (pari-dispari), in mm²: per i test e per dire quanto è grande.
pub fn contour_area(contours: &[Contour]) -> f64 {
    contours
        .to_vec()
        .simplify_shape(FillRule::EvenOdd)
        .iter()
        .flatten()
        .map(|c| area(c))
        .sum::<f64>()
        .abs()
}

/// Pixel rifatti appena dentro la linea di taglio prima di allungarne i colori: lì il soggetto
/// si mescola con lo sfondo (l'antialias) e l'abbondanza ne verrebbe un alone chiaro.
const EDGE_PX: f64 = 2.0;

/// Strati massimi con cui i colori si allungano oltre la fustella.
const MAX_LAYERS: usize = 16_000;

/// L'abbondanza lungo il contorno: fuori dalla fustella `cut` (in mm), fino a `band_mm` oltre,
/// ogni pixel prende il colore del pixel più vicino dentro la fustella. Dentro resta tutto
/// com'è (il bordo bianco di un adesivo resta bianco, e la sua abbondanza è bianca), salvo i
/// due pixel appena dentro la linea, rifatti col colore di poco più dentro.
pub(crate) fn bleed_around(
    img: &DynamicImage,
    dpi: f64,
    cut: &[Contour],
    band_mm: f64,
) -> Result<DynamicImage, ImageEditError> {
    let band = if band_mm.is_finite() {
        band_mm.clamp(0.0, 100.0)
    } else {
        0.0
    };
    if band < 0.05 || !dpi.is_finite() || dpi <= 0.0 {
        return Err(ImageEditError::BadSize);
    }
    let k = dpi / 25.4;
    let shapes = cut.to_vec().simplify_shape(FillRule::EvenOdd);
    let grown = |d: f64| -> Vec<Contour> {
        let style = OutlineStyle::new(d).line_join(LineJoin::Round(0.1));
        shapes
            .outline(&style)
            .into_iter()
            .flatten()
            .map(|c| c.iter().map(|p| [p[0] * k, p[1] * k]).collect())
            .collect()
    };
    let (w, h) = (img.width() as usize, img.height() as usize);
    let outer = rasterize(&grown(band), w, h);
    let seeds = rasterize(&grown(-EDGE_PX / k), w, h);
    if !seeds.iter().any(|v| *v) {
        return Err(ImageEditError::Unsupported("la fustella è troppo piccola"));
    }
    // Restano come sono i pixel ben dentro la fustella e quelli oltre l'abbondanza.
    let filled: Vec<bool> = seeds.iter().zip(&outer).map(|(s, o)| *s || !*o).collect();
    let layers = w.saturating_add(h).min(MAX_LAYERS);
    let seeds = &seeds;
    Ok(match img {
        DynamicImage::ImageLuma8(b) => DynamicImage::ImageLuma8(spread(b, seeds, filled, layers)),
        DynamicImage::ImageLumaA8(b) => DynamicImage::ImageLumaA8(spread(b, seeds, filled, layers)),
        DynamicImage::ImageRgb8(b) => DynamicImage::ImageRgb8(spread(b, seeds, filled, layers)),
        DynamicImage::ImageRgba8(b) => DynamicImage::ImageRgba8(spread(b, seeds, filled, layers)),
        DynamicImage::ImageLuma16(b) => DynamicImage::ImageLuma16(spread(b, seeds, filled, layers)),
        DynamicImage::ImageLumaA16(b) => DynamicImage::ImageLumaA16(spread(b, seeds, filled, layers)),
        DynamicImage::ImageRgb16(b) => DynamicImage::ImageRgb16(spread(b, seeds, filled, layers)),
        DynamicImage::ImageRgba16(b) => DynamicImage::ImageRgba16(spread(b, seeds, filled, layers)),
        DynamicImage::ImageRgb32F(b) => DynamicImage::ImageRgb32F(spread(b, seeds, filled, layers)),
        DynamicImage::ImageRgba32F(b) => DynamicImage::ImageRgba32F(spread(b, seeds, filled, layers)),
        other => DynamicImage::ImageRgba16(spread(&other.to_rgba16(), seeds, filled, layers)),
    })
}

/// I pixel il cui centro sta dentro i contorni (pari-dispari), riga per riga.
fn rasterize(contours: &[Contour], w: usize, h: usize) -> Vec<bool> {
    let mut out = vec![false; w * h];
    let mut xs: Vec<f64> = Vec::new();
    for y in 0..h {
        let cy = y as f64 + 0.5;
        xs.clear();
        for c in contours {
            let n = c.len();
            for i in 0..n {
                let (Some(a), Some(b)) = (c.get(i), c.get((i + 1) % n)) else {
                    continue;
                };
                if (a[1] > cy) != (b[1] > cy) {
                    xs.push(a[0] + (cy - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
        }
        xs.sort_by(f64::total_cmp);
        for pair in xs.chunks_exact(2) {
            let (Some(x0), Some(x1)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            let from = (x0 - 0.5).ceil().max(0.0) as usize;
            let to = ((x1 - 0.5).floor() + 1.0).clamp(0.0, w as f64) as usize;
            for x in from..to {
                if let Some(v) = out.get_mut(y * w + x) {
                    *v = true;
                }
            }
        }
    }
    out
}

/// I vicini di un pixel (gli otto attorno), come indici.
fn neighbours(i: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let (x, y) = ((i % w) as i64, (i / w) as i64);
    [
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ]
    .into_iter()
    .filter_map(move |(dx, dy)| {
        let (nx, ny) = (x + dx, y + dy);
        (nx >= 0 && ny >= 0 && nx < w as i64 && ny < h as i64).then(|| ny as usize * w + nx as usize)
    })
}

/// Allunga i colori dei pixel `seeds` sui vicini non ancora `filled`, uno strato alla volta,
/// per al più `band` strati.
fn spread<P: image::Pixel>(
    img: &image::ImageBuffer<P, Vec<P::Subpixel>>,
    seeds: &[bool],
    mut filled: Vec<bool>,
    band: usize,
) -> image::ImageBuffer<P, Vec<P::Subpixel>> {
    let mut out = img.clone();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut front: Vec<usize> = (0..seeds.len())
        .filter(|&i| {
            seeds.get(i).copied().unwrap_or(false)
                && neighbours(i, w, h).any(|n| !filled.get(n).copied().unwrap_or(true))
        })
        .collect();
    for _ in 0..band {
        let mut next = Vec::new();
        for &i in &front {
            let from = *out.get_pixel((i % w) as u32, (i / w) as u32);
            for n in neighbours(i, w, h) {
                if let Some(v) = filled.get_mut(n)
                    && !*v
                {
                    *v = true;
                    out.put_pixel((n % w) as u32, (n / w) as u32, from);
                    next.push(n);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        front = next;
    }
    out
}
