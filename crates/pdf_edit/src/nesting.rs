//! Nesting (ADR 0027): le sagome da disporre lette dalle pagine, e il PDF dei fogli disposti per
//! il RIP. Ogni pagina di partenza entra una volta sola come modulo (form XObject) con le sue
//! risorse, poi si disegna in ogni copia spostata, girata e ritagliata alla sua sagoma più
//! l'abbondanza. Una copia ritagliata non si porta dietro i tracciati che stanno tutti fuori
//! dalla sua sagoma: il ritaglio li nasconderebbe, ma il RIP li leggerebbe lo stesso. Le tinte
//! piatte (fustella, bianco, vernice) restano com'erano.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::stroke::offset::StrokeOffset;
use i_overlay::mesh::float::style::{LineJoin, OutlineStyle, StrokeStyle};
use pdfcraft_cos::{Dict, Document, Object, Stream};
use pdfcraft_organize as organize;
use winfiles_geom::{mm_to_pt, pt_to_mm};

use crate::paths::{self, NewDie, PageScan, PathId};
use crate::{EditError, PdfFile};

/// Contorni in millimetri, y verso il basso: un contorno dentro un altro è un foro.
pub type Contours = Vec<Vec<[f64; 2]>>;

/// Lo spessore con cui si chiudono le fustelle fatte di tratti aperti (mm).
const OPEN_DIE_MM: f64 = 0.2;
/// Il lato più lungo che un PDF tiene in punti normali (200 pollici, 508 cm). Oltre, la pagina
/// dichiara una `UserUnit` più grande (PDF 1.6): i lettori la moltiplicano per le sue misure.
const MAX_PLAIN_PT: f64 = 14_400.0;
/// Il foglio o rotolo più lungo che si accetta: 1 km, in pratica nessun limite.
pub const MAX_SHEET_MM: f64 = 1_000_000.0;

/// La `UserUnit` intera più piccola con cui un foglio di `w`×`h` punti sta nei 200 pollici.
pub fn user_unit(w: f64, h: f64) -> f64 {
    (w.max(h) / MAX_PLAIN_PT).ceil().max(1.0)
}
/// Un modulo dei fogli: sorgente, pagina e tracciati tolti.
type FormKey = (usize, usize, Vec<PathId>);

/// Pagine di partenza diverse in un PDF di fogli.
const MAX_FORMS: usize = 2000;

/// La sagoma dei tracciati `ids` (la fustella): l'area che chiudono. Se sono tutti chiusi è la
/// loro unione; se ci sono tratti aperti che si toccano, l'area dentro il giro che fanno.
pub fn die_shape(scan: &PageScan, ids: &[PathId]) -> Result<Contours, EditError> {
    let chosen: Vec<_> = scan.paths.iter().filter(|p| ids.contains(&p.id)).collect();
    let closed = chosen.iter().all(|p| p.closed());
    if closed {
        // Ogni tracciato con la sua forma; poi pari-dispari fra tracciati: un contorno dentro
        // un altro (il buco della O, fatta di due cerchi) è un foro, non si riempie.
        let mut all: Contours = Vec::new();
        for p in &chosen {
            if let Ok(shapes) = paths::outline(scan, &[p.id]) {
                all.extend(shapes.into_iter().flatten());
            }
        }
        let shape: Contours = all
            .simplify_shape(FillRule::EvenOdd)
            .into_iter()
            .flatten()
            .filter(|c| c.len() >= 3)
            .collect();
        if !shape.is_empty() {
            return Ok(shape);
        }
    }
    // Tratti aperti: ognuno diventa una striscia sottile; il contorno esterno del giro è la
    // sagoma.
    let mut strips: Contours = Vec::new();
    for p in &chosen {
        for part in &p.parts {
            if part.points.len() < 2 {
                continue;
            }
            let style = StrokeStyle::new(OPEN_DIE_MM).line_join(LineJoin::Miter(0.1));
            for shape in part.points.stroke(style, part.closed) {
                strips.extend(shape);
            }
        }
    }
    let outer: Contours = strips
        .simplify_shape(FillRule::NonZero)
        .into_iter()
        .filter_map(|s| s.into_iter().next())
        .collect();
    let filled: Contours = outer
        .simplify_shape(FillRule::NonZero)
        .into_iter()
        .flatten()
        .filter(|c| c.len() >= 3)
        .collect();
    if filled.is_empty() {
        return Err(EditError::Invalid("la fustella non chiude un'area".into()));
    }
    Ok(filled)
}

