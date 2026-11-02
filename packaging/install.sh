#!/bin/sh
# Installs CreateCalculated: copies the programs, builds recipes and pictures on this computer
# and, when run as root, adds a system service. Game files are downloaded here, never shipped.
set -eu

usage="usage: install.sh [options]

  --prefix DIR     where to install, default /opt/createcalc
  --address ADDR   address to serve on, default 0.0.0.0:3000
  --from DIR       folder with the programs, default the folder of this script
  --no-service     do not add a system service
"

prefix=/opt/createcalc
address=0.0.0.0:3000
from=$(dirname "$0")
service=1
while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) prefix=$2; shift 2 ;;
        --address) address=$2; shift 2 ;;
        --from) from=$2; shift 2 ;;
        --no-service) service=0; shift ;;
        -h|--help) printf '%s' "$usage"; exit 0 ;;
        *) printf 'unknown argument %s\n%s' "$1" "$usage" >&2; exit 2 ;;
    esac
done

here=$(cd "$(dirname "$0")" && pwd)
unit=$here/createcalc.service
if [ "$service" = 1 ] && [ "$(id -u)" != 0 ]; then
    echo "Not running as root, so no system service will be added. Use sudo to get one, or --no-service to silence this."
    service=0
fi
if [ "$service" = 1 ] && ! command -v systemctl >/dev/null 2>&1; then
    echo "systemctl was not found, so no system service will be added."
    service=0
fi

mkdir -p "$prefix/data"
for program in createcalc-server createcalc-recipes createcalc-icons createcalc-import; do
    if [ ! -f "$from/$program" ]; then
        echo "$from/$program was not found" >&2
        exit 1
    fi
    # Through a new file, so a running server does not stop the copy.
    cp "$from/$program" "$prefix/$program.new"
    chmod 755 "$prefix/$program.new"
    mv -f "$prefix/$program.new" "$prefix/$program"
done
prefix=$(cd "$prefix" && pwd)

as_owner() {
    if [ "$service" = 1 ]; then
        runuser -u createcalc -- "$@"
    else
        "$@"
    fi
}

if [ "$service" = 1 ]; then
    if ! id createcalc >/dev/null 2>&1; then
        useradd --system --home-dir "$prefix" --shell /usr/sbin/nologin createcalc
    fi
    chown -R createcalc "$prefix/data"
fi

cd "$prefix"
if [ ! -f data/bundle.json ]; then
    echo "Downloading Minecraft, NeoForge and Create, about 60 MB, to build the recipes..."
    as_owner ./createcalc-recipes
fi
if [ ! -f data/icons/index.json ]; then
    echo "Building item pictures..."
    as_owner ./createcalc-icons
fi

if [ "$service" = 1 ]; then
    sed -e "s#@PREFIX@#$prefix#g" -e "s#@ADDRESS@#$address#g" "$unit" >/etc/systemd/system/createcalc.service
    systemctl daemon-reload
    systemctl enable createcalc >/dev/null
    systemctl restart createcalc
    echo "Installed in $prefix, the service createcalc serves http://$address/"
    echo "There is no login: keep it behind a firewall or a reverse proxy if it should not be public."
else
    echo "Installed in $prefix. Start it with: cd $prefix && ./createcalc-server data/bundle.json $address data/icons"
fi
