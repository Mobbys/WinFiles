//! I disegni DXF e DWG per la scheda Visualizza (ADR 0033): solo da vedere e stampare.
//!
//! Il file si legge con il lettore di CADCraft (`cadcraft-dxf`, DXF da R12 a 2018, testo e
//! binari); un DWG (da R13 a 2018) diventa prima un DXF con il ponte di CADCraft (`cadcraft-dwg`,
//! che usa acadrust in un processo a parte). Le entità si traducono qui in forme semplici in mm
//! (linee, curve spezzate, riempimenti, testi) e il disegno diventa una pagina PDF vettoriale, che
//! l'app disegna, ingrandisce e stampa come tutti gli altri PDF.
//!
//! Si leggono linee, polilinee (anche con archi), cerchi, archi, ellissi, spline, testi e testi
//! multiriga, blocchi (anche annidati e in serie), quote (il loro blocco), retini pieni e bordi
//! dei retini, solidi e facce. Tutto il resto si conta e si salta, senza errori.
//!
//! I numeri vengono da un file che può essere rovinato o ostile: tetti al numero di forme, alla
//! profondità dei blocchi e ai punti di ogni curva, nessun indice senza controllo.

use std::collections::HashMap;
use std::path::Path;

use cadcraft_color::{Rgb, aci_rgb};
use cadcraft_dxf::{Tag, records, sections};

mod geometry;
mod pdf;

pub use pdf::{MAX_PAGE_MM, PdfOptions, TextLine, text_page};

/// File più grande che si apre.
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
/// Forme al massimo in un disegno (i blocchi in serie si moltiplicano).
const MAX_SHAPES: usize = 2_000_000;
/// Blocchi dentro blocchi.
const MAX_DEPTH: usize = 16;
/// Punti al massimo di una curva spezzata.
const MAX_POINTS: usize = 100_000;
/// Copie al massimo di un blocco in serie (righe × colonne).
const MAX_ARRAY: i64 = 10_000;

#[derive(Debug, thiserror::Error)]
pub enum CadError {
    #[error("{}", io_text(.0))]
    Io(#[from] std::io::Error),
    #[error("il file è troppo grande ({0} byte)")]
    TooLarge(u64),
    #[error("DXF non leggibile: {0}")]
    Dxf(String),
    #[error("DWG non leggibile: {0}")]
    Dwg(String),
    #[error("questo DWG è di una versione troppo vecchia (prima di AutoCAD R13): salvalo di nuovo da AutoCAD")]
    OldDwg,
    #[error("il disegno è vuoto")]
    Empty,
}

fn io_text(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "file non trovato".into(),
        std::io::ErrorKind::PermissionDenied => "accesso al file negato".into(),
        _ => format!("impossibile leggere il file: {e}"),
    }
}

/// Le unità dichiarate dal disegno (`$INSUNITS`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Units {
    /// Millimetri per unità del disegno; `None` se il file non lo dice.
    pub mm: Option<f64>,
    /// Il nome, per il pannello: "millimetri", "metri", "non dichiarate".
    pub name: &'static str,
}

impl Units {
    fn from_code(code: i64) -> Self {
        let (mm, name) = match code {
            1 => (Some(25.4), "pollici"),
            2 => (Some(304.8), "piedi"),
            4 => (Some(1.0), "millimetri"),
            5 => (Some(10.0), "centimetri"),
            6 => (Some(1000.0), "metri"),
            10 => (Some(914.4), "iarde"),
            14 => (Some(100.0), "decimetri"),
            _ => (None, "non dichiarate"),
        };
        Self { mm, name }
    }
}

/// Un layer: nome, colore e quante forme ha.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub name: String,
    pub color: [u8; 3],
    /// Il colore 7 del CAD, che è bianco su fondo scuro e nero sulla carta.
    pub aci7: bool,
    /// Spento o congelato nel file: parte nascosto.
    pub off: bool,
    pub shapes: usize,
}

/// Il colore di una forma.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paint {
    pub rgb: [u8; 3],
    pub aci7: bool,
}

/// Un testo su una riga, già messo in posizione.
#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    /// Il punto di partenza della riga (in basso a sinistra), in unità del disegno.
    pub at: [f64; 2],
    pub height: f64,
    /// Rotazione in gradi, in senso antiorario.
    pub angle: f64,
    pub text: String,
}

