# 0040. Disegni DXF, DWG, PLT e SVG nelle Miniature con il lettore di Visualizza

**Stato:** accettata, 2026-11-16

## Contesto
Michel (2026-11-16): alcuni DXF si vedono in Visualizza ma non in anteprima. In Miniature i
disegni non erano proprio un tipo conosciuto: `FileKind::from_path` li dava «non supportati»,
quindi niente misure, niente miniatura, solo il riquadro grigio. Visualizza li leggeva con
`winfiles-cad` (ADR 0033, 0034) e le anteprime di Esplora file (ADR 0036) con una terza strada,
in 200 mm con margine e linee di 0,8 mm.

## Alternative
1. **Convertire il disegno in un PDF nascosto accanto al file** e trattarlo come un PDF: scrive
   nelle cartelle dei clienti, file da tenere allineati.
2. **Un `FileKind::Drawing` letto da `winfiles-cad` in memoria** (scelta): misure da
   `winfiles_formats::probe`, miniature da `winfiles_thumbs::render`, quindi anteprima, stampa,
   PDF esportato e anteprime di Esplora file passano tutti dalla stessa funzione
   (`winfiles_cad::paper`).
3. **Lasciare i disegni a Visualizza**: è proprio quello che Michel segnala come difetto.

## Decisione
- `FileKind::Drawing` («Disegno»): dxf, dwg, plt, hpgl, hpg, svg, svgz; pulsante «Disegni» nel
  filtro dei tipi.
- `winfiles_cad::paper(path, lato_px)`: pagina PDF bianca alla misura vera (unità dichiarate,
  altrimenti 1 unità = 1 mm come in Visualizza; oltre 2,54 m la pagina si riduce ma la misura
  resta vera). Le linee hanno uno spessore minimo di 1,2 pixel della miniatura (quelle
  sottilissime di Visualizza, rimpicciolite, diventano puntini grigi), e il margine di uno
  spessore tiene intere quelle sul bordo. Gli ultimi 6 disegni letti restano in memoria:
  misura e miniature a più grandezze non rileggono il file.
- Le anteprime di Esplora file usano `winfiles_thumbs::render` come le Miniature: un disegno si
  vede uguale nei due posti.
- Nel `.winfiles` un disegno si scrive come `"Unsupported"` e alla lettura il nome del file lo
  fa tornare `Drawing`: un collega con una versione vecchia apre comunque il progetto (con un
  tipo nuovo serde rifiuterebbe tutto il file).
- Visualizza resta com'è (layer, fondo scuro, unità a scelta): usa lo stesso lettore.

## Conseguenze
- I disegni hanno misure in mm e m², vanno in stampa e nel PDF come gli altri file.
- Il Nesting non li accetta ancora: servirebbero i loro tracciati come fustelle, è un lavoro a sé.
