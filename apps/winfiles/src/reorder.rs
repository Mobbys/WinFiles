//! Cambiare l'ordine trascinando nell'elenco dei file: i file si spostano dentro la loro
//! cartella, le cartelle si spostano intere. Così ogni cartella resta un blocco solo, con una sola
//! intestazione sul foglio.
//!
//! Questo modulo decide soltanto dove cadrebbe ciò che si trascina; lo spostamento vero lo fa
//! `Project::move_files`.

use std::collections::BTreeSet;

/// Che cosa si sta trascinando.
#[derive(Clone, Debug, PartialEq)]
pub enum Drag {
    /// Alcuni file della stessa cartella (indici nel lavoro).
    Files { group: usize, files: BTreeSet<usize> },
    /// Una cartella intera, con l'indice del suo gruppo nell'elenco.
    Group(usize),
}

/// Dove cadrebbe ciò che si trascina.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drop {
    /// I file finiscono subito prima di questo file (o in fondo, se è la lunghezza del lavoro).
    pub before: usize,
    /// La riga dell'elenco su cui si disegna la linea di arrivo.
    pub row: usize,
    /// Vero se la linea va sopra la riga, falso se sotto.
    pub top: bool,
}

/// Le righe dell'elenco: per ogni gruppo l'intestazione e poi i suoi file, indici consecutivi
/// del lavoro.
pub struct Rows<'a> {
    pub groups: &'a [Vec<usize>],
}

impl Rows<'_> {
    /// Il gruppo e la posizione del file (None per l'intestazione) di una riga.
    pub fn locate(&self, row: usize) -> Option<(usize, Option<usize>)> {
        let mut first = 0;
        for (g, files) in self.groups.iter().enumerate() {
            let end = first + 1 + files.len();
            if row < end {
                return Some((g, (row > first).then(|| row - first - 1)));
            }
            first = end;
        }
        None
    }

    /// La riga dell'intestazione di un gruppo.
    fn header(&self, group: usize) -> usize {
        self.groups.iter().take(group).map(|files| 1 + files.len()).sum()
    }

    /// Dove cade `drag` con il puntatore sulla riga `row`, nella metà alta (`upper`) o bassa.
    /// None se lì non può andare.
    pub fn drop_at(&self, drag: &Drag, row: usize, upper: bool) -> Option<Drop> {
        let (g, pos) = self.locate(row)?;
        let files = self.groups.get(g)?;
        let first = *files.first()?;
        let last = *files.last()?;
        let header = self.header(g);
        match drag {
            Drag::Files { group, .. } if *group == g => Some(match pos {
                // Sull'intestazione: in cima alla cartella.
                None => Drop {
                    before: first,
                    row: header,
                    top: false,
                },
                Some(k) if upper => Drop {
                    before: *files.get(k)?,
                    row,
                    top: true,
                },
                Some(k) => Drop {
                    before: files.get(k + 1).copied().unwrap_or(last + 1),
                    row,
                    top: false,
                },
            }),
            Drag::Files { .. } => None,
            Drag::Group(moving) if *moving == g => None,
            Drag::Group(_) if pos.is_none() && upper => Some(Drop {
                before: first,
                row: header,
                top: true,
            }),
            Drag::Group(_) => Some(Drop {
                before: last + 1,
                row: header + files.len(),
                top: false,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Due cartelle: la prima con i file 0, 1, 2 (righe 1-3), la seconda con 3 e 4 (righe 5-6).
    fn groups() -> Vec<Vec<usize>> {
        vec![vec![0, 1, 2], vec![3, 4]]
    }

    #[test]
    fn righe_e_gruppi() {
        let g = groups();
        let rows = Rows { groups: &g };
        assert_eq!(rows.locate(0), Some((0, None)));
        assert_eq!(rows.locate(3), Some((0, Some(2))));
        assert_eq!(rows.locate(4), Some((1, None)));
        assert_eq!(rows.locate(6), Some((1, Some(1))));
        assert_eq!(rows.locate(7), None);
    }

    #[test]
    fn i_file_restano_nella_loro_cartella() {
        let g = groups();
        let rows = Rows { groups: &g };
        let drag = Drag::Files {
            group: 0,
            files: BTreeSet::from([2]),
        };
        // Metà alta del file 1: prima di lui, linea sopra la sua riga.
        let d = rows.drop_at(&drag, 2, true).unwrap();
        assert_eq!((d.before, d.row, d.top), (1, 2, true));
        // Metà bassa dell'ultimo: in fondo alla cartella, cioè prima del primo file della seconda.
        let d = rows.drop_at(&drag, 3, false).unwrap();
        assert_eq!((d.before, d.row, d.top), (3, 3, false));
        // Sull'intestazione: in cima.
        assert_eq!(rows.drop_at(&drag, 0, true).unwrap().before, 0);
        // In un'altra cartella no.
        assert_eq!(rows.drop_at(&drag, 5, true), None);
        assert_eq!(rows.drop_at(&drag, 4, false), None);
    }

    #[test]
    fn le_cartelle_si_spostano_intere() {
        let g = groups();
        let rows = Rows { groups: &g };
        let seconda = Drag::Group(1);
        // Sopra l'intestazione della prima: la seconda cartella va in testa.
        let d = rows.drop_at(&seconda, 0, true).unwrap();
        assert_eq!((d.before, d.row, d.top), (0, 0, true));
        // Su un file della prima: dopo tutta la prima, linea sotto la sua ultima riga.
        let d = rows.drop_at(&seconda, 1, true).unwrap();
        assert_eq!((d.before, d.row, d.top), (3, 3, false));
        // Su se stessa: niente.
        assert_eq!(rows.drop_at(&seconda, 5, true), None);
        // La prima dopo la seconda: in fondo al lavoro.
        let d = rows.drop_at(&Drag::Group(0), 6, false).unwrap();
        assert_eq!((d.before, d.row, d.top), (5, 6, false));
    }
}
