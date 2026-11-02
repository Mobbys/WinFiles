//! I comandi nello stile di Windows 11 (Fluent): campi con il bordo inferiore che diventa una
//! riga d'accento quando sono attivi, caselle e pallini pieni d'accento, cursori con il pallino,
//! pulsanti a due stati. egui disegna i suoi comandi da sé e non ne lascia cambiare la forma:
//! quelli che non bastano a colori e raggi del tema ([`crate::theme::apply`]) sono disegnati qui.
//! Tutti i colori vengono dal tema, così valgono per il tema chiaro e per quello scuro.

use std::ops::RangeInclusive;

use egui::emath::Numeric;
use egui::{
    Color32, CornerRadius, Margin, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget, pos2, vec2,
};

use crate::theme::Tokens;

/// Altezza dei campi e dei pulsanti, come in Windows 11.
pub const HEIGHT: f32 = 32.0;
/// Margine del testo dentro un campo.
pub const MARGIN: Margin = Margin::symmetric(10, 7);
/// Raggio degli angoli di campi, caselle e pulsanti.
pub const RADIUS: u8 = 4;

/// Esegue `add` con lo stile dei campi: fondo appena staccato dalla scheda, bordo sottile, nessun
/// bordo colorato quando il campo è attivo (lo dice la riga d'accento in basso).
pub fn field_scope<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        let cr = CornerRadius::same(RADIUS);
        v.text_edit_bg_color = Some(t.field);
        v.extreme_bg_color = t.field;
        // Il colore di `selection.stroke` è anche quello del testo selezionato: deve leggersi sul
        // fondo della selezione. Lo spessore 0 toglie il bordo che egui disegna al campo attivo;
        // il bordo sottile lo ridisegna `field_edge`, uguale a quello del campo a riposo.
        v.selection.stroke = Stroke::new(0.0, t.text_strong);
        for (w, fill) in [
            (&mut v.widgets.inactive, t.field),
            (&mut v.widgets.hovered, t.field_hover),
            (&mut v.widgets.active, t.field),
        ] {
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.bg_stroke = Stroke::new(1.0, t.field_border);
            w.corner_radius = cr;
            w.expansion = 0.0;
        }
        add(ui)
    })
    .inner
}

/// Il bordo inferiore di un campo: una riga scura sottile, o la riga d'accento di 2 punti quando
/// il campo è attivo. Segue gli angoli arrotondati, come in Windows 11.
pub fn field_edge(ui: &Ui, rect: Rect, focused: bool) {
    let t = Tokens::get(ui.ctx());
    let (h, color) = if focused {
        (2.0, t.accent)
    } else {
        (1.0, t.field_bottom)
    };
    if focused {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(RADIUS),
            Stroke::new(1.0, t.field_border),
            egui::StrokeKind::Inside,
        );
    }
    let strip = Rect::from_min_max(pos2(rect.left(), rect.bottom() - h), rect.max);
    ui.painter()
        .with_clip_rect(strip.intersect(ui.clip_rect()))
        .rect_filled(rect, RADIUS, color);
}

/// Un campo di testo, su una riga o su più righe. Il margine si sceglie con
/// `TextEdit::margin` (di solito [`MARGIN`]).
pub fn text_field(ui: &mut Ui, edit: egui::TextEdit<'_>) -> Response {
    let resp = field_scope(ui, |ui| ui.add(edit.min_size(vec2(0.0, HEIGHT))));
    field_edge(ui, resp.rect, resp.has_focus());
    resp
}

/// Un campo numerico che si trascina o si scrive, largo `width`.
pub fn number(ui: &mut Ui, width: f32, drag: egui::DragValue<'_>) -> Response {
    let resp = field_scope(ui, |ui| ui.add_sized(vec2(width, HEIGHT), drag));
    field_edge(ui, resp.rect, resp.has_focus());
    resp
}

