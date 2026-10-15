//! La calibrazione della stampante: una pagina con righelli, cornice e quadrato da stampare al
//! 100% e misurare con un righello, e la finestra dove scrivere le misure. Dalle misure viene la
//! correzione (`winfiles_geom::Calibration`) che la stampa applica a ogni foglio per quella
//! stampante. Dove va ogni segno lo decide `winfiles_layout::calibration`.

use std::collections::BTreeMap;

use craft_print::{Duplex, Job, Printer, Scaling};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, vec2};
use winfiles_geom::{Calibration, Measures, Paper, format_number};
use winfiles_layout::calibration::{CalibrationSheet, EDGE_MARKS, INSET, Ruler, Side};
use winfiles_scene::{Page, Weight};

use crate::app::{dim_label, segmented};
use crate::canvas::{INK, INK_SOFT};
use crate::fluent;
use crate::sketch::{PX_PER_MM, Sketch, font};
use crate::theme::{Tokens, semibold};

/// Spessore delle linee da misurare, in mm: sottili, per misurare preciso.
const LINE: f64 = 0.2;
/// Corpo del testo delle istruzioni, in mm (circa 9 pt).
const BODY: f64 = 3.2;
/// Corpo dei numeri sui righelli, in mm.
const TICK_TEXT: f64 = 2.2;

/// La correzione detta a parole: "+0,56% in larghezza, −0,20% in altezza, 0,8 mm a destra".
pub fn describe(c: &Calibration) -> String {
    if c.is_identity() {
        return "nessuna".into();
    }
    let pct = |s: f64| {
        let v = (s - 1.0) * 100.0;
        let sign = if v > 0.0 { "+" } else { "" };
        format!("{sign}{}%", format_number(v, 2, false))
    };
    let mut parts = Vec::new();
    if (c.scale_x - 1.0).abs() >= 5e-5 {
        parts.push(format!("{} in larghezza", pct(c.scale_x)));
    }
    if (c.scale_y - 1.0).abs() >= 5e-5 {
        parts.push(format!("{} in altezza", pct(c.scale_y)));
    }
    let shift = |v: f64, plus: &str, minus: &str| {
        (v.abs() >= 0.05).then(|| {
            format!(
                "{} mm {}",
                format_number(v.abs(), 1, false),
                if v > 0.0 { plus } else { minus }
            )
        })
    };
    parts.extend(shift(c.offset_x, "a destra", "a sinistra"));
    parts.extend(shift(c.offset_y, "in giù", "in su"));
    if parts.is_empty() {
        "nessuna".into()
    } else {
        parts.join(", ")
    }
}

/// Mette la correzione della stampante del lavoro sui fogli da stampare. Riducendo all'area
/// stampabile le misure non sono esatte comunque, e la correzione non si usa.
pub fn apply(pages: &mut [Page], job: &Job, calibrations: &BTreeMap<String, Calibration>) {
    if job.scaling != Scaling::Exact {
        return;
    }
    let Some(c) = job.printer.as_ref().and_then(|name| calibrations.get(name)) else {
        return;
    };
    for page in pages {
        page.calibration = *c;
    }
}

