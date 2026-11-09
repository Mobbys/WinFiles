# 0020. La copia di Illustrator nei PDF: si chiede al salvataggio se toglierla

**Stato:** accettata, 2026-10-16

## Contesto
Michel ha rinominato una tinta piatta in WinFiles: riaperto in WinFiles il PDF aveva il nome
nuovo, in Adobe ancora il vecchio. Una parte era un difetto nostro (le DeviceN scritte in più
oggetti, corretto nella 0.2.4). L'altra non dipende da noi: un PDF salvato da Illustrator con
"Mantieni le funzioni di modifica di Illustrator" porta in `/PieceInfo /Illustrator` della pagina
il file .ai intero (`/Private`, `AIPrivateData1…N`). Illustrator, riaprendo il PDF, legge quella
copia e non le pagine, quindi non vede nessuna modifica fatta altrove: tinte, tagli, margini.

## Alternative
1. **Lasciarla sempre.** Il file resta modificabile come .ai, ma in Illustrator le modifiche di
   WinFiles spariscono senza avviso: è il difetto che Michel ha visto.
2. **Toglierla sempre.** Illustrator mostra le modifiche, ma apre il file come PDF importato
   (senza livelli né testi modificabili): a volte serve tornare in Illustrator.
3. **Scelta (di Michel): chiedere al salvataggio.** Se il PDF ha la copia, "Salva" chiede una
   volta per documento: "Togli e salva" (consigliato), "Tienila e salva", "Annulla".

## Decisione
- `Editor::has_illustrator_data` cerca `/PieceInfo` con la voce `/Illustrator` in qualsiasi
  oggetto (pagine, moduli, catalogo); `strip_illustrator_data` toglie solo quella voce (le altre
  di `/PieceInfo` restano) come modifica annullabile. Il salvataggio completo scrive solo gli
  oggetti raggiungibili, quindi la copia sparisce anche dal file e il PDF si alleggerisce.
- La risposta vale per il documento aperto; un nuovo PDF la richiede.
- Se il salvataggio arriva da "Salvare le modifiche?" (chiudere, aprire un altro PDF), dopo la
  risposta si riprende quello che si stava facendo.

## Conseguenze
- Le pagine non cambiano: Acrobat, la stampa e WinFiles vedono lo stesso PDF di prima.
- Un PDF da cui si è tolta la copia, riaperto in Illustrator, perde i livelli: lo dice la domanda.
