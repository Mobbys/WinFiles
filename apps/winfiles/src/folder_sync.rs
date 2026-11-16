//! Il salvataggio automatico delle note nel file `note.winfiles` di ogni cartella
//! (ADR 0038), comune a Miniature e Visualizza.
//!
//! Ogni scheda tiene le sue note come prima (un [`Holder`]); qui si guarda cosa è cambiato e lo si
//! scrive nella cartella, e si rilegge il file ogni tanto per vedere le note dei colleghi. Letture e
//! scritture vanno in un thread a parte: una cartella di rete lenta non ferma la finestra.
//!
//! Se la cartella non si può scrivere (sola lettura, server staccato) le note vanno in una copia
//! su questo PC, nella cartella delle impostazioni, e si riprova la cartella ogni tanto: appena si
//! può, ci finiscono e la copia sparisce.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime};

use winfiles_model::folder_notes::{self, Cells, FolderNotes, Key};
use winfiles_model::{ModelError, Project};

/// Ogni quanto si guarda se le note sono cambiate.
const TICK: Duration = Duration::from_millis(300);
/// Quanto aspettare dopo l'ultima modifica prima di scrivere: mentre si scrive una nota non si
/// salva a ogni lettera.
const SETTLE: Duration = Duration::from_millis(800);
/// Ogni quanto si rilegge il file per le note dei colleghi.
const POLL: Duration = Duration::from_secs(3);
/// Ogni quanto si riprova una cartella che non si può scrivere o leggere.
const RETRY: Duration = Duration::from_secs(30);
/// Letture e scritture insieme al più.
const MAX_JOBS: usize = 8;

/// Chi tiene le note: Miniature o Visualizza.
pub trait Holder {
    /// Le celle che possiede di ogni file caricato, per cartella: i nomi dei file e le celle.
    fn snapshot(&self) -> BTreeMap<PathBuf, (BTreeSet<String>, Cells)>;
    /// Mette nelle sue note le celle `cells` dei file `names` della cartella `dir`.
    fn apply(&mut self, dir: &Path, names: &BTreeSet<String>, cells: &Cells);
    /// Un gesto a metà: le note non si cambiano sotto le mani.
    fn busy(&self) -> bool {
        false
    }
}

/// Le celle che possiede chi salva: quali campi delle pagine e se la nota di cartella.
#[derive(Clone, Copy, Debug)]
pub struct Owned {
    pub fields: &'static [&'static str],
    pub folder_note: bool,
}

impl Owned {
    fn owns(&self, k: &Key, members: &BTreeSet<String>) -> bool {
        match k {
            Key::Folder => self.folder_note,
            Key::Cell { file, field, .. } => members.contains(file) && self.fields.contains(&field.as_str()),
        }
    }

