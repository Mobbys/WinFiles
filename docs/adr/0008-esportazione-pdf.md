# 0008. Esportazione in PDF da una scena condivisa con l'anteprima

**Stato:** accettata, 2026-10-14

## Contesto
La Fase 3 chiede di esportare i fogli in un PDF alla misura vera (A4 o A3), base anche per la stampa
della Fase 5. Il PDF deve essere identico all'anteprima, a capo dei testi e "…" compresi, con i
caratteri incorporati, le pagine di PDF e AI nitide e le immagini a una risoluzione da stampa.
Le licenze GPL e AGPL sono escluse, quindi niente MuPDF né Ghostscript.

## Alternative
1. `printpdf`: semplice, ma senza sottoinsiemi di font affidabili né pagine PDF incorporabili
   come vettori; le pagine di PDF e AI diventerebbero immagini.
2. `lopdf` scritto a mano: controllo totale, ma testi, font e copia delle pagine sarebbero tutti da
   scrivere e da mantenere.
3. **Scelta:** `krilla` 0.8 (MIT/Apache, lo stesso autore di `hayro`, usato da Typst): sottoinsiemi
   di font TrueType e immagini. Si usa senza le funzioni predefinite, solo `raster-images` e
   `simple-text`.

## La scena
Il nuovo crate `scene` (L1) descrive un foglio come elenco di rettangoli, linee, righe di testo
già posizionate e miniature, tutto in millimetri. L'app compone la scena di ogni foglio una volta,
impostando i testi con i caratteri di egui a 10 pixel per mm; l'anteprima (`paint.rs`) e il PDF
(`pdf_out`, L2) disegnano la stessa scena. Così il motore di layout resta l'unico che decide dove
va ogni cosa, e il PDF non rifà nessun conto.

## Decisioni collegate
- I testi usano Inter, gli stessi file TTF dell'interfaccia, incorporati come sottoinsieme.
- Ogni miniatura (immagini, pagine di PDF e AI, anteprime degli EPS) diventa un JPEG di qualità
  85 a 150 DPI della sua misura sul foglio, con la stessa decodifica delle miniature
  dell'anteprima (orientamento EXIF compreso), ritagliata al formato al vivo come nell'anteprima.
  La prima versione incorporava le pagine di PDF e AI come vettori e le immagini a 300 DPI: un
  foglio di prova pesava 6,9 MB. Michel ha fatto notare che su una miniatura non serve; ora lo
  stesso foglio pesa meno di 100 KB.
- Un file illeggibile o mancante lascia un riquadro grigio e finisce negli avvisi; l'esportazione
  non si ferma.
- L'esportazione gira in un thread, con il foglio a cui è arrivata e Annulla nella barra di stato;
  il file si scrive prima in un temporaneo e poi si rinomina. A fine lavoro il PDF si apre.
- `--pdf uscita.pdf` sulla riga di comando esporta e chiude: serve agli script e alle prove.

## Conseguenze
krilla 0.8 porta `rustybuzz`, segnalato come non più mantenuto (RUSTSEC-2026-0206, nessuna
falla nota): lo legge solo con Inter, che è nell'app, quindi è ignorato in `deny.toml` finché
krilla non passa a `harfrust`.

Il PDF è leggero qualunque sia la risoluzione dei file originali. Il testo dentro le pagine di PDF
e AI diventa pixel: a 150 DPI si legge fino a corpi piccoli, e il PDF serve a controllare, non a
stampare il lavoro finale. Se servisse più dettaglio, la risoluzione si potrà rendere regolabile.
