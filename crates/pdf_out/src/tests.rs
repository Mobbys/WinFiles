use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use winfiles_geom::Size;
use winfiles_model::FileKind;

use super::*;

const FONTS: Fonts = Fonts {
    regular: include_bytes!("../../../apps/winfiles/assets/fonts/Inter-Regular.ttf"),
    semibold: include_bytes!("../../../apps/winfiles/assets/fonts/Inter-SemiBold.ttf"),
};

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("winfiles-pdfout-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Un PDF di prova di una pagina A5 tutta rossa, scritto con lo stesso krilla.
fn red_pdf() -> Vec<u8> {
    let mut doc = Document::new();
    let mut page = doc.start_page_with(PageSettings::new(KSize::from_wh(420.0, 595.0).unwrap()));
    let mut s = page.surface();
    s.set_fill(Some(fill(Color::rgb(220, 0, 0))));
    s.draw_path(&rounded(Rect::new(0.0, 0.0, 148.0, 210.0), 0.0).unwrap());
    s.finish();
    page.finish();
    doc.finish().unwrap()
}

fn scene(pdf: &Path, png: &Path) -> Page {
    let mut page = Page::new(Size::new(210.0, 297.0));
    page.push(Item::Rect {
        rect: Rect::new(10.0, 10.0, 190.0, 15.0),
        radius: 2.0,
        fill: Some(Color::rgb(246, 247, 249)),
        stroke: Some((Color::rgb(85, 85, 85), 0.35)),
    });
    page.push(Item::Text(Text {
        x: 14.0,
        baseline: 20.0,
        size: 5.0,
        weight: Weight::SemiBold,
        color: Color::rgb(20, 20, 20),
        text: "Cliente Rossi, ordine 124 · 0,062 m²".into(),
        width: 100.0,
    }));
    page.push(Item::Line {
        from: (10.0, 30.0),
        to: (200.0, 30.0),
        color: Color::rgb(190, 190, 190),
        width: 0.25,
    });
    page.push(Item::Picture(Picture {
        rect: Rect::new(10.0, 40.0, 60.0, 85.0),
        source: Source {
            path: pdf.to_path_buf(),
            kind: FileKind::Pdf,
            page: 0,
        },
        crop: Rect::new(0.0, 0.0, 1.0, 1.0),
        dpi: None,
    }));
    page.push(Item::Picture(Picture {
        rect: Rect::new(80.0, 40.0, 60.0, 40.0),
        source: Source {
            path: png.to_path_buf(),
            kind: FileKind::Image,
            page: 0,
        },
        crop: Rect::new(0.0, 0.0, 1.0, 1.0),
        dpi: None,
    }));
    page
}

#[test]
fn un_foglio_con_testi_vettori_e_immagini() {
    let d = dir("ok");
    let pdf = d.join("rosso.pdf");
    std::fs::write(&pdf, red_pdf()).unwrap();
    let png = d.join("blu.png");
    image::RgbImage::from_pixel(300, 200, image::Rgb([0, 0, 200]))
        .save(&png)
        .unwrap();
    let pages = vec![scene(&pdf, &png), Page::new(Size::new(420.0, 297.0))];
    let mut seen = 0;
    let (bytes, report) = write(&pages, &FONTS, &AtomicBool::new(false), |n| seen = n).unwrap();
    assert_eq!((report.pages, seen), (2, 2));
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);

    // Il PDF si rilegge, e le misure sono quelle del foglio.
    let out = d.join("uscita.pdf");
    save(&out, &bytes).unwrap();
    let probe = winfiles_formats_like_size(&out);
    assert_eq!(probe.len(), 2);
    assert!((probe[0].0 - 210.0).abs() < 0.1 && (probe[0].1 - 297.0).abs() < 0.1);
    assert!((probe[1].0 - 420.0).abs() < 0.1);

    // Disegnata, la pagina ha il rosso del PDF incorporato e il blu dell'immagine al loro posto.
    let thumb = winfiles_thumbs::render(&out, FileKind::Pdf, 0, 840).unwrap();
    let px = |x_mm: f64, y_mm: f64| {
        let k = f64::from(thumb.width) / 210.0;
        let (x, y) = ((x_mm * k) as u32, (y_mm * k) as u32);
        let i = ((y * thumb.width + x) * 4) as usize;
        (thumb.rgba[i], thumb.rgba[i + 1], thumb.rgba[i + 2])
    };
    let red = px(40.0, 80.0);
    assert!(red.0 > 180 && red.1 < 60, "{red:?}");
    let blue = px(110.0, 60.0);
    assert!(blue.2 > 150 && blue.0 < 60, "{blue:?}");
    let white = px(150.0, 200.0);
    assert!(white.0 > 240 && white.2 > 240, "{white:?}");
    std::fs::remove_dir_all(&d).unwrap();
}

/// Le misure delle pagine di un PDF in mm, lette come le legge l'app.
fn winfiles_formats_like_size(path: &Path) -> Vec<(f64, f64)> {
    winfiles_formats::probe(path, FileKind::Pdf)
        .unwrap()
        .pages
        .iter()
        .map(|p| (p.size.w, p.size.h))
        .collect()
}

