//! Jarvis beim Anmelden automatisch starten – ohne Konsolenfenster.
//!
//! `jarvis-hintergrund` überwacht `jarvisd` und `jarvis-listen` und startet
//! sie nach einem Absturz neu. Eingetragen wird er
//! - Windows: Registry `HKCU\…\Run` (keine Adminrechte nötig)
//! - macOS: LaunchAgent in `~/Library/LaunchAgents`
//! - Linux: XDG-Autostart in `~/.config/autostart`

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// Lokaler Port, über den genau eine Instanz von `jarvis-hintergrund` läuft
/// und über den sie sich beenden lässt.
pub const SUPERVISOR_PORT: u16 = 7877;
pub const SUPERVISOR_EXE: &str = "jarvis-hintergrund";

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// Programm, das neben dem laufenden liegt (gleicher Ordner).
pub fn sibling_exe(name: &str) -> Result<PathBuf> {
    let me = std::env::current_exe().context("eigener Programmpfad unbekannt")?;
    let dir = me.parent().context("Programmordner unbekannt")?;
    Ok(dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)))
}

pub fn logs_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jarvis")
        .join("logs")
}

/// Unter Windows kein Konsolenfenster für Hilfsprogramme aufpoppen lassen.
pub fn quiet(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[derive(Debug)]
pub struct Status {
    pub enabled: bool,
    pub running: bool,
    pub logs: PathBuf,
}

pub fn status() -> Result<Status> {
    Ok(Status {
        enabled: is_enabled()?,
        running: is_running(),
        logs: logs_dir(),
    })
}

pub fn is_running() -> bool {
    TcpStream::connect_timeout(
        &([127, 0, 0, 1], SUPERVISOR_PORT).into(),
        Duration::from_millis(300),
    )
    .is_ok()
}

/// Trägt Jarvis in den Autostart ein und startet ihn sofort.
pub fn enable() -> Result<()> {
    let exe = sibling_exe(SUPERVISOR_EXE)?;
    if !exe.is_file() {
        bail!(
            "{} fehlt – liegt es im selben Ordner wie jarvisd?",
            exe.display()
        );
    }
    register(&exe)?;
    if !is_running() {
        let mut cmd = std::process::Command::new(&exe);
        quiet(&mut cmd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| format!("{} startet nicht", exe.display()))?;
        wait_until(true);
    }
    Ok(())
}

/// Startet den laufenden Jarvis neu (z. B. nach geänderten Einstellungen).
/// `false`, wenn er nicht über den Autostart lief.
pub fn restart() -> Result<bool> {
    if !is_running() {
        return Ok(false);
    }
    if stop_running() {
        wait_until(false);
    }
    let exe = sibling_exe(SUPERVISOR_EXE)?;
    let mut cmd = std::process::Command::new(&exe);
    quiet(&mut cmd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("{} startet nicht", exe.display()))?;
    wait_until(true);
    Ok(true)
}

/// Entfernt den Autostart-Eintrag und beendet den laufenden Jarvis.
pub fn disable() -> Result<()> {
    unregister()?;
    if stop_running() {
        wait_until(false);
    }
    Ok(())
}

/// Wartet bis zu 3 s, bis `jarvis-hintergrund` läuft bzw. beendet ist.
fn wait_until(running: bool) {
    for _ in 0..30 {
        if is_running() == running {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Bittet eine laufende Instanz von `jarvis-hintergrund`, sich zu beenden.
pub fn stop_running() -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(
        &([127, 0, 0, 1], SUPERVISOR_PORT).into(),
        Duration::from_millis(300),
    ) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    if s.write_all(b"stop\n").is_err() {
        return false;
    }
    let mut answer = String::new();
    let _ = BufReader::new(s).read_line(&mut answer);
    answer.trim() == "ok"
}

#[cfg(windows)]
fn register(exe: &std::path::Path) -> Result<()> {
    let value = format!("\"{}\"", exe.display());
    let status = quiet(
        std::process::Command::new("reg")
            .args(["add", RUN_KEY, "/v", "Jarvis", "/t", "REG_SZ", "/d"]),
    )
    .arg(value)
    .arg("/f")
    .stdout(std::process::Stdio::null())
    .status()?;
    if !status.success() {
        bail!("Autostart-Eintrag konnte nicht gesetzt werden");
    }
    Ok(())
}

#[cfg(windows)]
fn unregister() -> Result<()> {
    if is_enabled()? {
        let status = quiet(
            std::process::Command::new("reg").args(["delete", RUN_KEY, "/v", "Jarvis", "/f"]),
        )
        .stdout(std::process::Stdio::null())
        .status()?;
        if !status.success() {
            bail!("Autostart-Eintrag konnte nicht entfernt werden");
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_enabled() -> Result<bool> {
    Ok(
        quiet(std::process::Command::new("reg").args(["query", RUN_KEY, "/v", "Jarvis"]))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?
            .success(),
    )
}

#[cfg(not(windows))]
fn entry_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Home-Verzeichnis unbekannt")?;
    Ok(if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents/de.jarvis.hintergrund.plist")
    } else {
        dirs::config_dir()
            .unwrap_or(home.join(".config"))
            .join("autostart/jarvis.desktop")
    })
}

#[cfg(not(windows))]
fn register(exe: &std::path::Path) -> Result<()> {
    let path = entry_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let exe = exe.display().to_string();
    let content = if cfg!(target_os = "macos") {
        let xml = exe
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\"><dict>\n\
             <key>Label</key><string>de.jarvis.hintergrund</string>\n\
             <key>ProgramArguments</key><array><string>{xml}</string></array>\n\
             <key>RunAtLoad</key><true/>\n\
             </dict></plist>\n"
        )
    } else {
        let quoted = exe.replace('\\', "\\\\").replace('"', "\\\"");
        format!(
            "[Desktop Entry]\nType=Application\nName=Jarvis\nComment=Sprachassistent im Hintergrund\n\
             Exec=\"{quoted}\"\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n"
        )
    };
    std::fs::write(&path, content)?;
    Ok(())
}

#[cfg(not(windows))]
fn unregister() -> Result<()> {
    let path = entry_path()?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn is_enabled() -> Result<bool> {
    Ok(entry_path()?.exists())
}
