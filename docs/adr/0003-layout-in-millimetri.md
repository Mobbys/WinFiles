# 0003. Un solo layout in millimetri per anteprima, PDF e stampa

**Stato:** accettata, 2026-10-14

## Contesto
In WinFile 4.9 l'impaginazione è calcolata dal browser in pixel e centimetri CSS, e la "stampa
fedele" è un'immagine prodotta da html2canvas: scala non garantita, testo non nitido.

## Decisione
`winfiles-layout` è una funzione pura: impostazioni + elementi → fogli con rettangoli in
millimetri. Anteprima, PDF (con `krilla`) e stampa leggono quel risultato senza ricalcolare nulla.
Le sue regole (nulla fuori dall'area stampabile, nessuna sovrapposizione, ogni elemento una sola
volta, proporzioni conservate) sono verificate con test di proprietà su migliaia di casi casuali.

## Conseguenze
Ogni nuova vista (elenco, nesting) è una nuova funzione del crate `layout` con gli stessi test di
proprietà. L'obiettivo di accettazione è ±0,5 mm misurati con un righello sul foglio stampato.
