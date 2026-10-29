//! I tracciati di una pagina, per lo strumento "Seleziona tracciato": dove sono (in millimetri
//! della pagina come si vede), di che colore, quanto spessi, se sono in sovrastampa; e il cambio
//! di colore (anche in una tinta piatta nuova, per esempio la fustella), di spessore e di
//! sovrastampa di quelli scelti.
//!
//! Il cambio tocca solo l'operatore che dipinge il tracciato: prima del tracciato si salva lo
//! stato grafico (`q`), si mettono colore, spessore e sovrastampa nuovi, e dopo l'operatore lo si
//! ripristina (`Q`). Il resto del contenuto resta com'è, operatore per operatore.
//!
//! Un tracciato dentro un modulo (form XObject) può essere disegnato più volte: cambiarlo cambia
//! tutte le sue copie.

use std::collections::{HashMap, HashSet};

use pdfcraft_content::{Matrix, Op, parse, serialize_ops};
use pdfcraft_cos::{Dict, Document, ObjRef, Object, Stream};
use pdfcraft_organize as organize;
use winfiles_geom::pt_to_mm;

use crate::EditError;
use crate::spots::{MAX_CONTENT, add_states, dict_of, gs_name, page_content, page_resources};

/// Profondità massima di moduli dentro moduli.
const MAX_DEPTH: usize = 24;
/// Tracciati letti al massimo da una pagina: oltre, l'elenco si ferma (e lo dice).
const MAX_PATHS: usize = 200_000;
/// Punti tenuti al massimo per pagina, sommando tutti i tracciati.
const MAX_POINTS: usize = 6_000_000;
/// Operatori interpretati al massimo per pagina (moduli ripetuti compresi).
const MAX_OPS: usize = 20_000_000;
/// Prefisso degli spazi colore aggiunti nelle risorse.
const CS_PREFIX: &str = "WFTinta";

/// Il contenuto che disegna il tracciato: la pagina o un modulo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Owner {
    Page(ObjRef),
    Form(ObjRef),
}

/// Quale tracciato: il contenuto che lo disegna e la posizione dell'operatore che lo dipinge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PathId {
    owner: Owner,
    op: usize,
}

/// Un colore come è scritto nel file.
#[derive(Clone, Debug, PartialEq)]
pub enum Ink {
    Gray(f64),
    Rgb([f64; 3]),
    Cmyk([f64; 4]),
    /// Una tinta piatta (Separation) con la sua percentuale, da 0 a 1.
    Spot {
        name: String,
        tint: f64,
    },
    /// Tutto il resto (DeviceN, pattern, Lab, indicizzati): solo una descrizione.
    Other(String),
}

impl Ink {
    /// Lo stesso colore, a meno di mezzo punto percentuale.
    pub fn same(&self, other: &Ink) -> bool {
        let close =
            |a: &[f64], b: &[f64]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.005);
        match (self, other) {
            (Ink::Gray(a), Ink::Gray(b)) => close(&[*a], &[*b]),
            (Ink::Rgb(a), Ink::Rgb(b)) => close(a, b),
            (Ink::Cmyk(a), Ink::Cmyk(b)) => close(a, b),
            (Ink::Spot { name: a, tint: x }, Ink::Spot { name: b, tint: y }) => a == b && close(&[*x], &[*y]),
            (Ink::Other(a), Ink::Other(b)) => a == b,
            _ => false,
        }
    }

    /// Il colore come lo si legge: "C 0 M 100 Y 0 K 0", "Tinta CutContour 100%".
    pub fn label(&self) -> String {
        let pc = |v: f64| format!("{:.0}", (v * 100.0).clamp(0.0, 100.0));
        match self {
            Ink::Gray(g) => format!("Grigio, nero {}%", pc(1.0 - g)),
            Ink::Rgb([r, g, b]) => format!(
                "RGB {:.0} {:.0} {:.0}",
                r.clamp(0.0, 1.0) * 255.0,
                g.clamp(0.0, 1.0) * 255.0,
                b.clamp(0.0, 1.0) * 255.0
            ),
            Ink::Cmyk([c, m, y, k]) => format!("C {} M {} Y {} K {}", pc(*c), pc(*m), pc(*y), pc(*k)),
            Ink::Spot { name, tint } => format!("Tinta {name} {}%", pc(*tint)),
            Ink::Other(d) => d.clone(),
        }
    }

    /// Il colore per lo schermo, quando si sa dire senza il file (non per le tinte piatte).
    pub fn screen(&self) -> Option<[u8; 3]> {
        let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        match self {
            Ink::Gray(g) => Some([byte(*g); 3]),
            Ink::Rgb([r, g, b]) => Some([byte(*r), byte(*g), byte(*b)]),
            Ink::Cmyk([c, m, y, k]) => {
                let ink = |v: f64| (1.0 - v) * (1.0 - k);
                Some([byte(ink(*c)), byte(ink(*m)), byte(ink(*y))])
            }
            _ => None,
        }
    }

    /// È un colore di quadricromia, scala di grigi o RGB (non una tinta piatta).
    pub fn is_process(&self) -> bool {
        matches!(self, Ink::Gray(_) | Ink::Rgb(_) | Ink::Cmyk(_))
    }
}

/// Il contorno di un tracciato.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub ink: Ink,
    /// Spessore in millimetri sulla pagina (0 = il più sottile che la macchina sa fare).
    pub width: f64,
    pub overprint: bool,
}

/// Il riempimento di un tracciato.
#[derive(Clone, Debug, PartialEq)]
pub struct Fill {
    pub ink: Ink,
    pub overprint: bool,
}

