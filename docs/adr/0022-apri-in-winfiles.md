# 0022. "Apri in WinFiles" nel tasto destro e una finestra sola

**Stato:** accettata, 2026-10-21

## Contesto
Michel vuole aprire file e cartelle in WinFiles dal tasto destro di Esplora file. In ditta tutti
i PC hanno Windows 11. Le voci classiche del registro (`HKCU\Software\Classes\*\shell\…`) su
Windows 11 stanno in "Mostra altre opzioni"; nel menu corto entrano solo i programmi con un
pacchetto firmato (identità di pacchetto) e un gestore `IExplorerCommand`. Esplora file avvia
inoltre il comando una volta per ogni file selezionato: senza accorgimenti, dieci file
diventerebbero dieci finestre.

## Alternative per la finestra unica
1. Mutex e pipe con nome di Windows: richiedono chiamate di sistema `unsafe` o una dipendenza
   nuova, e le pipe con nome sono condivise fra le sessioni dello stesso PC.
2. **Scelta:** una porta TCP su 127.0.0.1, diversa per ogni utente (FNV-1a del nome), con solo
   la libreria standard (`apps/winfiles/src/instance.rs`). Prendere la porta è atomico: la prima
   copia che ci riesce apre la finestra, le altre avviate con `--aggiungi` le mandano i percorsi e
   chiudono. Il messaggio porta il nome dell'utente; se qualcosa non va, la copia apre la sua
   finestra (nel caso peggiore una finestra per file, mai file persi).

Solo `--aggiungi` manda i file a un'altra finestra: il doppio clic su un `.winfiles` e le
esportazioni da riga di comando restano come prima.

## Alternative per il menu
1. **Voce classica nel registro per l'utente (fatta):** su file, cartelle e sfondo della
   cartella, con `MultiSelectModel=Player` per accettare più di 15 file. Su Windows 10 è nel menu
   principale, su Windows 11 in "Mostra altre opzioni".
2. **Menu corto di Windows 11 (fatto, scelta di Michel il 21 ottobre 2026):** pacchetto "con
   posizione esterna" firmato più una DLL con `IExplorerCommand`, firmato con un certificato
   Winsport nostro, gratis, da installare una volta su ogni PC. Un certificato commerciale
   eviterebbe quel passaggio ma costa ogni anno.
3. Rimettere il menu classico di Windows 10 per tutto Esplora file: tocca il sistema intero,
   tenuta come ultima scelta.

## Il menu corto di Windows 11, come funziona
- `crates/menu11` è la DLL (`winfiles_menu11.dll`) con la classe COM della voce: titolo, icona di
  WinFiles.exe e, al clic, un solo avvio di `WinFiles.exe --aggiungi` con tutti i file (più avvii
  solo oltre i 30 000 caratteri di riga di comando). È l'unico crate con `unsafe` permesso: COM
  passa puntatori grezzi e chiama funzioni esportate. Ogni blocco è commentato e i puntatori sono
  controllati prima dell'uso. Usa `windows`, `windows-core` e `windows-registry`, già nel progetto
  come dipendenze di eframe, rfd e arboard.
- `installer/menu-win11/AppxManifest.xml` dichiara la voce su file e cartelle (non sullo sfondo
  della cartella, dove resta solo la voce classica) e la DLL come server COM in un processo
  surrogato. La scrittura nel disco e nel registro non è virtualizzata: WinFiles avviato dal
  menu trova le sue impostazioni dove sono sempre state.
- La CI (`installer/crea-pacchetto-menu.ps1`) crea il pacchetto con MakeAppx e lo firma con il
  certificato dei segreti `WINSPORT_PFX` e `WINSPORT_PFX_PASSWORD`, creato una volta da
  `installer/crea-certificato-winsport.ps1`. Senza segreti l'MSI esce senza menu di Windows 11.
  Nella CI di ogni push il pacchetto si firma con un certificato di prova, si registra sul
  runner e si chiede alla DLL il titolo attraverso COM.
- L'MSI copia DLL e pacchetto nella cartella di WinFiles. WinFiles all'avvio, in un thread,
  controlla con PowerShell se il pacchetto della sua versione è registrato e, se il certificato è
  fra le "Persone attendibili" del computer, lo registra (`apps/winfiles/src/menu11.rs`). Se
  manca il certificato, una barra propone di installarlo: la conferma dell'amministratore
  arriva da Windows. "Non ora" non la ripropone; la si ritrova in Aiuto.
- Alla disinstallazione l'MSI avvia `WinFiles.exe --togli-menu`, che toglie il pacchetto.
- Aggiornando WinFiles, la DLL può essere ancora caricata dal processo surrogato: Windows la
  sostituisce al riavvio, intanto la voce funziona con quella vecchia.
