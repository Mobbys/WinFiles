//! Aprire un file per la scheda Visualizza e disegnarne le pagine, in thread a parte: la finestra
//! non aspetta mai un file grande.
//!
//! Ogni file aperto diventa un [`Content`]: un PDF in memoria (anche gli AI, gli EPS di
//! Illustrator e i disegni DXF e DWG tradotti in PDF) oppure un'immagine già decodificata. Il
//! [`Renderer`] di ogni file disegna la pagina intera a un lato fisso e, ingrandendo, solo il pezzo
//! che si vede alla risoluzione dello schermo, così il dettaglio resta nitido.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use winfiles_geom::Size;
use winfiles_model::FileKind;
use winfiles_pdf_edit::{Details, PageScan, PdfFile, Spot};
use winfiles_thumbs::{PdfPages, Thumb};

/// Lato massimo di un'immagine disegnata (pagina intera o pezzo).
pub const MAX_SIDE: u32 = winfiles_thumbs::MAX_SIDE;

/// Che file è, per la scheda.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewKind {
    Pdf,
    Ai,
    Eps,
    Image,
    Cad,
    /// Disegno SVG, mostrato come PDF vettoriale.
    Svg,
    /// Word, Excel, PowerPoint, InDesign: convertiti in PDF (ADR 0035).
    Office(super::office::Office),
    Unsupported,
}

impl ViewKind {
    pub fn of(path: &Path) -> Self {
        if winfiles_cad::is_cad_path(path) {
            return ViewKind::Cad;
        }
        if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"))
        {
            return ViewKind::Svg;
        }
        if let Some(o) = super::office::Office::of(path) {
            return ViewKind::Office(o);
        }
        match FileKind::from_path(path) {
            FileKind::Pdf => ViewKind::Pdf,
            FileKind::Ai => ViewKind::Ai,
            FileKind::Eps => ViewKind::Eps,
            FileKind::Image => ViewKind::Image,
            FileKind::Unsupported => ViewKind::Unsupported,
        }
    }

    /// Il nome per il file: i PLT sono disegni del plotter, non del CAD.
    pub fn label_for(self, path: &Path) -> &'static str {
        let plotter = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| ["plt", "hpgl", "hpg"].iter().any(|x| e.eq_ignore_ascii_case(x)));
        if self == ViewKind::Cad && plotter {
            "File del plotter"
        } else {
            self.label()
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ViewKind::Pdf => "PDF",
            ViewKind::Ai => "Illustrator",
            ViewKind::Eps => "EPS",
            ViewKind::Image => "Immagine",
            ViewKind::Cad => "Disegno CAD",
            ViewKind::Svg => "SVG",
            ViewKind::Office(o) => o.label(),
            ViewKind::Unsupported => "Non supportato",
        }
    }
}

/// Quello che si disegna.
#[derive(Clone)]
pub enum Content {
    Pdf(Arc<Vec<u8>>),
    Image(Arc<image::DynamicImage>, f64),
}

/// I riquadri di una pagina in mm, dall'alto a sinistra del formato visibile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageView {
    pub size: Size,
    pub trim: Option<[f64; 4]>,
    pub bleed: Option<[f64; 4]>,
}

impl PageView {
    pub fn finished(&self) -> Size {
        self.trim.map_or(self.size, |t| Size::new(t[2], t[3]))
    }
}

/// Le informazioni di un'immagine.
#[derive(Clone, Debug)]
pub struct ImageFacts {
    pub width: u32,
    pub height: u32,
    pub dpi: f64,
    pub dpi_from_file: bool,
    pub color: winfiles_image_edit::ColorMode,
    pub bits: u8,
    pub icc: Option<String>,
    pub origin: Option<String>,
}

/// Il disegno CAD con le scelte con cui è diventato PDF.
#[derive(Clone)]
pub struct CadFacts {
    pub drawing: Arc<winfiles_cad::Drawing>,
    /// Millimetri veri per unità del disegno (dalle unità del file o dalla scelta di chi guarda).
    pub unit_mm: f64,
    /// Millimetri della pagina disegnata per unità del disegno.
    pub page_scale: f64,
}