/// Un pezzo di tracciato: punti in millimetri dall'angolo in alto a sinistra della pagina come
/// si vede (le curve sono spezzate in segmenti corti).
#[derive(Clone, Debug, PartialEq)]
pub struct SubPath {
    pub points: Vec<[f64; 2]>,
    pub closed: bool,
}

/// Un tracciato dipinto sulla pagina.
#[derive(Clone, Debug)]
pub struct PathInfo {
    pub id: PathId,
    pub parts: Vec<SubPath>,
    pub line: Option<Line>,
    pub fill: Option<Fill>,
    /// Il rettangolo che lo contiene: x0, y0, x1, y1 in millimetri.
    pub bounds: [f64; 4],
    /// La lunghezza di tutti i pezzi, in millimetri.
    pub length: f64,
    /// Sta dentro un modulo: cambiarlo cambia ogni copia del modulo.
    pub in_form: bool,
    /// Ritaglia anche quello che viene dopo: non si può cambiare senza cambiare il disegno.
    pub clips: bool,
    /// Millimetri per unità del contenuto, per scrivere uno spessore nuovo.
    scale: f64,
}

impl PathInfo {
    /// Tutti i pezzi sono chiusi.
    pub fn closed(&self) -> bool {
        !self.parts.is_empty() && self.parts.iter().all(|p| p.closed)
    }
}

/// Cosa c'è su una pagina.
#[derive(Clone, Debug, Default)]
pub struct PageScan {
    pub paths: Vec<PathInfo>,
    /// Gli altri oggetti dipinti (immagini, sfumature), come rettangoli in millimetri.
    pub others: Vec<[f64; 4]>,
    /// Il formato finito (TrimBox) sulla pagina come si vede, se diverso dalla pagina.
    pub trim: Option<[f64; 4]>,
    /// La pagina come si vede, in millimetri.
    pub width: f64,
    pub height: f64,
    /// La pagina ha più tracciati di quelli letti.
    pub truncated: bool,
}

/// Lo spazio colore in uso.
#[derive(Clone, Debug, PartialEq)]
enum Space {
    Gray,
    Rgb,
    Cmyk,
    Spot(String),
    Pattern,
    Other(String),
}

impl Space {
    fn initial(&self) -> Ink {
        match self {
            Space::Gray => Ink::Gray(0.0),
            Space::Rgb => Ink::Rgb([0.0; 3]),
            Space::Cmyk => Ink::Cmyk([0.0, 0.0, 0.0, 1.0]),
            Space::Spot(name) => Ink::Spot {
                name: name.clone(),
                tint: 1.0,
            },
            Space::Pattern => Ink::Other("Motivo (pattern)".into()),
            Space::Other(d) => Ink::Other(d.clone()),
        }
    }

    fn ink(&self, values: &[f64]) -> Ink {
        match (self, values) {
            (Space::Gray, [g]) => Ink::Gray(*g),
            (Space::Rgb, [r, g, b]) => Ink::Rgb([*r, *g, *b]),
            (Space::Cmyk, [c, m, y, k]) => Ink::Cmyk([*c, *m, *y, *k]),
            (Space::Spot(name), [t]) => Ink::Spot {
                name: name.clone(),
                tint: *t,
            },
            (Space::Pattern, _) => Ink::Other("Motivo (pattern)".into()),
            (Space::Other(d), _) => Ink::Other(d.clone()),
            _ => self.initial(),
        }
    }
}

/// Lo spazio colore che un nome o un oggetto descrive.
fn space_of(doc: &Document, cs: &Object, depth: usize) -> Space {
    let cs = doc.resolve(cs);
    if let Some(n) = cs.as_name() {
        return match n {
            b"DeviceGray" | b"G" | b"CalGray" => Space::Gray,
            b"DeviceRGB" | b"RGB" | b"CalRGB" => Space::Rgb,
            b"DeviceCMYK" | b"CMYK" => Space::Cmyk,
            b"Pattern" => Space::Pattern,
            other => Space::Other(String::from_utf8_lossy(other).into_owned()),
        };
    }
    let Some(a) = cs.as_array() else {
        return Space::Other("Spazio colore sconosciuto".into());
    };
    match a.first().and_then(Object::as_name).unwrap_or(b"") {
        b"Separation" => {
            let name = a
                .get(1)
                .map(|n| doc.resolve(n))
                .and_then(|n| n.as_name().map(|n| String::from_utf8_lossy(n).into_owned()))
                .unwrap_or_default();
            match name.as_str() {
                "All" => Space::Other("Registrazione (tutte le lastre)".into()),
                "None" => Space::Other("Nessuna lastra".into()),
                _ => Space::Spot(name),
            }
        }
        b"ICCBased" => {
            let n = a.get(1).map(|s| doc.resolve(s)).and_then(|s| match &*s {
                Object::Stream(s) => s.dict.int(b"N"),
                _ => None,
            });
            match n {
                Some(1) => Space::Gray,
                Some(3) => Space::Rgb,
                Some(4) => Space::Cmyk,
                _ => Space::Other("Profilo ICC".into()),
            }
        }
        b"CalGray" => Space::Gray,
        b"CalRGB" => Space::Rgb,
        b"Pattern" => Space::Pattern,
        b"DeviceN" => {
            let names: Vec<String> = match a.get(1).map(|n| doc.resolve(n)).as_deref() {
                Some(Object::Array(v)) => v
                    .iter()
                    .filter_map(Object::as_name)
                    .map(|n| String::from_utf8_lossy(n).into_owned())
                    .collect(),
                _ => Vec::new(),
            };
            Space::Other(format!("DeviceN ({})", names.join(", ")))
        }
        b"Indexed" if depth < MAX_DEPTH => match a.get(1).map(|b| space_of(doc, b, depth + 1)) {
            Some(Space::Spot(n)) => Space::Other(format!("Indicizzato su {n}")),
            _ => Space::Other("Colori indicizzati".into()),
        },
        b"Lab" => Space::Other("Lab".into()),
        other => Space::Other(String::from_utf8_lossy(other).into_owned()),
    }
}

