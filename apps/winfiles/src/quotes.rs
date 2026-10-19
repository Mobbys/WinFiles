//! Le quote: rettangoli e linee tracciati sull'anteprima di Modifica PDF e Modifica immagini per
//! ricavare misure in proporzione, come la "Simulazione quote" della 4.9. All'inizio ogni quota
//! mostra la misura vera del file; una misura scritta a mano diventa la quota master e tutte le
//! altre si adeguano alla stessa scala. Le quote non cambiano il file e non si salvano.
//!
//! Le coordinate sono nell'unità di chi le usa (millimetri della pagina nel PDF, pixel nelle
//! immagini), come la taglierina; `mm_per_unit` le porta in millimetri.

use egui::{Align, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use winfiles_geom::{Unit, format_number};

use crate::icons;
use crate::theme::{Tokens, semibold};

/// Quanto vicino bisogna essere a una maniglia o a una linea perché risponda, in punti.
const GRIP: f32 = 9.0;
/// Sotto questa misura a schermo, in punti, una quota appena tracciata era un clic: si toglie.
const MIN_DRAWN: f32 = 4.0;
/// La misura più grande che si può scrivere, in millimetri (un chilometro).
const MAX_VALUE: f64 = 1_000_000.0;

/// Una quota: una linea tra due punti o un rettangolo con base e altezza.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Line {
        a: Pos2,
        b: Pos2,
    },
    /// Due angoli opposti, in qualsiasi ordine finché si trascina.
    Rect {
        a: Pos2,
        b: Pos2,
    },
}

impl Shape {
    fn normalized(self) -> Self {
        match self {
            Shape::Line { .. } => self,
            Shape::Rect { a, b } => Shape::Rect {
                a: pos2(a.x.min(b.x), a.y.min(b.y)),
                b: pos2(a.x.max(b.x), a.y.max(b.y)),
            },
        }
    }

    /// La misura che decide se la quota c'è davvero, nell'unità di chi chiama: la lunghezza di
    /// una linea, il lato più corto di un rettangolo (uno schiacciato non dà una proporzione).
    fn extent(self) -> f32 {
        match self {
            Shape::Line { a, b } => a.distance(b),
            Shape::Rect { a, b } => (b.x - a.x).abs().min((b.y - a.y).abs()),
        }
    }

    fn moved(self, d: Vec2) -> Self {
        match self {
            Shape::Line { a, b } => Shape::Line { a: a + d, b: b + d },
            Shape::Rect { a, b } => Shape::Rect { a: a + d, b: b + d },
        }
    }

    /// Il riquadro che la contiene.
    fn bounds(self) -> Rect {
        match self {
            Shape::Line { a, b } | Shape::Rect { a, b } => Rect::from_two_pos(a, b),
        }
    }
}

/// Quale misura di una quota: la lunghezza di una linea, la base o l'altezza di un rettangolo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Length,
    Base,
    Height,
}

/// Che cosa si traccia trascinando su un punto vuoto.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Rect,
    Line,
}

/// La quota master: la sua misura è quella scritta a mano, in millimetri.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Master {
    item: usize,
    side: Side,
    value: f64,
}

/// Che cosa si sta trascinando.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    /// Un capo di una linea: `true` il secondo.
    End { item: usize, second: bool },
    /// Un lato o un angolo di un rettangolo, partendo da com'era.
    Edge {
        item: usize,
        start: Shape,
        left: bool,
        right: bool,
        top: bool,
        bottom: bool,
    },
    /// Tutta la quota.
    Move { item: usize, from: Pos2, start: Shape },
}

impl Grab {
    fn item(self) -> usize {
        match self {
            Grab::End { item, .. } | Grab::Edge { item, .. } | Grab::Move { item, .. } => item,
        }
    }
}

/// Le quote di una pagina o di un'immagine.
pub struct Quotes {
    pub items: Vec<Shape>,
    pub selected: Option<usize>,
    pub tool: Tool,
    master: Option<Master>,
    /// Millimetri veri per millimetro del file quando non c'è una master: 1, o la scala
    /// dell'ultima master tolta, così le altre quote non cambiano sotto gli occhi.
    kept_scale: f64,
    mm_per_unit: f64,
    /// Di quale pagina o immagine sono le quote: cambiando, si ricomincia.
    owner: Option<u64>,
    grab: Option<Grab>,
    /// Dove tenere la lente (a schermo), mentre si trascina.
    pub lens: Option<Pos2>,
    /// La misura da aprire per scriverla, dopo un doppio clic sull'anteprima.
    focus: Option<(usize, Side)>,
}

impl Default for Quotes {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            selected: None,
            tool: Tool::Rect,
            master: None,
            kept_scale: 1.0,
            mm_per_unit: 1.0,
            owner: None,
            grab: None,
            lens: None,
            focus: None,
        }
    }
}

impl Quotes {
    /// Le quote valgono per `owner` (una pagina, un'immagine) misurato in un'unità che vale
    /// `mm_per_unit` millimetri. Se cambia la pagina o l'immagine, si ricomincia da capo.
    pub fn prepare(&mut self, owner: u64, mm_per_unit: f64) {
        if self.owner != Some(owner) {
            let tool = self.tool;
            *self = Self {
                tool,
                owner: Some(owner),
                ..Self::default()
            };
        }
        self.mm_per_unit = if mm_per_unit.is_finite() && mm_per_unit > 0.0 {
            mm_per_unit
        } else {
            1.0
        };
    }

