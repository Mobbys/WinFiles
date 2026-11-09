//! I file di tipo non supportato nell'elenco: entrano come gli altri (nome, cartella, nota, un
//! riquadro sul foglio), senza anteprima né misure. La casella "Includi i file non supportati"
//! li toglie tutti insieme e li tiene da parte finché l'app è aperta, per rimetterli al loro posto.

use winfiles_model::{FileEntry, FileKind};

/// Divide i file: prima quelli supportati, poi quelli non supportati, ognuno nel suo ordine.
pub fn split(files: Vec<FileEntry>) -> (Vec<FileEntry>, Vec<FileEntry>) {
    files.into_iter().partition(|f| f.kind != FileKind::Unsupported)
}

/// Rimette i file tolti nell'elenco senza cambiare l'ordine degli altri, che può essere quello
/// scelto trascinando: ognuno torna nella sua cartella, prima del primo file che lo segue per
/// nome; in una cartella non più nell'elenco, al posto che le spetta per cartella.
pub fn restore(files: &mut Vec<FileEntry>, parked: Vec<FileEntry>) {
    for f in parked {
        if files.iter().any(|g| g.path == f.path) {
            continue;
        }
        let folder = f.folder().to_path_buf();
        let same: Vec<usize> = files
            .iter()
            .enumerate()
            .filter(|(_, g)| g.folder() == folder)
            .map(|(i, _)| i)
            .collect();
        let at = match (same.first(), same.last()) {
            (Some(_), Some(last)) => same
                .iter()
                .copied()
                .find(|i| files.get(*i).is_some_and(|g| after(&g.path, &f.path)))
                .unwrap_or(last + 1),
            _ => files
                .iter()
                .position(|g| after(&g.path, &f.path))
                .unwrap_or(files.len()),
        };
        files.insert(at.min(files.len()), f);
    }
}

/// L'estensione del file in maiuscolo, per riconoscerne il tipo ("DOCX"); "FILE" se non ne ha.
/// Al più 8 caratteri: un nome con un punto qualsiasi non diventa una scritta lunghissima.
pub fn extension(path: &std::path::Path) -> String {
    match path.extension().map(|e| e.to_string_lossy().to_uppercase()) {
        Some(e) if !e.is_empty() => e.chars().take(8).collect(),
        _ => "FILE".into(),
    }
}

/// Vero se `a` viene dopo `b` nell'ordine dei file appena aggiunti.
fn after(a: &std::path::Path, b: &std::path::Path) -> bool {
    winfiles_model::file_order(a, b) == std::cmp::Ordering::Greater
}

/// Quanti file non supportati ci sono.
pub fn count(files: &[FileEntry]) -> usize {
    files.iter().filter(|f| f.kind == FileKind::Unsupported).count()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn entry(path: &str) -> FileEntry {
        let path = PathBuf::from(path);
        let kind = FileKind::from_path(&path);
        FileEntry::new(path, kind)
    }

    fn names(files: &[FileEntry]) -> Vec<String> {
        files.iter().map(|f| f.path.display().to_string()).collect()
    }

    #[test]
    fn separa_i_non_supportati_tenendo_l_ordine() {
        let files = vec![
            entry("/a/z.pdf"),
            entry("/a/b.docx"),
            entry("/a/c.jpg"),
            entry("/a/a.zip"),
        ];
        let (keep, parked) = split(files);
        assert_eq!(names(&keep), ["/a/z.pdf", "/a/c.jpg"]);
        assert_eq!(names(&parked), ["/a/b.docx", "/a/a.zip"]);
        assert_eq!(count(&parked), 2);
        assert_eq!(count(&keep), 0);
    }

    #[test]
    fn rimette_i_file_al_loro_posto_senza_toccare_l_ordine_scelto() {
        // L'ordine della cartella /a è stato cambiato a mano: z.pdf prima di c.jpg.
        let mut files = vec![entry("/a/z.pdf"), entry("/a/c.jpg"), entry("/b/x.png")];
        let parked = vec![
            entry("/a/b.docx"),
            entry("/b/y.txt"),
            entry("/a/zz.txt"),
            entry("/c/n.txt"),
        ];
        restore(&mut files, parked);
        assert_eq!(
            names(&files),
            [
                "/a/b.docx",
                "/a/z.pdf",
                "/a/c.jpg",
                "/a/zz.txt",
                "/b/x.png",
                "/b/y.txt",
                "/c/n.txt"
            ]
        );
    }

    #[test]
    fn estensione_in_maiuscolo_e_corta() {
        assert_eq!(extension(std::path::Path::new("/a/Lettera.docx")), "DOCX");
        assert_eq!(extension(std::path::Path::new("/a/LEGGIMI")), "FILE");
        assert_eq!(
            extension(std::path::Path::new("/a/x.unestensionelunga")),
            "UNESTENS"
        );
    }

    #[test]
    fn non_raddoppia_un_file_gia_nell_elenco() {
        let mut files = vec![entry("/a/b.docx")];
        restore(&mut files, vec![entry("/a/b.docx")]);
        assert_eq!(files.len(), 1);
    }
}
