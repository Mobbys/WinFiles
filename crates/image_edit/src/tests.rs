use super::*;

/// Una cartella di prova tutta nostra, che si svuota da sé.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "winfiles-image-edit-{}-{name}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Un'immagine di prova senza simmetrie: il colore dice dove si è, così rotazioni, specchi e
/// ritagli si riconoscono guardando un pixel.
fn picture(width: u32, height: u32, dpi: f64) -> Picture {
    let mut buf = image::RgbImage::new(width, height);
    for (x, y, p) in buf.enumerate_pixels_mut() {
        *p = image::Rgb([
            (x % 256) as u8,
            (y % 256) as u8,
            if x < width / 3 && y < height / 3 { 240 } else { 20 },
        ]);
    }
    Picture {
        image: std::sync::Arc::new(DynamicImage::ImageRgb8(buf)),
        dpi,
        icc: None,
        color: ColorMode::Rgb,
        bits: 8,
    }
}

/// Scrive l'immagine su disco e la riapre con l'`Editor`, come fa l'app.
fn editor(dir: &TempDir, name: &str, picture: &Picture, options: SaveOptions) -> Editor {
    let path = dir.join(name);
    let data = save::encode(picture, options).unwrap();
    std::fs::write(&path, data).unwrap();
    Editor::open(&path).unwrap()
}

/// La risoluzione letta dal file come la legge il resto dell'app.
fn dpi_of(path: &Path) -> Option<f64> {
    let probe = winfiles_formats::probe(path, winfiles_model::FileKind::Image).ok()?;
    let info = probe.image?;
    info.dpi_from_file.then_some(info.dpi_x)
}

#[test]
fn la_risoluzione_finisce_nel_file() {
    let dir = TempDir::new("dpi");
    for (format, name) in [
        (Format::Jpeg, "prova.jpg"),
        (Format::Png, "prova.png"),
        (Format::Tiff, "prova.tif"),
    ] {
        let path = dir.join(name);
        let options = SaveOptions {
            format,
            ..SaveOptions::default()
        };
        save::write(&picture(40, 30, 300.0), &path, options).unwrap();
        let read = dpi_of(&path).unwrap_or_else(|| panic!("{name}: il file non dichiara la risoluzione"));
        assert!(
            (read - 300.0).abs() < 0.5,
            "{name}: risoluzione {read} invece di 300"
        );
    }
}

#[test]
fn il_profilo_icc_resta_nel_file() {
    let dir = TempDir::new("icc");
    // Un profilo finto: qui conta solo che i byte tornino indietro uguali, come li riapre l'app.
    let profile: Vec<u8> = (0u16..600).map(|i| (i % 251) as u8).collect();
    let mut pic = picture(20, 20, 150.0);
    pic.icc = Some(profile.clone());
    for (format, name) in [
        (Format::Jpeg, "icc.jpg"),
        (Format::Png, "icc.png"),
        (Format::Tiff, "icc.tif"),
    ] {
        let path = dir.join(name);
        let options = SaveOptions {
            format,
            ..SaveOptions::default()
        };
        save::write(&pic, &path, options).unwrap();
        let reopened = Editor::open(&path).unwrap();
        assert_eq!(reopened.picture().icc(), Some(profile.as_slice()), "{name}");
    }
    // Chi non lo vuole, non se lo ritrova.
    let path = dir.join("senza.png");
    save::write(
        &pic,
        &path,
        SaveOptions {
            format: Format::Png,
            keep_icc: false,
            ..SaveOptions::default()
        },
    )
    .unwrap();
    assert_eq!(Editor::open(&path).unwrap().picture().icc(), None);
}

