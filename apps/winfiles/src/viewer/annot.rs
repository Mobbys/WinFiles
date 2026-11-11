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

use crate::annotate::Annotator;
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
fn sheet(size: Size) -> (Sheet, MmRect) {
    let f = factor(size);
    let v = Size::new(size.w * f, size.h * f);
    let image = MmRect::new(0.0, 0.0, v.w, v.h);
    let sheet = Sheet {
        size: v,
        area: image,
        title: None,
        headers: Vec::new(),
        items: vec![Placed {
            id: 0,
            cell: image,
            image,
            caption: MmRect::new(0.0, v.h, 0.0, 0.0),
        }],
    };
    (sheet, image)
}

/// La scena delle annotazioni di una pagina sul suo foglio virtuale.
pub fn scene(ctx: &egui::Context, list: &[Annotation], size: Size, unit: Unit) -> Page {
    let (sheet, image) = sheet(size);
    let mut sk = Sketch::new(ctx, sheet.size);
    crate::annotate::draw(&mut sk, image, list, None, unit, &|_, _| None);
    sk.page
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
        }
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

    /// Le annotazioni sopra la pagina: `paper` è la pagina a schermo, `scale` i punti per mm
    /// della pagina, `resp` la tavola (che sente clic e trascinamenti).
    #[allow(clippy::too_many_arguments)]
    pub fn view(
        &mut self,
        ui: &mut egui::Ui,
        resp: &egui::Response,
        clip: Rect,
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
        let (sheet, image) = sheet(size);
        let vscale = scale / factor(size) as f32;
        let origin = paper.min;
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
            self.annot
                .interact(&mut child, resp, origin, vscale, &sheet, &slots, &mut self.shown);
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
        let out = self.annot.bar(ui, selected.as_ref(), &mut show);
        self.show = show;
        self.annot.apply(&out, &mut self.project);
        self.save_if_changed();
    }

    /// I tasti delle annotazioni, prima di quelli della scheda.
    pub fn keys(&mut self, ctx: &egui::Context) {
        if !self.annot.on {
            return;
        }
        self.annot.keys(ctx, &mut self.project, &mut self.shown);
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
