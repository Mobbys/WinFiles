use super::*;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use winfiles_geom::{Margins, mm_to_pt};

/// Una pagina di prova: MediaBox, TrimBox, Rotate e contenuto.
struct TestPage {
    media: [f64; 4],
    trim: Option<[f64; 4]>,
    rotate: i64,
    content: &'static str,
    resources: &'static str,
}

/// Quadrato rosso nell'angolo in alto a sinistra di una pagina 200 × 100 (non ruotata).
const RED_TOP_LEFT: &str = "1 0 0 rg 0 80 20 20 re f";

fn page(media: [f64; 4], rotate: i64, content: &'static str) -> TestPage {
    TestPage {
        media,
        trim: None,
        rotate,
        content,
        resources: "<< >>",
    }
}

/// PDF con offset xref corretti.
fn pdf(pages: &[TestPage]) -> Vec<u8> {
    pdf_extra(pages, &[])
}

/// Come [`pdf`], con altri oggetti dopo le pagine (numerati da 3 + 2 × pagine).
fn pdf_extra(pages: &[TestPage], extra: &[&str]) -> Vec<u8> {
    let n = pages.len();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {n} >>",
            (0..n)
                .map(|i| format!("{} 0 R", 3 + 2 * i))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    ];
    let r = |v: [f64; 4]| format!("[{} {} {} {}]", v[0], v[1], v[2], v[3]);
    for (i, p) in pages.iter().enumerate() {
        let trim = p.trim.map(|t| format!(" /TrimBox {}", r(t))).unwrap_or_default();
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox {}{trim} /Rotate {} /Contents {} 0 R /Resources {} >>",
            r(p.media),
            p.rotate,
            4 + 2 * i,
            p.resources
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            p.content.len() + 1,
            p.content
        ));
    }
    objects.extend(extra.iter().map(|o| o.to_string()));
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for off in offsets {
        out.extend(format!("{off:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .bytes(),
    );
    out
}

fn editor(pages: &[TestPage]) -> Editor {
    Editor::new(
        PdfFile::from_bytes(pdf(pages)).unwrap(),
        PathBuf::from("prova.pdf"),
    )
}

/// Il PDF salvato, riletto da hayro come lo leggerà l'anteprima.
fn reread(e: &Editor) -> Pdf {
    Pdf::new(e.to_bytes().unwrap()).unwrap()
}

/// La pagina disegnata a 1 pixel per punto: RGBA, larghezza, altezza.
fn render(pdf: &Pdf, page: usize) -> (Vec<u8>, u32, u32) {
    let page = &pdf.pages()[page];
    let pix = hayro::render(
        page,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings {
            x_scale: 1.0,
            y_scale: 1.0,
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        },
    );
    (
        pix.data_as_u8_slice().to_vec(),
        u32::from(pix.width()),
        u32::from(pix.height()),
    )
}

/// Colore del pixel (x, y) contati dall'alto a sinistra.
fn pixel(img: &(Vec<u8>, u32, u32), x: u32, y: u32) -> [u8; 3] {
    let i = ((y * img.1 + x) * 4) as usize;
    [img.0[i], img.0[i + 1], img.0[i + 2]]
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.05
}

#[test]
fn misure_con_rotazione_e_formato_finito() {
    let mut p = page([0.0, 0.0, 200.0, 100.0], 90, RED_TOP_LEFT);
    p.trim = Some([10.0, 10.0, 190.0, 90.0]);
    let e = editor(&[p]);
    let info = e.pages()[0];
    assert_eq!(info.rotation, 90);
    assert!(near(info.size.w, pt_to_mm(100.0)) && near(info.size.h, pt_to_mm(200.0)));
    let trim = info.trim.unwrap();
    assert!(near(trim.w, pt_to_mm(80.0)) && near(trim.h, pt_to_mm(180.0)));
}

#[test]
fn ruota_elimina_sposta_e_annulla() {
    let mut e = editor(&[
        page([0.0, 0.0, 100.0, 100.0], 0, ""),
        page([0.0, 0.0, 200.0, 100.0], 0, ""),
        page([0.0, 0.0, 300.0, 100.0], 0, ""),
    ]);
    e.rotate(&[0], 1).unwrap();
    assert_eq!(e.pages()[0].rotation, 90);
    e.delete(&[1]).unwrap();
    assert_eq!(e.page_count(), 2);
    e.move_pages(&[1], 0).unwrap();
    let widths: Vec<f64> = e.pages().iter().map(|p| p.size.w).collect();
    assert!(near(widths[0], pt_to_mm(300.0)) && near(widths[1], pt_to_mm(100.0)));
    assert_eq!(e.undo_label(), Some("Sposta pagine"));
    assert!(e.undo());
    assert!(e.undo());
    assert_eq!(e.page_count(), 3);
    assert!(e.redo());
    assert_eq!(e.page_count(), 2);
    // Dopo una modifica nuova non si rifà più nulla.
    e.rotate(&[0], -1).unwrap();
    assert!(!e.redo());
    // Le impronte seguono le pagine: spostare non cambia quella di una pagina.
    let keys = e.page_keys();
    e.move_pages(&[1], 0).unwrap();
    let moved = e.page_keys();
    assert_eq!((moved[0], moved[1]), (keys[1], keys[0]));
    e.rotate(&[0], 1).unwrap();
    assert_ne!(e.page_keys()[0], moved[0]);
    // Il file salvato ha le stesse pagine.
    let pdf = reread(&e);
    assert_eq!(pdf.pages().len(), 2);
}

#[test]
fn non_si_eliminano_tutte_le_pagine() {
    let mut e = editor(&[page([0.0, 0.0, 100.0, 100.0], 0, "")]);
    assert!(matches!(e.delete(&[0]), Err(EditError::LastPage)));
    assert!(!e.is_modified());
    assert_eq!(e.undo_label(), None);
}

#[test]
fn rifila_al_formato_finito() {
    let mut with_trim = page([0.0, 0.0, 200.0, 100.0], 0, "");
    with_trim.trim = Some([10.0, 10.0, 190.0, 90.0]);
    let mut e = editor(&[with_trim, page([0.0, 0.0, 100.0, 100.0], 0, "")]);
    assert_eq!(e.trim(&[0, 1]).unwrap(), 1);
    let info = e.pages();
    assert!(near(info[0].size.w, pt_to_mm(180.0)) && near(info[0].size.h, pt_to_mm(80.0)));
    assert_eq!(info[0].trim, None);
    // Niente da rifilare: nessuna modifica e niente da annullare in più.
    assert_eq!(e.trim(&[0, 1]).unwrap(), 0);
    assert!(e.undo());
    assert_eq!(e.undo_label(), None);
}

#[test]
fn margini_colorati_attorno_al_contenuto() {
    let mut e = editor(&[page([0.0, 0.0, 200.0, 100.0], 0, RED_TOP_LEFT)]);
    let m = Margins {
        top: pt_to_mm(10.0),
        right: pt_to_mm(20.0),
        bottom: pt_to_mm(30.0),
        left: pt_to_mm(40.0),
    };
    e.add_margins(&[0], m, [0, 0, 255]).unwrap();
    let info = e.pages()[0];
    assert!(near(info.size.w, pt_to_mm(260.0)) && near(info.size.h, pt_to_mm(140.0)));
    let img = render(&reread(&e), 0);
    assert_eq!((img.1, img.2), (260, 140));
    // Margine blu a sinistra e in alto, il quadrato rosso spostato di 40 a destra e 10 in giù.
    assert_eq!(pixel(&img, 5, 70), [0, 0, 255]);
    assert_eq!(pixel(&img, 100, 5), [0, 0, 255]);
    assert_eq!(pixel(&img, 50, 20), [255, 0, 0]);
    // Il bianco della pagina resta bianco.
    assert_eq!(pixel(&img, 150, 70), [255, 255, 255]);
}

#[test]
fn margini_su_una_pagina_ruotata() {
    // Pagina 200 × 100 girata di 90°: si vede 100 × 200, col quadrato rosso in alto a destra.
    let mut e = editor(&[page([0.0, 0.0, 200.0, 100.0], 90, RED_TOP_LEFT)]);
    let before = render(&reread(&e), 0);
    assert_eq!((before.1, before.2), (100, 200));
    assert_eq!(pixel(&before, 90, 10), [255, 0, 0]);
    e.add_margins(&[0], Margins::uniform(pt_to_mm(10.0)), [0, 255, 0])
        .unwrap();
    assert_eq!(e.pages()[0].rotation, 0);
    let after = render(&reread(&e), 0);
    assert_eq!((after.1, after.2), (120, 220));
    assert_eq!(pixel(&after, 100, 20), [255, 0, 0]);
    assert_eq!(pixel(&after, 3, 100), [0, 255, 0]);
}

#[test]
fn margini_negativi_tolgono_dai_bordi() {
    let mut e = editor(&[page([0.0, 0.0, 200.0, 100.0], 0, RED_TOP_LEFT)]);
    let m = Margins {
        top: pt_to_mm(-10.0),
        right: pt_to_mm(-50.0),
        bottom: pt_to_mm(-20.0),
        left: pt_to_mm(-5.0),
    };
    e.add_margins(&[0], m, [0, 0, 255]).unwrap();
    let info = e.pages()[0];
    assert!(near(info.size.w, pt_to_mm(145.0)) && near(info.size.h, pt_to_mm(70.0)));
    let img = render(&reread(&e), 0);
    assert_eq!((img.1, img.2), (145, 70));
    // Il quadrato rosso (0..20 × 0..20 dall'alto) ora parte da 0 e arriva a 15 × 10.
    assert_eq!(pixel(&img, 2, 2), [255, 0, 0]);
    assert_eq!(pixel(&img, 14, 9), [255, 0, 0]);
    assert_eq!(pixel(&img, 20, 2), [255, 255, 255]);
    // Niente fasce blu: i margini negativi tolgono, non aggiungono.
    assert_eq!(pixel(&img, 140, 65), [255, 255, 255]);
}

#[test]
fn margini_negativi_non_tolgono_tutta_la_pagina() {
    let mut e = editor(&[page([0.0, 0.0, 200.0, 100.0], 0, RED_TOP_LEFT)]);
    let m = Margins::uniform(pt_to_mm(-60.0));
    assert!(e.add_margins(&[0], m, [255, 255, 255]).is_err());
    assert_eq!(e.undo_label(), None);
}

#[test]
fn la_taglierina_tiene_solo_il_riquadro() {
    let mut e = editor(&[
        page([0.0, 0.0, 200.0, 100.0], 0, RED_TOP_LEFT),
        page([0.0, 0.0, 100.0, 50.0], 0, RED_TOP_LEFT),
    ]);
    // Il quadrato rosso in alto a sinistra, 20 × 20 punti, con un po' d'aria a destra e sotto.
    let cut = winfiles_geom::Rect::new(0.0, 0.0, pt_to_mm(30.0), pt_to_mm(30.0));
    assert_eq!(e.crop(&[0], cut).unwrap(), 1);
    let info = e.pages()[0];
    assert!(near(info.size.w, pt_to_mm(30.0)) && near(info.size.h, pt_to_mm(30.0)));
    let img = render(&reread(&e), 0);
    assert_eq!(pixel(&img, 10, 10), [255, 0, 0]);
    assert_eq!(pixel(&img, 25, 25), [255, 255, 255]);
    // La seconda pagina non era tra quelle scelte.
    assert!(near(e.pages()[1].size.w, pt_to_mm(100.0)));
    assert_eq!(e.undo_label(), Some("Taglia"));
}

#[test]
fn la_taglierina_si_ferma_al_bordo_della_pagina() {
    let mut e = editor(&[
        page([0.0, 0.0, 200.0, 100.0], 0, RED_TOP_LEFT),
        page([0.0, 0.0, 100.0, 50.0], 0, RED_TOP_LEFT),
    ]);
    // Più largo della seconda pagina: su quella si taglia solo quello che c'è.
    let cut = winfiles_geom::Rect::new(pt_to_mm(10.0), pt_to_mm(10.0), pt_to_mm(150.0), pt_to_mm(80.0));
    assert_eq!(e.crop(&[0, 1], cut).unwrap(), 2);
    let pages = e.pages();
    assert!(near(pages[0].size.w, pt_to_mm(150.0)) && near(pages[0].size.h, pt_to_mm(80.0)));
    assert!(near(pages[1].size.w, pt_to_mm(90.0)) && near(pages[1].size.h, pt_to_mm(40.0)));
    // Un riquadro fuori dalla pagina non la tocca.
    let outside = winfiles_geom::Rect::new(pt_to_mm(500.0), 0.0, 10.0, 10.0);
    assert_eq!(e.crop(&[1], outside).unwrap(), 0);
}

#[test]
fn adatta_ad_a4_in_orizzontale() {
    let mut p = page([0.0, 0.0, 200.0, 100.0], 0, RED_TOP_LEFT);
    p.trim = Some([10.0, 10.0, 190.0, 90.0]);
    let mut e = editor(&[p]);
    e.fit_paper(&[0], Paper::A4).unwrap();
    let info = e.pages()[0];
    assert!(near(info.size.w, 297.0) && near(info.size.h, 210.0), "{info:?}");
    // Scala 297/200 in larghezza (in altezza ci starebbe di più): il formato finito segue.
    let s = mm_to_pt(297.0) / 200.0;
    let trim = info.trim.unwrap();
    assert!(near(trim.w, pt_to_mm(180.0 * s)) && near(trim.h, pt_to_mm(80.0 * s)));
}

#[test]
fn ridimensiona_in_proporzione_e_al_centro() {
    let mut e = editor(&[page([0.0, 0.0, 200.0, 100.0], 0, RED_TOP_LEFT)]);
    e.resize(&[0], Size::new(pt_to_mm(400.0), pt_to_mm(400.0)))
        .unwrap();
    let img = render(&reread(&e), 0);
    assert_eq!((img.1, img.2), (400, 400));
    // Scala 2, centrato in verticale: il contenuto va da y = 100 a y = 300 (dall'alto).
    assert_eq!(pixel(&img, 20, 120), [255, 0, 0]);
    assert_eq!(pixel(&img, 20, 90), [255, 255, 255]);
    assert!(e.resize(&[0], Size::new(0.0, 10.0)).is_err());
}

#[test]
fn estrai_e_dividi() {
    let e = editor(&[
        page([0.0, 0.0, 100.0, 100.0], 0, ""),
        page([0.0, 0.0, 200.0, 100.0], 0, ""),
        page([0.0, 0.0, 300.0, 100.0], 0, ""),
    ]);
    let part = e.extract(&[2, 0]).unwrap();
    let bytes = part.to_bytes().unwrap();
    let pdf = Pdf::new(bytes).unwrap();
    assert_eq!(pdf.pages().len(), 2);
    assert!(near(f64::from(pdf.pages()[0].render_dimensions().0), 300.0));
    assert_eq!(split_ranges(3, &[1]), vec![0..1, 1..3]);
}

#[test]
fn unisci_due_pdf() {
    let a = PdfFile::from_bytes(pdf(&[page([0.0, 0.0, 100.0, 100.0], 0, "")])).unwrap();
    let b = PdfFile::from_bytes(pdf(&[
        page([0.0, 0.0, 200.0, 100.0], 0, ""),
        page([0.0, 0.0, 300.0, 100.0], 0, ""),
    ]))
    .unwrap();
    let all = combine(&[("a", &a), ("b", &b)]).unwrap();
    let mut e = Editor::new(all, PathBuf::from("unito.pdf"));
    assert_eq!(e.page_count(), 3);
    e.insert_from(&a, &[0], 1).unwrap();
    assert_eq!(e.page_count(), 4);
}

#[test]
fn salva_senza_rovinare_il_file_se_fallisce() {
    let dir = std::env::temp_dir().join(format!("winfiles-pdf-edit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("prova.pdf");
    std::fs::write(&path, pdf(&[page([0.0, 0.0, 100.0, 100.0], 0, "")])).unwrap();
    let mut e = Editor::open(&path).unwrap();
    e.rotate(&[0], 2).unwrap();
    assert!(e.is_modified());
    e.save(&path).unwrap();
    assert!(!e.is_modified());
    let again = Editor::open(&path).unwrap();
    assert_eq!(again.pages()[0].rotation, 180);
    // Una cartella al posto del file: errore, nessun file temporaneo rimasto.
    assert!(e.save(&dir).is_err());
    let leftovers = std::fs::read_dir(&dir).unwrap().count();
    assert_eq!(leftovers, 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn file_non_pdf() {
    assert!(PdfFile::from_bytes(b"ciao".to_vec()).is_err());
}

/// Due tinte piatte: un Pantone (nome con spazi) e un oro, più un DeviceN con il Pantone.
const SPOT_RESOURCES: &str = "<< /ColorSpace << \
    /P [/Separation /PANTONE#20185#20C /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0.9 0.8 0] /N 1 >>] \
    /O [/Separation /Oro /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0.2 0.8 0.2] /N 1 >>] \
    /R [/Separation /All /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [1 1 1 1] /N 1 >>] \
    >> /ExtGState << /Sovra << /OP true /op true >> >> >>";

/// Un quadrato Pantone, uno blu, una linea oro, una croce di registrazione.
const SPOT_CONTENT: &str = "/P cs 1 sc 0 0 50 50 re f 0 0 1 rg 50 50 20 20 re f \
    /O CS 1 SC 0 0 m 100 100 l S /R CS 1 SC 90 0 m 100 0 l S";

fn spot_page() -> TestPage {
    let mut p = page([0.0, 0.0, 200.0, 100.0], 0, SPOT_CONTENT);
    p.resources = SPOT_RESOURCES;
    p
}

#[test]
fn elenca_le_tinte_piatte_senza_registrazione() {
    let e = editor(&[
        spot_page(),
        page([0.0, 0.0, 200.0, 100.0], 0, RED_TOP_LEFT),
        spot_page(),
    ]);
    let spots = e.spots().unwrap();
    let names: Vec<_> = spots.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Oro", "PANTONE 185 C"]);
    assert!(spots.iter().all(|s| s.pages == [0, 2]));
    assert!(spots.iter().all(|s| s.overprint == Overprint::None));
    // Il Pantone 185 è rosso: 0 90 80 0 in CMYK.
    assert_eq!(spots[1].swatch, Some([255, 25, 51]));
}

#[test]
fn rinomina_una_tinta() {
    let mut e = editor(&[spot_page()]);
    e.rename_spot(b"Oro", "PANTONE 871 C").unwrap();
    let names: Vec<_> = e.spots().unwrap().into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["PANTONE 185 C", "PANTONE 871 C"]);
    // Rinominata come l'altra, le due diventano una lastra sola.
    e.rename_spot(b"PANTONE 871 C", "PANTONE 185 C").unwrap();
    assert_eq!(e.spots().unwrap().len(), 1);
    // Il PDF salvato si rilegge e si disegna.
    let img = render(&reread(&e), 0);
    assert_eq!((img.1, img.2), (200, 100));
    assert!(e.rename_spot(b"Nessuna", "X").is_err());
}

#[test]
fn sovrastampa_solo_sulla_tinta_scelta() {
    let mut e = editor(&[spot_page()]);
    assert_eq!(e.set_spot_overprint(b"PANTONE 185 C", true).unwrap(), 1);
    let spots = e.spots().unwrap();
    let get = |n: &str| spots.iter().find(|s| s.name == n).unwrap().overprint;
    assert_eq!(get("PANTONE 185 C"), Overprint::All);
    assert_eq!(get("Oro"), Overprint::None);
    // Già in sovrastampa: niente da fare, niente da annullare.
    assert_eq!(e.set_spot_overprint(b"PANTONE 185 C", true).unwrap(), 0);
    assert_eq!(e.undo_label(), Some("Sovrastampa"));
    // Il quadrato blu dopo il Pantone non è in sovrastampa: il disegno resta quello di prima.
    let before = render(&Pdf::new(pdf(&[spot_page()])).unwrap(), 0);
    let img = render(&reread(&e), 0);
    assert_eq!(pixel(&img, 60, 40), pixel(&before, 60, 40));
    // E si toglie.
    e.set_spot_overprint(b"PANTONE 185 C", false).unwrap();
    let spots = e.spots().unwrap();
    assert!(spots.iter().all(|s| s.overprint == Overprint::None));
}

#[test]
fn la_sovrastampa_del_documento_si_legge() {
    let mut p = page(
        [0.0, 0.0, 200.0, 100.0],
        0,
        "/Sovra gs /P cs 1 sc 0 0 50 50 re f /O CS 1 SC 0 0 m 100 100 l S",
    );
    p.resources = SPOT_RESOURCES;
    let e = editor(&[p]);
    assert!(e.spots().unwrap().iter().all(|s| s.overprint == Overprint::All));
}

#[test]
fn la_sovrastampa_entra_nei_moduli() {
    // La pagina disegna un modulo (oggetto 5) che dipinge col Pantone.
    let mut p = page([0.0, 0.0, 200.0, 100.0], 0, "/F Do");
    p.resources = "<< /XObject << /F 5 0 R >> >>";
    let form_content = "/P cs 1 sc 0 0 50 50 re f";
    let form = format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 200 100] /Resources {SPOT_RESOURCES} /Length {} >>\nstream\n{form_content}\nendstream",
        form_content.len() + 1
    );
    let mut e = Editor::new(
        PdfFile::from_bytes(pdf_extra(&[p], &[&form])).unwrap(),
        PathBuf::from("prova.pdf"),
    );
    let spots = e.spots().unwrap();
    // L'oro è definito nelle risorse ma il modulo non lo usa.
    assert_eq!(spots[0].name, "Oro");
    assert_eq!(spots[0].overprint, Overprint::Unused);
    assert!(spots[0].pages.is_empty());
    assert_eq!(spots[1].pages, [0]);
    assert_eq!(e.set_spot_overprint(b"PANTONE 185 C", true).unwrap(), 1);
    assert_eq!(e.spots().unwrap()[1].overprint, Overprint::All);
    let img = render(&reread(&e), 0);
    assert_eq!((img.1, img.2), (200, 100));
}

#[test]
fn le_sfumature_a_pattern_contano_come_la_loro_tinta() {
    // Una sfumatura di Illustrator: riempimento con un pattern di sfumatura sull'Argento.
    let mut p = page(
        [0.0, 0.0, 200.0, 100.0],
        0,
        "/Pattern cs /S0 scn 0 0 200 100 re f 0 0 1 rg 10 10 20 20 re f",
    );
    p.resources = "<< /Pattern << /S0 << /PatternType 2 /Shading << /ShadingType 2 \
        /ColorSpace [/Separation /Argento /DeviceCMYK << /FunctionType 2 /Domain [0 1] \
        /C0 [0 0 0 0] /C1 [0 0 0 0.4] /N 1 >>] /Coords [0 0 200 0] \
        /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >> >> >> >>";
    let mut e = editor(&[p]);
    let spots = e.spots().unwrap();
    assert_eq!(spots.len(), 1);
    assert_eq!(spots[0].name, "Argento");
    assert_eq!(spots[0].pages, vec![0]);
    assert_eq!(spots[0].overprint, Overprint::None);
    assert_eq!(e.set_spot_overprint(b"Argento", true).unwrap(), 1);
    assert_eq!(e.spots().unwrap()[0].overprint, Overprint::All);
}

#[test]
fn rinomina_anche_le_parti_indirette_delle_devicen() {
    // Come le scrive Illustrator: l'elenco dei nomi, gli attributi e i Colorants in oggetti a sé.
    let mut p = page([0.0, 0.0, 200.0, 100.0], 0, "/DN cs 1 sc 0 0 50 50 re f");
    p.resources = "<< /ColorSpace << /DN [/DeviceN 5 0 R /DeviceCMYK \
        << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0.2 0.8 0.25] /N 1 >> 6 0 R] >> >>";
    let sep = "[/Separation /Oro /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] \
        /C1 [0 0.2 0.8 0.25] /N 1 >>]";
    let colorants = format!("<< /Oro {sep} >>");
    let mut e = Editor::new(
        PdfFile::from_bytes(pdf_extra(
            &[p],
            &[
                "[/Oro]",
                "<< /Subtype /DeviceN /Colorants 7 0 R /MixingHints << /PrintingOrder [/Oro] \
                 /Solidities << /Oro 0.5 >> >> >>",
                &colorants,
            ],
        ))
        .unwrap(),
        PathBuf::from("prova.pdf"),
    );
    e.rename_spot(b"Oro", "Oro lucido").unwrap();
    // Nel file salvato e riletto il vecchio nome non c'è più da nessuna parte.
    let again = Editor::new(
        PdfFile::from_bytes(e.to_bytes().unwrap()).unwrap(),
        PathBuf::from("prova.pdf"),
    );
    let doc = &again.doc;
    for (_, o) in doc.scan_objects_subset(doc.object_numbers()) {
        assert!(!names_in(&o.unwrap()).iter().any(|n| n == b"Oro"));
    }
    let names: Vec<_> = e.spots().unwrap().into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["Oro lucido"]);
}

#[test]
fn toglie_la_copia_di_illustrator() {
    let mut p = page([0.0, 0.0, 200.0, 100.0], 0, "0 0 1 rg 0 0 50 50 re f");
    p.resources = "<< >> /PieceInfo << /Illustrator << /LastModified (D:20261016) \
        /Private 5 0 R >> /Altro << /Dato 1 >> >>";
    let mut e = Editor::new(
        PdfFile::from_bytes(pdf_extra(
            &[p],
            &[
                "<< /AIMetaData 6 0 R /ContainerVersion 12 >>",
                "<< /Length 9 >>\nstream\n%AI12_File\nendstream",
            ],
        ))
        .unwrap(),
        PathBuf::from("prova.pdf"),
    );
    assert!(e.has_illustrator_data());
    assert_eq!(e.strip_illustrator_data().unwrap(), 1);
    assert!(!e.has_illustrator_data());
    // Riletto il file salvato: la copia non c'è più, le altre voci di PieceInfo restano.
    let again = Editor::new(
        PdfFile::from_bytes(e.to_bytes().unwrap()).unwrap(),
        PathBuf::from("prova.pdf"),
    );
    assert!(!again.has_illustrator_data());
    let doc = &again.doc;
    let objects: Vec<_> = doc
        .scan_objects_subset(doc.object_numbers())
        .filter_map(|(_, o)| o.ok())
        .collect();
    let piece = objects.iter().find_map(|o| match &**o {
        pdfcraft_cos::Object::Dict(d) => d.get(b"PieceInfo").and_then(|p| doc.dict(p)),
        _ => None,
    });
    assert!(piece.is_some_and(|p| p.get(b"Altro").is_some()));
    // La copia stessa, non più raggiunta da niente, non è stata scritta.
    assert!(!objects.iter().any(|o| match &**o {
        pdfcraft_cos::Object::Dict(d) => d.get(b"ContainerVersion").is_some(),
        _ => false,
    }));
    let img = render(&reread(&e), 0);
    assert_eq!((img.1, img.2), (200, 100));
    // Annullabile, e senza copia non c'è niente da fare.
    assert!(e.undo());
    assert!(e.has_illustrator_data());
    let mut plain = editor(&[page([0.0, 0.0, 200.0, 100.0], 0, "")]);
    assert_eq!(plain.strip_illustrator_data().unwrap(), 0);
    assert_eq!(plain.undo_label(), None);
}

/// Tutti i nomi e le chiavi di un oggetto, a qualsiasi profondità.
fn names_in(o: &pdfcraft_cos::Object) -> Vec<Vec<u8>> {
    use pdfcraft_cos::Object;
    match o {
        Object::Name(n) => vec![n.clone()],
        Object::Array(a) => a.iter().flat_map(names_in).collect(),
        Object::Dict(d) => d
            .iter()
            .flat_map(|(k, v)| [vec![k.clone()], names_in(v)].concat())
            .collect(),
        Object::Stream(s) => names_in(&Object::Dict(s.dict.clone())),
        _ => Vec::new(),
    }
}

/// Un filetto magenta chiuso sopra un fondo blu: la fustella fatta in quadricromia.
const HAIRLINE: &str = "0 0 1 rg 0 0 200 100 re f 0 1 0 0 K 4 w 10 10 m 50 10 l 50 50 l 10 50 l h S";

#[test]
fn legge_i_tracciati_con_colore_spessore_e_posizione() {
    let e = editor(&[page([0.0, 0.0, 200.0, 100.0], 0, HAIRLINE)]);
    let scan = e.scan_page(0).unwrap();
    assert_eq!(scan.paths.len(), 2);
    assert!(near(scan.width, pt_to_mm(200.0)) && near(scan.height, pt_to_mm(100.0)));
    let fondo = &scan.paths[0];
    assert!(fondo.line.is_none());
    assert_eq!(fondo.fill.as_ref().unwrap().ink, Ink::Rgb([0.0, 0.0, 1.0]));
    let filetto = &scan.paths[1];
    assert!(filetto.fill.is_none() && filetto.closed() && !filetto.in_form);
    let line = filetto.line.as_ref().unwrap();
    assert_eq!(line.ink, Ink::Cmyk([0.0, 1.0, 0.0, 0.0]));
    assert!(near(line.width, pt_to_mm(4.0)));
    // In millimetri dall'alto a sinistra: y del PDF 10..50 su una pagina alta 100.
    let b = filetto.bounds;
    assert!(near(b[0], pt_to_mm(10.0)) && near(b[1], pt_to_mm(50.0)));
    assert!(near(b[2], pt_to_mm(50.0)) && near(b[3], pt_to_mm(90.0)));
    assert!(near(filetto.length, pt_to_mm(160.0)));
}

#[test]
fn le_pagine_ruotate_si_leggono_come_si_vedono() {
    let e = editor(&[page([0.0, 0.0, 200.0, 100.0], 90, "0 0 0 1 k 0 80 20 20 re f")]);
    let scan = e.scan_page(0).unwrap();
    assert!(near(scan.width, pt_to_mm(100.0)) && near(scan.height, pt_to_mm(200.0)));
    // Ruotata di 90° in senso orario, l'angolo in alto a sinistra va in alto a destra.
    let b = scan.paths[0].bounds;
    assert!(near(b[0], pt_to_mm(80.0)) && near(b[1], 0.0));
    assert!(near(b[2], pt_to_mm(100.0)) && near(b[3], pt_to_mm(20.0)));
}

#[test]
fn il_filetto_diventa_una_fustella_in_tinta() {
    let mut e = editor(&[page([0.0, 0.0, 200.0, 100.0], 0, HAIRLINE)]);
    let before = render(&reread(&e), 0);
    let id = e.scan_page(0).unwrap().paths[1].id;
    let change = Restyle {
        part: Part::Line,
        ink: Some(NewInk::Spot {
            name: "CutContour".into(),
            cmyk: [0.0, 1.0, 0.0, 0.0],
        }),
        width: None,
        overprint: Some(true),
    };
    let r = e.restyle_paths(0, &[id], &change).unwrap();
    assert_eq!((r.done, r.skipped), (1, 0));
    assert_eq!(r.ids, [e.scan_page(0).unwrap().paths[1].id]);
    assert_eq!(e.undo_label(), Some("Tracciati in tinta"));
    let spots = e.spots().unwrap();
    assert_eq!(spots.len(), 1);
    assert_eq!(spots[0].name, "CutContour");
    assert_eq!(spots[0].overprint, Overprint::All);
    let scan = e.scan_page(0).unwrap();
    let line = scan.paths[1].line.as_ref().unwrap();
    assert_eq!(
        line.ink,
        Ink::Spot {
            name: "CutContour".into(),
            tint: 1.0
        }
    );
    assert!(line.overprint && near(line.width, pt_to_mm(4.0)));
    // Il fondo dopo la linea resta senza sovrastampa e com'era.
    assert!(!scan.paths[0].fill.as_ref().unwrap().overprint);
    let img = render(&reread(&e), 0);
    assert_eq!(pixel(&img, 120, 50), pixel(&before, 120, 50));
    // Lo spessore si cambia a parte.
    let thin = Restyle {
        part: Part::Line,
        ink: None,
        width: Some(0.1),
        overprint: None,
    };
    let id = e.scan_page(0).unwrap().paths[1].id;
    assert_eq!(e.restyle_paths(0, &[id], &thin).unwrap().done, 1);
    let line = e.scan_page(0).unwrap().paths[1].line.clone().unwrap();
    assert!(near(line.width, 0.1));
    assert_eq!(
        line.ink,
        Ink::Spot {
            name: "CutContour".into(),
            tint: 1.0
        }
    );
}

#[test]
fn un_tracciato_che_ritaglia_non_si_cambia() {
    let mut e = editor(&[page(
        [0.0, 0.0, 200.0, 100.0],
        0,
        "0 1 0 0 K 10 10 50 50 re W S 0 0 1 rg 0 0 200 100 re f",
    )]);
    let scan = e.scan_page(0).unwrap();
    assert!(scan.paths[0].clips);
    let change = Restyle {
        part: Part::Line,
        ink: Some(NewInk::Cmyk([0.0, 0.0, 0.0, 1.0])),
        width: None,
        overprint: None,
    };
    let r = e.restyle_paths(0, &[scan.paths[0].id], &change).unwrap();
    assert_eq!((r.done, r.skipped), (0, 1));
    assert!(e.undo_label().is_none());
    // Il fondo blu, ritagliato, sta dentro il ritaglio.
    let b = scan.paths[1].bounds;
    assert!(near(b[2] - b[0], 200.0 * 25.4 / 72.0));
    assert!(scan.others.is_empty());
}

#[test]
fn i_tracciati_dei_moduli_si_trovano_e_si_cambiano() {
    let mut p = page([0.0, 0.0, 200.0, 100.0], 0, "q 1 0 0 1 100 0 cm /F Do Q");
    p.resources = "<< /XObject << /F 5 0 R >> >>";
    let form_content = "0 1 0 0 K 1 w 0 0 m 50 50 l S";
    let form = format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 200 100] /Length {} >>\nstream\n{form_content}\nendstream",
        form_content.len() + 1
    );
    let mut e = Editor::new(
        PdfFile::from_bytes(pdf_extra(&[p], &[&form])).unwrap(),
        PathBuf::from("prova.pdf"),
    );
    let scan = e.scan_page(0).unwrap();
    let path = &scan.paths[0];
    assert!(path.in_form && !path.closed());
    assert!(near(path.bounds[0], pt_to_mm(100.0)));
    let change = Restyle {
        part: Part::Line,
        ink: Some(NewInk::Spot {
            name: "Cordonatura".into(),
            cmyk: [0.0, 1.0, 0.0, 0.0],
        }),
        width: None,
        overprint: None,
    };
    assert_eq!(e.restyle_paths(0, &[path.id], &change).unwrap().done, 1);
    assert_eq!(e.spots().unwrap()[0].name, "Cordonatura");
    let img = render(&reread(&e), 0);
    assert_eq!((img.1, img.2), (200, 100));
}

