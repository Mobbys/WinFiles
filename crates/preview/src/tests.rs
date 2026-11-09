use super::*;

fn temp(name: &str, data: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("winfiles-anteprime-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, data).unwrap();
    p
}

#[test]
fn un_svg_ha_la_sua_anteprima_nelle_proporzioni_giuste() {
    let p = temp(
        "a.svg",
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="100mm"><rect width="100%" height="100%" fill="#f00"/></svg>"##,
    );
    let t = thumbnail(&p, 256).unwrap();
    assert_eq!(t.width, 256);
    assert!((t.height as i32 - 128).abs() <= 1, "{}", t.height);
    // Rosso al centro.
    let i = ((t.height / 2 * t.width + t.width / 2) * 4) as usize;
    assert_eq!(&t.rgba[i..i + 3], &[255, 0, 0]);
}

#[test]
fn un_file_del_plotter_ha_la_sua_anteprima() {
    let p = temp("b.plt", b"IN;SP1;PU0,0;PD4000,0,4000,2000,0,2000,0,0;PU;");
    let t = thumbnail(&p, 200).unwrap();
    assert_eq!(t.width.max(t.height), 200);
    // Qualche pixel scuro: le linee.
    assert!(t.rgba.chunks(4).any(|px| px[0] < 128));
}

#[test]
fn un_file_rotto_o_sconosciuto_da_errore_senza_cadere() {
    assert!(thumbnail(&temp("c.pdf", b"%PDF-1.7 rotto"), 256).is_err());
    assert!(thumbnail(&temp("d.dxf", b"non un dxf"), 256).is_err());
    assert!(thumbnail(&temp("e.xyz", b"x"), 256).is_err());
}

#[test]
fn le_estensioni_sono_quelle_dell_installer() {
    let wxs = include_str!("../../../installer/winfiles.wxs");
    for e in EXTENSIONS {
        assert!(
            wxs.contains(&format!(
                r"Software\Classes\.{e}\ShellEx\{{e357fccd-a995-4576-b01f-234630154e96}}"
            )),
            ".{e}"
        );
    }
    assert!(wxs.contains(CLSID));
}
