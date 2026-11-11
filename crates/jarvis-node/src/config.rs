use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use jarvis_protocol::Platform;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DEFAULT_MESH_PORT: u16 = 7879;
pub const DEFAULT_CONTROL_PORT: u16 = 7878;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub device: DeviceSection,
    pub mesh: MeshSection,
    /// Eigene Kurzbefehle: gesprochener Name -> Programm/Befehl,
    /// z. B. `minecraft = "C:\\XboxGames\\Minecraft Launcher\\Content\\Minecraft.exe"`.
    #[serde(default)]
    pub apps: BTreeMap<String, String>,
    #[serde(default)]
    pub brain: BrainSection,
    #[serde(default)]
    pub voice: VoiceSection,
}

/// Einstellungen für `jarvis-listen` (Spracheingabe).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceSection {
    /// Spracheingabe beim Autostart mitstarten.
    pub enabled: bool,
    /// Whisper-Modell, z. B. "large-v3-turbo-q5_0" (Grafikkarte) oder
    /// "small-q5_1" (nur Prozessor). Leer = automatisch passend zum Build.
    pub model: Option<String>,
    /// Sprache für die Erkennung ("de", "en" oder "auto").
    pub language: String,
    pub wake_words: Vec<String>,
    /// Teil des Mikrofonnamens; leer = Standardmikrofon.
    pub input_device: Option<String>,
    /// Antworten vorlesen.
    pub speak_replies: bool,
    /// Nach „Jarvis" ohne Befehl so lange auf den Befehl warten (Sekunden).
    pub follow_up_seconds: u64,
}

impl Default for VoiceSection {
    fn default() -> Self {
        Self {
            enabled: true,
            model: None,
            language: "de".into(),
            wake_words: vec!["jarvis".into()],
            input_device: None,
            speak_replies: true,
            follow_up_seconds: 8,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceSection {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Nur nötig, um iPadOS von iOS zu unterscheiden.
    #[serde(default)]
    pub platform: Option<Platform>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshSection {
    #[serde(default = "default_mesh_port")]
    pub port: u16,
    #[serde(default = "default_control_port")]
    pub control_port: u16,
    /// 32 Byte, hex. Alle eigenen Geräte teilen denselben Schlüssel.
    pub pairing_key: String,
    /// Feste Adressen ("192.168.1.20:7879"), falls mDNS im Netz nicht geht.
    #[serde(default)]
    pub static_peers: Vec<String>,
    #[serde(default = "yes")]
    pub mdns: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrainSection {
    /// Wer freie Sätze versteht.
    #[serde(default)]
    pub provider: BrainProvider,
    /// Modellname; Standard je nach Anbieter (siehe `jarvis_brain`).
    pub model: Option<String>,
    /// Adresse des Ollama-Servers, Standard `http://127.0.0.1:11434`.
    pub ollama_url: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrainProvider {
    /// Claude, wenn `ANTHROPIC_API_KEY` gesetzt ist, sonst Ollama (kostenlos).
    #[default]
    Auto,
    /// Lokales Modell über Ollama – kostenlos, offline.
    Ollama,
    /// Claude über die Anthropic-API (kostenpflichtig).
    Claude,
    /// Nur die eingebauten Befehle, kein Sprachmodell.
    Off,
}

impl BrainSection {
    /// Wählt das Backend; `Err` mit Begründung, wenn keins nutzbar ist.
    pub fn backend(&self) -> std::result::Result<Option<jarvis_brain::Backend>, String> {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.is_empty());
        let ollama = || jarvis_brain::Backend::Ollama {
            url: self
                .ollama_url
                .clone()
                .unwrap_or_else(|| jarvis_brain::DEFAULT_OLLAMA_URL.into()),
            model: self
                .model
                .clone()
                .unwrap_or_else(|| jarvis_brain::DEFAULT_OLLAMA_MODEL.into()),
        };
        let claude = |api_key: String| jarvis_brain::Backend::Claude {
            api_key,
            model: self
                .model
                .clone()
                .unwrap_or_else(|| jarvis_brain::DEFAULT_CLAUDE_MODEL.into()),
        };
        match (self.provider, key) {
            (BrainProvider::Off, _) => Ok(None),
            (BrainProvider::Ollama, _) | (BrainProvider::Auto, None) => Ok(Some(ollama())),
            (BrainProvider::Claude | BrainProvider::Auto, Some(k)) => Ok(Some(claude(k))),
            (BrainProvider::Claude, None) => {
                Err("provider = \"claude\", aber ANTHROPIC_API_KEY fehlt".into())
            }
        }
    }
}

fn default_mesh_port() -> u16 {
    DEFAULT_MESH_PORT
}
fn default_control_port() -> u16 {
    DEFAULT_CONTROL_PORT
}
fn yes() -> bool {
    true
}

impl Config {
    pub fn path() -> PathBuf {
        if let Ok(p) = std::env::var("JARVIS_CONFIG") {
            return PathBuf::from(p);
        }
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("jarvis")
            .join("config.toml")
    }

    pub fn load() -> Result<Self> {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &std::path::Path) -> Result<Self> {
        let path = path.to_path_buf();
        let text = std::fs::read_to_string(&path).with_context(|| {
            format!("{} fehlt – zuerst `jarvisd init` ausführen", path.display())
        })?;
        let cfg: Config =
            toml::from_str(&text).with_context(|| format!("{} ist ungültig", path.display()))?;
        cfg.pairing_key()?;
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Neue Konfiguration mit frischer Geräte-ID. Ohne `pairing_key` wird ein
    /// neuer Schlüssel erzeugt (erstes Gerät), sonst wird er übernommen.
    pub fn new(name: String, pairing_key: Option<String>) -> Result<Self> {
        let pairing_key = match pairing_key {
            Some(k) => k.trim().to_lowercase(),
            None => {
                let mut key = [0u8; 32];
                rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut key);
                hex::encode(key)
            }
        };
        let cfg = Config {
            device: DeviceSection {
                id: Uuid::new_v4(),
                name,
                aliases: vec![],
                platform: None,
            },
            mesh: MeshSection {
                port: DEFAULT_MESH_PORT,
                control_port: DEFAULT_CONTROL_PORT,
                pairing_key,
                static_peers: vec![],
                mdns: true,
            },
            apps: BTreeMap::new(),
            brain: BrainSection::default(),
            voice: VoiceSection::default(),
        };
        cfg.pairing_key()?;
        Ok(cfg)
    }

    pub fn pairing_key(&self) -> Result<[u8; 32]> {
        let bytes = hex::decode(&self.mesh.pairing_key).context("pairing_key ist kein Hex")?;
        match <[u8; 32]>::try_from(bytes.as_slice()) {
            Ok(k) => Ok(k),
            Err(_) => bail!("pairing_key muss 64 Hex-Zeichen (32 Byte) lang sein"),
        }
    }

    pub fn platform(&self) -> Platform {
        self.device.platform.unwrap_or_else(Platform::current)
    }
}
