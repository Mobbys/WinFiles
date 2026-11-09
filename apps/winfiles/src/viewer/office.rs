//! Documenti di Word, Excel, PowerPoint e InDesign in Visualizza (ADR 0035): li trasformano in PDF
//! i convertitori di WordCraft, GridCraft, DeckCraft e DesignCraft, che l'installazione mette
//! nella cartella `convertitori` accanto a WinFiles. Girano in un processo a parte, con un tempo
//! massimo: un file rovinato che li fa cadere non tocca la finestra.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Il programma che ha creato il file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Office {
    Word,
    Excel,
    PowerPoint,
    InDesign,
}

/// Tempo massimo di una conversione.
const TIMEOUT: Duration = Duration::from_secs(180);
/// Byte massimi del file da convertire.
const MAX_BYTES: u64 = 512 * 1024 * 1024;

impl Office {
    pub fn of(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "docx" | "docm" | "dotx" | "doc" | "odt" | "rtf" => Office::Word,
            "xlsx" | "xlsm" | "xltx" | "xls" | "xlsb" | "ods" => Office::Excel,
            "pptx" | "pptm" | "potx" => Office::PowerPoint,
            "indd" | "indt" | "idml" => Office::InDesign,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Office::Word => "Documento di Word",
            Office::Excel => "Foglio di Excel",
            Office::PowerPoint => "Presentazione di PowerPoint",
            Office::InDesign => "Documento di InDesign",
        }
    }

    /// Il convertitore e chi lo fa.
    fn tool(self) -> (&'static str, &'static str) {
        match self {
            Office::Word => ("wordcraft-cli", "WordCraft"),
            Office::Excel => ("gridcraft-cli", "GridCraft"),
            Office::PowerPoint => ("deckcraft-cli", "DeckCraft"),
            Office::InDesign => ("designcraft-cli", "DesignCraft"),
        }
    }

    /// Gli argomenti per convertire `input` in `output` (PDF).
    pub fn args(self, input: &Path, output: &Path) -> Vec<std::ffi::OsString> {
        let (i, o) = (input.as_os_str().to_owned(), output.as_os_str().to_owned());
        match self {
            Office::InDesign => vec!["run".into(), "--in".into(), i, "--export".into(), o],
            _ => vec!["convert".into(), i, o],
        }
    }

    /// La nota per il pannello Informazioni.
    pub fn note(self) -> String {
        format!(
            "{}: convertito in PDF con {} (impaginazione e caratteri possono differire un poco dall'originale)",
            self.label(),
            self.tool().1
        )
    }
}

/// La cartella dei convertitori: `WINFILES_CONVERTITORI` (per le prove) o `convertitori` accanto
/// all'eseguibile.
fn tools_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("WINFILES_CONVERTITORI") {
        return Some(PathBuf::from(dir));
    }
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|d| d.join("convertitori"))
}

fn tool_path(kind: Office) -> Option<PathBuf> {
    let name = format!("{}{}", kind.tool().0, std::env::consts::EXE_SUFFIX);
    tools_dir().map(|d| d.join(name)).filter(|p| p.is_file())
}

/// Converte il file in PDF e ne restituisce i byte.
pub fn to_pdf(path: &Path, kind: Office) -> Result<Vec<u8>, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > MAX_BYTES {
        return Err(format!("il file è troppo grande ({len} byte)"));
    }
    let Some(tool) = tool_path(kind) else {
        return Err(format!(
            "per aprire un {} serve WinFiles installato con l'MSI: il convertitore {} non c'è (la copia portatile non lo ha)",
            kind.label().to_lowercase(),
            kind.tool().1
        ));
    };
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let out = std::env::temp_dir().join(format!("winfiles-documento-{}-{n}.pdf", std::process::id()));
    let result = run(&tool, &kind.args(path, &out));
    let pdf = result
        .and_then(|()| std::fs::read(&out).map_err(|_| "il convertitore non ha scritto il PDF".to_string()));
    let _ = std::fs::remove_file(&out);
    pdf.map_err(|e| format!("{} non è riuscito a convertire il file: {e}", kind.tool().1))
}

fn run(tool: &Path, args: &[std::ffi::OsString]) -> Result<(), String> {
    let mut cmd = std::process::Command::new(tool);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: niente console che lampeggia.
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    // L'errore del convertitore, letto mentre lavora (un tubo pieno lo fermerebbe).
    let stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut s) = stderr {
            let _ = std::io::Read::read_to_string(&mut std::io::Read::take(&mut s, 64 * 1024), &mut text);
        }
        text
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("ci ha messo più di {} secondi", TIMEOUT.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let err = reader.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        let line = err
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .unwrap_or("errore sconosciuto");
        Err(line.chars().take(300).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i_tipi_si_riconoscono_dall_estensione() {
        assert_eq!(Office::of(Path::new("a.DOCX")), Some(Office::Word));
        assert_eq!(Office::of(Path::new("b.xls")), Some(Office::Excel));
        assert_eq!(Office::of(Path::new("c.pptx")), Some(Office::PowerPoint));
        assert_eq!(Office::of(Path::new("d.indd")), Some(Office::InDesign));
        assert_eq!(Office::of(Path::new("e.idml")), Some(Office::InDesign));
        assert_eq!(Office::of(Path::new("f.pdf")), None);
    }

    #[test]
    fn gli_argomenti_dei_convertitori() {
        let (i, o) = (Path::new("in.docx"), Path::new("out.pdf"));
        assert_eq!(Office::Word.args(i, o), vec!["convert", "in.docx", "out.pdf"]);
        assert_eq!(
            Office::InDesign.args(Path::new("in.indd"), o),
            vec!["run", "--in", "in.indd", "--export", "out.pdf"]
        );
    }
}
