#!/bin/sh
# Crea l'installer MSI di WinFiles da un winfiles.exe già compilato per Windows (ADR 0014).
# Uso: installer/crea-msi.sh percorso/winfiles.exe uscita.msi [cartella-menu]
# La cartella-menu, se c'è, contiene winfiles_menu11.dll e WinFilesMenu.msix firmato: la voce del
# menu corto di Windows 11 (ADR 0022).
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
# shellcheck disable=SC2086
wixl -D Version="$version" -D Exe="$exe" -D Icon="$dir/../apps/winfiles/assets/icon/icona.ico" \
    $menu -o "$out" "$dir/winfiles.wxs"
echo "$out: WinFiles $version${menu:+ con il menu di Windows 11}"
