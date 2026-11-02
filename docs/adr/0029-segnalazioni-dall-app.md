# 0029. Segnalazioni di problemi e richieste dall'app

Stato: Accettata (2026-11-02)

## Contesto
Michel vuole che chi usa WinFiles in ditta possa segnalare un problema o chiedere una nuova
funzione dal menu Aiuto, e che le segnalazioni finiscano nelle Issues di `Mobbys/WinFiles`.
Molti colleghi non hanno un account GitHub, e la segnalazione va scritta dentro WinFiles, non
su una pagina web.

## Alternative
1. **Aprire il modulo di GitHub nel browser** con i campi già riempiti: semplice, nessun
   segreto, ma serve un account GitHub. Michel l'ha scartata come strada principale.
2. **Chiave nell'eseguibile (scelta di Michel)**: una chiave fine-grained di GitHub che vale solo
   per `Mobbys/WinFiles` con il solo permesso Issues (lettura e scrittura); WinFiles crea la issue
   con l'API. Chi estrae la chiave dall'exe può al più creare o modificare issue, che si chiudono.
3. **Un piccolo servizio intermedio** (per esempio un Cloudflare Worker) che tiene la chiave: più
   sicuro, ma un account e un servizio in più da mantenere.
4. **Email** a un indirizzo di Michel: niente GitHub, ma le segnalazioni restano nella posta.

## Decisione
Alternativa 2, con la 1 come ripiego.
- La finestra (`apps/winfiles/src/report.rs`) chiede titolo, descrizione, passi (facoltativi) e
  nome; versione di WinFiles (installata o portatile), versione di Windows (dal registro: build
  22000 e oltre = Windows 11) e scheda in uso si aggiungono da sole. Il nome si ricorda nelle
  impostazioni.
- La chiave entra al momento della compilazione da `WINFILES_SEGNALAZIONI` (segreto del
  repository, `option_env!`), mai nel codice. L'invio passa da `curl` in un thread a parte; la
  chiave va a `curl` dallo standard input (`-K -`), non sulla riga di comando.
- Senza chiave (compilazione locale, PR da fork) o se l'invio non riesce, la finestra offre
  «Apri su GitHub» (modulo `.github/ISSUE_TEMPLATE/*.yml` riempito dall'indirizzo) e «Copia il
  testo».
- Il repository è pubblico: la finestra e i moduli avvisano di non scrivere dati dei clienti.
- `WINFILES_SEGNALAZIONI_API` sostituisce l'indirizzo dell'API, per provare con un server finto.

## Conseguenze
- La chiave scade (al massimo dopo un anno, se GitHub non permette "senza scadenza"): va rifatta e
  rimessa nel segreto, poi serve un rilascio. Con la chiave scaduta la finestra lo dice e lascia
  copiare il testo.
- Le issue risultano create dall'account di chi ha fatto la chiave; chi segnala è nel testo.
