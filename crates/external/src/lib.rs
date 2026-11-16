//! I programmi esterni dei pulsanti «Apri in …» (ADR 0037).
//!
//! - [`types`]: i tipi di file, i programmi noti e quelli consigliati.
//! - [`Config`]: le scelte dell'utente (quali programmi per ogni tipo e in che ordine), salvate in
//!   `impostazioni.json` dall'app.
//! - [`detect`]: i programmi installati, chiesti a Windows (la lista «Apri con» di ogni tipo) e
//!   cercati per nome (App Paths del registro, cartelle di Adobe).
//! - [`launch`]: apre i file con un programma.
//!
//! Tutto ciò che parla con Windows sta in `win.rs`; qui solo logica pura, provata su ogni sistema.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub mod types;
#[cfg(windows)]
mod win;

pub use types::{FAMILIES, Family, FileType, Group, TYPES, type_by_key, type_of_ext};

/// Tetti contro un `impostazioni.json` rovinato o scritto a mano.
const MAX_PROGRAMS: usize = 64;
const MAX_PER_TYPE: usize = 12;
const MAX_TEXT: usize = 512;
/// Mai più file di così in una volta sola.
pub const MAX_FILES: usize = 200;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} non si trova più su questo PC")]
    Missing(String),
    #[error("non riesco ad avviare {0}: {1}")]
    Spawn(String, std::io::Error),
    #[error("Windows non riesce ad aprire i file con {0}: {1}")]
    Shell(String, String),
    #[error("nessun file da aprire")]
    NoFiles,
}

/// Un programma scelto dall'utente.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Program {
    /// L'identità: la chiave di un programma noto (`photoshop`), `app:<id>` per le app dello Store,
    /// `exe:<percorso>` per gli altri.
    pub key: String,
    /// Il nome completo, nei suggerimenti.
    pub name: String,
    /// Il nome sul pulsante, modificabile.
    pub label: String,
    /// L'eseguibile, se c'è.
    pub exe: Option<PathBuf>,
    /// L'app dello Store (AppUserModelID), se è una di quelle.
    pub app_id: Option<String>,
    /// Il nome con cui Windows lo elenca in «Apri con», quando non c'è né l'uno né l'altro.
    pub handler: Option<String>,
}

impl Program {
    pub fn from_found(f: &Found) -> Self {
        Program {
            key: f.key.clone(),
            name: f.name.clone(),
            label: f.label.clone(),
            exe: f.exe.clone(),
            app_id: f.app_id.clone(),
            handler: f.handler.clone(),
        }
    }

    /// Un eseguibile scelto a mano con Sfoglia…
    pub fn from_exe(path: &Path) -> Self {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(f) = types::family_of_exe(&file) {
            return Program {
                key: f.key.into(),
                name: f.name.into(),
                label: f.label.into(),
                exe: Some(path.to_path_buf()),
                ..Program::default()
            };
        }
        let stem = path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.clone());
        Program {
            key: exe_key(path),
            name: stem.clone(),
            label: stem,
            exe: Some(path.to_path_buf()),
            ..Program::default()
        }
    }

    /// Il programma noto, se lo è.
    pub fn family(&self) -> Option<&'static Family> {
        types::family(&self.key)
    }

    /// Il nome sul pulsante, mai vuoto.
    pub fn button_label(&self) -> &str {
        if !self.label.trim().is_empty() {
            &self.label
        } else if !self.name.trim().is_empty() {
            &self.name
        } else {
            &self.key
        }
    }
}

fn exe_key(path: &Path) -> String {
    format!("exe:{}", path.to_string_lossy().to_lowercase())
}

/// Un programma trovato sul PC.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub key: String,
    pub name: String,
    pub label: String,
    pub exe: Option<PathBuf>,
    pub app_id: Option<String>,
    pub handler: Option<String>,
    /// I tipi che sa aprire (chiavi di [`TYPES`]).
    pub types: BTreeSet<String>,
}

/// Un'icona in RGBA con alfa premoltiplicato, riga per riga dall'alto.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Le scelte dell'utente.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// I programmi scelti per almeno un tipo.
    pub programs: Vec<Program>,
    /// Per ogni tipo, le chiavi dei programmi in ordine (= l'ordine dei pulsanti).
    pub types: BTreeMap<String, Vec<String>>,
    /// I consigliati sono già stati messi una volta: da qui in poi decide l'utente.
    pub seeded: bool,
}

