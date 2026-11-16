# 0040. Le stesse annotazioni in Miniature e in Visualizza

**Stato:** accettata, 2026-11-16

## Contesto
Fino alla 0.2.42 Miniature e Visualizza avevano due serie di annotazioni per pagina
(`annotazioni` e `annotazioni_visualizza` in `note.winfiles`, ADR 0038). Michel ha messo note in
Visualizza su un file e in Miniature su un altro; aprendo `note.winfiles` vedeva in Miniature solo
quelle fatte lì. I punti erano già in frazioni della pagina in tutte e due; cambiava la misura di
spessori e corpi: in Miniature mm della carta alla grandezza della miniatura, in Visualizza mm di
una pagina con il lato lungo di 297 mm (un A4).

## Alternative
1. **Lasciare due serie.**
2. **Due serie, ognuna visibile anche nell'altra scheda** ma modificabile solo dove è nata: niente
   da convertire, ma due sistemi da ricordare («dove l'avevo fatta?»).
3. **Scelta (Michel, 2026-11-16): una serie sola per pagina**, che si sposta, cambia e cancella
   da tutte e due le schede.

## Decisione
- Spessori, corpi e larghezze dei testi sono **in proporzione alla pagina**: mm di una pagina con
  il lato lungo di `PAGE_LONG_MM` = 297 (come il Visualizza di prima). Sulla miniatura si
  disegnano moltiplicati per lato lungo della miniatura / 297 (`annotate::page_k`): una scritta
  segue la miniatura quando cambiano colonne o foglio, come un disegno sopra la pagina. I comandi
  della riga (0,5 mm, 4 mm…) restano in mm di ciò che si vede: in Miniature si dividono per
  `page_k` quando vanno sull'annotazione e si moltiplicano quando si leggono.
- Nel `.winfiles` il campo diventa `annotazioni_pagina`; in `note.winfiles` `annotazioni_pagina`,
  scritto da Miniature e da Visualizza.
- Le annotazioni vecchie passano da sole: `annotazioni_visualizza` (già in proporzione) va nel
  campo unico appena una delle due schede legge la cartella; quelle di Miniature (`annotations`
  nei `.winfiles`, `annotazioni` in `note.winfiles`) restano in `FileEntry.legacy_annotations`
  finché la miniatura è nella griglia, poi passano con la sua grandezza
  (`WinFilesApp::convert_annotations`): sulla miniatura restano uguali. Dopo il passaggio i campi
  vecchi si svuotano. Due copie che passano le stesse annotazioni non le raddoppiano
  (`annotate::add_missing`).
- Passando fra Miniature e Visualizza la scheda che si lascia scrive subito e l'altra rilegge
  (`FolderSync::refresh`).
- Le annotazioni fatte nella Stampa di Visualizza fuori dalla pagina si vedono intere anche
  attorno alla miniatura, come quelle che Miniature permette nei margini (tagliarle alla cella
  avrebbe tagliato anche queste).

## Conseguenze
- Una WinFiles 0.2.42 o precedente non vede le annotazioni scritte nel campo unico: si aggiorna da
  sola.
- Una scritta fatta su una miniatura piccola diventa grande in Visualizza, e viceversa: è la stessa
  scritta sulla stessa pagina.
