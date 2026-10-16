use photocraft_psd::{ColorMode as PsdMode, PixelData, PsdBuilder};
use photocraft_raw::testgen::DngSpec;

use super::*;

/// Un DNG sintetico grigio uniforme, `w` × `h`.
fn dng(w: usize, h: usize, orientation: u16) -> Vec<u8> {
    let mut spec = DngSpec::cfa(w, h, vec![20_000; w * h]);
    spec.orientation = orientation;
    spec.build()
}

#[test]
fn riconosce_raw_e_psd_dai_byte() {
    assert_eq!(sniff(&dng(16, 8, 1)), Some(Kind::Raw));
    let psd = PsdBuilder::new(2, 2).to_bytes().unwrap();
    assert_eq!(sniff(&psd), Some(Kind::Psd));
    // Un TIFF qualsiasi non è un RAW.
    let mut tiff = Vec::new();
    image::DynamicImage::new_rgb8(4, 4)
        .write_to(&mut std::io::Cursor::new(&mut tiff), image::ImageFormat::Tiff)
        .unwrap();
    assert_eq!(sniff(&tiff), None);
    assert_eq!(sniff(b"niente"), None);
}

#[test]
fn sviluppa_un_dng_in_srgb_a_16_bit() {
    let photo = decode(&dng(32, 16, 1), Quality::Full).unwrap();
    assert_eq!((photo.image.width(), photo.image.height()), (32, 16));
    assert!(matches!(photo.image, DynamicImage::ImageRgb16(_)));
    assert_eq!(photo.bits, 16);
    assert_eq!(photo.icc.as_deref(), Some(srgb_icc().as_slice()));
    assert!(photo.origin.starts_with("RAW DNG sviluppato"), "{}", photo.origin);
}

#[test]
fn lo_sviluppo_gira_la_foto_come_la_fotocamera() {
    // Orientamento 6: la foto si mostra ruotata di 90°.
    let photo = decode(&dng(32, 16, 6), Quality::Full).unwrap();
    assert_eq!((photo.image.width(), photo.image.height()), (16, 32));
}

#[test]
fn un_raw_rovinato_da_un_errore_non_un_crash() {
    let mut data = dng(32, 16, 1);
    let half = data.len() / 2;
    data.truncate(half);
    assert!(decode(&data, Quality::Full).is_err());
    assert!(decode(&data, Quality::Preview).is_err());
}

#[test]
fn legge_l_immagine_unita_di_un_psd_rgb() {
    let mut pixels = Vec::new();
    for i in 0..(4 * 2) {
        pixels.extend_from_slice(&[i as u8 * 30, 100, 200, 255]);
    }
    let mut b = PsdBuilder::new(4, 2).resolution(300.0).icc_profile(srgb_icc());
    b.composite(PixelData::Rgba8(pixels));
    let photo = decode(&b.to_bytes().unwrap(), Quality::Full).unwrap();
    assert_eq!((photo.image.width(), photo.image.height()), (4, 2));
    assert_eq!(photo.dpi, Some(300.0));
    assert_eq!(photo.mode, Mode::Rgb);
    assert!(photo.icc.is_some());
    let rgb = photo.image.to_rgb8();
    assert_eq!(rgb.get_pixel(1, 0).0, [30, 100, 200]);
}

#[test]
fn un_psd_cmyk_si_converte_con_il_profilo() {
    // Inchiostro: un pixel bianco (niente) e uno nero pieno (tutto il nero).
    let ink = vec![0, 0, 0, 0, 255, 0, 0, 0, 255, 255];
    let mut b = PsdBuilder::new(2, 1).color_mode(PsdMode::Cmyk);
    b.composite(PixelData::Cmyka8(ink));
    let photo = decode(&b.to_bytes().unwrap(), Quality::Full).unwrap();
    assert_eq!(photo.mode, Mode::Cmyk);
    let rgb = photo.image.to_rgb8();
    let white = rgb.get_pixel(0, 0).0;
    let black = rgb.get_pixel(1, 0).0;
    assert!(white.iter().all(|&c| c > 240), "bianco: {white:?}");
    assert!(black.iter().all(|&c| c < 40), "nero: {black:?}");
}

#[test]
fn un_psd_rovinato_da_un_errore() {
    let mut b = PsdBuilder::new(64, 64);
    b.composite(PixelData::Rgba8(vec![128; 64 * 64 * 4]));
    let mut data = b.to_bytes().unwrap();
    data.truncate(60);
    assert!(decode(&data, Quality::Full).is_err());
}
