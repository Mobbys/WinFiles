//! Le tinte piatte (colori Separation e DeviceN, i Pantone): quali ci sono, su quali pagine, se
//! sono in sovrastampa; rinominarle e mettere o togliere la sovrastampa.
//!
//! Rinominare cambia il nome nello spazio colore, dovunque sia definito: due tinte con lo stesso
//! nome diventano una sola lastra in stampa. La sovrastampa si decide nel contenuto: prima di ogni
//! oggetto dipinto con la tinta si mette uno stato grafico (ExtGState) con la sovrastampa accesa,
//! e prima del primo oggetto di un altro colore quello che il documento aveva. Il resto del
//! contenuto resta com'è, operatore per operatore.
//!
//! Un modulo (form XObject) eredita lo stato grafico da chi lo disegna: dentro si parte supponendo
//! la sovrastampa spenta, che è il caso di quasi tutti i file.

use std::collections::{HashMap, HashSet};

use pdfcraft_content::{Op, parse, serialize_ops};
use pdfcraft_cos::{Dict, Document, ObjRef, Object, Stream};
use pdfcraft_organize as organize;

use crate::EditError;

/// Profondità massima di moduli dentro moduli, spazi colore dentro spazi colore.
const MAX_DEPTH: usize = 24;
/// Tinte piatte che si seguono insieme (una per bit).
const MAX_SPOTS: usize = 64;
/// Nomi degli stati grafici aggiunti: sovrastampa del riempimento e del contorno.
const GS_PREFIX: &str = "WFSovra";
/// Contenuto più grande che si riscrive.
const MAX_CONTENT: usize = 512 * 1024 * 1024;

/// I nomi che non sono tinte: la registrazione (tutte le lastre), nessuna, la quadricromia.
const NOT_SPOTS: [&[u8]; 6] = [b"All", b"None", b"Cyan", b"Magenta", b"Yellow", b"Black"];

/// La sovrastampa degli oggetti dipinti con una tinta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overprint {
    /// Tutti in sovrastampa.
    All,
    /// Alcuni sì e altri no.
    Some,
    /// Nessuno.
    None,
    /// La tinta è definita ma nessun oggetto la usa.
    Unused,
}

/// Una tinta piatta del documento.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spot {
    pub name: String,
    /// Il nome com'è nel file.
    pub raw: Vec<u8>,
    /// Le pagine (da 0) dove si vede.
    pub pages: Vec<usize>,
    pub overprint: Overprint,
    /// Il colore che il file dà alla tinta al 100% per lo schermo, se si sa leggere.
    pub swatch: Option<[u8; 3]>,
}