/// I pezzi staccati di una sagoma: ogni contorno esterno con i suoi fori (la O resta una, anche
/// se fatta di due tracciati); una sagoma dentro il foro di un'altra è un pezzo a sé.
pub fn split_shape(shape: &[Vec<[f64; 2]>]) -> Vec<Contours> {
    shape
        .to_vec()
        .simplify_shape(FillRule::EvenOdd)
        .into_iter()
        .map(|s| s.into_iter().filter(|c| c.len() >= 3).collect::<Contours>())
        .filter(|s| !s.is_empty())
        .collect()
}

/// La sagoma allargata di `by` mm, con gli spigoli vivi (un rettangolo resta un rettangolo).
pub fn grow_shape(shape: &[Vec<[f64; 2]>], by: f64) -> Contours {
    let clean: Vec<Contours> = shape.to_vec().simplify_shape(FillRule::EvenOdd);
    if !(by.is_finite() && by > 0.0) {
        return clean.into_iter().flatten().collect();
    }
    let style = OutlineStyle::new(by).line_join(LineJoin::Miter(0.1));
    clean.outline(&style).into_iter().flatten().collect()
}

/// Il rettangolo che contiene i contorni: x0, y0, x1, y1.
pub fn shape_bounds(shape: &[Vec<[f64; 2]>]) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    for p in shape.iter().flatten() {
        b = Some(match b {
            None => [p[0], p[1], p[0], p[1]],
            Some(b) => [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])],
        });
    }
    b
}

/// L'area dei contorni (mm², i fori tolti).
pub fn shape_area(shape: &[Vec<[f64; 2]>]) -> f64 {
    let empty: Contours = Vec::new();
    shape
        .to_vec()
        .overlay(&empty, OverlayRule::Subject, FillRule::EvenOdd)
        .iter()
        .flatten()
        .map(|c| paths::signed_area(c))
        .sum::<f64>()
        .abs()
}

/// Una copia sul foglio.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetPiece {
    /// Quale PDF fra quelli passati a [`nest_document`] e quale pagina.
    pub source: usize,
    pub page: usize,
    /// Dalla pagina come si vede (mm, y in giù) al foglio (mm, y in giù):
    /// x' = a·x + c·y + e, y' = b·x + d·y + f.
    pub matrix: [f64; 6],
    /// Il ritaglio sulla pagina (mm, y in giù): la sagoma con l'abbondanza. Vuoto: nessuno.
    pub clip: Contours,
    /// I tracciati della pagina da non disegnare: la fustella, quando la tagliano le linee comuni.
    pub hide: Vec<PathId>,
}

/// Un foglio (o un pezzo di rotolo).
#[derive(Clone, Debug, PartialEq)]
pub struct NestSheet {
    pub width: f64,
    pub height: f64,
    pub pieces: Vec<SheetPiece>,
    /// Le linee di taglio comuni (polilinee in mm sul foglio, y in giù), sopra le copie.
    pub lines: Contours,
}

fn num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn rect_object(r: [f64; 4]) -> Object {
    Object::Array(r.iter().map(|v| Object::Real(*v)).collect())
}

