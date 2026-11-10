//! Prova della CI su Windows (ADR 0036): chiede a Windows le anteprime di alcuni file come fa
//! Esplora file, cioè attraverso la shell e il suo processo delle anteprime, con la DLL
//! registrata dalle chiavi dell'installer. Esce con errore se una manca.
//!
//! Uso: `cargo run --release -p winfiles-preview --example esplora -- <cartella>`

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_THUMBNAILONLY,
    };
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
    // Controllo: un PNG, di cui Windows fa l'anteprima da solo. Se manca anche questa, il PC non
    // fa anteprime (per esempio la CI che gira come servizio, senza sessione utente).
    let png: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xde, 0x00,
        0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00, 0x00, 0x03, 0x01,
        0x01, 0x00, 0xc9, 0xfe, 0x92, 0xef, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60,
        0x82,
    ];
    let psd = winfiles_preview::prova::psd(200, 100);
    let files: [(&str, &[u8]); 7] = [
        ("controllo.png", png),
        ("prova.svg", svg.as_bytes()),
        ("prova.plt", plt.as_bytes()),
        ("prova.pdf", &pdf),
        // Gli AI moderni sono PDF.
        ("prova.ai", &pdf),
        ("prova.dxf", winfiles_preview::prova::DXF.as_bytes()),
        // Il PSD ha un altro gestore nel suo ProgID (vedi prova-anteprime.ps1).
        ("prova.psd", &psd),
    ];
    let mut ok = true;
    // SAFETY: inizializzazione di COM e chiamate alla shell con argomenti validi.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        // Prima la classe dal registro, nel processo: dice se la DLL vera si carica e funziona.
        let svg_path = dir.join("diretto.svg");
        if std::fs::write(&svg_path, svg).is_ok() {
            println!("classe nel processo: {}", direct(&svg_path));
        }
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

/// La classe creata da COM con il registro (come farebbe il processo delle anteprime), nel
/// processo stesso: carica winfiles_preview.dll e chiede l'anteprima di `path`.
#[cfg(windows)]
fn direct(path: &std::path::Path) -> String {
    use windows::Win32::Graphics::Gdi::HBITMAP;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, STGM_READ};
    use windows::Win32::UI::Shell::PropertiesSystem::IInitializeWithStream;
    use windows::Win32::UI::Shell::{IThumbnailProvider, SHCreateStreamOnFileEx, WTS_ALPHATYPE};
    use windows_core::{GUID, HSTRING, Interface};
    let Ok(clsid) = GUID::try_from(winfiles_preview::CLSID.trim_matches(['{', '}'])) else {
        return "CLSID non valido".into();
    };
    // SAFETY: chiamate COM con argomenti validi.
    let result = unsafe {
        CoCreateInstance::<_, IThumbnailProvider>(&clsid, None, CLSCTX_INPROC_SERVER).and_then(|p| {
            let init: IInitializeWithStream = p.cast()?;
            let stream =
                SHCreateStreamOnFileEx(&HSTRING::from(path.as_os_str()), STGM_READ.0, 0, false, None)?;
            init.Initialize(&stream, STGM_READ.0)?;
            let mut hbmp = HBITMAP::default();
            let mut alpha = WTS_ALPHATYPE::default();
            p.GetThumbnail(256, &mut hbmp, &mut alpha)?;
            Ok(hbmp)
        })
    };
    match result {
        Ok(_) => "anteprima di WinFiles".into(),
        Err(e) => format!("ERRORE {e}"),
    }
}

#[cfg(not(windows))]
fn main() {}
