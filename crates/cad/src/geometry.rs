//! Curve spezzate: archi, ellissi, polilinee con rigonfiamento, spline; ingombro e testi.

use crate::{Geometry, MAX_POINTS, Shape};

/// Segmenti al massimo per un cerchio intero: abbastanza per non vedere spigoli ingrandendo.
const CIRCLE_SEGMENTS: f64 = 256.0;

/// Un arco da `a` a `b` gradi (in senso antiorario), con il centro `c`.
pub fn arc(c: [f64; 2], r: f64, a: f64, b: f64) -> Vec<[f64; 2]> {
    if !(r.is_finite() && r > 0.0 && a.is_finite() && b.is_finite()) {
        return Vec::new();
    }
    let mut sweep = (b - a).rem_euclid(360.0);
    if sweep < 1e-9 {
        sweep = 360.0;
    }
    sample(sweep, |t| {
        let ang = (a + sweep * t).to_radians();
        [c[0] + r * ang.cos(), c[1] + r * ang.sin()]
    })
}

/// Un arco di un bordo di retino in senso orario: il file dà gli angoli riflessi.
pub fn arc_cw(c: [f64; 2], r: f64, a: f64, b: f64) -> Vec<[f64; 2]> {
    let mut p = arc(c, r, a, b);
    for q in &mut p {
        q[1] = 2.0 * c[1] - q[1];
    }
    p
}

fn sample(sweep_degrees: f64, at: impl Fn(f64) -> [f64; 2]) -> Vec<[f64; 2]> {
    let n = ((sweep_degrees / 360.0 * CIRCLE_SEGMENTS).ceil() as usize).clamp(2, 4096);
    (0..=n).map(|i| at(i as f64 / n as f64)).collect()
}

/// Un'ellisse: centro, fine dell'asse maggiore (rispetto al centro), rapporto fra gli assi e
/// angoli di partenza e fine in radianti.
pub fn ellipse(c: [f64; 2], major: [f64; 2], ratio: f64, a: f64, b: f64) -> Vec<[f64; 2]> {
    let len = major[0].hypot(major[1]);
    if !(len.is_finite() && len > 0.0 && ratio.is_finite() && a.is_finite() && b.is_finite()) {
        return Vec::new();
    }
    let minor = [-major[1] * ratio, major[0] * ratio];
    let mut sweep = (b - a).rem_euclid(std::f64::consts::TAU);
    if sweep < 1e-9 {
        sweep = std::f64::consts::TAU;
    }
    sample(sweep.to_degrees(), |t| {
        let ang = a + sweep * t;
        let (s, co) = ang.sin_cos();
        [
            c[0] + major[0] * co + minor[0] * s,
            c[1] + major[1] * co + minor[1] * s,
        ]
    })
}

/// I punti di una polilinea con il rigonfiamento di ogni lato (tangente di un quarto dell'angolo
/// dell'arco; 0 = lato dritto).
pub fn bulged(verts: &[([f64; 2], f64)], closed: bool) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::new();
    let n = verts.len();
    for i in 0..n {
        let Some(&(p, bulge)) = verts.get(i) else {
            break;
        };
        let next = if i + 1 < n {
            verts.get(i + 1)
        } else if closed {
            verts.first()
        } else {
            None
        };
        if out.is_empty() {
            out.push(p);
        }
        let Some(&(q, _)) = next else {
            break;
        };
        if bulge.abs() > 1e-9 && bulge.is_finite() && out.len() < MAX_POINTS {
            let pts = bulge_arc(p, q, bulge);
            out.extend(pts.into_iter().skip(1));
        } else {
            out.push(q);
        }
    }
    // Chiusa: l'ultimo punto ripete il primo, lo chiude il disegno.
    if closed && out.len() > 2 && out.first() == out.last() {
        out.pop();
    }
    out
}

