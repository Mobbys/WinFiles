//! File costruiti a mano, byte per byte, con misure note.

use proptest::prelude::*;
use winfiles_model::FileKind;

use crate::{DEFAULT_DPI, Probe, ProbeError, probe_source};

fn run(bytes: &[u8], kind: FileKind) -> Result<Probe, ProbeError> {
    let mut src: &[u8] = bytes;
    probe_source(&mut src, kind)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.01
}

/// JPEG minimo: SOI, APP0 JFIF opzionale, APP1 EXIF opzionale, SOF0, EOI.
fn jpeg(w: u16, h: u16, jfif: Option<(u8, u16)>, exif: Option<&[u8]>) -> Vec<u8> {
    let mut b = vec![0xFF, 0xD8];
    if let Some((unit, d)) = jfif {
        b.extend([0xFF, 0xE0, 0, 16]);
        b.extend(b"JFIF\0");
        b.extend([1, 2, unit]);
        b.extend(d.to_be_bytes());
        b.extend(d.to_be_bytes());
        b.extend([0, 0]);
    }
    if let Some(tiff) = exif {
        let len = u16::try_from(tiff.len() + 8).unwrap();
        b.extend([0xFF, 0xE1]);
        b.extend(len.to_be_bytes());
        b.extend(b"Exif\0\0");
        b.extend(tiff);
    }
    b.extend([0xFF, 0xC0, 0, 11, 8]);
    b.extend(h.to_be_bytes());
    b.extend(w.to_be_bytes());
    b.extend([1, 1, 0x11, 0]);
    b.extend([0xFF, 0xD9]);
    b
}

/// TIFF little endian con un IFD: (tag, tipo, valore). I RATIONAL puntano a numeratore/1.
fn tiff(entries: &[(u16, u16, u32)]) -> Vec<u8> {
    let mut b = b"II*\0".to_vec();
    b.extend(8u32.to_le_bytes());
    let n = u16::try_from(entries.len()).unwrap();
    b.extend(n.to_le_bytes());
    let data_at = 8 + 2 + 12 * entries.len() + 4;
    let mut data = Vec::new();
    for (tag, kind, value) in entries {
        b.extend(tag.to_le_bytes());
        b.extend(kind.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        if *kind == 5 {
            let at = u32::try_from(data_at + data.len()).unwrap();
            data.extend(value.to_le_bytes());
            data.extend(1u32.to_le_bytes());
            b.extend(at.to_le_bytes());
        } else if *kind == 3 {
            b.extend(u16::try_from(*value).unwrap().to_le_bytes());
            b.extend([0, 0]);
        } else {
            b.extend(value.to_le_bytes());
        }
    }
    b.extend(0u32.to_le_bytes());
    b.extend(data);
    b
}

fn png(w: u32, h: u32, phys_ppm: Option<u32>) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    b.extend(13u32.to_be_bytes());
    b.extend(b"IHDR");
    b.extend(w.to_be_bytes());
    b.extend(h.to_be_bytes());
    b.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
    if let Some(ppm) = phys_ppm {
        b.extend(9u32.to_be_bytes());
        b.extend(b"pHYs");
        b.extend(ppm.to_be_bytes());
        b.extend(ppm.to_be_bytes());
        b.extend([1, 0, 0, 0, 0]);
    }
    b.extend(0u32.to_be_bytes());
    b.extend(b"IEND");
    b.extend([0, 0, 0, 0]);
    b
}

/// PDF con offset xref corretti: una pagina per voce (MediaBox, TrimBox, Rotate).
fn pdf(pages: &[(&str, Option<&str>, i32)]) -> Vec<u8> {
    let mut objects = vec![String::new(), String::new()];
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", i + 3)).collect();
    objects[0] = "<< /Type /Catalog /Pages 2 0 R >>".into();
    objects[1] = format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        pages.len()
    );
    for (media, trim, rotate) in pages {
        let trim = trim.map(|t| format!(" /TrimBox [{t}]")).unwrap_or_default();
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [{media}]{trim} /Rotate {rotate} >>"
        ));
    }
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
fn jpeg_with_jfif_300_dpi() {
    let p = run(&jpeg(3508, 2480, Some((1, 300)), None), FileKind::Image).unwrap();
    let img = p.image.unwrap();
    assert!(img.dpi_from_file && close(img.dpi_x, 300.0));
    // 3508 px a 300 DPI = 297 mm: un A4 orizzontale.
    assert!(close(p.pages[0].size.w, 297.010) && close(p.pages[0].size.h, 209.97));
}

#[test]
fn jpeg_dots_per_cm() {
    let p = run(&jpeg(1181, 1181, Some((2, 118)), None), FileKind::Image).unwrap();
    // 118 punti/cm ≈ 299,7 DPI.
    assert!(close(p.image.unwrap().dpi_x, 299.72));
}