/// Le tinte usate da uno spazio colore.
fn colorants(doc: &Document, cs: &Object, depth: usize) -> Vec<Vec<u8>> {
    if depth > MAX_DEPTH {
        return Vec::new();
    }
    let cs = doc.resolve(cs);
    let Object::Array(a) = &*cs else {
        return Vec::new();
    };
    let family = a.first().and_then(Object::as_name).unwrap_or(b"");
    let keep = |n: &[u8]| !NOT_SPOTS.contains(&n);
    match family {
        b"Separation" => a
            .get(1)
            .map(|n| doc.resolve(n))
            .and_then(|n| n.as_name().map(<[u8]>::to_vec))
            .filter(|n| keep(n))
            .into_iter()
            .collect(),
        b"DeviceN" => match a.get(1).map(|n| doc.resolve(n)).as_deref() {
            Some(Object::Array(names)) => names
                .iter()
                .filter_map(Object::as_name)
                .filter(|n| keep(n))
                .map(<[u8]>::to_vec)
                .collect(),
            _ => Vec::new(),
        },
        b"Indexed" | b"Pattern" => a
            .get(1)
            .map(|base| colorants(doc, base, depth + 1))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Il colore di una tinta Separation al 100%, dalla sua funzione verso lo spazio alternativo
/// (solo la funzione esponenziale, quella di quasi tutti i Pantone; CMYK convertito alla buona).
fn swatch(doc: &Document, cs: &Object) -> Option<(Vec<u8>, [u8; 3])> {
    let cs = doc.resolve(cs);
    let a = cs.as_array()?;
    if a.first().and_then(Object::as_name) != Some(b"Separation".as_slice()) {
        return None;
    }
    let name = doc.resolve(a.get(1)?).as_name()?.to_vec();
    let alt = doc.resolve(a.get(2)?);
    let family = match &*alt {
        Object::Name(n) => n.clone(),
        Object::Array(v) => v.first().and_then(Object::as_name)?.to_vec(),
        _ => return None,
    };
    let f = doc.resolve(a.get(3)?);
    let f = match &*f {
        Object::Dict(d) => d.clone(),
        Object::Stream(s) => s.dict.clone(),
        _ => return None,
    };
    if f.int(b"FunctionType") != Some(2) {
        return None;
    }
    let c1: Vec<f64> = match f.get(b"C1").map(|o| doc.resolve(o)).as_deref() {
        Some(Object::Array(v)) => v.iter().filter_map(Object::as_f64).collect(),
        _ => vec![1.0],
    };
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let rgb = match (family.as_slice(), c1.as_slice()) {
        (b"DeviceCMYK", [c, m, y, k]) => {
            let ink = |v: f64| (1.0 - v) * (1.0 - k);
            [byte(ink(*c)), byte(ink(*m)), byte(ink(*y))]
        }
        (b"DeviceRGB" | b"CalRGB", [r, g, b]) => [byte(*r), byte(*g), byte(*b)],
        (b"DeviceGray" | b"CalGray", [g]) => [byte(*g); 3],
        (b"Lab", [l, a, b]) => lab_to_srgb(*l, *a, *b),
        _ => return None,
    };
    Some((name, rgb))
}

/// Da Lab (D50) a sRGB, per i Pantone definiti in Lab.
fn lab_to_srgb(l: f64, a: f64, b: f64) -> [u8; 3] {
    let fy = (l + 16.0) / 116.0;
    let (fx, fz) = (fy + a / 500.0, fy - b / 200.0);
    let inv = |t: f64| {
        if t > 6.0 / 29.0 {
            t * t * t
        } else {
            3.0 * (6.0f64 / 29.0).powi(2) * (t - 4.0 / 29.0)
        }
    };
    let (x, y, z) = (0.9642 * inv(fx), inv(fy), 0.8249 * inv(fz));
    // D50 → sRGB lineare (Bradford compreso).
    let r = 3.1339 * x - 1.6169 * y - 0.4906 * z;
    let g = -0.9788 * x + 1.9161 * y + 0.0335 * z;
    let bl = 0.0719 * x - 0.2290 * y + 1.4052 * z;
    let gamma = |v: f64| {
        let v = v.clamp(0.0, 1.0);
        let s = if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (s * 255.0).round() as u8
    };
    [gamma(r), gamma(g), gamma(bl)]
}

/// Le tinte che vale la pena seguire: un indice per nome.
#[derive(Default)]
struct Catalog {
    names: Vec<Vec<u8>>,
    index: HashMap<Vec<u8>, usize>,
    swatches: HashMap<Vec<u8>, [u8; 3]>,
}

impl Catalog {
    fn mask(&mut self, names: Vec<Vec<u8>>) -> u64 {
        let mut m = 0;
        for n in names {
            let i = match self.index.get(&n) {
                Some(i) => *i,
                None if self.names.len() < MAX_SPOTS => {
                    self.names.push(n.clone());
                    self.index.insert(n, self.names.len() - 1);
                    self.names.len() - 1
                }
                None => continue,
            };
            m |= 1 << i;
        }
        m
    }
}

/// Le risorse di un contenuto, già lette: cosa vuol dire ogni nome che il contenuto usa.
#[derive(Default)]
struct Res {
    /// Spazi colore per nome, con le tinte che usano.
    spaces: HashMap<Vec<u8>, u64>,
    /// Stati grafici per nome: sovrastampa del contorno (OP) e del riempimento (op), se li danno.
    states: HashMap<Vec<u8>, (Option<bool>, Option<bool>)>,
    /// Immagini per nome: tinte dello spazio colore, ed è una maschera (dipinge col riempimento).
    images: HashMap<Vec<u8>, (u64, bool)>,
    /// Moduli per nome.
    forms: HashMap<Vec<u8>, ObjRef>,
    /// Sfumature per nome, con le loro tinte.
    shadings: HashMap<Vec<u8>, u64>,
    /// Pattern di sfumatura per nome (le sfumature di Illustrator), con le loro tinte.
    patterns: HashMap<Vec<u8>, u64>,
}

fn dict_of(doc: &Document, d: Option<&Object>) -> Dict {
    d.and_then(|o| doc.dict(o)).unwrap_or_default()
}

fn read_res(doc: &Document, resources: &Dict, cat: &mut Catalog) -> Res {
    let mut r = Res::default();
    for (k, v) in dict_of(doc, resources.get(b"ColorSpace")).iter() {
        if let Some((name, rgb)) = swatch(doc, v) {
            cat.swatches.entry(name).or_insert(rgb);
        }
        let m = cat.mask(colorants(doc, v, 0));
        r.spaces.insert(k.clone(), m);
    }
    for (k, v) in dict_of(doc, resources.get(b"ExtGState")).iter() {
        let g = dict_of(doc, Some(v));
        let flag = |key: &[u8]| match g.get(key).map(|o| doc.resolve(o)).as_deref() {
            Some(Object::Bool(b)) => Some(*b),
            _ => None,
        };
        r.states.insert(k.clone(), (flag(b"OP"), flag(b"op")));
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
                let mask = matches!(
                    s.dict.get(b"ImageMask").map(|o| doc.resolve(o)).as_deref(),
                    Some(Object::Bool(true))
                );
                let m = s
                    .dict
                    .get(b"ColorSpace")
                    .map(|cs| cat.mask(colorants(doc, cs, 0)))
                    .unwrap_or(0);
                r.images.insert(k.clone(), (m, mask));
            }
            _ => {}
        }
    }
    for (k, v) in dict_of(doc, resources.get(b"Shading")).iter() {
        let obj = doc.resolve(v);
        let d = match &*obj {
            Object::Stream(s) => s.dict.clone(),
            Object::Dict(d) => d.clone(),
            _ => continue,
        };
        let m = d
            .get(b"ColorSpace")
            .map(|cs| cat.mask(colorants(doc, cs, 0)))
            .unwrap_or(0);
        r.shadings.insert(k.clone(), m);
    }
    for (k, v) in dict_of(doc, resources.get(b"Pattern")).iter() {
        let obj = doc.resolve(v);
        let d = match &*obj {
            Object::Stream(s) => s.dict.clone(),
            Object::Dict(d) => d.clone(),
            _ => continue,
        };
        if d.int(b"PatternType") != Some(2) {
            continue;
        }
        let m = dict_of(doc, d.get(b"Shading"))
            .get(b"ColorSpace")
            .map(|cs| cat.mask(colorants(doc, cs, 0)))
            .unwrap_or(0);
        r.patterns.insert(k.clone(), m);
    }
    r
}