/// La scena della pagina di calibrazione. `printer` e `used` finiscono scritti sul foglio, per
/// sapere poi con che cosa è stata stampata.
pub fn compose(
    ctx: &egui::Context,
    sheet: &CalibrationSheet,
    printer: Option<&str>,
    used: &Calibration,
) -> Page {
    let mut sk = Sketch::new(ctx, sheet.size);
    let (w, h) = (sheet.size.w, sheet.size.h);

    // La cornice: linee singole centrate a INSET mm dai bordi, per misurare dal bordo del foglio.
    let f = sheet.frame;
    for (a, b) in [
        ((0.0, f.y), (w, f.y)),
        ((0.0, f.bottom()), (w, f.bottom())),
        ((f.x, 0.0), (f.x, h)),
        ((f.right(), 0.0), (f.right(), h)),
    ] {
        sk.line(a, b, INK_SOFT, LINE);
    }

    for marks in &sheet.edges {
        edge_marks(&mut sk, marks.side, marks.at, w, h);
    }
    ruler(&mut sk, &sheet.horizontal);
    ruler(&mut sk, &sheet.vertical);

    // Il quadrato con le diagonali e la croce al centro.
    let s = sheet.square;
    sk.rect(s, 0.0, None, Some((INK, LINE)));
    let (cx, cy) = (s.x + s.w / 2.0, s.y + s.h / 2.0);
    sk.line((s.x, s.y), (s.right(), s.bottom()), INK_SOFT, 0.1);
    sk.line((s.right(), s.y), (s.x, s.bottom()), INK_SOFT, 0.1);
    sk.line((cx - 5.0, cy), (cx + 5.0, cy), INK, LINE);
    sk.line((cx, cy - 5.0), (cx, cy + 5.0), INK, LINE);
    // La didascalia sopra il quadrato, lontano dalle diagonali.
    let side = format_number(s.w, 0, true);
    let label = sk.layout_line(&format!("Quadrato {side} × {side} mm"), BODY, Weight::SemiBold);
    let (lw, lh) = Sketch::size_mm(&label);
    sk.text((s.x, s.y - lh - 1.5), &label, BODY, Weight::SemiBold, INK, None);
    let hint = sk.layout_line("·  i quattro lati devono essere uguali", BODY, Weight::Regular);
    sk.text(
        (s.x + lw + 1.5, s.y - lh - 1.5),
        &hint,
        BODY,
        Weight::Regular,
        INK_SOFT,
        None,
    );

    // Titolo e istruzioni.
    let t = sheet.text;
    let title = sk.layout_line("Pagina di calibrazione", 6.0, Weight::SemiBold);
    sk.text((t.x, t.y), &title, 6.0, Weight::SemiBold, INK, None);
    let correction = if used.is_identity() {
        "stampata senza correzione".to_string()
    } else {
        format!("stampata con la correzione {}", describe(used))
    };
    let about = format!(
        "WinFiles · {} · {} al 100% · {correction}",
        printer.unwrap_or("nessuna stampante"),
        sheet.paper.name(),
    );
    let mut y = t.y + 9.0;
    y = paragraph(&mut sk, &about, (t.x, y), t.w, INK_SOFT) + 3.0;
    let mm = |v: f64| format_number(v, 0, true);
    let expected = sheet.expected();
    let steps = [
        "Nel driver della stampante la scala deve essere al 100%, senza «Adatta alla pagina».".to_string(),
        format!(
            "Misura con un righello la linea orizzontale, da una tacca lunga all'altra: deve essere {} mm.",
            mm(expected.width)
        ),
        format!("Misura allo stesso modo la linea verticale a sinistra: deve essere {} mm.", mm(expected.height)),
        format!(
            "Misura la distanza dal bordo sinistro del foglio alla cornice, poi dal bordo in alto alla cornice: deve essere {} mm.",
            mm(INSET)
        ),
        "Scrivi le quattro misure in WinFiles, nel menu File › Calibra la stampante, e salva la correzione.".to_string(),
        "Al centro di ogni bordo le tacche segnano i millimetri dal bordo: la prima stampata dice quanto margine la stampante lascia bianco.".to_string(),
    ];
    for (i, step) in steps.iter().enumerate() {
        let n = sk.layout_line(&format!("{}.", i + 1), BODY, Weight::SemiBold);
        sk.text((t.x, y), &n, BODY, Weight::SemiBold, INK, None);
        y = paragraph(&mut sk, step, (t.x + 5.0, y), t.w - 5.0, INK) + 1.6;
    }
    sk.page
}

/// Un paragrafo che va a capo in `width` mm; restituisce dove finisce in basso.
fn paragraph(sk: &mut Sketch<'_>, text: &str, at: (f64, f64), width: f64, ink: Color32) -> f64 {
    let mut job = LayoutJob::single_section(
        text.to_string(),
        TextFormat {
            font_id: font(BODY, Weight::Regular),
            color: Color32::BLACK,
            ..Default::default()
        },
    );
    job.wrap.max_width = width as f32 * PX_PER_MM;
    let galley = sk.layout(job);
    let (_, height) = Sketch::size_mm(&galley);
    sk.text(at, &galley, BODY, Weight::Regular, ink, None);
    at.1 + height
}

