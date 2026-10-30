//! Motore di nesting delle sagome (ADR 0027), dal pannello per Illustrator (`nester.js`).
//!
//! Metodo: no-fit polygon (NFP) con piazzamento "in alto a sinistra".
//! - Ogni sagoma si riduce al suo contorno esterno (i fori restano del pezzo e non si riempiono),
//!   semplificato e allargato di metà distanza.
//! - Il contorno si scompone in parti convesse: l'NFP fra due pezzi è l'unione delle somme di
//!   Minkowski fra le loro parti, calcolate in tempo lineare.
//! - Ogni pezzo va nella posizione libera più in alto (poi più a sinistra), provando tutte le
//!   rotazioni ammesse.
//! - Un'ottimizzazione prova ordini diversi dei pezzi finché c'è tempo e tiene la disposizione
//!   migliore.
//!
//! Misure in millimetri, y verso il basso. Un punto p della sagoma finisce in R(rot)·F·p + (x, y)
//! sul foglio (rot in gradi, senso orario a schermo; F specchia in orizzontale, x → -x).

mod geometry;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use geometry::{Bounds, P, Poly};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay::Overlay;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::i_float::int::angle::Angle;
use i_overlay::mesh::int::outline::offset::IntOutlineOffset;
use i_overlay::mesh::int::style::{IntLineJoin, IntOutlineStyle};

/// Unità interne per millimetro (centesimi di millimetro).
const S: f64 = 100.0;
/// Tolleranza di semplificazione del contorno, in millimetri.
const SIMPLIFY_MM: f64 = 0.3;
/// Margine contro gli arrotondamenti, in millimetri.
const SAFETY_MM: f64 = 0.05;
/// Il foglio o il rotolo più grande, in millimetri (100 m): tiene i conti negli interi.
const MAX_MM: f64 = 100_000.0;
/// I pezzi in tutto (copie comprese).
pub const MAX_PIECES: usize = 3000;
/// I punti di una sagoma.
const MAX_POINTS: usize = 20_000;
/// L'area minima (mm²) di un foro che può contenere altri pezzi.
const MIN_HOLE_MM2: f64 = 25.0;

#[derive(Debug, thiserror::Error)]
pub enum NestError {
    #[error("{0}")]
    Invalid(String),
}

/// Una sagoma da disporre, ripetuta `copies` volte.
#[derive(Clone, Debug, Default)]
pub struct Item {
    /// I contorni in millimetri, y verso il basso; pari-dispari: un contorno dentro un altro è un
    /// foro.
    pub shape: Vec<Vec<[f64; 2]>>,
    pub copies: usize,
    /// Solo rotazioni di 90° (immagini, testi che non devono girare storti).
    pub right_angles_only: bool,
}

/// Dove si dispongono i pezzi.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stock {
    /// Lastre tutte uguali.
    Sheet { width: f64, height: f64 },
    /// Un rotolo largo `width`: la lunghezza si riduce al minimo; oltre `max_length` si passa a
    /// un altro pezzo di rotolo.
    Roll { width: f64, max_length: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub stock: Stock,
    /// Margine dal bordo del foglio (mm).
    pub margin: f64,
    /// Distanza minima fra due sagome (mm).
    pub spacing: f64,
    /// Passo di rotazione in gradi; 0 = nessuna rotazione.
    pub rotation_step: f64,
    /// Prova anche i pezzi specchiati.
    pub mirror: bool,
    /// Usa i fori dei pezzi per quelli piccoli.
    pub holes: bool,
    /// Tempo per cercare disposizioni migliori dopo la prima.
    pub time_limit: Duration,
    pub max_sheets: usize,
    pub seed: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            stock: Stock::Sheet {
                width: 3050.0,
                height: 2050.0,
            },
            margin: 10.0,
            spacing: 5.0,
            rotation_step: 90.0,
            mirror: false,
            holes: false,
            time_limit: Duration::from_secs(10),
            max_sheets: 50,
            seed: 1,
        }
    }
}

