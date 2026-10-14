//! L'esportazione in PDF in un thread a parte: le scene dei fogli si compongono prima, sul thread
//! dell'interfaccia (servono i caratteri di egui), poi `winfiles_pdf_out` le scrive mentre
//! l'interfaccia resta libera e mostra a che foglio è arrivato.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use winfiles_pdf_out::{Fonts, PdfOutError, Report};
use winfiles_scene::Page;

use crate::theme::{INTER_REGULAR, INTER_SEMIBOLD};

const FONTS: Fonts = Fonts {
    regular: INTER_REGULAR,
    semibold: INTER_SEMIBOLD,
};

/// Un'esportazione in corso.
pub struct Export {
    pub path: PathBuf,
    pub total: usize,
    done: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Result<Report, PdfOutError>>,
}

impl Export {
    /// Scrive `pages` in `path`; `wake` ridisegna l'interfaccia a ogni foglio.
    pub fn start(pages: Vec<Page>, path: PathBuf, wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let done = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let total = pages.len();
        let (d, c, p) = (done.clone(), cancel.clone(), path.clone());
        std::thread::Builder::new()
            .name("esporta-pdf".into())
            .spawn(move || {
                let result = winfiles_pdf_out::write(&pages, &FONTS, &c, |n| {
                    d.store(n, Ordering::Relaxed);
                    wake();
                })
                .and_then(|(bytes, report)| {
                    winfiles_pdf_out::save(&p, &bytes)?;
                    Ok(report)
                });
                // Se l'interfaccia è già chiusa non c'è più nessuno a cui dirlo.
                let _ = tx.send(result);
                wake();
            })?;
        Ok(Self {
            path,
            total,
            done,
            cancel,
            rx,
        })
    }

    /// Fogli già scritti.
    pub fn done(&self) -> usize {
        self.done.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Il risultato, quando il thread ha finito.
    pub fn poll(&self) -> Option<Result<Report, PdfOutError>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(PdfOutError::Write("il lavoro si è interrotto".into())))
            }
        }
    }
}

/// Il messaggio per la barra di stato a esportazione finita.
pub fn outcome(path: &Path, result: &Result<Report, PdfOutError>) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match result {
        Ok(report) => {
            let sheets = match report.pages {
                1 => "1 foglio".to_string(),
                n => format!("{n} fogli"),
            };
            match report.warnings.as_slice() {
                [] => format!("Esportato {name}: {sheets}"),
                [(file, why)] => format!(
                    "Esportato {name}: {sheets}; {} non disegnato ({why})",
                    file_name(file)
                ),
                [(file, _), rest @ ..] => format!(
                    "Esportato {name}: {sheets}; non disegnati {} e altri {}",
                    file_name(file),
                    rest.len()
                ),
            }
        }
        Err(PdfOutError::Cancelled) => "Esportazione annullata".into(),
        Err(e) => format!("Esportazione di {name} non riuscita: {e}"),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messaggi_di_fine_esportazione() {
        let path = Path::new("/lavori/Ordine 124.pdf");
        let ok = Ok(Report {
            pages: 3,
            warnings: Vec::new(),
        });
        assert_eq!(outcome(path, &ok), "Esportato Ordine 124.pdf: 3 fogli");
        let one = Ok(Report {
            pages: 1,
            warnings: vec![(PathBuf::from("/a/logo.pdf"), "file non trovato".into())],
        });
        assert_eq!(
            outcome(path, &one),
            "Esportato Ordine 124.pdf: 1 foglio; logo.pdf non disegnato (file non trovato)"
        );
        assert_eq!(
            outcome(path, &Err(PdfOutError::Cancelled)),
            "Esportazione annullata"
        );
    }

    #[test]
    fn esporta_e_si_annulla() {
        let dir = std::env::temp_dir().join(format!("winfiles-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prova.pdf");
        let pages = vec![Page::new(winfiles_geom::Size::new(210.0, 297.0)); 2];
        let job = Export::start(pages, path.clone(), || {}).unwrap();
        let result = loop {
            if let Some(r) = job.poll() {
                break r;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(result.unwrap().pages, 2);
        assert_eq!(job.done(), 2);
        assert!(std::fs::read(&path).unwrap().starts_with(b"%PDF-"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
