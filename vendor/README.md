# Librerie esterne modificate

Copie di crate di crates.io con piccole modifiche che servono a WinFiles e non sono ancora nella
versione ufficiale. Il workspace le usa con `[patch.crates-io]` in `Cargo.toml` e le esclude dai
suoi controlli (formattazione, clippy, lint "mai un crash"): restano com'erano a monte, più le
modifiche segnate con un commento `WinFiles:`.

| Crate | Versione | Modifica | Perché |
|---|---|---|---|
| `hayro-interpret` | 0.8.0 | Legge `OP` e `op` dagli ExtGState e disegna in Multiply gli inchiostri (CMYK, colori speciali, DeviceN) in sovrastampa | Anteprima della sovrastampa nelle miniature ([ADR 0011](../docs/adr/0011-anteprima-sovrastampa.md)) |

Aggiornare la versione a monte vuol dire rifare le modifiche sulla copia nuova e rifar passare il
test `la_sovrastampa_si_vede_come_in_stampa` di `crates/thumbs`. Quando la sovrastampa arriverà
nella versione ufficiale, la copia si toglie.
