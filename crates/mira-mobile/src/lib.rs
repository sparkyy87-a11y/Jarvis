//! Mira-Kern für die Handy-Apps (iPhone, iPad, Android).
//!
//! Die Apps rufen diese Funktionen über automatisch erzeugte Kotlin- bzw.
//! Swift-Anbindungen (UniFFI) auf. Netz, Verschlüsselung, Befehlserkennung
//! und Dateiübertragung sind exakt dieselben wie auf dem PC; nur das, was
//! nur das Handy kann (Apps öffnen, vorlesen), erledigt die App über
//! [`PlatformBridge`].
//!
//! Alle Methoden blockieren und sind für einen Hintergrund-Thread gedacht.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::bail;
use mira_node::config::{BrainProvider, Config};
use mira_node::host::Host;
use mira_node::node::Node;
use mira_node::trust::{Joining, TrustStore};
use mira_node::{discovery, files, pairing};
use mira_protocol::{Action, Command, Folder, Platform, Target};

uniffi::setup_scaffolding!();

/// „Flach": Kotlin/Swift sehen nur den Text (`e.message` bzw. `localizedDescription`).
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum MiraError {
    #[error("{message}")]
    Failed { message: String },
}

impl From<anyhow::Error> for MiraError {
    fn from(e: anyhow::Error) -> Self {
        MiraError::Failed {
            message: format!("{e:#}"),
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Reply {
    pub ok: bool,
    pub text: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Device {
    pub name: String,
    pub platform: String,
    pub this_device: bool,
    pub address: Option<String>,
}

/// Ein gekoppeltes Gerät mit Rolle und Rechten (verwaltet am Hauptgerät).
#[derive(Debug, Clone, uniffi::Record)]
pub struct Member {
    pub name: String,
    pub online: bool,
    pub this_device: bool,
    /// Hauptgerät: koppelt, sperrt, vergibt Rechte.
    pub admin: bool,
    /// Darf andere Geräte steuern.
    pub control: bool,
    /// Darf auch das Hauptgerät steuern.
    pub control_main: bool,
    /// Darf Dateien schicken.
    pub files: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PairingInfo {
    /// Gerät, das den Code gezeigt hat (meist der PC).
    pub name: String,
    pub addresses: Vec<String>,
}

/// Was die App für Mira erledigt.
#[uniffi::export(with_foreign)]
pub trait PlatformBridge: Send + Sync {
    /// App öffnen (Android: Launcher-Eintrag, iOS: URL-Schema).
    fn open_app(&self, app: String) -> Reply;
    /// Text vorlesen.
    fn speak(&self, text: String);
    /// Eine Datei ist angekommen (z. B. Benachrichtigung zeigen).
    fn file_received(&self, path: String);
    /// Adresse öffnen (Browser bzw. zuständige App), z. B. eine Google-Suche.
    fn open_url(&self, url: String) -> Reply;
    /// Musik suchen/abspielen. `service`: "apple-music", "spotify", "youtube-music".
    fn play_music(&self, query: String, service: String) -> Reply;
    /// Wiedergabe steuern: "play", "pause", "next", "previous".
    fn media(&self, command: String) -> Reply;
}

/// Prüft einen Kopplungscode (`mira://pair?…`) ohne ihn zu speichern.
#[uniffi::export]
pub fn parse_pairing_code(code: String) -> Result<PairingInfo, MiraError> {
    let p = pairing::parse(&code)?;
    Ok(PairingInfo {
        name: p.name,
        addresses: p.addrs,
    })
}

/// Ist dieses Handy schon mit einem Mira gekoppelt?
#[uniffi::export]
pub fn is_paired(data_dir: String) -> bool {
    config_path(&data_dir).is_file()
}

/// Koppelt dieses Handy mit dem Kopplungscode vom PC (`mirad pair`).
/// `platform`: "android", "ios" oder "ipados".
#[uniffi::export]
pub fn pair(
    data_dir: String,
    device_name: String,
    platform: String,
    code: String,
) -> Result<PairingInfo, MiraError> {
    let p = pairing::parse(&code)?;
    let platform = parse_platform(&platform)?;
    // Bei erneuter Kopplung Geräte-ID und -Schlüssel behalten: die anderen
    // Geräte kennen sie schon.
    let mut cfg = Config::new(device_name)?;
    if let Ok(old) = Config::load_from(&config_path(&data_dir)) {
        cfg.device.id = old.device.id;
        cfg.device.private_key = old.device.private_key;
        cfg.device.public_key = old.device.public_key;
        cfg.device.signing_key = old.device.signing_key;
    }
    cfg.device.platform = Some(platform);
    cfg.mesh.static_peers = p.addrs.clone();
    // iOS erlaubt Multicast nur mit Sondergenehmigung; dort genügt die feste Adresse.
    cfg.mesh.mdns = platform == Platform::Android;
    // Freie Sätze deutet das Sprachmodell auf dem PC.
    cfg.brain.provider = BrainProvider::Off;
    cfg.voice.enabled = false;
    cfg.save_to(&config_path(&data_dir))?;
    // Die eigentliche Kopplung passiert beim ersten Kontakt: Einmal-Code und
    // erwarteter Schlüssel des PCs (Schutz gegen ein Gerät dazwischen).
    let mut trust = TrustStore::load(&cfg.trust_path())?;
    trust.joining = Some(Joining {
        token: p.token,
        key: p.key,
        sign_key: p.sign_key,
        name: p.name.clone(),
    });
    trust.save(&cfg.trust_path())?;
    Ok(PairingInfo {
        name: p.name,
        addresses: p.addrs,
    })
}

/// Kopplung löschen.
#[uniffi::export]
pub fn unpair(data_dir: String) {
    let _ = std::fs::remove_file(PathBuf::from(&data_dir).join("trust.toml"));
    let _ = std::fs::remove_file(config_path(&data_dir));
}

#[derive(uniffi::Object)]
pub struct Mira {
    rt: tokio::runtime::Runtime,
    node: Arc<Node>,
    files_dir: PathBuf,
}

#[uniffi::export]
impl Mira {
    /// Startet Mira auf dem Handy. `data_dir` ist ein privater App-Ordner
    /// (Kopplung). Empfangene Dateien landen in `files_dir` – ohne Angabe in
    /// `data_dir/Mira` (iOS übergibt „Dokumente", sichtbar in der Dateien-App).
    #[uniffi::constructor]
    pub fn start(
        data_dir: String,
        files_dir: Option<String>,
        bridge: Arc<dyn PlatformBridge>,
    ) -> Result<Arc<Self>, MiraError> {
        let cfg = Config::load_from(&config_path(&data_dir)).map_err(|_| MiraError::Failed {
            message: "Noch nicht gekoppelt – am PC `mirad pair` ausführen und den QR-Code scannen."
                .into(),
        })?;
        let files_dir = files_dir
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&data_dir).join("Mira"));
        std::fs::create_dir_all(&files_dir).map_err(anyhow::Error::from)?;
        let host = Arc::new(MobileHost {
            bridge,
            files_dir: files_dir.clone(),
            music_service: cfg.music.service.clone(),
        });
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("mira")
            .build()
            .map_err(anyhow::Error::from)?;
        let node = {
            let _guard = rt.enter();
            Node::with_host(cfg.clone(), host)?
        };
        rt.spawn(node.clone().listen());
        rt.spawn(node.clone().maintain_static_peers());
        if cfg.mesh.mdns {
            rt.spawn(discovery::run(node.clone(), cfg.mesh.port));
        }
        Ok(Arc::new(Self {
            rt,
            node,
            files_dir,
        }))
    }

    /// Text (getippt oder gesprochen) an Mira. Blockiert bis zur Antwort.
    pub fn ask(&self, text: String) -> Reply {
        let r = self.rt.block_on(self.node.handle_text(&text));
        Reply {
            ok: r.ok,
            text: r.text,
        }
    }

    /// Eine Datei (z. B. aus dem Teilen-Menü) an ein Gerät schicken.
    pub fn send_file(&self, path: String, device: String, folder: Option<String>) -> Reply {
        let cmd = Command {
            action: Action::SendFile {
                file: path,
                folder: folder.as_deref().map(Folder::from_spoken),
            },
            target: Target::Named(device.to_lowercase()),
        };
        match self.rt.block_on(self.node.run(&cmd)) {
            Ok(text) => Reply { ok: true, text },
            Err(e) => Reply {
                ok: false,
                text: format!("{e:#}"),
            },
        }
    }

    pub fn devices(&self) -> Vec<Device> {
        self.node
            .devices()
            .into_iter()
            .map(|d| Device {
                name: d.name,
                platform: format!("{:?}", d.platform),
                this_device: d.this_device,
                address: d.address,
            })
            .collect()
    }

    /// Alle gekoppelten Geräte (auch offline), Hauptgerät zuerst.
    pub fn members(&self) -> Vec<Member> {
        self.node
            .trusted_devices()
            .into_iter()
            .map(|m| Member {
                name: m.name,
                online: m.online,
                this_device: m.this_device,
                admin: m.admin,
                control: m.control,
                control_main: m.control_main,
                files: m.files,
            })
            .collect()
    }

    /// Ordner mit empfangenen Dateien.
    pub fn files_dir(&self) -> String {
        self.files_dir.to_string_lossy().into_owned()
    }

    pub fn device_name(&self) -> String {
        self.node.me.name.clone()
    }

    /// Hinweis für die App, solange die Kopplung nicht abgeschlossen ist
    /// (z. B. „Kopplungscode abgelaufen …"). `None`, wenn alles gut ist.
    pub fn status(&self) -> Option<String> {
        self.node.pairing_problem().or_else(|| {
            self.node
                .pairing_pending()
                .then(|| "Kopplung läuft – PC und Handy müssen im selben WLAN sein.".to_string())
        })
    }
}

fn config_path(data_dir: &str) -> PathBuf {
    PathBuf::from(data_dir).join("mira.toml")
}

fn parse_platform(p: &str) -> anyhow::Result<Platform> {
    Ok(match p.to_lowercase().as_str() {
        "android" => Platform::Android,
        "ios" | "iphone" => Platform::Ios,
        "ipados" | "ipad" => Platform::IpadOs,
        other => bail!("Unbekannte Plattform „{other}“"),
    })
}

struct MobileHost {
    bridge: Arc<dyn PlatformBridge>,
    files_dir: PathBuf,
    music_service: String,
}

fn reply_to_result(r: Reply) -> anyhow::Result<String> {
    if r.ok {
        Ok(r.text)
    } else {
        bail!(r.text)
    }
}

impl Host for MobileHost {
    fn open_app(&self, app: &str) -> anyhow::Result<String> {
        let r = self.bridge.open_app(app.to_string());
        if r.ok {
            Ok(r.text)
        } else {
            bail!(r.text)
        }
    }

    fn close_app(&self, app: &str) -> anyhow::Result<String> {
        bail!("Apps schließen kann ich auf dem Handy nicht ({app}).")
    }

    fn say(&self, text: &str) -> anyhow::Result<()> {
        self.bridge.speak(text.to_string());
        Ok(())
    }

    fn receive_dir(&self, folder: Option<&Folder>) -> anyhow::Result<PathBuf> {
        let sub = match folder {
            None | Some(Folder::Downloads) => "Downloads".to_string(),
            Some(Folder::Documents) => "Dokumente".into(),
            Some(Folder::Desktop) => "Desktop".into(),
            Some(Folder::Pictures) => "Bilder".into(),
            Some(Folder::Music) => "Musik".into(),
            Some(Folder::Videos) => "Videos".into(),
            Some(Folder::Custom(rel)) => {
                let rel = Path::new(rel);
                if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
                    bail!("Ungültiger Zielordner „{}“", rel.display());
                }
                rel.to_string_lossy().into_owned()
            }
        };
        Ok(self.files_dir.join(sub))
    }

    fn locate_file(&self, spoken: &str) -> anyhow::Result<PathBuf> {
        files::locate_in(std::slice::from_ref(&self.files_dir), spoken)
    }

    fn file_received(&self, path: &Path) {
        self.bridge
            .file_received(path.to_string_lossy().into_owned());
    }

    fn open_url(&self, url: &str) -> anyhow::Result<String> {
        reply_to_result(self.bridge.open_url(url.to_string()))
    }

    fn play_music(&self, query: &str, service: Option<&str>) -> anyhow::Result<String> {
        let service = service.unwrap_or(&self.music_service).to_string();
        reply_to_result(self.bridge.play_music(query.to_string(), service))
    }

    fn media(&self, command: mira_protocol::MediaCommand) -> anyhow::Result<String> {
        let word = match command {
            mira_protocol::MediaCommand::Play => "play",
            mira_protocol::MediaCommand::Pause => "pause",
            mira_protocol::MediaCommand::Next => "next",
            mira_protocol::MediaCommand::Previous => "previous",
        };
        reply_to_result(self.bridge.media(word.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeBridge {
        spoken: Mutex<Vec<String>>,
        opened: Mutex<Vec<String>>,
    }

    impl PlatformBridge for FakeBridge {
        fn open_app(&self, app: String) -> Reply {
            self.opened.lock().unwrap().push(app.clone());
            Reply {
                ok: true,
                text: format!("{app} geöffnet"),
            }
        }
        fn speak(&self, text: String) {
            self.spoken.lock().unwrap().push(text);
        }
        fn file_received(&self, _path: String) {}
        fn open_url(&self, url: String) -> Reply {
            self.opened.lock().unwrap().push(url);
            Reply {
                ok: true,
                text: "geöffnet".into(),
            }
        }
        fn play_music(&self, query: String, service: String) -> Reply {
            self.opened
                .lock()
                .unwrap()
                .push(format!("{service}:{query}"));
            Reply {
                ok: true,
                text: format!("{query} läuft"),
            }
        }
        fn media(&self, command: String) -> Reply {
            Reply {
                ok: true,
                text: command,
            }
        }
    }

    fn code() -> String {
        format!(
            "mira://pair?v=3&key={}&skey={}&token={}&name=Gaming-PC&addr=127.0.0.1:1",
            "ab".repeat(32),
            "ef".repeat(32),
            "cd".repeat(16)
        )
    }

    #[test]
    fn pair_and_start() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().to_string_lossy().to_string();
        assert!(!is_paired(d.clone()));
        assert!(Mira::start(d.clone(), None, Arc::new(FakeBridge::default())).is_err());

        let info = pair(d.clone(), "Handy".into(), "android".into(), code()).unwrap();
        assert_eq!(info.name, "Gaming-PC");
        assert!(is_paired(d.clone()));

        let bridge = Arc::new(FakeBridge::default());
        let j = Mira::start(d.clone(), None, bridge.clone()).unwrap();
        assert_eq!(j.device_name(), "Handy");
        let r = j.ask("öffne Spotify".into());
        assert!(r.ok, "{}", r.text);
        assert_eq!(*bridge.opened.lock().unwrap(), vec!["Spotify".to_string()]);
        let r = j.ask("google Wetter Berlin".into());
        assert!(r.ok && r.text.contains("Wetter Berlin"), "{}", r.text);
        let r = j.ask("spiel Musik von Adele".into());
        assert_eq!(r.text, "Adele läuft");
        assert_eq!(
            bridge.opened.lock().unwrap()[1..],
            [
                "https://www.google.com/search?q=Wetter+Berlin".to_string(),
                "apple-music:Adele".to_string()
            ]
        );
        let r = j.ask("schließe Spotify".into());
        assert!(!r.ok);
        // Ohne verbundenen PC kein Sprachmodell für freie Sätze:
        let r = j.ask("wie wird das Wetter".into());
        assert!(!r.ok && r.text.contains("Sprachmodell"), "{}", r.text);
    }
}
