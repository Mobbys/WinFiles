# 0012. Foto HEIC con il lettore di PhotoCraft

**Stato:** accettata, 2026-10-15

## Contesto
Le foto degli iPhone e dei Mac sono HEIC (HEIF con immagini HEVC). `image` 0.25 non le legge e
WinFiles le scartava. Servono le misure (pixel) e la miniatura, anche nel PDF e in stampa.

## Alternative
1. `libheif` con `libde265`: completo, ma C e LGPL, da distribuire come DLL accanto all'eseguibile.
2. Il codec HEIF di Windows (estensione "HEIF Image Extensions" dello Store) attraverso WIC: niente
   codice in più, ma su molti PC manca e serve `unsafe` per COM, vietato nel workspace.
3. **Scelta:** `photocraft-heif`, il lettore di PhotoCraft (`storytold/photocraft`, a commit
   fissato), che avvolge `heic-rs` 0.1.4: Rust puro, senza `unsafe`, MIT/Apache. Legge foto in
   un pezzo e a tessere, 8 e 10 bit, trasparenza, rotazione e ritaglio del contenitore, e
   trasforma ogni panic di `heic-rs` in un errore. È lo stesso codice che userà il modulo
   Immagini (Fase 8).

## Decisioni collegate
- Le misure si leggono dal solo riquadro `meta` in testa al file (di solito pochi KB); se non
  basta si legge il file intero, fino a 512 MB.
- Le HEIC non dichiarano una risoluzione di stampa: valgono i DPI predefiniti (96), segnalati in
  arancione come per ogni immagine senza DPI.
- La miniatura si decodifica intera e si riduce; tetto di 120 milioni di pixel (sotto 1 GB di
  memoria anche a 16 bit con trasparenza). Le immagini a 10 bit si mostrano a 8.
- Le foto di prova in `testdata/heic/` vengono da `heic-rs` (MIT/Apache, pixel sintetici).

## Conseguenze
- HEVC è coperto da brevetti in alcuni paesi; decodificarlo in un programma per uso interno è la
  stessa scelta fatta da PhotoCraft. Se WinFiles venisse distribuito a terzi va riconsiderato.
- `heic-rs` è giovane e con un solo autore: aggiornarlo è una PR a sé, con i test delle foto di
  prova, come per le altre librerie del team.
