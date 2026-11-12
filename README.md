# Mira

Ein KI-Assistent, der alle eigenen Geräte als *ein* System behandelt: Du sprichst
mit dem Gerät, das gerade vor dir steht, und Mira führt den Befehl dort aus, wo
er hingehört.

```text
am PC:    „Öffne Minecraft"                              → startet Minecraft auf dem PC
am iPad:  „Sende Datei a an den PC in Ordner Dokumente"  → a.txt landet in PC\Dokumente
am Handy: „Beende Minecraft am PC"                       → beendet Minecraft auf dem PC
```

Geschrieben in Rust: ein Dienst pro Gerät (`mirad`), schnelle lokale
Befehlserkennung und ein Sprachmodell (kostenlos lokal oder Claude) nur dann, wenn die
Regeln einen Satz nicht verstehen.

## Stand

| Funktion | Status |
|---|---|
| Geräte finden sich im LAN (mDNS) oder über feste Adressen | ✅ |
| Verschlüsselt, eigener Schlüssel pro Gerät, Kopplung per Einmal-Code, Geräte sperrbar | ✅ |
| Befehle verstehen, Deutsch und Englisch, ohne Netz (< 1 ms) | ✅ |
| Freie Sätze und Fragen über ein lokales Modell (Ollama) – kostenlos | ✅ |
| … oder über Claude (bessere Antworten, API kostet) | ✅ optional |
| Apps öffnen und schließen: Windows, macOS, Linux | ✅ |
| Dateien senden, mit Prüfsumme, in bekannte Ordner | ✅ |
| Sprachausgabe über die Systemstimme | ✅ |
| Spracheingabe: „Mira, …" + Whisper lokal, optional auf der Grafikkarte | ✅ |
| Startet automatisch mit Windows/macOS/Linux, ohne Fenster | ✅ |
| HUD: leuchtender Ring, der sich beim Sprechen mit Mira einblendet | ✅ |
| Google-Suche und Musik (Apple Music, Spotify, YouTube Music) | ✅ |
| Computer Use (Claude bedient den Bildschirm) | Phase 3 |
| Apps für Android, iPhone und iPad (Mikrofon-Taste, Dateien, Kopplung per QR-Code) | ✅ |
| Zugriff von unterwegs (Relay) | Phase 5 |

## Installation

**Windows (PC, Laptop):** `Mira-Setup-….exe` aus dem neuesten Release bzw.
*Actions › Release › Artifacts* starten. Der Installer
- braucht keine Adminrechte und keine weiteren Programme,
- fragt nach dem Gerätenamen und ob es das erste Gerät ist oder gekoppelt wird,
- installiert auf Wunsch das kostenlose Sprachmodell (Ollama) – nur auf *einem*
  Gerät nötig, die anderen fragen dieses –,
- lädt die Spracherkennung vorab, gibt auf Wunsch die Firewall **nur fürs
  Heimnetz** frei und startet Mira bei jeder Anmeldung.

