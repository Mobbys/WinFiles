# 0033. DXF e DWG: solo da guardare e stampare

**Stato:** accettata, 2026-11-06

## Contesto
Michel ha chiesto di vedere anche DXF e DWG, appoggiandosi a storytold/cadcraft, solo per guardarli
e stamparli in modo semplice: si modificano con i loro programmi.

## Alternative
1. **Tutto cadcraft (cadcraft-io, craft-pdf, la sua interfaccia).** Porta krilla, hayro ed egui
   in versioni modificate via `[patch]`, in conflitto con le nostre (lo stesso problema di
   VectorCraft): non compila insieme a WinFiles.
2. **Convertire con un programma esterno (ODA, LibreCAD).** Un'installazione in più, licenze GPL o
   proprietarie.
3. **Scelta: solo i lettori di cadcraft (`cadcraft-dxf`, `cadcraft-dwg`, `cadcraft-color`, commit
   fissato) e il disegno fatto da noi.**

## Decisione
- Il crate `winfiles-cad` (livello L1) legge le entità in `Drawing`:
  - LINE, polilinee con bombature, archi, cerchi, ellissi, spline, solidi, tratteggi pieni e
    contorni, testi e MTEXT, blocchi (anche in serie e annidati, fino a 16 livelli), quote e
    LEADER;
  - unità da `$INSUNITS`, layer con colore e stato.
  - Il resto si conta e lo si dice nel pannello.
- Il disegno diventa un PDF vettoriale scritto a mano (Helvetica WinAnsi, flusso compresso). La
  scheda lo disegna come gli altri PDF, quindi lo zoom resta nitido e la stampa usa la stessa
  strada.
  - Sullo schermo il fondo è nero e il colore 7 è bianco, come nei CAD; dal pannello Layer si
    sceglie il fondo bianco.
  - In stampa tutto è in nero, a 1:N o adattato.
- I DWG (R13–2018) si leggono con acadrust 0.6.6, **MPL-2.0**, non modificato. È ammesso in
  `deny.toml` solo per quel crate: MPL vale file per file e non tocca il nostro codice. È fermo a
  0.6.6 perché 0.6.7/0.6.8 leggono vuoti alcuni DWG rovinati (nota di cadcraft).
  - Il lettore gira in una copia di WinFiles (`--dwg-worker`): un file rovinato che lo fa cadere
    non chiude la finestra.
  - I DWG prima di R13 si rifiutano con un messaggio.
- Limiti: 200 MB per file, 2 milioni di forme, 100 000 punti per forma, pagina PDF fino a 5,08 m
  (oltre si guarda ridotto e righelli e quote dicono i mm veri).
