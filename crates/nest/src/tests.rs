//! I test del pannello per Illustrator (`test/nester.test.js`), con le stesse forme.

use super::*;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};

type C = Vec<[f64; 2]>;

fn l(w: f64, h: f64, t: f64) -> C {
    vec![[0.0, 0.0], [t, 0.0], [t, h - t], [w, h - t], [w, h], [0.0, h]]
}

fn t_shape(w: f64, h: f64, t: f64) -> C {
    vec![
        [0.0, 0.0],
        [w, 0.0],
        [w, t],
        [(w + t) / 2.0, t],
        [(w + t) / 2.0, h],
        [(w - t) / 2.0, h],
        [(w - t) / 2.0, t],
        [0.0, t],
    ]
}

fn u(w: f64, h: f64, t: f64) -> C {
    vec![
        [0.0, 0.0],
        [t, 0.0],
        [t, h - t],
        [w - t, h - t],
        [w - t, 0.0],
        [w, 0.0],
        [w, h],
        [0.0, h],
    ]
}

fn circle(r: f64, n: usize) -> C {
    (0..n)
        .map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            [r + r * a.cos(), r + r * a.sin()]
        })
        .collect()
}

fn star(r1: f64, r2: f64, n: usize) -> C {
    (0..2 * n)
        .map(|i| {
            let r = if i % 2 == 1 { r1 } else { r2 };
            let a = i as f64 / (2 * n) as f64 * std::f64::consts::TAU;
            [r2 + r * a.cos(), r2 + r * a.sin()]
        })
        .collect()
}

fn moved(c: &C, dx: f64, dy: f64) -> C {
    c.iter().map(|p| [p[0] + dx, p[1] + dy]).collect()
}

fn rect(w: f64, h: f64) -> C {
    vec![[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]]
}

fn item(shape: Vec<C>, copies: usize) -> Item {
    Item {
        shape,
        copies,
        right_angles_only: false,
    }
}

fn letter_o() -> Vec<C> {
    let mut inner = moved(&circle(30.0, 32), 30.0, 30.0);
    inner.reverse();
    vec![circle(60.0, 32), inner]
}

fn sheet(w: f64, h: f64) -> Stock {
    Stock::Sheet { width: w, height: h }
}

fn roll(w: f64, max: f64) -> Stock {
    Stock::Roll {
        width: w,
        max_length: max,
    }
}

fn opts(stock: Stock, margin: f64, spacing: f64, step: f64, ms: u64) -> Options {
    Options {
        stock,
        margin,
        spacing,
        rotation_step: step,
        time_limit: Duration::from_millis(ms),
        ..Options::default()
    }
}

fn run(items: &[Item], o: &Options) -> Outcome {
    let stop = AtomicBool::new(false);
    nest(items, o, &stop, |_| {}, |_| {}).unwrap()
}

/// Le forme vere sul foglio (pari-dispari: i fori restano fori).
fn placed(items: &[Item], p: &Placement) -> Vec<Vec<C>> {
    let rings: Vec<C> = items[p.item]
        .shape
        .iter()
        .map(|c| c.iter().map(|q| p.apply(*q)).collect())
        .collect();
    let empty: Vec<C> = Vec::new();
    rings.overlay(&empty, OverlayRule::Subject, FillRule::EvenOdd)
}

fn area_of(shapes: &[Vec<C>]) -> f64 {
    shapes
        .iter()
        .flatten()
        .map(|c| {
            let n = c.len();
            (0..n)
                .map(|i| c[i][0] * c[(i + 1) % n][1] - c[(i + 1) % n][0] * c[i][1])
                .sum::<f64>()
                / 2.0
        })
        .sum()
}

