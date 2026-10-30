//! Fustella e abbondanza sulle immagini (ADR 0028): il pannello a destra di Modifica immagini.
//! L'abbondanza si fa con i pixel (`Op::Bleed`, che si annulla come le altre modifiche); la
//! fustella, rettangolare o con gli angoli tondi sul formato finito, o che segue il contorno del
//! soggetto (staccato da un colore di sfondo o dalla trasparenza), si scrive in tinta piatta nel
//! PDF che si salva da qui, con l'immagine alla sua risoluzione, TrimBox e BleedBox.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use egui::{Color32, Pos2, Rect, Stroke, pos2};
use serde::{Deserialize, Serialize};
use winfiles_geom::{Unit, format_length, format_size};
use winfiles_image_edit::{Background, BleedMethod, ContourSpec, Format, Op, Picture, SaveOptions};
use winfiles_pdf_edit::{NewDie, PdfFile};

use crate::app::segmented;
use crate::nesting::{heading, hint, hint_color, length_row};
use crate::pdf::tracciati::{InkName, Role};
use crate::theme::{Tokens, semibold};

/// La forma della fustella.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DieShape {
    /// Solo l'immagine con TrimBox e BleedBox, senza linea di taglio.
    None,
    #[default]
    Rect,
    Rounded,
    /// Il contorno del soggetto, staccato dallo sfondo.
    Contour,
}

type Contour = Vec<[f64; 2]>;

/// Le scelte del pannello, ricordate nelle impostazioni.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DieSettings {
    /// L'abbondanza da aggiungere, in mm.
    pub amount: f64,
    pub mirror: bool,
    pub shape: DieShape,
    /// Raggio degli angoli tondi, in mm.
    pub radius: f64,
    /// Per il contorno: lo sfondo è la trasparenza invece di un colore.
    pub transparent: bool,
    /// Il colore dello sfondo.
    pub background: [u8; 3],
    /// Quanto un pixel può scostarsi dal colore dello sfondo, in percento.
    pub tolerance: f64,
    /// Di quanto la fustella sta fuori dal soggetto (negativo: dentro), in mm.
    pub offset: f64,
    /// Lisciatura della fustella dal contorno, in mm.
    pub smooth: f64,
    /// Pezzi e fori più piccoli di questa area si ignorano, in mm².
    pub min_area: f64,
    /// Taglia anche i fori dentro il soggetto.
    pub holes: bool,
    /// Il nome della tinta della fustella (vuoto: la tinta di taglio dell'elenco).
    pub ink: String,
    /// Il colore con cui la tinta si vede (C, M, Y, K da 0 a 1); senza, quello dell'elenco.
    pub ink_cmyk: Option<[f64; 4]>,
    /// Lo spessore della linea, in mm.
    pub width: f64,
}

impl Default for DieSettings {
    fn default() -> Self {
        Self {
            amount: 3.0,
            mirror: true,
            shape: DieShape::Rect,
            radius: 5.0,
            transparent: false,
            background: [255, 255, 255],
            tolerance: 10.0,
            offset: 2.0,
            smooth: 0.0,
            min_area: 4.0,
            holes: false,
            ink: String::new(),
            ink_cmyk: None,
            width: 0.1,
        }
    }
}

impl DieSettings {
    pub fn sanitized(mut self) -> Self {
        let fix = |v: f64, lo: f64, hi: f64, def: f64| if v.is_finite() { v.clamp(lo, hi) } else { def };
        self.amount = fix(self.amount, 0.5, 100.0, 3.0);
        self.radius = fix(self.radius, 0.0, 500.0, 5.0);
        self.tolerance = fix(self.tolerance, 0.0, 100.0, 10.0);
        self.smooth = fix(self.smooth, 0.0, 20.0, 0.0);
        self.width = fix(self.width, 0.0, 5.0, 0.1);
        self.ink = self.ink.trim().chars().take(127).collect();
        if let Some(c) = &mut self.ink_cmyk {
            for v in c.iter_mut() {
                *v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
            }
        }
        self.offset = fix(self.offset, -20.0, 50.0, 2.0);
        self.min_area = fix(self.min_area, 0.0, 10_000.0, 4.0);
        self
    }

