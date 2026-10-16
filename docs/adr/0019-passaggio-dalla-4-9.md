# 0019. Passaggio dalla WinFile 4.9 a WinFiles con l'aggiornamento della 4.9

**Stato:** accettata, 2026-10-16

## Contesto
Michel vuole che chi ha installata la vecchia WinFile 4.9 veda un aggiornamento e, accettandolo,
si trovi WinFiles. La 4.9 (`winfile.py`, repository `Mobbys/WinFile`) all'avvio legge
`releases/latest` di `Mobbys/WinFile`; se il tag (senza "v") è più alto di `4.9` scarica in
silenzio il primo `.zip` del rilascio, altrimenti il primo `.exe` o `.msi`, poi chiede "Vuoi
chiudere l'applicazione e installarlo ora?" mostrando le note del rilascio. Un `.zip` viene
scompattato sopra la cartella della 4.9; un `.exe` o `.msi` viene aperto con `os.startfile`.
La 4.9 non si può più cambiare: chi la usa ha questo codice.

## Alternative
1. **Uno zip che sostituisce i file della 4.9** con WinFiles: la 4.9 si riavvia lanciando il suo
   `WinFile.exe`, che non ci sarebbe più o sarebbe un'altra cosa; la cartella resterebbe quella
   della 4.9 (`%AppData%\WinFile`) e l'aggiornatore di WinFiles non la riconoscerebbe come
   installata. Fragile.
2. **Rilasci di WinFiles pubblicati anche su `Mobbys/WinFile` a ogni versione**: serve un token
   con accesso a due repository, il piano già scartato in Fase 7.
3. **Scelta: un rilascio `5.0` su `Mobbys/WinFile`, fatto una volta a mano, con il solo MSI di
   WinFiles.** La 4.9 lo trova, lo scarica e lo apre: WinFiles si installa al suo posto normale,
   con il menu Start e l'icona sul Desktop. Da lì WinFiles si aggiorna da sé da `Mobbys/WinFiles`.

## Decisioni collegate
- Il rilascio `5.0` non ha `.zip` (la 4.9 lo preferirebbe all'MSI) e le sue note dicono in poche
  righe che cos'è WinFiles.
- La 4.9 resta installata e a ogni avvio riproporrebbe il `5.0`. Per questo WinFiles, se trova la
  sua voce in "App installate" (`HKCU\...\Uninstall\WinFile`, scritta dall'installer NSIS),
  mostra una barra "Su questo PC c'è ancora WinFile 4.9" con "Togli WinFile 4.9", che avvia il
  suo disinstallatore, e "Tienila", che non lo chiede più (la Simulazione quote c'è solo nella
  4.9). Il disinstallatore si avvia come programma, senza shell.
- L'MSI mette l'icona di WinFiles sul Desktop, dove chi usava la 4.9 la cercava.

## Conseguenze
- Chi usa la 4.9 portatile (senza installer) riceve lo stesso MSI: si trova WinFiles installata
  e la sua cartella della 4.9 resta dov'è.
- Le versioni dopo la `5.0` su `Mobbys/WinFile` non servono: chi ha WinFiles non guarda più lì.
