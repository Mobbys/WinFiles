//! La copia di Illustrator dentro un PDF (ADR 0020).
//!
//! Un PDF salvato da Illustrator con "Mantieni le funzioni di modifica di Illustrator" porta in
//! `/PieceInfo /Illustrator` della pagina il file .ai intero. Illustrator, riaprendo il PDF, legge
//! quella copia e non le pagine: le modifiche fatte altrove non le vede. Togliere la voce lascia
//! le pagine come sono; la copia, non più raggiungibile, non viene scritta nel file salvato.

use pdfcraft_cos::{Dict, Document, ObjRef, Object};

use crate::EditError;

/// Gli oggetti (pagine, moduli, catalogo) con la copia di Illustrator.
fn holders(doc: &Document) -> Vec<ObjRef> {
    let mut out = Vec::new();
    for (r, obj) in doc.scan_objects_subset(doc.object_numbers()) {
        let Ok(obj) = obj else { continue };
        let dict = match &*obj {
            Object::Dict(d) => d,
            Object::Stream(s) => &s.dict,
            _ => continue,
        };
        let piece = dict
            .get(b"PieceInfo")
            .and_then(|p| doc.dict(p))
            .unwrap_or_default();
        if piece.get(b"Illustrator").is_some() {
            out.push(r);
        }
    }
    out
}

pub(crate) fn has(doc: &Document) -> bool {
    !holders(doc).is_empty()
}

/// Toglie la copia di Illustrator dove c'è; le altre voci di `/PieceInfo` restano. Restituisce
/// quanti oggetti sono cambiati.
pub(crate) fn strip(doc: &mut Document) -> Result<usize, EditError> {
    let found = holders(doc);
    for &r in &found {
        let mut copy = (*doc.get(r)).clone();
        let dict: &mut Dict = match &mut copy {
            Object::Dict(d) => d,
            Object::Stream(s) => &mut s.dict,
            _ => continue,
        };
        let mut piece = dict
            .get(b"PieceInfo")
            .and_then(|p| doc.dict(p))
            .unwrap_or_default();
        piece.remove(b"Illustrator");
        if piece.iter().next().is_none() {
            dict.remove(b"PieceInfo");
        } else {
            dict.set(b"PieceInfo".to_vec(), Object::Dict(piece));
        }
        doc.set(r, copy);
    }
    Ok(found.len())
}
