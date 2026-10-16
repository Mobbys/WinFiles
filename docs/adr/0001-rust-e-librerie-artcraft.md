# 0001. Rust con le librerie del team ArtCraft

**Stato:** accettata, 2026-10-14

## Contesto
WinFile 4.9 è in Python con customtkinter, PyMuPDF (licenza AGPL) e una stampa che passa da una
pagina HTML rasterizzata con html2canvas. Serve un'app veloce, stabile, distribuibile come singolo
eseguibile Windows e con stampa a scala esatta su A4 e A3.

## Decisione
Rust, interfaccia egui, e riuso delle librerie open source (MIT o Apache-2.0) del team ArtCraft:
`craft-libs` (docking, stampa Windows, colore ICC, salvataggi atomici, impostazioni), il crate EPS
di VectorCraft, PdfCraft per le operazioni sulle pagine PDF, PhotoCraft per i formati immagine.
PDF e AI si leggono con `hayro` (Rust puro); PDFium resta il ripiego se `hayro` risulta lento.

## Alternative
- Python ripulito: niente risolve la stampa a scala esatta né la distribuzione.
- Tauri con interfaccia web: rimette nel mezzo un browser, la causa dei problemi di stampa di oggi.
- MuPDF o Ghostscript: licenza AGPL, incompatibile con un'app distribuita senza sorgenti.

## Conseguenze
Dipendiamo da progetti di terzi alla versione 0.x che cambiano spesso: si usano da git con un
commit fissato, gli aggiornamenti sono PR dedicate, e se un progetto si ferma il codice si copia
nel repository (la licenza lo permette).
