//! Cosa va sul foglio per ogni pagina di ogni file, e quanto è alto il suo testo.
//!
//! Il motore di layout decide dove va ogni cosa, ma le altezze dei testi dipendono dai caratteri:
//! le misura questo modulo, in millimetri, con le stesse regole che l'anteprima usa per
//! disegnarli. Così didascalie e note non escono mai dal loro spazio.

use egui::Color32;
use egui::text::{LayoutJob, TextFormat};
use winfiles_geom::{Size, Unit, area_m2, format_area, format_size};
use winfiles_layout::{Item, caption_width};
use winfiles_model::{FileKind, PageInfo, PrintSettings, Project};

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
/// Pixel per millimetro con cui si misurano i testi: gli stessi con cui si compone la scena.
const MEASURE_PX_PER_MM: f64 = crate::sketch::PX_PER_MM as f64;

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

/// Una miniatura per ogni pagina in stampa di ogni file, nell'ordine del progetto.
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
        out.extend(f.printed_pages().map(|page| Slot {
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

/// Il nome del file sotto la miniatura: in neretto, centrato, a capo quando non sta in una riga
/// (tra le parole, o dentro una parola troppo lunga), mai troncato.
pub fn name_job(name: &str, pt: f64, width_mm: f64, px_per_mm: f32) -> LayoutJob {
    let size = (pt * MM_PER_PT) as f32 * px_per_mm;
    let mut job = LayoutJob::single_section(
        name.to_string(),
        TextFormat {
            font_id: crate::theme::semibold(size.max(0.5)),
            color: Color32::from_rgb(20, 20, 20),
            line_height: Some((line_mm(pt) as f32 * px_per_mm).max(0.5)),
            ..Default::default()
        },
    );
    job.wrap.max_width = (width_mm as f32 * px_per_mm).max(1.0);
    job.wrap.break_anywhere = false;
    job.halign = egui::Align::Center;
    job
}

/// Quante righe occupa il nome di un file in una didascalia larga `width_mm` (almeno una).
pub fn name_rows(ctx: &egui::Context, name: &str, pt: f64, width_mm: f64) -> usize {
    if name.is_empty() {
        return 1;
    }
    let job = name_job(name, pt, width_mm, MEASURE_PX_PER_MM as f32);
    ctx.fonts_mut(|f| f.layout_job(job)).rows.len().max(1)
}

/// Il nome da scrivere per un file.
pub fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Spazio tra il riquadro del titolo e il suo testo, in mm.
pub const TITLE_PAD_X: f64 = 3.0;
pub const TITLE_PAD_Y: f64 = 1.8;

/// Il titolo in cima ai fogli, impostato come le note ma in nero e in neretto.
pub fn title_job(text: &str, pt: f64, width_mm: f32, px_per_mm: f32) -> LayoutJob {
    let mut job = note_job(text, pt, f64::from(width_mm), px_per_mm);
    for section in &mut job.sections {
        section.format.font_id = crate::theme::semibold(section.format.font_id.size);
        section.format.color = Color32::from_rgb(20, 20, 20);
    }
    job
}

/// Altezza del riquadro del titolo in cima ai fogli, in mm (0 se non c'è titolo).
pub fn title_height(ctx: &egui::Context, text: &str, pt: f64, width_mm: f64) -> f64 {
    if text.trim().is_empty() {
        return 0.0;
    }
    let inner = (width_mm - 2.0 * TITLE_PAD_X).max(1.0);
    let job = title_job(text, pt, inner as f32, MEASURE_PX_PER_MM as f32);
    let rows = ctx.fonts_mut(|f| f.layout_job(job)).rows.len();
    rows as f64 * line_mm(pt) + 2.0 * TITLE_PAD_Y
}

/// Altezza della riga di intestazione di una cartella con il testo di corpo `pt`, in mm:
/// l'etichetta colorata più un po' d'aria sopra e sotto.
pub fn header_height(pt: f64) -> f64 {
    line_mm(pt) + pt * MM_PER_PT * 0.5 + 1.5
}

/// Quante righe occupa una nota larga `width_mm`.
pub fn note_rows(ctx: &egui::Context, text: &str, pt: f64, width_mm: f64) -> usize {
    if text.trim().is_empty() {
        return 0;
    }
    let job = note_job(text, pt, width_mm, MEASURE_PX_PER_MM as f32);
    ctx.fonts_mut(|f| f.layout_job(job)).rows.len()
}

/// Una riga di misure sotto la miniatura.
#[derive(Clone, Debug, PartialEq)]
pub struct InfoLine {
    pub text: String,
    /// Misura al vivo, scritta in rosso come nella 4.9.
    pub trim: bool,
}

/// Le righe di misure di una pagina: il formato pagina (con abbondanza e crocini) e, se è
/// diverso, il formato al vivo. Le due caselle dell'impaginazione decidono quali si vedono.
pub fn info_lines(page: &PageInfo, unit: Unit, print: &PrintSettings) -> Vec<InfoLine> {
    let mut out = Vec::with_capacity(2);
    if print.show_sizes {
        out.push(InfoLine {
            text: format!(
                "{}  ·  {}",
                format_size(page.size, unit),
                format_area(area_m2(page.size))
            ),
            trim: false,
        });
    }
    if print.show_trim
        && let Some(trim) = page.trim.filter(|t| *t != page.size)
    {
        out.push(InfoLine {
            text: format!(
                "Al vivo: {}  ·  {}",
                format_size(trim, unit),
                format_area(area_m2(trim))
            ),
            trim: true,
        });
    }
    out
}

/// Quante righe di misure ha una pagina: come `info_lines(..).len()`, senza comporre i testi
/// (con migliaia di file si conta a ogni fotogramma).
pub fn info_count(page: &PageInfo, print: &PrintSettings) -> usize {
    usize::from(print.show_sizes) + usize::from(print.show_trim && page.trim.is_some_and(|t| t != page.size))
}

/// L'area di un gruppo di file per le intestazioni: con abbondanza e, tra parentesi, al vivo
/// quando è diversa, come nella 4.9. `None` se le caselle nascondono entrambe le misure.
pub fn area_text(full: f64, trim: f64, show_sizes: bool, show_trim: bool) -> Option<String> {
    let bleed = (full - trim).abs() > 1e-6;
    match (show_sizes, show_trim && bleed) {
        (true, true) => Some(format!("{} (al vivo {})", format_area(full), format_area(trim))),
        (true, false) => Some(format_area(full)),
        (false, true) => Some(format!("al vivo {}", format_area(trim))),
        (false, false) => show_trim.then(|| format_area(trim)),
    }
}

/// Altezza della didascalia (nome intero, `lines` righe di misure e nota), in mm.
pub fn caption_height(
    ctx: &egui::Context,
    name: &str,
    note: &str,
    lines: usize,
    caption_pt: f64,
    note_pt: f64,
    width: f64,
) -> f64 {
    let info = line_mm(caption_pt * INFO_RATIO);
    let names = name_rows(ctx, name, caption_pt, width) as f64;
    let mut h = CAPTION_PAD * 2.0 + names * line_mm(caption_pt) + lines as f64 * info;
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
    let width = caption_width(print);
    let sheet_width = winfiles_geom::sheet_size(print.paper, print.orientation).w
        - print.margins.left
        - print.margins.right;
    // I file sono in ordine di cartella: etichetta e nota si calcolano una volta per cartella.
    let mut current: Option<(&std::path::Path, String, f64)> = None;
    let mut out = Vec::with_capacity(slots.len());
    for (id, slot) in slots.iter().enumerate() {
        let Some(f) = project.files.get(slot.file) else {
            continue;
        };
        let folder = f.folder();
        if current
            .as_ref()
            .is_none_or(|(p, _, _)| p.as_os_str() != folder.as_os_str())
        {
            let label = labels
                .get(folder)
                .cloned()
                .unwrap_or_else(|| folder.display().to_string());
            let note = project.printed_folder_note(folder);
            current = Some((
                folder,
                label,
                folder_note_height(ctx, note, print.note_text, sheet_width),
            ));
        }
        let Some((_, label, folder_note)) = &current else {
            continue;
        };
        let page = f.pages.get(slot.page);
        // Un file non ancora misurato tiene il posto di una riga.
        let lines = page.map_or(usize::from(print.show_sizes || print.show_trim), |p| {
            info_count(p, print)
        });
        out.push(Item {
            id,
            content: page.map_or_else(|| placeholder_size(f.kind), PageInfo::finished),
            folder: label.clone(),
            caption: caption_height(
                ctx,
                &file_name(&f.path),
                project.printed_note(f, slot.page),
                lines,
                print.caption_text,
                print.note_text,
                width,
            ),
            folder_note: *folder_note,
        });
    }
    out
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
    fn le_pagine_escluse_non_vanno_sul_foglio() {
        let mut p = Project::new("prova");
        p.files = vec![file("/a/uno.pdf", 3), file("/a/due.pdf", 1)];
        let colore = slots(&p)[0].color;
        p.files[0].set_printed(1, false);
        p.files[1].set_all_printed(false);
        let s = slots(&p);
        // Restano la prima e la terza pagina, con il loro numero e il colore del file.
        assert_eq!(
            s.iter().map(|s| (s.file, s.page, s.pages)).collect::<Vec<_>>(),
            [(0, 0, 3), (0, 2, 3)]
        );
        assert_eq!(s[1].color, colore);
    }

    #[test]
    fn misure_con_abbondanza_e_al_vivo() {
        let page = PageInfo {
            size: Size::new(216.0, 303.0),
            trim: Some(Size::new(210.0, 297.0)),
        };
        let mut print = PrintSettings::default();
        let lines = info_lines(&page, Unit::Cm, &print);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].text.starts_with("21,6 × 30,3 cm") && !lines[0].trim);
        assert!(lines[1].text.starts_with("Al vivo: 21 × 29,7 cm") && lines[1].trim);
        // Senza TrimBox (o uguale alla pagina) c'è solo la prima riga.
        let plain = PageInfo {
            size: Size::new(210.0, 297.0),
            trim: Some(Size::new(210.0, 297.0)),
        };
        assert_eq!(info_lines(&plain, Unit::Cm, &print).len(), 1);
        // Le caselle tolgono le righe.
        print.show_sizes = false;
        assert_eq!(info_lines(&page, Unit::Cm, &print).len(), 1);
        assert_eq!(info_count(&page, &print), 1);
        print.show_trim = false;
        assert!(info_lines(&page, Unit::Cm, &print).is_empty());

        assert_eq!(
            area_text(0.0654, 0.0624, true, true).as_deref(),
            Some("0,065 m² (al vivo 0,062 m²)")
        );
        assert_eq!(area_text(0.5, 0.5, true, true).as_deref(), Some("0,500 m²"));
        assert_eq!(
            area_text(0.0654, 0.0624, false, true).as_deref(),
            Some("al vivo 0,062 m²")
        );
        assert_eq!(area_text(0.0654, 0.0624, false, false), None);
    }

    #[test]
    fn la_nota_allunga_la_didascalia() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let ctx = ui.ctx();
            let empty = caption_height(ctx, "a.pdf", "", 2, 7.0, 8.0, 44.5);
            let short = caption_height(ctx, "a.pdf", "Controllare il nero", 2, 7.0, 8.0, 44.5);
            let long = caption_height(ctx, "a.pdf", &"parola ".repeat(40), 2, 7.0, 8.0, 44.5);
            assert!(short > empty && long > short);
            // Testo più grande, didascalia più alta; meno righe di misure, più bassa.
            assert!(caption_height(ctx, "a.pdf", "", 2, 10.0, 8.0, 44.5) > empty);
            assert!(caption_height(ctx, "a.pdf", "", 1, 7.0, 8.0, 44.5) < empty);
            // Un nome lungo va a capo, intero: la didascalia cresce di una riga per ogni riga del
            // nome, anche senza spazi.
            let lungo = "Brochure_pieghevole_tre_ante_cliente_Rossi_definitivo_v12_stampa.pdf";
            assert!(name_rows(ctx, lungo, 7.0, 44.5) >= 2);
            assert!(name_rows(ctx, &lungo.replace('_', ""), 7.0, 44.5) >= 2);
            let alta = caption_height(ctx, lungo, "", 2, 7.0, 8.0, 44.5);
            let righe = name_rows(ctx, lungo, 7.0, 44.5) as f64;
            assert!((alta - empty - (righe - 1.0) * line_mm(7.0)).abs() < 1e-9);
            assert_eq!(folder_note_height(ctx, "  ", 8.0, 190.0), 0.0);
        });
        out.textures_delta.clear();
    }
}