fn bulge_arc(p: [f64; 2], q: [f64; 2], bulge: f64) -> Vec<[f64; 2]> {
    let chord = (q[0] - p[0]).hypot(q[1] - p[1]);
    if chord < 1e-12 {
        return vec![p, q];
    }
    let theta = 4.0 * bulge.atan();
    let r = chord / (2.0 * (theta / 2.0).sin()).abs();
    // Il centro sta sulla perpendicolare al centro della corda.
    let mid = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
    let dir = [(q[0] - p[0]) / chord, (q[1] - p[1]) / chord];
    let left = [-dir[1], dir[0]];
    // Rigonfiamento positivo = arco in senso antiorario. La distanza con il segno porta il centro
    // a sinistra della corda per un arco antiorario minore di mezzo giro, a destra negli altri casi.
    let half = (theta / 2.0).tan();
    if !(r.is_finite() && half.is_finite() && half.abs() > 1e-12) {
        return vec![p, q];
    }
    let d = chord / 2.0 / half;
    let c = [mid[0] + left[0] * d, mid[1] + left[1] * d];
    let a0 = (p[1] - c[1]).atan2(p[0] - c[0]);
    sample(theta.abs().to_degrees(), |t| {
        let ang = a0 + theta * t;
        [c[0] + r * ang.cos(), c[1] + r * ang.sin()]
    })
}

/// Una NURBS (spline del CAD) in punti, con l'algoritmo di de Boor. Senza nodi validi: uniformi
/// e agganciati agli estremi.
pub fn nurbs(control: &[[f64; 2]], knots: &[f64], weights: &[f64], degree: usize) -> Vec<[f64; 2]> {
    let n = control.len();
    if n <= degree || degree == 0 {
        return control.to_vec();
    }
    let needed = n + degree + 1;
    let knots: Vec<f64> = if knots.len() == needed && knots.windows(2).all(|w| w[1] >= w[0]) {
        knots.to_vec()
    } else {
        let inner = n - degree;
        (0..needed)
            .map(|i| {
                if i <= degree {
                    0.0
                } else if i >= n {
                    1.0
                } else {
                    (i - degree) as f64 / inner as f64
                }
            })
            .collect()
    };
    let w = |i: usize| {
        weights
            .get(i)
            .copied()
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(1.0)
    };
    let (Some(&t0), Some(&t1)) = (knots.get(degree), knots.get(n)) else {
        return control.to_vec();
    };
    if t1.partial_cmp(&t0) != Some(std::cmp::Ordering::Greater) {
        return control.to_vec();
    }
    let steps = (n * 16).clamp(16, 4096);
    let mut out = Vec::with_capacity(steps + 1);
    for s in 0..=steps {
        let t = t0 + (t1 - t0) * s as f64 / steps as f64;
        // L'intervallo k con knots[k] <= t < knots[k+1].
        let mut k = degree;
        while k + 1 < n && knots.get(k + 1).is_some_and(|v| *v <= t) {
            k += 1;
        }
        let mut d: Vec<[f64; 3]> = (0..=degree)
            .map(|j| {
                let i = k + j - degree;
                let p = control.get(i).copied().unwrap_or([0.0, 0.0]);
                let wi = w(i);
                [p[0] * wi, p[1] * wi, wi]
            })
            .collect();
        for r in 1..=degree {
            for j in (r..=degree).rev() {
                let i = k + j - degree;
                let (Some(&ki), Some(&kj)) = (knots.get(i), knots.get(i + degree + 1 - r)) else {
                    continue;
                };
                let alpha = if kj - ki > 0.0 { (t - ki) / (kj - ki) } else { 0.0 };
                let (Some(&a), Some(&b)) = (d.get(j - 1), d.get(j)) else {
                    continue;
                };
                if let Some(slot) = d.get_mut(j) {
                    *slot = [
                        (1.0 - alpha) * a[0] + alpha * b[0],
                        (1.0 - alpha) * a[1] + alpha * b[1],
                        (1.0 - alpha) * a[2] + alpha * b[2],
                    ];
                }
            }
        }
        if let Some(p) = d.get(degree)
            && p[2].abs() > 1e-12
        {
            out.push([p[0] / p[2], p[1] / p[2]]);
        }
    }
    out
}

