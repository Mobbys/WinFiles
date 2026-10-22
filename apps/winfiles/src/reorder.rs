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
/// del lavoro; sotto un file aperto, le righe delle sue pagine.
pub struct Rows<'a> {
    pub groups: &'a [Vec<usize>],
    /// Per ogni gruppo e ogni file, quante righe di pagina ha sotto (0 se è chiuso).
    pub pages: &'a [Vec<usize>],
}

/// Che cosa c'è su una riga dell'elenco.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot {
    pub group: usize,
    /// Posizione del file nel gruppo; None per l'intestazione.
    pub file: Option<usize>,
    /// Vero per la riga di una pagina, sotto il suo file.
    pub page: bool,
}

impl Rows<'_> {
    fn page_rows(&self, group: usize, file: usize) -> usize {
        self.pages
            .get(group)
            .and_then(|p| p.get(file))
            .copied()
            .unwrap_or(0)
    }

    /// Le righe di un gruppo: intestazione, file e pagine aperte.
    fn group_rows(&self, group: usize) -> usize {
        let files = self.groups.get(group).map_or(0, Vec::len);
        1 + (0..files).map(|k| 1 + self.page_rows(group, k)).sum::<usize>()
    }

    /// Che cosa c'è sulla riga `row`.
    pub fn spot(&self, row: usize) -> Option<Spot> {
        let mut first = 0;
        for group in 0..self.groups.len() {
            let end = first + self.group_rows(group);
            if row < end {
                if row == first {
                    return Some(Spot {
                        group,
                        file: None,
                        page: false,
                    });
                }
                let mut r = first + 1;
                for k in 0..self.groups.get(group).map_or(0, Vec::len) {
                    let next = r + 1 + self.page_rows(group, k);
                    if row < next {
                        return Some(Spot {
                            group,
                            file: Some(k),
                            page: row > r,
                        });
                    }
                    r = next;
                }
            }
            first = end;
        }
        None
    }

    /// La riga dell'intestazione di un gruppo.
    fn header(&self, group: usize) -> usize {
        (0..group).map(|g| self.group_rows(g)).sum()
    }

    /// L'ultima riga di un file del gruppo: la sua, o quella della sua ultima pagina aperta.
    fn file_last_row(&self, group: usize, file: usize) -> usize {
        let before: usize = (0..file).map(|k| 1 + self.page_rows(group, k)).sum();
        self.header(group) + 1 + before + self.page_rows(group, file)
    }

    /// Dove cade `drag` con il puntatore sulla riga `row`, nella metà alta (`upper`) o bassa.
    /// None se lì non può andare.
    pub fn drop_at(&self, drag: &Drag, row: usize, upper: bool) -> Option<Drop> {
        let spot = self.spot(row)?;
        let g = spot.group;
        let files = self.groups.get(g)?;
        let first = *files.first()?;
        let last = *files.last()?;
        let header = self.header(g);
        match drag {
            Drag::Files { group, .. } if *group == g => Some(match spot.file {
                // Sull'intestazione: in cima alla cartella.
                None => Drop {
                    before: first,
                    row: header,
                    top: false,
                },
                Some(k) if upper && !spot.page => Drop {
                    before: *files.get(k)?,
                    row,
                    top: true,
                },
                // Sotto un file, dopo le sue pagine aperte.
                Some(k) => Drop {
                    before: files.get(k + 1).copied().unwrap_or(last + 1),
                    row: self.file_last_row(g, k),
                    top: false,
                },
            }),
            Drag::Files { .. } => None,
            Drag::Group(moving) if *moving == g => None,
            Drag::Group(_) if spot.file.is_none() && upper => Some(Drop {
                before: first,
                row: header,
                top: true,
            }),
            Drag::Group(_) => Some(Drop {
                before: last + 1,
                row: header + self.group_rows(g) - 1,
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

    fn closed(g: &[Vec<usize>]) -> Vec<Vec<usize>> {
        g.iter().map(|f| vec![0; f.len()]).collect()
    }

    fn at(group: usize, file: Option<usize>) -> Option<Spot> {
        Some(Spot {
            group,
            file,
            page: false,
        })
    }

    #[test]
    fn righe_e_gruppi() {
        let g = groups();
        let p = closed(&g);
        let rows = Rows {
            groups: &g,
            pages: &p,
        };
        assert_eq!(rows.spot(0), at(0, None));
        assert_eq!(rows.spot(3), at(0, Some(2)));
        assert_eq!(rows.spot(4), at(1, None));
        assert_eq!(rows.spot(6), at(1, Some(1)));
        assert_eq!(rows.spot(7), None);
    }

    #[test]
    fn le_pagine_aperte_restano_attaccate_al_loro_file() {
        let g = groups();
        // Il file 1 ha tre pagine aperte: righe 3, 4, 5; il file 2 scende alla riga 6.
        let p = vec![vec![0, 3, 0], vec![0, 0]];
        let rows = Rows {
            groups: &g,
            pages: &p,
        };
        assert_eq!(
            rows.spot(4),
            Some(Spot {
                group: 0,
                file: Some(1),
                page: true
            })
        );
        assert_eq!(rows.spot(6), at(0, Some(2)));
        assert_eq!(rows.spot(7), at(1, None));
        let drag = Drag::Files {
            group: 0,
            files: BTreeSet::from([0]),
        };
        // Su una pagina del file 1: dopo il file 1, linea sotto la sua ultima pagina.
        let d = rows.drop_at(&drag, 4, true).unwrap();
        assert_eq!((d.before, d.row, d.top), (2, 5, false));
        // Metà bassa della riga del file 1: lo stesso.
        let d = rows.drop_at(&drag, 2, false).unwrap();
        assert_eq!((d.before, d.row, d.top), (2, 5, false));
        // La seconda cartella dopo la prima: linea sotto l'ultima riga della prima.
        let d = rows.drop_at(&Drag::Group(1), 1, true).unwrap();
        assert_eq!((d.before, d.row, d.top), (3, 6, false));
    }

    #[test]
    fn i_file_restano_nella_loro_cartella() {
        let g = groups();
        let p = closed(&g);
        let rows = Rows {
            groups: &g,
            pages: &p,
        };
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
        let p = closed(&g);
        let rows = Rows {
            groups: &g,
            pages: &p,
        };
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