    fn filter(&self, cells: &Cells, members: &BTreeSet<String>) -> Cells {
        cells
            .iter()
            .filter(|(k, _)| self.owns(k, members))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

/// Dove sono finite le note di una cartella.
#[derive(Clone, Debug, PartialEq)]
pub enum Where {
    Folder,
    /// Sulla copia di questo PC: la cartella non si può scrivere.
    Local,
}

struct State {
    /// Com'era il file all'ultima lettura; `None` finché non si è letto.
    base: Option<Cells>,
    /// I file della cartella di cui si tengono le note.
    members: BTreeSet<String>,
    busy: bool,
    last_sync: Option<Instant>,
    mtime: Option<SystemTime>,
    place: Where,
    error: Option<String>,
    /// L'ultima versione delle note vista e da quando.
    seen: Cells,
    changed_at: Option<Instant>,
}

impl State {
    fn new() -> Self {
        Self {
            base: None,
            members: BTreeSet::new(),
            busy: false,
            last_sync: None,
            mtime: None,
            place: Where::Folder,
            error: None,
            seen: Cells::new(),
            changed_at: None,
        }
    }
}

struct Job {
    dir: PathBuf,
    base: Option<Cells>,
    mine: Cells,
    members: BTreeSet<String>,
    owned: Owned,
    local: Option<PathBuf>,
    mtime: Option<SystemTime>,
}

struct Synced {
    merged: Cells,
    mtime: Option<SystemTime>,
    place: Where,
}

struct Done {
    dir: PathBuf,
    mine: Cells,
    members: BTreeSet<String>,
    outcome: Result<Synced, String>,
}

pub struct FolderSync {
    owned: Owned,
    /// Dove tenere le copie delle cartelle che non si possono scrivere.
    local_dir: Option<PathBuf>,
    states: BTreeMap<PathBuf, State>,
    /// Se c'è, solo questa cartella si rilegge ogni pochi secondi per le note dei colleghi; le
    /// altre si scrivono quando cambiano (Visualizza, che ricorda i file di tante cartelle).
    pub focus: Option<PathBuf>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    last_tick: Option<Instant>,
}

impl FolderSync {
    pub fn new(owned: Owned) -> Self {
        let local_dir = crate::prefs::config_dir().map(|d| d.join("note-cartelle"));
        Self::with_local(owned, local_dir)
    }

    pub fn with_local(owned: Owned, local_dir: Option<PathBuf>) -> Self {
        let (tx, rx) = channel();
        Self {
            owned,
            local_dir,
            states: BTreeMap::new(),
            focus: None,
            tx,
            rx,
            last_tick: None,
        }
    }

    /// Le cartelle le cui note sono finite solo su questo PC.
    pub fn local_folders(&self) -> Vec<&Path> {
        self.states
            .iter()
            .filter(|(_, s)| s.place == Where::Local)
            .map(|(d, _)| d.as_path())
            .collect()
    }

    /// I file delle note che non si riescono a leggere (rovinati): non si sovrascrivono.
    pub fn errors(&self) -> Vec<(&Path, &str)> {
        self.states
            .iter()
            .filter_map(|(d, s)| s.error.as_deref().map(|e| (d.as_path(), e)))
            .collect()
    }

    /// Dimentica le cartelle: un altro lavoro le rilegge da capo, e le sue note non passano davanti a
    /// quelle delle cartelle. Le letture in corso si buttano.
    pub fn forget(&mut self) {
        self.states.clear();
        while self.rx.try_recv().is_ok() {}
    }

    /// Vero se le note di tutte le cartelle sono state lette almeno una volta.
    pub fn settled(&self) -> bool {
        self.states.values().all(|s| s.base.is_some() && !s.busy)
    }

    /// Da chiamare a ogni giro della finestra: porta le note dei colleghi e salva le proprie.
    pub fn tick(&mut self, ctx: &egui::Context, holder: &mut dyn Holder) {
        let now = Instant::now();
        let mut fresh = false;
        while let Ok(done) = self.rx.try_recv() {
            self.finish(done, holder, now);
            fresh = true;
        }
        if !fresh && self.last_tick.is_some_and(|t| now.duration_since(t) < TICK) {
            return;
        }
        self.last_tick = Some(now);
        let snapshot = holder.snapshot();
        self.states.retain(|d, s| s.busy || snapshot.contains_key(d));
        let mut running = self.states.values().filter(|s| s.busy).count();
        for (dir, (names, cells)) in snapshot {
            let st = self.states.entry(dir.clone()).or_insert_with(State::new);
            if st.busy || running >= MAX_JOBS {
                continue;
            }
            let wait = if st.error.is_some() || st.place == Where::Local {
                RETRY
            } else {
                POLL
            };
            let polled = st.base.is_none()
                || st.error.is_some()
                || st.place == Where::Local
                || self.focus.as_ref().is_none_or(|f| *f == dir);
            let due = polled && st.last_sync.is_none_or(|t| now.duration_since(t) >= wait);
            let Some(base) = &st.base else {
                if due {
                    let mine = self.owned.filter(&cells, &names);
                    st.members = names;
                    launch(
                        st,
                        &self.tx,
                        ctx,
                        self.local_dir.clone(),
                        self.owned,
                        &dir,
                        mine,
                        now,
                    );
                    running += 1;
                }
                continue;
            };
            let joined: BTreeSet<String> = names.difference(&st.members).cloned().collect();
            if !joined.is_empty() {
                // Un file appena aggiunto prende le note che la cartella ha già.
                let base = base.clone();
                holder.apply(&dir, &joined, &base);
            }
            st.members = names;
            let mine = if joined.is_empty() {
                self.owned.filter(&cells, &st.members)
            } else {
                let fresh = holder.snapshot();
                fresh
                    .get(&dir)
                    .map(|(_, c)| self.owned.filter(c, &st.members))
                    .unwrap_or_default()
            };
            let base_mine = st
                .base
                .as_ref()
                .map(|b| self.owned.filter(b, &st.members))
                .unwrap_or_default();
            if mine != st.seen {
                st.seen = mine.clone();
                st.changed_at = Some(now);
            }
            let settled = st.changed_at.is_none_or(|t| now.duration_since(t) >= SETTLE);
            let changed = mine != base_mine;
            // Un file appena aggiunto: si rilegge subito, per avere anche le note più fresche.
            if (changed && settled) || due || !joined.is_empty() {
                launch(
                    st,
                    &self.tx,
                    ctx,
                    self.local_dir.clone(),
                    self.owned,
                    &dir,
                    mine,
                    now,
                );
                running += 1;
            }
        }
        if !self.states.is_empty() {
            ctx.request_repaint_after(POLL);
        }
    }

    /// Salva subito, senza thread, le note cambiate e non ancora scritte: prima di svuotare
    /// l'elenco, aprire un altro lavoro o chiudere.
    pub fn flush(&mut self, holder: &mut dyn Holder) {
        let now = Instant::now();
        while let Ok(done) = self.rx.try_recv() {
            self.finish(done, holder, now);
        }
        let snapshot = holder.snapshot();
        for (dir, (_, cells)) in snapshot {
            let Some(st) = self.states.get_mut(&dir) else {
                continue;
            };
            let Some(base) = &st.base else {
                continue;
            };
            let mine = self.owned.filter(&cells, &st.members);
            if mine == self.owned.filter(base, &st.members) {
                continue;
            }
            let job = Job {
                dir: dir.clone(),
                base: Some(base.clone()),
                mine: mine.clone(),
                members: st.members.clone(),
                owned: self.owned,
                local: self.local_dir.clone(),
                mtime: st.mtime,
            };
            let outcome = run(&job);
            let members = st.members.clone();
            self.finish(
                Done {
                    dir,
                    mine,
                    members,
                    outcome,
                },
                holder,
                now,
            );
        }
    }

    fn finish(&mut self, done: Done, holder: &mut dyn Holder, now: Instant) {
        let Some(st) = self.states.get_mut(&done.dir) else {
            return;
        };
        st.busy = false;
        st.last_sync = Some(now);
        let synced = match done.outcome {
            Ok(s) => s,
            Err(e) => {
                st.error = Some(e);
                return;
            }
        };
        st.error = None;
        // Le note sono cambiate mentre si salvava, o c'è un gesto a metà: si rifà al prossimo
        // giro, con le note di adesso.
        let current = holder
            .snapshot()
            .remove(&done.dir)
            .map(|(_, c)| self.owned.filter(&c, &done.members))
            .unwrap_or_default();
        if holder.busy() || current != done.mine {
            st.last_sync = None;
            return;
        }
        let theirs = self.owned.filter(&synced.merged, &done.members);
        if theirs != done.mine {
            holder.apply(&done.dir, &done.members, &synced.merged);
        }
        st.seen = theirs;
        st.base = Some(synced.merged);
        st.mtime = synced.mtime;
        st.place = synced.place;
    }
}

/// Le note di un lavoro (`Project`): quelle di Miniature, o le annotazioni di Visualizza.
pub struct ProjectHolder<'a> {
    pub project: &'a mut Project,
    /// Un'altra copia da tenere uguale: il lavoro com'era all'ultimo salvataggio, così le note
    /// salvate da sé non lo fanno risultare modificato.
    pub also: Option<&'a mut Project>,
    /// Note per pagina e nota di cartella (solo Miniature).
    pub notes: bool,
    /// Il campo in cui vanno le annotazioni dei file.
    pub annotations: &'static str,
    pub busy: bool,
}

/// Cosa salva Miniature: note per pagina, nota di cartella e annotazioni.
pub const MINIATURE: Owned = Owned {
    fields: &[folder_notes::NOTE, folder_notes::ANNOTATIONS],
    folder_note: true,
};
/// Cosa salva Visualizza: le sue annotazioni.
pub const VIEWER: Owned = Owned {
    fields: &[folder_notes::VIEWER_ANNOTATIONS],
    folder_note: false,
};

fn apply_to(
    project: &mut Project,
    dir: &Path,
    names: &BTreeSet<String>,
    cells: &Cells,
    notes: bool,
    field: &str,
) {
    for f in project.files.iter_mut().filter(|f| f.folder() == dir) {
        let Some(name) = folder_notes::file_key(&f.path) else {
            continue;
        };
        if !names.contains(&name) {
            continue;
        }
        if notes {
            f.notes.clear();
        }
        f.annotations.clear();
        for (k, v) in cells {
            let Key::Cell {
                file,
                page,
                field: fd,
            } = k
            else {
                continue;
            };
            if *file != name {
                continue;
            }
            if notes && fd == folder_notes::NOTE {
                f.set_note(*page, folder_notes::note_from(v));
            } else if fd == field {
                f.set_annotations(*page, folder_notes::annotations_from(v, dir));
            }
        }
    }
    if notes && !names.is_empty() {
        match cells.get(&Key::Folder) {
            Some(v) => {
                let text = folder_notes::note_from(v);
                if text.trim().is_empty() {
                    project.folder_notes.remove(dir);
                } else {
                    project.folder_notes.insert(dir.to_path_buf(), text);
                }
            }
            None => {
                project.folder_notes.remove(dir);
            }
        }
    }
}

impl Holder for ProjectHolder<'_> {
    fn snapshot(&self) -> BTreeMap<PathBuf, (BTreeSet<String>, Cells)> {
        let mut out: BTreeMap<PathBuf, (BTreeSet<String>, Cells)> = BTreeMap::new();
        for f in &self.project.files {
            let Some(name) = folder_notes::file_key(&f.path) else {
                continue;
            };
            let dir = f.folder();
            if dir.as_os_str().is_empty() {
                continue;
            }
            let e = out.entry(dir.to_path_buf()).or_default();
            e.0.insert(name.clone());
            if self.notes {
                for (p, t) in &f.notes {
                    if let Some(v) = folder_notes::note_value(t) {
                        e.1.insert(Key::cell(&name, *p, folder_notes::NOTE), v);
                    }
                }
            }
            for (p, list) in &f.annotations {
                if let Some(v) = folder_notes::annotations_value(list, dir) {
                    e.1.insert(Key::cell(&name, *p, self.annotations), v);
                }
            }
        }
        if self.notes {
            for (dir, (_, cells)) in out.iter_mut() {
                if let Some(v) = self
                    .project
                    .folder_notes
                    .get(dir)
                    .and_then(|t| folder_notes::note_value(t))
                {
                    cells.insert(Key::Folder, v);
                }
            }
        }
        out
    }

