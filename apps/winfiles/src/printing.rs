//! La finestra Stampa: stampante, copie, fogli, fronte e retro, bianco e nero e scala. I fogli si
//! stampano alla misura vera (100%), come nel PDF; la riduzione all'area stampabile è una scelta.

use craft_print::{Duplex, Job, Printer, Scaling};
use egui::vec2;

use crate::app::{dim_label, segmented, stepper};
use crate::fluent;
use crate::theme::{Tokens, semibold};

/// Quali fogli stampare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range {
    All,
    Current,
    /// Dal foglio `from` al foglio `to`, contati da 1.
    Some,
}

/// Le scelte della finestra Stampa; restano per la stampa successiva.
#[derive(Clone, Debug)]
pub struct PrintDialog {
    pub printers: Vec<Printer>,
    /// La stampante scelta, per nome; `None` = la predefinita del sistema.
    pub printer: Option<String>,
    pub copies: u32,
    pub duplex: Duplex,
    pub gray: bool,
    pub fit: bool,
    pub range: Range,
    pub from: u32,
    pub to: u32,
}

impl Default for PrintDialog {
    fn default() -> Self {
        Self {
            printers: Vec::new(),
            printer: None,
            copies: 1,
            duplex: Duplex::Off,
            gray: false,
            fit: false,
            range: Range::All,
            from: 1,
            to: 1,
        }
    }
}

/// Cosa ha deciso chi stampa.
pub enum Answer {
    Print(Job, std::ops::RangeInclusive<usize>),
    Cancel,
}

impl PrintDialog {
    /// Rilegge le stampanti del sistema e tiene la scelta precedente se c'è ancora, altrimenti la
    /// predefinita.
    pub fn refresh(&mut self) {
        self.printers = craft_print::printers();
        let still_there = self
            .printer
            .as_ref()
            .is_some_and(|name| self.printers.iter().any(|p| &p.name == name));
        if !still_there {
            self.printer = self
                .printers
                .iter()
                .find(|p| p.default)
                .or(self.printers.first())
                .map(|p| p.name.clone());
        }
    }

    fn chosen(&self) -> Option<&Printer> {
        let name = self.printer.as_ref()?;
        self.printers.iter().find(|p| &p.name == name)
    }

    /// I fogli da stampare, da 0, su `count` fogli con `current` in anteprima.
    pub fn sheets(&self, count: usize, current: usize) -> Option<std::ops::RangeInclusive<usize>> {
        let last = count.checked_sub(1)?;
        Some(match self.range {
            Range::All => 0..=last,
            Range::Current => {
                let c = current.min(last);
                c..=c
            }
            Range::Some => {
                let a = (self.from.max(1) as usize - 1).min(last);
                let b = (self.to.max(1) as usize - 1).min(last);
                a.min(b)..=a.max(b)
            }
        })
    }

    /// Il lavoro per lo spooler.
    pub fn job(&self, title: &str) -> Job {
        let two_sided = self.chosen().and_then(|p| p.two_sided) != Some(false);
        Job {
            printer: self.printer.clone(),
            copies: self.copies.max(1),
            collate: true,
            duplex: Some(if two_sided { self.duplex } else { Duplex::Off }),
            scaling: if self.fit {
                Scaling::ShrinkToFit
            } else {
                Scaling::Exact
            },
            grayscale: self.gray,
            title: title.to_string(),
            options: Vec::new(),
        }
    }