/// Una forma da disegnare, in unità del disegno (y in su).
#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    /// Una linea spezzata, chiusa o aperta.
    Line { points: Vec<[f64; 2]>, closed: bool },
    /// Un'area piena (retino pieno, solido): uno o più contorni, pari-dispari.
    Fill { contours: Vec<Vec<[f64; 2]>> },
    Text(Text),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    /// Indice nell'elenco dei layer.
    pub layer: usize,
    pub paint: Paint,
    /// Spessore in mm sulla carta, se il file lo dice.
    pub weight: Option<f64>,
    pub geometry: Geometry,
}

/// Un disegno letto.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawing {
    pub layers: Vec<Layer>,
    pub shapes: Vec<Shape>,
    pub units: Units,
    /// L'ingombro `[x0, y0, x1, y1]` in unità del disegno.
    pub extents: [f64; 4],
    /// La versione del file: "DXF AC1032", "DWG AC1027".
    pub version: String,
    /// Entità che non si sanno disegnare, per tipo.
    pub skipped: Vec<(String, usize)>,
    /// Note del lettore dei DWG su ciò che ha saltato.
    pub notes: Vec<String>,
}

impl Drawing {
    /// Larghezza e altezza dell'ingombro in mm, con `mm` millimetri per unità.
    pub fn size_mm(&self, mm: f64) -> (f64, f64) {
        let [x0, y0, x1, y1] = self.extents;
        ((x1 - x0) * mm, (y1 - y0) * mm)
    }
}

/// Il tipo del file dal nome.
pub fn is_cad_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dxf") || e.eq_ignore_ascii_case("dwg"))
}

/// Il lettore dei DWG gira in una copia di WinFiles: da chiamare all'avvio del programma.
/// Restituisce `Some(codice)` se questa copia è il lettore e deve chiudere subito dopo.
pub fn serve_dwg_worker() -> Option<i32> {
    cadcraft_dwg::serve_if_asked()
}

/// Da chiamare una volta all'avvio dell'app: i DWG si leggono in una copia di WinFiles, così un
/// file rovinato che fa cadere il lettore non chiude la finestra.
pub fn use_dwg_worker() {
    cadcraft_dwg::use_own_worker();
}

/// Legge un DXF o un DWG dal disco.
pub fn open(path: &Path) -> Result<Drawing, CadError> {
    let len = std::fs::metadata(path)?.len();
    if len > MAX_FILE_BYTES {
        return Err(CadError::TooLarge(len));
    }
    let bytes = std::fs::read(path)?;
    from_bytes(&bytes)
}

/// Legge un DXF o un DWG già in memoria.
pub fn from_bytes(bytes: &[u8]) -> Result<Drawing, CadError> {
    if bytes.starts_with(b"AC10") || bytes.starts_with(b"AC1.") || bytes.starts_with(b"AC2") {
        if !cadcraft_dwg::is_dwg(bytes) {
            return Err(CadError::OldDwg);
        }
        let version = cadcraft_dwg::version(bytes).unwrap_or_default();
        if version.as_str() < "AC1012" {
            return Err(CadError::OldDwg);
        }
        let (dxf, notes) =
            cadcraft_dwg::decode(bytes, cadcraft_dwg::Limits::DEFAULT).map_err(CadError::Dwg)?;
        let mut d = parse_dxf(&dxf)?;
        d.version = format!("DWG {version}");
        d.notes = notes;
        return Ok(d);
    }
    parse_dxf(bytes)
}

fn parse_dxf(bytes: &[u8]) -> Result<Drawing, CadError> {
    let tags = cadcraft_dxf::parse(bytes).map_err(|e| CadError::Dxf(e.to_string()))?;
    let mut header = Vec::new();
    let mut tables = Vec::new();
    let mut blocks = Vec::new();
    let mut entities = Vec::new();
    for s in sections(&tags) {
        match s.name.as_str() {
            "HEADER" => header = s.tags,
            "TABLES" => tables = s.tags,
            "BLOCKS" => blocks = s.tags,
            "ENTITIES" => entities = s.tags,
            _ => {}
        }
    }
    let mut r = Reader::new(&header, &tables, &blocks);
    let records = records(&entities);
    r.entities(&records, &Transform::IDENTITY, 0, None);
    let extents = geometry::extents(&r.shapes).ok_or(CadError::Empty)?;
    for s in &r.shapes {
        if let Some(l) = r.layers.get_mut(s.layer) {
            l.shapes += 1;
        }
    }
    let mut skipped: Vec<(String, usize)> = r.skipped.into_iter().collect();
    skipped.sort();
    Ok(Drawing {
        layers: r.layers,
        shapes: r.shapes,
        units: r.units,
        extents,
        version: format!("DXF {}", r.version),
        skipped,
        notes: Vec::new(),
    })
}