/// Una linea da misurare con le tacche: piccole ogni mm, medie ogni 5, lunghe e numerate ogni
/// 10; alle due estremità una tacca lunga che attraversa la linea.
fn ruler(sk: &mut Sketch<'_>, r: &Ruler) {
    let (a, b) = (r.from, r.to());
    sk.line(a, b, INK, LINE);
    // Lungo la linea (u) e di traverso (v), verso il basso o verso destra.
    let at = |u: f64, v: f64| {
        if r.vertical {
            (a.0 + v, a.1 + u)
        } else {
            (a.0 + u, a.1 + v)
        }
    };
    let count = r.length.round().max(0.0) as u32;
    for k in 0..=count {
        let u = f64::from(k);
        let len = if k % 10 == 0 {
            4.0
        } else if k % 5 == 0 {
            2.5
        } else {
            1.5
        };
        sk.line(at(u, 0.0), at(u, len), INK, 0.1);
        if k % 10 == 0 && k > 0 && k < count {
            let label = sk.layout_line(&k.to_string(), TICK_TEXT, Weight::Regular);
            let (lw, lh) = Sketch::size_mm(&label);
            let pos = if r.vertical {
                (a.0 + 5.0, a.1 + u - lh / 2.0)
            } else {
                (a.0 + u - lw / 2.0, a.1 + 4.6)
            };
            sk.text(pos, &label, TICK_TEXT, Weight::Regular, INK_SOFT, None);
        }
    }
    for u in [0.0, r.length] {
        sk.line(at(u, -4.0), at(u, 6.0), INK, LINE);
    }
    // La lunghezza da trovare, in mezzo alla linea orizzontale o sopra la verticale.
    let text = format!("{} mm", format_number(r.length, 0, true));
    let label = sk.layout_line(&text, BODY, Weight::SemiBold);
    let (lw, lh) = Sketch::size_mm(&label);
    let pos = if r.vertical {
        (a.0 - lw / 2.0, a.1 - lh - 4.5)
    } else {
        (a.0 + r.length / 2.0 - lw / 2.0, a.1 - lh - 1.5)
    };
    sk.text(pos, &label, BODY, Weight::SemiBold, INK, None);
}

/// Le tacche dei margini su un lato: una ogni mm dal bordo, centrate in `at`.
fn edge_marks(sk: &mut Sketch<'_>, side: Side, at: f64, w: f64, h: f64) {
    for k in 1..EDGE_MARKS {
        let d = f64::from(k);
        let half = if k % 5 == 0 { 3.0 } else { 1.5 };
        let (from, to) = match side {
            Side::Top => ((at - half, d), (at + half, d)),
            Side::Bottom => ((at - half, h - d), (at + half, h - d)),
            Side::Left => ((d, at - half), (d, at + half)),
            Side::Right => ((w - d, at - half), (w - d, at + half)),
        };
        sk.line(from, to, INK, 0.1);
    }
    let label = sk.layout_line("5", TICK_TEXT, Weight::Regular);
    let (lw, lh) = Sketch::size_mm(&label);
    let pos = match side {
        Side::Top => (at + 4.0, 5.0 - lh / 2.0),
        Side::Bottom => (at + 4.0, h - 5.0 - lh / 2.0),
        Side::Left => (5.0 - lw / 2.0, at + 3.6),
        Side::Right => (w - 5.0 - lw / 2.0, at + 3.6),
    };
    sk.text(pos, &label, TICK_TEXT, Weight::Regular, INK_SOFT, None);
}

/// Cosa chiede la finestra all'app.
pub enum Action {
    /// Stampare la pagina di calibrazione.
    Print(Box<Page>, Job),
    /// Salvarla in un PDF per guardarla.
    Pdf(Box<Page>),
    /// Tenere questa correzione per la stampante.
    Save(String, Calibration),
    /// Togliere la correzione della stampante.
    Remove(String),
    Close,
}

/// La finestra "Calibra la stampante".
pub struct CalibrateDialog {
    pub printers: Vec<Printer>,
    pub printer: Option<String>,
    pub paper: Paper,
    /// Stampare (e quindi misurare) la pagina con la correzione già salvata, per controllarla.
    pub with_current: bool,
    /// Le misure scritte, in mm.
    pub measured: Measures,
    /// Il formato per cui valgono le misure scritte: cambiando formato si riparte dal previsto.
    measured_for: Paper,
}

