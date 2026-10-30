# Roadmap

Dieci fasi in sequenza: si passa alla successiva solo quando il criterio è verificato.
WinFile 4.9 resta in uso per le miniature fino alla Fase 7 e per immagini e PDF fino alle Fasi 9 e 8.

| Fase | Contenuto | Criterio per passare oltre | Stato |
|---|---|---|---|
| 0 | Base del progetto, CI, CLAUDE.md, corpus; prototipi di craft-ui, hayro, EPS e craft-print | Stack confermato o ripieghi decisi | In corso: base, pannelli e misure pronti; prototipi da fare |
| 1 | Scansione ricorsiva, metadati, file di progetto, note salvate | Progetto da 10.000 file aperto e salvato | In corso: apri e salva pronti |
| 2 | Decodificatori in processo separato, profili ICC, cache su disco | Corpus completo senza crash | Da fare |
| 3 | Griglia, elenco e nesting su A4 e A3, intestazioni cartella, export PDF | Parità con la stampa attuale, ±0,5 mm | In corso: griglia, elenco, nesting, riepilogo ed export PDF pronti |
| 4 | Pannelli agganciabili, anteprima, note, ordine a trascinamento, filtri | Un lavoro vero fatto solo con la nuova app | In corso: pannelli, note, stampa sì/no per file e pagina, ricerca, filtri e ordine a trascinamento pronti |
| 5 | Stampa Windows a scala 100%, area stampabile del driver, calibrazione | A4 e A3 esatti su due stampanti | Fatta: stampa diretta ([ADR 0009](adr/0009-stampa-diretta.md)) e calibrazione ([ADR 0010](adr/0010-calibrazione-stampante.md)), A4 e A3 esatti provati da Michel |
| 6 | CSV, appunti, modelli salvati, HEIC, AI in PostScript | Ogni extra con i suoi criteri | Fatta e provata da Michel: CSV, appunti, sovrastampa, modelli salvati, HEIC, AI in PostScript |
| 7 | Installer MSI, aggiornamenti, beta in parallelo con WinFile 4.9 | Liste Anteprime 4.9 messa a riposo | Fatta: installer provato da Michel, Liste Anteprime 4.9 non serve più, 0.1.0 pubblicata ([ADR 0014](adr/0014-installer-e-aggiornamenti.md)) |
| 8 | Modulo Immagini: dati tecnici, calcolo DPI, ruota, specchia, ritaglia, RAW, HEIC | Tutto ciò che fa oggi Controllo Immagini | Fatta: modulo "Modifica immagini" nella finestra ([ADR 0016](adr/0016-modifica-immagini.md)), con RAW e PSD ([ADR 0017](adr/0017-raw-e-psd.md)), da provare |
| 9 | Modulo PDF: unisci, dividi, ruota, elimina, ritaglia, margini, adatta ad A4 o A3 | WinFile 4.9 messo a riposo | Fatta: modulo "Modifica PDF" nella finestra ([ADR 0015](adr/0015-modifica-pdf.md)), da provare |

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
- [x] Pagina di calibrazione (File › Calibra la stampante)
- [ ] Corpus di file di prova difficili

## Fase 1 in dettaglio
- [x] Apri, salva e salva con nome un lavoro `.winfiles` con note, titoli e impaginazione ([ADR 0007](adr/0007-file-di-lavoro.md))
- [x] "Salvare le modifiche?" prima di chiudere, aprire o creare un lavoro
- [x] File mancanti segnati all'apertura, misure rilette in background
- [x] Togliere file dall'elenco, con scelta multipla
- [x] Note per pagina e foglio di riepilogo in cima
- [x] Ricerca dei file in background con conteggio e Annulla
- [x] Prova con un lavoro da 10.000 file: JSON 2,6 MB letto e scritto in 60 ms, impaginazione in 5 ms per fotogramma (625 fogli A4)
- [x] Ricalcolare l'impaginazione solo quando il lavoro cambia, non a ogni fotogramma
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

