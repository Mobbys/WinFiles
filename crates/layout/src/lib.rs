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

pub mod calibration;
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
        View::Nesting => nesting_caption(settings),
        View::List => {
            let size = sheet_size(settings.paper, settings.orientation);
            let area = Rect::new(0.0, 0.0, size.w, size.h).inset(settings.margins);
            (area.w - list_thumb(settings) - settings.gap).max(0.0)
        }
    }
}

fn printable(settings: &PrintSettings) -> Rect {
    let size = sheet_size(settings.paper, settings.orientation);
    Rect::new(0.0, 0.0, size.w, size.h).inset(settings.margins)
}

/// Nel nesting la didascalia è larga 38 mm (o il foglio, se più stretto): le miniature piccole si
/// accostano senza che il nome diventi illeggibile.
fn nesting_caption(settings: &PrintSettings) -> f64 {
    NESTING_CAPTION.min(printable(settings).w)
}

/// Il lato più lungo di una miniatura nel nesting: la larghezza del foglio, ma non oltre metà
/// dell'altezza, così sotto resta posto per intestazione e didascalia. Oltre, il file si riduce.
fn nesting_max(settings: &PrintSettings) -> f64 {
    let area = printable(settings);
    area.w.min(area.h / 2.0)
}

/// Larghezza della didascalia nel nesting, in mm.
const NESTING_CAPTION: f64 = 38.0;
/// Scala più piccola ammessa nel nesting (1:N).
pub const NESTING_MAX_DENOMINATOR: u32 = 10_000;
/// Nella scala automatica il file più grande è largo al più questa parte del foglio.
const NESTING_AUTO_SHARE: f64 = 1.0 / 3.0;
/// Larghezza delle miniature nell'elenco ammessa, in mm.
pub const LIST_THUMB_MM: std::ops::RangeInclusive<f64> = 10.0..=150.0;
/// Lato lungo minimo di una miniatura nel nesting, in mm: sotto, un file piccolissimo accanto a uno
/// enorme non si vedrebbe più. Solo questi escono dalla scala comune.
pub const NESTING_MIN_SIDE: f64 = 6.0;

/// La scala del nesting come 1:N. Scelta dall'utente, oppure automatica: il numero intero più
/// piccolo per cui il file più grande sta in un terzo della larghezza del foglio (mai ingrandito).
pub fn nesting_denominator(settings: &PrintSettings, items: &[Item]) -> u32 {
    if settings.nesting_scale > 0 {
        return settings.nesting_scale.min(NESTING_MAX_DENOMINATOR);
    }
    let longest = items
        .iter()
        .map(|i| i.content.w.max(i.content.h))
        .filter(|v| v.is_finite() && *v > 0.0)
        .fold(0.0, f64::max);
    let room = printable(settings).w * NESTING_AUTO_SHARE;
    if longest <= 0.0 || room <= 0.0 {
        return 1;
    }
    let n = (longest / room).ceil();
    if n.is_finite() && n >= 1.0 {
        // `n` è già tra 1 e il tetto: la conversione non perde nulla.
        n.min(f64::from(NESTING_MAX_DENOMINATOR)) as u32
    } else {
        1
    }
}

/// La scala del nesting: millimetri sul foglio per millimetro vero.
pub fn nesting_scale(settings: &PrintSettings, items: &[Item]) -> f64 {
    1.0 / f64::from(nesting_denominator(settings, items).max(1))
}

/// Larghezza delle miniature nell'elenco: quella scelta, ma mai più di metà riga, così
/// resta sempre posto per il testo.
fn list_thumb(settings: &PrintSettings) -> f64 {
    let area = printable(settings);
    let wanted = if settings.list_thumb.is_finite() {
        settings
            .list_thumb
            .clamp(*LIST_THUMB_MM.start(), *LIST_THUMB_MM.end())
    } else {
        *LIST_THUMB_MM.start()
    };
    wanted.min((area.w - settings.gap) / 2.0)
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

/// Le righe del nesting: ogni file alla stessa scala (`nesting_scale`), accostati da sinistra a
/// destra finché c'è posto, cartella per cartella; ogni riga è centrata e le immagini poggiano sulla
/// didascalia come nella griglia.
fn nesting_rows(settings: &PrintSettings, items: &[Item], area: Rect, max_text: f64) -> Vec<Row> {
    let cell_w = nesting_max(settings);
    let scale = nesting_scale(settings, items);
    let cap_w = nesting_caption(settings);
    // La misura sul foglio di ogni file: in scala, ma mai sotto il minimo né oltre il massimo.
    let sized = |i: &Item| -> Size {
        let s = Size::new(i.content.w * scale, i.content.h * scale);
        let long = s.w.max(s.h);
        let s = if long.is_finite() && long > 0.0 && long < NESTING_MIN_SIDE {
            Size::new(s.w * NESTING_MIN_SIDE / long, s.h * NESTING_MIN_SIDE / long)
        } else {
            s
        };
        // Solo rimpicciolire, per i valori al limite: `fit_within` ingrandirebbe anche.
        let long = s.w.max(s.h);
        if long.is_finite() && long > cell_w {
            Size::new(s.w * cell_w / long, s.h * cell_w / long)
        } else if long.is_finite() && long > 0.0 {
            s
        } else {
            Size::new(cell_w, cell_w)
        }
    };
    let mut rows = Vec::new();
    let mut start = 0usize;
    while start < items.len() {
        let folder = items.get(start).map(|i| i.folder.as_str());
        // Quante celle stanno nella riga: almeno una.
        let mut used = 0.0;
        let mut members: Vec<(&Item, Size, f64)> = Vec::new();
        for item in items.iter().skip(start) {
            if Some(item.folder.as_str()) != folder {
                break;
            }
            let size = sized(item);
            let box_w = size.w.max(cap_w);
            let need = if members.is_empty() {
                box_w
            } else {
                used + settings.gap + box_w
            };
            if !members.is_empty() && need > area.w + EPS {
                break;
            }
            used = need;
            members.push((item, size, box_w));
        }
        let img_h = members
            .iter()
            .map(|(_, s, _)| s.h)
            .fold(0.0, f64::max)
            .clamp(cell_w * 0.05, cell_w);
        let caption_h = members
            .iter()
            .map(|(i, _, _)| height(i.caption, max_text))
            .fold(0.0, f64::max);
        let row_h = img_h + caption_h;
        let mut x = area.x + ((area.w - used) / 2.0).max(0.0);
        let mut cells = Vec::with_capacity(members.len());
        for (item, size, box_w) in &members {
            let (w, h) = (size.w.min(*box_w), size.h.min(img_h));
            cells.push(Placed {
                id: item.id,
                cell: Rect::new(x, 0.0, *box_w, row_h),
                image: Rect::new(x + (box_w - w) / 2.0, img_h - h, w, h),
                caption: Rect::new(x + (box_w - cap_w) / 2.0, img_h, cap_w, caption_h),
            });
            x += box_w + settings.gap;
        }
        rows.push(Row {
            first: start,
            height: row_h,
            cells,
        });
        start += members.len().max(1);
    }
    rows
}

/// Impagina `items` nell'ordine dato, nella vista scelta (griglia, elenco o nesting), con lo spazio per un
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
        View::Nesting => nesting_max(settings),
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
        View::Nesting => nesting_rows(settings, items, area, max_text),
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
