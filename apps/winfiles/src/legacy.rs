//! La vecchia WinFile (4.9 e prima, in Python) sullo stesso PC. Chi aggiorna la 4.9 si trova
//! WinFiles installata accanto: la 4.9 resta e a ogni avvio riproporrebbe l'aggiornamento.
//! WinFiles la trova dalla sua voce in "App installate" e propone di toglierla, una volta sola
//! se si sceglie di tenerla (la 4.9 ha anche la Simulazione quote, che WinFiles non ha).

use std::process::Command;
use std::sync::mpsc::{Receiver, channel};

/// La chiave che l'installer NSIS della 4.9 scrive per l'utente.
#[cfg(windows)]
const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\WinFile";

/// La vecchia WinFile installata: versione e comando per toglierla.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Legacy {
    pub version: String,
    pub uninstall: String,
}

/// Legge l'uscita di `reg query` sulla chiave della 4.9.
pub fn parse(out: &str) -> Option<Legacy> {
    let value = |name: &str| {
        out.lines().find_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix(name)?;
            let (_, v) = rest.split_once("REG_SZ")?;
            Some(v.trim().to_string())
        })
    };
    let uninstall = value("UninstallString").filter(|s| !s.is_empty())?;
    let version = value("DisplayVersion").unwrap_or_default();
    Some(Legacy { version, uninstall })
}

/// Il percorso del disinstallatore, senza virgolette: lo si avvia come programma, mai con la
/// shell, così un percorso scritto male nel registro non diventa un comando.
fn program(uninstall: &str) -> &str {
    let s = uninstall.trim();
    match s.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(rest),
        None => s,
    }
}

/// Cerca la 4.9 senza fermare la finestra: il risultato arriva sul canale.
pub fn find() -> Receiver<Option<Legacy>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(query());
    });
    rx
}

#[cfg(windows)]
fn query() -> Option<Legacy> {
    use std::os::windows::process::CommandExt;
    let out = Command::new("reg")
        .args(["query", KEY])
        // CREATE_NO_WINDOW: niente console che lampeggia.
        .creation_flags(0x0800_0000)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse(&String::from_utf8_lossy(&out.stdout))
}

/// Fuori da Windows la 4.9 non c'è; per provare la barra si passa l'uscita di `reg query` in
/// `WINFILES_VECCHIA`.
#[cfg(not(windows))]
fn query() -> Option<Legacy> {
    std::env::var("WINFILES_VECCHIA").ok().as_deref().and_then(parse)
}

/// Avvia il disinstallatore della 4.9, che mostra la sua finestra.
pub fn remove(l: &Legacy) -> std::io::Result<()> {
    Command::new(program(&l.uninstall)).spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\WinFile\r\n    DisplayName    REG_SZ    WinFile\r\n    UninstallString    REG_SZ    \"C:\\Users\\Ana\\AppData\\Roaming\\WinFile\\uninstall.exe\"\r\n    DisplayIcon    REG_SZ    C:\\Users\\Ana\\AppData\\Roaming\\WinFile\\icona.ico\r\n    DisplayVersion    REG_SZ    4.9\r\n";

    #[test]
    fn legge_la_voce_della_4_9() {
        let l = parse(OUT).unwrap();
        assert_eq!(l.version, "4.9");
        assert_eq!(
            program(&l.uninstall),
            "C:\\Users\\Ana\\AppData\\Roaming\\WinFile\\uninstall.exe"
        );
    }

    #[test]
    fn senza_disinstallatore_non_c_e() {
        assert_eq!(parse("ERRORE: impossibile trovare la chiave"), None);
        assert_eq!(parse("    DisplayVersion    REG_SZ    4.9"), None);
    }

    #[test]
    fn percorso_senza_virgolette() {
        assert_eq!(program(r"C:\WinFile\uninstall.exe"), r"C:\WinFile\uninstall.exe");
        assert_eq!(
            program(r#""C:\Win File\uninstall.exe" /S"#),
            r"C:\Win File\uninstall.exe"
        );
    }
}
