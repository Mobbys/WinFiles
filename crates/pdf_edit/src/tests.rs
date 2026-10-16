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
