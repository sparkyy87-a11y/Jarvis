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
    locate_in(&roots, spoken)
}

/// Wie [`locate`], aber in selbst gewählten Ordnern (z. B. auf dem Handy).
pub fn locate_in(roots: &[PathBuf], spoken: &str) -> Result<PathBuf> {
    let direct = expand_home(spoken);
    if direct.is_file() {
        return Ok(direct);
    }

    let wanted_name = spoken.trim().to_lowercase();
    let wanted_stem = squash(spoken);
    let all = walk(roots, 4, &|_| true);
    let stem_of = |p: &PathBuf| {
        p.file_stem()
            .map(|n| squash(&n.to_string_lossy()))
            .unwrap_or_default()
    };
    let name_of = |p: &PathBuf| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    };

    // Exakt (Name oder Name ohne Endung), sonst unscharf – Spracherkennung
    // schreibt „Bericht" gern als „Berichte".
    let tolerance = (wanted_stem.chars().count() / 4).max(1);
    let best = all
        .iter()
        .filter_map(|p| {
            let distance = if name_of(p) == wanted_name {
                0
            } else {
                levenshtein(&stem_of(p), &wanted_stem)
            };
            (distance <= tolerance).then_some((distance, p))
        })
        .min_by_key(|(d, p)| {
            let age = p
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (*d, std::cmp::Reverse(age))
        })
        .map(|(_, p)| p.clone());
    match best {
        Some(p) => Ok(p),
        None => bail!("Ich finde keine Datei „{spoken}“ in Dokumente, Downloads, Desktop, Bilder, Musik oder Videos."),
    }
}

/// Anzahl Einfüge-, Lösch- und Ersetzungsschritte zwischen zwei Wörtern.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != *cb))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
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

/// Größte Datei, die Jarvis annimmt (schützt vor vollgeschriebener Festplatte).
pub const MAX_RECEIVE_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// Kennzeichnet eine empfangene Datei wie einen Download aus dem Internet
/// („Mark of the Web"): Windows warnt dann vor dem Ausführen (SmartScreen).
pub fn mark_as_downloaded(path: &Path) {
    #[cfg(windows)]
    {
        let mut ads = path.as_os_str().to_owned();
        ads.push(":Zone.Identifier");
        let _ = std::fs::write(
            ads,
            "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=jarvis://\r\n",
        );
    }
    #[cfg(not(windows))]
    let _ = path;
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
    fn fuzzy_locate() {
        let home = tempfile::tempdir().unwrap();
        let docs = home.path().join("Documents");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("Bericht.pdf"), "x").unwrap();
        std::fs::write(docs.join("Rechnung.pdf"), "x").unwrap();
        let roots = [docs.clone()];
        assert_eq!(
            locate_in(&roots, "Bericht").unwrap(),
            docs.join("Bericht.pdf")
        );
        assert_eq!(
            locate_in(&roots, "Berichte").unwrap(),
            docs.join("Bericht.pdf")
        );
        assert_eq!(
            locate_in(&roots, "bericht.pdf").unwrap(),
            docs.join("Bericht.pdf")
        );
        assert!(locate_in(&roots, "Urlaub").is_err());
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
