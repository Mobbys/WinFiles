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
    let _ = std::process::Command::new("regsvr32")
        .args(args)
        // CREATE_NO_WINDOW: niente console che lampeggia.
        .creation_flags(0x0800_0000)
        .status();
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
