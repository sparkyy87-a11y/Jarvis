//! Lädt Whisper- und VAD-Modelle beim ersten Start herunter.

use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

pub const VAD_MODEL: &str = "silero-v5.1.2";

pub fn models_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jarvis")
        .join("models")
}

fn url_for(name: &str) -> String {
    if name.starts_with("silero") {
        format!("https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-{name}.bin")
    } else {
        format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{name}.bin")
    }
}

/// Pfad zum Modell; lädt es herunter, falls es noch fehlt. Ein vollständiger
/// Pfad auf eine vorhandene Datei wird direkt verwendet.
pub fn ensure(name: &str) -> Result<PathBuf> {
    let direct = PathBuf::from(name);
    if direct.is_file() {
        return Ok(direct);
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
    {
        bail!("Ungültiger Modellname „{name}“");
    }
    let dir = models_dir();
    let path = dir.join(format!("ggml-{name}.bin"));
    if path.is_file() {
        return Ok(path);
    }
    std::fs::create_dir_all(&dir)?;
    let url = url_for(name);
    eprintln!("Lade Modell {name} herunter (einmalig) …");
    let mut resp = reqwest::blocking::Client::builder()
        .timeout(None)
        .build()?
        .get(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("Download fehlgeschlagen: {url}"))?;
    let total = resp.content_length().unwrap_or(0);
    let part = path.with_extension("part");
    let mut file = std::fs::File::create(&part)?;
    let mut buf = vec![0u8; 1 << 20];
    let (mut done, mut shown) = (0u64, 0u64);
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        if total > 0 && done * 10 / total > shown {
            shown = done * 10 / total;
            eprintln!(
                "  {:>3} %  ({} von {} MB)",
                shown * 10,
                done >> 20,
                total >> 20
            );
        }
    }
    file.flush()?;
    if total > 0 && done != total {
        bail!("Download unvollständig ({done} von {total} Byte)");
    }
    std::fs::rename(&part, &path)?;
    Ok(path)
}