#[test]
fn immagini_e_formato_finito() {
    let mut p = page([0.0, 0.0, 200.0, 100.0], 0, "q 100 0 0 50 0 0 cm /Im Do Q");
    p.trim = Some([10.0, 10.0, 190.0, 90.0]);
    p.resources = "<< /XObject << /Im 5 0 R >> >>";
    let img = "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 1 >>\nstream\n\u{0}\nendstream";
    let e = Editor::new(
        PdfFile::from_bytes(pdf_extra(&[p], &[img])).unwrap(),
        PathBuf::from("prova.pdf"),
    );
    let scan = e.scan_page(0).unwrap();
    let t = scan.trim.unwrap();
    assert!(near(t[0], pt_to_mm(10.0)) && near(t[1], pt_to_mm(10.0)));
    assert_eq!(scan.others.len(), 1);
    let o = scan.others[0];
    assert!(
        near(o[0], 0.0)
            && near(o[1], pt_to_mm(50.0))
            && near(o[2], pt_to_mm(100.0))
            && near(o[3], pt_to_mm(100.0))
    );
}

#[test]
fn colore_in_quadricromia_sul_riempimento() {
    let mut e = editor(&[page([0.0, 0.0, 200.0, 100.0], 0, HAIRLINE)]);
    let id = e.scan_page(0).unwrap().paths[0].id;
    let change = Restyle {
        part: Part::Fill,
        ink: Some(NewInk::Cmyk([0.0, 0.0, 0.0, 1.0])),
        width: None,
        overprint: None,
    };
    assert_eq!(e.restyle_paths(0, &[id], &change).unwrap().done, 1);
    assert_eq!(e.undo_label(), Some("Colore dei tracciati"));
    let scan = e.scan_page(0).unwrap();
    assert_eq!(
        scan.paths[0].fill.as_ref().unwrap().ink,
        Ink::Cmyk([0.0, 0.0, 0.0, 1.0])
    );
    // Il filetto dopo il fondo resta magenta.
    assert_eq!(
        scan.paths[1].line.as_ref().unwrap().ink,
        Ink::Cmyk([0.0, 1.0, 0.0, 0.0])
    );
    let img = render(&reread(&e), 0);
    // Il nero in quadricromia a schermo è un grigio molto scuro (conversione CMYK).
    assert!(pixel(&img, 120, 50).iter().all(|v| *v < 60));
}

