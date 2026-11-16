//! I pulsanti «Apri in …» con i programmi esterni (ADR 0037): Acrobat, Illustrator, Photoshop,
//! Foto di Windows… scelti per tipo di file nella finestra Programmi esterni.
//!
//! Lo stato è uno solo per tutta l'app (le schede, i menu del tasto destro e la finestra lo
//! leggono da qui): le scelte (salvate in `impostazioni.json`), i programmi trovati sul PC, le
//! icone. La ricerca dei programmi, le icone e gli avvii vanno in sottofondo: Windows può
//! metterci un attimo e la finestra non deve fermarsi.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, MutexGuard, OnceLock};

use egui::{Align, Rect, Sense, Stroke, Vec2, vec2};
use winfiles_external::{self as ext, Config, Found, Program, TYPES};

use crate::icons;
use crate::theme::{Tokens, semibold};

/// Più file di così in una volta si aprono solo dopo una conferma.
const MANY_FILES: usize = 10;
/// I file aperti in un programma esterno che si tengono d'occhio al massimo, e ogni quanti
/// secondi.
const MAX_WATCHED: usize = 64;
const WATCH_EVERY: f64 = 2.0;

enum Msg {
    Found(Vec<Found>),
    Icon(String, Option<ext::Icon>),
    Done(String),
}

struct State {
    config: Config,
    found: Vec<Found>,
    detecting: bool,
    /// La ricerca è finita almeno una volta.
    detected: bool,
    /// Le icone per chiave del programma: `None` = chiesta o non disponibile.
    icons: HashMap<String, Option<egui::TextureHandle>>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// I messaggi per la barra di stato della scheda in uso.
    messages: Vec<String>,
    window: Option<Settings>,
    /// I file da aprire che aspettano la conferma (troppi file).
    confirm: Option<(String, Vec<PathBuf>)>,
    /// I file aperti in un programma esterno, con la data dell'ultima modifica: quando la data
    /// cambia (salvato in Illustrator, Photoshop…) WinFiles li rilegge.
    watched: Vec<Watched>,
    last_watch: f64,
    changed: Vec<PathBuf>,
    ctx: Option<egui::Context>,
}

fn state() -> MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    let m = STATE.get_or_init(|| {
        let (tx, rx) = channel();
        Mutex::new(State {
            config: Config::default(),
            found: Vec::new(),
            detecting: false,
            detected: false,
            icons: HashMap::new(),
            tx,
            rx,
            messages: Vec::new(),
            window: None,
            confirm: None,
            watched: Vec::new(),
            last_watch: 0.0,
            changed: Vec::new(),
            ctx: None,
        })
    });
    // Un errore in un altro thread non deve bloccare i pulsanti: lo stato resta usabile.
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// All'avvio: le scelte salvate, poi la ricerca dei programmi in sottofondo.
pub fn init(ctx: &egui::Context, config: Config) {
    {
        let mut s = state();
        s.config = config.sanitized();
        s.ctx = Some(ctx.clone());
    }
    detect();
}

/// Le scelte di adesso, da salvare.
pub fn config() -> Config {
    state().config.clone()
}

/// Cerca di nuovo i programmi installati.
pub fn detect() {
    let mut s = state();
    if s.detecting {
        return;
    }
    s.detecting = true;
    let tx = s.tx.clone();
    let wake = s.ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("programmi-esterni".into())
        .spawn(move || {
            let _ = tx.send(Msg::Found(ext::detect()));
            if let Some(c) = wake {
                c.request_repaint();
            }
        });
    if spawned.is_err() {
        s.detecting = false;
    }
}

