# 0037. Programmi esterni: «Apri in …» con i programmi scelti per ogni tipo di file

**Stato:** accettata, 2026-11-16

## Contesto
Michel vuole aprire il file che sta guardando nel programma giusto con un clic: un PDF in Acrobat
o in Illustrator, un AI in Illustrator, un JPG in Foto di Windows o in Photoshop. «Apri con…»
c'era già, ma apre solo il programma predefinito di Windows, e quello è spesso WinFiles stesso.
Le sue scelte sul piano (2026-11-16):
- un pulsante per ogni programma scelto, senza menu a tendina;
- niente esporta/importa delle impostazioni e niente Onyx;
- in Miniature solo il tasto destro;
- per ora nessun programma in più oltre a quelli noti, ma la porta resta aperta.

## Alternative
1. **Solo l'elenco «Apri con» di Windows** (`SHOpenWithDialog`). Non serve configurare niente, ma
   è sempre una finestra in più e non c'è un pulsante con il nome del programma.
2. **Percorsi scritti a mano nelle impostazioni.** È semplice, ma si rompe a ogni versione nuova
   di Adobe (`Adobe Photoshop 2025` → `2026`) e Michel dovrebbe cercare gli .exe da solo.
3. **Scelta: programmi noti per nome, cercati da WinFiles, più l'elenco «Apri con» di Windows.**
   Le scelte si salvano per «famiglia» (acrobat, illustrator, photoshop…), non per percorso.

## Decisione
- Nuovo crate `crates/external` (`winfiles-external`), senza egui. Contiene:
  - le tabelle dei tipi (PDF, AI, EPS, JPG… raggruppati come nella finestra);
  - i programmi noti e quelli consigliati per ogni tipo;
  - la `Config` salvata in `impostazioni.json`, con un `sanitized` che ripulisce un file rovinato;
  - la ricerca dei programmi, le icone e l'avvio.
- Su Windows (`win.rs`, l'unico posto con `unsafe` del crate, ogni blocco commentato):
  - `SHAssocEnumHandlers` dà i programmi dell'elenco «Apri con» di ogni estensione;
  - App Paths del registro e le cartelle `Program Files\Adobe` trovano gli altri, prendendo la
    versione più nuova;
  - le icone arrivano da `IShellItemImageFactory`;
  - le app dello Store (Foto, Paint) si avviano con `IApplicationActivationManager`.
- La prima volta che si trovano programmi si mettono i consigliati installati (per i PDF:
  Acrobat e Illustrator). Dopo, le scelte di Michel non si toccano più. Quando un programma cambia
  versione, si aggiorna solo il suo percorso.
- Dove si vedono i programmi:
  - Visualizza, Modifica PDF e Modifica immagini: un pulsante per programma, con l'icona vera e
    il nome; con la finestra stretta resta solo l'icona.
  - Ctrl+Maiusc+1…9 apre il programma in quel posto.
  - Tasto destro in Visualizza (pagina), Miniature, Modifica immagini e Nesting: sottomenu
    «Apri in».
  - Impostazioni: Aiuto › Programmi esterni…, oppure la matita accanto ai pulsanti.
- Acrobat si apre alla pagina che si sta guardando (`/A page=N`).
- Oltre 10 file si chiede conferma; il tetto è 200 file.
- Con modifiche non salvate (Modifica PDF, Modifica immagini) si chiede: «Salva e apri», «Apri il
  file com'è», «Annulla».
- I file aperti così si tengono d'occhio: se l'altro programma li salva, WinFiles li rilegge
  (misure, miniature, Visualizza). Il file deve essere fermo da un giro di 2 s, così non si legge
  a metà scrittura.
- Nessuna icona di Adobe o Microsoft nel pacchetto: le icone si chiedono a Windows sul PC.
  Dove non ce ne sono, si usa la nostra icona «apri fuori».

## Conseguenze
- Su Linux e nella CI il crate trova solo i programmi scritti a mano. La parte Windows la prova il
  test `windows_risponde` (Blocco note per i .txt, icona non vuota) nel job Windows. Acrobat,
  Illustrator e Photoshop veri si provano solo sul PC di Michel.
- Un programma tolto dal PC resta nelle scelte, grigio, con «Non si trova più su questo PC». Così
  la scelta non si perde se il programma torna (per esempio dopo una reinstallazione).
- Altri programmi si aggiungono con «Sfoglia…» (qualunque .exe), senza cambiare il codice. Uno noto
  in più è una riga in `types.rs`.
