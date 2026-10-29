//! Margini e nuove misure: la pagina diventa un "modulo" (form XObject) disegnato dentro una
//! pagina nuova, spostato o in scala. Il contenuto resta vettoriale e le risorse (font, immagini)
//! sono le stesse della pagina, senza copie. Annotazioni e formato finito seguono il contenuto.
//!
//! Le coordinate sono quelle del PDF (punti, origine in basso a sinistra). "Come si vede" vuol
//! dire con la rotazione della pagina già applicata: dopo il ridisegno la pagina non è più ruotata.

use pdfcraft_cos::{Dict, Document, ObjRef, Object, Stream};
use pdfcraft_organize as organize;
use winfiles_geom::{Margins, Size, mm_to_pt};

use crate::EditError;

/// Nome del modulo nelle risorse della pagina nuova.
const FORM_NAME: &str = "WFPagina";
/// Profondità massima dell'albero delle pagine quando si cercano gli attributi ereditati.
const MAX_DEPTH: usize = 64;

/// Matrice PDF `[a b c d e f]`: x' = a·x + c·y + e, y' = b·x + d·y + f.
type Matrix = [f64; 6];

fn apply(m: &Matrix, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

/// Prima `first`, poi `then`.
fn compose(then: &Matrix, first: &Matrix) -> Matrix {
    [
        then[0] * first[0] + then[2] * first[1],
        then[1] * first[0] + then[3] * first[1],
        then[0] * first[2] + then[2] * first[3],
        then[1] * first[2] + then[3] * first[3],
        then[0] * first[4] + then[2] * first[5] + then[4],
        then[1] * first[4] + then[3] * first[5] + then[5],
    ]
}

/// Il rettangolo che contiene `r` trasformato (le trasformazioni qui sono rotazioni di 90°,
/// spostamenti e scale: il risultato resta un rettangolo).
fn map_rect(m: &Matrix, r: [f64; 4]) -> [f64; 4] {
    let corners = [(r[0], r[1]), (r[2], r[1]), (r[0], r[3]), (r[2], r[3])];
    let mut out = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for (x, y) in corners {
        let (x, y) = apply(m, x, y);
        out = [out[0].min(x), out[1].min(y), out[2].max(x), out[3].max(y)];
    }
    out
}

/// Dal riquadro visibile (CropBox) alla pagina come si vede, con l'angolo in basso a sinistra
/// in (0, 0): la stessa matrice di `page_as_form` di PdfCraft.
pub(crate) fn upright(rotation: i64, c: [f64; 4]) -> Matrix {
    match rotation {
        90 => [0.0, -1.0, 1.0, 0.0, -c[1], c[2]],
        180 => [-1.0, 0.0, 0.0, -1.0, c[2], c[3]],
        270 => [0.0, 1.0, -1.0, 0.0, c[3], -c[0]],
        _ => [1.0, 0.0, 0.0, 1.0, -c[0], -c[1]],
    }
}

/// Come ridisegnare una pagina larga `w` e alta `h` punti (come si vede).
struct Plan {
    width: f64,
    height: f64,
    /// Dalla pagina come si vede alla pagina nuova.
    place: Matrix,
    /// Rettangoli pieni sotto il contenuto: x, y, larghezza, altezza e colore.
    fills: Vec<([f64; 4], [u8; 3])>,
    /// Il formato finito segue il contenuto (nuove misure) o diventa la pagina (margini).
    keep_trim: bool,
}

/// Margini attorno alla pagina; quelli negativi tolgono dal bordo (il modulo resta intero, è la
/// pagina più piccola a nasconderne una parte).
pub(crate) fn margins(doc: &mut Document, page: usize, m: Margins, color: [u8; 3]) -> Result<(), EditError> {
    let (top, right, bottom, left) = (
        mm_to_pt(m.top),
        mm_to_pt(m.right),
        mm_to_pt(m.bottom),
        mm_to_pt(m.left),
    );
    reframe(doc, page, |w, h| {
        let (width, height) = (w + left + right, h + top + bottom);
        let bands = [
            [0.0, 0.0, left, height],
            [width - right, 0.0, right, height],
            [left, 0.0, w, bottom],
            [left, height - top, w, top],
        ];
        Plan {
            width,
            height,
            place: [1.0, 0.0, 0.0, 1.0, left, bottom],
            fills: bands
                .into_iter()
                .filter(|b| b[2] > 0.0 && b[3] > 0.0)
                .map(|b| (b, color))
                .collect(),
            keep_trim: false,
        }
    })
}

/// La pagina nella misura `target` (mm), in proporzione e al centro. Con `auto_orient` la misura
/// si gira come la pagina (A4 orizzontale per una pagina larga).
pub(crate) fn fit(doc: &mut Document, page: usize, target: Size, auto_orient: bool) -> Result<(), EditError> {
    let (mut tw, mut th) = (mm_to_pt(target.w), mm_to_pt(target.h));
    reframe(doc, page, |w, h| {
        if auto_orient && (w > h) != (tw > th) {
            std::mem::swap(&mut tw, &mut th);
        }
        let s = (tw / w).min(th / h);
        Plan {
            width: tw,
            height: th,
            place: [s, 0.0, 0.0, s, (tw - w * s) / 2.0, (th - h * s) / 2.0],
            fills: Vec::new(),
            keep_trim: true,
        }
    })
}

/// Un attributo della pagina, anche se lo eredita dai nodi sopra di lei.
fn inherited(doc: &Document, page: ObjRef, key: &[u8]) -> Option<Object> {
    let mut node = page;
    for _ in 0..MAX_DEPTH {
        let obj = doc.get(node);
        let d = obj.as_dict()?;
        if let Some(v) = d.get(key) {
            return Some(v.clone());
        }
        node = d.reference(b"Parent")?;
    }
    None
}

/// Il contenuto della pagina, decodificato e in un pezzo solo.
fn content(doc: &Document, page: ObjRef, number: usize) -> Result<Vec<u8>, EditError> {
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
            let data = s
                .decoded()
                .map_err(|e| EditError::Pdf(format!("contenuto della pagina {number}: {e}")))?;
            out.extend(data);
            out.push(b'\n');
        }
    }
    Ok(out)
}