/// Ogni fotogramma: i risultati arrivati dai thread, la finestra e la domanda dei tanti file.
/// Restituisce il messaggio per la barra di stato, se ce n'è uno.
pub fn poll(ctx: &egui::Context) -> Option<String> {
    let mut s = state();
    while let Ok(m) = s.rx.try_recv() {
        match m {
            Msg::Found(found) => {
                s.detecting = false;
                s.detected = true;
                s.config.refresh(&found);
                s.found = found;
                // Le icone si rileggono: un programma aggiornato può averne una nuova.
                s.icons.clear();
            }
            Msg::Icon(key, icon) => {
                let tex = icon.and_then(|i| {
                    let size = [usize::try_from(i.width).ok()?, usize::try_from(i.height).ok()?];
                    let len = size[0].checked_mul(size[1])?.checked_mul(4)?;
                    (len == i.rgba.len()).then(|| {
                        ctx.load_texture(
                            format!("programma-{key}"),
                            egui::ColorImage::from_rgba_premultiplied(size, &i.rgba),
                            egui::TextureOptions::LINEAR,
                        )
                    })
                });
                s.icons.insert(key, tex);
            }
            Msg::Done(text) => s.messages.push(text),
        }
    }
    let msg = s.messages.pop();
    s.messages.clear();
    let now = ctx.input(|i| i.time);
    if !s.watched.is_empty() && now - s.last_watch >= WATCH_EVERY {
        s.last_watch = now;
        let mut changed = Vec::new();
        for w in &mut s.watched {
            let current = modified(&w.path);
            if current.is_none() || current == w.stamp {
                w.seen = None;
            } else if current == w.seen {
                // Ferma da un giro: il programma ha finito di scrivere.
                w.stamp = current;
                w.seen = None;
                changed.push(w.path.clone());
            } else {
                w.seen = current;
            }
        }
        s.changed.extend(changed);
    }
    if !s.watched.is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(WATCH_EVERY));
    }
    drop(s);
    show_confirm(ctx);
    show_settings(ctx);
    msg
}

/// Un file aperto in un programma esterno.
struct Watched {
    path: PathBuf,
    /// La data di modifica conosciuta.
    stamp: Option<std::time::SystemTime>,
    /// Una data nuova vista al giro prima: diventa buona se al giro dopo è la stessa (il file
    /// non si rilegge mentre l'altro programma lo sta ancora scrivendo).
    seen: Option<std::time::SystemTime>,
}

fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// I file aperti in un programma esterno e poi cambiati (salvati là): da rileggere.
pub fn take_changed() -> Vec<PathBuf> {
    std::mem::take(&mut state().changed)
}

/// Da qui in poi si tiene d'occhio il file: se cambia, WinFiles lo rilegge.
fn watch(s: &mut State, paths: &[PathBuf]) {
    for p in paths {
        let stamp = modified(p);
        match s.watched.iter_mut().find(|w| w.path == *p) {
            Some(w) => {
                w.stamp = stamp;
                w.seen = None;
            }
            None => s.watched.push(Watched {
                path: p.clone(),
                stamp,
                seen: None,
            }),
        }
    }
    let excess = s.watched.len().saturating_sub(MAX_WATCHED);
    s.watched.drain(..excess);
}

/// L'icona del programma (chiesta in sottofondo la prima volta).
fn icon(key: &str) -> Option<egui::TextureHandle> {
    let mut s = state();
    if let Some(t) = s.icons.get(key) {
        return t.clone();
    }
    s.icons.insert(key.to_string(), None);
    let program = s
        .config
        .program(key)
        .cloned()
        .or_else(|| s.found.iter().find(|f| f.key == key).map(Program::from_found))?;
    let tx = s.tx.clone();
    let wake = s.ctx.clone();
    let key = key.to_string();
    let _ = std::thread::Builder::new()
        .name("icona-programma".into())
        .spawn(move || {
            let _ = tx.send(Msg::Icon(key, ext::icon(&program)));
            if let Some(c) = wake {
                c.request_repaint();
            }
        });
    None
}

fn faded(enabled: bool) -> egui::Color32 {
    if enabled {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_white_alpha(110)
    }
}

/// L'icona del programma dentro `rect`: quella vera o la nostra.
fn paint_icon(ui: &egui::Ui, key: &str, rect: Rect, enabled: bool) {
    let t = Tokens::get(ui.ctx());
    match icon(key) {
        Some(tex) => egui::Image::new(egui::load::SizedTexture::new(tex.id(), rect.size()))
            .tint(faded(enabled))
            .paint_at(ui, rect),
        None => icons::paint(
            ui,
            "external-link",
            rect.shrink(rect.width() * 0.12),
            if enabled { t.icon } else { t.text_disabled },
        ),
    }
}

fn icon_image(key: &str, side: f32, enabled: bool, ctx: &egui::Context) -> egui::Image<'static> {
    let t = Tokens::get(ctx);
    match icon(key) {
        Some(tex) => {
            egui::Image::new(egui::load::SizedTexture::new(tex.id(), Vec2::splat(side))).tint(faded(enabled))
        }
        None => icons::image(
            "external-link",
            side * 0.9,
            if enabled { t.icon } else { t.text_disabled },
        ),
    }
}

/// Un programma come lo mostrano pulsanti e menu.
#[derive(Clone)]
struct Entry {
    key: String,
    label: String,
    name: String,
    ok: bool,
}