/// Lo stato grafico che conta qui.
#[derive(Clone, Copy)]
struct State {
    /// Tinte del colore di riempimento e di contorno.
    fill: u64,
    stroke: u64,
    /// Tinte dello spazio colore di riempimento e di contorno (senza il pattern scelto).
    fill_cs: u64,
    stroke_cs: u64,
    /// La sovrastampa che il documento ha chiesto (riempimento, contorno).
    doc_fill: bool,
    doc_stroke: bool,
    /// La sovrastampa in vigore nel contenuto scritto; `None` se non si sa (dentro un modulo).
    cur_fill: Option<bool>,
    cur_stroke: Option<bool>,
    /// Modo di disegno del testo (Tr).
    text_mode: i64,
}

/// Cosa fare del contenuto.
#[derive(Clone, Copy)]
enum Mode {
    /// Contare gli oggetti dipinti con ogni tinta, in sovrastampa o no.
    Count,
    /// Accendere (`true`) o spegnere la sovrastampa degli oggetti con le tinte del bit.
    Set(u64, bool),
}

/// Quante volte ogni tinta è dipinta in sovrastampa e senza.
#[derive(Clone, Default)]
struct Tally {
    over: Vec<usize>,
    plain: Vec<usize>,
}

impl Tally {
    fn add(&mut self, mask: u64, overprint: bool) {
        for i in 0..MAX_SPOTS {
            if mask & (1 << i) == 0 {
                continue;
            }
            let v = if overprint {
                &mut self.over
            } else {
                &mut self.plain
            };
            if v.len() <= i {
                v.resize(i + 1, 0);
            }
            if let Some(n) = v.get_mut(i) {
                *n += 1;
            }
        }
    }

