# 0028. Fustella e abbondanza sulle immagini

Stato: Accettata (2026-10-30)

## Contesto
Fustella e abbondanza funzionano sui PDF (ADR 0023–0026), ma molti lavori arrivano come immagini
da sole (JPEG, TIFF, PNG, file di Photoshop) o come PDF con dentro una foto. Per Onyx e per il
nesting serve un PDF con la grafica, la linea di taglio in tinta piatta e il formato finito.
Un'immagine non può portare una tinta piatta né un TrimBox: il risultato deve essere un PDF.

## Alternative
1. **Aprire l'immagine in Modifica PDF** (un PDF di una pagina) e usare gli strumenti che ci
   sono: l'abbondanza vettoriale ribalta la pagina disegnata, cioè l'immagine intera, più volte;
   funziona ma il PDF porta più copie della stessa foto e non si vede la risoluzione.
2. **Abbondanza con i pixel in Modifica immagini (scelta)**: l'immagine si allarga con i suoi
   pixel (a specchio o allungando l'ultima fila), come modifica che si annulla; la fustella si
   aggiunge solo nel PDF che si salva.

## Decisione
Alternativa 2.
- `Op::Bleed` in `winfiles-image-edit` (`bleed.rs`) allarga l'immagine per ogni tipo di pixel
  (8 e 16 bit, grigi, trasparenza) senza convertirla; `Picture::trim` ricorda il formato finito
  in pixel e lo segue nelle altre modifiche (rotazioni, specchi, ritagli, ridimensionamenti). Una
  fascia più larga dell'immagine si ribalta di nuovo.
- "Salva PDF con fustella": l'immagine com'è adesso entra a tutta pagina e a piena risoluzione
  (`pdf_out::image_page_bytes`; JPEG di qualità 95 se il file era un JPEG, altrimenti PNG senza
  perdite), il formato finito diventa TrimBox e la pagina BleedBox (`PdfFile::set_trim`), la
  fustella rettangolare o con gli angoli tondi si disegna sul formato finito nella tinta con il
  ruolo Taglio, in sovrastampa, linea da 0,1 mm (`PdfFile::draw_die`, lo stesso codice della
  fustella dalla selezione). Gli angoli tondi sono archi in segmenti di 3°.
- Le scelte (abbondanza, metodo, forma, raggio) si ricordano nelle impostazioni.

## Fase 2: il contorno del soggetto
- Forma "Contorno" (`winfiles_image_edit::trace`): sfondo un colore (scelto col contagocce, con
  il pixel dell'angolo o a mano, più una tolleranza) o la trasparenza. Senza "fori interni" lo
  sfondo parte dai bordi e si allarga ai vicini simili, così il bianco dentro il soggetto resta
  soggetto; con i fori ogni zona del colore di sfondo si taglia.
- Su un'immagine ridotta a 2000 pixel di lato: file di pixel in rettangoli uniti con i_overlay,
  pezzi piccoli tolti, Douglas–Peucker a 0,9 pixel, Chaikin due volte con il taglio limitato a
  1,5 pixel dallo spigolo (un quarto di un lato lungo mangiava la punta di un triangolo), poi
  l'offset con giunti tondi (positivo fuori, negativo dentro).
- Si calcola in un thread quando cambiano l'immagine o le scelte; il PDF si salva solo quando il
  contorno è pronto. Alternativa scartata: tracciare il contorno a vettori (potrace) con una
  dipendenza nuova; i rettangoli uniti bastano e restano nel codice già usato dal nesting.

## Lisciatura e abbondanza lungo il contorno
- "Lisciatura" (raggio in mm): dopo la distanza, chiusura morfologica (fuori e dentro di r:
  riempie le rientranze strette) e apertura (dentro e fuori) con raggio al più la distanza, così
  la lama non entra mai nel soggetto.
- `Op::ContourBleed`: fuori dalla fustella, per la misura scelta, ogni pixel prende il colore più
  vicino dentro la fustella (strati di vicini a 8, dai pixel 2 px dentro la linea per evitare
  l'alone dell'antialias). Dentro non cambia nulla: con un bordo bianco l'abbondanza è bianca,
  con la distanza a 0 continua il soggetto. Scartato "colore del soggetto più vicino" su tutta
  la fascia: riempiva anche il bianco fra le lettere dentro la fustella, cambiando la grafica.
- Dopo l'abbondanza lo sfondo attorno è pieno e un nuovo contorno verrebbe più largo: la
  fustella usata si fissa nella `Picture` (`cut`, in pixel) e la seguono Dpi; le operazioni che
  spostano i pixel la tolgono, Ctrl+Z la riporta indietro.

## Fase 3: Modifica PDF
- "Fustella dal contorno della grafica" nello strumento Tracciati: la pagina si disegna con hayro
  a 2000 pixel sul lato lungo (la misura su cui lavora comunque il motore) e passa da
  `trace_image` con le stesse scelte delle immagini; il contorno si vede sulla pagina prima di
  crearlo e diventa la fustella con `Editor::add_die_outline` (lo stesso `draw_die`).
- "Fustella dal tracciato di ritaglio": `PageScan::clip_outlines` dà i ritagli con una forma vera
  (almeno 8 punti, non rettangoli diritti), cioè gli scontorni di Photoshop e i gruppi di
  ritaglio. Non si propongono più quando la pagina ha già una fustella.
- L'abbondanza resta quella dello strumento Tracciati (specchio della pagina, o la forma
  allungata dalla fustella); quella "con i pixel" per le foto ritagliate è rimandata.

## Conseguenze
- Il PDF salvato si apre nel nesting con la sua fustella, angoli tondi compresi.
- L'immagine salvata con "Salva" (JPEG, TIFF…) porta l'abbondanza ma non il formato finito: chi
  la riapre non sa più dove si taglia. Per Onyx si usa il PDF.
- I CMYK si leggono in RGB come nel resto di Modifica immagini: per un TIFF CMYK il PDF esce in RGB.
- Le fasi dopo (abbondanza "colore più vicino" attorno alle
  forme, le immagini dentro i PDF) useranno lo stesso `Op::Bleed` e la stessa uscita in PDF.