fn entries(paths: &[PathBuf]) -> Vec<Entry> {
    let s = state();
    s.config
        .for_paths(paths)
        .into_iter()
        .map(|p| Entry {
            key: p.key.clone(),
            label: p.button_label().to_string(),
            name: if p.name.is_empty() {
                p.button_label().to_string()
            } else {
                p.name.clone()
            },
            // Prima della fine della ricerca si dà per buono: lo dirà l'avvio.
            ok: !s.detected || ext::available(p, &s.found),
        })
        .collect()
}

/// Se il programma è fra quelli scelti per questo file.
pub fn opens(key: &str, path: &Path) -> bool {
    state().config.for_path(path).iter().any(|p| p.key == key)
}

fn type_label(paths: &[PathBuf]) -> Option<&'static str> {
    paths
        .first()
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .and_then(ext::type_of_ext)
        .map(|t| t.label)
}

fn type_key(path: &Path) -> Option<&'static str> {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(ext::type_of_ext)
        .map(|t| t.key)
}

fn missing_tip(name: &str) -> String {
    format!(
        "{name} non si trova più su questo PC. Se l'hai reinstallato o aggiornato, apri Programmi \
         esterni e premi «Cerca di nuovo i programmi»."
    )
}

fn program_label(c: &Config, key: &str) -> String {
    c.program(key)
        .map_or_else(|| key.to_string(), |p| p.button_label().to_string())
}

/// Apre i file con il programma, in sottofondo; l'esito arriva poi nella barra di stato. Con più
/// di dieci file prima chiede. Restituisce il messaggio di adesso.
pub fn launch(key: &str, paths: &[PathBuf], page: Option<u32>) -> String {
    let mut s = state();
    let Some(program) = s.config.program(key).cloned() else {
        return "Il programma non è più fra quelli scelti".into();
    };
    let name = program.button_label().to_string();
    let paths: Vec<PathBuf> = paths.iter().take(ext::MAX_FILES).cloned().collect();
    if paths.is_empty() {
        return String::new();
    }
    if paths.len() > MANY_FILES && s.confirm.is_none() {
        s.confirm = Some((key.to_string(), paths));
        return String::new();
    }
    if s.detected && !ext::available(&program, &s.found) {
        return missing_tip(&name);
    }
    let what = match paths.as_slice() {
        [one] => crate::viewer::file_name(one),
        many => format!("{} file", many.len()),
    };
    watch(&mut s, &paths);
    let tx = s.tx.clone();
    let wake = s.ctx.clone();
    let (what2, name2) = (what.clone(), name.clone());
    let spawned = std::thread::Builder::new().name("apri-in".into()).spawn(move || {
        let text = match ext::launch(&program, &paths, page) {
            Ok(()) => format!("Aperto {what2} in {name2}"),
            Err(e) => format!("Non riesco ad aprire {what2}: {e}"),
        };
        let _ = tx.send(Msg::Done(text));
        if let Some(c) = wake {
            c.request_repaint();
        }
    });
    match spawned {
        Ok(_) => format!("Apro {what} in {name}…"),
        Err(e) => format!("Non riesco ad avviare {name}: {e}"),
    }
}

fn show_confirm(ctx: &egui::Context) {
    let Some((key, paths)) = state().confirm.clone() else {
        return;
    };
    let name = program_label(&state().config, &key);
    let t = Tokens::get(ctx);
    let mut answer = None;
    egui::Window::new("Aprire tanti file?")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(format!("Stai per aprire {} file in {name}.", paths.len())).color(t.text),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(format!("Apri i {} file", paths.len())).clicked() {
                    answer = Some(true);
                }
                if ui.button("Annulla").clicked() {
                    answer = Some(false);
                }
            });
        });
    if let Some(go) = answer {
        // La domanda resta aperta durante l'avvio, così `launch` non la ripropone.
        let text = if go {
            launch(&key, &paths, None)
        } else {
            String::new()
        };
        let mut s = state();
        s.confirm = None;
        if !text.is_empty() {
            s.messages.push(text);
        }
    }
}

