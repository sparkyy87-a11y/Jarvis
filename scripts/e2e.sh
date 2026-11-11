#!/usr/bin/env bash
# Startet zwei Jarvis-Geräte ("Gaming-PC" und "iPad") auf diesem Rechner und
# prüft Fernsteuerung und Dateiübertragung von Ende zu Ende.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build -q
for port in 7878 7879 7888 7889; do
  if (exec 3<>/dev/tcp/127.0.0.1/$port) 2>/dev/null; then
    echo "Port $port ist belegt – läuft noch ein jarvisd? Bitte zuerst beenden." >&2; exit 2
  fi
done
B=$PWD/target/debug
W=$(mktemp -d)
export RUST_BACKTRACE=0 ANTHROPIC_API_KEY=
PIDS=()
trap 'kill "${PIDS[@]}" 2>/dev/null || true; wait 2>/dev/null || true; rm -rf "$W"' EXIT

pc()   { env HOME="$W/pc"   JARVIS_CONFIG="$W/pc/config.toml"   "$@"; }
ipad() { env HOME="$W/ipad" JARVIS_CONFIG="$W/ipad/config.toml" "$@"; }

pc   "$B/jarvisd" init --name Gaming-PC >/dev/null
ipad "$B/jarvisd" join "$(pc "$B/jarvisd" key)" --name iPad >/dev/null
sed -i 's/^mdns = true/mdns = false/' "$W/pc/config.toml"
sed -i "s|^\[apps\]|[apps]\nminecraft = \"echo gestartet > '$W/pc/minecraft.log'\"|" "$W/pc/config.toml"
sed -i 's/^port = 7879/port = 7889/; s/^control_port = 7878/control_port = 7888/; s/^mdns = true/mdns = false/;
        s/^static_peers = \[\]/static_peers = ["127.0.0.1:7879"]/; s/^\[device\]/[device]\nplatform = "IpadOs"/' "$W/ipad/config.toml"
mkdir -p "$W/ipad/Documents" && echo "Hallo vom iPad" > "$W/ipad/Documents/a.txt"
head -c 3000000 /dev/urandom > "$W/ipad/Documents/gross.bin"

# iPad zuerst: prüft, dass es den später startenden PC von selbst findet.
env HOME="$W/ipad" JARVIS_CONFIG="$W/ipad/config.toml" "$B/jarvisd" > "$W/ipad.log" 2>&1 & PIDS+=($!)
sleep 0.5
env HOME="$W/pc" JARVIS_CONFIG="$W/pc/config.toml" "$B/jarvisd" > "$W/pc.log" 2>&1 & PIDS+=($!)
for _ in $(seq 100); do ipad "$B/jarvis" geräte 2>/dev/null | grep -q Gaming-PC && break; sleep 0.1; done

fail=0
check() { if eval "$2"; then echo "✔ $1"; else echo "✘ $1"; fail=1; fi; }

ipad "$B/jarvis" geräte
ipad "$B/jarvis" öffne Minecraft am PC
sleep 0.3
check "Minecraft am PC gestartet" "grep -q gestartet '$W/pc/minecraft.log'"
ipad "$B/jarvis" sende Datei a an den PC in Ordner Dokumente
check "Datei a angekommen" "cmp -s '$W/ipad/Documents/a.txt' '$W/pc/Documents/a.txt'"
ipad "$B/jarvis" sende Datei a an den PC in Ordner Dokumente >/dev/null
check "zweite Kopie umbenannt" "test -f '$W/pc/Documents/a (1).txt'"
ipad "$B/jarvis" schick gross.bin an den PC
check "3 MB in Downloads, Prüfsumme ok" "cmp -s '$W/ipad/Documents/gross.bin' '$W/pc/Downloads/gross.bin'"
out=$(pc "$B/jarvis" öffne Steam am Handy || true); echo "$out"
check "unbekanntes Ziel wird gemeldet" "grep -q 'kein verbundenes Gerät' <<<\"\$out\""
out=$(pc "$B/jarvis" wie wird das Wetter || true); echo "$out"
check "ohne API-Key ehrliche Rückmeldung" "grep -q 'nicht verstanden' <<<\"\$out\""
exit $fail