/// Margini e distanze rispettati: ogni pezzo allargato di metà distanza (meno 0,02 mm) non
/// tocca gli altri.
fn assert_valid(items: &[Item], res: &Outcome, o: &Options) {
    let max_h = match o.stock {
        Stock::Roll { max_length, .. } => max_length,
        Stock::Sheet { height, .. } => height,
    };
    for sh in &res.sheets {
        let shapes: Vec<Vec<Vec<C>>> = sh.placements.iter().map(|p| placed(items, p)).collect();
        for (s, p) in shapes.iter().zip(&sh.placements) {
            let pts = s.iter().flatten().flatten();
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for q in pts {
                x0 = x0.min(q[0]);
                y0 = y0.min(q[1]);
                x1 = x1.max(q[0]);
                y1 = y1.max(q[1]);
            }
            assert!(
                x0 >= o.margin - 0.02 && y0 >= o.margin - 0.02,
                "pezzo {p:?} fuori dal margine"
            );
            assert!(
                x1 <= sh.width - o.margin + 0.02,
                "pezzo {p:?} oltre il bordo destro"
            );
            assert!(y1 <= max_h - o.margin + 0.02, "pezzo {p:?} oltre il bordo sotto");
        }
        let half = (o.spacing / 2.0 - 0.02).max(0.001);
        let style = OutlineStyle::new(half).line_join(LineJoin::Round(0.1));
        let grown: Vec<Vec<Vec<C>>> = shapes.iter().map(|s| s.outline(&style)).collect();
        for i in 0..grown.len() {
            for j in i + 1..grown.len() {
                let common = grown[i].overlay(&grown[j], OverlayRule::Intersect, FillRule::NonZero);
                let a = area_of(&common);
                assert!(
                    a.abs() < 1e-3,
                    "pezzi {:?} e {:?} troppo vicini ({a} mm²)",
                    sh.placements[i],
                    sh.placements[j]
                );
            }
        }
    }
}

#[test]
fn forme_concave_su_lastra() {
    let items = vec![
        item(vec![moved(&l(200.0, 300.0, 60.0), 1000.0, -400.0)], 4),
        item(vec![moved(&t_shape(250.0, 200.0, 60.0), -300.0, 50.0)], 4),
        item(vec![moved(&u(220.0, 180.0, 50.0), 30.0, 30.0)], 4),
    ];
    let o = opts(sheet(1000.0, 800.0), 10.0, 5.0, 90.0, 0);
    let res = run(&items, &o);
    assert!(res.unplaced.is_empty());
    assert_valid(&items, &res, &o);
}

#[test]
fn le_sagome_si_incastrano() {
    // 8 L: con i rettangoli d'ingombro servono 4 righe da 300 mm; incastrate occupano meno.
    let items = vec![item(vec![l(300.0, 300.0, 80.0)], 8)];
    let o = opts(roll(640.0, 5000.0), 0.0, 4.0, 90.0, 2000);
    let res = run(&items, &o);
    assert!(res.unplaced.is_empty());
    assert_valid(&items, &res, &o);
    let len = res.total_length.unwrap();
    assert!(len < 1100.0, "lunghezza usata {len:.0} mm");
}

#[test]
fn il_foro_della_o_resta_vuoto() {
    let items = vec![
        item(letter_o(), 1),
        item(vec![moved(&circle(10.0, 32), 500.0, 500.0)], 1),
    ];
    let o = opts(sheet(130.0, 200.0), 0.0, 2.0, 90.0, 0);
    let res = run(&items, &o);
    assert_valid(&items, &res, &o);
    let all: Vec<(usize, Placement)> = res
        .sheets
        .iter()
        .enumerate()
        .flat_map(|(k, s)| s.placements.iter().map(move |p| (k, *p)))
        .collect();
    let (ks, o_pl) = all.iter().find(|(_, p)| p.item == 0).unwrap();
    let (kc, c_pl) = all.iter().find(|(_, p)| p.item == 1).unwrap();
    let a = o_pl.apply([60.0, 60.0]);
    let b = c_pl.apply([510.0, 510.0]);
    let inside = ks == kc && (a[0] - b[0]).hypot(a[1] - b[1]) < 30.0;
    assert!(!inside, "il pezzo piccolo non deve stare nel foro della O");
}

#[test]
fn stelle_girate_a_passi_di_15_gradi() {
    let items = vec![item(vec![star(30.0, 80.0, 10)], 6)];
    let o = opts(roll(400.0, 3000.0), 5.0, 3.0, 15.0, 0);
    let res = run(&items, &o);
    assert!(res.unplaced.is_empty());
    assert_valid(&items, &res, &o);
}

#[test]
fn il_pezzo_troppo_grande_resta_fuori() {
    let items = vec![
        item(vec![l(900.0, 900.0, 100.0)], 1),
        item(vec![l(100.0, 100.0, 30.0)], 1),
    ];
    let o = opts(sheet(500.0, 500.0), 10.0, 5.0, 90.0, 0);
    let res = run(&items, &o);
    assert_eq!(res.unplaced, vec![(0, 0)]);
}

#[test]
fn lastra_piena_altri_fogli() {
    let items = vec![item(vec![u(400.0, 400.0, 100.0)], 6)];
    let o = opts(sheet(900.0, 450.0), 10.0, 5.0, 90.0, 0);
    let res = run(&items, &o);
    assert!(res.unplaced.is_empty());
    assert!(res.sheets.len() >= 2);
    assert_valid(&items, &res, &o);
}

