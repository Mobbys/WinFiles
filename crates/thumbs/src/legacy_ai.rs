//! Disegni di Illustrator in PostScript: gli AI nel formato "legacy" (Illustrator 3–8, ancora
//! usato per i plotter da taglio) e gli EPS di Illustrator senza anteprima. Questi file non sono
//! PDF e non dicono come sono fatti i loro operatori (il prologo è solo nominato), ma gli
//! operatori del disegno sono pubblici (Adobe Illustrator File Format Specification 7.0) e quasi
//! uguali a quelli di un PDF. Qui si traducono le forme e i colori in una pagina PDF, che si
//! disegna come le altre.
//!
//! Si leggono tracciati, tracciati composti, maschere, colori (grigio, quadricromia, tinte
//! piatte, RGB), spessori e tratteggi. Testi, immagini, sfumature e motivi non si disegnano:
//! per una miniatura bastano le forme (e i file da taglio hanno solo quelle).

use std::fmt::Write as _;

/// Comandi letti al massimo.
const MAX_OPS: usize = 20_000_000;
/// Contenuto PDF scritto al massimo.
const MAX_CONTENT: usize = 256 * 1024 * 1024;
/// Valori in attesa al massimo prima di un comando.
const MAX_STACK: usize = 4096;
/// Profondità massima di parentesi e procedure.
const MAX_NESTING: usize = 64;

/// È un file PostScript scritto da Illustrator.
pub(crate) fn is_illustrator(ps: &[u8]) -> bool {
    let head = ps.get(..ps.len().min(8192)).unwrap_or_default();
    ps.starts_with(b"%!")
        && (contains(head, b"%%Creator: Adobe Illustrator")
            || contains(head, b"%AI3_")
            || contains(head, b"%AI5_"))
}

