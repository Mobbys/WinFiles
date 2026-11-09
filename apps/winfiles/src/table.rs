//! Il lavoro come tabella: una riga per pagina in stampa, con cartella, file, tipo, misure in
//! colonne numeriche, metri quadrati e nota. Diventa un CSV per Excel (punto e virgola, virgola
//! decimale, BOM per gli accenti), testo a tabulazioni e una tabella HTML per gli appunti, così si
//! incolla in Excel, Word o in una mail con le cartelle e i totali.
//!
//! Come nella 4.9 ("Esporta in CSV", "Copia Tabella", "Copia Formattata"), ma con base e altezza
//! in colonne separate, che in Excel si sommano e si ordinano.

use std::collections::BTreeSet;
use std::path::Path;

use winfiles_geom::{Size, Unit, area_m2, format_number};
use winfiles_model::{FileEntry, FileKind, Project};

/// Una riga della tabella: una pagina in stampa.
pub struct Line<'a> {
    pub folder: String,
    pub file: &'a FileEntry,
    pub page: usize,
    pub size: Option<Size>,
    /// Il formato al vivo, solo se diverso dal formato pagina.
    pub trim: Option<Size>,
    pub note: &'a str,
}

/// Le righe del lavoro, nel suo ordine: solo le pagine in stampa, e solo dei file `only` se
/// c'è (gli indici dei file scelti nell'elenco).
pub fn lines<'a>(project: &'a Project, only: Option<&BTreeSet<usize>>) -> Vec<Line<'a>> {
    let labels = project.folder_labels();
    let mut out = Vec::new();
    for (i, f) in project.files.iter().enumerate() {
        if only.is_some_and(|set| !set.contains(&i)) {
            continue;
        }
        let folder = labels
            .get(f.folder())
            .cloned()
            .unwrap_or_else(|| f.folder().display().to_string());
        for page in f.printed_pages() {
            let info = f.pages.get(page);
            out.push(Line {
                folder: folder.clone(),
                file: f,
                page,
                size: info.map(|p| p.size),
                trim: info.and_then(|p| p.trim.filter(|t| *t != p.size)),
                note: f.note(page),
            });
        }
    }
    out
}

fn kind(k: FileKind) -> &'static str {
    match k {
        FileKind::Image => "Immagine",
        FileKind::Pdf => "PDF",
        FileKind::Ai => "AI",
        FileKind::Eps => "EPS",
        FileKind::Unsupported => "Altro",
    }
}

fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Una nota su una riga sola: gli a capo diventano " / ".
fn one_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" / ")
}

/// Le intestazioni delle colonne, con l'unità delle misure.
fn header(unit: Unit) -> Vec<String> {
    let u = unit.symbol();
    vec![
        "Cartella".into(),
        "File".into(),
        "Tipo".into(),
        "Pagina".into(),
        format!("Base ({u})"),
        format!("Altezza ({u})"),
        "m²".into(),
        format!("Base al vivo ({u})"),
        format!("Altezza al vivo ({u})"),
        "m² al vivo".into(),
        "Nota".into(),
        "Percorso".into(),
    ]
}

/// Le celle di una riga, numeri con la virgola decimale e senza unità.
fn cells(line: &Line<'_>, unit: Unit) -> Vec<String> {
    let len = |mm: f64| format_number(mm / unit.mm(), unit.decimals(), true).replace('.', "");
    let area = |s: Size| format_number(area_m2(s), 4, false).replace('.', "");
    let (w, h, a) = match line.size {
        Some(s) => (len(s.w), len(s.h), area(s)),
        None => Default::default(),
    };
    // Il m² al vivo c'è sempre quando la misura c'è: senza TrimBox è quello del formato pagina.
    let (tw, th) = match line.trim {
        Some(t) => (len(t.w), len(t.h)),
        None => Default::default(),
    };
    let ta = line
        .size
        .map(|s| area(line.trim.unwrap_or(s)))
        .unwrap_or_default();
    let pages = line.file.sheet_pages().max(1);
    vec![
        line.folder.clone(),
        name(&line.file.path),
        kind(line.file.kind).into(),
        format!("{} di {pages}", line.page + 1),
        w,
        h,
        a,
        tw,
        th,
        ta,
        one_line(line.note),
        line.file.path.display().to_string(),
    ]
}

