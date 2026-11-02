# 0006. Miniature per l'anteprima e temi con contrasto verificato

**Stato:** accettata, 2026-10-14

## Contesto
Michel ha provato la prima versione: l'anteprima mostrava riquadri grigi con scritto "Immagine",
e il tema grigio medio (copiato da VectorCraft) aveva testi poco leggibili. Un'anteprima serve a
vedere i file, e i difetti di contrasto vanno trovati prima che arrivino a chi usa l'app.

## Alternative per le miniature
1. Rimandare alla Fase 2 con decodificatori in un processo separato: corretto per la stampa, ma
   lascia l'anteprima vuota per settimane.
2. **Scelta:** un crate `winfiles-thumbs` subito, nel processo dell'app ma isolato: `image` per
   JPEG, PNG, TIFF, GIF, BMP e WebP (con orientamento EXIF), `hayro` per la prima pagina di PDF e AI,
   l'anteprima TIFF dentro gli EPS con intestazione DOS. Ogni chiamata ha tetti di memoria e passa
   da `catch_unwind`; le miniature si chiedono solo per il foglio visibile, si preparano in due
   thread e restano in una cache di 600 texture al massimo.

Il processo separato della Fase 2 resta: servirà per la risoluzione di stampa e per i file che
fanno cadere un decodificatore in modo non recuperabile (memoria esaurita).

## Decisione sui temi
Due temi, scuro (il "Scuro" di VectorCraft con testi più chiari) e chiaro, più "Auto" che segue
Windows. Un test calcola il contrasto WCAG di ogni colore di testo su ogni sfondo: almeno 4,5:1 per
testi, avvisi e icone, 3:1 per i comandi disattivati. Il foglio nell'anteprima resta bianco con
inchiostro scuro in entrambi i temi, perché rappresenta la carta.

## Conseguenze
Un nuovo colore nel tema passa la CI solo se leggibile. Gli EPS senza anteprima e gli AI solo
PostScript mostrano ancora un segnaposto, finché non arriva l'interprete PostScript di VectorCraft.
