//! Il lavoro su disco: nuovo, apri, salva, salva con nome e la domanda "salvare le modifiche?"
//! prima di perderle. Il file `.winfiles` è il JSON del modello (vedi `winfiles-model`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use egui::ViewportCommand;
use winfiles_model::folder_notes::{FolderNotes, NotesJob};
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

/// Uno dei lavori da riaprire col doppio clic sul file delle note di una cartella.
#[derive(Clone, Debug, PartialEq)]
pub struct JobChoice {
    /// I file da aprire, nell'ordine del lavoro.
    pub files: Vec<PathBuf>,
    /// Il lavoro com'era (impaginazione); `None` per i file con note fuori dai lavori.
    pub job: Option<NotesJob>,
    /// Quanti di questi file hanno note.
    pub noted: usize,
}

/// Più lavori nello stesso file delle note: si chiede quale aprire.
#[derive(Clone, Debug, PartialEq)]
pub struct NotesPick {
    pub dir: PathBuf,
    pub options: Vec<JobChoice>,
}

/// Cosa aprire per i percorsi dati (riga di comando, trascinati, "Apri in WinFiles").
#[derive(Debug, Default, PartialEq)]
pub struct Opening {
    /// File, cartelle e lavori `.winfiles`.
    pub paths: Vec<PathBuf>,
    /// Il lavoro delle note da rimettere com'era: impaginazione e ordine dei file.
    pub restore: Option<(NotesJob, Vec<PathBuf>)>,
    /// Più lavori nel file delle note: la domanda.
    pub pick: Option<NotesPick>,
}

/// I lavori che il file delle note di `dir` può riaprire: ognuno di quelli ricordati (con file
/// che ci sono ancora), più i file con note che non stanno in nessuno (note scritte prima dei
/// lavori, o da Visualizza).
pub fn job_choices(notes: &FolderNotes, dir: &Path) -> Vec<JobChoice> {
    let annotated = notes.annotated_files(dir);
    let noted_files = notes.noted_files(dir);
    let mut out: Vec<JobChoice> = Vec::new();
    let mut covered: BTreeSet<PathBuf> = BTreeSet::new();
    for job in notes.jobs() {
        let files: Vec<PathBuf> = job.paths(dir).into_iter().filter(|p| p.is_file()).collect();
        if files.is_empty() {
            continue;
        }
        let noted = files.iter().filter(|f| noted_files.contains(*f)).count();
        // Un lavoro da cui sono state tolte tutte le note non si propone più.
        if noted == 0 {
            continue;
        }
        covered.extend(files.iter().cloned());
        out.push(JobChoice {
            files,
            job: Some(job),
            noted,
        });
    }
    let others: Vec<PathBuf> = annotated.into_iter().filter(|f| !covered.contains(f)).collect();
    if !others.is_empty() {
        out.push(JobChoice {
            noted: others.iter().filter(|f| noted_files.contains(*f)).count(),
            files: others,
            job: None,
        });
    }
    out
}

/// Il titolo e la riga sotto di una scelta della domanda "quale lavoro apro?": i primi nomi, i
/// file, quanti hanno note, quando e com'era impaginato.
pub fn choice_text(c: &JobChoice, now: u64) -> (String, String) {
    let names: Vec<String> = c
        .files
        .iter()
        .take(3)
        .filter_map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    let mut title = names.join(", ");
    if c.files.len() > names.len() {
        title.push_str(&format!(" e altri {}", c.files.len() - names.len()));
    }
    let files = match c.files.len() {
        1 => "1 file".to_owned(),
        n => format!("{n} file"),
    };
    let noted = match c.noted {
        1 => "1 con note".to_owned(),
        n => format!("{n} con note"),
    };
    let detail = match &c.job {
        Some(j) => {
            let days = now.saturating_sub(j.data) / 86_400;
            let when = match (j.data, days) {
                (0, _) => String::new(),
                (_, 0) => " · scritte oggi".to_owned(),
                (_, 1) => " · scritte ieri".to_owned(),
                (_, d) => format!(" · scritte {d} giorni fa"),
            };
            let verso = match j.verso {
                winfiles_geom::Orientation::Portrait => "verticale",
                winfiles_geom::Orientation::Landscape => "orizzontale",
            };
            format!("{files}, {noted}{when} · {} {verso}", j.foglio.name())
        }
        None => format!("{files}, {noted} · note scritte fuori da un lavoro"),
    };
    (title, detail)
}

