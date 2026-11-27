//! Le note di una cartella salvate da sé in un file della cartella stessa, `note.winfiles`
//! (ADR 0038): note per pagina, nota di cartella, annotazioni di
//! Miniature e di Visualizza. Chi apre la stessa cartella in rete, da un altro PC, le ritrova.
//!
//! Il file è JSON: `{ versione, nota_cartella, file: { "<nome>": { pagine: { "<n>": { campo:
//! valore } } } } }`. I campi di ogni pagina sono liberi (`nota`, `annotazioni`,
//! `annotazioni_visualizza`, `scala`…): chi legge tiene anche quelli che non conosce, così una
//! versione più vecchia di WinFiles non cancella quello che ha scritto una più nuova.
//!
//! Due colleghi possono scrivere insieme: prima di salvare si rilegge il file e si uniscono le
//! modifiche cella per cella (file, pagina, campo) con [`merge`]. Vince l'ultimo solo se tutti e
//! due hanno cambiato la stessa cella.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use winfiles_geom::{Orientation, Paper};

use crate::{Annotation, MAX_PER_PAGE, ModelError};

/// Il nome del file delle note in ogni cartella. Ha l'estensione dei lavori, già associata a
/// WinFiles: il doppio clic apre i file della cartella che hanno note.
pub const FILE_NAME: &str = "note.winfiles";
/// Il nome delle prime prove (MSI di prova del 2026-11-16): si rinomina alla prima lettura.
const OLD_FILE_NAME: &str = "_Note WinFiles.winfiles-note";
/// La versione del formato scritta nel file.
pub const NOTES_FORMAT: u32 = 1;

/// Campo della nota scritta di una pagina.
pub const NOTE: &str = "nota";
/// Campo delle annotazioni di una pagina, le stesse in Miniature e in Visualizza (dalla
/// 0.2.43, ADR 0040): spessori e corpi in proporzione alla pagina.
pub const PAGE_ANNOTATIONS: &str = "annotazioni_pagina";
/// Campo delle annotazioni di Miniature della 0.2.41 e 0.2.42, in mm della carta alla grandezza
/// della miniatura: Miniature le porta in [`PAGE_ANNOTATIONS`] e lo svuota.
pub const ANNOTATIONS: &str = "annotazioni";
/// Campo delle annotazioni di Visualizza della 0.2.41 e 0.2.42, già in proporzione alla
/// pagina: passano in [`PAGE_ANNOTATIONS`] e il campo si svuota.
pub const VIEWER_ANNOTATIONS: &str = "annotazioni_visualizza";

/// Campo della scala di una pagina: il numero N di 1:N.
pub const SCALE: &str = "scala";

/// Tetto alla dimensione del file letto.
const MAX_BYTES: u64 = 32 << 20;
/// File al più in una cartella: oltre il file è sicuramente rovinato.
const MAX_FILES: usize = 50_000;
/// Lavori ricordati al più in un file delle note.
pub const MAX_JOBS: usize = 30;
/// Pagine al più per file.
const MAX_PAGES: usize = 100_000;
/// Lettere al più di una nota.
const MAX_NOTE_CHARS: usize = 100_000;

/// Il contenuto del file delle note di una cartella.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FolderNotes {
    #[serde(default)]
    pub versione: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub nota_cartella: String,
    #[serde(default)]
    pub file: BTreeMap<String, FileNotes>,
    /// I lavori di Miniature in cui sono state scritte le note ([`NotesJob`]), dal più recente:
    /// aprendo il file delle note se ne riapre uno com'era, sugli stessi fogli. Tenuti come valore
    /// grezzo: un lavoro rovinato non deve far perdere le note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lavori: Option<Value>,
    /// Campi scritti da versioni più nuove: si rimettono come sono.
    #[serde(flatten)]
    pub altro: BTreeMap<String, Value>,
}