/// Cosa si sa del file, per il pannello Informazioni.
#[derive(Clone)]
pub enum Facts {
    Pdf {
        details: Details,
        spots: Result<Vec<Spot>, String>,
        /// Da dove viene il PDF quando non è il file: "AI in PostScript (Illustrator 3–8)".
        note: Option<String>,
    },
    Image(ImageFacts),
    Cad(CadFacts),
}

/// Un file aperto.
#[derive(Clone)]
pub struct Opened {
    pub content: Content,
    pub pages: Vec<PageView>,
    pub facts: Facts,
    pub bytes: u64,
    pub modified: Option<std::time::SystemTime>,
}

/// Apre il file: PDF, AI, EPS, immagini, DXF e DWG. `cad` sono le scelte per un disegno CAD.
pub fn open(path: &Path, kind: ViewKind, cad: &CadChoice) -> Result<Opened, String> {
    let meta = std::fs::metadata(path).map_err(|e| io_text(&e))?;
    let (bytes, modified) = (meta.len(), meta.modified().ok());
    let mut opened = match kind {
        ViewKind::Pdf | ViewKind::Ai | ViewKind::Eps => open_pdf_like(path, kind)?,
        ViewKind::Image => open_image(path)?,
        ViewKind::Office(o) => {
            let pdf = super::office::to_pdf(path, o)?;
            pdf_opened(pdf, Some(o.note()))?
        }
        ViewKind::Svg => {
            let (pdf, _) = winfiles_cad::svg::open(path).map_err(|e| e.to_string())?;
            pdf_opened(
                pdf,
                Some("SVG: misura dalle unità del file (i pixel a 96 per pollice)".into()),
            )?
        }
        ViewKind::Cad => {
            let drawing = winfiles_cad::open(path).map_err(|e| e.to_string())?;
            cad_content(Arc::new(drawing), cad)?
        }
        ViewKind::Unsupported => return Err("questo tipo di file WinFiles non lo sa mostrare".into()),
    };
    opened.bytes = bytes;
    opened.modified = modified;
    Ok(opened)
}

fn io_text(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "file non trovato".into(),
        std::io::ErrorKind::PermissionDenied => "accesso al file negato".into(),
        _ => format!("impossibile leggere il file: {e}"),
    }
}

fn open_pdf_like(path: &Path, kind: ViewKind) -> Result<Opened, String> {
    // Gli AI in PostScript e gli EPS di Illustrator diventano un PDF; gli altri EPS mostrano la
    // loro anteprima come immagine.
    let (data, note) = match winfiles_thumbs::illustrator_pdf(path).map_err(|e| e.to_string())? {
        Some(pdf) => (
            pdf,
            Some(if kind == ViewKind::Eps {
                "EPS di Illustrator: si vedono tracciati, colori e tinte, non testi e immagini".to_string()
            } else {
                "AI in PostScript (Illustrator 3–8): si vedono tracciati, colori e tinte, non testi e immagini"
                    .to_string()
            }),
        ),
        None if kind == ViewKind::Eps => return open_eps_preview(path),
        None => {
            let len = std::fs::metadata(path).map_err(|e| io_text(&e))?.len();
            if len > 2 * 1024 * 1024 * 1024 {
                return Err(format!("il file è troppo grande ({len} byte)"));
            }
            (std::fs::read(path).map_err(|e| io_text(&e))?, None)
        }
    };
    pdf_opened(data, note)
}

/// Un PDF in memoria con le sue pagine, misure e tinte.
fn pdf_opened(data: Vec<u8>, note: Option<String>) -> Result<Opened, String> {
    let file = PdfFile::from_bytes(data.clone()).map_err(|e| e.to_string())?;
    let details = file.details();
    let spots = file.spots().map_err(|e| e.to_string());
    let pages = details
        .pages
        .iter()
        .map(|p| PageView {
            size: p.size,
            trim: p.trim,
            bleed: p.bleed,
        })
        .collect::<Vec<_>>();
    if pages.is_empty() {
        return Err("il PDF non ha pagine".into());
    }
    Ok(Opened {
        content: Content::Pdf(Arc::new(data)),
        pages,
        facts: Facts::Pdf { details, spots, note },
        bytes: 0,
        modified: None,
    })
}