fn rect_object(r: [f64; 4]) -> Object {
    Object::Array(r.iter().map(|v| Object::Real(*v)).collect())
}

fn num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// Sposta i rettangoli delle annotazioni (link, commenti, campi) insieme al contenuto.
fn move_annotations(doc: &mut Document, page: ObjRef, m: &Matrix) -> Result<(), EditError> {
    let obj = doc.get(page);
    let Some(annots) = obj.as_dict().and_then(|d| d.get(b"Annots")).cloned() else {
        return Ok(());
    };
    let move_dict = |d: &mut Dict| {
        let rect = d
            .get(b"Rect")
            .and_then(Object::as_array)
            .map(|a| a.iter().filter_map(Object::as_f64).collect::<Vec<_>>());
        if let Some(&[x0, y0, x1, y1]) = rect.as_deref() {
            let r = [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)];
            d.set(b"Rect".to_vec(), rect_object(map_rect(m, r)));
        }
        let quads = d
            .get(b"QuadPoints")
            .and_then(Object::as_array)
            .map(|a| a.iter().filter_map(Object::as_f64).collect::<Vec<_>>());
        if let Some(q) = quads.filter(|q| q.len() % 2 == 0) {
            let moved = q
                .chunks_exact(2)
                .flat_map(|p| match p {
                    [x, y] => {
                        let (x, y) = apply(m, *x, *y);
                        vec![Object::Real(x), Object::Real(y)]
                    }
                    _ => Vec::new(),
                })
                .collect();
            d.set(b"QuadPoints".to_vec(), Object::Array(moved));
        }
    };
    // Le annotazioni sono di solito riferimenti a oggetti, ma possono stare anche dentro la
    // pagina: in quel caso si riscrive tutto l'elenco.
    if let Object::Array(list) = &*doc.resolve(&annots) {
        let mut direct = list.clone();
        let mut changed = false;
        for item in &mut direct {
            match item {
                Object::Ref(r) => doc.update_dict(*r, move_dict)?,
                Object::Dict(d) => {
                    move_dict(d);
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            doc.update_dict(page, |d| d.set(b"Annots".to_vec(), Object::Array(direct)))?;
        }
    }
    Ok(())
}

/// La pagina da ridisegnare: dove sta, le sue misure e come raddrizzarla.
struct Source {
    page: ObjRef,
    number: usize,
    /// MediaBox, CropBox, BleedBox, TrimBox, ArtBox.
    boxes: [[f64; 4]; 5],
    crop: [f64; 4],
    /// Larghezza e altezza come si vede, in punti.
    w: f64,
    h: f64,
    /// Dal CropBox alla pagina come si vede.
    upright: Matrix,
}

fn source(doc: &Document, index: usize) -> Result<Source, EditError> {
    let number = index.saturating_add(1);
    let page = organize::pages(doc)?
        .get(index)
        .map(|p| p.obj)
        .ok_or(EditError::NoPage(number))?;
    let boxes = organize::page_boxes(doc)?
        .get(index)
        .copied()
        .ok_or(EditError::NoPage(number))?;
    let rotation = organize::page_rotation(doc, index)?;
    let crop = boxes[1];
    let (cw, ch) = (crop[2] - crop[0], crop[3] - crop[1]);
    if !(cw.is_finite() && ch.is_finite() && cw >= 1.0 && ch >= 1.0) {
        return Err(EditError::Pdf(format!("la pagina {number} non ha una misura")));
    }
    let (w, h) = if rotation == 90 || rotation == 270 {
        (ch, cw)
    } else {
        (cw, ch)
    };
    Ok(Source {
        page,
        number,
        boxes,
        crop,
        w,
        h,
        upright: upright(rotation, crop),
    })
}

/// Il modulo: il contenuto `body` della pagina con le sue risorse, così com'è.
fn page_form(doc: &mut Document, src: &Source, body: &[u8]) -> ObjRef {
    let mut form = Dict::new();
    form.set(b"Type".to_vec(), Object::name("XObject"));
    form.set(b"Subtype".to_vec(), Object::name("Form"));
    form.set(b"BBox".to_vec(), rect_object(src.crop));
    form.set(
        b"Matrix".to_vec(),
        Object::Array(src.upright.iter().map(|v| Object::Real(*v)).collect()),
    );
    form.set(
        b"Resources".to_vec(),
        inherited(doc, src.page, b"Resources").unwrap_or(Object::Dict(Dict::new())),
    );
    let group = doc.get(src.page).as_dict().and_then(|d| d.get(b"Group")).cloned();
    if let Some(g) = group {
        form.set(b"Group".to_vec(), g);
    }
    doc.add(Object::Stream(Stream::flate(form, body)))
}

/// La pagina nuova, larga `width` e alta `height`: disegna `ops` con i moduli `forms`.
/// `total` porta dal CropBox di prima alla pagina nuova (per le annotazioni).
struct NewPage<'a> {
    forms: &'a [(&'a str, ObjRef)],
    ops: String,
    width: f64,
    height: f64,
    total: Matrix,
    trim: Option<[f64; 4]>,
    bleed: Option<[f64; 4]>,
}

