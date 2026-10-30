//! Fustella e abbondanza sulle immagini (ADR 0028): il pannello a destra di Modifica immagini.
//! L'abbondanza si fa con i pixel (`Op::Bleed`, che si annulla come le altre modifiche); la
//! fustella, rettangolare o con gli angoli tondi sul formato finito, si scrive in tinta piatta nel
//! PDF che si salva da qui, con l'immagine alla sua risoluzione, TrimBox e BleedBox.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use egui::{Color32, Pos2, Rect, Stroke, pos2};
use serde::{Deserialize, Serialize};
use winfiles_geom::{Unit, format_length, format_size};
use winfiles_image_edit::{BleedMethod, Format, Op, Picture, SaveOptions};
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
}

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
}

impl Default for DieSettings {
    fn default() -> Self {
        Self {
            amount: 3.0,
            mirror: true,
            shape: DieShape::Rect,
            radius: 5.0,
        }
    }
}

impl DieSettings {
    pub fn sanitized(mut self) -> Self {
        let fix = |v: f64, lo: f64, hi: f64, def: f64| if v.is_finite() { v.clamp(lo, hi) } else { def };
        self.amount = fix(self.amount, 0.5, 100.0, 3.0);
        self.radius = fix(self.radius, 0.0, 500.0, 5.0);
        self
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

#[derive(Default)]
pub struct DieTool {
    pub open: bool,
    pub settings: DieSettings,
    saving: Option<mpsc::Receiver<Result<PathBuf, String>>>,
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

    /// La fustella in mm dell'immagine (dall'angolo in alto a sinistra), se se ne vuole una.
    pub fn outline(&self, picture: &Picture) -> Option<Vec<[f64; 2]>> {
        let t = picture.trim_mm();
        match self.settings.shape {
            DieShape::None => None,
            DieShape::Rect => Some(rounded_rect(t, 0.0)),
            DieShape::Rounded => Some(rounded_rect(t, self.settings.radius)),
        }
    }

    pub fn panel(
        &mut self,
        ui: &mut egui::Ui,
        picture: &Picture,
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
        ui.add_space(12.0);

        heading(ui, "Fustella");
        segmented(
            ui,
            "forma-fustella-immagine",
            &mut s.shape,
            &[
                (DieShape::Rect, "Rettangolo"),
                (DieShape::Rounded, "Angoli tondi"),
                (DieShape::None, "Nessuna"),
            ],
        );
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
        let ink = cut_ink(inks);
        if s.shape != DieShape::None {
            hint(
                ui,
                &format!(
                    "Sul formato finito, in tinta piatta «{}» in sovrastampa, linea da 0,1 mm.",
                    ink.name
                ),
            );
        }
        ui.add_space(14.0);
        let save = egui::Button::image_and_text(
            crate::icons::image("save", 18.0, t.on_primary),
            egui::RichText::new("Salva PDF con fustella…")
                .font(semibold(14.0))
                .color(t.on_primary),
        )
        .fill(t.primary)
        .min_size(egui::vec2(0.0, 34.0));
        if ui
            .add_enabled(!self.busy(), save)
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
        let outline = self.outline(&picture);
        let ink = cut_ink(inks);
        let die = NewDie {
            name: ink.name.clone(),
            cmyk: ink.color(),
            width: 0.1,
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
    pub fn paint(&self, ui: &egui::Ui, paper: Rect, picture: &Picture, inks: &[InkName]) {
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
        if let Some(line) = self.outline(picture) {
            let pts: Vec<Pos2> = line.iter().map(|p| to(*p)).collect();
            let c = cut_ink(inks).color();
            let rgb =
                |v: f64, kk: f64| ((1.0 - v.clamp(0.0, 1.0)) * (1.0 - kk.clamp(0.0, 1.0)) * 255.0) as u8;
            let color = Color32::from_rgb(rgb(c[0], c[3]), rgb(c[1], c[3]), rgb(c[2], c[3]));
            painter.add(egui::Shape::closed_line(pts, Stroke::new(1.5, color)));
        }
    }
}

/// La tinta del taglio: la prima con il ruolo Taglio, o «Taglio».
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
    outline: Option<&[[f64; 2]]>,
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
    if let Some(line) = outline {
        file.draw_die(0, &[line.to_vec()], die)
            .map_err(|e| e.to_string())?;
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
        write_pdf(pic, Some(&outline), &die, false, &out).unwrap();
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
