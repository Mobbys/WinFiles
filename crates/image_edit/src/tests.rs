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
        origin: None,
        trim: None,
        icc_name: None,
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
        origin: None,
        trim: None,
        icc_name: None,
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

#[test]
fn apre_un_raw_sviluppato_e_propone_il_jpeg() {
    let dir = TempDir::new("raw");
    let spec = photocraft_raw::testgen::DngSpec::cfa(48, 32, vec![30_000; 48 * 32]);
    let path = dir.join("scatto.dng");
    std::fs::write(&path, spec.build()).unwrap();
    let editor = Editor::open(&path).unwrap();
    let p = editor.picture();
    assert_eq!((p.width(), p.height()), (48, 32));
    assert_eq!(p.bits(), 16);
    assert!(p.icc().is_some(), "sviluppato in sRGB, con il profilo");
    assert!(p.origin().unwrap().contains("DNG"));
    assert_eq!(default_format(&path), Format::Jpeg);
    // Si salva in JPEG e in TIFF a 16 bit come qualsiasi immagine.
    let out = dir.join("scatto.tif");
    let mut editor = editor;
    editor
        .save(
            &out,
            SaveOptions {
                format: Format::Tiff,
                ..SaveOptions::default()
            },
        )
        .unwrap();
    let back = Editor::open(&out).unwrap();
    assert_eq!(back.picture().width(), 48);
}

#[test]
fn apre_un_psd_con_risoluzione_e_propone_il_tiff() {
    let dir = TempDir::new("psd");
    let mut b = photocraft_psd::PsdBuilder::new(30, 20).resolution(150.0);
    b.composite(photocraft_psd::PixelData::Rgba8(vec![200; 30 * 20 * 4]));
    let path = dir.join("manifesto.psd");
    std::fs::write(&path, b.to_bytes().unwrap()).unwrap();
    let editor = Editor::open(&path).unwrap();
    let p = editor.picture();
    assert_eq!((p.width(), p.height()), (30, 20));
    assert!((p.dpi() - 150.0).abs() < 0.01);
    assert_eq!(default_format(&path), Format::Tiff);
    assert!(extensions().contains(&"psd") && extensions().contains(&"nef"));
}

#[test]
fn l_abbondanza_a_specchio_ribalta_l_immagine_sul_bordo() {
    let p = picture(30, 20, 254.0);
    let sides = [3, 2, 4, 1];
    let out = apply_one(
        &p,
        Op::Bleed {
            sides,
            method: BleedMethod::Mirror,
        },
    )
    .unwrap();
    assert_eq!((out.width(), out.height()), (37, 23));
    let px = |pic: &Picture, x: u32, y: u32| pic.image().to_rgb8().get_pixel(x, y).0;
    // Dentro, l'immagine di prima spostata; fuori a sinistra la colonna 0 è ribaltata.
    assert_eq!(px(&out, 3, 2), px(&p, 0, 0));
    assert_eq!(px(&out, 2, 2), px(&p, 0, 0));
    assert_eq!(px(&out, 0, 2), px(&p, 2, 0));
    // A destra e in basso.
    assert_eq!(px(&out, 33, 21), px(&p, 29, 19));
    assert_eq!(px(&out, 36, 22), px(&p, 26, 19));
    assert_eq!(
        out.trim(),
        Some(CropPx {
            x: 3,
            y: 2,
            width: 30,
            height: 20
        })
    );
    // 254 DPI: 10 pixel al millimetro.
    let t = out.trim_mm();
    assert!((t[0] - 0.3).abs() < 1e-9 && (t[2] - 3.3).abs() < 1e-9, "{t:?}");
}

#[test]
fn l_abbondanza_allungata_ripete_l_ultima_riga() {
    let p = picture(10, 10, 100.0);
    let out = apply_one(
        &p,
        Op::Bleed {
            sides: [5, 5, 5, 5],
            method: BleedMethod::Stretch,
        },
    )
    .unwrap();
    let px = |pic: &Picture, x: u32, y: u32| pic.image().to_rgb8().get_pixel(x, y).0;
    assert_eq!(px(&out, 0, 0), px(&p, 0, 0));
    assert_eq!(px(&out, 19, 7), px(&p, 9, 2));
}

