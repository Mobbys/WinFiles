# 0005. Misure dei file lette dalle intestazioni, aree al formato finito

**Stato:** accettata, 2026-10-14

## Contesto
Chi stampa ha bisogno di vedere per ogni file le misure in mm o cm e i metri quadri, per file,
per cartella e in totale. Le misure servono anche al layout, per le proporzioni vere delle
miniature. Leggere i file per intero (decodificare immagini e PDF) costerebbe secondi per file.

## Alternative
1. Decodificare tutto con le librerie della Fase 2 (photocraft, hayro-interpret): lento e con
   molte più dipendenze solo per sapere due numeri.
2. Crate generici come `imagesize`: danno i pixel ma non la risoluzione, che è ciò che serve per
   i millimetri.
3. **Scelta:** un crate `winfiles-formats` che legge solo le intestazioni: JPEG (JFIF ed EXIF, con
   l'orientamento), PNG (`pHYs`), TIFF, GIF, BMP e WebP scritti a mano e testati byte per byte;
   PDF e AI con compatibilità PDF con `hayro-syntax` senza i decodificatori di immagini;
   EPS e AI PostScript dai commenti DSC, anche con intestazione binaria DOS.

## Decisione
- Le immagini senza risoluzione dichiarata usano 96 DPI come la 4.9, e l'app lo segnala in
  arancione: le misure vanno verificate.
- Il formato pagina di un PDF è il CropBox dentro il MediaBox, ruotato come lo mostrano i lettori;
  il TrimBox, se c'è, è il formato finito. **Le aree in m² usano il formato finito**, cioè ciò che
  si consegna, non l'abbondanza.
- La lettura avviene in un thread a parte; ogni numero letto ha un tetto (pixel, pagine, byte) e un
  file rovinato dà un errore mostrato sul file, mai un crash. Un test di proprietà prova migliaia
  di file casuali.

## Conseguenze
Le miniature vere (Fase 2) useranno altri decodificatori, ma le misure restano queste: veloci e
indipendenti. Un PDF si legge tutto in memoria (tetto 2 GiB) perché `hayro-syntax` lavora su un
buffer; se i file di rete molto grandi risulteranno lenti si leggerà solo la coda con la xref.
