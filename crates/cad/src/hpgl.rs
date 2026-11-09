//! I file dei plotter da taglio e da disegno (PLT, HPGL e HPGL/2): penna su, penna giù,
//! coordinate in unità del plotter (40 per millimetro), cerchi e archi. Diventano un [`Drawing`]
//! con un layer per penna, così la scheda Visualizza li mostra a misura vera come i DXF.
//!
//! Il file è ostile: i comandi sconosciuti si saltano, i numeri fuori misura si ignorano e i punti
//! hanno un tetto.

use crate::geometry;
use crate::{CadError, Drawing, Geometry, Layer, Paint, Shape, Units};

/// Millimetri per unità del plotter (1 plu = 0,025 mm).
pub const PLU_MM: f64 = 0.025;
/// Punti massimi del disegno.
const MAX_POINTS: usize = 5_000_000;
/// Penne massime (layer).
const MAX_PENS: usize = 256;
/// Coordinata massima accettata, in unità del plotter (100 km).
const MAX_COORD: f64 = 4.0e9;

/// I colori delle penne 1-8 come li usano i programmi di taglio; la 1 è il nero del CAD.
const PEN_COLORS: [[u8; 3]; 8] = [
    [255, 255, 255],
    [230, 30, 30],
    [30, 170, 60],
    [40, 90, 230],
    [0, 180, 200],
    [200, 40, 200],
    [230, 200, 0],
    [255, 140, 0],
];

/// Il file sembra un HPGL: comincia (anche dopo un preambolo PCL) con un comando noto.
pub fn looks_like_hpgl(bytes: &[u8]) -> bool {
    let head: Vec<u8> = bytes
        .iter()
        .take(4096)
        .copied()
        .filter(|b| b.is_ascii_alphabetic())
        .take(64)
        .map(|b| b.to_ascii_uppercase())
        .collect();
    ["IN", "PU", "PD", "PA", "SP", "DF", "VS", "CT"]
        .iter()
        .any(|c| head.windows(2).any(|w| w == c.as_bytes()))
}

struct Reader {
    /// Le forme finite, per penna.
    shapes: Vec<Shape>,
    pens: Vec<usize>,
    pen: usize,
    down: bool,
    relative: bool,
    at: [f64; 2],
    line: Vec<[f64; 2]>,
    points: usize,
    skipped: std::collections::BTreeMap<String, usize>,
}

impl Reader {
    fn layer(&mut self, pen: usize) -> usize {
        match self.pens.iter().position(|&p| p == pen) {
            Some(i) => i,
            None if self.pens.len() < MAX_PENS => {
                self.pens.push(pen);
                self.pens.len() - 1
            }
            None => 0,
        }
    }

    fn flush(&mut self) {
        if self.line.len() >= 2 {
            let points = std::mem::take(&mut self.line);
            let closed = points.len() > 2
                && points
                    .first()
                    .zip(points.last())
                    .is_some_and(|(a, b)| geometry::near(*a, *b));
            let layer = self.layer(self.pen);
            self.shapes.push(Shape {
                layer,
                paint: pen_paint(self.pen),
                weight: None,
                geometry: Geometry::Line { points, closed },
            });
        } else {
            self.line.clear();
        }
    }

    fn add(&mut self, p: [f64; 2]) -> Result<(), CadError> {
        self.points += 1;
        if self.points > MAX_POINTS {
            return Err(CadError::Dxf(format!("troppi punti (oltre {MAX_POINTS})")));
        }
        self.line.push(p);
        Ok(())
    }

    /// Una mossa verso `p` con la penna com'è.
    fn move_to(&mut self, p: [f64; 2]) -> Result<(), CadError> {
        if self.down {
            if self.line.is_empty() {
                let at = self.at;
                self.add(at)?;
            }
            self.add(p)?;
        } else {
            self.flush();
        }
        self.at = p;
        Ok(())
    }

    /// Le coppie di numeri di PU, PD, PA, PR.
    fn moves(&mut self, nums: &[f64]) -> Result<(), CadError> {
        for pair in nums.chunks_exact(2) {
            let (Some(&x), Some(&y)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            let p = if self.relative {
                [self.at[0] + x, self.at[1] + y]
            } else {
                [x, y]
            };
            if p[0].abs() > MAX_COORD || p[1].abs() > MAX_COORD {
                continue;
            }
            self.move_to(p)?;
        }
        Ok(())
    }

    /// Un arco attorno a `c` per `sweep` gradi, a partire dal punto attuale.
    fn arc(&mut self, c: [f64; 2], sweep: f64) -> Result<(), CadError> {
        let r = ((self.at[0] - c[0]).powi(2) + (self.at[1] - c[1]).powi(2)).sqrt();
        if !(r.is_finite() && sweep.is_finite()) || r <= 0.0 || r > MAX_COORD {
            return Ok(());
        }
        let a = (self.at[1] - c[1]).atan2(self.at[0] - c[0]).to_degrees();
        let sweep = sweep.clamp(-360.0, 360.0);
        if sweep.abs() < 1e-9 {
            return Ok(());
        }
        let pts = if sweep > 0.0 {
            geometry::arc(c, r, a, a + sweep)
        } else {
            // In senso orario: lo stesso arco percorso al contrario.
            let mut p = geometry::arc(c, r, a + sweep, a);
            p.reverse();
            p
        };
        for p in pts.into_iter().skip(1) {
            self.move_to(p)?;
        }
        Ok(())
    }

    fn circle(&mut self, r: f64) -> Result<(), CadError> {
        if !r.is_finite() || r == 0.0 || r.abs() > MAX_COORD {
            return Ok(());
        }
        self.flush();
        let pts = geometry::arc(self.at, r.abs(), 0.0, 360.0);
        if pts.len() < 3 {
            return Ok(());
        }
        let layer = self.layer(self.pen);
        self.points = self.points.saturating_add(pts.len());
        if self.points > MAX_POINTS {
            return Err(CadError::Dxf(format!("troppi punti (oltre {MAX_POINTS})")));
        }
        self.shapes.push(Shape {
            layer,
            paint: pen_paint(self.pen),
            weight: None,
            geometry: Geometry::Line {
                points: pts,
                closed: true,
            },
        });
        Ok(())
    }
}

fn pen_paint(pen: usize) -> Paint {
    let i = pen.saturating_sub(1) % PEN_COLORS.len();
    Paint {
        rgb: PEN_COLORS.get(i).copied().unwrap_or([255, 255, 255]),
        aci7: i == 0,
    }
}

/// I numeri dei parametri di un comando: "100,200 300 -4.5".
fn numbers(params: &str) -> Vec<f64> {
    params
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f64>().ok())
        .filter(|n| n.is_finite())
        .collect()
}

