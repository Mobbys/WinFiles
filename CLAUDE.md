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
| L2 | `crates/layout` | impaginazione in fogli A4/A3 |
| app | `apps/winfiles` | finestra egui (provvisoria finché non c'è il crate `ui`) |
| strumenti | `xtask` | `cargo xtask ci`, `cargo xtask layers` |

In arrivo: `scan` (Fase 1), `formats` e `thumbs` (Fase 2), `pdf_out` (Fase 3), `ui` con il docking
di `craft-ui` (Fasi 0 e 4), stampa con `craft-print` (Fase 5).
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
- `cargo xtask ci`: tutti i controlli.
- L'eseguibile Windows lo produce la CI (job "Windows") come artifact `winfiles-windows`.
