//! Modifica dei PDF (Fase 9, livello L2, ADR 0015): il modulo che sostituisce "Controllo PDF"
//! della 4.9. Le operazioni sulla struttura (spostare, ruotare, eliminare, aggiungere da un altro
//! PDF, estrarre, dividere, rifilare) sono quelle di PdfCraft, che riscrivono il file senza perdere
//! annotazioni, segnalibri, livelli e intenti di stampa. Margini e nuove misure ridisegnano la
//! pagina dentro una pagina nuova (`frame`), con i contenuti che restano vettoriali.
//!
//! Ogni modifica tiene una copia del documento prima di sé: [`Editor::undo`] e
//! [`Editor::redo`] costano poco perché il documento di PdfCraft si copia solo dove cambia.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pdfcraft_cos::{CosError, Document, SaveOptions};
use pdfcraft_organize::{self as organize, BoxSpec, OrganizeError, PageBox};
use winfiles_geom::{Paper, Size, pt_to_mm};

mod frame;
mod illustrator;
mod paths;
mod spots;

pub use frame::{BleedMode, BleedSpec};
pub use paths::{
    Fill, Ink, Line, NewDie, NewInk, PageScan, Part, PathId, PathInfo, Restyle, Restyled, SubPath,
    outline_is_rectangle,
};
pub use spots::{Overprint, Spot};

#[cfg(test)]
mod tests;

/// PDF più grande che si apre (il file si legge tutto in memoria).
const MAX_PDF_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Passi di Annulla tenuti in memoria.
const MAX_UNDO: usize = 100;

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("{}", io_text(.0))]
    Io(#[from] std::io::Error),
    #[error("il file è troppo grande ({0} byte)")]
    TooLarge(u64),
    #[error("il PDF è protetto da password")]
    Encrypted,
    #[error("PDF non leggibile: {0}")]
    Pdf(String),
    #[error("il PDF deve tenere almeno una pagina")]
    LastPage,
    #[error("la pagina {0} non esiste")]
    NoPage(usize),
    #[error("{0}")]
    Invalid(String),
}

fn io_text(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "file non trovato".into(),
        std::io::ErrorKind::PermissionDenied => "accesso al file negato".into(),
        _ => format!("impossibile leggere o scrivere il file: {e}"),
    }
}

impl From<CosError> for EditError {
    fn from(e: CosError) -> Self {
        match e {
            CosError::NeedsPassword | CosError::WrongPassword => EditError::Encrypted,
            other => EditError::Pdf(other.to_string()),
        }
    }
}

impl From<OrganizeError> for EditError {
    fn from(e: OrganizeError) -> Self {
        match e {
            OrganizeError::NoSuchPage(i) => EditError::NoPage(i.saturating_add(1)),
            OrganizeError::WouldRemoveAllPages => EditError::LastPage,
            OrganizeError::Cos(c) => c.into(),
            OrganizeError::NoPageTree => EditError::Pdf("manca l'elenco delle pagine".into()),
            OrganizeError::InvalidBox(m) | OrganizeError::Invalid(m) => EditError::Invalid(m),
        }
    }
}

/// Le misure di una pagina come la mostrano i lettori (rotazione applicata), in millimetri.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageInfo {
    /// Il formato visibile: il CropBox dentro il MediaBox.
    pub size: Size,
    /// Il formato finito (TrimBox), se diverso dal formato visibile.
    pub trim: Option<Size>,
    /// Rotazione della pagina in gradi, in senso orario.
    pub rotation: i64,
}

/// Un PDF aperto nel modulo PDF, con la storia delle modifiche.
pub struct Editor {
    doc: Document,
    path: PathBuf,
    undo: Vec<(String, Document)>,
    redo: Vec<(String, Document)>,
    /// Il documento cambiato dall'ultimo salvataggio.
    modified: bool,
}

/// Un PDF letto dal disco o costruito (unione, estrazione), non ancora in modifica.
#[derive(Clone)]
pub struct PdfFile(Document);

