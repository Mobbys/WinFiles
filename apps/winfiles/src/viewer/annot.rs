//! Le annotazioni del Visualizza: la stessa riga Annota di Miniature (testi, frecce, forme,
//! evidenziatore) sopra la pagina che si guarda. Il file non si tocca mai: le annotazioni si
//! salvano in WinFiles, per file e per pagina, in `annotazioni-visualizza.json` nella cartella
//! delle impostazioni, e si ritrovano riaprendo il file.
//!
//! Spessori e corpi sono in mm su un foglio "virtuale" con il lato lungo di un A4: la pagina vi
//! si adatta. Così un testo di 4 mm si legge uguale su un biglietto da visita e su un cartellone,
//! e stampando la pagina su un A4 le misure sono quelle scritte.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use egui::Rect;
use serde::{Deserialize, Serialize};
use winfiles_geom::{Rect as MmRect, Size, Unit};
use winfiles_layout::{Placed, Sheet};
use winfiles_model::{Annotation, FileEntry, FileKind, MAX_PER_PAGE, Project};
use winfiles_scene::Page;

use crate::annotate::{Annotator, Style};
use crate::captions::Slot;
use crate::sketch::Sketch;

/// Il lato lungo del foglio virtuale, in mm: quello di un A4.
pub const SHEET_LONG_MM: f64 = 297.0;
/// Tetto alla dimensione del file letto.
const MAX_BYTES: u64 = 32 << 20;
/// File con annotazioni al più: oltre si tengono gli ultimi.
const MAX_FILES: usize = 5000;

/// Le annotazioni di un file, come si salvano.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Stored {
    path: PathBuf,
    pages: BTreeMap<usize, Vec<Annotation>>,
}

/// mm del foglio virtuale per mm della pagina.
pub fn factor(size: Size) -> f64 {
    SHEET_LONG_MM / size.w.max(size.h).max(1e-6)
}

/// Il foglio virtuale di una pagina, con la pagina che lo riempie.
#[cfg(test)]
fn sheet(size: Size) -> (Sheet, MmRect) {
    let f = factor(size);
    let image = MmRect::new(0.0, 0.0, size.w * f, size.h * f);
    (sheet_with(Size::new(image.w, image.h), image), image)
}

/// Un foglio virtuale grande `size` con la pagina in `image`: le annotazioni possono andare
/// anche fuori dalla pagina, fin dove arriva il foglio.
fn sheet_with(size: Size, image: MmRect) -> Sheet {
    Sheet {
        size,
        area: MmRect::new(0.0, 0.0, size.w, size.h),
        title: None,
        headers: Vec::new(),
        items: vec![Placed {
            id: 0,
            cell: image,
            image,
            caption: MmRect::new(image.x, image.bottom(), 0.0, 0.0),
        }],
    }
}

/// Margine attorno alle annotazioni nella scena di stampa (mm del foglio virtuale): punte e
/// spessori escono un poco dal riquadro dei punti.
const SCENE_PAD: f64 = 10.0;
/// Fin dove una scena di stampa si allarga oltre la pagina (mm del foglio virtuale).
const SCENE_REACH: f64 = 3000.0;

/// La scena delle annotazioni di una pagina, grande quanto serve a contenerle tutte, anche
/// quelle fuori dalla pagina; con il posto dell'angolo della pagina nella scena (mm virtuali).
pub fn scene(ctx: &egui::Context, list: &[Annotation], size: Size, unit: Unit) -> (Page, (f64, f64)) {
    let f = factor(size);
    let v = Size::new(size.w * f, size.h * f);
    let page = MmRect::new(0.0, 0.0, v.w, v.h);
    let (mut x0, mut y0, mut x1, mut y1) = (0.0_f64, 0.0_f64, v.w, v.h);
    for a in list {
        let b = crate::annotate::bounds_mm(ctx, a, page);
        x0 = x0.min(b.x - SCENE_PAD);
        y0 = y0.min(b.y - SCENE_PAD);
        x1 = x1.max(b.right() + SCENE_PAD);
        y1 = y1.max(b.bottom() + SCENE_PAD);
    }
    let x0 = x0.max(-SCENE_REACH);
    let y0 = y0.max(-SCENE_REACH);
    let x1 = x1.min(v.w + SCENE_REACH);
    let y1 = y1.min(v.h + SCENE_REACH);
    let image = MmRect::new(-x0, -y0, v.w, v.h);
    let sheet = sheet_with(Size::new(x1 - x0, y1 - y0), image);
    let mut sk = Sketch::new(ctx, sheet.size);
    crate::annotate::draw(&mut sk, image, list, None, unit, &|_, _| None);
    (sk.page, (-x0, -y0))
}