/// Il sottomenu «Apri in ›» del tasto destro per questi file. Restituisce il programma scelto.
pub fn menu(ui: &mut egui::Ui, paths: &[PathBuf]) -> Option<String> {
    let list = entries(paths);
    let mut chosen = None;
    let label = match paths.len() {
        0 | 1 => "Apri in".to_string(),
        n => format!("Apri i {n} file in"),
    };
    ui.menu_button(label, |ui| {
        for e in &list {
            let b = egui::Button::image_and_text(icon_image(&e.key, 16.0, e.ok, ui.ctx()), e.label.as_str());
            let r = ui.add_enabled(e.ok, b);
            let r = if e.ok {
                r.on_hover_text(&e.name)
            } else {
                r.on_disabled_hover_text(missing_tip(&e.name))
            };
            if r.clicked() {
                chosen = Some(e.key.clone());
                ui.close();
            }
        }
        if list.is_empty() {
            let what = type_label(paths).map_or("questo tipo di file".to_string(), |l| format!("i file {l}"));
            ui.add_enabled(false, egui::Button::new(format!("Nessun programma per {what}")));
        }
        ui.separator();
        if ui
            .button("Programmi esterni…")
            .on_hover_text("Scegli con quali programmi aprire ogni tipo di file")
            .clicked()
        {
            open_settings(paths.first().and_then(|p| type_key(p)));
            ui.close();
        }
    });
    chosen
}

/// I pulsanti dei programmi del file nella barra di una scheda: uno per programma, con icona e
/// nome (solo l'icona se `compact`), poi la matita per cambiarli. Restituisce il programma premuto.
pub fn buttons(ui: &mut egui::Ui, paths: &[PathBuf], compact: bool) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    let list = entries(paths);
    let mut chosen = None;
    for (n, e) in list.iter().enumerate() {
        let tint = if e.ok { t.text_strong } else { t.text_disabled };
        let image = icon_image(&e.key, 18.0, e.ok, ui.ctx());
        let b = if compact {
            egui::Button::image(image)
        } else {
            egui::Button::image_and_text(image, egui::RichText::new(&e.label).color(tint))
        }
        .min_size(vec2(32.0, 32.0));
        let keys = if n < 9 {
            format!(" (Ctrl+Maiusc+{})", n + 1)
        } else {
            String::new()
        };
        let r = ui.add_enabled(e.ok, b);
        let r = if e.ok {
            r.on_hover_text(format!("Apri in {}{keys}", e.name))
        } else {
            r.on_disabled_hover_text(missing_tip(&e.name))
        };
        if r.clicked() {
            chosen = Some(e.key.clone());
        }
    }
    let what = type_label(paths).map_or("questo tipo di file".to_string(), |l| format!("i file {l}"));
    let tip = if list.is_empty() {
        format!("Programmi esterni: scegli con quali programmi aprire {what}")
    } else {
        format!("Programmi esterni: cambia i programmi per {what}")
    };
    if !paths.is_empty() && icons::button(ui, "pencil", &tip, false, true, 28.0).clicked() {
        open_settings(paths.first().and_then(|p| type_key(p)));
    }
    chosen
}

/// Ctrl+Maiusc+1…9: il programma in quel posto per questi file. Il tasto si toglie, così non vale
/// anche come Ctrl+1…9 («Passa a»).
pub fn shortcut(ctx: &egui::Context, paths: &[PathBuf]) -> Option<String> {
    use egui::Key;
    const KEYS: [Key; 9] = [
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
    ];
    if paths.is_empty() || ctx.egui_wants_keyboard_input() {
        return None;
    }
    let mut hit = None;
    ctx.input_mut(|i| {
        i.events.retain(|e| {
            let egui::Event::Key {
                key,
                physical_key,
                pressed,
                modifiers,
                ..
            } = e
            else {
                return true;
            };
            if !(modifiers.command && modifiers.shift && !modifiers.alt) {
                return true;
            }
            match KEYS
                .iter()
                .position(|k| physical_key.as_ref() == Some(k) || key == k)
            {
                Some(n) => {
                    if *pressed {
                        hit = Some(n);
                    }
                    false
                }
                None => true,
            }
        });
    });
    let n = hit?;
    entries(paths).get(n).filter(|e| e.ok).map(|e| e.key.clone())
}

// --- Le modifiche non salvate ----------------------------------------------------------------

/// La risposta alla domanda delle modifiche non salvate.
pub enum SaveChoice {
    Save,
    AsIs,
    Cancel,
}

/// «Il programma esterno vede il file salvato»: `None` finché non si risponde.
pub fn ask_save(ctx: &egui::Context, key: &str, what: &str) -> Option<SaveChoice> {
    let name = program_label(&state().config, key);
    let t = Tokens::get(ctx);
    let mut answer = None;
    egui::Window::new("Modifiche non salvate")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.label(
                egui::RichText::new(format!(
                    "{name} apre {what} com'è salvato su disco: le modifiche fatte qui non le vede."
                ))
                .color(t.text),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Salva e apri").clicked() {
                    answer = Some(SaveChoice::Save);
                }
                if ui.button("Apri il file com'è").clicked() {
                    answer = Some(SaveChoice::AsIs);
                }
                if ui.button("Annulla").clicked() {
                    answer = Some(SaveChoice::Cancel);
                }
            });
        });
    answer
}