/// Uno stato grafico delle risorse: sovrastampa del contorno (OP), del riempimento (op),
/// spessore (LW), quando li dà.
#[derive(Clone, Copy, Default)]
struct GsEntry {
    stroke: Option<bool>,
    fill: Option<bool>,
    width: Option<f64>,
}

/// Le risorse di un contenuto che servono qui.
#[derive(Default)]
struct Res {
    spaces: HashMap<Vec<u8>, Space>,
    states: HashMap<Vec<u8>, GsEntry>,
    images: HashSet<Vec<u8>>,
    forms: HashMap<Vec<u8>, ObjRef>,
}

fn read_res(doc: &Document, resources: &Dict) -> Res {
    let mut r = Res::default();
    for (k, v) in dict_of(doc, resources.get(b"ColorSpace")).iter() {
        r.spaces.insert(k.clone(), space_of(doc, v, 0));
    }
    for (k, v) in dict_of(doc, resources.get(b"ExtGState")).iter() {
        let g = dict_of(doc, Some(v));
        let flag = |key: &[u8]| match g.get(key).map(|o| doc.resolve(o)).as_deref() {
            Some(Object::Bool(b)) => Some(*b),
            _ => None,
        };
        let lw = g.get(b"LW").map(|o| doc.resolve(o)).and_then(|o| o.as_f64());
        r.states.insert(
            k.clone(),
            GsEntry {
                stroke: flag(b"OP"),
                fill: flag(b"op"),
                width: lw,
            },
        );
    }
    for (k, v) in dict_of(doc, resources.get(b"XObject")).iter() {
        let Some(rf) = v.as_ref() else { continue };
        let obj = doc.get(rf);
        let Object::Stream(s) = &*obj else { continue };
        match s.dict.name(b"Subtype") {
            Some(b"Form") => {
                r.forms.insert(k.clone(), rf);
            }
            Some(b"Image") => {
                r.images.insert(k.clone());
            }
            _ => {}
        }
    }
    r
}

/// Lo stato grafico che conta qui.
#[derive(Clone)]
struct State {
    ctm: Matrix,
    stroke_space: Space,
    fill_space: Space,
    stroke: Ink,
    fill: Ink,
    width: f64,
    op_stroke: bool,
    op_fill: bool,
    /// Il ritaglio in vigore, come rettangolo in millimetri.
    clip: Option<[f64; 4]>,
}

fn space_operand(doc: &Document, op: &Op, res: &Res) -> Space {
    match op.name(0) {
        Some(n) => match res.spaces.get(n) {
            Some(s) => s.clone(),
            None => space_of(doc, &Object::Name(n.to_vec()), 0),
        },
        None => Space::Other("Spazio colore sconosciuto".into()),
    }
}

fn values(op: &Op) -> Vec<f64> {
    op.operands.iter().filter_map(Object::as_f64).collect()
}

fn union(a: Option<[f64; 4]>, b: [f64; 4]) -> [f64; 4] {
    match a {
        Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
        None => b,
    }
}

fn intersect(a: [f64; 4], b: [f64; 4]) -> Option<[f64; 4]> {
    let r = [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])];
    (r[0] <= r[2] && r[1] <= r[3]).then_some(r)
}

fn bounds_of(parts: &[SubPath]) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    for p in parts.iter().flat_map(|p| &p.points) {
        b = Some(union(b, [p[0], p[1], p[0], p[1]]));
    }
    b
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Una curva di Bézier spezzata in segmenti (senza il primo punto).
fn flatten(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], p3: [f64; 2]) -> Vec<[f64; 2]> {
    let rough = dist(p0, p1) + dist(p1, p2) + dist(p2, p3);
    let n = if rough.is_finite() {
        (rough / 0.5).ceil().clamp(4.0, 64.0) as usize
    } else {
        4
    };
    (1..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            [
                a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0],
                a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1],
            ]
        })
        .collect()
}

/// Lettura delle pagine: tracciati, altri oggetti, limiti.
struct Scanner<'a> {
    doc: &'a Document,
    out: PageScan,
    points: usize,
    ops: usize,
}

