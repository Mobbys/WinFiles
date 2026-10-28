//! Miniature per l'anteprima: si chiedono solo per il foglio visibile, si preparano in due thread
//! e si tengono come texture in una cache con un tetto, così anche 20.000 file non riempiono la
//! memoria.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use winfiles_model::FileKind;
use winfiles_thumbs::{Thumb, render};

/// Lato lungo delle miniature, in pixel.
const SIDE: u32 = 512;
/// Texture tenute in memoria al massimo (circa 1 MB l'una).
const MAX_CACHED: usize = 600;
const WORKERS: usize = 2;

/// Lo stato della miniatura di un file.
pub enum ThumbState {
    Loading,
    Ready(egui::TextureHandle),
    Failed(String),
}

/// Una miniatura: file e pagina (da 0).
type Key = (PathBuf, usize);

struct Done {
    key: Key,
    result: Result<Thumb, String>,
}

pub struct Thumbnails {
    jobs: Option<Sender<(Key, FileKind)>>,
    results: Receiver<Done>,
    cache: HashMap<Key, (ThumbState, u64)>,
    /// Contatore per sapere quale miniatura è stata usata meno di recente.
    clock: u64,
}

impl Thumbnails {
    pub fn start(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (jobs, inbox) = channel::<(Key, FileKind)>();
        let (outbox, results) = channel();
        let inbox = Arc::new(Mutex::new(inbox));
        let wake = Arc::new(wake);
        let mut started = false;
        for n in 0..WORKERS {
            let (inbox, outbox, wake) = (Arc::clone(&inbox), outbox.clone(), Arc::clone(&wake));
            let spawned = std::thread::Builder::new()
                .name(format!("miniature-{n}"))
                .spawn(move || {
                    loop {
                        // Un lucchetto avvelenato o un canale chiuso fermano il thread, non l'app.
                        let job = match inbox.lock() {
                            Ok(rx) => rx.recv(),
                            Err(_) => break,
                        };
                        let Ok((key, kind)) = job else { break };
                        let result = render(&key.0, kind, key.1, SIDE).map_err(|e| e.to_string());
                        if outbox.send(Done { key, result }).is_err() {
                            break;
                        }
                        wake();
                    }
                });
            started |= spawned.is_ok();
        }
        Self {
            jobs: started.then_some(jobs),
            results,
            cache: HashMap::new(),
            clock: 0,
        }
    }

    /// La miniatura della pagina `page` di `path`, chiesta ai thread se non c'è ancora.
    pub fn get(&mut self, path: &Path, kind: FileKind, page: usize) -> &ThumbState {
        self.clock += 1;
        let clock = self.clock;
        let key = (path.to_path_buf(), page);
        if !self.cache.contains_key(&key) {
            let state = match &self.jobs {
                Some(jobs) if jobs.send((key.clone(), kind)).is_ok() => ThumbState::Loading,
                _ => ThumbState::Failed("miniature non disponibili".into()),
            };
            self.cache.insert(key.clone(), (state, clock));
        }
        match self.cache.get_mut(&key) {
            Some(entry) => {
                entry.1 = clock;
                &entry.0
            }
            None => &ThumbState::Loading,
        }
    }

    /// Trasforma in texture le miniature arrivate e libera le più vecchie oltre il tetto.
    pub fn receive(&mut self, ctx: &egui::Context) {
        for done in self.results.try_iter() {
            let state = match done.result {
                Ok(t) => match image_of(&t) {
                    Some(img) => ThumbState::Ready(ctx.load_texture(
                        format!("{}#{}", done.key.0.display(), done.key.1),
                        img,
                        egui::TextureOptions::LINEAR,
                    )),
                    None => ThumbState::Failed("miniatura non valida".into()),
                },
                Err(e) => ThumbState::Failed(e),
            };
            // Una miniatura arrivata dopo essere uscita dalla cache si scarta.
            if let Some(entry) = self.cache.get_mut(&done.key) {
                entry.0 = state;
            }
        }
        if self.cache.len() > MAX_CACHED {
            let mut ages: Vec<(u64, Key)> = self
                .cache
                .iter()
                .filter(|(_, (s, _))| !matches!(s, ThumbState::Loading))
                .map(|(k, (_, t))| (*t, k.clone()))
                .collect();
            ages.sort();
            let excess = self.cache.len() - MAX_CACHED;
            for (_, p) in ages.into_iter().take(excess) {
                self.cache.remove(&p);
            }
        }
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }
}

fn image_of(t: &Thumb) -> Option<egui::ColorImage> {
    let w = usize::try_from(t.width).ok()?;
    let h = usize::try_from(t.height).ok()?;
    (t.rgba.len() == w.checked_mul(h)?.checked_mul(4)?)
        .then(|| egui::ColorImage::from_rgba_unmultiplied([w, h], &t.rgba))
}
