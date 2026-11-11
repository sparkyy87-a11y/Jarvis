//! Das "Gehirn" für alles, was die lokalen Regeln nicht verstehen.
//!
//! Zwei austauschbare Backends:
//! - **Ollama** (Standard): ein lokales Sprachmodell auf dem eigenen Rechner –
//!   kostenlos, offline, privat.
//! - **Claude** über die Anthropic-API: deutlich klüger, aber kostenpflichtig
//!   (API-Schlüssel nötig; das kostenlose claude.ai-Abo gilt dafür nicht).
//!
//! Beide werden in dasselbe JSON-Schema gezwungen und liefern einen [`Command`].

use std::time::Duration;

use anyhow::{bail, Context, Result};
use jarvis_protocol::{normalize_music_service, Action, Command, Folder, MediaCommand, Target};
use serde::Deserialize;
use serde_json::json;

pub const DEFAULT_CLAUDE_MODEL: &str = "claude-opus-5-5";
pub const DEFAULT_OLLAMA_MODEL: &str = "qwen2.5:3b";
pub const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";

const CLAUDE_URL: &str = "https://api.anthropic.com/v1/messages";
const CLAUDE_API_VERSION: &str = "2026-10-21";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    Ollama { url: String, model: String },
    Claude { api_key: String, model: String },
}

impl Backend {
    pub fn describe(&self) -> String {
        match self {
            Backend::Ollama { model, .. } => format!("lokales Modell {model} (Ollama)"),
            Backend::Claude { model, .. } => format!("Claude ({model})"),
        }
    }
}

/// Was das Modell zurückgibt: entweder ein Befehl oder eine direkte Antwort.
#[derive(Debug, Clone, PartialEq)]
pub enum Interpretation {
    Command { command: Command, reply: String },
    Answer(String),
}

pub struct Brain {
    backend: Backend,
    http: reqwest::Client,
}

impl Brain {
    pub fn new(backend: Backend) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("HTTP-Client");
        Self { backend, http }
    }

    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    /// `devices`: (Name, Plattform) aller bekannten Geräte, `here`: Name des
    /// Geräts, auf dem gesprochen wurde.
    pub async fn interpret(
        &self,
        utterance: &str,
        here: &str,
        devices: &[(String, String)],
    ) -> Result<Interpretation> {
        let system = system_prompt(here, devices);
        let text = match &self.backend {
            Backend::Ollama { url, model } => {
                self.ask_ollama(url, model, &system, utterance).await?
            }
            Backend::Claude { api_key, model } => {
                match self.ask_claude(api_key, model, &system, utterance).await? {
                    Some(text) => text,
                    None => {
                        return Ok(Interpretation::Answer(
                            "Dabei kann ich leider nicht helfen.".into(),
                        ))
                    }
                }
            }
        };
        let raw: RawCommand = serde_json::from_str(text.trim())
            .with_context(|| format!("unerwartete Antwort des Modells: {text}"))?;
        Ok(raw.into_interpretation())
    }

    async fn ask_ollama(
        &self,
        url: &str,
        model: &str,
        system: &str,
        utterance: &str,
    ) -> Result<String> {
        let body = json!({
            "model": model,
            "stream": false,
            "format": schema(false),
            "options": {"temperature": 0},
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": utterance}
            ]
        });
        let resp = self
            .http
            .post(format!("{}/api/chat", url.trim_end_matches('/')))
            .json(&body)
            .send()
            .await
            .context(
                "Ollama läuft nicht – installieren von ollama.com und `ollama serve` starten",
            )?;
        let status = resp.status();
        let value: serde_json::Value = resp
            .json()
            .await
            .context("Antwort von Ollama ist kein JSON")?;
        if !status.is_success() {
            let msg = value["error"].as_str().unwrap_or("unbekannt");
            if msg.contains("not found") {
                bail!("Modell fehlt – einmalig `ollama pull {model}` ausführen");
            }
            bail!("Ollama-Fehler {status}: {msg}");
        }
        match value["message"]["content"].as_str() {
            Some(text) => Ok(text.to_string()),
            None => bail!("Ollama hat keinen Text geliefert"),
        }
    }

    /// `None` bei einer Ablehnung durch die Sicherheitsfilter.
    async fn ask_claude(
        &self,
        api_key: &str,
        model: &str,
        system: &str,
        utterance: &str,
    ) -> Result<Option<String>> {
        let body = json!({
            "model": model,
            "max_tokens": 4096,
            "system": system,
            "messages": [{"role": "user", "content": utterance}],
            "output_config": {
                // `low` hält die Latenz klein; für Befehlserkennung reicht das.
                "effort": "low",
                "format": {"type": "json_schema", "schema": schema(true)}
            },
            // Lehnt das Modell ab, springt serverseitig ein passendes Ersatzmodell ein.
            "fallbacks": "default"
        });
        let resp = self
            .http
            .post(CLAUDE_URL)
            .header("x-api-key", api_key)
            .header("anthropic-version", CLAUDE_API_VERSION)
            .header("anthropic-beta", "server-side-fallback-2026-07-01")
            .json(&body)
            .send()
            .await
            .context("Claude nicht erreichbar")?;
        let status = resp.status();
        let value: serde_json::Value = resp
            .json()
            .await
            .context("Antwort von Claude ist kein JSON")?;
        if !status.is_success() {
            bail!(
                "Claude-API-Fehler {status}: {}",
                value["error"]["message"].as_str().unwrap_or("unbekannt")
            );
        }
        if value["stop_reason"] == "refusal" {
            return Ok(None);
        }
        let text = value["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<String>();
        Ok(Some(text))
    }
}

