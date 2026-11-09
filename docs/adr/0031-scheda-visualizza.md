# 0031. La scheda Visualizza

**Stato:** accettata, 2026-11-06

## Contesto
Michel vuole usare WinFiles anche per guardare i file: immagini, PDF, AI, EPS e anche DXF/DWG.
Vuole le misure vere in mm/cm e m², lo zoom e le tinte piatte dei vettoriali. Vuole aprire un
file con il doppio clic da Esplora file (ADR 0032) e poi passarlo a una delle altre schede. La 4.9
non aveva un visualizzatore.

## Alternative
1. **Una finestra a parte, un programma piccolo.** Parte più in fretta, ma è un secondo programma
   da installare e aggiornare, e il "Passa a" diventerebbe un passaggio fra processi.
2. **L'anteprima di Modifica PDF in sola lettura.** Copre solo i PDF, e da sola lettura a modifica
   il passo è troppo facile: si salva per sbaglio.
3. **Scelta: una scheda a sé, la prima a sinistra, che non cambia mai il file.**

## Decisione
- `Module::Viewer` è la prima scheda: linguette per i file aperti, frecce (← →) per i file della
  cartella in ordine naturale. Se il file arriva dalle miniature (doppio clic su una miniatura o
  su un file dell'elenco), le frecce scorrono invece i file del lavoro, e c'è "← Miniature".
- Il disegno passa da `viewer/load.rs`.
  - Un thread apre il file; un altro disegna la pagina intera ai lati 512–4096.
  - Ingrandendo oltre la pagina intera disegna il **pezzo che si vede** alla risoluzione dello
    schermo (`PdfPages::render_region`). Il vettoriale resta nitido a ogni zoom; delle immagini
    si ritagliano i pixel veri.
  - Gli AI in PostScript e gli EPS di Illustrator passano da `illustrator_pdf`; gli altri EPS
    usano l'anteprima.
- Il pannello Misure mostra: formato, al vivo (TrimBox), abbondanza per lato, m² al vivo e con
  abbondanza; per le immagini pixel, DPI, colore e profilo. I dati del PDF vengono da
  `pdf_edit::Details` (PDF/X, profilo di uscita, programma). Il pannello ha anche "Copia misure".
- Il pannello Tinte usa `pdf_edit::spots`.
  - Un clic su una tinta **sbiadisce il resto** con un velo bianco.
  - I tracciati che la usano (da `PdfFile::scan_page`) vengono ripassati nel loro colore.
  - Con la tinta c'è il ruolo Onyx dalle tinte di Modifica PDF e la lunghezza delle linee della
    pagina. Testi e immagini in tinta non si ripassano.
- Righelli in mm dall'angolo della pagina (in metri per i disegni grandi), riquadri al vivo e
  abbondanza tratteggiati, quote come in Modifica PDF.
- "Passa a" (Ctrl+1…6): Miniature, Modifica PDF, Tracciati, Modifica immagini, Fustella, Nesting.
  Le voci che non valgono per quel file sono spente; i DXF/DWG non si passano (scelta di Michel:
  si modificano nei loro programmi).
- La **pagina Stampa** (`viewer/print.rs`) occupa tutta la scheda, con l'anteprima dei fogli.
  - Scale: A4/A3, misura vera, percentuale, adatta al foglio, 1:N per i disegni.
  - Tasselli con sovrapposizione, crocette agli angoli e nome (A1, B2…), più il bordo non
    stampabile.
  - Il PDF di stampa è vettoriale (`nest_document`), con la calibrazione della stampante nella
    matrice (ADR 0010).
  - Le scelte restano in `impostazioni.json`, tranne le copie.
