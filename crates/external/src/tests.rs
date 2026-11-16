use super::*;

fn found(key: &str, exe: Option<&str>) -> Found {
    let fam = types::family(key);
    Found {
        key: key.into(),
        name: fam.map_or(key, |f| f.name).into(),
        label: fam.map_or(key, |f| f.label).into(),
        exe: exe.map(PathBuf::from),
        app_id: None,
        handler: None,
        types: fam
            .map(|f| f.types.iter().map(|t| (*t).to_string()).collect())
            .unwrap_or_default(),
    }
}

fn keys(c: &Config, t: &str) -> Vec<String> {
    c.for_type(t).iter().map(|p| p.key.clone()).collect()
}

#[test]
fn ogni_estensione_ha_un_solo_tipo() {
    let mut seen = BTreeSet::new();
    for t in TYPES {
        for e in t.exts {
            assert!(seen.insert(*e), "{e} in due tipi");
        }
    }
    assert_eq!(type_of_ext(".JPEG").map(|t| t.key), Some("jpg"));
    assert_eq!(type_of_ext("NEF").map(|t| t.key), Some("raw"));
    assert!(type_of_ext("xyz").is_none());
}

#[test]
fn i_consigliati_e_le_famiglie_nominano_tipi_e_programmi_veri() {
    for t in TYPES {
        for slot in types::recommended(t.key) {
            for k in *slot {
                assert!(types::family(k).is_some(), "{k} sconosciuto in {}", t.key);
            }
        }
    }
    for f in FAMILIES {
        for t in f.types {
            assert!(type_by_key(t).is_some(), "{t} sconosciuto in {}", f.key);
        }
        for e in f.exes {
            assert_eq!(*e, e.to_ascii_lowercase());
        }
    }
}

#[test]
fn la_prima_volta_mette_i_consigliati_installati() {
    let found = vec![
        found("acrobat", Some("C:/a/Acrobat.exe")),
        found("illustrator", Some("C:/a/Illustrator.exe")),
        found("photoshop", Some("C:/a/Photoshop.exe")),
        found("foto", None),
        found("trueview", Some("C:/t/dwgviewr.exe")),
    ];
    let mut c = Config::default();
    assert!(c.refresh(&found));
    assert!(c.seeded);
    assert_eq!(keys(&c, "pdf"), ["acrobat", "illustrator"]);
    assert_eq!(keys(&c, "ai"), ["illustrator", "acrobat"]);
    assert_eq!(keys(&c, "jpg"), ["foto", "photoshop"]);
    // Lightroom non c'è: un solo pulsante per i RAW.
    assert_eq!(keys(&c, "raw"), ["photoshop"]);
    // AutoCAD non c'è: il posto va a TrueView.
    assert_eq!(keys(&c, "dxf"), ["trueview", "illustrator"]);
    assert!(keys(&c, "plt").is_empty());
    // Una seconda volta non tocca le scelte.
    c.remove("pdf", "illustrator");
    assert!(!c.refresh(&found));
    assert_eq!(keys(&c, "pdf"), ["acrobat"]);
}

#[test]
fn senza_programmi_trovati_non_si_decide_nulla() {
    let mut c = Config::default();
    assert!(!c.refresh(&[]));
    assert!(!c.seeded);
}

#[test]
fn una_versione_nuova_prende_il_posto_della_vecchia() {
    let mut c = Config::default();
    c.add(
        Program::from_found(&found("photoshop", Some("C:/Adobe/Photoshop 2025/Photoshop.exe"))),
        &["psd"],
    );
    c.rename("photoshop", "PS");
    assert!(c.refresh(&[found("photoshop", Some("C:/Adobe/Photoshop 2026/Photoshop.exe"))]));
    let p = c.program("photoshop").cloned().unwrap_or_default();
    assert_eq!(
        p.exe,
        Some(PathBuf::from("C:/Adobe/Photoshop 2026/Photoshop.exe"))
    );
    assert_eq!(p.label, "PS");
}

#[test]
fn aggiungi_togli_sposta() {
    let mut c = Config::default();
    let ps = Program::from_exe(Path::new("C:/x/Photoshop.exe"));
    assert_eq!(ps.key, "photoshop");
    c.add(ps, &["jpg", "png", "nonesiste"]);
    let gimp = Program::from_exe(Path::new("D:/Tools/MioViewer.exe"));
    assert_eq!(gimp.key, "exe:d:/tools/mioviewer.exe");
    assert_eq!(gimp.label, "MioViewer");
    c.add(gimp.clone(), &["jpg"]);
    c.add(gimp, &["jpg"]);
    assert_eq!(keys(&c, "jpg"), ["photoshop", "exe:d:/tools/mioviewer.exe"]);
    c.shift("jpg", "exe:d:/tools/mioviewer.exe", true);
    assert_eq!(keys(&c, "jpg"), ["exe:d:/tools/mioviewer.exe", "photoshop"]);
    c.shift("jpg", "exe:d:/tools/mioviewer.exe", true);
    assert_eq!(keys(&c, "jpg")[0], "exe:d:/tools/mioviewer.exe");
    c.remove("jpg", "exe:d:/tools/mioviewer.exe");
    assert!(c.program("exe:d:/tools/mioviewer.exe").is_none());
    c.remove("jpg", "photoshop");
    // Ancora usato per i PNG.
    assert!(c.program("photoshop").is_some());
    c.remove("png", "photoshop");
    assert!(c.programs.is_empty());
    assert!(c.types.is_empty());
}