#[test]
fn la_distanza_si_rispetta() {
    let items = vec![item(vec![rect(100.0, 100.0)], 2)];
    let o = opts(roll(400.0, 1000.0), 0.0, 6.0, 0.0, 0);
    let res = run(&items, &o);
    assert_valid(&items, &res, &o);
}

#[test]
fn cinquanta_lettere_in_tempi_ragionevoli() {
    let items = vec![
        item(vec![l(120.0, 180.0, 35.0)], 10),
        item(vec![t_shape(150.0, 180.0, 35.0)], 10),
        item(vec![u(150.0, 170.0, 35.0)], 10),
        item(vec![star(25.0, 70.0, 10)], 10),
        item(letter_o(), 10),
    ];
    let o = opts(roll(1370.0, 5000.0), 10.0, 5.0, 90.0, 0);
    let t = Instant::now();
    let res = run(&items, &o);
    let ms = t.elapsed().as_millis();
    assert!(res.unplaced.is_empty());
    assert_valid(&items, &res, &o);
    eprintln!(
        "50 pezzi: {ms} ms, lunghezza {:.0} mm, utilizzo {:.0}%",
        res.total_length.unwrap(),
        res.utilization * 100.0
    );
    assert!(ms < 20_000, "troppo lento: {ms} ms");
}

#[test]
fn il_quadratino_va_nel_foro_della_o() {
    let items = vec![
        item(letter_o(), 1),
        item(vec![moved(&rect(35.0, 35.0), 300.0, 300.0)], 1),
    ];
    let base = opts(sheet(125.0, 125.0), 0.0, 2.0, 90.0, 0);
    let off = run(&items, &base);
    assert_eq!(off.sheets.len(), 2, "senza l'opzione servono due fogli");
    let on = run(
        &items,
        &Options {
            holes: true,
            ..base.clone()
        },
    );
    assert_eq!(on.sheets.len(), 1, "con l'opzione basta un foglio");
    assert!(on.unplaced.is_empty());
    assert_valid(&items, &on, &base);
}

#[test]
fn i_pezzi_specchiati_restano_validi() {
    let items = vec![item(vec![l(200.0, 300.0, 60.0)], 8)];
    let o = Options {
        mirror: true,
        ..opts(roll(700.0, 3000.0), 5.0, 4.0, 90.0, 1500)
    };
    let res = run(&items, &o);
    assert!(res.unplaced.is_empty());
    assert_valid(&items, &res, &o);
}

#[test]
fn rettangoli_a_distanza_zero_si_toccano() {
    let items = vec![item(vec![rect(100.0, 50.0)], 4)];
    let o = opts(roll(200.5, 1000.0), 0.0, 0.0, 0.0, 0);
    let res = run(&items, &o);
    let len = res.total_length.unwrap();
    assert!(len < 101.0, "lunghezza {len}");
}

#[test]
fn le_immagini_girano_solo_di_90_gradi() {
    let items = vec![Item {
        shape: vec![rect(100.0, 30.0)],
        copies: 5,
        right_angles_only: true,
    }];
    let o = opts(roll(400.0, 2000.0), 0.0, 2.0, 15.0, 300);
    let res = run(&items, &o);
    assert!(
        res.sheets
            .iter()
            .flat_map(|s| &s.placements)
            .all(|p| p.rotation % 90.0 == 0.0)
    );
}

#[test]
fn ci_si_puo_fermare() {
    let items = vec![item(vec![star(30.0, 80.0, 10)], 20)];
    let o = opts(roll(400.0, 5000.0), 5.0, 3.0, 15.0, 60_000);
    let stop = AtomicBool::new(false);
    let t = Instant::now();
    let res = nest(
        &items,
        &o,
        &stop,
        |_| {},
        |p| {
            if p.tries >= 3 {
                stop.store(true, Ordering::Relaxed);
            }
        },
    )
    .unwrap();
    assert!(res.unplaced.is_empty());
    assert!(t.elapsed() < Duration::from_secs(30));
}

#[test]
fn misure_sbagliate() {
    let items = vec![item(vec![rect(10.0, 10.0)], 1)];
    let stop = AtomicBool::new(false);
    let bad = opts(sheet(f64::NAN, 100.0), 0.0, 0.0, 0.0, 0);
    assert!(nest(&items, &bad, &stop, |_| {}, |_| {}).is_err());
    let big = opts(sheet(100.0, 100.0), 60.0, 0.0, 0.0, 0);
    assert!(nest(&items, &big, &stop, |_| {}, |_| {}).is_err());
}