    fn apply(&mut self, dir: &Path, names: &BTreeSet<String>, cells: &Cells) {
        apply_to(self.project, dir, names, cells, self.notes, self.annotations);
        if let Some(also) = self.also.as_deref_mut() {
            apply_to(also, dir, names, cells, self.notes, self.annotations);
        }
    }

    fn busy(&self) -> bool {
        self.busy
    }
}

#[allow(clippy::too_many_arguments)]
fn launch(
    st: &mut State,
    tx: &Sender<Done>,
    ctx: &egui::Context,
    local: Option<PathBuf>,
    owned: Owned,
    dir: &Path,
    mine: Cells,
    now: Instant,
) {
    let job = Job {
        dir: dir.to_path_buf(),
        base: st.base.clone(),
        mine,
        members: st.members.clone(),
        owned,
        local,
        mtime: st.mtime,
    };
    st.busy = true;
    st.last_sync = Some(now);
    let tx = tx.clone();
    let wake = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("winfiles-note-cartella".into())
        .spawn(move || {
            let outcome = run(&job);
            let _ = tx.send(Done {
                dir: job.dir,
                mine: job.mine,
                members: job.members,
                outcome,
            });
            wake.request_repaint();
        });
    if spawned.is_err() {
        // Senza thread si riprova al prossimo giro.
        st.busy = false;
    }
}