    /// Dimentica le quote e di chi erano: la prossima `prepare` ricomincia comunque.
    pub fn forget(&mut self) {
        let tool = self.tool;
        *self = Self {
            tool,
            ..Self::default()
        };
    }

    /// Toglie tutte le quote.
    pub fn clear(&mut self) {
        let (tool, owner, mm) = (self.tool, self.owner, self.mm_per_unit);
        *self = Self {
            tool,
            owner,
            mm_per_unit: mm,
            ..Self::default()
        };
    }

    /// La misura nel file, in millimetri.
    fn measured(&self, item: usize, side: Side) -> Option<f64> {
        let units = match (self.items.get(item)?.normalized(), side) {
            (Shape::Line { a, b }, Side::Length) => a.distance(b),
            (Shape::Rect { a, b }, Side::Base) => b.x - a.x,
            (Shape::Rect { a, b }, Side::Height) => b.y - a.y,
            _ => return None,
        };
        Some(f64::from(units) * self.mm_per_unit)
    }

    /// Millimetri veri per millimetro del file.
    pub fn scale(&self) -> f64 {
        self.master
            .and_then(|m| {
                let len = self.measured(m.item, m.side)?;
                (len > 1e-9).then(|| m.value / len)
            })
            .filter(|s| s.is_finite() && *s > 0.0)
            .unwrap_or(self.kept_scale)
    }

    /// La misura mostrata, in millimetri: quella del file per la scala.
    pub fn shown(&self, item: usize, side: Side) -> Option<f64> {
        if let Some(m) = self.master.filter(|m| m.item == item && m.side == side) {
            return Some(m.value);
        }
        Some(self.measured(item, side)? * self.scale())
    }

    /// La misura `side` di `item` vale `value` millimetri: diventa la master.
    pub fn set_value(&mut self, item: usize, side: Side, value: f64) {
        let ok = value.is_finite() && value > 0.0 && value <= MAX_VALUE;
        if ok && self.measured(item, side).is_some_and(|l| l > 1e-9) {
            self.master = Some(Master { item, side, value });
        }
    }

    /// `item` diventa la master con la misura che mostra già: nessun numero cambia.
    pub fn make_master(&mut self, item: usize, side: Side) {
        if let Some(v) = self.shown(item, side) {
            self.set_value(item, side, v);
        }
    }

    /// La misura master di `item`, se è sua.
    pub fn master_side(&self, item: usize) -> Option<Side> {
        self.master.filter(|m| m.item == item).map(|m| m.side)
    }

    /// Le misure mostrate sono quelle vere del file.
    pub fn is_true_scale(&self) -> bool {
        self.master.is_none() && (self.kept_scale - 1.0).abs() < 1e-12
    }

    /// Torna alle misure vere del file.
    pub fn true_sizes(&mut self) {
        self.master = None;
        self.kept_scale = 1.0;
    }

    /// Toglie una quota; se era la master, le altre restano alla sua scala.
    pub fn remove(&mut self, item: usize) {
        if item >= self.items.len() {
            return;
        }
        self.kept_scale = self.scale();
        self.master = match self.master {
            Some(m) if m.item == item => None,
            Some(m) if m.item > item => Some(Master {
                item: m.item - 1,
                ..m
            }),
            other => other,
        };
        self.items.remove(item);
        self.selected = match self.selected {
            Some(s) if s == item => None,
            Some(s) if s > item => Some(s - 1),
            other => other,
        };
        self.grab = None;
        self.lens = None;
        self.focus = None;
    }

    /// Toglie la quota scelta; dice se ce n'era una.
    pub fn remove_selected(&mut self) -> bool {
        match self.selected {
            Some(i) => {
                self.remove(i);
                true
            }
            None => false,
        }
    }

    /// Le misure in testo, una quota per riga, da incollare dove serve.
    pub fn summary(&self, unit: Unit) -> String {
        let mut out = if self.is_true_scale() {
            "Quote (misure vere del file)".to_string()
        } else {
            format!(
                "Quote (in proporzione: × {} rispetto al file)",
                format_number(self.scale(), 4, true)
            )
        };
        for (i, shape) in self.items.iter().enumerate() {
            let master = self.master_side(i);
            let mark = |side: Side| if master == Some(side) { " (master)" } else { "" };
            let line = match shape {
                Shape::Line { .. } => format!(
                    "{}. Linea: {}{}",
                    i + 1,
                    length_text(self.shown(i, Side::Length), unit),
                    mark(Side::Length)
                ),
                Shape::Rect { .. } => {
                    let (b, h) = (self.shown(i, Side::Base), self.shown(i, Side::Height));
                    format!(
                        "{}. Rettangolo: base {}{} × altezza {}{} (proporzione {})",
                        i + 1,
                        length_text(b, unit),
                        mark(Side::Base),
                        length_text(h, unit),
                        mark(Side::Height),
                        ratio_text(b, h)
                    )
                }
            };
            out.push('\n');
            out.push_str(&line);
        }
        out
    }

    // --- Sull'anteprima ----------------------------------------------------------------------

