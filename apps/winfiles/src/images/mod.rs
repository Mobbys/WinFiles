//! Il modulo immagini (Fase 8): apre una o più immagini e le controlla e corregge, come
//! "Controllo immagini" della 4.9. A sinistra le immagini aperte, al centro quella scelta in
//! grande con il ritaglio da trascinare, a destra i dati tecnici e la misura di stampa.
//! Le modifiche le fa `winfiles-image-edit`; qui ci sono solo la finestra e le scelte.

mod dialogs;
mod render;

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use egui::{Align, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use winfiles_geom::{Size, Unit, format_area, format_number, format_size};
use winfiles_image_edit::{Axis, CropPx, Editor, Format, Op, SaveOptions};

use crate::app::card;
use crate::cutter::{Crop, Cutter};
use crate::icons;
use crate::quotes::Quotes;
use crate::theme::{Tokens, semibold};
use dialogs::{Action, Dialog};

/// Lato lungo delle miniature nell'elenco a sinistra, in pixel.
const THUMB_SIDE: u32 = 320;
const CELL_H: f32 = 96.0;
/// Quanto ingrandisce la lente sul ritaglio, come nella 4.9.
const LENS_ZOOM: f32 = 5.0;
/// Lato della lente a schermo, in punti.
const LENS_SIDE: f32 = 116.0;

/// Cosa fare dopo la domanda "salvare le modifiche?".
#[derive(Clone, Debug)]
enum Then {
    Close(usize),
    Quit,
}

/// Un'immagine aperta.
struct Doc {
    editor: Editor,
    /// Cresce a ogni modifica: le immagini già disegnate di una versione vecchia si buttano.
    generation: u64,
    /// Il file su cui scrive "Salva". Finché non c'è, salvare chiede il nome: l'originale non si
    /// sovrascrive senza dirlo (come la 4.9, che proponeva "_modificato").
    target: Option<PathBuf>,
    options: SaveOptions,
}

/// I campi della misura di stampa, mentre si scrivono.
#[derive(Default)]
struct Fields {
    width: String,
    height: String,
    dpi: String,
    /// Di quale immagine e versione sono: se cambia, si riempiono di nuovo.
    owner: Option<(usize, u64)>,
}

pub struct ImagesModule {
    docs: Vec<Doc>,
    current: usize,
    /// La taglierina sull'anteprima, in pixel dell'immagine.
    cutter: Cutter,
    /// Le quote sono accese: trascinare sull'immagine traccia quote invece del ritaglio, e a
    /// destra c'è il loro pannello al posto dei dati.
    quoting: bool,
    /// Le quote dell'immagine corrente, in pixel.
    quotes: Quotes,
    fields: Fields,
    dialog: Option<Dialog>,
    asking: Option<Then>,
    renderer: render::Renderer,
    pub status: String,
    zoom: Option<f32>,
    shown_zoom: f32,
    /// L'unità scelta nell'app (mm o cm): la usano i campi e i dati.
    unit: Unit,
    /// Le immagini si aprono in un thread a parte (sviluppare un RAW richiede un paio di
    /// secondi): arrivano da qui, una alla volta.
    loading: Loading,
    pub quit_confirmed: bool,
    /// La stampa o il PDF con le quote chiesti, che l'app prende e manda avanti.
    pub output: Option<crate::quotes::Output>,
}

/// Le aperture in corso.
struct Loading {
    tx: mpsc::Sender<(PathBuf, Result<Editor, String>)>,
    rx: mpsc::Receiver<(PathBuf, Result<Editor, String>)>,
    wake: egui::Context,
    /// File chiesti e non ancora arrivati.
    pending: usize,
    /// Della richiesta in corso: quante aperte, quali no, e se la prima è già stata mostrata.
    opened: usize,
    failed: Vec<String>,
    shown: bool,
}

impl ImagesModule {
    pub fn new(ctx: &egui::Context) -> Self {
        let wake = ctx.clone();
        Self {
            docs: Vec::new(),
            current: 0,
            cutter: Cutter::default(),
            quoting: false,
            quotes: Quotes::default(),
            fields: Fields::default(),
            dialog: None,
            asking: None,
            renderer: render::Renderer::start(move || wake.request_repaint()),
            status: String::new(),
            zoom: None,
            shown_zoom: 1.0,
            unit: Unit::Mm,
            loading: {
                let (tx, rx) = mpsc::channel();
                Loading {
                    tx,
                    rx,
                    wake: ctx.clone(),
                    pending: 0,
                    opened: 0,
                    failed: Vec::new(),
                    shown: false,
                }
            },
            quit_confirmed: false,
            output: None,
        }
    }

    pub fn is_modified(&self) -> bool {
        self.docs.iter().any(|d| d.editor.is_modified())
    }

    /// Il titolo della finestra quando è aperto questo modulo.
    pub fn title(&self) -> Option<String> {
        let d = self.docs.get(self.current)?;
        let mark = if d.editor.is_modified() { " •" } else { "" };
        Some(format!("{}{mark} — WinFiles", d.editor.file_name()))
    }

    /// Chiudere WinFiles: se c'è qualcosa di non salvato prima si chiede.
    pub fn can_quit(&mut self) -> bool {
        if self.quit_confirmed || !self.is_modified() {
            return true;
        }
        if let Some(i) = self.docs.iter().position(|d| d.editor.is_modified()) {
            self.current = i;
        }
        self.asking = Some(Then::Quit);
        false
    }

    // --- Immagini aperte -------------------------------------------------------------------

    /// Apre le immagini in un thread a parte; arrivano con [`Self::receive`] e si mostra la
    /// prima appena è pronta.
    pub fn open(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        let l = &mut self.loading;
        if l.pending == 0 {
            l.opened = 0;
            l.failed.clear();
            l.shown = false;
        }
        l.pending += paths.len();
        self.status = match paths {
            [one] => format!("Apertura di {}…", file_name(one)),
            many => format!("Apertura di {} immagini…", many.len()),
        };
        let tx = l.tx.clone();
        let wake = l.wake.clone();
        let paths = paths.to_vec();
        let spawned = std::thread::Builder::new()
            .name("apri-immagini".into())
            .spawn(move || {
                for path in paths {
                    let result = Editor::open(&path).map_err(|e| e.to_string());
                    if tx.send((path, result)).is_err() {
                        return;
                    }
                    wake.request_repaint();
                }
            });
        if let Err(e) = spawned {
            self.loading.pending = 0;
            self.status = format!("Impossibile aprire le immagini: {e}");
        }
    }

    /// Le immagini aperte dal thread: si aggiungono all'elenco, la prima si mostra subito.
    fn receive(&mut self) {
        while let Ok((path, result)) = self.loading.rx.try_recv() {
            let l = &mut self.loading;
            l.pending = l.pending.saturating_sub(1);
            match result {
                Ok(editor) => {
                    let format = winfiles_image_edit::default_format(editor.path());
                    self.docs.push(Doc {
                        editor,
                        generation: 0,
                        target: None,
                        options: SaveOptions {
                            format,
                            ..SaveOptions::default()
                        },
                    });
                    l.opened += 1;
                    if !l.shown {
                        l.shown = true;
                        self.current = self.docs.len() - 1;
                        self.cutter.crop = None;
                        self.fields.owner = None;
                    }
                }
                Err(e) => l.failed.push(format!("{}: {e}", file_name(&path))),
            }
            let l = &self.loading;
            self.status = if l.pending > 0 {
                format!("Apertura… ne mancano {}", l.pending)
            } else if let Some(first) = l.failed.first() {
                if l.failed.len() > 1 {
                    format!("{first} (e altre {} non aperte)", l.failed.len() - 1)
                } else {
                    first.clone()
                }
            } else {
                match l.opened {
                    1 => format!("Aperta {}", self.name_of(self.current)),
                    n => format!("Aperte {n} immagini"),
                }
            };
        }
    }

    /// Dalle miniature (tasto destro › Modifica immagine): le immagini già aperte non si aprono
    /// una seconda volta, si mostra la prima.
    pub fn open_from_list(&mut self, paths: &[PathBuf]) {
        let open = |p: &PathBuf| self.docs.iter().position(|d| d.editor.path() == p.as_path());
        if let Some(i) = paths.first().and_then(open) {
            self.current = i;
            self.cutter.crop = None;
            self.fields.owner = None;
        }
        let new: Vec<PathBuf> = paths.iter().filter(|p| open(p).is_none()).cloned().collect();
        self.open(&new);
    }

    pub fn dropped(&mut self, paths: Vec<PathBuf>) {
        let images: Vec<PathBuf> = paths.into_iter().filter(|p| !p.is_dir()).collect();
        if !images.is_empty() {
            self.open(&images);
        }
    }

    fn name_of(&self, i: usize) -> String {
        self.docs.get(i).map(|d| d.editor.file_name()).unwrap_or_default()
    }

    fn close(&mut self, i: usize) {
        if i >= self.docs.len() {
            return;
        }
        self.docs.remove(i);
        self.current = self.current.min(self.docs.len().saturating_sub(1));
        self.cutter.crop = None;
        self.quotes.forget();
        self.fields.owner = None;
        self.renderer.keep(&self.live());
    }

    fn live(&self) -> Vec<(usize, u64)> {
        self.docs
            .iter()
            .enumerate()
            .map(|(i, d)| (i, d.generation))
            .collect()
    }

    // --- Modifiche -------------------------------------------------------------------------

    fn apply(&mut self, op: Op) {
        let Some(d) = self.docs.get_mut(self.current) else {
            return;
        };
        match d.editor.apply(op) {
            Ok(()) => {
                d.generation += 1;
                self.status = op.label();
                self.cutter.crop = None;
                self.fields.owner = None;
                let live = self.live();
                self.renderer.keep(&live);
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn undo(&mut self) {
        let Some(d) = self.docs.get_mut(self.current) else {
            return;
        };
        let label = d.editor.undo_label();
        match d.editor.undo() {
            Ok(()) => {
                d.generation += 1;
                self.status = label.map_or("Annullato".into(), |l| format!("Annullato: {l}"));
                self.cutter.crop = None;
                self.fields.owner = None;
                let live = self.live();
                self.renderer.keep(&live);
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn redo(&mut self) {
        let Some(d) = self.docs.get_mut(self.current) else {
            return;
        };
        let label = d.editor.redo_label();
        match d.editor.redo() {
            Ok(()) => {
                d.generation += 1;
                self.status = label.map_or("Rifatto".into(), |l| format!("Rifatto: {l}"));
                self.cutter.crop = None;
                self.fields.owner = None;
                let live = self.live();
                self.renderer.keep(&live);
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn crop_now(&mut self) {
        let Some(crop) = self.cutter.crop else { return };
        let Some(d) = self.docs.get(self.current) else {
            return;
        };
        let (w, h) = (d.editor.picture().width(), d.editor.picture().height());
        match to_px(crop, w, h) {
            Some(px) => self.apply(Op::Crop(px)),
            None => self.status = "Il ritaglio è troppo piccolo".into(),
        }
    }

    /// Salva: la prima volta chiede dove, poi riscrive lì.
    fn save(&mut self, ask: bool) {
        let Some(d) = self.docs.get(self.current) else {
            return;
        };
        let has_target = d.target.is_some() && !ask;
        let has_alpha = matches!(
            d.editor.picture().color(),
            winfiles_image_edit::ColorMode::Rgba | winfiles_image_edit::ColorMode::GrayAlpha
        );
        self.dialog = Some(Dialog::save(
            d.options,
            has_target,
            has_alpha,
            d.editor.suggested_name(d.options.format),
        ));
    }

    fn write(&mut self, options: SaveOptions, ask: bool) {
        let Some(d) = self.docs.get_mut(self.current) else {
            return;
        };
        d.options = options;
        let target = if ask || d.target.is_none() {
            let start = d.editor.path().parent().map(Path::to_path_buf);
            let name = d.editor.suggested_name(options.format);
            let mut dialog = rfd::FileDialog::new()
                .set_title("Salva l'immagine")
                .set_file_name(name)
                .add_filter(options.format.label(), &[options.format.extension()]);
            if let Some(dir) = start {
                dialog = dialog.set_directory(dir);
            }
            match dialog.save_file() {
                Some(p) => with_extension(p, options.format),
                None => return,
            }
        } else {
            match &d.target {
                Some(p) => p.clone(),
                None => return,
            }
        };
        match d.editor.save(&target, options) {
            Ok(()) => {
                self.status = format!("Salvata in {}", file_name(&target));
                if let Some(d) = self.docs.get_mut(self.current) {
                    d.target = Some(target);
                }
            }
            Err(e) => self.status = format!("Non salvata: {e}"),
        }
    }

    fn act(&mut self, action: Action) {
        match action {
            Action::Resize { width, height, dpi } => {
                self.apply(Op::Resize { width, height });
                let current = self.docs.get(self.current).map(|d| d.editor.picture().dpi());
                if current.is_some_and(|c| (c - dpi).abs() > 0.01) {
                    self.apply(Op::Dpi(dpi));
                }
            }
            Action::Save { options, ask } => self.write(options, ask),
        }
    }

    // --- Tasti e menu ----------------------------------------------------------------------

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
        } else if pressed(Modifiers::COMMAND, Key::P) {
            self.print(ctx);
        } else if pressed(Modifiers::COMMAND, Key::O) {
            let paths = pick_images();
            if !paths.is_empty() {
                self.open(&paths);
            }
        } else if pressed(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
            || pressed(Modifiers::COMMAND, Key::Y)
        {
            self.redo();
        } else if pressed(Modifiers::COMMAND, Key::Z) {
            self.undo();
        }
        if ctx.egui_wants_keyboard_input() || self.docs.is_empty() {
            return;
        }
        if pressed(Modifiers::COMMAND, Key::R) {
            self.apply(Op::Rotate(1));
        }
        // Con le quote accese Canc toglie la quota scelta ed Esc la lascia, poi spegne le quote.
        if self.quoting {
            if ctx.input(|i| i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace)) {
                self.quotes.remove_selected();
            }
            if ctx.input(|i| i.key_pressed(Key::Escape)) && self.quotes.selected.take().is_none() {
                self.quoting = false;
            }
        }
        if ctx.input(|i| i.key_pressed(Key::Enter)) && self.cutter.crop.is_some() {
            self.crop_now();
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) && self.cutter.crop.is_some() {
            self.cutter.crop = None;
        }
        let step =
            ctx.input(|i| i32::from(i.key_pressed(Key::ArrowDown)) - i32::from(i.key_pressed(Key::ArrowUp)));
        if step != 0 && !self.docs.is_empty() {
            let n = self.docs.len() as i32;
            let next = (self.current as i32 + step).clamp(0, n - 1);
            self.choose(next.max(0) as usize);
        }
    }

    fn choose(&mut self, i: usize) {
        if i != self.current {
            self.current = i;
            self.cutter.crop = None;
            self.fields.owner = None;
        }
    }

    pub fn file_menu(&mut self, ui: &mut egui::Ui) {
        let item = |ui: &mut egui::Ui, text: &str, keys: &str, enabled: bool| {
            ui.add_enabled(enabled, egui::Button::new(text).shortcut_text(keys))
                .clicked()
        };
        let open = !self.docs.is_empty();
        if item(ui, "Apri immagini…", "Ctrl+O", true) {
            ui.close();
            let paths = pick_images();
            if !paths.is_empty() {
                self.open(&paths);
            }
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
            self.print(ui.ctx());
        }
        ui.separator();
        if item(ui, "Ridimensiona…", "", open) {
            ui.close();
            self.open_resize();
        }
        if item(ui, "Chiudi l'immagine", "", open) {
            ui.close();
            let i = self.current;
            self.guard(Then::Close(i));
        }
    }

    fn open_resize(&mut self) {
        if let Some(d) = self.docs.get(self.current) {
            let p = d.editor.picture();
            self.dialog = Some(Dialog::resize((p.width(), p.height()), p.dpi(), self.unit));
        }
    }

    fn guard(&mut self, then: Then) {
        let dirty = match &then {
            Then::Close(i) => self.docs.get(*i).is_some_and(|d| d.editor.is_modified()),
            _ => self.is_modified(),
        };
        if dirty {
            self.asking = Some(then);
        } else {
            self.run(then);
        }
    }

    fn run(&mut self, then: Then) {
        match then {
            Then::Close(i) => self.close(i),
            Then::Quit => self.quit_confirmed = true,
        }
    }

    // --- Barre -----------------------------------------------------------------------------

    pub fn control_bar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE.fill(t.app_bar).inner_margin(egui::Margin {
            left: 14,
            right: 14,
            top: 2,
            bottom: 6,
        });
        egui::Panel::top("barra-immagini")
            .exact_size(46.0)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let open = !self.docs.is_empty();
                    if text_button(ui, "folder-open", "Apri", true)
                        .on_hover_text("Apri una o più immagini (Ctrl+O)")
                        .clicked()
                    {
                        let paths = pick_images();
                        if !paths.is_empty() {
                            self.open(&paths);
                        }
                    }
                    separator(ui);
                    let (undo, redo) = self
                        .docs
                        .get(self.current)
                        .map_or((None, None), |d| (d.editor.undo_label(), d.editor.redo_label()));
                    let tip = undo
                        .as_ref()
                        .map_or("Annulla (Ctrl+Z)".into(), |l| format!("Annulla: {l} (Ctrl+Z)"));
                    if icons::button(ui, "undo-2", &tip, false, undo.is_some(), 32.0).clicked() {
                        self.undo();
                    }
                    let tip = redo
                        .as_ref()
                        .map_or("Ripeti (Ctrl+Y)".into(), |l| format!("Ripeti: {l} (Ctrl+Y)"));
                    if icons::button(ui, "redo-2", &tip, false, redo.is_some(), 32.0).clicked() {
                        self.redo();
                    }
                    separator(ui);
                    if icons::button(ui, "rotate-ccw", "Ruota a sinistra", false, open, 32.0).clicked() {
                        self.apply(Op::Rotate(-1));
                    }
                    if icons::button(ui, "rotate-cw", "Ruota a destra (Ctrl+R)", false, open, 32.0).clicked()
                    {
                        self.apply(Op::Rotate(1));
                    }
                    if icons::button(
                        ui,
                        "flip-vertical-2",
                        "Specchia in orizzontale",
                        false,
                        open,
                        32.0,
                    )
                    .clicked()
                    {
                        self.apply(Op::Flip(Axis::Horizontal));
                    }
                    if icons::button(
                        ui,
                        "flip-horizontal-2",
                        "Specchia in verticale",
                        false,
                        open,
                        32.0,
                    )
                    .clicked()
                    {
                        self.apply(Op::Flip(Axis::Vertical));
                    }
                    separator(ui);
                    let can_crop = self.cutter.crop.is_some();
                    if text_button(ui, "crop", "Ritaglia", can_crop)
                        .on_hover_text("Taglia l'immagine sul rettangolo scelto (Invio)")
                        .clicked()
                    {
                        self.crop_now();
                    }
                    if ui
                        .add_enabled(can_crop, egui::Button::new("Azzera").min_size(vec2(0.0, 32.0)))
                        .on_hover_text("Toglie il rettangolo di ritaglio (Esc)")
                        .clicked()
                    {
                        self.cutter.crop = None;
                    }
                    separator(ui);
                    if text_button(ui, "scaling", "Ridimensiona", open)
                        .on_hover_text("Rifà i pixel alla misura voluta")
                        .clicked()
                    {
                        self.open_resize();
                    }
                    separator(ui);
                    if text_toggle(ui, "ruler", "Quote", open, self.quoting)
                        .on_hover_text(
                            "Misure in proporzione su un rettangolo o una linea: scrivi una misura e \
                             le altre si adeguano (l'immagine non cambia)",
                        )
                        .clicked()
                    {
                        self.quoting = !self.quoting;
                        if self.quoting {
                            self.cutter.crop = None;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        let (fill, ink) = if open {
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
                        let tip = match self.docs.get(self.current).and_then(|d| d.target.as_ref()) {
                            Some(p) => format!("Salva in {} (Ctrl+S)", file_name(p)),
                            None => "Salva l'immagine con un nome nuovo (Ctrl+S)".into(),
                        };
                        if ui.add_enabled(open, save).on_hover_text(tip).clicked() {
                            self.save(false);
                        }
                        let tip = if self.quoting {
                            "Stampa l'immagine alla sua misura, con le quote (Ctrl+P)"
                        } else {
                            "Stampa l'immagine alla sua misura di stampa (Ctrl+P)"
                        };
                        if text_button(ui, "printer", "Stampa", open)
                            .on_hover_text(tip)
                            .clicked()
                        {
                            self.print(ui.ctx());
                        }
                    });
                });
            });
    }

    pub fn status_bar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let frame = egui::Frame::NONE
            .fill(t.app_bar)
            .inner_margin(egui::Margin::symmetric(16, 0));
        egui::Panel::bottom("barra-stato-immagini")
            .exact_size(32.0)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(d) = self.docs.get(self.current) {
                        let p = d.editor.picture();
                        let text = format!(
                            "{}  ·  {} × {} pixel  ·  {}",
                            d.editor.file_name(),
                            format_number(f64::from(p.width()), 0, false),
                            format_number(f64::from(p.height()), 0, false),
                            format_size(p.size_mm(), self.unit),
                        );
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

    // --- Finestre --------------------------------------------------------------------------

    pub fn windows(&mut self, ctx: &egui::Context) {
        self.unsaved_dialog(ctx);
        if let Some(mut d) = self.dialog.take() {
            match d.show(ctx) {
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
        let name = self.name_of(self.current);
        let mut answer = None;
        let modal = egui::Modal::new(egui::Id::new("img-salvare")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(
                egui::RichText::new(format!("Salvare le modifiche a «{name}»?"))
                    .font(semibold(15.0))
                    .color(t.text_strong),
            );
            ui.add_space(6.0);
            ui.label("Se non salvi, le modifiche all'immagine andranno perse.");
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
                self.save(false);
            }
            Some(false) => {
                self.asking = None;
                if let Some(d) = self.docs.get_mut(self.current) {
                    d.editor.reset();
                    d.generation += 1;
                }
                self.run(then);
            }
            None => {}
        }
    }

    // --- Corpo -----------------------------------------------------------------------------

    pub fn body(&mut self, ui: &mut egui::Ui, unit: Unit) {
        self.unit = unit;
        self.receive();
        self.renderer.receive(ui.ctx());
        let full = ui.max_rect();
        if self.docs.is_empty() {
            self.empty(ui, full);
            return;
        }
        let list_w = (full.width() * 0.18).clamp(150.0, 240.0);
        let data_w = (full.width() * 0.26).clamp(240.0, 340.0);
        let list = Rect::from_min_size(full.min, vec2(list_w, full.height()));
        let data = Rect::from_min_max(pos2(full.right() - data_w, full.top()), full.max);
        let view = Rect::from_min_max(
            pos2(list.right() + 10.0, full.top()),
            pos2(data.left() - 10.0, full.bottom()),
        );
        self.file_list(ui, list);
        self.preview(ui, view);
        self.data_panel(ui, data);
    }

    fn empty(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        let c = rect.center();
        icons::paint(
            ui,
            "image",
            Rect::from_center_size(c - vec2(0.0, 64.0), Vec2::splat(48.0)),
            t.text_dim,
        );
        if self.loading.pending > 0 {
            let spinner = Rect::from_center_size(c + vec2(0.0, 6.0), Vec2::splat(28.0));
            ui.put(spinner, egui::Spinner::new().size(28.0).color(t.primary));
            ui.painter().text(
                c + vec2(0.0, 44.0),
                egui::Align2::CENTER_CENTER,
                &self.status,
                egui::FontId::proportional(13.5),
                t.text_dim,
            );
            return;
        }
        let p = ui.painter();
        p.text(
            c - vec2(0.0, 16.0),
            egui::Align2::CENTER_CENTER,
            "Trascina qui le immagini da controllare",
            semibold(17.0),
            t.text_strong,
        );
        p.text(
            c + vec2(0.0, 10.0),
            egui::Align2::CENTER_CENTER,
            "JPEG, PNG, TIFF, HEIC di iPhone, foto RAW e file di Photoshop",
            egui::FontId::proportional(13.5),
            t.text_dim,
        );
        let button = Rect::from_center_size(c + vec2(0.0, 52.0), vec2(170.0, 34.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(button));
        let open = egui::Button::new(
            egui::RichText::new("Apri immagini…")
                .font(semibold(13.5))
                .color(t.on_primary),
        )
        .fill(t.primary)
        .min_size(button.size());
        if child.add(open).clicked() {
            let paths = pick_images();
            if !paths.is_empty() {
                self.open(&paths);
            }
        }
    }

    fn file_list(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 40.0)).shrink2(vec2(14.0, 0.0));
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        bar.label(
            egui::RichText::new("Immagini")
                .font(semibold(14.0))
                .color(t.text_strong),
        );
        bar.label(egui::RichText::new(self.docs.len().to_string()).color(t.text_dim));
        let body = Rect::from_min_max(
            pos2(rect.left() + 6.0, header.bottom()),
            rect.max - vec2(6.0, 8.0),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
        let mut close = None;
        let mut choose = None;
        egui::ScrollArea::vertical()
            .id_salt("elenco-immagini")
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                for i in 0..self.docs.len() {
                    let (cell, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), CELL_H), Sense::click());
                    if !ui.is_rect_visible(cell) {
                        continue;
                    }
                    let chosen = i == self.current;
                    let fill = if chosen {
                        t.selection
                    } else if resp.hovered() {
                        t.hover
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(cell.shrink(2.0), t.radius + 2, fill);
                    let thumb_rect = Rect::from_min_size(cell.min + vec2(8.0, 8.0), Vec2::splat(60.0));
                    let (version, size) = self
                        .docs
                        .get(i)
                        .map(|d| {
                            let p = d.editor.picture();
                            (
                                d.generation,
                                Size::new(f64::from(p.width()), f64::from(p.height())),
                            )
                        })
                        .unwrap_or((0, Size::new(1.0, 1.0)));
                    if let Some(tex) = self.renderer.get((i, version, THUMB_SIDE)) {
                        let at = fit_rect(thumb_rect, size);
                        ui.painter().rect_filled(at, 2.0, egui::Color32::WHITE);
                        ui.painter().image(
                            tex.id(),
                            at,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    } else {
                        ui.painter().rect_filled(thumb_rect, 2.0, t.pasteboard);
                    }
                    let text_rect = Rect::from_min_max(
                        pos2(thumb_rect.right() + 8.0, cell.top() + 8.0),
                        cell.max - vec2(8.0, 6.0),
                    );
                    let name = self.name_of(i);
                    let modified = self.docs.get(i).is_some_and(|d| d.editor.is_modified());
                    let label = if modified { format!("{name} •") } else { name };
                    let galley = ui.painter().layout(
                        label,
                        semibold(12.5),
                        if chosen { t.text_strong } else { t.text },
                        text_rect.width(),
                    );
                    ui.painter().galley(text_rect.min, galley, t.text);
                    if resp.clicked() {
                        choose = Some(i);
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Chiudi l'immagine").clicked() {
                            close = Some(i);
                            ui.close();
                        }
                    });
                }
            });
        if let Some(i) = choose {
            self.choose(i);
        }
        if let Some(i) = close {
            self.guard(Then::Close(i));
        }
        // Le miniature che mancano si chiedono al thread.
        let wanted: Vec<(usize, u64)> = self.live();
        for (i, version) in wanted {
            let key = (i, version, THUMB_SIDE);
            if let Some(d) = self.docs.get(i) {
                let shared = d.editor.picture().shared();
                self.renderer.request(key, || shared);
            }
        }
    }

    fn data_panel(&mut self, ui: &mut egui::Ui, rect: Rect) {
        card(ui, rect);
        if self.docs.get(self.current).is_none() {
            return;
        }
        if self.quoting {
            self.quotes_panel(ui, rect);
            return;
        }
        let body = rect.shrink2(vec2(14.0, 12.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(body)
                .layout(egui::Layout::top_down(Align::Min)),
        );
        // Su una finestra bassa i dati non ci stanno tutti: si scorrono.
        egui::ScrollArea::vertical()
            .id_salt("dati-immagine")
            .auto_shrink([false, false])
            .show(&mut child, |ui| self.data_contents(ui));
    }

    /// Al posto dei dati, mentre le quote sono accese: il loro pannello.
    fn quotes_panel(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let t = Tokens::get(ui.ctx());
        let mut ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(vec2(14.0, 12.0)))
                .layout(egui::Layout::top_down(Align::Min)),
        );
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Quote")
                    .font(semibold(14.0))
                    .color(t.text_strong),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if icons::button(
                    ui,
                    "x",
                    "Spegni le quote e torna ai dati (Esc)",
                    false,
                    true,
                    24.0,
                )
                .clicked()
                {
                    self.quoting = false;
                }
            });
        });
        ui.add_space(6.0);
        let unit = self.unit;
        let action = egui::ScrollArea::vertical()
            .id_salt("quote-immagine")
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

    /// L'immagine com'è adesso in un file temporaneo, con i pixel che bastano alla stampa, e la
    /// sua pagina alla misura di stampa, con le quote se sono accese.
    fn output_page(&mut self) -> Option<(Size, winfiles_scene::Source)> {
        let d = self.docs.get(self.current)?;
        let p = d.editor.picture();
        let size = p.size_mm();
        let image = p.image();
        let (w, h) = (image.width(), image.height());
        // Più pixel di quelli che servono a 300 DPI (e oltre il tetto del PDF) non escono.
        let need = (size.w.max(size.h) / 25.4 * crate::quotes::PRINT_DPI).ceil();
        let long = f64::from(w.max(h));
        let image = if need.is_finite() && need >= 64.0 && long > need.min(6000.0) {
            let k = need.min(6000.0) / long;
            let nw = ((f64::from(w) * k).round() as u32).max(1);
            let nh = ((f64::from(h) * k).round() as u32).max(1);
            image.resize_exact(nw, nh, image::imageops::FilterType::Triangle)
        } else {
            image.clone()
        };
        let path = crate::quotes::temp_source("png");
        let written = std::fs::File::create(&path)
            .map_err(|e| e.to_string())
            .and_then(|f| {
                let encoder = image::codecs::png::PngEncoder::new_with_quality(
                    std::io::BufWriter::new(f),
                    image::codecs::png::CompressionType::Fast,
                    image::codecs::png::FilterType::Adaptive,
                );
                image::DynamicImage::ImageRgba8(image.to_rgba8())
                    .write_with_encoder(encoder)
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = written {
            self.status = format!("Non riesco a preparare la stampa: {e}");
            return None;
        }
        let source = winfiles_scene::Source {
            path,
            kind: winfiles_model::FileKind::Image,
            page: 0,
        };
        Some((size, source))
    }

    /// La pagina dell'immagine corrente, con le sue quote se sono accese.
    fn quoted_page(&mut self, ctx: &egui::Context) -> Option<winfiles_scene::Page> {
        let (size, source) = self.output_page()?;
        let owner = self
            .docs
            .get(self.current)
            .map(|d| (self.current as u64).wrapping_shl(40) ^ d.generation);
        let quoted = self.quoting && self.quotes.owner().is_some() && self.quotes.owner() == owner;
        let quotes = quoted.then_some((&self.quotes, self.unit));
        Some(crate::quotes::page(ctx, size, source, quotes))
    }

    /// Stampa l'immagine com'è adesso alla sua misura di stampa, con le quote se accese.
    pub fn print(&mut self, ctx: &egui::Context) {
        if self.output.is_some() {
            return;
        }
        let Some(page) = self.quoted_page(ctx) else {
            return;
        };
        let title = self
            .docs
            .get(self.current)
            .map_or_else(|| "Immagine".into(), |d| file_name(d.editor.path()));
        self.output = Some(crate::quotes::Output::Print {
            pages: vec![page],
            title,
            current: 0,
        });
    }

    /// Salva in un PDF l'immagine con le quote disegnate sopra, alla misura di stampa.
    fn save_quoted(&mut self, ctx: &egui::Context) {
        let Some(d) = self.docs.get(self.current) else {
            return;
        };
        let source = d.editor.path();
        let stem = source
            .file_stem()
            .map_or_else(|| "immagine".into(), |s| s.to_string_lossy().into_owned());
        let mut dialog = rfd::FileDialog::new()
            .set_title("Salva l'immagine con le quote")
            .set_file_name(format!("{stem}_quote.pdf"))
            .add_filter("PDF", &["pdf"]);
        if let Some(dir) = source.parent() {
            dialog = dialog.set_directory(dir);
        }
        let Some(mut path) = dialog.save_file() else {
            return;
        };
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            path.as_mut_os_string().push(".pdf");
        }
        if let Some(page) = self.quoted_page(ctx) {
            self.output = Some(crate::quotes::Output::Save {
                pages: vec![page],
                path,
            });
        }
    }

    fn data_contents(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let Some(d) = self.docs.get(self.current) else {
            return;
        };
        let p = d.editor.picture();
        let (w, h) = (p.width(), p.height());
        let dpi = p.dpi();
        let unit = self.unit;
        let version = d.generation;
        let icc = p.icc().map(|i| (i.len(), p.icc_name()));
        let rows = [
            (
                "Pixel",
                format!(
                    "{} × {}",
                    format_number(f64::from(w), 0, false),
                    format_number(f64::from(h), 0, false)
                ),
            ),
            ("Colore", format!("{} · {} bit", p.color().label(), p.bits())),
            (
                "Profilo",
                match icc {
                    Some((_, Some(name))) => name.to_string(),
                    Some((bytes, None)) => format!("incorporato ({})", bytes_text(bytes as u64)),
                    None => "nessuno".into(),
                },
            ),
            ("Rapporto", p.ratio()),
            ("Peso", bytes_text(d.editor.bytes_on_disk())),
        ];
        ui.label(
            egui::RichText::new("Dati")
                .font(semibold(14.0))
                .color(t.text_strong),
        );
        ui.add_space(6.0);
        egui::Grid::new("dati-tecnici")
            .num_columns(2)
            .spacing(vec2(10.0, 4.0))
            .show(ui, |ui| {
                for (name, value) in rows {
                    ui.label(egui::RichText::new(name).color(t.text_dim));
                    ui.add(egui::Label::new(egui::RichText::new(value).color(t.text)).truncate());
                    ui.end_row();
                }
            });
        // Foto RAW e file di Photoshop: da dove vengono i pixel che si vedono.
        if let Some(origin) = d.editor.original().origin() {
            ui.add_space(4.0);
            ui.add(egui::Label::new(egui::RichText::new(origin).size(12.0).color(t.text_dim)).wrap());
        }
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("Misura di stampa")
                .font(semibold(14.0))
                .color(t.text_strong),
        );
        ui.label(
            egui::RichText::new("I pixel non cambiano: cambia quanto misura sulla carta.")
                .size(12.0)
                .color(t.text_dim),
        );
        ui.add_space(6.0);
        // I campi si riempiono quando cambia immagine o versione; poi li tiene chi scrive.
        if self.fields.owner != Some((self.current, version)) {
            self.fields.width = dialogs::number(f64::from(w) * 25.4 / dpi, unit);
            self.fields.height = dialogs::number(f64::from(h) * 25.4 / dpi, unit);
            self.fields.dpi = format_number(dpi, 0, true);
            self.fields.owner = Some((self.current, version));
        }
        egui::Grid::new("misura-stampa")
            .num_columns(2)
            .spacing(vec2(10.0, 8.0))
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Larghezza").color(t.text_dim));
                let rw = field(ui, &mut self.fields.width, unit.symbol());
                ui.end_row();
                ui.label(egui::RichText::new("Altezza").color(t.text_dim));
                let rh = field(ui, &mut self.fields.height, unit.symbol());
                ui.end_row();
                ui.label(egui::RichText::new("Risoluzione").color(t.text_dim));
                let rd = field(ui, &mut self.fields.dpi, "DPI");
                ui.end_row();
                // Le tre caselle sono legate: scrivendone una le altre si rifanno, perché i
                // pixel restano quelli.
                if let (true, Some(mm)) = (
                    rw.changed(),
                    dialogs::parse(&self.fields.width).map(|v| v * unit.mm()),
                ) {
                    {
                        let new = f64::from(w) * 25.4 / mm;
                        self.fields.height = dialogs::number(f64::from(h) * 25.4 / new, unit);
                        self.fields.dpi = format_number(new, 0, true);
                    }
                } else if let (true, Some(mm)) = (
                    rh.changed(),
                    dialogs::parse(&self.fields.height).map(|v| v * unit.mm()),
                ) {
                    {
                        let new = f64::from(h) * 25.4 / mm;
                        self.fields.width = dialogs::number(f64::from(w) * 25.4 / new, unit);
                        self.fields.dpi = format_number(new, 0, true);
                    }
                } else if let (true, Some(new)) = (rd.changed(), dialogs::parse(&self.fields.dpi)) {
                    {
                        self.fields.width = dialogs::number(f64::from(w) * 25.4 / new, unit);
                        self.fields.height = dialogs::number(f64::from(h) * 25.4 / new, unit);
                    }
                }
            });
        let typed = dialogs::parse(&self.fields.dpi).filter(|v| (1.0..=10_000.0).contains(v));
        let changed = typed.is_some_and(|v| (v - dpi).abs() > 0.01);
        ui.add_space(8.0);
        let apply = egui::Button::new(
            egui::RichText::new("Applica la misura")
                .font(semibold(13.0))
                .color(if changed { t.on_primary } else { t.text_disabled }),
        )
        .fill(if changed { t.primary } else { t.button })
        .min_size(vec2(ui.available_width().min(300.0), 30.0));
        let pressed = ui.add_enabled(changed, apply).clicked();
        ui.add_space(10.0);
        // Quello che si otterrebbe con la misura scritta, prima ancora di applicarla.
        let shown_dpi = typed.unwrap_or(dpi);
        let shown = Size::new(f64::from(w) * 25.4 / shown_dpi, f64::from(h) * 25.4 / shown_dpi);
        egui::Grid::new("misure-calcolate")
            .num_columns(2)
            .spacing(vec2(10.0, 4.0))
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Misura").color(t.text_dim));
                ui.label(format_size(shown, unit));
                ui.end_row();
                ui.label(egui::RichText::new("Area").color(t.text_dim));
                ui.label(format_area(winfiles_geom::area_m2(shown)));
                ui.end_row();
                ui.label(egui::RichText::new("Visione").color(t.text_dim));
                ui.label(format!("da ~{} m", format_number(80.0 / shown_dpi, 2, true)))
                    .on_hover_text(
                        "La distanza da cui l'immagine si vede nitida: la regola dei grandi formati.",
                    );
                ui.end_row();
            });
        // Il ritaglio, quando c'è.
        let mut cut = false;
        if let Some(crop) = self.cutter.crop.map(Crop::normalized) {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("Ritaglio")
                    .font(semibold(14.0))
                    .color(t.text_strong),
            );
            ui.add_space(4.0);
            let size = Size::new(
                f64::from(crop.width()) * 25.4 / dpi,
                f64::from(crop.height()) * 25.4 / dpi,
            );
            ui.label(
                egui::RichText::new(format!(
                    "{} × {} pixel",
                    format_number(f64::from(crop.width().round()), 0, false),
                    format_number(f64::from(crop.height().round()), 0, false)
                ))
                .color(t.text),
            );
            ui.label(egui::RichText::new(format_size(size, unit)).color(t.text_dim));
            ui.add_space(6.0);
            let button = egui::Button::new(
                egui::RichText::new("Ritaglia")
                    .font(semibold(13.0))
                    .color(t.on_primary),
            )
            .fill(t.primary)
            .min_size(vec2(ui.available_width().min(300.0), 30.0));
            cut = ui.add(button).clicked();
        }
        if let (true, Some(v)) = (pressed, typed) {
            self.apply(Op::Dpi(v));
        }
        if cut {
            self.crop_now();
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let t = Tokens::get(ui.ctx());
        card(ui, rect);
        let Some(d) = self.docs.get(self.current) else {
            return;
        };
        let p = d.editor.picture();
        let (iw, ih) = (p.width().max(1), p.height().max(1));
        let generation = d.generation;
        let unit = self.unit;
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 44.0)).shrink2(vec2(14.0, 0.0));
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        // Prima il menu dello zoom a destra, poi nome e misura nello spazio che resta: un nome
        // lungo si accorcia con "…" invece di finire sotto il menu.
        let name = d.editor.file_name();
        let size = format_size(p.size_mm(), unit);
        let hint = self.cutter.crop.is_none() && !self.quoting;
        bar.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            egui::ComboBox::from_id_salt("img-zoom")
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
                    for z in [25.0, 50.0, 100.0, 200.0, 400.0] {
                        if ui
                            .selectable_label(self.zoom == Some(z / 100.0), format!("{z:.0}%"))
                            .clicked()
                        {
                            self.zoom = Some(z / 100.0);
                        }
                    }
                });
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(name)
                            .font(semibold(14.0))
                            .color(t.text_strong),
                    )
                    .truncate(),
                );
                ui.add(egui::Label::new(egui::RichText::new(size).color(t.text_dim)).truncate());
                if hint && ui.available_width() > 160.0 {
                    ui.label(
                        egui::RichText::new("·  trascina per ritagliare")
                            .size(12.5)
                            .color(t.text_dim),
                    );
                }
            });
        });

        let board = Rect::from_min_max(
            pos2(rect.left() + 6.0, header.bottom()),
            rect.max - vec2(6.0, 6.0),
        );
        ui.painter().rect_filled(board, t.radius, t.pasteboard);
        let natural = vec2(iw as f32, ih as f32);
        // Si adatta alla finestra, ma senza ingrandire oltre i pixel veri.
        let room = ((board.width() - 40.0) / natural.x).min((board.height() - 40.0) / natural.y);
        let fit = room.clamp(0.01, 1.0);
        if let Some(z) = crate::app::wheel_zoom(ui, board) {
            let base = self.zoom.unwrap_or(fit);
            self.zoom = Some((base * z).clamp(0.02, 8.0));
        }
        let scale = self.zoom.unwrap_or(fit);
        self.shown_zoom = scale;
        let shown = natural * scale;
        let paper = Rect::from_center_size(board.center(), shown);
        // L'immagine si chiede al lato che serve davvero, per scatti, così non si rifà a ogni
        // movimento del cursore.
        let ppp = ui.ctx().pixels_per_point();
        let needed = (shown.x.max(shown.y) * ppp).ceil() as u32;
        let side = [THUMB_SIDE, 768, 1536, 3072, 4096]
            .into_iter()
            .find(|s| *s >= needed)
            .unwrap_or(4096);
        let shared = p.shared();
        self.renderer.request((self.current, generation, side), || shared);
        let tex = self
            .renderer
            .get((self.current, generation, side))
            .or_else(|| self.renderer.nearest(self.current, generation));
        ui.painter().rect_filled(
            paper.translate(vec2(0.0, 2.0)).expand(1.0),
            2.0,
            egui::Color32::from_black_alpha(40),
        );
        ui.painter().rect_filled(paper, 0.0, egui::Color32::WHITE);
        if let Some(tex) = tex {
            ui.painter().image(
                tex.id(),
                paper,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(80));
        }
        let lens = if self.quoting {
            // Le quote in pixel: un pixel vale 25,4 / DPI millimetri.
            let owner = (self.current as u64).wrapping_shl(40) ^ generation;
            self.quotes.prepare(owner, 25.4 / p.dpi());
            self.quotes
                .interact(ui, ui.id().with("quote"), paper, iw as f32, ih as f32);
            self.quotes.paint(ui, paper, iw as f32, unit);
            self.quotes.lens
        } else {
            self.cutter
                .interact(ui, ui.id().with("ritaglio"), paper, iw as f32, ih as f32);
            self.cutter.paint(ui, paper, iw as f32);
            self.cutter.lens
        };
        self.paint_lens(ui, board, paper, iw, ih, lens);
    }

    /// La lente: mentre si tira un lato mostra i pixel veri attorno al cursore, come la 4.9.
    fn paint_lens(
        &mut self,
        ui: &mut egui::Ui,
        board: Rect,
        paper: Rect,
        iw: u32,
        ih: u32,
        at: Option<egui::Pos2>,
    ) {
        let Some(at) = at else { return };
        let Some(d) = self.docs.get(self.current) else {
            return;
        };
        let scale = paper.width() / iw.max(1) as f32;
        let px = ((at.x - paper.left()) / scale).round();
        let py = ((at.y - paper.top()) / scale).round();
        // Il quadratino di pixel veri attorno al cursore.
        let span = (LENS_SIDE / LENS_ZOOM).round().max(8.0) as u32;
        let x = (px as i64 - i64::from(span) / 2).clamp(0, i64::from(iw.saturating_sub(span)));
        let y = (py as i64 - i64::from(span) / 2).clamp(0, i64::from(ih.saturating_sub(span)));
        let w = span.min(iw);
        let h = span.min(ih);
        let piece = d
            .editor
            .picture()
            .image()
            .crop_imm(x.max(0) as u32, y.max(0) as u32, w, h)
            .to_rgba8();
        let img = egui::ColorImage::from_rgba_unmultiplied(
            [piece.width() as usize, piece.height() as usize],
            piece.as_raw(),
        );
        let tex = ui.ctx().load_texture("lente", img, egui::TextureOptions::NEAREST);
        // La lente sta vicino al cursore ma dentro la finestra.
        let mut rect = Rect::from_min_size(at + vec2(18.0, 18.0), Vec2::splat(LENS_SIDE));
        if rect.right() > board.right() {
            rect = rect.translate(vec2(-LENS_SIDE - 36.0, 0.0));
        }
        if rect.bottom() > board.bottom() {
            rect = rect.translate(vec2(0.0, -LENS_SIDE - 36.0));
        }
        let t = Tokens::get(ui.ctx());
        let p = ui.painter();
        p.rect_filled(rect.expand(2.0), 4.0, t.panel);
        p.image(
            tex.id(),
            rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        p.rect_stroke(rect, 4.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
        let c = rect.center();
        let cross = Stroke::new(1.0, t.accent);
        p.line_segment([pos2(c.x, rect.top()), pos2(c.x, rect.bottom())], cross);
        p.line_segment([pos2(rect.left(), c.y), pos2(rect.right(), c.y)], cross);
    }
}

/// Il ritaglio in pixel interi, dentro un'immagine di `w` × `h`.
fn to_px(c: Crop, w: u32, h: u32) -> Option<CropPx> {
    let c = c.normalized();
    let x = c.x0.round().clamp(0.0, w as f32) as u32;
    let y = c.y0.round().clamp(0.0, h as f32) as u32;
    let right = c.x1.round().clamp(0.0, w as f32) as u32;
    let bottom = c.y1.round().clamp(0.0, h as f32) as u32;
    (right > x && bottom > y).then_some(CropPx {
        x,
        y,
        width: right - x,
        height: bottom - y,
    })
}

fn fit_rect(bounds: Rect, size: Size) -> Rect {
    let (w, h) = (size.w.max(0.1) as f32, size.h.max(0.1) as f32);
    let s = (bounds.width() / w).min(bounds.height() / h);
    Rect::from_center_size(bounds.center(), vec2(w * s, h * s))
}

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

fn field(ui: &mut egui::Ui, value: &mut String, suffix: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let mut resp = None;
    ui.horizontal(|ui| {
        resp = Some(crate::fluent::text_field(
            ui,
            egui::TextEdit::singleline(value).desired_width(76.0),
        ));
        ui.label(egui::RichText::new(suffix).color(t.text_dim));
    });
    resp.unwrap_or_else(|| ui.label(""))
}

/// I formati che si aprono, per la finestra di sistema (in minuscolo e in maiuscolo: su Linux il
/// filtro distingue).
fn pick_images() -> Vec<PathBuf> {
    let mut extensions: Vec<String> = Vec::new();
    for e in winfiles_image_edit::extensions() {
        extensions.push(e.to_string());
        extensions.push(e.to_ascii_uppercase());
    }
    rfd::FileDialog::new()
        .set_title("Apri immagini")
        .add_filter("Immagini, foto RAW e Photoshop", &extensions)
        .pick_files()
        .unwrap_or_default()
}

fn with_extension(path: PathBuf, format: Format) -> PathBuf {
    match Format::from_extension(&path.to_string_lossy()) {
        Some(f) if f == format => path,
        _ => path.with_extension(format.extension()),
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Il peso di un file detto in kB o MB.
fn bytes_text(bytes: u64) -> String {
    let b = bytes as f64;
    if b < 1024.0 {
        format!("{bytes} byte")
    } else if b < 1024.0 * 1024.0 {
        format!("{} kB", format_number(b / 1024.0, 0, true))
    } else {
        format!("{} MB", format_number(b / (1024.0 * 1024.0), 1, true))
    }
}