/// La copia su questo PC delle note di `dir`.
fn local_file(local: &Path, dir: &Path) -> PathBuf {
    // FNV-1a del percorso: stabile fra versioni, a differenza dell'hash di Rust.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in dir.to_string_lossy().to_lowercase().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    local.join(format!("{h:016x}.json"))
}

/// Il campo con cui la copia locale ricorda la sua cartella.
const LOCAL_FOLDER: &str = "cartella";

fn mtime_of(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(folder_notes::path_in(dir))
        .and_then(|m| m.modified())
        .ok()
}

/// Legge, unisce e scrive le note di una cartella.
fn run(job: &Job) -> Result<Synced, String> {
    let dir = &job.dir;
    let local = job.local.as_deref().map(|l| local_file(l, dir));
    let local_notes = local
        .as_deref()
        .and_then(|l| FolderNotes::read_file(l).ok().flatten())
        .map(|mut n| {
            n.altro.remove(LOCAL_FOLDER);
            n
        });
    let mtime = mtime_of(dir);
    if let Some(base) = &job.base
        && local_notes.is_none()
        && mtime == job.mtime
        && job.owned.filter(base, &job.members) == job.mine
    {
        // Niente di nuovo, né qui né nella cartella.
        return Ok(Synced {
            merged: base.clone(),
            mtime,
            place: Where::Folder,
        });
    }
    let (disk, readable) = match FolderNotes::read(dir) {
        Ok(n) => (n.unwrap_or_default(), true),
        Err(ModelError::Io { .. }) => (FolderNotes::default(), false),
        Err(e) => {
            return Err(format!(
                "il file delle note di {} non si legge ({e}): non lo sovrascrivo",
                dir.display()
            ));
        }
    };
    // Una cartella che non si legge (server staccato) non vuol dire "nessuna nota": si fa come
    // se fosse rimasta com'era.
    let disk_cells = if readable {
        disk.cells()
    } else {
        job.base.clone().unwrap_or_default()
    };
    // Le note rimaste su questo PC valgono più di quelle della cartella, file per file.
    let mut seen = disk_cells.clone();
    if let Some(l) = &local_notes {
        let lc = l.cells();
        let files: BTreeSet<Option<&str>> = lc.keys().map(Key::file).collect();
        seen.retain(|k, _| !files.contains(&k.file()));
        seen.extend(lc);
    }
    let base = match &job.base {
        Some(b) => b.clone(),
        None => folder_notes::first_base(&job.mine, &seen),
    };
    let merged = folder_notes::merge(&base, &job.mine, &seen, |k| job.owned.owns(k, &job.members));
    let mut place = if local_notes.is_some() {
        Where::Local
    } else {
        Where::Folder
    };
    let mut mtime = mtime;
    if merged != disk_cells || local_notes.is_some() {
        let out = disk.with_cells(&merged);
        let written = readable && out.write(dir).is_ok();
        if written {
            place = Where::Folder;
            mtime = mtime_of(dir);
            if let Some(l) = &local {
                let _ = std::fs::remove_file(l);
            }
        } else if let Some(l) = &local {
            let mut copy = out;
            copy.altro.insert(
                LOCAL_FOLDER.into(),
                serde_json::Value::String(dir.to_string_lossy().into_owned()),
            );
            if let Some(parent) = l.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            copy.write_file(l)
                .map_err(|e| format!("non riesco a salvare le note di {}: {e}", dir.display()))?;
            place = Where::Local;
        } else {
            return Err(format!("non riesco a salvare le note in {}", dir.display()));
        }
    }
    Ok(Synced { merged, mtime, place })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use winfiles_model::folder_notes::NOTE;

    /// Un holder finto: note per (cartella, nome, pagina).
    #[derive(Default)]
    struct Fake {
        notes: BTreeMap<(PathBuf, String), BTreeMap<usize, String>>,
    }

    impl Holder for Fake {
        fn snapshot(&self) -> BTreeMap<PathBuf, (BTreeSet<String>, Cells)> {
            let mut out: BTreeMap<PathBuf, (BTreeSet<String>, Cells)> = BTreeMap::new();
            for ((dir, name), pages) in &self.notes {
                let e = out.entry(dir.clone()).or_default();
                e.0.insert(name.clone());
                for (p, t) in pages {
                    e.1.insert(Key::cell(name, *p, NOTE), Value::String(t.clone()));
                }
            }
            out
        }

        fn apply(&mut self, dir: &Path, names: &BTreeSet<String>, cells: &Cells) {
            for name in names {
                let pages = self.notes.entry((dir.to_path_buf(), name.clone())).or_default();
                pages.clear();
                for (k, v) in cells {
                    if let Key::Cell { file, page, field } = k
                        && file == name
                        && field == NOTE
                    {
                        pages.insert(*page, folder_notes::note_from(v));
                    }
                }
            }
        }
    }

    const OWNED: Owned = Owned {
        fields: &[NOTE],
        folder_note: false,
    };

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("winfiles-sync-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Fa girare finché tutto è letto e scritto.
    fn settle(sync: &mut FolderSync, holder: &mut Fake) {
        let ctx = egui::Context::default();
        for _ in 0..200 {
            sync.last_tick = None;
            for s in sync.states.values_mut() {
                s.changed_at = None;
            }
            sync.tick(&ctx, holder);
            std::thread::sleep(Duration::from_millis(5));
            if sync.settled() {
                sync.flush(holder);
                return;
            }
        }
        panic!("non si è fermato");
    }

    #[test]
    fn due_colleghi_si_scambiano_le_note() {
        let dir = temp("colleghi");
        let local = temp("colleghi-locale");
        let mut michel = Fake::default();
        michel.notes.insert(
            (dir.clone(), "a.pdf".into()),
            BTreeMap::from([(0, "colore più scuro".into())]),
        );
        let mut sync_m = FolderSync::with_local(OWNED, Some(local.clone()));
        settle(&mut sync_m, &mut michel);
        assert!(folder_notes::path_in(&dir).is_file());

        // Il collega apre la stessa cartella con un altro file e scrive la sua nota.
        let mut collega = Fake::default();
        collega
            .notes
            .insert((dir.clone(), "a.pdf".into()), BTreeMap::new());
        collega.notes.insert(
            (dir.clone(), "b.pdf".into()),
            BTreeMap::from([(1, "manca la fustella".into())]),
        );
        let mut sync_c = FolderSync::with_local(OWNED, Some(local.clone()));
        settle(&mut sync_c, &mut collega);
        // Ritrova la nota di Michel e la sua resta.
        assert_eq!(
            collega.notes[&(dir.clone(), "a.pdf".into())][&0],
            "colore più scuro"
        );
        assert_eq!(
            collega.notes[&(dir.clone(), "b.pdf".into())][&1],
            "manca la fustella"
        );

        // Michel aggiunge b.pdf all'elenco: prende la nota del collega.
        michel
            .notes
            .insert((dir.clone(), "b.pdf".into()), BTreeMap::new());
        settle(&mut sync_m, &mut michel);
        assert_eq!(
            michel.notes[&(dir.clone(), "b.pdf".into())][&1],
            "manca la fustella"
        );

        // Michel toglie la sua nota: sparisce anche per il collega.
        michel
            .notes
            .get_mut(&(dir.clone(), "a.pdf".into()))
            .unwrap()
            .clear();
        settle(&mut sync_m, &mut michel);
        for s in sync_c.states.values_mut() {
            s.last_sync = None;
        }
        settle(&mut sync_c, &mut collega);
        assert!(collega.notes[&(dir.clone(), "a.pdf".into())].is_empty());
        assert_eq!(
            collega.notes[&(dir.clone(), "b.pdf".into())][&1],
            "manca la fustella"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&local);
    }

    #[test]
    fn un_file_rovinato_non_si_sovrascrive() {
        let dir = temp("rovinato");
        std::fs::write(folder_notes::path_in(&dir), "{ rovinato").unwrap();
        let mut h = Fake::default();
        h.notes
            .insert((dir.clone(), "a.pdf".into()), BTreeMap::from([(0, "x".into())]));
        let mut sync = FolderSync::with_local(OWNED, None);
        sync.tick(&egui::Context::default(), &mut h);
        for _ in 0..200 {
            std::thread::sleep(Duration::from_millis(5));
            sync.last_tick = None;
            sync.tick(&egui::Context::default(), &mut h);
            if !sync.errors().is_empty() {
                break;
            }
        }
        assert_eq!(sync.errors().len(), 1);
        assert_eq!(
            std::fs::read_to_string(folder_notes::path_in(&dir)).unwrap(),
            "{ rovinato"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cartella_che_non_si_scrive_le_note_restano_sul_pc() {
        let dir = temp("solalettura");
        let local = temp("solalettura-locale");
        // Al posto del file delle note c'è una cartella: non si legge e non si scrive, come una
        // cartella in sola lettura (che da root non si può provare).
        std::fs::create_dir_all(folder_notes::path_in(&dir)).unwrap();
        let mut h = Fake::default();
        h.notes.insert(
            (dir.clone(), "a.pdf".into()),
            BTreeMap::from([(0, "nota".into())]),
        );
        let mut sync = FolderSync::with_local(OWNED, Some(local.clone()));
        settle(&mut sync, &mut h);
        assert_eq!(sync.local_folders(), vec![dir.as_path()]);
        assert!(local_file(&local, &dir).is_file());
        // Rileggere la cartella che non si legge non toglie la nota.
        for s in sync.states.values_mut() {
            s.last_sync = None;
        }
        settle(&mut sync, &mut h);
        assert_eq!(h.notes[&(dir.clone(), "a.pdf".into())][&0], "nota");

        // Riaperta da capo, ritrova la nota dalla copia del PC.
        let mut again = Fake::default();
        again.notes.insert((dir.clone(), "a.pdf".into()), BTreeMap::new());
        let mut sync2 = FolderSync::with_local(OWNED, Some(local.clone()));
        settle(&mut sync2, &mut again);
        assert_eq!(again.notes[&(dir.clone(), "a.pdf".into())][&0], "nota");

        // La cartella torna scrivibile: la nota ci finisce e la copia sparisce.
        std::fs::remove_dir(folder_notes::path_in(&dir)).unwrap();
        for s in sync2.states.values_mut() {
            s.last_sync = None;
        }
        settle(&mut sync2, &mut again);
        assert!(folder_notes::path_in(&dir).is_file());
        assert!(sync2.local_folders().is_empty());
        assert!(!local_file(&local, &dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&local);
    }
}