/// Striscia rossa larga 5 punti a sinistra, il resto verde, e una linea magenta (la fustella
/// finta) 3 punti dentro il bordo sinistro, su una pagina 200 × 100 senza abbondanza.
const NO_BLEED: &str = "0 1 0 rg 0 0 200 100 re f 1 0 0 rg 0 0 5 100 re f 0 1 0 0 K 1 w 3 20 m 3 80 l S";

fn bleed_spec(mode: BleedMode, sides: [bool; 4]) -> BleedSpec {
    BleedSpec {
        finished: [0.0, 0.0, pt_to_mm(200.0), pt_to_mm(100.0)],
        amount: pt_to_mm(10.0),
        sides,
        mode,
    }
}

#[test]
fn abbondanza_a_specchio_con_formato_finito() {
    let mut e = editor(&[page([0.0, 0.0, 200.0, 100.0], 0, NO_BLEED)]);
    let left = e
        .add_bleed(0, &bleed_spec(BleedMode::Mirror, [true; 4]), &[], &[], &[])
        .unwrap();
    assert_eq!(left, 0);
    assert_eq!(e.undo_label(), Some("Crea abbondanza"));
    let info = e.pages();
    assert!(near(info[0].size.w, pt_to_mm(220.0)) && near(info[0].size.h, pt_to_mm(120.0)));
    let trim = info[0].trim.unwrap();
    assert!(near(trim.w, pt_to_mm(200.0)) && near(trim.h, pt_to_mm(100.0)));
    let img = render(&reread(&e), 0);
    // A sinistra del taglio (x = 10) la grafica ribaltata attorno a x = 11 (un punto dentro, per
    // non lasciare fili nell'anteprima): 2 → 10 dentro (verde), 7 → 5 (rossa).
    assert_eq!(pixel(&img, 2, 60), [0, 255, 0]);
    assert_eq!(pixel(&img, 7, 60), [255, 0, 0]);
    // Sopra e sotto la grafica continua; dentro la pagina è com'era.
    assert_eq!(pixel(&img, 100, 3), [0, 255, 0]);
    assert_eq!(pixel(&img, 12, 3), [255, 0, 0]);
    assert_eq!(pixel(&img, 100, 60), [0, 255, 0]);
    // La fustella finta è ribaltata anche lei (x = 9), perché non è stata tenuta fuori.
    assert!(pixel(&img, 8, 60)[1].min(pixel(&img, 9, 60)[1]) < 200);
    // Il formato finito si legge come TrimBox nella pagina riletta.
    let scan = e.scan_page(0).unwrap();
    let t = scan.trim.unwrap();
    assert!(near(t[0], pt_to_mm(10.0)) && near(t[2], pt_to_mm(210.0)));
}

