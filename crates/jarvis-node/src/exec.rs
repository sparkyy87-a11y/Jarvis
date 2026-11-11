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
        match cmd.stdout(Stdio::null()).stderr(Stdio::null()).status() {
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
    c.stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow!("Start fehlgeschlagen: {e}"))?;
    Ok(())
}

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
