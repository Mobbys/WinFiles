//! Cosa va sul foglio per ogni pagina di ogni file, e quanto è alto il suo testo.
//!
//! Il motore di layout decide dove va ogni cosa, ma le altezze dei testi dipendono dai caratteri:
//! le misura questo modulo, in millimetri, con le stesse regole che l'anteprima usa per
//! disegnarli. Così didascalie e note non escono mai dal loro spazio.

use egui::Color32;
use egui::text::{LayoutJob, TextFormat};
use winfiles_geom::Size;
use winfiles_layout::{Item, cell_width};
use winfiles_model::{FileKind, Project};

/// Millimetri per punto tipografico.
pub const MM_PER_PT: f64 = 25.4 / 72.0;
/// Interlinea rispetto al corpo del testo.
const LEADING: f64 = 1.25;
/// Le righe delle misure sono un po' più piccole del nome del file.
pub const INFO_RATIO: f64 = 0.85;
/// Spazio sopra e sotto la didascalia, in mm.
pub const CAPTION_PAD: f64 = 1.0;
/// Spazio tra le misure e la nota del file, in mm.
pub const NOTE_GAP: f64 = 0.8;
/// Rientro della nota della cartella dai bordi, in mm.
pub const FOLDER_NOTE_INSET: f64 = 2.0;
/// Pixel per millimetro con cui si misurano i testi: abbastanza da non arrotondare i caratteri.
const MEASURE_PX_PER_MM: f64 = 10.0;

/// Colori che distinguono i file con più pagine: la stessa etichetta "Pag. 2/4" ha lo stesso
/// colore in tutte le pagine dello stesso file. Sono colori di stampa, scuri abbastanza da
/// leggerci sopra il testo bianco.
const PAGE_COLORS: [Color32; 8] = [
    Color32::from_rgb(0x1f, 0x5f, 0xc8),
    Color32::from_rgb(0xc2, 0x4a, 0x0c),
    Color32::from_rgb(0x2b, 0x7a, 0x36),
    Color32::from_rgb(0x6a, 0x3f, 0xc8),
    Color32::from_rgb(0xb0, 0x23, 0x23),
    Color32::from_rgb(0x0b, 0x72, 0x85),
    Color32::from_rgb(0xa8, 0x1e, 0x5a),
    Color32::from_rgb(0x7a, 0x4a, 0x1e),
];

/// Colore del testo delle note sul foglio, rosso come nella 4.9.
pub const NOTE_INK: Color32 = Color32::from_rgb(0xc0, 0x10, 0x10);

/// Una miniatura sul foglio: una pagina di un file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    /// Indice del file nel progetto.
    pub file: usize,
    /// Pagina, da 0.
    pub page: usize,
    /// Pagine del file.
    pub pages: usize,
    /// Colore dell'etichetta di pagina, solo per i file con più pagine.
    pub color: Option<Color32>,
}

/// Una miniatura per ogni pagina di ogni file, nell'ordine del progetto.
pub fn slots(project: &Project) -> Vec<Slot> {
    let mut out = Vec::with_capacity(project.files.len());
    let mut multi = 0usize;
    for (file, f) in project.files.iter().enumerate() {
        let pages = f.pages.len().max(1);
        let color = (pages > 1).then(|| {
            let c = PAGE_COLORS
                .get(multi % PAGE_COLORS.len())
                .copied()
                .unwrap_or(Color32::DARK_GRAY);
            multi += 1;
            c
        });
        out.extend((0..pages).map(|page| Slot {
            file,
            page,
            pages,
            color,
        }));
    }
    out
}

/// Altezza di una riga di testo di corpo `pt`, in mm.
pub fn line_mm(pt: f64) -> f64 {
    pt * MM_PER_PT * LEADING
}

/// Il testo di una nota impostato per l'anteprima o per la misura: `px_per_mm` pixel per mm.
pub fn note_job(text: &str, pt: f64, width_mm: f64, px_per_mm: f32) -> LayoutJob {
    let size = (pt * MM_PER_PT) as f32 * px_per_mm;
    let mut job = LayoutJob::single_section(
        text.trim().to_string(),
        TextFormat {
            font_id: egui::FontId::proportional(size.max(0.5)),
            color: NOTE_INK,
            line_height: Some((line_mm(pt) as f32 * px_per_mm).max(0.5)),
            ..Default::default()
        },
    );
    job.wrap.max_width = (width_mm as f32 * px_per_mm).max(1.0);
    job.wrap.break_anywhere = false;
    job
}

/// Quante righe occupa una nota larga `width_mm`.
pub fn note_rows(ctx: &egui::Context, text: &str, pt: f64, width_mm: f64) -> usize {
    if text.trim().is_empty() {
        return 0;
    }
    let job = note_job(text, pt, width_mm, MEASURE_PX_PER_MM as f32);
    ctx.fonts_mut(|f| f.layout_job(job)).rows.len()
}

