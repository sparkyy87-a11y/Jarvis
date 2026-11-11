//! Führt Aktionen auf *diesem* Gerät aus.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{anyhow, bail, Result};

/// Vergleichsform für App-Namen: klein, ohne Leer-/Sonderzeichen.
pub fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Öffnet eine App. Eigene Einträge in `[apps]` haben Vorrang.
pub fn open_app(app: &str, custom: &BTreeMap<String, String>) -> Result<String> {
    if let Some(cmd) = custom
        .iter()
        .find(|(k, _)| squash(k) == squash(app))
        .map(|(_, v)| v)
    {
        spawn_shell(cmd)?;
        return Ok(format!("{app} wird gestartet."));
    }
    platform_open(app)?;
    Ok(format!("{app} wird gestartet."))
}

#[cfg(target_os = "windows")]
fn platform_open(app: &str) -> Result<()> {
    // 1. Schnell: Verknüpfungen im Startmenü durchsuchen.
    let mut roots = vec![];
    if let Some(p) = std::env::var_os("ProgramData") {
        roots.push(PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Some(p) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Some(dir) = dirs::desktop_dir() {
        roots.push(dir);
    }
    let candidates = walk(&roots, 4, &|p| matches!(ext(p).as_str(), "lnk" | "url"));
    if let Some(best) = best_match(app, &candidates) {
        std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(best)
            .spawn()?;
        return Ok(());
    }
    // 2. Store-Apps (z. B. Minecraft for Windows) kennt nur Get-StartApps.
    let needle = app.replace('\'', "''");
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!(
            "Get-StartApps | Where-Object {{ $_.Name -like '*{needle}*' }} | Sort-Object {{ $_.Name.Length }} | Select-Object -First 1 -ExpandProperty AppID"
        ))
        .output()?;
    let app_id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if app_id.is_empty() {
        bail!("Ich finde keine App namens „{app}“.");
    }
    std::process::Command::new("explorer.exe")
        .arg(format!("shell:AppsFolder\\{app_id}"))
        .spawn()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn platform_open(app: &str) -> Result<()> {
    let status = std::process::Command::new("open")
        .args(["-a", app])
        .status()?;
    if !status.success() {
        bail!("Ich finde keine App namens „{app}“.");
    }
    Ok(())
}

#[cfg(all(
    unix,
    not(target_os = "macos"),
    not(target_os = "android"),
    not(target_os = "ios")
))]
fn platform_open(app: &str) -> Result<()> {
    let mut roots = vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
    ];
    if let Some(d) = dirs::data_dir() {
        roots.push(d.join("applications"));
    }
    let desktop_files = walk(&roots, 2, &|p| ext(p) == "desktop");
    // .desktop-Dateien nach ihrem Anzeigenamen (Name=) vergleichen.
    let named: Vec<(String, PathBuf)> = desktop_files
        .into_iter()
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            let name = text
                .lines()
                .find_map(|l| l.strip_prefix("Name="))?
                .to_string();
            Some((name, p))
        })
        .collect();
    let names: Vec<PathBuf> = named.iter().map(|(n, _)| PathBuf::from(n)).collect();
    if let Some(best) = best_match(app, &names) {
        let (_, file) = named
            .iter()
            .find(|(n, _)| Path::new(n) == best)
            .expect("Treffer stammt aus der Liste");
        let id = file
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or_default();
        std::process::Command::new("gtk-launch")
            .arg(id)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        return Ok(());
    }
    bail!("Ich finde keine App namens „{app}“.")
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn platform_open(app: &str) -> Result<()> {
    // Auf Mobilgeräten übernimmt die native App das Öffnen (Intent / URL-Schema).
    bail!("„{app}“ muss die Jarvis-App auf diesem Gerät öffnen.")
}

/// Beendet alle Prozesse, deren Name zur App passt.
#[cfg(not(feature = "desktop"))]
pub fn close_app(app: &str) -> Result<String> {
    bail!("{app} kann ich auf diesem Gerät nicht schließen.")
}

/// Beendet alle Prozesse, deren Name zur App passt.
#[cfg(feature = "desktop")]
pub fn close_app(app: &str) -> Result<String> {
    use sysinfo::System;
    let wanted = squash(app);
    if wanted.is_empty() {
        bail!("Welche App soll ich schließen?");
    }
    let mut sys = System::new();
    sys.refresh_processes();
    let me = std::process::id();
    let mut killed = 0;
    for (pid, p) in sys.processes() {
        let name = squash(p.name().trim_end_matches(".exe"));
        let matches = name.contains(&wanted) || (name.len() >= 4 && wanted.contains(&name));
        if pid.as_u32() != me && !name.is_empty() && matches && p.kill() {
            killed += 1;
        }
    }
    if killed == 0 {
        bail!("{app} läuft gerade nicht.");
    }
    Ok(format!("{app} wurde beendet."))
}