#[test]
fn una_fascia_piu_larga_dell_immagine_si_ribalta_ancora() {
    let p = picture(4, 4, 100.0);
    let out = apply_one(
        &p,
        Op::Bleed {
            sides: [10, 0, 0, 0],
            method: BleedMethod::Mirror,
        },
    )
    .unwrap();
    assert_eq!(out.width(), 14);
    let px = |pic: &Picture, x: u32, y: u32| pic.image().to_rgb8().get_pixel(x, y).0;
    // Le colonne nuove prendono 1 0 0 1 2 3 3 2 1 0, poi l'immagine 0 1 2 3.
    assert_eq!(px(&out, 9, 0), px(&p, 0, 0));
    assert_eq!(px(&out, 6, 0), px(&p, 3, 0));
    assert_eq!(px(&out, 2, 0), px(&p, 0, 0));
    assert_eq!(px(&out, 0, 0), px(&p, 1, 0));
}

#[test]
fn il_formato_finito_segue_rotazioni_specchi_e_ritagli() {
    let p = picture(30, 20, 100.0);
    let b = apply_one(
        &p,
        Op::Bleed {
            sides: [3, 2, 4, 1],
            method: BleedMethod::Mirror,
        },
    )
    .unwrap();
    // 37 × 23 con il formato finito a (3, 2) di 30 × 20.
    let r = apply_one(&b, Op::Rotate(1)).unwrap();
    assert_eq!(
        r.trim(),
        Some(CropPx {
            x: 1,
            y: 3,
            width: 20,
            height: 30
        })
    );
    let back = apply_one(&r, Op::Rotate(-1)).unwrap();
    assert_eq!(back.trim(), b.trim());
    let f = apply_one(&b, Op::Flip(Axis::Horizontal)).unwrap();
    assert_eq!(
        f.trim(),
        Some(CropPx {
            x: 4,
            y: 2,
            width: 30,
            height: 20
        })
    );
    let c = apply_one(
        &b,
        Op::Crop(CropPx {
            x: 5,
            y: 0,
            width: 10,
            height: 23,
        }),
    )
    .unwrap();
    assert_eq!(
        c.trim(),
        Some(CropPx {
            x: 0,
            y: 2,
            width: 10,
            height: 20
        })
    );
    let z = apply_one(
        &b,
        Op::Resize {
            width: 74,
            height: 46,
        },
    )
    .unwrap();
    assert_eq!(
        z.trim(),
        Some(CropPx {
            x: 6,
            y: 4,
            width: 60,
            height: 40
        })
    );
    // Una seconda abbondanza allarga ancora: il formato finito resta quello.
    let again = apply_one(
        &b,
        Op::Bleed {
            sides: [1, 1, 1, 1],
            method: BleedMethod::Stretch,
        },
    )
    .unwrap();
    assert_eq!(
        again.trim(),
        Some(CropPx {
            x: 4,
            y: 3,
            width: 30,
            height: 20
        })
    );
}

#[test]
fn l_abbondanza_vale_anche_a_16_bit_e_con_la_trasparenza() {
    let mut p = picture(8, 8, 100.0);
    p.image = std::sync::Arc::new(DynamicImage::ImageRgba16(p.image().to_rgba16()));
    let out = apply_one(
        &p,
        Op::Bleed {
            sides: [2, 2, 2, 2],
            method: BleedMethod::Mirror,
        },
    )
    .unwrap();
    assert!(matches!(out.image(), DynamicImage::ImageRgba16(_)));
    assert_eq!(out.width(), 12);
    assert!(
        apply_one(
            &p,
            Op::Bleed {
                sides: [0, 0, 0, 0],
                method: BleedMethod::Mirror
            }
        )
        .is_err()
    );
}

/// Un cerchio blu di raggio `r` pixel al centro, su fondo bianco, con un buco bianco al centro
/// (raggio `r / 3`): il bianco dentro il soggetto non è sfondo.
fn disc(side: u32, r: f64, alpha: bool) -> Picture {
    let c = f64::from(side) / 2.0;
    let buf = image::RgbaImage::from_fn(side, side, |x, y| {
        let d = (f64::from(x) + 0.5 - c).hypot(f64::from(y) + 0.5 - c);
        if d <= r / 3.0 {
            image::Rgba([255, 255, 255, 255])
        } else if d <= r {
            image::Rgba([20, 60, 200, 255])
        } else if alpha {
            image::Rgba([255, 255, 255, 0])
        } else {
            image::Rgba([250, 250, 250, 255])
        }
    });
    let mut p = picture(1, 1, 254.0);
    p.image = std::sync::Arc::new(DynamicImage::ImageRgba8(buf));
    p
}

