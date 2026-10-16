#!/bin/sh
# Crea l'installer MSI di WinFiles da un winfiles.exe già compilato per Windows (ADR 0014).
# Uso: installer/crea-msi.sh percorso/winfiles.exe uscita.msi
# Serve wixl (pacchetto "wixl" di Debian e Ubuntu, da msitools). La versione è quella del
# workspace in Cargo.toml.
set -eu
exe=$(realpath "$1")
out=$2
dir=$(dirname "$0")
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$dir/../Cargo.toml" | head -n 1)
wixl -D Version="$version" -D Exe="$exe" -D Icon="$dir/../apps/winfiles/assets/icon/icona.ico" \
    -o "$out" "$dir/winfiles.wxs"
echo "$out: WinFiles $version"
