# 0007. Il lavoro salvato in un file `.winfiles`

**Stato:** accettata, 2026-10-14

## Contesto
Fino alla Fase 0 l'elenco, le note e l'impaginazione si perdevano chiudendo l'app. La Fase 1 chiede
di aprire e salvare un lavoro da 10.000 file. Servono anche le finestre di sistema per scegliere
dove salvare.

## Alternative
1. Salvataggio automatico in una cartella dell'app, senza file visibili: semplice, ma non si può
   mandare un lavoro a un collega né tenerne più d'uno.
2. Un database SQLite per lavoro: veloce su progetti enormi, ma illeggibile a mano e una dipendenza
   in C.
3. **Scelta:** un file JSON con estensione `.winfiles`, quello che il modello sa già scrivere e
   leggere (`Project::save` scrive un file temporaneo e poi sostituisce l'originale). Contiene
   percorsi, misure lette, note, titoli delle cartelle e impostazioni di stampa. Con 10.000 file
   pesa pochi MB; il tetto in lettura è 64 MB.

## Decisioni collegate
- Le finestre Apri e Salva usano `rfd` (MIT): su Windows il dialogo comune di Esplora file.
- `serde_json` con `float_roundtrip`: le misure tornano identiche al bit, così un lavoro appena
  aperto non risulta modificato.
- All'apertura le misure si rileggono in background (i file possono essere cambiati); i file che
  non ci sono più restano nell'elenco in arancione con "file non trovato".
- Chiudere, aprire o creare un lavoro con modifiche non salvate chiede prima "Salvare le modifiche?".
- Le aree si mostrano sia con abbondanza e crocini sia al vivo, come nella 4.9; due caselle
  dell'impaginazione nascondono l'una o l'altra riga sotto le miniature (completa l'ADR 0005).

## Conseguenze
I percorsi sono assoluti: un lavoro spostato su un altro PC trova i file solo se le cartelle sono
le stesse. Percorsi relativi al file di lavoro e "Cerca i file mancanti" sono da valutare quando
servirà.
