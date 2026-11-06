//! Prova a mano: `cargo run -p winfiles-cad --example disegna -- disegno.dxf uscita.png`.
fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("uso: disegna disegno.dxf uscita.png");
        return;
    };
    match winfiles_cad::open(std::path::Path::new(&input)) {
        Ok(d) => {
            println!("{} · unità {} · ingombro {:?}", d.version, d.units.name, d.extents);
            for l in &d.layers {
                println!("  layer {:20} {:?} forme {}", l.name, l.color, l.shapes);
            }
            println!("  saltate: {:?}", d.skipped);
            let o = winfiles_cad::PdfOptions {
                mm_per_unit: d.fit_scale(400.0),
                ..Default::default()
            };
            let pdf = d.to_pdf(&o).unwrap_or_default();
            let _ = std::fs::write(format!("{output}.pdf"), &pdf);
            if let Ok(p) = winfiles_thumbs::PdfPages::open(pdf)
                && let Ok(t) = p.render(0, 2000)
            {
                let _ = image_save(&output, &t);
            }
        }
        Err(e) => eprintln!("{e}"),
    }
}

fn image_save(path: &str, t: &winfiles_thumbs::Thumb) -> std::io::Result<()> {
    // PPM: senza dipendenze in più.
    let mut out = format!("P6\n{} {}\n255\n", t.width, t.height).into_bytes();
    for p in t.rgba.chunks(4) {
        out.extend_from_slice(&p[..3]);
    }
    std::fs::write(path, out)
}
