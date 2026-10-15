//! Motore di impaginazione (livello L2). È una funzione pura: impostazioni di stampa + elenco di
//! miniature → fogli con la posizione di ogni elemento in millimetri. Anteprima, PDF e stampa
//! partono tutti da questo risultato, così ciò che si vede è ciò che si stampa.
//!
//! Oggi c'è la vista a griglia: colonne di uguale larghezza, riquadro immagine quadrato, didascalia
//! sotto, intestazione a ogni cambio di cartella (ripetuta in testa al foglio se la cartella
//! continua) con la nota della cartella. Ogni riga è alta quanto la didascalia più alta che
//! contiene. Nella vista a elenco ogni miniatura ha la sua riga, con la didascalia a destra.
//!
//! Le altezze dei testi (didascalia e note) le misura chi chiama, perché dipendono dai caratteri:
//! qui arrivano già in millimetri.

use winfiles_geom::{Rect, Size, sheet_size};
use winfiles_model::{PrintSettings, View};

mod summary;
pub use summary::{Line, PlacedLine, SummaryBlocks, SummarySheet, summary};

/// Colonne massime ammesse.
pub const MAX_COLUMNS: u32 = 20;
/// Tolleranza nei confronti tra misure, in mm.
pub const EPS: f64 = 1e-6;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LayoutError {
    #[error("impostazione non valida: {0}")]
    InvalidSettings(&'static str),
    #[error("con questi margini e queste misure non sta nemmeno una riga di miniature sul foglio")]
    NothingFits,
}

/// Un elemento da impaginare: una pagina di un file.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Identificativo scelto dal chiamante (per esempio l'indice nel progetto).
    pub id: usize,
    /// Dimensioni del contenuto, usate solo per le proporzioni.
    pub content: Size,
    /// Cartella di appartenenza: un cambio di cartella apre una nuova intestazione.
    pub folder: String,
    /// Altezza della didascalia sotto l'immagine (nome, misure, nota del file), in mm.
    pub caption: f64,
    /// Altezza della nota della cartella sotto l'intestazione, in mm; conta quella del primo
    /// elemento della cartella.
    pub folder_note: f64,
}

/// Una miniatura posizionata.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub id: usize,
    /// Cella intera (immagine + didascalia).
    pub cell: Rect,
    /// Dove disegnare il contenuto, con le sue proporzioni, centrato nel riquadro immagine.
    pub image: Rect,
    /// Spazio per la didascalia.
    pub caption: Rect,
}

/// Un'intestazione di cartella.
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub folder: String,
    /// Tutta l'intestazione, nota della cartella compresa.
    pub rect: Rect,
    /// Dove scrivere la nota della cartella, sotto il titolo; assente se non c'è nota.
    pub note: Option<Rect>,
    /// Vero se la cartella era già iniziata su un foglio precedente.
    pub continued: bool,
}

/// Un foglio impaginato.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Sheet {
    pub size: Size,
    /// Area stampabile (foglio meno margini).
    pub area: Rect,
    /// Dove scrivere il titolo in cima al foglio, se c'è.
    pub title: Option<Rect>,
    pub headers: Vec<Header>,
    pub items: Vec<Placed>,
}

impl Sheet {
    fn is_empty(&self) -> bool {
        self.headers.is_empty() && self.items.is_empty()
    }
}

fn check(settings: &PrintSettings) -> Result<(), LayoutError> {
    let m = settings.margins;
    let numbers = [
        m.top,
        m.right,
        m.bottom,
        m.left,
        settings.gap,
        settings.row_gap,
        settings.header,
        settings.caption_text,
        settings.note_text,
    ];
    if numbers.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(LayoutError::InvalidSettings(
            "margini e spazi devono essere numeri positivi",
        ));
    }
    if settings.columns == 0 || settings.columns > MAX_COLUMNS {
        return Err(LayoutError::InvalidSettings(
            "le colonne devono essere tra 1 e 20",
        ));
    }
    Ok(())
}

/// Larghezza delle celle della griglia, in mm: serve a chi misura i testi delle didascalie.
pub fn cell_width(settings: &PrintSettings) -> f64 {
    let size = sheet_size(settings.paper, settings.orientation);
    let area = Rect::new(0.0, 0.0, size.w, size.h).inset(settings.margins);
    let cols = f64::from(settings.columns.max(1));
    (area.w - settings.gap * (cols - 1.0)) / cols
}

/// Un'altezza arrivata da fuori: negativa o non valida vale zero, e non supera `max`.
fn height(v: f64, max: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, max.max(0.0))
    } else {
        0.0
    }
}

