//! Il foglio di riepilogo in cima al lavoro: l'elenco testuale di cartelle e file con le misure.
//! Come la griglia, qui si decide solo dove va ogni riga; i testi e le loro altezze li prepara chi
//! chiama.

use winfiles_geom::{Rect, Size, sheet_size};
use winfiles_model::PrintSettings;

use crate::{EPS, LayoutError, check, height};

/// Una riga del riepilogo.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Altezza in mm, note comprese.
    pub height: f64,
    /// Riga di cartella: si ripete in testa al foglio se la cartella continua, e non resta mai
    /// da sola in fondo a un foglio.
    pub group: bool,
}

/// Una riga posizionata.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedLine {
    /// Indice della riga nell'elenco dato.
    pub line: usize,
    pub rect: Rect,
    /// Vero per una riga di cartella ripetuta su un foglio successivo.
    pub continued: bool,
}

/// Un foglio del riepilogo.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SummarySheet {
    pub size: Size,
    pub area: Rect,
    /// Il titolo del lavoro, come sui fogli delle miniature.
    pub title: Option<Rect>,
    /// L'intestazione del riepilogo con i totali: solo sul primo foglio.
    pub heading: Option<Rect>,
    /// I nomi delle colonne, su ogni foglio.
    pub columns: Option<Rect>,
    pub lines: Vec<PlacedLine>,
}

/// Altezze dei blocchi fissi del riepilogo, in mm.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SummaryBlocks {
    /// Titolo in cima a ogni foglio (0 = nessuno).
    pub title: f64,
    /// Intestazione del riepilogo sul primo foglio.
    pub heading: f64,
    /// Nomi delle colonne su ogni foglio.
    pub columns: f64,
}

/// Impagina le righe del riepilogo su uno o più fogli.
pub fn summary(
    settings: &PrintSettings,
    blocks: SummaryBlocks,
    lines: &[Line],
) -> Result<Vec<SummarySheet>, LayoutError> {
    check(settings)?;
    let size = sheet_size(settings.paper, settings.orientation);
    let area = Rect::new(0.0, 0.0, size.w, size.h).inset(settings.margins);
    let gap = settings.gap;
    // I blocchi fissi non possono prendersi tutto il foglio: resta sempre almeno metà area.
    let title = height(blocks.title, area.h / 6.0);
    let heading = height(blocks.heading, area.h / 6.0);
    let columns = height(blocks.columns, area.h / 6.0);
    let fixed = |first: bool| {
        let mut h = columns;
        if title > EPS {
            h += title + gap;
        }
        if first && heading > EPS {
            h += heading;
        }
        h
    };
    let free = area.h - fixed(true);
    if free <= EPS {
        return Err(LayoutError::NothingFits);
    }
    // Una cartella e la sua prima riga stanno sempre insieme su un foglio vuoto.
    let max_group = free / 3.0;
    let max_row = free - max_group;
    let clamp = |l: &Line| height(l.height, if l.group { max_group } else { max_row });

    let new_sheet = |first: bool| {
        let mut sheet = SummarySheet {
            size,
            area,
            ..SummarySheet::default()
        };
        let mut y = area.y;
        if title > EPS {
            sheet.title = Some(Rect::new(area.x, y, area.w, title));
            y += title + gap;
        }
        if first && heading > EPS {
            sheet.heading = Some(Rect::new(area.x, y, area.w, heading));
            y += heading;
        }
        if columns > EPS {
            sheet.columns = Some(Rect::new(area.x, y, area.w, columns));
            y += columns;
        }
        (sheet, y)
    };

    let mut sheets = Vec::new();
    let (mut sheet, mut y) = new_sheet(true);
    let mut group: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let h = clamp(line);
        // Una cartella porta con sé la sua prima riga.
        let need = if line.group {
            h + lines.get(i + 1).filter(|n| !n.group).map_or(0.0, clamp)
        } else {
            h
        };
        if y + need > area.bottom() + EPS && !sheet.lines.is_empty() {
            let (next, top) = new_sheet(false);
            sheets.push(std::mem::replace(&mut sheet, next));
            y = top;
            if !line.group
                && let Some(g) = group.and_then(|g| lines.get(g).map(|l| (g, clamp(l))))
            {
                sheet.lines.push(PlacedLine {
                    line: g.0,
                    rect: Rect::new(area.x, y, area.w, g.1),
                    continued: true,
                });
                y += g.1;
            }
        }
        sheet.lines.push(PlacedLine {
            line: i,
            rect: Rect::new(area.x, y, area.w, h),
            continued: false,
        });
        y += h;
        if line.group {
            group = Some(i);
        }
    }
    sheets.push(sheet);
    Ok(sheets)
}
