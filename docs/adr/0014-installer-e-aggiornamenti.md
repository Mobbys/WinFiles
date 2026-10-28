# 0014. Installer MSI e aggiornamenti dai rilasci di GitHub

**Stato:** accettata, 2026-10-15

## Contesto
Per la Fase 7 WinFiles deve installarsi come un programma vero (menu Start, file `.winfiles` che
si aprono con un doppio clic, disinstallazione) e aggiornarsi da solo, come la 4.9 che all'avvio
chiede a GitHub l'ultimo rilascio (`winfile.py`, righe 90-180), scarica il pacchetto e lo
installa. Il repository era privato: una copia installata non può leggerne i rilasci senza una
password, e una password dentro il programma sarebbe leggibile da chiunque lo abbia.

## Alternative per i rilasci
1. Repository privato con un token nel programma: scartata, il token si estrae dall'eseguibile.
2. Repository pubblico a parte (`WinFiles-rilasci`) con i soli installer, riempito dalla CI con
   un token salvato nei segreti di GitHub: funziona, ma sono due repository e un token da rinnovare.
3. **Scelta (Michel, 15 ottobre 2026):** il repository WinFiles diventa pubblico con licenza
   proprietaria (`LICENSE`: codice leggibile, tutti i diritti riservati, i programmi dei rilasci si
   possono usare). I rilasci stanno in WinFiles stesso, la CI li pubblica con il proprio
   `GITHUB_TOKEN`, le copie installate li leggono senza password. In più GitHub Actions non ha
   limiti di minuti sui repository pubblici.

## Alternative per l'installer
1. WiX Toolset su Windows: lo standard, ma dalla versione 6 chiede un contributo di manutenzione
   alle aziende.
2. Inno Setup o NSIS: installer `.exe`, meno adatti alla distribuzione aziendale e agli
   aggiornamenti silenziosi.
3. **Scelta:** un MSI descritto in `installer/winfiles.wxs` (schema WiX 3) e compilato da `wixl`
   (msitools, LGPL, usato solo per costruire: non finisce in WinFiles) su Linux in CI, con
   `installer/crea-msi.sh`.

## Decisioni collegate
- Installazione per l'utente in `%LocalAppData%\Programs\WinFiles`, senza permessi di
  amministratore (come VS Code e Teams): l'aggiornamento non chiede mai la password dell'IT.
- L'`UpgradeCode` non cambia mai: ogni versione nuova sostituisce la vecchia (MajorUpgrade); una
  versione più vecchia non scavalca una più nuova.
- L'aggiornamento è in `apps/winfiles/src/update.rs`: all'avvio un thread chiede
  `api.github.com/repos/Mobbys/WinFiles/releases/latest` con `curl.exe` (presente in Windows 10 e
  11), così WinFiles non porta dentro librerie di rete e TLS. Bozze e versioni di prova non si
  annunciano. Se la versione è più nuova compare una barra con Novità, Aggiorna e ✕.
- Copia installata: "Aggiorna" scarica l'MSI nella cartella temporanea; alla chiusura di WinFiles
  (con la solita domanda sulle modifiche non salvate) parte `msiexec /passive` e poi la versione
  nuova. Copia portatile: "Scarica" apre la pagina del rilascio nel browser.
- Senza rete o senza risposta da GitHub il controllo all'avvio tace; "Aiuto › Controlla
  aggiornamenti" invece dice sempre com'è andata.
- L'icona di WinFiles è disegnata da uno script (`assets/icon/genera-icona.py`), niente asset di
  Microsoft o Adobe; `build.rs` la mette nell'eseguibile con `winresource` (MIT, solo in
  compilazione e solo per Windows).
- Una versione si pubblica alzando la versione in `Cargo.toml` e scrivendo le note in
  `docs/novita/X.Y.Z.md`: al primo push che porta una versione senza rilascio,
  `.github/workflows/rilascio.yml` compila, crea MSI ed eseguibile portatile e pubblica il
  rilascio, con il tag `vX.Y.Z` creato da GitHub sul commit. Niente tag da spingere né passaggi a
  mano (Michel, 15 ottobre 2026); la versione si alza solo quando Michel chiede di pubblicare.
  Un rilascio creato a mano su GitHub riceve lo stesso i file.

## Conseguenze
- Il codice si può leggere da chiunque: niente file di clienti, password o dati riservati nel
  repository (controllata la cronologia il 15 ottobre 2026).
- L'MSI non è firmato: al primo avvio Windows SmartScreen può chiedere conferma ("Esegui
  comunque"). Un certificato di firma del codice si valuta a parte.
- La 4.9 continua a leggere i rilasci di `Mobbys/WinFile` (ADR 0004): le due versioni convivono.
