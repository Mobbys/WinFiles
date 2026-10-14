//! WinFiles: l'applicazione desktop. Per ora (Fase 0) è uno scheletro che collega modello,
//! impaginazione e anteprima: si trascinano file o cartelle nella finestra e si vede subito
//! l'impaginazione su A4 o A3. Le miniature vere arrivano con i decodificatori della Fase 2.

// In release su Windows niente finestra di console dietro l'app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod preview;
mod scan;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(concat!("WinFiles ", env!("CARGO_PKG_VERSION")))
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 560.0]),
        #[cfg(feature = "opengl")]
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    // File e cartelle passati sulla riga di comando (anche "Apri con" di Windows) si caricano subito.
    let paths: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(Into::into).collect();
    eframe::run_native(
        "WinFiles",
        options,
        Box::new(|_cc| Ok(Box::new(app::WinFilesApp::with_paths(paths)))),
    )
}
