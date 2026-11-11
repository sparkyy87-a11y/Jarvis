# Jarvis

Ein KI-Assistent, der alle eigenen Geräte als *ein* System behandelt: Du sprichst
mit dem Gerät, das gerade vor dir steht, und Jarvis führt den Befehl dort aus, wo
er hingehört.

```text
am PC:    „Öffne Minecraft"                              → startet Minecraft auf dem PC
am iPad:  „Sende Datei a an den PC in Ordner Dokumente"  → a.txt landet in PC\Dokumente
am Handy: „Beende Minecraft am PC"                       → beendet Minecraft auf dem PC
```

Geschrieben in Rust: ein Dienst pro Gerät (`jarvisd`), schnelle lokale
Befehlserkennung und ein Sprachmodell (kostenlos lokal oder Claude) nur dann, wenn die
Regeln einen Satz nicht verstehen.

## Stand

| Funktion | Status |
|---|---|
| Geräte finden sich im LAN (mDNS) oder über feste Adressen | ✅ |
| Verschlüsselte Verbindung, nur eigene Geräte (Noise + Pairing-Schlüssel) | ✅ |
| Befehle verstehen, Deutsch und Englisch, ohne Netz (< 1 ms) | ✅ |
| Freie Sätze und Fragen über ein lokales Modell (Ollama) – kostenlos | ✅ |
| … oder über Claude (bessere Antworten, API kostet) | ✅ optional |
| Apps öffnen und schließen: Windows, macOS, Linux | ✅ |
| Dateien senden, mit Prüfsumme, in bekannte Ordner | ✅ |
| Sprachausgabe über die Systemstimme | ✅ |
| Spracheingabe: „Jarvis, …" + Whisper lokal, optional auf der Grafikkarte | ✅ |
| Startet automatisch mit Windows/macOS/Linux, ohne Fenster | ✅ |
| Computer Use (Claude bedient den Bildschirm) | Phase 3 |
| Apps für iOS, iPadOS, Android | Phase 4 |
| Zugriff von unterwegs (Relay) | Phase 5 |

## Schnellstart

```bash
git clone https://github.com/sparkyy87-a11y/Jarvis.git && cd Jarvis
cargo build --release          # Programme liegen danach in target/release/

# Erstes Gerät (z. B. der PC)
jarvisd init --name Gaming-PC  # gibt den Pairing-Schlüssel aus
jarvisd                        # Dienst starten

# Jedes weitere Gerät
jarvisd join <schlüssel> --name Laptop
jarvisd

# Befehle geben (auf jedem Gerät)
jarvis geräte
jarvis öffne Minecraft
jarvis sende Datei a an den Laptop in Ordner Dokumente
jarvis                         # interaktiver Modus
```

## Kostenlos: lokales Sprachmodell