/// Dove va una copia.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub item: usize,
    pub copy: usize,
    /// Gradi, senso orario a schermo.
    pub rotation: f64,
    pub mirror: bool,
    pub x: f64,
    pub y: f64,
}

impl Placement {
    /// Il punto `p` della sagoma (mm) sul foglio.
    pub fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        let (s, c) = self.rotation.to_radians().sin_cos();
        let x = if self.mirror { -p[0] } else { p[0] };
        [x * c - p[1] * s + self.x, x * s + p[1] * c + self.y]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    pub width: f64,
    /// L'altezza della lastra, o la lunghezza usata del rotolo.
    pub height: f64,
    /// Area delle sagome (mm²).
    pub area: f64,
    pub placements: Vec<Placement>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub sheets: Vec<Sheet>,
    /// Le copie rimaste fuori: (sagoma, copia).
    pub unplaced: Vec<(usize, usize)>,
    pub placed: usize,
    /// Rotolo: la lunghezza di tutti i pezzi di rotolo (mm).
    pub total_length: Option<f64>,
    /// Area delle sagome sull'area dei fogli, da 0 a 1.
    pub utilization: f64,
    /// Disposizioni provate.
    pub tries: usize,
}

/// A che punto è il calcolo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    pub tries: usize,
    pub elapsed: Duration,
}

// --- Preparazione dei pezzi ------------------------------------------------------------------

/// Una sagoma girata e pronta: tutto spostato in modo che l'ingombro parta da (0, 0).
struct Variant {
    angle: f64,
    flip: bool,
    w: i32,
    h: i32,
    off_x: i32,
    off_y: i32,
    parts: Vec<Poly>,
    holes: Vec<Poly>,
    outline: Vec<Poly>,
}

struct Prepared {
    area: f64,
    variants: Vec<Variant>,
    bbox: Bounds,
}

fn to_int(c: &[[f64; 2]]) -> Poly {
    c.iter()
        .map(|p| P::new(geometry::round(p[0] * S), geometry::round(p[1] * S)))
        .collect()
}

fn overlay(subj: &[Poly], clip: &[Poly], rule: OverlayRule, fill: FillRule) -> Vec<Vec<Poly>> {
    Overlay::from_subj_and_clip(subj, clip).overlay(rule, fill)
}

fn union(paths: &[Poly], fill: FillRule) -> Vec<Vec<Poly>> {
    Overlay::from_subj(paths).overlay(OverlayRule::Subject, fill)
}

/// L'unione di tanti poligoni che si sovrappongono molto (le somme di Minkowski): a gruppi,
/// poi i risultati fra loro. Tutti insieme in un colpo solo costa molto di più. Ogni forma
/// passa intera (contorno e fori) al giro dopo.
fn union_many(paths: Vec<Poly>) -> Vec<Poly> {
    const GROUP: usize = 8;
    let mut shapes: Vec<Vec<Poly>> = paths.into_iter().map(|p| vec![p]).collect();
    while shapes.len() > GROUP {
        let next: Vec<Vec<Poly>> = shapes
            .chunks(GROUP)
            .flat_map(|g| {
                let flat: Vec<Poly> = g.iter().flatten().cloned().collect();
                union(&flat, FillRule::NonZero)
            })
            .collect();
        // Forme separate fra loro non si riducono più: si chiude con l'unione di tutte.
        let stuck = next.len() >= shapes.len();
        shapes = next;
        if stuck {
            break;
        }
    }
    let flat: Vec<Poly> = shapes.into_iter().flatten().collect();
    union(&flat, FillRule::NonZero).into_iter().flatten().collect()
}

/// Allarga (o restringe) le forme di `delta` unità, con gli spigoli a punta tagliata: il
/// risultato contiene sempre quello allargato con gli angoli tondi.
fn offset(shapes: &[Vec<Poly>], delta: i32) -> Vec<Vec<Poly>> {
    // Sotto i 60° la punta si taglia a una distanza che resta oltre il cerchio.
    let style = IntOutlineStyle::new(delta).line_join(IntLineJoin::Miter(Angle::from_bits((1u32 << 31) / 3)));
    shapes.outline(&style).unwrap_or_default()
}