impl PdfFile {
    pub fn open(path: &Path) -> Result<Self, EditError> {
        let len = std::fs::metadata(path)?.len();
        if len > MAX_PDF_BYTES {
            return Err(EditError::TooLarge(len));
        }
        Self::from_bytes(std::fs::read(path)?)
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Self, EditError> {
        let doc = Document::open(Arc::new(data))?;
        // Un PDF senza pagine leggibili non si può modificare: meglio dirlo subito.
        if organize::page_count(&doc)? == 0 {
            return Err(EditError::Pdf("nessuna pagina".into()));
        }
        Ok(Self(doc))
    }

    pub fn page_count(&self) -> usize {
        organize::page_count(&self.0).unwrap_or(0)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, EditError> {
        save_bytes(&self.0)
    }

    pub fn save(&self, path: &Path) -> Result<(), EditError> {
        write_atomic(path, &self.to_bytes()?)
    }
}

/// Una copia del documento da scrivere in un altro thread, per l'anteprima.
pub struct Snapshot(Document);

impl Snapshot {
    /// I tracciati della pagina `page` (da 0), da leggere in un altro thread.
    pub fn scan_page(&self, page: usize) -> Result<PageScan, EditError> {
        paths::scan(&self.0, page)
    }

    /// Il PDF da disegnare: senza flussi di oggetti compressi, che qui farebbero solo perdere tempo.
    pub fn preview_bytes(&self) -> Result<Vec<u8>, EditError> {
        let opts = SaveOptions {
            object_streams: false,
            ..SaveOptions::default()
        };
        Ok(pdfcraft_cos::write_full(&self.0, &opts)?)
    }
}

impl Editor {
    pub fn open(path: &Path) -> Result<Self, EditError> {
        Ok(Self::new(PdfFile::open(path)?, path.to_path_buf()))
    }

    /// Mette in modifica `file`; `path` è il file da cui viene (per il nome e la cartella).
    pub fn new(file: PdfFile, path: PathBuf) -> Self {
        Self {
            doc: file.0,
            path,
            undo: Vec::new(),
            redo: Vec::new(),
            modified: false,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot(self.doc.clone())
    }

    /// Un'impronta per pagina: cambia quando cambia quello che la pagina mostra (contenuto,
    /// misure, rotazione), non quando la pagina si sposta. Per tenere le miniature già fatte.
    pub fn page_keys(&self) -> Vec<u64> {
        use std::hash::{Hash, Hasher};
        let Ok(pages) = organize::pages(&self.doc) else {
            return Vec::new();
        };
        pages
            .iter()
            .map(|p| {
                let mut bytes = Vec::new();
                pdfcraft_cos::serialize(&self.doc.get(p.obj), &mut bytes);
                // Gli attributi ereditati non sono nel dizionario: il numero dell'oggetto distingue
                // due pagine uguali a parte quelli.
                let mut h = std::collections::hash_map::DefaultHasher::new();
                bytes.hash(&mut h);
                p.obj.num.hash(&mut h);
                h.finish()
            })
            .collect()
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Un documento nuovo (unione di più PDF): va salvato anche senza altre modifiche.
    pub fn mark_modified(&mut self) {
        self.modified = true;
    }

    /// "Non salvare": le modifiche restano in memoria ma non si chiede più di salvarle.
    pub fn discard_changes(&mut self) {
        self.modified = false;
    }

    pub fn page_count(&self) -> usize {
        organize::page_count(&self.doc).unwrap_or(0)
    }

    /// Misure di tutte le pagine, nell'ordine.
    pub fn pages(&self) -> Vec<PageInfo> {
        let Ok(boxes) = organize::page_boxes(&self.doc) else {
            return Vec::new();
        };
        boxes
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let rotation = organize::page_rotation(&self.doc, i).unwrap_or(0);
                let turned = rotation == 90 || rotation == 270;
                let size_of = |r: [f64; 4]| {
                    let (w, h) = (pt_to_mm(r[2] - r[0]), pt_to_mm(r[3] - r[1]));
                    if turned { Size::new(h, w) } else { Size::new(w, h) }
                };
                let size = size_of(b[1]);
                let trim = Some(size_of(b[3]))
                    .filter(|t| (t.w - size.w).abs() > 0.01 || (t.h - size.h).abs() > 0.01);
                PageInfo { size, trim, rotation }
            })
            .collect()
    }

    /// Cosa annulla "Annulla", se c'è qualcosa da annullare.
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|(l, _)| l.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|(l, _)| l.as_str())
    }

