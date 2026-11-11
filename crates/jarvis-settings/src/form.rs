//! Das Formular: was die Oberfläche bearbeitet, übersetzt von und zur Konfiguration.
//! Reine Logik – testbar ohne Fenster.

use jarvis_node::config::{BrainProvider, Config};

#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub name: String,
    pub aliases: String,
    pub static_peers: String,
    pub mdns: bool,

    pub voice_enabled: bool,
    pub microphone: Option<String>,
    pub language: String,
    pub whisper_model: Option<String>,
    pub wake_words: String,
    pub speak_replies: bool,
    pub follow_up_seconds: u64,

    pub provider: BrainProvider,
    pub model: String,
    pub ollama_url: String,
    pub api_key: String,

    pub music_service: String,
    pub apps: Vec<(String, String)>,

    pub hud_enabled: bool,
    pub hud_position: String,
}

fn list(s: &str) -> Vec<String> {
    s.split([',', '\n'])
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(String::from)
        .collect()
}

fn opt(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

impl Form {
    pub fn from_config(c: &Config) -> Self {
        Self {
            name: c.device.name.clone(),
            aliases: c.device.aliases.join(", "),
            static_peers: c.mesh.static_peers.join("\n"),
            mdns: c.mesh.mdns,
            voice_enabled: c.voice.enabled,
            microphone: c.voice.input_device.clone(),
            language: c.voice.language.clone(),
            whisper_model: c.voice.model.clone(),
            wake_words: c.voice.wake_words.join(", "),
            speak_replies: c.voice.speak_replies,
            follow_up_seconds: c.voice.follow_up_seconds,
            provider: c.brain.provider,
            model: c.brain.model.clone().unwrap_or_default(),
            ollama_url: c.brain.ollama_url.clone().unwrap_or_default(),
            api_key: c.brain.api_key.clone().unwrap_or_default(),
            music_service: c.music.service.clone(),
            apps: c.apps.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            hud_enabled: c.hud.enabled,
            hud_position: c.hud.position.clone(),
        }
    }

    /// Fehler, die das Speichern verhindern.
    pub fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        if self.name.trim().is_empty() {
            p.push("Das Gerät braucht einen Namen.".into());
        }
        if list(&self.wake_words).is_empty() {
            p.push("Mindestens ein Weckwort angeben (z. B. Jarvis).".into());
        }
        for a in list(&self.static_peers) {
            if a.rsplit_once(':')
                .and_then(|(_, port)| port.parse::<u16>().ok())
                .is_none()
            {
                p.push(format!(
                    "Adresse „{a}“ braucht einen Port, z. B. 192.168.1.20:7879."
                ));
            }
        }
        if !self.ollama_url.trim().is_empty() && !self.ollama_url.trim().starts_with("http") {
            p.push("Die Ollama-Adresse beginnt mit http:// oder https://.".into());
        }
        if self.provider == BrainProvider::Claude
            && self.api_key.trim().is_empty()
            && std::env::var("ANTHROPIC_API_KEY").is_err()
        {
            p.push("Für Claude wird ein API-Schlüssel gebraucht.".into());
        }
        if self
            .apps
            .iter()
            .any(|(k, v)| k.trim().is_empty() != v.trim().is_empty())
        {
            p.push("Bei eigenen Kurzbefehlen braucht jede Zeile Name und Befehl.".into());
        }
        p
    }

    /// Überträgt das Formular in die Konfiguration; Schlüssel, Ports usw. bleiben.
    pub fn apply(&self, c: &mut Config) {
        c.device.name = self.name.trim().to_string();
        c.device.aliases = list(&self.aliases);
        c.mesh.static_peers = list(&self.static_peers);
        c.mesh.mdns = self.mdns;
        c.voice.enabled = self.voice_enabled;
        c.voice.input_device = self.microphone.clone();
        c.voice.language = self.language.clone();
        c.voice.model = self.whisper_model.clone();
        c.voice.wake_words = list(&self.wake_words)
            .into_iter()
            .map(|w| w.to_lowercase())
            .collect();
        c.voice.speak_replies = self.speak_replies;
        c.voice.follow_up_seconds = self.follow_up_seconds;
        c.brain.provider = self.provider;
        c.brain.model = opt(&self.model);
        c.brain.ollama_url = opt(&self.ollama_url);
        c.brain.api_key = opt(&self.api_key);
        c.music.service = self.music_service.clone();
        c.apps = self
            .apps
            .iter()
            .filter(|(k, v)| !k.trim().is_empty() && !v.trim().is_empty())
            .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
            .collect();
        c.hud.enabled = self.hud_enabled;
        c.hud.position = self.hud_position.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_keeps_keys_and_ports() {
        let mut cfg = Config::new("PC".into()).unwrap();
        cfg.mesh.port = 7999;
        let key = cfg.device.private_key.clone();
        let mut f = Form::from_config(&cfg);
        f.name = "  Gaming-PC ".into();
        f.aliases = "großer Rechner, Zocker-PC".into();
        f.apps
            .push(("Minecraft".into(), r"C:\Games\Minecraft.exe".into()));
        f.apps.push((String::new(), String::new())); // leere Zeile wird ignoriert
        f.wake_words = "Jarvis, Friday".into();
        assert!(f.problems().is_empty(), "{:?}", f.problems());
        f.apply(&mut cfg);
        assert_eq!(cfg.device.name, "Gaming-PC");
        assert_eq!(cfg.device.aliases, vec!["großer Rechner", "Zocker-PC"]);
        assert_eq!(
            cfg.apps.get("minecraft").map(String::as_str),
            Some(r"C:\Games\Minecraft.exe")
        );
        assert_eq!(cfg.voice.wake_words, vec!["jarvis", "friday"]);
        assert_eq!(
            cfg.device.private_key, key,
            "Geräteschlüssel bleibt unangetastet"
        );
        assert_eq!(cfg.mesh.port, 7999);
        assert_eq!(Form::from_config(&cfg).name, "Gaming-PC");
    }

    #[test]
    fn problems_are_reported() {
        let cfg = Config::new("PC".into()).unwrap();
        let mut f = Form::from_config(&cfg);
        f.name = " ".into();
        f.static_peers = "192.168.1.20".into();
        f.ollama_url = "localhost:11434".into();
        f.apps.push(("spotify".into(), String::new()));
        let p = f.problems();
        assert_eq!(p.len(), 4, "{p:?}");
    }
}
