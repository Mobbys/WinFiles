//! Aiuto › «Segnala un problema» e «Chiedi una nuova funzione»: una finestra dentro WinFiles
//! dove si scrive la segnalazione, che WinFiles manda da solo alle Issues di Mobbys/WinFiles.
//! A chi segnala non serve un account GitHub: l'invio usa una chiave che può solo creare issue
//! su quel repository (ADR 0029), messa nell'eseguibile dalla CI con la variabile
//! `WINFILES_SEGNALAZIONI` (segreto del repository). Una copia compilata senza chiave, o un invio
//! non riuscito, lascia aprire il modulo di GitHub nel browser con tutto già scritto
//! (`.github/ISSUE_TEMPLATE`) o copiare il testo.
//!
//! Versione di WinFiles, versione di Windows e scheda in uso si scrivono da sole. La rete passa da
//! `curl`, in un thread a parte, come gli aggiornamenti.

use std::fmt::Write as _;
use std::sync::mpsc::{Receiver, channel};

use crate::app::dim_label;
use crate::theme::{Tokens, semibold};

/// Dove si creano le issue con la chiave.
const API: &str = "https://api.github.com/repos/Mobbys/WinFiles/issues";
/// Il modulo nel browser, per chi ha un account GitHub.
const NEW_ISSUE: &str = "https://github.com/Mobbys/WinFiles/issues/new";
/// Testo massimo di una segnalazione, per non mandare a GitHub un incolla sbagliato di megabyte.
const MAX_TEXT: usize = 20_000;
/// Risposta di GitHub letta al massimo.
const MAX_ANSWER: usize = 1024 * 1024;

/// La chiave, se questa copia è stata compilata dalla CI con il segreto.
fn token() -> Option<&'static str> {
    option_env!("WINFILES_SEGNALAZIONI")
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Problem,
    Feature,
}

impl Kind {
    /// Il file del modulo in `.github/ISSUE_TEMPLATE`.
    fn template(self) -> &'static str {
        match self {
            Self::Problem => "problema.yml",
            Self::Feature => "funzione.yml",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Problem => "Segnala un problema",
            Self::Feature => "Chiedi una nuova funzione",
        }
    }

    /// Davanti al titolo della issue, per riconoscerle nell'elenco di GitHub.
    fn prefix(self) -> &'static str {
        match self {
            Self::Problem => "Problema",
            Self::Feature => "Funzione",
        }
    }
}

/// Quello che WinFiles sa di sé e scrive da solo nella segnalazione.
#[derive(Clone, Debug)]
pub struct Info {
    pub version: String,
    pub windows: String,
    pub installed: bool,
    pub module: &'static str,
}

impl Info {
    pub fn current(module: &'static str) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            windows: windows_version(),
            installed: crate::update::installed(),
            module,
        }
    }

    fn version_line(&self) -> String {
        let copy = if self.installed { "installata" } else { "portatile" };
        format!("WinFiles {} ({copy})", self.version)
    }
}

/// Quello che scrive chi segnala.
#[derive(Clone, Debug, Default)]
pub struct Draft {
    pub kind: Option<Kind>,
    pub title: String,
    pub what: String,
    pub steps: String,
    pub name: String,
}

impl Draft {
    fn kind(&self) -> Kind {
        self.kind.unwrap_or(Kind::Problem)
    }

    fn ready(&self) -> bool {
        !self.title.trim().is_empty()
            && !self.what.trim().is_empty()
            && self.title.len() + self.what.len() + self.steps.len() + self.name.len() <= MAX_TEXT
    }

    /// Il titolo della issue.
    pub fn issue_title(&self) -> String {
        let title: String = self.title.split_whitespace().collect::<Vec<_>>().join(" ");
        format!("{}: {title}", self.kind().prefix())
    }