#[test]
fn per_piu_file_solo_i_programmi_comuni() {
    let mut c = Config::default();
    c.add(Program::from_exe(Path::new("C:/a/Acrobat.exe")), &["pdf"]);
    c.add(
        Program::from_exe(Path::new("C:/a/Illustrator.exe")),
        &["pdf", "ai"],
    );
    let one = c.for_paths(&[PathBuf::from("x/A.PDF")]);
    assert_eq!(one.len(), 2);
    let mixed = c.for_paths(&[PathBuf::from("a.pdf"), PathBuf::from("b.ai")]);
    assert_eq!(
        mixed.iter().map(|p| p.key.as_str()).collect::<Vec<_>>(),
        ["illustrator"]
    );
    assert!(c.for_paths(&[]).is_empty());
    assert!(c.for_paths(&[PathBuf::from("senza-estensione")]).is_empty());
}

#[test]
fn un_file_rovinato_si_ripulisce() {
    let json = r#"{
        "programs": [
            {"key": "acrobat", "label": "Acrobat"},
            {"key": "acrobat", "label": "doppio"},
            {"key": "", "label": "vuoto"},
            {"key": "orfano"}
        ],
        "types": {"pdf": ["acrobat", "acrobat", "manca"], "boh": ["acrobat"], "ai": []},
        "seeded": true
    }"#;
    let c: Config = serde_json::from_str::<Config>(json)
        .map(Config::sanitized)
        .unwrap_or_default();
    assert_eq!(c.programs.len(), 1);
    assert_eq!(c.programs[0].label, "Acrobat");
    assert_eq!(keys(&c, "pdf"), ["acrobat"]);
    assert_eq!(c.types.len(), 1);
    // Un file vecchio senza la sezione.
    let c: Config = serde_json::from_str("{}").unwrap_or_default();
    assert!(!c.seeded);
}

#[test]
fn acrobat_si_apre_alla_pagina() {
    let p = Program::from_exe(Path::new("C:/a/Acrobat.exe"));
    let one = [PathBuf::from("C:/l/x.pdf")];
    let args = exe_args(&p, &one, Some(7));
    assert_eq!(args, ["/A", "page=7", "C:/l/x.pdf"].map(std::ffi::OsString::from));
    // Più file o un altro programma: solo i file.
    let two = [PathBuf::from("a.pdf"), PathBuf::from("b.pdf")];
    assert_eq!(exe_args(&p, &two, Some(7)).len(), 2);
    let ill = Program::from_exe(Path::new("C:/a/Illustrator.exe"));
    assert_eq!(exe_args(&ill, &one, Some(7)).len(), 1);
}

#[test]
fn un_programma_sparito_non_e_disponibile() {
    let p = Program::from_exe(Path::new("/non/esiste/Illustrator.exe"));
    assert!(!available(&p, &[]));
    assert!(matches!(
        launch(&p, &[PathBuf::from("a.pdf")], None),
        Err(Error::Missing(_))
    ));
    assert!(matches!(launch(&p, &[], None), Err(Error::NoFiles)));
    let mut store = found("foto", None);
    store.app_id = Some("Microsoft.Windows.Photos_8wekyb3d8bbwe!App".into());
    let foto = Program::from_found(&store);
    assert!(available(&foto, &[store]));
}

#[test]
fn le_app_dello_store_si_riconoscono() {
    assert_eq!(
        types::family_of_app_id("Microsoft.Windows.Photos_8wekyb3d8bbwe!App").map(|f| f.key),
        Some("foto")
    );
    assert_eq!(
        types::family_of_exe("AcroRd32.exe").map(|f| f.key),
        Some("acrobat")
    );
}

/// Su Windows: la lista «Apri con» si legge, le icone pure (sulla macchina della CI c'è almeno il
/// Blocco note per i .txt e Paint per le immagini).
#[cfg(windows)]
#[test]
fn windows_risponde() {
    let notepad = win::handlers("txt");
    assert!(!notepad.is_empty(), "nessun programma per i .txt");
    let found = detect();
    eprintln!("trovati: {found:#?}");
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let exe = PathBuf::from(windir).join("notepad.exe");
    let icon = icon(&Program::from_exe(&exe));
    let icon = icon.expect("icona del Blocco note");
    assert!(icon.width >= 16 && icon.rgba.len() == (icon.width * icon.height * 4) as usize);
    assert!(icon.rgba.chunks(4).any(|p| p[3] > 0), "icona tutta trasparente");
}