    /// Le quote si tracciano, si scelgono, si spostano e si tirano su `paper`, l'anteprima di
    /// una pagina o immagine larga `w` e alta `h` nell'unità di chi chiama.
    pub fn interact(&mut self, ui: &mut egui::Ui, id: egui::Id, paper: Rect, w: f32, h: f32) {
        let resp = ui.interact(paper, id, Sense::click_and_drag());
        let scale = paper.width() / w.max(f32::EPSILON);
        let to_unit = |p: Pos2| -> Pos2 {
            pos2(
                ((p.x - paper.left()) / scale).clamp(0.0, w),
                ((p.y - paper.top()) / scale).clamp(0.0, h),
            )
        };
        let shift = ui.input(|i| i.modifiers.shift);
        if resp.drag_started() || resp.clicked() {
            // Chi aveva la tastiera la lascia: Canc deve togliere la quota, non un carattere.
            ui.memory_mut(|m| {
                if let Some(f) = m.focused() {
                    m.surrender_focus(f);
                }
            });
        }
        if resp.drag_started() || resp.clicked() && !resp.double_clicked() {
            self.focus = None;
        }
        if resp.double_clicked() {
            if let Some((item, side)) = resp
                .interact_pointer_pos()
                .and_then(|pos| self.hit_item(to_unit(pos), scale))
            {
                self.selected = Some(item);
                self.focus = Some((item, side));
            }
        } else if resp.clicked() {
            self.selected = resp
                .interact_pointer_pos()
                .and_then(|pos| self.hit_item(to_unit(pos), scale))
                .map(|(item, _)| item);
        }
        if resp.drag_started() {
            // Dove il pulsante è stato premuto: il primo movimento può essere già lontano.
            let origin = ui
                .ctx()
                .input(|i| i.pointer.press_origin())
                .or_else(|| resp.interact_pointer_pos());
            if let Some(pos) = origin {
                let at = to_unit(pos);
                let grab = self.hit_grab(at, scale).unwrap_or_else(|| {
                    let item = self.items.len();
                    match self.tool {
                        Tool::Line => {
                            self.items.push(Shape::Line { a: at, b: at });
                            Grab::End { item, second: true }
                        }
                        Tool::Rect => {
                            let start = Shape::Rect { a: at, b: at };
                            self.items.push(start);
                            Grab::Edge {
                                item,
                                start,
                                left: false,
                                right: true,
                                top: false,
                                bottom: true,
                            }
                        }
                    }
                });
                self.selected = Some(grab.item());
                self.grab = Some(grab);
            }
        }
        if let (true, Some(grab), Some(pos)) = (resp.dragged(), self.grab, resp.interact_pointer_pos()) {
            let at = to_unit(pos);
            let changed = match grab {
                Grab::End { item, second } => self.items.get(item).and_then(|s| match *s {
                    Shape::Line { a, b } => {
                        let fixed = if second { a } else { b };
                        // Con Maiusc la linea resta orizzontale o verticale, come nella 4.9.
                        let p = if shift { straight(fixed, at) } else { at };
                        Some(if second {
                            Shape::Line { a, b: p }
                        } else {
                            Shape::Line { a: p, b }
                        })
                    }
                    Shape::Rect { .. } => None,
                }),
                Grab::Edge {
                    start,
                    left,
                    right,
                    top,
                    bottom,
                    ..
                } => match start.normalized() {
                    Shape::Rect { a, b } => Some(Shape::Rect {
                        a: pos2(if left { at.x } else { a.x }, if top { at.y } else { a.y }),
                        b: pos2(if right { at.x } else { b.x }, if bottom { at.y } else { b.y }),
                    }),
                    Shape::Line { .. } => None,
                },
                Grab::Move { from, start, .. } => {
                    // Si sposta senza uscire dal foglio.
                    let r = start.bounds();
                    let d = vec2(
                        (at.x - from.x).clamp(-r.left(), w - r.right()),
                        (at.y - from.y).clamp(-r.top(), h - r.bottom()),
                    );
                    Some(start.moved(d))
                }
            };
            if let (Some(shape), Some(slot)) = (changed, self.items.get_mut(grab.item())) {
                *slot = shape;
            }
            self.lens = Some(pos);
        }
        if resp.drag_stopped() {
            if let Some(grab) = self.grab.take() {
                let item = grab.item();
                if let Some(slot) = self.items.get_mut(item) {
                    *slot = slot.normalized();
                    // Un clic senza trascinare non lascia una quota vuota.
                    if slot.extent() * scale < MIN_DRAWN {
                        self.remove(item);
                    }
                }
            }
            self.lens = None;
        }
        // Il cursore dice cosa si sta per fare.
        if let Some(pos) = ui.ctx().pointer_latest_pos().filter(|p| paper.contains(*p)) {
            let cursor = match self.hit_grab(to_unit(pos), scale) {
                None => egui::CursorIcon::Crosshair,
                Some(Grab::End { .. }) => egui::CursorIcon::Grab,
                Some(Grab::Move { .. }) => egui::CursorIcon::Move,
                Some(Grab::Edge { left, right, .. }) if !left && !right => egui::CursorIcon::ResizeVertical,
                Some(Grab::Edge { top, bottom, .. }) if !top && !bottom => egui::CursorIcon::ResizeHorizontal,
                Some(Grab::Edge { .. }) => egui::CursorIcon::ResizeNwSe,
            };
            ui.ctx().set_cursor_icon(cursor);
        }
    }

