use super::*;

/// Un DXF di prova scritto a mano: intestazione, un layer rosso, un blocco e le entità.
fn dxf(entities: &str) -> Vec<u8> {
    let mut s = String::new();
    let mut tag = |c: i32, v: &str| {
        s.push_str(&format!("{c}\n{v}\n"));
    };
    tag(0, "SECTION");
    tag(2, "HEADER");
    tag(9, "$ACADVER");
    tag(1, "AC1032");
    tag(9, "$INSUNITS");
    tag(70, "4");
    tag(0, "ENDSEC");
    tag(0, "SECTION");
    tag(2, "TABLES");
    tag(0, "TABLE");
    tag(2, "LAYER");
    tag(0, "LAYER");
    tag(2, "Taglio");
    tag(62, "1");
    tag(70, "0");
    tag(0, "LAYER");
    tag(2, "Spento");
    tag(62, "-3");
    tag(70, "0");
    tag(0, "ENDTAB");
    tag(0, "ENDSEC");
    tag(0, "SECTION");
    tag(2, "BLOCKS");
    tag(0, "BLOCK");
    tag(2, "QUADRO");
    tag(10, "0");
    tag(20, "0");
    tag(0, "LINE");
    tag(8, "0");
    tag(10, "0");
    tag(20, "0");
    tag(11, "10");
    tag(21, "0");
    tag(0, "ENDBLK");
    tag(0, "ENDSEC");
    tag(0, "SECTION");
    tag(2, "ENTITIES");
    s.push_str(entities);
    s.push_str("0\nENDSEC\n0\nEOF\n");
    s.into_bytes()
}

#[test]
fn una_linea_e_un_cerchio_danno_misure_e_layer() {
    let d = from_bytes(&dxf(
        "0\nLINE\n8\nTaglio\n10\n0\n20\n0\n11\n100\n21\n50\n0\nCIRCLE\n8\nSpento\n10\n50\n20\n25\n40\n10\n",
    ))
    .unwrap();
    assert_eq!(d.units.mm, Some(1.0));
    assert_eq!(d.version, "DXF AC1032");
    let (w, h) = d.size_mm(1.0);
    assert!((w - 100.0).abs() < 1e-6 && (h - 50.0).abs() < 1e-6, "{w} {h}");
    let taglio = d.layers.iter().find(|l| l.name == "Taglio").unwrap();
    assert_eq!(taglio.color, [255, 0, 0]);
    assert_eq!(taglio.shapes, 1);
    assert!(d.layers.iter().find(|l| l.name == "Spento").unwrap().off);
}

#[test]
fn il_blocco_si_mette_al_suo_posto_girato_e_scalato() {
    let d = from_bytes(&dxf(
        "0\nINSERT\n8\nTaglio\n2\nQUADRO\n10\n5\n20\n5\n41\n2\n42\n2\n50\n90\n",
    ))
    .unwrap();
    let Geometry::Line { points, .. } = &d.shapes.first().unwrap().geometry else {
        panic!("una linea");
    };
    // La linea da (0,0) a (10,0), scalata 2 e girata di 90°, parte da (5,5) e sale di 20.
    assert!(geometry::near(points[0], [5.0, 5.0]), "{points:?}");
    assert!(geometry::near(points[1], [5.0, 25.0]), "{points:?}");
    // Il layer 0 del blocco prende quello dell'inserimento.
    assert_eq!(d.layers.get(d.shapes[0].layer).unwrap().name, "Taglio");
}

#[test]
fn la_polilinea_con_rigonfiamento_fa_mezzo_cerchio() {
    // Da (0,0) a (10,0) con rigonfiamento 1: mezzo cerchio di raggio 5 sotto la corda (antiorario).
    let d = from_bytes(&dxf(
        "0\nLWPOLYLINE\n8\n0\n90\n2\n70\n0\n10\n0\n20\n0\n42\n1\n10\n10\n20\n0\n",
    ))
    .unwrap();
    let [x0, y0, x1, y1] = d.extents;
    assert!((x0 - 0.0).abs() < 1e-6 && (x1 - 10.0).abs() < 1e-6);
    assert!((y0 + 5.0).abs() < 1e-3 && y1.abs() < 1e-6, "{:?}", d.extents);
}

#[test]
fn i_codici_del_testo_multiriga_spariscono() {
    assert_eq!(
        geometry::mtext_lines("{\\fArial|b0|i0;Prima}\\PSeconda %%c20 \\H2;x"),
        vec!["Prima".to_string(), "Seconda Ø20 x".to_string()]
    );
    assert!(geometry::mtext_lines("\\P\\P").is_empty());
}