pub struct Notes {
    pub annot: Annotator,
    /// Le annotazioni vere: un file del "lavoro" per ogni file annotato.
    project: Project,
    /// La copia mostrata, dove cambia un gesto a metà.
    shown: Project,
    /// Le annotazioni si vedono (e vanno in stampa).
    pub show: bool,
    /// Com'erano all'ultimo salvataggio.
    saved: Vec<Stored>,
    store: Option<PathBuf>,
    /// mm del foglio virtuale per mm di carta, mentre si annota nell'anteprima di stampa: lì
    /// spessori e corpi si scrivono in mm della carta.
    pub paper: Option<f64>,
    /// Lo stile dell'anteprima di stampa, in mm della carta: a parte da quello del Visualizza.
    paper_style: Style,
}

impl Notes {
    pub fn new() -> Self {
        let store = crate::prefs::config_dir().map(|d| d.join("annotazioni-visualizza.json"));
        let saved = store.as_deref().map(load_from).unwrap_or_default();
        Self::with(saved, store)
    }

    fn with(saved: Vec<Stored>, store: Option<PathBuf>) -> Self {
        let mut project = Project::new("Visualizza");
        for s in &saved {
            let mut f = FileEntry::new(s.path.clone(), FileKind::from_path(&s.path));
            for (page, list) in &s.pages {
                f.set_annotations(*page, list.clone());
            }
            project.files.push(f);
        }
        let mut annot = Annotator::default();
        annot.no_quote = true;
        Self {
            annot,
            shown: project.clone(),
            project,
            show: true,
            saved,
            store,
            paper: None,
            paper_style: Style::default(),
        }
    }

    /// Fa `f` con lo stile della carta se si annota nella stampa: in mm del foglio virtuale se
    /// `virtual_mm` (per tracciare), in mm della carta altrimenti (per la riga dei comandi).
    fn styled<R>(&mut self, virtual_mm: bool, f: impl FnOnce(&mut Self) -> R) -> R {
        let Some(r) = self.paper else {
            return f(self);
        };
        let k = if virtual_mm { r } else { 1.0 };
        let own = std::mem::replace(&mut self.annot.style, self.paper_style.scaled(k));
        let out = f(self);
        self.paper_style = self.annot.style.scaled(1.0 / k);
        self.annot.style = own;
        out
    }

    /// Le annotazioni di una pagina di un file.
    pub fn list(&self, path: &Path, page: usize) -> &[Annotation] {
        self.project
            .files
            .iter()
            .find(|f| f.path == path)
            .map_or(&[], |f| f.annotations(page))
    }

    /// Il file ha annotazioni, su qualche pagina.
    pub fn any(&self, path: &Path) -> bool {
        self.project
            .files
            .iter()
            .any(|f| f.path == path && !f.annotations.is_empty())
    }

    /// Il posto del file nel lavoro delle annotazioni, aggiunto se non c'è.
    fn index(&mut self, path: &Path) -> usize {
        if let Some(i) = self.project.files.iter().position(|f| f.path == path) {
            return i;
        }
        let f = FileEntry::new(path.to_path_buf(), FileKind::from_path(path));
        self.project.files.push(f.clone());
        self.shown.files.push(f);
        self.project.files.len() - 1
    }

    /// I riquadri delle annotazioni di una pagina grande `size`, in mm della pagina.
    pub fn bounds(&self, ctx: &egui::Context, path: &Path, page: usize, size: Size) -> Vec<MmRect> {
        let f = factor(size);
        let image = MmRect::new(0.0, 0.0, size.w * f, size.h * f);
        self.list(path, page)
            .iter()
            .map(|a| {
                let b = crate::annotate::bounds_mm(ctx, a, image);
                MmRect::new(b.x / f, b.y / f, b.w / f, b.h / f)
            })
            .collect()
    }

