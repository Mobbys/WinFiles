# 0038. Note e annotazioni salvate da sé nel file della cartella

**Stato:** accettata, 2026-11-16

## Contesto
Fino alla 0.2.40 le annotazioni di Miniature (e le note per pagina e di cartella) si salvavano
solo nel `.winfiles`: senza salvare il lavoro si perdevano, e un collega che apriva gli stessi file
in rete non le vedeva. Le annotazioni di Visualizza si salvavano da sé, ma in
`annotazioni-visualizza.json` nella cartella delle impostazioni di quel PC. Michel vuole un
salvataggio automatico unico, condiviso fra colleghi sui file in rete; il Salva `.winfiles` resta.

## Alternative
1. **Un file accanto a ogni file** (`manifesto.pdf.note`): segue il file copiato da solo, ma
   riempie le cartelle di file in più, che finiscono anche nelle cartelle di stampa di Onyx.
2. **Una cartella comune sul server**, scelta in Impostazioni: cartelle pulite, ma va configurata
   su ogni PC e le note si perdono se il file cambia percorso o se due PC vedono il server con
   percorsi diversi (`Z:` e `\\server\…`).
3. **Scelta (Michel, 2026-11-16): un file per cartella, visibile**, `note.winfiles`. Ha
   l'estensione dei lavori, già associata a WinFiles: WinFiles lo riconosce dal nome e il doppio
   clic apre in Miniature solo i file della cartella con note o annotazioni (tutta la cartella se
   c'è solo la nota di cartella). Un lavoro non si può salvare con quel nome. Michel ha preferito
   visibile a nascosto e un nome semplice (il primo MSI di prova usava
   `_Note WinFiles.winfiles-note`, che si rinomina alla prima lettura).

## Decisione
- Formato (crate model, `folder_notes`): JSON compatto
  `{ versione, nota_cartella, file: { "<nome>": { pagine: { "<n>": { campo: valore } } } } }`.
  Campi delle pagine: `nota`, `annotazioni` (Miniature), `annotazioni_visualizza`, `scala` (il
  numero N di 1:N, ADR 0039, posseduta da Miniature) e quelli futuri. I campi sconosciuti si rimettono come sono.
- I capi delle frecce attaccati a un file della stessa cartella si scrivono col solo nome: valgono
  anche da un PC che vede la cartella con un altro percorso.
- Unione fra colleghi: prima di scrivere si rilegge il file e si unisce cella per cella
  (file, pagina, campo): vince la propria cella solo se è cambiata dall'ultima lettura. Si scrive
  in un file a parte e poi lo si mette al posto (rename). Vince l'ultimo solo se due persone
  cambiano la stessa cella insieme.
- Ogni scheda possiede le sue celle: Miniature note, note di cartella e `annotazioni`; Visualizza
  `annotazioni_visualizza`. Le due serie restano separate perché quelle di Miniature sono pensate
  per il foglio stampato (frecce fra miniature, testo di 4 mm a ogni grandezza).
- Alla prima lettura di una cartella vince il file della cartella per i file che vi hanno già
  qualcosa; per gli altri valgono le note che WinFiles ha già (un `.winfiles` aperto, le vecchie
  annotazioni di Visualizza), che così finiscono nel file.
- `apps/winfiles/src/folder_sync.rs`: letture e scritture in thread a parte (al più 8 insieme), si
  scrive 0,8 s dopo l'ultima modifica, si rilegge ogni 3 s (in Visualizza solo la cartella del file
  che si guarda), si scrive subito prima di svuotare l'elenco, aprire un altro lavoro, togliere
  file o chiudere.
- Cartella che non si scrive o non si legge: le note vanno in una copia su questo PC
  (`%APPDATA%\WinFiles\note-cartelle`), si riprova ogni 30 s e, appena si può, la copia passa nella
  cartella e sparisce. Una cartella che non si legge vale "com'era", mai "nessuna nota".
- Un file delle note rovinato non si sovrascrive mai: si segnala e si lascia com'è.
- Il file delle note sparisce da solo quando in una cartella non resta nessuna nota.
- `annotazioni-visualizza.json` passa nelle cartelle al primo avvio e poi si rinomina
  `annotazioni-visualizza.json.passato-nelle-cartelle`.
- Le note che arrivano dalla cartella non fanno risultare il lavoro modificato.

## Conseguenze
- Le note seguono la cartella se la si sposta o copia intera; copiando un file da solo no.
- Due colleghi sulla stessa pagina nello stesso momento: vince l'ultimo che scrive.
- Chi non ha permesso di scrivere nella cartella tiene le note solo sul suo PC finché non può.
