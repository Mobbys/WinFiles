//! Linee di taglio comuni, per il laser (dal pannello per Illustrator, `cutlines.js`): quando due
//! pezzi sono affiancati senza distanza, il tratto che condividono si taglia una volta sola.
//!
//! I contorni dei pezzi già disposti si passano uno dopo l'altro: di ogni lato si tiene solo la
//! parte che nessun lato già accettato copre (parallelo entro 1° e a meno di `tol` mm), e i
//! tratti rimasti consecutivi dello stesso contorno si uniscono in polilinee.

use std::collections::HashMap;

/// Due lati sono paralleli se l'angolo fra loro è sotto 1°.
const SIN_PARALLEL: f64 = 0.017_452_406_437_283_51;
/// I tratti più corti (mm) si scartano.
pub const MIN_LEN: f64 = 0.3;
/// La distanza (mm) entro cui due lati paralleli sono lo stesso taglio.
pub const TOLERANCE: f64 = 0.6;
/// Il lato delle celle della griglia che trova i lati vicini (mm).
const CELL: f64 = 20.0;
/// Le celle che un lato solo può occupare: oltre, si usa una griglia più grossa per lui.
const MAX_CELLS: i64 = 4096;

/// Le linee da tagliare di un foglio e la lunghezza del taglio prima e dopo (mm).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CutLines {
    pub lines: Vec<Vec<[f64; 2]>>,
    pub before: f64,
    pub after: f64,
}

#[derive(Clone, Copy)]
struct Segment {
    a: [f64; 2],
    b: [f64; 2],
}

/// La parte [t0, t1] del lato a→b che `u` copre già.
fn covered(a: [f64; 2], b: [f64; 2], u: &Segment, tol: f64) -> Option<[f64; 2]> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let (ux, uy) = (u.b[0] - u.a[0], u.b[1] - u.a[1]);
    let ulen = ux.hypot(uy);
    if len2 <= 0.0 || ulen <= 0.0 {
        return None;
    }
    let len = len2.sqrt();
    if (dx * uy - dy * ux).abs() / (len * ulen) > SIN_PARALLEL {
        return None;
    }
    let dist = |p: [f64; 2]| ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len;
    if dist(u.a) > tol || dist(u.b) > tol {
        return None;
    }
    let t = |p: [f64; 2]| ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2;
    let (t1, t2) = (t(u.a), t(u.b));
    let lo = t1.min(t2).max(0.0);
    let hi = t1.max(t2).min(1.0);
    (hi - lo > 1e-9).then_some([lo, hi])
}

/// Le parti di [0, 1] che gli intervalli non coprono.
fn subtract(mut intervals: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    intervals.sort_by(|p, q| p[0].total_cmp(&q[0]));
    let mut free = Vec::new();
    let mut t = 0.0;
    for iv in intervals {
        if iv[0] > t {
            free.push([t, iv[0]]);
        }
        t = f64::max(t, iv[1]);
    }
    if t < 1.0 {
        free.push([t, 1.0]);
    }
    free
}

fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// La lunghezza di una polilinea.
pub fn length(line: &[[f64; 2]]) -> f64 {
    line.windows(2)
        .map(|w| match w {
            [a, b] => dist(*a, *b),
            _ => 0.0,
        })
        .sum()
}

/// I lati accettati, divisi per celle per trovare in fretta quelli vicini.
#[derive(Default)]
struct Grid {
    segments: Vec<Segment>,
    cells: HashMap<(i64, i64), Vec<usize>>,
}

impl Grid {
    fn range(a: [f64; 2], b: [f64; 2], tol: f64) -> Option<[i64; 4]> {
        let cell = |v: f64| (v / CELL).floor();
        let r = [
            cell(a[0].min(b[0]) - tol),
            cell(a[1].min(b[1]) - tol),
            cell(a[0].max(b[0]) + tol),
            cell(a[1].max(b[1]) + tol),
        ];
        if !r.iter().all(|v| v.is_finite() && v.abs() < 1e9) {
            return None;
        }
        let r = r.map(|v| v as i64);
        let cells = (r[2] - r[0] + 1).saturating_mul(r[3] - r[1] + 1);
        (cells <= MAX_CELLS).then_some(r)
    }