fn prepare(item: &Item, cfg: &Config) -> Option<Prepared> {
    let rings: Vec<Poly> = item
        .shape
        .iter()
        .filter(|c| c.len() >= 3)
        .map(|c| to_int(c))
        .collect();
    if rings.iter().map(Vec::len).sum::<usize>() > MAX_POINTS {
        return None;
    }
    // Pari-dispari, come i tracciati composti: i contorni dentro sono fori.
    let shapes = union(&rings, FillRule::EvenOdd);
    let outers: Vec<Poly> = shapes.iter().filter_map(|s| s.first().cloned()).collect();
    if outers.is_empty() {
        return None;
    }
    let area = outers.iter().map(|p| geometry::area(p).abs()).sum::<f64>() / (S * S);

    // Semplifica, poi allarga di metà distanza più la tolleranza: la sagoma del calcolo contiene
    // sempre quella vera. Se la semplificazione non toglie punti (i rettangoli) la tolleranza non
    // serve, così i pezzi possono toccarsi.
    let mut dev = 0.0;
    let mut simplify = |p: &Poly| -> Poly {
        let c = geometry::clean(p);
        let s = geometry::simplify(&c, SIMPLIFY_MM * S);
        if s.len() < c.len() {
            dev = SIMPLIFY_MM;
        }
        s
    };
    let source: Vec<Vec<Poly>> = if cfg.holes {
        shapes
            .iter()
            .map(|s| s.iter().map(&mut simplify).filter(|c| c.len() >= 3).collect())
            .collect()
    } else {
        outers.iter().map(|o| vec![simplify(o)]).collect()
    };
    let grow = geometry::round((cfg.spacing / 2.0 + dev + SAFETY_MM) * S);
    let grown = offset(&source, grow);
    let inflated: Vec<Poly> = grown
        .iter()
        .filter_map(|s| s.first())
        .map(|p| geometry::clean(p))
        .filter(|p| p.len() >= 3)
        .collect();
    if inflated.is_empty() {
        return None;
    }
    // I fori già ristretti della distanza, abbastanza grandi da contenere qualcosa.
    let holes: Vec<Poly> = if cfg.holes {
        grown
            .iter()
            .flat_map(|s| s.iter().skip(1))
            .filter(|h| geometry::area(h).abs() > MIN_HOLE_MM2 * S * S)
            .map(|h| geometry::positive(&geometry::clean(h)))
            .collect()
    } else {
        Vec::new()
    };
    let parts: Vec<Poly> = inflated.iter().flat_map(|p| geometry::convex_parts(p)).collect();

    let mut angles: Vec<f64> = cfg
        .angles
        .iter()
        .copied()
        .filter(|a| !item.right_angles_only || a % 90.0 == 0.0)
        .collect();
    if angles.is_empty() {
        angles.push(0.0);
    }
    let flips: &[bool] = if cfg.mirror { &[false, true] } else { &[false] };
    let mut variants = Vec::new();
    for &flip in flips {
        let tf = |p: &Poly| -> Poly {
            if flip {
                geometry::positive(&geometry::mirror(p))
            } else {
                p.clone()
            }
        };
        for &angle in &angles {
            let rot_outers: Vec<Poly> = inflated.iter().map(|p| geometry::rotate(&tf(p), angle)).collect();
            let b = geometry::bounds(&rot_outers);
            let (dx, dy) = (b.min_x.saturating_neg(), b.min_y.saturating_neg());
            let place =
                |p: &Poly| geometry::clean(&geometry::translate(&geometry::rotate(&tf(p), angle), dx, dy));
            variants.push(Variant {
                angle,
                flip,
                w: b.width(),
                h: b.height(),
                off_x: b.min_x,
                off_y: b.min_y,
                parts: parts.iter().map(place).filter(|p| p.len() >= 3).collect(),
                holes: holes.iter().map(place).filter(|p| p.len() >= 3).collect(),
                outline: rot_outers
                    .iter()
                    .map(|p| geometry::translate(p, dx, dy))
                    .collect(),
            });
        }
    }
    Some(Prepared {
        area,
        bbox: geometry::bounds(&inflated),
        variants,
    })
}