#[test]
fn jpeg_exif_resolution_and_rotation() {
    // Fotocamera: 72 DPI nell'EXIF, orientamento 6 (ruotata di 90°).
    let exif = tiff(&[(274, 3, 6), (282, 5, 72), (283, 5, 72), (296, 3, 2)]);
    let p = run(&jpeg(4000, 3000, None, Some(&exif)), FileKind::Image).unwrap();
    let img = p.image.unwrap();
    assert_eq!((img.width_px, img.height_px), (3000, 4000));
    assert!(img.dpi_from_file && close(img.dpi_x, 72.0));
    assert!(p.pages[0].size.h > p.pages[0].size.w);
}

#[test]
fn jfif_wins_over_exif() {
    let exif = tiff(&[(282, 5, 72), (283, 5, 72)]);
    let p = run(&jpeg(100, 100, Some((1, 300)), Some(&exif)), FileKind::Image).unwrap();
    assert!(close(p.image.unwrap().dpi_x, 300.0));
}

#[test]
fn missing_resolution_uses_the_default_and_says_so() {
    let p = run(&jpeg(960, 960, Some((0, 1)), None), FileKind::Image).unwrap();
    let img = p.image.unwrap();
    assert!(!img.dpi_from_file && close(img.dpi_x, DEFAULT_DPI));
    assert!(close(p.pages[0].size.w, 254.0));
}

#[test]
fn png_with_and_without_phys() {
    // 11811 pixel per metro = 300 DPI.
    let p = run(&png(2480, 3508, Some(11811)), FileKind::Image).unwrap();
    assert!(close(p.image.unwrap().dpi_x, 300.0));
    assert!(close(p.pages[0].size.w, 209.97));
    let p = run(&png(96, 96, None), FileKind::Image).unwrap();
    assert!(!p.image.unwrap().dpi_from_file);
    assert!(close(p.pages[0].size.w, 25.4));
}

#[test]
fn tiff_in_centimetres() {
    let t = tiff(&[
        (256, 3, 1000),
        (257, 4, 500),
        (282, 5, 100),
        (283, 5, 100),
        (296, 3, 3),
    ]);
    let p = run(&t, FileKind::Image).unwrap();
    let img = p.image.unwrap();
    assert_eq!((img.width_px, img.height_px), (1000, 500));
    // 100 pixel/cm: 1000 px = 10 cm.
    assert!(close(p.pages[0].size.w, 100.0) && close(p.pages[0].size.h, 50.0));
}

#[test]
fn gif_bmp_webp() {
    let mut gif = b"GIF89a".to_vec();
    gif.extend([0x40, 0x01, 0xF0, 0x00, 0, 0, 0]);
    let p = run(&gif, FileKind::Image).unwrap();
    assert_eq!(p.image.map(|i| (i.width_px, i.height_px)), Some((320, 240)));

    let mut bmp = b"BM".to_vec();
    bmp.extend([0; 12]);
    bmp.extend(40u32.to_le_bytes());
    bmp.extend(200i32.to_le_bytes());
    bmp.extend((-100i32).to_le_bytes());
    bmp.extend([1, 0, 24, 0]);
    bmp.extend([0; 8]);
    bmp.extend(11811u32.to_le_bytes());
    bmp.extend(11811u32.to_le_bytes());
    bmp.extend([0; 8]);
    let p = run(&bmp, FileKind::Image).unwrap();
    let img = p.image.unwrap();
    assert_eq!((img.width_px, img.height_px), (200, 100));
    assert!(close(img.dpi_x, 300.0));

    let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
    webp.extend(10u32.to_le_bytes());
    webp.extend([0; 4]);
    webp.extend([0x1F, 0x03, 0x00, 0xDF, 0x01, 0x00]);
    let p = run(&webp, FileKind::Image).unwrap();
    assert_eq!(p.image.map(|i| (i.width_px, i.height_px)), Some((800, 480)));
}

#[test]
fn pdf_pages_trim_and_rotation() {
    let bytes = pdf(&[
        ("0 0 595.276 841.89", None, 0),
        // A4 con 3 mm di abbondanza per lato e TrimBox al formato finito.
        ("0 0 612.283 858.898", Some("8.504 8.504 603.78 850.394"), 0),
        ("0 0 595.276 841.89", None, 90),
    ]);
    let p = run(&bytes, FileKind::Pdf).unwrap();
    assert_eq!(p.pages.len(), 3);
    assert!(close(p.pages[0].size.w, 210.0) && close(p.pages[0].size.h, 297.0));
    assert_eq!(p.pages[0].trim, None);
    assert!(close(p.pages[1].size.w, 216.0));
    let trim = p.pages[1].trim.unwrap();
    assert!(close(trim.w, 210.0) && close(trim.h, 297.0));
    assert!(close(p.pages[2].size.w, 297.0) && close(p.pages[2].size.h, 210.0));
    assert!(p.image.is_none());
}

