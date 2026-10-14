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
   di font TrueType, immagini, e `draw_pdf_page` che incorpora una pagina di un altro PDF come
   vettore. Si usa senza le funzioni predefinite, solo `raster-images`, `simple-text` e `pdf`.

## La scena
Il nuovo crate `scene` (L1) descrive un foglio come elenco di rettangoli, linee, righe di testo
già posizionate e miniature, tutto in millimetri. L'app compone la scena di ogni foglio una volta,
impostando i testi con i caratteri di egui a 10 pixel per mm; l'anteprima (`paint.rs`) e il PDF
(`pdf_out`, L2) disegnano la stessa scena. Così il motore di layout resta l'unico che decide dove
va ogni cosa, e il PDF non rifà nessun conto.

## Decisioni collegate
- I testi usano Inter, gli stessi file TTF dell'interfaccia, incorporati come sottoinsieme.
- Le pagine di PDF e AI si incorporano come vettori, ritagliate al formato al vivo come
  nell'anteprima. I PDF 2.0, quelli oltre 512 MB e una pagina che non c'è passano all'immagine:
  krilla altrimenti farebbe fallire l'intero documento.
- Immagini ed EPS diventano JPEG di qualità 90 a 300 DPI, al massimo 4096 pixel di lato, con
  la stessa decodifica delle miniature (orientamento EXIF compreso).
- Un file illeggibile o mancante lascia un riquadro grigio e finisce negli avvisi; l'esportazione
  non si ferma.
- L'esportazione gira in un thread, con il foglio a cui è arrivata e Annulla nella barra di stato;
  il file si scrive prima in un temporaneo e poi si rinomina. A fine lavoro il PDF si apre.
- `--pdf uscita.pdf` sulla riga di comando esporta e chiude: serve agli script e alle prove.

## Conseguenze
krilla 0.8 porta `rustybuzz`, segnalato come non più mantenuto (RUSTSEC-2026-0206, nessuna
falla nota): lo legge solo con Inter, che è nell'app, quindi è ignorato in `deny.toml` finché
krilla non passa a `harfrust`.

Un'immagine molto grande pesa nel PDF (circa 1-3 MB per miniatura a 300 DPI); se i PDF diventano
troppo pesanti si potrà scegliere la risoluzione. Le fotografie restano JPEG anche quando il file
era PNG: per le miniature di controllo va bene, per stampe finali no, ma non è lo scopo dell'app.
