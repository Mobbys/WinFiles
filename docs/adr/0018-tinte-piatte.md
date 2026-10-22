# 0018. Tinte piatte: elenco, nome e sovrastampa dentro il contenuto delle pagine

**Stato:** accettata, 2026-10-16

## Contesto
Michel vuole, nel modulo Modifica PDF, un pannello con le tinte piatte del documento, per
rinominarle (un "PANTONE 185 C" che in macchina si chiama "Rosso") e per metterle in sovrastampa
(una vernice, un bianco, un oro che non devono bucare i colori sotto). L'ADR 0015 aveva scelto di
non toccare il contenuto delle pagine; la sovrastampa però non sta nella struttura del file: è uno
stato grafico (`/OP`, `/op` in un ExtGState) attivato dentro il flusso di contenuto, prima di ogni
oggetto che dipinge.

## Alternative
1. **Sovrastampa su tutto il contenuto della pagina.** Un `gs` all'inizio della pagina: semplice,
   ma mette in sovrastampa anche la quadricromia, e un nero 100% su un'immagine si stampa diverso.
   Sbagliato per chi va in macchina.
2. **Ridisegnare la pagina** con hayro + krilla: perde quello che l'ADR 0015 vuole conservare.
3. **Scelta: riscrivere solo gli operatori** con `pdfcraft-content` (stesso commit fissato del
   resto di pdfcraft, già una dipendenza indiretta di `pdfcraft-organize`). Si legge il flusso
   con `parse`, si segue lo spazio colore di riempimento e di contorno (`cs`/`CS`, `g`, `rg`, `k`,
   `q`/`Q`, `gs`) e si inserisce `/WFSovraXY gs` solo davanti agli oggetti che dipingono con la
   tinta scelta (tracciati, testo, immagini a maschera, sfumature anche dentro i pattern di
   sfumatura, moduli), e dopo un `gs` che
   rimette la sovrastampa com'era per tutto il resto. Le pagine e i moduli (XObject form) si
   riscrivono in un flusso nuovo compresso; tutto il resto esce com'era.

## Decisioni collegate
- `crates/pdf_edit/src/spots.rs` trova le tinte da `Separation`, `DeviceN` (anche gli attributi
  `Colorants`), `Indexed` e `Pattern`; `All` e `None` (registro) e i quattro colori di processo
  non sono tinte piatte. Il colore del quadratino viene dalla funzione di tinta (tipo 2) verso
  CMYK, RGB, grigio o Lab; per le altre si mostra un quadratino vuoto.
- Per ogni tinta si dice su quali pagine compare e se la sovrastampa c'è su tutti, su una parte
  o su nessuno dei suoi oggetti ("Sovrastampa (solo in parte)").
- **Rinominare** cambia il nome in tutti gli spazi colore che lo usano, nelle pagine e nei moduli;
  un nome vuoto, uguale o più lungo di 127 byte si rifiuta.
- Gli ExtGState `WFSovra00`…`WFSovra11` si creano una volta per documento; ogni riscrittura ha
  un tetto (512 MiB di contenuto, 24 livelli di moduli annidati, 64 tinte) e ogni modulo si
  riscrive una volta sola.
- Un cambio prova prima su una copia: se non cambierebbe nulla non entra in Annulla.

## Conseguenze
- Il contenuto delle pagine toccate cambia in byte (un flusso nuovo), non nell'aspetto: testo,
  vettori e trasparenze restano quello che erano.
- Una dipendenza in più nel `Cargo.toml` (`pdfcraft-content`), ma nessun crate nuovo nel
  `Cargo.lock`.
- Le tinte usate solo dentro le annotazioni o nei pattern a tassello per ora non compaiono
  nell'elenco; una tinta definita nelle risorse ma mai dipinta si vede come "non usata nelle
  pagine", con la casella spenta.
