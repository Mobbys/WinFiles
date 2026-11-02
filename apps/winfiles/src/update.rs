//! Aggiornamenti: all'avvio WinFiles chiede a GitHub l'ultima versione pubblicata nei
//! rilasci del repository (pubblico, codice proprietario: ADR 0014). Se è più nuova la si
//! annuncia; con "Aggiorna" si scarica l'installer e, alla chiusura di WinFiles, si installa e
//! si riapre la versione nuova. La copia portatile (fuori dalla cartella dell'installer) apre
//! invece la pagina del rilascio. Come la 4.9, senza mai bloccare la finestra: rete e download
//! stanno in un thread a parte.
//!
//! Le richieste passano da `curl`, che c'è in Windows 10 e 11: niente librerie di rete e TLS
//! dentro WinFiles.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, channel};

/// L'ultima versione pubblicata, dall'API di GitHub.
const LATEST: &str = "https://api.github.com/repos/Mobbys/WinFiles/releases/latest";
/// Risposta dell'API letta al massimo.
const MAX_ANSWER: usize = 1024 * 1024;

/// Una versione `maggiore.minore.correzione`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// Da "v0.2.0", "0.2" o "0.2.0-beta" (la parte dopo il trattino non conta).
    pub fn parse(text: &str) -> Option<Self> {
        let core = text.trim().trim_start_matches(['v', 'V']);
        let core = core.split(['-', '+']).next()?;
        let mut parts = core.split('.').map(|p| p.parse::<u32>().ok());
        let major = parts.next()??;
        let minor = parts.next().unwrap_or(Some(0))?;
        let patch = parts.next().unwrap_or(Some(0))?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self(major, minor, patch))
    }

    /// Questa copia di WinFiles.
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Self(0, 0, 0))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// Un rilascio pubblicato.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: Version,
    /// Le novità, scritte nel rilascio.
    pub notes: String,
    /// La pagina del rilascio, per chi usa la copia portatile.
    pub page: String,
    /// L'installer MSI, se il rilascio lo ha.
    pub msi: Option<String>,
}

/// Il rilascio dalla risposta dell'API; `None` per bozze, versioni di prova e risposte rotte.
pub fn parse(json: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    if v.get("draft")?.as_bool()? || v.get("prerelease").and_then(|p| p.as_bool()).unwrap_or(false) {
        return None;
    }
    let version = Version::parse(v.get("tag_name")?.as_str()?)?;
    let text = |key: &str| {
        v.get(key)
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let msi = v
        .get("assets")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let name = a.get("name")?.as_str()?;
            let url = a.get("browser_download_url")?.as_str()?;
            (name.to_ascii_lowercase().ends_with(".msi") && url.starts_with("https://"))
                .then(|| url.to_string())
        })
        .next();
    let page = text("html_url");
    Some(Release {
        version,
        notes: text("body").chars().take(4000).collect(),
        page: if page.starts_with("https://") {
            page
        } else {
            String::new()
        },
        msi,
    })
}

/// A che punto è l'aggiornamento.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading(Release),
    /// Installer scaricato: si installa alla chiusura.
    Ready(Release, PathBuf),
    Failed(String),
}

enum Event {
    Checked(Result<Option<Release>, String>),
    Downloaded(Release, Result<PathBuf, String>),
}

/// Controllo e download in un thread a parte.
pub struct Updater {
    pub state: State,
    /// Il controllo l'ha chiesto chi usa WinFiles (Aiuto): si dice anche "è aggiornato".
    pub asked: bool,
    /// L'avviso è stato chiuso con "Più tardi".
    pub dismissed: bool,
    rx: Option<Receiver<Event>>,
}

impl Default for Updater {
    fn default() -> Self {
        Self {
            state: State::Idle,
            asked: false,
            dismissed: false,
            rx: None,
        }
    }
}

impl Updater {
    /// Chiede l'ultima versione, se non lo sta già facendo.
    pub fn check(&mut self, asked: bool, wake: impl Fn() + Send + 'static) {
        if self.rx.is_some() || matches!(self.state, State::Downloading(_) | State::Ready(..)) {
            return;
        }
        self.asked = asked;
        self.dismissed = false;
        // Per le prove si può indicare un'altra risposta (anche un file:// locale).
        let url = std::env::var("WINFILES_AGGIORNAMENTI").unwrap_or_else(|_| LATEST.to_string());
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("aggiornamenti".into())
            .spawn(move || {
                let result = fetch(&url).map(|json| parse(&json).filter(|r| r.version > Version::current()));
                let _ = tx.send(Event::Checked(result));
                wake();
            });
        if spawned.is_ok() {
            self.rx = Some(rx);
            self.state = State::Checking;
        }
    }

    /// Scarica l'installer del rilascio annunciato.
    pub fn download(&mut self, wake: impl Fn() + Send + 'static) {
        let State::Available(release) = self.state.clone() else {
            return;
        };
        let Some(url) = release.msi.clone() else {
            return;
        };
        let (tx, rx) = channel();
        let r = release.clone();
        let spawned = std::thread::Builder::new()
            .name("scarica-aggiornamento".into())
            .spawn(move || {
                let path = std::env::temp_dir().join(format!("WinFiles-{}.msi", r.version));
                let result = download(&url, &path).map(|()| path);
                let _ = tx.send(Event::Downloaded(r, result));
                wake();
            });
        if spawned.is_ok() {
            self.rx = Some(rx);
            self.state = State::Downloading(release);
        }
    }

