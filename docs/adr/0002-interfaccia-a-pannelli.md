# 0002. Interfaccia a pannelli agganciabili, non ribbon

**Stato:** accettata; il docking di `craft-ui` è da provare nella Fase 0, 2026-10-14

## Contesto
Il lavoro tipico usa insieme tre cose: elenco dei file, anteprima del foglio, note e proprietà del
file selezionato. I comandi sono circa 30. Riferimenti esaminati: WordCraft (ribbon stile Office) e
VectorCraft (pannelli stile Adobe), entrambi in Rust su egui.

## Decisione
Pannelli agganciabili come VectorCraft, con il docking condiviso `craft_ui::docking`, una barra
comandi compatta e la ricerca comandi. Pannelli: File, Anteprima (centrale, fisso), Proprietà,
Note, Impaginazione. Aree di lavoro salvate e "Ripristina".

## Alternative
Ribbon come WordCraft: il codice è interno all'app (circa 2.600 righe) e andrebbe riscritto; toglie
120–150 px di altezza all'anteprima e nasconde le note dietro le schede.

## Conseguenze
I pannelli fluttuano solo dentro la finestra principale (limite dichiarato di `craft-ui`): niente
secondo monitor nella v1. Fino al prototipo, l'app usa pannelli fissi di egui.
