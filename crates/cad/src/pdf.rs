//! Il disegno in una pagina PDF vettoriale: linee, aree piene e testi in Helvetica. La pagina è
//! grande come l'ingombro alla scala chiesta più un margine; la scheda Visualizza la disegna con
//! lo stesso motore dei PDF e la stampa la mette sul foglio.

use std::fmt::Write as _;
use std::io::Write as _;

use crate::{Drawing, Geometry, Paint};

/// La pagina più grande di un PDF senza UserUnit: 14 400 punti, 5,08 m.
pub const MAX_PAGE_MM: f64 = 5080.0;
const PT_PER_MM: f64 = 72.0 / 25.4;

/// Come disegnare la pagina.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfOptions {
    /// Millimetri sulla pagina per ogni unità del disegno (la scala).
    pub mm_per_unit: f64,
    /// I layer da non disegnare (indici in [`Drawing::layers`]).
    pub hidden: Vec<usize>,
    /// Fondo nero come nei CAD: il colore 7 diventa bianco.
    pub dark: bool,
    /// Tutto in nero (stampa dei disegni tecnici).
    pub black: bool,
    /// Spessore delle linee in mm sulla pagina: `None` = il più sottile possibile (un punto dello
    /// schermo o della stampante), altrimenti quello del file con questo come minimo.
    pub line_mm: Option<f64>,
    /// Margine attorno al disegno, in mm sulla pagina.
    pub margin_mm: f64,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            mm_per_unit: 1.0,
            hidden: Vec::new(),
            dark: false,
            black: false,
            line_mm: None,
            margin_mm: 5.0,
        }
    }
}

impl Drawing {
    /// La misura della pagina in mm con queste scelte.
    pub fn page_mm(&self, o: &PdfOptions) -> (f64, f64) {
        let (w, h) = self.size_mm(o.mm_per_unit);
        (w + 2.0 * o.margin_mm, h + 2.0 * o.margin_mm)
    }

    /// La scala che fa stare il disegno in una pagina di `long` mm sul lato lungo.
    pub fn fit_scale(&self, long: f64) -> f64 {
        let (w, h) = self.size_mm(1.0);
        let side = w.max(h);
        if side > 0.0 && side.is_finite() {
            long / side
        } else {
            1.0
        }
    }

    /// La pagina PDF del disegno.
    pub fn to_pdf(&self, o: &PdfOptions) -> Result<Vec<u8>, String> {
        let k = o.mm_per_unit;
        if !(k.is_finite() && k > 0.0) {
            return Err("scala non valida".into());
        }
        let (pw, ph) = self.page_mm(o);
        let max = MAX_PAGE_MM;
        if !(pw.is_finite() && ph.is_finite()) || pw > max || ph > max {
            return Err(format!(
                "a questa scala il disegno è più grande di {:.2} m: scegli una scala più piccola",
                max / 1000.0
            ));
        }
        let (pw, ph) = (pw.max(1.0), ph.max(1.0));
        let mut ops = String::new();
        if o.dark {
            let _ = writeln!(
                ops,
                "0.06 0.06 0.07 rg 0 0 {} {} re f",
                n(pw * PT_PER_MM),
                n(ph * PT_PER_MM)
            );
        }
        // Dalle unità del disegno ai punti della pagina, con l'angolo in basso a sinistra
        // dell'ingombro sul margine.
        let s = k * PT_PER_MM;
        let [x0, y0, _, _] = self.extents;
        let m = o.margin_mm * PT_PER_MM;
        let _ = writeln!(
            ops,
            "q {} 0 0 {} {} {} cm 1 J 1 j",
            n(s),
            n(s),
            n(m - x0 * s),
            n(m - y0 * s)
        );
        let mut last_color: Option<[u8; 3]> = None;
        let mut last_width: Option<f64> = None;
        for shape in &self.shapes {
            if o.hidden.contains(&shape.layer) {
                continue;
            }
            let rgb = color(shape.paint, o);
            if last_color != Some(rgb) {
                let [r, g, b] = rgb.map(|c| f64::from(c) / 255.0);
                let _ = writeln!(ops, "{} {} {} RG {} {} {} rg", n(r), n(g), n(b), n(r), n(g), n(b));
                last_color = Some(rgb);
            }
            match &shape.geometry {
                Geometry::Line { points, closed } => {
                    // Lo spessore in unità del disegno: mm sulla carta diviso la scala.
                    let width = match o.line_mm {
                        None => 0.0,
                        Some(min) => shape.weight.unwrap_or(min).max(min) / k,
                    };
                    if last_width.is_none_or(|w| (w - width).abs() > 1e-12) {
                        let _ = writeln!(ops, "{} w", n(width));
                        last_width = Some(width);
                    }
                    path(&mut ops, points);
                    ops.push_str(if *closed { "s\n" } else { "S\n" });
                }
                Geometry::Fill { contours } => {
                    for c in contours {
                        path(&mut ops, c);
                        ops.push_str("h\n");
                    }
                    ops.push_str("f*\n");
                }
                Geometry::Text(t) => {
                    if !(t.height.is_finite() && t.height > 0.0) {
                        continue;
                    }
                    let (sn, c) = t.angle.to_radians().sin_cos();
                    let h = t.height;
                    let _ = writeln!(
                        ops,
                        "BT /F1 1 Tf {} {} {} {} {} {} Tm ({}) Tj ET",
                        n(c * h),
                        n(sn * h),
                        n(-sn * h),
                        n(c * h),
                        n(t.at[0]),
                        n(t.at[1]),
                        escape(&t.text)
                    );
                }
            }
        }
        ops.push_str("Q\n");
        Ok(document(&ops, pw * PT_PER_MM, ph * PT_PER_MM))
    }
}

