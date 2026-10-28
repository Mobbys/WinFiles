//! Misura dei file in un thread a parte: l'interfaccia resta fluida anche con migliaia di file o
//! con PDF da centinaia di megabyte su una cartella di rete.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use winfiles_formats::{Probe, ProbeError, probe};
use winfiles_model::FileKind;

/// Il risultato di una misura.
pub struct Measured {
    pub path: PathBuf,
    pub result: Result<Probe, ProbeError>,
}

/// Coda di lavoro verso il thread che misura i file.
pub struct Measurer {
    jobs: Option<Sender<(PathBuf, FileKind)>>,
    results: Receiver<Measured>,
    /// Richieste inviate e non ancora tornate.
    pending: usize,
}

impl Measurer {
    /// Avvia il thread. `wake` sveglia l'interfaccia quando arriva un risultato.
    pub fn start(wake: impl Fn() + Send + 'static) -> Self {
        let (jobs, inbox) = channel::<(PathBuf, FileKind)>();
        let (outbox, results) = channel();
        let spawned = std::thread::Builder::new().name("misure".into()).spawn(move || {
            for (path, kind) in inbox {
                let result = probe(&path, kind);
                if outbox.send(Measured { path, result }).is_err() {
                    break;
                }
                wake();
            }
        });
        Self {
            jobs: spawned.is_ok().then_some(jobs),
            results,
            pending: 0,
        }
    }

    /// Chiede la misura di un file. Senza thread (non partito) la richiesta si perde e il file
    /// resta "da misurare": l'app continua a funzionare con le proporzioni provvisorie. I file
    /// di tipo non supportato non hanno misure: non si chiedono.
    pub fn request(&mut self, path: PathBuf, kind: FileKind) {
        if kind != FileKind::Unsupported
            && let Some(jobs) = &self.jobs
            && jobs.send((path, kind)).is_ok()
        {
            self.pending += 1;
        }
    }

    /// I risultati arrivati finora.
    pub fn drain(&mut self) -> Vec<Measured> {
        let out: Vec<Measured> = self.results.try_iter().collect();
        self.pending = self.pending.saturating_sub(out.len());
        out
    }

    pub fn pending(&self) -> usize {
        self.pending
    }
}