impl CalibrateDialog {
    pub fn new(printer: Option<String>, paper: Paper) -> Self {
        let mut d = Self {
            printers: craft_print::printers(),
            printer,
            paper,
            with_current: true,
            measured: CalibrationSheet::new(paper).expected(),
            measured_for: paper,
        };
        let there = d
            .printer
            .as_ref()
            .is_some_and(|name| d.printers.iter().any(|p| &p.name == name));
        if !there {
            d.printer = d
                .printers
                .iter()
                .find(|p| p.default)
                .or(d.printers.first())
                .map(|p| p.name.clone());
        }
        d
    }

    /// La correzione salvata per la stampante scelta.
    fn current(&self, calibrations: &BTreeMap<String, Calibration>) -> Option<Calibration> {
        self.printer.as_ref().and_then(|n| calibrations.get(n)).copied()
    }

    /// La correzione con cui si stampa (e si è stampata) la pagina.
    fn used(&self, calibrations: &BTreeMap<String, Calibration>) -> Calibration {
        match self.current(calibrations) {
            Some(c) if self.with_current => c,
            _ => Calibration::IDENTITY,
        }
    }

    /// La pagina per la stampante e il formato scelti.
    pub fn page(&self, ctx: &egui::Context, calibrations: &BTreeMap<String, Calibration>) -> Page {
        let sheet = CalibrationSheet::new(self.paper);
        let used = self.used(calibrations);
        let mut page = compose(ctx, &sheet, self.printer.as_deref(), &used);
        page.calibration = used;
        page
    }

    /// Il lavoro per stampare la pagina: una copia, solo fronte, al 100%.
    fn job(&self) -> Job {
        Job {
            printer: self.printer.clone(),
            copies: 1,
            collate: true,
            duplex: Some(Duplex::Off),
            scaling: Scaling::Exact,
            grayscale: false,
            title: "WinFiles, pagina di calibrazione".into(),
            options: Vec::new(),
        }
    }