/// Una cella CSV: tra virgolette se contiene il separatore, virgolette o a capo.
fn csv_cell(text: &str) -> String {
    if text.contains([';', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

/// Il CSV per Excel in italiano: BOM UTF-8, punto e virgola, a capo di Windows.
pub fn csv(lines: &[Line<'_>], unit: Unit) -> String {
    let mut out = String::from("\u{feff}");
    let row = |cells: Vec<String>| cells.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(";");
    out.push_str(&row(header(unit)));
    out.push_str("\r\n");
    for line in lines {
        out.push_str(&row(cells(line, unit)));
        out.push_str("\r\n");
    }
    out
}

/// Testo a tabulazioni, per chi incolla dove non arriva l'HTML.
pub fn tsv(lines: &[Line<'_>], unit: Unit) -> String {
    let clean = |c: &String| c.replace(['\t', '\n', '\r'], " ");
    let mut out = header(unit).iter().map(clean).collect::<Vec<_>>().join("\t");
    for line in lines {
        out.push('\n');
        out.push_str(&cells(line, unit).iter().map(clean).collect::<Vec<_>>().join("\t"));
    }
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// La tabella HTML: una riga di intestazione per cartella con i suoi totali, poi le pagine, e in
/// fondo il totale del lavoro. Il percorso non c'è: in una mail si legge male.
pub fn html(lines: &[Line<'_>], unit: Unit) -> String {
    const CELL: &str = "border:1px solid #999;padding:3px 6px";
    let head = header(unit);
    let cols = head.len() - 1;
    let mut out = String::from(
        "<table style=\"border-collapse:collapse;font-family:Segoe UI,Arial,sans-serif;font-size:10pt\"><tr>",
    );
    for h in head.iter().take(cols) {
        out.push_str(&format!(
            "<th style=\"{CELL};background:#e6e8ec\">{}</th>",
            escape(h)
        ));
    }
    out.push_str("</tr>");
    let total = |ls: &[&Line<'_>]| {
        let files: BTreeSet<&Path> = ls.iter().map(|l| l.file.path.as_path()).collect();
        let area: f64 = ls.iter().filter_map(|l| l.size).map(area_m2).sum();
        let trim: f64 = ls
            .iter()
            .filter_map(|l| l.size.map(|s| l.trim.unwrap_or(s)))
            .map(area_m2)
            .sum();
        let pages = if ls.len() == 1 { "pagina" } else { "pagine" };
        let mut text = format!(
            "{} file · {} {pages} · {} m²",
            files.len(),
            ls.len(),
            format_number(area, 3, false)
        );
        if (area - trim).abs() > 1e-6 {
            text.push_str(&format!(" (al vivo {} m²)", format_number(trim, 3, false)));
        }
        text
    };
    let mut i = 0;
    while let Some(first) = lines.get(i) {
        let group: Vec<&Line<'_>> = lines[i..]
            .iter()
            .take_while(|l| l.folder == first.folder)
            .collect();
        out.push_str(&format!(
            "<tr><td colspan=\"{cols}\" style=\"{CELL};background:#dbe7fb\"><b>{}</b> &nbsp; {}</td></tr>",
            escape(&first.folder),
            escape(&total(&group))
        ));
        for line in &group {
            out.push_str("<tr>");
            for (k, c) in cells(line, unit).iter().take(cols).enumerate() {
                // Le colonne dei numeri allineate a destra.
                let align = if (4..=9).contains(&k) {
                    ";text-align:right"
                } else {
                    ""
                };
                out.push_str(&format!("<td style=\"{CELL}{align}\">{}</td>", escape(c)));
            }
            out.push_str("</tr>");
        }
        i += group.len().max(1);
    }
    let all: Vec<&Line<'_>> = lines.iter().collect();
    out.push_str(&format!(
        "<tr><td colspan=\"{cols}\" style=\"{CELL};background:#e6e8ec\"><b>Totale</b> &nbsp; {}</td></tr></table>",
        escape(&total(&all))
    ));
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use winfiles_model::PageInfo;

    use super::*;

    fn project() -> Project {
        let mut p = Project::new("Prova");
        let mut libro = FileEntry::new(PathBuf::from("/lavori/Rossi/libro.pdf"), FileKind::Pdf);
        libro.pages = vec![
            PageInfo {
                size: Size::new(216.0, 303.0),
                trim: Some(Size::new(210.0, 297.0)),
            };
            3
        ];
        libro.set_printed(1, false);
        libro.set_note(2, "Controllare;\nil \"nero\"".into());
        let mut foto = FileEntry::new(PathBuf::from("/lavori/Rossi/Foto/vetrina.jpg"), FileKind::Image);
        foto.pages = vec![PageInfo {
            size: Size::new(1000.0, 500.0),
            trim: None,
        }];
        let mut fuori = FileEntry::new(PathBuf::from("/lavori/Rossi/Foto/fuori.png"), FileKind::Image);
        fuori.set_all_printed(false);
        p.files = vec![libro, foto, fuori];
        p
    }

    #[test]
    fn una_riga_per_pagina_in_stampa() {
        let p = project();
        let all = lines(&p, None);
        let pages: Vec<(String, usize)> = all.iter().map(|l| (name(&l.file.path), l.page)).collect();
        assert_eq!(
            pages,
            [
                ("libro.pdf".into(), 0),
                ("libro.pdf".into(), 2),
                ("vetrina.jpg".into(), 0)
            ]
        );
        assert_eq!(all[0].folder, "Rossi");
        assert_eq!(all[2].folder, "Rossi › Foto");
        // Solo i file scelti.
        let only = lines(&p, Some(&BTreeSet::from([1])));
        assert_eq!(only.len(), 1);
    }

    #[test]
    fn il_csv_si_apre_in_excel() {
        let p = project();
        let text = csv(&lines(&p, None), Unit::Cm);
        assert!(text.starts_with('\u{feff}'));
        let rows: Vec<&str> = text.trim_start_matches('\u{feff}').split("\r\n").collect();
        assert_eq!(
            rows[0],
            "Cartella;File;Tipo;Pagina;Base (cm);Altezza (cm);m²;Base al vivo (cm);Altezza al vivo (cm);m² al vivo;Nota;Percorso"
        );
        assert_eq!(
            rows[1],
            "Rossi;libro.pdf;PDF;1 di 3;21,6;30,3;0,0654;21;29,7;0,0624;;/lavori/Rossi/libro.pdf"
        );
        // La nota con punto e virgola e virgolette resta in una cella sola, su una riga.
        assert!(
            rows[2].contains(";\"Controllare; / il \"\"nero\"\"\";"),
            "{}",
            rows[2]
        );
        // Senza TrimBox le colonne al vivo restano vuote ma il m² al vivo c'è; niente migliaia col punto.
        assert_eq!(
            rows[3],
            "Rossi › Foto;vetrina.jpg;Immagine;1 di 1;100;50;0,5000;;;0,5000;;/lavori/Rossi/Foto/vetrina.jpg"
        );
        assert_eq!(rows.len(), 5);
    }

    #[test]
    fn gli_appunti_hanno_testo_e_tabella() {
        let p = project();
        let ls = lines(&p, None);
        let text = tsv(&ls, Unit::Mm);
        assert_eq!(text.lines().count(), 4);
        assert!(
            text.lines()
                .nth(1)
                .unwrap()
                .starts_with("Rossi\tlibro.pdf\tPDF\t1 di 3\t216\t303")
        );
        let table = html(&ls, Unit::Mm);
        assert!(table.contains("<b>Rossi</b> &nbsp; 1 file · 2 pagine · 0,131 m² (al vivo 0,125 m²)"));
        assert!(table.contains("<b>Rossi › Foto</b> &nbsp; 1 file · 1 pagina · 0,500 m²"));
        assert!(table.contains("<b>Totale</b> &nbsp; 2 file · 3 pagine"));
        assert!(table.contains("il &quot;nero&quot;"));
        assert!(!table.contains("/lavori/"));
    }
}