    /// L'ordine in cui cercare sotto il cursore: prima la quota scelta, poi le ultime tracciate.
    fn search_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.items.len()).rev().collect();
        if let Some(s) = self.selected.filter(|s| *s < self.items.len()) {
            order.retain(|i| *i != s);
            order.insert(0, s);
        }
        order
    }

    /// Che cosa si afferra premendo in `at`; `None` su un punto vuoto (si traccia una quota).
    fn hit_grab(&self, at: Pos2, scale: f32) -> Option<Grab> {
        let near = GRIP / scale.max(0.0001);
        for item in self.search_order() {
            let Some(shape) = self.items.get(item).map(|s| s.normalized()) else {
                continue;
            };
            match shape {
                Shape::Line { a, b } => {
                    if at.distance(b) <= near {
                        return Some(Grab::End { item, second: true });
                    }
                    if at.distance(a) <= near {
                        return Some(Grab::End { item, second: false });
                    }
                    if segment_distance(at, a, b) <= near {
                        return Some(Grab::Move {
                            item,
                            from: at,
                            start: shape,
                        });
                    }
                }
                Shape::Rect { a, b } => {
                    let left = (at.x - a.x).abs() <= near;
                    let right = (at.x - b.x).abs() <= near;
                    let top = (at.y - a.y).abs() <= near;
                    let bottom = (at.y - b.y).abs() <= near;
                    let inside_x = at.x >= a.x - near && at.x <= b.x + near;
                    let inside_y = at.y >= a.y - near && at.y <= b.y + near;
                    if (left || right) && inside_y || (top || bottom) && inside_x {
                        return Some(Grab::Edge {
                            item,
                            start: shape,
                            left: left && inside_y,
                            right: right && inside_y,
                            top: top && inside_x,
                            bottom: bottom && inside_x,
                        });
                    }
                    // Dentro il rettangolo scelto lo si sposta; dentro gli altri si traccia
                    // (una linea su un dettaglio del logo, per esempio).
                    let inside = at.x > a.x && at.x < b.x && at.y > a.y && at.y < b.y;
                    if inside && self.selected == Some(item) {
                        return Some(Grab::Move {
                            item,
                            from: at,
                            start: shape,
                        });
                    }
                }
            }
        }
        None
    }

    /// La quota sotto il cursore e la misura più vicina: per sceglierla o per scriverla.
    fn hit_item(&self, at: Pos2, scale: f32) -> Option<(usize, Side)> {
        let near = GRIP / scale.max(0.0001);
        let mut inside = None;
        for item in self.search_order() {
            match self.items.get(item).map(|s| s.normalized()) {
                Some(Shape::Line { a, b }) if segment_distance(at, a, b) <= near => {
                    return Some((item, Side::Length));
                }
                Some(Shape::Rect { a, b }) => {
                    let r = Rect::from_min_max(a, b);
                    let ring = r.expand(near).contains(at);
                    let side_edge = (at.x - a.x).abs() <= near || (at.x - b.x).abs() <= near;
                    let top_edge = (at.y - a.y).abs() <= near || (at.y - b.y).abs() <= near;
                    if ring && side_edge && !top_edge {
                        return Some((item, Side::Height));
                    }
                    if ring && top_edge {
                        return Some((item, Side::Base));
                    }
                    if inside.is_none() && r.contains(at) {
                        inside = Some((item, Side::Base));
                    }
                }
                _ => {}
            }
        }
        inside
    }

    /// Le quote sopra l'anteprima: linee con le stanghette, rettangoli, e le misure nelle
    /// etichette. La master è arancione.
    pub fn paint(&self, ui: &egui::Ui, paper: Rect, w: f32, unit: Unit) {
        let t = Tokens::get(ui.ctx());
        let scale = paper.width() / w.max(f32::EPSILON);
        let screen = |p: Pos2| pos2(paper.left() + p.x * scale, paper.top() + p.y * scale);
        let p = ui.painter().with_clip_rect(paper.expand(40.0));
        // Un alone chiaro sotto le linee: si vedono anche su un fondo scuro.
        let halo = egui::Color32::from_white_alpha(170);
        for (i, shape) in self.items.iter().enumerate() {
            let master = self.master_side(i);
            let chosen = self.selected == Some(i);
            let color_of = |side: Side| {
                if master == Some(side) {
                    t.quote_master
                } else {
                    t.quote
                }
            };
            let width = if chosen { 2.5 } else { 1.75 };
            match shape.normalized() {
                Shape::Line { a, b } => {
                    let (a, b) = (screen(a), screen(b));
                    let color = color_of(Side::Length);
                    let dir = (b - a).normalized();
                    let normal = vec2(-dir.y, dir.x);
                    for s in [Stroke::new(width + 2.5, halo), Stroke::new(width, color)] {
                        p.line_segment([a, b], s);
                        // Le stanghette ai capi, come sulle quote dei disegni.
                        for end in [a, b] {
                            p.line_segment([end - normal * 7.0, end + normal * 7.0], s);
                        }
                    }
                    // L'etichetta sta dalla parte di sopra della linea.
                    let up = if normal.y > 0.0 { -normal } else { normal };
                    let at = a + (b - a) / 2.0 + up * 15.0;
                    pill(&p, at, &length_text(self.shown(i, Side::Length), unit), color, t);
                    if chosen {
                        for end in [a, b] {
                            handle(&p, end, color);
                        }
                    }
                }
                Shape::Rect { a, b } => {
                    let r = Rect::from_min_max(screen(a), screen(b));
                    p.rect_stroke(r, 0.0, Stroke::new(width + 2.5, halo), StrokeKind::Middle);
                    let color = if master.is_some() { t.quote_master } else { t.quote };
                    p.rect_stroke(r, 0.0, Stroke::new(width, color), StrokeKind::Middle);
                    // La base sopra il rettangolo (dentro, se sopra non c'è posto), l'altezza a
                    // destra (o dentro).
                    let top = if r.top() - 16.0 > paper.top() - 30.0 {
                        r.top() - 14.0
                    } else {
                        r.top() + 14.0
                    };
                    let base = self.shown(i, Side::Base);
                    pill(
                        &p,
                        pos2(r.center().x, top),
                        &length_text(base, unit),
                        color_of(Side::Base),
                        t,
                    );
                    let text = length_text(self.shown(i, Side::Height), unit);
                    let half = text_width(ui, &text) / 2.0 + 12.0;
                    let x = if r.right() + 2.0 * half < paper.right() + 30.0 {
                        r.right() + half
                    } else {
                        r.right() - half
                    };
                    pill(&p, pos2(x, r.center().y), &text, color_of(Side::Height), t);
                    if chosen {
                        for h in [
                            r.left_top(),
                            r.center_top(),
                            r.right_top(),
                            r.right_center(),
                            r.right_bottom(),
                            r.center_bottom(),
                            r.left_bottom(),
                            r.left_center(),
                        ] {
                            handle(&p, h, color);
                        }
                    }
                }
            }
        }
    }

    // --- Il pannello -------------------------------------------------------------------------

    /// Il pannello delle quote: cosa tracciare, la scala, ogni quota con le sue misure da
    /// scrivere, e i pulsanti per copiarle o toglierle.
    pub fn panel(&mut self, ui: &mut egui::Ui, unit: Unit) {
        let t = Tokens::get(ui.ctx());
        ui.label(egui::RichText::new("Trascina sull'anteprima per tracciare").color(t.text_dim));
        ui.add_space(4.0);
        crate::app::segmented(
            ui,
            "quote-strumento",
            &mut self.tool,
            &[(Tool::Rect, "Rettangolo"), (Tool::Line, "Linea")],
        );
        ui.add_space(10.0);
        // La scala.
        egui::Frame::NONE
            .fill(t.panel_darker)
            .corner_radius(t.radius)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if self.is_true_scale() {
                    ui.label(egui::RichText::new("Misure vere del file").font(semibold(13.0)).color(t.text_strong));
                    ui.label(
                        egui::RichText::new(
                            "Scrivi una misura: diventa la quota master e le altre si adeguano in proporzione.",
                        )
                        .size(12.0)
                        .color(t.text_dim),
                    );
                } else {
                    ui.label(
                        egui::RichText::new(format!(
                            "In scala × {} sul file",
                            format_number(self.scale(), 2, true)
                        ))
                        .font(semibold(13.0))
                        .color(t.text_strong),
                    );
                    let what = match self.master {
                        Some(m) => format!(
                            "La master è {} della quota {}.",
                            match m.side {
                                Side::Length => "la lunghezza",
                                Side::Base => "la base",
                                Side::Height => "l'altezza",
                            },
                            m.item + 1
                        ),
                        None => "La quota master è stata tolta: le altre restano alla sua scala.".into(),
                    };
                    ui.label(egui::RichText::new(what).size(12.0).color(t.text_dim));
                    ui.add_space(4.0);
                    if ui
                        .add(egui::Button::new("Torna alle misure vere").min_size(vec2(0.0, 28.0)))
                        .clicked()
                    {
                        self.true_sizes();
                    }
                }
            });
        ui.add_space(10.0);
        if self.items.is_empty() {
            ui.label(
                egui::RichText::new(
                    "Nessuna quota. Un rettangolo dà base e altezza, una linea una misura sola \
                     (con Maiusc resta dritta).",
                )
                .color(t.text_dim),
            );
        }
        let mut remove = None;
        let mut select = None;
        for i in 0..self.items.len() {
            let Some(shape) = self.items.get(i).copied() else {
                continue;
            };
            let chosen = self.selected == Some(i);
            let master = self.master_side(i);
            let frame = egui::Frame::NONE
                .fill(if chosen { t.selection } else { t.panel_darker })
                .corner_radius(t.radius)
                .inner_margin(egui::Margin::symmetric(10, 8));
            let row = frame.show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(
                        rect.center(),
                        5.0,
                        if master.is_some() { t.quote_master } else { t.quote },
                    );
                    let name = match shape {
                        Shape::Line { .. } => format!("Linea {}", i + 1),
                        Shape::Rect { .. } => format!("Rettangolo {}", i + 1),
                    };
                    let ink = if chosen { t.accent_text } else { t.text_strong };
                    ui.label(egui::RichText::new(name).font(semibold(13.0)).color(ink));
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if icons::button(ui, "trash-2", "Togli la quota (Canc)", false, true, 24.0).clicked()
                        {
                            remove = Some(i);
                        }
                        let first = match shape {
                            Shape::Line { .. } => Side::Length,
                            Shape::Rect { .. } => Side::Base,
                        };
                        if master.is_none()
                            && ui
                                .add(egui::Button::new("Master").min_size(vec2(0.0, 24.0)))
                                .on_hover_text(
                                    "Le altre quote si adeguano a questa, con la misura che ha ora",
                                )
                                .clicked()
                        {
                            self.make_master(i, first);
                        }
                    });
                });
                ui.add_space(4.0);
                let sides: &[(Side, &str)] = match shape {
                    Shape::Line { .. } => &[(Side::Length, "Lunghezza")],
                    Shape::Rect { .. } => &[(Side::Base, "Base"), (Side::Height, "Altezza")],
                };
                ui.horizontal(|ui| {
                    for (side, label) in sides {
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(*label).size(11.5).color(t.text_dim));
                                if master == Some(*side) {
                                    ui.label(egui::RichText::new("master").size(11.5).color(t.warning));
                                }
                            });
                            let mut v = self.shown(i, *side).unwrap_or(0.0);
                            let resp = crate::fluent::number(ui, 104.0, value_field(&mut v, unit));
                            // Il doppio clic sull'anteprima apre la misura da scrivere: si insiste
                            // finché il campo non ha davvero la tastiera (il clic appena finito
                            // gliela toglierebbe).
                            if self.focus == Some((i, *side)) {
                                if ui.memory(|m| m.has_focus(resp.id)) {
                                    self.focus = None;
                                } else {
                                    resp.request_focus();
                                    // Il numero di adesso già scelto: si scrive sopra.
                                    select_all(ui, resp.id, &length_text(Some(v), unit));
                                }
                            }
                            // Solo un numero diverso fa la master: entrare e uscire dal campo no.
                            let before = self.shown(i, *side).unwrap_or(0.0);
                            if resp.changed() && (v - before).abs() > before.abs() * 1e-9 + 1e-9 {
                                self.set_value(i, *side, v);
                            }
                            if resp.gained_focus() || resp.clicked() {
                                select = Some(i);
                            }
                        });
                    }
                });
                if let Shape::Rect { .. } = shape {
                    let ratio = ratio_text(self.shown(i, Side::Base), self.shown(i, Side::Height));
                    ui.label(
                        egui::RichText::new(format!("Proporzione base : altezza  {ratio}"))
                            .size(12.0)
                            .color(t.text_dim),
                    );
                }
            });
            if row.response.interact(Sense::click()).clicked() {
                select = Some(i);
            }
            ui.add_space(6.0);
        }
        if let Some(i) = select {
            self.selected = Some(i);
        }
        if let Some(i) = remove {
            self.remove(i);
        }
        if !self.items.is_empty() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui
                    .add(egui::Button::new("Copia le misure").min_size(vec2(0.0, 30.0)))
                    .on_hover_text(
                        "Copia tutte le quote come testo, da incollare in una mail o in un preventivo",
                    )
                    .clicked()
                {
                    ui.ctx().copy_text(self.summary(unit));
                }
                if ui
                    .add(egui::Button::new("Togli tutte").min_size(vec2(0.0, 30.0)))
                    .clicked()
                {
                    self.clear();
                }
            });
        }
    }
}