/// Un EPS che non viene da Illustrator: si vede la sua anteprima, con le misure della
/// BoundingBox.
fn open_eps_preview(path: &Path) -> Result<Opened, String> {
    let thumb = winfiles_thumbs::render(path, FileKind::Eps, 0, MAX_SIDE).map_err(|e| e.to_string())?;
    let probe = winfiles_formats::probe(path, FileKind::Eps).map_err(|e| e.to_string())?;
    let size = probe.pages.first().map_or(Size::new(100.0, 100.0), |p| p.size);
    let img = thumb_image(thumb).ok_or("anteprima non valida")?;
    let dpi = f64::from(img.width().max(1)) / (size.w / 25.4).max(1e-6);
    Ok(Opened {
        content: Content::Image(Arc::new(img), dpi),
        pages: vec![PageView {
            size,
            trim: None,
            bleed: None,
        }],
        facts: Facts::Pdf {
            details: Details::default(),
            spots: Ok(Vec::new()),
            note: Some(
                "EPS con anteprima: si vede l'anteprima salvata nel file, le tinte non si leggono".into(),
            ),
        },
        bytes: 0,
        modified: None,
    })
}

fn open_image(path: &Path) -> Result<Opened, String> {
    let editor = winfiles_image_edit::Editor::open(path).map_err(|e| e.to_string())?;
    let pic = editor.original();
    let probe = winfiles_formats::probe(path, FileKind::Image).ok();
    let from_file = probe
        .as_ref()
        .and_then(|p| p.image)
        .is_none_or(|i| i.dpi_from_file);
    let facts = ImageFacts {
        width: pic.width(),
        height: pic.height(),
        dpi: pic.dpi(),
        dpi_from_file: from_file,
        color: pic.color(),
        bits: pic.bits(),
        icc: pic.icc_name().map(str::to_string),
        origin: pic.origin().map(str::to_string),
    };
    Ok(Opened {
        content: Content::Image(pic.shared(), pic.dpi()),
        pages: vec![PageView {
            size: pic.size_mm(),
            trim: None,
            bleed: None,
        }],
        facts: Facts::Image(facts),
        bytes: 0,
        modified: None,
    })
}

/// Le scelte di chi guarda un disegno CAD.
#[derive(Clone, Debug, PartialEq)]
pub struct CadChoice {
    /// Millimetri per unità quando il file non dichiara le unità.
    pub unit_mm: f64,
    pub hidden: Vec<usize>,
    pub dark: bool,
}

impl Default for CadChoice {
    fn default() -> Self {
        Self {
            unit_mm: 1.0,
            hidden: Vec::new(),
            dark: true,
        }
    }
}

/// Lato lungo della pagina con cui si guarda un disegno: abbastanza per non perdere dettagli
/// ingrandendo, sotto il limite dei PDF.
const CAD_VIEW_MM: f64 = 1000.0;

/// Il disegno come PDF per guardarlo.
pub fn cad_content(drawing: Arc<winfiles_cad::Drawing>, choice: &CadChoice) -> Result<Opened, String> {
    let unit_mm = drawing.units.mm.unwrap_or(choice.unit_mm);
    let (w, h) = drawing.size_mm(unit_mm);
    // A misura vera se ci sta, altrimenti ridotto.
    let page_scale = if w.max(h) <= CAD_VIEW_MM {
        unit_mm
    } else {
        drawing.fit_scale(CAD_VIEW_MM)
    };
    // Un po' di carta attorno al disegno.
    let margin = w.max(h) * page_scale / unit_mm * 0.02;
    let options = winfiles_cad::PdfOptions {
        mm_per_unit: page_scale,
        hidden: choice.hidden.clone(),
        dark: choice.dark,
        black: false,
        line_mm: None,
        margin_mm: margin,
    };
    let pdf = drawing.to_pdf(&options)?;
    let (pw, ph) = drawing.page_mm(&options);
    Ok(Opened {
        content: Content::Pdf(Arc::new(pdf)),
        pages: vec![PageView {
            size: Size::new(pw, ph),
            trim: None,
            bleed: None,
        }],
        facts: Facts::Cad(CadFacts {
            drawing,
            unit_mm,
            page_scale,
        }),
        bytes: 0,
        modified: None,
    })
}

