# 0024. Abbondanza creata in vettoriale, ribaltando la pagina

Stato: Accettata (2026-10-29)

## Contesto
Molti PDF dei clienti arrivano senza abbondanza: la grafica si ferma al formato finito (TrimBox,
fustella o filetto). Per la stampa in medio e grande formato con Onyx servono 3–5 mm oltre il
taglio. Il controllo della pagina (ADR 0023) misura già l'abbondanza per lato.

## Alternative
1. **Striscia raster**: disegnare il bordo della pagina in un'immagine e incollarla attorno.
   Funziona con qualunque contenuto ma perde la qualità vettoriale, pesa e dipende dalla
   risoluzione scelta.
2. **Ribaltare la pagina stessa (scelta)**: la pagina diventa un modulo (form XObject, come per i
   margini) disegnato più volte: una volta al suo posto e, ritagliato su ogni fascia
   dell'abbondanza e negli angoli, ribaltato attorno al bordo del formato finito ("A specchio") o
   con l'ultimo mezzo millimetro stirato fino al bordo ("Allunga il bordo"). Tutto resta
   vettoriale e il file cresce di poche righe.
3. **Spostare la grafica oltre il bordo** (scalare il contenuto): cambia le misure del lavoro, non
   accettabile.

## Decisione
Alternativa 2, in `crates/pdf_edit/src/frame.rs` (`bleed`). La fustella non deve finire ribaltata
nell'abbondanza: la copia ribaltata usa un secondo modulo con il contenuto della pagina dove i
tracciati di taglio e cordonatura non si dipingono (`paths::without`, l'operatore che dipinge
diventa `n`). La pagina vera si ritaglia 1 mm oltre il formato finito sui lati nuovi: lo spessore
della fustella resta intero e i segni di taglio più in là spariscono. Si scrivono TrimBox (formato
finito) e BleedBox (la pagina nuova).

## Conseguenze
- Solo abbondanza rettangolare, attorno al rettangolo del formato finito: con una fustella
  sagomata (angoli tondi, forme) dove la grafica si ferma sulla fustella l'abbondanza lungo la
  forma è la fase successiva.
- I tracciati della fustella dentro un modulo del PDF non si possono togliere dalla copia
  ribaltata: il messaggio lo dice.
- Una pagina sola alla volta (quella aperta), perché il formato finito si legge pagina per pagina.
