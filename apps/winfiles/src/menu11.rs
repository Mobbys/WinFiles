//! "Apri in WinFiles" nel menu corto di Esplora file di Windows 11 (ADR 0022).
//!
//! L'MSI mette nella cartella di WinFiles il pacchetto firmato `WinFilesMenu.msix` e la DLL della
//! voce (`crates/menu11`). Il pacchetto va registrato per l'utente, e Windows lo accetta solo se il
//! certificato Winsport che l'ha firmato è fra le "Persone attendibili" del computer. All'avvio
//! WinFiles controlla, in un thread a parte, e registra il pacchetto della sua versione; se manca
//! il certificato, una barra propone di installarlo (una volta per PC, con la conferma
//! dell'amministratore). Tutto passa da PowerShell, senza codice `unsafe`.

// Fuori da Windows gli script non si avviano: restano per i test.
#![cfg_attr(not(windows), allow(dead_code))]

use std::sync::mpsc::{Receiver, channel};

/// Com'è messa la voce nel menu corto.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Registrata per questa versione.
    Ready,
    /// Serve il certificato Winsport su questo PC.
    NeedCert,
    /// Non c'entra: Windows 10, copia portatile, pacchetto assente.
    Unavailable,
    /// La conferma dell'amministratore è stata annullata.
    Cancelled,
    /// Errore di Windows, da mostrare.
    Failed(String),
}

/// Il nome del pacchetto, uguale a `installer/menu-win11/AppxManifest.xml`.
const PACKAGE: &str = "Winsport.WinFiles";
/// Il file del pacchetto nella cartella di WinFiles.
const MSIX: &str = "WinFilesMenu.msix";

/// La versione del pacchetto: quella di WinFiles con uno zero in fondo.
fn package_version() -> String {
    format!("{}.0", env!("CARGO_PKG_VERSION"))
}

/// Un testo fra apici singoli di PowerShell.
fn quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Lo script che controlla e, se può, registra il pacchetto; scrive una parola sola.
pub fn register_script(dir: &str, version: &str) -> String {
    format!(
        r#"$ErrorActionPreference = 'Stop'
try {{
  if ([Environment]::OSVersion.Version.Build -lt 22000) {{ 'NO'; exit }}
  $dir = {dir}
  $pkg = Join-Path $dir {msix}
  if (-not (Test-Path -LiteralPath $pkg)) {{ 'NO'; exit }}
  $p = Get-AppxPackage -Name {name}
  if ($p -and $p.Version -eq {version}) {{ 'OK'; exit }}
  $t = (Get-AuthenticodeSignature -LiteralPath $pkg).SignerCertificate.Thumbprint
  if ($t -and -not (Test-Path ('Cert:\LocalMachine\TrustedPeople\' + $t))) {{ 'CERT'; exit }}
  Add-AppxPackage -Path $pkg -ExternalLocation $dir -ForceUpdateFromAnyVersion
  'OK'
}} catch {{
  $m = $_.Exception.Message
  if ($m -match '0x800B0109|0x800B010A|0x800B0100') {{ 'CERT' }} else {{ 'ERR ' + $m }}
}}"#,
        dir = quoted(dir),
        msix = quoted(MSIX),
        name = quoted(PACKAGE),
        version = quoted(version),
    )
}

/// Lo script che chiede la conferma dell'amministratore e mette il certificato del pacchetto fra
/// le "Persone attendibili" del computer.
pub fn certificate_script(dir: &str) -> String {
    let pkg = format!("{}\\{MSIX}", dir.trim_end_matches(['\\', '/']));
    let elevated = format!(
        r#"$c = (Get-AuthenticodeSignature -LiteralPath {pkg}).SignerCertificate
$s = New-Object System.Security.Cryptography.X509Certificates.X509Store('TrustedPeople', 'LocalMachine')
$s.Open('ReadWrite'); $s.Add($c); $s.Close()"#,
        pkg = quoted(&pkg),
    );
    format!(
        r#"try {{
  $p = Start-Process powershell.exe -Verb RunAs -WindowStyle Hidden -Wait -PassThru -ArgumentList '-NoProfile', '-NonInteractive', '-EncodedCommand', '{encoded}'
}} catch {{ 'ANNULLATO'; exit }}
'FATTO'"#,
        encoded = encoded(&elevated),
    )
}

/// Il testo per `-EncodedCommand`: UTF-16LE in base64, così nessuna virgoletta si perde.
pub fn encoded(script: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let index = ((n >> (18 - 6 * i)) & 63) as usize;
                out.push(char::from(TABLE.get(index).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Legge la parola scritta da [`register_script`].
pub fn parse(out: &str) -> Status {
    let line = out.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("");
    match line {
        "OK" => Status::Ready,
        "CERT" => Status::NeedCert,
        "NO" | "" => Status::Unavailable,
        "ANNULLATO" => Status::Cancelled,
        other => Status::Failed(other.strip_prefix("ERR ").unwrap_or(other).to_string()),
    }
}

/// Controlla e registra il pacchetto in un thread a parte.
pub fn check() -> Receiver<Status> {
    run(false)
}

/// Installa il certificato (con la conferma dell'amministratore), poi registra il pacchetto.
pub fn install_certificate() -> Receiver<Status> {
    run(true)
}

fn run(certificate: bool) -> Receiver<Status> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(work(certificate));
    });
    rx
}

#[cfg(windows)]
fn work(certificate: bool) -> Status {
    if !crate::update::installed() {
        return Status::Unavailable;
    }
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.to_string_lossy().into_owned()))
    else {
        return Status::Unavailable;
    };
    if certificate && parse(&powershell(&certificate_script(&dir))) == Status::Cancelled {
        return Status::Cancelled;
    }
    parse(&powershell(&register_script(&dir, &package_version())))
}

