use std::path::PathBuf;

use winfiles_model::FileKind;

use crate::{ThumbError, render};

/// Un file temporaneo che si cancella da solo.
struct Temp(PathBuf);

impl Temp {
    fn new(name: &str, bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!("winfiles-thumbs-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        Temp(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, _| image::Rgb([if x < w / 2 { 255 } else { 0 }, 0, 0]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// PDF di una pagina 200 × 100 pt con un rettangolo nero sulla metà sinistra.
fn pdf() -> Vec<u8> {
    pdf_page("0 0 0 rg 0 0 100 100 re f", 200, "")
}

/// PDF di una pagina larga `width` pt e alta 100 con il contenuto e le risorse dati.
fn pdf_page(content: &str, width: u32, resources: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} 100] /Resources << {resources} >> /Contents 4 0 R >>"
        ),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
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

#[test]
fn image_is_scaled_down_keeping_proportions() {
    let f = Temp::new("a.png", &png(400, 200));
    let t = render(&f.0, FileKind::Image, 0, 100).unwrap();
    assert_eq!((t.width, t.height), (100, 50));
    assert_eq!(t.rgba.len(), 100 * 50 * 4);
    // Metà sinistra rossa, metà destra nera.
    assert_eq!(&t.rgba[..4], &[255, 0, 0, 255]);
    let right = ((25 * 100 + 90) * 4) as usize;
    assert_eq!(&t.rgba[right..right + 3], &[0, 0, 0]);
}

#[test]
fn small_images_are_not_enlarged() {
    let f = Temp::new("b.png", &png(20, 10));
    let t = render(&f.0, FileKind::Image, 0, 512).unwrap();
    assert_eq!((t.width, t.height), (20, 10));
}

#[test]
fn pdf_first_page_is_rendered_on_white() {
    let f = Temp::new("c.pdf", &pdf());
    let t = render(&f.0, FileKind::Pdf, 0, 200).unwrap();
    assert_eq!((t.width, t.height), (200, 100));
    let px = |x: u32, y: u32| {
        let i = ((y * t.width + x) * 4) as usize;
        t.rgba[i..i + 4].to_vec()
    };
    assert_eq!(px(10, 50), vec![0, 0, 0, 255]);
    assert_eq!(px(190, 50), vec![255, 255, 255, 255]);
}

#[test]
fn damaged_files_are_errors() {
    let f = Temp::new("d.jpg", b"\xFF\xD8\xFF\xE0 non e' un jpeg");
    assert!(render(&f.0, FileKind::Image, 0, 100).is_err());
    let f = Temp::new("e.pdf", b"%PDF-1.4 rotto");
    assert!(render(&f.0, FileKind::Pdf, 0, 100).is_err());
    let f = Temp::new("f.eps", b"%!PS-Adobe-3.0 EPSF-3.0\n");
    assert!(matches!(
        render(&f.0, FileKind::Eps, 0, 100),
        Err(ThumbError::NoPreview)
    ));
    assert!(matches!(
        render(&PathBuf::from("/non/esiste.png"), FileKind::Image, 0, 100),
        Err(ThumbError::Io(_))
    ));
}

#[test]
fn pages_beyond_the_last_are_errors_not_crashes() {
    let f = Temp::new("g.pdf", &pdf());
    assert!(matches!(
        render(&f.0, FileKind::Pdf, 1, 100),
        Err(ThumbError::NoPage(2))
    ));
    let f = Temp::new("h.png", &png(4, 4));
    assert!(matches!(
        render(&f.0, FileKind::Image, 3, 100),
        Err(ThumbError::NoPage(4))
    ));
}

#[test]
fn un_file_mancante_si_dice_in_italiano() {
    let e = render(std::path::Path::new("/non/esiste/x.jpg"), FileKind::Image, 0, 64).unwrap_err();
    assert_eq!(e.to_string(), "file non trovato");
}

#[test]
fn la_sovrastampa_si_vede_come_in_stampa() {
    // Ciano a sinistra (0-200), magenta a destra (100-300) sopra; un quadrato bianco sul ciano.
    let draw = |gs: &str| {
        format!("1 0 0 0 k 0 0 200 100 re f {gs} 0 1 0 0 k 100 0 200 100 re f 0 0 0 0 k 20 20 40 40 re f")
    };
    let resources = "/ExtGState << /Sovra << /OP true /op true /OPM 1 >> >>";
    let px = |t: &crate::Thumb, x: u32| {
        let i = ((50 * t.width + x) * 4) as usize;
        (t.rgba[i], t.rgba[i + 1], t.rgba[i + 2])
    };
    // Senza sovrastampa il magenta copre il ciano e il bianco copre tutto.
    let f = Temp::new("coprente.pdf", &pdf_page(&draw(""), 300, resources));
    let t = render(&f.0, FileKind::Pdf, 0, 300).unwrap();
    let (r, g, b) = px(&t, 150);
    assert!(r > 200 && g < 80 && b > 100, "magenta coprente: {:?}", (r, g, b));
    assert!(
        px(&t, 40).0 > 240 && px(&t, 40).1 > 240,
        "bianco coprente: {:?}",
        px(&t, 40)
    );
    // In sovrastampa ciano e magenta si sommano in blu, e il bianco sparisce.
    let f = Temp::new("sovrastampa.pdf", &pdf_page(&draw("/Sovra gs"), 300, resources));
    let t = render(&f.0, FileKind::Pdf, 0, 300).unwrap();
    let (r, g, b) = px(&t, 150);
    assert!(
        r < 80 && g < 120 && b > 120,
        "ciano e magenta in sovrastampa: {:?}",
        (r, g, b)
    );
    assert_eq!(px(&t, 40), px(&t, 90), "il bianco in sovrastampa sparisce");
    // Il magenta da solo resta magenta.
    let (r, g, _) = px(&t, 250);
    assert!(r > 200 && g < 80);
}

#[test]
fn foto_heic() {
    // Tre strisce rosso, verde e blu (testdata/heic, foto di prova di heic-rs).
    let f = Temp::new(
        "strisce.heic",
        include_bytes!("../../../testdata/heic/rgb-strips-96.heic"),
    );
    let t = render(&f.0, FileKind::Image, 0, 48).unwrap();
    assert_eq!((t.width, t.height), (48, 16));
    let px = |x: u32, y: u32| {
        let i = ((y * t.width + x) * 4) as usize;
        t.rgba[i..i + 4].to_vec()
    };
    for (x, want) in [(8, [255, 0, 0]), (24, [0, 255, 0]), (40, [0, 0, 255])] {
        let got = px(x, 8);
        for c in 0..3 {
            assert!(got[c].abs_diff(want[c]) < 40, "x {x}: {got:?}");
        }
    }
    // A tessere: 1024×1024 ridotta.
    let f = Temp::new(
        "scacchi.heic",
        include_bytes!("../../../testdata/heic/checker-1024.heic"),
    );
    let t = render(&f.0, FileKind::Image, 0, 256).unwrap();
    assert_eq!((t.width, t.height), (256, 256));
    // Rovinata: un errore, mai un crash.
    let f = Temp::new(
        "rotta.heic",
        &include_bytes!("../../../testdata/heic/checker-1024.heic")[..900],
    );
    assert!(render(&f.0, FileKind::Image, 0, 256).is_err());
}

/// Un AI in PostScript come lo scrive Illustrator nel formato "Illustrator 3", pagina 200 × 100 pt
/// spostata in coordinate negative come nei file veri: a sinistra un quadrato ciano con un buco
/// (tracciato composto), a destra un quadrato in tinta piatta magenta al 50%.
fn legacy_ai() -> Vec<u8> {
    let art = "%!PS-Adobe-3.0 \r\n%%Creator: Adobe Illustrator(TM) 3.2\r\n\
        %%BoundingBox: -300 100 -100 200\r\n%AI3_ColorUsage: Color\r\n\
        %%IncludeResource: procset Adobe_IllustratorA_AI3 1.0 1\r\n%%EndProlog\r\n%%BeginSetup\r\n\
        Adobe_IllustratorA_AI3 /initialize get exec\r\n%%EndSetup\r\n0 A\r\nu\r\n*u\r\n\
        1 0 0 0 k\r\n0 J 0 j 1 w 10 M []0 d\r-290 110 m\r-290 190 L\r-210 190 L\r-210 110 L\r\n\
        f\r\n1 D\r\n-270 130 m\r\n-230 130 L\r\n-230 170 L\r\n-270 170 L\r\nf\r\n*U\r\n\
        0 1 0 0 (Rosso marca) 0.5 x\r\n-190 110 m -110 110 l -110 190 l -190 190 l f\r\nU\r\n\
        (testo ignorato) 0 To 0 0 0 1 k 1 0 0 1 -150 150 Tp TP (Ciao) Tx TO\r\n\
        %%PageTrailer\r\nshowpage\r\n%%Trailer\r\n%%EOF\r\n";
    art.as_bytes().to_vec()
}

#[test]
fn ai_in_postscript_si_disegna() {
    let f = Temp::new("legacy.ai", &legacy_ai());
    let t = render(&f.0, FileKind::Ai, 0, 200).unwrap();
    assert_eq!((t.width, t.height), (200, 100));
    let px = |x: u32, y: u32| {
        let i = ((y * t.width + x) * 4) as usize;
        t.rgba[i..i + 3].to_vec()
    };
    let near = |a: Vec<u8>, b: [u8; 3]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) < 40);
    // Pieno del quadrato ciano, buco bianco, tinta al 50% rosa, fondo bianco, testo ignorato.
    assert!(!near(px(15, 50), [255, 255, 255]), "{:?}", px(15, 50));
    assert!(px(15, 50)[0] < 80 && px(15, 50)[2] > 180, "{:?}", px(15, 50));
    assert!(near(px(50, 50), [255, 255, 255]), "{:?}", px(50, 50));
    let tint = px(150, 50);
    assert!(tint[0] > 200 && tint[1] < 200 && tint[1] > 60, "{tint:?}");
    assert!(near(px(105, 5), [255, 255, 255]));
}

