use super::*;
use proptest::prelude::*;
use winfiles_geom::{Margins, Orientation, Paper};
use winfiles_model::View;

fn item(id: usize, w: f64, h: f64, folder: &str) -> Item {
    Item {
        id,
        content: Size::new(w, h),
        folder: folder.into(),
        caption: 12.0,
        folder_note: 0.0,
    }
}

#[test]
fn quattro_colonne_su_a4() {
    let settings = PrintSettings::default();
    let items: Vec<_> = (0..4).map(|i| item(i, 100.0, 50.0, "a")).collect();
    let sheets = grid(&settings, &items).unwrap();
    assert_eq!(sheets.len(), 1);
    let s = &sheets[0];
    assert_eq!(s.size, Size::new(210.0, 297.0));
    // 190 mm utili, 3 spazi da 4 mm: celle da 44,5 mm.
    assert!((s.items[0].cell.w - 44.5).abs() < 1e-9);
    // Un'immagine 2:1 occupa tutta la larghezza e metà altezza; in una riga di soli formati
    // orizzontali la riga è alta quanto l'immagine, senza vuoto sopra la didascalia.
    let img = s.items[0].image;
    assert!((img.w - 44.5).abs() < 1e-9 && (img.h - 22.25).abs() < 1e-9);
    assert!((img.y - s.items[0].cell.y).abs() < 1e-9);
    assert!((s.items[0].caption.y - img.bottom()).abs() < 1e-9);
    assert_eq!(s.headers.len(), 1);
}

#[test]
fn salto_pagina_per_cartella() {
    let settings = PrintSettings {
        page_break_per_folder: true,
        ..PrintSettings::default()
    };
    let items = vec![item(0, 1.0, 1.0, "a"), item(1, 1.0, 1.0, "b")];
    let sheets = grid(&settings, &items).unwrap();
    assert_eq!(sheets.len(), 2);
    assert_eq!(sheets[1].headers[0].folder, "b");
}

#[test]
fn intestazione_ripetuta_sul_foglio_successivo() {
    let settings = PrintSettings {
        columns: 1,
        ..PrintSettings::default()
    };
    let items: Vec<_> = (0..5).map(|i| item(i, 1.0, 1.0, "a")).collect();
    let sheets = grid(&settings, &items).unwrap();
    assert!(sheets.len() > 1);
    assert!(sheets[1].headers[0].continued);
}

#[test]
fn la_riga_e_alta_quanto_la_didascalia_piu_alta() {
    let settings = PrintSettings::default();
    let mut items: Vec<_> = (0..6).map(|i| item(i, 1.0, 1.0, "a")).collect();
    items[1].caption = 30.0;
    let s = &grid(&settings, &items).unwrap()[0];
    // Prima riga: 4 celle alte come la didascalia da 30 mm; seconda riga: 12 mm.
    assert!(s.items[..4].iter().all(|p| (p.caption.h - 30.0).abs() < 1e-9));
    assert!((s.items[4].caption.h - 12.0).abs() < 1e-9);
    assert!((s.items[4].cell.y - (s.items[0].cell.bottom() + settings.row_gap)).abs() < 1e-9);
}

#[test]
fn la_nota_della_cartella_sta_sotto_l_intestazione() {
    let settings = PrintSettings::default();
    let mut items = vec![item(0, 1.0, 1.0, "a"), item(1, 1.0, 1.0, "b")];
    items[0].folder_note = 8.0;
    let s = &grid(&settings, &items).unwrap()[0];
    let note = s.headers[0].note.unwrap();
    assert!((note.y - (s.headers[0].rect.y + settings.header)).abs() < 1e-9);
    assert!((note.h - 8.0).abs() < 1e-9);
    assert!(s.items[0].cell.y >= note.bottom() + settings.row_gap - 1e-9);
    assert!(s.headers[1].note.is_none());
}

#[test]
fn il_titolo_sta_in_cima_a_ogni_foglio() {
    let settings = PrintSettings {
        columns: 1,
        ..PrintSettings::default()
    };
    let items: Vec<_> = (0..5).map(|i| item(i, 1.0, 1.0, "a")).collect();
    let sheets = grid_with_title(&settings, &items, 12.0).unwrap();
    assert!(sheets.len() > 1);
    for s in &sheets {
        let t = s.title.unwrap();
        assert!((t.y - s.area.y).abs() < 1e-9 && (t.h - 12.0).abs() < 1e-9);
        assert!(s.headers[0].rect.y >= t.bottom() + settings.row_gap - 1e-9);
    }
    assert!(grid(&settings, &items).unwrap()[0].title.is_none());
}