#[test]
fn ruota_specchia_e_annulla() {
    let dir = TempDir::new("ruota");
    let mut ed = editor(
        &dir,
        "r.png",
        &picture(40, 20, 96.0),
        SaveOptions {
            format: Format::Png,
            ..SaveOptions::default()
        },
    );
    assert_eq!((ed.picture().width(), ed.picture().height()), (40, 20));
    ed.apply(Op::Rotate(1)).unwrap();
    assert_eq!((ed.picture().width(), ed.picture().height()), (20, 40));
    assert_eq!(ed.undo_label().as_deref(), Some("Ruota a destra"));
    ed.apply(Op::Flip(Axis::Horizontal)).unwrap();
    assert_eq!(ed.undo_label().as_deref(), Some("Specchia in orizzontale"));
    ed.undo().unwrap();
    assert_eq!((ed.picture().width(), ed.picture().height()), (20, 40));
    ed.undo().unwrap();
    assert_eq!((ed.picture().width(), ed.picture().height()), (40, 20));
    assert!(!ed.can_undo());
    ed.redo().unwrap();
    assert_eq!((ed.picture().width(), ed.picture().height()), (20, 40));
    assert_eq!(ed.redo_label().as_deref(), Some("Specchia in orizzontale"));
}

#[test]
fn specchiare_due_volte_torna_uguale() {
    let dir = TempDir::new("specchio");
    let mut ed = editor(
        &dir,
        "s.png",
        &picture(16, 9, 96.0),
        SaveOptions {
            format: Format::Png,
            ..SaveOptions::default()
        },
    );
    let before = ed.picture().image().to_rgb8().into_raw();
    ed.apply(Op::Flip(Axis::Vertical)).unwrap();
    assert_ne!(ed.picture().image().to_rgb8().into_raw(), before);
    ed.apply(Op::Flip(Axis::Vertical)).unwrap();
    assert_eq!(ed.picture().image().to_rgb8().into_raw(), before);
}

#[test]
fn il_ritaglio_prende_la_parte_giusta() {
    let dir = TempDir::new("ritaglio");
    let mut ed = editor(
        &dir,
        "c.png",
        &picture(40, 30, 96.0),
        SaveOptions {
            format: Format::Png,
            ..SaveOptions::default()
        },
    );
    let whole = ed.picture().image().to_rgb8();
    ed.apply(Op::Crop(CropPx {
        x: 10,
        y: 5,
        width: 8,
        height: 6,
    }))
    .unwrap();
    assert_eq!((ed.picture().width(), ed.picture().height()), (8, 6));
    assert_eq!(
        ed.picture().image().to_rgb8().get_pixel(0, 0),
        whole.get_pixel(10, 5)
    );
    // Un ritaglio fuori dall'immagine non cambia niente e lo dice.
    assert!(matches!(
        ed.apply(Op::Crop(CropPx {
            x: 4,
            y: 0,
            width: 100,
            height: 2
        })),
        Err(ImageEditError::BadCrop)
    ));
    assert_eq!((ed.picture().width(), ed.picture().height()), (8, 6));
    assert!(matches!(
        ed.apply(Op::Crop(CropPx {
            x: 0,
            y: 0,
            width: 0,
            height: 2
        })),
        Err(ImageEditError::BadCrop)
    ));
}

#[test]
fn misure_in_millimetri_e_risoluzione() {
    let pic = picture(1181, 591, 300.0);
    let size = pic.size_mm();
    assert!((size.w - 100.0).abs() < 0.2, "larghezza {}", size.w);
    assert!((size.h - 50.0).abs() < 0.2, "altezza {}", size.h);
    assert!((pic.area_m2() - 0.005).abs() < 0.0005);
    // La distanza di visione della 4.9: 80 diviso i DPI.
    assert!((pic.viewing_distance_m().unwrap() - 80.0 / 300.0).abs() < 1e-9);
    assert_eq!(picture(1920, 1080, 96.0).ratio(), "16:9");
}

