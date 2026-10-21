//! Il modello del progetto (livello L1): l'elenco dei file con le loro pagine, le note per file e
//! per cartella e le impostazioni di stampa. Si salva come file `.winfiles` in JSON.
//!
//! Le note vivono qui e non nel risultato stampato: nella versione 4.9 stavano solo nella pagina
//! HTML generata e si perdevano alla chiusura.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use winfiles_geom::{Margins, Orientation, Paper, Size, Unit, area_m2};

/// Versione del formato del file di progetto. Si alza a ogni modifica incompatibile.
pub const FORMAT_VERSION: u32 = 1;
/// Estensione dei file di progetto.
pub const EXTENSION: &str = "winfiles";

/// Dimensione massima accettata per un file di progetto (64 MiB): oltre è sicuramente danneggiato.
const MAX_PROJECT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("impossibile leggere o scrivere {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("il file di progetto non è valido: {0}")]
    Json(#[from] serde_json::Error),
    #[error(
        "il progetto è stato salvato da una versione più recente (formato {found}, supportato fino a {supported})"
    )]
    TooNew { found: u32, supported: u32 },
    #[error("il file non contiene un progetto WinFiles")]
    NotAnObject,
    #[error("il file di progetto è troppo grande ({0} byte)")]
    TooLarge(u64),
}

/// Tipo di file riconosciuto.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileKind {
    Image,
    Pdf,
    Ai,
    Eps,
    Unsupported,
}

impl FileKind {
    /// Riconosce il tipo dall'estensione, senza leggere il file.
    pub fn from_path(path: &Path) -> Self {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "jpg" | "jpeg" | "png" | "tif" | "tiff" | "webp" | "bmp" | "gif" | "heic" | "heif" | "hif" => {
                FileKind::Image
            }
            // File di Photoshop e foto RAW delle fotocamere (le stesse estensioni di PhotoCraft).
            "psd" | "psb" | "dng" | "cr2" | "cr3" | "nef" | "nrw" | "arw" | "srf" | "sr2" | "pef" | "orf"
            | "rw2" | "raf" => FileKind::Image,
            "pdf" => FileKind::Pdf,
            "ai" => FileKind::Ai,
            "eps" => FileKind::Eps,
            _ => FileKind::Unsupported,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            FileKind::Image => "Immagine",
            FileKind::Pdf => "PDF",
            FileKind::Ai => "AI",
            FileKind::Eps => "EPS",
            FileKind::Unsupported => "Non supportato",
        }
    }
}

/// Una pagina di un file (le immagini hanno una sola pagina).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageInfo {
    /// Formato della pagina (MediaBox per i PDF, pixel/DPI per le immagini), in millimetri.
    pub size: Size,
    /// Formato al vivo (TrimBox), se diverso dal formato pagina.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim: Option<Size>,
}

impl PageInfo {
    /// Il formato finito: il TrimBox se c'è, altrimenti il formato pagina.
    pub fn finished(&self) -> Size {
        self.trim.unwrap_or(self.size)
    }
}

/// Risoluzione dichiarata da un'immagine (o presunta quando il file non la dice).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageInfo {
    pub width_px: u32,
    pub height_px: u32,
    /// Punti per pollice in orizzontale e in verticale.
    pub dpi_x: f64,
    pub dpi_y: f64,
    /// Falso se il file non dichiara la risoluzione e si usa quella predefinita (come la 4.9).
    pub dpi_from_file: bool,
}

/// L'ordine dei file appena aggiunti: per cartella, e nella cartella per nome, senza badare alle
/// maiuscole, come in Esplora file. I file di una cartella stanno tutti insieme, prima delle sue
/// sottocartelle: in ordine di percorso puro "IMG_1.JPG" andrebbe prima della sottocartella
/// "Stampa" e "foto.jpg" dopo, e la cartella comparirebbe due volte.
pub fn file_order(a: &Path, b: &Path) -> std::cmp::Ordering {
    file_order_key(a).cmp(&file_order_key(b))
}

