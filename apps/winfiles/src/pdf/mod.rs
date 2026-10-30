//! Il modulo PDF (Fase 9, ADR 0015): apre un PDF e ne modifica le pagine, come "Controllo PDF"
//! della 4.9. A sinistra le pagine in miniatura (si scelgono, si trascinano per cambiare ordine),
//! al centro la pagina corrente in grande, sopra i comandi. Le modifiche le fa
//! `winfiles-pdf-edit`; qui ci sono solo la finestra, le scelte e le domande.

mod dialogs;
mod render;
pub mod tracciati;

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use egui::{Align, Rect, Sense, Stroke, Vec2, vec2};
use winfiles_geom::{Paper, Size, Unit, format_size};
use winfiles_pdf_edit::{Editor, Overprint, PageInfo, PdfFile, Spot};

use crate::app::{POINTS_PER_MM, card};
use crate::cutter::Cutter;
use crate::icons;
use crate::quotes::Quotes;
use crate::theme::{Tokens, semibold};
use dialogs::{Action, Dialog, Position, Scope};
use render::{Image, Renderer};

/// Lato lungo delle miniature nell'elenco, in pixel.
const THUMB_SIDE: u32 = 280;
/// Larghezza di una casella dell'elenco delle pagine.
const CELL_W: f32 = 132.0;
const CELL_H: f32 = 176.0;
const ZOOM_STOPS: [f32; 8] = [25.0, 50.0, 75.0, 100.0, 150.0, 200.0, 300.0, 400.0];

/// Cosa fare dopo la domanda "salvare le modifiche?".
#[derive(Clone, Debug)]
enum Then {
    /// Aprire un PDF (quello scelto nella finestra di sistema, se manca).
    Open(Option<PathBuf>),
    /// Unire più PDF in uno nuovo (scelti nella finestra di sistema, se mancano).
    Combine(Vec<PathBuf>),
    /// Chiudere il PDF.
    CloseDocument,
    /// Chiudere WinFiles.
    Quit,
}

/// La pagina trascinata nell'elenco e dove andrebbe lasciata.
struct Drag {
    page: usize,
    /// Prima di quale pagina (0..=n) finisce.
    before: Option<usize>,
}

pub struct PdfModule {
    editor: Option<Editor>,
    /// Il file su cui scrive "Salva". Prima del primo "Salva con nome" non c'è: il PDF di partenza
    /// non si sovrascrive senza chiederlo (come la 4.9, che proponeva "_modificato").
    target: Option<PathBuf>,
    /// Cresce a ogni modifica del documento.
    generation: u64,
    keys: Vec<u64>,
    infos: Vec<PageInfo>,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    current: usize,
    /// Ingrandimento (1 = 100%); `None` adatta la pagina alla finestra.
    zoom: Option<f32>,
    shown_zoom: f32,
    /// Lo scorrimento dell'anteprima all'ultimo disegno, e quello da imporre al prossimo (Ctrl+rotella
    /// verso il puntatore, barra spaziatrice per trascinare la vista).
    scroll: Vec2,
    scroll_to: Option<Vec2>,
    zoom_anchor: crate::pan::Anchor,
    pub status: String,
    dialog: Option<Dialog>,
    drag: Option<Drag>,
    renderer: Renderer,
    asking: Option<Then>,
    /// La chiusura di WinFiles è già stata confermata per il PDF.
    pub quit_confirmed: bool,
    /// Portare l'elenco alla pagina corrente al prossimo disegno.
    reveal: bool,
    /// La taglierina sull'anteprima, in millimetri della pagina come si vede.
    cutter: Cutter,
    /// Il pannello delle tinte piatte a destra è aperto.
    spots_open: bool,
    /// Le quote sono accese: trascinare sulla pagina traccia quote invece della taglierina, e a
    /// destra c'è il loro pannello.
    quoting: bool,
    /// L'altezza della barra dei comandi, che va a capo quando la finestra è stretta (misurata al
    /// disegno precedente).
    bar_height: f32,
    /// Le quote della pagina corrente, in millimetri della pagina come si vede.
    quotes: Quotes,
    /// Le tinte piatte del documento e di quale versione sono.
    spots: Option<(u64, Result<Vec<Spot>, String>)>,
    /// La tinta che si sta rinominando, con il nome in scrittura.
    renaming: Option<(Vec<u8>, String)>,
    /// Per il PDF aperto: togliere (`true`) o tenere la copia di Illustrator al salvataggio;
    /// `None` finché non lo si è chiesto (ADR 0020).
    illustrator: Option<bool>,
    /// Il salvataggio fermo sulla domanda della copia di Illustrator: con "Salva con nome" o no,
    /// e cosa fare dopo.
    illustrator_ask: Option<(bool, Option<Then>)>,
    /// L'unità dell'ultima anteprima, per le quote stampate.
    unit: Unit,
    /// La stampa o il PDF con le quote chiesti, che l'app prende e manda avanti.
    pub output: Option<crate::quotes::Output>,
    /// Lo strumento Seleziona tracciato, con il suo pannello a destra.
    paths: tracciati::PathTool,
    /// I nomi delle tinte usati (taglio, cordonatura…), ricordati nelle impostazioni.
    pub inks: Vec<tracciati::InkName>,
    /// I file da mandare alla scheda Nesting, che l'app prende e svuota.
    pub to_nesting: Vec<PathBuf>,
}

impl PdfModule {
    pub fn new(ctx: &egui::Context) -> Self {
        let wake = ctx.clone();
        Self {
            editor: None,
            target: None,
            generation: 0,
            keys: Vec::new(),
            infos: Vec::new(),
            selected: BTreeSet::new(),
            anchor: None,
            current: 0,
            zoom: None,
            shown_zoom: 1.0,
            scroll: Vec2::ZERO,
            scroll_to: None,
            zoom_anchor: crate::pan::Anchor::default(),
            status: String::new(),
            dialog: None,
            drag: None,
            renderer: Renderer::start(move || wake.request_repaint()),
            asking: None,
            quit_confirmed: false,
            reveal: false,
            cutter: Cutter::default(),
            spots_open: true,
            quoting: false,
            bar_height: 46.0,
            quotes: Quotes::default(),
            spots: None,
            renaming: None,
            illustrator: None,
            illustrator_ask: None,
            unit: Unit::Mm,
            output: None,
            paths: tracciati::PathTool::default(),
            inks: tracciati::default_inks(),
            to_nesting: Vec::new(),
        }
    }

    /// Manda alla scheda Nesting il PDF aperto, com'è salvato su disco.
    fn send_to_nesting(&mut self) {
        let Some(e) = self.editor.as_ref() else { return };
        let path = self.target.clone().unwrap_or_else(|| e.path().to_path_buf());
        if e.is_modified() {
            self.status =
                "Al nesting va il PDF com'è salvato: le modifiche non salvate restano qui.".to_string();
        }
        self.to_nesting.push(path);
    }

    pub fn is_modified(&self) -> bool {
        self.editor.as_ref().is_some_and(Editor::is_modified)
    }

    /// Il nome del PDF aperto, per il titolo della finestra.
    pub fn title(&self) -> Option<String> {
        let e = self.editor.as_ref()?;
        let path = self.target.as_deref().unwrap_or(e.path());
        let name = path.file_name()?.to_string_lossy().into_owned();
        let mark = if e.is_modified() { " •" } else { "" };
        Some(format!("{name}{mark} — WinFiles"))
    }

    /// Chiudere WinFiles: se il PDF ha modifiche prima si chiede. `true` se si può chiudere ora.
    pub fn can_quit(&mut self) -> bool {
        if self.quit_confirmed || !self.is_modified() {
            return true;
        }
        self.asking = Some(Then::Quit);
        false
    }

    // --- Documento -----------------------------------------------------------------------

    fn guard(&mut self, ctx: &egui::Context, then: Then) {
        if self.is_modified() {
            self.asking = Some(then);
        } else {
            self.run(ctx, then);
        }
    }