fn finish(doc: &mut Document, src: &Source, new: NewPage<'_>) -> Result<(), EditError> {
    let contents = doc.add(Object::Stream(Stream::flate(Dict::new(), new.ops.as_bytes())));
    let media = [0.0, 0.0, new.width, new.height];
    move_annotations(doc, src.page, &new.total)?;
    doc.update_dict(src.page, |d| {
        let mut xobjects = Dict::new();
        for (name, r) in new.forms {
            xobjects.set(name.as_bytes().to_vec(), Object::Ref(*r));
        }
        let mut resources = Dict::new();
        resources.set(b"XObject".to_vec(), Object::Dict(xobjects));
        d.set(b"Resources".to_vec(), Object::Dict(resources));
        d.set(b"Contents".to_vec(), Object::Ref(contents));
        d.set(b"MediaBox".to_vec(), rect_object(media));
        // Valori espliciti: quelli ereditati dai nodi sopra non devono tornare.
        d.set(b"CropBox".to_vec(), rect_object(media));
        d.set(b"Rotate".to_vec(), Object::Int(0));
        d.remove(b"ArtBox");
        d.remove(b"Group");
        match new.bleed {
            Some(b) => d.set(b"BleedBox".to_vec(), rect_object(b)),
            None => {
                d.remove(b"BleedBox");
            }
        }
        match new.trim {
            Some(t) => d.set(b"TrimBox".to_vec(), rect_object(t)),
            None => {
                d.remove(b"TrimBox");
            }
        }
    })?;
    Ok(())
}