impl Scanner<'_> {
    fn content(&mut self, owner: Owner, ops: &[Op], res: &Res, start: State, depth: usize) {
        let mut s = start;
        let mut stack: Vec<State> = Vec::new();
        let mut parts: Vec<SubPath> = Vec::new();
        let mut clip_pending = false;
        let mut text = false;
        let mut cur = [0.0, 0.0];
        for (i, op) in ops.iter().enumerate() {
            self.ops += 1;
            if self.ops > MAX_OPS {
                self.out.truncated = true;
                return;
            }
            let at = |x: f64, y: f64, s: &State| {
                let (x, y) = s.ctm.apply(x, y);
                [x, y]
            };
            match op.op.as_slice() {
                b"q" => stack.push(s.clone()),
                b"Q" => {
                    if let Some(prev) = stack.pop() {
                        s = prev;
                    }
                }
                b"cm" => {
                    if let Some(m) = Matrix::from_operands(&op.operands) {
                        s.ctm = m.then(&s.ctm);
                    }
                }
                b"w" => s.width = op.num(0).unwrap_or(s.width),
                b"CS" => {
                    s.stroke_space = space_operand(self.doc, op, res);
                    s.stroke = s.stroke_space.initial();
                }
                b"cs" => {
                    s.fill_space = space_operand(self.doc, op, res);
                    s.fill = s.fill_space.initial();
                }
                b"SC" | b"SCN" => s.stroke = s.stroke_space.ink(&values(op)),
                b"sc" | b"scn" => s.fill = s.fill_space.ink(&values(op)),
                b"G" => (s.stroke_space, s.stroke) = (Space::Gray, Space::Gray.ink(&values(op))),
                b"g" => (s.fill_space, s.fill) = (Space::Gray, Space::Gray.ink(&values(op))),
                b"RG" => (s.stroke_space, s.stroke) = (Space::Rgb, Space::Rgb.ink(&values(op))),
                b"rg" => (s.fill_space, s.fill) = (Space::Rgb, Space::Rgb.ink(&values(op))),
                b"K" => (s.stroke_space, s.stroke) = (Space::Cmyk, Space::Cmyk.ink(&values(op))),
                b"k" => (s.fill_space, s.fill) = (Space::Cmyk, Space::Cmyk.ink(&values(op))),
                b"gs" => {
                    if let Some(GsEntry {
                        stroke,
                        fill,
                        width: lw,
                    }) = op.name(0).and_then(|n| res.states.get(n)).copied()
                    {
                        if let Some(v) = stroke {
                            s.op_stroke = v;
                            // Senza "op" il riempimento segue "OP".
                            if fill.is_none() {
                                s.op_fill = v;
                            }
                        }
                        if let Some(v) = fill {
                            s.op_fill = v;
                        }
                        if let Some(w) = lw {
                            s.width = w;
                        }
                    }
                }
                b"BT" => text = true,
                b"ET" => text = false,
                b"m" if !text => {
                    if let Some([x, y]) = op.nums::<2>() {
                        cur = at(x, y, &s);
                        parts.push(SubPath {
                            points: vec![cur],
                            closed: false,
                        });
                    }
                }
                b"l" if !text => {
                    if let (Some([x, y]), Some(p)) = (op.nums::<2>(), parts.last_mut()) {
                        cur = at(x, y, &s);
                        p.points.push(cur);
                    }
                }
                b"c" | b"v" | b"y" if !text => {
                    let pts: Option<([f64; 2], [f64; 2], [f64; 2])> = match op.op.as_slice() {
                        b"c" => op
                            .nums::<6>()
                            .map(|v| (at(v[0], v[1], &s), at(v[2], v[3], &s), at(v[4], v[5], &s))),
                        b"v" => op
                            .nums::<4>()
                            .map(|v| (cur, at(v[0], v[1], &s), at(v[2], v[3], &s))),
                        _ => op.nums::<4>().map(|v| {
                            let end = at(v[2], v[3], &s);
                            (at(v[0], v[1], &s), end, end)
                        }),
                    };
                    if let (Some((a, b, c)), Some(p)) = (pts, parts.last_mut()) {
                        p.points.extend(flatten(cur, a, b, c));
                        cur = c;
                    }
                }
                b"h" if !text => {
                    if let Some(p) = parts.last_mut() {
                        p.closed = true;
                        if let Some(first) = p.points.first() {
                            cur = *first;
                        }
                    }
                }
                b"re" if !text => {
                    if let Some([x, y, w, h]) = op.nums::<4>() {
                        let pts = vec![
                            at(x, y, &s),
                            at(x + w, y, &s),
                            at(x + w, y + h, &s),
                            at(x, y + h, &s),
                        ];
                        cur = pts[0];
                        parts.push(SubPath {
                            points: pts,
                            closed: true,
                        });
                    }
                }
                b"W" | b"W*" => clip_pending = true,
                b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n" => {
                    let kw = op.op.as_slice();
                    if matches!(kw, b"s" | b"b" | b"b*")
                        && let Some(p) = parts.last_mut()
                    {
                        p.closed = true;
                    }
                    let strokes = matches!(kw, b"S" | b"s" | b"B" | b"B*" | b"b" | b"b*");
                    let fills = matches!(kw, b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*");
                    let taken = std::mem::take(&mut parts);
                    let bounds = bounds_of(&taken);
                    if (strokes || fills)
                        && let Some(bounds) = bounds
                    {
                        self.path(
                            owner,
                            i,
                            taken,
                            bounds,
                            &s,
                            strokes,
                            fills,
                            clip_pending,
                            depth > 0,
                        );
                    }
                    if clip_pending {
                        if let Some(b) = bounds {
                            s.clip = Some(match s.clip {
                                Some(c) => intersect(c, b).unwrap_or([b[0], b[1], b[0], b[1]]),
                                None => b,
                            });
                        }
                        clip_pending = false;
                    }
                }
                b"Do" => {
                    let Some(n) = op.name(0) else { continue };
                    if res.images.contains(n) {
                        self.other(s.ctm.bbox([0.0, 0.0, 1.0, 1.0]), &s);
                    } else if let Some(&form) = res.forms.get(n) {
                        if depth >= MAX_DEPTH {
                            continue;
                        }
                        let obj = self.doc.get(form);
                        let Object::Stream(st) = &*obj else { continue };
                        let Ok(data) = st.decoded_within(MAX_CONTENT) else {
                            continue;
                        };
                        let m = st
                            .dict
                            .get(b"Matrix")
                            .and_then(|o| self.doc.resolve(o).as_array().cloned())
                            .and_then(|v| Matrix::from_operands(&v))
                            .unwrap_or(Matrix::IDENTITY);
                        let fres = read_res(self.doc, &dict_of(self.doc, st.dict.get(b"Resources")));
                        let mut inner = s.clone();
                        inner.ctm = m.then(&s.ctm);
                        let ops = parse(&data).ops;
                        self.content(Owner::Form(form), &ops, &fres, inner, depth + 1);
                    }
                }
                b"BI" => self.other(s.ctm.bbox([0.0, 0.0, 1.0, 1.0]), &s),
                b"sh" => {
                    let all = [0.0, 0.0, self.out.width, self.out.height];
                    self.other(s.clip.unwrap_or(all), &s);
                }
                _ => {}
            }
        }
    }

    fn other(&mut self, b: [f64; 4], s: &State) {
        let b = match s.clip {
            Some(c) => intersect(b, c),
            None => Some(b),
        };
        if let Some(b) = b {
            self.out.others.push(b);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn path(
        &mut self,
        owner: Owner,
        op: usize,
        parts: Vec<SubPath>,
        bounds: [f64; 4],
        s: &State,
        strokes: bool,
        fills: bool,
        clips: bool,
        in_form: bool,
    ) {
        if self.out.paths.len() >= MAX_PATHS || self.points > MAX_POINTS {
            self.out.truncated = true;
            return;
        }
        // Un tracciato tutto fuori dal ritaglio non si vede: non si può cliccare.
        if let Some(c) = s.clip
            && intersect(c, bounds).is_none()
        {
            return;
        }
        let [a, b, c, d, _, _] = s.ctm.0;
        let scale = (a * d - b * c).abs().sqrt();
        let length = parts
            .iter()
            .map(|p| {
                let open: f64 = p.points.windows(2).map(|w| dist(w[0], w[1])).sum();
                let close = match (p.closed, p.points.first(), p.points.last()) {
                    (true, Some(f), Some(l)) => dist(*f, *l),
                    _ => 0.0,
                };
                open + close
            })
            .sum();
        self.points += parts.iter().map(|p| p.points.len()).sum::<usize>();
        self.out.paths.push(PathInfo {
            id: PathId { owner, op },
            parts,
            line: strokes.then(|| Line {
                ink: s.stroke.clone(),
                width: (s.width * scale).max(0.0),
                overprint: s.op_stroke,
            }),
            fill: fills.then(|| Fill {
                ink: s.fill.clone(),
                overprint: s.op_fill,
            }),
            bounds,
            length,
            in_form,
            clips,
            scale,
        });
    }
}

const fn start_state(ctm: Matrix) -> State {
    State {
        ctm,
        stroke_space: Space::Gray,
        fill_space: Space::Gray,
        stroke: Ink::Gray(0.0),
        fill: Ink::Gray(0.0),
        width: 1.0,
        op_stroke: false,
        op_fill: false,
        clip: None,
    }
}

/// La pagina come si vede: la matrice dal PDF ai millimetri dall'angolo in alto a sinistra,
/// larghezza e altezza in millimetri, il formato finito se diverso dalla pagina.
struct Frame {
    matrix: Matrix,
    width: f64,
    height: f64,
    trim: Option<[f64; 4]>,
}

fn page_matrix(doc: &Document, page: usize) -> Result<Frame, EditError> {
    let boxes = organize::page_boxes(doc)?;
    let b = boxes.get(page).ok_or(EditError::NoPage(page.saturating_add(1)))?;
    let rotation = organize::page_rotation(doc, page).unwrap_or(0);
    let crop = b[1];
    let up = Matrix(crate::frame::upright(rotation, crop));
    let (w, h) = (crop[2] - crop[0], crop[3] - crop[1]);
    let (w, h) = if rotation == 90 || rotation == 270 {
        (h, w)
    } else {
        (w, h)
    };
    let mm = pt_to_mm(1.0);
    let flip = Matrix([mm, 0.0, 0.0, -mm, 0.0, h * mm]);
    let m = up.then(&flip);
    let trim = b[3];
    let differs = trim.iter().zip(crop).any(|(t, c)| (t - c).abs() > 0.01);
    let trim = differs.then(|| m.bbox(trim));
    Ok(Frame {
        matrix: m,
        width: pt_to_mm(w),
        height: pt_to_mm(h),
        trim,
    })
}

/// I tracciati e gli altri oggetti della pagina `page` (da 0).
pub(crate) fn scan(doc: &Document, page: usize) -> Result<PageScan, EditError> {
    let pages = organize::pages(doc)?;
    let p = pages.get(page).ok_or(EditError::NoPage(page.saturating_add(1)))?;
    let Frame {
        matrix: m,
        width,
        height,
        trim,
    } = page_matrix(doc, page)?;
    let res = read_res(doc, &page_resources(doc, p.obj));
    let ops = parse(&page_content(doc, p.obj)?).ops;
    let mut sc = Scanner {
        doc,
        out: PageScan {
            width,
            height,
            trim,
            ..PageScan::default()
        },
        points: 0,
        ops: 0,
    };
    sc.content(Owner::Page(p.obj), &ops, &res, start_state(m), 0);
    Ok(sc.out)
}

/// Che parte del tracciato cambiare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Line,
    Fill,
}