/// Il file delle note di una cartella (`note.winfiles`) riapre il lavoro in cui sono state
/// scritte le note, con gli stessi file nello stesso ordine e la stessa impaginazione; se i
/// lavori sono più d'uno chiede quale. Senza lavori ricordati (note di versioni di prima) apre i
/// file con note; se nessun file ne ha (solo la nota di cartella), tutta la cartella.
pub fn notes_to_folders(paths: Vec<PathBuf>) -> Opening {
    use winfiles_model::folder_notes::is_notes_file;
    let mut opening = Opening::default();
    let add = |out: &mut Vec<PathBuf>, f: PathBuf| {
        if !out.contains(&f) {
            out.push(f);
        }
    };
    for p in paths {
        if !is_notes_file(&p) {
            add(&mut opening.paths, p);
            continue;
        }
        let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) else {
            continue;
        };
        let notes = FolderNotes::read(dir).ok().flatten().unwrap_or_default();
        let mut options = job_choices(&notes, dir);
        if options.len() > 1 && opening.pick.is_none() {
            opening.pick = Some(NotesPick {
                dir: dir.to_path_buf(),
                options,
            });
            continue;
        }
        // Un lavoro solo (o la domanda già fatta per un'altra cartella): il primo, il più recente.
        if options.is_empty() {
            add(&mut opening.paths, dir.to_path_buf());
            continue;
        }
        let first = options.swap_remove(0);
        for f in &first.files {
            add(&mut opening.paths, f.clone());
        }
        if let Some(job) = first.job
            && opening.restore.is_none()
        {
            opening.restore = Some((job, first.files));
        }
    }
    opening
}