impl Config {
    /// Toglie ciò che non si può usare: chiavi vuote o ripetute, tipi sconosciuti, programmi non
    /// usati, testi enormi, troppi elementi.
    pub fn sanitized(mut self) -> Self {
        let mut seen = BTreeSet::new();
        self.programs
            .retain(|p| !p.key.is_empty() && p.key.len() <= MAX_TEXT && seen.insert(p.key.clone()));
        self.programs.truncate(MAX_PROGRAMS);
        for p in &mut self.programs {
            cut(&mut p.name);
            cut(&mut p.label);
        }
        let known: BTreeSet<String> = self.programs.iter().map(|p| p.key.clone()).collect();
        self.types.retain(|k, _| type_by_key(k).is_some());
        for list in self.types.values_mut() {
            let mut seen = BTreeSet::new();
            list.retain(|k| known.contains(k) && seen.insert(k.clone()));
            list.truncate(MAX_PER_TYPE);
        }
        self.types.retain(|_, l| !l.is_empty());
        self.drop_unused();
        self
    }

    fn drop_unused(&mut self) {
        let used: BTreeSet<&String> = self.types.values().flatten().collect();
        let keep: BTreeSet<String> = used.into_iter().cloned().collect();
        self.programs.retain(|p| keep.contains(&p.key));
    }

    pub fn program(&self, key: &str) -> Option<&Program> {
        self.programs.iter().find(|p| p.key == key)
    }

    pub fn program_mut(&mut self, key: &str) -> Option<&mut Program> {
        self.programs.iter_mut().find(|p| p.key == key)
    }

    /// I programmi di un tipo, in ordine.
    pub fn for_type(&self, type_key: &str) -> Vec<&Program> {
        self.types
            .get(type_key)
            .map(|l| l.iter().filter_map(|k| self.program(k)).collect())
            .unwrap_or_default()
    }

    /// I programmi per un file, dalla sua estensione.
    pub fn for_path(&self, path: &Path) -> Vec<&Program> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(type_of_ext)
            .map(|t| self.for_type(t.key))
            .unwrap_or_default()
    }

    /// I programmi buoni per tutti questi file (quelli del primo che valgono anche per gli altri).
    pub fn for_paths(&self, paths: &[PathBuf]) -> Vec<&Program> {
        let Some(first) = paths.first() else {
            return Vec::new();
        };
        let mut list = self.for_path(first);
        for p in paths.iter().skip(1) {
            let other: BTreeSet<&str> = self.for_path(p).iter().map(|p| p.key.as_str()).collect();
            list.retain(|p| other.contains(p.key.as_str()));
        }
        list
    }

    /// Mette il programma in fondo ai tipi dati (se non c'è già) e lo ricorda.
    pub fn add(&mut self, program: Program, type_keys: &[&str]) {
        let key = program.key.clone();
        match self.program_mut(&key) {
            Some(p) => {
                // Lo stesso programma in un'altra versione: si tiene il nome scelto dall'utente.
                p.exe = program.exe.or(p.exe.take());
                p.app_id = program.app_id.or(p.app_id.take());
                p.handler = program.handler.or(p.handler.take());
            }
            None => self.programs.push(program),
        }
        for t in type_keys {
            if type_by_key(t).is_none() {
                continue;
            }
            let list = self.types.entry((*t).to_string()).or_default();
            if !list.contains(&key) && list.len() < MAX_PER_TYPE {
                list.push(key.clone());
            }
        }
        self.seeded = true;
    }

    /// Toglie il programma da un tipo (e lo dimentica se non serve più a nessun tipo).
    pub fn remove(&mut self, type_key: &str, key: &str) {
        if let Some(list) = self.types.get_mut(type_key) {
            list.retain(|k| k != key);
            if list.is_empty() {
                self.types.remove(type_key);
            }
        }
        self.drop_unused();
        self.seeded = true;
    }

    /// Sposta il programma di un posto in su (`up`) o in giù nel tipo.
    pub fn shift(&mut self, type_key: &str, key: &str, up: bool) {
        let Some(list) = self.types.get_mut(type_key) else {
            return;
        };
        let Some(i) = list.iter().position(|k| k == key) else {
            return;
        };
        let j = if up {
            i.checked_sub(1)
        } else {
            i.checked_add(1).filter(|j| *j < list.len())
        };
        if let Some(j) = j {
            list.swap(i, j);
        }
    }

    /// Rinomina il pulsante di un programma.
    pub fn rename(&mut self, key: &str, label: &str) {
        if let Some(p) = self.program_mut(key) {
            p.label = label.chars().take(60).collect();
        }
    }

    /// Mette i programmi consigliati presenti fra quelli trovati, al posto delle scelte di prima.
    pub fn reset_recommended(&mut self, found: &[Found]) {
        self.types.clear();
        for t in TYPES {
            for slot in types::recommended(t.key) {
                if let Some(f) = slot.iter().find_map(|k| found.iter().find(|f| f.key == *k)) {
                    let p = self
                        .program(&f.key)
                        .cloned()
                        .unwrap_or_else(|| Program::from_found(f));
                    self.add(p, &[t.key]);
                }
            }
        }
        self.drop_unused();
        self.seeded = true;
    }

    /// I programmi appena trovati: aggiorna percorsi e identità (un programma noto cambia cartella
    /// a ogni versione) e, la prima volta, mette i consigliati. Vero se qualcosa è cambiato.
    pub fn refresh(&mut self, found: &[Found]) -> bool {
        let before = self.clone();
        for p in &mut self.programs {
            if let Some(f) = found.iter().find(|f| f.key == p.key) {
                // Il programma scelto a mano con Sfoglia resta quello, se c'è ancora.
                let manual = p.exe.as_deref().is_some_and(Path::is_file) && f.exe.is_none();
                if !manual {
                    if f.exe.is_some() {
                        p.exe = f.exe.clone();
                    }
                    if f.app_id.is_some() {
                        p.app_id = f.app_id.clone();
                    }
                    if f.handler.is_some() {
                        p.handler = f.handler.clone();
                    }
                }
            }
        }
        if !self.seeded && !found.is_empty() {
            self.reset_recommended(found);
        }
        *self != before
    }
}