// --- La finestra Programmi esterni ----------------------------------------------------------

/// Un programma che si sta aggiungendo: per quali tipi.
struct Adding {
    program: Program,
    types: BTreeMap<&'static str, bool>,
}

struct Settings {
    current: &'static str,
    adding: Option<Adding>,
    /// «Ripristina i consigliati» premuto una volta: al secondo clic si fa.
    confirm_reset: bool,
    /// I nomi dei pulsanti in scrittura.
    labels: HashMap<String, String>,
    status: String,
}

/// Apre la finestra Programmi esterni sul tipo dato (o sul primo).
pub fn open_settings(type_key: Option<&str>) {
    let current = type_key
        .and_then(ext::type_by_key)
        .or_else(|| TYPES.first())
        .map_or("pdf", |t| t.key);
    state().window = Some(Settings {
        current,
        adding: None,
        confirm_reset: false,
        labels: HashMap::new(),
        status: String::new(),
    });
}

/// Le cose da fare dopo il disegno della finestra (lo stato non resta bloccato mentre si disegna).
enum Change {
    Select(&'static str),
    Shift(String, bool),
    Remove(String),
    Rename(String, String),
    StartAdd(Program),
    ConfirmAdd,
    CancelAdd,
    Toggle(&'static str, bool),
    Reset,
    Detect,
    Close,
}

/// Ciò che serve per disegnare la finestra, copiato dallo stato.
struct View {
    config: Config,
    found: Vec<Found>,
    detecting: bool,
    detected: bool,
    current: &'static str,
    adding: Option<(Program, BTreeMap<&'static str, bool>)>,
    confirm_reset: bool,
    labels: HashMap<String, String>,
    status: String,
}

fn show_settings(ctx: &egui::Context) {
    let v = {
        let s = state();
        let Some(w) = &s.window else {
            return;
        };
        View {
            config: s.config.clone(),
            found: s.found.clone(),
            detecting: s.detecting,
            detected: s.detected,
            current: w.current,
            adding: w.adding.as_ref().map(|a| (a.program.clone(), a.types.clone())),
            confirm_reset: w.confirm_reset,
            labels: w.labels.clone(),
            status: w.status.clone(),
        }
    };
    let t = Tokens::get(ctx);
    let mut changes: Vec<Change> = Vec::new();
    let mut open = true;
    let screen = ctx.content_rect();
    let size = vec2(780.0, 560.0).min(screen.size() - vec2(40.0, 80.0));
    egui::Window::new("Programmi esterni")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .fixed_size(size)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(
                    "Per ogni tipo di file scegli i programmi da avere a portata di clic: ognuno diventa un \
                     pulsante «Apri in …» in Visualizza, Modifica PDF e Modifica immagini, e una voce del \
                     tasto destro.",
                )
                .color(t.text),
            );
            ui.add_space(10.0);
            let body_h = (ui.available_height() - 50.0).max(160.0);
            ui.horizontal_top(|ui| {
                // A sinistra i tipi, raggruppati.
                ui.allocate_ui_with_layout(vec2(230.0, body_h), egui::Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(vec2(230.0, body_h));
                    let rect = ui.max_rect();
                    ui.painter().rect_filled(rect, t.card_radius, t.panel_darker);
                    egui::ScrollArea::vertical()
                        .id_salt("tipi-esterni")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add_space(4.0);
                            for g in ext::Group::ALL {
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(12.0);
                                    ui.label(
                                        egui::RichText::new(g.label())
                                            .font(semibold(12.0))
                                            .color(t.text_dim),
                                    );
                                });
                                for ty in TYPES.iter().filter(|ty| ty.group == g) {
                                    if type_row(ui, ty, v.current == ty.key, &v.config) {
                                        changes.push(Change::Select(ty.key));
                                    }
                                }
                            }
                            ui.add_space(6.0);
                        });
                });
                ui.add_space(14.0);
                // A destra i programmi del tipo scelto.
                let w = ui.available_width();
                ui.allocate_ui_with_layout(vec2(w, body_h), egui::Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(vec2(w, body_h));
                    egui::ScrollArea::vertical()
                        .id_salt("programmi-esterni")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width() - 8.0);
                            programs_side(ui, &v, &mut changes);
                        });
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().interact_size.y = 30.0;
                if v.detecting {
                    ui.add(egui::Spinner::new().color(t.accent));
                    ui.label(egui::RichText::new("Cerco i programmi installati…").color(t.text_dim));
                } else if ui
                    .button("Cerca di nuovo i programmi")
                    .on_hover_text("Dopo aver installato o aggiornato un programma")
                    .clicked()
                {
                    changes.push(Change::Detect);
                }
                let reset = if v.confirm_reset {
                    "Sicuro? Clic di nuovo per rimettere i consigliati"
                } else {
                    "Ripristina i consigliati"
                };
                if ui
                    .add_enabled(!v.detecting, egui::Button::new(reset))
                    .on_hover_text(
                        "Toglie le tue scelte e rimette per ogni tipo i programmi consigliati che hai sul PC",
                    )
                    .clicked()
                {
                    changes.push(Change::Reset);
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Chiudi").clicked() {
                        changes.push(Change::Close);
                    }
                    if !v.status.is_empty() {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&v.status).size(12.5).color(t.text_dim))
                                .truncate(),
                        );
                    }
                });
            });
        });
    if !open {
        changes.push(Change::Close);
    }
    apply(changes);
}

