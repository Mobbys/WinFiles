# WinFiles: istruzioni per chi sviluppa (persone e agenti)

WinFiles è la riscrittura in Rust di WinFile 4.9 (Python, repository `Mobbys/WinFile`): un'app
Windows per stampare su A4 e A3 le miniature di immagini, PDF, AI ed EPS, raggruppate per cartella,
con note per file e per cartella. Il piano completo, con scelte e rischi, è nel documento
"Piano di riscrittura WinFile in Rust" del progetto; lo stato delle fasi è in
[`docs/roadmap.md`](docs/roadmap.md) e le decisioni in [`docs/adr/`](docs/adr/).

## Il ciclo per ogni modifica
1. Una issue con criteri di accettazione verificabili.
2. Analisi dei casi limite e almeno due alternative; una scelta che dura diventa un ADR.
3. Test prima del codice: ogni bug riceve un test che lo riproduce prima della correzione.
4. PR piccole, una funzione per PR.
5. `cargo xtask ci` verde prima di ogni commit (formattazione, clippy senza avvisi, test,
   livelli, licenze con cargo-deny se installato).
6. Per tutto ciò che si vede (interfaccia, anteprima, PDF): guardare il risultato, non solo i test.

## Mai un crash
Chi usa WinFiles ci affida il proprio lavoro: un crash perde note e impaginazione.
- Niente `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `unreachable!` fuori dai test,
  niente `unsafe`: lo impongono i lint del workspace (`Cargo.toml`), i test sono esentati
  (`clippy.toml`).
- Gli errori sono `Result` con il tipo di errore del crate (`thiserror`).
- I numeri letti dai file sono ostili: limiti alle dimensioni, `get()` invece di `[i]`,
  aritmetica controllata, nessuna allocazione decisa dal file senza un tetto.

## Misure
Tutte le misure interne sono in **millimetri** (`f64`), origine in alto a sinistra del foglio.
Le conversioni in punti PDF o pixel stanno in `winfiles-geom` e si fanno solo ai bordi.
Il motore di layout è l'unico posto che decide dove va qualcosa sul foglio: anteprima, PDF e
stampa leggono il suo risultato e non ricalcolano nulla.

## Architettura
| Livello | Crate | Compito |
|---|---|---|
| L0 | `crates/geom` | millimetri, formati carta, rettangoli |
| L1 | `crates/model` | progetto `.winfiles`: file, pagine, note, impostazioni di stampa |
| L1 | `crates/formats` | misure dei file dalle intestazioni (pixel, DPI, pagine, TrimBox, BoundingBox) |
| L1 | `crates/thumbs` | miniature per l'anteprima (immagini, prima pagina di PDF/AI, anteprima degli EPS) |
| L1 | `crates/scene` | la scena di un foglio in mm: ciò che anteprima, PDF e stampa disegnano |
| L2 | `crates/layout` | impaginazione in fogli A4/A3 |
| L2 | `crates/pdf_out` | scene dei fogli in un PDF (krilla), vedi ADR 0008 |
| app | `apps/winfiles` | finestra egui (provvisoria finché non c'è il crate `ui`) |
| strumenti | `xtask` | `cargo xtask ci`, `cargo xtask layers` |

L'app usa già i pannelli agganciabili di `craft-ui` e il tema di VectorCraft (`apps/winfiles/src/theme.rs`):
nessun colore scritto a mano nei widget, e ogni testo deve passare il test di contrasto in
`theme.rs` sia nel tema scuro sia in quello chiaro.
I comandi seguono lo stile di Windows 11 (Fluent): campi, caselle, pallini, cursori e pulsanti a
due stati si prendono da `apps/winfiles/src/fluent.rs`, non direttamente da egui.
In arrivo: decodificatori in processo separato (Fase 2), il crate `ui` (Fase 4).
La stampa usa già `craft-print` con lo stesso PDF dell'esportazione (ADR 0009) e la correzione
della stampante calibrata (ADR 0010).
`cargo xtask layers` vieta egui, eframe, winit e rfd sotto l'interfaccia.

## Dipendenze
- Nessuna nuova dipendenza senza motivo scritto nella PR (e un ADR se è strutturale) e
  `cargo deny check` verde. Vietate le licenze GPL e AGPL (quindi niente MuPDF né Ghostscript
  incorporati).
- Le librerie del team ArtCraft (`storytold/craft-libs`, `vectorcraft`, `pdfcraft`, `photocraft`)
  si usano da git con un commit fissato, mai un branch; aggiornarle è una PR a sé, con i test.
- Nessuna icona, immagine o font di Microsoft o Adobe. Ogni asset ha una riga in
  [`ATTRIBUTION.md`](ATTRIBUTION.md).

## Lingua
Interfaccia, messaggi di errore, commenti e documentazione in italiano. Nomi di tipi e funzioni
in inglese.

## Comandi utili
- `cargo run -p winfiles -- <file o cartelle>`: apre l'app con quei file già caricati.
- `cargo run -p winfiles --features opengl`: disegno OpenGL invece di wgpu (PC senza GPU
  moderna, macchine virtuali, screenshot sotto Xvfb).
- `cargo run -p winfiles -- --pdf uscita.pdf lavoro.winfiles`: esporta i fogli in PDF e chiude;
  per guardare il risultato senza interfaccia (`pdftoppm -png uscita.pdf`).
- `cargo run -p winfiles -- --calibrazione A4 calibrazione.pdf`: la pagina di calibrazione (A4 o A3)
  in PDF, poi chiude.
- `cargo xtask ci`: tutti i controlli.
- L'eseguibile Windows lo produce la CI (job "Windows") come artifact `winfiles-windows`.
