# 0009. Stampa diretta con `craft-print`, dallo stesso PDF dell'esportazione

**Stato:** accettata, 2026-10-15

## Contesto
La 4.9 stampa passando dal PDF e da un programma esterno. WinFiles deve stampare i fogli A4 e A3
alla misura vera, con un pulsante Stampa grande e blu e una finestra con stampante, copie, fronte e
retro e intervallo di fogli. Le misure sul foglio devono restare esatte al millimetro.

## Alternative
1. Aprire il PDF esportato con il lettore predefinito e lasciare la stampa a lui: nessun codice,
   ma ogni lettore applica la sua scala ("Adatta") e le misure non sono più garantite.
2. Disegnare la scena direttamente su un contesto GDI di Windows: controllo totale, ma un secondo
   disegnatore da tenere identico al PDF, e codice `unsafe` vietato nel workspace.
3. **Scelta:** `craft-print` di `storytold/craft-libs` (MIT/Apache, commit fissato), che manda allo
   spooler lo stesso PDF dell'esportazione: su Windows lo rasterizza in processo con `winprint`, su
   Linux lo passa a CUPS con `lp`. Scala esatta al 100% oppure riduzione all'area stampabile.

## Decisioni collegate
- La stampa riusa il thread dell'esportazione (`export.rs`): prima si scrive il PDF dei fogli
  scelti, poi si invia. Annulla funziona finché il PDF si sta scrivendo; l'invio allo spooler non si
  può interrompere, e la barra di stato lo dice.
- La finestra ricorda le scelte per la sessione; la stampante predefinita è preselezionata. Se la
  stampante dichiara di non avere il fronte e retro, il lavoro va comunque in solo fronte.
- Al 100% una stampante senza il formato rifiuta il lavoro invece di ridurlo di nascosto.
- Dipendenze in più, controllate con `cargo deny`: `winprint` da git (`storytold/winprint.rs`,
  aggiunto ad `allow-git`); la licenza CDLA-Permissive-2.0 di `webpki-root-certs`, usato solo in
  compilazione; l'avviso RUSTSEC-2024-0370 su `proc-macro-error`, una macro di compilazione di
  `winprint` senza codice nell'eseguibile.

## Conseguenze
- Anteprima, PDF e stampa disegnano la stessa scena: una sola fonte delle posizioni.
- Manca ancora la pagina di calibrazione con le misure di controllo (Fase 5) e la memoria delle
  scelte di stampa tra una sessione e l'altra.
