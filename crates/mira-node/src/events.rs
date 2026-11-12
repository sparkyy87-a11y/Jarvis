//! Ereignisse für Oberflächen (HUD, Apps): was Mira gerade hört, denkt, sagt.
//!
//! `mirad` verteilt sie an alle, die sich über die Steuerschnittstelle mit
//! `{"cmd":"subscribe"}` angemeldet haben; `mira-listen` steuert eigene
//! Ereignisse (Weckwort, Lautstärke, Sprechen) mit `{"cmd":"event",…}` bei.

use mira_protocol::Action;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HudEvent {
    /// Weckwort erkannt – Mira hört zu.
    Wake,
    /// Mikrofonpegel 0..1, solange Mira zuhört.
    Level {
        value: f32,
    },
    /// Ein Befehl ist angekommen (`source`: Gerät oder „Sprache").
    Heard {
        text: String,
        source: Option<String>,
    },
    Thinking,
    Reply {
        text: String,
        ok: bool,
    },
    /// Mira spricht gerade (Antwort wird vorgelesen).
    Speaking {
        active: bool,
    },
    File {
        name: String,
        from: String,
    },
    Idle,
}

/// Kurzbeschreibung einer Aktion für die Anzeige.
pub fn describe(action: &Action) -> String {
    match action {
        Action::OpenApp { app } => format!("öffne {app}"),
        Action::CloseApp { app } => format!("schließe {app}"),
        Action::Say { text } => format!("sag „{text}“"),
        Action::SendFile { file, .. } => format!("Datei {file}"),
        Action::ComputerUse { task } => task.clone(),
        Action::WebSearch { query } => format!("suche nach {query}"),
        Action::PlayMusic { query, .. } => format!("spiele {query}"),
        Action::Media { command } => format!("{command:?}").to_lowercase(),
        Action::PairDevice => "neues Gerät koppeln".into(),
    }
}