/// Il punto `to` portato sulla linea orizzontale o verticale che passa da `from`, la più vicina.
fn straight(from: Pos2, to: Pos2) -> Pos2 {
    if (to.x - from.x).abs() >= (to.y - from.y).abs() {
        pos2(to.x, from.y)
    } else {
        pos2(from.x, to.y)
    }
}

/// La distanza di `p` dal segmento da `a` a `b`.
fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 <= f32::EPSILON {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Una misura in millimetri nell'unità scelta: `1.500 mm`, `64,25 cm`.
fn length_text(mm: Option<f64>, unit: Unit) -> String {
    match mm {
        Some(v) => format!(
            "{} {}",
            format_number(v / unit.mm(), unit.decimals(), true),
            unit.symbol()
        ),
        None => "–".into(),
    }
}

/// Il rapporto tra base e altezza, con 1 dalla parte più corta: `2,35 : 1`, `1 : 1,5`.
fn ratio_text(base: Option<f64>, height: Option<f64>) -> String {
    match (base, height) {
        (Some(b), Some(h)) if b > 0.0 && h > 0.0 => {
            if b >= h {
                format!("{} : 1", format_number(b / h, 2, true))
            } else {
                format!("1 : {}", format_number(h / b, 2, true))
            }
        }
        _ => "–".into(),
    }
}

/// Il campo di una misura in millimetri, mostrata e scritta nell'unità scelta.
fn value_field(value: &mut f64, unit: Unit) -> egui::DragValue<'_> {
    let (factor, suffix, decimals) = (unit.mm(), unit.symbol(), unit.decimals());
    let speed = (*value * 0.002).max(0.01);
    egui::DragValue::new(value)
        .range(0.0..=MAX_VALUE)
        .speed(speed)
        .custom_formatter(move |v, _| format!("{} {suffix}", format_number(v / factor, decimals, true)))
        .custom_parser(move |text| parse_number(text.trim().trim_end_matches(suffix)).map(|v| v * factor))
}