    /// La correzione nuova dalle misure scritte, o perché non si può.
    fn result(&self, calibrations: &BTreeMap<String, Calibration>) -> Result<Calibration, String> {
        let expected = CalibrationSheet::new(self.paper).expected();
        Calibration::from_measures(self.used(calibrations), expected, self.measured).map_err(|e| {
            let mut s = e.to_string();
            if let Some(first) = s.get(..1) {
                s = first.to_uppercase() + s.get(1..).unwrap_or("");
            }
            s
        })
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        calibrations: &BTreeMap<String, Calibration>,
        thumbs: &mut crate::thumbs::Thumbnails,
    ) -> Option<Action> {
        let t = Tokens::get(ctx);
        if self.measured_for != self.paper {
            self.measured = CalibrationSheet::new(self.paper).expected();
            self.measured_for = self.paper;
        }
        let page = self.page(ctx, calibrations);
        let current = self.current(calibrations);
        let mut action = None;
        let modal = egui::Modal::new(egui::Id::new("calibra")).show(ctx, |ui| {
            ui.set_width(720.0);
            ui.label(egui::RichText::new("Calibra la stampante").font(semibold(17.0)).color(t.text_strong));
            ui.add_space(4.0);
            dim_label(
                ui,
                "Si stampa una pagina con misure note, la si misura con un righello e WinFiles corregge le stampe successive su quella stampante.",
            );
            ui.add_space(12.0);
            // In una finestra bassa si scorre, e Chiudi resta sotto.
            let room = (ctx.content_rect().height() - 170.0).max(200.0);
            // L'altezza si dà da fuori: dentro la finestra centrata lo spazio sotto dipende da
            // dove sta la finestra, che dipende dalla sua altezza.
            let max = egui::Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), room));
            ui.scope_builder(egui::UiBuilder::new().max_rect(max), |ui| {
            egui::ScrollArea::vertical().max_height(room).auto_shrink([false, true]).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                preview(ui, &page, thumbs);
                ui.add_space(16.0);
                ui.vertical(|ui| {
                    egui::Grid::new("calibra-scelte")
                        .num_columns(2)
                        .spacing([14.0, 10.0])
                        .show(ui, |ui| {
                            dim_label(ui, "Stampante");
                            if self.printers.is_empty() {
                                ui.label(egui::RichText::new("Nessuna stampante trovata").color(t.warning));
                            } else {
                                egui::ComboBox::from_id_salt("calibra-stampante")
                                    .icon(fluent::combo_icon)
                                    .selected_text(self.printer.clone().unwrap_or_default())
                                    .width(300.0)
                                    .show_ui(ui, |ui| {
                                        for p in &self.printers {
                                            let on = self.printer.as_deref() == Some(p.name.as_str());
                                            if ui.selectable_label(on, &p.name).clicked() {
                                                self.printer = Some(p.name.clone());
                                            }
                                        }
                                    });
                            }
                            ui.end_row();

                            dim_label(ui, "Formato");
                            segmented(ui, "calibra-formato", &mut self.paper, &[(Paper::A4, "A4"), (Paper::A3, "A3")]);
                            ui.end_row();

                            dim_label(ui, "Correzione");
                            let text = current.map_or_else(|| "nessuna".to_string(), |c| describe(&c));
                            ui.add(egui::Label::new(egui::RichText::new(text).color(t.text_strong)).wrap());
                            ui.end_row();
                        });
                    ui.add_space(10.0);
                    step(ui, &t, "1", "Stampa la pagina");
                    ui.horizontal(|ui| {
                        let ready = self.printer.is_some();
                        let print = egui::Button::new(
                            egui::RichText::new("Stampa la pagina di calibrazione")
                                .font(semibold(14.0))
                                .color(t.on_primary),
                        )
                        .fill(t.primary)
                        .min_size(vec2(0.0, 34.0));
                        if ui.add_enabled(ready, print).clicked() {
                            action = Some(Action::Print(Box::new(page.clone()), self.job()));
                        }
                        if ui.button("Salva in PDF…").on_hover_text("Per guardarla: da un altro programma la scala potrebbe non essere 100%").clicked() {
                            let mut plain = page.clone();
                            plain.calibration = Calibration::IDENTITY;
                            action = Some(Action::Pdf(Box::new(plain)));
                        }
                    });
                    if current.is_some() {
                        ui.add(fluent::checkbox(
                            &mut self.with_current,
                            "Con la correzione attuale, per controllarla",
                        ));
                    }
                    ui.add_space(10.0);
                    step(ui, &t, "2", "Scrivi le misure prese con il righello");
                    let expected = CalibrationSheet::new(self.paper).expected();
                    egui::Grid::new("calibra-misure")
                        .num_columns(3)
                        .spacing([12.0, 8.0])
                        .show(ui, |ui| {
                            let rows: [(&str, &mut f64, f64); 4] = [
                                ("Linea orizzontale", &mut self.measured.width, expected.width),
                                ("Linea verticale", &mut self.measured.height, expected.height),
                                ("Dal bordo sinistro alla cornice", &mut self.measured.left, expected.left),
                                ("Dal bordo in alto alla cornice", &mut self.measured.top, expected.top),
                            ];
                            for (label, value, want) in rows {
                                dim_label(ui, label);
                                fluent::number(ui, 110.0, mm_field(value));
                                dim_label(ui, &format!("deve essere {} mm", format_number(want, 0, true)));
                                ui.end_row();
                            }
                        });
                    ui.add_space(10.0);
                    step(ui, &t, "3", "Salva la correzione");
                    let result = self.result(calibrations);
                    match &result {
                        Ok(c) => {
                            let same = current.is_some_and(|now| {
                                (now.scale_x - c.scale_x).abs() < 5e-5
                                    && (now.scale_y - c.scale_y).abs() < 5e-5
                                    && (now.offset_x - c.offset_x).abs() < 0.05
                                    && (now.offset_y - c.offset_y).abs() < 0.05
                            });
                            let text = if same {
                                "Le misure confermano la correzione attuale.".to_string()
                            } else if c.is_identity() {
                                "Con queste misure la stampante è esatta: nessuna correzione.".to_string()
                            } else {
                                format!("Con queste misure: {}.", describe(c))
                            };
                            ui.add(egui::Label::new(egui::RichText::new(text).color(t.text)).wrap());
                        }
                        Err(e) => {
                            ui.add(egui::Label::new(egui::RichText::new(e).color(t.warning)).wrap());
                        }
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let can_save = result.is_ok() && self.printer.is_some();
                        if ui
                            .add_enabled(can_save, egui::Button::new("Salva la correzione").min_size(vec2(0.0, 32.0)))
                            .clicked()
                            && let (Ok(c), Some(name)) = (&result, &self.printer)
                        {
                            action = Some(Action::Save(name.clone(), *c));
                            // La prossima pagina si stampa con la correzione nuova, per controllarla.
                            self.with_current = true;
                            self.measured = expected;
                        }
                        if current.is_some()
                            && ui.button("Togli la correzione").clicked()
                            && let Some(name) = &self.printer
                        {
                            action = Some(Action::Remove(name.clone()));
                        }
                    });
                });
            });
            });
            });
            ui.add_space(12.0);
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), 34.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui.add(egui::Button::new("Chiudi").min_size(vec2(90.0, 34.0))).clicked() {
                        action = Some(Action::Close);
                    }
                },
            );
        });
        if modal.should_close() && action.is_none() {
            action = Some(Action::Close);
        }
        action
    }
}