#[test]
fn la_fustella_non_si_ribalta_nell_abbondanza() {
    let mut e = editor(&[page([0.0, 0.0, 200.0, 100.0], 0, NO_BLEED)]);
    let die = e.scan_page(0).unwrap().paths[2].id;
    let left = e
        .add_bleed(0, &bleed_spec(BleedMode::Mirror, [true; 4]), &[die], &[], &[])
        .unwrap();
    assert_eq!(left, 0);
    let img = render(&reread(&e), 0);
    // Nell'abbondanza al posto della linea c'è la grafica sotto (rossa); la linea vera resta.
    assert_eq!(pixel(&img, 8, 60), [255, 0, 0]);
    assert_eq!(pixel(&img, 9, 60), [255, 0, 0]);
    assert!(pixel(&img, 13, 60)[1] < 100);
    let scan = e.scan_page(0).unwrap();
    assert!(scan.paths.iter().filter(|p| p.line.is_some()).count() >= 1);
}

#[test]
fn abbondanza_allungata_e_solo_su_un_lato() {
    let mut e = editor(&[page([0.0, 0.0, 200.0, 100.0], 0, NO_BLEED)]);
    e.add_bleed(
        0,
        &bleed_spec(BleedMode::Stretch, [false, false, false, true]),
        &[],
        &[],
        &[],
    )
    .unwrap();
    let info = e.pages();
    assert!(near(info[0].size.w, pt_to_mm(210.0)) && near(info[0].size.h, pt_to_mm(100.0)));
    let img = render(&reread(&e), 0);
    // Il bordo rosso si allunga per tutta l'abbondanza.
    assert_eq!(pixel(&img, 1, 50), [255, 0, 0]);
    assert_eq!(pixel(&img, 8, 50), [255, 0, 0]);
    assert_eq!(pixel(&img, 150, 50), [0, 255, 0]);
}