fn system_prompt(here: &str, devices: &[(String, String)]) -> String {
    let device_list = devices
        .iter()
        .map(|(n, p)| format!("- {n} ({p})"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Du bist Jarvis, ein Sprachassistent, der mehrere Geräte des Nutzers steuert. \
         Übersetze die Äußerung in genau eine Aktion. Antworte in der Sprache des Nutzers, kurz und natürlich, \
         denn `reply` wird vorgelesen.\n\n\
         Der Nutzer spricht gerade an: {here}\nBekannte Geräte:\n{device_list}\n\n\
         Regeln:\n\
         - target_device ist der Name aus der Liste, auf dem die Aktion passieren soll; leer oder null für das aktuelle Gerät.\n\
         - Bei send_file ist target_device das Empfängergerät und file der Dateiname auf dem aktuellen Gerät.\n\
         - folder: documents, downloads, desktop, pictures, music, videos oder ein relativer Pfad; sonst leer.\n\
         - web_search: Suchbegriff in text (öffnet Google im Browser des Zielgeräts).\n\
         - play_music: Titel/Künstler/Album in text, Dienst (apple music, spotify, youtube music) in app oder leer.\n\
         - media: Wiedergabe steuern; text ist play, pause, next oder previous.\n\
         - pair_device: ein neues Gerät verbinden/koppeln (zeigt einen QR-Code).\n\
         - computer_use nur für Aufgaben, die echte Bedienung des Bildschirms brauchen; die Aufgabe kommt in text.\n\
         - Ist es eine Frage oder Unterhaltung, nimm action=answer und schreibe die Antwort in reply.\n\
         - Felder, die nicht passen, bleiben leer."
    )
}

/// JSON-Schema der Antwort. Kleine lokale Modelle kommen mit `null` in
/// Typ-Listen schlechter zurecht, daher dort nur Strings (leer = nicht gesetzt).
fn schema(nullable: bool) -> serde_json::Value {
    let field = |desc: &str| {
        if nullable {
            json!({"type": ["string", "null"], "description": desc})
        } else {
            json!({"type": "string", "description": desc})
        }
    };
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["action", "target_device", "app", "file", "folder", "text", "reply"],
        "properties": {
            "action": {"type": "string", "enum": ["open_app", "close_app", "send_file", "say", "web_search", "play_music", "media", "pair_device", "computer_use", "answer"]},
            "target_device": field("Gerätename aus der Liste, sonst leer"),
            "app": field("App-Name für open_app/close_app"),
            "file": field("Dateiname für send_file"),
            "folder": field("Zielordner für send_file"),
            "text": field("Text für say bzw. Aufgabe für computer_use"),
            "reply": {"type": "string", "description": "Kurze gesprochene Rückmeldung"}
        }
    })
}

#[derive(Debug, Deserialize)]
struct RawCommand {
    action: String,
    #[serde(default)]
    target_device: Option<String>,
    #[serde(default)]
    app: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    folder: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    reply: String,
}