#[test]
fn la_fustella_segue_il_soggetto_sul_fondo_bianco() {
    // 600 pixel a 254 DPI: 60 mm; cerchio di raggio 20 mm.
    let p = disc(600, 200.0, false);
    let spec = ContourSpec {
        background: Background::Color([255, 255, 255]),
        tolerance: 5.0,
        offset: 0.0,
        min_area: 1.0,
        holes: false,
    };
    let c = trace(&p, &spec).unwrap();
    assert_eq!(c.len(), 1, "il buco bianco al centro non si taglia");
    let pi = std::f64::consts::PI;
    let a = contour_area(&c);
    assert!((a - pi * 400.0).abs() < pi * 400.0 * 0.02, "{a}");
    // Con 3 mm fuori il raggio diventa 23 mm.
    let grown = trace(&p, &ContourSpec { offset: 3.0, ..spec }).unwrap();
    let g = contour_area(&grown);
    assert!((g - pi * 529.0).abs() < pi * 529.0 * 0.02, "{g}");
    // Con i fori: il centro (raggio 6,67 mm) è tolto.
    let holed = trace(&p, &ContourSpec { holes: true, ..spec }).unwrap();
    assert_eq!(holed.len(), 2);
    let r3 = 20.0_f64 / 3.0;
    let hval = contour_area(&holed);
    assert!(
        (hval - pi * (400.0 - r3 * r3)).abs() < pi * 400.0 * 0.03,
        "{hval}"
    );
}

#[test]
fn la_fustella_segue_la_trasparenza_e_avvisa_se_manca_lo_sfondo() {
    let p = disc(400, 120.0, true);
    let spec = ContourSpec {
        background: Background::Transparent,
        tolerance: 0.0,
        offset: 0.0,
        min_area: 1.0,
        holes: false,
    };
    let c = trace(&p, &spec).unwrap();
    let pi = std::f64::consts::PI;
    let a = contour_area(&c);
    assert!((a - pi * 144.0).abs() < pi * 144.0 * 0.03, "{a}");
    // Nessuno sfondo del colore scelto: lo dice invece di tagliare il rettangolo.
    let red = ContourSpec {
        background: Background::Color([255, 0, 0]),
        ..spec
    };
    assert!(trace(&disc(100, 30.0, false), &red).is_err());
}

#[test]
fn la_fustella_tiene_gli_spigoli_del_triangolo() {
    // Un triangolo blu su bianco accanto a un anello rosso, come un adesivo vero: la fustella
    // deve girare attorno a tutti e tre gli spigoli, non tagliarne uno.
    let (w, h) = (1200u32, 900u32);
    let inside = |x: f64, y: f64| {
        let s = |ax: f64, ay: f64, bx: f64, by: f64| (bx - ax) * (y - ay) - (by - ay) * (x - ax);
        let (a, b, c) = (
            s(600.0, 700.0, 850.0, 150.0),
            s(850.0, 150.0, 1100.0, 700.0),
            s(1100.0, 700.0, 600.0, 700.0),
        );
        (a >= 0.0 && b >= 0.0 && c >= 0.0) || (a <= 0.0 && b <= 0.0 && c <= 0.0)
    };
    let buf = image::RgbaImage::from_fn(w, h, |x, y| {
        let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
        let d = ((fx - 300.0) / 200.0).hypot((fy - 400.0) / 250.0);
        if inside(fx, fy) {
            image::Rgba([30, 110, 200, 255])
        } else if d <= 1.0 && ((fx - 300.0) / 80.0).hypot((fy - 400.0) / 120.0) > 1.0 {
            image::Rgba([220, 40, 60, 255])
        } else {
            image::Rgba([255, 255, 255, 255])
        }
    });
    let mut p = picture(1, 1, 254.0);
    p.image = std::sync::Arc::new(DynamicImage::ImageRgba8(buf));
    let spec = ContourSpec {
        background: Background::Color([255, 255, 255]),
        tolerance: 10.0,
        offset: 0.0,
        min_area: 4.0,
        holes: false,
    };
    let c = trace(&p, &spec).unwrap();
    assert_eq!(c.len(), 2);
    // Lo spigolo in basso a destra (x 1100 pixel = 110 mm) resta.
    let right = c.iter().flatten().map(|q| q[0]).fold(0.0, f64::max);
    assert!(right > 109.0, "lo spigolo è tagliato: arriva a {right} mm");
    // Area del triangolo: 500 × 550 / 2 pixel, a 0,1 mm per pixel.
    let tri = 500.0 * 550.0 / 2.0 * 0.01;
    let ring = std::f64::consts::PI * (200.0 * 250.0) * 0.01;
    let a = contour_area(&c);
    assert!(
        (a - tri - ring).abs() < (tri + ring) * 0.02,
        "{a} invece di {}",
        tri + ring
    );
    // Allargata di 2 mm: più grande, mai più piccola (lo spigolo resta dentro).
    let g = contour_area(&trace(&p, &ContourSpec { offset: 2.0, ..spec }).unwrap());
    assert!(g > a * 1.1, "{g} contro {a}");
}