    fn spec(&self) -> ContourSpec {
        ContourSpec {
            background: if self.transparent {
                Background::Transparent
            } else {
                Background::Color(self.background)
            },
            tolerance: self.tolerance,
            offset: self.offset,
            min_area: self.min_area,
            holes: self.holes,
            smooth: self.smooth,
        }
    }

    fn method(&self) -> BleedMethod {
        if self.mirror {
            BleedMethod::Mirror
        } else {
            BleedMethod::Stretch
        }
    }
}

/// Cosa chiede il pannello all'immagine.
pub enum DieAction {
    /// Una modifica dell'immagine (l'abbondanza).
    Apply(Op),
    /// Salvare il PDF con la fustella.
    Save,
    Close,
}

/// Il contorno calcolato: per quale immagine e quali scelte, e com'è venuto.
type TraceResult = Result<Vec<Contour>, String>;
type Traced = (String, TraceResult);

#[derive(Default)]
pub struct DieTool {
    pub open: bool,
    pub settings: DieSettings,
    saving: Option<mpsc::Receiver<Result<PathBuf, String>>>,
    /// Il contagocce acceso: il prossimo clic sull'immagine sceglie il colore dello sfondo.
    pub picking: bool,
    traced: Option<Traced>,
    tracing: Option<(String, mpsc::Receiver<TraceResult>)>,
    /// Tinta e saturazione della ruota dei colori, tenute quando il colore passa dal grigio.
    hue: f64,
    sat: f64,
}

impl DieTool {
    pub fn busy(&self) -> bool {
        self.saving.is_some()
    }

    /// Il salvataggio finito, con il messaggio per la barra di stato.
    pub fn poll(&mut self) -> Option<String> {
        let r = self.saving.as_ref()?.try_recv().ok()?;
        self.saving = None;
        Some(match r {
            Ok(p) => format!("PDF con fustella salvato in {}", file_name(&p)),
            Err(e) => format!("PDF non salvato: {e}"),
        })
    }

    /// La chiave del contorno: l'immagine (`owner`, che cambia a ogni modifica) e le scelte.
    fn key(&self, owner: u64) -> String {
        format!("{owner}:{:?}", self.settings.spec())
    }

    /// Tiene aggiornato il contorno del soggetto: lo ricalcola in un thread quando cambiano
    /// l'immagine o le scelte. `owner` dice quale immagine e quale versione.
    pub fn update(&mut self, picture: &Picture, owner: u64, wake: &egui::Context) {
        if let Some((key, rx)) = &self.tracing
            && let Ok(r) = rx.try_recv()
        {
            self.traced = Some((key.clone(), r));
            self.tracing = None;
        }
        if !self.open || self.settings.shape != DieShape::Contour || picture.cut_mm().is_some() {
            return;
        }
        let key = self.key(owner);
        let done = self.traced.as_ref().is_some_and(|(k, _)| *k == key);
        let running = self.tracing.as_ref().is_some_and(|(k, _)| *k == key);
        if done || running || self.tracing.is_some() {
            // Uno alla volta: finito questo, al prossimo disegno parte quello giusto.
            return;
        }
        let picture = picture.clone();
        let spec = self.settings.spec();
        let (tx, rx) = mpsc::channel();
        let wake = wake.clone();
        let spawned = std::thread::Builder::new()
            .name("contorno-immagine".into())
            .spawn(move || {
                let r = winfiles_image_edit::trace(&picture, &spec).map_err(|e| e.to_string());
                let _ = tx.send(r);
                wake.request_repaint();
            });
        if spawned.is_ok() {
            self.tracing = Some((key, rx));
        }
    }

    /// Il contorno calcolato per `owner` e le scelte di adesso, se è pronto.
    fn traced(&self, owner: u64) -> Option<&Result<Vec<Contour>, String>> {
        let key = self.key(owner);
        self.traced.as_ref().filter(|(k, _)| *k == key).map(|(_, r)| r)
    }