## Fase 4 in dettaglio
- [x] "Stampa le note": le note restano nel lavoro ma si possono togliere dal foglio
- [x] L'impaginazione si rifà solo quando il lavoro cambia
- [x] In stampa sì/no per file, cartella e pagina (casella nell'elenco, Spazio, Proprietà): il file resta nell'elenco con le sue note, i totali contano solo quello che si stampa
- [x] Cerca per nome o cartella (Ctrl+F, Esc svuota) e filtra per tipo nell'elenco dei file: cambia solo quello che si vede, non la stampa
- [x] Pagine dei PDF nell'elenco: freccia per aprirle, una riga per pagina con la sua casella di stampa e la nota; la ricerca trova le singole pagine ("catalogo 3", "pag. 12", testo delle note) e il clic porta l'anteprima al foglio della pagina
- [x] Ordine a trascinamento nell'elenco: i file dentro la loro cartella (anche più file scelti insieme), le cartelle intere
- [x] Ogni tipo di file nell'elenco (Michel, 16 ottobre 2026): i non supportati hanno nome, nota e un riquadro con l'estensione sul foglio, senza anteprima né misure; la casella "Includi i file non supportati" li toglie e li rimette tutti; restano fuori solo i file nascosti e di sistema (Thumbs.db, desktop.ini, `~$` di Office)
- [x] I file appena aggiunti si ordinano per cartella e poi per nome senza badare alle maiuscole: una cartella non compare più due volte se ha sottocartelle e nomi in maiuscolo

## Fase 6 in dettaglio
- [x] Esporta tabella CSV (File): una riga per pagina in stampa, base e altezza in colonne, m², nota; per Excel in italiano
- [x] Copia tabella (File, Ctrl+Maiusc+C; Ctrl+C con file scelti; clic destro): HTML con cartelle e totali per Excel, Word e posta, più testo a tabulazioni
- [x] Anteprima della sovrastampa nelle miniature di PDF e AI ([ADR 0011](adr/0011-anteprima-sovrastampa.md))
- [x] Modelli di impaginazione salvati (pannello Impaginazione › Modello: si salvano con un nome, il titolo resta del lavoro)
- ~~Segni di taglio~~: tolti, non servono (Michel, 15 ottobre 2026)
- ~~Miniature vettoriali nel PDF~~: tolte, il PDF resta leggero (Michel, 15 ottobre 2026)
- [x] Foto HEIC/HEIF di iPhone e Mac: misure, miniature, PDF e stampa ([ADR 0012](adr/0012-foto-heic.md))
- [x] Miniature degli AI in PostScript ("legacy", file da taglio) e degli EPS di Illustrator senza anteprima ([ADR 0013](adr/0013-ai-in-postscript.md))

## Fase 7 in dettaglio
- [x] Icona di WinFiles (finestra, eseguibile, menu Start, file `.winfiles`)
- [x] Installer MSI per l'utente, senza amministratore: menu Start, file `.winfiles`, disinstallazione ([ADR 0014](adr/0014-installer-e-aggiornamenti.md))
- [x] Aggiornamenti: controllo all'avvio, barra con Novità e Aggiorna, installazione alla chiusura; Aiuto › Controlla aggiornamenti
- [x] Rilascio automatico da un tag `vX.Y.Z`: MSI, eseguibile portatile e novità su GitHub
- [x] Repository pubblico (Michel) e primo rilascio, 0.1.0
- [x] Installer provato da Michel; Liste Anteprime della 4.9 non serve più (15 ottobre 2026)
- [x] Passaggio dalla 4.9: barra "Togli WinFile 4.9" e icona sul Desktop ([ADR 0019](adr/0019-passaggio-dalla-4-9.md))
- [ ] Rilascio `5.0` su `Mobbys/WinFile` con l'MSI di WinFiles 0.2.3

## Fase 9 in dettaglio
- [x] Motore `crates/pdf_edit` sopra `pdfcraft`: apre, modifica la struttura, salva in modo atomico ([ADR 0015](adr/0015-modifica-pdf.md))
- [x] Modulo "Modifica PDF" nella finestra (scheda in alto, `--modifica-pdf file.pdf` dalla riga di comando)
- [x] Elenco delle pagine con miniature vere, misure in mm o cm, scelta multipla (Ctrl, Maiusc, Tutte, Nessuna)
- [x] Anteprima grande della pagina scelta, con zoom (Adatta e scatti, Ctrl+rotella) e disegno progressivo
- [x] Ruota a destra e a sinistra, elimina (mai l'ultima pagina), duplica, ordine a trascinamento
- [x] Unisci più PDF, inserisci pagine da un altro PDF (all'inizio, dopo la pagina corrente, alla fine)
- [x] Estrai le pagine scelte in un file nuovo; dividi in più file ai punti scelti
- [x] Rifila al TrimBox, margini colorati, misura libera, adatta ad A4 o A3 senza ridisegnare la pagina
- [x] Annulla e Ripeti con l'etichetta dell'operazione; "Salvare le modifiche?" prima di chiudere o aprire
- [x] "Modifica PDF" e "Modifica immagine" dal tasto destro sui file, nell'elenco e sul foglio
- [x] Taglierina sull'anteprima della pagina, con lente, Invio ed Esc; taglia una pagina o tutte
- [x] Margini negativi: un valore sotto zero toglie dal bordo
- [x] Pannello "Tinte piatte" a destra, chiudibile: elenco con colore e pagine, rinomina,
  sovrastampa solo sugli oggetti della tinta ([ADR 0018](adr/0018-tinte-piatte.md))
- [x] Strumento "Tracciati": clic su una linea per vederne colore, spessore, lunghezza; farla
  diventare una tinta con nome libero (fustella, cordonatura…), cambiare spessore e sovrastampa;
  controllo della pagina con fustella e abbondanza per lato ([ADR 0023](adr/0023-tracciati-e-fustelle.md))
- [x] "Crea l'abbondanza": a specchio o allungando il bordo, sui lati scelti, in vettoriale, senza
  ribaltare la fustella; TrimBox e BleedBox scritti ([ADR 0024](adr/0024-abbondanza-vettoriale.md))
- [x] Abbondanza solo per i tracciati scelti (lo sfondo), il resto resta com'è
- [x] "Crea la fustella" dal contorno dei tracciati scelti, uniti se si sovrappongono
  ([ADR 0025](adr/0025-fustella-dalla-selezione.md))
- [x] Abbondanza lungo una fustella sagomata (angoli tondi, forme), ADR 0026
- [x] Scheda "Nesting": copie dei PDF sagomati su lastre o rotolo, sulla fustella di taglio, con
  l'abbondanza; PDF dei fogli per Onyx ([ADR 0027](adr/0027-nesting.md))
- [x] Nesting: immagini come rettangoli, scala 1:N per file, "Manda al nesting" dal tasto destro
- [ ] Nesting: linee di taglio comuni (laser)
- [x] Fustella e abbondanza sulle immagini, fase 1: abbondanza con i pixel e PDF con fustella
  rettangolare o ad angoli tondi ([ADR 0028](adr/0028-fustella-e-abbondanza-sulle-immagini.md))
- [x] Fase 2: fustella dal contorno del soggetto (colore di sfondo con contagocce, o trasparenza)
- [x] Abbondanza lungo la fustella dal contorno (colore più vicino dentro la fustella) e lisciatura
- [ ] Fase 3: in Modifica PDF fustella dal contorno della grafica e dai tracciati di ritaglio,
  abbondanza con i pixel delle foto dentro il PDF
- [ ] Provata da Michel su file veri

## Fase 8 in dettaglio
- [x] Motore `crates/image_edit`: apri, ruota, specchia, ritaglia, ricampiona, salva ([ADR 0016](adr/0016-modifica-immagini.md))
- [x] Modulo "Modifica immagini" nella finestra (scheda in alto, `--modifica-immagini foto.jpg …`)
- [x] Più immagini aperte insieme, con le miniature nell'elenco a sinistra
- [x] Dati tecnici: pixel, modo colore e bit, profilo ICC, rapporto, peso del file
- [x] Misura di stampa: larghezza, altezza e DPI legati, i pixel non si toccano; area in m² e distanza di visione
- [x] Ridimensiona: rifà i pixel alla misura voluta, con le proporzioni bloccate
- [x] Ruota a destra e a sinistra, specchia in orizzontale e in verticale
- [x] Ritaglio a trascinamento: rettangolo con maniglie, si sposta, lente sui pixel veri
- [x] Orientamento EXIF applicato all'apertura, risoluzione letta dal file
- [x] Salva con formato (JPEG, PNG, TIFF, WebP), qualità del JPEG e profilo colore mantenuto; scrittura atomica
- [x] Annulla e Ripeti senza copiare i pixel
- [x] Foto RAW (sviluppate con i colori della fotocamera) e file di Photoshop, anche CMYK, aperti
  e mostrati tra le miniature ([ADR 0017](adr/0017-raw-e-psd.md))
- [ ] Salvataggio in CMYK: quando servirà
- [ ] Provata da Michel su file veri

Dopo la Fase 7 viene la Fase 9 (PDF), poi la 8 (Immagini): l'ordine l'ha scelto Michel il 15 ottobre 2026.