/// Una trasformazione affine: x' = a·x + c·y + e, y' = b·x + d·y + f.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Transform([f64; 6]);

impl Transform {
    pub(crate) const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub(crate) fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        let [a, b, c, d, e, f] = self.0;
        [a * p[0] + c * p[1] + e, b * p[0] + d * p[1] + f]
    }

    /// Prima `inner`, poi `self`.
    fn then(&self, inner: &Transform) -> Transform {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = inner.0;
        Transform([
            a * a2 + c * b2,
            b * a2 + d * b2,
            a * c2 + c * d2,
            b * c2 + d * d2,
            a * e2 + c * f2 + e,
            b * e2 + d * f2 + f,
        ])
    }

    /// Quanto si allunga una lunghezza (media dei due assi).
    pub(crate) fn scale(&self) -> f64 {
        let [a, b, c, d, _, _] = self.0;
        ((a * d - b * c).abs()).sqrt()
    }

    /// L'angolo dell'asse x, in gradi.
    fn angle(&self) -> f64 {
        self.0[1].atan2(self.0[0]).to_degrees()
    }

    fn insert(at: [f64; 2], sx: f64, sy: f64, degrees: f64, base: [f64; 2]) -> Transform {
        let (s, c) = degrees.to_radians().sin_cos();
        // Prima si toglie il punto base, poi scala, rotazione e posizione.
        let a = c * sx;
        let b = s * sx;
        let cc = -s * sy;
        let d = c * sy;
        let e = at[0] - (a * base[0] + cc * base[1]);
        let f = at[1] - (b * base[0] + d * base[1]);
        Transform([a, b, cc, d, e, f])
    }
}

/// Un blocco: punto base e le sue entità.
struct Block {
    base: [f64; 2],
    records: Vec<(String, Vec<Tag>)>,
}

struct Reader {
    layers: Vec<Layer>,
    layer_index: HashMap<String, usize>,
    blocks: HashMap<String, Block>,
    shapes: Vec<Shape>,
    skipped: HashMap<String, usize>,
    units: Units,
    version: String,
}

/// I valori di un'entità per codice, il primo di ciascuno.
fn first(tags: &[Tag], code: i32) -> Option<&Tag> {
    tags.iter().find(|t| t.code == code)
}

fn real(tags: &[Tag], code: i32, default: f64) -> f64 {
    first(tags, code).map(Tag::f64).filter(|v| v.is_finite()).unwrap_or(default)
}

fn int(tags: &[Tag], code: i32, default: i64) -> i64 {
    first(tags, code).map_or(default, Tag::i64)
}

fn text(tags: &[Tag], code: i32) -> String {
    first(tags, code).map(|t| t.str().trim_end().to_string()).unwrap_or_default()
}

fn point(tags: &[Tag], x: i32) -> [f64; 2] {
    [real(tags, x, 0.0), real(tags, x + 10, 0.0)]
}

