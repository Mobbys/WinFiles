#!/usr/bin/env bash
# Installa wixl (msitools) sul runner Ubuntu. Se i server di Ubuntu non rispondono (è successo:
# "Temporary failure resolving archive.ubuntu.com"), riprova con mirror di riserva.
set -u

installa() {
    sudo apt-get update -q -o Acquire::Retries=3 && sudo apt-get install -y -q -o Acquire::Retries=3 wixl
}

installa && exit 0

codename=$(. /etc/os-release && echo "$VERSION_CODENAME")
for mirror in \
    http://mirrors.edge.kernel.org/ubuntu \
    http://ftp.halifax.rwth-aachen.de/ubuntu \
    http://mirror.math.princeton.edu/pub/ubuntu; do
    echo "I server di Ubuntu non rispondono: riprovo con $mirror"
    sudo mkdir -p /etc/apt/sorgenti-messe-da-parte
    for f in /etc/apt/sources.list.d/*.sources /etc/apt/sources.list.d/*.list; do
        [ -e "$f" ] && sudo mv "$f" /etc/apt/sorgenti-messe-da-parte/
    done
    [ -s /etc/apt/sources.list ] && sudo mv /etc/apt/sources.list /etc/apt/sorgenti-messe-da-parte/sources.list
    echo "deb $mirror $codename main universe" | sudo tee /etc/apt/sources.list.d/riserva.list >/dev/null
    installa && exit 0
done
echo "wixl non si installa: nessun mirror di Ubuntu raggiungibile" >&2
exit 1