    fn run(&mut self, ctx: &egui::Context, then: Then) {
        match then {
            Then::Open(path) => {
                let path = path.or_else(|| pdf_dialog().set_title("Apri un PDF").pick_file());
                if let Some(path) = path {
                    self.open(&path);
                }
            }
            Then::Combine(paths) => {
                let paths = if paths.is_empty() {
                    pdf_dialog()
                        .set_title("Scegli i PDF da unire")
                        .pick_files()
                        .unwrap_or_default()
                } else {
                    paths
                };
                if !paths.is_empty() {
                    self.combine(&paths);
                }
            }
            Then::CloseDocument => {
                self.editor = None;
                self.target = None;
                self.renderer.clear();
                self.refresh();
                self.status = String::new();
            }
            Then::Quit => {
                self.quit_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Dalle miniature (tasto destro › Modifica PDF): se il PDF aperto ha modifiche non salvate
    /// prima si chiede; se è già quello aperto si resta lì.
    pub fn open_from_list(&mut self, ctx: &egui::Context, path: &Path) {
        if self.editor.as_ref().is_some_and(|e| e.path() == path) {
            return;
        }
        self.guard(ctx, Then::Open(Some(path.to_path_buf())));
    }

    /// Apre un PDF al posto di quello aperto (le modifiche sono già state salvate o lasciate).
    pub fn open(&mut self, path: &Path) {
        match Editor::open(path) {
            Ok(e) => {
                let n = e.page_count();
                self.set_editor(e, None);
                self.status = format!("Aperto {}: {}", file_name(path), crate::canvas::pages_text(n));
            }
            Err(e) => self.status = format!("{}: {e}", file_name(path)),
        }
    }

    fn set_editor(&mut self, e: Editor, target: Option<PathBuf>) {
        self.editor = Some(e);
        self.target = target;
        self.illustrator = None;
        self.illustrator_ask = None;
        self.selected.clear();
        self.anchor = None;
        self.current = 0;
        self.zoom = None;
        self.renderer.clear();
        self.refresh();
        self.reveal = true;
    }

    /// Unisce i PDF in uno nuovo, senza nome finché non si salva.
    fn combine(&mut self, paths: &[PathBuf]) {
        let mut files = Vec::new();
        for p in paths {
            match PdfFile::open(p) {
                Ok(f) => files.push((file_stem(p), f)),
                Err(e) => {
                    self.status = format!("{}: {e}", file_name(p));
                    return;
                }
            }
        }
        let list: Vec<(&str, &PdfFile)> = files.iter().map(|(n, f)| (n.as_str(), f)).collect();
        match winfiles_pdf_edit::combine(&list) {
            Ok(all) => {
                let dir = paths
                    .first()
                    .and_then(|p| p.parent())
                    .map(Path::to_path_buf)
                    .unwrap_or_default();
                let n = all.page_count();
                let mut e = Editor::new(all, dir.join("PDF uniti.pdf"));
                // Un documento nuovo: da salvare, anche se non è cambiato nulla dopo l'unione.
                e.mark_modified();
                self.set_editor(e, None);
                self.status = format!("Uniti {} PDF: {}", paths.len(), crate::canvas::pages_text(n));
            }
            Err(e) => self.status = format!("Unione non riuscita: {e}"),
        }
    }

    /// File lasciati sulla finestra: il primo PDF si apre se non ce n'è uno, gli altri si
    /// aggiungono in fondo (con Annulla per tornare indietro).
    pub fn dropped(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let pdfs: Vec<PathBuf> = paths.into_iter().filter(|p| is_pdf(p)).collect();
        if pdfs.is_empty() {
            self.status = "Qui si lavorano solo i PDF".into();
            return;
        }
        if self.editor.is_none() {
            if pdfs.len() == 1 {
                self.run(ctx, Then::Open(pdfs.first().cloned()));
            } else {
                self.run(ctx, Then::Combine(pdfs));
            }
            return;
        }
        let mut added = 0;
        for p in &pdfs {
            let at = self.infos.len();
            let result = PdfFile::open(p).and_then(|f| {
                let all: Vec<usize> = (0..f.page_count()).collect();
                let n = all.len();
                self.editor_mut()?.insert_from(&f, &all, at).map(|()| n)
            });
            match result {
                Ok(n) => added += n,
                Err(e) => {
                    self.status = format!("{}: {e}", file_name(p));
                    break;
                }
            }
        }
        if added > 0 {
            self.refresh();
            self.status = format!("Aggiunte in fondo {}", crate::canvas::pages_text(added));
        }
    }

    fn editor_mut(&mut self) -> Result<&mut Editor, winfiles_pdf_edit::EditError> {
        self.editor
            .as_mut()
            .ok_or_else(|| winfiles_pdf_edit::EditError::Invalid("nessun PDF aperto".into()))
    }

    /// Dopo ogni modifica: misure, impronte, scelte ancora valide.
    fn refresh(&mut self) {
        self.generation += 1;
        match &self.editor {
            Some(e) => {
                self.infos = e.pages();
                self.keys = e.page_keys();
            }
            None => {
                self.infos.clear();
                self.keys.clear();
            }
        }
        let n = self.infos.len();
        self.selected.retain(|&i| i < n);
        self.current = self.current.min(n.saturating_sub(1));
        let alive: HashSet<u64> = self.keys.iter().copied().collect();
        self.renderer.keep(&alive);
    }

    /// Le pagine su cui agiscono i comandi: quelle scelte, o la corrente.
    fn chosen(&self) -> Vec<usize> {
        if self.selected.is_empty() {
            if self.infos.is_empty() {
                Vec::new()
            } else {
                vec![self.current]
            }
        } else {
            self.selected.iter().copied().collect()
        }
    }

    fn scope_pages(&self, scope: Scope) -> Vec<usize> {
        match scope {
            Scope::Chosen => self.chosen(),
            Scope::All => (0..self.infos.len()).collect(),
        }
    }

    /// Fa una modifica e ne scrive l'esito nella barra di stato.
    fn apply(&mut self, done: impl FnOnce(&mut Editor) -> Result<String, winfiles_pdf_edit::EditError>) {
        let Some(e) = self.editor.as_mut() else {
            return;
        };
        match done(e) {
            Ok(msg) => {
                self.refresh();
                self.status = msg;
            }
            Err(err) => self.status = err.to_string(),
        }
    }

    fn rotate(&mut self, quarters: i64) {
        let pages = self.chosen();
        self.apply(|e| {
            e.rotate(&pages, quarters)?;
            Ok(format!("Ruotate {}", crate::canvas::pages_text(pages.len())))
        });
    }

    fn delete(&mut self) {
        let pages = self.chosen();
        self.apply(|e| {
            e.delete(&pages)?;
            Ok(format!("Eliminate {}", crate::canvas::pages_text(pages.len())))
        });
        self.selected.clear();
    }

    fn duplicate(&mut self) {
        let pages = self.chosen();
        self.apply(|e| {
            e.duplicate(&pages)?;
            Ok(format!("Duplicate {}", crate::canvas::pages_text(pages.len())))
        });
    }

    fn trim(&mut self) {
        let pages = self.chosen();
        self.apply(|e| {
            let n = e.trim(&pages)?;
            Ok(match n {
                0 => "Nessuna pagina da rifilare: il formato finito è già quello della pagina".into(),
                n => format!("Rifilate al vivo {}", crate::canvas::pages_text(n)),
            })
        });
    }

    /// La taglierina: di ogni pagina resta il rettangolo disegnato sull'anteprima.
    /// Il riquadro di partenza di "Rifila libero": la pagina corrente con 10 mm in meno per lato
    /// (meno, se la pagina è piccola), così le maniglie si vedono e si prendono.
    fn start_free_crop(&mut self) {
        let Some(page) = self.infos.get(self.current).map(|p| p.size) else {
            return;
        };
        let inset = 10.0_f64.min(page.w / 6.0).min(page.h / 6.0);
        self.quoting = false;
        self.cutter.clear();
        self.cutter.crop = Some(crate::cutter::Crop {
            x0: inset as f32,
            y0: inset as f32,
            x1: (page.w - inset) as f32,
            y1: (page.h - inset) as f32,
        });
    }

    fn cut(&mut self, scope: Scope) {
        let Some(c) = self.cutter.crop.map(crate::cutter::Crop::normalized) else {
            return;
        };
        let rect = winfiles_geom::Rect::new(
            f64::from(c.x0),
            f64::from(c.y0),
            f64::from(c.width()),
            f64::from(c.height()),
        );
        let pages = self.scope_pages(scope);
        self.apply(|e| {
            let n = e.crop(&pages, rect)?;
            Ok(match n {
                0 => "Il riquadro non tocca le pagine scelte".into(),
                n => format!("Tagliate {}", crate::canvas::pages_text(n)),
            })
        });
        self.cutter.clear();
    }

    fn fit_paper(&mut self, paper: Paper) {
        let pages = self.chosen();
        self.apply(|e| {
            e.fit_paper(&pages, paper)?;
            Ok(format!(
                "Adattate ad {} {}",
                paper.name(),
                crate::canvas::pages_text(pages.len())
            ))
        });
    }

    fn undo(&mut self) {
        if let Some(e) = self.editor.as_mut()
            && let Some(label) = e.undo_label().map(str::to_string)
            && e.undo()
        {
            self.refresh();
            self.status = format!("Annullato: {label}");
        }
    }

    fn redo(&mut self) {
        if let Some(e) = self.editor.as_mut()
            && let Some(label) = e.redo_label().map(str::to_string)
            && e.redo()
        {
            self.refresh();
            self.status = format!("Rifatto: {label}");
        }
    }

    /// Salva sul file scelto; la prima volta (o con `ask`) chiede dove. `false` se non salvato.
    fn save(&mut self, ask: bool) -> bool {
        let Some(e) = self.editor.as_ref() else {
            return false;
        };
        // Prima si chiede che fare della copia di Illustrator: la risposta riprende il salvataggio.
        if self.illustrator.is_none() && e.has_illustrator_data() {
            self.illustrator_ask = Some((ask, None));
            return false;
        }
        let path = match (&self.target, ask) {
            (Some(p), false) => Some(p.clone()),
            _ => {
                // La finestra propone l'ultimo file salvato, o il nome di partenza con "_modificato".
                let source = e.path();
                let (dir, name) = match &self.target {
                    Some(t) => (
                        t.parent().map(Path::to_path_buf).unwrap_or_default(),
                        file_name(t),
                    ),
                    None => (
                        source.parent().map(Path::to_path_buf).unwrap_or_default(),
                        format!("{}_modificato.pdf", file_stem(source)),
                    ),
                };
                pdf_dialog()
                    .set_title("Salva il PDF")
                    .set_directory(dir)
                    .set_file_name(name)
                    .save_file()
            }
        };
        let Some(path) = path else {
            return false;
        };
        let path = with_pdf_extension(path);
        let Some(e) = self.editor.as_mut() else {
            return false;
        };
        match e.save(&path) {
            Ok(()) => {
                self.status = format!("Salvato {}", file_name(&path));
                self.target = Some(path);
                true
            }
            Err(err) => {
                self.status = format!("Salvataggio non riuscito: {err}");
                false
            }
        }
    }

    fn act(&mut self, action: Action) {
        match action {
            Action::Margins(scope, m, color) => {
                let pages = self.scope_pages(scope);
                self.apply(|e| {
                    e.add_margins(&pages, m, color)?;
                    Ok(format!(
                        "Margini cambiati su {}",
                        crate::canvas::pages_text(pages.len())
                    ))
                });
            }
            Action::Resize(scope, size) => {
                let pages = self.scope_pages(scope);
                self.apply(|e| {
                    e.resize(&pages, size)?;
                    Ok(format!(
                        "Ridimensionate {}",
                        crate::canvas::pages_text(pages.len())
                    ))
                });
            }
            Action::Insert(file, list, at) => {
                let pos = match at {
                    Position::Start => 0,
                    Position::AfterCurrent => (self.current + 1).min(self.infos.len()),
                    Position::End => self.infos.len(),
                };
                self.apply(|e| {
                    e.insert_from(&file, &list, pos)?;
                    Ok(format!("Aggiunte {}", crate::canvas::pages_text(list.len())))
                });
                self.selected = (pos..pos + list.len()).collect();
                self.current = pos;
                self.reveal = true;
            }
            Action::Extract { one_per_page } => self.extract(one_per_page),
            Action::Split(cuts) => self.split(&cuts),
        }
    }

    fn extract(&mut self, one_per_page: bool) {
        let Some(e) = self.editor.as_ref() else {
            return;
        };
        let pages = self.chosen();
        let stem = file_stem(self.target.as_deref().unwrap_or(e.path()));
        let dir = e.path().parent().map(Path::to_path_buf).unwrap_or_default();
        if one_per_page {
            let Some(folder) = rfd::FileDialog::new()
                .set_title("Cartella per le pagine estratte")
                .set_directory(&dir)
                .pick_folder()
            else {
                return;
            };
            let width = self.infos.len().to_string().len();
            for &p in &pages {
                let path = folder.join(format!("{stem}_pagina{:0width$}.pdf", p + 1));
                if let Err(err) = e.extract(&[p]).and_then(|f| f.save(&path)) {
                    self.status = format!("{}: {err}", file_name(&path));
                    return;
                }
            }
            self.status = format!(
                "Estratte {} in {}",
                crate::canvas::pages_text(pages.len()),
                folder.display()
            );
        } else {
            let name = match pages.as_slice() {
                [one] => format!("{stem}_pagina{}.pdf", one + 1),
                _ => format!("{stem}_estratte.pdf"),
            };
            let Some(path) = pdf_dialog()
                .set_title("Salva le pagine estratte")
                .set_directory(&dir)
                .set_file_name(name)
                .save_file()
            else {
                return;
            };
            let path = with_pdf_extension(path);
            self.status = match e.extract(&pages).and_then(|f| f.save(&path)) {
                Ok(()) => format!(
                    "Estratte {} in {}",
                    crate::canvas::pages_text(pages.len()),
                    file_name(&path)
                ),
                Err(err) => format!("Estrazione non riuscita: {err}"),
            };
        }
    }

    fn split(&mut self, cuts: &[usize]) {
        let Some(e) = self.editor.as_ref() else {
            return;
        };
        let stem = file_stem(self.target.as_deref().unwrap_or(e.path()));
        let dir = e.path().parent().map(Path::to_path_buf).unwrap_or_default();
        let Some(folder) = rfd::FileDialog::new()
            .set_title("Cartella per le parti")
            .set_directory(&dir)
            .pick_folder()
        else {
            return;
        };
        let ranges = winfiles_pdf_edit::split_ranges(self.infos.len(), cuts);
        let width = ranges.len().to_string().len();
        for (k, r) in ranges.iter().enumerate() {
            let path = folder.join(format!("{stem}_parte{:0width$}.pdf", k + 1));
            let pages: Vec<usize> = r.clone().collect();
            if let Err(err) = e.extract(&pages).and_then(|f| f.save(&path)) {
                self.status = format!("{}: {err}", file_name(&path));
                return;
            }
        }
        self.status = format!("Diviso in {} file in {}", ranges.len(), folder.display());
    }

    fn open_insert(&mut self) {
        let Some(path) = pdf_dialog().set_title("Aggiungi pagine da un PDF").pick_file() else {
            return;
        };
        match PdfFile::open(&path) {
            Ok(file) => {
                self.dialog = Some(Dialog::Insert(Box::new(dialogs::Insert {
                    file,
                    path,
                    pages: String::new(),
                    at: Position::AfterCurrent,
                })));
            }
            Err(e) => self.status = format!("{}: {e}", file_name(&path)),
        }
    }

    // --- Tastiera --------------------------------------------------------------------------

    pub fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        if self.dialog.is_some() || self.asking.is_some() {
            return;
        }
        let pressed = |mods, key| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(mods, key)));
        if pressed(Modifiers::COMMAND | Modifiers::SHIFT, Key::S) {
            self.save(true);
        } else if pressed(Modifiers::COMMAND, Key::S) {
            self.save(false);
        } else if pressed(Modifiers::COMMAND, Key::O) {
            self.guard(ctx, Then::Open(None));
        } else if pressed(Modifiers::COMMAND, Key::P) {
            self.print(ctx);
        } else if pressed(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
            || pressed(Modifiers::COMMAND, Key::Y)
        {
            self.redo();
        } else if pressed(Modifiers::COMMAND, Key::Z) {
            self.undo();
        }
        if ctx.egui_wants_keyboard_input() || self.infos.is_empty() {
            return;
        }
        // Con le quote accese Canc toglie la quota scelta (mai le pagine) ed Esc la lascia, poi
        // spegne le quote.
        if self.quoting {
            if ctx.input(|i| i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace)) {
                self.quotes.remove_selected();
                return;
            }
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                if self.quotes.selected.take().is_none() {
                    self.quoting = false;
                }
                return;
            }
        }
        // Con la taglierina aperta Invio taglia e Esc la chiude, come nelle immagini.
        if self.cutter.crop.is_some() {
            if ctx.input(|i| i.key_pressed(Key::Enter)) {
                self.cut(Scope::Chosen);
                return;
            }
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.cutter.clear();
                return;
            }
        }
        let n = self.infos.len();
        if pressed(Modifiers::COMMAND, Key::A) {
            self.selected = (0..n).collect();
        }
        if pressed(Modifiers::COMMAND, Key::R) {
            self.rotate(1);
        }
        if ctx.input(|i| i.key_pressed(Key::Delete)) {
            self.delete();
        }
        let step = ctx.input(|i| {
            if i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::PageDown)
            {
                1
            } else if i.key_pressed(Key::ArrowLeft)
                || i.key_pressed(Key::ArrowUp)
                || i.key_pressed(Key::PageUp)
            {
                -1
            } else {
                0
            }
        });
        if step != 0 {
            let next = (self.current as i64 + step).clamp(0, n as i64 - 1);
            self.current = usize::try_from(next).unwrap_or(0);
            let shift = ctx.input(|i| i.modifiers.shift);
            if shift {
                self.selected.insert(self.current);
            } else {
                self.selected = [self.current].into();
                self.anchor = Some(self.current);
            }
            self.reveal = true;
        }
    }

    // --- Menu e barre ----------------------------------------------------------------------

    /// Il menu File quando è aperto il modulo PDF.
    pub fn file_menu(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let item = |ui: &mut egui::Ui, text: &str, keys: &str, enabled: bool| {
            ui.add_enabled(enabled, egui::Button::new(text).shortcut_text(keys))
                .clicked()
        };
        let open = self.editor.is_some();
        if item(ui, "Apri PDF…", "Ctrl+O", true) {
            ui.close();
            self.guard(&ctx, Then::Open(None));
        }
        if item(ui, "Unisci PDF…", "", true) {
            ui.close();
            self.guard(&ctx, Then::Combine(Vec::new()));
        }
        ui.separator();
        if item(ui, "Salva", "Ctrl+S", open) {
            ui.close();
            self.save(false);
        }
        if item(ui, "Salva con nome…", "Ctrl+Maiusc+S", open) {
            ui.close();
            self.save(true);
        }
        if item(ui, "Stampa…", "Ctrl+P", open) {
            ui.close();
            self.print(&ctx);
        }
        ui.separator();
        if item(ui, "Aggiungi pagine da un PDF…", "", open) {
            ui.close();
            self.open_insert();
        }
        if item(ui, "Estrai pagine…", "", open) {
            ui.close();
            self.dialog = Some(Dialog::Extract { one_per_page: false });
        }
        if item(ui, "Dividi il PDF…", "", open) {
            ui.close();
            self.dialog = Some(Dialog::Split {
                every: true,
                count: 1,
                before: String::new(),
            });
        }
        ui.separator();
        if item(ui, "Chiudi il PDF", "", open) {
            ui.close();
            self.guard(&ctx, Then::CloseDocument);
        }
    }

    /// La barra dei comandi sotto ai menu.
    pub fn control_bar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE.fill(t.app_bar).inner_margin(egui::Margin {
            left: 14,
            right: 14,
            top: 2,
            bottom: 6,
        });
        egui::Panel::top("barra-pdf")
            .exact_size(self.bar_height)
            .frame(frame)
            .show(ui, |ui| {
                // Salva e Stampa a destra; i comandi a sinistra vanno a capo se la finestra è stretta.
                let area = ui.available_rect_before_wrap();
                let area = area.with_max_y(area.min.y + BAR_ROW);
                let ctx = ui.ctx().clone();
                let open = self.editor.is_some();
                let n = self.chosen().len();
                let which = if n > 1 {
                    format!("alle {n} pagine scelte")
                } else {
                    "alla pagina".into()
                };
                let mut right = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(area)
                        .layout(egui::Layout::right_to_left(Align::Center)),
                );
                {
                    let ui = &mut right;
                    let enabled = open;
                    let (fill, ink) = if enabled {
                        (t.primary, t.on_primary)
                    } else {
                        (t.button, t.text_disabled)
                    };
                    let save = egui::Button::image_and_text(
                        icons::image("save", 18.0, ink),
                        egui::RichText::new("Salva").font(semibold(14.0)).color(ink),
                    )
                    .fill(fill)
                    .min_size(vec2(108.0, 34.0));
                    // Il pulsante chiede sempre dove salvare; Ctrl+S riscrive l'ultimo file senza chiedere.
                    let tip = match &self.target {
                        Some(p) => format!(
                            "Salva il PDF modificato: propone {} (Ctrl+S lo riscrive senza chiedere)",
                            file_name(p)
                        ),
                        None => "Salva il PDF modificato con un nome nuovo (Ctrl+S)".into(),
                    };
                    if ui.add_enabled(enabled, save).on_hover_text(tip).clicked() {
                        self.save(true);
                    }
                    let tip = if self.quoting {
                        "Stampa il PDF com'è adesso, con le quote (Ctrl+P)"
                    } else {
                        "Stampa il PDF com'è adesso, anche con le modifiche non salvate (Ctrl+P)"
                    };
                    if text_button(ui, "printer", "Stampa", enabled)
                        .on_hover_text(tip)
                        .clicked()
                    {
                        self.print(&ctx);
                    }
                }
                let right_w = right.min_rect().width();
                let mut left = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(area.with_max_x((area.max.x - right_w - 16.0).max(area.min.x)))
                        .layout(egui::Layout::left_to_right(Align::Center).with_main_wrap(true)),
                );
                {
                    let ui = &mut left;
                    ui.spacing_mut().interact_size.y = 34.0;
                    if text_button(ui, "folder-open", "Apri", true)
                        .on_hover_text("Apri un PDF (Ctrl+O)")
                        .clicked()
                    {
                        self.guard(&ctx, Then::Open(None));
                    }
                    if icons::button(ui, "files", "Unisci più PDF in uno", false, true, 32.0).clicked() {
                        self.guard(&ctx, Then::Combine(Vec::new()));
                    }
                    if icons::button(
                        ui,
                        "file-plus",
                        "Aggiungi pagine da un altro PDF",
                        false,
                        open,
                        32.0,
                    )
                    .clicked()
                    {
                        self.open_insert();
                    }
                    separator(ui);
                    let (can_undo, can_redo) = self.editor.as_ref().map_or((None, None), |e| {
                        (
                            e.undo_label().map(str::to_string),
                            e.redo_label().map(str::to_string),
                        )
                    });
                    let undo_tip = can_undo
                        .as_ref()
                        .map_or("Annulla (Ctrl+Z)".into(), |l| format!("Annulla: {l} (Ctrl+Z)"));
                    if icons::button(ui, "undo-2", &undo_tip, false, can_undo.is_some(), 32.0).clicked() {
                        self.undo();
                    }
                    let redo_tip = can_redo
                        .as_ref()
                        .map_or("Ripeti (Ctrl+Y)".into(), |l| format!("Ripeti: {l} (Ctrl+Y)"));
                    if icons::button(ui, "redo-2", &redo_tip, false, can_redo.is_some(), 32.0).clicked() {
                        self.redo();
                    }
                    separator(ui);
                    if icons::button(ui, "rotate-ccw", "Ruota a sinistra", false, open, 32.0).clicked() {
                        self.rotate(-1);
                    }
                    if icons::button(ui, "rotate-cw", "Ruota a destra (Ctrl+R)", false, open, 32.0).clicked()
                    {
                        self.rotate(1);
                    }
                    if icons::button(ui, "copy-plus", "Duplica", false, open, 32.0).clicked() {
                        self.duplicate();
                    }
                    let can_delete = open && n < self.infos.len();
                    if icons::button(ui, "trash-2", "Elimina (Canc)", false, can_delete, 32.0).clicked() {
                        self.delete();
                    }
                    separator(ui);
                    if text_button(ui, "crop", "Rifila", open)
                        .on_hover_text(format!(
                            "Rifila al formato finito (TrimBox) {which}: l'abbondanza si nasconde"
                        ))
                        .clicked()
                    {
                        self.trim();
                    }
                    let free = self.cutter.crop.is_some();
                    if text_button(ui, "square-dashed", "Rifila libero", open)
                        .on_hover_text(
                            "Un riquadro sulla pagina da tirare per i lati o da scrivere a numeri, \
                             poi Taglia (si può anche trascinare sulla pagina)",
                        )
                        .clicked()
                    {
                        if free {
                            self.cutter.clear();
                        } else {
                            self.start_free_crop();
                        }
                    }
                    if text_button(ui, "frame", "Margini", open)
                        .on_hover_text("Aggiungi margini attorno alle pagine, o toglili con valori negativi")
                        .clicked()
                    {
                        self.dialog = Some(Dialog::margins());
                    }
                    if text_button(ui, "scaling", "Misura", open)
                        .on_hover_text("Porta le pagine a una misura in mm")
                        .clicked()
                    {
                        let size = self
                            .infos
                            .get(self.current)
                            .map_or(Size::new(210.0, 297.0), |p| p.size);
                        self.dialog = Some(Dialog::resize(size));
                    }
                    for paper in [Paper::A4, Paper::A3] {
                        if ui
                            .add_enabled(open, egui::Button::new(paper.name()).min_size(vec2(40.0, 32.0)))
                            .on_hover_text(format!(
                                "Adatta {which} ad {}, in verticale o in orizzontale come la pagina",
                                paper.name()
                            ))
                            .clicked()
                        {
                            self.fit_paper(paper);
                        }
                    }
                    separator(ui);
                    if icons::button(
                        ui,
                        "palette",
                        "Tinte piatte: elenco, rinomina e sovrastampa",
                        self.spots_open,
                        open,
                        32.0,
                    )
                    .clicked()
                    {
                        self.spots_open = !self.spots_open;
                        if self.spots_open {
                            self.quoting = false;
                            self.paths.on = false;
                        }
                    }
                    if text_toggle(ui, "ruler", "Quote", open, self.quoting)
                        .on_hover_text(
                            "Misure in proporzione su un rettangolo o una linea: scrivi una misura e \
                             le altre si adeguano (il PDF non cambia)",
                        )
                        .clicked()
                    {
                        self.quoting = !self.quoting;
                        if self.quoting {
                            self.cutter.clear();
                            self.paths.on = false;
                        }
                    }
                    if text_toggle(ui, "mouse-pointer-2", "Tracciati", open, self.paths.on)
                        .on_hover_text(
                            "Clicca una linea sulla pagina per vederne colore, spessore e lunghezza, e \
                             farla diventare una tinta (fustella, cordonatura…). Controlla anche \
                             fustella e abbondanza della pagina",
                        )
                        .clicked()
                    {
                        self.paths.on = !self.paths.on;
                        if self.paths.on {
                            self.quoting = false;
                            self.cutter.clear();
                        }
                    }
                    separator(ui);
                    if icons::button(ui, "file-output", "Estrai le pagine scelte", false, open, 32.0)
                        .clicked()
                    {
                        self.dialog = Some(Dialog::Extract { one_per_page: false });
                    }
                    if icons::button(
                        ui,
                        "scissors",
                        "Dividi il PDF in più file",
                        false,
                        open && self.infos.len() > 1,
                        32.0,
                    )
                    .clicked()
                    {
                        self.dialog = Some(Dialog::Split {
                            every: true,
                            count: 1,
                            before: String::new(),
                        });
                    }
                }
                let height = left.min_rect().height().max(BAR_ROW) + 8.0;
                if (height - self.bar_height).abs() > 0.5 {
                    self.bar_height = height;
                    ui.ctx().request_repaint();
                }
            });
    }

    pub fn status_bar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE
            .fill(t.app_bar)
            .inner_margin(egui::Margin::symmetric(16, 0));
        egui::Panel::bottom("barra-stato-pdf")
            .exact_size(32.0)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(e) = &self.editor {
                        let name = file_name(self.target.as_deref().unwrap_or(e.path()));
                        let mut text = format!("{name}  ·  {}", crate::canvas::pages_text(self.infos.len()));
                        if self.selected.len() > 1 {
                            text.push_str(&format!("  ·  {} scelte", self.selected.len()));
                        }
                        ui.label(egui::RichText::new(text).font(semibold(12.5)).color(t.text));
                    }
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if !self.status.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&self.status).size(12.0).color(t.text_dim),
                                )
                                .truncate(),
                            );
                        }
                    });
                });
            });
    }

    /// Le finestre aperte: domanda sulle modifiche e finestre dei comandi.
    pub fn windows(&mut self, ctx: &egui::Context) {
        self.unsaved_dialog(ctx);
        self.illustrator_dialog(ctx);
        if let Some(mut d) = self.dialog.take() {
            let info = dialogs::Context {
                chosen: self.chosen().len(),
                total: self.infos.len(),
                current: self.current,
                current_size: self
                    .infos
                    .get(self.current)
                    .map_or(Size::new(210.0, 297.0), |p| p.size),
            };
            match d.show(ctx, &info) {
                None => self.dialog = Some(d),
                Some(Some(action)) => self.act(action),
                Some(None) => {}
            }
        }
    }

    fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        let Some(then) = self.asking.clone() else {
            return;
        };
        let t = Tokens::get(ctx);
        let name = self
            .editor
            .as_ref()
            .map(|e| file_name(self.target.as_deref().unwrap_or(e.path())))
            .unwrap_or_default();
        let mut answer = None;
        let modal = egui::Modal::new(egui::Id::new("pdf-salvare")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(
                egui::RichText::new(format!("Salvare le modifiche a «{name}»?"))
                    .font(semibold(15.0))
                    .color(t.text_strong),
            );
            ui.add_space(6.0);
            ui.label("Se non salvi, le modifiche alle pagine andranno perse.");
            ui.add_space(14.0);
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), 36.0),
                egui::Layout::right_to_left(Align::Center),
                |ui| {
                    let save = egui::Button::new(
                        egui::RichText::new("Salva")
                            .font(semibold(13.0))
                            .color(t.on_primary),
                    )
                    .fill(t.primary)
                    .min_size(vec2(90.0, 30.0));
                    if ui.add(save).clicked() {
                        answer = Some(true);
                    }
                    if ui
                        .add(egui::Button::new("Non salvare").min_size(vec2(100.0, 30.0)))
                        .clicked()
                    {
                        answer = Some(false);
                    }
                    if ui
                        .add(egui::Button::new("Annulla").min_size(vec2(80.0, 30.0)))
                        .clicked()
                    {
                        self.asking = None;
                    }
                },
            );
        });
        if modal.should_close() {
            self.asking = None;
        }
        match answer {
            Some(true) => {
                self.asking = None;
                if self.save(false) {
                    self.run(ctx, then);
                } else if let Some((_, after)) = self.illustrator_ask.as_mut() {
                    *after = Some(then);
                }
            }
            Some(false) => {
                self.asking = None;
                if let Some(e) = self.editor.as_mut() {
                    e.discard_changes();
                }
                self.run(ctx, then);
            }
            None => {}
        }
    }

    /// Il PDF da salvare porta la copia di Illustrator: toglierla o tenerla (ADR 0020).
    fn illustrator_dialog(&mut self, ctx: &egui::Context) {
        if self.illustrator_ask.is_none() {
            return;
        }
        let t = Tokens::get(ctx);
        let mut answer = None;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("pdf-illustrator")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.label(
                egui::RichText::new("Questo PDF contiene il file di Illustrator")
                    .font(semibold(15.0))
                    .color(t.text_strong),
            );
            ui.add_space(6.0);
            ui.label(
                "Riaprendo il PDF, Illustrator mostra quella copia e non le modifiche fatte qui. \
                 Acrobat e la stampa usano sempre le pagine vere.",
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "Togliendola, Illustrator apre il PDF con le modifiche, ma come PDF importato \
                     e non più come il suo file .ai con livelli e testi modificabili.",
                )
                .color(t.text_dim),
            );
            ui.add_space(14.0);
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), 36.0),
                egui::Layout::right_to_left(Align::Center),
                |ui| {
                    let strip = egui::Button::new(
                        egui::RichText::new("Togli e salva")
                            .font(semibold(13.0))
                            .color(t.on_primary),
                    )
                    .fill(t.primary)
                    .min_size(vec2(110.0, 30.0));
                    if ui.add(strip).clicked() {
                        answer = Some(true);
                    }
                    if ui
                        .add(egui::Button::new("Tienila e salva").min_size(vec2(120.0, 30.0)))
                        .clicked()
                    {
                        answer = Some(false);
                    }
                    if ui
                        .add(egui::Button::new("Annulla").min_size(vec2(80.0, 30.0)))
                        .clicked()
                    {
                        cancel = true;
                    }
                },
            );
        });
        if cancel || modal.should_close() {
            self.illustrator_ask = None;
            return;
        }
        let Some(strip) = answer else {
            return;
        };
        let Some((ask, then)) = self.illustrator_ask.take() else {
            return;
        };
        self.illustrator = Some(strip);
        if strip {
            self.apply(|e| {
                e.strip_illustrator_data()?;
                Ok("Tolta la copia di Illustrator".into())
            });
        }
        if self.save(ask)
            && let Some(then) = then
        {
            self.run(ctx, then);
        }
    }

    // --- Corpo -----------------------------------------------------------------------------

    pub fn body(&mut self, ui: &mut egui::Ui, unit: Unit) {
        self.unit = unit;
        self.renderer.receive(ui.ctx());
        let full = ui.max_rect();
        if self.editor.is_none() {
            self.empty(ui, full);
            return;
        }
        let list_w = (full.width() * 0.3).clamp(170.0, 460.0);
        let list = Rect::from_min_size(full.min, vec2(list_w, full.height()));
        if self.paths.on
            && let Some(e) = &self.editor
        {
            let key = (self.generation, self.current);
            self.paths.update(ui.ctx(), key, || e.snapshot());
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.paths.selected.clear();
            }
        }
        let side_w = if self.spots_open || self.quoting || self.paths.on {
            SPOTS_W
        } else {
            0.0
        };
        let view = Rect::from_min_max(
            egui::pos2(list.right() + 10.0, full.top()),
            egui::pos2(full.right() - side_w, full.bottom()),
        );
        self.page_list(ui, list, unit);
        self.preview(ui, view, unit);
        let side = Rect::from_min_max(egui::pos2(view.right() + 10.0, full.top()), full.max);
        if self.quoting {
            self.quotes_panel(ui, side, unit);
        } else if self.paths.on {
            self.paths_panel(ui, side, unit);
        } else if self.spots_open {
            self.spots_panel(ui, side);
        }
    }

    fn empty(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        let c = rect.center();
        icons::paint(
            ui,
            "files",
            Rect::from_center_size(c - vec2(0.0, 64.0), Vec2::splat(48.0)),
            t.text_dim,
        );
        let p = ui.painter();
        p.text(
            c - vec2(0.0, 16.0),
            egui::Align2::CENTER_CENTER,
            "Trascina qui un PDF da modificare",
            semibold(17.0),
            t.text_strong,
        );
        p.text(
            c + vec2(0.0, 10.0),
            egui::Align2::CENTER_CENTER,
            "Più PDF insieme si uniscono in uno",
            egui::FontId::proportional(13.5),
            t.text_dim,
        );
        let button = Rect::from_center_size(c + vec2(0.0, 52.0), vec2(150.0, 34.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(button));
        let open = egui::Button::new(
            egui::RichText::new("Apri un PDF…")
                .font(semibold(13.5))
                .color(t.on_primary),
        )
        .fill(t.primary)
        .min_size(button.size());
        if child.add(open).clicked() {
            let ctx = ui.ctx().clone();
            self.run(&ctx, Then::Open(None));
        }
    }

    /// Chiede le immagini che servono: miniature di tutte le pagine e la pagina corrente grande.
    fn request(&mut self, preview_side: Option<u32>) {
        let Some(e) = self.editor.as_ref() else {
            return;
        };
        let mut wanted: Vec<(usize, render::Key)> = Vec::new();
        if let (Some(side), Some(&k)) = (preview_side, self.keys.get(self.current)) {
            wanted.push((self.current, (k, side)));
        }
        // Prima le miniature vicine alla pagina corrente.
        let mut order: Vec<usize> = (0..self.keys.len()).collect();
        order.sort_by_key(|&i| i.abs_diff(self.current));
        wanted.extend(
            order
                .into_iter()
                .filter_map(|i| self.keys.get(i).map(|&k| (i, (k, THUMB_SIDE)))),
        );
        self.renderer.request(self.generation, || e.snapshot(), &wanted);
    }

    fn page_list(&mut self, ui: &mut egui::Ui, rect: Rect, unit: Unit) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 44.0)).shrink2(vec2(14.0, 0.0));
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        bar.label(
            egui::RichText::new("Pagine")
                .font(semibold(14.0))
                .color(t.text_strong),
        );
        bar.label(egui::RichText::new(self.infos.len().to_string()).color(t.text_dim));
        bar.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if !self.selected.is_empty() && ui.small_button("Nessuna").clicked() {
                self.selected.clear();
            }
            if ui
                .small_button("Tutte")
                .on_hover_text("Sceglie tutte le pagine (Ctrl+A)")
                .clicked()
            {
                self.selected = (0..self.infos.len()).collect();
            }
        });
        let body = Rect::from_min_max(
            egui::pos2(rect.left() + 6.0, header.bottom()),
            rect.max - vec2(6.0, 8.0),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
        let n = self.infos.len();
        let cols = ((body.width() - 12.0) / CELL_W).floor().max(1.0) as usize;
        let rows = n.div_ceil(cols);
        let gap = ((body.width() - 12.0) - cols as f32 * CELL_W) / cols as f32;
        let row_h = CELL_H + 8.0;
        let mut cells: Vec<Rect> = Vec::with_capacity(n);
        let reveal = std::mem::take(&mut self.reveal);
        let current = self.current;
        egui::ScrollArea::vertical()
            .id_salt("pdf-pagine")
            .auto_shrink([false, false])
            .show_rows(&mut child, row_h, rows, |ui, range| {
                let start = range.start * cols;
                let top = ui.min_rect().top() - range.start as f32 * row_h;
                let left = ui.min_rect().left() + 6.0;
                // Spazio per tutte le righe, anche quelle non disegnate.
                ui.allocate_space(vec2(ui.available_width(), range.len() as f32 * row_h));
                for row in range {
                    for col in 0..cols {
                        let i = row * cols + col;
                        if i >= n {
                            break;
                        }
                        let min = egui::pos2(
                            left + col as f32 * (CELL_W + gap) + gap / 2.0,
                            top + row as f32 * row_h,
                        );
                        let cell = Rect::from_min_size(min, vec2(CELL_W, CELL_H));
                        cells.push(cell);
                        self.cell(ui, i, cell, unit);
                        if reveal && i == current {
                            ui.scroll_to_rect(cell, None);
                        }
                    }
                }
                self.drop_marker(ui, &cells, start);
            });
        self.finish_drag(ui);
    }

    fn cell(&mut self, ui: &mut egui::Ui, i: usize, cell: Rect, unit: Unit) {
        let t = Tokens::get(ui.ctx());
        let resp = ui.interact(cell, ui.id().with(("pagina", i)), Sense::click_and_drag());
        let chosen = self.selected.contains(&i);
        let is_current = i == self.current;
        let fill = if chosen {
            t.selection
        } else if resp.hovered() {
            t.hover
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(cell, t.radius + 2, fill);
        if is_current {
            ui.painter().rect_stroke(
                cell,
                t.radius + 2,
                Stroke::new(2.0, t.accent),
                egui::StrokeKind::Inside,
            );
        }
        let image_box = Rect::from_min_size(cell.min + vec2(10.0, 8.0), vec2(CELL_W - 20.0, CELL_H - 54.0));
        let info = self.infos.get(i).copied();
        let key = self.keys.get(i).map(|&k| (k, THUMB_SIDE));
        let size = info.map_or(Size::new(210.0, 297.0), |p| p.size);
        let paper = fit_rect(image_box, size);
        match key.and_then(|k| self.renderer.get(k)) {
            Some(Image::Ready(tex)) => {
                ui.painter().rect_filled(paper, 0.0, egui::Color32::WHITE);
                ui.painter().image(
                    tex.id(),
                    paper,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            Some(Image::Failed(e)) => {
                ui.painter().rect_filled(paper, 0.0, t.thumb);
                ui.painter().text(
                    paper.center(),
                    egui::Align2::CENTER_CENTER,
                    "!",
                    semibold(18.0),
                    t.error,
                );
                let _ = resp.clone().on_hover_text(e.clone());
            }
            None => {
                ui.painter().rect_filled(paper, 0.0, egui::Color32::WHITE);
            }
        }
        ui.painter()
            .rect_stroke(paper, 0.0, Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
        let text_top = image_box.bottom() + 6.0;
        ui.painter().text(
            egui::pos2(cell.center().x, text_top),
            egui::Align2::CENTER_TOP,
            (i + 1).to_string(),
            semibold(13.0),
            if chosen { t.accent_text } else { t.text_strong },
        );
        ui.painter().text(
            egui::pos2(cell.center().x, text_top + 18.0),
            egui::Align2::CENTER_TOP,
            format_size(size, unit),
            egui::FontId::proportional(11.5),
            t.text_dim,
        );

        if resp.clicked() {
            let mods = ui.input(|inp| inp.modifiers);
            self.pick(i, mods);
        }
        if resp.drag_started() {
            if !self.selected.contains(&i) {
                self.selected = [i].into();
                self.anchor = Some(i);
            }
            self.current = i;
            self.drag = Some(Drag {
                page: i,
                before: None,
            });
        }
        resp.context_menu(|ui| {
            if !self.selected.contains(&i) {
                self.selected = [i].into();
                self.current = i;
            }
            if ui.button("Ruota a sinistra").clicked() {
                ui.close();
                self.rotate(-1);
            }
            if ui.button("Ruota a destra").clicked() {
                ui.close();
                self.rotate(1);
            }
            if ui.button("Duplica").clicked() {
                ui.close();
                self.duplicate();
            }
            if ui.button("Rifila al vivo").clicked() {
                ui.close();
                self.trim();
            }
            if ui.button("Estrai…").clicked() {
                ui.close();
                self.dialog = Some(Dialog::Extract { one_per_page: false });
            }
            if ui
                .button("Manda il PDF al nesting")
                .on_hover_text("Apre la scheda Nesting con questo PDF, com'è salvato")
                .clicked()
            {
                ui.close();
                self.send_to_nesting();
            }
            ui.separator();
            let can_delete = self.selected.len() < self.infos.len();
            if ui.add_enabled(can_delete, egui::Button::new("Elimina")).clicked() {
                ui.close();
                self.delete();
            }
        });
    }

    fn pick(&mut self, i: usize, mods: egui::Modifiers) {
        if mods.shift
            && let Some(a) = self.anchor
        {
            let (lo, hi) = (a.min(i), a.max(i));
            if !mods.command {
                self.selected.clear();
            }
            self.selected.extend(lo..=hi);
        } else if mods.command {
            if !self.selected.insert(i) {
                self.selected.remove(&i);
            }
            self.anchor = Some(i);
        } else {
            self.selected = [i].into();
            self.anchor = Some(i);
        }
        self.current = i;
    }

    /// Mentre si trascina: la linea dove finirebbero le pagine. `cells` sono le caselle visibili,
    /// dalla pagina `start` in poi.
    fn drop_marker(&mut self, ui: &mut egui::Ui, cells: &[Rect], start: usize) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let Some(pointer) = ui.ctx().pointer_latest_pos() else {
            return;
        };
        let t = Tokens::get(ui.ctx());
        // La casella più vicina al puntatore, e da che parte.
        let Some((k, cell)) = cells.iter().enumerate().min_by(|a, b| {
            a.1.distance_sq_to_pos(pointer)
                .total_cmp(&b.1.distance_sq_to_pos(pointer))
        }) else {
            return;
        };
        let after = pointer.x > cell.center().x;
        drag.before = Some(start + k + usize::from(after));
        let x = if after {
            cell.right() + 2.0
        } else {
            cell.left() - 2.0
        };
        ui.painter().line_segment(
            [
                egui::pos2(x, cell.top() + 6.0),
                egui::pos2(x, cell.bottom() - 6.0),
            ],
            Stroke::new(3.0, t.accent),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }

    fn finish_drag(&mut self, ui: &mut egui::Ui) {
        if self.drag.is_none() || !ui.input(|i| i.pointer.any_released()) {
            return;
        }
        let Some(drag) = self.drag.take() else {
            return;
        };
        let Some(before) = drag.before else {
            return;
        };
        let moving: Vec<usize> = if self.selected.contains(&drag.page) {
            self.selected.iter().copied().collect()
        } else {
            vec![drag.page]
        };
        // Dove comincia il gruppo nel risultato: prima di `before` tolte le pagine che si spostano.
        let to = before - moving.iter().filter(|&&p| p < before).count();
        let unchanged = moving.iter().enumerate().all(|(k, &p)| p == to + k);
        if unchanged {
            return;
        }
        let count = moving.len();
        self.apply(|e| {
            e.move_pages(&moving, to)?;
            Ok(format!("Spostate {}", crate::canvas::pages_text(count)))
        });
        self.selected = (to..to + count).collect();
        self.current = to;
    }

    fn preview(&mut self, ui: &mut egui::Ui, rect: Rect, unit: Unit) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        let n = self.infos.len();
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 48.0)).shrink2(vec2(14.0, 0.0));
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        let info = self.infos.get(self.current).copied();
        bar.label(
            egui::RichText::new("Pagina")
                .font(semibold(14.0))
                .color(t.text_strong),
        );
        if icons::button(
            &mut bar,
            "chevron-left",
            "Pagina precedente",
            false,
            self.current > 0,
            28.0,
        )
        .clicked()
        {
            self.current -= 1;
            self.reveal = true;
        }
        bar.label(
            egui::RichText::new(format!("{} di {n}", self.current + 1))
                .font(semibold(13.0))
                .color(t.text_strong),
        );
        if icons::button(
            &mut bar,
            "chevron-right",
            "Pagina seguente",
            false,
            self.current + 1 < n,
            28.0,
        )
        .clicked()
        {
            self.current += 1;
            self.reveal = true;
        }
        if let Some(p) = info {
            let mut text = format_size(p.size, unit);
            if let Some(trim) = p.trim {
                text.push_str(&format!("  ·  al vivo {}", format_size(trim, unit)));
            }
            if p.rotation != 0 {
                text.push_str(&format!("  ·  ruotata di {}°", p.rotation));
            }
            bar.label(egui::RichText::new(text).color(t.text_dim));
        }
        bar.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            egui::ComboBox::from_id_salt("pdf-zoom")
                .icon(crate::fluent::combo_icon)
                .selected_text(match self.zoom {
                    None => format!("Adatta ({:.0}%)", self.shown_zoom * 100.0),
                    Some(z) => format!("{:.0}%", z * 100.0),
                })
                .width(130.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(self.zoom.is_none(), "Adatta alla finestra")
                        .clicked()
                    {
                        self.zoom = None;
                    }
                    for z in ZOOM_STOPS {
                        if ui
                            .selectable_label(self.zoom == Some(z / 100.0), format!("{z:.0}%"))
                            .clicked()
                        {
                            self.zoom = Some(z / 100.0);
                        }
                    }
                });
        });

        let board = Rect::from_min_max(
            egui::pos2(rect.left() + 6.0, header.bottom()),
            rect.max - vec2(6.0, 6.0),
        );
        ui.painter().rect_filled(board, t.radius, t.pasteboard);
        let size = info.map_or(Size::new(210.0, 297.0), |p| p.size);
        let natural = vec2(size.w as f32, size.h as f32) * POINTS_PER_MM;
        // Sotto la pagina, mentre c'è la taglierina, la barra con i numeri: la pagina le sta sopra.
        let reserve = if self.cutter.crop.is_some() {
            CUT_BAR_SPACE
        } else {
            0.0
        };
        let fit = ((board.width() - 48.0) / natural.x)
            .min((board.height() - 48.0 - reserve) / natural.y)
            .max(0.01);
        // Ctrl+rotella ingrandisce, come nell'anteprima delle miniature.
        // Dove sta l'angolo della pagina nel contenuto che scorre, a una scala (la pagina è al
        // centro di uno spazio largo almeno quanto l'anteprima).
        let paper_min = |scale: f32| {
            let shown = natural * scale;
            let area = vec2(shown.x + 48.0, shown.y + 48.0 + reserve).max(board.size());
            vec2((area.x - shown.x) / 2.0, (area.y - reserve - shown.y) / 2.0)
        };
        if let Some(z) = crate::app::wheel_zoom(ui, board) {
            let base = self.zoom.unwrap_or(fit);
            let new = (base * z).clamp(0.1, 8.0);
            // Il punto della pagina sotto il puntatore resta sotto il puntatore.
            if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
                let at = p - board.min;
                let now = ui.input(|i| i.time);
                let scroll = self.scroll;
                let on_page = self
                    .zoom_anchor
                    .point(p, now, || (scroll + at - paper_min(base)) / base);
                let to = paper_min(new) + on_page * new - at;
                self.scroll_to = Some(to.max(Vec2::ZERO));
            }
            self.zoom = Some(new);
        }
        let scale = self.zoom.unwrap_or(fit);
        self.shown_zoom = scale;
        let shown = natural * scale;
        let hand = crate::pan::hand(ui, board);
        if let Some(d) = hand
            && d != Vec2::ZERO
        {
            let from = self.scroll_to.unwrap_or(self.scroll);
            self.scroll_to = Some((from - d).max(Vec2::ZERO));
        }
        let ppp = ui.ctx().pixels_per_point();
        let needed = (shown.x.max(shown.y) * ppp).ceil() as u32;
        let side = [768_u32, 1536, 3072, 4096]
            .into_iter()
            .find(|s| *s >= needed)
            .unwrap_or(4096);
        self.request(Some(side));

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(board));
        child.set_clip_rect(board);
        let mut lens = None;
        let mut area = egui::ScrollArea::both();
        let wanted = self.scroll_to.take();
        if let Some(to) = wanted {
            area = area.scroll_offset(to);
        }
        let out = area
            .id_salt("pdf-anteprima")
            .auto_shrink([false, false])
            // Trascinare sulla pagina disegna la taglierina, non sposta la vista.
            .scroll_source(
                egui::scroll_area::ScrollSource::MOUSE_WHEEL | egui::scroll_area::ScrollSource::SCROLL_BAR,
            )
            .show(&mut child, |ui| {
                let area = vec2(shown.x + 48.0, shown.y + 48.0 + reserve).max(board.size());
                let (space, _) = ui.allocate_exact_size(area, Sense::hover());
                let paper =
                    Rect::from_center_size(space.with_max_y(space.bottom() - reserve).center(), shown);
                let key = self.keys.get(self.current).copied();
                let big = key.and_then(|k| self.renderer.get((k, side)));
                let small = key.and_then(|k| self.renderer.get((k, THUMB_SIDE)));
                ui.painter().rect_filled(
                    paper.translate(vec2(0.0, 2.0)).expand(1.0),
                    2.0,
                    egui::Color32::from_black_alpha(40),
                );
                ui.painter().rect_filled(paper, 0.0, egui::Color32::WHITE);
                // Finché quella grande non c'è, la miniatura ingrandita.
                match big.or(small) {
                    Some(Image::Ready(tex)) => {
                        ui.painter().image(
                            tex.id(),
                            paper,
                            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                    Some(Image::Failed(e)) => {
                        ui.painter().text(
                            paper.center(),
                            egui::Align2::CENTER_CENTER,
                            format!("Pagina non disegnabile: {e}"),
                            egui::FontId::proportional(13.0),
                            t.error,
                        );
                    }
                    None => {}
                }
                if big.is_none() {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(100));
                }
                let (w, h) = (size.w as f32, size.h as f32);
                // Con la mano attiva trascinare sposta la vista: quote e taglierina stanno ferme.
                let at = if self.quoting {
                    self.quotes.prepare(key.unwrap_or(0), 1.0);
                    if hand.is_none() {
                        self.quotes.interact(ui, ui.id().with("quote"), paper, w, h);
                    }
                    self.quotes.paint(ui, paper, w, unit);
                    self.quotes.lens
                } else if self.paths.on {
                    if hand.is_none() {
                        self.paths.interact(ui, ui.id().with("tracciati"), paper, w);
                    }
                    None
                } else {
                    if hand.is_none() {
                        self.cutter.interact(ui, ui.id().with("taglierina"), paper, w, h);
                    }
                    self.cutter.paint(ui, paper, w);
                    self.cutter.lens
                };
                if let (Some(at), Some(Image::Ready(tex))) = (at, big.or(small)) {
                    lens = Some((at, paper, tex.id()));
                }
            });
        self.scroll = out.state.offset;
        // Appena ingrandito, lo scorrimento può essere tagliato sulla misura vecchia del
        // contenuto: si richiede al giro dopo, finché non arriva (o non può arrivare oltre).
        if let Some(to) = wanted {
            let max = (out.content_size - out.inner_rect.size()).max(Vec2::ZERO);
            let reachable = to.min(max);
            if (out.state.offset - reachable).length() > 0.5 {
                self.scroll = to;
                self.scroll_to = Some(to);
                ui.ctx().request_repaint();
            }
        }
        if let Some((at, paper, tex)) = lens {
            paint_lens(ui, board, paper, at, tex);
        }
        if self.cutter.crop.is_some() {
            self.cut_bar(ui, board, unit, size);
        }
    }

    /// Sotto la pagina, mentre c'è un rettangolo: la misura e i pulsanti per tagliare.
    fn cut_bar(&mut self, ui: &mut egui::Ui, board: Rect, unit: Unit, page: Size) {
        let Some(c) = self.cutter.crop.map(crate::cutter::Crop::normalized) else {
            return;
        };
        let t = Tokens::get(ui.ctx());
        let chosen = self.chosen().len();
        let total = self.infos.len();
        let area = Rect::from_center_size(
            egui::pos2(board.center().x, board.bottom() - 12.0 - CUT_BAR_HEIGHT / 2.0),
            vec2(board.width().min(620.0) - 24.0, CUT_BAR_HEIGHT),
        );
        ui.painter().rect(
            area,
            8.0,
            t.panel,
            Stroke::new(1.0, t.border),
            egui::StrokeKind::Outside,
        );
        // Sopra: il riquadro in numeri, da aggiustare dopo averlo tracciato a occhio.
        let mut fields = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(area.shrink2(vec2(12.0, 10.0)).with_max_y(area.top() + 62.0))
                .layout(egui::Layout::left_to_right(Align::Min)),
        );
        let (mut x, mut y) = (f64::from(c.x0), f64::from(c.y0));
        let (mut w, mut h) = (f64::from(c.width()), f64::from(c.height()));
        let before = (x, y, w, h);
        let column = |ui: &mut egui::Ui, label: &str, hint: &str, value: &mut f64, max: f64| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(label).size(11.5).color(t.text_dim));
                crate::fluent::number(ui, 104.0, length_field(value, max, unit)).on_hover_text(hint);
            });
            ui.add_space(8.0);
        };
        column(
            &mut fields,
            "Da sinistra",
            "Distanza del riquadro dal bordo sinistro della pagina",
            &mut x,
            page.w - 1.0,
        );
        column(
            &mut fields,
            "Dall'alto",
            "Distanza del riquadro dal bordo in alto della pagina",
            &mut y,
            page.h - 1.0,
        );
        column(&mut fields, "Larghezza", "Larghezza del riquadro", &mut w, page.w);
        column(&mut fields, "Altezza", "Altezza del riquadro", &mut h, page.h);
        if (x, y, w, h) != before {
            // Il riquadro resta dentro la pagina e largo almeno 1 mm.
            let x = x.clamp(0.0, (page.w - 1.0).max(0.0));
            let y = y.clamp(0.0, (page.h - 1.0).max(0.0));
            let w = w.clamp(1.0, (page.w - x).max(1.0));
            let h = h.clamp(1.0, (page.h - y).max(1.0));
            self.cutter.crop = Some(crate::cutter::Crop {
                x0: x as f32,
                y0: y as f32,
                x1: (x + w) as f32,
                y1: (y + h) as f32,
            });
        }
        // Sotto: i pulsanti.
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(area.with_min_y(area.bottom() - 44.0))
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        bar.add_space(12.0);
        bar.label(
            egui::RichText::new(format!("Pagina {}", format_size(page, unit)))
                .size(12.0)
                .color(t.text_dim),
        );
        let mut action = None;
        bar.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(8.0);
            if ui
                .add(egui::Button::new("Annulla").min_size(vec2(0.0, 30.0)))
                .on_hover_text("Togli la taglierina (Esc)")
                .clicked()
            {
                action = Some(None);
            }
            if total > 1
                && chosen < total
                && ui
                    .add(egui::Button::new("Tutte le pagine").min_size(vec2(0.0, 30.0)))
                    .on_hover_text("Taglia allo stesso riquadro tutte le pagine del PDF")
                    .clicked()
            {
                action = Some(Some(Scope::All));
            }
            let label = if chosen > 1 {
                format!("Taglia le {chosen} pagine")
            } else {
                "Taglia la pagina".into()
            };
            let button = egui::Button::new(
                egui::RichText::new(label)
                    .font(semibold(13.0))
                    .color(t.on_primary),
            )
            .fill(t.primary)
            .min_size(vec2(0.0, 30.0));
            if ui.add(button).on_hover_text("Invio").clicked() {
                action = Some(Some(Scope::Chosen));
            }
        });
        match action {
            Some(Some(scope)) => self.cut(scope),
            Some(None) => self.cutter.clear(),
            None => {}
        }
    }
}

