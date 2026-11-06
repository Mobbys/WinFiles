//! Programma predefinito (ADR 0032): per quali tipi di file il doppio clic in Esplora file apre
//! WinFiles nella scheda Visualizza.
//!
//! L'installer registra WinFiles solo come *candidato* (un ProgID `WinFiles.<estensione>` per tipo,
//! `OpenWithProgids` e `RegisteredApplications`): all'installazione nessun tipo cambia programma.
//! Windows non lascia scegliere il predefinito a un programma (la scelta in `UserChoice` è
//! protetta): le caselle mostrano la scelta di adesso e aprono la pagina "App predefinite" di
//! WinFiles nelle Impostazioni, dove si conferma con un clic per tipo.

use std::path::Path;
#[cfg(windows)]
use std::process::Command;
use std::sync::mpsc::{Receiver, channel};

use crate::theme::Tokens;

/// I tipi che WinFiles sa mostrare, con le estensioni.
pub const TYPES: [(&str, &[&str]); 12] = [
    ("PDF", &["pdf"]),
    ("Illustrator", &["ai"]),
    ("EPS", &["eps"]),
    ("JPEG", &["jpg", "jpeg"]),
    ("PNG", &["png"]),
    ("TIFF", &["tif", "tiff"]),
    ("WebP", &["webp"]),
    ("BMP", &["bmp"]),
    ("HEIC (foto degli iPhone)", &["heic", "heif"]),
    ("Photoshop", &["psd", "psb"]),
    ("DXF", &["dxf"]),
    ("DWG", &["dwg"]),
];

/// Il ProgID di WinFiles per un'estensione (lo stesso dell'installer).
pub fn prog_id(ext: &str) -> String {
    format!("WinFiles.{ext}")
}

/// Il programma scelto in Esplora file per un'estensione, dall'uscita di `reg query` della
/// chiave `UserChoice`.
pub fn parse_choice(out: &str) -> Option<String> {
    out.lines().find_map(|l| {
        let mut parts = l.split_whitespace();
        (parts.next()? == "ProgId" && parts.next()? == "REG_SZ").then(|| parts.collect::<Vec<_>>().join(" "))
    })
}

/// Lo stato di un tipo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeState {
    pub label: &'static str,
    pub exts: &'static [&'static str],
    /// WinFiles è il predefinito per tutte le estensioni del tipo.
    pub winfiles: bool,
    /// Il programma scelto ora (ProgID), se non è WinFiles.
    pub other: Option<String>,
}

#[cfg(windows)]
fn reg(args: &[&str]) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = Command::new("reg")
        .args(args)
        // CREATE_NO_WINDOW: niente console che lampeggia.
        .creation_flags(0x0800_0000)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(not(windows))]
fn reg(_args: &[&str]) -> Option<String> {
    None
}

/// WinFiles è installato (l'installer ha scritto i suoi ProgID). La versione portatile non
/// registra niente.
pub fn registered() -> bool {
    reg(&["query", &format!(r"HKCU\Software\Classes\{}", prog_id("pdf"))]).is_some()
}

fn user_choice(ext: &str) -> Option<String> {
    let key = format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.{ext}\UserChoice");
    reg(&["query", &key, "/v", "ProgId"]).and_then(|o| parse_choice(&o))
}

/// Lo stato di tutti i tipi (legge il registro: va fatto fuori dall'interfaccia).
pub fn states() -> Vec<TypeState> {
    TYPES
        .iter()
        .map(|(label, exts)| {
            let choices: Vec<Option<String>> = exts.iter().map(|e| user_choice(e)).collect();
            let winfiles = exts
                .iter()
                .zip(&choices)
                .all(|(e, c)| c.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&prog_id(e))));
            let other = choices
                .into_iter()
                .flatten()
                .find(|c| !c.starts_with("WinFiles."));
            TypeState {
                label,
                exts,
                winfiles,
                other,
            }
        })
        .collect()
}

/// Apre la pagina delle Impostazioni dove si sceglie WinFiles come predefinito.
pub fn open_settings() -> std::io::Result<()> {
    crate::shell::open_url("ms-settings:defaultapps?registeredAppUser=WinFiles")
}

/// "Apri con…": la finestra di Windows per scegliere con quale programma aprire il file (c'è
/// anche quello che lo apriva prima di WinFiles).
pub fn open_with_previous(path: &Path) -> Result<String, String> {
    #[cfg(windows)]
    {
        Command::new("rundll32")
            .arg("shell32.dll,OpenAs_RunDLL")
            .arg(path)
            .spawn()
            .map(|_| "la scelta di Windows".to_string())
            .map_err(|e| format!("Non riesco ad aprire la scelta del programma: {e}"))
    }
    #[cfg(not(windows))]
    {
        crate::shell::open(path)
            .map(|()| "il programma predefinito".to_string())
            .map_err(|e| format!("Non riesco ad aprire il file: {e}"))
    }
}

/// La finestra "Programma predefinito…".
pub struct DefaultsDialog {
    states: Option<Vec<TypeState>>,
    rx: Option<Receiver<Vec<TypeState>>>,
    installed: Option<bool>,
    /// Quando rileggere (le scelte cambiano nelle Impostazioni, fuori da WinFiles).
    next_check: f64,
    pub status: String,
}

