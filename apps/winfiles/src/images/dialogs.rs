//! Le finestre del modulo immagini: ridimensiona (ricampiona davvero) e salva (formato,
//! qualità, profilo colore).

use egui::{Align, vec2};
use winfiles_geom::{Unit, format_number};
use winfiles_image_edit::{Format, SaveOptions};

use crate::fluent;
use crate::theme::{Tokens, semibold};

/// Quello che la finestra chiede di fare.
pub enum Action {
    Resize { width: u32, height: u32, dpi: f64 },
    Save { options: SaveOptions, ask: bool },
}

pub enum Dialog {
    Resize(Resize),
    Save(Save),
}

pub struct Resize {
    /// Misure in millimetri.
    pub width: String,
    pub height: String,
    pub dpi: String,
    pub keep_ratio: bool,
    /// Pixel dell'immagine di partenza, per le proporzioni.
    pub px: (u32, u32),
    pub unit: Unit,
}

pub struct Save {
    pub options: SaveOptions,
    /// Vero se il file su cui salvare c'è già: il pulsante dice "Salva", non "Scegli il file".
    pub has_target: bool,
    /// Vero se l'immagine ha parti trasparenti: solo allora il JPEG è un problema.
    pub has_alpha: bool,
    pub name: String,
}

impl Dialog {
    pub fn resize(px: (u32, u32), dpi: f64, unit: Unit) -> Self {
        let mm = |n: u32| f64::from(n) * 25.4 / dpi;
        Dialog::Resize(Resize {
            width: number(mm(px.0), unit),
            height: number(mm(px.1), unit),
            dpi: format_number(dpi, 0, true),
            keep_ratio: true,
            px,
            unit,
        })
    }

    pub fn save(options: SaveOptions, has_target: bool, has_alpha: bool, name: String) -> Self {
        Dialog::Save(Save {
            options,
            has_target,
            has_alpha,
            name,
        })
    }

    /// Disegna la finestra. `None` se resta aperta, `Some(azione)` quando si chiude.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Option<Action>> {
        match self {
            Dialog::Resize(r) => r.show(ctx),
            Dialog::Save(s) => s.show(ctx),
        }
    }
}

impl Resize {
    fn show(&mut self, ctx: &egui::Context) -> Option<Option<Action>> {
        let t = Tokens::get(ctx);
        let mut done = None;
        let unit = self.unit;
        let modal = egui::Modal::new(egui::Id::new("img-ridimensiona")).show(ctx, |ui| {
            ui.set_width(380.0);
            title(ui, "Ridimensiona l'immagine");
            ui.label(
                egui::RichText::new("I pixel si rifanno: ingrandire troppo fa perdere nitidezza.")
                    .color(t.text_dim),
            );
            ui.add_space(12.0);
            let ratio = f64::from(self.px.1) / f64::from(self.px.0).max(1.0);
            egui::Grid::new("misure")
                .num_columns(2)
                .spacing(vec2(10.0, 8.0))
                .show(ui, |ui| {
                    ui.label("Larghezza");
                    let w = field(ui, &mut self.width, unit.symbol());
                    ui.end_row();
                    ui.label("Altezza");
                    let h = field(ui, &mut self.height, unit.symbol());
                    ui.end_row();
                    ui.label("Risoluzione");
                    field(ui, &mut self.dpi, "DPI");
                    ui.end_row();
                    if let (true, Some(v)) = (self.keep_ratio && w.changed(), parse(&self.width)) {
                        self.height = number(v * unit.mm() * ratio, unit);
                    } else if let (true, Some(v)) = (self.keep_ratio && h.changed(), parse(&self.height)) {
                        self.width = number(v * unit.mm() / ratio.max(0.0001), unit);
                    }
                });
            ui.add_space(6.0);
            ui.add(fluent::checkbox(&mut self.keep_ratio, "Mantieni le proporzioni"));
            ui.add_space(10.0);
            let size = self.pixels();
            match size {
                Some((w, h)) => {
                    let grow = f64::from(w) / f64::from(self.px.0).max(1.0);
                    let mut text = format!(
                        "{} × {} pixel",
                        format_number(f64::from(w), 0, false),
                        format_number(f64::from(h), 0, false)
                    );
                    if grow > 1.05 {
                        text.push_str(&format!("  ·  ingrandita del {:.0}%", (grow - 1.0) * 100.0));
                    }
                    ui.label(egui::RichText::new(text).color(t.text_dim));
                }
                None => {
                    ui.label(egui::RichText::new("Misura non valida").color(t.error));
                }
            }
            ui.add_space(14.0);
            buttons(ui, "Ridimensiona", size.is_some(), &mut done, || {
                let (width, height) = size.unwrap_or((1, 1));
                Action::Resize {
                    width,
                    height,
                    dpi: parse(&self.dpi).unwrap_or(96.0),
                }
            });
        });
        if modal.should_close() {
            done = Some(None);
        }
        done
    }

    /// I pixel che vengono fuori dalle misure scritte.
    fn pixels(&self) -> Option<(u32, u32)> {
        let dpi = parse(&self.dpi).filter(|d| (1.0..=10_000.0).contains(d))?;
        let w = parse(&self.width)? * self.unit.mm();
        let h = parse(&self.height)? * self.unit.mm();
        let px = |mm: f64| (mm / 25.4 * dpi).round();
        let (w, h) = (px(w), px(h));
        (w >= 1.0 && h >= 1.0 && w <= 200_000.0 && h <= 200_000.0).then_some((w as u32, h as u32))
    }
}