fn thumb_image(t: Thumb) -> Option<image::DynamicImage> {
    image::RgbaImage::from_raw(t.width, t.height, t.rgba).map(image::DynamicImage::ImageRgba8)
}

/// Un file in apertura in un thread a parte.
pub struct Opening {
    rx: Receiver<Result<Opened, String>>,
}

impl Opening {
    pub fn start(path: PathBuf, kind: ViewKind, cad: CadChoice, wake: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("visualizza-apri".into())
            .spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| open(&path, kind, &cad)))
                    .unwrap_or_else(|_| Err("il file ha fermato il lettore".into()));
                let _ = tx.send(r);
                wake();
            });
        if spawned.is_err() {
            let (tx, rx2) = channel();
            let _ = tx.send(Err("non riesco ad avviare la lettura".into()));
            return Self { rx: rx2 };
        }
        Self { rx }
    }

    /// Un disegno CAD rifatto con altre scelte (layer, fondo), senza rileggere il file.
    pub fn cad(
        drawing: Arc<winfiles_cad::Drawing>,
        cad: CadChoice,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        let (tx, rx) = channel();
        let _ = std::thread::Builder::new()
            .name("visualizza-cad".into())
            .spawn(move || {
                let r = cad_content(drawing, &cad);
                let _ = tx.send(r);
                wake();
            });
        Self { rx }
    }

    pub fn poll(&self) -> Option<Result<Opened, String>> {
        self.rx.try_recv().ok()
    }
}

/// Un pezzo di pagina da disegnare: in mm sulla pagina, a `px_per_mm` pixel per millimetro.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    pub page: usize,
    pub region: [f64; 4],
    pub px_per_mm: f64,
}

impl Tile {
    /// Questo pezzo basta per mostrare `region` a `px_per_mm`.
    pub fn covers(&self, page: usize, region: [f64; 4], px_per_mm: f64) -> bool {
        let [x, y, w, h] = self.region;
        let [rx, ry, rw, rh] = region;
        self.page == page
            && rx >= x - 0.01
            && ry >= y - 0.01
            && rx + rw <= x + w + 0.01
            && ry + rh <= y + h + 0.01
            && self.px_per_mm >= px_per_mm * 0.9
            && self.px_per_mm <= px_per_mm * 1.6
    }
}

enum Job {
    Base {
        page: usize,
        side: u32,
    },
    Tile(Tile),
    Scan {
        page: usize,
    },
    Text {
        page: usize,
    },
    /// Un pezzo di pagina (mm) da copiare come immagine, a `px_per_mm`.
    Copy {
        page: usize,
        region: [f64; 4],
        px_per_mm: f64,
    },
}

enum Done {
    Base {
        page: usize,
        side: u32,
        result: Result<Thumb, String>,
    },
    Tile {
        tile: Tile,
        result: Result<(Thumb, [f64; 4]), String>,
    },
    Scan {
        page: usize,
        result: Result<PageScan, String>,
    },
    Text {
        page: usize,
        result: Result<Vec<TextGlyph>, String>,
    },
    Copy(Result<Thumb, String>),
}

/// Un carattere della pagina in mm, dall'alto a sinistra.
#[derive(Clone, Debug)]
pub struct TextGlyph {
    pub text: String,
    pub rect: [f64; 4],
}

/// Le pagine di un file disegnate in un thread a parte.
pub struct Renderer {
    jobs: Option<Sender<Job>>,
    results: Receiver<Done>,
    /// La pagina intera per pagina: lato e immagine.
    pub base: HashMap<usize, (u32, egui::TextureHandle)>,
    pub failed: HashMap<usize, String>,
    /// L'ultimo pezzo: com'era chiesto, la parte della pagina che copre davvero (mm) e l'immagine.
    pub tile: Option<(Tile, [f64; 4], egui::TextureHandle)>,
    pub scans: HashMap<usize, Result<Arc<PageScan>, String>>,
    /// Il testo delle pagine (solo i PDF).
    pub texts: HashMap<usize, Result<Arc<Vec<TextGlyph>>, String>>,
    asked_text: Vec<usize>,
    /// L'immagine chiesta da "Copia immagine", appena pronta.
    pub copied: Option<Result<Thumb, String>>,
    pub copying: bool,
    asked_base: HashMap<usize, u32>,
    asked_tile: Option<Tile>,
    asked_scan: Vec<usize>,
    name: String,
}