impl Reader {
    fn new(header: &[Tag], tables: &[Tag], blocks: &[Tag]) -> Self {
        let mut units = Units::from_code(0);
        let mut version = String::new();
        let mut it = header.iter().peekable();
        while let Some(t) = it.next() {
            if t.code == 9 {
                let name = t.str();
                if let Some(v) = it.peek() {
                    match name.trim() {
                        "$INSUNITS" => units = Units::from_code(v.i64()),
                        "$ACADVER" => version = v.str().trim().to_string(),
                        _ => {}
                    }
                }
            }
        }
        let mut r = Self {
            layers: Vec::new(),
            layer_index: HashMap::new(),
            blocks: HashMap::new(),
            shapes: Vec::new(),
            skipped: HashMap::new(),
            units,
            version,
        };
        for (kind, tags) in records(tables) {
            if kind.trim() == "LAYER" {
                let name = text(&tags, 2);
                if name.is_empty() || r.layer_index.contains_key(&name.to_uppercase()) {
                    continue;
                }
                let aci = int(&tags, 62, 7);
                let flags = int(&tags, 70, 0);
                let (color, aci7) = match first(&tags, 420) {
                    Some(t) => (rgb(Rgb::from_u32(t.i64() as u32)), false),
                    None => {
                        let i = u8::try_from(aci.unsigned_abs().clamp(1, 255)).unwrap_or(7);
                        (rgb(aci_rgb(i)), i == 7)
                    }
                };
                r.add_layer(name, color, aci7, aci < 0 || flags & 1 != 0);
            }
        }
        // I blocchi: BLOCK … ENDBLK, con le loro entità.
        let mut current: Option<(String, Block)> = None;
        for (kind, tags) in records(blocks) {
            match kind.trim() {
                "BLOCK" => {
                    let name = text(&tags, 2).to_uppercase();
                    current = Some((
                        name,
                        Block {
                            base: point(&tags, 10),
                            records: Vec::new(),
                        },
                    ));
                }
                "ENDBLK" => {
                    if let Some((name, b)) = current.take() {
                        r.blocks.insert(name, b);
                    }
                }
                _ => {
                    if let Some((_, b)) = current.as_mut() {
                        b.records.push((kind, tags));
                    }
                }
            }
        }
        r
    }

    fn add_layer(&mut self, name: String, color: [u8; 3], aci7: bool, off: bool) -> usize {
        let key = name.to_uppercase();
        if let Some(i) = self.layer_index.get(&key) {
            return *i;
        }
        self.layers.push(Layer {
            name,
            color,
            aci7,
            off,
            shapes: 0,
        });
        let i = self.layers.len() - 1;
        self.layer_index.insert(key, i);
        i
    }

    fn layer_of(&mut self, tags: &[Tag], parent: Option<(usize, Paint)>) -> usize {
        let name = text(tags, 8);
        // Il layer "0" dentro un blocco prende quello del blocco, come nei CAD.
        if (name.is_empty() || name == "0")
            && let Some((l, _)) = parent
        {
            return l;
        }
        let name = if name.is_empty() { "0".to_string() } else { name };
        self.add_layer(name, [255, 255, 255], true, false)
    }

    fn paint_of(&self, tags: &[Tag], layer: usize, parent: Option<(usize, Paint)>) -> Paint {
        if let Some(t) = first(tags, 420) {
            return Paint {
                rgb: rgb(Rgb::from_u32(t.i64() as u32)),
                aci7: false,
            };
        }
        let by_layer = || {
            self.layers.get(layer).map_or(
                Paint {
                    rgb: [255, 255, 255],
                    aci7: true,
                },
                |l| Paint {
                    rgb: l.color,
                    aci7: l.aci7,
                },
            )
        };
        match int(tags, 62, 256) {
            0 => parent.map_or_else(by_layer, |(_, p)| p),
            i @ 1..=255 => {
                let i = u8::try_from(i).unwrap_or(7);
                Paint {
                    rgb: rgb(aci_rgb(i)),
                    aci7: i == 7,
                }
            }
            _ => by_layer(),
        }
    }

    fn push(&mut self, layer: usize, paint: Paint, weight: Option<f64>, geometry: Geometry) -> bool {
        if self.shapes.len() >= MAX_SHAPES {
            return false;
        }
        self.shapes.push(Shape {
            layer,
            paint,
            weight,
            geometry,
        });
        true
    }

    fn skip(&mut self, kind: &str) {
        *self.skipped.entry(kind.to_string()).or_insert(0) += 1;
    }