#[test]
fn abbondanza_non_valida() {
    let mut e = editor(&[page([0.0, 0.0, 200.0, 100.0], 0, NO_BLEED)]);
    let mut spec = bleed_spec(BleedMode::Mirror, [false; 4]);
    assert!(e.add_bleed(0, &spec, &[], &[], &[]).is_err());
    spec.sides = [true; 4];
    spec.amount = f64::NAN;
    assert!(e.add_bleed(0, &spec, &[], &[], &[]).is_err());
    spec.amount = 3.0;
    spec.finished = [10.0, 10.0, 10.2, 10.2];
    assert!(e.add_bleed(0, &spec, &[], &[], &[]).is_err());
    assert_eq!(e.undo_label(), None);
}

#[test]
fn abbondanza_su_una_pagina_ruotata() {
    // 100 × 200 ruotata di 90°: si vede 200 × 100. Il quadrato rosso in basso a sinistra del
    // file finisce in alto a sinistra come si vede.
    let mut e = editor(&[page(
        [0.0, 0.0, 100.0, 200.0],
        90,
        "0 1 0 rg 0 0 100 200 re f 1 0 0 rg 0 0 20 20 re f",
    )]);
    e.add_bleed(0, &bleed_spec(BleedMode::Mirror, [true; 4]), &[], &[], &[])
        .unwrap();
    let info = e.pages();
    assert!(near(info[0].size.w, pt_to_mm(220.0)) && near(info[0].size.h, pt_to_mm(120.0)));
    assert_eq!(info[0].rotation, 0);
    let img = render(&reread(&e), 0);
    // Il rosso in alto a sinistra si ribalta anche nell'angolo dell'abbondanza.
    assert_eq!(pixel(&img, 15, 15), [255, 0, 0]);
    assert_eq!(pixel(&img, 5, 5), [255, 0, 0]);
    assert_eq!(pixel(&img, 215, 115), [0, 255, 0]);
}