/// Il PDF dei fogli: una pagina per foglio, alla misura vera, con TrimBox uguale alla pagina.
/// Le linee comuni dei fogli (`lines`) si tracciano nella tinta `ink`; senza, non si disegnano.
pub fn nest_document(
    sources: &[&PdfFile],
    sheets: &[NestSheet],
    ink: Option<&NewDie>,
) -> Result<PdfFile, EditError> {
    if sheets.is_empty() {
        return Err(EditError::Invalid("nessun foglio da salvare".into()));
    }
    if let Some(die) = ink {
        let name = die.name.trim();
        if name.is_empty() || name.len() > 127 {
            return Err(EditError::Invalid("nome della tinta non valido".into()));
        }
        if !(die.width.is_finite() && (0.0..=100.0).contains(&die.width)) {
            return Err(EditError::Invalid("spessore non valido".into()));
        }
    }
    let mut doc = Document::new_empty();
    let stroke = ink.map(|die| paths::spot_stroke(&mut doc, die));
    // Ogni pagina di partenza una volta sola, intera: (sorgente, pagina) → (modulo, altezza in
    // punti, usato così com'è).
    let mut bases: Vec<((usize, usize), pdfcraft_cos::ObjRef, f64, bool)> = Vec::new();
    // Le varianti della pagina senza i tracciati che non servono a una sagoma: (sorgente, pagina,
    // tracciati tolti) → (modulo, altezza). Le copie della stessa sagoma usano la stessa.
    let mut forms: Vec<(FormKey, pdfcraft_cos::ObjRef, f64)> = Vec::new();
    // Quello che c'è sulle pagine, letto una volta sola.
    let mut scans: Vec<((usize, usize), Option<PageScan>)> = Vec::new();
    // I tracciati fuori da ogni ritaglio già visto: (sorgente, pagina, ritaglio) → tracciati.
    let mut outside: Vec<((usize, usize), Contours, Vec<PathId>)> = Vec::new();
    for (n, sheet) in sheets.iter().enumerate() {
        let ok = |v: f64| v.is_finite() && (1.0..=MAX_SHEET_MM).contains(&v);
        if !ok(sheet.width) || !ok(sheet.height) {
            return Err(EditError::Invalid(format!(
                "il foglio {} è troppo grande per un PDF (al massimo {:.0} m)",
                n + 1,
                MAX_SHEET_MM / 1000.0
            )));
        }
        let (sw, sh) = (mm_to_pt(sheet.width), mm_to_pt(sheet.height));
        // Oltre i 508 cm la pagina conta in unità più grandi: il disegno si rimpicciolisce dello
        // stesso fattore e i lettori lo rimettono a misura vera.
        let unit = user_unit(sw, sh);
        let mut ops = String::new();
        if unit > 1.0 {
            // Con tutte le cifre: su 50 m anche un decimillesimo farebbe 5 mm.
            let k = format!("{:.12}", 1.0 / unit);
            let k = k.trim_end_matches('0');
            ops.push_str(&format!("q {k} 0 0 {k} 0 0 cm\n"));
        }
        let mut used = Dict::new();
        for piece in &sheet.pieces {
            let src = sources
                .get(piece.source)
                .ok_or_else(|| EditError::Invalid("file mancante".into()))?;
            let at = (piece.source, piece.page);
            // Cosa non disegnare: la fustella tolta per le linee comuni e ogni tracciato che sta
            // tutto fuori dal ritaglio della copia. Il ritaglio li nasconderebbe alla vista, ma il
            // RIP li leggerebbe lo stesso: con tante sagome sulla stessa pagina, ogni copia si
            // porterebbe dietro i tracciati di tutte le altre.
            let mut hide = piece.hide.clone();
            if !piece.clip.is_empty() {
                let known = outside
                    .iter()
                    .find(|(k, c, _)| *k == at && *c == piece.clip)
                    .map(|(_, _, ids)| ids.clone());
                let ids = match known {
                    Some(ids) => ids,
                    None => {
                        if !scans.iter().any(|(k, _)| *k == at) {
                            scans.push((at, src.scan_page(piece.page).ok()));
                        }
                        let ids = scans
                            .iter()
                            .find(|(k, _)| *k == at)
                            .and_then(|(_, s)| s.as_ref())
                            .map(|s| paths_outside(s, &piece.clip))
                            .unwrap_or_default();
                        outside.push((at, piece.clip.clone(), ids.clone()));
                        ids
                    }
                };
                hide.extend(ids);
            }
            hide.sort();
            hide.dedup();
            let key = (piece.source, piece.page, hide);
            let (form, h) = match forms.iter().find(|(k, _, _)| *k == key) {
                Some((_, r, h)) => (*r, *h),
                None => {
                    if forms.len() >= MAX_FORMS {
                        return Err(EditError::Invalid("troppi file diversi in un foglio".into()));
                    }
                    let (base, h) = match bases.iter().find(|(k, _, _, _)| *k == at) {
                        Some((_, r, h, _)) => (*r, *h),
                        None => {
                            let (r, (_, h)) = organize::page_as_form(&mut doc, &src.0, piece.page)?;
                            bases.push((at, r, h, false));
                            (r, h)
                        }
                    };
                    let r = if key.2.is_empty() {
                        if let Some(b) = bases.iter_mut().find(|(k, _, _, _)| *k == at) {
                            b.3 = true;
                        }
                        base
                    } else {
                        // Lo stesso modulo (risorse, riquadro, matrice) con il contenuto sfoltito:
                        // le risorse sono in comune, immagini e caratteri non si copiano due volte.
                        let content = paths::pruned(&src.0, piece.page, &key.2)?;
                        let dict = match &*doc.get(base) {
                            Object::Stream(st) => st.dict.clone(),
                            _ => return Err(EditError::Invalid("pagina non leggibile".into())),
                        };
                        doc.add(Object::Stream(Stream::flate(dict, &content)))
                    };
                    forms.push((key.clone(), r, h));
                    (r, h)
                }
            };
            let name = format!("WFN{}", forms.iter().position(|(k, _, _)| *k == key).unwrap_or(0));
            used.set(name.as_bytes().to_vec(), Object::Ref(form));
            // Dai punti del modulo (origine in basso) ai punti del foglio (origine in basso),
            // passando per i millimetri con la y in giù di `matrix`.
            let [a, b, c, d, e, f] = piece.matrix;
            let k = pt_to_mm(1.0);
            let cm = [a, -b, -c, d, c * h + e / k, sh - d * h - f / k];
            if !cm.iter().all(|v| v.is_finite()) {
                return Err(EditError::Invalid("posizione non valida".into()));
            }
            ops.push_str("q ");
            ops.push_str(&cm.map(num).join(" "));
            ops.push_str(" cm\n");
            let mut any = false;
            for contour in piece.clip.iter().filter(|c| c.len() >= 3) {
                for (i, p) in contour.iter().enumerate() {
                    let (x, y) = (p[0] / k, h - p[1] / k);
                    ops.push_str(&format!(
                        "{} {} {}\n",
                        num(x),
                        num(y),
                        if i == 0 { "m" } else { "l" }
                    ));
                }
                ops.push_str("h\n");
                any = true;
            }
            if any {
                ops.push_str("W* n\n");
            }
            ops.push_str(&format!("/{name} Do Q\n"));
        }
        // Le linee comuni, sopra tutto: aperte, come le ha lasciate il calcolo.
        let lines: Vec<_> = sheet.lines.iter().filter(|l| l.len() >= 2).collect();
        let draw = match (ink, &stroke) {
            (Some(die), Some(st)) if !lines.is_empty() => Some((die, st)),
            _ => None,
        };
        if let Some((die, _)) = draw {
            let k = pt_to_mm(1.0);
            ops.push_str(&format!(
                "q /WFTaglio CS 1 SCN /WFTaglioGS gs {} w 1 j 1 J [] 0 d\n",
                num(mm_to_pt(die.width))
            ));
            for line in &lines {
                for (i, p) in line.iter().enumerate() {
                    let (x, y) = (p[0] / k, sh - p[1] / k);
                    if !(x.is_finite() && y.is_finite()) {
                        return Err(EditError::Invalid("linea di taglio non valida".into()));
                    }
                    ops.push_str(&format!(
                        "{} {} {}\n",
                        num(x),
                        num(y),
                        if i == 0 { "m" } else { "l" }
                    ));
                }
            }
            ops.push_str("S Q\n");
        }
        if unit > 1.0 {
            ops.push_str("Q\n");
        }
        let (pw, ph) = (sw / unit, sh / unit);
        let page = organize::insert_blank_page(&mut doc, n, pw, ph)?;
        let contents = doc.add(Object::Stream(Stream::flate(Dict::new(), ops.as_bytes())));
        doc.update_dict(page, |p| {
            let mut resources = Dict::new();
            if let Some((_, (cs, gs))) = draw {
                let mut spaces = Dict::new();
                spaces.set(b"WFTaglio".to_vec(), Object::Ref(*cs));
                resources.set(b"ColorSpace".to_vec(), Object::Dict(spaces));
                let mut states = Dict::new();
                states.set(b"WFTaglioGS".to_vec(), Object::Ref(*gs));
                resources.set(b"ExtGState".to_vec(), Object::Dict(states));
            }
            resources.set(b"XObject".to_vec(), Object::Dict(used));
            p.set(b"Resources".to_vec(), Object::Dict(resources));
            p.set(b"Contents".to_vec(), Object::Ref(contents));
            p.set(b"TrimBox".to_vec(), rect_object([0.0, 0.0, pw, ph]));
            if unit > 1.0 {
                p.set(b"UserUnit".to_vec(), Object::Real(unit));
            }
        })?;
    }
    // Le pagine intere che nessuna copia usa così com'è non restano nel file.
    for (_, r, _, used) in &bases {
        if !used {
            doc.free(*r);
        }
    }
    Ok(PdfFile(doc))
}