    fn entities(
        &mut self,
        records: &[(String, Vec<Tag>)],
        tf: &Transform,
        depth: usize,
        parent: Option<(usize, Paint)>,
    ) {
        let mut i = 0;
        while let Some((kind, tags)) = records.get(i) {
            i += 1;
            if self.shapes.len() >= MAX_SHAPES {
                return;
            }
            let kind = kind.trim();
            // Invisibile (60 = 1) o nello spazio carta (67 = 1): lo spazio modello basta.
            if int(tags, 60, 0) == 1 || (depth == 0 && int(tags, 67, 0) == 1) {
                if kind == "POLYLINE" {
                    i = skip_vertices(records, i);
                }
                continue;
            }
            let layer = self.layer_of(tags, parent);
            let paint = self.paint_of(tags, layer, parent);
            let weight = match int(tags, 370, -1) {
                w @ 1..=211 => Some(w as f64 / 100.0),
                _ => None,
            };
            let line = |r: &mut Self, points: Vec<[f64; 2]>, closed: bool| {
                if points.len() >= 2 {
                    let points = points.iter().map(|p| tf.apply(*p)).collect();
                    r.push(layer, paint, weight, Geometry::Line { points, closed });
                }
            };
            match kind {
                "LINE" => line(self, vec![point(tags, 10), point(tags, 11)], false),
                "LWPOLYLINE" => {
                    let closed = int(tags, 70, 0) & 1 != 0;
                    let pts = lw_vertices(tags);
                    line(self, geometry::bulged(&pts, closed), closed);
                }
                "POLYLINE" => {
                    let closed = int(tags, 70, 0) & 1 != 0;
                    let mut pts = Vec::new();
                    while let Some((k, v)) = records.get(i) {
                        if k.trim() != "VERTEX" {
                            break;
                        }
                        i += 1;
                        if pts.len() < MAX_POINTS {
                            pts.push((point(v, 10), real(v, 42, 0.0)));
                        }
                    }
                    if records.get(i).is_some_and(|(k, _)| k.trim() == "SEQEND") {
                        i += 1;
                    }
                    line(self, geometry::bulged(&pts, closed), closed);
                }
                "CIRCLE" => {
                    let r = real(tags, 40, 0.0);
                    line(self, geometry::arc(point(tags, 10), r, 0.0, 360.0), true);
                }
                "ARC" => {
                    let r = real(tags, 40, 0.0);
                    line(
                        self,
                        geometry::arc(point(tags, 10), r, real(tags, 50, 0.0), real(tags, 51, 360.0)),
                        false,
                    );
                }
                "ELLIPSE" => {
                    let pts = geometry::ellipse(
                        point(tags, 10),
                        point(tags, 11),
                        real(tags, 40, 1.0),
                        real(tags, 41, 0.0),
                        real(tags, 42, std::f64::consts::TAU),
                    );
                    let closed = (real(tags, 42, std::f64::consts::TAU) - real(tags, 41, 0.0)).abs()
                        >= std::f64::consts::TAU - 1e-6;
                    line(self, pts, closed);
                }
                "SPLINE" => {
                    let closed = int(tags, 70, 0) & 1 != 0;
                    let pts = spline(tags);
                    line(self, pts, closed);
                }
                "SOLID" | "TRACE" | "3DFACE" => {
                    // I quattro vertici; nei solidi il terzo e il quarto sono scambiati.
                    let mut q = [point(tags, 10), point(tags, 11), point(tags, 12), point(tags, 13)];
                    if kind != "3DFACE" {
                        q.swap(2, 3);
                    }
                    let contour: Vec<[f64; 2]> = q.iter().map(|p| tf.apply(*p)).collect();
                    if kind == "3DFACE" {
                        self.push(layer, paint, weight, Geometry::Line { points: contour, closed: true });
                    } else {
                        self.push(layer, paint, weight, Geometry::Fill { contours: vec![contour] });
                    }
                }
                "HATCH" => {
                    let solid = int(tags, 70, 0) == 1;
                    let contours: Vec<Vec<[f64; 2]>> = hatch_boundaries(tags)
                        .into_iter()
                        .filter(|c| c.len() >= 3)
                        .map(|c| c.iter().map(|p| tf.apply(*p)).collect())
                        .collect();
                    if contours.is_empty() {
                        self.skip("HATCH");
                    } else if solid {
                        self.push(layer, paint, weight, Geometry::Fill { contours });
                    } else {
                        // Il motivo del retino non si disegna: solo il bordo.
                        for c in contours {
                            self.push(layer, paint, weight, Geometry::Line { points: c, closed: true });
                        }
                    }
                }
                "TEXT" | "ATTRIB" => {
                    let value = text(tags, 1);
                    let h = real(tags, 40, 0.0);
                    if !value.trim().is_empty() && h > 0.0 {
                        let (h_al, v_al) = (int(tags, 72, 0), int(tags, 73, 0));
                        let at = if h_al != 0 || v_al != 0 {
                            point(tags, 11)
                        } else {
                            point(tags, 10)
                        };
                        let angle = real(tags, 50, 0.0);
                        let width = geometry::text_width(&value, h);
                        let dx = match h_al {
                            1 | 4 => -width / 2.0,
                            2 => -width,
                            _ => 0.0,
                        };
                        let dy = match v_al {
                            1 => 0.0,
                            2 => -h / 2.0,
                            3 => -h,
                            _ => 0.0,
                        };
                        self.text_line(layer, paint, tf, at, angle, dx, dy, h, value);
                    }
                }
                "MTEXT" => {
                    let raw = mtext_raw(tags);
                    let h = real(tags, 40, 0.0);
                    let lines = geometry::mtext_lines(&raw);
                    if !lines.is_empty() && h > 0.0 {
                        let at = point(tags, 10);
                        let angle = first(tags, 50).map_or_else(
                            || {
                                let d = point(tags, 11);
                                if d == [0.0, 0.0] { 0.0 } else { d[1].atan2(d[0]).to_degrees() }
                            },
                            Tag::f64,
                        );
                        // Attacco 1-9: in alto, al centro, in basso × sinistra, centro, destra.
                        let attach = int(tags, 71, 1).clamp(1, 9);
                        let gap = h * 1.6667;
                        let total = gap * (lines.len().saturating_sub(1)) as f64 + h;
                        let top = match (attach - 1) / 3 {
                            0 => 0.0,
                            1 => total / 2.0,
                            _ => total,
                        };
                        for (n, l) in lines.into_iter().enumerate() {
                            let width = geometry::text_width(&l, h);
                            let dx = match (attach - 1) % 3 {
                                1 => -width / 2.0,
                                2 => -width,
                                _ => 0.0,
                            };
                            let dy = top - h - gap * n as f64;
                            self.text_line(layer, paint, tf, at, angle, dx, dy, h, l);
                        }
                    }
                }
                "INSERT" | "DIMENSION" => {
                    let name = text(tags, 2).to_uppercase();
                    let (at, sx, sy, rot, cols, rows, dc, dr) = if kind == "INSERT" {
                        (
                            point(tags, 10),
                            real(tags, 41, 1.0),
                            real(tags, 42, 1.0),
                            real(tags, 50, 0.0),
                            int(tags, 70, 1).clamp(1, MAX_ARRAY),
                            int(tags, 71, 1).clamp(1, MAX_ARRAY),
                            real(tags, 44, 0.0),
                            real(tags, 45, 0.0),
                        )
                    } else {
                        // Il blocco di una quota è già disegnato al suo posto.
                        ([0.0, 0.0], 1.0, 1.0, 0.0, 1, 1, 0.0, 0.0)
                    };
                    if depth >= MAX_DEPTH || cols.saturating_mul(rows) > MAX_ARRAY {
                        self.skip(kind);
                    } else if let Some(block) = self.blocks.get(&name) {
                        let base = if kind == "INSERT" { block.base } else { [0.0, 0.0] };
                        let inner_records = block.records.clone();
                        for r in 0..rows {
                            for c in 0..cols {
                                // La serie si sposta lungo gli assi del blocco ruotato.
                                let (s, co) = rot.to_radians().sin_cos();
                                let (ox, oy) = (c as f64 * dc, r as f64 * dr);
                                let p = [at[0] + ox * co - oy * s, at[1] + ox * s + oy * co];
                                let inner = Transform::insert(p, sx, sy, rot, base);
                                let next = tf.then(&inner);
                                self.entities(&inner_records, &next, depth + 1, Some((layer, paint)));
                            }
                        }
                    } else {
                        self.skip(kind);
                    }
                    // Gli attributi di un blocco seguono l'INSERT fino a SEQEND.
                    let mut attribs = Vec::new();
                    while let Some((k, v)) = records_at(records, i) {
                        if k == "ATTRIB" {
                            i += 1;
                            attribs.push(("ATTRIB".to_string(), v.clone()));
                        } else {
                            if k == "SEQEND" {
                                i += 1;
                            }
                            break;
                        }
                    }
                    if !attribs.is_empty() {
                        self.entities(&attribs, tf, depth, parent);
                    }
                }
                "LEADER" => {
                    let pts: Vec<[f64; 2]> = lw_vertices(tags).into_iter().map(|(p, _)| p).collect();
                    line(self, pts, false);
                }
                "POINT" | "VIEWPORT" | "SEQEND" | "VERTEX" | "ATTDEF" => {}
                other => self.skip(other),
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text_line(
        &mut self,
        layer: usize,
        paint: Paint,
        tf: &Transform,
        at: [f64; 2],
        angle: f64,
        dx: f64,
        dy: f64,
        h: f64,
        text: String,
    ) {
        let (s, c) = angle.to_radians().sin_cos();
        let start = [at[0] + dx * c - dy * s, at[1] + dx * s + dy * c];
        self.push(
            layer,
            paint,
            None,
            Geometry::Text(Text {
                at: tf.apply(start),
                height: h * tf.scale(),
                angle: angle + tf.angle(),
                text,
            }),
        );
    }
}

fn records_at(records: &[(String, Vec<Tag>)], i: usize) -> Option<(&str, &Vec<Tag>)> {
    records.get(i).map(|(k, v)| (k.trim(), v))
}

/// Salta i VERTEX e il SEQEND di una polilinea non disegnata.
fn skip_vertices(records: &[(String, Vec<Tag>)], mut i: usize) -> usize {
    while let Some((k, _)) = records_at(records, i) {
        match k {
            "VERTEX" => i += 1,
            "SEQEND" => return i + 1,
            _ => return i,
        }
    }
    i
}

fn rgb(c: Rgb) -> [u8; 3] {
    [c.0, c.1, c.2]
}

/// I vertici di una LWPOLYLINE con il loro rigonfiamento (42), nell'ordine.
fn lw_vertices(tags: &[Tag]) -> Vec<([f64; 2], f64)> {
    let mut out: Vec<([f64; 2], f64)> = Vec::new();
    for t in tags {
        match t.code {
            10 if out.len() < MAX_POINTS => out.push(([t.f64(), 0.0], 0.0)),
            20 => {
                if let Some(last) = out.last_mut() {
                    last.0[1] = t.f64();
                }
            }
            42 => {
                if let Some(last) = out.last_mut() {
                    last.1 = t.f64();
                }
            }
            _ => {}
        }
    }
    out
}

/// Il testo intero di un MTEXT: i pezzi da 250 caratteri (3) e l'ultimo (1).
fn mtext_raw(tags: &[Tag]) -> String {
    let mut s = String::new();
    for t in tags.iter().filter(|t| t.code == 3) {
        s.push_str(&t.str());
    }
    s.push_str(&text(tags, 1));
    s
}

/// I punti di una spline: dai punti di controllo (con i nodi) o, se mancano, dai punti di
/// passaggio uniti da linee.
fn spline(tags: &[Tag]) -> Vec<[f64; 2]> {
    let degree = usize::try_from(int(tags, 71, 3).clamp(1, 10)).unwrap_or(3);
    let knots: Vec<f64> = tags.iter().filter(|t| t.code == 40).map(Tag::f64).take(MAX_POINTS).collect();
    let weights: Vec<f64> = tags.iter().filter(|t| t.code == 41).map(Tag::f64).take(MAX_POINTS).collect();
    let pick = |x: i32| {
        let mut pts: Vec<[f64; 2]> = Vec::new();
        for t in tags {
            if t.code == x && pts.len() < MAX_POINTS {
                pts.push([t.f64(), 0.0]);
            } else if t.code == x + 10
                && let Some(last) = pts.last_mut()
            {
                last[1] = t.f64();
            }
        }
        pts
    };
    let control = pick(10);
    if control.len() > degree {
        let pts = geometry::nurbs(&control, &knots, &weights, degree);
        if pts.len() >= 2 {
            return pts;
        }
    }
    pick(11)
}

/// I contorni di un retino: polilinee (con rigonfiamenti) o lati (linee, archi, ellissi).
fn hatch_boundaries(tags: &[Tag]) -> Vec<Vec<[f64; 2]>> {
    let mut out = Vec::new();
    // Si parte dal numero di contorni (91) e si legge un contorno alla volta.
    let Some(start) = tags.iter().position(|t| t.code == 91) else {
        return out;
    };
    let count = tags.get(start).map_or(0, Tag::i64).clamp(0, 10_000);
    let mut i = start + 1;
    let get = |i: usize| tags.get(i);
    for _ in 0..count {
        // Ogni contorno comincia con il tipo (92).
        while get(i).is_some_and(|t| t.code != 92) {
            i += 1;
        }
        let Some(kind) = get(i).map(Tag::i64) else {
            break;
        };
        i += 1;
        let mut contour: Vec<[f64; 2]> = Vec::new();
        if kind & 2 != 0 {
            // Polilinea: 72 rigonfiamenti sì/no, 73 chiusa, 93 vertici.
            let mut verts: Vec<([f64; 2], f64)> = Vec::new();
            let mut n: Option<usize> = None;
            while let Some(t) = get(i) {
                match t.code {
                    72 | 73 if n.is_none() => {}
                    93 if n.is_none() => n = usize::try_from(t.i64().clamp(0, MAX_POINTS as i64)).ok(),
                    10 if n.is_some_and(|n| verts.len() < n) => verts.push(([t.f64(), 0.0], 0.0)),
                    20 => {
                        if let Some(v) = verts.last_mut() {
                            v.0[1] = t.f64();
                        }
                    }
                    42 => {
                        if let Some(v) = verts.last_mut() {
                            v.1 = t.f64();
                        }
                    }
                    _ => break,
                }
                i += 1;
            }
            contour = geometry::bulged(&verts, true);
        } else {
            // Lati: 93 quanti, poi per ciascuno 72 tipo e i suoi numeri.
            let edges = get(i).filter(|t| t.code == 93).map_or(0, Tag::i64).clamp(0, 100_000);
            if edges > 0 {
                i += 1;
            }
            for _ in 0..edges {
                let Some(edge) = get(i).filter(|t| t.code == 72).map(Tag::i64) else {
                    break;
                };
                i += 1;
                let mut vals: HashMap<i32, Vec<f64>> = HashMap::new();
                while let Some(t) = get(i) {
                    if t.code == 72 || t.code == 92 || t.code == 97 || t.code == 75 {
                        break;
                    }
                    vals.entry(t.code).or_default().push(t.f64());
                    i += 1;
                }
                let v = |c: i32, n: usize| vals.get(&c).and_then(|l| l.get(n)).copied().unwrap_or(0.0);
                let pts = match edge {
                    1 => vec![[v(10, 0), v(20, 0)], [v(11, 0), v(21, 0)]],
                    2 => {
                        let ccw = vals.get(&73).and_then(|l| l.first()).is_none_or(|f| *f != 0.0);
                        let (a, b) = (v(50, 0), v(51, 0));
                        if ccw {
                            geometry::arc([v(10, 0), v(20, 0)], v(40, 0), a, b)
                        } else {
                            // In senso orario gli angoli del file sono riflessi sull'asse x.
                            geometry::arc_cw([v(10, 0), v(20, 0)], v(40, 0), a, b)
                        }
                    }
                    3 => {
                        let ccw = vals.get(&73).and_then(|l| l.first()).is_none_or(|f| *f != 0.0);
                        let (a, b) = (v(50, 0).to_radians(), v(51, 0).to_radians());
                        let mut p = geometry::ellipse([v(10, 0), v(20, 0)], [v(11, 0), v(21, 0)], v(40, 0), a, b);
                        if !ccw {
                            p.reverse();
                        }
                        p
                    }
                    4 => {
                        let degree = v(94, 0).clamp(1.0, 10.0) as usize;
                        let knots = vals.get(&40).cloned().unwrap_or_default();
                        let weights = vals.get(&42).cloned().unwrap_or_default();
                        let xs = vals.get(&10).cloned().unwrap_or_default();
                        let ys = vals.get(&20).cloned().unwrap_or_default();
                        let control: Vec<[f64; 2]> = xs.iter().zip(ys.iter()).map(|(x, y)| [*x, *y]).collect();
                        geometry::nurbs(&control, &knots, &weights, degree)
                    }
                    _ => Vec::new(),
                };
                // Un lato attacca dove finisce il precedente: il primo punto non si ripete.
                let skip = usize::from(contour.last().zip(pts.first()).is_some_and(|(a, b)| {
                    (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
                }));
                if contour.len() + pts.len() <= MAX_POINTS {
                    contour.extend(pts.into_iter().skip(skip));
                }
            }
        }
        if contour.len() >= 3 {
            out.push(contour);
        }
    }
    out
}

#[cfg(test)]
mod tests;