// --- NFP -------------------------------------------------------------------------------------

struct Nfp {
    paths: Vec<Poly>,
    bounds: Bounds,
}

/// Una variante di una sagoma: (sagoma, variante).
type Key = (usize, usize);

/// Le sagome preparate e le zone vietate già calcolate (le copie della stessa sagoma le
/// condividono).
struct Job {
    shapes: Vec<Prepared>,
    /// Per ogni copia: la sua sagoma.
    shape_of: Vec<usize>,
    nfp: HashMap<(Key, Key), Nfp>,
    holes: HashMap<(Key, Key), Option<Nfp>>,
}

fn negate(parts: &[Poly]) -> Vec<Poly> {
    parts
        .iter()
        .map(|p| {
            p.iter()
                .map(|q| P::new(q.x.saturating_neg(), q.y.saturating_neg()))
                .collect()
        })
        .collect()
}

impl Job {
    fn variant(&self, k: Key) -> Option<&Variant> {
        self.shapes.get(k.0)?.variants.get(k.1)
    }

    /// Dove non può andare il riferimento di B quando A sta nell'origine.
    fn nfp(&mut self, a: Key, b: Key) -> Option<&Nfp> {
        if !self.nfp.contains_key(&(a, b)) {
            let (va, vb) = (self.variant(a)?, self.variant(b)?);
            let neg_b = negate(&vb.parts);
            let mut sums = Vec::with_capacity(va.parts.len() * neg_b.len());
            for pa in &va.parts {
                for nb in &neg_b {
                    let s = geometry::minkowski_convex(pa, nb);
                    if s.len() >= 3 {
                        sums.push(geometry::positive(&s));
                    }
                }
            }
            let paths = union_many(sums);
            let bounds = geometry::bounds(&paths);
            self.nfp.insert((a, b), Nfp { paths, bounds });
        }
        self.nfp.get(&(a, b))
    }

    /// Dove può andare il riferimento di B dentro i fori di A (A nell'origine): B non deve
    /// toccare il bordo del foro e un suo punto deve stare dentro.
    fn hole_fit(&mut self, a: Key, b: Key) -> Option<&Nfp> {
        if !self.holes.contains_key(&(a, b)) {
            let entry = self.compute_hole_fit(a, b);
            self.holes.insert((a, b), entry);
        }
        self.holes.get(&(a, b)).and_then(Option::as_ref)
    }

    fn compute_hole_fit(&self, a: Key, b: Key) -> Option<Nfp> {
        let (va, vb) = (self.variant(a)?, self.variant(b)?);
        let [outline] = vb.outline.as_slice() else {
            return None;
        };
        let c0 = *outline.first()?;
        let neg = negate(&vb.parts);
        let mut out = Vec::new();
        for h in &va.holes {
            let hb = geometry::bounds([h]);
            if hb.width() < vb.w || hb.height() < vb.h {
                continue;
            }
            // Le posizioni in cui B tocca il bordo del foro: ogni lato del foro sommato a -B (per
            // una parte convessa, l'involucro delle due copie spostate agli estremi del lato).
            let mut band = Vec::new();
            let n = h.len();
            for i in 0..n {
                let (Some(&p), Some(&q)) = (h.get(i), h.get((i + 1) % n)) else {
                    continue;
                };
                for nb in &neg {
                    let mut pts = geometry::translate(nb, p.x, p.y);
                    pts.extend(geometry::translate(nb, q.x, q.y));
                    let s = geometry::convex_hull(&pts);
                    if s.len() >= 3 {
                        band.push(s);
                    }
                }
            }
            let inside = geometry::translate(h, c0.x.saturating_neg(), c0.y.saturating_neg());
            let free = overlay(&[inside], &band, OverlayRule::Difference, FillRule::NonZero);
            out.extend(free.into_iter().flatten());
        }
        (!out.is_empty()).then(|| Nfp {
            bounds: geometry::bounds(&out),
            paths: out,
        })
    }
}

