use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use winfiles_geom::Size;

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
    }));
    page.push(Item::Picture(Picture {
        rect: Rect::new(80.0, 40.0, 60.0, 40.0),
        source: Source {
            path: png.to_path_buf(),
            kind: FileKind::Image,
            page: 0,
        },
        crop: Rect::new(0.0, 0.0, 1.0, 1.0),
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

/// Le misure delle pagine di un PDF in mm, lette con lo stesso lettore delle miniature.
fn winfiles_formats_like_size(path: &Path) -> Vec<(f64, f64)> {
    let bytes = std::fs::read(path).unwrap();
    let pdf = Pdf::new(bytes).unwrap();
    pdf.pages()
        .iter()
        .map(|p| {
            let (w, h) = p.render_dimensions();
            (f64::from(w) * 25.4 / 72.0, f64::from(h) * 25.4 / 72.0)
        })
        .collect()
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
