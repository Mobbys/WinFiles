//! Geometria per il nesting delle sagome: pulizia, semplificazione, scomposizione in parti
//! convesse e somma di Minkowski di poligoni convessi (dal pannello per Illustrator,
//! `geometry.js`).
//!
//! I poligoni sono punti interi (centesimi di millimetro). "Positivo" vuol dire area con segno
//! maggiore di zero con la formula di Gauss, la stessa convenzione dei contorni esterni di
//! i_overlay.

use i_overlay::i_float::int::point::IntPoint;

pub(crate) type P = IntPoint<i32>;
pub(crate) type Poly = Vec<P>;

/// Il rettangolo che contiene dei poligoni.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bounds {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

impl Bounds {
    pub fn width(&self) -> i32 {
        self.max_x.saturating_sub(self.min_x)
    }

    pub fn height(&self) -> i32 {
        self.max_y.saturating_sub(self.min_y)
    }
}

/// Area con il segno, doppia (intera, senza arrotondamenti).
pub(crate) fn area2(p: &[P]) -> i64 {
    let n = p.len();
    let mut a: i64 = 0;
    for i in 0..n {
        let (Some(u), Some(v)) = (p.get(i), p.get((i + 1) % n)) else {
            continue;
        };
        a = a.saturating_add(
            i64::from(u.x)
                .saturating_mul(i64::from(v.y))
                .saturating_sub(i64::from(v.x).saturating_mul(i64::from(u.y))),
        );
    }
    a
}

pub(crate) fn area(p: &[P]) -> f64 {
    area2(p) as f64 / 2.0
}

fn cross(o: P, a: P, b: P) -> i64 {
    let (ax, ay) = (i64::from(a.x) - i64::from(o.x), i64::from(a.y) - i64::from(o.y));
    let (bx, by) = (i64::from(b.x) - i64::from(o.x), i64::from(b.y) - i64::from(o.y));
    ax.saturating_mul(by).saturating_sub(ay.saturating_mul(bx))
}

pub(crate) fn positive(p: &[P]) -> Poly {
    let mut out = p.to_vec();
    if area2(p) < 0 {
        out.reverse();
    }
    out
}

pub(crate) fn bounds<'a>(polys: impl IntoIterator<Item = &'a Poly>) -> Bounds {
    let mut b = Bounds {
        min_x: i32::MAX,
        min_y: i32::MAX,
        max_x: i32::MIN,
        max_y: i32::MIN,
    };
    for p in polys {
        for q in p {
            b.min_x = b.min_x.min(q.x);
            b.min_y = b.min_y.min(q.y);
            b.max_x = b.max_x.max(q.x);
            b.max_y = b.max_y.max(q.y);
        }
    }
    b
}

/// Toglie i punti doppi e quelli allineati (rompono la somma di Minkowski).
pub(crate) fn clean(p: &[P]) -> Poly {
    let mut out: Poly = Vec::with_capacity(p.len());
    for &q in p {
        if out.last() == Some(&q) {
            continue;
        }
        out.push(q);
    }
    if out.len() > 1 && out.first() == out.last() {
        out.pop();
    }
    let mut changed = true;
    while changed && out.len() > 3 {
        changed = false;
        let n = out.len();
        for k in 0..n {
            let (Some(&a), Some(&b), Some(&c)) = (out.get((k + n - 1) % n), out.get(k), out.get((k + 1) % n))
            else {
                continue;
            };
            if cross(a, b, c) == 0 {
                out.remove(k);
                changed = true;
                break;
            }
        }
    }
    out
}

/// Ramer–Douglas–Peucker su un poligono chiuso, dai due punti più lontani (così non dipende dal
/// punto di partenza).
pub(crate) fn simplify(p: &[P], tol: f64) -> Poly {
    if p.len() <= 4 {
        return p.to_vec();
    }
    let Some(&first) = p.first() else {
        return Vec::new();
    };
    let mut far = 0usize;
    let mut best = -1.0f64;
    for (i, q) in p.iter().enumerate().skip(1) {
        let d = dist2(*q, first);
        if d > best {
            best = d;
            far = i;
        }
    }
    let a = rdp(p.get(..=far).unwrap_or_default(), tol);
    let mut tail: Poly = p.get(far..).unwrap_or_default().to_vec();
    tail.push(first);
    let b = rdp(&tail, tol);
    let mut out: Poly = a.get(..a.len().saturating_sub(1)).unwrap_or_default().to_vec();
    out.extend_from_slice(b.get(..b.len().saturating_sub(1)).unwrap_or_default());
    if out.len() >= 3 { out } else { p.to_vec() }
}

fn dist2(a: P, b: P) -> f64 {
    let dx = f64::from(a.x) - f64::from(b.x);
    let dy = f64::from(a.y) - f64::from(b.y);
    dx * dx + dy * dy
}

