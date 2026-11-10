//! La diagnosi delle anteprime sul PC di chi le usa: cosa vede Windows per ogni tipo, se la classe
//! di WinFiles si avvia e se Esplora file la usa. Si lancia con
//! `rundll32 "%LOCALAPPDATA%\Programs\WinFiles\winfiles_preview.dll",Diagnosi`
//! e scrive il resoconto in `%TEMP%\winfiles-diagnosi.txt`, che apre nel Blocco note.

use std::fmt::Write as _;

use windows::Win32::Foundation::{HINSTANCE, HWND, SIZE};
use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP, HGDIOBJ};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, STGM_READ,
};
use windows::Win32::System::Registry::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use windows::Win32::UI::Shell::PropertiesSystem::IInitializeWithStream;
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, IThumbnailProvider, SHCreateItemFromParsingName, SHCreateStreamOnFileEx,
    SIIGBF_THUMBNAILONLY, WTS_ALPHATYPE,
};
use windows_core::{GUID, HSTRING, Interface, PCSTR};

use crate::reg::{THUMB_KEY, exists, get, prog_ids};

fn or_none(v: Option<String>) -> String {
    v.unwrap_or_else(|| "(niente)".into())
}

/// La classe di WinFiles creata nel processo, dal registro, e l'anteprima di `path`.
fn in_process(path: &std::path::Path) -> String {
    let Ok(clsid) = GUID::try_from(crate::CLSID.trim_matches(['{', '}'])) else {
        return "CLSID non valido".into();
    };
    // SAFETY: chiamate COM con argomenti validi; la bitmap si libera.
    let result = unsafe {
        CoCreateInstance::<_, IThumbnailProvider>(&clsid, None, CLSCTX_INPROC_SERVER).and_then(|p| {
            let init: IInitializeWithStream = p.cast()?;
            let stream =
                SHCreateStreamOnFileEx(&HSTRING::from(path.as_os_str()), STGM_READ.0, 0, false, None)?;
            init.Initialize(&stream, STGM_READ.0)?;
            let mut hbmp = HBITMAP::default();
            let mut alpha = WTS_ALPHATYPE::default();
            p.GetThumbnail(256, &mut hbmp, &mut alpha)?;
            let _ = DeleteObject(HGDIOBJ(hbmp.0));
            Ok(())
        })
    };
    match result {
        Ok(()) => "OK".into(),
        Err(e) => format!("ERRORE {e}"),
    }
}

/// L'anteprima chiesta a Windows come fa Esplora file (con il suo processo delle anteprime).
fn from_shell(path: &std::path::Path) -> String {
    // SAFETY: chiamate alla shell con argomenti validi; la bitmap si libera.
    let result = unsafe {
        SHCreateItemFromParsingName::<_, _, IShellItemImageFactory>(&HSTRING::from(path.as_os_str()), None)
            .and_then(|f| f.GetImage(SIZE { cx: 256, cy: 256 }, SIIGBF_THUMBNAILONLY))
            .map(|h| {
                let _ = DeleteObject(HGDIOBJ(h.0));
            })
    };
    match result {
        Ok(()) => "OK".into(),
        Err(e) => format!("ERRORE {e}"),
    }
}