impl Renderer {
    pub fn start(content: Content, name: String, wake: impl Fn() + Send + 'static) -> Self {
        let (jobs, inbox) = channel::<Job>();
        let (outbox, results) = channel();
        let spawned = std::thread::Builder::new()
            .name("visualizza-disegna".into())
            .spawn(move || work(content, &inbox, &outbox, &wake));
        Self {
            jobs: spawned.is_ok().then_some(jobs),
            results,
            base: HashMap::new(),
            failed: HashMap::new(),
            tile: None,
            scans: HashMap::new(),
            texts: HashMap::new(),
            asked_text: Vec::new(),
            copied: None,
            copying: false,
            asked_base: HashMap::new(),
            asked_tile: None,
            asked_scan: Vec::new(),
            name,
        }
    }

    pub fn want_base(&mut self, page: usize, side: u32) {
        let have = self.base.get(&page).map_or(0, |(s, _)| *s);
        let asked = self.asked_base.get(&page).copied().unwrap_or(0);
        if have >= side || asked >= side || self.failed.contains_key(&page) {
            return;
        }
        if let Some(tx) = &self.jobs
            && tx.send(Job::Base { page, side }).is_ok()
        {
            self.asked_base.insert(page, side);
        }
    }

    pub fn want_tile(&mut self, tile: Tile) {
        if self.tile.as_ref().is_some_and(|(t, _, _)| *t == tile) || self.asked_tile == Some(tile) {
            return;
        }
        if let Some(tx) = &self.jobs
            && tx.send(Job::Tile(tile)).is_ok()
        {
            self.asked_tile = Some(tile);
        }
    }

    pub fn want_text(&mut self, page: usize) {
        if self.texts.contains_key(&page) || self.asked_text.contains(&page) {
            return;
        }
        if let Some(tx) = &self.jobs
            && tx.send(Job::Text { page }).is_ok()
        {
            self.asked_text.push(page);
        }
    }

    /// Chiede un pezzo di pagina come immagine da copiare; arriva in `copied`.
    pub fn want_copy(&mut self, page: usize, region: [f64; 4], px_per_mm: f64) {
        if self.copying {
            return;
        }
        if let Some(tx) = &self.jobs
            && tx
                .send(Job::Copy {
                    page,
                    region,
                    px_per_mm,
                })
                .is_ok()
        {
            self.copying = true;
        }
    }

    pub fn tile_pending(&self) -> bool {
        self.asked_tile.is_some()
    }

    pub fn want_scan(&mut self, page: usize) {
        if self.scans.contains_key(&page) || self.asked_scan.contains(&page) {
            return;
        }
        if let Some(tx) = &self.jobs
            && tx.send(Job::Scan { page }).is_ok()
        {
            self.asked_scan.push(page);
        }
    }

    /// Prende i risultati arrivati e ne fa immagini per la scheda.
    pub fn receive(&mut self, ctx: &egui::Context) {
        while let Ok(done) = self.results.try_recv() {
            match done {
                Done::Base { page, side, result } => match result {
                    Ok(t) => {
                        let tex = texture(ctx, &format!("{}-{page}-{side}", self.name), &t);
                        if self.base.get(&page).is_none_or(|(s, _)| *s <= side) {
                            self.base.insert(page, (side, tex));
                        }
                    }
                    Err(e) => {
                        self.failed.insert(page, e);
                    }
                },
                Done::Tile { tile, result } => {
                    if self.asked_tile == Some(tile) {
                        self.asked_tile = None;
                    }
                    if let Ok((t, actual)) = result {
                        let tex = texture(ctx, &format!("{}-pezzo", self.name), &t);
                        self.tile = Some((tile, actual, tex));
                    }
                }
                Done::Scan { page, result } => {
                    self.asked_scan.retain(|p| *p != page);
                    self.scans.insert(page, result.map(Arc::new));
                }
                Done::Text { page, result } => {
                    self.asked_text.retain(|p| *p != page);
                    self.texts.insert(page, result.map(Arc::new));
                }
                Done::Copy(result) => {
                    self.copying = false;
                    self.copied = Some(result);
                }
            }
        }
    }
}