// --- Piazzamento -----------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Placed {
    /// La copia.
    copy: usize,
    key: Key,
    x: i32,
    y: i32,
}

#[derive(Clone, Copy)]
struct Bin {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

struct Filled {
    placed: Vec<Placed>,
    rest: Vec<usize>,
    ext_y: i32,
}

fn lex_less(a: [f64; 2], b: [f64; 2]) -> bool {
    a[0] < b[0] || (a[0] == b[0] && a[1] < b[1])
}

/// Riempie un foglio con le copie nell'ordine dato.
fn fill_bin(order: &[usize], job: &mut Job, bin: Bin) -> Filled {
    let mut placed: Vec<Placed> = Vec::new();
    let mut rest = Vec::new();
    let mut ext_y = bin.y0;
    for &copy in order {
        let Some(&shape) = job.shape_of.get(copy) else {
            continue;
        };
        let count = job.shapes.get(shape).map_or(0, |s| s.variants.len());
        let mut best: Option<(Placed, [f64; 2])> = None;
        for vi in 0..count {
            let Some((w, h)) = job.variant((shape, vi)).map(|v| (v.w, v.h)) else {
                continue;
            };
            let ifp = Bin {
                x0: bin.x0,
                y0: bin.y0,
                x1: bin.x1.saturating_sub(w),
                y1: bin.y1.saturating_sub(h),
            };
            if ifp.x1 < ifp.x0 || ifp.y1 < ifp.y0 {
                continue;
            }
            for q in candidates(ifp, &placed, job, (shape, vi)) {
                // Prima la lunghezza usata, poi una "gravità" verso l'alto a sinistra.
                let score = [
                    f64::from(ext_y.max(q.y.saturating_add(h))),
                    f64::from(q.y) + f64::from(h) / 2.0 + f64::from(q.x) * 0.1,
                ];
                if best.as_ref().is_none_or(|(_, s)| lex_less(score, *s)) {
                    let p = Placed {
                        copy,
                        key: (shape, vi),
                        x: q.x,
                        y: q.y,
                    };
                    best = Some((p, score));
                }
            }
        }
        match best {
            Some((b, _)) => {
                let h = job.variant(b.key).map_or(0, |v| v.h);
                ext_y = ext_y.max(b.y.saturating_add(h));
                placed.push(b);
            }
            None => rest.push(copy),
        }
    }
    Filled { placed, rest, ext_y }
}

fn near(b: &Bounds, dx: i32, dy: i32, ifp: Bin) -> bool {
    !(b.max_x.saturating_add(dx) < ifp.x0
        || b.min_x.saturating_add(dx) > ifp.x1.saturating_add(1)
        || b.max_y.saturating_add(dy) < ifp.y0
        || b.min_y.saturating_add(dy) > ifp.y1.saturating_add(1))
}

/// Le posizioni da provare per la variante `b`: i vertici delle zone libere.
fn candidates(ifp: Bin, placed: &[Placed], job: &mut Job, b: Key) -> Vec<P> {
    if placed.is_empty() {
        return vec![P::new(ifp.x0, ifp.y0)];
    }
    // Il rettangolo dei punti ammessi, allargato di 1 per non perdere i pezzi che entrano giusti.
    let rect = vec![
        P::new(ifp.x0, ifp.y0),
        P::new(ifp.x1.saturating_add(1), ifp.y0),
        P::new(ifp.x1.saturating_add(1), ifp.y1.saturating_add(1)),
        P::new(ifp.x0, ifp.y1.saturating_add(1)),
    ];
    // Le zone vietate dai pezzi già piazzati.
    let nfps: Vec<Vec<Poly>> = placed
        .iter()
        .map(|a| match job.nfp(a.key, b) {
            Some(nfp) if near(&nfp.bounds, a.x, a.y, ifp) => nfp
                .paths
                .iter()
                .map(|p| geometry::translate(p, a.x, a.y))
                .collect(),
            _ => Vec::new(),
        })
        .collect();

    let mut out = Vec::new();
    let all: Vec<Poly> = nfps.iter().flatten().cloned().collect();
    let free = overlay(&[rect], &all, OverlayRule::Difference, FillRule::NonZero);
    for q in free.iter().flatten().flatten() {
        out.push(P::new(q.x.clamp(ifp.x0, ifp.x1), q.y.clamp(ifp.y0, ifp.y1)));
    }

    // Pezzi dentro i fori: posizioni nel foro di A, tolte quelle occupate dagli altri.
    for (i, a) in placed.iter().enumerate() {
        if job.variant(a.key).is_none_or(|v| v.holes.is_empty()) {
            continue;
        }
        let Some(hf) = job.hole_fit(a.key, b) else {
            continue;
        };
        if !near(&hf.bounds, a.x, a.y, ifp) {
            continue;
        }
        let subj: Vec<Poly> = hf
            .paths
            .iter()
            .map(|p| geometry::translate(p, a.x, a.y))
            .collect();
        let others: Vec<Poly> = nfps
            .iter()
            .enumerate()
            .filter(|(k, _)| *k != i)
            .flat_map(|(_, p)| p.iter().cloned())
            .collect();
        let in_hole = overlay(&subj, &others, OverlayRule::Difference, FillRule::NonZero);
        for q in in_hole.iter().flatten().flatten() {
            if q.x >= ifp.x0 && q.x <= ifp.x1 && q.y >= ifp.y0 && q.y <= ifp.y1 {
                out.push(*q);
            }
        }
    }
    out
}

struct Layout {
    sheets: Vec<Filled>,
    unplaced: Vec<usize>,
}

/// Dispone tutte le copie su uno o più fogli (o pezzi di rotolo).
fn layout(order: &[usize], job: &mut Job, cfg: &Config) -> Layout {
    let d = (cfg.spacing / 2.0 + SAFETY_MM) * S;
    let bin = Bin {
        x0: geometry::round(cfg.margin * S - d),
        y0: geometry::round(cfg.margin * S - d),
        x1: geometry::round((cfg.width - cfg.margin) * S + d),
        y1: geometry::round((cfg.height - cfg.margin) * S + d),
    };
    let mut sheets = Vec::new();
    let mut remaining = order.to_vec();
    while !remaining.is_empty() && sheets.len() < cfg.max_sheets {
        let res = fill_bin(&remaining, job, bin);
        if res.placed.is_empty() {
            break;
        }
        remaining.clone_from(&res.rest);
        sheets.push(res);
    }
    Layout {
        sheets,
        unplaced: remaining,
    }
}

/// Meno copie fuori, poi meno fogli, poi meno lunghezza nell'ultimo.
fn score_of(lay: &Layout) -> [f64; 3] {
    let last = lay.sheets.last().map_or(0, |s| s.ext_y);
    [
        lay.unplaced.len() as f64,
        lay.sheets.len() as f64,
        f64::from(last),
    ]
}

fn better(a: [f64; 3], b: [f64; 3]) -> bool {
    for (x, y) in a.iter().zip(b) {
        if *x < y {
            return true;
        }
        if *x > y {
            return false;
        }
    }
    false
}

// --- Ottimizzazione --------------------------------------------------------------------------

/// Generatore pseudo-casuale (mulberry32), lo stesso del pannello.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6D2B_79F5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }

    fn below(&mut self, n: usize) -> usize {
        ((self.next() * n as f64) as usize).min(n.saturating_sub(1))
    }
}

fn mutate(order: &[usize], rng: &mut Rng) -> Vec<usize> {
    let mut o = order.to_vec();
    let n = o.len();
    if n < 2 {
        return o;
    }
    let (i, j) = (rng.below(n), rng.below(n));
    if rng.next() < 0.5 {
        o.swap(i, j);
    } else {
        let item = o.remove(i);
        o.insert(j.min(o.len()), item);
    }
    o
}

struct Config {
    width: f64,
    height: f64,
    roll: bool,
    margin: f64,
    spacing: f64,
    angles: Vec<f64>,
    mirror: bool,
    holes: bool,
    max_sheets: usize,
}

fn config(o: &Options) -> Result<Config, NestError> {
    let (width, height, roll) = match o.stock {
        Stock::Sheet { width, height } => (width, height, false),
        Stock::Roll { width, max_length } => (width, max_length, true),
    };
    let ok = |v: f64| v.is_finite() && (1.0..=MAX_MM).contains(&v);
    if !ok(width) || !ok(height) {
        return Err(NestError::Invalid("misura del foglio non valida".into()));
    }
    let margin = if o.margin.is_finite() {
        o.margin.clamp(0.0, 1000.0)
    } else {
        0.0
    };
    if 2.0 * margin >= width.min(height) {
        return Err(NestError::Invalid("il margine è più grande del foglio".into()));
    }
    let spacing = if o.spacing.is_finite() {
        o.spacing.clamp(0.0, 1000.0)
    } else {
        0.0
    };
    let step = o.rotation_step;
    let mut angles = vec![0.0];
    if step.is_finite() && (1.0..360.0).contains(&step) {
        angles.clear();
        let mut a = 0.0;
        while a < 360.0 - 1e-9 {
            angles.push((a * 1000.0_f64).round() / 1000.0);
            a += step;
        }
    }
    Ok(Config {
        width,
        height,
        roll,
        margin,
        spacing,
        angles,
        mirror: o.mirror,
        holes: o.holes,
        max_sheets: o.max_sheets.clamp(1, 500),
    })
}

