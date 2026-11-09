# 0013. Miniature degli AI in PostScript ("legacy") e degli EPS di Illustrator senza anteprima

**Stato:** accettata, 2026-10-15

## Contesto
Gli AI salvati nel formato "legacy" di Illustrator (versioni 3–8, usato ancora per i plotter da
taglio) non sono PDF: sono PostScript con il prologo di Illustrator solo nominato, non incluso.
`hayro` non li apre e non hanno un'anteprima: la miniatura restava grigia (la 4.9 li segnava
"AI (Non compatibile)"). Le misure c'erano già, dal `%%BoundingBox`.

## Alternative
1. Il lettore PostScript di VectorCraft (`vectorcraft-eps` e `vectorcraft-render`): completo, ma
   richiede `craft-pdf` di craft-libs con le sue copie di `hayro` 0.7 e `krilla`, che non convivono
   con il nostro `hayro` 0.8 (e la sovrastampa dell'ADR 0011). Sarebbe un cambio del motore PDF.
2. Ghostscript: GPL/AGPL, vietato.
3. **Scelta:** una traduzione degli operatori di disegno di Illustrator (pubblici: Adobe Illustrator
   File Format Specification 7.0) in una pagina PDF, disegnata dal nostro `hayro` come gli altri
   PDF (`crates/thumbs/src/legacy_ai.rs`). Sono quasi gli stessi operatori del PDF.

## Decisioni collegate
- Si leggono tracciati, tracciati composti (un solo riempimento, con i buchi), maschere, colori in
  grigio, quadricromia, tinta piatta (con la percentuale) e RGB, spessori e tratteggi.
- Testi, immagini, sfumature e motivi non si disegnano: per una miniatura bastano le forme, e i file
  da taglio sono fatti di sole forme (i testi sono già convertiti in tracciati).
- Vale per i file PostScript scritti da Illustrator (`%%Creator: Adobe Illustrator` o commenti
  `%AI…`): gli `.ai` e gli EPS senza anteprima TIFF. Gli altri PostScript restano senza anteprima.
- Tetti su comandi letti, contenuto scritto, valori in attesa e annidamenti: un file rovinato dà
  una miniatura parziale o un errore, mai un crash.

## Conseguenze
- Il PDF esportato e la stampa usano la stessa miniatura.
- Se un giorno il motore PDF passerà alle copie di craft-libs, il lettore di VectorCraft potrà
  sostituire questa traduzione e aggiungere testi e sfumature.