fn cut(s: &mut String) {
    if s.len() > MAX_TEXT {
        *s = s.chars().take(120).collect();
    }
}

/// Se il programma si può avviare su questo PC, fra quelli trovati.
pub fn available(p: &Program, found: &[Found]) -> bool {
    if p.exe.as_deref().is_some_and(Path::is_file) {
        return true;
    }
    found.iter().any(|f| {
        f.key == p.key
            && (f.exe.as_deref().is_some_and(Path::is_file) || f.app_id.is_some() || f.handler.is_some())
    })
}

/// I programmi trovati sul PC, con i tipi che aprono. Lento (registro e disco): va chiamato
/// fuori dal thread dell'interfaccia.
pub fn detect() -> Vec<Found> {
    #[cfg(windows)]
    {
        win::detect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// L'icona del programma, se Windows la dà.
pub fn icon(p: &Program) -> Option<Icon> {
    #[cfg(windows)]
    {
        win::icon(p)
    }
    #[cfg(not(windows))]
    {
        let _ = p;
        None
    }
}

/// Gli argomenti per l'eseguibile: i file, e per Acrobat la pagina se è un solo PDF.
pub fn exe_args(p: &Program, files: &[PathBuf], page: Option<u32>) -> Vec<std::ffi::OsString> {
    let mut args = Vec::new();
    if p.key == "acrobat"
        && let ([file], Some(page)) = (files, page)
        && file.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
    {
        args.push("/A".into());
        args.push(format!("page={page}").into());
    }
    args.extend(files.iter().take(MAX_FILES).map(|f| f.as_os_str().to_os_string()));
    args
}

/// Apre i file con il programma (`page` = la pagina da mostrare, da 1, se il programma lo sa
/// fare). Può aspettare Windows: va chiamata fuori dal thread dell'interfaccia.
pub fn launch(p: &Program, files: &[PathBuf], page: Option<u32>) -> Result<(), Error> {
    if files.is_empty() {
        return Err(Error::NoFiles);
    }
    let name = p.button_label().to_string();
    if let Some(exe) = p.exe.as_deref().filter(|e| e.is_file()) {
        let mut cmd = std::process::Command::new(exe);
        cmd.args(exe_args(p, files, page));
        if let Some(dir) = files.first().and_then(|f| f.parent()) {
            cmd.current_dir(dir);
        }
        return cmd.spawn().map(drop).map_err(|e| Error::Spawn(name, e));
    }
    #[cfg(windows)]
    {
        let files: Vec<PathBuf> = files.iter().take(MAX_FILES).cloned().collect();
        if let Some(id) = &p.app_id {
            return win::activate(id, &files).map_err(|e| Error::Shell(name, e));
        }
        if let Some(h) = &p.handler {
            return win::invoke(h, &files).map_err(|e| Error::Shell(name, e));
        }
    }
    Err(Error::Missing(name))
}

#[cfg(test)]
mod tests;