/// Befehle für die Systemstimme, in Reihenfolge der Vorliebe.
fn say_commands(text: &str) -> Vec<std::process::Command> {
    if cfg!(target_os = "windows") {
        // Text über eine Umgebungsvariable übergeben, damit nichts als Code ausgeführt wird.
        let mut c = std::process::Command::new("powershell");
        c.args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg("Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).Speak($env:JARVIS_SAY)")
            .env("JARVIS_SAY", text);
        vec![c]
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("say");
        c.arg("--").arg(text);
        vec![c]
    } else {
        let mut spd = std::process::Command::new("spd-say");
        spd.args(["--wait", "--"]).arg(text);
        let mut espeak = std::process::Command::new("espeak-ng");
        espeak.args(["-v", "de", "--"]).arg(text);
        vec![spd, espeak]
    }
}

/// Liest Text mit der Systemstimme vor und wartet, bis er gesprochen ist.
pub fn say_blocking(text: &str) -> Result<()> {
    for mut cmd in say_commands(text) {
        match crate::autostart::quiet(&mut cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(s) if s.success() => return Ok(()),
            Ok(_) => bail!("Sprachausgabe fehlgeschlagen"),
            Err(_) => continue, // Programm nicht installiert -> nächstes probieren
        }
    }
    bail!("Keine Sprachausgabe verfügbar")
}

/// Wie [`say_blocking`], ohne den async-Laufzeitkern zu blockieren.
pub async fn say(text: &str) -> Result<()> {
    let text = text.to_string();
    tokio::task::spawn_blocking(move || say_blocking(&text)).await?
}

fn spawn_shell(cmd: &str) -> Result<()> {
    let mut c = if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", cmd]);
        c
    } else {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", cmd]);
        c
    };
    crate::autostart::quiet(&mut c)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow!("Start fehlgeschlagen: {e}"))?;
    Ok(())
}

// Auf macOS öffnet `open -a` Apps direkt; dort wird die Endung nicht gebraucht.
#[cfg_attr(
    any(target_os = "macos", target_os = "ios", target_os = "android"),
    allow(dead_code)
)]
fn ext(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase()
}

/// Rekursive Dateisuche bis `depth` Ebenen, versteckte Ordner werden übersprungen.
pub fn walk(roots: &[PathBuf], depth: usize, keep: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack: Vec<(PathBuf, usize)> = roots.iter().map(|r| (r.clone(), 0)).collect();
    while let Some((dir, d)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            let hidden = e.file_name().to_string_lossy().starts_with('.');
            match e.file_type() {
                Ok(t) if t.is_dir() && !hidden && d < depth => stack.push((path, d + 1)),
                Ok(t) if t.is_file() && keep(&path) => out.push(path),
                _ => {}
            }
        }
    }
    out
}

/// Bester Treffer nach Dateiname (ohne Endung): exakt > beginnt mit > enthält,
/// bei Gleichstand der kürzeste Name.
pub fn best_match(wanted: &str, candidates: &[PathBuf]) -> Option<PathBuf> {
    let w = squash(wanted);
    if w.is_empty() {
        return None;
    }
    candidates
        .iter()
        .filter_map(|p| {
            let stem = squash(&p.file_stem()?.to_string_lossy());
            let rank = if stem == w {
                0
            } else if stem.starts_with(&w) {
                1
            } else if stem.contains(&w) {
                2
            } else {
                return None;
            };
            Some((rank, stem.len(), p))
        })
        .min_by_key(|(rank, len, _)| (*rank, *len))
        .map(|(_, _, p)| p.clone())
}