#[test]
fn eps_di_illustrator_senza_anteprima_e_altri_postscript() {
    // Con l'intestazione DOS e nessuna anteprima: si disegna lo stesso.
    let ps = legacy_ai();
    let mut dos = vec![0xC5, 0xD0, 0xD3, 0xC6];
    dos.extend(30u32.to_le_bytes());
    dos.extend((ps.len() as u32).to_le_bytes());
    dos.extend([0; 18]);
    dos.extend(&ps);
    let f = Temp::new("illustrator.eps", &dos);
    assert_eq!(
        render(&f.0, FileKind::Eps, 0, 100).map(|t| t.width).ok(),
        Some(100)
    );
    // Un PostScript di un altro programma resta senza anteprima.
    let other = b"%!PS-Adobe-3.0 EPSF-3.0\n%%Creator: CorelDRAW\n%%BoundingBox: 0 0 10 10\n0 0 m 10 10 l S\n";
    let f = Temp::new("corel.eps", other);
    assert!(matches!(
        render(&f.0, FileKind::Eps, 0, 100),
        Err(ThumbError::NoPreview)
    ));
}

#[test]
fn ai_in_postscript_rovinati_non_bloccano() {
    let good = legacy_ai();
    for cut in [20, 120, 300, good.len() - 30] {
        let f = Temp::new(&format!("tagliato-{cut}.ai"), &good[..cut]);
        let _ = render(&f.0, FileKind::Ai, 0, 100);
    }
    // Riquadro assurdo o mancante: niente miniatura, nessun crash.
    let bad = String::from_utf8(good.clone())
        .unwrap()
        .replace("-300 100 -100 200", "5 5 5 5");
    let f = Temp::new("riquadro.ai", bad.as_bytes());
    assert!(render(&f.0, FileKind::Ai, 0, 100).is_err());
    let mut noise = good.clone();
    noise.extend(std::iter::repeat_n(b"( [ { 1e999 -- 9 9 9 9 9 c ".as_slice(), 2000).flatten());
    let f = Temp::new("rumore.ai", &noise);
    let _ = render(&f.0, FileKind::Ai, 0, 100);
}