Reihenfolge: zuerst den PC („erstes Gerät"), danach **„Mira, verbinde dich mit
einem neuen Gerät"** sagen (oder Startmenü › Mira-Einstellungen › Geräte) – das
zeigt einen QR-Code mit Einmal-Code für das nächste Gerät:
Laptop (Code in den Installer einfügen), Handy und iPads (QR-Code mit der Kamera
scannen). Für jedes weitere Gerät einen neuen Code holen.

Windows zeigt beim ersten Start des Installers eventuell „Unbekannter
Herausgeber" (SmartScreen), weil er nicht mit einem kostenpflichtigen
Zertifikat signiert ist: *Weitere Informationen › Trotzdem ausführen*.

**Android:** `Mira-Android-….apk` aus dem Release öffnen. **iPhone/iPad:** siehe
[Handy und Tablet](#handy-und-tablet).

## Selbst bauen

```bash
git clone https://github.com/sparkyy87-a11y/Mira.git && cd Mira
cargo build --release          # Programme liegen danach in target/release/

# Erstes Gerät = Hauptgerät (das Gerät, das DU benutzt, z. B. dein Laptop)
mirad init --name Laptop
mirad autostart an           # Dienst starten (und bei jeder Anmeldung)
mirad pair                   # QR-Code + Einmal-Code für das nächste Gerät

# Weiterer PC: den Befehl aus `mirad pair` ausführen
mirad join "mira://pair?…" --name PC
mirad autostart an
# Handy/Tablet: QR-Code mit der Kamera scannen, Namen eingeben, bestätigen

mirad geraete                # alle gekoppelten Geräte mit Rolle und Rechten
mirad entfernen Handy        # verlorenes Gerät sperren            (nur Hauptgerät)
mirad rechte PC --steuern nein --dateien nein                   # (nur Hauptgerät)
mirad rechte iPhone --hauptgeraet ja   # iPhone darf auch den Laptop steuern
mirad hauptgeraet iPad       # Hauptrolle abgeben                  (nur Hauptgerät)

# Befehle geben (auf jedem Gerät)
mira geräte
mira öffne Minecraft
mira sende Datei a an den Laptop in Ordner Dokumente
mira                         # interaktiver Modus
```

## Kostenlos: lokales Sprachmodell

Die eingebauten Befehle (öffnen, schließen, Datei senden, „sag …") brauchen gar
kein Sprachmodell. Für freie Sätze („Mach mir Musik zum Arbeiten an") nutzt
Mira standardmäßig ein **lokales Modell über [Ollama](https://ollama.com)** –
kostenlos, ohne Konto, die Sprache verlässt den Rechner nicht:

```bash
# Ollama installieren (Windows/macOS: Installer von ollama.com), dann einmalig:
ollama pull qwen2.5:3b         # ca. 2 GB, läuft auch ohne Grafikkarte
```

Ollama muss nur auf *einem* Gerät laufen, z. B. dem PC; andere Geräte zeigen mit
`ollama_url` darauf. Mit Grafikkarte ist ein größeres Modell (`qwen2.5:7b`) klüger.

**Claude statt lokalem Modell:** Das kostenlose Claude-Abo auf claude.ai gilt nur
für die Chat-Oberfläche; Programme wie Mira können es nicht verwenden. Wer
Claude nutzen will, braucht einen API-Schlüssel von console.anthropic.com
(Abrechnung nach Verbrauch) und setzt ihn als `ANTHROPIC_API_KEY`. Ist der
Schlüssel gesetzt, nimmt Mira automatisch Claude.

**Windows:** Beim ersten Start fragt die Firewall nach Port 7879. Für das Heimnetz
(privates Netzwerk) erlauben.

## Suchen und Musik

```text
„Mira, google Wetter Berlin"            → Google-Suche im Browser
„Such nach Minecraft Mods am PC"          → … im Browser des PCs
„Spiel Thunderstruck von AC/DC"            → Musikdienst (Standard: Apple Music)
„Spiel Musik von Adele auf Spotify"        → bestimmter Dienst
„Pause" · „Weiter" · „Nächstes Lied" · „Stopp die Musik am PC"
```

Was „spiel …" genau tut, hängt davon ab, was das jeweilige System erlaubt:

| Gerät | Apple Music | Spotify |
|---|---|---|
| iPhone/iPad | spielt Treffer aus deiner Mediathek sofort, sonst öffnet sich die Suche in der Musik-App | öffnet die Suche |
| Android | spielt den besten Treffer direkt („Spiele per Suche") | spielt den besten Treffer direkt |
| Mac | spielt Treffer aus der Mediathek sofort, sonst Suche in der Musik-App | öffnet die Suche |
| Windows/Linux | öffnet die Suche auf music.apple.com | öffnet die Suche in der Spotify-App |

Pause/Weiter/Nächstes Lied wirken als Medientasten auf den Player, der gerade
läuft (Windows, Android), bzw. auf die Musik-App (Mac, iPhone). Linux braucht
`playerctl`. Den Standarddienst stellst du ein mit:

```toml
[music]
service = "apple-music"   # oder "spotify", "youtube-music"
```

## HUD

<img src="docs/hud.gif" width="300" align="right" alt="Mira-HUD">

`mira-hud` blendet sich ein, sobald du mit Mira sprichst – ein leuchtender
Ring mit Skalen und Segmenten, der

- **mit deiner Stimme pulsiert**, solange Mira zuhört („HÖRE ZU"),
- beim Nachdenken schneller rotiert („VERARBEITE"),
- die Antwort Zeichen für Zeichen einblendet und beim Vorlesen mitschwingt,
- bei Problemen bernsteinfarben wird,
- auch Befehle und Dateien von anderen Geräten zeigt („Datei von iPhone").

Danach blendet es sich aus. Das Fenster ist randlos, durchklickbar und immer
im Vordergrund; es startet mit `mirad autostart an` automatisch mit.
`mira-hud --demo` zeigt eine Vorführung ohne laufenden Mira.

```toml
[hud]
enabled = true
position = "rechts-unten"   # links-unten, rechts-oben, links-oben, mitte
```

Eigener Entwurf im Stil eines Holo-Displays – keine Grafiken oder Logos aus Filmen.

## Einstellungen

**Mira-Einstellungen** (Startmenü oder `mira-einstellungen`) – alles ohne
Konfigurationsdatei:

- **Übersicht:** Gerätename, weitere Namen, Autostart, Neustart
- **Geräte:** gekoppelte Geräte mit Online-Status, *Neues Gerät koppeln* mit
  QR-Code direkt im Fenster, *Sperren* für verlorene Geräte, feste Adressen
- **Sprache:** Spracheingabe an/aus, Mikrofon, Sprache, Genauigkeit der
  Erkennung, Weckwort, Vorlesen
- **Sprachmodell:** Ollama (zeigt, ob es läuft und welche Modelle da sind) oder
  Claude mit API-Schlüssel
- **Musik & Apps:** Musikdienst, eigene Kurzbefehle („minecraft" → Programmpfad)
- **HUD:** an/aus, Position, Vorschau

„Speichern & übernehmen" startet Mira mit den neuen Einstellungen neu.

## Automatisch starten

```bash
mirad autostart an       # jetzt starten und bei jeder Anmeldung, ohne Fenster
mirad autostart status
mirad autostart aus      # austragen und beenden
```

Dahinter steckt `mira-hintergrund`: Es startet `mirad` und `mira-listen`
unsichtbar, startet sie nach einem Absturz neu und schreibt Logs nach
`%LOCALAPPDATA%\mira\logs` (macOS/Linux: `~/.local/share/mira/logs` bzw.
`~/Library/Application Support/mira/logs`). Eingetragen wird unter Windows
der „Ausführen"-Schlüssel des Benutzers – keine Adminrechte nötig. Soll nur
`mirad` ohne Spracheingabe starten: `enabled = false` unter `[voice]`.

## Handy und Tablet

Die Apps für **Android** (`mobile/android`) und **iPhone/iPad** (`mobile/ios`)
nutzen denselben Rust-Kern wie der PC – gleiche Verschlüsselung, gleiche
Befehle, gleiche Dateiübertragung.

**Koppeln:** Am PC `mirad pair` ausführen und den QR-Code mit der normalen
Kamera-App scannen. Mira öffnet sich und ist gekoppelt.

**Was geht:**
- Der Mira-Ring ist die Mikrofon-Taste: antippen, sprechen – er pulsiert mit
  deiner Stimme, dreht beim Nachdenken schneller und schwingt beim Vorlesen
- Sprachbefehle: „Öffne Minecraft am PC", „Schick die Datei Bericht an mein Handy"
  – erkannt von der Spracherkennung des Handys (auf dem Gerät, kostenlos)
- Freie Sätze deutet das Sprachmodell auf dem PC (Ollama) – das Handy fragt es einfach
- Dateien senden: Büroklammer in der App, auf Android auch „Teilen → Mira"
- Dateien empfangen: Android mit Benachrichtigung; iPhone in der Dateien-App
  unter „Auf meinem iPhone › Mira"
- „Öffne Spotify am Handy": Android startet jede installierte App; iOS nur
  Apps mit bekanntem Link (Spotify, YouTube, WhatsApp, Netflix, Karten, …)

**Ohne PC:** Die Geräte merken sich gegenseitig ihre Adressen. Ist der PC aus,
verbinden sich z. B. iPad und Android-Handy direkt – alle festen Befehle gehen
weiter („öffne …", „schick die Datei …", Musik, Suche). Freie Sätze brauchen das
Sprachmodell und damit den PC (oder Claude mit API-Schlüssel). Voraussetzung:
die Geräte waren einmal gleichzeitig mit dem PC verbunden, und auf dem iPad ist
die App offen.

**Grenzen der Systeme:** Ein Dauer-Weckwort („Mira" ohne Taste) erlauben
Android und iOS fremden Apps nicht stromsparend – auf dem Handy gibt es deshalb
die Mikrofon-Taste. Android bleibt über eine dezente Dauer-Benachrichtigung für
den PC erreichbar; iOS hält Apps im Hintergrund an, auf dem iPhone ist Mira
also erreichbar, solange die App offen ist.

### Android installieren

Die fertige APK baut GitHub bei jedem Push: **Actions › Android › neuester Lauf ›
Artifacts › mira-android-apk**. Auf dem Handy öffnen und „Installation aus
unbekannten Quellen" für den Browser/Dateimanager erlauben.

Selbst bauen: Android SDK + NDK, `cargo install cargo-ndk`, dann
`cd mobile/android && ./gradlew assembleRelease`.

### iPhone/iPad installieren

Apple lässt Apps außerhalb des App Stores nur signiert auf Geräte. Wege:
- **Mit Mac:** `mobile/ios/build-rust.sh`, dann `cd mobile/ios && xcodegen`,
  `Mira.xcodeproj` öffnen, unter *Signing* die eigene Apple-ID wählen und aufs
  iPhone spielen. Mit kostenloser Apple-ID läuft die App 7 Tage, dann neu aufspielen.
- **Ohne Mac (Windows):** Die unsignierte `.ipa` aus **Actions › iOS › Artifacts**
  mit [Sideloadly](https://sideloadly.io) oder AltStore und der eigenen Apple-ID
  aufs iPhone bringen (ebenfalls 7 Tage gültig, danach erneut).
- Dauerhaft und bequem (TestFlight) geht es mit einem Apple-Developer-Konto (99 €/Jahr).

## Spracheingabe

`mira-listen` hört über das Mikrofon zu und reagiert auf „Mira":

```text
„Mira, öffne Minecraft."                    → sofort ausgeführt
„Mira."  –  „Ja?"  –  „Sende Datei a an …"   → Befehl nach kurzer Pause
„Wie war dein Tag?"                          → ignoriert (kein Weckwort)
```

Alles läuft lokal: Silero erkennt, *wann* gesprochen wird, Whisper schreibt mit,
danach geht der Text an `mirad` wie bei der Tastatureingabe. Beim ersten Start
werden die Modelle geladen (einmalig).

| Gerät | Modell (Standard) | Erkennungszeit pro Befehl |
|---|---|---|
| mit NVIDIA-Grafikkarte (CUDA) | `large-v3-turbo-q5_0` (~550 MB) | Bruchteile einer Sekunde, sehr genau |
| nur Prozessor | `small-q5_1` (~190 MB) | ca. 0,4–1 s auf einem 4-Kern-Rechner |

```bash
mira-listen                  # zuhören
mira-listen --mikrofone      # Mikrofone anzeigen, dann in der Config wählen
mira-listen --datei test.wav --trocken   # Aufnahme testen, nichts ausführen
```

### Windows mit NVIDIA-Grafikkarte

Mit Grafikkarte ist die Erkennung schneller und genauer. Auf einer GTX 1080 Ti
laufen Whisper (`large-v3-turbo`) und das lokale Sprachmodell (`qwen2.5:7b`)
gleichzeitig im Grafikspeicher (11 GB).

1. Visual Studio Build Tools mit „Desktopentwicklung mit C++"
2. `winget install Kitware.CMake LLVM.LLVM`
3. **CUDA Toolkit 12.x** – nicht 13, das unterstützt GTX-10xx-Karten nicht mehr
4. Neues Terminal, dann im Mira-Ordner:

```powershell
.\scripts\build-windows.ps1     # baut mit CUDA für GTX 10xx; -CudaArch 86 für RTX 30xx
ollama pull qwen2.5:7b          # klügeres lokales Modell, passt auf die 1080 Ti
```

In der Config dann `model = "qwen2.5:7b"` unter `[brain]` eintragen.

## Konfiguration

`%APPDATA%\mira\config.toml` (Windows), `~/Library/Application Support/mira/config.toml`
(macOS), `~/.config/mira/config.toml` (Linux):

```toml
[device]
name = "Gaming-PC"
aliases = ["großer rechner"]      # zusätzliche Namen: „öffne Steam am großen Rechner"

[mesh]
port = 7879                        # Verbindungen zwischen Geräten
control_port = 7878                # lokale Steuerung (nur 127.0.0.1)
static_peers = ["192.168.1.20:7879"]  # falls mDNS im Netz blockiert ist
mdns = true

[apps]                             # eigene Kurzbefehle, haben Vorrang
minecraft = 'C:\XboxGames\Minecraft Launcher\Content\Minecraft.exe'

[voice]
enabled = true                     # beim Autostart mitstarten
language = "de"
wake_words = ["mira"]
# input_device = "USB"             # Teil des Mikrofonnamens
# model = "large-v3-turbo"         # Whisper-Modell
speak_replies = true
follow_up_seconds = 8              # so lange wartet Mira nach „Mira." auf den Befehl

[brain]
provider = "auto"                  # auto | ollama | claude | off
# model = "qwen2.5:7b"             # Standard: qwen2.5:3b bzw. claude-opus-5-5
# ollama_url = "http://192.168.1.20:11434"
```

## Architektur

```text
                    ┌──────────────── mirad (pro Gerät) ────────────────┐
 mira-listen ─►  │ control ──► mira-intent ──(nicht erkannt)──► brain │ ──► Ollama/Claude
 (127.0.0.1:7878)   │                 │                                    │
                    │                 ▼                                    │
                    │            node::run ── hier? ──► exec / files       │
                    │                 │                                    │
                    │                 └─ anderes Gerät ──► transport ──────┼──► mirad (PC, iPad …)
                    └──────────────────────────────────────────────────────┘
                                         ▲ discovery (mDNS)
```

| Crate | Aufgabe |
|---|---|
| `mira-protocol` | Gemeinsame Typen (`Action`, `Command`, `Frame`), Binärformat (postcard) |
| `mira-intent` | Regelbasierte Befehlserkennung DE/EN, inkl. Zielgerät („am PC", „aufs Handy") |
| `mira-brain` | Sprachmodell für freie Sätze: Ollama (lokal) oder Claude, festes JSON-Schema |
| `mira-node` | `mirad` und `mira`: Mesh, Routing, Ausführung, Dateiübertragung |
| `mira-voice` | `mira-listen`: Mikrofon, Silero-VAD, Whisper, Weckwort |
| `mira-hud` | Overlay (egui): zeigt Zuhören, Nachdenken, Antworten |
| `mira-mobile` | Kern für die Handy-Apps (UniFFI → Kotlin/Swift) |
| `mobile/android` | Android-App (Kotlin, Jetpack Compose) |
| `mobile/ios` | iPhone/iPad-App (SwiftUI) |

**Warum so schnell:** Die häufigen Befehle erkennt `mira-intent` per Regex in
Mikrosekunden, ohne Netz. Die Geräte halten eine dauerhafte, verschlüsselte
TCP-Verbindung offen; ein Befehl an den PC ist ein einziges kleines Paket. Das
Sprachmodell wird nur für Sätze gefragt, die die Regeln nicht kennen.

**Wie ein Zielgerät gefunden wird:** Erst exakter Name oder Alias, dann ein
Gerät, dessen Name das Wort enthält („am PC" → „Gaming-PC", „am iPad" → „iPad
Büro"), zuletzt der Gattungsname („PC", „iPad", „Handy"). Passen zwei Geräte,
fragt Mira nach, statt zu raten – bei zwei iPads also am besten „iPad
Wohnzimmer" und „iPad Büro" nennen.

## Sicherheit

**Hauptgerät und Rechte**
- Das zuerst eingerichtete Gerät ist das **Hauptgerät**. Nur dort werden neue
  Geräte gekoppelt, Geräte gesperrt, Rechte vergeben und die Hauptrolle
  weitergegeben (`mirad hauptgeraet <Gerät>` oder Einstellungen › Geräte).
- Die Geräteliste mit allen Rechten ist vom Hauptgerät **digital signiert**
  (Ed25519). Jedes Gerät merkt sich beim Koppeln den Signaturschlüssel aus dem
  QR-Code und übernimmt nur Listen, die damit unterschrieben und neuer sind.
  Ein anderes Gerät kann sich also keine Rechte geben – auch nicht, indem
  jemand an dessen Dateien herumändert.
- Rechte je Gerät: *andere Geräte steuern*, *das Hauptgerät steuern* (standard-
  mäßig aus – für deine eigenen Geräte einschalten), *Dateien schicken*.
  Geprüft wird auf dem Gerät, das den Befehl bekommt.
- Grenze: Auf seinem eigenen Rechner hat dessen Benutzer weiter die volle
  Kontrolle (er kann Mira dort beenden oder deinstallieren). Andere Geräte
  steuern kann er nur, soweit das Hauptgerät es erlaubt.

**Geräte und Kopplung**
- Jedes Gerät hat ein **eigenes Schlüsselpaar** (X25519). Verbindungen laufen über
  Noise `XX` mit ChaCha20-Poly1305: beide Seiten beweisen ihren Schlüssel, alles
  ist verschlüsselt, alte Mitschnitte bleiben auch später geheim (Forward Secrecy).
- Es gibt **kein gemeinsames Passwort**. Ein neues Gerät kommt nur mit einem
  **Einmal-Code** hinein (`mirad pair`, 10 Minuten gültig, nur einmal nutzbar).
  Der QR-Code enthält außerdem den Fingerabdruck des PCs – ein Gerät, das sich
  dazwischenschalten will, fällt auf.
- Ein `mira://`-Link koppelt nie still: die App zeigt „Mit „Gaming-PC“ koppeln"
  und wartet auf deine Bestätigung.
- **Verlorenes Gerät:** `mirad entfernen Handy` sperrt es sofort; die Sperre
  verbreitet sich an alle deine Geräte, es kommt nicht wieder hinein.
- Fremde Mira-Geräte im selben WLAN werden ignoriert; im Netz wird kein
  Gerätename angekündigt.

**Auf dem Gerät**
- Die lokale Steuerung (`127.0.0.1:7878`) verlangt ein **Zugangs-Token**, das nur
  dein Benutzerkonto lesen kann – Webseiten im Browser und andere Benutzer am
  Rechner können Mira nicht steuern.
- Konfiguration, Geräteschlüssel und Vertrauensliste sind nur für dich lesbar
  (Unix 0600, Windows: Benutzerprofil).

**Dateien**
- Empfangene Dateinamen werden auf den reinen Namen reduziert (kein `../`),
  eigene Zielordner liegen immer unterhalb des Home-Verzeichnisses.
- Dateien landen erst als `.mira-part` und werden nur nach korrekter SHA-256
  umbenannt; vorhandene Dateien werden nie überschrieben; höchstens 8 GB.
- Unter Windows tragen empfangene Dateien die Internet-Markierung: SmartScreen
  warnt, bevor eine empfangene `.exe` startet.

**Bewusste Grenzen**
- Ein gekoppeltes Gerät darf – soweit das Hauptgerät es erlaubt – auf den
  anderen Apps öffnen, Musik steuern und Dateien in deine Ordner legen.
  Koppel nur Geräte, denen du traust, und schränke die übrigen ein.
- Mira lauscht im WLAN auf Port 7879. Windows fragt beim ersten Start nach der
  Firewall: nur **„Private Netzwerke"** erlauben.
- Auf dem Handy liegt der Geräteschlüssel im privaten App-Speicher (geschützt
  durch die Sandbox des Systems), noch nicht im Schlüsselbund/Keystore.

## Fahrplan

**Phase 2 – Sprache (erledigt).** Offen: eine natürlichere Stimme für die
Antworten und Whisper-Halluzinationen weiter zähmen.

**Phase 3 – Computer Use.** Für Aufgaben ohne festen Befehl („trag den Termin in
meinen Kalender ein"): Bildschirmfotos (`xcap`), Maus/Tastatur (`enigo`) und
Claude mit dem Computer-Use-Toolset (`computer_toolset_20260801`) in einer
Schleife – mit Rückfrage vor riskanten Schritten.

**Phase 4 – Mobile Apps (erledigt).** Offen: „Teilen → Mira" auch auf iOS
(Share-Extension), Siri-Kurzbefehl „Hey Siri, Mira …".

**Phase 5 – Unterwegs.** Ein kleiner Relay-Server (oder Tailscale/WireGuard),
damit „schick die Datei an den PC" auch aus dem Mobilfunknetz geht.

## Entwicklung

```bash
cargo test                 # Unit-Tests
cargo clippy --all-targets
./scripts/e2e.sh           # zwei Geräte lokal: Fernstart + Dateiübertragung
./scripts/voice-e2e.sh     # dasselbe per Sprache (braucht espeak-ng, ffmpeg)
cd mobile/android && ./gradlew :bindings-test:test   # Kotlin-Anbindung gegen echten mirad
```