/// La pagina PDF con il disegno, o `None` se il file non ha un riquadro credibile.
pub(crate) fn to_pdf(ps: &[u8]) -> Option<Vec<u8>> {
    let header = ps.get(..ps.len().min(256 * 1024)).unwrap_or_default();
    let bbox =
        bounding_box(header, b"%%HiResBoundingBox:").or_else(|| bounding_box(header, b"%%BoundingBox:"))?;
    let start = find(ps, b"%%EndSetup")
        .or_else(|| find(ps, b"%%EndProlog"))
        .unwrap_or(0);
    let end = find_from(ps, start, b"%%PageTrailer")
        .or_else(|| find_from(ps, start, b"%%Trailer"))
        .unwrap_or(ps.len());
    let art = ps.get(start..end)?;
    let content = Translator::default().run(art);
    Some(page_pdf(bbox, &content))
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    find(hay, needle).is_some()
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn find_from(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    find(hay.get(from..)?, needle).map(|i| i + from)
}

/// `[llx lly urx ury]` dal commento DSC, se è un riquadro vero (non `(atend)`).
fn bounding_box(header: &[u8], key: &[u8]) -> Option<[f64; 4]> {
    let at = find(header, key)? + key.len();
    let rest = header.get(at..)?;
    let line_end = rest
        .iter()
        .position(|b| *b == b'\r' || *b == b'\n')
        .unwrap_or(rest.len());
    let line = std::str::from_utf8(rest.get(..line_end)?).ok()?;
    let mut n = line.split_whitespace().map(|v| v.parse::<f64>().ok());
    let b = [n.next()??, n.next()??, n.next()??, n.next()??];
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    (b.iter().all(|v| v.is_finite() && v.abs() < 1.0e7) && w > 0.0 && h > 0.0).then_some(b)
}

/// Un valore in attesa del comando che lo usa.
#[derive(Clone, Debug)]
enum Operand {
    Num(f64),
    Str,
    Array(Vec<f64>),
    Name,
}

/// I comandi di disegno tradotti in un contenuto PDF.
#[derive(Default)]
struct Translator {
    out: String,
    stack: Vec<Operand>,
    /// Dentro un tracciato composto (`*u` … `*U`): si dipinge una volta sola, alla fine.
    compound: bool,
    compound_paint: Option<&'static str>,
    /// Regola pari-dispari per i riempimenti (`1 XR`).
    even_odd: bool,
    /// Maschere aperte (`q`), da chiudere tutte alla fine.
    clips: usize,
    /// Dentro un testo (`To` … `TO`): si salta.
    text: usize,
    full: bool,
}

impl Translator {
    fn run(mut self, art: &[u8]) -> String {
        let mut lexer = Lexer { data: art, at: 0 };
        let mut ops = 0;
        while let Some(token) = lexer.next() {
            if self.full {
                break;
            }
            match token {
                Token::Num(v) => self.push(Operand::Num(v)),
                Token::Str => self.push(Operand::Str),
                Token::Array(a) => self.push(Operand::Array(a)),
                Token::Name => self.push(Operand::Name),
                Token::Op(op) => {
                    ops += 1;
                    if ops > MAX_OPS {
                        break;
                    }
                    self.op(op);
                    self.stack.clear();
                }
            }
        }
        for _ in 0..self.clips {
            self.out.push_str("Q\n");
        }
        self.out
    }

    fn push(&mut self, o: Operand) {
        if self.stack.len() < MAX_STACK {
            self.stack.push(o);
        }
    }

    /// Gli ultimi `n` numeri in attesa, se ci sono tutti.
    fn nums<const N: usize>(&self) -> Option<[f64; N]> {
        let tail = self.stack.get(self.stack.len().checked_sub(N)?..)?;
        let mut out = [0.0; N];
        for (o, v) in out.iter_mut().zip(tail) {
            match v {
                Operand::Num(n) if n.is_finite() => *o = *n,
                _ => return None,
            }
        }
        Some(out)
    }

    fn emit(&mut self, values: &[f64], op: &str) {
        for v in values {
            // Quattro decimali bastano per una miniatura; niente esponenziali nel PDF.
            let _ = write!(self.out, "{} ", Fixed(*v));
        }
        self.out.push_str(op);
        self.out.push('\n');
        if self.out.len() > MAX_CONTENT {
            self.full = true;
        }
    }

    fn op(&mut self, op: &[u8]) {
        if self.text > 0 {
            match op {
                b"To" => self.text += 1,
                b"TO" => self.text -= 1,
                _ => {}
            }
            return;
        }
        match op {
            b"To" => self.text = 1,
            b"m" => self.path::<2>("m"),
            b"l" | b"L" => self.path::<2>("l"),
            b"c" | b"C" => self.path::<6>("c"),
            b"v" | b"V" => self.path::<4>("v"),
            b"y" | b"Y" => self.path::<4>("y"),
            b"f" => self.paint("h f", "h f*"),
            b"F" => self.paint("f", "f*"),
            b"s" => self.paint("s", "s"),
            b"S" => self.paint("S", "S"),
            b"b" => self.paint("b", "b*"),
            b"B" => self.paint("B", "B*"),
            b"n" | b"N" => self.paint("n", "n"),
            b"W" => self.out.push_str(if self.even_odd { "W*\n" } else { "W\n" }),
            b"*u" => {
                self.compound = true;
                self.compound_paint = None;
            }
            b"*U" => {
                self.compound = false;
                let paint = self.compound_paint.take().unwrap_or("n");
                self.out.push_str(paint);
                self.out.push('\n');
            }
            b"q" => {
                if self.clips < MAX_NESTING {
                    self.clips += 1;
                    self.out.push_str("q\n");
                }
            }
            b"Q" => {
                if self.clips > 0 {
                    self.clips -= 1;
                    self.out.push_str("Q\n");
                }
            }
            b"XR" => self.even_odd = self.nums::<1>().is_some_and(|[v]| v > 0.5),
            b"g" | b"G" => {
                if let Some([v]) = self.nums::<1>() {
                    self.emit(&[unit(v)], if op == b"g" { "g" } else { "G" });
                }
            }
            b"k" | b"K" => {
                if let Some(c) = self.nums::<4>() {
                    self.emit(&c.map(unit), if op == b"k" { "k" } else { "K" });
                }
            }
            b"Xa" | b"XA" => {
                if let Some(c) = self.nums::<3>() {
                    self.emit(&c.map(unit), if op == b"Xa" { "rg" } else { "RG" });
                }
            }
            b"x" | b"X" => self.custom(op == b"x", false),
            b"Xx" | b"XX" | b"Xk" | b"XK" => self.custom(op == b"Xx" || op == b"Xk", true),
            b"w" => {
                if let Some([v]) = self.nums::<1>() {
                    self.emit(&[v.clamp(0.0, 1000.0)], "w");
                }
            }
            b"j" | b"J" => {
                if let Some([v]) = self.nums::<1>() {
                    let style = v.round().clamp(0.0, 2.0);
                    self.emit(&[style], if op == b"j" { "j" } else { "J" });
                }
            }
            b"M" => {
                if let Some([v]) = self.nums::<1>() {
                    self.emit(&[v.clamp(1.0, 1000.0)], "M");
                }
            }
            b"d" => {
                if let (Some(Operand::Array(dash)), Some([phase])) = (
                    self.stack.len().checked_sub(2).and_then(|i| self.stack.get(i)),
                    self.nums::<1>(),
                ) {
                    let valid =
                        dash.iter().all(|v| v.is_finite() && *v >= 0.0) && dash.iter().any(|v| *v > 0.0);
                    let mut text = String::from("[");
                    if valid {
                        for v in dash.iter().take(32) {
                            let _ = write!(text, "{} ", Fixed(*v));
                        }
                    }
                    text.push(']');
                    let _ = writeln!(self.out, "{text} {} d", Fixed(if valid { phase } else { 0.0 }));
                }
            }
            _ => {}
        }
    }

    fn path<const N: usize>(&mut self, op: &str) {
        if let Some(values) = self.nums::<N>() {
            self.emit(&values, op);
        }
    }

    fn paint(&mut self, nonzero: &'static str, even_odd: &'static str) {
        let op = if self.even_odd { even_odd } else { nonzero };
        if self.compound {
            // Il riempimento vale per tutto il tracciato composto: si dà alla fine.
            if op != "n" {
                self.compound_paint = Some(op.trim_start_matches("h "));
            }
            return;
        }
        self.out.push_str(op);
        self.out.push('\n');
    }

    /// Colore con nome (tinta piatta): `c m y k (nome) grigio x` in Illustrator 3, con un tipo
    /// (0 quadricromia, 1 RGB) dopo il grigio da Illustrator 8. Il grigio è il complemento della
    /// tinta: 0 = colore pieno.
    fn custom(&mut self, fill: bool, typed: bool) {
        let n = self.stack.len();
        let (kind, gray_at) = if typed {
            let Some(Operand::Num(t)) = n.checked_sub(1).and_then(|i| self.stack.get(i)) else {
                return;
            };
            (if *t > 0.5 { 1 } else { 0 }, n.checked_sub(2))
        } else {
            (0, n.checked_sub(1))
        };
        let Some(gray_at) = gray_at else { return };
        let Some(Operand::Num(gray)) = self.stack.get(gray_at) else {
            return;
        };
        let tint = 1.0 - unit(*gray);
        let channels = if kind == 1 { 3 } else { 4 };
        // Prima del grigio c'è il nome, prima ancora i canali.
        let Some(first) = gray_at.checked_sub(1 + channels) else {
            return;
        };
        let mut values = Vec::with_capacity(channels);
        for o in self.stack.get(first..gray_at - 1).unwrap_or_default() {
            match o {
                Operand::Num(v) if v.is_finite() => values.push(unit(*v)),
                _ => return,
            }
        }
        if kind == 1 {
            // RGB schiarito verso il bianco come una tinta.
            let rgb: Vec<f64> = values.iter().map(|v| 1.0 - (1.0 - v) * tint).collect();
            self.emit(&rgb, if fill { "rg" } else { "RG" });
        } else {
            let cmyk: Vec<f64> = values.iter().map(|v| v * tint).collect();
            self.emit(&cmyk, if fill { "k" } else { "K" });
        }
    }
}

fn unit(v: f64) -> f64 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

/// Un numero con al massimo quattro decimali, senza esponente.
struct Fixed(f64);

impl std::fmt::Display for Fixed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let v = if self.0.is_finite() {
            self.0.clamp(-1.0e7, 1.0e7)
        } else {
            0.0
        };
        let s = format!("{v:.4}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        f.write_str(if s == "-0" { "0" } else { s })
    }
}

