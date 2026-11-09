//! La fustella dal contorno della grafica, nello strumento Tracciati (ADR 0028, fase 3): la
//! pagina si disegna in pixel e passa dallo stesso motore delle immagini, così vale per le foto,
//! per i vettoriali e per i misti. Il contorno si vede sulla pagina prima di crearlo.

use std::sync::mpsc::{Receiver, channel};

use egui::{Color32, Pos2, Rect, Stroke};
use winfiles_geom::format_number;
use winfiles_image_edit::{Background, ContourSpec};
use winfiles_pdf_edit::Snapshot;

use crate::theme::{Tokens, semibold};

/// Lato lungo, in pixel, della pagina disegnata per cercare il contorno: lo stesso su cui lavora
/// il motore delle immagini (su un metro, mezzo millimetro).
const RENDER_SIDE: u32 = 2000;

type Contour = Vec<[f64; 2]>;
type Traced = Result<Vec<Contour>, String>;

/// Le scelte del contorno.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Il colore dello sfondo (di solito il bianco della carta).
    pub background: [u8; 3],
    pub tolerance: f64,
    /// Di quanto la fustella sta fuori dalla grafica (negativo: dentro), in mm.
    pub offset: f64,
    pub smooth: f64,
    pub holes: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            background: [255, 255, 255],
            tolerance: 10.0,
            offset: 2.0,
            smooth: 0.0,
            holes: false,
        }
    }
}

impl Settings {
    fn spec(&self) -> ContourSpec {
        ContourSpec {
            background: Background::Color(self.background),
            tolerance: self.tolerance.clamp(0.0, 100.0),
            offset: self.offset.clamp(-20.0, 50.0),
            smooth: self.smooth.clamp(0.0, 20.0),
            min_area: 4.0,
            holes: self.holes,
        }
    }
}

/// Lo stato: le scelte, il contorno calcolato per quale pagina e il calcolo in corso.
#[derive(Default)]
pub struct OutlineDie {
    /// Il contorno si calcola e si vede sulla pagina.
    pub on: bool,
    pub settings: Settings,
    /// La chiave della pagina mostrata adesso (versione, pagina, scelte).
    key: String,
    done: Option<(String, Traced)>,
    running: Option<(String, Receiver<Traced>)>,
}

impl OutlineDie {
    fn key_for(&self, page: (u64, usize)) -> String {
        format!("{}:{}:{:?}", page.0, page.1, self.settings)
    }

    /// Raccoglie il calcolo finito e, se la pagina o le scelte sono cambiate, ne chiede un altro.
    /// `width_mm` è la larghezza della pagina come si vede.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        page: (u64, usize),
        width_mm: f64,
        doc: impl FnOnce() -> Snapshot,
    ) {
        if let Some((k, rx)) = &self.running
            && let Ok(r) = rx.try_recv()
        {
            self.done = Some((k.clone(), r));
            self.running = None;
        }
        self.key = self.key_for(page);
        if !self.on || self.running.is_some() {
            return;
        }
        if self.done.as_ref().is_some_and(|(k, _)| *k == self.key) {
            return;
        }
        let snap = doc();
        let spec = self.settings.spec();
        let (tx, rx) = channel();
        let wake = ctx.clone();
        let index = page.1;
        let spawned = std::thread::Builder::new()
            .name("contorno-pagina".into())
            .spawn(move || {
                let _ = tx.send(trace_page(&snap, index, width_mm, &spec));
                wake.request_repaint();
            });
        if spawned.is_ok() {
            self.running = Some((self.key.clone(), rx));
        }
    }

    /// Il contorno della pagina di adesso, se è pronto.
    fn current(&self) -> Option<&Traced> {
        self.done.as_ref().filter(|(k, _)| *k == self.key).map(|(_, r)| r)
    }

    /// I contorni pronti per la pagina di adesso.
    pub fn outlines(&self) -> Option<&[Contour]> {
        self.current().and_then(|r| r.as_ref().ok()).map(Vec::as_slice)
    }

    /// La riga di stato sotto le scelte: quanti contorni, l'errore, o il calcolo in corso.
    pub fn status(&self) -> (String, bool) {
        match self.current() {
            Some(Ok(c)) => {
                let n = c.len();
                (
                    format!(
                        "{n} {} a {} dalla grafica.",
                        if n == 1 { "contorno" } else { "contorni" },
                        format_mm(self.settings.offset)
                    ),
                    false,
                )
            }
            Some(Err(e)) => (e.clone(), true),
            None => ("Calcolo del contorno…".into(), false),
        }
    }

    /// Il contorno sulla pagina a schermo (`paper`, larga `w` mm), nel colore della fustella.
    pub fn paint(&self, ui: &egui::Ui, paper: Rect, w: f32, color: Color32) {
        if !self.on {
            return;
        }
        let Some(lines) = self.outlines() else {
            return;
        };
        let per_mm = paper.width() / w.max(0.001);
        let to = |p: [f64; 2]| -> Pos2 {
            egui::pos2(
                paper.left() + p[0] as f32 * per_mm,
                paper.top() + p[1] as f32 * per_mm,
            )
        };
        let painter = ui.painter().with_clip_rect(paper.expand(40.0));
        for line in lines {
            let pts: Vec<Pos2> = line.iter().map(|p| to(*p)).collect();
            painter.add(egui::Shape::closed_line(
                pts.clone(),
                Stroke::new(3.0, Color32::from_white_alpha(170)),
            ));
            painter.add(egui::Shape::closed_line(pts, Stroke::new(1.5, color)));
        }
    }

    /// Le scelte del contorno, dentro la scheda dello strumento.
    pub fn controls(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let s = &mut self.settings;
        let label = |ui: &mut egui::Ui, text: &str| {
            ui.label(egui::RichText::new(text).color(t.text_dim));
        };
        ui.horizontal(|ui| {
            label(ui, "Distanza");
            crate::fluent::number(
                ui,
                90.0,
                egui::DragValue::new(&mut s.offset)
                    .range(-20.0..=50.0)
                    .speed(0.1)
                    .custom_formatter(|v, _| format_mm(v))
                    .custom_parser(parse_mm),
            )
            .on_hover_text("Quanto la fustella sta fuori dalla grafica; negativa: dentro");
        });
        ui.add_space(2.0);
        label(ui, "Lisciatura");
        {
            ui.add(
                crate::fluent::Slider::new(&mut s.smooth, 0.0..=20.0)
                    .step_by(0.5)
                    .custom_formatter(format_mm)
                    .custom_parser(parse_mm),
            )
            .on_hover_text("Riempie le rientranze strette e arrotonda le punte, senza entrare nella grafica");
        }
        ui.add_space(2.0);
        label(ui, "Tolleranza");
        {
            ui.add(
                crate::fluent::Slider::new(&mut s.tolerance, 0.0..=60.0)
                    .suffix(" %")
                    .fixed_decimals(0),
            )
            .on_hover_text("Più alta, più colori vicini allo sfondo restano fuori (ombre, sfumature)");
        }
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            label(ui, "Sfondo");
            let swatch = ui
                .color_edit_button_srgb(&mut s.background)
                .on_hover_text("Il colore dello sfondo da lasciare fuori");
            ui.painter().rect_stroke(
                swatch.rect,
                2.0,
                Stroke::new(1.0, t.field_border),
                egui::StrokeKind::Outside,
            );
            if s.background != [255, 255, 255]
                && ui
                    .button("Bianco")
                    .on_hover_text("Il bianco della carta")
                    .clicked()
            {
                s.background = [255, 255, 255];
            }
        });
        ui.add_space(4.0);
        ui.add(crate::fluent::checkbox(
            &mut s.holes,
            "Taglia anche i fori interni",
        ))
        .on_hover_text("Il centro di una O, gli spazi chiusi del colore dello sfondo");
        ui.add_space(2.0);
        let (text, bad) = self.status();
        let color = if bad { t.warning } else { t.text_dim };
        ui.add(egui::Label::new(egui::RichText::new(text).size(12.0).color(color)).wrap());
    }
}

