# 0042. Il file delle note riapre il lavoro in cui sono state scritte

**Stato:** accettata, 2026-11-27

## Contesto
Il doppio clic su `note.winfiles` (ADR 0038) apriva in Miniature solo i file della cartella con
note. Michel (2026-11-27): tre file impaginati in orizzontale da "Su N fogli", annotazioni su due;
aprendo il file delle note arrivavano solo quei due, "Su N fogli" ricalcolava, girava il foglio e
cambiava la grandezza delle miniature, e le pagine non erano più quelle annotate. Una cartella può
anche avere più impaginati diversi (10 file: un lavoro con 5, un altro con gli altri 5), con un
solo file delle note.

## Alternative
1. **Solo i file con note, con l'impaginazione di allora** (foglio, verso, colonne fissi):
   miniature uguali, ma i file si accostano (il 3° prende il posto del 2° se il 2° non ha note) e
   le annotazioni fuori dalle miniature finiscono altrove.
2. **Scelta (Michel, 2026-11-27): tutti i file del lavoro**, nel loro ordine, con l'impaginazione
   di allora: i fogli tornano identici.

## Decisione
- `note.winfiles` ha un campo `lavori`: un elenco (dal più recente, al più 30) di
  `{ file, data, foglio, verso, colonne, una_per_foglio, tutta_pagina }`. `file` sono i file con
  almeno una pagina in stampa, relativi alla cartella delle note e scritti con "/"
  (`a.pdf`, `sotto/b.pdf`, `../altra/c.pdf`), così valgono da ogni PC; percorsi assoluti o con
  l'unità si scartano. `verso` e `colonne` sono quelli davvero usati, anche se scelti da
  "Su N fogli". Il campo è letto come valore grezzo: un lavoro rovinato si salta e si riscrive
  com'è, le note non si perdono.
- Il lavoro si scrive solo insieme alle proprie note cambiate, mai a una semplice rilettura: chi
  apre la cartella senza scrivere non copre il lavoro di un collega. Un lavoro con gli stessi file,
  o con una parte, o che li contiene tutti, prende il posto del vecchio (lo stesso lavoro riaperto
  e cambiato); uno con altri file si aggiunge.
- Il doppio clic: un lavoro solo si riapre subito; più lavori (o file con note fuori dai lavori,
  per esempio scritte prima della 0.2.46 o da Visualizza) danno la finestra "Quale lavoro apro?",
  con i primi nomi, i file, quanti hanno note, quando e il foglio. Senza lavori, come prima.
- Riaprendo in un elenco vuoto i file tornano nell'ordine di allora e l'impaginazione si rimette
  come era, con "Su N fogli" spento (con altri file ricalcolerebbe). In un elenco già pieno i file
  si aggiungono e l'impaginazione non cambia.
- Le frecce che arrivano a un file senza note portano anche quel file.

## Conseguenze
- Le versioni di prima tengono il campo `lavori` fra quelli sconosciuti e lo riscrivono.
- Se si riapre un lavoro e lo si cambia senza toccare le note, il file delle note ricorda ancora
  l'impaginazione di quando sono state scritte.