/// Larghezza dello spazio per la didascalia, in mm: serve a chi misura i testi. Nella griglia è
/// la colonna; nell'elenco è ciò che resta a destra della miniatura.
pub fn caption_width(settings: &PrintSettings) -> f64 {
    match settings.view {
        View::Grid => cell_width(settings),
        View::List => {
            let size = sheet_size(settings.paper, settings.orientation);
            let area = Rect::new(0.0, 0.0, size.w, size.h).inset(settings.margins);
            (area.w - list_thumb(settings) - settings.gap).max(0.0)
        }
    }
}

/// Larghezza delle miniature nell'elenco: una colonna della griglia, ma mai più di metà riga, così
/// resta sempre posto per il testo.
fn list_thumb(settings: &PrintSettings) -> f64 {
    let size = sheet_size(settings.paper, settings.orientation);
    let area = Rect::new(0.0, 0.0, size.w, size.h).inset(settings.margins);
    cell_width(settings).min((area.w - settings.gap) / 2.0)
}

/// Impagina `items` nell'ordine dato, in griglia.
pub fn grid(settings: &PrintSettings, items: &[Item]) -> Result<Vec<Sheet>, LayoutError> {
    grid_with_title(settings, items, 0.0)
}

/// Una riga di miniature, con le celle già al loro posto in orizzontale e in alto a zero.
struct Row {
    /// Indice in `items` del primo elemento della riga.
    first: usize,
    height: f64,
    cells: Vec<Placed>,
}

/// Le righe della griglia: fino a `columns` celle della stessa cartella. Ogni riga è alta quanto
/// la sua immagine più alta più la sua didascalia più alta, e le immagini poggiano sulla
/// didascalia, così una riga di soli formati orizzontali non lascia un vuoto.
fn grid_rows(settings: &PrintSettings, items: &[Item], area: Rect, max_text: f64) -> Vec<Row> {
    let cols = settings.columns.max(1) as usize;
    let cell_w = cell_width(settings);
    let mut rows = Vec::new();
    let mut start = 0usize;
    while start < items.len() {
        let folder = items.get(start).map(|i| i.folder.as_str());
        let members: Vec<&Item> = items
            .iter()
            .skip(start)
            .take(cols)
            .take_while(|i| Some(i.folder.as_str()) == folder)
            .collect();
        let img_h = members
            .iter()
            .map(|i| {
                i.content
                    .fit_within(Size::new(cell_w, cell_w))
                    .map_or(cell_w, |s| s.h)
            })
            .fold(0.0, f64::max)
            .clamp(cell_w * 0.05, cell_w);
        let caption_h = members
            .iter()
            .map(|i| height(i.caption, max_text))
            .fold(0.0, f64::max);
        let row_h = img_h + caption_h;
        let cells = members
            .iter()
            .enumerate()
            .map(|(col, item)| {
                let x = area.x + col as f64 * (cell_w + settings.gap);
                let fitted = item
                    .content
                    .fit_within(Size::new(cell_w, img_h))
                    .unwrap_or(Size::new(cell_w, img_h));
                Placed {
                    id: item.id,
                    cell: Rect::new(x, 0.0, cell_w, row_h),
                    image: Rect::new(
                        x + (cell_w - fitted.w) / 2.0,
                        img_h - fitted.h,
                        fitted.w,
                        fitted.h,
                    ),
                    caption: Rect::new(x, img_h, cell_w, caption_h),
                }
            })
            .collect();
        rows.push(Row {
            first: start,
            height: row_h,
            cells,
        });
        start += members.len().max(1);
    }
    rows
}

/// Le righe dell'elenco: una miniatura per riga, larga quanto una colonna della griglia e in alto
/// a sinistra (al più metà riga), con la didascalia a destra. La riga è alta quanto la più alta delle due.
fn list_rows(settings: &PrintSettings, items: &[Item], area: Rect, max_text: f64) -> Vec<Row> {
    let cell_w = list_thumb(settings);
    let text_w = caption_width(settings);
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let fitted = item
                .content
                .fit_within(Size::new(cell_w, cell_w))
                .unwrap_or(Size::new(cell_w, cell_w));
            let img_h = fitted.h.clamp(cell_w * 0.05, cell_w);
            let caption_h = height(item.caption, max_text);
            let row_h = img_h.max(caption_h);
            let text_x = area.x + cell_w + settings.gap;
            Row {
                first: index,
                height: row_h,
                cells: vec![Placed {
                    id: item.id,
                    cell: Rect::new(area.x, 0.0, area.w, row_h),
                    image: Rect::new(
                        area.x + (cell_w - fitted.w) / 2.0,
                        0.0,
                        fitted.w,
                        fitted.h.min(img_h),
                    ),
                    caption: Rect::new(text_x, 0.0, text_w, caption_h),
                }],
            }
        })
        .collect()
}

