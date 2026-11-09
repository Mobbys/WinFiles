//! L'esportazione in PDF e la stampa, in un thread a parte: le scene dei fogli si compongono
//! prima, sul thread dell'interfaccia (servono i caratteri di egui), poi `winfiles_pdf_out` le
//! scrive mentre l'interfaccia resta libera e mostra a che foglio è arrivato. Per la stampa lo
//! stesso PDF, senza passare dal disco, va allo spooler con `craft-print` alla misura vera.

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

/// Dove vanno i fogli.
#[derive(Clone, Debug)]
pub enum Target {
    File(PathBuf),
    Printer(craft_print::Job),
}

/// A che punto è il lavoro.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Si compone il PDF: `done` fogli su `total`.
    Building,
    /// Il PDF è pronto e va alla stampante.
    Sending,
}

/// Com'è andata: il resoconto del PDF e, per la stampa, la stampante che ha preso il lavoro.
pub type Outcome = Result<Report, Failure>;

/// Perché non è andata.
#[derive(Debug)]
pub enum Failure {
    Pdf(PdfOutError),
    Print(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Pdf(e) => e.fmt(f),
            Failure::Print(e) => f.write_str(e),
        }
    }
}

/// Un'esportazione o una stampa in corso.
pub struct Export {
    pub target: Target,
    pub total: usize,
    done: Arc<AtomicUsize>,
    sending: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Outcome>,
}

impl Export {
    /// Scrive `pages` nel PDF e lo manda a `target`; `wake` ridisegna l'interfaccia a ogni
    /// foglio.
    pub fn start(
        pages: Vec<Page>,
        target: Target,
        wake: impl Fn() + Send + 'static,
    ) -> std::io::Result<Self> {
        let done = Arc::new(AtomicUsize::new(0));
        let sending = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let total = pages.len();
        let (d, s, c, t) = (done.clone(), sending.clone(), cancel.clone(), target.clone());
        std::thread::Builder::new()
            .name("esporta-pdf".into())
            .spawn(move || {
                let built = winfiles_pdf_out::write(&pages, &FONTS, &c, |n| {
                    d.store(n, Ordering::Relaxed);
                    wake();
                })
                .map_err(Failure::Pdf);
                let result = built.and_then(|(bytes, report)| {
                    match &t {
                        Target::File(path) => winfiles_pdf_out::save(path, &bytes).map_err(Failure::Pdf)?,
                        Target::Printer(job) => {
                            s.store(true, Ordering::Relaxed);
                            d.store(0, Ordering::Relaxed);
                            wake();
                            craft_print::submit_with_progress(&bytes, job, &mut |n, _| {
                                d.store(n, Ordering::Relaxed);
                                wake();
                            })
                            .map_err(|e| Failure::Print(e.to_string()))?;
                        }
                    }
                    Ok(report)
                });
                // Se l'interfaccia è già chiusa non c'è più nessuno a cui dirlo.
                let _ = tx.send(result);
                wake();
            })?;
        Ok(Self {
            target,
            total,
            done,
            sending,
            cancel,
            rx,
        })
    }

    /// Fogli già fatti nella fase in corso.
    pub fn done(&self) -> usize {
        self.done.load(Ordering::Relaxed)
    }

    pub fn stage(&self) -> Stage {
        if self.sending.load(Ordering::Relaxed) {
            Stage::Sending
        } else {
            Stage::Building
        }
    }

    /// Si può ancora fermare: lo spooler, una volta ricevuto il lavoro, non si ferma da qui.
    pub fn can_cancel(&self) -> bool {
        self.stage() == Stage::Building
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Il risultato, quando il thread ha finito.
    pub fn poll(&self) -> Option<Outcome> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(Failure::Pdf(PdfOutError::Write(
                "il lavoro si è interrotto".into(),
            )))),
        }
    }

    /// Il testo per la barra di stato mentre lavora.
    pub fn progress_text(&self) -> String {
        match (&self.target, self.stage()) {
            (Target::File(_), _) => format!("Esporto il PDF… foglio {} di {}", self.done(), self.total),
            (Target::Printer(_), Stage::Building) => {
                format!("Preparo la stampa… foglio {} di {}", self.done(), self.total)
            }
            (Target::Printer(_), Stage::Sending) => {
                format!("Invio alla stampante… foglio {} di {}", self.done(), self.total)
            }
        }
    }
}

/// Il nome della stampante per i messaggi.
pub fn printer_name(job: &craft_print::Job) -> String {
    job.printer
        .clone()
        .unwrap_or_else(|| "stampante predefinita".into())
}

/// Il messaggio per la barra di stato a stampa finita.
pub fn print_outcome(job: &craft_print::Job, sheets: usize, result: &Outcome) -> String {
    let printer = printer_name(job);
    match result {
        Ok(report) => {
            let what = match sheets {
                1 => "1 foglio".to_string(),
                n => format!("{n} fogli"),
            };
            let copies = if job.copies > 1 {
                format!(", {} copie", job.copies)
            } else {
                String::new()
            };
            let mut text = format!("Inviato a {printer}: {what}{copies}");
            if let Some((file, _)) = report.warnings.first() {
                text = format!(
                    "{text}; {} non disegnato{}",
                    file_name(file),
                    match report.warnings.len() {
                        1 => String::new(),
                        n => format!(" e altri {}", n - 1),
                    }
                );
            }
            text
        }
        Err(Failure::Pdf(PdfOutError::Cancelled)) => "Stampa annullata".into(),
        Err(e) => format!("Stampa su {printer} non riuscita: {e}"),
    }
}

/// Il messaggio per la barra di stato a esportazione finita.
pub fn outcome(path: &Path, result: &Outcome) -> String {
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
        Err(Failure::Pdf(PdfOutError::Cancelled)) => "Esportazione annullata".into(),
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
            outcome(path, &Err(Failure::Pdf(PdfOutError::Cancelled))),
            "Esportazione annullata"
        );
    }

    #[test]
    fn messaggi_di_fine_stampa() {
        let job = craft_print::Job {
            printer: Some("Ufficio A3".into()),
            copies: 2,
            ..Default::default()
        };
        let ok = Ok(Report {
            pages: 3,
            warnings: Vec::new(),
        });
        assert_eq!(
            print_outcome(&job, 3, &ok),
            "Inviato a Ufficio A3: 3 fogli, 2 copie"
        );
        let failed = Err(Failure::Print("carta A3 non disponibile".into()));
        assert_eq!(
            print_outcome(&job, 3, &failed),
            "Stampa su Ufficio A3 non riuscita: carta A3 non disponibile"
        );
        let default = craft_print::Job::default();
        assert_eq!(
            print_outcome(&default, 1, &Ok(Report::default())),
            "Inviato a stampante predefinita: 1 foglio"
        );
    }

    #[test]
    fn esporta_e_si_annulla() {
        let dir = std::env::temp_dir().join(format!("winfiles-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prova.pdf");
        let pages = vec![Page::new(winfiles_geom::Size::new(210.0, 297.0)); 2];
        let job = Export::start(pages, Target::File(path.clone()), || {}).unwrap();
        let result = loop {
            if let Some(r) = job.poll() {
                break r;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(result.unwrap().pages, 2);
        assert_eq!(job.stage(), Stage::Building);
        assert_eq!(job.done(), 2);
        assert!(std::fs::read(&path).unwrap().starts_with(b"%PDF-"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
