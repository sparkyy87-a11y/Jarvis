//! Dateien auf diesem Gerät finden und eingehende Dateien sicher ablegen.

use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use anyhow::{bail, Result};
use jarvis_protocol::Folder;

use crate::exec::{squash, walk};

/// Sucht eine Datei anhand dessen, was der Nutzer gesagt hat.
///
/// Reihenfolge: existierender Pfad, dann exakter Dateiname, dann Name ohne
/// Endung in Dokumente/Downloads/Desktop/Bilder/…; bei mehreren Treffern die
/// zuletzt geänderte Datei.
pub fn locate(spoken: &str) -> Result<PathBuf> {
    let direct = expand_home(spoken);
    if direct.is_file() {
        return Ok(direct);
    }
    let mut roots: Vec<PathBuf> = [
        Folder::Documents,
        Folder::Downloads,
        Folder::Desktop,
        Folder::Pictures,
        Folder::Music,
        Folder::Videos,
    ]
    .iter()
    .filter_map(|f| resolve_folder(Some(f)).ok())
    .collect();
    roots.dedup();

    let wanted_name = spoken.trim().to_lowercase();
    let wanted_stem = squash(spoken);
    let found = walk(&roots, 4, &|p| {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let stem = p
            .file_stem()
            .map(|n| squash(&n.to_string_lossy()))
            .unwrap_or_default();
        name == wanted_name || stem == wanted_stem
    });
    let newest = found.into_iter().max_by_key(|p| {
        p.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    match newest {
        Some(p) => Ok(p),
        None => bail!("Ich finde keine Datei „{spoken}“ in Dokumente, Downloads, Desktop, Bilder, Musik oder Videos."),
    }
}

fn expand_home(s: &str) -> PathBuf {
    match (s.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(s),
    }
}

/// Zielordner auf diesem Gerät. Eigene Ordner liegen immer unterhalb des
/// Home-Verzeichnisses; `..` und absolute Pfade werden abgelehnt.
pub fn resolve_folder(folder: Option<&Folder>) -> Result<PathBuf> {
    let home = dirs::home_dir();
    let std_dir = |known: Option<PathBuf>, fallback: &str| {
        known.or_else(|| home.as_ref().map(|h| h.join(fallback)))
    };
    let dir = match folder {
        None | Some(Folder::Downloads) => std_dir(dirs::download_dir(), "Downloads"),
        Some(Folder::Documents) => std_dir(dirs::document_dir(), "Documents"),
        Some(Folder::Desktop) => std_dir(dirs::desktop_dir(), "Desktop"),
        Some(Folder::Pictures) => std_dir(dirs::picture_dir(), "Pictures"),
        Some(Folder::Music) => std_dir(dirs::audio_dir(), "Music"),
        Some(Folder::Videos) => std_dir(dirs::video_dir(), "Videos"),
        Some(Folder::Custom(rel)) => {
            let rel = Path::new(rel);
            if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
                bail!("Ungültiger Zielordner „{}“", rel.display());
            }
            home.as_ref().map(|h| h.join(rel))
        }
    };
    match dir {
        Some(d) => Ok(d),
        None => bail!("Zielordner gibt es auf diesem Gerät nicht"),
    }
}

/// Nur der reine Dateiname – verhindert, dass ein Sender Pfade wie
/// `../../autostart/x.exe` unterschiebt.
pub fn safe_file_name(name: &str) -> Result<String> {
    let base = Path::new(name.trim())
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let base = base.replace(['\\', '/', ':'], "_");
    if base.is_empty() || base == "." || base == ".." {
        bail!("Ungültiger Dateiname „{name}“");
    }
    Ok(base)
}

/// `bericht.pdf` -> `bericht (1).pdf`, falls die Datei schon existiert.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(name);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (1..)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .expect("unendlicher Iterator")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_sanitized() {
        assert_eq!(safe_file_name("../../etc/passwd").unwrap(), "passwd");
        assert_eq!(
            safe_file_name("C:\\Windows\\evil.exe").unwrap(),
            if cfg!(windows) {
                "evil.exe"
            } else {
                "C__Windows_evil.exe"
            }
        );
        assert!(safe_file_name("..").is_err());
        assert!(safe_file_name("").is_err());
    }

    #[test]
    fn custom_folder_must_stay_in_home() {
        assert!(resolve_folder(Some(&Folder::Custom("../x".into()))).is_err());
        assert!(resolve_folder(Some(&Folder::Custom("/etc".into()))).is_err());
    }

    #[test]
    fn unique_names() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique_path(dir.path(), "a.txt"), dir.path().join("a.txt"));
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        assert_eq!(
            unique_path(dir.path(), "a.txt"),
            dir.path().join("a (1).txt")
        );
    }
}
