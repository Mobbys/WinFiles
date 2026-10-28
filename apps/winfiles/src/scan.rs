//! Ricerca dei file in file e cartelle trascinati, in un thread a parte: con cartelle di rete o
//! migliaia di file l'interfaccia resta fluida, mostra quanti file ha trovato e si può annullare.
//! Entrano tutti i file, anche quelli di tipo non supportato (senza anteprima né misure); restano
//! fuori solo i file nascosti e quelli che il sistema lascia nelle cartelle (Thumbs.db,
//! desktop.ini, i file temporanei di Office).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};

use winfiles_model::{FileEntry, FileKind};

/// Profondità massima delle sottocartelle, per non perdersi in collegamenti circolari.
const MAX_DEPTH: usize = 32;
/// Numero massimo di file raccolti in una volta.
pub const MAX_FILES: usize = 20_000;

/// Avanzamento di una ricerca, condiviso con il thread che la fa.
#[derive(Default)]
struct Progress {
    found: AtomicUsize,
    cancel: AtomicBool,
}

/// Raccoglie i file dai percorsi dati, entrando nelle cartelle. Ordina per cartella e nome
/// ([`winfiles_model::file_order`]), così i file della stessa cartella restano vicini.
pub fn collect(paths: &[PathBuf]) -> Vec<FileEntry> {
    collect_with(paths, &Progress::default())
}

fn collect_with(paths: &[PathBuf], progress: &Progress) -> Vec<FileEntry> {
    let mut out = Vec::new();
    for path in paths {
        visit(path, 0, &mut out, progress);
    }
    out.sort_by_cached_key(|f| winfiles_model::file_order_key(&f.path));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

fn visit(path: &Path, depth: usize, out: &mut Vec<FileEntry>, progress: &Progress) {
    if out.len() >= MAX_FILES || depth > MAX_DEPTH || progress.cancel.load(Ordering::Relaxed) {
        return;
    }
    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            visit(&entry.path(), depth + 1, out, progress);
        }
    } else {
        if is_system_file(path) {
            return;
        }
        out.push(FileEntry::new(path.to_path_buf(), FileKind::from_path(path)));
        progress.found.store(out.len(), Ordering::Relaxed);
    }
}

/// I file che nessuno vuole stampare: nascosti (il punto davanti, o l'attributo di Windows), le
/// miniature e le impostazioni di cartella di Windows e macOS, i file di blocco di Office (`~$`).
fn is_system_file(path: &Path) -> bool {
    const SYSTEM: [&str; 4] = ["thumbs.db", "ehthumbs.db", "desktop.ini", ".ds_store"];
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    name.starts_with('.') || name.starts_with("~$") || SYSTEM.contains(&name.as_str()) || hidden(path)
}

#[cfg(windows)]
fn hidden(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    std::fs::metadata(path).is_ok_and(|m| m.file_attributes() & (HIDDEN | SYSTEM) != 0)
}

#[cfg(not(windows))]
fn hidden(_: &Path) -> bool {
    false
}

/// Com'è finita una ricerca.
pub struct Found {
    pub files: Vec<FileEntry>,
    pub cancelled: bool,
    /// Vero se si è fermata al tetto di [`MAX_FILES`].
    pub truncated: bool,
}

/// Una ricerca in corso.
pub struct Scan {
    progress: Arc<Progress>,
    result: Receiver<Found>,
}

impl Scan {
    /// Avvia la ricerca. `wake` sveglia l'interfaccia mentre lavora e quando ha finito.
    pub fn start(paths: Vec<PathBuf>, wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let progress = Arc::new(Progress::default());
        let shared = Arc::clone(&progress);
        let (tx, result) = channel();
        std::thread::Builder::new()
            .name("ricerca-file".into())
            .spawn(move || {
                let mut files = collect_with(&paths, &shared);
                let cancelled = shared.cancel.load(Ordering::Relaxed);
                if cancelled {
                    files.clear();
                }
                let truncated = files.len() >= MAX_FILES;
                // Se l'interfaccia non c'è più, il risultato non serve a nessuno.
                let _ = tx.send(Found {
                    files,
                    cancelled,
                    truncated,
                });
                wake();
            })?;
        Ok(Self { progress, result })
    }

    /// File trovati finora.
    pub fn found(&self) -> usize {
        self.progress.found.load(Ordering::Relaxed)
    }

    /// Chiede di fermarsi: il thread smette alla prossima voce e non consegna nessun file.
    pub fn cancel(&self) {
        self.progress.cancel.store(true, Ordering::Relaxed);
    }

    /// Il risultato, quando è pronto.
    pub fn poll(&self) -> Option<Found> {
        self.result.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("winfiles-scan-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sotto")).unwrap();
        for name in [
            "a.pdf",
            "b.txt",
            "sotto/c.JPG",
            "Thumbs.db",
            ".nascosto",
            "~$lettera.docx",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        dir
    }

    #[test]
    fn raccoglie_ricorsivamente_ogni_file_tranne_quelli_di_sistema() {
        let dir = tree("sync");
        let files = collect(&[dir.clone(), dir.join("a.pdf")]);
        let names: Vec<_> = files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.pdf", "b.txt", "c.JPG"]);
        let kinds: Vec<_> = files.iter().map(|f| f.kind).collect();
        assert_eq!(kinds, [FileKind::Pdf, FileKind::Unsupported, FileKind::Image]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn la_ricerca_in_background_consegna_e_si_annulla() {
        let dir = tree("bg");
        let scan = Scan::start(vec![dir.clone()], || {}).unwrap();
        let found = loop {
            if let Some(f) = scan.poll() {
                break f;
            }
            std::thread::yield_now();
        };
        assert_eq!(found.files.len(), 3);
        assert!(!found.cancelled && !found.truncated);
        assert_eq!(scan.found(), 3);

        let progress = Progress::default();
        progress.cancel.store(true, Ordering::Relaxed);
        assert!(collect_with(std::slice::from_ref(&dir), &progress).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