    /// La fustella in mm dell'immagine (dall'angolo in alto a sinistra), se se ne vuole una.
    pub fn outline(&self, picture: &Picture, owner: u64) -> Option<Vec<Contour>> {
        let t = picture.trim_mm();
        match self.settings.shape {
            DieShape::None => None,
            DieShape::Rect => Some(vec![rounded_rect(t, 0.0)]),
            DieShape::Rounded => Some(vec![rounded_rect(t, self.settings.radius)]),
            DieShape::Contour => picture
                .cut_mm()
                .or_else(|| self.traced(owner).and_then(|r| r.as_ref().ok()).cloned()),
        }
    }

    pub fn panel(
        &mut self,
        ui: &mut egui::Ui,
        picture: &Picture,
        owner: u64,
        inks: &[InkName],
        unit: Unit,
    ) -> Option<DieAction> {
        let t = Tokens::get(ui.ctx());
        let mut action = None;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Fustella e abbondanza")
                    .font(semibold(14.0))
                    .color(t.text_strong),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::icons::button(ui, "x", "Chiudi e torna ai dati", false, true, 24.0).clicked() {
                    action = Some(DieAction::Close);
                }
            });
        });
        ui.add_space(8.0);
        let pinned = picture.cut_mm();
        let traced = pinned
            .as_ref()
            .map(Ok)
            .or_else(|| self.traced(owner).map(|r| r.as_ref()))
            .map(|r: Result<&Vec<Contour>, &String>| {
                r.map(|c| (c.len(), winfiles_image_edit::contour_area(c) / 1_000_000.0))
                    .map_err(|e| {
                        let mut c = e.chars();
                        c.next()
                            .map(|f| f.to_uppercase().chain(c).collect::<String>() + ".")
                            .unwrap_or_default()
                    })
            });
        let s = &mut self.settings;
        let f = picture.trim_mm();
        let finished = winfiles_geom::Size::new(f[2] - f[0], f[3] - f[1]);
        let full = picture.size_mm();

        heading(ui, "Formato finito");
        match picture.trim() {
            Some(_) => {
                let sides = [f[0], f[1], full.w - f[2], full.h - f[3]];
                let same = sides.iter().all(|v| (v - sides[0]).abs() < 0.05);
                let bleed = if same {
                    format!("{} per lato", format_length(sides[0], Unit::Mm))
                } else {
                    sides.map(|v| format_length(v, Unit::Mm)).join(" · ")
                };
                hint_color(
                    ui,
                    &format!("{} · abbondanza {bleed}", format_size(finished, unit)),
                    t.text,
                );
            }
            None => {
                hint_color(
                    ui,
                    &format!(
                        "{} · l'immagine intera, ancora senza abbondanza",
                        format_size(finished, unit)
                    ),
                    t.text,
                );
            }
        }
        ui.add_space(12.0);

        heading(ui, "Abbondanza");
        egui::Grid::new("abbondanza-immagine")
            .num_columns(2)
            .spacing(egui::vec2(10.0, 8.0))
            .show(ui, |ui| {
                length_row(ui, "Per lato", &mut s.amount, Unit::Mm, 0.5..=100.0);
            });
        ui.add_space(6.0);
        segmented(
            ui,
            "metodo-abbondanza-immagine",
            &mut s.mirror,
            &[(true, "A specchio"), (false, "Allunga il bordo")],
        );
        ui.add_space(8.0);
        let px = winfiles_image_edit::pixels_for_mm(s.amount, picture.dpi());
        let label = format!("Aggiungi {} per lato", format_length(s.amount, Unit::Mm));
        if ui
            .add(egui::Button::new(label).min_size(egui::vec2(0.0, 32.0)))
            .on_hover_text(
                "Allarga l'immagine con i suoi pixel, ribaltati o allungati: quella di adesso diventa il \
                 formato finito (Ctrl+Z annulla)",
            )
            .clicked()
        {
            action = Some(DieAction::Apply(Op::Bleed {
                sides: [px; 4],
                method: s.method(),
            }));
        }
        hint(
            ui,
            if s.mirror {
                "A specchio continua linee e sfumature: va bene quasi sempre."
            } else {
                "Allunga il bordo ripete l'ultima fila di pixel: meglio sui fondi uniti."
            },
        );
        if s.shape == DieShape::Contour {
            ui.add_space(8.0);
            let label = format!("Aggiungi {} lungo la fustella", format_length(s.amount, Unit::Mm));
            let ok = matches!(traced, Some(Ok(_)));
            if ui
                .add_enabled(ok, egui::Button::new(label).min_size(egui::vec2(0.0, 32.0)))
                .on_hover_text(
                    "Oltre la fustella, per questa misura, ogni pixel prende il colore più vicino \
                     dentro la fustella; dentro non cambia nulla (Ctrl+Z annulla)",
                )
                .clicked()
            {
                action = Some(DieAction::Apply(Op::ContourBleed {
                    spec: s.spec(),
                    band: s.amount,
                }));
            }
            hint(
                ui,
                "Lungo il contorno: la fustella resta dov'è e oltre la linea continua quello che c'è \
                 dentro (con la distanza a 0 il soggetto, con un bordo il bordo).",
            );
        }
        ui.add_space(12.0);

        heading(ui, "Fustella");
        segmented(
            ui,
            "forma-fustella-immagine",
            &mut s.shape,
            &[
                (DieShape::Rect, "Rettangolo"),
                (DieShape::Rounded, "Tondi"),
                (DieShape::Contour, "Contorno"),
                (DieShape::None, "Nessuna"),
            ],
        );
        if s.shape == DieShape::Contour {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Sfondo").color(t.text));
            ui.add_space(2.0);
            segmented(
                ui,
                "sfondo-fustella-immagine",
                &mut s.transparent,
                &[(false, "Colore"), (true, "Trasparente")],
            );
            if !s.transparent {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let swatch = ui
                        .color_edit_button_srgb(&mut s.background)
                        .on_hover_text("Il colore dello sfondo da togliere");
                    // Un bordo, così anche il bianco sul pannello chiaro si vede.
                    ui.painter().rect_stroke(
                        swatch.rect,
                        2.0,
                        Stroke::new(1.0, t.field_border),
                        egui::StrokeKind::Outside,
                    );
                    let label = if self.picking {
                        "Clicca sull'immagine…"
                    } else {
                        "Contagocce"
                    };
                    if crate::fluent::toggle(ui, self.picking, label)
                        .on_hover_text("Prendi il colore dello sfondo con un clic sull'immagine")
                        .clicked()
                    {
                        self.picking = !self.picking;
                    }
                    if ui
                        .button("Dall'angolo")
                        .on_hover_text("Il colore del pixel in alto a sinistra")
                        .clicked()
                        && let Some(p) = pixel(picture, 0, 0)
                    {
                        s.background = p;
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Tolleranza").color(t.text));
                    ui.add(
                        crate::fluent::Slider::new(&mut s.tolerance, 0.0..=60.0)
                            .suffix(" %")
                            .fixed_decimals(0),
                    )
                    .on_hover_text("Più alta, più colori vicini allo sfondo si tolgono (ombre, sfumature)");
                });
            }
            ui.add_space(6.0);
            egui::Grid::new("contorno-fustella-immagine")
                .num_columns(2)
                .spacing(egui::vec2(10.0, 8.0))
                .show(ui, |ui| {
                    length_row(ui, "Distanza", &mut s.offset, Unit::Mm, -20.0..=50.0);
                });
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Lisciatura").color(t.text));
                ui.add(
                    crate::fluent::Slider::new(&mut s.smooth, 0.0..=20.0)
                        .step_by(0.5)
                        .custom_formatter(|v| format!("{} mm", winfiles_geom::format_number(v, 1, true)))
                        .custom_parser(|text| {
                            let clean = text.trim().trim_end_matches("mm").trim().replace(',', ".");
                            clean.parse::<f64>().ok()
                        }),
                )
                .on_hover_text(
                    "Arrotonda le rientranze strette e le punte: la lama segue una linea \
                     morbida invece di ogni dettaglio, senza mai entrare nel soggetto",
                );
            });
            ui.add_space(4.0);
            if pinned.is_some() {
                hint_color(
                    ui,
                    "Fustella fissata dall'abbondanza lungo il contorno: per cambiarla, Ctrl+Z.",
                    t.text,
                );
            }
            ui.add(crate::fluent::checkbox(
                &mut s.holes,
                "Taglia anche i fori interni",
            ))
            .on_hover_text("Il centro di una O, gli spazi chiusi del colore dello sfondo");
            let state = match traced {
                Some(Ok((n, area))) => {
                    hint(
                        ui,
                        &format!(
                            "{} {} · {} · {} fuori dal soggetto (negativa: dentro).",
                            n,
                            if n == 1 { "contorno" } else { "contorni" },
                            winfiles_geom::format_area(area),
                            format_length(s.offset, Unit::Mm)
                        ),
                    );
                    None
                }
                Some(Err(e)) => Some(e),
                None => {
                    hint(ui, "Calcolo del contorno…");
                    None
                }
            };
            if let Some(e) = state {
                hint_color(ui, &e, t.warning);
            }
        }
        if s.shape == DieShape::Rounded {
            ui.add_space(6.0);
            egui::Grid::new("raggio-fustella-immagine")
                .num_columns(2)
                .spacing(egui::vec2(10.0, 8.0))
                .show(ui, |ui| {
                    let max = (finished.w.min(finished.h) / 2.0).max(0.0);
                    length_row(ui, "Raggio", &mut s.radius, Unit::Mm, 0.0..=max.max(0.1));
                });
        }
        if s.shape != DieShape::None {
            ui.add_space(10.0);
            heading(ui, "Tinta della fustella");
            let (name, cmyk) = die_ink(s, inks);
            let mut text = name.clone();
            ui.horizontal(|ui| {
                let edit = egui::TextEdit::singleline(&mut text)
                    .hint_text("Nome della tinta")
                    .desired_width((ui.available_width() - 64.0).max(80.0));
                if crate::fluent::text_field(ui, edit).changed() {
                    s.ink = text.trim().chars().take(127).collect();
                    // Un nome già nell'elenco porta il suo colore.
                    s.ink_cmyk = inks
                        .iter()
                        .find(|i| i.name == s.ink)
                        .map(InkName::color)
                        .or(Some(cmyk));
                }
                egui::ComboBox::from_id_salt("tinta-fustella-immagine")
                    .icon(crate::fluent::combo_icon)
                    .selected_text("")
                    .width(28.0)
                    .show_ui(ui, |ui| {
                        for i in inks {
                            let label = format!("{}  ·  {}", i.name, i.role.label());
                            if ui.selectable_label(i.name == name, label).clicked() {
                                s.ink = i.name.clone();
                                s.ink_cmyk = Some(i.color());
                            }
                        }
                    })
                    .response
                    .on_hover_text("Le tinte dell'elenco (Taglio, Cordone…)");
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Spessore").color(t.text));
                crate::fluent::number(
                    ui,
                    90.0,
                    egui::DragValue::new(&mut s.width)
                        .range(0.0..=5.0)
                        .speed(0.01)
                        .custom_formatter(|v, _| format!("{} mm", winfiles_geom::format_number(v, 2, true)))
                        .custom_parser(|text| {
                            let clean = text.trim().trim_end_matches("mm").trim().replace(',', ".");
                            clean.parse::<f64>().ok()
                        }),
                );
            });
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Colore a schermo e nelle prove").color(t.text))
                .on_hover_text("Il RIP stampa la tinta per nome; questo è il colore con cui si vede");
            let mut c = cmyk;
            crate::pdf::tracciati::colour_wheel(
                ui,
                "fustella-immagine",
                &mut c,
                &mut self.hue,
                &mut self.sat,
            );
            if c != cmyk {
                s.ink_cmyk = Some(c);
                if s.ink.is_empty() {
                    s.ink = name.clone();
                }
            }
            hint(
                ui,
                &format!(
                    "{} tinta piatta «{name}» in sovrastampa.",
                    if s.shape == DieShape::Contour {
                        "In"
                    } else {
                        "Sul formato finito, in"
                    }
                ),
            );
        }
        ui.add_space(14.0);
        let ready = s.shape != DieShape::Contour || self.outline(picture, owner).is_some();
        let save = egui::Button::image_and_text(
            crate::icons::image("save", 18.0, t.on_primary),
            egui::RichText::new("Salva PDF con fustella…")
                .font(semibold(14.0))
                .color(t.on_primary),
        )
        .fill(t.primary)
        .min_size(egui::vec2(0.0, 34.0));
        if ui
            .add_enabled(!self.busy() && ready, save)
            .on_hover_text("Un PDF alla misura dell'immagine, con l'immagine a piena risoluzione, la fustella, TrimBox e BleedBox: pronto per Onyx e per il nesting")
            .clicked()
        {
            action = Some(DieAction::Save);
        }
        if self.busy() {
            hint(ui, "Salvataggio in corso…");
        } else if picture.trim().is_none() {
            hint(
                ui,
                "Senza abbondanza il PDF va bene solo per tagli che non arrivano al bordo.",
            );
        }
        action
    }

    /// Chiede dove salvare e scrive il PDF in un thread a parte. `source` è il file aperto (per
    /// il nome proposto e la cartella).
    pub fn save(
        &mut self,
        picture: &Picture,
        owner: u64,
        source: &Path,
        inks: &[InkName],
        wake: &egui::Context,
    ) -> Option<String> {
        if self.busy() {
            return None;
        }
        let stem = source
            .file_stem()
            .map_or_else(|| "immagine".into(), |s| s.to_string_lossy().into_owned());
        let mut dialog = rfd::FileDialog::new()
            .set_title("Salva il PDF con la fustella")
            .set_file_name(format!("{stem}_fustella.pdf"))
            .add_filter("PDF", &["pdf"]);
        if let Some(dir) = source.parent() {
            dialog = dialog.set_directory(dir);
        }
        let mut path = dialog.save_file()?;
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            path.as_mut_os_string().push(".pdf");
        }
        let picture = picture.clone();
        let outline = self.outline(&picture, owner);
        let (name, cmyk) = die_ink(&self.settings, inks);
        let die = NewDie {
            name,
            cmyk,
            width: self.settings.width,
            overprint: true,
        };
        let jpeg = matches!(winfiles_image_edit::default_format(source), Format::Jpeg);
        let (tx, rx) = mpsc::channel();
        let wake = wake.clone();
        let spawned = std::thread::Builder::new()
            .name("fustella-immagine".into())
            .spawn(move || {
                let result = write_pdf(&picture, outline.as_deref(), &die, jpeg, &path).map(|()| path);
                let _ = tx.send(result);
                wake.request_repaint();
            });
        if spawned.is_err() {
            return Some("Impossibile avviare il salvataggio".into());
        }
        self.saving = Some(rx);
        Some("Salvataggio del PDF con la fustella…".into())
    }

    /// Sull'anteprima: il formato finito tratteggiato e la fustella. `paper` è l'immagine a
    /// schermo.
    pub fn paint(&self, ui: &egui::Ui, paper: Rect, picture: &Picture, owner: u64, inks: &[InkName]) {
        let full = picture.size_mm();
        if full.w <= 0.0 || full.h <= 0.0 {
            return;
        }
        let k = paper.width() / full.w as f32;
        let to = |p: [f64; 2]| pos2(paper.left() + p[0] as f32 * k, paper.top() + p[1] as f32 * k);
        let painter = ui.painter_at(paper.expand(2.0));
        let f = picture.trim_mm();
        if picture.trim().is_some() {
            let r = Rect::from_min_max(to([f[0], f[1]]), to([f[2], f[3]]));
            let corners = [
                r.left_top(),
                r.right_top(),
                r.right_bottom(),
                r.left_bottom(),
                r.left_top(),
            ];
            for w in corners.windows(2) {
                if let [a, b] = w {
                    painter.extend(egui::Shape::dashed_line(
                        &[*a, *b],
                        Stroke::new(1.0, Color32::from_white_alpha(220)),
                        5.0,
                        4.0,
                    ));
                    painter.extend(egui::Shape::dashed_line(
                        &[*a, *b],
                        Stroke::new(1.0, Color32::from_black_alpha(160)),
                        4.0,
                        5.0,
                    ));
                }
            }
        }
        if let Some(lines) = self.outline(picture, owner) {
            let c = die_ink(&self.settings, inks).1;
            let rgb =
                |v: f64, kk: f64| ((1.0 - v.clamp(0.0, 1.0)) * (1.0 - kk.clamp(0.0, 1.0)) * 255.0) as u8;
            let color = Color32::from_rgb(rgb(c[0], c[3]), rgb(c[1], c[3]), rgb(c[2], c[3]));
            for line in lines {
                let pts: Vec<Pos2> = line.iter().map(|p| to(*p)).collect();
                // Un filo chiaro sotto, così la linea si vede anche su un fondo dello stesso colore.
                painter.add(egui::Shape::closed_line(
                    pts.clone(),
                    Stroke::new(3.0, Color32::from_white_alpha(170)),
                ));
                painter.add(egui::Shape::closed_line(pts, Stroke::new(1.5, color)));
            }
        }
    }

    /// Il contagocce: un clic sull'immagine a schermo (`paper`) prende il colore dello sfondo.
    pub fn pick(&mut self, picture: &Picture, paper: Rect, at: Pos2) {
        if paper.width() <= 0.0 || paper.height() <= 0.0 {
            return;
        }
        let x = ((at.x - paper.left()) / paper.width() * picture.width() as f32).floor();
        let y = ((at.y - paper.top()) / paper.height() * picture.height() as f32).floor();
        if x >= 0.0
            && y >= 0.0
            && let Some(p) = pixel(picture, x as u32, y as u32)
        {
            self.settings.background = p;
            self.settings.transparent = false;
        }
        self.picking = false;
    }
}

