//! La scena di un foglio (livello L1): l'elenco di ciò che va disegnato, in millimetri con
//! l'origine in alto a sinistra. L'anteprima, il PDF e la stampa disegnano la stessa scena, così
//! ciò che si vede è ciò che si stampa, a capo delle righe compreso.
//!
//! I testi arrivano già spezzati in righe e posizionati: chi costruisce la scena misura i caratteri
//! una volta sola, e chi la disegna non deve rifare i conti.

use std::path::PathBuf;

use winfiles_geom::{Rect, Size};
use winfiles_model::FileKind;

/// Un colore RGB con opacità.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
    pub const WHITE: Color = Color::rgb(255, 255, 255);
}

/// Spessore del carattere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    Regular,
    SemiBold,
}

/// Una riga di testo già posizionata.
#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    /// Inizio della riga, in mm.
    pub x: f64,
    /// Linea di base, in mm.
    pub baseline: f64,
    /// Corpo del carattere (em), in mm.
    pub size: f64,
    pub weight: Weight,
    pub color: Color,
    pub text: String,
    /// Larghezza della riga misurata da chi l'ha impostata, in mm: serve a chi disegna con un
    /// altro motore di testo per restare dentro lo stesso spazio.
    pub width: f64,
}

/// Da dove viene una miniatura: una pagina di un file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Source {
    pub path: PathBuf,
    pub kind: FileKind,
    pub page: usize,
}

/// Una miniatura: la pagina `source` disegnata in `rect`. `crop` è la parte della pagina da
/// mostrare, in frazioni (0..1) della pagina intera: per i PDF con TrimBox solo il formato finito.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub rect: Rect,
    pub source: Source,
    pub crop: Rect,
}

/// Una cosa da disegnare.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// Rettangolo con angoli arrotondati di `radius` mm, pieno e/o bordato (bordo all'interno).
    Rect {
        rect: Rect,
        radius: f64,
        fill: Option<Color>,
        stroke: Option<(Color, f64)>,
    },
    /// Segmento con spessore in mm.
    Line {
        from: (f64, f64),
        to: (f64, f64),
        color: Color,
        width: f64,
    },
    Text(Text),
    Picture(Picture),
}

/// Un foglio da disegnare.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Page {
    pub size: Size,
    pub items: Vec<Item>,
}

impl Page {
    pub fn new(size: Size) -> Self {
        Self {
            size,
            items: Vec::new(),
        }
    }

    pub fn push(&mut self, item: Item) {
        self.items.push(item);
    }

    /// Le miniature del foglio, per preparare i file prima di disegnarlo.
    pub fn pictures(&self) -> impl Iterator<Item = &Picture> {
        self.items.iter().filter_map(|i| match i {
            Item::Picture(p) => Some(p),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_miniature_si_trovano_tra_gli_elementi() {
        let mut page = Page::new(Size::new(210.0, 297.0));
        page.push(Item::Line {
            from: (0.0, 0.0),
            to: (10.0, 0.0),
            color: Color::WHITE,
            width: 0.2,
        });
        page.push(Item::Picture(Picture {
            rect: Rect::new(10.0, 10.0, 50.0, 70.0),
            source: Source {
                path: PathBuf::from("/a.pdf"),
                kind: FileKind::Pdf,
                page: 2,
            },
            crop: Rect::new(0.0, 0.0, 1.0, 1.0),
        }));
        assert_eq!(page.pictures().count(), 1);
        assert_eq!(page.pictures().next().map(|p| p.source.page), Some(2));
    }
}