fn outcome(
    lay: &Layout,
    job: &Job,
    copies: &[(usize, usize)],
    extra: &[(usize, usize)],
    cfg: &Config,
    tries: usize,
) -> Outcome {
    let d = (cfg.spacing / 2.0 + SAFETY_MM) * S;
    let sheets: Vec<Sheet> = lay
        .sheets
        .iter()
        .map(|sh| {
            let used = ((f64::from(sh.ext_y) - d) / S + cfg.margin).min(cfg.height);
            let placements = sh
                .placed
                .iter()
                .filter_map(|p| {
                    let v = job.variant(p.key)?;
                    let &(item, copy) = copies.get(p.copy)?;
                    Some(Placement {
                        item,
                        copy,
                        rotation: v.angle,
                        mirror: v.flip,
                        x: f64::from(p.x.saturating_sub(v.off_x)) / S,
                        y: f64::from(p.y.saturating_sub(v.off_y)) / S,
                    })
                })
                .collect();
            Sheet {
                width: cfg.width,
                height: if cfg.roll { used } else { cfg.height },
                area: sh
                    .placed
                    .iter()
                    .filter_map(|p| job.shapes.get(p.key.0))
                    .map(|s| s.area)
                    .sum(),
                placements,
            }
        })
        .collect();
    let mut unplaced: Vec<(usize, usize)> = lay
        .unplaced
        .iter()
        .filter_map(|&c| copies.get(c).copied())
        .collect();
    unplaced.extend_from_slice(extra);
    unplaced.sort_unstable();
    let piece_area: f64 = sheets.iter().map(|s| s.area).sum();
    let sheet_area: f64 = sheets.iter().map(|s| s.width * s.height).sum();
    let placed = sheets.iter().map(|s| s.placements.len()).sum();
    Outcome {
        placed,
        total_length: cfg.roll.then(|| sheets.iter().map(|s| s.height).sum()),
        utilization: if sheet_area > 0.0 {
            piece_area / sheet_area
        } else {
            0.0
        },
        sheets,
        unplaced,
        tries,
    }
}