/// Il colore RGB di un pixel dell'immagine.
fn pixel(picture: &Picture, x: u32, y: u32) -> Option<[u8; 3]> {
    use image::GenericImageView;
    let img = picture.image();
    if x >= img.width() || y >= img.height() {
        return None;
    }
    let p = img.get_pixel(x, y).0;
    Some([p[0], p[1], p[2]])
}

/// La tinta del taglio: la prima con il ruolo Taglio, o «Taglio».
/// Il nome e il colore della tinta della fustella: quelli scelti, o la tinta di taglio.
fn die_ink(s: &DieSettings, inks: &[InkName]) -> (String, [f64; 4]) {
    let cut = cut_ink(inks);
    let name = if s.ink.trim().is_empty() {
        cut.name.clone()
    } else {
        s.ink.trim().to_string()
    };
    let listed = inks.iter().find(|i| i.name == name).map(InkName::color);
    (name, s.ink_cmyk.or(listed).unwrap_or_else(|| cut.color()))
}

fn cut_ink(inks: &[InkName]) -> InkName {
    inks.iter()
        .find(|i| i.role == Role::Cut)
        .cloned()
        .unwrap_or(InkName {
            name: "Taglio".into(),
            role: Role::Cut,
            cmyk: None,
        })
}

/// Il PDF di una pagina: l'immagine a tutta pagina, il formato finito come TrimBox e la fustella.
fn write_pdf(
    picture: &Picture,
    outline: Option<&[Contour]>,
    die: &NewDie,
    jpeg: bool,
    path: &Path,
) -> Result<(), String> {
    // Un JPEG resta JPEG (alta qualità); il resto senza perdite.
    let options = if jpeg && !picture.image().color().has_alpha() {
        SaveOptions {
            format: Format::Jpeg,
            quality: 95,
            keep_icc: true,
        }
    } else {
        SaveOptions {
            format: Format::Png,
            quality: 100,
            keep_icc: true,
        }
    };
    let bytes = winfiles_image_edit::encode(picture, options).map_err(|e| e.to_string())?;
    let pdf = winfiles_pdf_out::image_page_bytes(bytes, picture.size_mm()).map_err(|e| e.to_string())?;
    let mut file = PdfFile::from_bytes(pdf).map_err(|e| e.to_string())?;
    file.set_trim(0, picture.trim_mm()).map_err(|e| e.to_string())?;
    if let Some(lines) = outline.filter(|l| !l.is_empty()) {
        file.draw_die(0, lines, die).map_err(|e| e.to_string())?;
    }
    file.save(path).map_err(|e| e.to_string())
}

