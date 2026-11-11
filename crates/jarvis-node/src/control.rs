//! Lokale Steuerschnittstelle (nur 127.0.0.1): eine JSON-Zeile rein, eine raus.
//!
//! Darüber sprechen CLI, Sprachmodul und HUD mit dem Dienst. **Jede Verbindung
//! muss sich zuerst mit dem Zugangs-Token ausweisen** (`control.token` neben der
//! Konfiguration, nur für den eigenen Benutzer lesbar). Damit kann weder eine
//! Webseite im Browser noch ein anderer Benutzer am Rechner Jarvis steuern.
//!
//! `{"cmd":"auth","token":"…"}` -> `{"ok":true}` (immer zuerst)
//! `{"cmd":"text","text":"öffne Minecraft"}` -> `{"ok":true,"text":"Minecraft wird gestartet."}`
//! `{"cmd":"devices"}` / `{"cmd":"trusted"}` / `{"cmd":"pair"}` / `{"cmd":"revoke","name":"…"}`
//! `{"cmd":"subscribe"}` -> fortlaufend Ereignisse (siehe `events`)
//! `{"cmd":"event","event":{…}}` -> keine Antwort

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream as StdTcpStream;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};
use tracing::{info, warn};

use crate::events::HudEvent;
use crate::node::{DeviceStatus, Node};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Request {
    Auth {
        token: String,
    },
    Text {
        text: String,
        /// Wer fragt („Sprache", „Tastatur") – erscheint im HUD.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
    /// Verbundene Geräte.
    Devices,
    /// Alle gekoppelten Geräte, auch offline.
    Trusted,
    /// Neuen Einmal-Code für die Kopplung erzeugen.
    Pair {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        addrs: Option<Vec<String>>,
    },
    /// Gerät sperren (verloren, verkauft …).
    Revoke {
        name: String,
    },
    /// Verbindung wird zum Ereignisstrom (eine JSON-Zeile pro [`HudEvent`]).
    Subscribe,
    /// Eigenes Ereignis an alle Abonnenten verteilen (z. B. von jarvis-listen).
    Event {
        event: HudEvent,
    },
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devices: Option<Vec<DeviceEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trusted: Option<Vec<TrustedEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

impl Response {
    fn text(ok: bool, text: impl Into<String>) -> Self {
        Self {
            ok,
            text: Some(text.into()),
            ..Default::default()
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceEntry {
    pub name: String,
    pub platform: String,
    pub this_device: bool,
    pub address: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrustedEntry {
    pub name: String,
    pub online: bool,
}

impl From<DeviceStatus> for DeviceEntry {
    fn from(d: DeviceStatus) -> Self {
        Self {
            name: d.name,
            platform: format!("{:?}", d.platform),
            this_device: d.this_device,
            address: d.address,
        }
    }
}

// ---------------------------------------------------------------- Token

/// Liest das Token oder legt es (nur für den Benutzer lesbar) neu an.
pub fn load_or_create_token(path: &Path) -> Result<String> {
    if let Some(t) = read_token(path) {
        return Ok(t);
    }
    let mut raw = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
    let token = hex::encode(raw);
    crate::trust::write_private(path, &token)?;
    Ok(token)
}

pub fn read_token(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| t.len() == 64)
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// Verbindung für Hilfsprogramme (blockierend): meldet sich mit dem Token an.
pub fn connect_blocking(
    port: u16,
    token_path: &Path,
) -> Result<(StdTcpStream, BufReader<StdTcpStream>)> {
    let token = read_token(token_path).context("Zugangs-Token fehlt – läuft jarvisd?")?;
    let mut stream = StdTcpStream::connect(("127.0.0.1", port)).context("jarvisd läuft nicht")?;
    stream.set_read_timeout(Some(Duration::from_secs(90)))?;
    let mut line = serde_json::to_string(&Request::Auth { token })?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut answer = String::new();
    reader.read_line(&mut answer)?;
    let resp: Response = serde_json::from_str(&answer).context("jarvisd antwortet unerwartet")?;
    if !resp.ok {
        bail!("Zugang verweigert: {}", resp.text.unwrap_or_default());
    }
    stream.set_read_timeout(None)?;
    Ok((stream, reader))
}

/// Eine Anfrage, eine Antwort (blockierend).
pub fn request_blocking(port: u16, token_path: &Path, req: &Request) -> Result<Response> {
    let (mut stream, mut reader) = connect_blocking(port, token_path)?;
    stream.set_read_timeout(Some(Duration::from_secs(90)))?;
    let mut line = serde_json::to_string(req)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    let mut answer = String::new();
    reader.read_line(&mut answer)?;
    Ok(serde_json::from_str(&answer)?)
}

// ---------------------------------------------------------------- Server

pub async fn serve(node: Arc<Node>, port: u16, token: String) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    info!("Steuerung auf 127.0.0.1:{port}");
    let token = Arc::new(token);
    loop {
        let (stream, _) = listener.accept().await?;
        let (node, token) = (node.clone(), token.clone());
        tokio::spawn(async move {
            let (r, w) = stream.into_split();
            let mut lines = FramedRead::new(r, LinesCodec::new_with_max_length(64 * 1024));
            let mut out = FramedWrite::new(w, LinesCodec::new());
            let send = |resp: &Response| serde_json::to_string(resp).unwrap_or_default();

            // 1. Anmeldung – sonst sofort trennen (auch Browser-Anfragen landen hier).
            let first = tokio::time::timeout(Duration::from_secs(5), lines.next()).await;
            let authed = matches!(
                first,
                Ok(Some(Ok(ref line))) if matches!(serde_json::from_str::<Request>(line), Ok(Request::Auth { token: ref t }) if same(t, &token))
            );
            if !authed {
                warn!("Steuerung: Verbindung ohne gültiges Token abgewiesen");
                let _ = out
                    .send(send(&Response::text(false, "Zugang verweigert")))
                    .await;
                return;
            }
            if out
                .send(send(&Response {
                    ok: true,
                    ..Default::default()
                }))
                .await
                .is_err()
            {
                return;
            }

            // 2. Anfragen
            while let Some(Ok(line)) = lines.next().await {
                let resp = match serde_json::from_str::<Request>(&line) {
                    Ok(Request::Subscribe) => {
                        let mut events = node.subscribe();
                        loop {
                            let event = match events.recv().await {
                                Ok(e) => e,
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                    continue
                                }
                                Err(_) => return,
                            };
                            if out
                                .send(serde_json::to_string(&event).unwrap_or_default())
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                    Ok(Request::Event { event }) => {
                        // Ohne Antwort: Ereignisse kommen bis zu 20-mal pro Sekunde.
                        node.emit(event);
                        continue;
                    }
                    Ok(Request::Auth { .. }) => Response {
                        ok: true,
                        ..Default::default()
                    },
                    Ok(Request::Text { text, source }) => {
                        let reply = node.handle_text_from(&text, source.as_deref()).await;
                        Response::text(reply.ok, reply.text)
                    }
                    Ok(Request::Devices) => Response {
                        ok: true,
                        devices: Some(node.devices().into_iter().map(Into::into).collect()),
                        ..Default::default()
                    },
                    Ok(Request::Trusted) => Response {
                        ok: true,
                        trusted: Some(
                            node.trusted_devices()
                                .into_iter()
                                .map(|(name, online)| TrustedEntry { name, online })
                                .collect(),
                        ),
                        ..Default::default()
                    },
                    Ok(Request::Pair { addrs }) => match node.create_invite(addrs) {
                        Ok(link) => Response {
                            ok: true,
                            link: Some(link),
                            ..Default::default()
                        },
                        Err(e) => Response::text(false, format!("{e:#}")),
                    },
                    Ok(Request::Revoke { name }) => match node.revoke(&name).await {
                        Ok(text) => Response::text(true, text),
                        Err(e) => Response::text(false, format!("{e:#}")),
                    },
                    Err(e) => Response::text(false, format!("Ungültige Anfrage: {e}")),
                };
                if out.send(send(&resp)).await.is_err() {
                    break;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_created_once_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("control.token");
        let a = load_or_create_token(&p).unwrap();
        let b = load_or_create_token(&p).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[tokio::test]
    async fn browser_and_wrong_token_are_rejected() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("JARVIS_CONFIG", dir.path().join("c.toml"));
        let mut cfg = crate::config::Config::new("Test".into()).unwrap();
        cfg.mesh.mdns = false;
        cfg.brain.provider = crate::config::BrainProvider::Off;
        cfg.save_to(&dir.path().join("c.toml")).unwrap();
        let node = Node::new(cfg).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        tokio::spawn(serve(node, port, "a".repeat(64)));
        tokio::time::sleep(Duration::from_millis(100)).await;

        // So sähe eine Anfrage einer präparierten Webseite aus:
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        s.write_all(b"POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\n\r\n{\"cmd\":\"text\",\"text\":\"sag hallo\"}\n").await.unwrap();
        let mut lines = tokio::io::BufReader::new(s).lines();
        assert!(lines
            .next_line()
            .await
            .unwrap()
            .unwrap()
            .contains("Zugang verweigert"));
        assert!(
            lines.next_line().await.unwrap().is_none(),
            "Verbindung muss danach getrennt sein"
        );

        // Falsches Token
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        s.write_all(format!("{{\"cmd\":\"auth\",\"token\":\"{}\"}}\n", "b".repeat(64)).as_bytes())
            .await
            .unwrap();
        let mut lines = tokio::io::BufReader::new(s).lines();
        assert!(lines
            .next_line()
            .await
            .unwrap()
            .unwrap()
            .contains("Zugang verweigert"));

        // Richtiges Token
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        s.write_all(
            format!(
                "{{\"cmd\":\"auth\",\"token\":\"{}\"}}\n{{\"cmd\":\"devices\"}}\n",
                "a".repeat(64)
            )
            .as_bytes(),
        )
        .await
        .unwrap();
        let mut lines = tokio::io::BufReader::new(s).lines();
        assert!(lines
            .next_line()
            .await
            .unwrap()
            .unwrap()
            .contains("\"ok\":true"));
        assert!(lines.next_line().await.unwrap().unwrap().contains("Test"));
    }
}
