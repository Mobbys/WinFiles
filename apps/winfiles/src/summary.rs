//! Il foglio di riepilogo in cima al lavoro: cartelle e file in una tabella con pagine, formato,
//! misure al vivo, metri quadrati e note, e i totali in testa. Il motore di layout decide dove va
//! ogni riga (`winfiles_layout::summary`); qui si preparano i testi, si misurano e si disegnano.

use std::path::PathBuf;

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Align, Color32, FontId, Pos2, Rect, Stroke, Vec2, pos2};
use winfiles_geom::{Unit, area_m2, format_area, format_size};
use winfiles_layout::{Header, LayoutError, Line, SummaryBlocks, SummarySheet};
use winfiles_model::{FileEntry, PrintSettings, Project, Totals};

use crate::canvas::{FRAME, INK, INK_SOFT, draw_header, draw_title, paper, to_screen, totals_text};
use crate::captions::{MM_PER_PT, NOTE_INK, area_text, header_height, line_mm, note_rows};
use crate::theme::semibold;

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
    let mut out = Vec::new();
    let mut last: Option<&std::path::Path> = None;
    let mut odd = false;
    for (i, f) in project.files.iter().enumerate() {
        if last != Some(f.folder()) {
            last = Some(f.folder());
            odd = false;
            out.push(Row {
                kind: Kind::Folder(f.folder().to_path_buf()),
                notes: Vec::new(),
            });
        }
        let pages = f.pages.len().max(1);
        if same_pages(f) {
            let notes = f
                .notes
                .iter()
                .filter(|(p, n)| **p < pages && !n.trim().is_empty())
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
            // Pagine di formati diversi: una riga per pagina.
            for p in 0..pages {
                let note = f.note(p).trim();
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
    let lines = rows
        .iter()
        .map(|row| match &row.kind {
            Kind::Folder(path) => {
                let note = project.folder_notes.get(path).map_or("", String::as_str);
                let n = note_rows(ctx, note, print.note_text, width - 2.0 * INDENT);
                let note_h = if n > 0 {
                    n as f64 * line_mm(print.note_text) + 2.0 * ROW_PAD
                } else {
                    0.0
                };
                Line {
                    height: GROUP_SPACE + header_height(print.header_text) + note_h,
                    group: true,
                }
            }
            Kind::File { .. } => {
                let notes: usize = row
                    .notes
                    .iter()
                    .map(|n| note_rows(ctx, n, print.note_text, width - 2.0 * INDENT))
                    .sum();
                Line {
                    height: line_mm(pt) + 2.0 * ROW_PAD + notes as f64 * line_mm(print.note_text),
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

fn text_job(text: &str, font: FontId, color: Color32, width: f32, align: Align) -> LayoutJob {
    let mut job = LayoutJob::single_section(
        text.to_string(),
        TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping {
        max_width: width.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    job.halign = align;
    job
}

/// Disegna un foglio del riepilogo. Restituisce il file cliccato, se c'è.
pub fn draw(
    ui: &mut egui::Ui,
    sheet: &SummarySheet,
    summary: &Summary,
    project: &Project,
    selected: &std::collections::BTreeSet<usize>,
    scale: f32,
) -> Option<usize> {
    let t = crate::theme::Tokens::get(ui.ctx());
    let print = &project.print;
    let unit = project.unit;
    let pt = body_pt(print);
    let (response, painter, origin) = paper(ui, sheet.size, scale);
    let map = |r: winfiles_geom::Rect| to_screen(origin, r, scale);
    let mm = |v: f64| v as f32 * scale;
    let font = |pt: f64| (pt * MM_PER_PT) as f32 * scale;
    let readable = |pt: f64| font(pt) >= 3.5;
    let cols = columns(print, sheet.area.w);

    if let Some(title) = sheet.title {
        draw_title(&painter, map(title), title.w, print, scale);
    }
    if let Some(h) = sheet.heading {
        let r = map(h);
        if readable(pt) {
            painter.text(
                r.min,
                egui::Align2::LEFT_TOP,
                "Riepilogo del lavoro",
                semibold(font(pt * 1.7)),
                INK,
            );
            let all = Totals::of(&project.files);
            let folders = project.folder_totals().len();
            let mut text = format!(
                "{folders} {}  ·  {} file  ·  {} pagine",
                if folders == 1 { "cartella" } else { "cartelle" },
                all.files,
                all.pages
            );
            if let Some(area) = area_text(all.area_full_m2, all.area_m2, print.show_sizes, print.show_trim) {
                text = format!("{text}  ·  {area}");
            }
            painter.text(
                r.min + Vec2::new(0.0, mm(line_mm(pt * 1.7) + 0.5)),
                egui::Align2::LEFT_TOP,
                text,
                FontId::proportional(font(pt)),
                INK_SOFT,
            );
        }
    }
    if let Some(c) = sheet.columns {
        let r = map(c);
        if readable(pt * 0.85) {
            for col in &cols {
                let cell = Rect::from_min_size(
                    pos2(r.left() + mm(col.x), r.top()),
                    Vec2::new(mm(col.w), r.height()),
                );
                let first = col.x == 0.0;
                let job = text_job(
                    col.title,
                    semibold(font(pt * 0.85)),
                    INK_SOFT,
                    cell.width() - mm(INDENT),
                    if first { Align::LEFT } else { Align::RIGHT },
                );
                let galley = painter.layout_job(job);
                let x = if first {
                    cell.left() + mm(INDENT)
                } else {
                    cell.right() - mm(INDENT) * 0.5
                };
                painter.galley(pos2(x, cell.top() + mm(0.6)), galley, INK_SOFT);
            }
        }
        painter.line_segment(
            [r.left_bottom(), r.right_bottom()],
            Stroke::new(mm(0.3).max(1.0), FRAME),
        );
    }

    let labels = project.folder_labels();
    let totals = project.folder_totals();
    let pointer = response.interact_pointer_pos();
    let mut clicked = None;
    for placed in &sheet.lines {
        let Some(row) = summary.rows.get(placed.line) else {
            continue;
        };
        let r = map(placed.rect);
        match &row.kind {
            Kind::Folder(path) => {
                let head = Rect::from_min_size(
                    r.min + Vec2::new(0.0, mm(GROUP_SPACE)),
                    Vec2::new(r.width(), mm(header_height(print.header_text))),
                );
                let header = Header {
                    folder: labels
                        .get(path.as_path())
                        .cloned()
                        .unwrap_or_else(|| path.display().to_string()),
                    rect: placed.rect,
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
                    &painter,
                    head,
                    &header,
                    custom,
                    tot.as_deref(),
                    font(print.header_text),
                    scale,
                );
                if !placed.continued
                    && readable(print.note_text)
                    && let Some(note) = project.folder_notes.get(path).filter(|n| !n.trim().is_empty())
                {
                    let job =
                        crate::captions::note_job(note, print.note_text, placed.rect.w - 2.0 * INDENT, scale);
                    let galley = painter.layout_job(job);
                    painter.with_clip_rect(r).galley(
                        pos2(r.left() + mm(INDENT), head.bottom() + mm(ROW_PAD)),
                        galley,
                        NOTE_INK,
                    );
                }
            }
            Kind::File { file, page, odd } => {
                let Some(f) = project.files.get(*file) else {
                    continue;
                };
                if *odd {
                    painter.rect_filled(r, 0.0, ZEBRA);
                }
                if selected.contains(file) {
                    painter.rect_stroke(r, 0.0, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
                }
                let y = r.top() + mm(ROW_PAD);
                let line_h = mm(line_mm(pt));
                if readable(pt) {
                    let name = if page.is_none_or(|p| p == 0) {
                        f.path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    let values = cells(f, *page, print, unit);
                    for (i, col) in cols.iter().enumerate() {
                        let text = if i == 0 {
                            name.as_str()
                        } else {
                            values.get(i - 1).map_or("", String::as_str)
                        };
                        let first = i == 0;
                        let trim = col.title == "Al vivo" && text != "—";
                        let color = if trim { NOTE_INK } else { INK };
                        let job = text_job(
                            text,
                            if first {
                                semibold(font(pt))
                            } else {
                                FontId::proportional(font(pt))
                            },
                            color,
                            mm(col.w - 1.5 * INDENT),
                            if first { Align::LEFT } else { Align::RIGHT },
                        );
                        let galley = painter.layout_job(job);
                        let x = if first {
                            r.left() + mm(col.x + INDENT)
                        } else {
                            r.left() + mm(col.x + col.w) - mm(INDENT) * 0.5
                        };
                        painter.galley(Pos2::new(x, y), galley, color);
                    }
                }
                if readable(print.note_text) {
                    let mut ny = y + line_h;
                    for note in &row.notes {
                        let job = crate::captions::note_job(
                            note,
                            print.note_text,
                            placed.rect.w - 2.0 * INDENT,
                            scale,
                        );
                        let galley = painter.layout_job(job);
                        let h = galley.size().y;
                        painter.with_clip_rect(r).galley(
                            pos2(r.left() + mm(INDENT * 2.0), ny),
                            galley,
                            NOTE_INK,
                        );
                        ny += h;
                    }
                }
                if response.clicked() && pointer.is_some_and(|p| r.contains(p)) {
                    clicked = Some(*file);
                }
            }
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