/// Un numero scritto all'italiana (`1.500,5`), ma anche col punto decimale (`1500.5`).
fn parse_number(text: &str) -> Option<f64> {
    let text = text.trim();
    let thousands = |t: &str| {
        let mut parts = t.split('.');
        parts.next().is_some_and(|p| (1..=3).contains(&p.len()))
            && t.contains('.')
            && parts.all(|p| p.len() == 3)
    };
    let clean = if text.contains(',') || thousands(text) {
        text.replace('.', "").replace(',', ".")
    } else {
        text.to_string()
    };
    clean.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Tutto il testo del campo `id` scelto, così quello che si scrive lo sostituisce.
fn select_all(ui: &egui::Ui, id: egui::Id, text: &str) {
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
        egui::text::CCursor::default(),
        egui::text::CCursor::new(text.chars().count()),
    )));
    state.store(ui.ctx(), id);
}

fn text_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_string(), semibold(12.0), egui::Color32::WHITE)
        .size()
        .x
}

/// L'etichetta di una misura: testo bianco su una pastiglia del colore della quota.
fn pill(p: &egui::Painter, at: Pos2, text: &str, fill: egui::Color32, t: Tokens) {
    let galley = p.layout_no_wrap(text.to_string(), semibold(12.0), t.on_quote);
    let r = Rect::from_center_size(at, galley.size() + vec2(12.0, 5.0));
    p.rect_filled(r.expand(1.0), 6.0, egui::Color32::from_white_alpha(170));
    p.rect_filled(r, 5.0, fill);
    p.galley(r.center() - galley.size() / 2.0, galley, t.on_quote);
}