    /// Il testo della issue, in Markdown, con le stesse sezioni dei moduli di GitHub.
    pub fn issue_body(&self, info: &Info) -> String {
        let mut body = String::new();
        let (what, steps) = match self.kind() {
            Kind::Problem => ("Cosa è successo", "Come rifarlo"),
            Kind::Feature => ("Cosa vorresti", "Per quale lavoro"),
        };
        let section = |body: &mut String, head: &str, text: &str| {
            let text = text.trim();
            if !text.is_empty() {
                let _ = write!(body, "### {head}\n\n{text}\n\n");
            }
        };
        section(&mut body, what, &self.what);
        section(&mut body, steps, &self.steps);
        section(&mut body, "Chi segnala", &self.name);
        if self.kind() == Kind::Problem {
            section(&mut body, "Scheda", info.module);
        }
        section(&mut body, "Versione di WinFiles", &info.version_line());
        section(&mut body, "Windows", &info.windows);
        body.push_str("_Inviata da WinFiles (Aiuto)._\n");
        body
    }
}

/// L'indirizzo del modulo di GitHub nel browser, con tutti i campi già riempiti.
pub fn form_url(draft: &Draft, info: &Info) -> String {
    let kind = draft.kind();
    let mut url = format!("{NEW_ISSUE}?template={}", kind.template());
    let mut param = |name: &str, value: &str| {
        if !value.trim().is_empty() {
            let _ = write!(url, "&{name}={}", encode(value.trim()));
        }
    };
    param("title", &draft.issue_title());
    match kind {
        Kind::Problem => {
            param("cosa", &draft.what);
            param("passi", &draft.steps);
            param("scheda", info.module);
        }
        Kind::Feature => {
            param("idea", &draft.what);
            param("perche", &draft.steps);
        }
    }
    param("versione", &info.version_line());
    param("windows", &info.windows);
    url
}

/// Una segnalazione creata su GitHub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sent {
    pub number: u64,
    pub url: String,
}

/// A che punto è la finestra.
enum Phase {
    Writing,
    Sending(Receiver<Result<Sent, String>>),
    Sent(Sent),
    Failed(String),
}

/// La finestra della segnalazione.
pub struct ReportDialog {
    pub draft: Draft,
    info: Info,
    phase: Phase,
}

/// Cosa chiede la finestra a chi la apre.
pub enum Answer {
    /// Chiusa: il nome di chi segnala va ricordato per la prossima volta.
    Close { name: String },
    /// Un messaggio per la barra di stato.
    Status(String),
}

impl ReportDialog {
    pub fn new(kind: Kind, module: &'static str, name: String) -> Self {
        Self {
            draft: Draft {
                kind: Some(kind),
                name,
                ..Draft::default()
            },
            info: Info::current(module),
            phase: Phase::Writing,
        }
    }