/// La freccia dei menu a tendina (`ComboBox::icon`): uno chevron sottile al posto del triangolo.
pub fn combo_icon(ui: &Ui, rect: Rect, visuals: &egui::style::WidgetVisuals, _open: bool) {
    let r = Rect::from_center_size(rect.center(), Vec2::splat(rect.height().min(14.0)));
    crate::icons::paint(ui, "chevron-down", r, visuals.fg_stroke.color);
}

/// Un pulsante a due stati (filtri, pagine): pieno d'accento quando è acceso.
pub fn toggle(ui: &mut Ui, on: bool, text: impl Into<String>) -> Response {
    let t = Tokens::get(ui.ctx());
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.into(), font, Color32::WHITE);
    let size = vec2((galley.size().x + 22.0).max(32.0), 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    let (fill, stroke, ink) = match (on, resp.hovered()) {
        (true, false) => (t.primary, Stroke::NONE, t.on_primary),
        (true, true) => (t.accent, Stroke::NONE, t.on_primary),
        (false, false) => (t.button, Stroke::new(1.0, t.field_border), t.text),
        (false, true) => (t.hover, Stroke::new(1.0, t.field_border), t.text_strong),
    };
    painter.rect(rect, RADIUS, fill, stroke, StrokeKind::Inside);
    painter.galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, ink);
    focus_ring(ui, &resp, rect);
    resp
}

/// L'anello di messa a fuoco attorno a un comando scelto con la tastiera.
fn focus_ring(ui: &Ui, resp: &Response, rect: Rect) {
    if resp.has_focus() {
        let t = Tokens::get(ui.ctx());
        ui.painter().rect_stroke(
            rect.expand(2.0),
            RADIUS + 2,
            Stroke::new(1.5, t.text_strong),
            StrokeKind::Outside,
        );
    }
}

/// Una casella di spunta: quadrato con gli angoli arrotondati, pieno d'accento con il segno
/// bianco quando è scelta.
pub fn checkbox<'a>(checked: &'a mut bool, text: impl Into<String>) -> Checkbox<'a> {
    Checkbox {
        checked,
        text: text.into(),
    }
}

pub struct Checkbox<'a> {
    checked: &'a mut bool,
    text: String,
}

impl Widget for Checkbox<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, mut resp, b) = choice_layout(ui, self.text);
        if resp.clicked() {
            *self.checked = !*self.checked;
            resp.mark_changed();
        }
        let t = Tokens::get(ui.ctx());
        let painter = ui.painter();
        if *self.checked {
            let fill = if resp.hovered() { t.accent } else { t.primary };
            painter.rect_filled(b, RADIUS, fill);
            let s = b.width() / 20.0;
            let pts = vec![
                pos2(b.left() + 5.0 * s, b.top() + 10.5 * s),
                pos2(b.left() + 8.5 * s, b.top() + 14.0 * s),
                pos2(b.left() + 15.0 * s, b.top() + 6.5 * s),
            ];
            painter.line(pts, Stroke::new(1.8, t.on_primary));
        } else {
            let fill = if resp.hovered() { t.field_hover } else { t.field };
            painter.rect(
                b,
                RADIUS,
                fill,
                Stroke::new(1.0, t.field_bottom),
                StrokeKind::Inside,
            );
        }
        focus_ring(ui, &resp, b);
        let _ = rect;
        resp
    }
}

/// Un pallino di scelta: cerchio vuoto, o pieno d'accento con il punto bianco al centro (che
/// cresce al passaggio del mouse) quando è la scelta attiva.
pub fn radio(ui: &mut Ui, selected: bool, text: impl Into<String>) -> Response {
    let (_, resp, b) = choice_layout(ui, text.into());
    let t = Tokens::get(ui.ctx());
    let painter = ui.painter();
    let c = b.center();
    let r = b.width() / 2.0;
    if selected {
        painter.circle_filled(c, r, if resp.hovered() { t.accent } else { t.primary });
        let inner = ui.ctx().animate_value_with_time(
            resp.id.with("punto"),
            if resp.hovered() { 5.0 } else { 4.0 },
            0.08,
        );
        painter.circle_filled(c, inner, t.on_primary);
    } else {
        let fill = if resp.hovered() { t.field_hover } else { t.field };
        painter.circle(c, r - 0.5, fill, Stroke::new(1.0, t.field_bottom));
    }
    if resp.has_focus() {
        painter.circle_stroke(c, r + 3.0, Stroke::new(1.5, t.text_strong));
    }
    resp
}