fn handle(p: &egui::Painter, at: Pos2, color: egui::Color32) {
    p.rect(
        Rect::from_center_size(at, Vec2::splat(8.0)),
        1.0,
        color,
        Stroke::new(1.0, egui::Color32::WHITE),
        StrokeKind::Middle,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(items: Vec<Shape>, mm_per_unit: f64) -> Quotes {
        let mut q = Quotes::default();
        q.prepare(1, mm_per_unit);
        q.items = items;
        q
    }

    fn rect(w: f32, h: f32) -> Shape {
        Shape::Rect {
            a: pos2(10.0, 10.0),
            b: pos2(10.0 + w, 10.0 + h),
        }
    }

    fn line(len: f32) -> Shape {
        Shape::Line {
            a: pos2(0.0, 0.0),
            b: pos2(len, 0.0),
        }
    }

    fn near(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-6)
    }

    #[test]
    fn senza_master_si_vedono_le_misure_vere() {
        // Un'immagine a 254 DPI: 10 pixel sono 1 mm.
        let q = with(vec![rect(100.0, 40.0), line(30.0)], 0.1);
        assert!(q.is_true_scale());
        assert!(near(q.shown(0, Side::Base), 10.0));
        assert!(near(q.shown(0, Side::Height), 4.0));
        assert!(near(q.shown(1, Side::Length), 3.0));
        // Una linea non ha base, un rettangolo non ha lunghezza.
        assert_eq!(q.shown(1, Side::Base), None);
        assert_eq!(q.shown(0, Side::Length), None);
    }

    #[test]
    fn scritta_la_base_si_ricava_l_altezza_e_viceversa() {
        let mut q = with(vec![rect(50.0, 20.0)], 1.0);
        q.set_value(0, Side::Base, 1500.0);
        assert!(near(q.shown(0, Side::Base), 1500.0));
        assert!(near(q.shown(0, Side::Height), 600.0));
        q.set_value(0, Side::Height, 300.0);
        assert!(near(q.shown(0, Side::Base), 750.0));
        assert_eq!(q.master_side(0), Some(Side::Height));
    }

    #[test]
    fn le_altre_quote_seguono_la_master() {
        let mut q = with(vec![line(10.0), rect(20.0, 5.0), line(25.0)], 1.0);
        q.set_value(0, Side::Length, 100.0);
        assert!(near(q.shown(1, Side::Base), 200.0));
        assert!(near(q.shown(1, Side::Height), 50.0));
        assert!(near(q.shown(2, Side::Length), 250.0));
        // Una nuova master prende il posto della vecchia.
        q.set_value(2, Side::Length, 50.0);
        assert!(near(q.shown(0, Side::Length), 20.0));
        assert_eq!(q.master_side(0), None);
        assert_eq!(q.master_side(2), Some(Side::Length));
    }

    #[test]
    fn spostando_la_master_cambia_la_scala_non_il_suo_valore() {
        let mut q = with(vec![line(10.0), line(20.0)], 1.0);
        q.set_value(0, Side::Length, 100.0);
        q.items[0] = line(20.0);
        assert!(near(q.shown(0, Side::Length), 100.0));
        assert!(near(q.shown(1, Side::Length), 100.0));
    }

    #[test]
    fn diventare_master_non_cambia_i_numeri() {
        let mut q = with(vec![line(10.0), line(20.0)], 1.0);
        q.set_value(0, Side::Length, 30.0);
        q.make_master(1, Side::Length);
        assert_eq!(q.master_side(1), Some(Side::Length));
        assert!(near(q.shown(0, Side::Length), 30.0));
        assert!(near(q.shown(1, Side::Length), 60.0));
    }

    #[test]
    fn togliendo_la_master_le_altre_restano_alla_sua_scala() {
        let mut q = with(vec![line(10.0), line(20.0), line(5.0)], 1.0);
        q.set_value(1, Side::Length, 40.0);
        q.selected = Some(2);
        q.remove(1);
        assert_eq!(q.items.len(), 2);
        assert!(!q.is_true_scale());
        assert!(near(q.shown(0, Side::Length), 20.0));
        assert!(near(q.shown(1, Side::Length), 10.0));
        // La scelta segue la quota, che ora è la seconda.
        assert_eq!(q.selected, Some(1));
        q.true_sizes();
        assert!(near(q.shown(0, Side::Length), 10.0));
    }

    #[test]
    fn togliendo_una_quota_prima_della_master_la_master_resta_la_stessa() {
        let mut q = with(vec![line(10.0), line(20.0)], 1.0);
        q.set_value(1, Side::Length, 40.0);
        q.remove(0);
        assert_eq!(q.master_side(0), Some(Side::Length));
        assert!(near(q.shown(0, Side::Length), 40.0));
    }

    #[test]
    fn valori_assurdi_non_cambiano_niente() {
        let mut q = with(vec![line(10.0), line(0.0)], 1.0);
        for v in [0.0, -5.0, f64::NAN, f64::INFINITY, MAX_VALUE * 2.0] {
            q.set_value(0, Side::Length, v);
            assert!(q.is_true_scale(), "{v}");
        }
        // Una quota lunga zero non può fare da master: non si saprebbe la scala.
        q.set_value(1, Side::Length, 100.0);
        assert!(q.is_true_scale());
        // Indici e misure che non esistono.
        q.set_value(7, Side::Length, 100.0);
        q.set_value(0, Side::Base, 100.0);
        assert!(q.is_true_scale());
        q.remove(9);
        assert_eq!(q.items.len(), 2);
    }

    #[test]
    fn la_master_schiacciata_a_zero_lascia_la_scala_di_prima() {
        let mut q = with(vec![line(10.0), line(20.0)], 1.0);
        q.set_value(0, Side::Length, 100.0);
        q.items[0] = line(0.0);
        // Nessuna divisione per zero: si torna alla scala tenuta (le misure vere).
        assert!(near(q.shown(1, Side::Length), 20.0));
    }

    #[test]
    fn cambiando_pagina_si_ricomincia() {
        let mut q = with(vec![line(10.0)], 1.0);
        q.tool = Tool::Line;
        q.set_value(0, Side::Length, 100.0);
        q.prepare(1, 1.0);
        assert_eq!(q.items.len(), 1);
        q.prepare(2, 1.0);
        assert!(q.items.is_empty());
        assert!(q.is_true_scale());
        // Lo strumento scelto resta.
        assert_eq!(q.tool, Tool::Line);
    }

    #[test]
    fn la_linea_con_maiusc_resta_dritta() {
        let from = pos2(10.0, 10.0);
        assert_eq!(straight(from, pos2(50.0, 14.0)), pos2(50.0, 10.0));
        assert_eq!(straight(from, pos2(12.0, -30.0)), pos2(10.0, -30.0));
    }

    #[test]
    fn distanza_dal_segmento() {
        let (a, b) = (pos2(0.0, 0.0), pos2(10.0, 0.0));
        assert!((segment_distance(pos2(5.0, 3.0), a, b) - 3.0).abs() < 1e-6);
        assert!((segment_distance(pos2(13.0, 4.0), a, b) - 5.0).abs() < 1e-6);
        assert!((segment_distance(pos2(1.0, 1.0), a, a) - 2f32.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn si_afferra_quello_che_c_e_sotto_il_cursore() {
        let mut q = with(vec![rect(100.0, 50.0), line(40.0)], 1.0);
        // Il capo della linea.
        assert!(matches!(
            q.hit_grab(pos2(40.0, 1.0), 1.0),
            Some(Grab::End {
                item: 1,
                second: true
            })
        ));
        // Il lato destro del rettangolo (da x 10 a 110).
        assert!(matches!(
            q.hit_grab(pos2(111.0, 30.0), 1.0),
            Some(Grab::Edge {
                item: 0,
                right: true,
                left: false,
                ..
            })
        ));
        // Dentro il rettangolo non scelto si traccia, dentro quello scelto lo si sposta.
        assert_eq!(q.hit_grab(pos2(60.0, 30.0), 1.0), None);
        q.selected = Some(0);
        assert!(matches!(
            q.hit_grab(pos2(60.0, 30.0), 1.0),
            Some(Grab::Move { item: 0, .. })
        ));
        // Per scrivere la misura: il lato verticale dà l'altezza, quello orizzontale la base.
        assert_eq!(q.hit_item(pos2(10.0, 30.0), 1.0), Some((0, Side::Height)));
        assert_eq!(q.hit_item(pos2(60.0, 60.0), 1.0), Some((0, Side::Base)));
        assert_eq!(q.hit_item(pos2(20.0, 0.5), 1.0), Some((1, Side::Length)));
        assert_eq!(q.hit_item(pos2(300.0, 300.0), 1.0), None);
    }

    #[test]
    fn le_misure_in_testo() {
        let mut q = with(vec![rect(50.0, 20.0), line(10.0)], 1.0);
        assert_eq!(
            q.summary(Unit::Mm),
            "Quote (misure vere del file)\n\
             1. Rettangolo: base 50 mm × altezza 20 mm (proporzione 2,5 : 1)\n\
             2. Linea: 10 mm"
        );
        q.set_value(0, Side::Base, 1500.0);
        assert_eq!(
            q.summary(Unit::Cm),
            "Quote (in proporzione: × 30 rispetto al file)\n\
             1. Rettangolo: base 150 cm (master) × altezza 60 cm (proporzione 2,5 : 1)\n\
             2. Linea: 30 cm"
        );
    }

    #[test]
    fn proporzioni_e_numeri_scritti() {
        assert_eq!(ratio_text(Some(20.0), Some(30.0)), "1 : 1,5");
        assert_eq!(ratio_text(Some(0.0), Some(30.0)), "–");
        assert_eq!(length_text(Some(1234.56), Unit::Mm), "1.234,6 mm");
        assert_eq!(parse_number("1.500,5"), Some(1500.5));
        assert_eq!(parse_number("1500,5"), Some(1500.5));
        assert_eq!(parse_number("1500.5"), Some(1500.5));
        assert_eq!(parse_number("1.500"), Some(1500.0));
        assert_eq!(parse_number("12.5"), Some(12.5));
        assert_eq!(parse_number("abc"), None);
    }
}
