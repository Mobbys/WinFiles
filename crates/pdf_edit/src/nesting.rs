//! Nesting (ADR 0027): le sagome da disporre lette dalle pagine, e il PDF dei fogli disposti per
//! il RIP. Ogni pagina di partenza entra una volta sola come modulo (form XObject) con le sue
//! risorse, poi si disegna in ogni copia spostata, girata e ritagliata alla sua sagoma più
//! l'abbondanza. Le tinte piatte (fustella, bianco, vernice) restano com'erano.

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
/// La pagina più lunga che un PDF tiene senza `UserUnit` (200 pollici).
pub const MAX_SHEET_MM: f64 = 5080.0;
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
    // Ogni pagina di partenza una volta sola: (sorgente, pagina, senza fustella) → (modulo,
    // altezza in punti).
    let mut forms: Vec<((usize, usize, bool), pdfcraft_cos::ObjRef, f64)> = Vec::new();
    // Le copie dei PDF da cui si è tolta la fustella di qualche pagina.
    let mut stripped: Vec<(usize, Document, Vec<usize>)> = Vec::new();
    for (n, sheet) in sheets.iter().enumerate() {
        let ok = |v: f64| v.is_finite() && (1.0..=MAX_SHEET_MM).contains(&v);
        if !ok(sheet.width) || !ok(sheet.height) {
            return Err(EditError::Invalid(format!(
                "il foglio {} è troppo grande per un PDF (al massimo {:.0} cm)",
                n + 1,
                MAX_SHEET_MM / 10.0
            )));
        }
        let (sw, sh) = (mm_to_pt(sheet.width), mm_to_pt(sheet.height));
        let mut ops = String::new();
        let mut used = Dict::new();
        for piece in &sheet.pieces {
            let key = (piece.source, piece.page, !piece.hide.is_empty());
            let (form, h) = match forms.iter().find(|(k, _, _)| *k == key) {
                Some((_, r, h)) => (*r, *h),
                None => {
                    if forms.len() >= MAX_FORMS {
                        return Err(EditError::Invalid("troppi file diversi in un foglio".into()));
                    }
                    let src = sources
                        .get(piece.source)
                        .ok_or_else(|| EditError::Invalid("file mancante".into()))?;
                    let (r, (_, h)) = if piece.hide.is_empty() {
                        organize::page_as_form(&mut doc, &src.0, piece.page)?
                    } else {
                        // La pagina senza la fustella, in una copia del PDF fatta una volta sola.
                        let at = match stripped.iter().position(|(s, _, _)| *s == piece.source) {
                            Some(i) => i,
                            None => {
                                stripped.push((piece.source, src.0.clone(), Vec::new()));
                                stripped.len() - 1
                            }
                        };
                        let (_, copy, done) = stripped
                            .get_mut(at)
                            .ok_or_else(|| EditError::Invalid("file mancante".into()))?;
                        if !done.contains(&piece.page) {
                            paths::hide(copy, piece.page, &piece.hide)?;
                            done.push(piece.page);
                        }
                        organize::page_as_form(&mut doc, copy, piece.page)?
                    };
                    forms.push((key, r, h));
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
        let page = organize::insert_blank_page(&mut doc, n, sw, sh)?;
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
            p.set(b"TrimBox".to_vec(), rect_object([0.0, 0.0, sw, sh]));
        })?;
    }
    Ok(PdfFile(doc))
}