/// Legge un file HPGL.
pub fn parse(bytes: &[u8]) -> Result<Drawing, CadError> {
    let mut r = Reader {
        shapes: Vec::new(),
        pens: Vec::new(),
        pen: 1,
        down: false,
        relative: false,
        at: [0.0, 0.0],
        line: Vec::new(),
        points: 0,
        skipped: std::collections::BTreeMap::new(),
    };
    let mut i = 0;
    let n = bytes.len();
    while i < n {
        let Some(&b) = bytes.get(i) else { break };
        // Preamboli PCL ed escape: ESC fino alla prima lettera maiuscola che chiude la sequenza.
        if b == 0x1b {
            i += 1;
            while let Some(&c) = bytes.get(i) {
                i += 1;
                if c.is_ascii_uppercase() || c == b'@' {
                    break;
                }
            }
            continue;
        }
        // Righe di PJL (@PJL ...): comandi della stampante, non disegno.
        if b == b'@' {
            while let Some(&c) = bytes.get(i) {
                i += 1;
                if c == b'\n' {
                    break;
                }
            }
            continue;
        }
        if !b.is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let Some(&b2) = bytes.get(i + 1) else { break };
        if !b2.is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let cmd = [b.to_ascii_uppercase(), b2.to_ascii_uppercase()];
        i += 2;
        // LB: testo fino al terminatore (ETX); si salta.
        if &cmd == b"LB" {
            while let Some(&c) = bytes.get(i) {
                i += 1;
                if c == 0x03 {
                    break;
                }
            }
            *r.skipped.entry("testi (LB)".into()).or_default() += 1;
            continue;
        }
        let start = i;
        while let Some(&c) = bytes.get(i) {
            if c == b';' || c.is_ascii_alphabetic() || c == 0x1b {
                break;
            }
            i += 1;
        }
        let params = String::from_utf8_lossy(bytes.get(start..i).unwrap_or_default());
        if bytes.get(i) == Some(&b';') {
            i += 1;
        }
        let nums = numbers(&params);
        match &cmd {
            b"IN" | b"DF" => {
                r.flush();
                r.down = false;
                r.relative = false;
                r.at = [0.0, 0.0];
            }
            b"SP" => {
                r.flush();
                r.pen = nums.first().map_or(1, |&p| p.clamp(0.0, 9999.0) as usize).max(1);
            }
            b"PU" => {
                r.down = false;
                r.flush();
                r.moves(&nums)?;
            }
            b"PD" => {
                r.down = true;
                r.moves(&nums)?;
            }
            b"PA" => {
                r.relative = false;
                r.moves(&nums)?;
            }
            b"PR" => {
                r.relative = true;
                r.moves(&nums)?;
            }
            b"AA" | b"AR" => {
                if let (Some(&x), Some(&y), Some(&sweep)) = (nums.first(), nums.get(1), nums.get(2)) {
                    let c = if &cmd == b"AR" {
                        [r.at[0] + x, r.at[1] + y]
                    } else {
                        [x, y]
                    };
                    r.arc(c, sweep)?;
                }
            }
            b"CI" => {
                if let Some(&rad) = nums.first() {
                    r.circle(rad)?;
                }
            }
            // Comandi che non cambiano il disegno: velocità, forza, tipo di linea, finestra.
            b"VS" | b"FS" | b"LT" | b"PW" | b"IP" | b"SC" | b"IW" | b"RO" | b"CT" | b"DT" | b"SI" | b"SS"
            | b"SR" | b"BP" | b"PS" | b"NP" | b"PC" | b"MC" | b"WU" | b"TR" | b"PG" => {}
            other => {
                let name = String::from_utf8_lossy(other).into_owned();
                *r.skipped.entry(name).or_default() += 1;
            }
        }
    }
    r.flush();
    if r.shapes.is_empty() {
        return Err(CadError::Empty);
    }
    let extents = geometry::extents(&r.shapes).ok_or(CadError::Empty)?;
    let mut layers: Vec<Layer> = r
        .pens
        .iter()
        .map(|&pen| {
            let paint = pen_paint(pen);
            Layer {
                name: format!("Penna {pen}"),
                color: paint.rgb,
                aci7: paint.aci7,
                off: false,
                shapes: 0,
            }
        })
        .collect();
    for s in &r.shapes {
        if let Some(l) = layers.get_mut(s.layer) {
            l.shapes += 1;
        }
    }
    Ok(Drawing {
        layers,
        shapes: r.shapes,
        units: Units {
            mm: Some(PLU_MM),
            name: "unità del plotter (0,025 mm)",
        },
        extents,
        version: "HPGL (plotter)".into(),
        skipped: r.skipped.into_iter().collect(),
        notes: Vec::new(),
    })
}