    fn add(&mut self, s: Segment, tol: f64) {
        let i = self.segments.len();
        self.segments.push(s);
        // Un lato lunghissimo va nella cella "fuori griglia", guardata sempre.
        let Some(r) = Self::range(s.a, s.b, tol) else {
            self.cells.entry((i64::MIN, i64::MIN)).or_default().push(i);
            return;
        };
        for x in r[0]..=r[2] {
            for y in r[1]..=r[3] {
                self.cells.entry((x, y)).or_default().push(i);
            }
        }
    }

    /// I lati che potrebbero coprire a→b (ognuno una volta).
    fn near(&self, a: [f64; 2], b: [f64; 2], tol: f64) -> Vec<usize> {
        let mut out: Vec<usize> = self.cells.get(&(i64::MIN, i64::MIN)).cloned().unwrap_or_default();
        match Self::range(a, b, tol) {
            Some(r) => {
                for x in r[0]..=r[2] {
                    for y in r[1]..=r[3] {
                        if let Some(v) = self.cells.get(&(x, y)) {
                            out.extend(v);
                        }
                    }
                }
            }
            None => out.extend(0..self.segments.len()),
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Le linee da tagliare di un foglio: `pieces` sono i contorni chiusi di ogni pezzo, già al
/// loro posto sul foglio (mm). Un pezzo non copre sé stesso.
pub fn common_lines(pieces: &[Vec<Vec<[f64; 2]>>], tol: f64) -> CutLines {
    let tol = if tol.is_finite() && tol >= 0.0 {
        tol
    } else {
        TOLERANCE
    };
    let mut grid = Grid::default();
    let mut lines: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut before = 0.0;
    for rings in pieces {
        let mut mine: Vec<Segment> = Vec::new();
        for ring in rings {
            let pts: Vec<[f64; 2]> = ring
                .iter()
                .copied()
                .filter(|p| p[0].is_finite() && p[1].is_finite())
                .collect();
            if pts.len() < 2 {
                continue;
            }
            // Il tratto in corso: la polilinea in `lines`, il lato dove finisce, se arriva in fondo.
            let mut current: Option<(usize, usize, bool)> = None;
            let n = pts.len();
            for i in 0..n {
                let (Some(&a), Some(&b)) = (pts.get(i), pts.get((i + 1) % n)) else {
                    continue;
                };
                before += dist(a, b);
                mine.push(Segment { a, b });
                let ivs: Vec<[f64; 2]> = grid
                    .near(a, b, tol)
                    .into_iter()
                    .filter_map(|k| grid.segments.get(k))
                    .filter_map(|u| covered(a, b, u, tol))
                    .collect();
                let free = subtract(ivs);
                if free.is_empty() {
                    current = None;
                }
                for iv in free {
                    let (p0, p1) = (lerp(a, b, iv[0]), lerp(a, b, iv[1]));
                    if iv[1] - iv[0] < 1.0 && dist(p0, p1) < MIN_LEN {
                        current = None;
                        continue;
                    }
                    // Il seguito del contorno si attacca al tratto prima.
                    let joined = match current {
                        Some((line, last, full)) if last + 1 == i && full && iv[0] == 0.0 => {
                            lines.get_mut(line).map(|l| l.push(p1)).is_some().then_some(line)
                        }
                        _ => None,
                    };
                    let line = joined.unwrap_or_else(|| {
                        lines.push(vec![p0, p1]);
                        lines.len() - 1
                    });
                    current = Some((line, i, iv[1] == 1.0));
                }
            }
        }
        for s in mine {
            grid.add(s, tol);
        }
    }
    lines.retain(|l| length(l) >= MIN_LEN);
    let after = lines.iter().map(|l| length(l)).sum();
    CutLines { lines, before, after }
}