/// Altezza della didascalia (nome, misura, area e nota), in mm.
pub fn caption_height(ctx: &egui::Context, note: &str, caption_pt: f64, note_pt: f64, width: f64) -> f64 {
    let info = line_mm(caption_pt * INFO_RATIO);
    let mut h = CAPTION_PAD * 2.0 + line_mm(caption_pt) + 2.0 * info;
    let rows = note_rows(ctx, note, note_pt, width);
    if rows > 0 {
        h += NOTE_GAP + rows as f64 * line_mm(note_pt);
    }
    h
}

/// Altezza della nota di una cartella sotto la sua intestazione, in mm.
pub fn folder_note_height(ctx: &egui::Context, note: &str, note_pt: f64, width: f64) -> f64 {
    match note_rows(ctx, note, note_pt, width - 2.0 * FOLDER_NOTE_INSET) {
        0 => 0.0,
        rows => rows as f64 * line_mm(note_pt) + 2.0 * CAPTION_PAD,
    }
}

/// Proporzioni provvisorie finché un file non è stato misurato.
fn placeholder_size(kind: FileKind) -> Size {
    match kind {
        FileKind::Image => Size::new(4.0, 3.0),
        _ => Size::new(210.0, 297.0),
    }
}

/// Gli elementi da impaginare: una cella per ogni pagina, con le proporzioni vere e l'altezza dei
/// suoi testi.
pub fn items(ctx: &egui::Context, project: &Project, slots: &[Slot]) -> Vec<Item> {
    let labels = project.folder_labels();
    let print = &project.print;
    let width = cell_width(print);
    let sheet_width = winfiles_geom::sheet_size(print.paper, print.orientation).w
        - print.margins.left
        - print.margins.right;
    slots
        .iter()
        .enumerate()
        .filter_map(|(id, slot)| {
            let f = project.files.get(slot.file)?;
            let folder_note = project.folder_notes.get(f.folder()).map_or("", String::as_str);
            Some(Item {
                id,
                content: f
                    .pages
                    .get(slot.page)
                    .map(|p| p.finished())
                    .unwrap_or_else(|| placeholder_size(f.kind)),
                folder: labels
                    .get(f.folder())
                    .cloned()
                    .unwrap_or_else(|| f.folder().display().to_string()),
                caption: caption_height(ctx, &f.note, print.caption_text, print.note_text, width),
                folder_note: folder_note_height(ctx, folder_note, print.note_text, sheet_width),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use winfiles_model::{FileEntry, PageInfo};

    use super::*;

    fn file(name: &str, pages: usize) -> FileEntry {
        let mut f = FileEntry::new(PathBuf::from(name), FileKind::Pdf);
        f.pages = (0..pages)
            .map(|_| PageInfo {
                size: Size::new(210.0, 297.0),
                trim: None,
            })
            .collect();
        f
    }

    #[test]
    fn ogni_pagina_ha_la_sua_cella_e_il_colore_del_suo_file() {
        let mut p = Project::new("prova");
        p.files = vec![
            file("/a/uno.pdf", 3),
            file("/a/due.pdf", 1),
            file("/a/tre.pdf", 2),
        ];
        let s = slots(&p);
        assert_eq!(s.len(), 6);
        assert_eq!((s[0].file, s[0].page, s[0].pages), (0, 0, 3));
        assert_eq!((s[2].file, s[2].page), (0, 2));
        // Le pagine dello stesso file hanno lo stesso colore, file diversi colori diversi.
        assert_eq!(s[0].color, s[2].color);
        assert_eq!(s[3].color, None);
        assert!(s[4].color.is_some() && s[4].color != s[0].color);
        // Un file non ancora misurato ha comunque la sua cella.
        p.files
            .push(FileEntry::new(PathBuf::from("/a/x.jpg"), FileKind::Image));
        assert_eq!(slots(&p).len(), 7);
    }

    #[test]
    fn la_nota_allunga_la_didascalia() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let ctx = ui.ctx();
            let empty = caption_height(ctx, "", 7.0, 8.0, 44.5);
            let short = caption_height(ctx, "Controllare il nero", 7.0, 8.0, 44.5);
            let long = caption_height(ctx, &"parola ".repeat(40), 7.0, 8.0, 44.5);
            assert!(short > empty && long > short);
            // Testo più grande, didascalia più alta.
            assert!(caption_height(ctx, "", 10.0, 8.0, 44.5) > empty);
            assert_eq!(folder_note_height(ctx, "  ", 8.0, 190.0), 0.0);
        });
        out.textures_delta.clear();
    }
}