fn texture(ctx: &egui::Context, name: &str, t: &Thumb) -> egui::TextureHandle {
    let size = [t.width as usize, t.height as usize];
    let img = egui::ColorImage::from_rgba_unmultiplied(size, &t.rgba);
    ctx.load_texture(name, img, egui::TextureOptions::LINEAR)
}

/// Il lavoro del thread: una richiesta alla volta, saltando i pezzi superati da uno più nuovo.
fn work(content: Content, inbox: &Receiver<Job>, outbox: &Sender<Done>, wake: &(impl Fn() + Send)) {
    let pdf = match &content {
        Content::Pdf(bytes) => Some(PdfPages::open(bytes.as_ref().clone()).map_err(|e| e.to_string())),
        Content::Image(..) => None,
    };
    let mut scan_file: Option<Result<PdfFile, String>> = None;
    while let Ok(first) = inbox.recv() {
        // Dei pezzi in coda conta solo l'ultimo; le pagine intere e le letture si fanno tutte.
        let mut queue = vec![first];
        while let Ok(next) = inbox.try_recv() {
            queue.push(next);
        }
        let last_tile = queue.iter().rposition(|j| matches!(j, Job::Tile(_)));
        for (i, job) in queue.into_iter().enumerate() {
            let done = match job {
                Job::Base { page, side } => {
                    let result = match (&content, &pdf) {
                        (Content::Pdf(_), Some(Ok(p))) => p.render(page, side).map_err(|e| e.to_string()),
                        (Content::Pdf(_), Some(Err(e))) => Err(e.clone()),
                        (Content::Image(img, _), _) => Ok(image_base(img, side)),
                        _ => Err("niente da disegnare".into()),
                    };
                    Done::Base { page, side, result }
                }
                Job::Tile(tile) => {
                    if Some(i) != last_tile {
                        continue;
                    }
                    let result = match (&content, &pdf) {
                        (Content::Pdf(_), Some(Ok(p))) => {
                            let k = (72.0 / 25.4) as f32;
                            let [x, y, w, h] = tile.region.map(|v| v as f32 * k);
                            p.render_region(tile.page, (tile.px_per_mm as f32) / k, [x, y, w, h])
                                .map(|t| (t, tile.region))
                                .map_err(|e| e.to_string())
                        }
                        (Content::Image(img, dpi), _) => image_tile(img, *dpi, &tile),
                        _ => Err("niente da disegnare".into()),
                    };
                    Done::Tile { tile, result }
                }
                Job::Scan { page } => {
                    let file = scan_file.get_or_insert_with(|| match &content {
                        Content::Pdf(bytes) => {
                            PdfFile::from_bytes(bytes.as_ref().clone()).map_err(|e| e.to_string())
                        }
                        Content::Image(..) => Err("non è un PDF".into()),
                    });
                    let result = match file {
                        Ok(f) => f.scan_page(page).map_err(|e| e.to_string()),
                        Err(e) => Err(e.clone()),
                    };
                    Done::Scan { page, result }
                }
                Job::Text { page } => {
                    let result = match &pdf {
                        Some(Ok(p)) => p.text(page).map_err(|e| e.to_string()).map(|g| {
                            let k = 25.4 / 72.0;
                            g.into_iter()
                                .map(|t| TextGlyph {
                                    text: t.text,
                                    rect: t.rect.map(|v| f64::from(v) * k),
                                })
                                .collect()
                        }),
                        Some(Err(e)) => Err(e.clone()),
                        None => Err("non è un PDF".into()),
                    };
                    Done::Text { page, result }
                }
                Job::Copy {
                    page,
                    region,
                    px_per_mm,
                } => {
                    let tile = Tile {
                        page,
                        region,
                        px_per_mm,
                    };
                    let result = match (&content, &pdf) {
                        (Content::Pdf(_), Some(Ok(p))) => {
                            let k = (72.0 / 25.4) as f32;
                            let [x, y, w, h] = region.map(|v| v as f32 * k);
                            p.render_region(page, (px_per_mm as f32) / k, [x, y, w, h])
                                .map_err(|e| e.to_string())
                        }
                        (Content::Image(img, dpi), _) => image_crop(img, *dpi, &tile),
                        (_, Some(Err(e))) => Err(e.clone()),
                        _ => Err("niente da copiare".into()),
                    };
                    Done::Copy(result)
                }
            };
            if outbox.send(done).is_err() {
                return;
            }
            wake();
        }
    }
}

