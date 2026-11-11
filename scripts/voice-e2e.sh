#!/usr/bin/env bash
# Sprache von Ende zu Ende: synthetische deutsche Sprachaufnahme -> Silero-VAD
# -> Whisper -> Weckwort -> jarvisd auf zwei Geräten.
# Braucht espeak-ng und ffmpeg; lädt beim ersten Lauf ~200 MB Modelle.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build -q --release
B=$PWD/target/release
W=$(mktemp -d)
MODELS=${JARVIS_MODELS:-$HOME/.local/share/jarvis/models}
mkdir -p "$MODELS"
export RUST_BACKTRACE=0 ANTHROPIC_API_KEY=
PIDS=()
trap 'kill "${PIDS[@]}" 2>/dev/null || true; wait 2>/dev/null || true; rm -rf "$W"' EXIT

say() { espeak-ng -v de -s 150 -w "$W/$1.wav" "$2"; }
say a "Jarvis, öffne Minecraft am PC."
say b "Wie war dein Tag heute?"
say c "Jarvis."
say d "Sende die Datei Bericht an den PC in den Ordner Dokumente."
ffmpeg -loglevel error -f lavfi -i "anoisesrc=d=1.2:c=pink:a=0.01:r=22050" -ac 1 "$W/s.wav"
ffmpeg -loglevel error -i "$W/s.wav" -i "$W/a.wav" -i "$W/s.wav" -i "$W/b.wav" -i "$W/s.wav" -i "$W/c.wav" -i "$W/s.wav" -i "$W/d.wav" -i "$W/s.wav" \
  -filter_complex "concat=n=9:v=0:a=1,aresample=48000[a]" -map "[a]" -ac 2 "$W/aufnahme.wav"

for dev in pc ipad; do mkdir -p "$W/$dev/.local/share/jarvis"; ln -s "$MODELS" "$W/$dev/.local/share/jarvis/models"; done
pc()   { env HOME="$W/pc"   JARVIS_CONFIG="$W/pc/config.toml"   "$@"; }
ipad() { env HOME="$W/ipad" JARVIS_CONFIG="$W/ipad/config.toml" "$@"; }
pc "$B/jarvisd" init --name Gaming-PC >/dev/null
sed -i 's/^mdns = true/mdns = false/' "$W/pc/config.toml"
sed -i "s|^\[apps\]|[apps]\nminecraft = \"echo gestartet > '$W/pc/minecraft.log'\"|" "$W/pc/config.toml"
env HOME="$W/pc" JARVIS_CONFIG="$W/pc/config.toml" "$B/jarvisd" > "$W/pc.log" 2>&1 & PIDS+=($!)
for _ in $(seq 50); do test -f "$W/pc/control.token" && break; sleep 0.1; done
sleep 0.3
link=$(pc "$B/jarvisd" pair | grep -o 'jarvis://pair?[^"]*' | sed 's/&addr=[^&]*/\&addr=127.0.0.1%3A7879/')
ipad "$B/jarvisd" join "$link" --name iPad >/dev/null
sed -i 's/^port = 7879/port = 7889/; s/^control_port = 7878/control_port = 7888/; s/^mdns = true/mdns = false/' "$W/ipad/config.toml"
mkdir -p "$W/ipad/Documents" && echo "Quartalsbericht" > "$W/ipad/Documents/Bericht.pdf"
env HOME="$W/ipad" JARVIS_CONFIG="$W/ipad/config.toml" "$B/jarvisd" > "$W/ipad.log" 2>&1 & PIDS+=($!)
for _ in $(seq 100); do ipad "$B/jarvis" geräte 2>/dev/null | grep -q Gaming-PC && break; sleep 0.1; done

ipad "$B/jarvis-listen" --datei "$W/aufnahme.wav" --leise 2>&1 | sed 's/\x1b\[[0-9;]*m//g; s/^.*jarvis_listen: //' | tee "$W/listen.log"

fail=0
check() { if eval "$2"; then echo "✔ $1"; else echo "✘ $1"; fail=1; fi; }
check "„Jarvis, öffne Minecraft am PC“ startet Minecraft am PC" "grep -q gestartet '$W/pc/minecraft.log'"
check "Satz ohne Weckwort wird ignoriert" "! grep -q '» Wie' '$W/listen.log'"
check "„Jarvis.“ + Befehl schickt Bericht.pdf in PC/Dokumente" "cmp -s '$W/ipad/Documents/Bericht.pdf' '$W/pc/Documents/Bericht.pdf'"
exit $fail
