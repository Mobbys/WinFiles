# 0015. Modifica dei PDF con pdfcraft, senza ridisegnare le pagine

**Stato:** accettata, 2026-10-15

## Contesto
La 4.9 ha una scheda "Controllo PDF" che unisce, divide, ruota, elimina pagine e adatta il formato.
Serve la stessa cosa in WinFiles, ma su file veri di tipografia: PDF/X con intenti di colore,
tinte piatte, trasparenze, segnalibri, annotazioni, livelli. Un modulo che *ridisegna* le pagine
(leggere con `hayro`, riscrivere con `krilla`) perde tutto questo: un PDF/X diventa un PDF
qualunque, le tinte piatte diventano quadricromia e il peso cresce. Per la modifica serve lavorare
sulla **struttura** del file, non sul suo contenuto.

## Alternative
1. **Ridisegnare con hayro + krilla**, come fa l'esportazione (ADR 0008). Codice già in casa, ma
   perde intenti, livelli, segnalibri e annotazioni, e rasterizza o riscrive ciò che non capisce.
   Va bene per una miniatura, non per il file che va in stampa.
2. **`lopdf` o uno scrittore COS fatto in casa.** `lopdf` è MIT ma non gestisce i flussi di oggetti
   compressi in scrittura né la riparazione dei file rotti; scriverne uno nostro è mesi di lavoro
   su un formato ostile, proprio dove un errore rovina il lavoro di chi stampa.
3. **Scelta: `pdfcraft`** (`pdfcraft-cos`, `pdfcraft-organize`) del team ArtCraft, a commit fissato
   (`afa8e18`, ADR 0001). Legge e riscrive gli oggetti COS senza toccare i flussi di contenuto:
   ciò che non cambia esce byte per byte com'era. È MIT/Apache, ha `unsafe_code = "forbid"` come
   noi, e convive con `hayro` 0.8 e `krilla` 0.8 senza doppioni di versione (a differenza di
   `craft-pdf`, vedi ADR 0013).

## Decisioni collegate
- Il motore sta in `crates/pdf_edit`, sotto l'interfaccia (nessun egui): apre, modifica, salva.
  L'app (`apps/winfiles/src/pdf/`) è solo finestra sopra di esso.
- **Ruota, elimina, sposta, duplica, estrai, dividi, unisci, inserisci** cambiano solo l'albero
  delle pagine: il contenuto non si tocca mai.
- **Margini, Misura, A4, A3** (`frame.rs`) non ridisegnano la pagina: la impacchettano in un
  XObject form (BBox = CropBox, Matrix che raddrizza la rotazione) e scrivono un contenuto nuovo
  di poche righe che la posiziona nel foglio nuovo, con le bande di colore attorno. Così resta
  tutto: testo selezionabile, vettori, tinte piatte, trasparenze. TrimBox, annotazioni e loro
  riquadri si spostano con la stessa matrice.
- **Annulla e Ripeti** tengono fino a 100 copie del documento con la loro etichetta ("Ruota 2
  pagine"). Un documento è piccolo in memoria perché i flussi di contenuto restano condivisi.
- Le miniature dell'elenco e l'anteprima grande si disegnano con `hayro`, come nel modulo
  Miniature: `pdf_edit` non disegna nulla. La chiave della cache è l'impronta della pagina
  (`page_keys`), così trascinare le pagine non ricalcola niente.
- **Il salvataggio è atomico** (`write_atomic`): si scrive un file temporaneo accanto e poi si
  rinomina. Una stampante che si pianta o la corrente che va via non lascia mai un PDF a metà
  al posto dell'originale.
- Si rifiuta di eliminare l'ultima pagina, e ogni numero letto dal file ha il suo tetto:
  un PDF rovinato dà un errore, mai un crash.

## Conseguenze
- WinFiles fa ciò che fa "Controllo PDF" della 4.9 e in più: duplica, inserisce pagine da un altro
  PDF nel punto scelto, mette margini colorati, adatta a un formato qualunque, annulla e ripete.
- Una dipendenza git in più (`pdfcraft`), aggiunta a `deny.toml` tra le sorgenti permesse.
- `pdfcraft` non firma né cifra: un PDF protetto da password non si apre, e lo si dice.
- Se un giorno servisse modificare il *contenuto* (testo, immagini dentro la pagina) sarà un'altra
  decisione: qui si è scelto di non entrarci.
