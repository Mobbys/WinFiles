//! Comandi di sviluppo del workspace.
//!
//! - `cargo xtask ci`: tutti i controlli da far passare prima di ogni commit (formattazione,
//!   clippy senza avvisi, test, livelli, licenze).
//! - `cargo xtask layers`: verifica che i crate sotto l'interfaccia non dipendano da egui.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Dipendenze consentite solo all'interfaccia (livello L6) e alle app.
const UI_ONLY: [&str; 5] = ["egui", "eframe", "winit", "rfd", "egui_extras"];

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    let result = match task.as_str() {
        "ci" => ci(),
        "layers" => layers(),
        _ => {
            eprintln!("uso: cargo xtask <ci|layers>");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

fn run(args: &[&str]) -> Result<(), String> {
    eprintln!("==> cargo {}", args.join(" "));
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(args)
        .current_dir(root())
        .status()
        .map_err(|e| format!("impossibile avviare cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` non è passato", args.join(" ")))
    }
}

fn ci() -> Result<(), String> {
    run(&["fmt", "--all", "--check"])?;
    run(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--",
        "-D",
        "warnings",
    ])?;
    run(&["test", "--workspace"])?;
    layers()?;
    deny()?;
    eprintln!("==> tutto verde");
    Ok(())
}

/// Licenze e vulnerabilità con cargo-deny, se installato (in CI lo è sempre).
fn deny() -> Result<(), String> {
    let installed = Command::new("cargo-deny")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if installed {
        run(&["deny", "check"])
    } else {
        eprintln!(
            "==> cargo-deny non installato: controllo licenze saltato (cargo install --locked cargo-deny)"
        );
        Ok(())
    }
}

/// I crate in `crates/` (tutto ciò che sta sotto l'interfaccia) non possono usare egui e simili:
/// così il motore resta testabile da solo e l'interfaccia si può sostituire.
fn layers() -> Result<(), String> {
    let crates = root().join("crates");
    let entries = std::fs::read_dir(&crates).map_err(|e| format!("{}: {e}", crates.display()))?;
    let mut problems = Vec::new();
    for entry in entries.flatten() {
        let manifest = entry.path().join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        // Il crate dell'interfaccia, quando ci sarà, si chiamerà `ui` ed è l'unico esentato.
        if name == "ui" {
            continue;
        }
        for line in text.lines() {
            let dep = line.split(['=', '.']).next().unwrap_or("").trim();
            if UI_ONLY.contains(&dep) {
                problems.push(format!("crates/{name} dipende da `{dep}`"));
            }
        }
    }
    if problems.is_empty() {
        eprintln!("==> livelli in ordine");
        Ok(())
    } else {
        Err(format!("livelli violati:\n  {}", problems.join("\n  ")))
    }
}