#[test]
fn nessun_filo_bianco_tra_abbondanza_e_pagina() {
    let mut e = editor(&[page([0.0, 0.0, 200.0, 100.0], 0, "0 1 0 rg 0 0 200 100 re f")]);
    // Il bordo del formato finito cade a metà di un pixel (10,5 punti).
    let mut spec = bleed_spec(BleedMode::Mirror, [true; 4]);
    spec.amount = pt_to_mm(10.5);
    e.add_bleed(0, &spec, &[], &[], &[]).unwrap();
    let img = render(&reread(&e), 0);
    for (x, y) in [(10, 60), (210, 60), (100, 10), (100, 110), (10, 10)] {
        assert_eq!(pixel(&img, x, y), [0, 255, 0], "({x}, {y})");
    }
}

#[test]
fn lavoro_piccolo_su_fondo_bianco_e_fustella_sul_bordo() {
    // Fondo bianco su tutta la pagina, un rettangolo verde 100 × 50 da (50, 25), una fustella
    // magenta spessa 4 proprio sul suo bordo e un segno di taglio nero più in là.
    let mut e = editor(&[page(
        [0.0, 0.0, 200.0, 100.0],
        0,
        "1 1 1 rg 0 0 200 100 re f 0 1 0 rg 50 25 100 50 re f \
         0 1 0 0 K 4 w 50 25 100 50 re S 0 0 0 1 K 0.5 w 30 25 m 42 25 l S",
    )]);
    let die = e.scan_page(0).unwrap().paths[2].id;
    let spec = BleedSpec {
        finished: [pt_to_mm(50.0), pt_to_mm(25.0), pt_to_mm(150.0), pt_to_mm(75.0)],
        amount: pt_to_mm(10.0),
        sides: [true; 4],
        mode: BleedMode::Mirror,
    };
    e.add_bleed(0, &spec, &[die], &[], &[]).unwrap();
    let info = e.pages();
    assert!(near(info[0].size.w, pt_to_mm(120.0)) && near(info[0].size.h, pt_to_mm(70.0)));
    let img = render(&reread(&e), 0);
    // L'abbondanza è verde fino al bordo: niente bianco del fondo, niente segno di taglio.
    for (x, y) in [(3, 35), (6, 35), (60, 3), (116, 66), (2, 2)] {
        assert_eq!(pixel(&img, x, y), [0, 255, 0], "({x}, {y})");
    }
    // La fustella resta intera: anche la metà fuori dal formato finito (x = 9 → 1 punto fuori).
    assert!(pixel(&img, 9, 35)[1] < 100);
    assert!(pixel(&img, 11, 35)[1] < 100);
}

