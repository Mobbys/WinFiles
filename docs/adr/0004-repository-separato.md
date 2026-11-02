# 0004. Repository separato da WinFile 4.9

**Stato:** accettata, 2026-10-14

## Contesto
WinFile 4.9 controlla gli aggiornamenti sulle Releases del proprio repository (`winfile.py`, riga
98). Un rilascio della versione Rust pubblicato lì verrebbe proposto come aggiornamento a chi usa
ancora la 4.9.

## Decisione
Il codice Rust vive nel repository `Mobbys/WinFiles`. WinFile 4.9 resta in uso e riceve correzioni
nel suo repository fino al rilascio della nuova versione.

## Conseguenze
Nessuna interferenza con gli aggiornamenti della 4.9. Quando WinFiles la sostituirà, un ultimo
rilascio della 4.9 potrà indicare dove scaricare la nuova app.
