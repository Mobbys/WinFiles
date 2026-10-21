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
2. Menu corto di Windows 11: pacchetto "sparse" firmato più una DLL con `IExplorerCommand`.
   Decisione di Michel in sospeso fra firma con un certificato nostro (installato una volta su
   ogni PC) e un certificato commerciale.
3. Rimettere il menu classico di Windows 10 per tutto Esplora file: tocca il sistema intero,
   tenuta come ultima scelta.