/// Un rettangolo `r` (sinistra, alto, destra, basso) con gli angoli tondi di raggio `radius`,
/// come contorno chiuso: ogni quarto di cerchio in segmenti di al massimo 3°, abbastanza fitti
/// perché il plotter segua una curva.
pub fn rounded_rect(r: [f64; 4], radius: f64) -> Vec<[f64; 2]> {
    let [x0, y0, x1, y1] = r;
    let rad = radius.max(0.0).min((x1 - x0) / 2.0).min((y1 - y0) / 2.0);
    if rad < 0.01 {
        return vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    }
    let steps = 30;
    let corner = |cx: f64, cy: f64, start: f64| {
        (0..=steps).map(move |i| {
            let a = (start + 90.0 * f64::from(i) / f64::from(steps)).to_radians();
            [cx + rad * a.cos(), cy + rad * a.sin()]
        })
    };
    // Senso orario a schermo (y in giù): in alto a destra, in basso a destra, in basso a
    // sinistra, in alto a sinistra.
    corner(x1 - rad, y0 + rad, 270.0)
        .chain(corner(x1 - rad, y1 - rad, 0.0))
        .chain(corner(x0 + rad, y1 - rad, 90.0))
        .chain(corner(x0 + rad, y0 + rad, 180.0))
        .collect()
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gli_angoli_tondi_stanno_nel_rettangolo() {
        let c = rounded_rect([10.0, 20.0, 110.0, 70.0], 5.0);
        assert_eq!(c.len(), 4 * 31);
        for p in &c {
            assert!(
                p[0] >= 10.0 - 1e-9 && p[0] <= 110.0 + 1e-9 && p[1] >= 20.0 - 1e-9 && p[1] <= 70.0 + 1e-9
            );
        }
        // I lati arrivano ai bordi.
        assert!(c.iter().any(|p| (p[1] - 20.0).abs() < 1e-9));
        assert!(c.iter().any(|p| (p[0] - 110.0).abs() < 1e-9));
        // Il raggio non supera metà del lato corto; senza raggio, quattro punti.
        let big = rounded_rect([0.0, 0.0, 100.0, 20.0], 50.0);
        assert!(big.iter().all(|p| p[1] >= -1e-9 && p[1] <= 20.0 + 1e-9));
        assert_eq!(rounded_rect([0.0, 0.0, 10.0, 10.0], 0.0).len(), 4);
    }

    #[test]
    fn il_pdf_ha_formato_finito_e_fustella() {
        let dir = std::env::temp_dir().join(format!("winfiles-fustella-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("prova.png");
        image::RgbImage::from_pixel(300, 200, image::Rgb([20, 120, 200]))
            .save(&src)
            .unwrap();
        let mut ed = winfiles_image_edit::Editor::open(&src).unwrap();
        ed.apply(Op::Dpi(254.0)).unwrap();
        ed.apply(Op::Bleed {
            sides: [30; 4],
            method: BleedMethod::Mirror,
        })
        .unwrap();
        let pic = ed.picture();
        // 300 × 200 pixel a 254 DPI: 30 × 20 mm, più 3 mm per lato.
        let outline = rounded_rect(pic.trim_mm(), 2.0);
        let die = NewDie {
            name: "Taglio".into(),
            cmyk: [0.0, 1.0, 0.0, 0.0],
            width: 0.1,
            overprint: true,
        };
        let out = dir.join("prova.pdf");
        write_pdf(pic, Some(&[outline]), &die, false, &out).unwrap();
        let probe = winfiles_formats::probe(&out, winfiles_model::FileKind::Pdf).unwrap();
        let page = probe.pages.first().unwrap();
        assert!(
            (page.size.w - 36.0).abs() < 0.05 && (page.size.h - 26.0).abs() < 0.05,
            "{page:?}"
        );
        let trim = page.trim.unwrap();
        assert!(
            (trim.w - 30.0).abs() < 0.05 && (trim.h - 20.0).abs() < 0.05,
            "{trim:?}"
        );
        let file = PdfFile::open(&out).unwrap();
        let scan = file.scan_page(0).unwrap();
        let check = crate::pdf::tracciati::check(&scan, &crate::pdf::tracciati::default_inks());
        assert!(
            check
                .dies
                .iter()
                .any(|(name, role, _)| name == "Taglio" && *role == Role::Cut)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