    fn count(&self, i: usize) -> (usize, usize) {
        (
            self.over.get(i).copied().unwrap_or(0),
            self.plain.get(i).copied().unwrap_or(0),
        )
    }

    /// Le tinte dipinte almeno una volta.
    fn used(&self) -> u64 {
        (0..MAX_SPOTS)
            .filter(|&i| {
                let (o, p) = self.count(i);
                o + p > 0
            })
            .fold(0, |m, i| m | (1 << i))
    }

    fn merge(&mut self, other: &Tally) {
        for (mine, theirs) in [(&mut self.over, &other.over), (&mut self.plain, &other.plain)] {
            if mine.len() < theirs.len() {
                mine.resize(theirs.len(), 0);
            }
            for (m, t) in mine.iter_mut().zip(theirs) {
                *m += t;
            }
        }
    }
}

/// Un contenuto interpretato: le operazioni riscritte (se è cambiato qualcosa), i conteggi e i
/// moduli che disegna.
struct Walked {
    ops: Option<Vec<Op>>,
    tally: Tally,
    forms: Vec<ObjRef>,
    /// Stati grafici aggiunti (riempimento, contorno) che il contenuto usa.
    added: HashSet<(bool, bool)>,
}

fn gs_name(fill: bool, stroke: bool) -> Vec<u8> {
    format!("{GS_PREFIX}{}{}", u8::from(fill), u8::from(stroke)).into_bytes()
}

/// Le tinte del pattern che `scn`/`SCN` sceglie con il suo ultimo operando, se è un nome.
fn pattern(op: &Op, res: &Res) -> u64 {
    op.operands
        .len()
        .checked_sub(1)
        .and_then(|i| op.name(i))
        .and_then(|n| res.patterns.get(n).copied())
        .unwrap_or(0)
}