/// Il titolo di un passo, con il suo numero in un cerchio d'accento.
fn step(ui: &mut egui::Ui, t: &Tokens, n: &str, text: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 11.0, t.primary);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            n,
            semibold(12.5),
            t.on_primary,
        );
        ui.label(
            egui::RichText::new(text)
                .font(semibold(14.0))
                .color(t.text_strong),
        );
    });
    ui.add_space(4.0);
}

/// Il foglio in piccolo, come verrà stampato.
fn preview(ui: &mut egui::Ui, page: &Page, thumbs: &mut crate::thumbs::Thumbnails) {
    let t = Tokens::get(ui.ctx());
    let width = 210.0_f32;
    let scale = width / page.size.w as f32;
    let size = vec2(width, page.size.h as f32 * scale);
    // Un margine attorno, perché il bordo non finisca tagliato dallo scorrimento.
    let (outer, _) = ui.allocate_exact_size(size + vec2(4.0, 4.0), egui::Sense::hover());
    let rect = egui::Rect::from_center_size(outer.center(), size);
    let painter = ui.painter_at(outer);
    painter.rect(
        rect,
        2.0,
        Color32::WHITE,
        egui::Stroke::new(1.0, t.border),
        egui::StrokeKind::Outside,
    );
    crate::paint::page(&painter, rect.min, scale, page, thumbs);
}

/// Un campo in millimetri con un decimale e la virgola, come si scrive in italiano.
fn mm_field(value: &mut f64) -> egui::DragValue<'_> {
    egui::DragValue::new(value)
        .range(0.0..=500.0)
        .speed(0.1)
        .custom_formatter(|v, _| format!("{} mm", format_number(v, 1, false)))
        .custom_parser(|text| {
            let clean = text.trim().trim_end_matches("mm").trim().replace(',', ".");
            clean.parse::<f64>().ok()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_correzione_a_parole() {
        assert_eq!(describe(&Calibration::IDENTITY), "nessuna");
        let c = Calibration {
            scale_x: 1.0056,
            scale_y: 0.998,
            offset_x: 0.8,
            offset_y: -1.24,
        };
        assert_eq!(
            describe(&c),
            "+0,56% in larghezza, -0,20% in altezza, 0,8 mm a destra, 1,2 mm in su"
        );
    }

    #[test]
    fn la_correzione_va_solo_alla_sua_stampante_e_al_100() {
        let c = Calibration {
            scale_x: 1.01,
            ..Calibration::IDENTITY
        };
        let map = BTreeMap::from([("Ufficio".to_string(), c)]);
        let mut pages = vec![Page::default(), Page::default()];
        let mut job = Job {
            printer: Some("Plotter".into()),
            ..Job::default()
        };
        apply(&mut pages, &job, &map);
        assert!(pages.iter().all(|p| p.calibration.is_identity()));
        job.printer = Some("Ufficio".into());
        job.scaling = Scaling::ShrinkToFit;
        apply(&mut pages, &job, &map);
        assert!(pages.iter().all(|p| p.calibration.is_identity()));
        job.scaling = Scaling::Exact;
        apply(&mut pages, &job, &map);
        assert!(pages.iter().all(|p| p.calibration == c));
    }
}