/// Öffnet eine Adresse im Standardbrowser bzw. der zuständigen App.
pub fn open_url(url: &str) -> Result<()> {
    let mut cmd = if cfg!(target_os = "windows") {
        // rundll32 statt `cmd /C start`: dort würde `&` in der Adresse als Befehlstrenner gelten.
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    crate::autostart::quiet(&mut cmd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow!("Browser startet nicht: {e}"))?;
    Ok(())
}

/// Musik abspielen. Apple Music auf dem Mac spielt Treffer aus der Mediathek
/// direkt; sonst öffnet Jarvis die Suche des Dienstes.
pub fn play_music(query: &str, service: &str) -> Result<String> {
    use crate::web;
    let name = web::service_name(service);
    match service {
        "spotify" => open_url(&web::spotify(query))?,
        "youtube-music" => open_url(&web::youtube_music(query))?,
        _ => {
            #[cfg(target_os = "macos")]
            if let Some(track) = macos_play_from_library(query) {
                return Ok(format!("Ich spiele {track}."));
            }
            if cfg!(target_os = "macos") {
                open_url(&web::apple_music_app(query))?;
            } else {
                open_url(&web::apple_music_web(query))?;
            }
        }
    }
    Ok(format!("{name} zeigt „{query}“ – zum Abspielen auswählen."))
}

/// Sucht in der Mediathek der Musik-App und spielt den ersten Treffer.
/// Der Suchbegriff wird als Argument übergeben, nie in das Skript eingesetzt.
#[cfg(target_os = "macos")]
fn macos_play_from_library(query: &str) -> Option<String> {
    let (title, artist) = match query.split_once(" von ") {
        Some((t, a)) => (t.trim().to_string(), a.trim().to_string()),
        None => (query.trim().to_string(), String::new()),
    };
    let script = r#"on run argv
  set t to item 1 of argv
  set a to item 2 of argv
  tell application "Music"
    if a is "" then
      set hits to (every track of library playlist 1 whose name contains t or artist contains t)
    else
      set hits to (every track of library playlist 1 whose name contains t and artist contains a)
    end if
    if hits is {} then return ""
    play item 1 of hits
    return (name of item 1 of hits) & " von " & (artist of item 1 of hits)
  end tell
end run"#;
    let out = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .arg(title)
        .arg(artist)
        .output()
        .ok()?;
    let track = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !track.is_empty()).then_some(track)
}

/// Wiedergabe steuern – wirkt auf den Player, der gerade läuft.
pub fn media(command: jarvis_protocol::MediaCommand, service: &str) -> Result<String> {
    use jarvis_protocol::MediaCommand::*;
    let done = match command {
        Play => "Weiter geht's.",
        Pause => "Pausiert.",
        Next => "Nächster Titel.",
        Previous => "Vorheriger Titel.",
    };
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            keybd_event, KEYEVENTF_KEYUP, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE,
            VK_MEDIA_PREV_TRACK,
        };
        // Windows kennt nur eine Umschalttaste für Abspielen/Pause.
        let key = match command {
            Play | Pause => VK_MEDIA_PLAY_PAUSE,
            Next => VK_MEDIA_NEXT_TRACK,
            Previous => VK_MEDIA_PREV_TRACK,
        } as u8;
        // SAFETY: keybd_event hat keine Zeiger-Argumente; simuliert eine Medientaste.
        unsafe {
            keybd_event(key, 0, 0, 0);
            keybd_event(key, 0, KEYEVENTF_KEYUP, 0);
        }
        let _ = service;
        Ok(done.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        let app = if service == "spotify" {
            "Spotify"
        } else {
            "Music"
        };
        let verb = match command {
            Play => "play",
            Pause => "pause",
            Next => "next track",
            Previous => "previous track",
        };
        let status = std::process::Command::new("osascript")
            .arg("-e")
            .arg(format!("tell application \"{app}\" to {verb}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !status.success() {
            bail!("{app} reagiert nicht.");
        }
        Ok(done.to_string())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = service;
        let verb = match command {
            Play => "play",
            Pause => "pause",
            Next => "next",
            Previous => "previous",
        };
        let status = std::process::Command::new("playerctl")
            .arg(verb)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| anyhow!("Für die Musiksteuerung bitte „playerctl“ installieren."))?;
        if !status.success() {
            bail!("Gerade spielt keine Musik.");
        }
        Ok(done.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching() {
        let c: Vec<PathBuf> = [
            "Minecraft Launcher.lnk",
            "Minecraft Education.lnk",
            "Steam.lnk",
            "Minecraft.lnk",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(
            best_match("minecraft", &c),
            Some(PathBuf::from("Minecraft.lnk"))
        );
        assert_eq!(
            best_match("Minecraft Launcher", &c),
            Some(PathBuf::from("Minecraft Launcher.lnk"))
        );
        assert_eq!(best_match("steam", &c), Some(PathBuf::from("Steam.lnk")));
        assert_eq!(best_match("discord", &c), None);
    }
}
