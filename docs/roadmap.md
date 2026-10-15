# Roadmap

Dieci fasi in sequenza: si passa alla successiva solo quando il criterio è verificato.
WinFile 4.9 resta in uso per le miniature fino alla Fase 7 e per immagini e PDF fino alla Fase 9.

| Fase | Contenuto | Criterio per passare oltre | Stato |
|---|---|---|---|
| 0 | Base del progetto, CI, CLAUDE.md, corpus; prototipi di craft-ui, hayro, EPS e craft-print | Stack confermato o ripieghi decisi | In corso: base, pannelli e misure pronti; prototipi da fare |
| 1 | Scansione ricorsiva, metadati, file di progetto, note salvate | Progetto da 10.000 file aperto e salvato | In corso: apri e salva pronti |
| 2 | Decodificatori in processo separato, profili ICC, cache su disco | Corpus completo senza crash | Da fare |
| 3 | Griglia, elenco e nesting su A4 e A3, intestazioni cartella, export PDF | Parità con la stampa attuale, ±0,5 mm | In corso: griglia, elenco, nesting, riepilogo ed export PDF pronti |
| 4 | Pannelli agganciabili, anteprima, note, ordine a trascinamento, filtri | Un lavoro vero fatto solo con la nuova app | Da fare |
| 5 | Stampa Windows a scala 100%, area stampabile del driver, calibrazione | A4 e A3 esatti su due stampanti | In corso: stampa diretta pronta ([ADR 0009](adr/0009-stampa-diretta.md)) |
| 6 | CSV, appunti, modelli salvati, segni di taglio, miniature vettoriali, HEIC | Ogni extra con i suoi criteri | Da fare |
| 7 | Installer MSI, aggiornamenti, beta in parallelo con WinFile 4.9 | Liste Anteprime 4.9 messa a riposo | Da fare |
| 8 | Modulo Immagini: dati tecnici, calcolo DPI, ruota, specchia, ritaglia, RAW, HEIC | Tutto ciò che fa oggi Controllo Immagini | Da fare |
| 9 | Modulo PDF: unisci, dividi, ruota, elimina, ritaglia, margini, adatta ad A4 o A3 | WinFile 4.9 messo a riposo | Da fare |

## Fase 0 in dettaglio
- [x] Workspace Cargo, lint "mai un crash", `cargo xtask ci`, controllo dei livelli
- [x] `cargo-deny` per licenze e vulnerabilità
- [x] CI: controlli su Linux, test ed eseguibile su Windows
- [x] Crate `geom`, `model` (progetto JSON con note), `layout` (griglia con test di proprietà)
- [x] Prima finestra: file trascinati, anteprima del foglio, note, impostazioni
- [x] Pannelli agganciabili con `craft-ui` e tema "Medio scuro" di VectorCraft (font Inter, icone Lucide)
- [x] Misure dei file: pixel e DPI delle immagini, pagine, formato e TrimBox di PDF e AI, BoundingBox degli EPS ([ADR 0005](adr/0005-misure-dei-file.md))
- [x] Misure in mm o cm e m² per file, cartella e totale
- [x] Miniature vere di immagini, PDF/AI ed EPS con anteprima; tema scuro, chiaro e automatico con test del contrasto ([ADR 0006](adr/0006-miniature-e-temi.md))
- [ ] Prototipo rendering PDF/AI con `hayro`, misura dei tempi sul corpus
- [ ] Prototipo EPS con il crate di VectorCraft
- [x] Stampa con `craft-print`: stampante, copie, fronte e retro, fogli, 100% o riduzione ([ADR 0009](adr/0009-stampa-diretta.md))
- [ ] Pagina di calibrazione
- [ ] Corpus di file di prova difficili

## Fase 1 in dettaglio
- [x] Apri, salva e salva con nome un lavoro `.winfiles` con note, titoli e impaginazione ([ADR 0007](adr/0007-file-di-lavoro.md))
- [x] "Salvare le modifiche?" prima di chiudere, aprire o creare un lavoro
- [x] File mancanti segnati all'apertura, misure rilette in background
- [x] Togliere file dall'elenco, con scelta multipla
- [x] Note per pagina e foglio di riepilogo in cima
- [x] Ricerca dei file in background con conteggio e Annulla
- [x] Prova con un lavoro da 10.000 file: JSON 2,6 MB letto e scritto in 60 ms, impaginazione in 5 ms per fotogramma (625 fogli A4)
- [ ] Ricalcolare l'impaginazione solo quando il lavoro cambia, non a ogni fotogramma
- [x] Lavori recenti nel menu File (in `%APPDATA%\WinFiles\recenti.json`)
- [x] Ultime scelte ricordate: pannelli, tema, unità, impaginazione, stampante, finestra (`impostazioni.json`)

## Fase 3 in dettaglio
- [x] Griglia con intestazioni di cartella, note e titolo
- [x] Foglio di riepilogo in cima
- [x] Esporta PDF (File, Ctrl+E e barra): stessa scena dell'anteprima, Inter incorporato, miniature JPEG a 150 DPI ([ADR 0008](adr/0008-esportazione-pdf.md))
- [x] La rotella cambia foglio nell'anteprima, Ctrl+rotella ingrandisce
- [x] Vista a elenco: una miniatura per riga, nome, misure e note a destra
- [x] Vista nesting: tutti i file alla stessa scala 1:N, automatica o scelta, accostati in righe centrate
- [x] Elenco con la larghezza delle miniature in mm
- [ ] Confronto con la stampa della 4.9, ±0,5 mm
