//! Il foglio di riepilogo in cima al lavoro: cartelle e file in una tabella con pagine, formato,
//! misure al vivo, metri quadrati e note, e i totali in testa. Il motore di layout decide dove va
//! ogni riga (`winfiles_layout::summary`); qui si preparano i testi, si misurano e si compone la
//! scena del foglio.

use std::path::PathBuf;

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Align, Color32, Stroke};
use winfiles_geom::{Rect as MmRect, Unit, area_m2, format_area, format_size};
use winfiles_layout::{Header, LayoutError, Line, SummaryBlocks, SummarySheet};
use winfiles_model::{FileEntry, PrintSettings, PrintState, Project, Totals};
use winfiles_scene::{Page, Weight};

use crate::canvas::{FRAME, INK, INK_SOFT, draw_header, draw_title, paper, to_screen, totals_text};
use crate::captions::{MM_PER_PT, NOTE_INK, area_text, line_mm, note_job, note_rows};
use crate::sketch::{self, PX_PER_MM, Sketch};

/// Spazio sopra e sotto il testo di ogni riga di file, in mm.
const ROW_PAD: f64 = 1.1;
/// Spazio prima di ogni cartella, in mm.
const GROUP_SPACE: f64 = 3.0;
/// Rientro del nome del file e delle note, in mm.
const INDENT: f64 = 2.0;
/// Fondo delle righe alterne: si legge la riga anche su un foglio lungo.
const ZEBRA: Color32 = Color32::from_rgb(244, 245, 247);

/// Che cosa c'è su una riga del riepilogo.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Folder(PathBuf),
    /// Un file intero (`page: None`, pagine tutte uguali) o una sua pagina.
    File {
        file: usize,
        page: Option<usize>,
        odd: bool,
    },
}

/// Una riga del riepilogo con le note da scrivere sotto.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub kind: Kind,
    pub notes: Vec<String>,
}

/// Il riepilogo impaginato.
pub struct Summary {
    pub rows: Vec<Row>,
    pub sheets: Vec<SummarySheet>,
}

/// Corpo del testo delle righe: segue il cursore delle intestazioni, mai sotto i 6 pt.
fn body_pt(print: &PrintSettings) -> f64 {
    (print.header_text * 0.95).max(6.0)
}

struct Col {
    title: &'static str,
    x: f64,
    w: f64,
}