/// Il colore nuovo.
#[derive(Clone, Debug, PartialEq)]
pub enum NewInk {
    /// Una tinta piatta al 100%, con il colore che mostra a schermo e nelle prove (CMYK).
    Spot {
        name: String,
        cmyk: [f64; 4],
    },
    Cmyk([f64; 4]),
}

/// Cosa cambiare dei tracciati scelti.
#[derive(Clone, Debug, PartialEq)]
pub struct Restyle {
    pub part: Part,
    pub ink: Option<NewInk>,
    /// Spessore del contorno in millimetri.
    pub width: Option<f64>,
    pub overprint: Option<bool>,
}

/// Dove sta il tracciato nel contenuto: da quale operatore comincia, se ritaglia, la sovrastampa
/// in vigore (riempimento, contorno).
struct Spot {
    start: usize,
    clips: bool,
    over: (bool, bool),
}

/// Per ogni operatore che dipinge: dove comincia il suo tracciato.
fn locate(ops: &[Op], res: &Res) -> HashMap<usize, Spot> {
    let mut out = HashMap::new();
    let mut start: Option<usize> = None;
    let mut clips = false;
    let mut over = (false, false);
    let mut stack = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        match op.op.as_slice() {
            b"q" => stack.push(over),
            b"Q" => over = stack.pop().unwrap_or(over),
            b"gs" => {
                if let Some(GsEntry { stroke, fill, .. }) =
                    op.name(0).and_then(|n| res.states.get(n)).copied()
                {
                    if let Some(v) = stroke {
                        over.1 = v;
                        if fill.is_none() {
                            over.0 = v;
                        }
                    }
                    if let Some(v) = fill {
                        over.0 = v;
                    }
                }
            }
            b"m" | b"re" => {
                start.get_or_insert(i);
            }
            b"W" | b"W*" => clips = true,
            b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n" => {
                if let Some(st) = start.take() {
                    out.insert(
                        i,
                        Spot {
                            start: st,
                            clips,
                            over,
                        },
                    );
                }
                clips = false;
            }
            _ => {}
        }
    }
    out
}