    /// Le annotazioni sopra la pagina: `paper` è la pagina a schermo, `frame` dove le
    /// annotazioni possono andare (anche fuori dalla pagina), `scale` i punti per mm della
    /// pagina, `resp` la tavola (che sente clic e trascinamenti).
    #[allow(clippy::too_many_arguments)]
    pub fn view(
        &mut self,
        ui: &mut egui::Ui,
        resp: &egui::Response,
        clip: Rect,
        frame: Rect,
        paper: Rect,
        scale: f32,
        size: Size,
        path: &Path,
        page: usize,
        pages: usize,
        unit: Unit,
    ) {
        if !self.annot.on && !(self.show && self.any(path)) {
            return;
        }
        let file = self.index(path);
        if !self.annot.busy() {
            self.shown = self.project.clone();
        }
        let f = factor(size);
        let vscale = (scale / f as f32).max(1e-6);
        let origin = frame.min;
        let image = MmRect::new(
            f64::from((paper.min.x - frame.min.x) / vscale),
            f64::from((paper.min.y - frame.min.y) / vscale),
            size.w * f,
            size.h * f,
        );
        let room = Size::new(
            f64::from(frame.width() / vscale),
            f64::from(frame.height() / vscale),
        );
        let sheet = sheet_with(room, image);
        if self.show || self.annot.on {
            let list = self
                .shown
                .files
                .get(file)
                .map(|f| f.annotations(page).to_vec())
                .unwrap_or_default();
            if !list.is_empty() {
                let mut sk = Sketch::new(ui.ctx(), sheet.size);
                crate::annotate::draw(&mut sk, image, &list, None, unit, &|_, _| None);
                crate::paint::vectors(&ui.painter_at(clip), origin, vscale, &sk.page);
            }
        }
        if self.annot.on {
            let slots = [Slot {
                file,
                page,
                pages,
                color: None,
            }];
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(clip));
            child.set_clip_rect(clip);
            self.styled(true, |n| {
                n.annot
                    .interact(&mut child, resp, origin, vscale, &sheet, &slots, &mut n.shown);
            });
            for (f, p) in self.annot.take_pending() {
                self.annot.commit(&mut self.project, &self.shown, f, p);
            }
            ui.ctx().request_repaint();
        }
        self.save_if_changed();
    }

    /// La riga Annota.
    pub fn bar(&mut self, ui: &mut egui::Ui) {
        let selected = self.annot.selected_annotation(&self.shown).cloned();
        let mut show = self.show;
        let out = self.styled(false, |n| n.annot.bar(ui, selected.as_ref(), &mut show));
        self.show = show;
        self.styled(true, |n| n.annot.apply(&out, &mut n.project));
        self.save_if_changed();
    }

    /// I tasti delle annotazioni, prima di quelli della scheda.
    pub fn keys(&mut self, ctx: &egui::Context) {
        if !self.annot.on {
            return;
        }
        self.styled(true, |n| n.annot.keys(ctx, &mut n.project, &mut n.shown));
        self.save_if_changed();
    }

    fn stored(&self) -> Vec<Stored> {
        self.project
            .files
            .iter()
            .filter(|f| !f.annotations.is_empty())
            .map(|f| Stored {
                path: f.path.clone(),
                pages: f.annotations.clone(),
            })
            .collect()
    }

    /// Salva le annotazioni se sono cambiate dall'ultima volta.
    fn save_if_changed(&mut self) {
        let now = self.stored();
        if now == self.saved {
            return;
        }
        if let Some(file) = &self.store {
            // Non riuscire a salvare non deve fermare chi lavora: si riprova alla prossima modifica.
            if save_to(file, &now).is_err() {
                return;
            }
        }
        self.saved = now;
    }
}

/// Legge le annotazioni salvate; un file illeggibile o rovinato vale "nessuna annotazione".
fn load_from(file: &Path) -> Vec<Stored> {
    let ok = std::fs::metadata(file).is_ok_and(|m| m.len() <= MAX_BYTES);
    if !ok {
        return Vec::new();
    }
    let mut list: Vec<Stored> = std::fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let skip = list.len().saturating_sub(MAX_FILES);
    list.drain(..skip);
    for s in &mut list {
        for annotations in s.pages.values_mut() {
            annotations.retain_mut(Annotation::sanitize);
            annotations.truncate(MAX_PER_PAGE);
        }
        s.pages.retain(|_, l| !l.is_empty());
    }
    list.retain(|s| !s.pages.is_empty());
    list
}

fn save_to(file: &Path, list: &[Stored]) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string(list).map_err(std::io::Error::other)?;
    // Prima in un file a parte, poi al suo posto: un'interruzione non lascia il file a metà.
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use winfiles_model::Rgba;

    fn arrow() -> Annotation {
        Annotation::Linea {
            a: (0.1, 0.1),
            b: (0.5, 0.5),
            color: Rgba::rgb(220, 38, 38),
            width: 0.5,
            start_arrow: false,
            end_arrow: true,
            end: None,
        }
    }

    #[test]
    fn le_annotazioni_si_ritrovano_riaprendo() {
        let dir = std::env::temp_dir().join(format!("winfiles-annot-visualizza-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("annotazioni-visualizza.json");
        let path = PathBuf::from("C:/lavori/manifesto.pdf");
        let mut notes = Notes::with(Vec::new(), Some(file.clone()));
        let i = notes.index(&path);
        notes.project.files[i].set_annotations(2, vec![arrow()]);
        notes.save_if_changed();
        let again = Notes::with(load_from(&file), Some(file));
        assert_eq!(again.list(&path, 2), &[arrow()]);
        assert!(again.list(&path, 0).is_empty());
        assert!(again.any(&path));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn un_file_rovinato_non_ferma_niente() {
        let dir = std::env::temp_dir().join(format!("winfiles-annot-rovinato-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.json");
        std::fs::write(&file, b"{ non e json").unwrap();
        assert!(load_from(&file).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn il_foglio_virtuale_ha_il_lato_lungo_di_un_a4() {
        let (s, image) = sheet(Size::new(1000.0, 500.0));
        assert!((s.size.w - 297.0).abs() < 1e-9);
        assert!((s.size.h - 148.5).abs() < 1e-9);
        assert_eq!(image.w, s.size.w);
    }
}