    /// Le risposte arrivate dal thread.
    pub fn poll(&mut self) {
        let Some(event) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.rx = None;
        self.state = match event {
            Event::Checked(Ok(Some(r))) => State::Available(r),
            Event::Checked(Ok(None)) => State::UpToDate,
            Event::Checked(Err(e)) => State::Failed(e),
            Event::Downloaded(r, Ok(path)) => State::Ready(r, path),
            Event::Downloaded(_, Err(e)) => State::Failed(e),
        };
    }
}

/// WinFiles gira dalla cartella dell'installer (non dalla copia portatile).
pub fn installed() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Some(local) = std::env::var_os("LOCALAPPDATA") else {
        return false;
    };
    exe.parent()
        .is_some_and(|dir| same_dir(dir, &Path::new(&local).join("Programs").join("WinFiles")))
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
    norm(a) == norm(b)
}

/// Il comando `curl` senza finestra nera su Windows.
fn curl() -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("curl.exe");
        // CREATE_NO_WINDOW: niente console che lampeggia sopra WinFiles.
        c.creation_flags(0x0800_0000);
        c
    }
    #[cfg(not(windows))]
    {
        Command::new("curl")
    }
}

fn fetch(url: &str) -> Result<String, String> {
    let out = curl()
        .args([
            "-sSfL",
            "--max-time",
            "20",
            "-H",
            "Accept: application/vnd.github+json",
        ])
        .args(["-A", concat!("WinFiles/", env!("CARGO_PKG_VERSION"))])
        .arg(url)
        .output()
        .map_err(|e| format!("Non riesco a controllare gli aggiornamenti: {e}"))?;
    if !out.status.success() {
        return Err("Non riesco a raggiungere GitHub per controllare gli aggiornamenti".into());
    }
    if out.stdout.len() > MAX_ANSWER {
        return Err("Risposta di GitHub troppo lunga".into());
    }
    String::from_utf8(out.stdout).map_err(|_| "Risposta di GitHub non leggibile".into())
}

fn download(url: &str, path: &Path) -> Result<(), String> {
    let part = path.with_extension("msi.part");
    let ok = curl()
        .args(["-sSfL", "--max-time", "600", "-o"])
        .arg(&part)
        .arg(url)
        .status()
        .map_err(|e| format!("Non riesco a scaricare l'aggiornamento: {e}"))?
        .success();
    if !ok {
        let _ = std::fs::remove_file(&part);
        return Err("Download dell'aggiornamento non riuscito".into());
    }
    std::fs::rename(&part, path).map_err(|e| format!("Non riesco a salvare l'aggiornamento: {e}"))
}

/// Avvia l'installazione di `msi` e la riapertura di WinFiles, dopo che questa copia si è chiusa.
pub fn install_after_exit(msi: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let exe = Path::new(&std::env::var_os("LOCALAPPDATA").unwrap_or_default())
            .join("Programs")
            .join("WinFiles")
            .join("WinFiles.exe");
        // Due secondi perché WinFiles finisca di chiudersi, poi l'installer con la sola barra di
        // avanzamento e la versione nuova.
        let script = format!(
            "timeout /t 2 /nobreak >nul & msiexec /i \"{}\" /passive & start \"\" \"{}\"",
            msi.display(),
            exe.display()
        );
        Command::new("cmd")
            .arg("/c")
            .raw_arg(script)
            .creation_flags(0x0800_0000)
            .spawn()
            .map(drop)
    }
    #[cfg(not(windows))]
    {
        let _ = msi;
        Err(std::io::Error::other("L'installer MSI si usa solo su Windows"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versioni() {
        assert_eq!(Version::parse("v0.2.0"), Some(Version(0, 2, 0)));
        assert_eq!(Version::parse("1.4"), Some(Version(1, 4, 0)));
        assert_eq!(Version::parse("2.0.1-beta"), Some(Version(2, 0, 1)));
        assert_eq!(Version::parse("pippo"), None);
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert_eq!(Version(1, 2, 3).to_string(), "1.2.3");
    }

    #[test]
    fn rilascio_da_github() {
        let json = r#"{"tag_name":"v0.3.1","draft":false,"prerelease":false,
            "html_url":"https://github.com/Mobbys/WinFiles/releases/tag/v0.3.1",
            "body":"- Foto HEIC\n- AI da taglio",
            "assets":[{"name":"WinFiles-0.3.1-portatile.exe","browser_download_url":"https://x/a.exe"},
                      {"name":"WinFiles-0.3.1.msi","browser_download_url":"https://x/b.msi"}]}"#;
        let r = parse(json).unwrap();
        assert_eq!(r.version, Version(0, 3, 1));
        assert_eq!(r.msi.as_deref(), Some("https://x/b.msi"));
        assert!(r.notes.contains("HEIC"));
        // Bozze e versioni di prova non si annunciano; risposte rotte nemmeno.
        assert!(parse(&json.replace("\"draft\":false", "\"draft\":true")).is_none());
        assert!(parse(&json.replace("\"prerelease\":false", "\"prerelease\":true")).is_none());
        assert!(parse("{\"message\":\"Not Found\"}").is_none());
        assert!(parse("<html>").is_none());
        // Un indirizzo non https non si scarica.
        assert_eq!(
            parse(&json.replace("https://x/b.msi", "http://x/b.msi"))
                .unwrap()
                .msi,
            None
        );
    }

    #[test]
    fn cartella_dell_installer() {
        assert!(same_dir(
            Path::new("C:\\Users\\Utente\\AppData\\Local\\Programs\\WinFiles\\"),
            Path::new("c:\\users\\utente\\appdata\\local\\Programs\\WinFiles")
        ));
        assert!(!same_dir(
            Path::new("C:\\Download"),
            Path::new("C:\\Programs\\WinFiles")
        ));
    }
}