#[test]
fn la_spline_passa_dagli_estremi() {
    let ctrl = [[0.0, 0.0], [5.0, 10.0], [10.0, 0.0]];
    let pts = geometry::nurbs(&ctrl, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], &[], 2);
    assert!(geometry::near(pts[0], [0.0, 0.0]));
    assert!(geometry::near(*pts.last().unwrap(), [10.0, 0.0]));
    // A metà: (5, 5) per la quadratica.
    let mid = pts[pts.len() / 2];
    assert!(
        (mid[0] - 5.0).abs() < 1e-6 && (mid[1] - 5.0).abs() < 1e-6,
        "{mid:?}"
    );
}

#[test]
fn la_pagina_pdf_si_apre_ed_e_della_misura_giusta() {
    let d = from_bytes(&dxf(
        "0\nLINE\n8\nTaglio\n10\n0\n20\n0\n11\n100\n21\n50\n0\nTEXT\n8\n0\n10\n10\n20\n10\n40\n5\n1\nCittà (prova)\n",
    ))
    .unwrap();
    let o = PdfOptions {
        mm_per_unit: 1.0,
        margin_mm: 5.0,
        ..PdfOptions::default()
    };
    let bytes = d.to_pdf(&o).unwrap();
    let pages = winfiles_thumbs::PdfPages::open(bytes).unwrap();
    let (w, h) = pages.page_size(0).unwrap();
    assert!((f64::from(w) - 110.0 * 72.0 / 25.4).abs() < 0.1, "{w}");
    assert!((f64::from(h) - 60.0 * 72.0 / 25.4).abs() < 0.1, "{h}");
    let img = pages.render(0, 400).unwrap();
    // Qualcosa di non bianco c'è (la linea rossa).
    assert!(img.rgba.chunks(4).any(|p| p[0] > 200 && p[1] < 80 && p[2] < 80));
    // Un layer nascosto non si disegna.
    let hidden = PdfOptions {
        hidden: vec![d.layers.iter().position(|l| l.name == "Taglio").unwrap()],
        ..o
    };
    let img = winfiles_thumbs::PdfPages::open(d.to_pdf(&hidden).unwrap())
        .unwrap()
        .render(0, 400)
        .unwrap();
    assert!(!img.rgba.chunks(4).any(|p| p[0] > 200 && p[1] < 80 && p[2] < 80));
}

#[test]
fn troppo_grande_per_la_pagina_chiede_un_altra_scala() {
    let d = from_bytes(&dxf("0\nLINE\n10\n0\n20\n0\n11\n10000\n21\n0\n")).unwrap();
    assert!(d.to_pdf(&PdfOptions::default()).is_err());
    let ok = PdfOptions {
        mm_per_unit: 0.01,
        ..PdfOptions::default()
    };
    assert!(d.to_pdf(&ok).is_ok());
}

#[test]
fn un_dwg_vecchio_ha_un_messaggio_chiaro() {
    assert!(matches!(from_bytes(b"AC1009\0\0\0\0\0\0"), Err(CadError::OldDwg)));
}

#[test]
fn un_file_vuoto_non_e_un_disegno() {
    assert!(from_bytes(&dxf("")).is_err());
}

#[test]
fn un_dwg_si_legge_come_il_suo_dxf() {
    let src = dxf(
        "0\nLINE\n8\nTaglio\n10\n0\n20\n0\n11\n100\n21\n50\n0\nCIRCLE\n8\nTaglio\n10\n50\n20\n25\n40\n10\n",
    );
    let dwg = cadcraft_dwg::encode_as(&src, "AC1032").unwrap();
    assert!(cadcraft_dwg::is_dwg(&dwg));
    let d = from_bytes(&dwg).unwrap();
    assert!(d.version.starts_with("DWG AC1032"), "{}", d.version);
    let (w, h) = d.size_mm(1.0);
    assert!((w - 100.0).abs() < 1e-3 && (h - 50.0).abs() < 1e-3, "{w} {h}");
    assert!(d.layers.iter().any(|l| l.name == "Taglio" && l.shapes == 2));
}

#[test]
fn la_pagina_di_testo_e_un_pdf_della_misura_giusta() {
    let pdf = crate::text_page(
        210.0,
        297.0,
        &[crate::TextLine {
            x: 10.0,
            y: 290.0,
            size: 2.5,
            gray: 0.3,
            text: "Prova (1:50) è ok".into(),
        }],
        &[[0.0, 0.0, 10.0, 10.0]],
    )
    .unwrap();
    let pages = winfiles_thumbs::PdfPages::open(pdf).unwrap();
    let (w, h) = pages.page_size(0).unwrap();
    assert!((w - 595.28).abs() < 0.1 && (h - 841.89).abs() < 0.1);
    assert!(crate::text_page(0.0, 10.0, &[], &[]).is_err());
}