impl WinFilesApp {
    /// Apre i percorsi dati: i lavori `.winfiles` (il primo; con `guard` chiedendo prima di
    /// perdere le modifiche), gli altri file e cartelle nell'elenco. Il lavoro di un file delle
    /// note torna com'era se l'elenco è vuoto; se i lavori sono più d'uno si chiede quale.
    pub(crate) fn open_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>, guard: bool) {
        let opening = notes_to_folders(paths);
        let (projects, files): (Vec<PathBuf>, Vec<PathBuf>) =
            opening.paths.into_iter().partition(|p| is_project(p));
        if let Some(p) = projects.into_iter().next() {
            if guard {
                self.guard(ctx, Then::Open(Some(p)));
            } else {
                self.open_path(&p);
            }
        }
        if let Some((job, order)) = opening.restore {
            self.restore_later(job, order);
        }
        if !files.is_empty() {
            self.add_paths(files);
        }
        if opening.pick.is_some() {
            self.notes_pick = opening.pick;
        }
    }

    /// Il lavoro delle note si rimette com'era quando arrivano i file, ma solo in un elenco vuoto:
    /// aggiunto a un lavoro già aperto non ne cambia l'impaginazione.
    fn restore_later(&mut self, job: NotesJob, order: Vec<PathBuf>) {
        if self.project.files.is_empty() && self.scans.is_empty() {
            self.restoring = Some((job, order));
        }
    }

    /// Rimette il lavoro delle note com'era: i file nell'ordine di allora e foglio, verso e
    /// colonne di allora, senza "Su N fogli" (con meno o più file ricalcolerebbe e girerebbe il
    /// foglio). Da chiamare quando i suoi file sono arrivati.
    pub(crate) fn restore_job(&mut self) {
        let Some((job, order)) = self.restoring.take() else {
            return;
        };
        let rank: std::collections::HashMap<&Path, usize> =
            order.iter().enumerate().map(|(i, p)| (p.as_path(), i)).collect();
        // Stabile: i file che non erano nel lavoro restano in fondo, nel loro ordine.
        self.project
            .files
            .sort_by_key(|f| rank.get(f.path.as_path()).copied().unwrap_or(usize::MAX));
        let p = &mut self.project.print;
        p.view = winfiles_model::View::Grid;
        p.paper = job.foglio;
        p.orientation = job.verso;
        p.columns = job.colonne.clamp(1, winfiles_layout::MAX_COLUMNS);
        p.fit = false;
        p.one_per_sheet = job.una_per_foglio;
        if job.tutta_pagina {
            p.fill_single = true;
        } else if self.project.files.len() == 1 {
            p.fill_single = false;
        }
        let verso = match job.verso {
            winfiles_geom::Orientation::Portrait => "verticale",
            winfiles_geom::Orientation::Landscape => "orizzontale",
        };
        self.sheet = 0;
        let columns = match p.columns {
            1 => "1 colonna".to_owned(),
            n => format!("{n} colonne"),
        };
        self.status = format!(
            "Riaperto il lavoro delle note come quando sono state scritte: {} file, {} {verso}, {columns}",
            self.project.files.len(),
            job.foglio.name(),
        );
    }

    /// La domanda "quale lavoro apro?" quando il file delle note ne ricorda più d'uno.
    pub(crate) fn notes_pick_dialog(&mut self, ctx: &egui::Context) {
        let Some(pick) = self.notes_pick.clone() else {
            return;
        };
        let t = Tokens::get(ctx);
        let mut chosen: Option<JobChoice> = None;
        let folder = pick
            .dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| pick.dir.display().to_string());
        let modal = egui::Modal::new(egui::Id::new("quale-lavoro-delle-note")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(
                egui::RichText::new(format!("Quale lavoro di «{folder}» apro?"))
                    .font(semibold(15.0))
                    .color(t.text_strong),
            );
            ui.add_space(6.0);
            ui.label("In questa cartella le note sono state scritte in più lavori. Si riapre com'era, con la stessa impaginazione.");
            ui.add_space(10.0);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for option in &pick.options {
                    let (title, detail) = choice_text(option, now);
                    let text = egui::text::LayoutJob::default();
                    let mut job = text;
                    job.append(
                        &title,
                        0.0,
                        egui::TextFormat::simple(semibold(13.0), t.text_strong),
                    );
                    job.append(
                        &format!("\n{detail}"),
                        0.0,
                        egui::TextFormat::simple(egui::FontId::proportional(12.0), t.text_dim),
                    );
                    let button = egui::Button::new(job)
                        .min_size(egui::vec2(ui.available_width(), 46.0))
                        .wrap();
                    if ui.add(button).clicked() {
                        chosen = Some(option.clone());
                    }
                    ui.add_space(4.0);
                }
            });
            ui.add_space(8.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 36.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui
                        .add(egui::Button::new("Annulla").min_size(egui::vec2(80.0, 30.0)))
                        .clicked()
                    {
                        self.notes_pick = None;
                    }
                },
            );
        });
        if modal.should_close() && chosen.is_none() {
            self.notes_pick = None;
        }
        if let Some(c) = chosen {
            self.notes_pick = None;
            if let Some(job) = c.job {
                self.restore_later(job, c.files.clone());
            }
            self.add_paths(c.files);
        }
    }

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
        self.flush_folder_notes();
        self.folder_sync.forget();
        self.cancel_scans();
        self.errors.clear();
        self.thumbs.clear();
        self.unselect();
        self.sheet = 0;
        self.renaming = None;
        self.filter = Default::default();
        self.parked.clear();
        self.project = project;
        // Con la casella spenta anche un lavoro salvato con file non supportati si apre senza:
        // il lavoro non risulta modificato solo per questo.
        if self.hide_unsupported {
            self.park_unsupported();
        }
        self.saved = self.project.clone();
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
        if winfiles_model::folder_notes::is_notes_file(&path) {
            self.status = format!(
                "Il nome {} è delle note della cartella: salva il lavoro con un altro nome",
                winfiles_model::folder_notes::FILE_NAME
            );
            return false;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use winfiles_geom::{Orientation, Paper};

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("winfiles-doc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn job(files: &[&str], data: u64) -> NotesJob {
        NotesJob {
            file: files.iter().map(|f| (*f).to_owned()).collect(),
            data,
            foglio: Paper::A4,
            verso: Orientation::Landscape,
            colonne: 3,
            una_per_foglio: false,
            tutta_pagina: false,
        }
    }

    /// Una cartella con a, b, c; note su a e c.
    fn folder(name: &str) -> (PathBuf, FolderNotes) {
        let dir = temp(name);
        for f in ["a.pdf", "b.pdf", "c.pdf", "d.pdf"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let mut n = FolderNotes::default();
        for f in ["a.pdf", "c.pdf"] {
            n.file
                .entry(f.into())
                .or_default()
                .pagine
                .entry(0)
                .or_default()
                .insert(winfiles_model::folder_notes::NOTE.into(), "nota".into());
        }
        (dir, n)
    }

    #[test]
    fn il_file_delle_note_riapre_tutto_il_lavoro_com_era() {
        // Tre file impaginati in orizzontale, note solo su due: tornano tutti e tre, in ordine.
        let (dir, mut n) = folder("un-lavoro");
        n.put_job(job(&["c.pdf", "a.pdf", "b.pdf"], 1));
        n.write(&dir).unwrap();
        let o = notes_to_folders(vec![winfiles_model::folder_notes::path_in(&dir)]);
        let order = vec![dir.join("c.pdf"), dir.join("a.pdf"), dir.join("b.pdf")];
        assert_eq!(o.paths, order);
        let (restored, files) = o.restore.unwrap();
        assert_eq!(restored.verso, Orientation::Landscape);
        assert_eq!(files, order);
        assert!(o.pick.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn piu_lavori_si_chiede_quale() {
        let (dir, mut n) = folder("due-lavori");
        n.put_job(job(&["a.pdf", "b.pdf"], 1));
        n.put_job(job(&["c.pdf", "d.pdf"], 2));
        n.write(&dir).unwrap();
        let o = notes_to_folders(vec![winfiles_model::folder_notes::path_in(&dir)]);
        assert!(o.paths.is_empty());
        let pick = o.pick.unwrap();
        assert_eq!(pick.options.len(), 2);
        assert_eq!(pick.options[0].files, vec![dir.join("c.pdf"), dir.join("d.pdf")]);
        assert_eq!(pick.options[0].noted, 1);
        let (title, detail) = choice_text(&pick.options[1], 1 + 86_400 * 3);
        assert_eq!(title, "a.pdf, b.pdf");
        assert_eq!(
            detail,
            "2 file, 1 con note · scritte 3 giorni fa · A4 orizzontale"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn un_lavoro_senza_piu_note_non_si_propone() {
        // Michel: due lavori, poi tolte tutte le note da uno; chiedeva ancora quale aprire.
        let (dir, mut n) = folder("lavoro-svuotato");
        n.put_job(job(&["a.pdf", "c.pdf"], 1));
        n.put_job(job(&["b.pdf", "d.pdf"], 2));
        n.write(&dir).unwrap();
        let o = notes_to_folders(vec![winfiles_model::folder_notes::path_in(&dir)]);
        assert!(o.pick.is_none());
        assert_eq!(o.paths, vec![dir.join("a.pdf"), dir.join("c.pdf")]);
        assert!(o.restore.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn senza_lavori_come_prima_i_file_con_note() {
        let (dir, n) = folder("senza-lavori");
        n.write(&dir).unwrap();
        let o = notes_to_folders(vec![winfiles_model::folder_notes::path_in(&dir)]);
        assert_eq!(o.paths, vec![dir.join("a.pdf"), dir.join("c.pdf")]);
        assert!(o.restore.is_none() && o.pick.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn note_fuori_dai_lavori_sono_una_scelta_in_piu() {
        // c ha note ma non sta nell'unico lavoro: si chiede fra il lavoro e "fuori".
        let (dir, mut n) = folder("fuori");
        n.put_job(job(&["a.pdf", "b.pdf"], 1));
        n.write(&dir).unwrap();
        let o = notes_to_folders(vec![winfiles_model::folder_notes::path_in(&dir)]);
        let pick = o.pick.unwrap();
        assert_eq!(pick.options[1].files, vec![dir.join("c.pdf")]);
        assert!(pick.options[1].job.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