impl Save {
    fn show(&mut self, ctx: &egui::Context) -> Option<Option<Action>> {
        let t = Tokens::get(ctx);
        let mut done = None;
        let modal = egui::Modal::new(egui::Id::new("img-salva")).show(ctx, |ui| {
            ui.set_width(380.0);
            title(ui, "Salva l'immagine");
            ui.add_space(8.0);
            egui::Grid::new("salva")
                .num_columns(2)
                .spacing(vec2(10.0, 10.0))
                .show(ui, |ui| {
                    ui.label("Formato");
                    egui::ComboBox::from_id_salt("img-formato")
                        .icon(fluent::combo_icon)
                        .selected_text(self.options.format.label())
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for f in [Format::Jpeg, Format::Png, Format::Tiff, Format::WebP] {
                                ui.selectable_value(&mut self.options.format, f, f.label());
                            }
                        });
                    ui.end_row();
                    if self.options.format == Format::Jpeg {
                        ui.label("Qualità");
                        ui.add(
                            fluent::Slider::new(&mut self.options.quality, 40..=100)
                                .suffix("%")
                                .fixed_decimals(0),
                        );
                        ui.end_row();
                    }
                });
            ui.add_space(8.0);
            ui.add(fluent::checkbox(
                &mut self.options.keep_icc,
                "Mantieni il profilo colore",
            ));
            let mut notes: Vec<&str> = Vec::new();
            if self.has_alpha && !self.options.format.keeps_alpha() {
                notes.push("Il JPEG non tiene la trasparenza: diventa bianca.");
            }
            if !self.options.format.keeps_dpi() {
                notes.push("Il WebP non porta con sé la risoluzione.");
            }
            if !notes.is_empty() {
                ui.add_space(8.0);
                for n in notes {
                    ui.label(egui::RichText::new(n).size(12.5).color(t.text_dim));
                }
            }
            ui.add_space(14.0);
            let label = if self.has_target {
                "Salva"
            } else {
                "Scegli il file…"
            };
            let options = self.options;
            let ask = !self.has_target;
            buttons(ui, label, true, &mut done, move || Action::Save { options, ask });
            if !self.has_target && !self.name.is_empty() {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!("Nome proposto: {}", self.name))
                        .size(12.0)
                        .color(t.text_dim),
                );
            }
        });
        if modal.should_close() {
            done = Some(None);
        }
        done
    }
}

fn title(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(
        egui::RichText::new(text)
            .font(semibold(15.0))
            .color(t.text_strong),
    );
    ui.add_space(6.0);
}

fn field(ui: &mut egui::Ui, value: &mut String, suffix: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let mut resp = None;
    ui.horizontal(|ui| {
        resp = Some(fluent::text_field(
            ui,
            egui::TextEdit::singleline(value).desired_width(90.0),
        ));
        ui.label(egui::RichText::new(suffix).color(t.text_dim));
    });
    resp.unwrap_or_else(|| ui.label(""))
}

fn buttons(
    ui: &mut egui::Ui,
    ok: &str,
    enabled: bool,
    done: &mut Option<Option<Action>>,
    action: impl FnOnce() -> Action,
) {
    let t = Tokens::get(ui.ctx());
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 34.0),
        egui::Layout::right_to_left(Align::Center),
        |ui| {
            let button = egui::Button::new(egui::RichText::new(ok).font(semibold(13.0)).color(if enabled {
                t.on_primary
            } else {
                t.text_disabled
            }))
            .fill(if enabled { t.primary } else { t.button })
            .min_size(vec2(120.0, 30.0));
            if ui.add_enabled(enabled, button).clicked() {
                *done = Some(Some(action()));
            }
            if ui
                .add(egui::Button::new("Annulla").min_size(vec2(90.0, 30.0)))
                .clicked()
            {
                *done = Some(None);
            }
        },
    );
}

/// Un numero scritto all'italiana: `12,5`.
pub fn number(mm: f64, unit: Unit) -> String {
    format_number(mm / unit.mm(), unit.decimals(), true)
}

/// Legge un numero scritto a mano, con la virgola o con il punto.
pub fn parse(text: &str) -> Option<f64> {
    let cleaned: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '.')
        .map(|c| if c == ',' { '.' } else { c })
        .collect();
    cleaned.parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legge_i_numeri_all_italiana() {
        assert_eq!(parse("12,5"), Some(12.5));
        assert_eq!(parse("12.5"), Some(125.0), "il punto separa le migliaia");
        assert_eq!(parse(" 300 "), Some(300.0));
        assert_eq!(parse(""), None);
        assert_eq!(parse("-3"), None);
        assert_eq!(parse("abc"), None);
    }

    #[test]
    fn i_pixel_vengono_dalle_misure() {
        let r = Resize {
            width: "100".into(),
            height: "50".into(),
            dpi: "300".into(),
            keep_ratio: true,
            px: (1181, 591),
            unit: Unit::Mm,
        };
        assert_eq!(r.pixels(), Some((1181, 591)));
        let bad = Resize {
            width: "0".into(),
            ..r
        };
        assert_eq!(bad.pixels(), None);
    }
}
