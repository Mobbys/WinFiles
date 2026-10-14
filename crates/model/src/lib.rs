//! Il modello del progetto (livello L1): l'elenco dei file con le loro pagine, le note per file e
//! per cartella e le impostazioni di stampa. Si salva come file `.winfiles` in JSON.
//!
//! Le note vivono qui e non nel risultato stampato: nella versione 4.9 stavano solo nella pagina
//! HTML generata e si perdevano alla chiusura.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use winfiles_geom::{Margins, Orientation, Paper, Size};

/// Versione del formato del file di progetto. Si alza a ogni modifica incompatibile.
pub const FORMAT_VERSION: u32 = 1;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
            "jpg" | "jpeg" | "png" | "tif" | "tiff" | "webp" | "bmp" | "gif" => FileKind::Image,
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

/// Un file del progetto.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: PathBuf,
    pub kind: FileKind,
    #[serde(default)]
    pub pages: Vec<PageInfo>,
    /// Nota libera dell'utente su questo file.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl FileEntry {
    /// La cartella che contiene il file (chiave per il raggruppamento).
    pub fn folder(&self) -> &Path {
        self.path.parent().unwrap_or_else(|| Path::new(""))
    }
}

/// Impostazioni di stampa del progetto.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrintSettings {
    pub paper: Paper,
    pub orientation: Orientation,
    pub margins: Margins,
    /// Numero di colonne della griglia.
    pub columns: u32,
    /// Spazio tra le celle, in mm.
    pub gap: f64,
    /// Altezza riservata alla didascalia sotto ogni miniatura, in mm.
    pub caption: f64,
    /// Altezza dell'intestazione di cartella, in mm.
    pub header: f64,
    /// Ogni cartella inizia su un foglio nuovo.
    pub page_break_per_folder: bool,
}

impl Default for PrintSettings {
    fn default() -> Self {
        Self {
            paper: Paper::A4,
            orientation: Orientation::Portrait,
            margins: Margins::uniform(10.0),
            columns: 4,
            gap: 4.0,
            caption: 12.0,
            header: 10.0,
            page_break_per_folder: false,
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
    pub print: PrintSettings,
}

impl Project {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            version: FORMAT_VERSION,
            title: title.into(),
            ..Self::default()
        }
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
        let project: Project = serde_json::from_value(value)?;
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
            note: "Verificare il colore Pantone, è più scuro".into(),
        });
        p.folder_notes
            .insert(PathBuf::from("/lavori/cliente"), "Consegna entro venerdì".into());
        p
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
