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

    /// Margini attorno alle pagine, in millimetri, del colore scelto.
    pub fn add_margins(
        &mut self,
        pages: &[usize],
        margins: winfiles_geom::Margins,
        color: [u8; 3],
    ) -> Result<(), EditError> {
        let ok = [margins.top, margins.right, margins.bottom, margins.left]
            .iter()
            .all(|m| m.is_finite() && (0.0..=5000.0).contains(m));
        if !ok {
            return Err(EditError::Invalid("margini non validi".into()));
        }
        self.edit("Aggiungi margini", |d| {
            for &i in pages {
                frame::margins(d, i, margins, color)?;
            }
            Ok(())
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