/// Il pezzo di un'immagine ai suoi pixel veri (per "Copia immagine").
fn image_crop(img: &image::DynamicImage, dpi: f64, tile: &Tile) -> Result<Thumb, String> {
    let k = dpi / 25.4;
    let [x, y, w, h] = tile.region;
    let x0 = ((x * k).floor().max(0.0) as u32).min(img.width());
    let y0 = ((y * k).floor().max(0.0) as u32).min(img.height());
    let x1 = (((x + w) * k).ceil().max(0.0) as u32).min(img.width());
    let y1 = (((y + h) * k).ceil().max(0.0) as u32).min(img.height());
    if x1 <= x0 || y1 <= y0 {
        return Err("la zona scelta è fuori dall'immagine".into());
    }
    let rgba = img.crop_imm(x0, y0, x1 - x0, y1 - y0).to_rgba8();
    Ok(Thumb {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

fn image_base(img: &image::DynamicImage, side: u32) -> Thumb {
    let (w, h) = (img.width(), img.height());
    let scaled = if w.max(h) > side {
        img.resize(side, side, image::imageops::FilterType::Triangle)
    } else {
        img.clone()
    };
    let rgba = scaled.to_rgba8();
    Thumb {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    }
}

/// Il pezzo di un'immagine: si ritagliano i pixel veri e si portano alla misura dello schermo.
fn image_tile(img: &image::DynamicImage, dpi: f64, tile: &Tile) -> Result<(Thumb, [f64; 4]), String> {
    let k = dpi / 25.4;
    let [x, y, w, h] = tile.region;
    let (iw, ih) = (img.width(), img.height());
    let x0 = ((x * k).floor().max(0.0) as u32).min(iw.saturating_sub(1));
    let y0 = ((y * k).floor().max(0.0) as u32).min(ih.saturating_sub(1));
    let x1 = (((x + w) * k).ceil().max(0.0) as u32).clamp(x0 + 1, iw);
    let y1 = (((y + h) * k).ceil().max(0.0) as u32).clamp(y0 + 1, ih);
    let crop = img.crop_imm(x0, y0, x1 - x0, y1 - y0);
    // La parte della pagina coperta davvero: i pixel interi attorno a quella chiesta.
    let actual = [
        f64::from(x0) / k,
        f64::from(y0) / k,
        f64::from(x1 - x0) / k,
        f64::from(y1 - y0) / k,
    ];
    let out_w = ((actual[2] * tile.px_per_mm).round() as u32).clamp(1, MAX_SIDE);
    let out_h = ((actual[3] * tile.px_per_mm).round() as u32).clamp(1, MAX_SIDE);
    let _ = (w, h);
    // Ingrandendo oltre i pixel veri, i pixel si vedono come quadretti (come nei programmi di
    // fotoritocco): così si giudica la risoluzione.
    let filter = if f64::from(out_w) > f64::from(x1 - x0) * 1.5 {
        image::imageops::FilterType::Nearest
    } else {
        image::imageops::FilterType::Triangle
    };
    let scaled = crop.resize_exact(out_w, out_h, filter).to_rgba8();
    Ok((
        Thumb {
            width: scaled.width(),
            height: scaled.height(),
            rgba: scaled.into_raw(),
        },
        actual,
    ))
}