/// Larghezza del pannello delle tinte piatte (e delle quote), con lo spazio prima.
const SPOTS_W: f32 = 290.0;

impl PdfModule {
    /// Il pannello a destra con le quote della pagina.
    fn quotes_panel(&mut self, ui: &mut egui::Ui, rect: Rect, unit: Unit) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        let inner = rect.shrink2(vec2(14.0, 10.0));
        let mut ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::top_down(Align::Min)),
        );
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Quote")
                    .font(semibold(14.0))
                    .color(t.text_strong),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if icons::button(ui, "x", "Spegni le quote (Esc)", false, true, 24.0).clicked() {
                    self.quoting = false;
                }
            });
        });
        ui.add_space(6.0);
        let action = egui::ScrollArea::vertical()
            .id_salt("pdf-quote")
            .auto_shrink([false, false])
            .show(&mut ui, |ui| self.quotes.panel(ui, unit))
            .inner;
        let ctx = ui.ctx().clone();
        match action {
            Some(crate::quotes::PanelAction::Print) => self.print(&ctx),
            Some(crate::quotes::PanelAction::Save) => self.save_quoted(&ctx),
            None => {}
        }
    }

    /// Il PDF com'è adesso (anche con le modifiche non salvate) in un file temporaneo, e le sue
    /// pagine `pages` pronte per la stampa, con le quote sulla pagina che le ha.
    fn output_pages(
        &mut self,
        ctx: &egui::Context,
        pages: impl Iterator<Item = usize>,
    ) -> Option<Vec<winfiles_scene::Page>> {
        let bytes = match self.editor.as_ref().map(Editor::to_bytes) {
            Some(Ok(b)) => b,
            Some(Err(e)) => {
                self.status = format!("Non riesco a preparare la stampa: {e}");
                return None;
            }
            None => return None,
        };
        let path = crate::quotes::temp_source("pdf");
        if let Err(e) = std::fs::write(&path, bytes) {
            self.status = format!("Non riesco a preparare la stampa: {e}");
            return None;
        }
        let owner = if self.quoting { self.quotes.owner() } else { None };
        Some(
            pages
                .filter_map(|i| {
                    let size = self.infos.get(i)?.size;
                    let source = winfiles_scene::Source {
                        path: path.clone(),
                        kind: winfiles_model::FileKind::Pdf,
                        page: i,
                    };
                    let quoted = owner.is_some() && self.keys.get(i).copied() == owner;
                    let quotes = quoted.then_some((&self.quotes, self.unit));
                    Some(crate::quotes::page(ctx, size, source, quotes))
                })
                .collect(),
        )
    }

    /// Stampa il PDF com'è adesso; le quote, se accese, escono sulla loro pagina.
    pub fn print(&mut self, ctx: &egui::Context) {
        if self.editor.is_none() || self.output.is_some() {
            return;
        }
        let Some(pages) = self.output_pages(ctx, 0..self.infos.len()) else {
            return;
        };
        let title = self
            .editor
            .as_ref()
            .map_or_else(|| "PDF".into(), |e| file_name(e.path()));
        self.output = Some(crate::quotes::Output::Print {
            pages,
            title,
            current: self.current,
        });
    }

    /// Salva in un PDF a parte la pagina con le quote disegnate sopra, alla misura vera.
    fn save_quoted(&mut self, ctx: &egui::Context) {
        let Some(e) = self.editor.as_ref() else {
            return;
        };
        let source = e.path();
        let dir = source.parent().map(Path::to_path_buf).unwrap_or_default();
        let name = if self.infos.len() > 1 {
            format!("{}_pagina{}_quote.pdf", file_stem(source), self.current + 1)
        } else {
            format!("{}_quote.pdf", file_stem(source))
        };
        let Some(path) = pdf_dialog()
            .set_title("Salva la pagina con le quote")
            .set_directory(dir)
            .set_file_name(name)
            .save_file()
        else {
            return;
        };
        let path = with_pdf_extension(path);
        if self.editor.as_ref().is_some_and(|e| e.path() == path.as_path()) {
            self.status = "Scegli un altro nome: il PDF aperto resta com'è".into();
            return;
        }
        if let Some(pages) = self.output_pages(ctx, std::iter::once(self.current)) {
            self.output = Some(crate::quotes::Output::Save { pages, path });
        }
    }

    /// Il pannello a destra con le tinte piatte: colore, nome (si rinomina), pagine e sovrastampa.
    /// Le tinte piatte del documento, rilette se il documento è cambiato.
    fn fresh_spots(&mut self) {
        if self.spots.as_ref().is_none_or(|(g, _)| *g != self.generation)
            && let Some(e) = &self.editor
        {
            let list = e.spots().map_err(|e| e.to_string());
            self.spots = Some((self.generation, list));
        }
    }

    /// Il pannello dello strumento Seleziona tracciato.
    fn paths_panel(&mut self, ui: &mut egui::Ui, rect: Rect, unit: Unit) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        self.fresh_spots();
        let spots = match &self.spots {
            Some((_, Ok(list))) => list.clone(),
            _ => Vec::new(),
        };
        let inner = rect.shrink2(vec2(14.0, 10.0));
        let mut ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::top_down(Align::Min)),
        );
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Tracciati")
                    .font(semibold(14.0))
                    .color(t.text_strong),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if icons::button(ui, "x", "Chiudi lo strumento", false, true, 24.0).clicked() {
                    self.paths.on = false;
                }
            });
        });
        ui.add_space(6.0);
        let page = self.current;
        match self.paths.panel(&mut ui, unit, &mut self.inks, &spots) {
            None => {}
            Some(tracciati::Request::Restyle(ids, change)) => {
                let mut new_ids = None;
                self.apply(|e| {
                    let r = e.restyle_paths(page, &ids, &change)?;
                    new_ids = Some(r.ids.clone());
                    let n = |k: usize| {
                        if k == 1 {
                            "1 tracciato".to_string()
                        } else {
                            format!("{k} tracciati")
                        }
                    };
                    let mut msg = match (&change.ink, r.done) {
                        (_, 0) => "Niente da cambiare".to_string(),
                        (Some(winfiles_pdf_edit::NewInk::Spot { name, .. }), k) => {
                            format!("{} in tinta {}", n(k), name.trim())
                        }
                        (_, k) => format!("{} cambiati", n(k)),
                    };
                    if r.skipped > 0 {
                        msg.push_str(&format!(" ({} di ritaglio lasciati com'erano)", r.skipped));
                    }
                    Ok(msg)
                });
                if let Some(ids) = new_ids.filter(|v| !v.is_empty()) {
                    self.paths.select_after_scan(ids);
                }
            }
            Some(tracciati::Request::Bleed {
                spec,
                die,
                paper,
                only,
                outline,
            }) => {
                self.apply(|e| {
                let left = if outline.is_empty() {
                    e.add_bleed(page, &spec, &die, &paper, &only)?
                } else {
                    e.add_shaped_bleed(page, spec.amount, &outline, &die, &paper, &only)?
                };
                let mut msg = format!(
                    "Abbondanza di {} mm creata",
                    winfiles_geom::format_number(spec.amount, 1, true)
                );
                if left > 0 {
                    msg.push_str(&format!(
                        " ({left} tracciati della fustella stanno in un gruppo del PDF e si sono ribaltati anche loro)"
                    ));
                }
                Ok(msg)
            });
            }
            Some(tracciati::Request::Die(ids, die)) => {
                self.apply(|e| {
                    let n = e.add_die(page, &ids, &die)?;
                    Ok(if n == 1 {
                        format!("Fustella «{}» creata", die.name.trim())
                    } else {
                        format!("Fustella «{}» creata: {n} contorni", die.name.trim())
                    })
                });
                self.paths.selected.clear();
            }
        }
    }

    fn spots_panel(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        self.fresh_spots();
        let inner = rect.shrink2(vec2(14.0, 10.0));
        let mut ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::top_down(Align::Min)),
        );
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Tinte piatte")
                    .font(semibold(14.0))
                    .color(t.text_strong),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if icons::button(ui, "x", "Chiudi il pannello", false, true, 24.0).clicked() {
                    self.spots_open = false;
                }
            });
        });
        ui.add_space(6.0);
        let spots = match &self.spots {
            Some((_, Ok(list))) => list.clone(),
            Some((_, Err(e))) => {
                ui.colored_label(t.error, format!("Tinte non leggibili: {e}"));
                return;
            }
            None => Vec::new(),
        };
        if spots.is_empty() {
            ui.label(egui::RichText::new("Nessuna tinta piatta in questo PDF.").color(t.text_dim));
            return;
        }
        let total = self.infos.len();
        let mut rename: Option<(Vec<u8>, String)> = None;
        let mut overprint: Option<(Vec<u8>, bool)> = None;
        egui::ScrollArea::vertical()
            .id_salt("pdf-tinte")
            .auto_shrink([false, false])
            .show(&mut ui, |ui| {
                for s in &spots {
                    egui::Frame::NONE
                        .fill(t.panel_darker)
                        .corner_radius(t.radius)
                        .inner_margin(egui::Margin::same(10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let (dot, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                                match s.swatch {
                                    Some([r, g, b]) => {
                                        ui.painter().rect(
                                            dot,
                                            4.0,
                                            egui::Color32::from_rgb(r, g, b),
                                            Stroke::new(1.0, t.border),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    None => {
                                        ui.painter().rect_stroke(
                                            dot,
                                            4.0,
                                            Stroke::new(1.0, t.border),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                }
                                let editing = self.renaming.as_ref().is_some_and(|(raw, _)| *raw == s.raw);
                                if editing {
                                    if let Some((raw, text)) = self.renaming.as_mut() {
                                        let edit = egui::TextEdit::singleline(text)
                                            .desired_width(ui.available_width() - 4.0);
                                        let resp = crate::fluent::text_field(ui, edit);
                                        resp.request_focus();
                                        let (enter, esc) = ui.input(|i| {
                                            (
                                                i.key_pressed(egui::Key::Enter),
                                                i.key_pressed(egui::Key::Escape),
                                            )
                                        });
                                        if enter || (resp.lost_focus() && !esc) {
                                            rename = Some((raw.clone(), text.clone()));
                                        } else if esc {
                                            rename = Some((Vec::new(), String::new()));
                                        }
                                    }
                                } else {
                                    let name = ui
                                        .add(
                                            egui::Label::new(
                                                egui::RichText::new(&s.name)
                                                    .font(semibold(13.0))
                                                    .color(t.text_strong),
                                            )
                                            .truncate()
                                            .sense(Sense::click()),
                                        )
                                        .on_hover_text(format!("{}\nDoppio clic per rinominare", s.name));
                                    if name.double_clicked() {
                                        self.renaming = Some((s.raw.clone(), s.name.clone()));
                                    }
                                }
                            });
                            let pages = match s.pages.len() {
                                0 => "non usata nelle pagine".to_string(),
                                n if n == total && total > 1 => "su tutte le pagine".to_string(),
                                _ => pages_list(&s.pages),
                            };
                            ui.label(egui::RichText::new(pages).size(12.0).color(t.text_dim));
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                let mut on = s.overprint == Overprint::All;
                                let used = s.overprint != Overprint::Unused;
                                let label = if s.overprint == Overprint::Some {
                                    "Sovrastampa (solo in parte)"
                                } else {
                                    "Sovrastampa"
                                };
                                let resp = ui
                                    .add_enabled(used, crate::fluent::checkbox(&mut on, label))
                                    .on_hover_text(
                                        "Gli oggetti di questa tinta si stampano sopra gli altri colori, \
                                         senza togliere quello che c'è sotto",
                                    );
                                if resp.changed() {
                                    overprint = Some((s.raw.clone(), on));
                                }
                                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                    if icons::button(ui, "pencil", "Rinomina la tinta", false, true, 24.0)
                                        .clicked()
                                    {
                                        self.renaming = Some((s.raw.clone(), s.name.clone()));
                                    }
                                });
                            });
                        });
                    ui.add_space(6.0);
                }
            });
        if let Some((raw, text)) = rename {
            self.renaming = None;
            let unchanged = spots.iter().any(|s| s.raw == raw && s.name == text.trim());
            if !raw.is_empty() && !unchanged {
                self.apply(|e| {
                    e.rename_spot(&raw, &text)?;
                    Ok(format!("Tinta rinominata in {}", text.trim()))
                });
            }
        }
        if let Some((raw, on)) = overprint {
            let name = String::from_utf8_lossy(&raw).into_owned();
            self.apply(|e| {
                Ok(match (e.set_spot_overprint(&raw, on)?, on) {
                    (0, _) => format!("{name}: niente da cambiare"),
                    (_, true) => format!("{name} in sovrastampa"),
                    (_, false) => format!("{name} senza sovrastampa"),
                })
            });
        }
    }
}