/// Il lavoro di Miniature quando si sono scritte le note: i file sui fogli, nel loro ordine, e
/// foglio, verso e colonne davvero usati (anche quelli scelti da "Su N fogli").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NotesJob {
    /// I file, col percorso relativo alla cartella delle note (`a.pdf`, `sotto/b.pdf`,
    /// `../altra/c.pdf`): così vale anche da un PC che vede la cartella con un altro percorso.
    #[serde(default)]
    pub file: Vec<String>,
    /// Quando è stato scritto, in secondi dal 1970 (per scegliere fra più lavori).
    #[serde(default)]
    pub data: u64,
    pub foglio: Paper,
    pub verso: Orientation,
    pub colonne: u32,
    /// "1 per foglio".
    #[serde(default)]
    pub una_per_foglio: bool,
    /// L'unico file del lavoro riempiva il foglio ("1 file a tutta pagina").
    #[serde(default)]
    pub tutta_pagina: bool,
}

/// Le note di un file, per pagina (da 0).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FileNotes {
    #[serde(default)]
    pub pagine: BTreeMap<usize, BTreeMap<String, Value>>,
    #[serde(flatten)]
    pub altro: BTreeMap<String, Value>,
}

/// Una cella del file: la nota di cartella o un campo di una pagina di un file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    Folder,
    Cell {
        file: String,
        page: usize,
        field: String,
    },
}

impl Key {
    pub fn cell(file: &str, page: usize, field: &str) -> Self {
        Key::Cell {
            file: file.to_owned(),
            page,
            field: field.to_owned(),
        }
    }

    /// Il file della cella; `None` per la nota di cartella.
    pub fn file(&self) -> Option<&str> {
        match self {
            Key::Folder => None,
            Key::Cell { file, .. } => Some(file),
        }
    }
}

/// Tutte le celle, una per voce: comodo da confrontare e da unire.
pub type Cells = BTreeMap<Key, Value>;

