# 0034. SVG e file del plotter (PLT/HPGL) in Visualizza

**Stato:** accettata, 2026-11-09

## Contesto
Michel ha chiesto di aprire in Visualizza anche SVG, PLT, foto RAW e immagini semplici (GIF, TGA,
ICO), con misure vere e stampa come gli altri file.

## Alternative
- **SVG**
  1. Disegnarlo come immagine con resvg (già nel progetto con egui_extras): lo zoom perde nitidezza
     e la stampa non è vettoriale.
  2. Scrivere a mano il PDF dall'albero di usvg: sfumature, maschere, filtri e testi andrebbero
     rifatti.
  3. **Scelta: `svg2pdf` 0.13 (MIT/Apache, typst), sopra lo stesso usvg 0.45 che c'è già.** L'SVG
     diventa un PDF vettoriale e segue la strada dei PDF: zoom nitido, Quote, Stampa.
- **PLT/HPGL**
  1. Trattarlo come immagine: niente misura vera.
  2. **Scelta: un lettore nostro (`winfiles-cad::hpgl`).** Penna su e giù, coordinate assolute e
     relative, cerchi e archi, a 40 unità per mm. Ogni penna è un layer, così il file segue la
     strada dei DXF: fondo scuro, layer, stampa 1:N.

## Decisione
- `svg2pdf` e `usvg` (con i caratteri del sistema per i testi) entrano in `winfiles-cad`. Le nuove
  dipendenze sono tutte MIT/Apache: fontdb, rustybuzz, subsetter, pdf-writer 0.12.
- La misura dell'SVG viene dalle sue unità (mm, cm, in); i pixel valgono 1/96 di pollice, come nei
  browser e in Illustrator.
- Il file HPGL è ostile come gli altri:
  - massimo 5 milioni di punti, 256 penne e coordinate fino a 100 km;
  - i comandi sconosciuti si contano e si saltano;
  - i preamboli PCL e PJL si saltano.
- GIF, TGA e ICO passano dal crate `image` (TGA e ICO sono due funzioni in più, senza
  dipendenze). AVIF resta fuori: per leggerlo `image` userebbe dav1d, che è in C.
- Le foto RAW si aprivano già; ora sono anche fra i tipi di "Programma predefinito…".

## Conseguenze
- Un SVG con caratteri che il PC non ha usa quelli che usvg trova al loro posto.
- Nei PLT i testi (LB) non si disegnano: si contano nel pannello.