    /// Mostra la finestra; ridisegna da sola mentre l'invio è in corso.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Answer> {
        self.receive(ctx);
        let t = Tokens::get(ctx);
        let mut answer = None;
        let sending = matches!(self.phase, Phase::Sending(_));
        let modal = egui::Modal::new(egui::Id::new("segnalazione")).show(ctx, |ui| {
            ui.set_width(480.0);
            let kind = self.draft.kind();
            ui.label(
                egui::RichText::new(kind.title())
                    .font(semibold(17.0))
                    .color(t.text_strong),
            );
            ui.add_space(12.0);
            if let Phase::Sent(sent) = &self.phase {
                answer = Self::sent_view(ui, &t, sent, &self.draft);
                return;
            }
            ui.add_enabled_ui(!sending, |ui| self.form(ui, &t));
            ui.add_space(8.0);
            dim_label(
                ui,
                &format!(
                    "Si aggiungono da sole: {} · {}{}",
                    self.info.version_line(),
                    self.info.windows,
                    if kind == Kind::Problem {
                        format!(" · scheda {}", self.info.module)
                    } else {
                        String::new()
                    }
                ),
            );
            if let Phase::Failed(e) = &self.phase {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(e).size(13.0).color(t.error));
            }
            ui.add_space(16.0);
            answer = self.buttons(ui, &t, sending);
        });
        if modal.should_close() && answer.is_none() && !sending {
            answer = Some(Answer::Close {
                name: self.draft.name.clone(),
            });
        }
        answer
    }

    fn form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let kind = self.draft.kind();
        ui.horizontal(|ui| {
            for (k, label) in [(Kind::Problem, "Problema"), (Kind::Feature, "Nuova funzione")] {
                if crate::fluent::radio(ui, kind == k, label).clicked() {
                    self.draft.kind = Some(k);
                }
                ui.add_space(12.0);
            }
        });
        ui.add_space(10.0);
        let (what, what_hint, steps, steps_hint) = match kind {
            Kind::Problem => (
                "Cosa è successo",
                "Cosa hai visto e cosa ti aspettavi invece",
                "Come rifarlo (se lo sai)",
                "1. Apro un PDF in Modifica PDF\n2. Clicco su …",
            ),
            Kind::Feature => (
                "Cosa vorresti",
                "Cosa dovrebbe fare WinFiles",
                "Per quale lavoro (facoltativo)",
                "Quando ti servirebbe e come lo fai oggi",
            ),
        };
        let width = ui.available_width();
        let field = |ui: &mut egui::Ui, label: &str, text: &mut String, hint: &str, rows: usize| {
            ui.label(egui::RichText::new(label).size(13.0).color(t.text));
            let edit = if rows > 1 {
                egui::TextEdit::multiline(text).desired_rows(rows)
            } else {
                egui::TextEdit::singleline(text)
            };
            crate::fluent::text_field(ui, edit.hint_text(hint).desired_width(width));
            ui.add_space(8.0);
        };
        field(ui, "Titolo", &mut self.draft.title, "In poche parole", 1);
        field(ui, what, &mut self.draft.what, what_hint, 5);
        field(ui, steps, &mut self.draft.steps, steps_hint, 3);
        field(
            ui,
            "Il tuo nome",
            &mut self.draft.name,
            "Per sapere a chi chiedere",
            1,
        );
        dim_label(
            ui,
            "La segnalazione è pubblica su GitHub: non scrivere dati dei clienti.",
        );
    }

    fn buttons(&mut self, ui: &mut egui::Ui, t: &Tokens, sending: bool) -> Option<Answer> {
        let mut answer = None;
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 36.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                let can_send = token().is_some();
                if sending {
                    ui.add(egui::Spinner::new().size(16.0).color(t.accent));
                    dim_label(ui, "Invio…");
                } else if can_send {
                    let send = egui::Button::new(
                        egui::RichText::new("Invia")
                            .font(semibold(14.0))
                            .color(t.on_primary),
                    )
                    .fill(t.primary)
                    .min_size(egui::vec2(120.0, 34.0));
                    if ui.add_enabled(self.draft.ready(), send).clicked() {
                        self.send();
                    }
                }
                if !sending
                    && ui
                        .add(egui::Button::new("Annulla").min_size(egui::vec2(90.0, 34.0)))
                        .clicked()
                {
                    answer = Some(Answer::Close {
                        name: self.draft.name.clone(),
                    });
                }
                // Senza chiave, o se l'invio non va: il modulo di GitHub nel browser, o il testo
                // da mandare in altro modo.
                if !sending && (!can_send || matches!(self.phase, Phase::Failed(_))) {
                    if ui
                        .add_enabled(
                            self.draft.ready(),
                            egui::Button::new("Apri su GitHub").min_size(egui::vec2(0.0, 34.0)),
                        )
                        .clicked()
                    {
                        let url = form_url(&self.draft, &self.info);
                        answer = Some(match crate::shell::open_url(&url) {
                            Ok(()) => Answer::Status("Apro GitHub nel browser…".into()),
                            Err(e) => Answer::Status(format!("Non riesco ad aprire il browser: {e}")),
                        });
                    }
                    if ui
                        .add_enabled(
                            self.draft.ready(),
                            egui::Button::new("Copia il testo").min_size(egui::vec2(0.0, 34.0)),
                        )
                        .clicked()
                    {
                        let text = format!(
                            "{}\n\n{}",
                            self.draft.issue_title(),
                            self.draft.issue_body(&self.info)
                        );
                        ui.ctx().copy_text(text);
                        answer = Some(Answer::Status("Testo della segnalazione copiato".into()));
                    }
                }
            },
        );
        answer
    }

    fn sent_view(ui: &mut egui::Ui, t: &Tokens, sent: &Sent, draft: &Draft) -> Option<Answer> {
        ui.label(
            egui::RichText::new(format!("Grazie! Segnalazione n. {} inviata.", sent.number))
                .size(14.0)
                .color(t.text_strong),
        );
        ui.add_space(6.0);
        dim_label(
            ui,
            "La puoi seguire, anche senza account, alla sua pagina su GitHub.",
        );
        ui.add_space(16.0);
        let mut answer = None;
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 36.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                let close = egui::Button::new(
                    egui::RichText::new("Chiudi")
                        .font(semibold(14.0))
                        .color(t.on_primary),
                )
                .fill(t.primary)
                .min_size(egui::vec2(120.0, 34.0));
                if ui.add(close).clicked() {
                    answer = Some(Answer::Close {
                        name: draft.name.clone(),
                    });
                }
                if ui
                    .add(egui::Button::new("Apri la pagina").min_size(egui::vec2(0.0, 34.0)))
                    .clicked()
                {
                    let _ = crate::shell::open_url(&sent.url);
                }
            },
        );
        answer
    }

    fn send(&mut self) {
        let Some(token) = token() else {
            return;
        };
        let payload = serde_json::json!({
            "title": self.draft.issue_title(),
            "body": self.draft.issue_body(&self.info),
        })
        .to_string();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(post_issue(token, &payload));
        });
        self.phase = Phase::Sending(rx);
    }

    fn receive(&mut self, ctx: &egui::Context) {
        let Phase::Sending(rx) = &self.phase else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(sent)) => self.phase = Phase::Sent(sent),
            Ok(Err(e)) => self.phase = Phase::Failed(e),
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.phase = Phase::Failed("Invio interrotto".into());
            }
        }
    }
}

