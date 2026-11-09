# 0025. Fustella dai tracciati scelti, unendo le forme

Stato: Accettata (2026-10-29)

## Contesto
Molti file arrivano senza fustella: c'è solo la grafica (uno sfondo, più forme). Michel vuole
scegliere lo sfondo, o più oggetti, e ottenere la fustella sul loro contorno, tenendo gli
originali; se le forme si sovrappongono la fustella deve essere una sola, sull'unione.

## Alternative
1. **Copiare il tracciato com'è** (curve comprese) in tinta: perfetto per una forma sola, ma con
   più forme che si sovrappongono resterebbero linee interne da tagliare.
2. **Unione booleana scritta da noi**: lavoro lungo e delicato (intersezioni, casi degeneri).
3. **`i_overlay` (scelta)**: unione di poligoni in Rust puro, MIT/Apache, senza dipendenze
   esterne oltre quelle dello stesso autore, robusta sui casi degeneri.

## Decisione
Alternativa 3, in `crates/pdf_edit/src/paths.rs` (`add_die`). I contorni sono quelli che la
lettura della pagina già calcola (curve spezzate: errore sotto 0,05 mm anche su archi da un
metro). Ogni tracciato si pulisce da solo con la regola pari-dispari (i buchi restano buchi), poi
tutti insieme si uniscono con la regola non-zero. La fustella si disegna sopra la pagina in una
tinta piatta nuova, in sovrastampa, con il contenuto di prima chiuso tra `q` e `Q`.

## Conseguenze
- La fustella è fatta di segmenti dritti, non di curve: per i plotter di taglio va bene, ma il
  file è un po' più pesante.
- Nell'abbondanza (ADR 0024) i tracciati scelti si possono allungare da soli: il resto della
  pagina, fustella compresa, resta sopra com'è.