/// "pag. 1, 3, 5–8": le pagine (da 0) in breve, con gli intervalli.
fn pages_list(pages: &[usize]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while let Some(&start) = pages.get(i) {
        let mut end = start;
        while pages.get(i + 1) == Some(&(end + 1)) {
            end += 1;
            i += 1;
        }
        parts.push(if end > start + 1 {
            format!("{}–{}", start + 1, end + 1)
        } else if end == start + 1 {
            format!("{}, {}", start + 1, end + 1)
        } else {
            format!("{}", start + 1)
        });
        i += 1;
    }
    let prefix = if pages.len() == 1 { "pag." } else { "pagg." };
    format!("{prefix} {}", parts.join(", "))
}

/// Quanto ingrandisce la lente della taglierina.
const LENS_ZOOM: f32 = 5.0;
/// Lato della lente a schermo, in punti.
const LENS_SIDE: f32 = 116.0;

/// La lente: mentre si tira la taglierina mostra ingrandito il punto sotto il cursore.
fn paint_lens(ui: &egui::Ui, board: Rect, paper: Rect, at: egui::Pos2, tex: egui::TextureId) {
    let t = Tokens::get(ui.ctx());
    let span = LENS_SIDE / LENS_ZOOM;
    let uv = |x: f32, w: f32| x / w.max(1.0);
    let half = vec2(uv(span, paper.width()), uv(span, paper.height())) / 2.0;
    let c = egui::pos2(
        uv(at.x - paper.left(), paper.width()),
        uv(at.y - paper.top(), paper.height()),
    );
    let mut rect = Rect::from_min_size(at + vec2(18.0, 18.0), Vec2::splat(LENS_SIDE));
    if rect.right() > board.right() {
        rect = rect.translate(vec2(-LENS_SIDE - 36.0, 0.0));
    }
    if rect.bottom() > board.bottom() {
        rect = rect.translate(vec2(0.0, -LENS_SIDE - 36.0));
    }
    let p = ui.painter();
    p.rect_filled(rect.expand(2.0), 4.0, t.panel);
    p.rect_filled(rect, 0.0, egui::Color32::WHITE);
    p.image(
        tex,
        rect,
        Rect::from_min_max(c - half, c + half),
        egui::Color32::WHITE,
    );
    p.rect_stroke(rect, 4.0, Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
    let mid = rect.center();
    let cross = Stroke::new(1.0, t.accent);
    p.line_segment(
        [egui::pos2(mid.x, rect.top()), egui::pos2(mid.x, rect.bottom())],
        cross,
    );
    p.line_segment(
        [egui::pos2(rect.left(), mid.y), egui::pos2(rect.right(), mid.y)],
        cross,
    );
}

/// Il rettangolo con le proporzioni di `size` più grande che sta in `bounds`, al centro.
fn fit_rect(bounds: Rect, size: Size) -> Rect {
    let (w, h) = (size.w.max(0.1) as f32, size.h.max(0.1) as f32);
    let s = (bounds.width() / w).min(bounds.height() / h);
    Rect::from_center_size(bounds.center(), vec2(w * s, h * s))
}

/// L'altezza di una riga della barra dei comandi.
const BAR_ROW: f32 = 38.0;

fn separator(ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(13.0, 24.0), Sense::hover());
    ui.painter()
        .vline(r.center().x, r.y_range(), Stroke::new(1.0, t.divider));
}

