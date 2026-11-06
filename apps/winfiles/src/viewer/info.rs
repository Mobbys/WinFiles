//! I pannelli a destra della scheda Visualizza: misure e informazioni, tinte piatte, layer CAD.

use egui::{Color32, Sense, Stroke, vec2};
use winfiles_geom::{Size, Unit, area_m2, format_area, format_length, format_number, format_size};
use winfiles_pdf_edit::Overprint;

use super::load::{Facts, ViewKind};
use super::{ViewerModule, file_name, ink_is, measures_text};
use crate::pdf::tracciati::Role;
use crate::theme::{Tokens, semibold};

/// Le unità del disegno da scegliere quando il file non le dichiara.
const CAD_UNITS: [(f64, &str); 5] = [
    (1.0, "millimetri"),
    (10.0, "centimetri"),
    (1000.0, "metri"),
    (25.4, "pollici"),
    (304.8, "piedi"),
];

fn heading(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(10.0);
    ui.label(egui::RichText::new(text).font(semibold(13.5)).color(t.text_strong));
    ui.add_space(2.0);
}

/// Una riga "nome: valore", con il valore che va a capo.
fn row(ui: &mut egui::Ui, name: &str, value: &str) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(96.0, 18.0), Sense::hover());
        ui.painter().text(
            r.left_top() + vec2(0.0, 1.0),
            egui::Align2::LEFT_TOP,
            name,
            egui::FontId::proportional(12.5),
            t.text_dim,
        );
        ui.add(egui::Label::new(egui::RichText::new(value).size(12.5).color(t.text_strong)).wrap());
    });
}

fn date_text(time: std::time::SystemTime) -> Option<String> {
    let secs = time.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let days = i64::try_from(secs / 86_400).ok()?;
    // Da giorni dal 1970 a data civile (Howard Hinnant), in UTC.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let rest = secs % 86_400;
    Some(format!("{d:02}/{m:02}/{y} {:02}:{:02} UTC", rest / 3600, rest / 60 % 60))
}

fn bytes_text(n: u64) -> String {
    let n = n as f64;
    if n >= 1_048_576.0 {
        format!("{} MB", format_number(n / 1_048_576.0, 1, false))
    } else if n >= 1024.0 {
        format!("{} KB", format_number(n / 1024.0, 0, false))
    } else {
        format!("{n} byte")
    }
}

fn overprint_text(o: Overprint) -> &'static str {
    match o {
        Overprint::All => "in sovrastampa",
        Overprint::Some => "sovrastampa solo in parte",
        Overprint::None => "senza sovrastampa",
        Overprint::Unused => "definita ma non usata",
    }
}

