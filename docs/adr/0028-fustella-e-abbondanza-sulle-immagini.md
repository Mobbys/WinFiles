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

## Conseguenze
- Il PDF salvato si apre nel nesting con la sua fustella, angoli tondi compresi.
- L'immagine salvata con "Salva" (JPEG, TIFF…) porta l'abbondanza ma non il formato finito: chi
  la riapre non sa più dove si taglia. Per Onyx si usa il PDF.
- I CMYK si leggono in RGB come nel resto di Modifica immagini: per un TIFF CMYK il PDF esce in RGB.
- Le fasi dopo (fustella dal contorno del soggetto, abbondanza "colore più vicino" attorno alle
  forme, le immagini dentro i PDF) useranno lo stesso `Op::Bleed` e la stessa uscita in PDF.