#[test]
fn le_immagini_della_riga_poggiano_sulla_didascalia() {
    // Una riga con un verticale e un orizzontale: alta quanto il verticale; l'orizzontale sta in
    // basso, appoggiato alla sua didascalia, e le didascalie sono allineate.
    let settings = PrintSettings::default();
    let items = vec![item(0, 210.0, 297.0, "a"), item(1, 297.0, 210.0, "a")];
    let s = &grid(&settings, &items).unwrap()[0];
    let (alto, largo) = (&s.items[0], &s.items[1]);
    assert!((alto.image.h - alto.cell.w).abs() < 1e-9);
    assert!((largo.image.bottom() - alto.image.bottom()).abs() < 1e-9);
    assert!((largo.caption.y - alto.caption.y).abs() < 1e-9);
}

#[test]
fn spazio_tra_le_righe_separato_da_quello_tra_le_colonne() {
    let settings = PrintSettings {
        gap: 6.0,
        row_gap: 1.0,
        ..PrintSettings::default()
    };
    let items: Vec<_> = (0..8).map(|i| item(i, 1.0, 1.0, "a")).collect();
    let s = &grid(&settings, &items).unwrap()[0];
    assert!((s.items[1].cell.x - (s.items[0].cell.right() + 6.0)).abs() < 1e-9);
    assert!((s.items[4].cell.y - (s.items[0].cell.bottom() + 1.0)).abs() < 1e-9);
}

#[test]
fn nessun_elemento_un_foglio_vuoto() {
    let sheets = grid(&PrintSettings::default(), &[]).unwrap();
    assert_eq!(sheets.len(), 1);
    assert!(sheets[0].items.is_empty());
}

#[test]
fn impostazioni_non_valide() {
    let zero = PrintSettings {
        columns: 0,
        ..PrintSettings::default()
    };
    assert!(matches!(grid(&zero, &[]), Err(LayoutError::InvalidSettings(_))));
    let nan = PrintSettings {
        gap: f64::NAN,
        ..PrintSettings::default()
    };
    assert!(matches!(grid(&nan, &[]), Err(LayoutError::InvalidSettings(_))));
    let huge = PrintSettings {
        margins: Margins::uniform(150.0),
        ..PrintSettings::default()
    };
    assert_eq!(grid(&huge, &[]), Err(LayoutError::NothingFits));
}

#[test]
fn elenco_una_miniatura_per_riga_con_il_testo_a_destra() {
    let settings = PrintSettings {
        view: View::List,
        ..PrintSettings::default()
    };
    let items: Vec<Item> = (0..5)
        .map(|i| Item {
            caption: 12.0,
            ..item(i, 210.0, 297.0, "a")
        })
        .collect();
    let sheets = grid(&settings, &items).unwrap();
    let placed = &sheets[0].items;
    assert_eq!(placed.len(), 5);
    let cell = list_thumb(&settings);
    for (a, b) in placed.iter().zip(placed.iter().skip(1)) {
        // Una sotto l'altra, staccate dello spazio tra le righe.
        assert!((b.cell.y - a.cell.bottom() - settings.row_gap).abs() < 1e-9);
    }
    for p in placed {
        assert!((p.cell.x - sheets[0].area.x).abs() < 1e-9);
        assert!((p.cell.w - sheets[0].area.w).abs() < 1e-9);
        assert!(p.image.right() <= p.cell.x + cell + 1e-9);
        assert!(p.caption.x >= p.image.right() + settings.gap - 1e-9);
        assert!((p.caption.w - caption_width(&settings)).abs() < 1e-9);
        assert!((p.caption.y - p.image.y).abs() < 1e-9);
    }
}

#[test]
fn elenco_la_riga_segue_il_testo_se_e_piu_alto_della_miniatura() {
    let settings = PrintSettings {
        view: View::List,
        ..PrintSettings::default()
    };
    let items = [Item {
        caption: 80.0,
        ..item(0, 300.0, 100.0, "a")
    }];
    let p = &grid(&settings, &items).unwrap()[0].items[0];
    assert!((p.cell.h - 80.0).abs() < 1e-9);
}