fn apply(changes: Vec<Change>) {
    if changes.is_empty() {
        return;
    }
    let mut s = state();
    let found = s.found.clone();
    let mut close = false;
    let mut redetect = false;
    for c in changes {
        let State {
            config,
            window: Some(w),
            ..
        } = &mut *s
        else {
            return;
        };
        let was_confirming = w.confirm_reset;
        w.confirm_reset = false;
        match c {
            Change::Select(k) => {
                w.current = k;
                w.adding = None;
            }
            Change::Shift(k, up) => config.shift(w.current, &k, up),
            Change::Remove(k) => {
                let name = program_label(config, &k);
                config.remove(w.current, &k);
                w.status = format!("Tolto {name}");
            }
            Change::Rename(k, label) => {
                config.rename(&k, &label);
                w.labels.insert(k, label);
            }
            Change::StartAdd(program) => w.adding = Some(adding(config, &found, w.current, program)),
            Change::Toggle(k, on) => {
                if let Some(a) = &mut w.adding
                    && let Some(v) = a.types.get_mut(k)
                {
                    *v = on;
                }
            }
            Change::ConfirmAdd => {
                if let Some(a) = w.adding.take() {
                    let keys: Vec<&str> = a.types.iter().filter(|(_, on)| **on).map(|(k, _)| *k).collect();
                    let name = a.program.button_label().to_string();
                    let n = keys.len();
                    config.add(a.program, &keys);
                    w.status = match n {
                        0 => String::new(),
                        1 => format!("Aggiunto {name}"),
                        n => format!("Aggiunto {name} a {n} tipi di file"),
                    };
                }
            }
            Change::CancelAdd => w.adding = None,
            Change::Reset => {
                if was_confirming {
                    config.reset_recommended(&found);
                    w.labels.clear();
                    w.status = "Rimessi i programmi consigliati".into();
                } else {
                    w.confirm_reset = true;
                }
            }
            Change::Detect => redetect = true,
            Change::Close => close = true,
        }
    }
    if close {
        s.window = None;
    }
    drop(s);
    if redetect {
        detect();
    }
}

/// Un programma da aggiungere al tipo `current`: per quali tipi proporlo. Accesi il tipo scelto e
/// quelli dello stesso gruppo che il programma sa aprire e non ha ancora.
fn adding(config: &Config, found: &[Found], current: &'static str, program: Program) -> Adding {
    let can: BTreeSet<String> = found
        .iter()
        .find(|f| f.key == program.key)
        .map(|f| f.types.clone())
        .or_else(|| {
            program
                .family()
                .map(|f| f.types.iter().map(|t| (*t).to_string()).collect())
        })
        .unwrap_or_default();
    let group = ext::type_by_key(current).map(|t| t.group);
    let types = TYPES
        .iter()
        .filter(|t| t.key == current || can.contains(t.key))
        .filter(|t| t.key == current || !config.for_type(t.key).iter().any(|p| p.key == program.key))
        .map(|t| (t.key, t.key == current || Some(t.group) == group))
        .collect();
    Adding { program, types }
}