/// Dispone le copie di `items`. Cerca disposizioni migliori fino a `options.time_limit`, o
/// finché `stop` diventa vero; `on_improve` riceve ogni disposizione migliore della precedente,
/// `on_progress` il conto dei tentativi.
pub fn nest(
    items: &[Item],
    options: &Options,
    stop: &AtomicBool,
    mut on_improve: impl FnMut(&Outcome),
    mut on_progress: impl FnMut(Progress),
) -> Result<Outcome, NestError> {
    let t0 = Instant::now();
    let cfg = config(options)?;
    let total: usize = items.iter().map(|i| i.copies).fold(0usize, usize::saturating_add);
    if total > MAX_PIECES {
        return Err(NestError::Invalid(format!(
            "troppi pezzi: {total} (al massimo {MAX_PIECES})"
        )));
    }
    let mut job = Job {
        shapes: Vec::new(),
        shape_of: Vec::with_capacity(total),
        nfp: HashMap::new(),
        holes: HashMap::new(),
    };
    // Le copie da disporre e quelle senza una forma (restano fuori).
    let mut copies: Vec<(usize, usize)> = Vec::with_capacity(total);
    let mut no_shape: Vec<(usize, usize)> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if item.copies == 0 {
            continue;
        }
        match prepare(item, &cfg) {
            Some(p) => {
                let shape = job.shapes.len();
                job.shapes.push(p);
                for c in 0..item.copies {
                    copies.push((i, c));
                    job.shape_of.push(shape);
                }
            }
            None => no_shape.extend((0..item.copies).map(|c| (i, c))),
        }
    }

    let order_by = |key: &dyn Fn(&Bounds) -> f64| -> Vec<usize> {
        let mut o: Vec<usize> = (0..copies.len()).collect();
        let k = |c: usize| {
            job.shape_of
                .get(c)
                .and_then(|&s| job.shapes.get(s))
                .map_or(0.0, |s| key(&s.bbox))
        };
        o.sort_by(|&a, &b| k(b).total_cmp(&k(a)));
        o
    };
    let w = |b: &Bounds| f64::from(b.width());
    let h = |b: &Bounds| f64::from(b.height());
    let starts = [
        order_by(&|b| w(b) * h(b)),
        order_by(&|b| w(b).max(h(b))),
        order_by(&h),
        order_by(&w),
    ];

    let mut tries = 0usize;
    let mut best: Option<(Layout, [f64; 3], Vec<usize>)> = None;
    let deadline = t0 + options.time_limit;
    let stopped = || stop.load(Ordering::Relaxed);
    let mut queue: Vec<Vec<usize>> = starts.into_iter().rev().collect();
    let mut rng = Rng(u32::try_from(options.seed & 0xFFFF_FFFF).unwrap_or(1));
    loop {
        // La prima disposizione si calcola sempre per intero; poi gli altri ordini di partenza e
        // piccole modifiche all'ordine migliore, finché c'è tempo.
        if tries > 0 && (Instant::now() >= deadline || stopped()) {
            break;
        }
        let order = match queue.pop() {
            Some(o) => o,
            None => {
                let Some(base) = best.as_ref().map(|(_, _, o)| o.clone()) else {
                    break;
                };
                if base.len() < 2 {
                    break;
                }
                let mut cand = mutate(&base, &mut rng);
                if rng.next() < 0.3 {
                    cand = mutate(&cand, &mut rng);
                }
                cand
            }
        };
        let lay = layout(&order, &mut job, &cfg);
        tries += 1;
        let sc = score_of(&lay);
        if best.as_ref().is_none_or(|(_, s, _)| better(sc, *s)) {
            on_improve(&outcome(&lay, &job, &copies, &no_shape, &cfg, tries));
            best = Some((lay, sc, order));
        }
        on_progress(Progress {
            tries,
            elapsed: t0.elapsed(),
        });
    }

    let empty = Layout {
        sheets: Vec::new(),
        unplaced: (0..copies.len()).collect(),
    };
    let lay = best.as_ref().map_or(&empty, |(l, _, _)| l);
    Ok(outcome(lay, &job, &copies, &no_shape, &cfg, tries))
}

#[cfg(test)]
mod tests;
