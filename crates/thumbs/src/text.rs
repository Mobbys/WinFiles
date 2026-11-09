//! Il testo di una pagina PDF: ogni carattere con il suo riquadro, per selezionarlo e copiarlo
//! dalla scheda Visualizza. Lo legge l'interprete di hayro, lo stesso che disegna la pagina,
//! quindi le posizioni coincidono con quello che si vede.

use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps, InterpreterCache,
    InterpreterSettings, SoftMask, interpret_page,
};
use hayro::kurbo::{Affine, BezPath, Point, Rect};

/// Caratteri massimi letti da una pagina.
const MAX_GLYPHS: usize = 500_000;

/// Un carattere della pagina.
#[derive(Clone, Debug, PartialEq)]
pub struct TextGlyph {
    /// Il testo (di solito una lettera; le legature ne hanno più d'una).
    pub text: String,
    /// Il riquadro in punti PDF, dall'angolo in alto a sinistra della pagina: `[x, y, w, h]`.
    pub rect: [f32; 4],
}

struct Collector {
    glyphs: Vec<TextGlyph>,
}

impl<'a> Device<'a> for Collector {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        for g in run.glyphs() {
            if self.glyphs.len() >= MAX_GLYPHS {
                return;
            }
            let Some(text) = g.as_unicode().map(|u| match u {
                hayro::hayro_interpret::hayro_cmap::BfString::Char(c) => c.to_string(),
                hayro::hayro_interpret::hayro_cmap::BfString::String(s) => s,
            }) else {
                continue;
            };
            if text.chars().all(char::is_control) {
                continue;
            }
            let advance = match &**g {
                Glyph::Outline(o) => o.advance_width().map(f64::from),
                Glyph::Type3(_) => None,
            }
            .filter(|a| a.is_finite() && *a > 0.0)
            .unwrap_or(500.0);
            // Il riquadro del carattere nello spazio del glifo (1000 unità per em).
            let m: Affine = props.transform * g.transform();
            let corners = [
                Point::new(0.0, -200.0),
                Point::new(advance, -200.0),
                Point::new(advance, 800.0),
                Point::new(0.0, 800.0),
            ]
            .map(|p| m * p);
            let mut r = Rect::from_points(corners[0], corners[1]);
            for p in &corners[2..] {
                r = r.union_pt(*p);
            }
            let rect = [r.x0 as f32, r.y0 as f32, r.width() as f32, r.height() as f32];
            if rect.iter().all(|v| v.is_finite()) && rect[2] < 10_000.0 && rect[3] < 10_000.0 {
                self.glyphs.push(TextGlyph { text, rect });
            }
        }
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// I caratteri di una pagina, nell'ordine in cui la pagina li disegna.
pub(crate) fn page_glyphs(page: &hayro::hayro_syntax::page::Page<'_>) -> Vec<TextGlyph> {
    use hayro::hayro_interpret::util::TransformExt;
    let transform = page.initial_transform(true).to_kurbo();
    let crop = page.intersected_crop_box();
    let bbox = Rect::new(0.0, 0.0, crop.width().abs(), crop.height().abs());
    let cache = InterpreterCache::new();
    let mut ctx = Context::new(
        transform,
        bbox,
        &cache,
        page.xref(),
        InterpreterSettings::default(),
    );
    let mut dev = Collector { glyphs: Vec::new() };
    interpret_page(page, &mut ctx, &mut dev);
    dev.glyphs
}

/// Il testo leggibile dei caratteri dati: a righe dall'alto, da sinistra a destra, con gli spazi
/// dove i caratteri sono staccati.
pub fn assemble(glyphs: &[&TextGlyph]) -> String {
    let mut sorted: Vec<&TextGlyph> = glyphs.to_vec();
    // Prima per riga (il centro verticale), poi da sinistra.
    sorted.sort_by(|a, b| {
        let (ca, cb) = (a.rect[1] + a.rect[3] / 2.0, b.rect[1] + b.rect[3] / 2.0);
        ca.total_cmp(&cb)
    });
    let mut lines: Vec<Vec<&TextGlyph>> = Vec::new();
    for g in sorted {
        let c = g.rect[1] + g.rect[3] / 2.0;
        let same = lines.last().and_then(|l| l.first()).is_some_and(|f| {
            let fc = f.rect[1] + f.rect[3] / 2.0;
            (c - fc).abs() < f.rect[3].min(g.rect[3]) * 0.5
        });
        match lines.last_mut() {
            Some(line) if same => line.push(g),
            _ => lines.push(vec![g]),
        }
    }
    let mut out = String::new();
    for (i, line) in lines.iter_mut().enumerate() {
        line.sort_by(|a, b| a.rect[0].total_cmp(&b.rect[0]));
        if i > 0 {
            out.push('\n');
        }
        let mut end: Option<f32> = None;
        for g in line.iter() {
            if let Some(e) = end {
                let gap = g.rect[0] - e;
                if gap > g.rect[3] * 0.2 && !out.ends_with(' ') && !g.text.starts_with(' ') {
                    out.push(' ');
                }
            }
            out.push_str(&g.text);
            end = Some(g.rect[0] + g.rect[2]);
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(t: &str, x: f32, y: f32) -> TextGlyph {
        TextGlyph {
            text: t.into(),
            rect: [x, y, 6.0, 10.0],
        }
    }

    #[test]
    fn i_caratteri_diventano_righe_e_parole() {
        let all = [
            g("o", 6.0, 0.0),
            g("C", 0.0, 0.0),
            g("a", 30.0, 1.0),
            g("i", 12.0, 0.0),
            g("B", 0.0, 20.0),
        ];
        let refs: Vec<&TextGlyph> = all.iter().collect();
        assert_eq!(assemble(&refs), "Coi a\nB");
        assert_eq!(assemble(&[]), "");
    }
}