impl ViewerModule {
    pub(super) fn info_panel(&mut self, ui: &mut egui::Ui, unit: Unit) {
        let t = Tokens::get(ui.ctx());
        let Some(d) = self.doc() else {
            return;
        };
        let name = file_name(&d.path);
        ui.add(egui::Label::new(egui::RichText::new(&name).font(semibold(14.0)).color(t.text_strong)).wrap());
        ui.label(egui::RichText::new(d.kind.label()).size(12.0).color(t.text_dim));
        let Some(o) = d.opened() else {
            return;
        };
        let page = o.pages.get(d.page).copied();
        let mut copy = None;
        if let Some(p) = page {
            heading(ui, if o.pages.len() > 1 { "Misure della pagina" } else { "Misure" });
            let cad = matches!(o.facts, Facts::Cad(_));
            if cad {
                if let Facts::Cad(f) = &o.facts {
                    let (w, h) = f.drawing.size_mm(f.unit_mm);
                    let s = Size::new(w, h);
                    row(ui, "Ingombro", &format_size(s, unit));
                    row(ui, "Area", &format_area(area_m2(s)));
                    if (f.page_scale - f.unit_mm).abs() > 1e-9 {
                        let ratio = f.unit_mm / f.page_scale;
                        row(ui, "Anteprima", &format!("ridotta 1:{}", format_number(ratio, 1, true)));
                    }
                }
            } else {
                row(ui, "Formato", &format_size(p.size, unit));
                if let Some(tr) = p.trim {
                    let fin = Size::new(tr[2], tr[3]);
                    row(ui, "Al vivo", &format_size(fin, unit));
                    let sides = [
                        tr[0],
                        tr[1],
                        p.size.w - tr[0] - tr[2],
                        p.size.h - tr[1] - tr[3],
                    ];
                    if sides.iter().any(|s| *s > 0.05) {
                        let all_same = sides.iter().all(|s| (s - sides[0]).abs() < 0.05);
                        let text = if all_same {
                            format_length(sides[0], unit)
                        } else {
                            format!(
                                "sx {} · su {} · dx {} · giù {}",
                                format_length(sides[0], unit),
                                format_length(sides[1], unit),
                                format_length(sides[2], unit),
                                format_length(sides[3], unit)
                            )
                        };
                        row(ui, "Abbondanza", &text);
                    } else {
                        row(ui, "Abbondanza", "nessuna");
                    }
                    row(ui, "Area al vivo", &format_area(area_m2(fin)));
                    row(ui, "Area con abb.", &format_area(area_m2(p.size)));
                } else {
                    row(ui, "Area", &format_area(area_m2(p.size)));
                }
                if o.pages.len() > 1 {
                    let total: f64 = o.pages.iter().map(|p| area_m2(p.finished())).sum();
                    row(ui, "Tutte le pagine", &format!("{} pagine · {}", o.pages.len(), format_area(total)));
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add(egui::Button::image_and_text(
                        crate::icons::image("copy", 15.0, t.text_strong),
                        "Copia misure",
                    ))
                    .on_hover_text("Copia nome, formato, al vivo e m² per incollarli in un messaggio")
                    .clicked()
                {
                    copy = Some(measures_text(&name, &p, unit));
                }
                if p.trim.is_some() || p.bleed.is_some() {
                    ui.add(crate::fluent::checkbox(&mut self.show_boxes, "Riquadri"));
                }
            });
        }
        let Some(d) = self.doc() else {
            return;
        };
        let Some(o) = d.opened() else {
            return;
        };
        match &o.facts {
            Facts::Pdf { details, note, .. } => {
                heading(ui, "Documento");
                if let Some(n) = note {
                    row(ui, "Letto come", n);
                }
                row(ui, "Pagine", &o.pages.len().to_string());
                row(ui, "Versione", &format!("PDF {}", details.version));
                if let Some(x) = &details.pdfx {
                    row(ui, "PDF/X", x);
                }
                if let Some(x) = &details.output_intent {
                    row(ui, "Profilo uscita", x);
                }
                if let Some(x) = &details.creator {
                    row(ui, "Creato con", x);
                }
                if let Some(x) = &details.producer {
                    row(ui, "Scritto da", x);
                }
                if let Some(x) = &details.title {
                    row(ui, "Titolo", x);
                }
                if details.illustrator_data {
                    row(ui, "Illustrator", "contiene la copia modificabile per Illustrator");
                }
                if let Some(bp) = details.pages.get(d.page)
                    && bp.rotation != 0
                {
                    row(ui, "Rotazione", &format!("{}°", bp.rotation));
                }
            }
            Facts::Image(f) => {
                heading(ui, "Immagine");
                row(ui, "Pixel", &format!("{} × {}", f.width, f.height));
                let dpi = format_number(f.dpi, 0, true);
                row(
                    ui,
                    "Risoluzione",
                    &if f.dpi_from_file {
                        format!("{dpi} DPI")
                    } else {
                        format!("{dpi} DPI (il file non la dice)")
                    },
                );
                row(ui, "Colore", &format!("{} · {} bit", f.color.label(), f.bits));
                row(ui, "Profilo", f.icc.as_deref().unwrap_or("nessuno"));
                if let Some(x) = &f.origin {
                    row(ui, "Origine", x);
                }
                if let Some(p) = page {
                    // Quanto si può ingrandire tenendo almeno 150 DPI.
                    let mpx = f64::from(f.width) * f64::from(f.height) / 1e6;
                    row(ui, "Megapixel", &format_number(mpx, 1, true));
                    let at150 = Size::new(f64::from(f.width) / 150.0 * 25.4, f64::from(f.height) / 150.0 * 25.4);
                    if at150.w > p.size.w + 0.5 {
                        row(ui, "A 150 DPI", &format_size(at150, unit));
                    }
                }
            }
            Facts::Cad(f) => {
                heading(ui, "Disegno");
                row(ui, "Formato", &f.drawing.version);
                row(ui, "Unità", f.drawing.units.name);
                row(ui, "Layer", &f.drawing.layers.len().to_string());
                row(ui, "Oggetti", &f.drawing.shapes.len().to_string());
                if !f.drawing.skipped.is_empty() {
                    let s: Vec<String> = f.drawing.skipped.iter().map(|(k, n)| format!("{k} ({n})")).collect();
                    row(ui, "Non disegnati", &s.join(", "));
                }
                for n in f.drawing.notes.iter().take(5) {
                    row(ui, "Nota", n);
                }
            }
        }
        heading(ui, "File");
        if o.bytes > 0 {
            row(ui, "Peso", &bytes_text(o.bytes));
        }
        if let Some(x) = o.modified.and_then(date_text) {
            row(ui, "Modificato", &x);
        }
        if let Some(dir) = d.path.parent() {
            row(ui, "Cartella", &dir.display().to_string());
        }
        if let Some(text) = copy {
            ui.ctx().copy_text(text);
            self.status = "Misure copiate".into();
        }
    }

