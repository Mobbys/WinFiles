# 0016. Modifica delle immagini: misura di stampa e ricampionamento separati, colore conservato

**Stato:** accettata, 2026-10-15

## Contesto
La 4.9 ha una scheda "Controllo immagini": apre delle immagini, ne dice pixel, modo colore, peso e
rapporto, mette in fila risoluzione e centimetri, ruota, specchia, ritaglia a trascinamento con una
lente e salva con nome. In tipografia però due cose della 4.9 non vanno bene: salva con PIL
**senza il profilo ICC** (una foto in Adobe RGB riaperta diventa sRGB, i colori cambiano) e mescola
in un unico pulsante "Applica dimensioni" due operazioni diverse — cambiare la risoluzione e
rifare i pixel — in modo che spesso non fa nulla.

## Alternative
1. **Copiare la 4.9 così com'è.** Semplice, ma si porterebbero dietro il profilo perso e la
   confusione fra misura e pixel.
2. **Appoggiarsi a PhotoCraft** (`photocraft-raster`, `-ops`, `-cms`): completo, con RAW, PSD e
   gestione colore, ma è un pezzo grosso che entra nell'albero per fare quattro operazioni.
   Resta la strada per i RAW e i PSD, quando serviranno.
3. **Scelta:** un crate nostro, `crates/image_edit`, sopra `image` 0.25 (già in casa per le
   miniature) più `tiff` per scrivere i TIFF. Le foto HEIC le legge `photocraft-heif`, come le
   miniature (ADR 0012).

## Decisioni collegate
- **Due cose separate, non una.** "Misura di stampa" lega larghezza, altezza e risoluzione
  lasciando i pixel dove sono: serve a dire quanto grande va stampata. "Ridimensiona" rifà i
  pixel (Lanczos3) alla misura voluta. Nella 4.9 erano lo stesso pulsante, e il risultato era che
  quasi sempre non ricampionava nulla.
- **Il colore si conserva.** Il profilo ICC incorporato si rilegge e si riscrive (JPEG, PNG, TIFF,
  WebP), e si può togliere con una spunta. `image` 0.25 perde il profilo dei TIFF appena gli si
  danno dei limiti di memoria: il profilo si rilegge dalla sola intestazione (`tiff_icc`).
- **La risoluzione finisce davvero nel file.** `image` non la scrive: nel JPEG si corregge il
  blocco JFIF, nel PNG si aggiunge il riquadro `pHYs`, nel TIFF si scrivono `XResolution` e
  `YResolution`. Senza questo un file stampato altrove esce della misura sbagliata. Il WebP non
  la porta e lo si dice nella finestra di salvataggio.
- **L'orientamento EXIF si applica all'apertura**, come nelle miniature: quello che si vede è
  quello che è, e le misure sono quelle giuste.
- **Annulla e Ripeti non copiano i pixel**: si tiene l'immagine di partenza e la fila delle
  operazioni, e annullare vuol dire rifare la fila. Su una foto da cento megapixel una pila di
  copie finirebbe la memoria.
- **L'originale non si sovrascrive da solo**: il primo salvataggio chiede il nome e ne propone
  uno (`foto_modificata.jpg`), come faceva la 4.9. La scrittura è atomica.
- **La lente del ritaglio mostra i pixel veri** (ritagliati dall'immagine a piena risoluzione),
  non l'anteprima ingrandita: serve proprio a vedere dove cade il taglio.
- Le immagini per lo schermo si rimpiccioliscono in un thread a parte, con l'immagine condivisa
  (`Arc`), così la finestra non si blocca su una foto grande.

## Conseguenze
- WinFiles fa quello che fa "Controllo immagini" della 4.9 e in più: tiene il profilo colore,
  scrive la risoluzione, separa misura e ricampionamento, ha Annulla e Ripeti, sceglie il formato
  e la qualità del JPEG, apre le foto HEIC.
- Una dipendenza in più, `tiff` (MIT), che era già nell'albero sotto `image`.
- Si salva sempre in RGB o in grigio: un JPEG CMYK si legge (convertito) ma non si riscrive in
  CMYK, e il modo di partenza si dice nei dati. Il CMYK, i RAW e i PSD restano per quando
  entrerà PhotoCraft.
