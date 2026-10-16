//! WinFiles: l'applicazione desktop. Per ora (Fase 0) è uno scheletro che collega modello,
//! impaginazione e anteprima: si trascinano file o cartelle nella finestra e si vede subito
//! l'impaginazione su A4 o A3. Le miniature vere arrivano con i decodificatori della Fase 2.

// In release su Windows niente finestra di console dietro l'app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod calibrate;
mod canvas;
mod captions;
mod cutter;
mod dock;
mod document;
mod export;
mod filter;
mod fluent;
mod icons;
mod images;
mod legacy;
mod measure;
mod paint;
mod panels;
mod pdf;
mod prefs;
mod presets;
mod printing;
mod recent;
mod reorder;
mod scan;
mod shell;
mod sketch;
mod summary;
mod table;
mod theme;
mod thumbs;
mod unsupported;
mod update;

fn main() -> eframe::Result {
    // Le ultime scelte: pannelli, tema, impaginazione, stampante e misura della finestra.
    let prefs = prefs::load();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(concat!("WinFiles ", env!("CARGO_PKG_VERSION")))
        .with_inner_size(prefs.window.unwrap_or([1280.0, 800.0]))
        .with_min_inner_size(prefs::MIN_WINDOW)
        .with_maximized(prefs.maximized);
    // L'icona della finestra e della barra delle applicazioni (assets/icon, genera-icona.py).
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon/icona-256.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        #[cfg(feature = "opengl")]
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    // File e cartelle passati sulla riga di comando (anche "Apri con" di Windows) si caricano subito.
    // `--pdf uscita.pdf` esporta i fogli in quel file e chiude: per gli script e per le prove.
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    // `--calibrazione A4 calibrazione.pdf` scrive la pagina di calibrazione in quel file e chiude.
    // `--modifica-pdf file.pdf` apre il modulo PDF su quel file; `--modifica-immagini a.jpg b.jpg`
    // apre il modulo immagini su quei file (tutto quello che segue).
    let mut pdf = None;
    let mut calibration = None;
    let mut edit_pdf = None;
    let mut edit_images: Vec<std::path::PathBuf> = Vec::new();
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--pdf" {
            pdf = args.next().map(Into::into);
        } else if arg == "--modifica-pdf" {
            edit_pdf = args.next().map(Into::into);
        } else if arg == "--modifica-immagini" {
            edit_images.extend(args.by_ref().map(Into::into));
        } else if arg == "--calibrazione" {
            let paper = match args.next() {
                Some(p) if p.eq_ignore_ascii_case("a3") => winfiles_geom::Paper::A3,
                _ => winfiles_geom::Paper::A4,
            };
            calibration = args.next().map(|path| (paper, path.into()));
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
            Ok(Box::new(app::WinFilesApp::new(
                &cc.egui_ctx,
                paths,
                pdf,
                calibration,
                edit_pdf,
                edit_images,
                prefs,
            )))
        }),
    )
}
