//! Cerca e filtra nell'elenco dei file: per nome o cartella e per tipo. Il filtro cambia solo
//! quello che l'elenco mostra, mai quello che va in stampa.

use winfiles_model::{FileEntry, FileKind};

/// I tipi che si possono scegliere, con il nome del loro pulsante.
pub const KINDS: [(FileKind, &str); 4] = [
    (FileKind::Image, "Immagini"),
    (FileKind::Pdf, "PDF"),
    (FileKind::Ai, "AI"),
    (FileKind::Eps, "EPS"),
];

/// Cosa mostrare nell'elenco dei file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileFilter {
    /// Parole da cercare nel nome del file o della cartella, tutte insieme, senza badare alle
    /// maiuscole.
    pub text: String,
    /// Tipi scelti; nessuno vuol dire tutti.
    pub kinds: Vec<FileKind>,
}

impl FileFilter {
    /// Vero se il filtro nasconde qualcosa.
    pub fn is_active(&self) -> bool {
        !self.text.trim().is_empty() || !self.kinds.is_empty()
    }

    /// Accende o spegne un tipo.
    pub fn toggle(&mut self, kind: FileKind) {
        if let Some(i) = self.kinds.iter().position(|k| *k == kind) {
            self.kinds.remove(i);
        } else {
            self.kinds.push(kind);
        }
    }

    /// Vero se il file resta visibile.
    pub fn matches(&self, f: &FileEntry) -> bool {
        // I tipi non supportati non hanno un pulsante: li mostra solo "Tutti".
        if !self.kinds.is_empty() && !self.kinds.contains(&f.kind) {
            return false;
        }
        let text = self.text.trim();
        if text.is_empty() {
            return true;
        }
        let path = f.path.to_string_lossy().to_lowercase();
        text.to_lowercase()
            .split_whitespace()
            .all(|word| path.contains(word))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn f(path: &str) -> FileEntry {
        let path = PathBuf::from(path);
        let kind = FileKind::from_path(&path);
        FileEntry::new(path, kind)
    }

    #[test]
    fn cerca_tutte_le_parole_nel_nome_e_nella_cartella() {
        let mut filter = FileFilter::default();
        assert!(!filter.is_active());
        assert!(filter.matches(&f("/lavori/Rossi/Volantino.pdf")));
        filter.text = "  rossi VOLANT ".into();
        assert!(filter.is_active());
        assert!(filter.matches(&f("/lavori/Rossi/Volantino.pdf")));
        assert!(!filter.matches(&f("/lavori/Bianchi/Volantino.pdf")));
        filter.text = "   ".into();
        assert!(!filter.is_active());
    }

    #[test]
    fn i_tipi_scelti_si_sommano() {
        let mut filter = FileFilter::default();
        filter.toggle(FileKind::Pdf);
        assert!(filter.matches(&f("/a/b.pdf")));
        assert!(!filter.matches(&f("/a/b.jpg")));
        filter.toggle(FileKind::Image);
        assert!(filter.matches(&f("/a/b.jpg")));
        assert!(!filter.matches(&f("/a/b.eps")));
        filter.text = "b".into();
        assert!(!filter.matches(&f("/a/c.jpg")));
        filter.toggle(FileKind::Pdf);
        filter.toggle(FileKind::Image);
        assert!(filter.kinds.is_empty());
    }
}