    pub fn undo(&mut self) -> bool {
        let Some((label, doc)) = self.undo.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.doc, doc);
        self.redo.push((label, current));
        self.modified = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some((label, doc)) = self.redo.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.doc, doc);
        self.undo.push((label, current));
        self.modified = true;
        true
    }

    /// Applica una modifica a una copia: se fallisce il documento resta com'era.
    fn edit<T>(
        &mut self,
        label: &str,
        f: impl FnOnce(&mut Document) -> Result<T, EditError>,
    ) -> Result<T, EditError> {
        let mut next = self.doc.clone();
        let out = f(&mut next)?;
        let before = std::mem::replace(&mut self.doc, next);
        self.undo.push((label.to_string(), before));
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.modified = true;
        Ok(out)
    }

    /// Ruota le pagine di `quarters` quarti di giro (positivi in senso orario).
    pub fn rotate(&mut self, pages: &[usize], quarters: i64) -> Result<(), EditError> {
        self.edit("Ruota", |d| Ok(organize::rotate_pages(d, pages, quarters * 90)?))
    }

    pub fn delete(&mut self, pages: &[usize]) -> Result<(), EditError> {
        self.edit("Elimina pagine", |d| Ok(organize::delete_pages(d, pages)?))
    }

    /// Sposta le pagine (nel loro ordine) perché comincino alla posizione `to` del risultato.
    pub fn move_pages(&mut self, pages: &[usize], to: usize) -> Result<(), EditError> {
        self.edit("Sposta pagine", |d| Ok(organize::move_pages(d, pages, to)?))
    }

    pub fn duplicate(&mut self, pages: &[usize]) -> Result<(), EditError> {
        self.edit("Duplica pagine", |d| Ok(organize::duplicate_pages(d, pages)?))
    }

    /// Rifila al formato finito (TrimBox), come "Rifila" della 4.9: il CropBox diventa il
    /// TrimBox e l'abbondanza resta nel file, nascosta. Restituisce quante pagine sono cambiate.
    pub fn trim(&mut self, pages: &[usize]) -> Result<usize, EditError> {
        let boxes = organize::page_boxes(&self.doc)?;
        let todo: Vec<(usize, [f64; 4])> = pages
            .iter()
            .filter_map(|&i| {
                let b = boxes.get(i)?;
                let differs = b[3].iter().zip(b[1]).any(|(t, c)| (t - c).abs() > 0.01);
                differs.then_some((i, b[3]))
            })
            .collect();
        if todo.is_empty() {
            return Ok(0);
        }
        self.edit("Rifila al vivo", |d| {
            for (i, trim) in &todo {
                organize::set_page_box(d, &[*i], PageBox::Crop, BoxSpec::Rect(*trim))?;
            }
            Ok(todo.len())
        })
    }

    /// Aggiunge le pagine `src_pages` del PDF `src` alla posizione `at`.
    pub fn insert_from(&mut self, src: &PdfFile, src_pages: &[usize], at: usize) -> Result<(), EditError> {
        self.edit("Aggiungi pagine", |d| {
            organize::import_pages(d, &src.0, src_pages, at)?;
            Ok(())
        })
    }

    /// Margini attorno alle pagine, in millimetri, del colore scelto. Un margine negativo toglie
    /// dal bordo invece di aggiungere; la pagina deve restare larga e alta almeno 1 mm.
    pub fn add_margins(
        &mut self,
        pages: &[usize],
        margins: winfiles_geom::Margins,
        color: [u8; 3],
    ) -> Result<(), EditError> {
        let sides = [margins.top, margins.right, margins.bottom, margins.left];
        let ok = sides
            .iter()
            .all(|m| m.is_finite() && (-5000.0..=5000.0).contains(m));
        if !ok {
            return Err(EditError::Invalid("margini non validi".into()));
        }
        let label = if sides.iter().all(|m| *m <= 0.0) {
            "Togli dai bordi"
        } else {
            "Margini"
        };
        self.edit(label, |d| {
            for &i in pages {
                frame::margins(d, i, margins, color)?;
            }
            Ok(())
        })
    }

    /// La taglierina: di ogni pagina resta solo il riquadro `cut`, in millimetri dall'angolo in
    /// alto a sinistra della pagina come si vede. Il riquadro si ferma al bordo di ogni pagina; le
    /// pagine che non tocca restano come sono. Restituisce quante pagine sono state tagliate.
    pub fn crop(&mut self, pages: &[usize], cut: winfiles_geom::Rect) -> Result<usize, EditError> {
        let ok = [cut.x, cut.y, cut.w, cut.h].iter().all(|v| v.is_finite());
        if !ok || cut.w < 1.0 || cut.h < 1.0 {
            return Err(EditError::Invalid("riquadro da tagliare non valido".into()));
        }
        let infos = self.pages();
        let todo: Vec<(usize, winfiles_geom::Margins)> = pages
            .iter()
            .filter_map(|&i| {
                let size = infos.get(i)?.size;
                let (x0, y0) = (cut.x.max(0.0), cut.y.max(0.0));
                let (x1, y1) = (cut.right().min(size.w), cut.bottom().min(size.h));
                (x1 - x0 >= 1.0 && y1 - y0 >= 1.0).then_some((
                    i,
                    winfiles_geom::Margins {
                        top: -y0,
                        left: -x0,
                        right: -(size.w - x1),
                        bottom: -(size.h - y1),
                    },
                ))
            })
            .collect();
        if todo.is_empty() {
            return Ok(0);
        }
        self.edit("Taglia", |d| {
            for (i, m) in &todo {
                frame::margins(d, *i, *m, [255, 255, 255])?;
            }
            Ok(todo.len())
        })
    }

    /// Porta le pagine alla misura data in millimetri: il contenuto si ingrandisce o rimpicciolisce
    /// in proporzione e sta al centro.
    pub fn resize(&mut self, pages: &[usize], size: Size) -> Result<(), EditError> {
        check_size(size)?;
        self.edit("Ridimensiona pagine", |d| {
            for &i in pages {
                frame::fit(d, i, size, false)?;
            }
            Ok(())
        })
    }

    /// Adatta le pagine al formato carta, in verticale o in orizzontale come la pagina.
    pub fn fit_paper(&mut self, pages: &[usize], paper: Paper) -> Result<(), EditError> {
        self.edit(&format!("Adatta ad {}", paper.name()), |d| {
            for &i in pages {
                frame::fit(d, i, paper.portrait(), true)?;
            }
            Ok(())
        })
    }

    /// Le tinte piatte (Separation e DeviceN) del documento, con le pagine e la sovrastampa.
    pub fn spots(&self) -> Result<Vec<Spot>, EditError> {
        spots::list(&self.doc)
    }

    /// Rinomina una tinta piatta dovunque sia definita. Due tinte con lo stesso nome diventano una.
    pub fn rename_spot(&mut self, from: &[u8], to: &str) -> Result<(), EditError> {
        let to = to.trim();
        if to.is_empty() || to.len() > 127 || to.as_bytes() == from {
            return Err(EditError::Invalid("nome della tinta non valido".into()));
        }
        self.edit("Rinomina tinta", |d| {
            match spots::rename(d, from, to.as_bytes())? {
                0 => Err(EditError::Invalid("tinta non trovata".into())),
                _ => Ok(()),
            }
        })
    }

    /// Accende o spegne la sovrastampa degli oggetti dipinti con una tinta piatta. Restituisce
    /// quanti contenuti (pagine e moduli) sono cambiati: 0 se era già così.
    pub fn set_spot_overprint(&mut self, name: &[u8], on: bool) -> Result<usize, EditError> {
        let mut probe = self.doc.clone();
        if spots::set_overprint(&mut probe, name, on)? == 0 {
            return Ok(0);
        }
        let label = if on { "Sovrastampa" } else { "Togli sovrastampa" };
        self.edit(label, |d| spots::set_overprint(d, name, on))
    }

    /// I tracciati della pagina `page` (da 0), per lo strumento Seleziona tracciato.
    pub fn scan_page(&self, page: usize) -> Result<PageScan, EditError> {
        paths::scan(&self.doc, page)
    }

    /// Cambia colore, spessore o sovrastampa dei tracciati scelti della pagina `page`.
    pub fn restyle_paths(
        &mut self,
        page: usize,
        ids: &[PathId],
        change: &Restyle,
    ) -> Result<Restyled, EditError> {
        let mut probe = self.doc.clone();
        let probed = paths::restyle(&mut probe, page, ids, change)?;
        if probed.done == 0 {
            return Ok(probed);
        }
        let label = match (&change.ink, change.width, change.overprint) {
            (Some(NewInk::Spot { .. }), _, _) => "Tracciati in tinta",
            (Some(NewInk::Cmyk(_)), _, _) => "Colore dei tracciati",
            (None, Some(_), _) => "Spessore dei tracciati",
            _ => "Sovrastampa dei tracciati",
        };
        self.edit(label, |d| paths::restyle(d, page, ids, change))
    }

    /// Crea l'abbondanza che manca attorno al formato finito della pagina (vedi [`BleedSpec`]).
    /// I tracciati `die` (la fustella) restano fuori dalla grafica ribaltata o allungata; i fondi
    /// bianchi `paper` sotto tutta la grafica si tolgono (sulla carta non cambia nulla, e senza di
    /// loro nell'anteprima non resta un filo chiaro sul bordo del formato finito). Con `only` non
    /// vuoto si allungano soltanto quei tracciati (lo sfondo scelto) e tutto il resto della pagina
    /// resta com'è, sopra di loro.
    /// Restituisce quanti tracciati della fustella non si sono potuti tenere fuori (stanno dentro
    /// un modulo del PDF).
    pub fn add_bleed(
        &mut self,
        page: usize,
        spec: &BleedSpec,
        die: &[PathId],
        paper: &[PathId],
        only: &[PathId],
    ) -> Result<usize, EditError> {
        let ok = spec.amount.is_finite()
            && (0.1..=100.0).contains(&spec.amount)
            && spec.finished.iter().all(|v| v.is_finite())
            && spec.sides.iter().any(|s| *s);
        if !ok {
            return Err(EditError::Invalid("abbondanza non valida".into()));
        }
        self.edit("Crea abbondanza", |d| {
            let (base, art, over, left) = bleed_layers(d, page, die, paper, only)?;
            frame::bleed(d, page, spec, &base, &art, over.as_deref())?;
            Ok(left)
        })
    }

    /// Crea l'abbondanza che segue la forma dei tracciati `outline` (una fustella sagomata, o la
    /// grafica scelta): la pagina diventa il loro rettangolo più `amount` mm per lato, e lungo
    /// tutto il contorno l'ultimo mezzo millimetro di grafica si allunga verso fuori. `die`,
    /// `paper` e `only` come per [`Editor::add_bleed`].
    pub fn add_shaped_bleed(
        &mut self,
        page: usize,
        amount: f64,
        outline: &[PathId],
        die: &[PathId],
        paper: &[PathId],
        only: &[PathId],
    ) -> Result<usize, EditError> {
        if !(amount.is_finite() && (0.1..=100.0).contains(&amount)) || outline.is_empty() {
            return Err(EditError::Invalid("abbondanza non valida".into()));
        }
        self.edit("Crea abbondanza", |d| {
            let shapes = paths::outline(&paths::scan(d, page)?, outline)?;
            let (base, art, over, left) = bleed_layers(d, page, die, paper, only)?;
            frame::bleed_shape(d, page, &shapes, amount, &base, &art, over.as_deref())?;
            Ok(left)
        })
    }

    /// Crea una fustella in tinta piatta dal contorno dei tracciati `ids` (uniti se si
    /// sovrappongono), disegnata sopra la pagina; i tracciati restano com'erano. Restituisce
    /// quanti contorni ha la fustella.
    pub fn add_die(&mut self, page: usize, ids: &[PathId], die: &NewDie) -> Result<usize, EditError> {
        self.edit("Crea fustella", |d| paths::add_die(d, page, ids, die))
    }

    /// Il PDF porta la copia del file di Illustrator, quella che Illustrator apre al posto delle
    /// pagine (ADR 0020).
    pub fn has_illustrator_data(&self) -> bool {
        illustrator::has(&self.doc)
    }

    /// Toglie la copia di Illustrator: riaperto in Illustrator, il PDF mostra le pagine vere.
    pub fn strip_illustrator_data(&mut self) -> Result<usize, EditError> {
        if !self.has_illustrator_data() {
            return Ok(0);
        }
        self.edit("Togli i dati di Illustrator", illustrator::strip)
    }

    /// Il PDF intero, com'è adesso.
    pub fn to_bytes(&self) -> Result<Vec<u8>, EditError> {
        save_bytes(&self.doc)
    }

    /// Salva il PDF in `path`, che diventa il file del documento.
    pub fn save(&mut self, path: &Path) -> Result<(), EditError> {
        write_atomic(path, &self.to_bytes()?)?;
        self.path = path.to_path_buf();
        self.modified = false;
        Ok(())
    }

    /// Un nuovo PDF con le sole pagine scelte, nell'ordine dato.
    pub fn extract(&self, pages: &[usize]) -> Result<PdfFile, EditError> {
        Ok(PdfFile(organize::extract_pages(&self.doc, pages)?))
    }
}

