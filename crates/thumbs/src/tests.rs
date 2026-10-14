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
    let content = "0 0 0 rg 0 0 100 100 re f";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R >>".to_string(),
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
