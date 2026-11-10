//! Le anteprime in Esplora file (ADR 0036): a ogni avvio WinFiles chiede alla sua DLL delle
//! anteprime di mettersi davanti ai gestori degli altri programmi (Illustrator, Photoshop, CAD),
//! che Windows interroga per primi. Lo fa `regsvr32 /s`, in un thread a parte; nella copia
//! portatile la DLL non c'è e non succede niente.

use std::path::PathBuf;

const DLL: &str = "winfiles_preview.dll";

fn dll() -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.parent()?.join(DLL);
    p.is_file().then_some(p)
}

#[cfg(windows)]
fn regsvr32(args: &[&std::ffi::OsStr]) {
    use std::os::windows::process::CommandExt;
    // Il regsvr32 a 64 bit di Windows, con il percorso intero.
    let exe = std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join(r"System32\regsvr32.exe"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("regsvr32"));
    let status = std::process::Command::new(&exe)
        .args(args)
        // CREATE_NO_WINDOW: niente console che lampeggia.
        .creation_flags(0x0800_0000)
        .status();
    // L'esito va nel registro della DLL, che la diagnosi mostra.
    let outcome = match status {
        Ok(s) if s.success() => "riuscito".to_string(),
        Ok(s) => format!("non riuscito, codice {:#x}", s.code().unwrap_or(-1)),
        Err(e) => format!("non riuscito: {e}"),
    };
    log(&format!("WinFiles: {} {args:?} {outcome}", exe.display()));
}

/// Una riga nel registro degli errori delle anteprime (`%TEMP%\winfiles-anteprime\errori.txt`).
#[cfg(windows)]
fn log(msg: &str) {
    use std::io::Write;
    let dir = std::env::temp_dir().join("winfiles-anteprime");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("errori.txt"))
    {
        let _ = writeln!(f, "[{}] {msg}", std::process::id());
    }
}

/// All'avvio: le anteprime di WinFiles anche per i tipi che un altro programma ha già preso.
pub fn register() {
    #[cfg(windows)]
    if let Some(dll) = dll() {
        let _ = std::thread::Builder::new()
            .name("anteprime".into())
            .spawn(move || regsvr32(&["/s".as_ref(), dll.as_os_str()]));
    }
    #[cfg(not(windows))]
    let _ = dll();
}

/// Alla disinstallazione: toglie quello che `register` aveva aggiunto.
pub fn unregister() {
    #[cfg(windows)]
    if let Some(dll) = dll() {
        regsvr32(&["/u".as_ref(), "/s".as_ref(), dll.as_os_str()]);
    }
}
