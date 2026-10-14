//! Motore di impaginazione (livello L2). È una funzione pura: impostazioni di stampa + elenco di
//! miniature → fogli con la posizione di ogni elemento in millimetri. Anteprima, PDF e stampa
//! partono tutti da questo risultato, così ciò che si vede è ciò che si stampa.
//!
//! Oggi c'è la vista a griglia: colonne di uguale larghezza, riquadro immagine quadrato, didascalia
//! sotto, intestazione a ogni cambio di cartella (ripetuta in testa al foglio se la cartella
//! continua). Elenco e nesting arriveranno nella Fase 3.

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
    pub rect: Rect,
    /// Vero se la cartella era già iniziata su un foglio precedente.
    pub continued: bool,
}

/// Un foglio impaginato.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Sheet {
    pub size: Size,
    /// Area stampabile (foglio meno margini).
    pub area: Rect,
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
        settings.caption,
        settings.header,
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

/// Impagina `items` nell'ordine dato, in griglia.
pub fn grid(settings: &PrintSettings, items: &[Item]) -> Result<Vec<Sheet>, LayoutError> {
    check(settings)?;
    let size = sheet_size(settings.paper, settings.orientation);
    let area = Rect::new(0.0, 0.0, size.w, size.h).inset(settings.margins);
    let cols = settings.columns as usize;
    let gaps = settings.gap * (cols as f64 - 1.0);
    let cell_w = (area.w - gaps) / cols as f64;
    let cell_h = cell_w + settings.caption;
    // Deve starci almeno un'intestazione più una riga, altrimenti l'impaginazione non termina mai.
    if cell_w <= EPS || settings.header + settings.gap + cell_h > area.h + EPS {
        return Err(LayoutError::NothingFits);
    }

    let new_sheet = || Sheet {
        size,
        area,
        ..Sheet::default()
    };
    let mut sheets = Vec::new();
    let mut sheet = new_sheet();
    let mut y = area.y;
    let mut col = 0usize;
    let mut current: Option<&str> = None;

    for item in items {
        let changed = current != Some(item.folder.as_str());
        if changed {
            if settings.page_break_per_folder && !sheet.is_empty() {
                sheets.push(std::mem::replace(&mut sheet, new_sheet()));
                y = area.y;
            } else if col != 0 {
                y += cell_h + settings.gap;
            }
            col = 0;
            if y + settings.header + settings.gap + cell_h > area.bottom() + EPS {
                sheets.push(std::mem::replace(&mut sheet, new_sheet()));
                y = area.y;
            }
            sheet.headers.push(Header {
                folder: item.folder.clone(),
                rect: Rect::new(area.x, y, area.w, settings.header),
                continued: false,
            });
            y += settings.header + settings.gap;
            current = Some(item.folder.as_str());
        } else if col == cols {
            col = 0;
            y += cell_h + settings.gap;
        }

        if col == 0 && y + cell_h > area.bottom() + EPS {
            sheets.push(std::mem::replace(&mut sheet, new_sheet()));
            sheet.headers.push(Header {
                folder: item.folder.clone(),
                rect: Rect::new(area.x, area.y, area.w, settings.header),
                continued: true,
            });
            y = area.y + settings.header + settings.gap;
        }

        let x = area.x + col as f64 * (cell_w + settings.gap);
        let cell = Rect::new(x, y, cell_w, cell_h);
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
        let caption = Rect::new(x, y + cell_w, cell_w, settings.caption);
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