fn reframe(doc: &mut Document, index: usize, plan: impl FnOnce(f64, f64) -> Plan) -> Result<(), EditError> {
    let src = source(doc, index)?;
    let number = src.number;
    let plan = plan(src.w, src.h);
    // Con i margini negativi la pagina si accorcia: almeno 1 mm deve restare.
    let min = mm_to_pt(1.0);
    if !(plan.width.is_finite() && plan.height.is_finite() && plan.width >= min && plan.height >= min) {
        return Err(EditError::Invalid(format!(
            "la pagina {number} diventerebbe troppo piccola"
        )));
    }
    let body = content(doc, src.page, number)?;
    let form = page_form(doc, &src, &body);

    // La pagina nuova: le fasce colorate, poi il modulo al suo posto.
    let mut ops = String::new();
    for (r, c) in &plan.fills {
        let [red, green, blue] = c.map(|v| num(f64::from(v) / 255.0));
        ops.push_str(&format!(
            "{red} {green} {blue} rg {} {} {} {} re f\n",
            num(r[0]),
            num(r[1]),
            num(r[2]),
            num(r[3])
        ));
    }
    let p = &plan.place;
    ops.push_str(&format!(
        "q {} {} {} {} {} {} cm /{FORM_NAME} Do Q\n",
        num(p[0]),
        num(p[1]),
        num(p[2]),
        num(p[3]),
        num(p[4]),
        num(p[5])
    ));

    let total = compose(&plan.place, &src.upright);
    let boxes = src.boxes;
    let trim_differs = boxes[3].iter().zip(src.crop).any(|(t, c)| (t - c).abs() > 0.01);
    let trim = (plan.keep_trim && trim_differs).then(|| {
        let t = map_rect(&total, boxes[3]);
        [
            t[0].max(0.0),
            t[1].max(0.0),
            t[2].min(plan.width),
            t[3].min(plan.height),
        ]
    });
    finish(
        doc,
        &src,
        NewPage {
            forms: &[(FORM_NAME, form)],
            ops,
            width: plan.width,
            height: plan.height,
            total,
            trim,
            bleed: None,
        },
    )
}

/// Come si crea l'abbondanza che manca.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BleedMode {
    /// La grafica vicino al bordo si ribalta fuori dal formato finito.
    Mirror,
    /// L'ultimo mezzo millimetro di grafica si allunga fino al bordo nuovo.
    Stretch,
}

/// L'abbondanza da creare attorno al formato finito.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BleedSpec {
    /// Il formato finito in millimetri dall'angolo in alto a sinistra della pagina come si vede:
    /// sinistra, sopra, destra, sotto.
    pub finished: [f64; 4],
    /// Quanta abbondanza, in millimetri.
    pub amount: f64,
    /// I lati dove si crea: sopra, destra, sotto, sinistra. Sugli altri resta la pagina com'è.
    pub sides: [bool; 4],
    pub mode: BleedMode,
}

/// Il modulo con la grafica senza fustella: quello ribaltato nell'abbondanza (l'altro, con la
/// pagina intera, è `FORM_NAME`).
const FORM_ART: &str = "WFGrafica";
/// Il modulo disegnato sopra tutto, senza ritaglio: la fustella (la pagina vera si ferma al
/// formato finito e taglierebbe a metà lo spessore della linea) o, quando l'abbondanza si crea
/// solo per i tracciati scelti, tutto il resto della pagina.
const FORM_OVER: &str = "WFSopra";
/// Di quanto (punti) le fasce dell'abbondanza passano sotto la pagina vera: senza, dove si
/// toccano i due bordi sfumati dall'antialias dell'anteprima si vedrebbe un filo bianco.
const SEAM: f64 = 1.0;
/// La striscia di grafica che "Allunga" stira fino al bordo nuovo.
const STRETCH_MM: f64 = 0.5;