#[test]
fn nessun_filo_bianco_con_il_fondo_bianco_dietro() {
    // Il lavoro verde sta su un fondo bianco e il suo bordo cade a metà di un pixel.
    let mut e = editor(&[page(
        [0.0, 0.0, 200.0, 100.0],
        0,
        "1 1 1 rg 0 0 200 100 re f 0 1 0 rg 50.5 25.5 100 50 re f",
    )]);
    let spec = BleedSpec {
        finished: [pt_to_mm(50.5), pt_to_mm(24.5), pt_to_mm(150.5), pt_to_mm(74.5)],
        amount: pt_to_mm(10.0),
        sides: [true; 4],
        mode: BleedMode::Mirror,
    };
    let paper = e.scan_page(0).unwrap().paths[0].id;
    e.add_bleed(0, &spec, &[], &[paper], &[]).unwrap();
    let img = render(&reread(&e), 0);
    for (x, y) in [
        (10, 35),
        (11, 35),
        (110, 35),
        (60, 10),
        (60, 11),
        (60, 60),
        (10, 10),
        (110, 60),
    ] {
        assert_eq!(pixel(&img, x, y), [0, 255, 0], "({x}, {y})");
    }
}

/// Come il file di prova di Michel: un fondo rosso, una cornice ciano spessa sul suo bordo (la
/// fustella fatta a mano) e una stella gialla, su una pagina 200 × 100.
const BACKGROUND_AND_FRAME: &str = "0 1 1 0 k 10 10 180 80 re f 1 0 0 0 K 4 w 10 10 180 80 re S \
     0 0 1 0 K 1 w 90 30 m 110 30 l 100 70 l h S";

