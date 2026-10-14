use super::*;
use proptest::prelude::*;
use winfiles_geom::{Margins, Orientation, Paper};

fn item(id: usize, w: f64, h: f64, folder: &str) -> Item {
    Item {
        id,
        content: Size::new(w, h),
        folder: folder.into(),
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
    // Un'immagine 2:1 occupa tutta la larghezza e metà altezza, centrata in verticale.
    let img = s.items[0].image;
    assert!((img.w - 44.5).abs() < 1e-9 && (img.h - 22.25).abs() < 1e-9);
    assert!((img.y - (s.items[0].cell.y + 11.125)).abs() < 1e-9);
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

fn settings_strategy() -> impl Strategy<Value = PrintSettings> {
    (
        prop_oneof![Just(Paper::A4), Just(Paper::A3)],
        prop_oneof![Just(Orientation::Portrait), Just(Orientation::Landscape)],
        0.0f64..30.0,
        1u32..=MAX_COLUMNS,
        0.0f64..10.0,
        0.0f64..20.0,
        0.0f64..15.0,
        any::<bool>(),
    )
        .prop_map(
            |(paper, orientation, margin, columns, gap, caption, header, page_break_per_folder)| {
                PrintSettings {
                    paper,
                    orientation,
                    margins: Margins::uniform(margin),
                    columns,
                    gap,
                    caption,
                    header,
                    page_break_per_folder,
                }
            },
        )
}

fn items_strategy() -> impl Strategy<Value = Vec<Item>> {
    prop::collection::vec((0.1f64..5000.0, 0.1f64..5000.0, 0usize..4), 0..120).prop_map(|v| {
        let mut v: Vec<_> = v
            .into_iter()
            .enumerate()
            .map(|(i, (w, h, f))| item(i, w, h, &format!("c{f}")))
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
    fn invarianti_della_griglia(settings in settings_strategy(), items in items_strategy()) {
        let sheets = match grid(&settings, &items) {
            Ok(s) => s,
            Err(LayoutError::NothingFits) => return Ok(()),
            Err(e) => return Err(TestCaseError::fail(e.to_string())),
        };
        // Gli id sono assegnati prima dell'ordinamento per cartella: si cercano per id, non per posizione.
        let by_id: std::collections::HashMap<usize, &Item> = items.iter().map(|i| (i.id, i)).collect();
        let mut seen = vec![0u32; items.len()];
        for sheet in &sheets {
            let mut rects: Vec<Rect> = sheet.headers.iter().map(|h| h.rect).collect();
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
