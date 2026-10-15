# 0017. Foto RAW e file di Photoshop con le librerie di PhotoCraft

**Stato:** accettata, 2026-10-15

## Contesto
In tipografia arrivano anche foto RAW delle fotocamere (CR2, NEF, ARW, DNG…) e file di Photoshop
(PSD, a volte PSB per i formati grandi). La 4.9 non li apriva. Michel li ha chiesti per il modulo
"Modifica immagini", oltre ai TIFF che già si aprivano.

Due trappole: un RAW comincia come un TIFF qualsiasi (`II*` o `MM*`), e un lettore di TIFF ne
mostra solo la miniatura da 160 pixel che la fotocamera mette in testa; e un PSD in CMYK
convertito con la formula ingenua (`255 − inchiostro`) sbaglia i neri e i blu.

## Alternative
1. **LibRaw / dcraw e libpsd.** Sono i lettori più diffusi, ma LibRaw è LGPL/CDDL, dcraw GPL e
   vanno in C con `unsafe`: fuori dalle regole del progetto.
2. **Solo l'anteprima della fotocamera per i RAW, e per i PSD la miniatura incorporata.** Veloce,
   ma la miniatura dei PSD è di 160 pixel e alcune fotocamere (Sony) mettono un'anteprima piccola:
   non si può stampare né misurare.
3. **Scelta: PhotoCraft** (`photocraft-raw`, `photocraft-psd`, `photocraft-cms`), allo stesso
   commit già usato per le foto HEIC (ADR 0012). Rust puro senza `unsafe`, MIT/Apache, scritto
   sulle specifiche pubbliche. Un crate nostro, `crates/photo` (`winfiles-photo`), li usa e dà
   all'app un'immagine `image` con profilo, risoluzione e una riga che dice da dove viene.

## Decisioni collegate
- **I RAW si sviluppano dai dati del sensore** (16 bit, ProPhoto) e si portano in sRGB con il
  profilo, che si scrive nel file salvato. Ai RAW che non descrivono il proprio colore (tutti
  tranne i DNG) si applicano colori e curva ricavati dall'anteprima JPEG della fotocamera
  (`fit_look`), così la foto somiglia a quella vista sul display e non a un negativo spento.
  Dove lo sviluppo non c'è ancora (i CR3 di Canon) si usa l'anteprima a piena misura.
- **Per le miniature e per le misure dell'elenco basta l'anteprima della fotocamera**: si legge in
  un decimo di secondo, lo sviluppo in uno o due. Le sue misure possono differire di qualche
  pixel da quelle della foto sviluppata (i bordi del sensore); nel modulo immagini valgono quelle
  vere. La risoluzione è quella scritta in testa al file, come per i JPEG della stessa macchina.
- **I RAW si riconoscono dall'estensione** quando sono TIFF a tutti gli effetti (NEF, ARW, DNG,
  PEF) e dai primi byte quando hanno una firma loro (CR2, CR3, ORF, RW2, RAF). Nel modulo
  immagini decide il contenuto: `photocraft_raw::identify` prima di ogni lettore di TIFF.
- **Dei PSD si legge l'immagine unita** che Photoshop salva accanto ai livelli, con profilo ICC
  e risoluzione. Se manca (salvato senza "Massimizza compatibilità") si dice come rimediare.
  I livelli non si ricompongono: sarebbe rifare Photoshop.
- **I PSD in CMYK si convertono con il loro profilo** (o con un CMYK patinato generico se non ne
  hanno), e la riga dei dati dice quale. I Lab e i multicanale si rifiutano con un messaggio.
- **Il primo salvataggio chiede sempre il nome**: un RAW o un PSD non si sovrascrive mai. Si
  propone il JPEG per i RAW e il TIFF (senza perdite) per i PSD.
- **Le immagini si aprono in un thread a parte**: sviluppare un RAW richiede un paio di secondi e
  la finestra non deve bloccarsi; nel frattempo si vede la rotellina.
- I dati mostrano ora il nome del profilo colore ("Adobe RGB (1998)", "sRGB IEC61966-2.1") invece
  dei suoi kilobyte.

## Conseguenze
- Tre dipendenze nuove dallo stesso repository di PhotoCraft, già ammesso in `deny.toml`. Portano
  con sé `zune-jpeg` (tasselli JPEG dei DNG), `jxl-oxide` (DNG in JPEG XL) e `rayon` (sviluppo in
  parallelo): tutte MIT/Apache/Zlib, `cargo deny` verde.
- I CR3 si vedono dall'anteprima della fotocamera (8 bit) finché PhotoCraft non li svilupperà.
- Il salvataggio in CMYK resta fuori: si salva in RGB o in grigio (ADR 0016).