impl DefaultsDialog {
    pub fn new() -> Self {
        Self {
            states: None,
            rx: None,
            installed: None,
            next_check: 0.0,
            status: String::new(),
        }
    }

    fn refresh(&mut self, ctx: &egui::Context) {
        if self.rx.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let wake = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("predefiniti".into())
            .spawn(move || {
                let _ = tx.send(states());
                wake.request_repaint();
            });
        if spawned.is_ok() {
            self.rx = Some(rx);
        }
        if self.installed.is_none() {
            self.installed = Some(registered());
        }
    }

    /// Mostra la finestra; `false` quando va chiusa.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let now = ctx.input(|i| i.time);
        if let Some(rx) = &self.rx
            && let Ok(s) = rx.try_recv()
        {
            self.states = Some(s);
            self.rx = None;
            self.next_check = now + 2.0;
        }
        if self.rx.is_none() && now >= self.next_check {
            self.refresh(ctx);
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        let t = Tokens::get(ctx);
        let mut open = true;
        let mut close = false;
        egui::Window::new("Programma predefinito")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(
                        "Scegli per quali file il doppio clic in Esplora file apre WinFiles, nella scheda Visualizza.",
                    )
                    .color(t.text),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "Windows vuole la conferma da te: la casella apre la pagina App predefinite di WinFiles; \
                         lì clicca il tipo e scegli WinFiles (o il programma di prima per toglierlo).",
                    )
                    .size(12.5)
                    .color(t.text_dim),
                );
                ui.add_space(8.0);
                if self.installed == Some(false) {
                    ui.label(
                        egui::RichText::new(
                            "Questa è la versione portatile: per il doppio clic serve WinFiles installato (MSI).",
                        )
                        .color(t.warning),
                    );
                    ui.add_space(6.0);
                }
                match &self.states {
                    None => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().color(t.accent));
                            ui.label("Leggo le scelte di Windows…");
                        });
                    }
                    Some(list) => {
                        let mut ask = false;
                        egui::Grid::new("predefiniti").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                            for s in list {
                                let mut on = s.winfiles;
                                let exts: Vec<String> = s.exts.iter().map(|e| format!(".{e}")).collect();
                                let text = format!("{}  ({})", s.label, exts.join(" "));
                                if ui.add(crate::fluent::checkbox(&mut on, text)).changed() {
                                    ask = true;
                                }
                                let now = if s.winfiles {
                                    "WinFiles".to_string()
                                } else {
                                    s.other.clone().unwrap_or_else(|| "scelta di Windows".into())
                                };
                                ui.label(egui::RichText::new(now).size(12.0).color(t.text_dim));
                                ui.end_row();
                            }
                        });
                        if ask {
                            match open_settings() {
                                Ok(()) => self.status = "Conferma nella pagina delle Impostazioni".into(),
                                Err(e) => self.status = format!("Non riesco ad aprire le Impostazioni: {e}"),
                            }
                        }
                    }
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Apri App predefinite").clicked()
                        && let Err(e) = open_settings()
                    {
                        self.status = format!("Non riesco ad aprire le Impostazioni: {e}");
                    }
                    if ui.button("Chiudi").clicked() {
                        close = true;
                    }
                });
                if !self.status.is_empty() {
                    ui.label(egui::RichText::new(&self.status).size(12.5).color(t.text_dim));
                }
            });
        open && !close
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_scelta_si_legge_dall_uscita_di_reg() {
        let out =
            "\r\nHKEY_CURRENT_USER\\Software\\...\\UserChoice\r\n    ProgId    REG_SZ    WinFiles.pdf\r\n";
        assert_eq!(parse_choice(out).as_deref(), Some("WinFiles.pdf"));
        let out = "    ProgId    REG_SZ    Acrobat.Document.DC\n";
        assert_eq!(parse_choice(out).as_deref(), Some("Acrobat.Document.DC"));
        assert_eq!(parse_choice("niente"), None);
    }

    #[test]
    fn ogni_estensione_ha_il_suo_progid() {
        assert_eq!(prog_id("pdf"), "WinFiles.pdf");
        let all: Vec<&str> = TYPES.iter().flat_map(|(_, e)| e.iter().copied()).collect();
        assert_eq!(all.len(), 16);
    }

    #[test]
    fn l_installer_registra_ogni_tipo_senza_renderlo_predefinito() {
        let wxs = include_str!("../../../installer/winfiles.wxs");
        for (_, exts) in TYPES {
            for e in exts {
                let id = prog_id(e);
                assert!(
                    wxs.contains(&format!(r"Software\Classes\{id}\shell\open\command")),
                    "{id}"
                );
                assert!(
                    wxs.contains(&format!(r"Software\Classes\.{e}\OpenWithProgids")),
                    ".{e}"
                );
            }
        }
        // Mai la voce predefinita di un'estensione: quella la sceglie chi usa WinFiles.
        for (_, exts) in TYPES {
            for e in exts {
                assert!(
                    !wxs.contains(&format!(r#"Key="Software\Classes\.{e}" Type="string""#)),
                    ".{e}"
                );
            }
        }
    }
}
