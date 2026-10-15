//! Il lavoro su disco: nuovo, apri, salva, salva con nome e la domanda "salvare le modifiche?"
//! prima di perderle. Il file `.winfiles` è il JSON del modello (vedi `winfiles-model`).

use std::path::{Path, PathBuf};

use egui::ViewportCommand;
use winfiles_model::{EXTENSION, Project};

use crate::app::WinFilesApp;
use crate::theme::{Tokens, semibold};

/// Cosa fare dopo aver risposto alla domanda sulle modifiche non salvate.
#[derive(Clone, Debug, PartialEq)]
pub enum Then {
    New,
    /// Apre un lavoro: quello indicato o, se manca, quello scelto nella finestra di sistema.
    Open(Option<PathBuf>),
    Close,
}

/// Messaggio per un file del lavoro che non c'è più dove era.
pub const MISSING: &str = "file non trovato: è stato spostato, rinominato o cancellato";

fn dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("Lavoro WinFiles", &[EXTENSION])
}

/// Vero per i file di progetto, da aprire invece che da aggiungere all'elenco.
pub fn is_project(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

impl WinFilesApp {
    /// Ci sono modifiche dall'ultimo salvataggio (o dall'apertura).
    pub(crate) fn is_dirty(&self) -> bool {
        self.project != self.saved
    }

    /// Il nome del lavoro: il file di progetto, o "Senza titolo".
    pub(crate) fn document_name(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_stem)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Senza titolo".into())
    }

    /// Il titolo della finestra, con un pallino quando ci sono modifiche da salvare.
    pub(crate) fn window_title(&self) -> String {
        let mark = if self.is_dirty() { " •" } else { "" };
        format!("{}{mark} — WinFiles", self.document_name())
    }

    /// Fa subito `then` se non c'è nulla da salvare, altrimenti prima chiede.
    pub(crate) fn guard(&mut self, ctx: &egui::Context, then: Then) {
        if self.is_dirty() {
            self.asking = Some(then);
        } else {
            self.run(ctx, then);
        }
    }

    fn run(&mut self, ctx: &egui::Context, then: Then) {
        match then {
            Then::New => {
                self.reset(self.blank(), None);
                self.status = "Nuovo lavoro".into();
            }
            Then::Open(path) => {
                let path = path.or_else(|| dialog().set_title("Apri un lavoro").pick_file());
                if let Some(path) = path {
                    self.open_path(&path);
                }
            }
            Then::Close => {
                self.closing = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }

    /// Apre un file di progetto. In caso di errore il lavoro aperto resta com'è.
    pub(crate) fn open_path(&mut self, path: &Path) {
        // Percorso completo: quello sulla riga di comando può essere relativo alla cartella da
        // cui si è partiti, e i lavori recenti devono ritrovarlo da ovunque.
        let path = &std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        match Project::load(path) {
            Ok(project) => {
                self.reset(project, Some(path.to_path_buf()));
                crate::recent::remember(&mut self.recent, path);
                let missing = self.errors.len();
                self.status = match missing {
                    0 => format!("Aperto {}", self.document_name()),
                    1 => format!("Aperto {}: 1 file non trovato", self.document_name()),
                    n => format!("Aperto {}: {n} file non trovati", self.document_name()),
                };
            }
            Err(e) => self.status = format!("Non riesco ad aprire {}: {e}", path.display()),
        }
    }

    /// Sostituisce il lavoro: rilegge le misure di tutti i file (possono essere cambiati da
    /// quando il lavoro è stato salvato) e segna quelli che non ci sono più.
    fn reset(&mut self, project: Project, path: Option<PathBuf>) {
        self.cancel_scans();
        self.errors.clear();
        self.thumbs.clear();
        self.unselect();
        self.sheet = 0;
        self.renaming = None;
        self.filter = Default::default();
        self.saved = project.clone();
        self.project = project;
        self.path = path;
        for f in &self.project.files {
            if f.path.is_file() {
                self.measurer.request(f.path.clone(), f.kind);
            } else {
                self.errors.insert(f.path.clone(), MISSING.into());
            }
        }
    }

    /// Salva il lavoro; con `ask`, o se non è mai stato salvato, chiede dove. Vero se è salvato.
    pub(crate) fn save(&mut self, ask: bool) -> bool {
        let path = match (&self.path, ask) {
            (Some(p), false) => p.clone(),
            _ => {
                let mut d = dialog()
                    .set_title("Salva il lavoro")
                    .set_file_name(format!("{}.{EXTENSION}", self.suggested_name()));
                if let Some(dir) = self
                    .path
                    .as_deref()
                    .and_then(Path::parent)
                    .or_else(|| self.project.files.first().map(|f| f.folder()))
                {
                    d = d.set_directory(dir);
                }
                match d.save_file() {
                    Some(p) => p,
                    None => return false,
                }
            }
        };
        let path = if is_project(&path) {
            path
        } else {
            path.with_extension(EXTENSION)
        };
        let path = std::path::absolute(&path).unwrap_or(path);
        if let Some(stem) = path.file_stem() {
            self.project.title = stem.to_string_lossy().into_owned();
        }
        match self.project.save(&path) {
            Ok(()) => {
                self.saved = self.project.clone();
                crate::recent::remember(&mut self.recent, &path);
                self.path = Some(path);
                self.status = format!("Salvato {}", self.document_name());
                true
            }
            Err(e) => {
                self.status = format!("Non riesco a salvare {}: {e}", path.display());
                false
            }
        }
    }

    /// Il nome proposto la prima volta: quello già scelto, il titolo dei fogli o la cartella.
    fn suggested_name(&self) -> String {
        if self.path.is_some() {
            return self.document_name();
        }
        let title = self.project.print.title.trim();
        let name = if title.is_empty() {
            self.project
                .files
                .first()
                .and_then(|f| f.folder().file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Lavoro".into())
        } else {
            title.to_string()
        };
        // Niente caratteri che Windows non accetta nei nomi dei file.
        name.chars()
            .map(|c| if "\\/:*?\"<>|".contains(c) { '-' } else { c })
            .collect()
    }

    /// La domanda "salvare le modifiche?", quando serve.
    pub(crate) fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        let Some(then) = self.asking.clone() else {
            return;
        };
        let t = Tokens::get(ctx);
        let name = self.document_name();
        let mut answer = None;
        let modal = egui::Modal::new(egui::Id::new("salvare-modifiche")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(
                egui::RichText::new(format!("Salvare le modifiche a «{name}»?"))
                    .font(semibold(15.0))
                    .color(t.text_strong),
            );
            ui.add_space(6.0);
            ui.label("Se non salvi, l'elenco, le note e l'impaginazione cambiati andranno persi.");
            ui.add_space(14.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 36.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    let save = egui::Button::new(
                        egui::RichText::new("Salva")
                            .font(semibold(13.0))
                            .color(t.on_primary),
                    )
                    .fill(t.primary)
                    .min_size(egui::vec2(90.0, 30.0));
                    if ui.add(save).clicked() {
                        answer = Some(true);
                    }
                    if ui
                        .add(egui::Button::new("Non salvare").min_size(egui::vec2(100.0, 30.0)))
                        .clicked()
                    {
                        answer = Some(false);
                    }
                    if ui
                        .add(egui::Button::new("Annulla").min_size(egui::vec2(80.0, 30.0)))
                        .clicked()
                    {
                        self.asking = None;
                    }
                },
            );
        });
        if modal.should_close() && answer.is_none() {
            self.asking = None;
        }
        match answer {
            Some(true) => {
                self.asking = None;
                if self.save(false) {
                    self.run(ctx, then);
                }
            }
            Some(false) => {
                self.asking = None;
                self.run(ctx, then);
            }
            None => {}
        }
    }
}
