//! Gemeinsame Sprache aller Jarvis-Geräte.
//!
//! Alles, was zwischen zwei Geräten über das Netz geht, ist ein [`Frame`].
//! Frames werden mit `postcard` (kompaktes Binärformat, sehr schnell) kodiert
//! und vom Transport (Noise-verschlüsselt, siehe `jarvis-node`) übertragen.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Version des Drahtprotokolls. Wird im `Hello` ausgetauscht.
pub const PROTOCOL_VERSION: u16 = 2;

/// Größe eines Datei-Chunks. Muss zusammen mit dem Frame-Overhead unter die
/// Noise-Grenze von 65 535 Byte pro Nachricht passen.
pub const FILE_CHUNK_SIZE: usize = 48 * 1024;

/// mDNS-Diensttyp, unter dem sich Jarvis-Geräte im LAN finden.
pub const MDNS_SERVICE_TYPE: &str = "_jarvis._tcp.local.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
    Android,
    Ios,
    IpadOs,
}

impl Platform {
    pub const ALL: [Platform; 6] = [
        Platform::Windows,
        Platform::MacOs,
        Platform::Linux,
        Platform::Android,
        Platform::Ios,
        Platform::IpadOs,
    ];

    /// Plattform, für die gerade kompiliert wird. iPadOS lässt sich zur
    /// Compile-Zeit nicht von iOS unterscheiden; die App setzt das zur Laufzeit.
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(target_os = "android") {
            Platform::Android
        } else if cfg!(target_os = "ios") {
            Platform::Ios
        } else {
            Platform::Linux
        }
    }

    /// Umgangssprachliche Namen, mit denen man ein Gerät dieser Plattform
    /// anspricht ("am PC", "auf dem iPad", "aufs Handy").
    pub fn generic_names(self) -> &'static [&'static str] {
        match self {
            Platform::Windows | Platform::Linux => &["pc", "computer", "rechner", "desktop"],
            Platform::MacOs => &["mac", "macbook", "computer", "rechner"],
            Platform::Android | Platform::Ios => {
                &["handy", "smartphone", "phone", "telefon", "iphone"]
            }
            Platform::IpadOs => &["ipad", "tablet"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: Uuid,
    /// Anzeigename, z. B. "Gaming-PC".
    pub name: String,
    pub platform: Platform,
    /// Zusätzliche Namen, unter denen das Gerät angesprochen werden darf.
    pub aliases: Vec<String>,
    pub capabilities: Vec<Capability>,
}

impl DeviceInfo {
    /// Alle Namen (klein geschrieben), auf die dieses Gerät hört.
    pub fn spoken_names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::iter::once(self.name.clone())
            .chain(self.aliases.iter().cloned())
            .map(|n| n.to_lowercase())
            .collect();
        names.extend(self.platform.generic_names().iter().map(|s| s.to_string()));
        names.sort();
        names.dedup();
        names
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    OpenApps,
    CloseApps,
    ReceiveFiles,
    Speak,
    ComputerUse,
    /// Hat ein Sprachmodell und deutet freie Sätze für andere Geräte.
    Brain,
}

/// Bekannte Zielordner. `Custom` ist ein Pfad relativ zum Home-Verzeichnis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Folder {
    Documents,
    Downloads,
    Desktop,
    Pictures,
    Music,
    Videos,
    Custom(String),
}

impl Folder {
    /// Erkennt deutsche und englische Ordnernamen.
    pub fn from_spoken(s: &str) -> Folder {
        let k = s.trim().trim_end_matches(['.', '!', '?']).to_lowercase();
        let k = k
            .trim_start_matches("meine ")
            .trim_start_matches("meinen ")
            .trim_start_matches("my ")
            .trim();
        match k {
            "dokumente" | "dokumenten" | "documents" | "docs" | "eigene dateien" => {
                Folder::Documents
            }
            "downloads" | "download" => Folder::Downloads,
            "desktop" | "schreibtisch" => Folder::Desktop,
            "bilder" | "bildern" | "fotos" | "pictures" | "photos" => Folder::Pictures,
            "musik" | "music" => Folder::Music,
            "videos" | "video" | "filme" => Folder::Videos,
            _ => Folder::Custom(s.trim().to_string()),
        }
    }
}

/// Was Jarvis tun soll – unabhängig davon, auf welchem Gerät.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    OpenApp {
        app: String,
    },
    CloseApp {
        app: String,
    },
    /// `file` liegt auf dem Gerät, auf dem der Befehl gesprochen wurde.
    SendFile {
        file: String,
        folder: Option<Folder>,
    },
    Say {
        text: String,
    },
    /// Freie Aufgabe für den Computer-Use-Agenten ("mach ein Backup von …").
    ComputerUse {
        task: String,
    },
}

/// Auf welchem Gerät eine Aktion stattfinden soll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    /// Das Gerät, auf dem der Befehl gesprochen wurde.
    Here,
    /// Ein Gerät, so wie der Nutzer es genannt hat ("pc", "ipad", "Gaming-PC").
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub action: Action,
    pub target: Target,
}

/// Eine Nachricht zwischen zwei verbundenen Geräten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Frame {
    Hello {
        version: u16,
        device: DeviceInfo,
    },
    Execute {
        id: Uuid,
        action: Action,
    },
    Outcome {
        id: Uuid,
        ok: bool,
        message: String,
    },
    FileStart {
        id: Uuid,
        name: String,
        size: u64,
        folder: Option<Folder>,
    },
    FileChunk {
        id: Uuid,
        data: Vec<u8>,
    },
    /// SHA-256 des gesamten Inhalts, hex-kodiert.
    FileEnd {
        id: Uuid,
        sha256: String,
    },
    Ping,
    Pong,
    /// Bitte um Deutung eines freien Satzes (an ein Gerät mit `Brain`).
    /// `here` ist der Name des fragenden Geräts.
    Interpret {
        id: Uuid,
        text: String,
        here: String,
    },
    /// Antwort darauf: Befehl zum Ausführen (beim Fragenden) und/oder Antwort.
    Interpreted {
        id: Uuid,
        ok: bool,
        command: Option<Command>,
        reply: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("Frame konnte nicht kodiert/dekodiert werden: {0}")]
    Postcard(#[from] postcard::Error),
}

impl Frame {
    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        Ok(postcard::to_stdvec(self)?)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        Ok(postcard::from_bytes(bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let f = Frame::Execute {
            id: Uuid::new_v4(),
            action: Action::SendFile {
                file: "a.txt".into(),
                folder: Some(Folder::Documents),
            },
        };
        assert_eq!(Frame::decode(&f.encode().unwrap()).unwrap(), f);
    }

    #[test]
    fn full_chunk_fits_into_noise_message() {
        let f = Frame::FileChunk {
            id: Uuid::new_v4(),
            data: vec![0xAB; FILE_CHUNK_SIZE],
        };
        // 16 Byte Noise-Auth-Tag kommen noch dazu.
        assert!(f.encode().unwrap().len() + 16 < 65_535);
    }

    #[test]
    fn folders() {
        assert_eq!(Folder::from_spoken("Dokumente"), Folder::Documents);
        assert_eq!(Folder::from_spoken("meine Bilder"), Folder::Pictures);
        assert_eq!(
            Folder::from_spoken("Projekte/Jarvis"),
            Folder::Custom("Projekte/Jarvis".into())
        );
    }
}