fn rdp(pts: &[P], tol: f64) -> Poly {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let last = pts.len() - 1;
    let mut keep = vec![false; pts.len()];
    if let Some(k) = keep.first_mut() {
        *k = true;
    }
    if let Some(k) = keep.last_mut() {
        *k = true;
    }
    let mut stack = vec![(0usize, last)];
    let tol2 = tol * tol;
    while let Some((s, e)) = stack.pop() {
        let (Some(&a), Some(&b)) = (pts.get(s), pts.get(e)) else {
            continue;
        };
        let (ax, ay) = (f64::from(a.x), f64::from(a.y));
        let (dx, dy) = (f64::from(b.x) - ax, f64::from(b.y) - ay);
        let len2 = dx * dx + dy * dy;
        let mut max_d = -1.0;
        let mut idx = s;
        for i in s + 1..e {
            let Some(&q) = pts.get(i) else { continue };
            let d = if len2 == 0.0 {
                dist2(q, a)
            } else {
                let c = (f64::from(q.x) - ax) * dy - (f64::from(q.y) - ay) * dx;
                c * c / len2
            };
            if d > max_d {
                max_d = d;
                idx = i;
            }
        }
        if max_d > tol2 {
            if let Some(k) = keep.get_mut(idx) {
                *k = true;
            }
            stack.push((s, idx));
            stack.push((idx, e));
        }
    }
    pts.iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(q, _)| *q)
        .collect()
}

pub(crate) fn convex_hull(points: &[P]) -> Poly {
    let mut pts = points.to_vec();
    pts.sort_by(|a, b| a.x.cmp(&b.x).then(a.y.cmp(&b.y)));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let mut lower: Poly = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Poly = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0 {
            upper.pop();
        }
        upper.push(p);
    }
    upper.pop();
    lower.pop();
    lower.extend(upper);
    lower
}

fn is_convex(p: &[P]) -> bool {
    let n = p.len();
    (0..n).all(|i| match (p.get(i), p.get((i + 1) % n), p.get((i + 2) % n)) {
        (Some(&a), Some(&b), Some(&c)) => cross(a, b, c) >= 0,
        _ => true,
    })
}

/// Triangolazione "ear clipping" di un poligono semplice positivo (indici). None se il poligono
/// non è semplice.
fn triangulate(poly: &[P]) -> Option<Vec<[usize; 3]>> {
    let mut idx: Vec<usize> = (0..poly.len()).collect();
    let mut tris = Vec::new();
    let mut guard = 0usize;
    while idx.len() > 3 && guard < 100_000 {
        guard += 1;
        let n = idx.len();
        let mut found = false;
        for k in 0..n {
            let ia = *idx.get((k + n - 1) % n)?;
            let ib = *idx.get(k)?;
            let ic = *idx.get((k + 1) % n)?;
            let (a, b, c) = (*poly.get(ia)?, *poly.get(ib)?, *poly.get(ic)?);
            if cross(a, b, c) <= 0 {
                continue;
            }
            let inside = idx.iter().any(|&ij| {
                if ij == ia || ij == ib || ij == ic {
                    return false;
                }
                let Some(&q) = poly.get(ij) else { return false };
                cross(a, b, q) >= 0 && cross(b, c, q) >= 0 && cross(c, a, q) >= 0
            });
            if inside {
                continue;
            }
            tris.push([ia, ib, ic]);
            idx.remove(k);
            found = true;
            break;
        }
        if !found {
            return None;
        }
    }
    if let [a, b, c] = idx.as_slice() {
        tris.push([*a, *b, *c]);
    }
    Some(tris)
}

/// Scomposizione in parti convesse (Hertel–Mehlhorn): triangola e unisce i triangoli vicini
/// finché il risultato resta convesso. Se il poligono è difettoso ripiega sull'involucro
/// convesso, più grande della sagoma e quindi sempre sicuro.
pub(crate) fn convex_parts(poly: &[P]) -> Vec<Poly> {
    let poly = clean(&positive(poly));
    if poly.len() < 3 {
        return Vec::new();
    }
    if is_convex(&poly) {
        return vec![poly];
    }
    let Some(tris) = triangulate(&poly) else {
        return vec![convex_hull(&poly)];
    };
    let tri_area = |t: &[usize; 3]| -> i64 {
        match (poly.get(t[0]), poly.get(t[1]), poly.get(t[2])) {
            (Some(&a), Some(&b), Some(&c)) => area2(&[a, b, c]),
            _ => 0,
        }
    };
    let sum: i64 = tris.iter().map(tri_area).sum();
    let whole = area2(&poly);
    if (sum - whole).abs() as f64 > whole.abs() as f64 * 1e-6 + 2.0 {
        return vec![convex_hull(&poly)];
    }
    let mut parts: Vec<Vec<usize>> = tris.iter().map(|t| t.to_vec()).collect();
    let pts = |ix: &[usize]| -> Poly { ix.iter().filter_map(|&n| poly.get(n).copied()).collect() };
    loop {
        let mut merged = None;
        'search: for i in 0..parts.len() {
            for j in i + 1..parts.len() {
                let (a_part, b_part) = (&parts[i], &parts[j]);
                for a in 0..a_part.len() {
                    let u = a_part[a];
                    let v = a_part[(a + 1) % a_part.len()];
                    for b in 0..b_part.len() {
                        if b_part[b] != v || b_part[(b + 1) % b_part.len()] != u {
                            continue;
                        }
                        let mut res = Vec::with_capacity(a_part.len() + b_part.len() - 2);
                        for k in 0..a_part.len() {
                            res.push(a_part[(a + 1 + k) % a_part.len()]);
                        }
                        for m in 2..b_part.len() {
                            res.push(b_part[(b + m) % b_part.len()]);
                        }
                        if is_convex(&pts(&res)) {
                            merged = Some((i, j, res));
                            break 'search;
                        }
                    }
                }
            }
        }
        match merged {
            Some((i, j, res)) => {
                parts[i] = res;
                parts.remove(j);
            }
            None => break,
        }
    }
    parts
        .iter()
        .map(|ix| clean(&pts(ix)))
        .filter(|p| p.len() >= 3)
        .collect()
}

