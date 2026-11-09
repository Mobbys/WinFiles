# 0035. Word, Excel, PowerPoint e InDesign in Visualizza

**Stato:** accettata, 2026-11-09

## Contesto
Michel ha chiesto di vedere in Visualizza anche i documenti di Excel, Word, InDesign e PowerPoint,
appoggiandosi a storytold/gridcraft, wordcraft, designcraft e deckcraft.

## Alternative
1. **Le loro librerie dentro WinFiles.** Portano egui 0.37 e krilla e hayro modificati via
   `[patch]`, in conflitto con le nostre versioni (lo stesso problema di cadcraft e VectorCraft,
   ADR 0033). Inoltre DesignCraft usa una revisione di craft-libs diversa dagli altri tre: non
   compilano neanche fra loro.
2. **Solo l'anteprima salvata nel file** (thumbnail.jpeg nei pacchetti Office, anteprima XMP
   negli INDD). Piccola e non sempre presente, e mostra solo la prima pagina.
3. **Scelta: i loro programmi a riga di comando come convertitori in PDF.**

## Decisione
- La CI (`.github/workflows/convertitori.yml`) compila per Windows `wordcraft-cli`,
  `gridcraft-cli`, `deckcraft-cli` e `designcraft-cli` dal commit fissato in
  `installer/convertitori.txt`. La cache li tiene: si ricompilano solo quando cambia il commit.
- L'MSI li mette in `convertitori\` accanto a WinFiles.exe. La copia portatile non li ha: in quel
  caso Visualizza dice che serve WinFiles installato.
- WinFiles li avvia in un processo a parte, senza finestra, con al massimo 3 minuti e 512 MB di
  file. Il PDF scritto in una cartella temporanea si legge e si cancella, poi la scheda lo mostra
  come gli altri PDF: misure della pagina, zoom, Quote, Stampa con le pagine scelte.
- I tipi:
  - Word: docx, docm, dotx, doc, odt, rtf (WordCraft);
  - Excel: xlsx, xlsm, xltx, xls, xlsb, ods (GridCraft);
  - PowerPoint: pptx, pptm, potx (DeckCraft);
  - InDesign: indd, indt, idml (DesignCraft, gli INDD attraverso indd-utils).
- "Apri con" di Esplora file propone WinFiles per docx, doc, xlsx, xls, pptx, indd e idml. Non sono
  fra i tipi di "Programma predefinito…": per questi file resta il loro programma.

## Conseguenze
- L'installazione cresce di circa 150 MB (i quattro convertitori); l'MSI compresso un po' meno.
- L'impaginazione e i caratteri possono differire un poco dall'originale: il pannello lo dice.
- Aggiornare un convertitore è una PR a sé: si cambia il commit in `convertitori.txt` e si provano
  i file.