enum Token<'a> {
    Num(f64),
    Str,
    Array(Vec<f64>),
    Name,
    Op(&'a [u8]),
}

/// Lo scanner PostScript ridotto a quello che serve: numeri, stringhe, array di numeri, nomi
/// letterali e comandi; commenti, procedure e stringhe esadecimali si saltano.
struct Lexer<'a> {
    data: &'a [u8],
    at: usize,
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | 0)
}

fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<u8> {
        self.data.get(self.at).copied()
    }

    fn next(&mut self) -> Option<Token<'a>> {
        loop {
            let b = self.peek()?;
            if is_space(b) {
                self.at += 1;
                continue;
            }
            match b {
                b'%' => self.skip_line(),
                b'(' => {
                    self.skip_string();
                    return Some(Token::Str);
                }
                b'<' => {
                    self.skip_hex();
                    return Some(Token::Str);
                }
                b'{' => self.skip_procedure(),
                b'[' => {
                    self.at += 1;
                    return Some(Token::Array(self.array()));
                }
                b'/' => {
                    self.at += 1;
                    self.word();
                    return Some(Token::Name);
                }
                b')' | b'>' | b']' | b'}' => self.at += 1,
                _ => {
                    let word = self.word();
                    if word.is_empty() {
                        self.at += 1;
                        continue;
                    }
                    return Some(match number(word) {
                        Some(v) => Token::Num(v),
                        None => Token::Op(word),
                    });
                }
            }
        }
    }

    fn word(&mut self) -> &'a [u8] {
        let start = self.at;
        while let Some(b) = self.peek() {
            if is_space(b) || is_delimiter(b) {
                break;
            }
            self.at += 1;
        }
        self.data.get(start..self.at).unwrap_or_default()
    }

    fn skip_line(&mut self) {
        while let Some(b) = self.peek() {
            self.at += 1;
            if b == b'\r' || b == b'\n' {
                break;
            }
        }
    }

    fn skip_string(&mut self) {
        self.at += 1;
        let mut depth = 1usize;
        while let Some(b) = self.peek() {
            self.at += 1;
            match b {
                b'\\' => self.at += 1,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    fn skip_hex(&mut self) {
        while let Some(b) = self.peek() {
            self.at += 1;
            if b == b'>' {
                return;
            }
        }
    }

    fn skip_procedure(&mut self) {
        let mut depth = 0usize;
        while let Some(b) = self.peek() {
            match b {
                b'{' => {
                    depth += 1;
                    self.at += 1;
                }
                b'}' => {
                    self.at += 1;
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return;
                    }
                }
                b'(' => self.skip_string(),
                b'%' => self.skip_line(),
                _ => self.at += 1,
            }
        }
    }

    /// I numeri di un array, fino a `]`; il resto dentro l'array si salta.
    fn array(&mut self) -> Vec<f64> {
        let mut out = Vec::new();
        while let Some(b) = self.peek() {
            if is_space(b) {
                self.at += 1;
                continue;
            }
            match b {
                b']' => {
                    self.at += 1;
                    break;
                }
                b'(' => self.skip_string(),
                b'{' => self.skip_procedure(),
                b'%' => self.skip_line(),
                b'[' | b'<' | b'>' | b'/' | b')' | b'}' => self.at += 1,
                _ => {
                    let word = self.word();
                    if word.is_empty() {
                        self.at += 1;
                    } else if let Some(v) = number(word)
                        && out.len() < 1024
                    {
                        out.push(v);
                    }
                }
            }
        }
        out
    }
}

/// Un numero PostScript decimale (i rari numeri in base, come `16#FF`, restano comandi ignoti).
fn number(word: &[u8]) -> Option<f64> {
    let first = *word.first()?;
    if !(first.is_ascii_digit() || matches!(first, b'-' | b'+' | b'.')) {
        return None;
    }
    std::str::from_utf8(word)
        .ok()?
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

/// Un PDF di una pagina grande come `bbox` con `content` come contenuto.
fn page_pdf(bbox: [f64; 4], content: &str) -> Vec<u8> {
    let [a, b, c, d] = bbox.map(Fixed);
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [{a} {b} {c} {d}] /Resources << >> /Contents 4 0 R >>"
        ),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
    }
    offsets.push(out.len());
    out.extend(format!("4 0 obj\n<< /Length {} >>\nstream\n", content.len()).bytes());
    out.extend(content.bytes());
    out.extend(b"\nendstream\nendobj\n");
    let xref = out.len();
    out.extend(b"xref\n0 5\n0000000000 65535 f \n");
    for off in offsets {
        out.extend(format!("{off:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").bytes());
    out
}
