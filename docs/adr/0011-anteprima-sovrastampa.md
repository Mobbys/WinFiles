# 0011. Anteprima della sovrastampa nelle miniature

**Stato:** accettata, 2026-10-15

## Contesto
Michel vuole che le miniature dei PDF e degli AI mostrino la sovrastampa come Acrobat con
"Anteprima sovrastampa": un testo nero in sovrastampa sul fondo colorato, un colore speciale sopra
la quadricromia, e soprattutto l'errore classico del bianco in sovrastampa, che in stampa sparisce.
`hayro` 0.8 ignora `OP` e `op` degli ExtGState e disegna tutto coprente.

## Alternative
1. Un motore che simula la sovrastampa davvero, separazione per separazione (MuPDF, Ghostscript):
   esatto, ma GPL/AGPL, vietati.
2. Rasterizzare per lastre in CMYK con un nostro motore: esatto, ma è un motore PDF nuovo.
3. **Scelta:** una copia di `hayro-interpret` 0.8.0 in `vendor/` che legge `OP`/`op` e disegna
   gli inchiostri in sovrastampa (DeviceCMYK, ICC a 4 canali, Separation, DeviceN) con il blend
   Multiply, che `hayro` sa già fare. È l'approssimazione dei visualizzatori leggeri: gli
   inchiostri si sommano come sulla carta, il bianco in sovrastampa sparisce, il nero sopra il
   colore resta nero.

## Decisioni collegate
- Si cambia solo `hayro-interpret`; il disegno resta di `hayro`. Le modifiche sono segnate
  `WinFiles:` e descritte in `vendor/README.md`, dove sta anche come aggiornarle.
- La sovrastampa vale per riempimenti, tratti e testi; le immagini restano coprenti (le immagini
  in sovrastampa sono rare nei file di stampa). `OPM` non cambia nulla: con Multiply i canali a
  zero non coprono comunque, come con `OPM 1`.
- Un blend già scelto dal file (diverso da Normal) vince sulla sovrastampa.
- Miniature dell'anteprima e immagini nel PDF esportato e stampato vengono dallo stesso disegno:
  la sovrastampa si vede ovunque.

## Conseguenze
- Gli EPS mostrano l'anteprima incorporata nel file, che resta com'è.
- Su colori RGB in sovrastampa non cambia nulla, come in Acrobat.
- Da proporre a monte (`LaurenzV/hayro`); quando ci sarà, `vendor/` si svuota.