/// La larghezza di una riga di testo alta `h` in Helvetica, il carattere della pagina PDF: in
/// media 0,55 dell'altezza per lettera, più stretti gli spazi e le lettere sottili.
pub fn text_width(text: &str, h: f64) -> f64 {
    text.chars()
        .map(|c| match c {
            ' ' | 'i' | 'l' | 'j' | '.' | ',' | ':' | ';' | '!' | '|' | '\'' => 0.28,
            'm' | 'w' | 'M' | 'W' => 0.85,
            c if c.is_uppercase() || c.is_ascii_digit() => 0.68,
            _ => 0.55,
        })
        .sum::<f64>()
        * h
}

/// Le righe di un MTEXT senza i codici di formattazione (`\P` a capo, `{\fArial;…}`, `\H2;`…).
pub fn mtext_lines(raw: &str) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('P') | Some('n') => lines.push(String::new()),
                Some('~') => push(&mut lines, ' '),
                Some('\\') => push(&mut lines, '\\'),
                Some('{') => push(&mut lines, '{'),
                Some('}') => push(&mut lines, '}'),
                // \U+00E9: una lettera Unicode.
                Some('U') if chars.peek() == Some(&'+') => {
                    chars.next();
                    let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        push(&mut lines, ch);
                    }
                }
                // Codici con un valore fino al punto e virgola: \f…; \H…; \C…; \T…; \Q…; \W…; \A…;
                Some('f' | 'F' | 'H' | 'C' | 'c' | 'T' | 'Q' | 'W' | 'A' | 'p') => {
                    for d in chars.by_ref() {
                        if d == ';' {
                            break;
                        }
                    }
                }
                // \S1/2; frazioni: si scrive il contenuto.
                Some('S') => {
                    for d in chars.by_ref() {
                        if d == ';' {
                            break;
                        }
                        push(&mut lines, if d == '^' || d == '#' { '/' } else { d });
                    }
                }
                // \L \l \O \o \K \k: sottolineato, sopralineato, barrato.
                Some(_) | None => {}
            },
            '{' | '}' => {}
            '%' if chars.peek() == Some(&'%') => {
                chars.next();
                match chars.next() {
                    Some('c' | 'C') => push(&mut lines, 'Ø'),
                    Some('d' | 'D') => push(&mut lines, '°'),
                    Some('p' | 'P') => push(&mut lines, '±'),
                    Some(other) => push(&mut lines, other),
                    None => {}
                }
            }
            c => push(&mut lines, c),
        }
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) && lines.len() > 1 {
        lines.pop();
    }
    if lines.iter().all(|l| l.trim().is_empty()) {
        return Vec::new();
    }
    lines
}

fn push(lines: &mut [String], c: char) {
    if let Some(l) = lines.last_mut() {
        l.push(c);
    }
}

/// L'ingombro di tutte le forme, `[x0, y0, x1, y1]`; i testi contano con la loro riga.
pub fn extents(shapes: &[Shape]) -> Option<[f64; 4]> {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut add = |p: [f64; 2]| {
        if p[0].is_finite() && p[1].is_finite() && p[0].abs() < 1e12 && p[1].abs() < 1e12 {
            b[0] = b[0].min(p[0]);
            b[1] = b[1].min(p[1]);
            b[2] = b[2].max(p[0]);
            b[3] = b[3].max(p[1]);
        }
    };
    for s in shapes {
        match &s.geometry {
            Geometry::Line { points, .. } => points.iter().for_each(|p| add(*p)),
            Geometry::Fill { contours } => contours.iter().flatten().for_each(|p| add(*p)),
            Geometry::Text(t) => {
                let w = text_width(&t.text, t.height);
                let (sn, c) = t.angle.to_radians().sin_cos();
                add(t.at);
                add([t.at[0] + w * c, t.at[1] + w * sn]);
                add([t.at[0] - t.height * sn, t.at[1] + t.height * c]);
            }
        }
    }
    (b[0] <= b[2] && b[1] <= b[3]).then_some(b)
}

/// Due punti uguali entro un'inezia.
pub fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}
