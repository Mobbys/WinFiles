# WinFiles

Stampa delle miniature di immagini, PDF, AI ed EPS su fogli A4 e A3, raggruppate per cartella e
sottocartella, con una nota per ogni file e per ogni cartella. È la riscrittura in Rust di
WinFile 4.9.

**Stato:** in uso. Si trascinano file o cartelle nella finestra, si vedono le miniature vere
impaginate su A4 o A3 (griglia, elenco o nesting), si scrivono le note, si esporta in PDF e si
stampa a scala esatta. Le fasi sono in [`docs/roadmap.md`](docs/roadmap.md).

L'app ha due moduli, come le schede della 4.9:
- **Miniature**: l'elenco dei file, l'impaginazione e la stampa.
- **Modifica PDF**: unisci, dividi, ruota, elimina, duplica, estrai pagine, margini, rifila e
  adatta ad A4 o A3 senza ridisegnare la pagina ([ADR 0015](docs/adr/0015-modifica-pdf.md)).

## Installare
Nella pagina **Releases** ci sono l'installer `WinFiles-X.Y.Z.msi` (si installa per l'utente, senza
amministratore, e si aggiorna da solo) e l'eseguibile singolo `winfiles.exe` per chi lo vuole
portatile. Per provare una modifica non ancora rilasciata: nella scheda **Actions** si apre
l'ultima esecuzione verde di "CI" e si scarica l'artifact `winfiles-windows`.

## Compilare in locale
1. Installare Rust da <https://rustup.rs> (su Windows serve anche "Desktop development with C++"
   di Visual Studio Build Tools, che rustup propone durante l'installazione).
2. `cargo run -p winfiles --release` avvia l'app; `cargo build -p winfiles --release` crea
   `target/release/winfiles.exe`.
3. `cargo xtask ci` esegue tutti i controlli che la CI farà sulla PR.

Le regole di sviluppo sono in [`CLAUDE.md`](CLAUDE.md).

## Licenza
Codice pubblicato da leggere, tutti i diritti riservati: vedi [LICENSE](LICENSE). I programmi dei
rilasci ufficiali si possono scaricare e usare.