/// Crea la issue con l'API di GitHub. La chiave va a `curl` dallo standard input (`-K -`), non
/// sulla riga di comando, dove altri programmi la vedrebbero; il testo da un file temporaneo.
fn post_issue(token: &str, payload: &str) -> Result<Sent, String> {
    use std::io::Write as _;
    let failed = "Non riesco a inviare la segnalazione";
    let file = std::env::temp_dir().join(format!("winfiles-segnalazione-{}.json", std::process::id()));
    std::fs::write(&file, payload).map_err(|e| format!("{failed} ({e})"))?;
    let child = crate::update::curl()
        .args(["-sS", "--max-time", "30", "-X", "POST", "-K", "-"])
        .args(["-H", "Accept: application/vnd.github+json"])
        .args(["-H", "Content-Type: application/json"])
        .args(["-A", concat!("WinFiles/", env!("CARGO_PKG_VERSION"))])
        .args(["-w", "\n%{http_code}"])
        .arg("--data-binary")
        .arg(format!("@{}", file.display()))
        // Per le prove: un server finto al posto di GitHub.
        .arg(std::env::var("WINFILES_SEGNALAZIONI_API").unwrap_or_else(|_| API.to_string()))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let result = (|| {
        let mut child = child.map_err(|e| format!("{failed} ({e})"))?;
        if let Some(mut stdin) = child.stdin.take() {
            // Fra virgolette nel file di configurazione di curl: della chiave si tengono solo
            // lettere, cifre e trattini bassi, così non può rompere la riga.
            let clean: String = token
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            let _ = writeln!(stdin, "header = \"Authorization: Bearer {clean}\"");
        }
        let out = child.wait_with_output().map_err(|e| format!("{failed} ({e})"))?;
        if out.stdout.len() > MAX_ANSWER {
            return Err("Risposta di GitHub troppo lunga".to_string());
        }
        parse_answer(&String::from_utf8_lossy(&out.stdout))
    })();
    let _ = std::fs::remove_file(&file);
    result
}