#[cfg(windows)]
fn powershell(script: &str) -> String {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
        ])
        .arg(encoded(script))
        // CREATE_NO_WINDOW: niente console che lampeggia.
        .creation_flags(0x0800_0000)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_else(|e| format!("ERR {e}"))
}

/// Toglie il pacchetto dell'utente: lo chiama l'MSI quando si disinstalla WinFiles
/// (`WinFiles.exe --togli-menu`), così la voce non resta a puntare a una cartella vuota.
pub fn unregister() {
    #[cfg(windows)]
    {
        let script = format!("Get-AppxPackage -Name {} | Remove-AppxPackage", quoted(PACKAGE));
        let _ = powershell(&script);
    }
}

/// Fuori da Windows il menu corto non c'è; per provare la barra si mette la risposta dello
/// script in `WINFILES_MENU11` (per esempio `CERT`).
#[cfg(not(windows))]
fn work(certificate: bool) -> Status {
    match std::env::var("WINFILES_MENU11") {
        Ok(_) if certificate => Status::Ready,
        Ok(answer) => parse(&answer),
        Err(_) => Status::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_codifica_e_quella_di_powershell() {
        // [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes('dir'))
        assert_eq!(encoded("dir"), "ZABpAHIA");
        assert_eq!(encoded("'OK'"), "JwBPAEsAJwA=");
        assert_eq!(encoded(""), "");
    }

    #[test]
    fn le_risposte_dello_script() {
        assert_eq!(parse("OK\r\n"), Status::Ready);
        assert_eq!(parse("CERT"), Status::NeedCert);
        assert_eq!(parse("NO"), Status::Unavailable);
        assert_eq!(parse(""), Status::Unavailable);
        assert_eq!(parse("FATTO\nOK"), Status::Ready);
        assert_eq!(parse("ANNULLATO"), Status::Cancelled);
        assert_eq!(
            parse("ERR Accesso negato"),
            Status::Failed("Accesso negato".into())
        );
    }

    #[test]
    fn un_apostrofo_nel_percorso_non_rompe_lo_script() {
        let s = register_script(
            "C:\\Users\\D'Amico\\AppData\\Local\\Programs\\WinFiles\\",
            "0.2.13.0",
        );
        assert!(s.contains("$dir = 'C:\\Users\\D''Amico\\AppData\\Local\\Programs\\WinFiles\\'"));
        assert!(s.contains("-eq '0.2.13.0'"));
        assert!(s.contains("'Winsport.WinFiles'"));
        let c = certificate_script("C:\\Users\\D'Amico\\WinFiles\\");
        assert!(c.contains("-Verb RunAs"));
        assert!(
            !c.contains("D'Amico"),
            "il percorso va solo nella parte codificata"
        );
    }

    #[test]
    fn il_nome_del_pacchetto_e_quello_del_manifesto() {
        let manifest = include_str!("../../../installer/menu-win11/AppxManifest.xml");
        assert!(manifest.contains(&format!("Identity Name=\"{PACKAGE}\"")));
        assert!(manifest.contains("Publisher=\"CN=Winsport\""));
    }
}