fn walk(ops: Vec<Op>, res: &Res, start: State, mode: Mode) -> Walked {
    let mut out: Vec<Op> = Vec::with_capacity(ops.len());
    let mut tally = Tally::default();
    let mut forms = Vec::new();
    let mut added = HashSet::new();
    let mut changed = false;
    let mut s = start;
    let mut stack: Vec<State> = Vec::new();
    // Dove comincia il tracciato in costruzione: lo stato grafico va messo prima.
    let mut path_start: Option<usize> = None;
    for op in ops {
        let name0 = op.name(0).map(<[u8]>::to_vec);
        let kw = op.op.as_slice();
        // Che cosa dipinge l'operatore: tinte del riempimento e del contorno, se li usa.
        let mut paint: (Option<u64>, Option<u64>) = (None, None);
        match kw {
            b"q" => stack.push(s),
            b"Q" => {
                if let Some(prev) = stack.pop() {
                    s = prev;
                }
            }
            b"cs" => {
                s.fill_cs = name0.and_then(|n| res.spaces.get(&n).copied()).unwrap_or(0);
                s.fill = s.fill_cs;
            }
            b"CS" => {
                s.stroke_cs = name0.and_then(|n| res.spaces.get(&n).copied()).unwrap_or(0);
                s.stroke = s.stroke_cs;
            }
            // Un pattern di sfumatura dipinge con le tinte della sua sfumatura.
            b"scn" => s.fill = s.fill_cs | pattern(&op, res),
            b"SCN" => s.stroke = s.stroke_cs | pattern(&op, res),
            b"g" | b"rg" | b"k" => (s.fill, s.fill_cs) = (0, 0),
            b"G" | b"RG" | b"K" => (s.stroke, s.stroke_cs) = (0, 0),
            b"Tr" => s.text_mode = op.num(0).map_or(0, |v| v as i64),
            b"gs" => {
                if let Some((stroke, fill)) = name0.and_then(|n| res.states.get(&n).copied()) {
                    if let Some(v) = stroke {
                        s.doc_stroke = v;
                        s.cur_stroke = Some(v);
                        // Senza "op" il riempimento segue "OP".
                        if fill.is_none() {
                            s.doc_fill = v;
                            s.cur_fill = Some(v);
                        }
                    }
                    if let Some(v) = fill {
                        s.doc_fill = v;
                        s.cur_fill = Some(v);
                    }
                }
            }
            b"m" | b"re" => {
                if path_start.is_none() {
                    path_start = Some(out.len());
                }
            }
            b"f" | b"F" | b"f*" => paint = (Some(s.fill), None),
            b"S" | b"s" => paint = (None, Some(s.stroke)),
            b"B" | b"B*" | b"b" | b"b*" => paint = (Some(s.fill), Some(s.stroke)),
            b"Tj" | b"TJ" | b"'" | b"\"" => {
                paint = match s.text_mode {
                    0 | 4 => (Some(s.fill), None),
                    1 | 5 => (None, Some(s.stroke)),
                    2 | 6 => (Some(s.fill), Some(s.stroke)),
                    _ => (None, None),
                };
            }
            b"Do" => {
                if let Some(n) = &name0 {
                    if let Some(&(m, mask)) = res.images.get(n) {
                        paint = (Some(if mask { s.fill } else { m }), None);
                    } else if let Some(f) = res.forms.get(n) {
                        forms.push(*f);
                    }
                }
            }
            b"sh" => {
                if let Some(m) = name0.and_then(|n| res.shadings.get(&n).copied()) {
                    paint = (Some(m), None);
                }
            }
            b"BI" => {
                let mask = op.inline.as_ref().is_some_and(|(d, _)| {
                    matches!(
                        d.get(b"IM").or_else(|| d.get(b"ImageMask")),
                        Some(Object::Bool(true))
                    )
                });
                if mask {
                    paint = (Some(s.fill), None);
                }
            }
            _ => {}
        }
        let paints = paint.0.is_some() || paint.1.is_some();
        if paints {
            match mode {
                Mode::Count => {
                    if let Some(m) = paint.0 {
                        tally.add(m, s.cur_fill.unwrap_or(s.doc_fill));
                    }
                    if let Some(m) = paint.1 {
                        tally.add(m, s.cur_stroke.unwrap_or(s.doc_stroke));
                    }
                }
                Mode::Set(target, on) => {
                    // Con la tinta: la sovrastampa scelta. Senza: quella del documento, ma solo se
                    // l'abbiamo cambiata noi (dentro un modulo quella ereditata non si tocca).
                    let want = |m: Option<u64>, doc: bool, cur: Option<bool>| {
                        m.and_then(|m| {
                            if m & target != 0 {
                                Some(on).filter(|v| cur != Some(*v))
                            } else {
                                cur.filter(|c| *c != doc).map(|_| doc)
                            }
                        })
                    };
                    let fill = want(paint.0, s.doc_fill, s.cur_fill);
                    let stroke = want(paint.1, s.doc_stroke, s.cur_stroke);
                    let needs = fill.is_some() || stroke.is_some();
                    if needs {
                        let f = fill.unwrap_or(s.cur_fill.unwrap_or(s.doc_fill));
                        let k = stroke.unwrap_or(s.cur_stroke.unwrap_or(s.doc_stroke));
                        let gs = Op::new("gs", vec![Object::Name(gs_name(f, k))]);
                        let at = path_start.unwrap_or(out.len()).min(out.len());
                        out.insert(at, gs);
                        added.insert((f, k));
                        s.cur_fill = Some(f);
                        s.cur_stroke = Some(k);
                        changed = true;
                    }
                }
            }
        }
        let ends_path = matches!(
            kw,
            b"f" | b"F" | b"f*" | b"S" | b"s" | b"B" | b"B*" | b"b" | b"b*" | b"n"
        );
        out.push(op);
        if ends_path {
            path_start = None;
        }
    }
    Walked {
        ops: changed.then_some(out),
        tally,
        forms,
        added,
    }
}