/// Casella o pallino a sinistra (20 punti) e testo a destra: il riquadro intero, la risposta e
/// il quadrato del segno.
fn choice_layout(ui: &mut Ui, text: String) -> (Rect, Response, Rect) {
    const BOX: f32 = 20.0;
    let t = Tokens::get(ui.ctx());
    let galley = ui
        .painter()
        .layout_no_wrap(text, egui::TextStyle::Body.resolve(ui.style()), t.text);
    let size = vec2(BOX + 10.0 + galley.size().x, galley.size().y.max(BOX).max(28.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let b = Rect::from_center_size(pos2(rect.left() + BOX / 2.0, rect.center().y), Vec2::splat(BOX));
    let ink = if ui.is_enabled() { t.text } else { t.text_disabled };
    ui.painter().galley_with_override_text_color(
        pos2(b.right() + 10.0, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    (rect, resp, b)
}

type GetSet<'a> = Box<dyn 'a + FnMut(Option<f64>) -> f64>;
type Formatter<'a> = Box<dyn 'a + Fn(f64) -> String>;
type Parser<'a> = Box<dyn 'a + Fn(&str) -> Option<f64>>;

/// Un cursore orizzontale con il valore in un campo a destra (trascinabile e scrivibile).
pub struct Slider<'a> {
    get_set: GetSet<'a>,
    range: RangeInclusive<f64>,
    step: Option<f64>,
    integral: bool,
    formatter: Option<Formatter<'a>>,
    parser: Option<Parser<'a>>,
    suffix: String,
    decimals: Option<usize>,
}

impl<'a> Slider<'a> {
    pub fn new<N: Numeric>(value: &'a mut N, range: RangeInclusive<N>) -> Self {
        let range = range.start().to_f64()..=range.end().to_f64();
        Self {
            get_set: Box::new(move |v: Option<f64>| {
                if let Some(v) = v {
                    *value = N::from_f64(v);
                }
                value.to_f64()
            }),
            range,
            step: None,
            integral: N::INTEGRAL,
            formatter: None,
            parser: None,
            suffix: String::new(),
            decimals: None,
        }
    }

    pub fn step_by(mut self, step: f64) -> Self {
        self.step = (step > 0.0).then_some(step);
        self
    }

    pub fn custom_formatter(mut self, f: impl 'a + Fn(f64) -> String) -> Self {
        self.formatter = Some(Box::new(f));
        self
    }

    pub fn custom_parser(mut self, p: impl 'a + Fn(&str) -> Option<f64>) -> Self {
        self.parser = Some(Box::new(p));
        self
    }

    pub fn suffix(mut self, s: impl Into<String>) -> Self {
        self.suffix = s.into();
        self
    }

    pub fn fixed_decimals(mut self, d: usize) -> Self {
        self.decimals = Some(d);
        self
    }
}

/// Porta `v` nell'intervallo `lo..=hi` e sul passo.
fn snap(v: f64, lo: f64, hi: f64, step: Option<f64>, integral: bool) -> f64 {
    let mut v = if v.is_finite() { v.clamp(lo, hi) } else { lo };
    if let Some(step) = step {
        v = lo + ((v - lo) / step).round() * step;
    }
    if integral {
        v = v.round();
    }
    v.clamp(lo, hi)
}

impl Widget for Slider<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let Slider {
            mut get_set,
            range,
            step,
            integral,
            formatter,
            parser,
            suffix,
            decimals,
        } = self;
        let t = Tokens::get(ui.ctx());
        let (lo, hi) = (*range.start(), *range.end());
        let fit = move |v: f64| snap(v, lo, hi, step, integral);
        let span = (hi - lo).max(f64::EPSILON);
        // Il binario prende la larghezza che resta, meno il campo del valore.
        const FIELD: f32 = 64.0;
        let width = (ui.available_width() - FIELD - ui.spacing().item_spacing.x).max(60.0);
        ui.horizontal(|ui| {
            let (rect, mut response) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::click_and_drag());
            // Il pallino non esce dal binario: i suoi centri vanno da un bordo all'altro meno il raggio.
            const R: f32 = 10.0;
            let left = rect.left() + R;
            let right = rect.right() - R;
            let current = get_set(None);
            if let Some(p) = response.interact_pointer_pos() {
                let f = f64::from(((p.x - left) / (right - left).max(1.0)).clamp(0.0, 1.0));
                let v = fit(lo + f * span);
                if v != current {
                    get_set(Some(v));
                    response.mark_changed();
                }
            }
            if response.has_focus() {
                let step = step.unwrap_or(if integral { 1.0 } else { span / 100.0 });
                let (l, r) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::ArrowLeft),
                        i.key_pressed(egui::Key::ArrowRight),
                    )
                });
                let delta = f64::from(i8::from(r) - i8::from(l)) * step;
                if delta != 0.0 {
                    let v = fit(get_set(None) + delta);
                    get_set(Some(v));
                    response.mark_changed();
                }
            }
            let value = get_set(None);
            let f = ((value - lo) / span).clamp(0.0, 1.0) as f32;
            let x = left + f * (right - left);
            let cy = rect.center().y;
            let rail = Rect::from_min_max(pos2(left, cy - 2.0), pos2(right, cy + 2.0));
            let painter = ui.painter();
            painter.rect_filled(rail, 2.0, t.field_bottom);
            painter.rect_filled(Rect::from_min_max(rail.min, pos2(x, rail.max.y)), 2.0, t.accent);
            // Il pallino: un cerchio pieno con il bordo appena visibile e il punto d'accento
            // al centro, che cresce sotto il puntatore e si stringe mentre si trascina.
            let inner_target = if response.dragged() {
                4.5
            } else if response.hovered() {
                7.0
            } else {
                5.5
            };
            let inner = ui
                .ctx()
                .animate_value_with_time(response.id.with("pallino"), inner_target, 0.08);
            let c = pos2(x, cy);
            painter.circle(c, R, t.thumb, Stroke::new(1.0, t.field_border));
            painter.circle_filled(c, inner, t.accent);
            if response.has_focus() {
                painter.circle_stroke(c, R + 3.0, Stroke::new(1.5, t.text_strong));
            }

            let mut drag = egui::DragValue::from_get_set(|v| {
                let v = v.map(fit);
                get_set(v)
            })
            .range(lo..=hi)
            .speed(step.unwrap_or(span / 200.0));
            if let Some(fmt) = formatter {
                drag = drag.custom_formatter(move |v, _| fmt(v));
            }
            if let Some(parse) = parser {
                drag = drag.custom_parser(move |s| parse(s));
            }
            if let Some(d) = decimals.or(integral.then_some(0)) {
                drag = drag.fixed_decimals(d);
            }
            if !suffix.is_empty() {
                drag = drag.suffix(suffix);
            }
            let field = number(ui, FIELD, drag);
            response | field
        })
        .inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn il_cursore_resta_nell_intervallo_e_sul_passo() {
        assert_eq!(snap(7.3, 6.0, 24.0, None, true), 7.0);
        assert_eq!(snap(99.0, 6.0, 24.0, None, false), 24.0);
        assert_eq!(snap(f64::NAN, 1.0, 10.0, None, true), 1.0);
        assert_eq!(snap(17.0, 10.0, 150.0, Some(5.0), false), 15.0);
        assert_eq!(snap(18.0, 10.0, 150.0, Some(5.0), false), 20.0);
    }
}