#[test]
fn cambiare_risoluzione_non_ricampiona() {
    let dir = TempDir::new("solo-dpi");
    let mut ed = editor(
        &dir,
        "d.png",
        &picture(600, 300, 300.0),
        SaveOptions {
            format: Format::Png,
            ..SaveOptions::default()
        },
    );
    let before = ed.picture().image().to_rgb8().into_raw();
    let wide = ed.picture().size_mm().w;
    ed.apply(Op::Dpi(150.0)).unwrap();
    assert_eq!(ed.picture().width(), 600, "i pixel non si toccano");
    assert_eq!(ed.picture().image().to_rgb8().into_raw(), before);
    assert!((ed.picture().size_mm().w - wide * 2.0).abs() < 0.1);
    assert_eq!(ed.undo_label().as_deref(), Some("Cambia risoluzione"));
    // Risoluzioni assurde rifiutate.
    assert!(ed.apply(Op::Dpi(0.0)).is_err());
    assert!(ed.apply(Op::Dpi(100_000.0)).is_err());
}

#[test]
fn ridimensionare_alla_misura_voluta() {
    let dir = TempDir::new("misura");
    let mut ed = editor(
        &dir,
        "m.png",
        &picture(400, 200, 300.0),
        SaveOptions {
            format: Format::Png,
            ..SaveOptions::default()
        },
    );
    // 50 mm di base a 300 DPI.
    let width = pixels_for_mm(50.0, 300.0);
    let height = pixels_for_mm(25.0, 300.0);
    ed.apply(Op::Resize { width, height }).unwrap();
    let size = ed.picture().size_mm();
    assert!((size.w - 50.0).abs() < 0.2, "{}", size.w);
    assert!((size.h - 25.0).abs() < 0.2, "{}", size.h);
    assert_eq!(ed.undo_label().as_deref(), Some("Ridimensiona"));
    // La risoluzione che fa misurare 50 mm a 591 pixel.
    assert!((dpi_for_mm(591, 50.0).unwrap() - 300.2).abs() < 0.5);
    assert_eq!(dpi_for_mm(0, 50.0), None);
    assert_eq!(dpi_for_mm(100, 0.0), None);
}

#[test]
fn misure_impossibili_rifiutate() {
    let dir = TempDir::new("limiti");
    let mut ed = editor(
        &dir,
        "l.png",
        &picture(10, 10, 96.0),
        SaveOptions {
            format: Format::Png,
            ..SaveOptions::default()
        },
    );
    assert!(matches!(
        ed.apply(Op::Resize { width: 0, height: 10 }),
        Err(ImageEditError::BadSize)
    ));
    assert!(matches!(
        ed.apply(Op::Resize {
            width: 100_000,
            height: 100_000
        }),
        Err(ImageEditError::TooManyPixels(_))
    ));
    assert!(!ed.can_undo(), "una modifica rifiutata non entra nella fila");
}

#[test]
fn il_jpeg_appiattisce_la_trasparenza_sul_bianco() {
    let dir = TempDir::new("alfa");
    let mut buf = image::RgbaImage::new(4, 1);
    buf.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
    buf.put_pixel(1, 0, image::Rgba([255, 0, 0, 0]));
    buf.put_pixel(2, 0, image::Rgba([0, 0, 0, 255]));
    buf.put_pixel(3, 0, image::Rgba([0, 0, 0, 0]));
    let pic = Picture {
        image: std::sync::Arc::new(DynamicImage::ImageRgba8(buf)),
        dpi: 96.0,
        icc: None,
        color: ColorMode::Rgba,
        bits: 8,
    };
    let path = dir.join("alfa.jpg");
    save::write(
        &pic,
        &path,
        SaveOptions {
            format: Format::Jpeg,
            quality: 100,
            keep_icc: true,
        },
    )
    .unwrap();
    let read = image::open(&path).unwrap().to_rgb8();
    let white = read.get_pixel(3, 0);
    assert!(
        white.0.iter().all(|c| *c > 240),
        "il trasparente deve diventare bianco, non {white:?}"
    );
    assert!(
        read.get_pixel(2, 0).0.iter().all(|c| *c < 20),
        "il nero resta nero"
    );
    // Il PNG invece la tiene.
    let png = dir.join("alfa.png");
    save::write(
        &pic,
        &png,
        SaveOptions {
            format: Format::Png,
            ..SaveOptions::default()
        },
    )
    .unwrap();
    assert_eq!(image::open(&png).unwrap().to_rgba8().get_pixel(1, 0).0[3], 0);
}