/// Un testo da mettere su una pagina con [`text_page`]: posizione in mm dall'alto a sinistra (la
/// linea di base), altezza in mm, grigio da 0 (nero) a 1.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub gray: f64,
    pub text: String,
}

/// Una pagina di `w`×`h` mm con solo dei testi in Helvetica e delle linee sottili (`lines`,
/// segmenti in mm dall'alto a sinistra): le didascalie e i segni dei fogli stampati.
pub fn text_page(w: f64, h: f64, texts: &[TextLine], lines: &[[f64; 4]]) -> Result<Vec<u8>, String> {
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 && w <= MAX_PAGE_MM && h <= MAX_PAGE_MM) {
        return Err("formato della pagina non valido".into());
    }
    let mut ops = String::new();
    let y = |v: f64| (h - v) * PT_PER_MM;
    if !lines.is_empty() {
        ops.push_str("0.25 w 0.4 G\n");
        for l in lines {
            let _ = writeln!(
                ops,
                "{} {} m {} {} l S",
                n(l[0] * PT_PER_MM),
                n(y(l[1])),
                n(l[2] * PT_PER_MM),
                n(y(l[3]))
            );
        }
    }
    for t in texts {
        if !(t.size.is_finite() && t.size > 0.0) {
            continue;
        }
        let _ = writeln!(
            ops,
            "{} g BT /F1 {} Tf {} {} Td ({}) Tj ET",
            n(t.gray.clamp(0.0, 1.0)),
            n(t.size * PT_PER_MM),
            n(t.x * PT_PER_MM),
            n(y(t.y)),
            escape(&t.text)
        );
    }
    Ok(document(&ops, w * PT_PER_MM, h * PT_PER_MM))
}

/// Il colore di una forma sulla pagina.
fn color(p: Paint, o: &PdfOptions) -> [u8; 3] {
    if o.black {
        return if o.dark { [255, 255, 255] } else { [0, 0, 0] };
    }
    if p.aci7 {
        return if o.dark { [255, 255, 255] } else { [0, 0, 0] };
    }
    p.rgb
}

fn path(ops: &mut String, points: &[[f64; 2]]) {
    for (i, p) in points.iter().enumerate() {
        if !(p[0].is_finite() && p[1].is_finite()) {
            continue;
        }
        let _ = writeln!(ops, "{} {} {}", n(p[0]), n(p[1]), if i == 0 { "m" } else { "l" });
    }
}

/// Un numero per il PDF, senza zeri inutili.
fn n(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{v:.5}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.into()
    }
}

/// Un testo fra parentesi in WinAnsi: le lettere accentate ci stanno, gli altri segni diventano `?`.
fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let b: u8 = match c {
            '€' => 0x80,
            c if (c as u32) < 0x20 => continue,
            c if (c as u32) < 0x7f || (0xa0..=0xff).contains(&(c as u32)) => c as u32 as u8,
            _ => b'?',
        };
        match b {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(b as char);
            }
            0x20..=0x7e => out.push(b as char),
            _ => {
                let _ = write!(out, "\\{b:03o}");
            }
        }
    }
    out
}

/// Un PDF di una pagina con questo contenuto, compresso.
fn document(ops: &str, w: f64, h: f64) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let compressed = z.write_all(ops.as_bytes()).and_then(|()| z.finish()).ok();
    let (content, filter) = match compressed {
        Some(c) => (c, " /Filter /FlateDecode"),
        None => (ops.as_bytes().to_vec(), ""),
    };
    let mut out: Vec<u8> = Vec::new();
    let mut offsets = Vec::new();
    out.extend_from_slice(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n");
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
            n(w),
            n(h)
        )
        .into_bytes(),
        {
            let mut s = format!("<< /Length {}{filter} >>\nstream\n", content.len()).into_bytes();
            s.extend_from_slice(&content);
            s.extend_from_slice(b"\nendstream");
            s
        },
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        b"<< /Producer (WinFiles) >>".to_vec(),
    ];
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info {} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1,
            objects.len()
        )
        .as_bytes(),
    );
    out
}