/// Il titolo della scheda.
pub fn title(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(
        egui::RichText::new(text)
            .font(semibold(13.0))
            .color(t.text_strong),
    );
}

fn format_mm(v: f64) -> String {
    format!("{} mm", format_number(v, 1, true))
}

fn parse_mm(text: &str) -> Option<f64> {
    let clean = text.trim().trim_end_matches("mm").trim().replace(',', ".");
    clean.parse::<f64>().ok()
}

/// Disegna la pagina `index` e ne cerca il contorno: contorni in mm della pagina come si vede.
fn trace_page(snap: &Snapshot, index: usize, width_mm: f64, spec: &ContourSpec) -> Traced {
    let bytes = snap.preview_bytes().map_err(|e| e.to_string())?;
    let pages = winfiles_thumbs::PdfPages::open(bytes).map_err(|e| e.to_string())?;
    let thumb = pages.render(index, RENDER_SIDE).map_err(|e| e.to_string())?;
    let width = thumb.width;
    let img = image::RgbaImage::from_raw(thumb.width, thumb.height, thumb.rgba)
        .ok_or_else(|| "pagina non disegnabile".to_string())?;
    if width == 0 || !(width_mm.is_finite() && width_mm > 0.0) {
        return Err("pagina senza misura".into());
    }
    let mm_per_px = width_mm / f64::from(width);
    winfiles_image_edit::trace_image(&image::DynamicImage::ImageRgba8(img), mm_per_px, spec).map_err(|e| {
        let text = e.to_string();
        let mut c = text.chars();
        c.next()
            .map(|f| f.to_uppercase().chain(c).collect::<String>() + ".")
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un PDF di una pagina `w` × `h` mm con il contenuto `content`.
    fn one_page_pdf(w: f64, h: f64, content: &str) -> Vec<u8> {
        let mm = 72.0 / 25.4;
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents 4 0 R >>",
                w * mm,
                h * mm
            ),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offs = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let x = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
        for o in offs {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n",
                objs.len() + 1
            )
            .bytes(),
        );
        out
    }

    #[test]
    fn il_contorno_della_pagina_segue_la_grafica() {
        // Una pagina 100 × 80 mm con un quadrato nero di 40 mm a 20, 20 mm dall'angolo in alto.
        let mm = 72.0 / 25.4;
        let content = format!(
            "0 0 0 rg {} {} {} {} re f",
            20.0 * mm,
            (80.0 - 60.0) * mm,
            40.0 * mm,
            40.0 * mm
        );
        let pdf = one_page_pdf(100.0, 80.0, &content);
        let file = winfiles_pdf_edit::PdfFile::from_bytes(pdf).unwrap();
        let editor = winfiles_pdf_edit::Editor::new(file, "prova.pdf".into());
        let spec = Settings {
            offset: 0.0,
            ..Settings::default()
        }
        .spec();
        let c = trace_page(&editor.snapshot(), 0, 100.0, &spec).unwrap();
        assert_eq!(c.len(), 1);
        let b = c
            .iter()
            .flatten()
            .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| {
                [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]
            });
        for (got, want) in b.iter().zip([20.0, 20.0, 60.0, 60.0]) {
            assert!((got - want).abs() < 0.5, "{b:?}");
        }
    }
}