fn report() -> String {
    let clsid = crate::CLSID;
    let mut r = String::new();
    let _ = writeln!(
        r,
        "Diagnosi delle anteprime di WinFiles {} (build {})\n",
        env!("CARGO_PKG_VERSION"),
        option_env!("GITHUB_SHA").map_or("locale", |s| s.get(..7).unwrap_or(s))
    );
    let server = format!(r"CLSID\{clsid}\InprocServer32");
    let dll = get(HKEY_CLASSES_ROOT, &server, None);
    let _ = writeln!(r, "Classe (HKCR): {}", or_none(dll.clone()));
    let _ = writeln!(
        r,
        "  file: {}",
        dll.as_deref()
            .map_or("-", |d| if std::path::Path::new(d).is_file() {
                "c'è"
            } else {
                "MANCA"
            })
    );
    let _ = writeln!(
        r,
        "Classe per l'utente (HKCU): {}",
        or_none(get(
            HKEY_CURRENT_USER,
            &format!(r"Software\Classes\{server}"),
            None
        ))
    );
    let _ = writeln!(
        r,
        "Classe per il PC (HKLM): {}",
        or_none(get(
            HKEY_LOCAL_MACHINE,
            &format!(r"Software\Classes\{server}"),
            None
        ))
    );
    let _ = writeln!(
        r,
        "ThreadingModel: {}",
        or_none(get(HKEY_CLASSES_ROOT, &server, Some("ThreadingModel")))
    );
    let _ = writeln!(
        r,
        "ProgID toccati da WinFiles: {}\n",
        or_none(get(
            HKEY_CURRENT_USER,
            r"Software\WinFiles",
            Some("AnteprimeProgId")
        ))
    );
    let _ = writeln!(r, "Gestori delle anteprime per tipo (ProgID, poi estensione):");
    for ext in crate::EXTENSIONS {
        let mut line = format!(".{ext}:");
        for p in prog_ids(ext) {
            let h = get(HKEY_CLASSES_ROOT, &format!(r"{p}\{THUMB_KEY}"), None);
            let _ = write!(line, " [{p} → {}]", or_none(h));
        }
        let e = get(HKEY_CLASSES_ROOT, &format!(r".{ext}\{THUMB_KEY}"), None);
        let _ = write!(line, " [estensione → {}]", or_none(e));
        let s = get(
            HKEY_CLASSES_ROOT,
            &format!(r"SystemFileAssociations\.{ext}\{THUMB_KEY}"),
            None,
        );
        if s.is_some() {
            let _ = write!(line, " [SystemFileAssociations → {}]", or_none(s));
        }
        let _ = writeln!(r, "{line}");
    }
    let _ = writeln!(
        r,
        "\nIsolamento disattivato: {}",
        if exists(HKEY_CLASSES_ROOT, &format!(r"CLSID\{clsid}"))
            && get(
                HKEY_CLASSES_ROOT,
                &format!(r"CLSID\{clsid}"),
                Some("DisableProcessIsolation")
            )
            .is_some()
        {
            "sì"
        } else {
            "no"
        }
    );

    // I file di prova, con nomi nuovi ogni volta (Windows ricorda le anteprime già fatte).
    let dir = std::env::temp_dir().join(format!(
        "winfiles-diagnosi-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    ));
    let _ = std::fs::create_dir_all(&dir);
    let pdf = crate::prova::pdf().unwrap_or_default();
    let psd = crate::prova::psd(200, 100);
    let files: [(&str, &[u8]); 5] = [
        ("prova.svg", crate::prova::SVG.as_bytes()),
        ("prova.pdf", &pdf),
        ("prova.ai", &pdf),
        ("prova.psd", &psd),
        ("prova.dxf", crate::prova::DXF.as_bytes()),
    ];
    let _ = writeln!(r, "\nProve (cartella {}):", dir.display());
    // SAFETY: inizializzazione di COM nel thread di rundll32.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    for (name, bytes) in files {
        let path = dir.join(name);
        if std::fs::write(&path, bytes).is_err() {
            let _ = writeln!(r, "{name}: non scritto");
            continue;
        }
        let _ = writeln!(
            r,
            "{name}: classe nel processo {}; da Esplora file {}",
            in_process(&path),
            from_shell(&path)
        );
    }
    // Il registro degli errori della DLL, se c'è.
    for d in crate::com::work_dirs() {
        if let Ok(text) = std::fs::read_to_string(d.join("errori.txt")) {
            let tail: Vec<&str> = text.lines().rev().take(40).collect();
            let _ = writeln!(r, "\nRegistro {}:", d.display());
            for l in tail.into_iter().rev() {
                let _ = writeln!(r, "  {l}");
            }
        }
    }
    r
}

/// Il punto d'ingresso per rundll32.
#[unsafe(no_mangle)]
pub extern "system" fn Diagnosi(_hwnd: HWND, _hinst: HINSTANCE, _cmd: PCSTR, _show: i32) {
    let text = std::panic::catch_unwind(report)
        .unwrap_or_else(|_| "La diagnosi si è fermata per un errore interno.".into());
    let path = std::env::temp_dir().join("winfiles-diagnosi.txt");
    if std::fs::write(&path, text.replace('\n', "\r\n")).is_ok() {
        let _ = std::process::Command::new("notepad.exe").arg(&path).spawn();
    }
}