impl RawCommand {
    fn into_interpretation(self) -> Interpretation {
        let need = |v: Option<String>| v.filter(|s| !s.trim().is_empty() && s.trim() != "null");
        let action = match self.action.as_str() {
            "open_app" => need(self.app).map(|app| Action::OpenApp { app }),
            "close_app" => need(self.app).map(|app| Action::CloseApp { app }),
            "send_file" => need(self.file).map(|file| Action::SendFile {
                file,
                folder: need(self.folder).map(|f| Folder::from_spoken(&f)),
            }),
            "say" => need(self.text).map(|text| Action::Say { text }),
            "computer_use" => need(self.text).map(|task| Action::ComputerUse { task }),
            "pair_device" => Some(Action::PairDevice),
            "web_search" => need(self.text).map(|query| Action::WebSearch { query }),
            "play_music" => need(self.text).map(|query| Action::PlayMusic {
                query,
                service: self.app.as_deref().and_then(normalize_music_service),
            }),
            "media" => self
                .text
                .as_deref()
                .and_then(MediaCommand::from_word)
                .map(|command| Action::Media { command }),
            _ => None,
        };
        match action {
            Some(action) => {
                let target = need(self.target_device)
                    .map(|d| Target::Named(d.to_lowercase()))
                    .unwrap_or(Target::Here);
                Interpretation::Command {
                    command: Command { action, target },
                    reply: self.reply,
                }
            }
            None => Interpretation::Answer(self.reply),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_style_answer_with_nulls() {
        let raw: RawCommand = serde_json::from_str(
            r#"{"action":"send_file","target_device":"Gaming-PC","app":null,"file":"a.txt","folder":"documents","text":null,"reply":"Mach ich."}"#,
        )
        .unwrap();
        assert_eq!(
            raw.into_interpretation(),
            Interpretation::Command {
                command: Command {
                    action: Action::SendFile {
                        file: "a.txt".into(),
                        folder: Some(Folder::Documents)
                    },
                    target: Target::Named("gaming-pc".into()),
                },
                reply: "Mach ich.".into(),
            }
        );
    }

    #[test]
    fn ollama_style_answer_with_empty_strings() {
        let raw: RawCommand = serde_json::from_str(
            r#"{"action":"open_app","target_device":"","app":"Spotify","file":"","folder":"","text":"","reply":"Spotify läuft gleich."}"#,
        )
        .unwrap();
        assert_eq!(
            raw.into_interpretation(),
            Interpretation::Command {
                command: Command {
                    action: Action::OpenApp {
                        app: "Spotify".into()
                    },
                    target: Target::Here
                },
                reply: "Spotify läuft gleich.".into(),
            }
        );
    }

    #[test]
    fn answer_or_incomplete_is_answer() {
        let raw: RawCommand = serde_json::from_str(
            r#"{"action":"open_app","target_device":null,"app":"","file":null,"folder":null,"text":null,"reply":"Welche App?"}"#,
        )
        .unwrap();
        assert_eq!(
            raw.into_interpretation(),
            Interpretation::Answer("Welche App?".into())
        );
    }

    #[test]
    fn local_schema_has_no_null_types() {
        assert!(!schema(false).to_string().contains("null"));
    }

    #[test]
    fn music_and_search_from_model() {
        let raw: RawCommand = serde_json::from_str(
            r#"{"action":"play_music","target_device":"","app":"Apple Music","file":"","folder":"","text":"Queen","reply":"Queen läuft."}"#,
        )
        .unwrap();
        assert_eq!(
            raw.into_interpretation(),
            Interpretation::Command {
                command: Command {
                    action: Action::PlayMusic {
                        query: "Queen".into(),
                        service: Some("apple-music".into())
                    },
                    target: Target::Here
                },
                reply: "Queen läuft.".into()
            }
        );
        let raw: RawCommand = serde_json::from_str(
            r#"{"action":"media","target_device":"PC","app":"","file":"","folder":"","text":"pause","reply":"Pause."}"#,
        )
        .unwrap();
        assert!(matches!(
            raw.into_interpretation(),
            Interpretation::Command {
                command: Command {
                    action: Action::Media {
                        command: MediaCommand::Pause
                    },
                    ..
                },
                ..
            }
        ));
    }
}