Die eingebauten Befehle (öffnen, schließen, Datei senden, „sag …") brauchen gar
kein Sprachmodell. Für freie Sätze („Mach mir Musik zum Arbeiten an") nutzt
Jarvis standardmäßig ein **lokales Modell über [Ollama](https://ollama.com)** –
kostenlos, ohne Konto, die Sprache verlässt den Rechner nicht:

```bash
# Ollama installieren (Windows/macOS: Installer von ollama.com), dann einmalig:
ollama pull qwen2.5:3b         # ca. 2 GB, läuft auch ohne Grafikkarte
```

Ollama muss nur auf *einem* Gerät laufen, z. B. dem PC; andere Geräte zeigen mit
`ollama_url` darauf. Mit Grafikkarte ist ein größeres Modell (`qwen2.5:7b`) klüger.

**Claude statt lokalem Modell:** Das kostenlose Claude-Abo auf claude.ai gilt nur
für die Chat-Oberfläche; Programme wie Jarvis können es nicht verwenden. Wer
Claude nutzen will, braucht einen API-Schlüssel von console.anthropic.com
(Abrechnung nach Verbrauch) und setzt ihn als `ANTHROPIC_API_KEY`. Ist der
Schlüssel gesetzt, nimmt Jarvis automatisch Claude.

**Windows:** Beim ersten Start fragt die Firewall nach Port 7879. Für das Heimnetz
(privates Netzwerk) erlauben.

## Automatisch starten

```bash
jarvisd autostart an       # jetzt starten und bei jeder Anmeldung, ohne Fenster
jarvisd autostart status
jarvisd autostart aus      # austragen und beenden
```

Dahinter steckt `jarvis-hintergrund`: Es startet `jarvisd` und `jarvis-listen`
unsichtbar, startet sie nach einem Absturz neu und schreibt Logs nach
`%LOCALAPPDATA%\jarvis\logs` (macOS/Linux: `~/.local/share/jarvis/logs` bzw.
`~/Library/Application Support/jarvis/logs`). Eingetragen wird unter Windows
der „Ausführen"-Schlüssel des Benutzers – keine Adminrechte nötig. Soll nur
`jarvisd` ohne Spracheingabe starten: `enabled = false` unter `[voice]`.

## Spracheingabe

`jarvis-listen` hört über das Mikrofon zu und reagiert auf „Jarvis":

```text
„Jarvis, öffne Minecraft."                    → sofort ausgeführt
„Jarvis."  –  „Ja?"  –  „Sende Datei a an …"   → Befehl nach kurzer Pause
„Wie war dein Tag?"                          → ignoriert (kein Weckwort)
```

Alles läuft lokal: Silero erkennt, *wann* gesprochen wird, Whisper schreibt mit,
danach geht der Text an `jarvisd` wie bei der Tastatureingabe. Beim ersten Start
werden die Modelle geladen (einmalig).

| Gerät | Modell (Standard) | Erkennungszeit pro Befehl |
|---|---|---|
| mit NVIDIA-Grafikkarte (CUDA) | `large-v3-turbo-q5_0` (~550 MB) | Bruchteile einer Sekunde, sehr genau |
| nur Prozessor | `small-q5_1` (~190 MB) | ca. 0,4–1 s auf einem 4-Kern-Rechner |

```bash
jarvis-listen                  # zuhören
jarvis-listen --mikrofone      # Mikrofone anzeigen, dann in der Config wählen
jarvis-listen --datei test.wav --trocken   # Aufnahme testen, nichts ausführen
```

### Windows mit NVIDIA-Grafikkarte

Mit Grafikkarte ist die Erkennung schneller und genauer. Auf einer GTX 1080 Ti
laufen Whisper (`large-v3-turbo`) und das lokale Sprachmodell (`qwen2.5:7b`)
gleichzeitig im Grafikspeicher (11 GB).

1. Visual Studio Build Tools mit „Desktopentwicklung mit C++"
2. `winget install Kitware.CMake LLVM.LLVM`
3. **CUDA Toolkit 12.x** – nicht 13, das unterstützt GTX-10xx-Karten nicht mehr
4. Neues Terminal, dann im Jarvis-Ordner:

```powershell
.\scripts\build-windows.ps1     # baut mit CUDA für GTX 10xx; -CudaArch 86 für RTX 30xx
ollama pull qwen2.5:7b          # klügeres lokales Modell, passt auf die 1080 Ti
```

In der Config dann `model = "qwen2.5:7b"` unter `[brain]` eintragen.

## Konfiguration

`%APPDATA%\jarvis\config.toml` (Windows), `~/Library/Application Support/jarvis/config.toml`
(macOS), `~/.config/jarvis/config.toml` (Linux):

```toml
[device]
name = "Gaming-PC"
aliases = ["großer rechner"]      # zusätzliche Namen: „öffne Steam am großen Rechner"

[mesh]
port = 7879                        # Verbindungen zwischen Geräten
control_port = 7878                # lokale Steuerung (nur 127.0.0.1)
pairing_key = "…"                  # auf allen eigenen Geräten gleich
static_peers = ["192.168.1.20:7879"]  # falls mDNS im Netz blockiert ist
mdns = true

[apps]                             # eigene Kurzbefehle, haben Vorrang
minecraft = 'C:\XboxGames\Minecraft Launcher\Content\Minecraft.exe'

[voice]
enabled = true                     # beim Autostart mitstarten
language = "de"
wake_words = ["jarvis"]
# input_device = "USB"             # Teil des Mikrofonnamens
# model = "large-v3-turbo"         # Whisper-Modell
speak_replies = true
follow_up_seconds = 8              # so lange wartet Jarvis nach „Jarvis." auf den Befehl

[brain]
provider = "auto"                  # auto | ollama | claude | off
# model = "qwen2.5:7b"             # Standard: qwen2.5:3b bzw. claude-opus-5-5
# ollama_url = "http://192.168.1.20:11434"
```

## Architektur

```text
                    ┌──────────────── jarvisd (pro Gerät) ────────────────┐
 jarvis-listen ─►  │ control ──► jarvis-intent ──(nicht erkannt)──► brain │ ──► Ollama/Claude
 (127.0.0.1:7878)   │                 │                                    │
                    │                 ▼                                    │
                    │            node::run ── hier? ──► exec / files       │
                    │                 │                                    │
                    │                 └─ anderes Gerät ──► transport ──────┼──► jarvisd (PC, iPad …)
                    └──────────────────────────────────────────────────────┘
                                         ▲ discovery (mDNS)
```

| Crate | Aufgabe |
|---|---|
| `jarvis-protocol` | Gemeinsame Typen (`Action`, `Command`, `Frame`), Binärformat (postcard) |
| `jarvis-intent` | Regelbasierte Befehlserkennung DE/EN, inkl. Zielgerät („am PC", „aufs Handy") |
| `jarvis-brain` | Sprachmodell für freie Sätze: Ollama (lokal) oder Claude, festes JSON-Schema |
| `jarvis-node` | `jarvisd` und `jarvis`: Mesh, Routing, Ausführung, Dateiübertragung |
| `jarvis-voice` | `jarvis-listen`: Mikrofon, Silero-VAD, Whisper, Weckwort |

**Warum so schnell:** Die häufigen Befehle erkennt `jarvis-intent` per Regex in
Mikrosekunden, ohne Netz. Die Geräte halten eine dauerhafte, verschlüsselte
TCP-Verbindung offen; ein Befehl an den PC ist ein einziges kleines Paket. Das
Sprachmodell wird nur für Sätze gefragt, die die Regeln nicht kennen.

**Wie ein Zielgerät gefunden wird:** Erst exakter Name oder Alias, dann
Gattungsname („PC", „iPad", „Handy"). Passen zwei Geräte, fragt Jarvis nach, statt zu raten.

## Sicherheit

- Geräte verbinden sich nur mit dem gemeinsamen Pairing-Schlüssel
  (Noise `NNpsk0`, ChaCha20-Poly1305). Wer den Schlüssel nicht hat, kommt nicht
  durch den Handshake; alle Daten sind verschlüsselt.
- Empfangene Dateinamen werden auf den reinen Namen reduziert (kein `../`),
  eigene Zielordner müssen unterhalb des Home-Verzeichnisses liegen.
- Dateien landen erst als `.jarvis-part` und werden nur nach korrekter SHA-256
  umbenannt; vorhandene Dateien werden nie überschrieben.
- Die Steuerung (`7878`) lauscht nur auf `127.0.0.1`.
- Ein Gerät mit dem Schlüssel darf auf allen anderen Apps starten. Den Schlüssel
  wie ein Passwort behandeln.

## Fahrplan

**Phase 2 – Sprache (erledigt).** Offen: eine natürlichere Stimme für die
Antworten und Whisper-Halluzinationen weiter zähmen.

**Phase 3 – Computer Use.** Für Aufgaben ohne festen Befehl („trag den Termin in
meinen Kalender ein"): Bildschirmfotos (`xcap`), Maus/Tastatur (`enigo`) und
Claude mit dem Computer-Use-Toolset (`computer_toolset_20260801`) in einer
Schleife – mit Rückfrage vor riskanten Schritten.

**Phase 4 – Mobile Apps.** Die Crates sind plattformneutral und werden per
UniFFI in eine Swift-App (iOS/iPadOS) und eine Kotlin-App (Android) eingebunden.
Einschränkungen, die man kennen muss:
- iOS/iPadOS erlauben keinen dauerhaften Hintergrunddienst; Jarvis ist dort
  aktiv, solange die App offen ist (oder über Siri-Kurzbefehle/App Intents).
- Andere Apps öffnen geht auf iOS nur über URL-Schemata und Kurzbefehle;
  Android kann Apps per Intent starten.

**Phase 5 – Unterwegs.** Ein kleiner Relay-Server (oder Tailscale/WireGuard),
damit „schick die Datei an den PC" auch aus dem Mobilfunknetz geht.

## Entwicklung

```bash
cargo test                 # Unit-Tests
cargo clippy --all-targets
./scripts/e2e.sh           # zwei Geräte lokal: Fernstart + Dateiübertragung
./scripts/voice-e2e.sh     # dasselbe per Sprache (braucht espeak-ng, ffmpeg)
```
