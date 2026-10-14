//! Motore di impaginazione (livello L2). È una funzione pura: impostazioni di stampa + elenco di
//! miniature → fogli con la posizione di ogni elemento in millimetri. Anteprima, PDF e stampa
//! partono tutti da questo risultato, così ciò che si vede è ciò che si stampa.
//!
//! Oggi c'è la vista a griglia: colonne di uguale larghezza, riquadro immagine quadrato, didascalia
//! sotto, intestazione a ogni cambio di cartella (ripetuta in testa al foglio se la cartella
//! continua) con la nota della cartella. Ogni riga è alta quanto la didascalia più alta che
//! contiene. Elenco e nesting arriveranno nella Fase 3.
//!
//! Le altezze dei testi (didascalia e note) le misura chi chiama, perché dipendono dai caratteri:
//! qui arrivano già in millimetri.

use winfiles_geom::{Rect, Size, sheet_size};
use winfiles_model::PrintSettings;

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

/// Impagina `items` nell'ordine dato, in griglia.
pub fn grid(settings: &PrintSettings, items: &[Item]) -> Result<Vec<Sheet>, LayoutError> {
    grid_with_title(settings, items, 0.0)
}

/// Come [`grid`], con un titolo alto `title` mm in cima a ogni foglio (0 = nessun titolo).
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
        let top = title + settings.gap;
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
    let cols = settings.columns as usize;
    let cell_w = cell_width(settings);
    // Deve starci almeno un'intestazione più una riga, altrimenti l'impaginazione non termina mai.
    if cell_w <= EPS || settings.header + settings.gap + cell_w > area.h + EPS {
        return Err(LayoutError::NothingFits);
    }
    // Lo spazio massimo per i testi sotto le immagini e sotto l'intestazione: oltre, il testo
    // viene tagliato, ma la riga sta sempre in un foglio.
    let max_text = area.h - settings.header - settings.gap - cell_w;
    // La didascalia della riga che inizia da `start`: la più alta tra le sue celle.
    let row_caption = |start: usize| {
        let folder = items.get(start).map(|i| i.folder.as_str());
        items
            .iter()
            .skip(start)
            .take(cols)
            .take_while(|i| Some(i.folder.as_str()) == folder)
            .map(|i| height(i.caption, max_text))
            .fold(0.0, f64::max)
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
    let mut col = 0usize;
    let mut row_h = 0.0;
    let mut current: Option<&str> = None;

    for (index, item) in items.iter().enumerate() {
        let changed = current != Some(item.folder.as_str());
        if changed {
            if settings.page_break_per_folder && !sheet.is_empty() {
                sheets.push(std::mem::replace(&mut sheet, new_sheet()));
                y = area.y;
            } else if col != 0 {
                y += row_h + settings.gap;
            }
            col = 0;
            row_h = cell_w + row_caption(index);
            let note = height(item.folder_note, max_text - (row_h - cell_w));
            let block = settings.header + note + settings.gap;
            if y + block + row_h > area.bottom() + EPS {
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
        } else if col == cols {
            col = 0;
            y += row_h + settings.gap;
            row_h = cell_w + row_caption(index);
        }

        if col == 0 && y + row_h > area.bottom() + EPS {
            sheets.push(std::mem::replace(&mut sheet, new_sheet()));
            sheet.headers.push(Header {
                folder: item.folder.clone(),
                rect: Rect::new(area.x, area.y, area.w, settings.header),
                note: None,
                continued: true,
            });
            y = area.y + settings.header + settings.gap;
        }

        let x = area.x + col as f64 * (cell_w + settings.gap);
        let cell = Rect::new(x, y, cell_w, row_h);
        let image_box = Rect::new(x, y, cell_w, cell_w);
        let fitted = item
            .content
            .fit_within(image_box.size())
            .unwrap_or(image_box.size());
        let image = Rect::new(
            image_box.x + (image_box.w - fitted.w) / 2.0,
            image_box.y + (image_box.h - fitted.h) / 2.0,
            fitted.w,
            fitted.h,
        );
        let caption = Rect::new(x, y + cell_w, cell_w, row_h - cell_w);
        sheet.items.push(Placed {
            id: item.id,
            cell,
            image,
            caption,
        });
        col += 1;
    }

    if !sheet.is_empty() || sheets.is_empty() {
        sheets.push(sheet);
    }
    Ok(sheets)
}

#[cfg(test)]
mod tests;
