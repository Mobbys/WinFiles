# 0023. Tracciati e fustelle: scegliere una linea sulla pagina e cambiarne colore, tinta e spessore

**Stato:** accettata, 2026-10-29

## Contesto
Nella stampa di medio e grande formato (RIP Onyx) i file dei clienti arrivano spesso con la
fustella sbagliata: un filetto in quadricromia (per esempio magenta 100%) al posto di una linea in
tinta piatta con il nome che legge il RIP, oppure senza abbondanza. Michel ha chiesto (29 ottobre
2026) di gestire abbondanze, fustelle e poi il nesting, partendo dalle fustelle: nomi delle tinte
liberi (taglio, cordonatura e altro), e la possibilità di cliccare un tracciato con il mouse,
vederne le caratteristiche e cambiarle. Lo studio di fattibilità è nel documento "Abbondanze,
fustelle e nesting: fattibilità" del progetto.

## Alternative
1. **Cambiare il colore per valore in tutta la pagina** ("ogni contorno magenta 100% diventa
   CutContour"). Semplice, ma tocca anche la grafica dello stesso colore e non lascia vedere cosa
   cambia.
2. **Ridisegnare la pagina** (hayro + krilla): perde tinte, sovrastampe e struttura (ADR 0015).
3. **Scelta: leggere i tracciati e cambiare solo quelli scelti.** `crates/pdf_edit/src/paths.rs`
   interpreta il contenuto con `pdfcraft-content` (matrice corrente, `q`/`Q`, spazi colore,
   spessore, sovrastampa, ritaglio, moduli annidati) e restituisce ogni tracciato dipinto con i
   punti in millimetri della pagina come si vede, colore, spessore, lunghezza e la posizione
   dell'operatore che lo dipinge. Il cambio mette `q`, il colore nuovo (una `Separation` con il
   nome scelto e il colore CMYK di prova), lo spessore e la sovrastampa prima del tracciato, e `Q`
   dopo l'operatore: il resto del contenuto resta identico.

## Decisioni collegate
- Un tracciato che è anche ritaglio (`W n` o `W S`) non si cambia: chiudere lo stato grafico dopo
  di lui toglierebbe il ritaglio al resto. Il pannello lo dice.
- Un tracciato dentro un modulo disegnato più volte cambia in tutte le copie; il pannello lo dice.
- La tinta nuova prende come colore di prova quello che il tracciato aveva (se era in
  quadricromia), così a schermo e nelle prove non cambia aspetto; senza, quello del ruolo
  (taglio magenta, cordonatura ciano…). Taglio, cordonatura, bianco e vernice vanno in sovrastampa.
- I nomi delle tinte con il loro ruolo stanno in `impostazioni.json` (`inks`), il più recente in
  cima; il ruolo di un nome sconosciuto si indovina dalle parole (le stesse del pannello di
  nesting per Illustrator, `Mobbys/illustratorNesting`).
- Il controllo della pagina misura l'abbondanza dal TrimBox, o dalla fustella in tinta di taglio,
  o dal filetto sospetto: grafica (tracciati, immagini, sfumature, testo escluso) oltre quel
  formato, per lato, tagliata alla pagina visibile.
- La lettura della pagina gira in un thread a parte (`Snapshot::scan_page`), con tetti a
  tracciati, punti e operatori per le pagine enormi.

## Conseguenze
- Il lettore dei tracciati serve anche alle prossime fasi: abbondanza attorno a una fustella
  sagomata e nesting, che hanno bisogno della forma della fustella in millimetri.
- Il testo non si sceglie come tracciato (resta fuori anche dal calcolo dell'abbondanza).