/// Impagina `items` nell'ordine dato, nella vista scelta (griglia o elenco), con lo spazio per un
/// titolo alto `title` mm in cima a ogni foglio.
pub fn grid_with_title(
    settings: &PrintSettings,
    items: &[Item],
    title: f64,
) -> Result<Vec<Sheet>, LayoutError> {
    check(settings)?;
    let size = sheet_size(settings.paper, settings.orientation);
    let printable = Rect::new(0.0, 0.0, size.w, size.h).inset(settings.margins);
    let title = height(title, printable.h);
    // Il titolo toglie spazio in cima; le miniature stanno nell'area che resta.
    let (area, title_rect) = if title > EPS {
        let top = title + settings.row_gap;
        (
            Rect::new(
                printable.x,
                printable.y + top,
                printable.w,
                (printable.h - top).max(0.0),
            ),
            Some(Rect::new(printable.x, printable.y, printable.w, title)),
        )
    } else {
        (printable, None)
    };
    let cell_w = match settings.view {
        View::Grid => cell_width(settings),
        View::List => list_thumb(settings),
    };
    // Deve starci almeno un'intestazione più una riga, altrimenti l'impaginazione non termina mai.
    if cell_w <= EPS || settings.header + settings.row_gap + cell_w > area.h + EPS {
        return Err(LayoutError::NothingFits);
    }
    // Lo spazio massimo per i testi sotto le immagini e sotto l'intestazione: oltre, il testo
    // viene tagliato, ma la riga sta sempre in un foglio.
    let max_text = area.h - settings.header - settings.row_gap - cell_w;
    let rows = match settings.view {
        View::Grid => grid_rows(settings, items, area, max_text),
        View::List => list_rows(settings, items, area, max_text),
    };

    let new_sheet = || Sheet {
        size,
        area: printable,
        title: title_rect,
        ..Sheet::default()
    };
    let mut sheets = Vec::new();
    let mut sheet = new_sheet();
    let mut y = area.y;
    let mut current: Option<&str> = None;
    // C'è già una riga sopra, nello stesso foglio: la prossima va sotto, staccata di `row_gap`.
    let mut after_row = false;

    for row in &rows {
        let Some(item) = items.get(row.first) else {
            continue;
        };
        if current != Some(item.folder.as_str()) {
            if settings.page_break_per_folder && !sheet.is_empty() {
                sheets.push(std::mem::replace(&mut sheet, new_sheet()));
                y = area.y;
            } else if after_row {
                y += settings.row_gap;
            }
            // La nota della cartella non può spingere la sua prima riga fuori dal foglio.
            let note = height(
                item.folder_note,
                area.h - settings.header - settings.row_gap - row.height,
            );
            let block = settings.header + note + settings.row_gap;
            if y + block + row.height > area.bottom() + EPS {
                sheets.push(std::mem::replace(&mut sheet, new_sheet()));
                y = area.y;
            }
            sheet.headers.push(Header {
                folder: item.folder.clone(),
                rect: Rect::new(area.x, y, area.w, settings.header + note),
                note: (note > EPS).then(|| Rect::new(area.x, y + settings.header, area.w, note)),
                continued: false,
            });
            y += block;
            current = Some(item.folder.as_str());
        } else if after_row {
            y += settings.row_gap;
        }

        if y + row.height > area.bottom() + EPS {
            sheets.push(std::mem::replace(&mut sheet, new_sheet()));
            sheet.headers.push(Header {
                folder: item.folder.clone(),
                rect: Rect::new(area.x, area.y, area.w, settings.header),
                note: None,
                continued: true,
            });
            y = area.y + settings.header + settings.row_gap;
        }

        for cell in &row.cells {
            let down = |r: Rect| Rect::new(r.x, r.y + y, r.w, r.h);
            sheet.items.push(Placed {
                id: cell.id,
                cell: down(cell.cell),
                image: down(cell.image),
                caption: down(cell.caption),
            });
        }
        y += row.height;
        after_row = true;
    }

    if !sheet.is_empty() || sheets.is_empty() {
        sheets.push(sheet);
    }
    Ok(sheets)
}

#[cfg(test)]
mod tests;
