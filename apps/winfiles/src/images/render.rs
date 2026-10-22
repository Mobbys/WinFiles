//! Le immagini da mostrare, ridotte in un thread a parte: una foto da cento megapixel non si
//! rimpicciolisce nel frattempo che si disegna la finestra. L'immagine viaggia dentro un `Arc`,
//! quindi non si copia; una richiesta più nuova fa saltare quelle vecchie ancora in coda.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use image::DynamicImage;

/// Quale immagine: il file aperto, la sua versione e il lato lungo in pixel.
pub type Key = (usize, u64, u32);

struct Job {
    key: Key,
    image: Arc<DynamicImage>,
}

struct Done {
    key: Key,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

pub struct Renderer {
    jobs: Option<Sender<Job>>,
    results: Receiver<Done>,
    /// Le immagini e le versioni che servono ancora: il thread salta il lavoro per le altre.
    live: Arc<Mutex<Vec<(usize, u64)>>>,
    images: HashMap<Key, egui::TextureHandle>,
    pending: Vec<Key>,
}

impl Renderer {
    pub fn start(wake: impl Fn() + Send + 'static) -> Self {
        let (jobs, inbox) = channel::<Job>();
        let (outbox, results) = channel();
        let live = Arc::new(Mutex::new(Vec::new()));
        let current = Arc::clone(&live);
        let spawned = std::thread::Builder::new()
            .name("immagini".into())
            .spawn(move || {
                while let Ok(job) = inbox.recv() {
                    // Il lavoro per un'immagine chiusa o per una versione superata si salta.
                    let wanted = (job.key.0, job.key.1);
                    if current.lock().is_ok_and(|live| !live.contains(&wanted)) {
                        continue;
                    }
                    let side = job.key.2;
                    let small = if job.image.width() > side || job.image.height() > side {
                        job.image.thumbnail(side, side)
                    } else {
                        job.image.as_ref().clone()
                    };
                    let rgba = small.to_rgba8();
                    let done = Done {
                        key: job.key,
                        width: rgba.width(),
                        height: rgba.height(),
                        rgba: rgba.into_raw(),
                    };
                    if outbox.send(done).is_err() {
                        return;
                    }
                    wake();
                }
            });
        Self {
            jobs: spawned.is_ok().then_some(jobs),
            results,
            live,
            images: HashMap::new(),
            pending: Vec::new(),
        }
    }

    /// Chiede l'immagine se non c'è già e non è già in coda.
    pub fn request(&mut self, key: Key, image: impl FnOnce() -> Arc<DynamicImage>) {
        if self.images.contains_key(&key) || self.pending.contains(&key) {
            return;
        }
        let Some(jobs) = &self.jobs else {
            return;
        };
        if let Ok(mut live) = self.live.lock() {
            let wanted = (key.0, key.1);
            if !live.contains(&wanted) {
                live.push(wanted);
            }
        }
        self.pending.push(key);
        let _ = jobs.send(Job { key, image: image() });
    }

    pub fn receive(&mut self, ctx: &egui::Context) {
        while let Ok(done) = self.results.try_recv() {
            self.pending.retain(|k| *k != done.key);
            let img = egui::ColorImage::from_rgba_unmultiplied(
                [done.width as usize, done.height as usize],
                &done.rgba,
            );
            let name = format!("img-{}-{}-{}", done.key.0, done.key.1, done.key.2);
            let tex = ctx.load_texture(name, img, egui::TextureOptions::LINEAR);
            self.images.insert(done.key, tex);
        }
    }

    pub fn get(&self, key: Key) -> Option<&egui::TextureHandle> {
        self.images.get(&key)
    }

    /// La versione più vicina a quella chiesta, da mostrare mentre si aspetta quella giusta.
    pub fn nearest(&self, doc: usize, generation: u64) -> Option<&egui::TextureHandle> {
        self.images
            .iter()
            .filter(|(k, _)| k.0 == doc && k.1 == generation)
            .max_by_key(|(k, _)| k.2)
            .map(|(_, v)| v)
    }

    /// Butta via le immagini delle versioni vecchie e dei file chiusi.
    pub fn keep(&mut self, wanted: &[(usize, u64)]) {
        self.images.retain(|k, _| wanted.contains(&(k.0, k.1)));
        self.pending.retain(|k| wanted.contains(&(k.0, k.1)));
        if let Ok(mut live) = self.live.lock() {
            live.retain(|k| wanted.contains(k));
        }
    }
}