/// La chiave di [`file_order`], da calcolare una volta per file con `sort_by_cached_key`.
pub fn file_order_key(p: &Path) -> (Vec<String>, String, PathBuf) {
    let folder = p
        .parent()
        .map(|f| {
            f.components()
                .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    (folder, name, p.to_path_buf())
}

/// Un file del progetto.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: PathBuf,
    pub kind: FileKind,
    /// Pagine con le misure lette dal file; vuoto finché il file non è stato letto.
    #[serde(default)]
    pub pages: Vec<PageInfo>,
    /// Pixel e risoluzione, solo per le immagini.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageInfo>,
    /// Note dell'utente per pagina, con l'indice della pagina (da 0) come chiave: ogni pagina di
    /// un PDF ha la sua, come ogni miniatura nella 4.9.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub notes: BTreeMap<usize, String>,
    /// Pagine (da 0) che restano nell'elenco ma non vanno in stampa: come le miniature senza
    /// spunta della 4.9. Un file senza misure ha comunque la sua pagina 0.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub skip: BTreeSet<usize>,
    /// Nota unica per file dei primi lavori salvati: all'apertura passa alla prima pagina.
    #[serde(default, rename = "note", skip_serializing)]
    legacy_note: String,
}

impl FileEntry {
    /// Un file appena aggiunto, ancora da misurare.
    pub fn new(path: PathBuf, kind: FileKind) -> Self {
        Self {
            path,
            kind,
            pages: Vec::new(),
            image: None,
            notes: BTreeMap::new(),
            skip: BTreeSet::new(),
            legacy_note: String::new(),
        }
    }

    /// Le pagine sul foglio: almeno una, anche per un file non ancora misurato.
    pub fn sheet_pages(&self) -> usize {
        self.pages.len().max(1)
    }

    /// Vero se la pagina va in stampa.
    pub fn printed(&self, page: usize) -> bool {
        !self.skip.contains(&page)
    }

    /// Mette o toglie una pagina dalla stampa.
    pub fn set_printed(&mut self, page: usize, on: bool) {
        if on {
            self.skip.remove(&page);
        } else if page < self.sheet_pages() {
            self.skip.insert(page);
        }
    }

    /// Mette o toglie dalla stampa tutte le pagine del file.
    pub fn set_all_printed(&mut self, on: bool) {
        self.skip.clear();
        if !on {
            self.skip.extend(0..self.sheet_pages());
        }
    }

    /// Le pagine che vanno in stampa, in ordine.
    pub fn printed_pages(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.sheet_pages()).filter(|p| self.printed(*p))
    }

    /// Quanto del file va in stampa.
    pub fn print_state(&self) -> PrintState {
        let n = self.printed_pages().count();
        if n == 0 {
            PrintState::None
        } else if n == self.sheet_pages() {
            PrintState::All
        } else {
            PrintState::Some
        }
    }

    /// La cartella che contiene il file (chiave per il raggruppamento).
    pub fn folder(&self) -> &Path {
        self.path.parent().unwrap_or_else(|| Path::new(""))
    }

    /// La nota di una pagina, vuota se non c'è.
    pub fn note(&self, page: usize) -> &str {
        self.notes.get(&page).map_or("", String::as_str)
    }

    /// Scrive la nota di una pagina; un testo vuoto la toglie, così il lavoro resta pulito.
    pub fn set_note(&mut self, page: usize, text: String) {
        if text.trim().is_empty() {
            self.notes.remove(&page);
        } else {
            self.notes.insert(page, text);
        }
    }

    /// Vero se almeno una pagina ha una nota.
    pub fn has_notes(&self) -> bool {
        self.notes.values().any(|n| !n.trim().is_empty())
    }

    /// Metri quadrati di tutte le pagine, al formato finito.
    pub fn area_m2(&self) -> f64 {
        self.pages.iter().map(|p| area_m2(p.finished())).sum()
    }

    /// Metri quadrati di tutte le pagine al formato pagina, cioè con abbondanza e crocini.
    pub fn area_full_m2(&self) -> f64 {
        self.pages.iter().map(|p| area_m2(p.size)).sum()
    }

    /// Le misure delle pagine in stampa.
    fn printed_info(&self) -> impl Iterator<Item = &PageInfo> + '_ {
        self.pages
            .iter()
            .enumerate()
            .filter(|(i, _)| self.printed(*i))
            .map(|(_, p)| p)
    }
}

