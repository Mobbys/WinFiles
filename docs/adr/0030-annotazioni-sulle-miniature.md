# 0030. Annotazioni sulle miniature, salvate nel lavoro

**Stato:** accettata, 2026-11-06

## Contesto
Michel vuole poter segnare le miniature in Miniature: testo, linee con frecce, rettangoli, cerchi e
ovali con contorno o riempimento, un evidenziatore, e ritrovarli nel file `.winfiles`. La 4.9 non
aveva strumenti di disegno (solo le annotazioni scritte, che in WinFiles sono le note per pagina).

## Alternative
1. **Annotazioni libere sul foglio, in mm.** Semplici, ma restano ferme quando cambiano colonne,
   formato, "Su N fogli" o l'ordine dei file: la freccia finisce sulla miniatura sbagliata.
2. **Disegnare dentro il file (PDF o immagine).** Cambia il file vero, che WinFiles in Miniature
   non tocca mai.
3. **Scelta: annotazioni attaccate alla pagina di un file, in frazioni della miniatura.**
   Seguono la miniatura ovunque vada; spessori e corpi restano in mm sulla carta.

## Decisione
- `winfiles_model::Annotation` (linea con punte, rettangolo, ovale, testo, evidenziatore, quota)
  in `FileEntry::annotations`, per pagina come le note; i punti sono in frazioni della parte
  mostrata della pagina (il formato finito, per i PDF con TrimBox) e possono uscire fino al bordo
  della cella. Un lavoro senza annotazioni non scrive il campo; i valori letti da un file sono
  ripuliti (`Annotation::sanitize`, tetti a punti, annotazioni per pagina e lettere).
- La scena ha un elemento nuovo `Item::Path` (spezzata o poligono con capi e giunzioni tondi):
  ovali, punte di freccia e tratti dell'evidenziatore. Anteprima (`paint.rs`) e PDF (`pdf_out`)
  lo disegnano uguale; la stampa usa lo stesso PDF, calibrazione compresa.
- `canvas::compose` disegna le annotazioni dopo didascalie e note, solo nella griglia e con
  `PrintSettings::show_annotations` (casella "Mostra annotazioni"). Elenco e Nesting non le
  mostrano. Il riepilogo scrive solo "N annotazioni sulla miniatura".
- `apps/winfiles/src/annotate.rs`: il tasto Annota apre una riga di strumenti sopra l'anteprima
  (non nella barra in alto, che deve stare a 1366 px). Mentre si traccia, l'annotazione cambia
  nella copia mostrata (`WinFilesApp::shown`); a fine gesto passa nel lavoro con un passo da
  annullare (Ctrl+Z / Ctrl+Y, cercando il file per percorso). Così l'impaginazione non si rifà a
  ogni movimento del mouse.
- Con Annota aperta un clic sceglie annotazioni e non file, e Canc toglie l'annotazione scelta,
  mai il file. Ctrl+C / Ctrl+V copia un'annotazione sulla miniatura sotto il puntatore.
- La quota riusa le maniglie e la misura delle Quote (ADR 0021): la misura scritta è quella vera
  della pagina (`PageInfo::finished`), qualunque sia la grandezza della miniatura.
- Lo stile delle annotazioni nuove si ricorda in `impostazioni.json`.

- (2026-11-06, dopo la prova di Michel) Le annotazioni possono stare ovunque sul foglio: un
  punto fuori dalla cella resta in frazioni della miniatura più vicina, e `compose` disegna tutte
  le annotazioni dopo tutte le miniature. Il capo `b` di una linea che finisce nella cella di
  un'altra miniatura si attacca a quella (`Anchor`: percorso del file, pagina, punto in frazioni);
  se quella miniatura va su un altro foglio vale `b`, tenuto dentro il foglio. Mentre si modifica,
  la linea è portata nelle frazioni della sua miniatura (`annotate::resolve`) e si riattacca
  quando passa nel lavoro (`anchor_lines`).
- Quote: `Quota::value` è la misura scritta a mano (mm); la prima quota della pagina che ce l'ha è
  la master e dà la scala alle altre della stessa pagina, come le Quote dei moduli di modifica.

## Conseguenze
- Un lavoro annotato riaperto e risalvato con una WinFiles fino alla 0.2.25 perde le annotazioni
  (il campo non è conosciuto): vanno aggiornati tutti i PC.
- Il riempimento con trasparenza vale per le forme convesse (rettangoli, ovali, punte): basta per
  gli strumenti di oggi.
