//! WinFiles: l'applicazione desktop. Per ora (Fase 0) è uno scheletro che collega modello,
//! impaginazione e anteprima: si trascinano file o cartelle nella finestra e si vede subito
//! l'impaginazione su A4 o A3. Le miniature vere arrivano con i decodificatori della Fase 2.

// In release su Windows niente finestra di console dietro l'app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas;
mod captions;
mod dock;
mod document;
mod export;
mod icons;
mod measure;
mod paint;
mod panels;
mod printing;
mod recent;
mod scan;
mod shell;
mod sketch;
mod summary;
mod theme;
mod thumbs;

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
    // `--pdf uscita.pdf` esporta i fogli in quel file e chiude: per gli script e per le prove.
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    let mut pdf = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--pdf" {
            pdf = args.next().map(Into::into);
        } else {
            paths.push(arg.into());
        }
    }
    eframe::run_native(
        "WinFiles",
        options,
        Box::new(|cc| {
            theme::install_fonts(&cc.egui_ctx);
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(app::WinFilesApp::new(&cc.egui_ctx, paths, pdf)))
        }),
    )
}