/// Divide le pagine in gruppi: prima di ognuna delle pagine `before` (da 0) comincia un file.
/// I tre disegni dell'abbondanza: la pagina vera (`base`), la grafica da allungare (`art`) e
/// quello che resta sopra intero (`over`), più quanti tracciati della fustella non si sono potuti
/// tenere fuori dalla grafica. Con `only` si allungano solo quei tracciati e tutto il resto della
/// pagina sta sopra di loro; altrimenti si allunga tutto tranne fustella e fondi bianchi, e sopra
/// resta la fustella.
type Layers = (Vec<u8>, Vec<u8>, Option<Vec<u8>>, usize);

fn bleed_layers(
    d: &Document,
    page: usize,
    die: &[PathId],
    paper: &[PathId],
    only: &[PathId],
) -> Result<Layers, EditError> {
    if !only.is_empty() {
        let art = paths::only(d, page, only)?;
        let hidden: Vec<PathId> = only.iter().chain(paper).copied().collect();
        let (rest, _) = paths::without(d, page, &hidden)?;
        return Ok((art.clone(), art, Some(rest), 0));
    }
    let hidden: Vec<PathId> = die.iter().chain(paper).copied().collect();
    let (art, left) = paths::without(d, page, &hidden)?;
    let (all, _) = paths::without(d, page, paper)?;
    let over = if die.is_empty() {
        None
    } else {
        Some(paths::only(d, page, die)?)
    };
    Ok((all, art, over, left))
}