/// Crea l'abbondanza: la pagina diventa formato finito più `amount` sui lati scelti; lì si
/// disegna `art` (la grafica senza fustella) ribaltata o allungata. Sopra, `base` (la pagina vera)
/// fino al formato finito (i segni di taglio e il bianco attorno spariscono) e `over` intero (la
/// fustella, o il resto della pagina quando si allunga solo la grafica scelta).
/// TrimBox e BleedBox si scrivono, così il RIP sa dove tagliare.
pub(crate) fn bleed(
    doc: &mut Document,
    index: usize,
    spec: &BleedSpec,
    base: &[u8],
    art: &[u8],
    over: Option<&[u8]>,
) -> Result<(), EditError> {
    let src = source(doc, index)?;
    let number = src.number;
    let (w, h) = (src.w, src.h);
    let f = spec.finished;
    // Dal formato finito in mm (alto a sinistra) ai punti della pagina come si vede (basso a sinistra).
    let fx0 = mm_to_pt(f[0]).clamp(0.0, w);
    let fx1 = mm_to_pt(f[2]).clamp(0.0, w);
    let fy0 = (h - mm_to_pt(f[3])).clamp(0.0, h);
    let fy1 = (h - mm_to_pt(f[1])).clamp(0.0, h);
    let min = mm_to_pt(1.0);
    if !(fx1 - fx0 >= min && fy1 - fy0 >= min) {
        return Err(EditError::Invalid(format!(
            "il formato finito della pagina {number} non è valido"
        )));
    }
    let b = mm_to_pt(spec.amount);
    let [top, right, bottom, left] = spec.sides;
    // La pagina nuova (come si vede): formato finito più l'abbondanza, o la pagina di prima.
    let r = [
        if left { fx0 - b } else { 0.0 },
        if bottom { fy0 - b } else { 0.0 },
        if right { fx1 + b } else { w },
        if top { fy1 + b } else { h },
    ];
    // Fin dove arriva la pagina vera: sui lati nuovi il formato finito, meno `SEAM` perché il
    // suo bordo sfumato cada sopra la fascia piena e non sopra il bordo sfumato della fascia
    // (il bianco dietro la grafica si vedrebbe come un filo).
    let c = [
        if left { fx0 + SEAM } else { r[0] },
        if bottom { fy0 + SEAM } else { r[1] },
        if right { fx1 - SEAM } else { r[2] },
        if top { fy1 - SEAM } else { r[3] },
    ];
    // Il bordo da cui si ribalta o si allunga: il formato finito.
    let edge = [fx0, fy0, fx1, fy1];
    let base = page_form(doc, &src, base);
    let art = page_form(doc, &src, art);
    let over = over.map(|d| page_form(doc, &src, d));
    let mut ops = format!("q 1 0 0 1 {} {} cm\n", num(-r[0]), num(-r[1]));
    // I pezzi attorno: sinistra/centro/destra per sotto/centro/sopra, tranne il centro.
    let xs = [(r[0], edge[0], -1), (edge[0], edge[2], 0), (edge[2], r[2], 1)];
    let ys = [(r[1], edge[1], -1), (edge[1], edge[3], 0), (edge[3], r[3], 1)];
    let s = mm_to_pt(STRETCH_MM);
    // Per un asse: x' = a·x + e.
    let axis = |dir: i32, lo: f64, hi: f64| -> (f64, f64) {
        match (dir, spec.mode) {
            (0, _) => (1.0, 0.0),
            // L'asse sta un soffio dentro il formato finito (vedi `SEAM`).
            (-1, BleedMode::Mirror) => (-1.0, 2.0 * (hi + SEAM)),
            (_, BleedMode::Mirror) => (-1.0, 2.0 * (lo - SEAM)),
            (-1, BleedMode::Stretch) => {
                // Da [hi, hi+s] a [lo, hi+s]: il punto hi+s resta fermo.
                let k = (hi - lo + s) / s;
                (k, (hi + s) * (1.0 - k))
            }
            (_, BleedMode::Stretch) => {
                let k = (hi - lo + s) / s;
                (k, (lo - s) * (1.0 - k))
            }
        }
    };
    for &(x0, x1, dx) in &xs {
        for &(y0, y1, dy) in &ys {
            if (dx == 0 && dy == 0) || x1 - x0 < 0.01 || y1 - y0 < 0.01 {
                continue;
            }
            let (ax, ex) = axis(dx, x0, x1);
            let (ay, ey) = axis(dy, y0, y1);
            // Il pezzo entra di poco nella pagina vera, così sotto il suo bordo non c'è il bianco.
            let grow = |v0: f64, v1: f64, dir: i32| match dir {
                -1 => (v0, v1 + 2.0 * SEAM),
                1 => (v0 - 2.0 * SEAM, v1),
                _ => (v0, v1),
            };
            let (x0, x1) = grow(x0, x1, dx);
            let (y0, y1) = grow(y0, y1, dy);
            ops.push_str(&format!(
                "q {} {} {} {} re W n {} 0 0 {} {} {} cm /{FORM_ART} Do Q\n",
                num(x0),
                num(y0),
                num(x1 - x0),
                num(y1 - y0),
                num(ax),
                num(ay),
                num(ex),
                num(ey)
            ));
        }
    }
    ops.push_str(&format!(
        "q {} {} {} {} re W n /{FORM_NAME} Do Q\n",
        num(c[0]),
        num(c[1]),
        num(c[2] - c[0]),
        num(c[3] - c[1])
    ));
    if over.is_some() {
        ops.push_str(&format!("/{FORM_OVER} Do\n"));
    }
    ops.push_str("Q\n");
    let mut forms = vec![(FORM_NAME, base), (FORM_ART, art)];
    if let Some(d) = over {
        forms.push((FORM_OVER, d));
    }
    let (width, height) = (r[2] - r[0], r[3] - r[1]);
    let place: Matrix = [1.0, 0.0, 0.0, 1.0, -r[0], -r[1]];
    let total = compose(&place, &src.upright);
    let trim = [fx0 - r[0], fy0 - r[1], fx1 - r[0], fy1 - r[1]];
    finish(
        doc,
        &src,
        NewPage {
            forms: &forms,
            ops,
            width,
            height,
            total,
            trim: Some(trim),
            bleed: Some([0.0, 0.0, width, height]),
        },
    )
}