/// Il percorso del file delle note della cartella `dir`.
pub fn path_in(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

/// Vero per un file delle note (si aprono i file annotati della sua cartella invece di
/// aprirlo come un lavoro).
pub fn is_notes_file(path: &Path) -> bool {
    path.file_name().is_some_and(|n| {
        let n = n.to_string_lossy();
        n.eq_ignore_ascii_case(FILE_NAME) || n.eq_ignore_ascii_case(OLD_FILE_NAME)
    })
}

impl FolderNotes {
    /// Legge il file delle note di `dir`; `Ok(None)` se non c'è.
    pub fn read(dir: &Path) -> Result<Option<Self>, ModelError> {
        let path = path_in(dir);
        let old = dir.join(OLD_FILE_NAME);
        if !path.exists() && old.is_file() {
            // Il nome delle prime prove: si passa a quello nuovo; se non si può, si legge com'è.
            if std::fs::rename(&old, &path).is_err() {
                return Self::read_file(&old);
            }
        }
        Self::read_file(&path)
    }

    /// I file della cartella `dir` con qualche nota o annotazione che ci sono ancora, più quelli
    /// a cui arriva una freccia (senza, resterebbe appesa).
    pub fn annotated_files(&self, dir: &Path) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
        let mut push = |p: PathBuf| {
            if out.len() < MAX_FILES && !seen.contains(&p) && p.is_file() {
                seen.insert(p.clone());
                out.push(p);
            }
        };
        let mut targets: Vec<String> = Vec::new();
        for (name, f) in self.noted() {
            if let Some(p) = name_in(dir, name) {
                push(p);
            }
            for v in f.pagine.values().flat_map(BTreeMap::values) {
                // I capi attaccati a un file della cartella sono scritti col solo nome.
                for a in annotations_from(v, Path::new("")) {
                    if let Annotation::Linea { end: Some(e), .. } = a
                        && let Some(end) = e.path.to_str()
                        && !targets.iter().any(|t| t == end)
                    {
                        targets.push(end.to_owned());
                    }
                }
            }
        }
        for t in &targets {
            if let Some(p) = name_in(dir, t) {
                push(p);
            }
        }
        out
    }

    /// I file (per nome) con qualche nota o annotazione loro.
    fn noted(&self) -> impl Iterator<Item = (&String, &FileNotes)> {
        self.file
            .iter()
            .filter(|(_, f)| f.pagine.values().any(|p| p.values().any(|v| !v.is_null())))
    }

    /// I file della cartella `dir` con qualche nota o annotazione loro (senza quelli a cui arriva
    /// solo una freccia).
    pub fn noted_files(&self, dir: &Path) -> BTreeSet<PathBuf> {
        self.noted().filter_map(|(name, _)| name_in(dir, name)).collect()
    }

    /// I lavori in cui sono state scritte le note, dal più recente; quelli che non si leggono
    /// restano fuori.
    pub fn jobs(&self) -> Vec<NotesJob> {
        let Some(Value::Array(items)) = &self.lavori else {
            return Vec::new();
        };
        items
            .iter()
            .take(MAX_JOBS)
            .filter_map(|v| serde_json::from_value::<NotesJob>(v.clone()).ok())
            .filter(|j| j.colonne >= 1 && !j.file.is_empty())
            .map(|mut j| {
                j.file.truncate(MAX_FILES);
                j
            })
            .collect()
    }

    /// Mette `job` in cima ai lavori. Prende il posto di quelli con gli stessi file o con una
    /// parte dei suoi: è lo stesso lavoro riaperto e cambiato. Se invece un lavoro già scritto ha
    /// tutti i suoi file e altri ancora, resta quello (in cima, con la data nuova): chi ha aperto
    /// solo una parte dei file e ci scrive una nota non deve far sparire gli altri dal lavoro.
    /// I lavori con altri file restano (due impaginati diversi della stessa cartella); al più
    /// [`MAX_JOBS`], si dimenticano i più vecchi.
    pub fn put_job(&mut self, job: NotesJob) {
        let mine: BTreeSet<&String> = job.file.iter().collect();
        let raw: Vec<Value> = match &self.lavori {
            Some(Value::Array(items)) => items.clone(),
            _ => Vec::new(),
        };
        let bigger = raw.iter().enumerate().find_map(|(i, v)| {
            let other = serde_json::from_value::<NotesJob>(v.clone()).ok()?;
            let theirs: BTreeSet<&String> = other.file.iter().collect();
            (mine.is_subset(&theirs) && theirs.len() > mine.len()).then_some((i, other))
        });
        if let Some((i, mut other)) = bigger {
            other.data = job.data;
            let mut out = raw;
            if i < out.len() {
                out.remove(i);
            }
            if let Ok(v) = serde_json::to_value(&other) {
                out.insert(0, v);
            }
            self.lavori = Some(Value::Array(out));
            return;
        }
        let mut out: Vec<Value> = Vec::with_capacity(raw.len() + 1);
        if let Ok(v) = serde_json::to_value(&job) {
            out.push(v);
        }
        for v in raw {
            // Uno che non si legge (scritto da una versione più nuova) si tiene com'è.
            let same = serde_json::from_value::<NotesJob>(v.clone()).is_ok_and(|other| {
                let theirs: BTreeSet<&String> = other.file.iter().collect();
                theirs.is_subset(&mine) || mine.is_subset(&theirs)
            });
            if !same && out.len() < MAX_JOBS {
                out.push(v);
            }
        }
        self.lavori = Some(Value::Array(out));
    }

    /// Legge un file delle note; `Ok(None)` se non c'è.
    pub fn read_file(path: &Path) -> Result<Option<Self>, ModelError> {
        let io = |source| ModelError::Io {
            path: path.to_path_buf(),
            source,
        };
        let len = match std::fs::metadata(path) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io(e)),
        };
        if len > MAX_BYTES {
            return Err(ModelError::TooLarge(len));
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io(e)),
        };
        Self::from_json(&text).map(Some)
    }

    /// Legge il JSON di un file delle note.
    pub fn from_json(text: &str) -> Result<Self, ModelError> {
        let value: Value = serde_json::from_str(text)?;
        if !value.is_object() {
            return Err(ModelError::NotAnObject);
        }
        let mut notes: FolderNotes = serde_json::from_value(value)?;
        if notes.file.len() > MAX_FILES {
            let extra: Vec<String> = notes.file.keys().skip(MAX_FILES).cloned().collect();
            for k in extra {
                notes.file.remove(&k);
            }
        }
        for f in notes.file.values_mut() {
            while f.pagine.len() > MAX_PAGES {
                f.pagine.pop_last();
            }
        }
        Ok(notes)
    }

    /// Vero se non resta niente da salvare.
    pub fn is_empty(&self) -> bool {
        self.nota_cartella.trim().is_empty()
            && self.altro.is_empty()
            && self
                .file
                .values()
                .all(|f| f.altro.is_empty() && f.pagine.values().all(BTreeMap::is_empty))
    }

    /// Le celle del file.
    pub fn cells(&self) -> Cells {
        let mut out = Cells::new();
        if !self.nota_cartella.trim().is_empty() {
            out.insert(Key::Folder, Value::String(self.nota_cartella.clone()));
        }
        for (name, f) in &self.file {
            for (page, fields) in &f.pagine {
                for (field, v) in fields {
                    if !v.is_null() {
                        out.insert(Key::cell(name, *page, field), v.clone());
                    }
                }
            }
        }
        out
    }

    /// Lo stesso file con le celle `cells` al posto delle sue; i campi sconosciuti restano.
    pub fn with_cells(&self, cells: &Cells) -> Self {
        let mut out = FolderNotes {
            versione: NOTES_FORMAT.max(self.versione),
            nota_cartella: String::new(),
            file: BTreeMap::new(),
            lavori: self.lavori.clone(),
            altro: self.altro.clone(),
        };
        for (name, f) in &self.file {
            if !f.altro.is_empty() {
                out.file.insert(
                    name.clone(),
                    FileNotes {
                        pagine: BTreeMap::new(),
                        altro: f.altro.clone(),
                    },
                );
            }
        }
        for (k, v) in cells {
            match k {
                Key::Folder => {
                    if let Value::String(s) = v {
                        out.nota_cartella = s.clone();
                    }
                }
                Key::Cell { file, page, field } => {
                    out.file
                        .entry(file.clone())
                        .or_default()
                        .pagine
                        .entry(*page)
                        .or_default()
                        .insert(field.clone(), v.clone());
                }
            }
        }
        out
    }

    /// Scrive il file delle note di `dir`, prima in un file a parte e poi al suo posto; vuoto, lo
    /// toglie, così nelle cartelle senza note non resta niente.
    pub fn write(&self, dir: &Path) -> Result<(), ModelError> {
        self.write_file(&path_in(dir))
    }

    /// Scrive un file delle note in `path` (vedi [`FolderNotes::write`]).
    pub fn write_file(&self, path: &Path) -> Result<(), ModelError> {
        let io = |source| ModelError::Io {
            path: path.to_path_buf(),
            source,
        };
        if self.is_empty() {
            return match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io(e)),
                _ => Ok(()),
            };
        }
        // Compatto: i tratti dell'evidenziatore hanno migliaia di punti.
        let json = serde_json::to_string(self)?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(format!(".{}.tmp", std::process::id()));
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, json).map_err(io)?;
        if let Err(e) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(io(e));
        }
        Ok(())
    }
}