/// Le risorse di una pagina, anche ereditate.
fn page_resources(doc: &Document, page: ObjRef) -> Dict {
    let mut node = page;
    for _ in 0..MAX_DEPTH {
        let obj = doc.get(node);
        let Some(d) = obj.as_dict() else { break };
        if let Some(r) = d.get(b"Resources") {
            return doc.dict(r).unwrap_or_default();
        }
        match d.reference(b"Parent") {
            Some(p) => node = p,
            None => break,
        }
    }
    Dict::new()
}

/// Il contenuto di una pagina, decodificato e in un pezzo solo.
fn page_content(doc: &Document, page: ObjRef) -> Result<Vec<u8>, EditError> {
    let obj = doc.get(page);
    let Some(c) = obj.as_dict().and_then(|d| d.get(b"Contents")) else {
        return Ok(Vec::new());
    };
    let list = match &*doc.resolve(c) {
        Object::Array(a) => a.clone(),
        _ => vec![c.clone()],
    };
    let mut out = Vec::new();
    for o in list {
        if let Object::Stream(s) = &*doc.resolve(&o) {
            out.extend(
                s.decoded_within(MAX_CONTENT)
                    .map_err(|e| EditError::Pdf(e.to_string()))?,
            );
            out.push(b'\n');
        }
    }
    Ok(out)
}

const START_PAGE: State = State {
    fill: 0,
    stroke: 0,
    fill_cs: 0,
    stroke_cs: 0,
    doc_fill: false,
    doc_stroke: false,
    cur_fill: Some(false),
    cur_stroke: Some(false),
    text_mode: 0,
};

const START_FORM: State = State {
    cur_fill: None,
    cur_stroke: None,
    ..START_PAGE
};

/// Un contenuto da interpretare: una pagina o un modulo.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Holder {
    Page(usize, ObjRef),
    Form(ObjRef),
}

/// Interpreta tutte le pagine e i moduli che disegnano. `f` riceve ogni contenuto interpretato.
fn each_content(
    doc: &Document,
    cat: &mut Catalog,
    mode: impl Fn(&mut Catalog) -> Mode,
    mut f: impl FnMut(Holder, Option<usize>, Walked),
) -> Result<(), EditError> {
    let pages = organize::pages(doc)?;
    let mut seen: HashSet<ObjRef> = HashSet::new();
    for (i, p) in pages.iter().enumerate() {
        let res = read_res(doc, &page_resources(doc, p.obj), cat);
        let ops = parse(&page_content(doc, p.obj)?).ops;
        let w = walk(ops, &res, START_PAGE, mode(cat));
        let mut todo: Vec<(ObjRef, usize)> = w.forms.iter().map(|f| (*f, 0)).collect();
        f(Holder::Page(i, p.obj), Some(i), w);
        // I moduli che la pagina disegna, e quelli dentro di loro; ognuno una volta sola.
        while let Some((form, depth)) = todo.pop() {
            if depth > MAX_DEPTH || !seen.insert(form) {
                continue;
            }
            let obj = doc.get(form);
            let Object::Stream(s) = &*obj else { continue };
            let res = read_res(doc, &dict_of(doc, s.dict.get(b"Resources")), cat);
            let data = s
                .decoded_within(MAX_CONTENT)
                .map_err(|e| EditError::Pdf(e.to_string()))?;
            let w = walk(parse(&data).ops, &res, START_FORM, mode(cat));
            todo.extend(w.forms.iter().map(|f| (*f, depth + 1)));
            f(Holder::Form(form), Some(i), w);
        }
    }
    Ok(())
}