/// Una riga dei tipi: nome e le icone dei programmi scelti. Vero se cliccata.
fn type_row(ui: &mut egui::Ui, ty: &ext::FileType, selected: bool, config: &Config) -> bool {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
    let row = rect.shrink2(vec2(6.0, 1.0));
    if selected {
        ui.painter().rect_filled(row, t.radius, t.selection);
        // La barretta d'accento a sinistra, come le liste di Windows 11.
        let bar = Rect::from_min_size(row.min + vec2(0.0, 7.0), vec2(3.0, row.height() - 14.0));
        ui.painter().rect_filled(bar, 2.0, t.accent);
    } else if resp.hovered() {
        ui.painter().rect_filled(row, t.radius, t.hover);
    }
    ui.painter().text(
        row.left_center() + vec2(14.0, 0.0),
        egui::Align2::LEFT_CENTER,
        ty.label,
        if selected {
            semibold(13.5)
        } else {
            egui::FontId::proportional(13.5)
        },
        if selected { t.text_strong } else { t.text },
    );
    let programs = config.for_type(ty.key);
    let mut x = row.right() - 6.0;
    for p in programs.iter().take(4).rev() {
        let r = Rect::from_min_size(egui::pos2(x - 18.0, row.center().y - 9.0), Vec2::splat(18.0));
        paint_icon(ui, &p.key, r, true);
        x -= 22.0;
    }
    let tip = if programs.is_empty() {
        format!(".{} · nessun programma", ty.exts.join("  ."))
    } else {
        format!(
            ".{} · {}",
            ty.exts.join("  ."),
            programs
                .iter()
                .map(|p| p.button_label())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    resp.on_hover_text(tip).clicked()
}

fn programs_side(ui: &mut egui::Ui, v: &View, changes: &mut Vec<Change>) {
    let t = Tokens::get(ui.ctx());
    let Some(ty) = ext::type_by_key(v.current) else {
        return;
    };
    ui.label(
        egui::RichText::new(format!("File {}", ty.label))
            .font(semibold(17.0))
            .color(t.text_strong),
    );
    ui.label(
        egui::RichText::new(format!(".{}", ty.exts.join("  .")))
            .size(12.0)
            .color(t.text_dim),
    );
    ui.add_space(10.0);
    let list = v.config.for_type(v.current);
    if list.is_empty() {
        ui.label(
            egui::RichText::new(
                "Nessun programma: aggiungine uno qui sotto. Senza programmi il pulsante non c'è.",
            )
            .color(t.text_dim),
        );
        ui.add_space(6.0);
    }
    let n = list.len();
    for (i, p) in list.iter().enumerate() {
        let ok = !v.detected || ext::available(p, &v.found);
        program_card(ui, p, i, n, ok, &v.labels, changes);
        ui.add_space(6.0);
    }
    ui.add_space(10.0);
    if let Some((program, types)) = &v.adding {
        add_panel(ui, program, types, changes);
        return;
    }
    ui.label(
        egui::RichText::new("Aggiungi un programma")
            .font(semibold(14.0))
            .color(t.text_strong),
    );
    ui.add_space(4.0);
    let chosen: BTreeSet<&str> = list.iter().map(|p| p.key.as_str()).collect();
    let mut fits: Vec<&Found> = v
        .found
        .iter()
        .filter(|f| f.types.contains(v.current) && !chosen.contains(f.key.as_str()))
        .collect();
    fits.sort_by_key(|f| (ext::types::family(&f.key).is_none(), f.name.to_lowercase()));
    if !v.detected {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().color(t.accent));
            ui.label(egui::RichText::new("Cerco i programmi installati…").color(t.text_dim));
        });
    } else if fits.is_empty() {
        ui.label(
            egui::RichText::new(format!(
                "Sul PC non ho trovato altri programmi per i file {}: scegline uno con «Sfoglia…».",
                ty.label
            ))
            .color(t.text_dim),
        );
    }
    for f in fits {
        ui.horizontal(|ui| {
            ui.set_min_height(30.0);
            let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
            paint_icon(ui, &f.key, r, true);
            ui.label(egui::RichText::new(&f.name).color(t.text));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Aggiungi").clicked() {
                    changes.push(Change::StartAdd(Program::from_found(f)));
                }
            });
        });
    }
    ui.add_space(8.0);
    if ui
        .button("Sfoglia…")
        .on_hover_text("Scegli il programma (.exe) dalla cartella dove è installato")
        .clicked()
        && let Some(exe) = pick_exe()
    {
        changes.push(Change::StartAdd(Program::from_exe(&exe)));
    }
}

fn pick_exe() -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new().set_title("Scegli il programma");
    if cfg!(windows) {
        d = d.add_filter("Programmi", &["exe"]);
    }
    if let Some(dir) = std::env::var_os("ProgramFiles") {
        d = d.set_directory(dir);
    }
    d.pick_file()
}

