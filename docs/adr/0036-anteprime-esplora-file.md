# 0036. Le anteprime dei file in Esplora file

**Stato:** accettata, 2026-11-09

## Contesto
Michel ha chiesto come vedere nelle cartelle di Windows le icone come anteprima del contenuto.
Windows lo fa da solo per JPEG, PNG, BMP, GIF e TIFF. Senza programmi in più non lo fa per PDF,
AI, EPS, PSD, DXF, DWG, PLT e SVG. HEIC, WebP e RAW li fa solo con le estensioni dello Store.
Michel ha detto di prepararle in WinFiles.

## Alternative
1. **Lasciare tutto a Windows e ai programmi di Adobe.** Acrobat dà le anteprime dei PDF, ma non
   ci sono quelle di PSD, DXF, DWG e PLT, e su PC senza Acrobat non c'è niente.
2. **Un gestore separato per ogni tipo** (estensioni di terze parti). Ognuno ha la sua
   installazione e la sua licenza, e i file da taglio (PLT, DXF) nessuno li copre bene.
3. **Scelta: una DLL di WinFiles che fa da gestore delle anteprime** (`IThumbnailProvider`). Usa
   gli stessi lettori delle miniature e di Visualizza.

## Decisione
- Crate `crates/preview` → `winfiles_preview.dll`. Si divide in due parti:
  - `thumbnail(path, lato)`: disegna l'anteprima, uguale su ogni sistema e provata con i test;
  - `com.rs`: la parte COM, l'unico posto con `unsafe`, come la DLL del menu (ADR 0022).
- Esplora file passa il file come flusso (`IInitializeWithStream`):
  - si legge fino a 1 GB;
  - si scrive in un file temporaneo con la stessa estensione, si disegna e si cancella.
  - Un panic diventa un errore, e Windows mostra allora l'icona normale.
- La bitmap è BGRA premoltiplicata, dall'alto in basso (`WTSAT_ARGB`).
- Windows carica la DLL in un processo a parte (dllhost, isolamento predefinito). Un file
  rovinato che la facesse cadere non ferma Esplora file.
- I tipi: pdf, ai, eps, psd, psb, heic, heif, webp, tga, i RAW (dng, cr2, cr3, nef, nrw, arw,
  orf, rw2, raf, pef), svg, svgz, dxf, dwg, plt, hpgl.
  - I DXF, DWG e PLT si disegnano a linee grosse su fondo bianco, perché si leggano anche in
    piccolo.
- L'MSI registra la classe per l'utente (HKCU):
  - `CLSID\{8B3C7E21-…}\InprocServer32`, con ThreadingModel Apartment;
  - `.ext\ShellEx\{e357fccd-…}` per ogni tipo.
  - Il programma predefinito dei file non cambia.
  - Nella copia portatile non ci sono anteprime.
- Un test su Windows nella CI fa il percorso di Esplora file: chiede la fabbrica alla DLL, crea la
  classe, le passa il flusso di un file SVG e controlla la bitmap.

## Conseguenze
- Le anteprime dei PDF prendono il posto di quelle di Acrobat: le chiavi per l'utente valgono
  prima di quelle della macchina.
- Le cartelle già aperte tengono le icone vecchie nella cache di Windows finché non la si svuota
  (Pulizia disco › Anteprime) o i file cambiano.
- Togliendo WinFiles si tolgono anche le chiavi: tornano le icone di prima.

## Aggiunta (2026-11-10): davanti ai gestori degli altri programmi
Dopo la 0.2.32 Michel vedeva le anteprime dei PDF ma non quelle di AI, DXF e PSD. Windows cerca il
gestore prima nel ProgID del file (il programma che lo apre) e solo dopo nell'estensione.
Illustrator, Photoshop e i programmi CAD hanno il loro gestore nel ProgID, e spesso non dà niente.
- La DLL esporta `DllRegisterServer`. WinFiles la chiama a ogni avvio con `regsvr32 /s`, in un
  thread a parte. Per ogni tipo conosciuto scrive il CLSID di WinFiles anche in
  `HKCU\Software\Classes\<ProgID>\ShellEx\{e357…}`, sia per il ProgID scelto dall'utente
  (UserChoice) sia per quello dell'estensione, e ricorda in `HKCU\Software\WinFiles\AnteprimeProgId`
  quali ha toccato. Poi avvisa Esplora file (`SHChangeNotify`).
- `DllUnregisterServer`, chiamata alla disinstallazione da `--togli-menu`, li toglie.
- Il programma che apre i file non cambia.
- Il disegno gira su un thread con 64 MB di pila: quella di dllhost è piccola.
- La CI su Windows registra le chiavi dell'installer e un finto "Photoshop" con un suo gestore
  nel ProgID del PSD. Poi chiama `DllRegisterServer` e chiede alla shell le anteprime di SVG, PLT,
  PDF, AI, DXF e PSD (`IShellItemImageFactory`, solo anteprime).

## Aggiunta del 2026-11-10 (seconda): la classe nella vista a 64 bit

Sul PC di Michel la diagnosi (`rundll32 winfiles_preview.dll,Diagnosi`) ha mostrato le chiavi
`.ext\ShellEx` al loro posto ma nessuna classe: `0x80040154`, la DLL non partiva mai.
- Causa: l'MSI di wixl era a 32 bit, e Windows Installer scrive `HKCU\Software\Classes\CLSID`
  dei pacchetti a 32 bit in `WOW6432Node`, che Esplora file (64 bit) non legge. Le chiavi delle
  estensioni non sono reindirizzate, per questo c'erano.
- Scelta: l'MSI si crea a 64 bit (`wixl -a x64`), così la classe la scrive l'installer nel posto
  giusto. In più `DllRegisterServer` scrive da sé la classe con il proprio percorso (riparazione
  a ogni avvio di WinFiles, il cui esito va in `errori.txt`) e `DllUnregisterServer` la toglie.
- Alternativa scartata: solo la registrazione all'avvio. Sul PC di Michel non è partita, e le
  anteprime devono funzionare anche prima del primo avvio.
- La CI installa l'MSI vero su Windows sopra il rilascio 0.2.33 (a 32 bit), controlla la classe
  nella vista a 64 bit e disinstalla (`installer/prova-msi.ps1`).
- Gli AI esportati da CorelDRAW (`%%Creator:Adobe Illustrator…` senza spazio, commenti `%AI` oltre
  i primi 1024 byte) ora sono riconosciuti come PostScript di Illustrator.