    /// Mostra la finestra. `count` sono i fogli del lavoro, `current` quello in anteprima.
    pub fn show(&mut self, ctx: &egui::Context, title: &str, count: usize, current: usize) -> Option<Answer> {
        let t = Tokens::get(ctx);
        let mut answer = None;
        let modal = egui::Modal::new(egui::Id::new("stampa")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.label(egui::RichText::new("Stampa").font(semibold(17.0)).color(t.text_strong));
            ui.add_space(12.0);
            egui::Grid::new("stampa-scelte")
                .num_columns(2)
                .spacing([14.0, 12.0])
                .show(ui, |ui| {
                    dim_label(ui, "Stampante");
                    ui.vertical(|ui| {
                        if self.printers.is_empty() {
                            ui.label(
                                egui::RichText::new("Nessuna stampante trovata").color(t.warning),
                            );
                        } else {
                            let shown = self.printer.clone().unwrap_or_default();
                            egui::ComboBox::from_id_salt("stampante")
                                .icon(crate::fluent::combo_icon)
                                .selected_text(shown)
                                .width(300.0)
                                .show_ui(ui, |ui| {
                                    for p in &self.printers {
                                        let label = if p.default {
                                            format!("{} (predefinita)", p.name)
                                        } else {
                                            p.name.clone()
                                        };
                                        let on = self.printer.as_deref() == Some(p.name.as_str());
                                        if ui.selectable_label(on, label).clicked() {
                                            self.printer = Some(p.name.clone());
                                        }
                                    }
                                });
                        }
                        if craft_print::HAS_PRINTER_PREFERENCES
                            && let Some(name) = self.printer.clone()
                            && ui
                                .link("Preferenze della stampante…")
                                .on_hover_text("Carta, cassetto e qualità nelle impostazioni del driver")
                                .clicked()
                        {
                            // Se il driver non apre la finestra non c'è altro da fare qui.
                            let _ = craft_print::open_printer_preferences(&name);
                        }
                    });
                    ui.end_row();

                    dim_label(ui, "Copie");
                    stepper(ui, &mut self.copies, 1, 99);
                    ui.end_row();

                    dim_label(ui, "Fogli");
                    ui.vertical(|ui| {
                        let choices = [
                            (Range::All, format!("Tutti ({count})")),
                            (Range::Current, format!("Solo quello in anteprima ({})", current + 1)),
                        ];
                        for (range, label) in choices {
                            if fluent::radio(ui, self.range == range, label).clicked() {
                                self.range = range;
                            }
                        }
                        ui.horizontal(|ui| {
                            if fluent::radio(ui, self.range == Range::Some, "Dal foglio").clicked() {
                                self.range = Range::Some;
                            }
                            let max = u32::try_from(count.max(1)).unwrap_or(u32::MAX);
                            let a = fluent::number(ui, 56.0, egui::DragValue::new(&mut self.from).range(1..=max));
                            ui.label("al");
                            let b = fluent::number(ui, 56.0, egui::DragValue::new(&mut self.to).range(1..=max));
                            if a.changed() || b.changed() {
                                self.range = Range::Some;
                            }
                        });
                    });
                    ui.end_row();

                    dim_label(ui, "Fronte e retro");
                    let two_sided = self.chosen().and_then(|p| p.two_sided) != Some(false);
                    ui.add_enabled_ui(two_sided, |ui| {
                        segmented(
                            ui,
                            "fronte-retro",
                            &mut self.duplex,
                            &[
                                (Duplex::Off, "No"),
                                (Duplex::LongEdge, "Lato lungo"),
                                (Duplex::ShortEdge, "Lato corto"),
                            ],
                        );
                    });
                    ui.end_row();

                    dim_label(ui, "Colore");
                    ui.add(fluent::checkbox(&mut self.gray, "Bianco e nero"));
                    ui.end_row();

                    dim_label(ui, "Scala");
                    ui.vertical(|ui| {
                        ui.add(fluent::checkbox(&mut self.fit, "Riduci all'area stampabile"));
                        let hint = if self.fit {
                            "I fogli si rimpiccioliscono quanto basta per i margini della stampante: le misure non sono più esatte."
                        } else {
                            "Al 100%: le misure sul foglio sono esatte. Se la stampante non ha il formato, la stampa viene rifiutata."
                        };
                        ui.add(
                            egui::Label::new(egui::RichText::new(hint).size(12.0).color(t.text_dim))
                                .wrap(),
                        );
                    });
                    ui.end_row();
                });
            ui.add_space(16.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 36.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                let ready = !self.printers.is_empty() && count > 0;
                let print = egui::Button::new(
                    egui::RichText::new("Stampa")
                        .font(semibold(14.0))
                        .color(t.on_primary),
                )
                .fill(t.primary)
                .min_size(vec2(120.0, 34.0));
                if ui.add_enabled(ready, print).clicked()
                    && let Some(range) = self.sheets(count, current)
                {
                    answer = Some(Answer::Print(self.job(title), range));
                }
                if ui
                    .add(egui::Button::new("Annulla").min_size(vec2(90.0, 34.0)))
                    .clicked()
                {
                    answer = Some(Answer::Cancel);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(Answer::Cancel);
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quali_fogli_si_stampano() {
        let mut d = PrintDialog::default();
        assert_eq!(d.sheets(5, 2), Some(0..=4));
        d.range = Range::Current;
        assert_eq!(d.sheets(5, 2), Some(2..=2));
        d.range = Range::Some;
        (d.from, d.to) = (4, 2);
        assert_eq!(d.sheets(5, 0), Some(1..=3));
        // Numeri oltre l'ultimo foglio si fermano all'ultimo; zero fogli, niente stampa.
        (d.from, d.to) = (3, 99);
        assert_eq!(d.sheets(5, 0), Some(2..=4));
        assert_eq!(d.sheets(0, 0), None);
    }

    #[test]
    fn il_lavoro_per_lo_spooler() {
        let mut d = PrintDialog {
            printers: vec![
                Printer {
                    name: "Ufficio".into(),
                    default: true,
                    two_sided: Some(false),
                },
                Printer {
                    name: "Plotter A3".into(),
                    default: false,
                    two_sided: Some(true),
                },
            ],
            printer: Some("Ufficio".into()),
            copies: 3,
            duplex: Duplex::LongEdge,
            gray: true,
            ..Default::default()
        };
        let job = d.job("Ordine 124");
        assert_eq!(job.printer.as_deref(), Some("Ufficio"));
        assert_eq!((job.copies, job.grayscale), (3, true));
        assert_eq!(job.scaling, Scaling::Exact);
        // Una stampante senza fronte e retro stampa solo fronte.
        assert_eq!(job.duplex, Some(Duplex::Off));
        d.printer = Some("Plotter A3".into());
        d.fit = true;
        let job = d.job("Ordine 124");
        assert_eq!(job.duplex, Some(Duplex::LongEdge));
        assert_eq!(job.scaling, Scaling::ShrinkToFit);
        assert_eq!(job.title, "Ordine 124");
    }
}
