# Decisioni di architettura (ADR)

Una decisione che dura nel tempo si scrive qui, numerata, con contesto, alternative e conseguenze.
Una decisione si cambia con un nuovo ADR che sostituisce il precedente, mai riscrivendo la storia.

| N. | Decisione | Stato |
|---|---|---|
| [0001](0001-rust-e-librerie-artcraft.md) | Rust con le librerie del team ArtCraft | Accettata |
| [0002](0002-interfaccia-a-pannelli.md) | Interfaccia a pannelli agganciabili, non ribbon | Accettata, docking in uso |
| [0003](0003-layout-in-millimetri.md) | Un solo layout in millimetri per anteprima, PDF e stampa | Accettata |
| [0004](0004-repository-separato.md) | Repository separato da WinFile 4.9 | Accettata |
| [0005](0005-misure-dei-file.md) | Misure dei file dalle intestazioni, aree al formato finito | Accettata |
| [0006](0006-miniature-e-temi.md) | Miniature per l'anteprima, temi con contrasto verificato | Accettata |
| [0007](0007-file-di-lavoro.md) | Il lavoro salvato in un file `.winfiles` | Accettata |
| [0008](0008-esportazione-pdf.md) | Esportazione in PDF da una scena condivisa con l'anteprima | Accettata |
| [0009](0009-stampa-diretta.md) | Stampa diretta con `craft-print`, dallo stesso PDF | Accettata |
| [0010](0010-calibrazione-stampante.md) | Calibrazione della stampante con una pagina da misurare | Accettata |
| [0011](0011-anteprima-sovrastampa.md) | Anteprima della sovrastampa nelle miniature | Accettata |
| [0012](0012-foto-heic.md) | Foto HEIC con il lettore di PhotoCraft | Accettata |
| [0013](0013-ai-in-postscript.md) | Miniature degli AI in PostScript e degli EPS di Illustrator senza anteprima | Accettata |
| [0014](0014-installer-e-aggiornamenti.md) | Installer MSI e aggiornamenti dai rilasci di GitHub | Accettata |
| [0015](0015-modifica-pdf.md) | Modifica dei PDF con pdfcraft, senza ridisegnare le pagine | Accettata |
| [0016](0016-modifica-immagini.md) | Modifica delle immagini: misura e ricampionamento separati, colore conservato | Accettata |
| [0017](0017-raw-e-psd.md) | Foto RAW e file di Photoshop con le librerie di PhotoCraft | Accettata |
| [0018](0018-tinte-piatte.md) | Tinte piatte: elenco, nome e sovrastampa dentro il contenuto delle pagine | Accettata |
| [0019](0019-passaggio-dalla-4-9.md) | Passaggio dalla WinFile 4.9 a WinFiles con l'aggiornamento della 4.9 | Accettata |
| [0020](0020-copia-di-illustrator.md) | La copia di Illustrator nei PDF: si chiede al salvataggio se toglierla | Accettata |
| [0021](0021-quote.md) | Quote in proporzione sull'anteprima di Modifica PDF e Modifica immagini | Accettata |
| [0022](0022-apri-in-winfiles.md) | "Apri in WinFiles" nel tasto destro e una finestra sola | Accettata |
| [0023](0023-tracciati-e-fustelle.md) | Tracciati e fustelle: scegliere una linea sulla pagina e cambiarne colore, tinta e spessore | Accettata |
| [0024](0024-abbondanza-vettoriale.md) | Abbondanza creata in vettoriale, ribaltando la pagina attorno al formato finito | Accettata |
| [0025](0025-fustella-dalla-selezione.md) | Fustella dai tracciati scelti, unendo le forme (`i_overlay`) | Accettata |
| [0026](0026-abbondanza-sagomata.md) | Abbondanza che segue una fustella sagomata, allungando l'ultima striscia di grafica | Accettata |
| [0027](0027-nesting.md) | Nesting delle sagome su lastre e rotoli, con il motore del pannello per Illustrator portato in Rust | Accettata |
| [0028](0028-fustella-e-abbondanza-sulle-immagini.md) | Fustella e abbondanza sulle immagini: abbondanza con i pixel, PDF con fustella e TrimBox | Accettata |
