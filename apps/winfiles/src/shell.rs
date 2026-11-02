//! Aprire un file con il programma predefinito o mostrarlo nella sua cartella, finché i moduli
//! Immagini e PDF (Fasi 8 e 9) non hanno un editor proprio.

use std::path::Path;
use std::process::Command;

/// Apre il file con il programma che il sistema associa al suo tipo.
pub fn open(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("explorer");
        c.arg(path);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(path);
        c
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    cmd.spawn().map(drop)
}

/// Apre la cartella del file in Esplora file, con il file già selezionato.
pub fn reveal(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Esplora file vuole `/select,"percorso"` come un solo argomento, senza altre virgolette.
        Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()
            .map(drop)
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg("-R").arg(path).spawn().map(drop)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        open(path.parent().unwrap_or(path))
    }
}
