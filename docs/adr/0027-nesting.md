# 0027. Nesting delle sagome su lastre e rotoli

Stato: Accettata (2026-10-30)

## Contesto
In ditta i lavori sagomati (adesivi, etichette, forme per il plotter) arrivano come PDF con la
fustella in tinta piatta (Taglio) e l'abbondanza, e vanno disposti su una lastra o su un rotolo
prima del RIP (Onyx). Michel lo fa con il suo pannello per Illustrator (`Mobbys/illustratorNesting`,
motore JavaScript con no-fit polygon e Clipper), che però richiede Illustrator e lavora sugli
oggetti di un documento aperto, non sui file. WinFiles legge già fustelle e abbondanze dei PDF
(ADR 0023–0026).

## Alternative
1. **Rettangoli d'ingombro** (un algoritmo a scaffali): semplice, ma su stelle, cerchi e forme a L
   spreca molto materiale, proprio i lavori dove il nesting serve.
2. **Una libreria esterna** (libnest2d, SVGnest): C++ con licenza LGPL, o JavaScript; il primo
   tentativo di Michel (`Mobbys/Nesting`, Python con libnest2d) è stato abbandonato.
3. **Portare in Rust il motore del pannello (scelta)**: lo stesso metodo, già provato da Michel,
   con i suoi test; le operazioni sui poligoni con `i_overlay` (già nel progetto, ADR 0025) al
   posto di Clipper.

## Decisione
Alternativa 3.
- `crates/nest` (L1, solo `i_overlay`): sagome in mm; ogni sagoma si riduce al contorno esterno,
  si semplifica a 0,3 mm e si allarga di metà distanza (spigoli a punta tagliata, che contengono
  sempre l'allargamento tondo); l'NFP fra due sagome è l'unione delle somme di Minkowski delle loro
  parti convesse (Hertel–Mehlhorn), calcolata a gruppi di otto (tutte insieme `i_overlay` impiega
  secondi). Ogni copia va nella posizione libera più in alto e poi più a sinistra, provando le
  rotazioni scelte; dopo la prima disposizione si provano ordini diversi per il tempo scelto.
  I test del pannello sono portati uno per uno (forme concave, incastro delle L, foro della O,
  stelle a 15°, pezzi troppo grandi, più lastre, distanze, 50 lettere, pezzi nei fori, specchio).
- La sagoma di una pagina è la fustella di taglio (ruolo "Taglio" delle tinte di Modifica PDF),
  anche fatta di tratti aperti che si toccano; senza fustella è il formato finito del controllo
  della pagina (TrimBox, grafica senza bianco, pagina). La distanza fra le fustelle è almeno due
  abbondanze.
- Il PDF dei fogli (`pdf_edit::nest_document`): ogni pagina di partenza entra una volta come modulo
  (`page_as_form` di PdfCraft) e si disegna in ogni copia con la sua matrice, ritagliata sulla
  sagoma più l'abbondanza (almeno 0,5 mm, per non tagliare a metà la linea di taglio). Le risorse
  restano quelle del file: la tinta Taglio arriva a Onyx con il suo nome. Una pagina per foglio,
  TrimBox uguale alla pagina; un rotolo è lungo quanto serve, al massimo 5,08 m per pagina (limite
  del PDF senza `UserUnit`): oltre si passa a un'altra pagina.
- Il modulo è una scheda "Nesting" con l'elenco delle sagome e le copie («x20» o «20pz» nel nome
  del file le imposta, come nel pannello), le scelte a destra (ricordate) e l'anteprima dei fogli
  con le pagine ritagliate sulla sagoma.

## Conseguenze
- Le linee di taglio comuni del pannello (laser) sono arrivate dopo: vedi la sezione qui sotto.
- Le immagini entrano come rettangoli: al salvataggio ognuna diventa un PDF di una pagina
  (`pdf_out::image_page`, JPEG e PNG senza ricomprimere, il resto in PNG) e da lì segue la strada
  dei PDF.
- La scala del file (1:N) moltiplica la matrice della pagina; l'abbondanza, che è in mm sul foglio,
  sulla pagina si divide per N.
- Un PDF di molte pagine diventa molte sagome, una per pagina, ognuna con le sue copie.
- L'anteprima mostra le pagine a 512 pixel di lato: basta per riconoscerle, non per controllare i
  dettagli (quelli si guardano nel PDF salvato).

## Linee di taglio comuni (laser), 30 ottobre 2026
- Il calcolo è `nest::cutlines` (da `cutlines.js` del pannello, con i suoi test): i contorni delle
  copie al loro posto, una dopo l'altra; di ogni lato resta solo la parte che nessun lato già
  accettato copre (parallelo entro 1°, a meno di 0,6 mm), e i tratti consecutivi dello stesso
  contorno diventano una polilinea. I lati accettati stanno in una griglia di celle da 20 mm, così
  i fogli con migliaia di lati restano veloci.
- Con la casella "Linee di taglio comuni (laser)" la distanza fra le sagome è solo quella scritta
  (le abbondanze non la allargano) e la grafica si ritaglia a metà distanza: con distanza 0 i pezzi
  si toccano e la grafica arriva alla fustella. Nel PDF le fustelle dei pezzi non si disegnano più
  (`paths::hide` sulla copia della pagina, l'operatore che dipinge diventa `n`) e sopra ogni foglio
  ci sono le linee comuni, aperte, nella tinta della prima fustella dei file (o quella di taglio),
  in sovrastampa, con il suo spessore.
- Il motore ora tiene fermi i lati dritti delle sagome con curve: prima, se la semplificazione a
  0,3 mm toglieva punti, tutta la sagoma si allargava di 0,3 mm e due rettangoli ad angoli tondi
  restavano a 0,7 mm. Ora ogni pezzetto rimasto fuori fra una curva e la sua corda si copre con il
  triangolo fra la corda e le tangenti (o un rettangolo, se non è convesso) e si controlla che la
  sagoma del calcolo contenga davvero quella vera; se no si torna all'allargamento di prima.
- Le fustelle dentro i moduli della pagina (rare) restano: si toglie solo quella del contenuto
  della pagina.