/// Un nome libero nelle risorse.
fn free_key(d: &Dict, prefix: &str) -> Vec<u8> {
    (1..)
        .map(|i| format!("{prefix}{i}").into_bytes())
        .find(|k| d.get(k).is_none())
        .unwrap_or_else(|| prefix.as_bytes().to_vec())
}

/// L'esito di un cambio.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Restyled {
    /// I tracciati cambiati.
    pub done: usize,
    /// Quelli rimasti com'erano perché ritagliano anche il resto del disegno.
    pub skipped: usize,
    /// I tracciati cambiati come si chiamano adesso (le posizioni nel contenuto si spostano).
    pub ids: Vec<PathId>,
}

/// Una tinta piatta nuova: Separation con il nome dato e il colore CMYK per lo schermo.
fn spot_space(doc: &mut Document, name: &str, cmyk: [f64; 4]) -> ObjRef {
    let mut f = Dict::new();
    f.set(b"FunctionType".to_vec(), Object::Int(2));
    f.set(
        b"Domain".to_vec(),
        Object::Array(vec![Object::Int(0), Object::Int(1)]),
    );
    f.set(b"C0".to_vec(), Object::Array(vec![Object::Int(0); 4]));
    f.set(
        b"C1".to_vec(),
        Object::Array(
            cmyk.iter()
                .map(|v| pdfcraft_content::num(v.clamp(0.0, 1.0)))
                .collect(),
        ),
    );
    f.set(b"N".to_vec(), Object::Int(1));
    doc.add(Object::Array(vec![
        Object::name("Separation"),
        Object::Name(name.trim().as_bytes().to_vec()),
        Object::name("DeviceCMYK"),
        Object::Dict(f),
    ]))
}

/// Cambia i tracciati `ids` della pagina.
pub(crate) fn restyle(
    doc: &mut Document,
    page: usize,
    ids: &[PathId],
    change: &Restyle,
) -> Result<Restyled, EditError> {
    if let Some(w) = change.width
        && !(w.is_finite() && (0.0..=100.0).contains(&w))
    {
        return Err(EditError::Invalid("spessore non valido".into()));
    }
    if let Some(NewInk::Spot { name, .. }) = &change.ink {
        let n = name.trim();
        if n.is_empty() || n.len() > 127 {
            return Err(EditError::Invalid("nome della tinta non valido".into()));
        }
    }
    let wanted: HashSet<PathId> = ids.iter().copied().collect();
    let scanned = scan(doc, page)?;
    // Per contenuto: operatore → scala (millimetri per unità).
    let mut by_owner: HashMap<Owner, HashMap<usize, f64>> = HashMap::new();
    for p in scanned.paths.iter().filter(|p| wanted.contains(&p.id)) {
        let has_part = match change.part {
            Part::Line => p.line.is_some(),
            Part::Fill => p.fill.is_some(),
        };
        if has_part {
            by_owner.entry(p.id.owner).or_default().insert(p.id.op, p.scale);
        }
    }
    // La tinta nuova: uno spazio colore per tutto il documento.
    let spot_cs = match &change.ink {
        Some(NewInk::Spot { name, cmyk }) => Some(spot_space(doc, name, *cmyk)),
        _ => None,
    };
    let mut states: HashMap<(bool, bool), ObjRef> = HashMap::new();
    let mut result = Restyled::default();
    let mut owners: Vec<_> = by_owner.into_iter().collect();
    owners.sort_by_key(|(o, _)| *o);
    for (owner, targets) in owners {
        let (data, mut resources) = match owner {
            Owner::Page(r) => (page_content(doc, r)?, page_resources(doc, r)),
            Owner::Form(r) => {
                let obj = doc.get(r);
                let Object::Stream(s) = &*obj else { continue };
                let data = s
                    .decoded_within(MAX_CONTENT)
                    .map_err(|e| EditError::Pdf(e.to_string()))?;
                (data, dict_of(doc, s.dict.get(b"Resources")))
            }
        };
        let res = read_res(doc, &resources);
        let ops = parse(&data).ops;
        let spots = locate(&ops, &res);
        let cs_key = free_key(&dict_of(doc, resources.get(b"ColorSpace")), CS_PREFIX);
        let mut before: HashMap<usize, Vec<Op>> = HashMap::new();
        let mut after: HashSet<usize> = HashSet::new();
        let mut used_states: HashSet<(bool, bool)> = HashSet::new();
        for (&i, &scale) in &targets {
            let Some(sp) = spots.get(&i) else { continue };
            if sp.clips {
                result.skipped += 1;
                continue;
            }
            let mut set = vec![Op::new("q", Vec::new())];
            let line = change.part == Part::Line;
            match &change.ink {
                Some(NewInk::Spot { .. }) => {
                    let (space, color) = if line { ("CS", "SCN") } else { ("cs", "scn") };
                    set.push(Op::new(space, vec![Object::Name(cs_key.clone())]));
                    set.push(Op::new(color, vec![Object::Int(1)]));
                }
                Some(NewInk::Cmyk(v)) => {
                    let ops: Vec<Object> = v
                        .iter()
                        .map(|x| pdfcraft_content::num(x.clamp(0.0, 1.0)))
                        .collect();
                    set.push(Op::new(if line { "K" } else { "k" }, ops));
                }
                None => {}
            }
            if line
                && let Some(w) = change.width
                && scale > 0.0
            {
                set.push(Op::new("w", vec![pdfcraft_content::num(w / scale)]));
            }
            if let Some(on) = change.overprint {
                let (f, k) = if line { (sp.over.0, on) } else { (on, sp.over.1) };
                set.push(Op::new("gs", vec![Object::Name(gs_name(f, k))]));
                used_states.insert((f, k));
            }
            before.insert(sp.start, set);
            after.insert(i);
            result.done += 1;
        }
        if before.is_empty() {
            continue;
        }
        let mut out: Vec<Op> = Vec::with_capacity(ops.len() + 4 * before.len());
        for (i, op) in ops.into_iter().enumerate() {
            if let Some(set) = before.remove(&i) {
                out.extend(set);
            }
            if after.contains(&i) {
                result.ids.push(PathId { owner, op: out.len() });
                out.push(op);
                out.push(Op::new("Q", Vec::new()));
            } else {
                out.push(op);
            }
        }
        if let Some(cs) = spot_cs {
            let mut d = dict_of(doc, resources.get(b"ColorSpace"));
            d.set(cs_key.clone(), Object::Ref(cs));
            resources.set(b"ColorSpace".to_vec(), Object::Dict(d));
        }
        let mut extra = Vec::new();
        for k in used_states {
            let r = *states.entry(k).or_insert_with(|| {
                let mut g = Dict::new();
                g.set(b"Type".to_vec(), Object::name("ExtGState"));
                g.set(b"OP".to_vec(), Object::Bool(k.1));
                g.set(b"op".to_vec(), Object::Bool(k.0));
                doc.add(Object::Dict(g))
            });
            extra.push((gs_name(k.0, k.1), r));
        }
        add_states(doc, &mut resources, &extra);
        let data = serialize_ops(&out);
        match owner {
            Owner::Page(r) => {
                let contents = doc.add(Object::Stream(Stream::flate(Dict::new(), &data)));
                doc.update_dict(r, |d| {
                    d.set(b"Contents".to_vec(), Object::Ref(contents));
                    d.set(b"Resources".to_vec(), Object::Dict(resources));
                })?;
            }
            Owner::Form(r) => {
                let obj = doc.get(r);
                let Object::Stream(s) = &*obj else { continue };
                let mut dict = s.dict.clone();
                dict.set(b"Resources".to_vec(), Object::Dict(resources));
                dict.remove(b"Length");
                dict.remove(b"Filter");
                dict.remove(b"DecodeParms");
                doc.set(r, Object::Stream(Stream::flate(dict, &data)));
            }
        }
    }
    Ok(result)
}