/// Un pulsante con la freccia in su o in giù.
fn arrow(ui: &mut egui::Ui, up: bool, enabled: bool, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::splat(30.0),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, t.radius, t.hover);
    }
    let tint = if !enabled {
        t.text_disabled
    } else if resp.hovered() {
        t.text_strong
    } else {
        t.icon
    };
    let mut img = icons::image("chevron-down", 18.0, tint);
    if up {
        img = img.rotate(std::f32::consts::PI, Vec2::splat(0.5));
    }
    img.paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(18.0)));
    enabled && resp.on_hover_text(tip).clicked()
}

fn program_card(
    ui: &mut egui::Ui,
    p: &Program,
    i: usize,
    n: usize,
    ok: bool,
    labels: &HashMap<String, String>,
    changes: &mut Vec<Change>,
) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE
        .fill(t.panel)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(t.card_radius)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                // Il numero in una colonna fissa: i campi delle schede restano in fila.
                let (num, _) = ui.allocate_exact_size(vec2(14.0, 32.0), Sense::hover());
                ui.painter().text(
                    num.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{}", i + 1),
                    semibold(13.0),
                    t.text_dim,
                );
                let (r, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
                paint_icon(ui, &p.key, r, ok);
                ui.add_space(4.0);
                let right = 110.0;
                let text_w = (ui.available_width() - right).max(120.0);
                ui.allocate_ui_with_layout(vec2(text_w, 0.0), egui::Layout::top_down(Align::Min), |ui| {
                    ui.horizontal(|ui| {
                        let mut label = labels.get(&p.key).cloned().unwrap_or_else(|| p.label.clone());
                        let r = crate::fluent::text_field(
                            ui,
                            egui::TextEdit::singleline(&mut label)
                                .desired_width(160.0)
                                .margin(crate::fluent::MARGIN)
                                .vertical_align(Align::Center)
                                .hint_text(p.button_label()),
                        )
                        .on_hover_text("Il nome sul pulsante «Apri in …»");
                        if r.changed() {
                            changes.push(Change::Rename(p.key.clone(), label));
                        }
                        let name = if p.name.is_empty() {
                            p.button_label()
                        } else {
                            &p.name
                        };
                        ui.add(
                            egui::Label::new(egui::RichText::new(name).size(12.5).color(t.text)).truncate(),
                        );
                    });
                    let detail = match (&p.exe, &p.app_id) {
                        (Some(e), _) => e.display().to_string(),
                        (None, Some(_)) => "App di Windows".to_string(),
                        _ => String::new(),
                    };
                    if !ok {
                        ui.label(
                            egui::RichText::new("Non si trova più su questo PC")
                                .size(12.0)
                                .color(t.warning),
                        );
                    } else if !detail.is_empty() {
                        ui.add(
                            egui::Label::new(egui::RichText::new(detail).size(11.5).color(t.text_dim))
                                .truncate(),
                        );
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if icons::button(ui, "trash-2", "Togli da questo tipo di file", false, true, 30.0)
                        .clicked()
                    {
                        changes.push(Change::Remove(p.key.clone()));
                    }
                    if arrow(ui, false, i + 1 < n, "Sposta dopo") {
                        changes.push(Change::Shift(p.key.clone(), false));
                    }
                    if arrow(ui, true, i > 0, "Sposta prima") {
                        changes.push(Change::Shift(p.key.clone(), true));
                    }
                });
            });
        });
}

fn add_panel(
    ui: &mut egui::Ui,
    program: &Program,
    types: &BTreeMap<&'static str, bool>,
    changes: &mut Vec<Change>,
) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE
        .fill(t.panel_darker)
        .stroke(Stroke::new(1.0, t.accent))
        .corner_radius(t.card_radius)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                paint_icon(ui, &program.key, r, true);
                ui.label(
                    egui::RichText::new(format!("Aggiungi {}", program.button_label()))
                        .font(semibold(14.0))
                        .color(t.text_strong),
                );
            });
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Per quali tipi di file:").color(t.text));
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                for ty in TYPES {
                    let Some(on) = types.get(ty.key) else {
                        continue;
                    };
                    let mut v = *on;
                    if ui.add(crate::fluent::checkbox(&mut v, ty.label)).changed() {
                        changes.push(Change::Toggle(ty.key, v));
                    }
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let any = types.values().any(|v| *v);
                if ui.add_enabled(any, egui::Button::new("Aggiungi")).clicked() {
                    changes.push(Change::ConfirmAdd);
                }
                if ui.button("Annulla").clicked() {
                    changes.push(Change::CancelAdd);
                }
            });
        });
}