/// La risposta di `curl -w "\n%{http_code}"`: il JSON della issue, poi il codice (201 = creata).
fn parse_answer(output: &str) -> Result<Sent, String> {
    let offline = "Non riesco a raggiungere GitHub: controlla la connessione a Internet";
    let (json, code) = output.trim_end().rsplit_once('\n').unwrap_or(("", output.trim()));
    match code.trim() {
        "201" => {}
        "" | "000" => return Err(offline.into()),
        "401" | "403" => {
            return Err("GitHub non accetta più la chiave delle segnalazioni (scaduta?): \
                        usa «Copia il testo» e avvisa chi cura WinFiles"
                .into());
        }
        other => {
            return Err(format!(
                "GitHub ha risposto con l'errore {other}: riprova più tardi"
            ));
        }
    }
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|_| "Risposta di GitHub non leggibile".to_string())?;
    let sent = (|| {
        Some(Sent {
            number: v.get("number")?.as_u64()?,
            url: v.get("html_url")?.as_str()?.to_string(),
        })
    })();
    sent.ok_or_else(|| "Risposta di GitHub non leggibile".into())
}

/// Codifica per un parametro d'indirizzo: restano solo lettere, cifre e `-_.~` (UTF-8 per il resto).
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// «Windows 11 24H2 (build 26100)», letto dal registro. Se non si riesce, «Windows».
fn windows_version() -> String {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let read = |name: &str| -> Option<String> {
            let out = std::process::Command::new("reg")
                .args([
                    "query",
                    r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
                    "/v",
                    name,
                ])
                // CREATE_NO_WINDOW: niente console che lampeggia.
                .creation_flags(0x0800_0000)
                .output()
                .ok()?;
            reg_value(&String::from_utf8_lossy(&out.stdout), name)
        };
        describe_windows(read("CurrentBuild").as_deref(), read("DisplayVersion").as_deref())
    }
    #[cfg(not(windows))]
    {
        format!("{} (non Windows)", std::env::consts::OS)
    }
}

/// Il valore di `name` nell'uscita di `reg query`: «    CurrentBuild    REG_SZ    26100».
#[cfg_attr(not(windows), allow(dead_code))]
fn reg_value(output: &str, name: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        (parts.next()? == name).then_some(())?;
        parts.next()?; // il tipo, REG_SZ
        let value = parts.collect::<Vec<_>>().join(" ");
        (!value.is_empty()).then_some(value)
    })
}

