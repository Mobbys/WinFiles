//! Scansione provvisoria di file e cartelle trascinati. Nella Fase 1 sarà sostituita dal crate
//! `scan`, con lettura dei metadati, annullamento e lavoro in background.

use std::path::{Path, PathBuf};

use winfiles_model::{FileEntry, FileKind};

/// Profondità massima delle sottocartelle, per non perdersi in collegamenti circolari.
const MAX_DEPTH: usize = 32;
/// Numero massimo di file raccolti in una volta.
const MAX_FILES: usize = 20_000;

/// Raccoglie i file supportati dai percorsi dati, entrando nelle cartelle. Ordina per percorso,
/// così i file della stessa cartella restano vicini.
pub fn collect(paths: &[PathBuf]) -> Vec<FileEntry> {
    let mut out = Vec::new();
    for path in paths {
        visit(path, 0, &mut out);
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

fn visit(path: &Path, depth: usize, out: &mut Vec<FileEntry>) {
    if out.len() >= MAX_FILES || depth > MAX_DEPTH {
        return;
    }
    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            visit(&entry.path(), depth + 1, out);
        }
    } else {
        let kind = FileKind::from_path(path);
        if kind != FileKind::Unsupported {
            out.push(FileEntry::new(path.to_path_buf(), kind));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raccoglie_ricorsivamente_solo_i_supportati() {
        let dir = std::env::temp_dir().join(format!("winfiles-scan-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sotto")).unwrap();
        for name in ["a.pdf", "b.txt", "sotto/c.JPG"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let files = collect(&[dir.clone(), dir.join("a.pdf")]);
        let names: Vec<_> = files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.pdf", "c.JPG"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