#[test]
fn abbondanza_solo_dello_sfondo_scelto() {
    let mut e = editor(&[page([0.0, 0.0, 200.0, 100.0], 0, BACKGROUND_AND_FRAME)]);
    let red = e.scan_page(0).unwrap().paths[0].id;
    let spec = BleedSpec {
        finished: [pt_to_mm(10.0), pt_to_mm(10.0), pt_to_mm(190.0), pt_to_mm(90.0)],
        amount: pt_to_mm(8.0),
        sides: [true; 4],
        mode: BleedMode::Mirror,
    };
    e.add_bleed(0, &spec, &[], &[], &[red]).unwrap();
    let info = e.pages();
    assert!(near(info[0].size.w, pt_to_mm(196.0)) && near(info[0].size.h, pt_to_mm(96.0)));
    let img = render(&reread(&e), 0);
    // Il rosso arriva al bordo nuovo; la cornice ciano resta una sola, al suo posto (x = 8 → 6..10).
    let red_px = pixel(&img, 1, 48);
    assert!(red_px[0] > 200 && red_px[1] < 80, "{red_px:?}");
    let frame_px = pixel(&img, 8, 48);
    assert!(frame_px[0] < 80 && frame_px[2] > 150, "{frame_px:?}");
    // Nessuna seconda cornice ribaltata nell'abbondanza.
    let inside_bleed = pixel(&img, 3, 48);
    assert!(inside_bleed[0] > 200 && inside_bleed[2] < 80, "{inside_bleed:?}");
    // La stella e la cornice sono ancora tracciati che si possono scegliere.
    let scan = e.scan_page(0).unwrap();
    assert!(scan.paths.iter().any(|p| {
        p.line
            .as_ref()
            .is_some_and(|l| l.ink == Ink::Cmyk([0.0, 0.0, 1.0, 0.0]))
    }));
}

#[test]
fn fustella_dall_unione_dei_tracciati_scelti() {
    // Due rettangoli che si sovrappongono e uno staccato.
    let mut e = editor(&[page(
        [0.0, 0.0, 200.0, 100.0],
        0,
        "0 0 1 rg 10 10 50 50 re f 40 40 50 50 re f 150 10 20 20 re f",
    )]);
    let ids: Vec<PathId> = e.scan_page(0).unwrap().paths.iter().map(|p| p.id).collect();
    let die = NewDie {
        name: "Taglio".into(),
        cmyk: [0.0, 1.0, 0.0, 0.0],
        width: 0.1,
        overprint: true,
    };
    assert_eq!(e.add_die(0, &ids, &die).unwrap(), 2);
    assert_eq!(e.undo_label(), Some("Crea fustella"));
    let scan = e.scan_page(0).unwrap();
    assert_eq!(scan.paths.len(), 4);
    let cut = &scan.paths[3];
    let line = cut.line.as_ref().unwrap();
    assert_eq!(
        line.ink,
        Ink::Spot {
            name: "Taglio".into(),
            tint: 1.0
        }
    );
    assert!(line.overprint && near(line.width, 0.1) && cut.fill.is_none());
    assert_eq!(cut.parts.len(), 2);
    // L'unione dei primi due: da (10, 10) a (90, 90) punti, cioè in mm dall'alto.
    let b = cut.bounds;
    assert!(near(b[0], pt_to_mm(10.0)) && near(b[2], pt_to_mm(170.0)));
    assert!(near(b[1], pt_to_mm(10.0)) && near(b[3], pt_to_mm(90.0)));
    // Il contorno unito ha 8 vertici (una L doppia), non due quadrati sovrapposti.
    let big = cut.parts.iter().max_by_key(|p| p.points.len()).unwrap();
    assert_eq!(big.points.len(), 8);
    assert!(e.spots().unwrap().iter().any(|s| s.name == "Taglio"));
    // Niente area chiusa: errore e nessuna modifica.
    let mut e = editor(&[page([0.0, 0.0, 200.0, 100.0], 0, "0 0 0 1 K 10 10 m 50 50 l S")]);
    let ids: Vec<PathId> = e.scan_page(0).unwrap().paths.iter().map(|p| p.id).collect();
    assert!(e.add_die(0, &ids, &die).is_err());
    assert_eq!(e.undo_label(), None);
}

#[test]
fn fustella_su_una_pagina_ruotata() {
    let mut e = editor(&[page([0.0, 0.0, 100.0, 200.0], 90, "0 0 1 rg 10 20 30 40 re f")]);
    let before = e.scan_page(0).unwrap().paths[0].bounds;
    let ids = [e.scan_page(0).unwrap().paths[0].id];
    let die = NewDie {
        name: "Taglio".into(),
        cmyk: [0.0, 1.0, 0.0, 0.0],
        width: 0.1,
        overprint: true,
    };
    e.add_die(0, &ids, &die).unwrap();
    let after = e.scan_page(0).unwrap().paths[1].bounds;
    for (a, b) in after.iter().zip(before) {
        assert!(near(*a, b), "{after:?} {before:?}");
    }
}