/// Il registro di Windows 11 dice ancora «Windows 10» nel nome del prodotto: conta il build
/// (22000 e oltre è Windows 11).
#[cfg_attr(not(windows), allow(dead_code))]
fn describe_windows(build: Option<&str>, display: Option<&str>) -> String {
    let Some(build) = build else {
        return "Windows".into();
    };
    let name = match build.parse::<u32>() {
        Ok(n) if n >= 22000 => "Windows 11",
        Ok(_) => "Windows 10",
        Err(_) => "Windows",
    };
    match display {
        Some(d) => format!("{name} {d} (build {build})"),
        None => format!("{name} (build {build})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> Info {
        Info {
            version: "0.2.24".into(),
            windows: "Windows 11 24H2 (build 26100)".into(),
            installed: true,
            module: "Modifica PDF",
        }
    }

    fn draft(kind: Kind) -> Draft {
        Draft {
            kind: Some(kind),
            title: "  La  taglierina   non taglia ".into(),
            what: "Clicco e non succede nulla".into(),
            steps: String::new(),
            name: "Mario".into(),
        }
    }

    #[test]
    fn body_has_the_sections_and_the_info() {
        let body = draft(Kind::Problem).issue_body(&info());
        assert_eq!(
            body,
            "### Cosa è successo\n\nClicco e non succede nulla\n\n\
             ### Chi segnala\n\nMario\n\n\
             ### Scheda\n\nModifica PDF\n\n\
             ### Versione di WinFiles\n\nWinFiles 0.2.24 (installata)\n\n\
             ### Windows\n\nWindows 11 24H2 (build 26100)\n\n\
             _Inviata da WinFiles (Aiuto)._\n"
        );
        assert_eq!(
            draft(Kind::Problem).issue_title(),
            "Problema: La taglierina non taglia"
        );
    }

    #[test]
    fn feature_body_has_no_tab() {
        let body = draft(Kind::Feature).issue_body(&info());
        assert!(body.starts_with("### Cosa vorresti"));
        assert!(!body.contains("Scheda"));
        assert_eq!(
            draft(Kind::Feature).issue_title(),
            "Funzione: La taglierina non taglia"
        );
    }

    #[test]
    fn ready_needs_title_and_text() {
        let mut d = draft(Kind::Problem);
        assert!(d.ready());
        d.what = "   ".into();
        assert!(!d.ready());
        d.what = "x".repeat(MAX_TEXT + 1);
        assert!(!d.ready());
    }

    #[test]
    fn form_url_fills_the_github_form() {
        let url = form_url(&draft(Kind::Problem), &info());
        assert_eq!(
            url,
            "https://github.com/Mobbys/WinFiles/issues/new?template=problema.yml\
             &title=Problema%3A%20La%20taglierina%20non%20taglia\
             &cosa=Clicco%20e%20non%20succede%20nulla\
             &scheda=Modifica%20PDF\
             &versione=WinFiles%200.2.24%20%28installata%29\
             &windows=Windows%2011%2024H2%20%28build%2026100%29"
        );
        let mut i = info();
        i.installed = false;
        let url = form_url(&draft(Kind::Feature), &i);
        assert!(url.contains("template=funzione.yml"));
        assert!(url.contains("&idea=Clicco"));
        assert!(url.contains("portatile"));
        assert!(!url.contains("scheda="));
    }

    #[test]
    fn reads_the_created_issue() {
        let out = "{\"number\":12,\"html_url\":\"https://github.com/Mobbys/WinFiles/issues/12\"}\n201";
        assert_eq!(
            parse_answer(out),
            Ok(Sent {
                number: 12,
                url: "https://github.com/Mobbys/WinFiles/issues/12".into()
            })
        );
        let expired = parse_answer("{\"message\":\"Bad credentials\"}\n401").unwrap_err();
        assert!(expired.contains("chiave"));
        assert!(parse_answer("\n000").unwrap_err().contains("connessione"));
        assert!(parse_answer("").unwrap_err().contains("connessione"));
        assert!(parse_answer("{}\n500").unwrap_err().contains("500"));
    }

    #[test]
    fn encode_keeps_only_safe_bytes() {
        assert_eq!(encode("a&b=c?d #è"), "a%26b%3Dc%3Fd%20%23%C3%A8");
    }

    #[test]
    fn reads_reg_query_output() {
        let out = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\r\n    \
                   DisplayVersion    REG_SZ    24H2\r\n\r\n";
        assert_eq!(reg_value(out, "DisplayVersion").as_deref(), Some("24H2"));
        assert_eq!(reg_value(out, "CurrentBuild"), None);
    }

    #[test]
    fn windows_11_from_build() {
        assert_eq!(
            describe_windows(Some("26100"), Some("24H2")),
            "Windows 11 24H2 (build 26100)"
        );
        assert_eq!(
            describe_windows(Some("19045"), Some("22H2")),
            "Windows 10 22H2 (build 19045)"
        );
        assert_eq!(describe_windows(None, None), "Windows");
    }
}