#[test]
fn nesting_tutti_alla_stessa_scala() {
    let settings = PrintSettings {
        view: View::Nesting,
        ..PrintSettings::default()
    };
    let items = [
        item(0, 420.0, 297.0, "a"),
        item(1, 210.0, 297.0, "a"),
        item(2, 85.0, 55.0, "a"),
        item(3, 1.0, 1.0, "a"),
    ];
    let sheets = grid(&settings, &items).unwrap();
    let p = &sheets[0].items;
    // Il più grande occupa una colonna; gli altri in proporzione alle misure vere.
    let scale = p[0].image.w / 420.0;
    assert!((p[0].image.w - cell_width(&settings)).abs() < 1e-9);
    assert!((p[1].image.h / 297.0 - scale).abs() < 1e-9);
    assert!((p[2].image.w / 85.0 - scale).abs() < 1e-9);
    // Un file minuscolo resta visibile.
    assert!((p[3].image.w - NESTING_MIN_SIDE).abs() < 1e-9);
    // Stanno tutti nella prima riga, accostati: nella griglia a 4 colonne anche, ma qui la riga
    // è centrata e più stretta dell'area.
    assert!(p.iter().all(|c| (c.cell.y - p[0].cell.y).abs() < 1e-9));
    assert!(p[0].cell.x > sheets[0].area.x);
}

#[test]
fn nesting_i_piccoli_si_accostano_piu_della_griglia() {
    let settings = PrintSettings {
        view: View::Nesting,
        columns: 3,
        ..PrintSettings::default()
    };
    let mut items = vec![item(0, 1000.0, 700.0, "a")];
    items.extend((1..9).map(|i| item(i, 85.0, 55.0, "a")));
    let sheets = grid(&settings, &items).unwrap();
    let first_row = sheets[0]
        .items
        .iter()
        .filter(|c| (c.cell.y - sheets[0].items[0].cell.y).abs() < 1e-9)
        .count();
    assert!(first_row > settings.columns as usize);
}

fn settings_strategy() -> impl Strategy<Value = PrintSettings> {
    (
        prop_oneof![Just(Paper::A4), Just(Paper::A3)],
        prop_oneof![Just(Orientation::Portrait), Just(Orientation::Landscape)],
        0.0f64..30.0,
        1u32..=MAX_COLUMNS,
        (0.0f64..10.0, 0.0f64..10.0),
        0.0f64..15.0,
        (any::<bool>(), 0u8..3),
    )
        .prop_map(
            |(paper, orientation, margin, columns, (gap, row_gap), header, (page_break_per_folder, view))| {
                PrintSettings {
                    paper,
                    orientation,
                    view: match view {
                        0 => View::Grid,
                        1 => View::List,
                        _ => View::Nesting,
                    },
                    margins: Margins::uniform(margin),
                    columns,
                    gap,
                    row_gap,
                    header,
                    page_break_per_folder,
                    ..PrintSettings::default()
                }
            },
        )
}

