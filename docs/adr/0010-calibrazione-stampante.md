# 0010. Calibrazione della stampante con una pagina da misurare

**Stato:** accettata, 2026-10-15

## Contesto
Al 100% un foglio dovrebbe uscire con le misure esatte, ma molti driver allungano o accorciano di
qualche millesimo, e alcuni spostano il foglio di qualche decimo di millimetro. Chi stampa le
miniature per controllare i formati deve poter fidarsi del righello. La 4.9 non ha nulla del genere.
I driver non dicono quanto sbagliano: si può solo stampare e misurare.

## Alternative
1. Leggere l'area stampabile e gli spostamenti dal driver (GDI `PHYSICALOFFSET`): dice dove la
   stampante non arriva, non quanto allunga; e `craft-print` non lo espone.
2. Una scala unica in percentuale scritta a mano: semplice, ma senza una pagina di riferimento non
   si sa che numero scrivere, e larghezza e altezza spesso sbagliano in modo diverso.
3. **Scelta:** una pagina di calibrazione con due linee lunghe a misura tonda (160 × 240 mm su A4,
   240 × 370 mm su A3), una cornice a 10 mm dai bordi, un quadrato di controllo e le tacche dei
   margini al centro di ogni lato. Chi stampa misura le due linee e la distanza dal bordo sinistro
   e da quello in alto alla cornice; WinFiles ricava allungamento e spostamento per asse.

## Decisioni collegate
- Il modello è affine per asse: la stampante mette in `s·p + o` ciò che riceve in `p`; la
  correzione manda `p/s − o/s`. Il calcolo (`winfiles_geom::Calibration::from_measures`) tiene
  conto della correzione con cui la pagina è stata stampata, così si può ristampare con la
  correzione per controllarla e affinarla.
- Si accettano solo allungamenti fino al 5% e spostamenti fino a 15 mm: oltre, la misura è
  sbagliata o il driver sta riducendo il foglio ("Adatta alla pagina"), e la finestra lo dice.
- La correzione è per stampante, per nome, in `impostazioni.json`. Si applica solo alla stampa al
  100%: riducendo all'area stampabile le misure non sono esatte comunque.
- La correzione è un campo della scena del foglio (`Page::calibration`) che solo chi scrive il PDF
  applica, come trasformazione di tutto il foglio: testi, linee e miniature si spostano insieme e
  il formato della pagina non cambia. Anteprima e PDF esportato restano senza correzione.
- Dove va ogni segno della pagina lo decide `winfiles_layout::calibration`, come per gli altri fogli.

## Conseguenze
- Su una stampante calibrata le misure stampate tornano esatte entro la precisione del righello
  (mezzo millimetro su 240 mm è il due per mille).
- `--calibrazione A4 uscita.pdf` scrive la pagina in un PDF senza aprire la finestra, per le prove.
- Se si rinomina una stampante in Windows la sua correzione resta sotto il nome vecchio e va rifatta.