/// I tracciati della pagina che stanno tutti fuori dal ritaglio `clip` (mm, y in giù): quelli
/// che una copia ritagliata così non mostra. Si guarda la parte visibile di ognuno, allargata di
/// mezzo spessore se è un contorno; per i soli riempimenti, la forma vera. Ci sono anche quelli
/// che già sulla pagina il loro ritaglio nasconde del tutto. I tracciati che ritagliano e quelli
/// dentro i moduli restano sempre; i tracciati che ritagliano solo se cadono tutti fuori.
fn paths_outside(scan: &PageScan, clip: &[Vec<[f64; 2]>]) -> Vec<PathId> {
    // Sotto questa area (mm²) il tocco è solo un bordo in comune.
    const TOUCH_MM2: f64 = 1e-4;
    let Some(cb) = shape_bounds(clip) else {
        return Vec::new();
    };
    let clip: Contours = clip.to_vec();
    let hits = |subject: &Contours, rule: FillRule| -> bool {
        subject
            .overlay(&clip, OverlayRule::Intersect, rule)
            .iter()
            .flatten()
            .map(|c| paths::signed_area(c).abs())
            .sum::<f64>()
            > TOUCH_MM2
    };
    // Quelli che già sulla pagina non si vedono (tutti fuori dal loro ritaglio) non servono mai.
    let mut out = scan.unseen.clone();
    for p in &scan.paths {
        if p.clips || p.in_form {
            continue;
        }
        let half = p.line.as_ref().map_or(0.0, |l| l.width.max(0.0) / 2.0);
        let v = p.visible;
        let r = [v[0] - half, v[1] - half, v[2] + half, v[3] + half];
        if !r.iter().all(|x| x.is_finite()) {
            continue;
        }
        // Lontano dal ritaglio: fuori senza fare conti.
        let apart = r[2] < cb[0] || r[0] > cb[2] || r[3] < cb[1] || r[1] > cb[3];
        let inside = if apart {
            false
        } else {
            let rect: Contours = vec![vec![[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]]];
            // Un rettangolo vuoto (una linea dritta senza spessore) si tiene: non si sa misurare.
            let degenerate = r[2] - r[0] <= 0.0 || r[3] - r[1] <= 0.0;
            degenerate
                || (hits(&rect, FillRule::EvenOdd)
                    && (p.line.is_some() || {
                        let shape: Contours = p
                            .parts
                            .iter()
                            .filter(|q| q.points.len() >= 3)
                            .map(|q| q.points.clone())
                            .collect();
                        // Non-zero: copre almeno quanto il riempimento vero, qualunque regola usi.
                        shape.is_empty() || hits(&shape, FillRule::NonZero)
                    }))
        };
        if !inside {
            out.push(p.id);
        }
    }
    // I ritagli tutti fuori dalla copia: quello che ritagliano non si vede comunque, e il loro
    // tracciato (a volte di migliaia di punti) non serve.
    for m in scan.masks() {
        let v = m.visible;
        let apart = v[2] < cb[0] || v[0] > cb[2] || v[3] < cb[1] || v[1] > cb[3];
        if apart && !m.in_form {
            out.push(m.id);
        }
    }
    out
}
