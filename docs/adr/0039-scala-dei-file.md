# 0039. Scala dei file: disegni 1:10, 1:20… segnati con un clic, misure vere ovunque

**Stato:** accettata, 2026-11-16

## Contesto
In ditta arrivano tavole e bozzetti disegnati in piccolo: un PDF di 594 × 420 mm a 1:20 si
stampa in realtà 11,88 × 8,40 m. Finora WinFiles mostrava solo la misura del file, e m² e
totali erano sbagliati. Il Nesting aveva già una scala per file (fino a 1:100), ma solo lì.
Michel (2026-11-16) vuole segnare la scala con un clic in Miniature e in Visualizza, e ha scelto
di vedere **la misura vera in grande e quella del file sotto**.

## Alternative
1. **Scala per file.** Più semplice, ma un PDF di tavole ha spesso pagine in scale diverse.
2. **Scala nelle impostazioni di stampa** (una per lavoro): non regge un lavoro misto.
3. **Scelta: scala per pagina**, come le note e le annotazioni (`FileEntry.scales`, chiave la
   pagina da 0, valore N di 1:N; salvata nel `.winfiles` come `"scala"`, assente se tutto 1:1).
   Il clic su una pillola la mette a tutte le pagine dei file scelti; il timbro e il tasto destro
   «Scala solo della pagina N» a una pagina sola.

## Decisione
- Il modello dà le misure vere (`real_page`, `real_pages`, `PageInfo::scaled`): aree, `Totals`,
  riepilogo, tabella CSV/appunti, elenco dei file e proprietà le usano. Tetto 1:1000 (`MAX_SCALE`).
- Didascalia: «Reale …  ·  m²», «Al vivo: …» (vero), poi «File 1:20: …» più chiara.
- Bollino «1:20» in basso a destra della miniatura (in alto c'è «Pag. 2/4»; scelta di Michel), ardesia scura con testo bianco: colori di
  carta, uguali nei due temi, leggibili anche in bianco e nero; casella «Bollino della scala».
- Le quote delle annotazioni misurano la lunghezza vera.
- Nella vista Nesting delle Miniature il disegno sta alla sua misura vera fra gli altri.
- Nella tabella CSV la scala è il solo numero N (colonna «Scala (1:N)»): «1:20» Excel lo
  leggerebbe come un'ora.
- Interfaccia: pulsante «Scala» nell'intestazione dell'anteprima (in tutte le viste), che apre
  una riga di pillole 1:1…1:100, «Altra 1:N», interruttore Timbro (Esc lo spegne). Niente doppio
  clic sulle pillole per il timbro: il primo clic metterebbe già la scala ai file scelti. Annota e Scala non stanno aperte insieme.

## Dopo (fasi successive)
Visualizza e Quote dei moduli di modifica; «Manda al nesting» con la scala; salvataggio nel
file condiviso della cartella `note.winfiles`, insieme alle annotazioni.
La scala si mette solo a mano: niente riconoscimento dal nome del file (Michel, 2026-11-16: in
ditta la scala non dipende dal nome).