/// Il contenuto della pagina con i tracciati `ids` che non si dipingono più (l'operatore che
/// dipinge diventa `n`), per esempio la grafica senza fustella. Si toccano solo i tracciati del
/// contenuto della pagina: quelli dentro i moduli restano, e si contano nel secondo valore.
pub(crate) fn without(doc: &Document, page: usize, ids: &[PathId]) -> Result<(Vec<u8>, usize), EditError> {
    let page_ref = organize::pages(doc)?
        .get(page)
        .map(|p| p.obj)
        .ok_or(EditError::NoPage(page.saturating_add(1)))?;
    let data = page_content(doc, page_ref)?;
    let targets: HashSet<usize> = ids
        .iter()
        .filter(|id| id.owner == Owner::Page(page_ref))
        .map(|id| id.op)
        .collect();
    let left = ids.len().saturating_sub(targets.len());
    if targets.is_empty() {
        return Ok((data, left));
    }
    let resources = page_resources(doc, page_ref);
    let res = read_res(doc, &resources);
    let ops = parse(&data).ops;
    let spots = locate(&ops, &res);
    let out: Vec<Op> = ops
        .into_iter()
        .enumerate()
        .map(|(i, op)| match spots.get(&i) {
            // Un tracciato che ritaglia deve continuare a ritagliare: resta com'è.
            Some(sp) if targets.contains(&i) && !sp.clips => Op::new("n", Vec::new()),
            _ => op,
        })
        .collect();
    Ok((serialize_ops(&out), left))
}

/// Il contenuto della pagina con soli i tracciati `ids` (per esempio la fustella da sola): gli
/// altri tracciati non si dipingono e testi, immagini, sfumature e moduli si tolgono.
pub(crate) fn only(doc: &Document, page: usize, ids: &[PathId]) -> Result<Vec<u8>, EditError> {
    let page_ref = organize::pages(doc)?
        .get(page)
        .map(|p| p.obj)
        .ok_or(EditError::NoPage(page.saturating_add(1)))?;
    let data = page_content(doc, page_ref)?;
    let targets: HashSet<usize> = ids
        .iter()
        .filter(|id| id.owner == Owner::Page(page_ref))
        .map(|id| id.op)
        .collect();
    let resources = page_resources(doc, page_ref);
    let res = read_res(doc, &resources);
    let ops = parse(&data).ops;
    let spots = locate(&ops, &res);
    let out: Vec<Op> = ops
        .into_iter()
        .enumerate()
        .filter_map(|(i, op)| match spots.get(&i) {
            Some(sp) if !targets.contains(&i) && !sp.clips => Some(Op::new("n", Vec::new())),
            Some(_) => Some(op),
            None => match op.op.as_slice() {
                b"Tj" | b"TJ" | b"'" | b"\"" | b"Do" | b"sh" | b"BI" | b"ID" | b"EI" => None,
                _ => Some(op),
            },
        })
        .collect();
    Ok(serialize_ops(&out))
}