    pub(super) fn spots_panel(&mut self, ui: &mut egui::Ui, unit: Unit) {
        let t = Tokens::get(ui.ctx());
        let Some(d) = self.docs.get_mut(self.current) else {
            return;
        };
        let page = d.page;
        let spots = match d.opened().map(|o| &o.facts) {
            Some(Facts::Pdf { spots, .. }) => spots.clone(),
            _ => return,
        };
        let list = match spots {
            Ok(l) => l,
            Err(e) => {
                ui.label(egui::RichText::new(format!("Non riesco a leggere le tinte: {e}")).color(t.error));
                return;
            }
        };
        if list.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Nessuna tinta piatta").font(semibold(14.0)).color(t.text_strong));
            ui.label(
                egui::RichText::new("Il file usa solo quadricromia, scala di grigi o RGB.")
                    .size(12.5)
                    .color(t.text_dim),
            );
            return;
        }
        ui.label(
            egui::RichText::new("Clic su una tinta: il resto si sbiadisce e la tinta resta in evidenza (Esc per tornare).")
                .size(12.0)
                .color(t.text_dim),
        );
        ui.add_space(6.0);
        // Le lunghezze dei tracciati della pagina, per le tinte di taglio.
        if let Some(r) = &mut d.renderer {
            r.want_scan(page);
        }
        let scan = d.renderer.as_ref().and_then(|r| r.scans.get(&page)).and_then(|s| s.as_ref().ok()).cloned();
        let mut toggle = None;
        for s in &list {
            let on_page = s.pages.contains(&page);
            let selected = self.highlight.as_deref() == Some(s.raw.as_slice());
            let known = self.inks.iter().find(|i| i.name.eq_ignore_ascii_case(&s.name));
            let role = known.map_or_else(|| Role::guess(&s.name), |i| i.role);
            let length: f64 = scan.as_ref().map_or(0.0, |sc| {
                sc.paths
                    .iter()
                    .filter(|p| p.line.as_ref().is_some_and(|l| ink_is(&l.ink, &s.raw)))
                    .map(|p| p.length)
                    .sum()
            });
            let filled = scan.as_ref().is_some_and(|sc| {
                sc.paths.iter().any(|p| p.fill.as_ref().is_some_and(|f| ink_is(&f.ink, &s.raw)))
            });
            let fill = if selected { t.selection } else { t.panel_darker };
            let resp = egui::Frame::NONE
                .fill(fill)
                .corner_radius(t.radius)
                .stroke(Stroke::new(1.0, if selected { t.accent } else { t.border }))
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                        let c = s.swatch.map_or(Color32::GRAY, |[r, g, b]| Color32::from_rgb(r, g, b));
                        ui.painter().rect_filled(r, 4.0, c);
                        ui.painter().rect_stroke(r, 4.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                        ui.vertical(|ui| {
                            ui.add(
                                egui::Label::new(egui::RichText::new(&s.name).font(semibold(13.0)).color(t.text_strong))
                                    .wrap(),
                            );
                            let pages = if s.pages.is_empty() {
                                "nessuna pagina".to_string()
                            } else if s.pages.len() == 1 {
                                format!("pagina {}", s.pages.first().map_or(1, |p| p + 1))
                            } else {
                                format!("{} pagine", s.pages.len())
                            };
                            let mut info = format!("{pages} · {}", overprint_text(s.overprint));
                            if role != Role::Other {
                                info.push_str(&format!(" · {}", role.label()));
                            }
                            ui.label(egui::RichText::new(info).size(12.0).color(t.text));
                            if on_page && length > 0.0 {
                                let mut l = format!("Linee su questa pagina: {}", metres(length, unit));
                                if filled {
                                    l.push_str(" · anche aree piene");
                                }
                                ui.label(egui::RichText::new(l).size(12.0).color(t.text_dim));
                            }
                        });
                    });
                })
                .response
                .interact(Sense::click());
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                toggle = Some(s.raw.clone());
            }
            ui.add_space(6.0);
        }
        if let Some(raw) = toggle {
            if self.highlight.as_deref() == Some(raw.as_slice()) {
                self.highlight = None;
            } else {
                let pages = list.iter().find(|s| s.raw == raw).map(|s| s.pages.clone()).unwrap_or_default();
                // Se la tinta non è su questa pagina si va alla prima dove c'è.
                if !pages.contains(&page)
                    && let Some(first) = pages.first()
                    && let Some(d) = self.docs.get_mut(self.current)
                {
                    d.page = *first;
                }
                self.highlight = Some(raw);
            }
        }
    }

    pub(super) fn layers_panel(&mut self, ui: &mut egui::Ui, _unit: Unit) {
        let t = Tokens::get(ui.ctx());
        let Some(d) = self.doc() else {
            return;
        };
        if d.kind != ViewKind::Cad {
            return;
        }
        let Some(Facts::Cad(f)) = d.opened().map(|o| &o.facts) else {
            return;
        };
        let drawing = std::sync::Arc::clone(&f.drawing);
        let mut choice = d.cad.clone();
        let before = choice.clone();
        ui.add_space(4.0);
        let mut dark = choice.dark;
        if crate::fluent::toggle(ui, dark, "Fondo nero, come nei CAD").clicked() {
            dark = !dark;
        }
        choice.dark = dark;
        if drawing.units.mm.is_none() {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Il file non dice in che unità è disegnato: scegli tu, le misure cambiano di conseguenza.")
                    .size(12.0)
                    .color(t.warning),
            );
            let current = CAD_UNITS
                .iter()
                .find(|(mm, _)| (*mm - choice.unit_mm).abs() < 1e-9)
                .map_or("millimetri", |(_, n)| n);
            egui::ComboBox::from_id_salt("visualizza-unita-cad")
                .icon(crate::fluent::combo_icon)
                .selected_text(format!("Unità: {current}"))
                .width(200.0)
                .show_ui(ui, |ui| {
                    for (mm, name) in CAD_UNITS {
                        if ui.selectable_label((mm - choice.unit_mm).abs() < 1e-9, name).clicked() {
                            choice.unit_mm = mm;
                        }
                    }
                });
        }
        heading(ui, "Layer");
        ui.horizontal(|ui| {
            if ui.small_button("Mostra tutti").clicked() {
                choice.hidden.clear();
            }
            if ui.small_button("Nascondi tutti").clicked() {
                choice.hidden = (0..drawing.layers.len()).collect();
            }
        });
        ui.add_space(4.0);
        for (i, l) in drawing.layers.iter().enumerate() {
            if l.shapes == 0 {
                continue;
            }
            let hidden = choice.hidden.contains(&i);
            let resp = ui
                .horizontal(|ui| {
                    let eye = if hidden { "eye-off" } else { "eye" };
                    let clicked = crate::icons::button(
                        ui,
                        eye,
                        if hidden { "Mostra il layer" } else { "Nascondi il layer" },
                        false,
                        true,
                        26.0,
                    )
                    .clicked();
                    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    let c = if l.aci7 {
                        if choice.dark { Color32::WHITE } else { Color32::BLACK }
                    } else {
                        let [r, g, b] = l.color;
                        Color32::from_rgb(r, g, b)
                    };
                    ui.painter().rect_filled(r, 3.0, c);
                    ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                    let color = if hidden { t.text_dim } else { t.text_strong };
                    ui.add(
                        egui::Label::new(egui::RichText::new(format!("{}  ({})", l.name, l.shapes)).size(12.5).color(color))
                            .truncate(),
                    );
                    clicked
                })
                .inner;
            if resp {
                if hidden {
                    choice.hidden.retain(|h| *h != i);
                } else {
                    choice.hidden.push(i);
                }
            }
        }
        if choice != before {
            self.cad_dark = choice.dark;
            if let Some(d) = self.doc_mut() {
                d.cad = choice;
            }
            self.redo_cad();
        }
    }
}

/// Una lunghezza lunga: in metri oltre il metro.
fn metres(mm: f64, unit: Unit) -> String {
    if mm >= 1000.0 {
        format!("{} m", format_number(mm / 1000.0, 2, false))
    } else {
        format_length(mm, unit)
    }
}