#[test]
fn salvare_non_lascia_file_a_meta() {
    let dir = TempDir::new("salva");
    let mut ed = editor(
        &dir,
        "s.png",
        &picture(30, 20, 200.0),
        SaveOptions {
            format: Format::Png,
            ..SaveOptions::default()
        },
    );
    assert!(!ed.is_modified());
    ed.apply(Op::Rotate(1)).unwrap();
    assert!(ed.is_modified());
    let out = dir.join("s.png");
    ed.save(
        &out,
        SaveOptions {
            format: Format::Png,
            ..SaveOptions::default()
        },
    )
    .unwrap();
    assert!(
        !ed.is_modified(),
        "dopo il salvataggio non ci sono modifiche in sospeso"
    );
    let leftovers: Vec<_> = std::fs::read_dir(&dir.0)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("winfiles-tmp"))
        .collect();
    assert!(leftovers.is_empty(), "file temporanei rimasti: {leftovers:?}");
    let reopened = Editor::open(&out).unwrap();
    assert_eq!(
        (reopened.picture().width(), reopened.picture().height()),
        (20, 30)
    );
    assert!((reopened.picture().dpi() - 200.0).abs() < 0.5);
}

#[test]
fn annullare_dopo_il_salvataggio_segna_le_modifiche() {
    let dir = TempDir::new("annulla-salva");
    let mut ed = editor(
        &dir,
        "a.png",
        &picture(12, 12, 96.0),
        SaveOptions {
            format: Format::Png,
            ..SaveOptions::default()
        },
    );
    let path = ed.path().to_path_buf();
    ed.apply(Op::Rotate(1)).unwrap();
    ed.save(
        &path,
        SaveOptions {
            format: Format::Png,
            ..SaveOptions::default()
        },
    )
    .unwrap();
    ed.undo().unwrap();
    assert!(
        ed.is_modified(),
        "tornando indietro il file su disco non è più quello"
    );
    ed.redo().unwrap();
    assert!(!ed.is_modified());
}

#[test]
fn riporta_tutto_com_era() {
    let dir = TempDir::new("reset");
    let mut ed = editor(
        &dir,
        "z.png",
        &picture(24, 12, 96.0),
        SaveOptions {
            format: Format::Png,
            ..SaveOptions::default()
        },
    );
    ed.apply(Op::Rotate(1)).unwrap();
    ed.apply(Op::Crop(CropPx {
        x: 0,
        y: 0,
        width: 6,
        height: 6,
    }))
    .unwrap();
    ed.reset();
    assert_eq!((ed.picture().width(), ed.picture().height()), (24, 12));
    assert!(!ed.can_undo() && !ed.can_redo());
}

#[test]
fn un_file_che_non_e_un_immagine_non_si_apre() {
    let dir = TempDir::new("spazzatura");
    let path = dir.join("finta.jpg");
    std::fs::write(&path, b"non sono un'immagine").unwrap();
    assert!(Editor::open(&path).is_err());
}

#[test]
fn il_nome_proposto_dice_cosa_e_cambiato() {
    let dir = TempDir::new("nome");
    let ed = editor(
        &dir,
        "foto vacanza.png",
        &picture(8, 8, 96.0),
        SaveOptions {
            format: Format::Png,
            ..SaveOptions::default()
        },
    );
    assert_eq!(ed.suggested_name(Format::Jpeg), "foto vacanza_modificata.jpg");
    assert_eq!(ed.file_name(), "foto vacanza.png");
    assert_eq!(Format::from_extension("x.TIFF"), Some(Format::Tiff));
    assert_eq!(Format::from_extension("x.doc"), None);
}