/// Le tinte piatte del documento, con le pagine dove si vedono e la sovrastampa.
pub(crate) fn list(doc: &Document) -> Result<Vec<Spot>, EditError> {
    let mut cat = Catalog::default();
    let mut tally = Tally::default();
    let mut pages: HashMap<usize, Vec<usize>> = HashMap::new();
    each_content(
        doc,
        &mut cat,
        |_| Mode::Count,
        |_, page, w| {
            let used = w.tally.used();
            tally.merge(&w.tally);
            if let Some(p) = page {
                for i in 0..MAX_SPOTS {
                    if used & (1 << i) != 0 {
                        let list = pages.entry(i).or_default();
                        if list.last() != Some(&p) {
                            list.push(p);
                        }
                    }
                }
            }
        },
    )?;
    let mut out: Vec<Spot> = cat
        .names
        .iter()
        .enumerate()
        .map(|(i, raw)| {
            let (over, plain) = tally.count(i);
            let overprint = match (over, plain) {
                (0, 0) => Overprint::Unused,
                (_, 0) => Overprint::All,
                (0, _) => Overprint::None,
                _ => Overprint::Some,
            };
            let mut p = pages.remove(&i).unwrap_or_default();
            p.sort_unstable();
            p.dedup();
            Spot {
                name: String::from_utf8_lossy(raw).into_owned(),
                raw: raw.clone(),
                pages: p,
                overprint,
                swatch: cat.swatches.get(raw).copied(),
            }
        })
        .collect();
    out.sort_by_key(|s| s.name.to_lowercase());
    Ok(out)
}

/// Rinomina la tinta `from` in `to` dovunque sia definita. Restituisce gli oggetti cambiati.
pub(crate) fn rename(doc: &mut Document, from: &[u8], to: &[u8]) -> Result<usize, EditError> {
    let mut changed = 0;
    let numbers = doc.object_numbers();
    let mut updates: Vec<(ObjRef, Object)> = Vec::new();
    for (r, obj) in doc.scan_objects_subset(numbers) {
        let Ok(obj) = obj else { continue };
        let mut copy = (*obj).clone();
        if rename_in(&mut copy, from, to, 0) {
            updates.push((r, copy));
        }
    }
    for (r, obj) in updates {
        doc.set(r, obj);
        changed += 1;
    }
    Ok(changed)
}

/// Rinomina dentro un oggetto (ricorsivo): vero se è cambiato.
fn rename_in(obj: &mut Object, from: &[u8], to: &[u8], depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    match obj {
        Object::Array(a) => {
            let family = a.first().and_then(Object::as_name).map(<[u8]>::to_vec);
            let mut changed = false;
            match family.as_deref() {
                Some(b"Separation") => {
                    if let Some(Object::Name(n)) = a.get_mut(1)
                        && n.as_slice() == from
                    {
                        *n = to.to_vec();
                        changed = true;
                    }
                }
                Some(b"DeviceN") => {
                    if let Some(Object::Array(names)) = a.get_mut(1) {
                        for n in names.iter_mut() {
                            if let Object::Name(n) = n
                                && n.as_slice() == from
                            {
                                *n = to.to_vec();
                                changed = true;
                            }
                        }
                    }
                    // Gli attributi: /Colorants ha le tinte per nome.
                    if let Some(Object::Dict(attrs)) = a.get_mut(4)
                        && let Some(Object::Dict(col)) = attrs.get_mut(b"Colorants")
                        && let Some(v) = col.remove(from)
                    {
                        col.set(to.to_vec(), v);
                        changed = true;
                    }
                }
                _ => {}
            }
            for item in a.iter_mut() {
                changed |= rename_in(item, from, to, depth + 1);
            }
            changed
        }
        Object::Dict(d) => rename_dict(d, from, to, depth),
        Object::Stream(s) => rename_dict(&mut s.dict, from, to, depth),
        _ => false,
    }
}

