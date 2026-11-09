# 0026. Abbondanza che segue una fustella sagomata

Stato: Accettata (2026-10-29)

## Contesto
Molti lavori di Michel (adesivi, etichette, sagomati per il plotter di Onyx) hanno una fustella
che non è un rettangolo: angoli tondi, cerchi, stelle, forme libere. La grafica si ferma sulla
fustella e l'abbondanza deve girarle attorno, alla stessa distanza, non sul rettangolo che la
contiene (ADR 0024), che lascerebbe la forma senza abbondanza proprio dove si taglia.

## Alternative
1. **Raster**: disegnare la pagina in pixel e riempire la fascia con il colore del punto più
   vicino dentro la forma. Funziona con tutto, ma la fascia esce in RGB e a una risoluzione
   limitata (su un 100 × 200 cm pochi DPI), e un colore CMYK o una tinta piatta convertiti in RGB
   non stampano uguali all'originale: si vedrebbe un bordo diverso dove il taglio non è preciso.
2. **Ribaltare a specchio lato per lato**: come l'ADR 0024, ma sulle punte strette e nelle forme
   sottili la grafica ribaltata arriva da fuori dalla forma (bianco).
3. **Allungare lato per lato l'ultima striscia (scelta)**, tutto vettoriale: per ogni lato del
   contorno, la striscia di mezzo millimetro appena dentro si stira fuori fino all'abbondanza;
   negli angoli sporgenti la punta della grafica si apre a ventaglio.

## Decisione
Alternativa 3, in `crates/pdf_edit/src/frame.rs` (`bleed_shape`). Il contorno è l'unione dei
tracciati della fustella (o di quelli scelti), con `i_overlay` come per l'ADR 0025, semplificato a
0,1 mm per i pezzi; la pagina vera si ritaglia sul contorno intero.
- Ogni lato è un pezzo: la grafica (senza fustella) disegnata con una matrice che stira la
  striscia tra 1 e 2,4 punti dentro il lato fino all'abbondanza fuori.
- Dove due lati girano poco (fino a 40°) i pezzi si incontrano a metà; oltre, l'angolo fuori si
  riempie a spicchi: la punta della grafica, un soffio dentro, si apre fino a coprire lo spicchio.
- Vicino a una punta stretta (meno di 90°) la striscia uscirebbe dall'altro lato: lì si allunga
  anche lungo il lato il primo pezzo di striscia che ci sta tutto.
- Negli angoli rientranti i pezzi dei due lati si sovrappongono; quello che entra nella forma
  resta sotto la pagina vera.
Il pannello Tracciati propone "Segui la forma della fustella" solo quando il contorno non è un
rettangolo (`outline_is_rectangle`); senza la spunta resta l'abbondanza rettangolare.

Quando quello che si allunga è solo uno sfondo scelto in tinta unita (un riempimento senza
contorno, non in un modulo, non ritagliato), non servono pezzi: la sua forma si allarga di
`amount` (`i_overlay`, angoli tondi; vivi se il formato finito è il suo rettangolo) e si riscrive
al suo posto con lo stesso colore, come curve morbide che passano per i punti del contorno
semplificato a 0,05 mm (`paths::grow`). Se gli sfondi scelti stanno dentro una maschera (un
tracciato che ritaglia, come i "gruppi di ritaglio" di Illustrator), si allarga allo stesso modo
anche la maschera: la lettura della pagina tiene, per ogni tracciato, le maschere che lo
ritagliano, e il formato finito è la parte che si vede dentro la maschera. Gli oggetti scelti si allargano solo
fuori dal formato finito (l'unione di quello che se ne vede), e ognuno solo davanti al pezzo di
bordo che fa lui: il bordo si divide lato per lato secondo l'oggetto che si vede appena dentro,
e la fascia fuori da quel lato va a lui, divisa lungo la perpendicolare dove il bordo cambia
oggetto. Dentro, dove gli oggetti si sovrappongono, il disegno resta com'era. Resta un tracciato solo, liscio, che si modifica come gli
altri in Illustrator; la pagina prende formato finito e abbondanza senza ritagli.

## Conseguenze
- Colori, tinte piatte e risoluzione restano quelli del file: la fascia è la grafica stessa.
- Ogni pezzo ridisegna la grafica (ritagliata): una forma con molte curve fa un PDF con qualche
  centinaio di pezzi, più lento da aprire se la grafica è pesante. Oltre 6000 pezzi si rinuncia
  con un messaggio.
- Nell'anteprima tra un pezzo e l'altro può vedersi un filo appena più chiaro (l'antialias dei
  bordi dei ritagli); in stampa non c'è.
- La pagina diventa il rettangolo della forma più l'abbondanza: fuori dalla fascia resta bianco.
