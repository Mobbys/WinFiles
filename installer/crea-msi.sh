#!/bin/sh
# Crea l'installer MSI di WinFiles da un winfiles.exe già compilato per Windows (ADR 0014).
# Uso: installer/crea-msi.sh percorso/winfiles.exe uscita.msi [cartella-menu|-] [cartella-convertitori]
# La cartella-menu, se c'è, contiene winfiles_menu11.dll e WinFilesMenu.msix firmato: la voce del
# menu corto di Windows 11 (ADR 0022). La cartella-convertitori, se c'è, contiene i quattro
# convertitori dei documenti (wordcraft-cli.exe, gridcraft-cli.exe, deckcraft-cli.exe,
# designcraft-cli.exe, ADR 0035), che vanno nella sottocartella "convertitori".
# Serve wixl (pacchetto "wixl" di Debian e Ubuntu, da msitools). La versione è quella del
# workspace in Cargo.toml.
set -eu
exe=$(realpath "$1")
out=$2
dir=$(dirname "$0")
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$dir/../Cargo.toml" | head -n 1)
menu=""
if [ $# -ge 3 ] && [ -f "$3/WinFilesMenu.msix" ] && [ -f "$3/winfiles_menu11.dll" ]; then
    m=$(realpath "$3")
    menu="-D Menu=$m/WinFilesMenu.msix -D MenuDll=$m/winfiles_menu11.dll"
fi
conv=""
if [ $# -ge 4 ]; then
    c=$(realpath "$4")
    for n in wordcraft gridcraft deckcraft designcraft; do
        test -f "$c/$n-cli.exe" || { echo "Manca $c/$n-cli.exe"; exit 1; }
    done
    conv="-D Convertitori=$c"
fi
# shellcheck disable=SC2086
wixl -D Version="$version" -D Exe="$exe" -D Icon="$dir/../apps/winfiles/assets/icon/icona.ico" \
    $menu $conv -o "$out" "$dir/winfiles.wxs"
echo "$out: WinFiles $version${menu:+ con il menu di Windows 11}${conv:+ con i convertitori dei documenti}"