/// Quanto di un file va in stampa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintState {
    All,
    Some,
    None,
}

/// Somme di un gruppo di file: numero di file, di pagine e metri quadrati.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Totals {
    pub files: usize,
    pub pages: usize,
    /// Metri quadrati al formato finito (al vivo).
    pub area_m2: f64,
    /// Metri quadrati al formato pagina, con abbondanza e crocini.
    pub area_full_m2: f64,
    /// File di cui non si conoscono ancora (o non si possono leggere) le misure. Quelli di tipo
    /// non supportato non contano: misure non ne avranno mai, sul foglio hanno una pagina.
    pub unmeasured: usize,
    /// Pagine lasciate fuori dalla stampa: non contano nei numeri qui sopra.
    pub excluded: usize,
}

impl Totals {
    /// Conta solo quello che va in stampa: un file tolto del tutto non è tra i file.
    fn add(&mut self, file: &FileEntry) {
        let printed = file.printed_pages().count();
        self.excluded += file.sheet_pages() - printed;
        if printed == 0 {
            return;
        }
        self.files += 1;
        if file.pages.is_empty() && file.kind != FileKind::Unsupported {
            self.unmeasured += 1;
        } else {
            self.pages += printed;
        }
        for p in file.printed_info() {
            self.area_m2 += area_m2(p.finished());
            self.area_full_m2 += area_m2(p.size);
        }
    }

    /// Somme di un insieme di file.
    pub fn of<'a>(files: impl IntoIterator<Item = &'a FileEntry>) -> Self {
        let mut t = Totals::default();
        for f in files {
            t.add(f);
        }
        t
    }

    /// Vero se le pagine hanno abbondanza: l'area al vivo è diversa da quella della pagina.
    pub fn has_bleed(&self) -> bool {
        (self.area_full_m2 - self.area_m2).abs() > 1e-6
    }
}

/// Come si dispongono le miniature sul foglio.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum View {
    /// Colonne di uguale larghezza, didascalia sotto ogni miniatura.
    #[default]
    Grid,
    /// Una miniatura per riga, con nome, misure e note a destra.
    List,
    /// Tutti i file alla stessa scala, accostati in righe centrate: si vedono le misure vere a
    /// confronto.
    Nesting,
}

/// Impostazioni di stampa del progetto.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrintSettings {
    pub paper: Paper,
    pub orientation: Orientation,
    /// Griglia, elenco o nesting.
    pub view: View,
    pub margins: Margins,
    /// Numero di colonne della griglia.
    pub columns: u32,
    /// Larghezza delle miniature nella vista a elenco, in mm.
    pub list_thumb: f64,
    /// Scala del nesting come 1:N; 0 = automatica (il file più grande largo un terzo del foglio).
    pub nesting_scale: u32,
    /// Spazio tra le colonne, in mm.
    pub gap: f64,
    /// Spazio tra le righe di miniature (e sotto titolo e intestazioni), in mm.
    pub row_gap: f64,
    /// Corpo del testo delle didascalie (nome del file e misure), in punti tipografici.
    pub caption_text: f64,
    /// Corpo del testo delle note di file e cartelle, in punti tipografici.
    pub note_text: f64,
    /// Corpo del testo delle intestazioni di cartella, in punti.
    pub header_text: f64,
    /// Corpo del titolo in cima ai fogli, in punti.
    pub title_text: f64,
    /// Altezza dell'intestazione di cartella, in mm.
    pub header: f64,
    /// Ogni cartella inizia su un foglio nuovo.
    pub page_break_per_folder: bool,
    /// Titolo scritto in cima a ogni foglio (per esempio il cliente o l'ordine); vuoto = nessuno.
    pub title: String,
    /// Mostra sul foglio le misure della pagina, con abbondanza e crocini.
    pub show_sizes: bool,
    /// Mostra sul foglio le misure al vivo (TrimBox), in rosso, quando sono diverse.
    pub show_trim: bool,
    /// Un foglio di riepilogo in cima: l'elenco di cartelle e file con le misure.
    pub show_summary: bool,
    /// Stampa le note di file, pagine e cartelle (spento: restano nel lavoro ma non sul foglio).
    pub show_notes: bool,
    /// Nella griglia, una sola miniatura in tutto il lavoro riempie il foglio (girato come il
    /// file) invece di stare in una colonna.
    pub fill_single: bool,
    /// Nella griglia, una miniatura per foglio, grande quanto il foglio e girata come il file.
    pub one_per_sheet: bool,
}