/// Come [`text_button`], ma resta evidenziato finché `on`: per gli strumenti che si accendono.
fn text_toggle(ui: &mut egui::Ui, icon: &str, text: &str, enabled: bool, on: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let tint = match (enabled, on) {
        (false, _) => t.text_disabled,
        (true, true) => t.accent_text,
        (true, false) => t.text_strong,
    };
    let mut button = egui::Button::image_and_text(
        icons::image(icon, 17.0, tint),
        egui::RichText::new(text).color(tint),
    )
    .min_size(vec2(0.0, 32.0));
    if on && enabled {
        button = button.fill(t.selection).stroke(Stroke::new(1.0, t.accent));
    }
    ui.add_enabled(enabled, button)
}

fn text_button(ui: &mut egui::Ui, icon: &str, text: &str, enabled: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let tint = if enabled { t.text_strong } else { t.text_disabled };
    ui.add_enabled(
        enabled,
        egui::Button::image_and_text(
            icons::image(icon, 17.0, tint),
            egui::RichText::new(text).color(tint),
        )
        .min_size(vec2(0.0, 32.0)),
    )
}

fn pdf_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("PDF", &["pdf", "PDF"])
}

fn is_pdf(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

fn with_pdf_extension(path: PathBuf) -> PathBuf {
    if is_pdf(&path) {
        path
    } else {
        path.with_extension("pdf")
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn file_stem(p: &Path) -> String {
    p.file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "PDF".into())
}

/// L'altezza della barra della taglierina e lo spazio che le si lascia sotto la pagina.
const CUT_BAR_HEIGHT: f32 = 108.0;
const CUT_BAR_SPACE: f32 = CUT_BAR_HEIGHT + 12.0;

/// Una misura in millimetri mostrata e scritta nell'unità scelta (mm o cm), con la virgola.
fn length_field(value: &mut f64, max: f64, unit: Unit) -> egui::DragValue<'_> {
    let (factor, suffix) = match unit {
        Unit::Mm => (1.0, "mm"),
        Unit::Cm => (10.0, "cm"),
    };
    egui::DragValue::new(value)
        .range(0.0..=max.max(0.0))
        .speed(0.2)
        .custom_formatter(move |v, _| {
            format!("{} {suffix}", winfiles_geom::format_number(v / factor, 1, false))
        })
        .custom_parser(move |text| {
            let clean = text.trim().trim_end_matches(suffix).trim().replace(',', ".");
            clean.parse::<f64>().ok().map(|v| v * factor)
        })
}

#[cfg(test)]
mod tests {
    use super::pages_list;

    #[test]
    fn pagine_in_breve() {
        assert_eq!(pages_list(&[0]), "pag. 1");
        assert_eq!(pages_list(&[0, 2]), "pagg. 1, 3");
        assert_eq!(pages_list(&[0, 1, 4, 5, 6, 7]), "pagg. 1, 2, 5–8");
    }
}