pub fn split_ranges(page_count: usize, before: &[usize]) -> Vec<std::ops::Range<usize>> {
    organize::split_ranges(page_count, &organize::SplitBy::Before(before.to_vec()))
}

/// Unisce più PDF in uno, nell'ordine, con un segnalibro per file (il nome).
pub fn combine(sources: &[(&str, &PdfFile)]) -> Result<PdfFile, EditError> {
    let docs: Vec<(&str, &Document)> = sources.iter().map(|(n, f)| (*n, &f.0)).collect();
    Ok(PdfFile(organize::combine(&docs)?))
}

fn check_size(size: Size) -> Result<(), EditError> {
    let ok = |v: f64| v.is_finite() && (1.0..=381_000.0).contains(&v);
    if ok(size.w) && ok(size.h) {
        Ok(())
    } else {
        Err(EditError::Invalid("misura non valida".into()))
    }
}

/// Il documento in un PDF completo, con la data di modifica di adesso.
fn save_bytes(doc: &Document) -> Result<Vec<u8>, EditError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(0))
        .unwrap_or(0);
    let opts = SaveOptions {
        mod_date: Some(pdfcraft_cos::pdf_date(now)),
        ..SaveOptions::default()
    };
    Ok(pdfcraft_cos::write_full(doc, &opts)?)
}

/// Scrive accanto e poi sostituisce: se qualcosa va storto il file di prima resta intero.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<(), EditError> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".winfiles-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, data)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}
