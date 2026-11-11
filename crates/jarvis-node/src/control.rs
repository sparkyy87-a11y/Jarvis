//! Lokale Steuerschnittstelle (nur 127.0.0.1): eine JSON-Zeile rein, eine raus.
//!
//! Darüber sprechen CLI, Sprachmodul und später die Desktop-Oberfläche mit
//! dem Dienst:
//! `{"cmd":"text","text":"öffne Minecraft"}` -> `{"ok":true,"text":"Minecraft wird gestartet."}`
//! `{"cmd":"devices"}` -> `{"ok":true,"devices":[...]}`
//! `{"cmd":"subscribe"}` -> fortlaufend Ereignisse (siehe `events`)
//! `{"cmd":"event","event":{…}}` -> keine Antwort

use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tracing::info;

use crate::events::HudEvent;
use crate::node::{DeviceStatus, Node};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Request {
    Text {
        text: String,
        /// Wer fragt („Sprache", „Tastatur") – erscheint im HUD.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
    Devices,
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceEntry {
    pub name: String,
    pub platform: String,
    pub this_device: bool,
    pub address: Option<String>,
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

pub async fn serve(node: Arc<Node>, port: u16) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    info!("Steuerung auf 127.0.0.1:{port}");
    loop {
        let (stream, _) = listener.accept().await?;
        let node = node.clone();
        tokio::spawn(async move {
            let (r, mut w) = stream.into_split();
            let mut lines = BufReader::new(r).lines();
            while let Ok(Some(line)) = lines.next_line().await {
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
                            let mut out = serde_json::to_string(&event).unwrap_or_default();
                            out.push('\n');
                            if w.write_all(out.as_bytes()).await.is_err() {
                                return;
                            }
                        }
                    }
                    Ok(Request::Event { event }) => {
                        // Ohne Antwort: Ereignisse kommen bis zu 20-mal pro Sekunde.
                        node.emit(event);
                        continue;
                    }
                    Ok(Request::Text { text, source }) => {
                        let reply = node.handle_text_from(&text, source.as_deref()).await;
                        Response {
                            ok: reply.ok,
                            text: Some(reply.text),
                            devices: None,
                        }
                    }
                    Ok(Request::Devices) => Response {
                        ok: true,
                        text: None,
                        devices: Some(node.devices().into_iter().map(Into::into).collect()),
                    },
                    Err(e) => Response {
                        ok: false,
                        text: Some(format!("Ungültige Anfrage: {e}")),
                        devices: None,
                    },
                };
                let mut out = serde_json::to_string(&resp).unwrap_or_default();
                out.push('\n');
                if w.write_all(out.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
    }
}
