//! Le immagini delle pagine del modulo PDF, disegnate in un thread a parte: il documento si
//! scrive una volta per ogni modifica e se ne disegnano le pagine che mancano. Una richiesta più
//! nuova fa saltare quelle vecchie ancora in coda.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use winfiles_pdf_edit::Snapshot;
use winfiles_thumbs::{PdfPages, Thumb};

/// Un'immagine: l'impronta della pagina e il lato lungo in pixel.
pub type Key = (u64, u32);

struct Job {
    generation: u64,
    doc: Snapshot,
    /// Pagina (indice nel documento) e immagine da fare.
    pages: Vec<(usize, Key)>,
}

struct Done {
    key: Key,
    result: Result<Thumb, String>,
}

pub enum Image {
    Ready(egui::TextureHandle),
    Failed(String),
}

pub struct Renderer {
    jobs: Option<Sender<Job>>,
    results: Receiver<Done>,
    latest: Arc<AtomicU64>,
    images: HashMap<Key, Image>,
    /// Chieste e non ancora arrivate, con la versione del documento per cui si sono chieste:
    /// quelle chieste per una versione vecchia il thread può averle saltate.
    pending: HashMap<Key, u64>,
}

impl Renderer {
    pub fn start(wake: impl Fn() + Send + 'static) -> Self {
        let (jobs, inbox) = channel::<Job>();
        let (outbox, results) = channel();
        let latest = Arc::new(AtomicU64::new(0));
        let current = Arc::clone(&latest);
        let spawned = std::thread::Builder::new()
            .name("pagine-pdf".into())
            .spawn(move || {
                while let Ok(mut job) = inbox.recv() {
                    // Solo l'ultima richiesta in coda conta.
                    while let Ok(next) = inbox.try_recv() {
                        job = next;
                    }
                    let pdf = job
                        .doc
                        .preview_bytes()
                        .map_err(|e| e.to_string())
                        .and_then(|b| PdfPages::open(b).map_err(|e| e.to_string()));
                    for (page, key) in job.pages {
                        if current.load(Ordering::Relaxed) != job.generation {
                            break;
                        }
                        let result = match &pdf {
                            Ok(p) => p.render(page, key.1).map_err(|e| e.to_string()),
                            Err(e) => Err(e.clone()),
                        };
                        if outbox.send(Done { key, result }).is_err() {
                            return;
                        }
                        wake();
                    }
                }
            });
        Self {
            jobs: spawned.is_ok().then_some(jobs),
            results,
            latest,
            images: HashMap::new(),
            pending: HashMap::new(),
        }
    }

    /// Chiede le immagini che mancano. `generation` cresce a ogni modifica del documento.
    pub fn request(&mut self, generation: u64, doc: impl FnOnce() -> Snapshot, wanted: &[(usize, Key)]) {
        let missing: Vec<(usize, Key)> = wanted
            .iter()
            .filter(|(_, k)| !self.images.contains_key(k) && self.pending.get(k) != Some(&generation))
            .copied()
            .collect();
        if missing.is_empty() {
            return;
        }
        let Some(jobs) = &self.jobs else {
            return;
        };
        self.latest.store(generation, Ordering::Relaxed);
        // Quello che era in coda per una versione vecchia si richiede con questa.
        let mut pages: Vec<(usize, Key)> = Vec::new();
        for (i, k) in wanted {
            if !self.images.contains_key(k) {
                pages.push((*i, *k));
                self.pending.insert(*k, generation);
            }
        }
        let _ = jobs.send(Job {
            generation,
            doc: doc(),
            pages,
        });
    }

    pub fn receive(&mut self, ctx: &egui::Context) {
        while let Ok(done) = self.results.try_recv() {
            self.pending.remove(&done.key);
            let image = match done.result {
                Ok(t) => {
                    let img = egui::ColorImage::from_rgba_unmultiplied(
                        [t.width as usize, t.height as usize],
                        &t.rgba,
                    );
                    Image::Ready(ctx.load_texture(
                        format!("pdf-{}-{}", done.key.0, done.key.1),
                        img,
                        egui::TextureOptions::LINEAR,
                    ))
                }
                Err(e) => Image::Failed(e),
            };
            self.images.insert(done.key, image);
        }
    }

    pub fn get(&self, key: Key) -> Option<&Image> {
        self.images.get(&key)
    }

    /// Tiene solo le immagini delle pagine che ci sono ancora (e quelle in arrivo).
    pub fn keep(&mut self, keys: &HashSet<u64>) {
        self.images.retain(|k, _| keys.contains(&k.0));
        self.pending.retain(|k, _| keys.contains(&k.0));
    }

    /// Dimentica tutto: un altro documento.
    pub fn clear(&mut self) {
        self.images.clear();
        self.pending.clear();
    }
}
