# 0021. Quote in proporzione sull'anteprima di Modifica PDF e Modifica immagini

**Stato:** accettata, 2026-10-19

## Contesto
La 4.9 aveva la scheda "Simulazione quote": si tracciava una linea, le si dava la misura vera e
le altre linee si calcolavano in proporzione; un doppio clic su una quota le riscalava tutte.
Michel la usa per ricavare misure da un logo (scrivo la base, voglio l'altezza, o viceversa) e
ha chiesto di averla sia nei PDF sia nelle immagini, senza salvare niente nel file.

## Alternative
1. **Un modulo a parte, come la 4.9.** Un'altra scheda da aprire e un altro modo di caricare i
   file, con prospettiva, ritaglio ed esportazione che WinFiles ha già altrove.
2. **Solo un rettangolo con base e altezza.** Risponde alla domanda più frequente, ma non alle
   quote multiple con una master che Michel ha chiesto.
3. **Scelta: uno strumento "Quote" nei due moduli di modifica.** Rettangoli e linee sulla stessa
   anteprima della taglierina, con una sola scala per tutte: la misura scritta a mano diventa la
   master e le altre si adeguano.

## Decisione
- `apps/winfiles/src/quotes.rs` tiene le quote nell'unità di chi le usa (millimetri della pagina
  nel PDF, pixel nelle immagini) con `mm_per_unit` (1 nel PDF, 25,4/DPI nelle immagini). Senza
  master si vedono le misure vere del file: meglio della 4.9, che partiva da una linea senza
  misura.
- Scala = valore della master / sua misura nel file. Spostando la master cambia la scala, non il
  suo numero. Tolta la master, le altre restano alla sua scala ("Torna alle misure vere" azzera).
- Le quote valgono per la pagina o l'immagine (e la sua versione) su cui sono tracciate:
  cambiando pagina, o modificando il file, si ricomincia.
- Con le quote accese, trascinare sull'anteprima traccia quote e non la taglierina; Canc toglie
  la quota scelta e mai le pagine; la lente è quella della taglierina.
- Prospettiva e ritaglio con le quote della 4.9 restano fuori.
- Stampa e "Salva con le quote" (0.2.7): il documento com'è adesso va in un file temporaneo
  (il PDF da `Editor::to_bytes`, l'immagine in PNG con i pixel che bastano a 300 DPI) e ogni
  pagina diventa una scena (`quotes::page`): il file su tutto il foglio, alla misura vera, con
  `Picture::dpi` = 300 invece dei 150 delle miniature, e le quote disegnate sopra da
  `Quotes::sketch` in millimetri della pagina. Stampa e PDF passano dallo stesso lavoro e dalla
  stessa finestra Stampa delle miniature (ADR 0008, 0009, calibrazione compresa). Il PDF salvato
  è un'immagine della pagina, non un vettoriale: serve da promemoria, il file vero non cambia.

## Conseguenze
- Niente da salvare né da annullare: le quote non toccano il documento.
- "Copia le misure" le porta in testo dove servono (preventivo, mail).
