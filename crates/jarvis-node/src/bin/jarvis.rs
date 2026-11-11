//! `jarvis` – Befehle an den lokalen Jarvis-Dienst schicken.
//!
//!   jarvis öffne Minecraft
//!   jarvis sende Datei a an den PC in Ordner Dokumente
//!   jarvis geräte
//!   jarvis            (interaktiv)

use std::io::{BufRead, Write};

use anyhow::{Context, Result};
use jarvis_node::config::{Config, DEFAULT_CONTROL_PORT};
use jarvis_node::control::{self, Request, Response};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::load().ok();
    let port = cfg
        .as_ref()
        .map(|c| c.mesh.control_port)
        .unwrap_or(DEFAULT_CONTROL_PORT);
    let token_path = cfg
        .map(|c| c.control_token_path())
        .context("Jarvis ist nicht eingerichtet – zuerst `jarvisd init`")?;
    let token = control::read_token(&token_path).context("Zugangs-Token fehlt – läuft jarvisd?")?;
    let stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .context("jarvisd läuft nicht – zuerst `jarvisd` starten")?;
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    let mut auth = serde_json::to_string(&Request::Auth { token })?;
    auth.push('\n');
    w.write_all(auth.as_bytes()).await?;
    let answer: Response = serde_json::from_str(
        &lines
            .next_line()
            .await?
            .context("jarvisd hat die Verbindung beendet")?,
    )?;
    anyhow::ensure!(answer.ok, "Zugang verweigert");

    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        return ask(&mut w, &mut lines, &args.join(" ")).await;
    }

    println!("Jarvis – sag mir, was ich tun soll (leere Zeile beendet).");
    let stdin = std::io::stdin();
    loop {
        print!("> ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 || line.trim().is_empty() {
            return Ok(());
        }
        ask(&mut w, &mut lines, line.trim()).await?;
    }
}

async fn ask(
    w: &mut tokio::net::tcp::OwnedWriteHalf,
    lines: &mut tokio::io::Lines<BufReader<tokio::net::tcp::OwnedReadHalf>>,
    input: &str,
) -> Result<()> {
    let req = match input.to_lowercase().as_str() {
        "geräte" | "geraete" | "devices" => Request::Devices,
        _ => Request::Text {
            text: input.to_string(),
            source: Some("Tastatur".into()),
        },
    };
    let mut line = serde_json::to_string(&req)?;
    line.push('\n');
    w.write_all(line.as_bytes()).await?;
    let answer = lines
        .next_line()
        .await?
        .context("jarvisd hat die Verbindung beendet")?;
    let resp: Response = serde_json::from_str(&answer)?;
    if let Some(devices) = resp.devices {
        for d in devices {
            let marker = if d.this_device {
                " (dieses Gerät)"
            } else {
                ""
            };
            println!(
                "• {} – {}{}{}",
                d.name,
                d.platform,
                marker,
                d.address.map(|a| format!(" @ {a}")).unwrap_or_default()
            );
        }
    }
    if let Some(text) = resp.text {
        println!("{}{text}", if resp.ok { "" } else { "⚠ " });
    }
    Ok(())
}