#[test]
fn ai_saved_as_pdf_is_read_as_pdf() {
    let p = run(&pdf(&[("0 0 283.465 283.465", None, 0)]), FileKind::Ai).unwrap();
    assert!(close(p.pages[0].size.w, 100.0));
}

#[test]
fn eps_plain_and_with_dos_header() {
    let ps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 284 142\n%%HiResBoundingBox: 0 0 283.465 141.732\n%%EndComments\nshowpage\n";
    let p = run(ps, FileKind::Eps).unwrap();
    assert!(close(p.pages[0].size.w, 100.0) && close(p.pages[0].size.h, 50.0));

    let mut dos = vec![0xC5, 0xD0, 0xD3, 0xC6];
    dos.extend(30u32.to_le_bytes());
    dos.extend(u32::try_from(ps.len()).unwrap().to_le_bytes());
    dos.extend([0; 18]);
    dos.extend(ps);
    dos.extend(b"II*\0 anteprima TIFF");
    let p = run(&dos, FileKind::Eps).unwrap();
    assert!(close(p.pages[0].size.w, 100.0));
}

#[test]
fn eps_bounding_box_at_end() {
    let ps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: (atend)\n%%EndComments\nshowpage\n%%Trailer\n%%BoundingBox: 0 0 72 72\n%%EOF\n";
    let p = run(ps, FileKind::Eps).unwrap();
    assert!(close(p.pages[0].size.w, 25.4));
}

#[test]
fn damaged_and_unknown_files_are_errors() {
    assert!(matches!(run(b"", FileKind::Image), Err(ProbeError::Unrecognized)));
    assert!(matches!(
        run(b"testo qualsiasi", FileKind::Pdf),
        Err(ProbeError::Unrecognized)
    ));
    let mut cut = jpeg(100, 100, Some((1, 300)), None);
    cut.truncate(12);
    assert!(run(&cut, FileKind::Image).is_err());
    assert!(run(&png(0, 10, None), FileKind::Image).is_err());
    assert!(run(&png(2_000_000, 10, None), FileKind::Image).is_err());
    assert!(run(b"%!PS-Adobe-3.0 EPSF-3.0\n%%EndComments\n", FileKind::Eps).is_err());
    assert!(run(b"%PDF-1.4 rotto", FileKind::Pdf).is_err());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// Qualsiasi sequenza di byte dà un risultato o un errore, mai un crash.
    #[test]
    fn never_panics(prefix in prop::sample::select(vec![
        &b"\xFF\xD8\xFF"[..], b"\x89PNG\r\n\x1a\n", b"II*\0", b"MM\0*", b"GIF89a", b"BM",
        b"RIFF\0\0\0\0WEBPVP8X", b"%PDF-1.7\n", b"%!PS-Adobe-3.0\n", b"\xC5\xD0\xD3\xC6",
    ]), tail in prop::collection::vec(any::<u8>(), 0..512)) {
        let mut bytes = prefix.to_vec();
        bytes.extend(tail);
        for kind in [FileKind::Image, FileKind::Pdf, FileKind::Ai, FileKind::Eps] {
            let _ = run(&bytes, kind);
        }
    }
}

#[test]
fn heic_singola_e_a_tessere() {
    // Foto di prova di heic-rs (testdata/heic, MIT/Apache): 96×32 in un pezzo, 1024×1024 in 2×2
    // tessere. Le HEIC non dichiarano DPI: vale il predefinito, segnalato.
    let p = run(
        include_bytes!("../../../testdata/heic/rgb-strips-96.heic"),
        FileKind::Image,
    )
    .unwrap();
    let img = p.image.unwrap();
    assert_eq!((img.width_px, img.height_px), (96, 32));
    assert!(close(img.dpi_x, DEFAULT_DPI) && !img.dpi_from_file);
    let p = run(
        include_bytes!("../../../testdata/heic/checker-1024.heic"),
        FileKind::Image,
    )
    .unwrap();
    assert_eq!(p.image.map(|i| (i.width_px, i.height_px)), Some((1024, 1024)));
}

#[test]
fn heic_rovinate_danno_errori() {
    let good = include_bytes!("../../../testdata/heic/checker-1024.heic");
    for cut in [12, 40, 200, 600, good.len() / 2] {
        let _ = run(good.get(..cut).unwrap(), FileKind::Image);
    }
    let mut bad = good.to_vec();
    for b in bad.iter_mut().skip(20).step_by(7) {
        *b ^= 0x5A;
    }
    let _ = run(&bad, FileKind::Image);
    // Un riquadro che dichiara 4 byte.
    let mut tiny = b"\0\0\0\x04ftypheic".to_vec();
    tiny.extend([0; 8]);
    assert!(run(&tiny, FileKind::Image).is_err());
}