/// Unisce le celle: quelle che questo WinFiles possiede (`owned`) e ha cambiato da `base` (com'era
/// il file all'ultima lettura) vengono da `mine`, tutte le altre da `disk` (il file riletto
/// adesso). Così le modifiche di un collega su altre celle non si perdono.
pub fn merge(base: &Cells, mine: &Cells, disk: &Cells, owned: impl Fn(&Key) -> bool) -> Cells {
    let keys: BTreeSet<&Key> = base.keys().chain(mine.keys()).chain(disk.keys()).collect();
    let mut out = Cells::new();
    for k in keys {
        let pick = if owned(k) && mine.get(k) != base.get(k) {
            mine.get(k)
        } else {
            disk.get(k)
        };
        if let Some(v) = pick {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

/// La base della prima lettura di una cartella: per i file che nel file delle note hanno già
/// qualcosa (e per la nota di cartella, se c'è) valgono quelle del file, per gli altri quelle che
/// WinFiles ha già (da un `.winfiles` aperto o dalle note di Visualizza di prima), che così
/// finiscono nel file.
pub fn first_base(mine: &Cells, disk: &Cells) -> Cells {
    let on_disk: BTreeSet<Option<&str>> = disk.keys().map(Key::file).collect();
    mine.iter()
        .filter(|(k, _)| on_disk.contains(&k.file()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Il valore di una nota scritta; `None` se vuota.
pub fn note_value(text: &str) -> Option<Value> {
    (!text.trim().is_empty()).then(|| Value::String(text.to_owned()))
}

/// La nota letta da una cella; vuota se la cella non è un testo.
pub fn note_from(v: &Value) -> String {
    match v {
        Value::String(s) => s.chars().take(MAX_NOTE_CHARS).collect(),
        _ => String::new(),
    }
}

/// Il valore della scala 1:N di una pagina; `None` per 1:1.
pub fn scale_value(scale: u32) -> Option<Value> {
    (scale > 1).then(|| Value::from(scale))
}

/// La scala letta da una cella: 1 se la cella non è un numero intero sensato.
pub fn scale_from(v: &Value) -> u32 {
    v.as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(1)
}

/// Il valore delle annotazioni di una pagina: i capi attaccati a un file della stessa cartella
/// si scrivono col solo nome, così valgono anche da un PC che vede la cartella con un altro
/// percorso. `None` se non ce ne sono.
pub fn annotations_value(list: &[Annotation], dir: &Path) -> Option<Value> {
    if list.is_empty() {
        return None;
    }
    let list: Vec<Annotation> = list
        .iter()
        .map(|a| {
            let mut a = a.clone();
            if let Annotation::Linea { end: Some(e), .. } = &mut a
                && e.path.parent() == Some(dir)
                && let Some(name) = e.path.file_name()
            {
                e.path = PathBuf::from(name);
            }
            a
        })
        .collect();
    serde_json::to_value(list).ok()
}

/// Le annotazioni lette da una cella, ripulite come quelle di un `.winfiles`.
pub fn annotations_from(v: &Value, dir: &Path) -> Vec<Annotation> {
    let mut list: Vec<Annotation> = match v {
        Value::Array(items) => items
            .iter()
            .take(MAX_PER_PAGE)
            .filter_map(|i| serde_json::from_value(i.clone()).ok())
            .collect(),
        _ => Vec::new(),
    };
    list.retain_mut(Annotation::sanitize);
    for a in &mut list {
        if let Annotation::Linea { end: Some(e), .. } = a
            && e.path.is_relative()
        {
            e.path = dir.join(&e.path);
        }
    }
    list
}

impl NotesJob {
    /// I file del lavoro, nel loro ordine, letti dalla cartella delle note `dir`.
    pub fn paths(&self, dir: &Path) -> Vec<PathBuf> {
        self.file
            .iter()
            .take(MAX_FILES)
            .filter_map(|f| job_path(dir, f))
            .collect()
    }

    /// Lo stesso lavoro con i file `files`, scritti relativi alla cartella delle note `dir`
    /// (quelli su un'altra unità restano fuori).
    pub fn with_files(&self, dir: &Path, files: &[PathBuf]) -> Self {
        Self {
            file: files
                .iter()
                .take(MAX_FILES)
                .filter_map(|f| relative_to(dir, f))
                .collect(),
            ..self.clone()
        }
    }
}

/// Un file della cartella `dir` scritto col solo nome; `None` se è un percorso (il file delle
/// note lo scrivono altri).
fn name_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    (p.file_name() == Some(p.as_os_str())).then(|| dir.join(p))
}

/// Un file del lavoro, scritto relativo alla cartella delle note; `None` se è un percorso
/// assoluto o di un'altra unità.
fn job_path(dir: &Path, rel: &str) -> Option<PathBuf> {
    use std::path::Component;
    // Scritto con "/" anche da Windows: si rimette il separatore di qui.
    let p: PathBuf = rel.split(['/', '\\']).filter(|c| !c.is_empty()).collect();
    // Assoluto (anche scritto con "\\" o con l'unità): non è del lavoro.
    let ok = !rel.is_empty()
        && !rel.starts_with(['/', '\\'])
        && !rel.contains(':')
        && p.components()
            .all(|c| matches!(c, Component::Normal(_) | Component::ParentDir));
    ok.then(|| dir.join(p))
}

/// Il percorso di `file` relativo alla cartella `dir`, con "/" come separatore, per
/// [`NotesJob::file`]; `None` se è su un'altra unità.
pub fn relative_to(dir: &Path, file: &Path) -> Option<String> {
    use std::path::Component;
    let d: Vec<Component> = dir.components().collect();
    let f: Vec<Component> = file.components().collect();
    let common = d.iter().zip(&f).take_while(|(a, b)| a == b).count();
    // Niente in comune (altra unità, altro server): non si scrive.
    if common == 0 {
        return None;
    }
    let mut parts: Vec<String> = vec!["..".to_owned(); d.len() - common];
    for c in f.iter().skip(common) {
        match c {
            Component::Normal(n) => parts.push(n.to_string_lossy().into_owned()),
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Il nome con cui un file sta nel file delle note della sua cartella.
pub fn file_key(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Anchor, Rgba};

    fn s(t: &str) -> Value {
        Value::String(t.into())
    }

    fn cells(list: &[(Key, &str)]) -> Cells {
        list.iter().map(|(k, v)| (k.clone(), s(v))).collect()
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("winfiles-note-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn due_colleghi_su_pagine_diverse_non_si_cancellano() {
        let a = Key::cell("a.pdf", 0, NOTE);
        let b = Key::cell("b.pdf", 0, NOTE);
        let base = cells(&[(a.clone(), "vecchia")]);
        let mine = cells(&[(a.clone(), "mia")]);
        let disk = cells(&[(a.clone(), "vecchia"), (b.clone(), "del collega")]);
        let out = merge(&base, &mine, &disk, |_| true);
        assert_eq!(out, cells(&[(a, "mia"), (b, "del collega")]));
    }

    #[test]
    fn una_cella_non_toccata_prende_quella_del_collega() {
        let a = Key::cell("a.pdf", 2, ANNOTATIONS);
        let base = cells(&[(a.clone(), "x")]);
        let disk = cells(&[(a.clone(), "y")]);
        assert_eq!(merge(&base, &base, &disk, |_| true), disk);
    }

    #[test]
    fn togliere_una_nota_la_toglie_anche_dal_file() {
        let a = Key::cell("a.pdf", 0, NOTE);
        let base = cells(&[(a.clone(), "x")]);
        let out = merge(&base, &Cells::new(), &base, |_| true);
        assert!(out.is_empty());
    }

    #[test]
    fn le_celle_degli_altri_non_si_toccano() {
        // Visualizza non possiede le note di Miniature: anche se non le ha, restano.
        let a = Key::cell("a.pdf", 0, NOTE);
        let v = Key::cell("a.pdf", 0, VIEWER_ANNOTATIONS);
        let disk = cells(&[(a.clone(), "nota")]);
        let mine = cells(&[(v.clone(), "frecce")]);
        let out = merge(
            &Cells::new(),
            &mine,
            &disk,
            |k| matches!(k, Key::Cell { field, .. } if field == VIEWER_ANNOTATIONS),
        );
        assert_eq!(out, cells(&[(a, "nota"), (v, "frecce")]));
    }

    #[test]
    fn alla_prima_lettura_vince_il_file_della_cartella() {
        let a = Key::cell("a.pdf", 0, NOTE);
        let b = Key::cell("b.pdf", 0, NOTE);
        // Il .winfiles ha note vecchie per a e una nota per b, che nel file non c'è.
        let mine = cells(&[(a.clone(), "vecchia"), (b.clone(), "solo nel lavoro")]);
        let disk = cells(&[(a.clone(), "nuova del collega")]);
        let base = first_base(&mine, &disk);
        let out = merge(&base, &mine, &disk, |_| true);
        assert_eq!(out, cells(&[(a, "nuova del collega"), (b, "solo nel lavoro")]));
    }

    #[test]
    fn si_scrive_si_rilegge_e_i_campi_sconosciuti_restano() {
        let dir = temp("rilegge");
        let text = r#"{"versione":1,"nota_cartella":"Cliente Rossi","futuro":7,
            "file":{"a.pdf":{"pagine":{"0":{"nota":"ciao","scala":"1:10"}}}}}"#;
        let notes = FolderNotes::from_json(text).unwrap();
        let mut c = notes.cells();
        assert_eq!(c.get(&Key::cell("a.pdf", 0, "scala")), Some(&s("1:10")));
        c.insert(Key::cell("b.pdf", 3, NOTE), s("altra"));
        notes.with_cells(&c).write(&dir).unwrap();
        let again = FolderNotes::read(&dir).unwrap().unwrap();
        assert_eq!(again.cells(), c);
        assert_eq!(again.altro.get("futuro"), Some(&Value::from(7)));
        assert_eq!(again.nota_cartella, "Cliente Rossi");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn senza_note_il_file_sparisce() {
        let dir = temp("vuoto");
        let c = cells(&[(Key::Folder, "nota")]);
        FolderNotes::default().with_cells(&c).write(&dir).unwrap();
        assert!(path_in(&dir).is_file());
        FolderNotes::default()
            .with_cells(&Cells::new())
            .write(&dir)
            .unwrap();
        assert!(!path_in(&dir).exists());
        assert!(FolderNotes::read(&dir).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn un_file_rovinato_e_un_errore_non_un_vuoto() {
        let dir = temp("rovinato");
        std::fs::write(path_in(&dir), "{ non è json").unwrap();
        assert!(FolderNotes::read(&dir).is_err());
        std::fs::write(path_in(&dir), "[]").unwrap();
        assert!(FolderNotes::read(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn le_frecce_fra_due_file_valgono_da_ogni_pc() {
        let dir = Path::new("Z:/lavori/rossi");
        let arrow = Annotation::Linea {
            a: (0.1, 0.1),
            b: (0.5, 0.5),
            color: Rgba::rgb(220, 38, 38),
            width: 0.5,
            start_arrow: false,
            end_arrow: true,
            end: Some(Anchor {
                path: dir.join("b.pdf"),
                page: 0,
                at: (0.2, 0.2),
            }),
        };
        let v = annotations_value(std::slice::from_ref(&arrow), dir).unwrap();
        assert!(v.to_string().contains("\"b.pdf\""));
        // Un altro PC vede la stessa cartella come \\server\lavori\rossi.
        let other = Path::new("//server/lavori/rossi");
        let back = annotations_from(&v, other);
        let Annotation::Linea { end: Some(e), .. } = &back[0] else {
            panic!("manca il capo attaccato");
        };
        assert_eq!(e.path, other.join("b.pdf"));
        assert_eq!(annotations_from(&v, dir), vec![arrow]);
    }

    #[test]
    fn la_scala_e_un_numero() {
        assert_eq!(scale_value(1), None);
        assert_eq!(scale_from(&scale_value(20).unwrap()), 20);
        assert_eq!(scale_from(&Value::String("1:20".into())), 1);
        assert_eq!(scale_from(&Value::from(-3)), 1);
    }

    #[test]
    fn riconosce_il_file_delle_note() {
        assert!(is_notes_file(Path::new("x/note.winfiles")));
        assert!(is_notes_file(Path::new("x/Note.WINFILES")));
        assert!(is_notes_file(Path::new("x/_Note WinFiles.winfiles-note")));
        assert!(!is_notes_file(Path::new("x/lavoro.winfiles")));
        assert!(!is_notes_file(Path::new("x/note.pdf")));
    }

    #[test]
    fn il_vecchio_nome_passa_a_quello_nuovo() {
        let dir = temp("vecchionome");
        std::fs::write(dir.join(OLD_FILE_NAME), r#"{"versione":1,"nota_cartella":"x"}"#).unwrap();
        let n = FolderNotes::read(&dir).unwrap().unwrap();
        assert_eq!(n.nota_cartella, "x");
        assert!(path_in(&dir).is_file());
        assert!(!dir.join(OLD_FILE_NAME).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn i_file_con_note_che_ci_sono_ancora() {
        let dir = temp("annotati");
        std::fs::write(dir.join("a.pdf"), "x").unwrap();
        std::fs::write(dir.join("b.pdf"), "x").unwrap();
        let text = r#"{"file":{
            "a.pdf":{"pagine":{"0":{"nota":"si"}}},
            "b.pdf":{"pagine":{}},
            "sparito.pdf":{"pagine":{"0":{"nota":"si"}}},
            "../fuori.pdf":{"pagine":{"0":{"nota":"si"}}}}}"#;
        let n = FolderNotes::from_json(text).unwrap();
        assert_eq!(n.annotated_files(&dir), vec![dir.join("a.pdf")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn la_freccia_porta_anche_il_file_senza_note() {
        let dir = temp("freccia");
        for f in ["a.pdf", "b.pdf", "c.pdf"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let arrow = Annotation::Linea {
            a: (0.1, 0.1),
            b: (0.5, 0.5),
            color: Rgba::rgb(220, 38, 38),
            width: 0.5,
            start_arrow: false,
            end_arrow: true,
            end: Some(Anchor {
                path: dir.join("c.pdf"),
                page: 0,
                at: (0.2, 0.2),
            }),
        };
        let v = annotations_value(&[arrow], &dir).unwrap();
        let mut n = FolderNotes::default();
        n.file
            .entry("a.pdf".into())
            .or_default()
            .pagine
            .entry(0)
            .or_default()
            .insert(PAGE_ANNOTATIONS.into(), v);
        assert_eq!(
            n.annotated_files(&dir),
            vec![dir.join("a.pdf"), dir.join("c.pdf")]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn job(files: &[&str]) -> NotesJob {
        NotesJob {
            file: files.iter().map(|f| (*f).to_owned()).collect(),
            data: 1,
            foglio: Paper::A4,
            verso: Orientation::Landscape,
            colonne: 3,
            una_per_foglio: false,
            tutta_pagina: false,
        }
    }

    #[test]
    fn i_lavori_diversi_restano_lo_stesso_si_aggiorna() {
        let mut n = FolderNotes::default();
        n.put_job(job(&["a.pdf", "b.pdf"]));
        n.put_job(job(&["c.pdf", "d.pdf"]));
        assert_eq!(n.jobs().len(), 2);
        // Lo stesso lavoro con un file in più prende il posto del vecchio, in cima.
        n.put_job(job(&["a.pdf", "b.pdf", "e.pdf"]));
        let files: Vec<Vec<String>> = n.jobs().into_iter().map(|j| j.file).collect();
        assert_eq!(files[0], vec!["a.pdf", "b.pdf", "e.pdf"]);
        assert_eq!(files[1], vec!["c.pdf", "d.pdf"]);
        assert_eq!(files.len(), 2);
        // Riaperta solo una parte del lavoro e cambiata una nota: il lavoro intero resta, in cima.
        n.put_job(job(&["c.pdf", "d.pdf"]));
        n.put_job(job(&["a.pdf"]));
        let files: Vec<Vec<String>> = n.jobs().into_iter().map(|j| j.file).collect();
        assert_eq!(files[0], vec!["a.pdf", "b.pdf", "e.pdf"]);
        assert_eq!(files[1], vec!["c.pdf", "d.pdf"]);
        assert_eq!(files.len(), 2);
        // Al più MAX_JOBS: i più vecchi si dimenticano.
        for i in 0..MAX_JOBS + 5 {
            n.put_job(job(&[&format!("f{i}.pdf")]));
        }
        assert_eq!(n.jobs().len(), MAX_JOBS);
    }

    #[test]
    fn un_lavoro_rovinato_non_fa_perdere_le_note() {
        let text = r#"{"nota_cartella":"x","lavori":[{"file":["a.pdf"],"foglio":"A9"},
            {"file":["b.pdf"],"foglio":"A4","verso":"Portrait","colonne":2},
            {"file":["c.pdf"],"foglio":"A4","verso":"Portrait","colonne":0}]}"#;
        let n = FolderNotes::from_json(text).unwrap();
        assert_eq!(n.nota_cartella, "x");
        let jobs = n.jobs();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].file, vec!["b.pdf"]);
        // Il lavoro rovinato (forse di una versione più nuova) si riscrive com'è.
        let mut m = n.clone();
        m.put_job(job(&["z.pdf"]));
        assert!(serde_json::to_string(&m).unwrap().contains("A9"));
        // Solo lavori, senza note: il file non serve più.
        let mut only = FolderNotes::default();
        only.put_job(job(&["a.pdf"]));
        assert!(only.is_empty());
    }

    #[test]
    fn i_file_del_lavoro_valgono_da_ogni_pc_e_restano_nelle_cartelle() {
        let dir = Path::new("/lavori/rossi");
        let files = [
            dir.join("a.pdf"),
            dir.join("sotto").join("b.pdf"),
            Path::new("/lavori/bianchi/c.pdf").to_path_buf(),
        ];
        let j = job(&[]).with_files(dir, &files);
        assert_eq!(j.file, vec!["a.pdf", "sotto/b.pdf", "../bianchi/c.pdf"]);
        let other = Path::new("/server/lavori/rossi");
        assert_eq!(
            j.paths(other),
            vec![
                other.join("a.pdf"),
                other.join("sotto").join("b.pdf"),
                other.join("..").join("bianchi").join("c.pdf"),
            ]
        );
        // Scritti da altri: niente percorsi assoluti né "." strani.
        let bad = job(&["/etc/passwd", "./x.pdf", "", "a\\b.pdf"]);
        assert_eq!(bad.paths(dir), vec![dir.join("a").join("b.pdf")]);
    }
}
