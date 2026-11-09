# 0032. WinFiles come programma predefinito, solo se lo si sceglie

**Stato:** accettata, 2026-11-06

## Contesto
Il doppio clic su un PDF o un'immagine in Esplora file dovrebbe poter aprire WinFiles nella scheda
Visualizza. Michel ha deciso che all'installazione nessun tipo cambia programma e che "Apri in
WinFiles" nel tasto destro apre sempre in Miniature. Ha chiesto anche "Invia al nesting WinFiles"
nel tasto destro.

## Alternative
1. **L'installer si fa predefinito.** Windows 10/11 protegge la scelta (`UserChoice` ha un hash):
   scriverla a mano non funziona, e toglierebbe ad Acrobat o Illustrator i loro file.
2. **Nessuna registrazione.** WinFiles non comparirebbe nemmeno in "Apri con".
3. **Scelta: WinFiles registrato come candidato, la scelta la fa chi lo usa.**

## Decisione
- L'MSI scrive per l'utente:
  - un ProgID `WinFiles.<estensione>` per PDF, AI, EPS, immagini, PSD, DXF e DWG, con il comando
    `--visualizza "%1"`;
  - `OpenWithProgids` per ogni tipo;
  - `Software\WinFiles\Capabilities` e `RegisteredApplications`.
- Il valore predefinito delle estensioni non si tocca. Un test (`defaults.rs`) lo controlla
  leggendo `winfiles.wxs`.
- Aiuto › **Programma predefinito…** mostra per ogni tipo chi lo apre ora (`UserChoice` letto con
  `reg query`). Le caselle aprono la pagina App predefinite di WinFiles nelle Impostazioni
  (`ms-settings:defaultapps?registeredAppUser=WinFiles`), dove si conferma con un clic per tipo.
  La versione portatile non registra niente e lo dice.
- "Apri con…" nella scheda apre la scelta di Windows (`OpenAs_RunDLL`), dove c'è anche il
  programma di prima.
- `instance.rs` passa alla finestra già aperta anche la scheda di destinazione (protocollo
  `WINFILES 2`): `--aggiungi` va in Miniature, `--invia-nesting` nel Nesting, `--visualizza` in
  Visualizza.
- "Invia al nesting WinFiles" funziona in due modi:
  - menu classico: `SystemFileAssociations\.<ext>\shell\WinFilesNesting` su PDF, AI, EPS e
    immagini;
  - menu corto di Windows 11: seconda classe COM nella stessa DLL (`menu11::NEST_CLSID`), con un
    verbo per ogni tipo nel pacchetto firmato (ADR 0022).
