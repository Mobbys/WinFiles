//! Prova della CI su Windows (ADR 0036): chiede a Windows le anteprime di alcuni file come fa
//! Esplora file, cioè attraverso la shell e il suo processo delle anteprime, con la DLL
//! registrata dalle chiavi dell'installer. Esce con errore se una manca.
//!
//! Uso: `cargo run --release -p winfiles-preview --example esplora -- <cartella>`

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_THUMBNAILONLY};
    use windows_core::HSTRING;

    let Some(dir) = std::env::args().nth(1).map(std::path::PathBuf::from) else {
        eprintln!("manca la cartella");
        return std::process::ExitCode::FAILURE;
    };
    let _ = std::fs::create_dir_all(&dir);
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect width="200" height="100" fill="red"/><text x="10" y="60" font-size="40">Prova</text></svg>"#;
    let plt = "IN;SP1;PU0,0;PD4000,0,4000,2000,0,2000,0,0;PU;SP2;PU2000,1000;CI800;";
    let pdf = winfiles_cad::hpgl::parse(plt.as_bytes())
        .map_err(|e| e.to_string())
        .and_then(|d| {
            d.to_pdf(&winfiles_cad::PdfOptions {
                mm_per_unit: d.fit_scale(200.0),
                ..winfiles_cad::PdfOptions::default()
            })
        });
    let Ok(pdf) = pdf else {
        eprintln!("PDF di prova non creato");
        return std::process::ExitCode::FAILURE;
    };
    let files: [(&str, &[u8]); 4] = [
        ("prova.svg", svg.as_bytes()),
        ("prova.plt", plt.as_bytes()),
        ("prova.pdf", &pdf),
        // Gli AI moderni sono PDF.
        ("prova.ai", &pdf),
    ];
    let mut ok = true;
    // SAFETY: inizializzazione di COM e chiamate alla shell con argomenti validi.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        for (name, bytes) in files {
            let path = dir.join(name);
            if std::fs::write(&path, bytes).is_err() {
                eprintln!("{name}: non scritto");
                ok = false;
                continue;
            }
            let result = SHCreateItemFromParsingName::<_, _, IShellItemImageFactory>(
                &HSTRING::from(path.as_os_str()),
                None,
            )
            .and_then(|f| f.GetImage(SIZE { cx: 256, cy: 256 }, SIIGBF_THUMBNAILONLY));
            match result {
                Ok(_) => println!("{name}: anteprima di WinFiles"),
                Err(e) => {
                    println!("{name}: NESSUNA anteprima ({e})");
                    ok = false;
                }
            }
        }
    }
    if ok {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

#[cfg(not(windows))]
fn main() {}