fn rename_dict(d: &mut Dict, from: &[u8], to: &[u8], depth: usize) -> bool {
    let keys: Vec<Vec<u8>> = d.iter().map(|(k, _)| k.clone()).collect();
    let mut changed = false;
    for k in keys {
        if let Some(v) = d.get_mut(&k) {
            changed |= rename_in(v, from, to, depth + 1);
        }
    }
    changed
}

/// Accende o spegne la sovrastampa degli oggetti dipinti con la tinta `name`. Restituisce i
/// contenuti (pagine e moduli) cambiati.
pub(crate) fn set_overprint(doc: &mut Document, name: &[u8], on: bool) -> Result<usize, EditError> {
    let mut cat = Catalog::default();
    let target_name = name.to_vec();
    let mut results: Vec<(Holder, Walked)> = Vec::new();
    each_content(
        doc,
        &mut cat,
        |cat| {
            let bit = cat.mask(vec![target_name.clone()]);
            Mode::Set(bit, on)
        },
        |holder, _, w| {
            if w.ops.is_some() {
                results.push((holder, w));
            }
        },
    )?;
    if results.is_empty() {
        return Ok(0);
    }
    // Gli stati grafici aggiunti, una volta per documento.
    let mut states: HashMap<(bool, bool), ObjRef> = HashMap::new();
    for (_, w) in &results {
        for &(f, k) in &w.added {
            if let std::collections::hash_map::Entry::Vacant(e) = states.entry((f, k)) {
                let mut g = Dict::new();
                g.set(b"Type".to_vec(), Object::name("ExtGState"));
                g.set(b"OP".to_vec(), Object::Bool(k));
                g.set(b"op".to_vec(), Object::Bool(f));
                e.insert(doc.add(Object::Dict(g)));
            }
        }
    }
    let n = results.len();
    for (holder, w) in results {
        let Some(ops) = w.ops else { continue };
        let data = serialize_ops(&ops);
        let extra: Vec<(Vec<u8>, ObjRef)> = w
            .added
            .iter()
            .filter_map(|k| states.get(k).map(|r| (gs_name(k.0, k.1), *r)))
            .collect();
        match holder {
            Holder::Page(_, page) => {
                let mut res = page_resources(doc, page);
                add_states(doc, &mut res, &extra);
                let contents = doc.add(Object::Stream(Stream::flate(Dict::new(), &data)));
                doc.update_dict(page, |d| {
                    d.set(b"Contents".to_vec(), Object::Ref(contents));
                    d.set(b"Resources".to_vec(), Object::Dict(res));
                })?;
            }
            Holder::Form(form) => {
                let obj = doc.get(form);
                let Object::Stream(s) = &*obj else { continue };
                let mut dict = s.dict.clone();
                let mut res = dict_of(doc, dict.get(b"Resources"));
                add_states(doc, &mut res, &extra);
                dict.set(b"Resources".to_vec(), Object::Dict(res));
                dict.remove(b"Length");
                doc.set(form, Object::Stream(Stream::flate(dict, &data)));
            }
        }
    }
    Ok(n)
}

/// Aggiunge gli stati grafici alle risorse (copiate: le risorse condivise restano com'erano).
fn add_states(doc: &Document, res: &mut Dict, states: &[(Vec<u8>, ObjRef)]) {
    let mut g = dict_of(doc, res.get(b"ExtGState"));
    for (name, r) in states {
        g.set(name.clone(), Object::Ref(*r));
    }
    res.set(b"ExtGState".to_vec(), Object::Dict(g));
}
