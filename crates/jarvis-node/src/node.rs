//! Ein Jarvis-Gerät im Verbund: kennt die anderen Geräte, versteht Befehle
//! und führt sie lokal oder auf dem richtigen Gerät aus.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use jarvis_brain::{Brain, Interpretation};
use jarvis_protocol::{
    Action, Capability, Command, DeviceInfo, Folder, Frame, Platform, Target, FILE_CHUNK_SIZE,
    PROTOCOL_VERSION,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config::Config;
use crate::transport::SecureConn;
use crate::{exec, files};

const REMOTE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct Peer {
    conn_id: Uuid,
    /// Welche Seite die Verbindung aufgebaut hat – entscheidet bei doppelten Verbindungen.
    initiator: Uuid,
    info: DeviceInfo,
    addr: SocketAddr,
    tx: mpsc::Sender<Frame>,
    /// Schließt die Verbindung, wenn sie durch eine bessere ersetzt wird.
    kill: Arc<tokio::sync::Notify>,
}

/// Antwort auf eine Äußerung – wird angezeigt bzw. vorgelesen.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Reply {
    pub ok: bool,
    pub text: String,
}

impl Reply {
    fn ok(text: impl Into<String>) -> Self {
        Self {
            ok: true,
            text: text.into(),
        }
    }
    fn err(text: impl Into<String>) -> Self {
        Self {
            ok: false,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceStatus {
    pub name: String,
    pub platform: Platform,
    pub this_device: bool,
    pub address: Option<String>,
}

type Pending = oneshot::Sender<(bool, String)>;

pub struct Node {
    pub me: DeviceInfo,
    cfg: Config,
    psk: [u8; 32],
    peers: Mutex<HashMap<Uuid, Peer>>,
    pending: Mutex<HashMap<Uuid, Pending>>,
    /// Feste Adresse -> Gerät, das dort zuletzt erreichbar war.
    static_links: Mutex<HashMap<String, Uuid>>,
    brain: Option<Brain>,
}

enum Resolved {
    Here,
    Remote(Peer),
}

impl Node {
    pub fn new(cfg: Config) -> Result<Arc<Self>> {
        let me = DeviceInfo {
            id: cfg.device.id,
            name: cfg.device.name.clone(),
            platform: cfg.platform(),
            aliases: cfg.device.aliases.clone(),
            capabilities: capabilities_for(cfg.platform()),
        };
        let brain = match cfg.brain.backend() {
            Ok(Some(backend)) => {
                info!("Freie Sätze versteht: {}", backend.describe());
                Some(Brain::new(backend))
            }
            Ok(None) => {
                info!("Kein Sprachmodell – nur eingebaute Befehle aktiv");
                None
            }
            Err(e) => bail!("[brain]: {e}"),
        };
        Ok(Arc::new(Self {
            me,
            psk: cfg.pairing_key()?,
            cfg,
            peers: Mutex::default(),
            pending: Mutex::default(),
            static_links: Mutex::default(),
            brain,
        }))
    }

    // ------------------------------------------------------------------
    // Verbindungen
    // ------------------------------------------------------------------

    /// Nimmt eingehende Verbindungen anderer Geräte an.
    pub async fn listen(self: Arc<Self>) -> Result<()> {
        let listener = TcpListener::bind(("0.0.0.0", self.cfg.mesh.port))
            .await
            .with_context(|| format!("Port {} ist belegt", self.cfg.mesh.port))?;
        info!("Mesh lauscht auf Port {}", self.cfg.mesh.port);
        loop {
            let (stream, addr) = listener.accept().await?;
            let node = self.clone();
            tokio::spawn(async move {
                match SecureConn::accept(stream, &node.psk).await {
                    Ok(conn) => {
                        if let Err(e) = node.run_connection(conn, addr, false, None).await {
                            debug!("Verbindung von {addr} beendet: {e:#}");
                        }
                    }
                    Err(e) => warn!("Abgelehnte Verbindung von {addr}: {e:#}"),
                }
            });
        }
    }

    /// Baut eine Verbindung zu `addr` auf und betreibt sie, bis sie endet.
    pub async fn connect(
        self: Arc<Self>,
        addr: SocketAddr,
        static_addr: Option<String>,
    ) -> Result<()> {
        let stream =
            tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr)).await??;
        let conn = SecureConn::initiate(stream, &self.psk).await?;
        self.run_connection(conn, addr, true, static_addr).await
    }

    pub fn is_connected(&self, id: &Uuid) -> bool {
        self.peers.lock().unwrap().contains_key(id)
    }

    /// Verbindet regelmäßig die fest eingetragenen Adressen.
    /// Wartezeit wächst von 1 s bis 15 s, solange ein Gerät nicht erreichbar ist.
    pub async fn maintain_static_peers(self: Arc<Self>) {
        let mut delay = Duration::from_secs(1);
        loop {
            let mut all_connected = true;
            for addr in self.cfg.mesh.static_peers.clone() {
                let linked = self.static_links.lock().unwrap().get(&addr).copied();
                if linked.is_some_and(|id| self.is_connected(&id)) {
                    continue;
                }
                all_connected = false;
                let Ok(sock) = tokio::net::lookup_host(&addr).await.map(|mut a| a.next()) else {
                    continue;
                };
                let Some(sock) = sock else { continue };
                let node = self.clone();
                tokio::spawn(async move {
                    if let Err(e) = node.connect(sock, Some(addr.clone())).await {
                        debug!("{addr} nicht erreichbar: {e:#}");
                    }
                });
            }
            delay = if all_connected {
                Duration::from_secs(1)
            } else {
                (delay * 2).min(Duration::from_secs(15))
            };
            tokio::time::sleep(delay).await;
        }
    }

    async fn run_connection(
        self: Arc<Self>,
        mut conn: SecureConn,
        addr: SocketAddr,
        initiated_by_me: bool,
        static_addr: Option<String>,
    ) -> Result<()> {
        conn.send(&Frame::Hello {
            version: PROTOCOL_VERSION,
            device: self.me.clone(),
        })
        .await?;
        let info = match conn.recv().await? {
            Some(Frame::Hello { version, device }) if version == PROTOCOL_VERSION => device,
            Some(Frame::Hello { version, .. }) => {
                bail!("Gerät spricht Protokoll v{version}, wir v{PROTOCOL_VERSION}")
            }
            _ => bail!("Kein Hello erhalten"),
        };
        if info.id == self.me.id {
            bail!("Verbindung zu mir selbst");
        }
        if let Some(a) = static_addr {
            self.static_links.lock().unwrap().insert(a, info.id);
        }

        let (tx, mut rx) = mpsc::channel::<Frame>(64);
        let kill = Arc::new(tokio::sync::Notify::new());
        let peer = Peer {
            conn_id: Uuid::new_v4(),
            initiator: if initiated_by_me { self.me.id } else { info.id },
            info: info.clone(),
            addr,
            tx: tx.clone(),
            kill: kill.clone(),
        };
        if !self.register(peer.clone()) {
            debug!("Doppelte Verbindung zu {} verworfen", info.name);
            return Ok(());
        }
        info!("Verbunden mit {} ({:?}, {addr})", info.name, info.platform);

        let mut incoming: HashMap<Uuid, Incoming> = HashMap::new();
        let result: Result<()> = async {
            loop {
                tokio::select! {
                    frame = conn.recv() => match frame? {
                        None => return Ok(()),
                        Some(f) => {
                            // Direkte Antworten gehen sofort raus, nicht über `tx`:
                            // sonst könnte die eigene Warteschlange uns blockieren.
                            if let Some(answer) = self.clone().on_frame(f, &tx, &mut incoming).await {
                                conn.send(&answer).await?;
                            }
                        }
                    },
                    Some(out) = rx.recv() => conn.send(&out).await?,
                    _ = kill.notified() => return Ok(()),
                }
            }
        }
        .await;

        for (_, inc) in incoming.drain() {
            let _ = tokio::fs::remove_file(&inc.part).await;
        }
        let mut peers = self.peers.lock().unwrap();
        if peers
            .get(&info.id)
            .is_some_and(|p| p.conn_id == peer.conn_id)
        {
            peers.remove(&info.id);
            info!("Getrennt von {}", info.name);
        }
        result
    }

    /// Trägt ein Gerät ein. Bestehen zwei Verbindungen zum selben Gerät, gewinnt
    /// auf beiden Seiten diejenige, die das Gerät mit der kleineren ID aufgebaut hat.
    fn register(&self, peer: Peer) -> bool {
        let mut peers = self.peers.lock().unwrap();
        let preferred = self.me.id.min(peer.info.id);
        if let Some(existing) = peers.get(&peer.info.id) {
            if existing.initiator == preferred || peer.initiator != preferred {
                return false;
            }
            existing.kill.notify_one();
        }
        peers.insert(peer.info.id, peer);
        true
    }

    /// Verarbeitet einen eingehenden Frame; eine direkte Antwort wird zurückgegeben.
    async fn on_frame(
        self: Arc<Self>,
        frame: Frame,
        tx: &mpsc::Sender<Frame>,
        incoming: &mut HashMap<Uuid, Incoming>,
    ) -> Option<Frame> {
        let fail = |id, e: anyhow::Error| {
            Some(Frame::Outcome {
                id,
                ok: false,
                message: format!("{e:#}"),
            })
        };
        match frame {
            Frame::Ping => Some(Frame::Pong),
            Frame::Pong | Frame::Hello { .. } => None,
            Frame::Execute { id, action } => {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let (ok, message) = match self.execute_local(&action).await {
                        Ok(m) => (true, m),
                        Err(e) => (false, format!("{e:#}")),
                    };
                    let _ = tx.send(Frame::Outcome { id, ok, message }).await;
                });
                None
            }
            Frame::Outcome { id, ok, message } => {
                if let Some(waiter) = self.pending.lock().unwrap().remove(&id) {
                    let _ = waiter.send((ok, message));
                }
                None
            }
            Frame::FileStart {
                id,
                name,
                size,
                folder,
            } => match Incoming::start(&name, size, folder.as_ref()).await {
                Ok(inc) => {
                    incoming.insert(id, inc);
                    None
                }
                Err(e) => fail(id, e),
            },
            Frame::FileChunk { id, data } => {
                let inc = incoming.get_mut(&id)?;
                let Err(e) = inc.write(&data).await else {
                    return None;
                };
                let inc = incoming.remove(&id).expect("eben gefunden");
                let _ = tokio::fs::remove_file(&inc.part).await;
                fail(id, e)
            }
            Frame::FileEnd { id, sha256 } => {
                let inc = incoming.remove(&id)?;
                match inc.finish(&sha256).await {
                    Ok(path) => {
                        info!("Datei empfangen: {}", path.display());
                        Some(Frame::Outcome {
                            id,
                            ok: true,
                            message: format!("gespeichert unter {}", path.display()),
                        })
                    }
                    Err(e) => fail(id, e),
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Befehle
    // ------------------------------------------------------------------

    /// Alle Namen, unter denen Geräte (inkl. diesem) erreichbar sind. Gattungs-
    /// namen ("pc", "ipad", …) sind immer dabei, damit "am PC" auch dann als Ziel
    /// erkannt wird, wenn der PC gerade offline ist.
    fn spoken_names(&self) -> Vec<String> {
        let mut names = self.me.spoken_names();
        names.extend(
            Platform::ALL
                .iter()
                .flat_map(|p| p.generic_names())
                .map(|n| n.to_string()),
        );
        for p in self.peers.lock().unwrap().values() {
            names.extend(p.info.spoken_names());
        }
        names.sort();
        names.dedup();
        names
    }

    pub fn devices(&self) -> Vec<DeviceStatus> {
        let mut list = vec![DeviceStatus {
            name: self.me.name.clone(),
            platform: self.me.platform,
            this_device: true,
            address: None,
        }];
        let mut others: Vec<DeviceStatus> = self
            .peers
            .lock()
            .unwrap()
            .values()
            .map(|p| DeviceStatus {
                name: p.info.name.clone(),
                platform: p.info.platform,
                this_device: false,
                address: Some(p.addr.to_string()),
            })
            .collect();
        others.sort_by(|a, b| a.name.cmp(&b.name));
        list.extend(others);
        list
    }

    /// Einstiegspunkt für alles, was der Nutzer sagt oder tippt.
    pub async fn handle_text(&self, text: &str) -> Reply {
        let started = std::time::Instant::now();
        let reply = match jarvis_intent::parse(text, &self.spoken_names()) {
            Some(cmd) => {
                debug!("Lokal erkannt: {cmd:?}");
                match self.run(&cmd).await {
                    Ok(msg) => Reply::ok(msg),
                    Err(e) => Reply::err(format!("{e:#}")),
                }
            }
            None => self.ask_brain(text).await,
        };
        debug!("Antwort nach {:?}", started.elapsed());
        reply
    }

    async fn ask_brain(&self, text: &str) -> Reply {
        let Some(brain) = &self.brain else {
            return Reply::err("Das habe ich nicht verstanden. (Sprachmodell ist ausgeschaltet.)");
        };
        let devices: Vec<(String, String)> = self
            .devices()
            .into_iter()
            .map(|d| (d.name, format!("{:?}", d.platform)))
            .collect();
        match brain.interpret(text, &self.me.name, &devices).await {
            Ok(Interpretation::Answer(a)) => Reply::ok(a),
            Ok(Interpretation::Command { command, reply }) => match self.run(&command).await {
                Ok(_) => Reply::ok(reply),
                Err(e) => Reply::err(format!("{e:#}")),
            },
            Err(e) => Reply::err(format!(
                "Das habe ich nicht verstanden, und {} hilft gerade nicht weiter: {e}",
                brain.backend().describe()
            )),
        }
    }

    /// Führt einen erkannten Befehl auf dem richtigen Gerät aus.
    pub async fn run(&self, cmd: &Command) -> Result<String> {
        let target = self.resolve(&cmd.target)?;
        match (&cmd.action, target) {
            (Action::SendFile { file, folder }, Resolved::Remote(peer)) => {
                let path = tokio::task::spawn_blocking({
                    let file = file.clone();
                    move || files::locate(&file)
                })
                .await??;
                self.send_file(&peer, &path, folder.clone()).await?;
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                Ok(format!("{name} ist auf {} angekommen.", peer.info.name))
            }
            (Action::SendFile { .. }, Resolved::Here) => {
                bail!("Die Datei ist schon auf diesem Gerät.")
            }
            (action, Resolved::Here) => self.execute_local(action).await,
            (action, Resolved::Remote(peer)) => {
                let msg = self.remote(&peer, action.clone()).await?;
                Ok(format!("{}: {msg}", peer.info.name))
            }
        }
    }

    fn resolve(&self, target: &Target) -> Result<Resolved> {
        let name = match target {
            Target::Here => return Ok(Resolved::Here),
            Target::Named(n) => n.to_lowercase(),
        };
        let exact = |d: &DeviceInfo| {
            d.name.to_lowercase() == name || d.aliases.iter().any(|a| a.to_lowercase() == name)
        };
        if exact(&self.me) {
            return Ok(Resolved::Here);
        }
        let peers: Vec<Peer> = self.peers.lock().unwrap().values().cloned().collect();
        if let Some(p) = peers.iter().find(|p| exact(&p.info)) {
            return Ok(Resolved::Remote(p.clone()));
        }
        // Gattungsnamen ("pc", "ipad"): eindeutig nur, wenn genau ein anderes
        // Gerät passt. Sagt man am PC "am PC", ist dieser PC gemeint.
        let generic: Vec<&Peer> = peers
            .iter()
            .filter(|p| p.info.spoken_names().contains(&name))
            .collect();
        match generic.as_slice() {
            [one] => Ok(Resolved::Remote((*one).clone())),
            [] if self.me.spoken_names().contains(&name) => Ok(Resolved::Here),
            [] => bail!("Ich kenne kein verbundenes Gerät „{name}“."),
            many => bail!(
                "Mehrere Geräte passen zu „{name}“: {}. Welches meinst du?",
                many.iter()
                    .map(|p| p.info.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    async fn execute_local(&self, action: &Action) -> Result<String> {
        match action {
            Action::OpenApp { app } => {
                let (app, apps) = (app.clone(), self.cfg.apps.clone());
                tokio::task::spawn_blocking(move || exec::open_app(&app, &apps)).await?
            }
            Action::CloseApp { app } => {
                let app = app.clone();
                tokio::task::spawn_blocking(move || exec::close_app(&app)).await?
            }
            Action::Say { text } => {
                exec::say(text).await?;
                Ok("Gesagt.".into())
            }
            Action::ComputerUse { task } => {
                bail!("Computer Use ist noch nicht eingebaut (Aufgabe: „{task}“).")
            }
            Action::SendFile { .. } => {
                bail!("Dateien werden über FileStart/FileChunk empfangen, nicht per Execute.")
            }
        }
    }

    fn expect_outcome(&self, id: Uuid) -> oneshot::Receiver<(bool, String)> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        rx
    }

    async fn wait_outcome(
        &self,
        id: Uuid,
        rx: oneshot::Receiver<(bool, String)>,
        timeout: Duration,
    ) -> Result<String> {
        let res = tokio::time::timeout(timeout, rx).await;
        self.pending.lock().unwrap().remove(&id);
        match res {
            Ok(Ok((true, msg))) => Ok(msg),
            Ok(Ok((false, msg))) => Err(anyhow!(msg)),
            Ok(Err(_)) => bail!("Verbindung unterbrochen"),
            Err(_) => bail!("Keine Antwort vom Gerät"),
        }
    }

    async fn remote(&self, peer: &Peer, action: Action) -> Result<String> {
        let id = Uuid::new_v4();
        let rx = self.expect_outcome(id);
        peer.tx
            .send(Frame::Execute { id, action })
            .await
            .map_err(|_| anyhow!("{} ist nicht mehr verbunden", peer.info.name))?;
        self.wait_outcome(id, rx, REMOTE_TIMEOUT).await
    }

    async fn send_file(
        &self,
        peer: &Peer,
        path: &std::path::Path,
        folder: Option<Folder>,
    ) -> Result<String> {
        let gone = || anyhow!("{} ist nicht mehr verbunden", peer.info.name);
        let mut file = tokio::fs::File::open(path)
            .await
            .with_context(|| format!("{} nicht lesbar", path.display()))?;
        let size = file.metadata().await?.len();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "datei".into());
        let id = Uuid::new_v4();
        let rx = self.expect_outcome(id);

        info!(
            "Sende {} ({size} Byte) an {}",
            path.display(),
            peer.info.name
        );
        peer.tx
            .send(Frame::FileStart {
                id,
                name,
                size,
                folder,
            })
            .await
            .map_err(|_| gone())?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; FILE_CHUNK_SIZE];
        loop {
            let n = file.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            peer.tx
                .send(Frame::FileChunk {
                    id,
                    data: buf[..n].to_vec(),
                })
                .await
                .map_err(|_| gone())?;
        }
        let sha256 = hex::encode(hasher.finalize());
        peer.tx
            .send(Frame::FileEnd { id, sha256 })
            .await
            .map_err(|_| gone())?;
        self.wait_outcome(id, rx, REMOTE_TIMEOUT).await
    }
}

fn capabilities_for(p: Platform) -> Vec<Capability> {
    use Capability::*;
    match p {
        Platform::Windows | Platform::MacOs | Platform::Linux => {
            vec![OpenApps, CloseApps, ReceiveFiles, Speak, ComputerUse]
        }
        Platform::Android | Platform::Ios | Platform::IpadOs => vec![OpenApps, ReceiveFiles, Speak],
    }
}

/// Eine Datei, die gerade empfangen wird. Sie landet erst in `*.jarvis-part`
/// und wird nach erfolgreicher Prüfsumme umbenannt.
struct Incoming {
    file: tokio::fs::File,
    part: PathBuf,
    dir: PathBuf,
    name: String,
    expected: u64,
    written: u64,
    hasher: Sha256,
}

impl Incoming {
    async fn start(name: &str, size: u64, folder: Option<&Folder>) -> Result<Self> {
        let name = files::safe_file_name(name)?;
        let dir = files::resolve_folder(folder)?;
        tokio::fs::create_dir_all(&dir).await?;
        let part = dir.join(format!(".{name}.{}.jarvis-part", Uuid::new_v4().simple()));
        let file = tokio::fs::File::create(&part).await?;
        Ok(Self {
            file,
            part,
            dir,
            name,
            expected: size,
            written: 0,
            hasher: Sha256::new(),
        })
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        self.written += data.len() as u64;
        if self.written > self.expected {
            bail!("Mehr Daten als angekündigt");
        }
        self.hasher.update(data);
        self.file.write_all(data).await?;
        Ok(())
    }

    async fn finish(mut self, sha256: &str) -> Result<PathBuf> {
        self.file.flush().await?;
        drop(self.file);
        let actual = hex::encode(self.hasher.finalize());
        if self.written != self.expected || actual != sha256 {
            let _ = tokio::fs::remove_file(&self.part).await;
            bail!("Übertragung beschädigt (Prüfsumme stimmt nicht)");
        }
        let target = files::unique_path(&self.dir, &self.name);
        tokio::fs::rename(&self.part, &target).await?;
        Ok(target)
    }
}