fn lowest(p: &[P]) -> usize {
    let mut k = 0;
    for (i, q) in p.iter().enumerate() {
        let Some(best) = p.get(k) else { break };
        if q.y < best.y || (q.y == best.y && q.x < best.x) {
            k = i;
        }
    }
    k
}

/// Somma di Minkowski di due poligoni convessi positivi, in tempo lineare.
pub(crate) fn minkowski_convex(a: &[P], b: &[P]) -> Poly {
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 {
        return Vec::new();
    }
    let (ia, ib) = (lowest(a), lowest(b));
    let mut out = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0usize, 0usize);
    while i < n || j < m {
        let pa = a[(ia + i) % n];
        let pb = b[(ib + j) % m];
        out.push(P::new(pa.x.saturating_add(pb.x), pa.y.saturating_add(pb.y)));
        let pa2 = a[(ia + i + 1) % n];
        let pb2 = b[(ib + j + 1) % m];
        let c = (i64::from(pa2.x) - i64::from(pa.x)) * (i64::from(pb2.y) - i64::from(pb.y))
            - (i64::from(pa2.y) - i64::from(pa.y)) * (i64::from(pb2.x) - i64::from(pb.x));
        if j >= m || (i < n && c > 0) {
            i += 1;
        } else if i >= n || c < 0 {
            j += 1;
        } else {
            i += 1;
            j += 1;
        }
    }
    clean(&out)
}

/// Rotazione in gradi attorno all'origine (y verso il basso: senso orario a schermo).
pub(crate) fn rotate(p: &[P], deg: f64) -> Poly {
    if deg == 0.0 {
        return p.to_vec();
    }
    let (s, c) = deg.to_radians().sin_cos();
    p.iter()
        .map(|q| {
            let (x, y) = (f64::from(q.x), f64::from(q.y));
            P::new(round(x * c - y * s), round(x * s + y * c))
        })
        .collect()
}

/// Specchiatura orizzontale (x → -x): inverte il verso del poligono.
pub(crate) fn mirror(p: &[P]) -> Poly {
    p.iter().map(|q| P::new(q.x.saturating_neg(), q.y)).collect()
}

pub(crate) fn translate(p: &[P], dx: i32, dy: i32) -> Poly {
    p.iter()
        .map(|q| P::new(q.x.saturating_add(dx), q.y.saturating_add(dy)))
        .collect()
}

pub(crate) fn round(v: f64) -> i32 {
    // I valori arrivano già limitati (vedi `MAX_MM`): la conversione non satura mai davvero.
    v.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poly(pts: &[(i32, i32)]) -> Poly {
        pts.iter().map(|&(x, y)| P::new(x, y)).collect()
    }

    #[test]
    fn le_parti_convesse_coprono_la_forma() {
        let l = poly(&[(0, 0), (10, 0), (10, 30), (30, 30), (30, 40), (0, 40)]);
        let parts = convex_parts(&l);
        assert!(parts.len() >= 2);
        let sum: i64 = parts.iter().map(|p| area2(p)).sum();
        assert_eq!(sum, area2(&positive(&l)));
        assert!(parts.iter().all(|p| is_convex(p)));
    }

    #[test]
    fn somma_di_due_quadrati() {
        let a = poly(&[(0, 0), (10, 0), (10, 10), (0, 10)]);
        let b = poly(&[(0, 0), (5, 0), (5, 5), (0, 5)]);
        let s = minkowski_convex(&a, &b);
        assert_eq!(area2(&s), 2 * 15 * 15);
    }

    #[test]
    fn la_semplificazione_toglie_i_punti_inutili() {
        let mut c = Vec::new();
        for i in 0..100 {
            let a = f64::from(i) / 100.0 * std::f64::consts::TAU;
            c.push(P::new(round(1000.0 * a.cos()), round(1000.0 * a.sin())));
        }
        let s = simplify(&c, 30.0);
        assert!(s.len() < 30 && s.len() >= 8, "{}", s.len());
    }
}