impl Default for PrintSettings {
    fn default() -> Self {
        Self {
            paper: Paper::A4,
            orientation: Orientation::Portrait,
            view: View::Grid,
            margins: Margins::uniform(10.0),
            columns: 4,
            list_thumb: 40.0,
            nesting_scale: 0,
            gap: 4.0,
            row_gap: 3.0,
            caption_text: 7.0,
            note_text: 8.0,
            header_text: 9.0,
            title_text: 14.0,
            header: 10.0,
            page_break_per_folder: false,
            title: String::new(),
            show_sizes: true,
            show_trim: true,
            show_summary: false,
            show_notes: true,
            fill_single: true,
            one_per_sheet: false,
        }
    }
}

/// Il progetto completo.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub version: u32,
    pub title: String,
    pub files: Vec<FileEntry>,
    /// Note per cartella, con il percorso della cartella come chiave.
    pub folder_notes: BTreeMap<PathBuf, String>,
    /// Intestazioni scelte dall'utente al posto del nome della cartella, per percorso.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub folder_titles: BTreeMap<PathBuf, String>,
    pub print: PrintSettings,
    /// Unità in cui mostrare le misure.
    pub unit: Unit,
}

impl Project {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            version: FORMAT_VERSION,
            title: title.into(),
            ..Self::default()
        }
    }

    /// La nota di una pagina come va sul foglio: vuota se le note sono nascoste.
    pub fn printed_note<'a>(&self, file: &'a FileEntry, page: usize) -> &'a str {
        if self.print.show_notes {
            file.note(page)
        } else {
            ""
        }
    }

    /// La nota di una cartella come va sul foglio: vuota se manca o se le note sono nascoste.
    pub fn printed_folder_note(&self, folder: &Path) -> &str {
        match self.folder_notes.get(folder) {
            Some(n) if self.print.show_notes => n,
            _ => "",
        }
    }

    /// Sposta i file `moving` (indici nell'elenco, nel loro ordine) subito prima del file
    /// `before`; un `before` oltre la fine li mette in fondo. Restituisce dove sono finiti.
    /// Gli indici fuori dall'elenco non contano.
    pub fn move_files(&mut self, moving: &BTreeSet<usize>, before: usize) -> std::ops::Range<usize> {
        let files = std::mem::take(&mut self.files);
        let mut moved = Vec::new();
        let mut rest = Vec::with_capacity(files.len());
        let mut at = None;
        for (i, f) in files.into_iter().enumerate() {
            if i == before {
                at = Some(rest.len());
            }
            if moving.contains(&i) {
                moved.push(f);
            } else {
                rest.push(f);
            }
        }
        let at = at.unwrap_or(rest.len());
        let range = at..at + moved.len();
        rest.splice(at..at, moved);
        self.files = rest;
        range
    }

    /// Somme per cartella, in ordine di cartella.
    pub fn folder_totals(&self) -> BTreeMap<&Path, Totals> {
        let mut out: BTreeMap<&Path, Totals> = BTreeMap::new();
        for f in &self.files {
            out.entry(f.folder()).or_default().add(f);
        }
        out
    }

    /// Nomi brevi delle cartelle per elenco e intestazioni: il percorso a partire dalla cartella
    /// comune a tutti i file, separato da " › " (per esempio `Cliente Rossi › Foto`).
    pub fn folder_labels(&self) -> BTreeMap<&Path, String> {
        let folders: Vec<&Path> = self.folder_totals().into_keys().collect();
        let mut common: Option<&Path> = folders.first().copied();
        for f in &folders {
            while let Some(c) = common {
                if f.starts_with(c) {
                    break;
                }
                common = c.parent();
            }
        }
        // Si parte dal genitore della cartella comune, così il suo nome resta visibile.
        let base = common.and_then(Path::parent);
        folders
            .into_iter()
            .map(|f| {
                let rel = base.and_then(|b| f.strip_prefix(b).ok()).unwrap_or(f);
                let parts: Vec<String> = rel.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                let label = if parts.is_empty() {
                    f.display().to_string()
                } else {
                    parts.join(" › ")
                };
                (f, label)
            })
            .collect()
    }

    /// Serializza in JSON leggibile.
    pub fn to_json(&self) -> Result<String, ModelError> {
        let mut copy = self.clone();
        copy.version = FORMAT_VERSION;
        Ok(serde_json::to_string_pretty(&copy)?)
    }

    /// Legge un progetto da JSON, rifiutando formati più recenti di quello supportato.
    pub fn from_json(text: &str) -> Result<Self, ModelError> {
        // Un progetto è sempre un oggetto JSON: con `serde(default)` anche `[]` diventerebbe un
        // progetto vuoto valido, e un file rovinato verrebbe aperto come se fosse vuoto.
        let value: serde_json::Value = serde_json::from_str(text)?;
        if !value.is_object() {
            return Err(ModelError::NotAnObject);
        }
        let mut project: Project = serde_json::from_value(value)?;
        for f in &mut project.files {
            let legacy = std::mem::take(&mut f.legacy_note);
            if !legacy.trim().is_empty() {
                f.notes.entry(0).or_insert(legacy);
            }
        }
        if project.version > FORMAT_VERSION {
            return Err(ModelError::TooNew {
                found: project.version,
                supported: FORMAT_VERSION,
            });
        }
        Ok(project)
    }

    /// Carica un progetto da disco.
    pub fn load(path: &Path) -> Result<Self, ModelError> {
        let io = |source| ModelError::Io {
            path: path.to_path_buf(),
            source,
        };
        let len = std::fs::metadata(path).map_err(io)?.len();
        if len > MAX_PROJECT_BYTES {
            return Err(ModelError::TooLarge(len));
        }
        let text = std::fs::read_to_string(path).map_err(io)?;
        Self::from_json(&text)
    }

    /// Salva su disco scrivendo prima un file temporaneo nella stessa cartella e poi
    /// sostituendo l'originale: un'interruzione non lascia mai un progetto a metà.
    pub fn save(&self, path: &Path) -> Result<(), ModelError> {
        let json = self.to_json()?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let io = |source| ModelError::Io {
            path: path.to_path_buf(),
            source,
        };
        std::fs::write(&tmp, json).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)
    }

    /// La nota di una cartella, vuota se non c'è.
    pub fn folder_note(&self, folder: &Path) -> &str {
        self.folder_notes.get(folder).map(String::as_str).unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Project {
        let mut p = Project::new("Catalogo");
        p.files.push(FileEntry {
            path: PathBuf::from("/lavori/cliente/logo.pdf"),
            kind: FileKind::Pdf,
            pages: vec![PageInfo {
                size: Size::new(210.0, 297.0),
                trim: Some(Size::new(200.0, 287.0)),
            }],
            image: None,
            notes: BTreeMap::from([(0, "Verificare il colore Pantone, è più scuro".into())]),
            skip: BTreeSet::new(),
            legacy_note: String::new(),
        });
        p.folder_notes
            .insert(PathBuf::from("/lavori/cliente"), "Consegna entro venerdì".into());
        p
    }

    #[test]
    fn i_file_di_una_cartella_stanno_insieme_prima_delle_sottocartelle() {
        let mut paths: Vec<PathBuf> = [
            "/l/foto.jpg",
            "/l/Stampa/a.pdf",
            "/l/IMG_1.JPG",
            "/l/b.docx",
            "/l2/x.png",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        paths.sort_by(|a, b| file_order(a, b));
        let got: Vec<_> = paths.iter().map(|p| p.display().to_string()).collect();
        assert_eq!(
            got,
            [
                "/l/b.docx",
                "/l/foto.jpg",
                "/l/IMG_1.JPG",
                "/l/Stampa/a.pdf",
                "/l2/x.png"
            ]
        );
    }

    #[test]
    fn le_note_nascoste_restano_nel_lavoro_ma_non_sul_foglio() {
        let mut p = sample();
        let folder = PathBuf::from("/lavori/cliente");
        assert_eq!(
            p.printed_note(&p.files[0], 0),
            "Verificare il colore Pantone, è più scuro"
        );
        assert_eq!(p.printed_folder_note(&folder), "Consegna entro venerdì");
        p.print.show_notes = false;
        assert_eq!(p.printed_note(&p.files[0], 0), "");
        assert_eq!(p.printed_folder_note(&folder), "");
        assert_eq!(p.files[0].note(0), "Verificare il colore Pantone, è più scuro");
        // Un lavoro salvato prima di questa opzione stampa le note come prima.
        let old: PrintSettings = serde_json::from_str("{}").unwrap();
        assert!(old.show_notes);
    }

    #[test]
    fn la_nota_del_file_dei_primi_lavori_passa_alla_prima_pagina() {
        let text = r#"{"version":1,"files":[{"path":"/a/b.pdf","kind":"Pdf","note":"Controllare"}]}"#;
        let p = Project::from_json(text).unwrap();
        assert_eq!(p.files[0].note(0), "Controllare");
        assert_eq!(p.files[0].note(1), "");
        // Si salva solo nel formato nuovo.
        let json = p.to_json().unwrap();
        assert!(json.contains("\"notes\"") && !json.contains("\"note\""));
        let mut f = p.files[0].clone();
        f.set_note(2, "Pagina tre".into());
        f.set_note(0, "  ".into());
        assert_eq!(f.notes.len(), 1);
        assert!(f.has_notes());
    }

    #[test]
    fn le_misure_tornano_identiche_dal_json() {
        // Le misure lette dai PDF hanno decimali qualsiasi: dopo salvataggio e riapertura devono
        // essere identiche, o un progetto appena aperto risulterebbe già modificato.
        let mut p = sample();
        p.files[0].pages[0].size = Size::new(215.900_000_000_000_03, 279.4 / 3.0);
        p.print.gap = 0.1 + 0.2;
        let back = Project::from_json(&p.to_json().unwrap()).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn andata_e_ritorno_json() {
        let p = sample();
        let back = Project::from_json(&p.to_json().unwrap()).unwrap();
        assert_eq!(back, p);
        assert_eq!(
            back.folder_note(Path::new("/lavori/cliente")),
            "Consegna entro venerdì"
        );
        assert_eq!(back.folder_note(Path::new("/altro")), "");
    }

    #[test]
    fn le_pagine_escluse_restano_nel_lavoro_ma_non_contano() {
        let mut p = sample();
        let a4 = PageInfo {
            size: Size::new(210.0, 297.0),
            trim: None,
        };
        let mut libro = FileEntry::new(PathBuf::from("/lavori/cliente/libro.pdf"), FileKind::Pdf);
        libro.pages = vec![a4; 4];
        p.files.push(libro);
        let tutto = Totals::of(&p.files);
        assert_eq!((tutto.files, tutto.pages, tutto.excluded), (2, 5, 0));

        // Due pagine del libro fuori dalla stampa: il file conta, le pagine no.
        p.files[1].set_printed(1, false);
        p.files[1].set_printed(3, false);
        assert_eq!(p.files[1].printed_pages().collect::<Vec<_>>(), [0, 2]);
        assert_eq!(p.files[1].print_state(), PrintState::Some);
        let t = Totals::of(&p.files);
        assert_eq!((t.files, t.pages, t.excluded), (2, 3, 2));
        assert!((t.area_full_m2 - tutto.area_full_m2 + 2.0 * 0.06237).abs() < 1e-9);
        // L'area del file resta quella di tutte le sue pagine.
        assert!((p.files[1].area_full_m2() - 4.0 * 0.06237).abs() < 1e-9);

        // Il logo tolto del tutto non è più tra i file della cartella.
        p.files[0].set_all_printed(false);
        assert_eq!(p.files[0].print_state(), PrintState::None);
        let t = p.folder_totals()[Path::new("/lavori/cliente")];
        assert_eq!((t.files, t.pages, t.excluded), (1, 2, 3));

        // Una pagina che il file non ha non si può escludere.
        p.files[0].set_printed(7, false);
        assert!(!p.files[0].skip.contains(&7));

        // Le esclusioni si salvano con il lavoro; senza esclusioni il campo non c'è.
        let back = Project::from_json(&p.to_json().unwrap()).unwrap();
        assert_eq!(back, p);
        p.files[0].set_all_printed(true);
        p.files[1].set_all_printed(true);
        assert!(!p.to_json().unwrap().contains("skip"));
    }

    #[test]
    fn un_file_senza_misure_si_puo_escludere() {
        let mut f = FileEntry::new(PathBuf::from("/x/a.eps"), FileKind::Eps);
        assert_eq!(f.sheet_pages(), 1);
        f.set_printed(0, false);
        let t = Totals::of([&f]);
        assert_eq!((t.files, t.unmeasured, t.excluded), (0, 0, 1));
        f.set_printed(0, true);
        let t = Totals::of([&f]);
        assert_eq!((t.files, t.unmeasured, t.excluded), (1, 1, 0));
    }

    #[test]
    fn un_file_non_supportato_e_una_pagina_senza_misure() {
        let f = FileEntry::new(PathBuf::from("/x/lettera.docx"), FileKind::Unsupported);
        let t = Totals::of([&f]);
        assert_eq!((t.files, t.pages, t.unmeasured), (1, 1, 0));
        assert_eq!(t.area_full_m2, 0.0);
    }

    #[test]
    fn spostare_i_file_tiene_il_loro_ordine() {
        let mut p = Project::new("x");
        for name in ["a", "b", "c", "d", "e"] {
            p.files.push(FileEntry::new(
                PathBuf::from(format!("/x/{name}.pdf")),
                FileKind::Pdf,
            ));
        }
        let names = |p: &Project| -> String {
            p.files
                .iter()
                .map(|f| f.path.file_stem().unwrap().to_string_lossy().into_owned())
                .collect()
        };
        // b e d prima di a.
        assert_eq!(p.move_files(&BTreeSet::from([1, 3]), 0), 0..2);
        assert_eq!(names(&p), "bdace");
        // In fondo.
        assert_eq!(p.move_files(&BTreeSet::from([0]), 99), 4..5);
        assert_eq!(names(&p), "daceb");
        // Prima di uno dei file che si spostano: restano dove sono.
        assert_eq!(p.move_files(&BTreeSet::from([1, 2]), 2), 1..3);
        assert_eq!(names(&p), "daceb");
        // Indici che non esistono: nessun danno.
        assert_eq!(p.move_files(&BTreeSet::from([42]), 1), 1..1);
        assert_eq!(names(&p), "daceb");
        assert_eq!(p.move_files(&BTreeSet::new(), 3), 3..3);
    }

    #[test]
    fn metri_quadri_al_formato_finito() {
        let mut p = sample();
        // Il PDF di prova ha un TrimBox 200 × 287: conta quello, non il MediaBox.
        assert!((p.files[0].area_m2() - 0.0574).abs() < 1e-9);
        // Con abbondanza e crocini conta il formato pagina.
        assert!((p.files[0].area_full_m2() - 0.06237).abs() < 1e-9);
        assert!(Totals::of(&p.files).has_bleed());
        let mut foto = FileEntry::new(PathBuf::from("/lavori/foto/a.jpg"), FileKind::Image);
        foto.pages.push(PageInfo {
            size: Size::new(1000.0, 500.0),
            trim: None,
        });
        p.files.push(foto);
        p.files.push(FileEntry::new(
            PathBuf::from("/lavori/foto/b.jpg"),
            FileKind::Image,
        ));
        let per_folder = p.folder_totals();
        let foto = per_folder[Path::new("/lavori/foto")];
        assert_eq!((foto.files, foto.pages, foto.unmeasured), (2, 1, 1));
        assert!((foto.area_m2 - 0.5).abs() < 1e-9);
        // Le immagini non hanno abbondanza: le due aree coincidono.
        assert!(!foto.has_bleed());
        let all = Totals::of(&p.files);
        assert_eq!(all.files, 3);
        assert!((all.area_m2 - 0.5574).abs() < 1e-9);
    }

    #[test]
    fn nomi_brevi_delle_cartelle() {
        let mut p = Project::new("x");
        for path in [
            "/lavori/corpus/Catalogo/a.pdf",
            "/lavori/corpus/Cliente Rossi/Foto/b.jpg",
            "/lavori/corpus/Cliente Rossi/c.jpg",
        ] {
            p.files.push(FileEntry::new(PathBuf::from(path), FileKind::Pdf));
        }
        let labels = p.folder_labels();
        assert_eq!(labels[Path::new("/lavori/corpus/Catalogo")], "corpus › Catalogo");
        assert_eq!(
            labels[Path::new("/lavori/corpus/Cliente Rossi/Foto")],
            "corpus › Cliente Rossi › Foto"
        );
        assert_eq!(
            labels[Path::new("/lavori/corpus/Cliente Rossi")],
            "corpus › Cliente Rossi"
        );
        let mut one = Project::new("y");
        one.files
            .push(FileEntry::new(PathBuf::from("/a/b/x.pdf"), FileKind::Pdf));
        assert_eq!(one.folder_labels()[Path::new("/a/b")], "b");
    }

    #[test]
    fn tipo_dall_estensione() {
        assert_eq!(FileKind::from_path(Path::new("a/B.JPG")), FileKind::Image);
        assert_eq!(FileKind::from_path(Path::new("x.ai")), FileKind::Ai);
        assert_eq!(
            FileKind::from_path(Path::new("senza_estensione")),
            FileKind::Unsupported
        );
    }

    #[test]
    fn rifiuta_formati_futuri() {
        let err = Project::from_json(r#"{"version": 999}"#).unwrap_err();
        assert!(matches!(err, ModelError::TooNew { found: 999, .. }));
    }

    #[test]
    fn campi_mancanti_prendono_i_valori_predefiniti() {
        let p = Project::from_json(r#"{"version": 1, "title": "x"}"#).unwrap();
        assert_eq!(p.print, PrintSettings::default());
        assert!(p.files.is_empty());
    }

    #[test]
    fn json_non_valido_non_va_in_panic() {
        for input in ["", "{", "[]", "null", r#"{"files": 3}"#, r#"{"version": -1}"#] {
            assert!(Project::from_json(input).is_err(), "{input}");
        }
    }

    #[test]
    fn salva_e_ricarica_da_disco() {
        let dir = std::env::temp_dir().join(format!("winfiles-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("progetto.winfiles");
        let p = sample();
        p.save(&path).unwrap();
        assert_eq!(Project::load(&path).unwrap(), p);
        assert!(!dir.join("progetto.winfiles.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
