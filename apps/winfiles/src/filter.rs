//! Cerca e filtra nell'elenco dei file: per nome o cartella, per numero di pagina e per testo
//! delle note, e per tipo. Come nella 4.9, quello che l'elenco nasconde sparisce anche da
//! anteprima, stampa, PDF e riepilogo (`FileFilter::apply`); il lavoro salvato non cambia.

use winfiles_model::{FileEntry, FileKind, Project};

/// I tipi che si possono scegliere, con il nome del loro pulsante.
pub const KINDS: [(FileKind, &str); 5] = [
    (FileKind::Image, "Immagini"),
    (FileKind::Pdf, "PDF"),
    (FileKind::Ai, "AI"),
    (FileKind::Eps, "EPS"),
    (FileKind::Unsupported, "Altri"),
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

    /// Che cosa del file corrisponde alla ricerca: il file intero (nome o cartella) oppure solo
    /// alcune pagine, quando una parola è il loro numero o sta nella loro nota. Per esempio
    /// "catalogo 3" trova la pagina 3 di catalogo.pdf, "pantone" le pagine con quella nota.
    pub fn find(&self, f: &FileEntry) -> Found {
        if !self.kinds.is_empty() && !self.kinds.contains(&f.kind) {
            return Found::Nothing;
        }
        let text = self.text.trim().to_lowercase();
        let words = words(&text);
        if words.is_empty() {
            return Found::File;
        }
        let path = f.path.to_string_lossy().to_lowercase();
        let name = f
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        // Una parola vale per il file intero se sta nel percorso; un numero solo se sta nel nome
        // (le cartelle hanno spesso date e codici), e mai se è chiesto come pagina.
        let in_file = |w: &Word| {
            !w.page
                && if w.number {
                    name.contains(w.text)
                } else {
                    path.contains(w.text)
                }
        };
        if words.iter().all(in_file) {
            return Found::File;
        }
        if f.sheet_pages() < 2 {
            return Found::Nothing;
        }
        let pages: Vec<usize> = (0..f.sheet_pages())
            .filter(|p| {
                let number = (p + 1).to_string();
                let note = f.note(*p).to_lowercase();
                words
                    .iter()
                    .all(|w| in_file(w) || note.contains(w.text) || (w.number && w.text == number))
            })
            .collect();
        if pages.is_empty() {
            Found::Nothing
        } else {
            Found::Pages(pages)
        }
    }
}

impl FileFilter {
    /// Il lavoro come lo vedono anteprima, stampa e PDF: i file che la ricerca non trova e le
    /// pagine non trovate vanno fuori stampa. Gli indici dei file restano quelli del lavoro.
    /// Senza filtro, il lavoro com'è.
    pub fn apply(&self, project: &Project) -> Project {
        let mut shown = project.clone();
        if !self.is_active() {
            return shown;
        }
        for f in &mut shown.files {
            match self.find(f) {
                Found::File => {}
                Found::Nothing => f.set_all_printed(false),
                Found::Pages(pages) => {
                    for p in 0..f.sheet_pages() {
                        if !pages.contains(&p) {
                            f.set_printed(p, false);
                        }
                    }
                }
            }
        }
        shown
    }

    /// Quanti file del lavoro la ricerca trova.
    pub fn count(&self, project: &Project) -> usize {
        project
            .files
            .iter()
            .filter(|f| self.find(f) != Found::Nothing)
            .count()
    }
}

/// Una parola cercata.
struct Word<'a> {
    text: &'a str,
    /// Solo cifre.
    number: bool,
    /// Chiesta come pagina: "pag. 3", "pagina 3", "p3".
    page: bool,
}

/// Le parole della ricerca. "pag", "pag.", "pagina" e "p." da soli dicono che il numero dopo è una
/// pagina; attaccati a un numero ("p3", "pag.3") lo stesso. Ogni altra parola, anche "pagamento",
/// resta com'è.
fn words(text: &str) -> Vec<Word<'_>> {
    const PREFIXES: [&str; 5] = ["pagina", "pag.", "pag", "p.", "p"];
    let digits = |w: &str| !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit());
    let mut out = Vec::new();
    let mut page_next = false;
    for w in text.split_whitespace() {
        if w != "p" && PREFIXES.contains(&w) {
            page_next = true;
            continue;
        }
        let glued = PREFIXES
            .iter()
            .find_map(|p| w.strip_prefix(p).filter(|rest| digits(rest)));
        let (text, page) = match glued {
            Some(rest) => (rest, true),
            None => (w, page_next && digits(w)),
        };
        out.push(Word {
            text,
            number: digits(text),
            page,
        });
        page_next = false;
    }
    out
}