fn items_strategy() -> impl Strategy<Value = Vec<Item>> {
    let one = (
        0.1f64..5000.0,
        0.1f64..5000.0,
        0usize..4,
        -5.0f64..400.0,
        -5.0f64..400.0,
    );
    prop::collection::vec(one, 0..120).prop_map(|v| {
        let mut v: Vec<_> = v
            .into_iter()
            .enumerate()
            .map(|(i, (w, h, f, caption, note))| Item {
                caption,
                folder_note: note,
                ..item(i, w, h, &format!("c{f}"))
            })
            .collect();
        // Le cartelle arrivano raggruppate, come dalla scansione.
        v.sort_by(|a, b| a.folder.cmp(&b.folder));
        v
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Le regole che ogni impaginazione deve rispettare, qualunque siano impostazioni e file.
    #[test]
    fn invarianti_della_griglia(settings in settings_strategy(), items in items_strategy(), title in prop_oneof![Just(0.0f64), -5.0f64..80.0]) {
        let sheets = match grid_with_title(&settings, &items, title) {
            Ok(s) => s,
            Err(LayoutError::NothingFits) => return Ok(()),
            Err(e) => return Err(TestCaseError::fail(e.to_string())),
        };
        // Gli id sono assegnati prima dell'ordinamento per cartella: si cercano per id, non per posizione.
        let by_id: std::collections::HashMap<usize, &Item> = items.iter().map(|i| (i.id, i)).collect();
        let mut seen = vec![0u32; items.len()];
        for sheet in &sheets {
            let mut rects: Vec<Rect> = sheet.headers.iter().map(|h| h.rect).collect();
            if let Some(t) = sheet.title {
                prop_assert!(sheet.area.contains(t, 1e-6));
                rects.push(t);
            }
            for p in &sheet.items {
                seen[p.id] += 1;
                // Tutto dentro l'area stampabile, l'immagine dentro la sua cella.
                prop_assert!(sheet.area.contains(p.cell, 1e-6));
                prop_assert!(p.cell.contains(p.image, 1e-6));
                prop_assert!(p.cell.contains(p.caption, 1e-6));
                // Proporzioni conservate.
                let src = by_id.get(&p.id).ok_or_else(|| TestCaseError::fail("id sconosciuto"))?.content;
                if p.image.w > 1e-3 && p.image.h > 1e-3 {
                    let ratio = (p.image.w / p.image.h) / (src.w / src.h);
                    prop_assert!((ratio - 1.0).abs() < 1e-6);
                }
                rects.push(p.cell);
            }
            for h in &sheet.headers {
                prop_assert!(sheet.area.contains(h.rect, 1e-6));
                if let Some(note) = h.note {
                    prop_assert!(h.rect.contains(note, 1e-6));
                }
            }
            // Nessuna sovrapposizione tra celle e intestazioni dello stesso foglio.
            for (i, a) in rects.iter().enumerate() {
                for b in rects.iter().skip(i + 1) {
                    prop_assert!(!a.overlaps(*b, 1e-6), "{a:?} si sovrappone a {b:?}");
                }
            }
            // Un foglio non inizia mai con una miniatura senza la sua intestazione.
            if let Some(first) = sheet.items.first() {
                let header = sheet.headers.first().ok_or_else(|| TestCaseError::fail("foglio senza intestazione"))?;
                prop_assert!(header.rect.y <= first.cell.y);
            }
        }
        // Ogni elemento compare una e una sola volta.
        prop_assert!(seen.iter().all(|&n| n == 1));
    }
}

mod riepilogo {
    use super::*;
    use crate::{Line, SummaryBlocks, summary};

    fn lines(groups: usize, rows: usize, h: f64) -> Vec<Line> {
        let mut out = Vec::new();
        for _ in 0..groups {
            out.push(Line {
                height: 8.0,
                group: true,
            });
            out.extend((0..rows).map(|_| Line {
                height: h,
                group: false,
            }));
        }
        out
    }

    #[test]
    fn tutte_le_righe_una_volta_e_dentro_l_area() {
        let s = PrintSettings::default();
        let input = lines(5, 40, 6.0);
        let blocks = SummaryBlocks {
            title: 10.0,
            heading: 15.0,
            columns: 6.0,
        };
        let sheets = summary(&s, blocks, &input).unwrap();
        assert!(sheets.len() > 1);
        let mut seen: Vec<usize> = sheets
            .iter()
            .flat_map(|sh| sh.lines.iter().filter(|l| !l.continued).map(|l| l.line))
            .collect();
        seen.dedup();
        assert_eq!(seen, (0..input.len()).collect::<Vec<_>>());
        for sh in &sheets {
            assert!(sh.title.is_some() && sh.columns.is_some());
            for l in &sh.lines {
                assert!(l.rect.bottom() <= sh.area.bottom() + 1e-6);
            }
        }
        // L'intestazione con i totali solo sul primo foglio.
        assert!(sheets[0].heading.is_some() && sheets[1].heading.is_none());
    }

    #[test]
    fn la_cartella_si_ripete_e_non_resta_sola_in_fondo() {
        let s = PrintSettings::default();
        let input = lines(3, 60, 6.0);
        let sheets = summary(&s, SummaryBlocks::default(), &input).unwrap();
        for sh in &sheets {
            // Il primo elemento di ogni foglio dopo il primo è una cartella (nuova o ripetuta).
            let first = &sh.lines[0];
            assert!(input[first.line].group);
            // L'ultima riga non è mai una cartella.
            let last = sh.lines.last().unwrap();
            assert!(!input[last.line].group || last.line + 1 == input.len());
        }
        assert!(sheets.iter().skip(1).any(|sh| sh.lines[0].continued));
    }

    #[test]
    fn altezze_ostili_non_bloccano() {
        let s = PrintSettings::default();
        let input = vec![
            Line {
                height: f64::INFINITY,
                group: true,
            },
            Line {
                height: 1e12,
                group: false,
            },
            Line {
                height: -5.0,
                group: false,
            },
            Line {
                height: f64::NAN,
                group: false,
            },
        ];
        let sheets = summary(
            &s,
            SummaryBlocks {
                title: 1e9,
                heading: 1e9,
                columns: 1e9,
            },
            &input,
        )
        .unwrap();
        let count: usize = sheets.iter().map(|s| s.lines.len()).sum();
        assert!(count >= 4 && sheets.len() <= 4);
        assert_eq!(summary(&s, SummaryBlocks::default(), &[]).unwrap().len(), 1);
    }
}