#[test]
fn le_miniature_pesano_poco_anche_se_il_file_e_grande() {
    // Una foto da 3000 × 2000 pixel stampata in una miniatura di 60 × 40 mm: a 150 DPI bastano
    // circa 354 pixel di lato, e il PDF resta leggero.
    let d = dir("peso");
    let big = d.join("grande.png");
    image::RgbImage::from_fn(3000, 2000, |x, y| {
        image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x ^ y) % 239) as u8])
    })
    .save(&big)
    .unwrap();
    let mut page = Page::new(Size::new(210.0, 297.0));
    page.push(Item::Picture(Picture {
        rect: Rect::new(10.0, 10.0, 60.0, 40.0),
        source: Source {
            path: big.clone(),
            kind: FileKind::Image,
            page: 0,
        },
        crop: Rect::new(0.0, 0.0, 1.0, 1.0),
        dpi: None,
    }));
    let (bytes, report) = write(&[page], &FONTS, &AtomicBool::new(false), |_| {}).unwrap();
    assert!(report.warnings.is_empty());
    assert!(bytes.len() < 200_000, "{} byte", bytes.len());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn un_file_mancante_lascia_un_riquadro_e_un_avviso() {
    let mut page = Page::new(Size::new(210.0, 297.0));
    let missing = PathBuf::from("/non/esiste/foto.jpg");
    page.push(Item::Picture(Picture {
        rect: Rect::new(10.0, 10.0, 50.0, 50.0),
        source: Source {
            path: missing.clone(),
            kind: FileKind::Image,
            page: 0,
        },
        crop: Rect::new(0.0, 0.0, 1.0, 1.0),
        dpi: None,
    }));
    // Anche un PDF che non c'è o una pagina oltre l'ultima non devono far fallire tutto.
    page.push(Item::Picture(Picture {
        rect: Rect::new(70.0, 10.0, 50.0, 50.0),
        source: Source {
            path: PathBuf::from("/non/esiste/x.pdf"),
            kind: FileKind::Pdf,
            page: 7,
        },
        crop: Rect::new(0.0, 0.0, 1.0, 1.0),
        dpi: None,
    }));
    let (bytes, report) = write(&[page], &FONTS, &AtomicBool::new(false), |_| {}).unwrap();
    assert!(!bytes.is_empty());
    assert_eq!(report.warnings.len(), 2);
    assert_eq!(report.warnings[0].0, missing);
}

#[test]
fn si_puo_annullare() {
    let pages = vec![Page::new(Size::new(210.0, 297.0)); 3];
    let r = write(&pages, &FONTS, &AtomicBool::new(true), |_| {});
    assert!(matches!(r, Err(PdfOutError::Cancelled)));
}

#[test]
fn la_correzione_della_stampante_sposta_e_allunga_il_foglio() {
    let d = dir("calibrazione");
    let mut page = Page::new(Size::new(210.0, 297.0));
    page.push(Item::Rect {
        rect: Rect::new(100.0, 100.0, 10.0, 10.0),
        radius: 0.0,
        fill: Some(Color::rgb(0, 0, 0)),
        stroke: None,
    });
    let mut moved = page.clone();
    // Il 5% più largo e 20 mm più in basso.
    moved.calibration = winfiles_geom::Calibration {
        scale_x: 1.05,
        scale_y: 1.0,
        offset_x: 0.0,
        offset_y: 20.0,
    };
    let render = |page: Page, name: &str| {
        let (bytes, _) = write(&[page], &FONTS, &AtomicBool::new(false), |_| {}).unwrap();
        let out = d.join(name);
        save(&out, &bytes).unwrap();
        winfiles_thumbs::render(&out, FileKind::Pdf, 0, 840).unwrap()
    };
    let dark = |thumb: &winfiles_thumbs::Thumb, x_mm: f64, y_mm: f64| {
        let k = f64::from(thumb.width) / 210.0;
        let (x, y) = ((x_mm * k) as u32, (y_mm * k) as u32);
        let i = ((y * thumb.width + x) * 4) as usize;
        thumb.rgba[i] < 80
    };
    let plain = render(page, "senza.pdf");
    assert!(dark(&plain, 105.0, 105.0));
    assert!(!dark(&plain, 105.0, 125.0));
    let fixed = render(moved, "con.pdf");
    // Il quadrato ora va da 105 a 115,5 in x e da 120 a 130 in y; la misura del foglio non cambia.
    assert!(!dark(&fixed, 105.0, 105.0));
    assert!(dark(&fixed, 106.0, 125.0) && dark(&fixed, 114.8, 125.0));
    assert!(!dark(&fixed, 116.5, 125.0));
    let probe = winfiles_formats_like_size(&d.join("con.pdf"));
    assert!((probe[0].0 - 210.0).abs() < 0.1);
    std::fs::remove_dir_all(&d).unwrap();
}