/// Il risultato della ricerca su un file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    Nothing,
    /// Il file intero: nome o cartella corrispondono, o non si cerca nulla.
    File,
    /// Solo queste pagine (da 0).
    Pages(Vec<usize>),
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
        assert!((filter.find(&f("/lavori/Rossi/Volantino.pdf")) != Found::Nothing));
        filter.text = "  rossi VOLANT ".into();
        assert!(filter.is_active());
        assert!((filter.find(&f("/lavori/Rossi/Volantino.pdf")) != Found::Nothing));
        assert!(!(filter.find(&f("/lavori/Bianchi/Volantino.pdf")) != Found::Nothing));
        filter.text = "   ".into();
        assert!(!filter.is_active());
    }

    #[test]
    fn cerca_le_singole_pagine_per_numero_e_per_nota() {
        // Le cartelle hanno cifre: non devono far trovare il file intero al posto della pagina.
        let mut libro = f("/lavori/2024-03/Rossi/catalogo.pdf");
        libro.pages = vec![
            winfiles_model::PageInfo {
                size: winfiles_geom::Size::new(210.0, 297.0),
                trim: None,
            };
            12
        ];
        libro.set_note(2, "Verificare il Pantone".into());
        let mut filter = FileFilter {
            text: "catalogo 3".into(),
            ..Default::default()
        };
        assert_eq!(filter.find(&libro), Found::Pages(vec![2]));
        // Un numero non trova le pagine 13, 23…: solo quella giusta.
        filter.text = "catalogo 1".into();
        assert_eq!(filter.find(&libro), Found::Pages(vec![0]));
        for text in ["catalogo pag. 12", "catalogo pagina 12", "catalogo p12"] {
            filter.text = text.into();
            assert_eq!(filter.find(&libro), Found::Pages(vec![11]), "{text}");
        }
        filter.text = "pantone".into();
        assert_eq!(filter.find(&libro), Found::Pages(vec![2]));
        // Le parole che cominciano per "pag" restano parole.
        filter.text = "pagamento".into();
        assert_eq!(filter.find(&libro), Found::Nothing);
        filter.text = "catalogo".into();
        assert_eq!(filter.find(&libro), Found::File);
        filter.text = "catalogo 40".into();
        assert_eq!(filter.find(&libro), Found::Nothing);
        // Un file di una pagina si trova solo intero.
        filter.text = "volantino 1".into();
        assert_eq!(filter.find(&f("/x/volantino.pdf")), Found::Nothing);
    }

    #[test]
    fn anteprima_e_stampa_seguono_il_filtro() {
        let mut project = Project::default();
        project.files.push(f("/a/rossi.pdf"));
        project.files.push(f("/a/bianchi.jpg"));
        let mut libro = f("/b/catalogo.pdf");
        libro.pages = vec![
            winfiles_model::PageInfo {
                size: winfiles_geom::Size::new(210.0, 297.0),
                trim: None,
            };
            4
        ];
        libro.set_printed(0, false);
        project.files.push(libro);
        let printed = |p: &Project| -> Vec<Vec<usize>> {
            p.files.iter().map(|f| f.printed_pages().collect()).collect()
        };
        // Senza filtro non cambia nulla, nemmeno le pagine già tolte a mano.
        let mut filter = FileFilter::default();
        assert_eq!(filter.apply(&project), project);
        // Per tipo: i JPG spariscono, i PDF restano come erano.
        filter.toggle(FileKind::Pdf);
        assert_eq!(
            printed(&filter.apply(&project)),
            [vec![0usize], vec![], vec![1, 2, 3]]
        );
        assert_eq!(filter.count(&project), 2);
        // Per pagina: solo quella trovata, e una pagina tolta a mano resta tolta.
        filter.text = "catalogo 3".into();
        assert_eq!(printed(&filter.apply(&project)), [vec![], vec![], vec![2usize]]);
        filter.text = "catalogo 1".into();
        assert_eq!(
            printed(&filter.apply(&project)),
            [Vec::<usize>::new(), vec![], vec![]]
        );
        // Il lavoro di partenza non si tocca.
        assert_eq!(printed(&project), [vec![0usize], vec![0], vec![1, 2, 3]]);
    }

    #[test]
    fn i_tipi_scelti_si_sommano() {
        let mut filter = FileFilter::default();
        filter.toggle(FileKind::Pdf);
        assert!((filter.find(&f("/a/b.pdf")) != Found::Nothing));
        assert!(!(filter.find(&f("/a/b.jpg")) != Found::Nothing));
        filter.toggle(FileKind::Image);
        assert!((filter.find(&f("/a/b.jpg")) != Found::Nothing));
        assert!(!(filter.find(&f("/a/b.eps")) != Found::Nothing));
        filter.text = "b".into();
        assert!(!(filter.find(&f("/a/c.jpg")) != Found::Nothing));
        filter.toggle(FileKind::Pdf);
        filter.toggle(FileKind::Image);
        assert!(filter.kinds.is_empty());
    }
}
