#![allow(clippy::unwrap_used)]
use winfiles_pdf_edit::*;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut e = Editor::open(std::path::Path::new(&a[1])).unwrap();
    let n = e.page_count();
    let all: Vec<usize> = (0..n).collect();
    println!("{:?}", e.pages());
    match a[3].as_str() {
        "margini" => e
            .add_margins(&all, winfiles_geom::Margins::uniform(10.0), [230, 0, 0])
            .unwrap(),
        "a3" => e.fit_paper(&all, winfiles_geom::Paper::A3).unwrap(),
        "rifila" => {
            e.trim(&all).unwrap();
        }
        _ => e.rotate(&all, 1).unwrap(),
    }
    println!("{:?}", e.pages());
    e.save(std::path::Path::new(&a[2])).unwrap();
}