/// Le colonne della tabella, in mm dal bordo sinistro dell'area: il nome prende lo spazio che
/// resta. Le caselle delle misure tolgono le loro colonne.
fn columns(print: &PrintSettings, width: f64) -> Vec<Col> {
    let mut right: Vec<(&'static str, f64)> = vec![("Pagine", 14.0)];
    if print.show_sizes {
        right.push(("Formato", 34.0));
        right.push(("m²", 17.0));
    }
    if print.show_trim {
        right.push(("Al vivo", 34.0));
        right.push(("m² al vivo", 19.0));
    }
    // Le colonne crescono con il testo, ma lasciano almeno il 30% al nome.
    let mut k = body_pt(print) / 8.5;
    let sum: f64 = right.iter().map(|c| c.1).sum();
    if sum * k > width * 0.7 {
        k = width * 0.7 / sum;
    }
    let name_w = width - sum * k;
    let mut out = vec![Col {
        title: "File",
        x: 0.0,
        w: name_w,
    }];
    let mut x = name_w;
    for (title, w) in right {
        out.push(Col { title, x, w: w * k });
        x += w * k;
    }
    out
}

fn same_pages(f: &FileEntry) -> bool {
    f.pages
        .first()
        .is_none_or(|first| f.pages.iter().all(|p| p == first))
}

/// I testi delle colonne dopo il nome, nello stesso ordine di [`columns`].
fn cells(f: &FileEntry, page: Option<usize>, print: &PrintSettings, unit: Unit) -> Vec<String> {
    let pages: Vec<_> = match page {
        Some(p) => f.pages.get(p).into_iter().collect(),
        None => f.pages.iter().collect(),
    };
    let mut out = vec![match page {
        Some(p) => format!("pag. {}", p + 1),
        None => f.pages.len().max(1).to_string(),
    }];
    let Some(first) = pages.first() else {
        let none = "—".to_string();
        out.extend(std::iter::repeat_n(
            none,
            2 * usize::from(print.show_sizes) + 2 * usize::from(print.show_trim),
        ));
        return out;
    };
    if print.show_sizes {
        out.push(format_size(first.size, unit));
        out.push(format_area(pages.iter().map(|p| area_m2(p.size)).sum()));
    }
    if print.show_trim {
        out.push(match first.trim.filter(|t| *t != first.size) {
            Some(t) => format_size(t, unit),
            None => "—".into(),
        });
        out.push(format_area(pages.iter().map(|p| area_m2(p.finished())).sum()));
    }
    out
}

/// Le righe del riepilogo, nell'ordine del lavoro.
fn rows(project: &Project) -> Vec<Row> {
    let print_notes = project.print.show_notes;
    let mut out = Vec::new();
    let mut last: Option<&std::path::Path> = None;
    let mut odd = false;
    for (i, f) in project.files.iter().enumerate() {
        // I file tolti dalla stampa non sono nel riepilogo, e nemmeno le cartelle rimaste vuote.
        if f.printed_pages().next().is_none() {
            continue;
        }
        if last != Some(f.folder()) {
            last = Some(f.folder());
            odd = false;
            out.push(Row {
                kind: Kind::Folder(f.folder().to_path_buf()),
                notes: Vec::new(),
            });
        }
        let pages = f.sheet_pages();
        if same_pages(f) && f.print_state() == PrintState::All {
            let notes = f
                .notes
                .iter()
                .filter(|(p, n)| print_notes && **p < pages && !n.trim().is_empty())
                .map(|(p, n)| {
                    if pages > 1 {
                        format!("Pag. {}: {}", p + 1, n.trim())
                    } else {
                        n.trim().to_string()
                    }
                })
                .collect();
            out.push(Row {
                kind: Kind::File {
                    file: i,
                    page: None,
                    odd,
                },
                notes,
            });
        } else {
            // Pagine di formati diversi, o solo alcune in stampa: una riga per pagina.
            for p in f.printed_pages() {
                let note = project.printed_note(f, p).trim();
                out.push(Row {
                    kind: Kind::File {
                        file: i,
                        page: Some(p),
                        odd,
                    },
                    notes: if note.is_empty() {
                        Vec::new()
                    } else {
                        vec![note.to_string()]
                    },
                });
            }
        }
        odd = !odd;
    }
    out
}

/// Prepara e impagina il riepilogo; `title` è l'altezza del titolo in cima ai fogli.
pub fn layout(ctx: &egui::Context, project: &Project, title: f64) -> Result<Summary, LayoutError> {
    let print = &project.print;
    let pt = body_pt(print);
    let width = winfiles_geom::sheet_size(print.paper, print.orientation).w
        - print.margins.left
        - print.margins.right;
    let rows = rows(project);
    let labels = project.folder_labels();
    let name_w = columns(print, width).first().map_or(width, |c| c.w) - 1.5 * INDENT;
    let lines = rows
        .iter()
        .map(|row| match &row.kind {
            Kind::Folder(path) => {
                let note = project.printed_folder_note(path);
                let n = note_rows(ctx, note, print.note_text, width - 2.0 * INDENT);
                let note_h = if n > 0 {
                    n as f64 * line_mm(print.note_text) + 2.0 * ROW_PAD
                } else {
                    0.0
                };
                let label = labels
                    .get(path.as_path())
                    .cloned()
                    .unwrap_or_else(|| path.display().to_string());
                // Anche ripetuta con "(continua)" deve starci.
                let head = crate::canvas::header_block(ctx, project, path, &label, width, false).max(
                    crate::canvas::header_block(ctx, project, path, &label, width, true),
                );
                Line {
                    height: GROUP_SPACE + head + note_h,
                    group: true,
                }
            }
            Kind::File { file, page, .. } => {
                let name = project
                    .files
                    .get(*file)
                    .map(|f| row_name(f, *page))
                    .unwrap_or_default();
                let names = if name.is_empty() {
                    1
                } else {
                    ctx.fonts_mut(|f| f.layout_job(name_cell_job(&name, pt, name_w)))
                        .rows
                        .len()
                        .max(1)
                };
                let notes: usize = row
                    .notes
                    .iter()
                    .map(|n| note_rows(ctx, n, print.note_text, width - 2.0 * INDENT))
                    .sum();
                Line {
                    height: names as f64 * line_mm(pt)
                        + 2.0 * ROW_PAD
                        + notes as f64 * line_mm(print.note_text),
                    group: false,
                }
            }
        })
        .collect::<Vec<_>>();
    let blocks = SummaryBlocks {
        title,
        heading: line_mm(pt * 1.7) + line_mm(pt) + 3.0,
        columns: line_mm(pt * 0.85) + 2.0,
    };
    let sheets = winfiles_layout::summary(print, blocks, &lines)?;
    Ok(Summary { rows, sheets })
}

/// Il nome del file nella prima colonna: intero, a capo quando non sta (`width` in mm).
fn name_cell_job(text: &str, pt: f64, width: f64) -> LayoutJob {
    let mut job = LayoutJob::single_section(
        text.to_string(),
        TextFormat {
            font_id: sketch::font(pt * MM_PER_PT, Weight::SemiBold),
            color: INK,
            line_height: Some(line_mm(pt) as f32 * PX_PER_MM),
            ..Default::default()
        },
    );
    job.wrap.max_width = (width as f32 * PX_PER_MM).max(1.0);
    job
}

/// Il nome da scrivere su una riga del riepilogo: le pagine dopo la prima lo lasciano vuoto.
fn row_name(f: &FileEntry, page: Option<usize>) -> String {
    if page.is_none_or(|p| p == 0) {
        crate::captions::file_name(&f.path)
    } else {
        String::new()
    }
}

/// Una riga che si tronca con "…", allineata a sinistra o a destra (`width` in mm).
fn text_job(text: &str, size: f64, weight: Weight, width: f64, align: Align) -> LayoutJob {
    let mut job = LayoutJob::single_section(
        text.to_string(),
        TextFormat {
            font_id: sketch::font(size, weight),
            color: INK,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping {
        max_width: (width as f32 * PX_PER_MM).max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    job.halign = align;
    job
}

/// La scena di un foglio del riepilogo.
pub fn compose(ctx: &egui::Context, sheet: &SummarySheet, summary: &Summary, project: &Project) -> Page {
    let mut sk = Sketch::new(ctx, sheet.size);
    let print = &project.print;
    let unit = project.unit;
    let pt = body_pt(print);
    let em = |pt: f64| pt * MM_PER_PT;
    let cols = columns(print, sheet.area.w);

    if let Some(title) = sheet.title {
        draw_title(&mut sk, title, print);
    }
    if let Some(h) = sheet.heading {
        let big = em(pt * 1.7);
        let galley = sk.layout_line("Riepilogo del lavoro", big, Weight::SemiBold);
        sk.text((h.x, h.y), &galley, big, Weight::SemiBold, INK, None);
        let all = Totals::of(&project.files);
        let folders = project.folder_totals().values().filter(|t| t.files > 0).count();
        let mut text = format!(
            "{folders} {}  ·  {} file  ·  {} pagine",
            if folders == 1 { "cartella" } else { "cartelle" },
            all.files,
            all.pages
        );
        if let Some(area) = area_text(all.area_full_m2, all.area_m2, print.show_sizes, print.show_trim) {
            text = format!("{text}  ·  {area}");
        }
        let galley = sk.layout_line(&text, em(pt), Weight::Regular);
        sk.text(
            (h.x, h.y + line_mm(pt * 1.7) + 0.5),
            &galley,
            em(pt),
            Weight::Regular,
            INK_SOFT,
            None,
        );
    }
    if let Some(c) = sheet.columns {
        let small = em(pt * 0.85);
        for col in &cols {
            let first = col.x == 0.0;
            let galley = sk.layout(text_job(
                col.title,
                small,
                Weight::SemiBold,
                col.w - INDENT,
                if first { Align::LEFT } else { Align::RIGHT },
            ));
            let x = if first {
                c.x + col.x + INDENT
            } else {
                c.x + col.x + col.w - INDENT * 0.5
            };
            sk.text((x, c.y + 0.6), &galley, small, Weight::SemiBold, INK_SOFT, None);
        }
        sk.line((c.x, c.bottom()), (c.right(), c.bottom()), FRAME, 0.3);
    }

    let labels = project.folder_labels();
    let totals = project.folder_totals();
    let note_em = em(print.note_text);
    for placed in &sheet.lines {
        let Some(row) = summary.rows.get(placed.line) else {
            continue;
        };
        let r = placed.rect;
        match &row.kind {
            Kind::Folder(path) => {
                let label = labels
                    .get(path.as_path())
                    .cloned()
                    .unwrap_or_else(|| path.display().to_string());
                let block = crate::canvas::header_block(ctx, project, path, &label, r.w, placed.continued);
                let head = MmRect::new(r.x, r.y + GROUP_SPACE, r.w, block);
                let header = Header {
                    folder: labels
                        .get(path.as_path())
                        .cloned()
                        .unwrap_or_else(|| path.display().to_string()),
                    rect: r,
                    note: None,
                    continued: placed.continued,
                };
                let custom = project
                    .folder_titles
                    .get(path)
                    .map(|t| t.trim())
                    .filter(|t| !t.is_empty());
                let tot = totals.get(path.as_path()).map(|t| totals_text(t, print));
                draw_header(
                    &mut sk,
                    head,
                    &header,
                    custom,
                    tot.as_deref(),
                    em(print.header_text),
                );
                if !placed.continued
                    && let Some(note) =
                        Some(project.printed_folder_note(path)).filter(|n| !n.trim().is_empty())
                {
                    let galley = sk.layout(note_job(note, print.note_text, r.w - 2.0 * INDENT, PX_PER_MM));
                    sk.text(
                        (r.x + INDENT, head.bottom() + ROW_PAD),
                        &galley,
                        note_em,
                        Weight::Regular,
                        NOTE_INK,
                        Some(r.bottom()),
                    );
                }
            }
            Kind::File { file, page, odd } => {
                let Some(f) = project.files.get(*file) else {
                    continue;
                };
                if *odd {
                    sk.rect(r, 0.0, Some(ZEBRA), None);
                }
                let y = r.y + ROW_PAD;
                let name = row_name(f, *page);
                let mut name_lines = 1;
                let values = cells(f, *page, print, unit);
                for (i, col) in cols.iter().enumerate() {
                    let text = if i == 0 {
                        name.as_str()
                    } else {
                        values.get(i - 1).map_or("", String::as_str)
                    };
                    if text.is_empty() {
                        continue;
                    }
                    let first = i == 0;
                    let trim = col.title == "Al vivo" && text != "—";
                    let color = if trim { NOTE_INK } else { INK };
                    let weight = if first { Weight::SemiBold } else { Weight::Regular };
                    let galley = if first {
                        let g = sk.layout(name_cell_job(text, pt, col.w - 1.5 * INDENT));
                        name_lines = g.rows.len().max(1);
                        g
                    } else {
                        sk.layout(text_job(text, em(pt), weight, col.w - 1.5 * INDENT, Align::RIGHT))
                    };
                    let x = if first {
                        r.x + col.x + INDENT
                    } else {
                        r.x + col.x + col.w - INDENT * 0.5
                    };
                    sk.text((x, y), &galley, em(pt), weight, color, None);
                }
                let mut ny = y + name_lines as f64 * line_mm(pt);
                for note in &row.notes {
                    let galley = sk.layout(note_job(note, print.note_text, r.w - 2.0 * INDENT, PX_PER_MM));
                    sk.text(
                        (r.x + INDENT * 2.0, ny),
                        &galley,
                        note_em,
                        Weight::Regular,
                        NOTE_INK,
                        Some(r.bottom()),
                    );
                    ny += Sketch::size_mm(&galley).1;
                }
            }
        }
    }
    sk.page
}

/// Disegna un foglio del riepilogo dalla sua scena, con sopra i file scelti. Restituisce il file
/// cliccato, se c'è.
pub fn draw(
    ui: &mut egui::Ui,
    sheet: &SummarySheet,
    scene: &Page,
    summary: &Summary,
    thumbs: &mut crate::thumbs::Thumbnails,
    selected: &std::collections::BTreeSet<usize>,
    scale: f32,
) -> Option<usize> {
    let t = crate::theme::Tokens::get(ui.ctx());
    let (response, painter, origin) = paper(ui, sheet.size, scale);
    crate::paint::page(&painter, origin, scale, scene, thumbs);
    let pointer = response.interact_pointer_pos();
    let mut clicked = None;
    for placed in &sheet.lines {
        let Some(Row {
            kind: Kind::File { file, .. },
            ..
        }) = summary.rows.get(placed.line)
        else {
            continue;
        };
        let r = to_screen(origin, placed.rect, scale);
        if selected.contains(file) {
            painter.rect_stroke(r, 0.0, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        }
        if response.clicked() && pointer.is_some_and(|p| r.contains(p)) {
            clicked = Some(*file);
        }
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use winfiles_geom::Size;
    use winfiles_model::{FileKind, PageInfo};

    fn pdf(path: &str, sizes: &[(f64, f64)]) -> FileEntry {
        let mut f = FileEntry::new(PathBuf::from(path), FileKind::Pdf);
        f.pages = sizes
            .iter()
            .map(|(w, h)| PageInfo {
                size: Size::new(*w, *h),
                trim: Some(Size::new(w - 6.0, h - 6.0)),
            })
            .collect();
        f
    }

    #[test]
    fn una_riga_per_file_o_per_pagina_se_i_formati_cambiano() {
        let mut p = Project::new("x");
        let mut uguali = pdf("/a/uguali.pdf", &[(216.0, 303.0), (216.0, 303.0)]);
        uguali.set_note(1, "Seconda pagina più scura".into());
        p.files = vec![
            uguali,
            pdf("/a/misti.pdf", &[(216.0, 303.0), (303.0, 216.0), (426.0, 303.0)]),
            pdf("/b/uno.pdf", &[(216.0, 303.0)]),
        ];
        let r = rows(&p);
        let kinds: Vec<_> = r
            .iter()
            .map(|r| match &r.kind {
                Kind::Folder(_) => "cartella".to_string(),
                Kind::File { page: None, .. } => "file".to_string(),
                Kind::File { page: Some(n), .. } => format!("pag{n}"),
            })
            .collect();
        assert_eq!(
            kinds,
            ["cartella", "file", "pag0", "pag1", "pag2", "cartella", "file"]
        );
        assert_eq!(r[1].notes, ["Pag. 2: Seconda pagina più scura"]);

        let print = PrintSettings::default();
        let c = cells(&p.files[0], None, &print, Unit::Mm);
        assert_eq!(c[0], "2");
        assert_eq!(c[1], "216 × 303 mm");
        assert_eq!(c[3], "210 × 297 mm");
        // Due pagine: le aree sono sommate.
        assert_eq!(c[2], format_area(2.0 * 0.216 * 0.303));
        let mut solo_vivo = print.clone();
        solo_vivo.show_sizes = false;
        assert_eq!(cells(&p.files[0], Some(1), &solo_vivo, Unit::Mm).len(), 3);
        assert_eq!(columns(&solo_vivo, 190.0).len(), 4);
        // Un file non ancora misurato ha comunque tutte le celle.
        let vuoto = FileEntry::new(PathBuf::from("/c/x.jpg"), FileKind::Image);
        assert_eq!(cells(&vuoto, None, &print, Unit::Mm).len(), 5);
    }

    #[test]
    fn il_riepilogo_lascia_fuori_quello_che_non_si_stampa() {
        let mut p = Project::new("x");
        p.files = vec![
            pdf("/a/libro.pdf", &[(216.0, 303.0); 3]),
            pdf("/a/foglio.pdf", &[(216.0, 303.0)]),
            pdf("/b/uno.pdf", &[(216.0, 303.0)]),
        ];
        p.files[0].set_printed(1, false);
        // Un'esclusione di una pagina che il file non ha più (il file è cambiato) non conta.
        p.files[1].skip.insert(4);
        p.files[2].set_all_printed(false);
        let kinds: Vec<_> = rows(&p)
            .iter()
            .map(|r| match &r.kind {
                Kind::Folder(f) => f.display().to_string(),
                Kind::File { page: None, .. } => "file".to_string(),
                Kind::File { page: Some(n), .. } => format!("pag{n}"),
            })
            .collect();
        // Il libro va per pagine, solo quelle in stampa; la cartella /b sparisce.
        assert_eq!(kinds, ["/a", "pag0", "pag2", "file"]);
    }

    #[test]
    fn il_nome_ha_sempre_spazio() {
        let print = PrintSettings {
            header_text: 24.0,
            ..Default::default()
        };
        let cols = columns(&print, 190.0);
        assert!(cols[0].w >= 190.0 * 0.3 - 1e-9);
        let last = cols.last().unwrap();
        assert!((last.x + last.w - 190.0).abs() < 1e-9);
    }
}
