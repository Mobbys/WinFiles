# WinFiles

Stampa delle miniature di immagini, PDF, AI ed EPS su fogli A4 e A3, raggruppate per cartella e
sottocartella, con una nota per ogni file e per ogni cartella. È la riscrittura in Rust di
WinFile 4.9.

**Stato:** Fase 0, la base del progetto. Si trascinano file o cartelle nella finestra e si vede
l'impaginazione su A4 o A3 con riquadri al posto delle miniature; le miniature vere arrivano nella
Fase 2. Le fasi sono in [`docs/roadmap.md`](docs/roadmap.md).

## Scaricare l'eseguibile
Ogni modifica viene compilata per Windows da GitHub Actions: nella scheda **Actions** si apre
l'ultima esecuzione verde di "CI" e si scarica l'artifact `winfiles-windows`, che contiene
`winfiles.exe`. Con i rilasci arriverà anche l'installer nella pagina Releases.

## Compilare in locale
1. Installare Rust da <https://rustup.rs> (su Windows serve anche "Desktop development with C++"
   di Visual Studio Build Tools, che rustup propone durante l'installazione).
2. `cargo run -p winfiles --release` avvia l'app; `cargo build -p winfiles --release` crea
   `target/release/winfiles.exe`.
3. `cargo xtask ci` esegue tutti i controlli che la CI farà sulla PR.

Le regole di sviluppo sono in [`CLAUDE.md`](CLAUDE.md).