/// La fustella nuova da creare attorno ai tracciati scelti.
#[derive(Clone, Debug, PartialEq)]
pub struct NewDie {
    /// Il nome della tinta piatta (per esempio "Taglio").
    pub name: String,
    /// Il colore con cui la tinta si vede a schermo e nelle prove.
    pub cmyk: [f64; 4],
    /// Spessore della linea in millimetri.
    pub width: f64,
    pub overprint: bool,
}

/// Disegna sopra la pagina una fustella nuova: il contorno dell'unione dei tracciati `ids`
/// (le forme che si sovrappongono diventano una sola), in tinta piatta. I tracciati restano
/// com'erano. Restituisce quanti contorni ha la fustella.
pub(crate) fn add_die(
    doc: &mut Document,
    page: usize,
    ids: &[PathId],
    die: &NewDie,
) -> Result<usize, EditError> {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::float::simplify::SimplifyShape;

    let name = die.name.trim();
    if name.is_empty() || name.len() > 127 {
        return Err(EditError::Invalid("nome della tinta non valido".into()));
    }
    if !(die.width.is_finite() && (0.0..=100.0).contains(&die.width)) {
        return Err(EditError::Invalid("spessore non valido".into()));
    }
    let wanted: HashSet<PathId> = ids.iter().copied().collect();
    let scanned = scan(doc, page)?;
    // Ogni tracciato si pulisce da solo (pari-dispari: i buchi restano buchi), poi tutti insieme si
    // uniscono (non-zero: quello che si sovrappone diventa una forma sola).
    let mut contours: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut points = 0usize;
    for p in scanned.paths.iter().filter(|p| wanted.contains(&p.id)) {
        let own: Vec<Vec<[f64; 2]>> = p
            .parts
            .iter()
            .filter(|part| part.points.len() >= 3)
            .map(|part| part.points.clone())
            .collect();
        points = points.saturating_add(own.iter().map(Vec::len).sum());
        if points > MAX_POINTS {
            return Err(EditError::Invalid(
                "i tracciati scelti sono troppo complessi".into(),
            ));
        }
        for shape in own.simplify_shape(FillRule::EvenOdd) {
            contours.extend(shape);
        }
    }
    let shapes = contours.simplify_shape(FillRule::NonZero);
    let outlines: Vec<&Vec<[f64; 2]>> = shapes.iter().flatten().filter(|c| c.len() >= 3).collect();
    if outlines.is_empty() {
        return Err(EditError::Invalid(
            "i tracciati scelti non chiudono un'area: niente fustella".into(),
        ));
    }
    // Dai millimetri della pagina come si vede alle unità del PDF.
    let back = page_matrix(doc, page)?
        .matrix
        .invert()
        .ok_or_else(|| EditError::Pdf("la pagina non ha una misura".into()))?;
    let pages = organize::pages(doc)?;
    let page_ref = pages
        .get(page)
        .map(|p| p.obj)
        .ok_or(EditError::NoPage(page.saturating_add(1)))?;
    let mut resources = page_resources(doc, page_ref);
    let cs_key = free_key(&dict_of(doc, resources.get(b"ColorSpace")), CS_PREFIX);
    let cs = spot_space(doc, name, die.cmyk);
    let mut d = dict_of(doc, resources.get(b"ColorSpace"));
    d.set(cs_key.clone(), Object::Ref(cs));
    resources.set(b"ColorSpace".to_vec(), Object::Dict(d));
    let mut state = Dict::new();
    state.set(b"Type".to_vec(), Object::name("ExtGState"));
    state.set(b"OP".to_vec(), Object::Bool(die.overprint));
    state.set(b"op".to_vec(), Object::Bool(false));
    let state = doc.add(Object::Dict(state));
    let gs = gs_name(false, die.overprint);
    add_states(doc, &mut resources, &[(gs.clone(), state)]);

    let n = |v: f64| pdfcraft_content::num(v);
    let mut ops = vec![
        Op::new("q", Vec::new()),
        Op::new("CS", vec![Object::Name(cs_key)]),
        Op::new("SCN", vec![Object::Int(1)]),
        Op::new("gs", vec![Object::Name(gs)]),
        Op::new("w", vec![n(winfiles_geom::mm_to_pt(die.width))]),
        Op::new("j", vec![Object::Int(0)]),
        Op::new("J", vec![Object::Int(0)]),
        Op::new("d", vec![Object::Array(Vec::new()), Object::Int(0)]),
    ];
    for c in &outlines {
        for (i, p) in c.iter().enumerate() {
            let (x, y) = back.apply(p[0], p[1]);
            ops.push(Op::new(if i == 0 { "m" } else { "l" }, vec![n(x), n(y)]));
        }
        ops.push(Op::new("h", Vec::new()));
    }
    ops.push(Op::new("S", Vec::new()));
    ops.push(Op::new("Q", Vec::new()));
    // Il contenuto di prima chiuso tra q e Q: quello che lascia in sospeso non tocca la fustella.
    let mut data = b"q\n".to_vec();
    data.extend(page_content(doc, page_ref)?);
    data.extend(b"\nQ\n");
    data.extend(serialize_ops(&ops));
    let contents = doc.add(Object::Stream(Stream::flate(Dict::new(), &data)));
    doc.update_dict(page_ref, |d| {
        d.set(b"Contents".to_vec(), Object::Ref(contents));
        d.set(b"Resources".to_vec(), Object::Dict(resources));
    })?;
    Ok(outlines.len())
}
