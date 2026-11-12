//! Ein Mira-Gerät im Verbund: kennt die anderen Geräte, versteht Befehle
//! und führt sie lokal oder auf dem richtigen Gerät aus.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use mira_brain::{Brain, Interpretation};
use mira_protocol::{
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
use crate::events::{describe, HudEvent};
use crate::files;
use crate::host::{DesktopHost, Host};
use crate::transport::{Identity, SecureConn};
use crate::trust::{member_from, Joining, Permission, TrustStore};

const REMOTE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct Peer {
    conn_id: Uuid,
    /// Welche Seite die Verbindung aufgebaut hat – entscheidet bei doppelten Verbindungen.
    initiator: Uuid,
    info: DeviceInfo,
    /// Im Handshake bewiesener Geräteschlüssel (hex).
    key: String,
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

/// Ein gekoppeltes Gerät mit Rolle und Rechten.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MemberStatus {
    pub name: String,
    pub online: bool,
    pub this_device: bool,
    /// Hauptgerät: darf koppeln, sperren, Rechte vergeben.
    pub admin: bool,
    /// Darf andere Geräte steuern.
    pub control: bool,
    /// Darf Dateien an andere Geräte schicken.
    pub files: bool,
    /// Darf auch das Hauptgerät steuern.
    pub control_main: bool,
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
    identity: Identity,
    trust: Mutex<TrustStore>,
    trust_path: std::path::PathBuf,
    /// Letztes Problem beim Koppeln (für die App-Anzeige).
    pairing_problem: Mutex<Option<String>>,
    peers: Mutex<HashMap<Uuid, Peer>>,
    pending: Mutex<HashMap<Uuid, Pending>>,
    /// Feste Adresse -> Gerät, das dort zuletzt erreichbar war.
    static_links: Mutex<HashMap<String, Uuid>>,
    brain: Option<Brain>,
    host: Arc<dyn Host>,
    events: tokio::sync::broadcast::Sender<HudEvent>,
    interpreting: Mutex<HashMap<Uuid, oneshot::Sender<Interpreted>>>,
}

/// Deutung eines freien Satzes, ggf. von einem anderen Gerät.
struct Interpreted {
    ok: bool,
    command: Option<Command>,
    reply: String,
}

#[allow(clippy::large_enum_variant)] // kurzlebig, nur auf dem Stack
enum Resolved {
    Here,
    Remote(Peer),
}

impl Node {
    pub fn new(cfg: Config) -> Result<Arc<Self>> {
        let host = Arc::new(DesktopHost::new(
            cfg.apps.clone(),
            cfg.music.service.clone(),
        ));
        Self::with_host(cfg, host)
    }

    /// Mit eigener Plattform-Umsetzung (Handy-Apps).
    pub fn with_host(cfg: Config, host: Arc<dyn Host>) -> Result<Arc<Self>> {
        let mut me = DeviceInfo {
            id: cfg.device.id,
            name: cfg.device.name.clone(),
            platform: cfg.platform(),
            aliases: cfg.device.aliases.clone(),
            capabilities: capabilities_for(cfg.platform()),
            port: cfg.mesh.port,
            sign_key: cfg.sign_public()?,
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
        if brain.is_some() {
            me.capabilities.push(Capability::Brain);
        }
        let trust_path = cfg.trust_path();
        let identity = cfg.identity()?;
        let mut trust = TrustStore::load(&trust_path)?;
        if trust.policy.is_none() && trust.joining.is_none() && trust.pinned.is_none() {
            // Erstes Gerät (nicht per Code beigetreten): wird Hauptgerät.
            trust.init_admin(
                member_from(&identity.public_hex(), &me, true, true),
                &cfg.device.signing_key,
            )?;
            trust.save(&trust_path)?;
            info!("{} ist das Hauptgerät", me.name);
        }
        Ok(Arc::new(Self {
            me,
            identity,
            trust: Mutex::new(trust),
            trust_path,
            pairing_problem: Mutex::default(),
            cfg,
            peers: Mutex::default(),
            pending: Mutex::default(),
            static_links: Mutex::default(),
            brain,
            host,
            events: tokio::sync::broadcast::channel(256).0,
            interpreting: Mutex::default(),
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
                match SecureConn::accept(stream, &node.identity).await {
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
        let conn = SecureConn::initiate(stream, &self.identity).await?;
        self.run_connection(conn, addr, true, static_addr).await
    }

    pub fn is_connected(&self, id: &Uuid) -> bool {
        self.peers.lock().unwrap().contains_key(id)
    }

    /// Verbindet regelmäßig die fest eingetragenen und die gelernten Adressen
    /// gekoppelter Geräte – so finden sich z. B. iPad und Handy auch ohne PC.
    /// Wartezeit wächst von 1 s bis 15 s, solange ein Gerät nicht erreichbar ist.
    pub async fn maintain_static_peers(self: Arc<Self>) {
        let mut delay = Duration::from_secs(1);
        loop {
            let mut all_connected = true;
            let connected: Vec<String> = self
                .peers
                .lock()
                .unwrap()
                .values()
                .map(|p| p.key.clone())
                .collect();
            let mut targets: Vec<(String, Option<String>)> = Vec::new();
            for addr in &self.cfg.mesh.static_peers {
                let linked = self.static_links.lock().unwrap().get(addr).copied();
                if !linked.is_some_and(|id| self.is_connected(&id)) {
                    targets.push((addr.clone(), Some(addr.clone())));
                }
            }
            for (key, addr) in self.trust.lock().unwrap().known_addresses() {
                if !connected.contains(&key) && !targets.iter().any(|(a, _)| *a == addr) {
                    targets.push((addr, None));
                }
            }
            for (addr, static_addr) in targets {
                all_connected = false;
                let Ok(sock) = tokio::net::lookup_host(&addr).await.map(|mut a| a.next()) else {
                    continue;
                };
                let Some(sock) = sock else { continue };
                let node = self.clone();
                tokio::spawn(async move {
                    if let Err(e) = node.connect(sock, static_addr).await {
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
        let key = conn.remote_key();
        if key == self.identity.public_hex() {
            bail!("Verbindung zu mir selbst");
        }
        if self.trust.lock().unwrap().is_revoked(&key) {
            bail!("gesperrtes Gerät ({addr}) abgewiesen");
        }
        // Unbekanntes Gerät: koppeln (mit Einmal-Code) – oder es bringt eine
        // vom Hauptgerät signierte Liste mit, in der es steht (z. B. ein Gerät,
        // das gerade erst gekoppelt wurde und uns zuerst erreicht).
        let mut early_hello = None;
        if !self.trust.lock().unwrap().is_trusted(&key) {
            early_hello = self
                .pair_unknown(&mut conn, &key, initiated_by_me, addr)
                .await?;
        }

        let (policy, addrs) = {
            let t = self.trust.lock().unwrap();
            (t.policy.clone(), t.addr_hints())
        };
        conn.send(&Frame::Hello {
            version: PROTOCOL_VERSION,
            device: self.me.clone(),
            policy,
            addrs,
        })
        .await?;
        let hello = match early_hello {
            Some(h) => Some(h),
            None => conn.recv().await?,
        };
        let (info, their_policy, their_addrs) = match hello {
            Some(Frame::Hello {
                version,
                device,
                policy,
                addrs,
            }) if version == PROTOCOL_VERSION => (device, policy, addrs),
            Some(Frame::Hello { version, .. }) => {
                bail!("Gerät spricht Protokoll v{version}, wir v{PROTOCOL_VERSION} – bitte alle Geräte aktualisieren")
            }
            _ => bail!("Kein Hello erhalten"),
        };
        if info.id == self.me.id {
            bail!("Verbindung zu mir selbst");
        }
        // Neuere, vom Hauptgerät signierte Liste übernehmen; Adressen abgleichen.
        let (policy_changed, addrs_changed) = {
            let mut t = self.trust.lock().unwrap();
            let policy_changed = match their_policy.as_ref().map(|p| t.accept(p)) {
                Some(Ok(changed)) => changed,
                Some(Err(e)) => {
                    warn!("Geräteliste von {} verworfen: {e:#}", info.name);
                    false
                }
                None => false,
            };
            if !t.is_trusted(&key) {
                bail!("{} ({addr}) gehört nicht (mehr) dazu", info.name);
            }
            // Adresse merken, unter der das Gerät Verbindungen annimmt.
            t.observe(&key, SocketAddr::new(addr.ip(), info.port).to_string());
            let addrs_changed = t.merge_addrs(&self.identity.public_hex(), &their_addrs);
            let _ = t.save(&self.trust_path);
            (policy_changed, addrs_changed)
        };
        if policy_changed {
            self.drop_untrusted();
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
            key: key.clone(),
            addr,
            tx: tx.clone(),
            kill: kill.clone(),
        };
        if !self.register(peer.clone()) {
            debug!("Doppelte Verbindung zu {} verworfen", info.name);
            return Ok(());
        }
        info!("Verbunden mit {} ({:?}, {addr})", info.name, info.platform);
        if policy_changed || addrs_changed {
            self.broadcast_trust(Some(&key));
        }

        let mut incoming: HashMap<Uuid, Incoming> = HashMap::new();
        let result: Result<()> = async {
            loop {
                tokio::select! {
                    frame = conn.recv() => match frame? {
                        None => return Ok(()),
                        Some(f) => {
                            // Direkte Antworten gehen sofort raus, nicht über `tx`:
                            // sonst könnte die eigene Warteschlange uns blockieren.
                            if let Some(answer) = self.clone().on_frame(f, &tx, &mut incoming, &info.name, &key).await {
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

    /// Unbekanntes Gerät: nur mit gültigem Einmal-Code des Hauptgeräts hinein
    /// (bzw. hinaus, wenn wir selbst gerade koppeln und das Gegenüber den
    /// erwarteten Schlüssel hat). Schickt das Gegenüber gleich sein Hello, wird
    /// es zurückgegeben – ob es dazugehört, entscheidet dann dessen signierte Liste.
    async fn pair_unknown(
        &self,
        conn: &mut SecureConn,
        key: &str,
        initiated_by_me: bool,
        addr: SocketAddr,
    ) -> Result<Option<Frame>> {
        if initiated_by_me {
            let joining: Option<Joining> = self.trust.lock().unwrap().joining.clone();
            let Some(j) = joining.filter(|j| j.key == key) else {
                // Vielleicht steht es in einer neueren Liste, die es uns gleich schickt.
                return Ok(None);
            };
            conn.send(&Frame::PairRequest {
                token: j.token.clone(),
                device: self.me.clone(),
            })
            .await?;
            match conn.recv().await? {
                Some(Frame::PairResult { ok: true, .. }) => {
                    let mut t = self.trust.lock().unwrap();
                    t.joining = None;
                    // Ab jetzt gilt nur, was dieses Hauptgerät unterschreibt.
                    t.pinned = Some(j.sign_key.clone());
                    t.policy = None;
                    t.save(&self.trust_path)?;
                    *self.pairing_problem.lock().unwrap() = None;
                    info!("Gekoppelt mit {}", j.name);
                    Ok(None)
                }
                Some(Frame::PairResult { ok: false, message }) => {
                    *self.pairing_problem.lock().unwrap() = Some(message.clone());
                    bail!("Kopplung abgelehnt: {message}")
                }
                _ => bail!("Kopplung: keine Antwort"),
            }
        } else {
            let request = tokio::time::timeout(Duration::from_secs(10), conn.recv()).await;
            let (token, device) = match request {
                Ok(Ok(Some(Frame::PairRequest { token, device }))) => (token, device),
                Ok(Ok(Some(hello @ Frame::Hello { .. }))) => return Ok(Some(hello)),
                _ => bail!("unbekanntes Gerät von {addr} abgewiesen"),
            };
            let accepted: Result<()> = {
                let mut t = self.trust.lock().unwrap();
                if !t.take_invite(&token) {
                    Err(anyhow!(
                        "Kopplungscode ungültig oder abgelaufen – am Hauptgerät einen neuen Code anzeigen."
                    ))
                } else {
                    let member = member_from(key, &device, true, true);
                    t.edit(&self.cfg.device.signing_key, |p| {
                        p.members.retain(|m| m.key != member.key);
                        p.members.push(member);
                        Ok(())
                    })
                    .and_then(|_| t.save(&self.trust_path))
                }
            };
            if let Err(e) = accepted {
                let _ = conn
                    .send(&Frame::PairResult {
                        ok: false,
                        message: format!("{e:#}"),
                    })
                    .await;
                warn!("Kopplungsversuch von {addr} abgewiesen: {e:#}");
                bail!("Kopplung abgewiesen");
            }
            conn.send(&Frame::PairResult {
                ok: true,
                message: format!("Willkommen bei {}", self.me.name),
            })
            .await?;
            info!("Neues Gerät gekoppelt: {} ({addr})", device.name);
            self.emit(HudEvent::Reply {
                text: format!("Neues Gerät gekoppelt: {}", device.name),
                ok: true,
            });
            Ok(None)
        }
    }

    /// Schickt die aktuelle Geräteliste und Adressen an alle verbundenen Geräte
    /// (außer `except`, das sie gerade im Hello bekommen hat).
    fn broadcast_trust(&self, except: Option<&str>) {
        let (policy, addrs) = {
            let t = self.trust.lock().unwrap();
            (t.policy.clone(), t.addr_hints())
        };
        for p in self.peers.lock().unwrap().values() {
            if Some(p.key.as_str()) != except {
                let _ = p.tx.try_send(Frame::Sync {
                    policy: policy.clone(),
                    addrs: addrs.clone(),
                });
            }
        }
    }

    /// Trennt Verbindungen zu Geräten, die laut Liste nicht (mehr) dazugehören.
    fn drop_untrusted(&self) {
        let keys: Vec<String> = {
            let t = self.trust.lock().unwrap();
            self.peers
                .lock()
                .unwrap()
                .values()
                .filter(|p| !t.is_trusted(&p.key))
                .map(|p| p.key.clone())
                .collect()
        };
        self.disconnect_keys(&keys);
    }

    /// Trennt Verbindungen zu (gerade gesperrten) Schlüsseln.
    fn disconnect_keys(&self, keys: &[String]) {
        if keys.is_empty() {
            return;
        }
        let mut peers = self.peers.lock().unwrap();
        peers.retain(|_, p| {
            let drop = keys.contains(&p.key);
            if drop {
                p.kill.notify_one();
            }
            !drop
        });
    }

    /// Neuer Einmal-Code; liefert den Kopplungslink für QR-Code bzw. App.
    /// Nur das Hauptgerät kann Geräte aufnehmen.
    pub fn create_invite(&self, addrs: Option<Vec<String>>) -> Result<String> {
        let mut t = self.trust.lock().unwrap();
        if !t.is_admin(&self.me.sign_key) {
            bail!(
                "Neue Geräte koppelst du am Hauptgerät ({}).",
                t.admin_name().unwrap_or_else(|| "unbekannt".into())
            );
        }
        let invite = t.new_invite();
        t.save(&self.trust_path)?;
        Ok(crate::pairing::link_for(&self.cfg, &invite.token, addrs))
    }

    /// Ändert die signierte Geräteliste (nur am Hauptgerät) und verteilt sie.
    fn edit_policy(
        &self,
        change: impl FnOnce(&mut mira_protocol::Policy) -> Result<()>,
    ) -> Result<()> {
        {
            let mut t = self.trust.lock().unwrap();
            t.edit(&self.cfg.device.signing_key, change)?;
            t.save(&self.trust_path)?;
        }
        self.drop_untrusted();
        self.broadcast_trust(None);
        Ok(())
    }

    /// Schlüssel eines gekoppelten Geräts (ohne dieses) nach Name.
    fn member_key(&self, name: &str) -> Result<(String, String)> {
        let wanted = name.trim().to_lowercase();
        let t = self.trust.lock().unwrap();
        let p = t.policy().context("Noch nicht gekoppelt.")?;
        std::iter::once(&p.admin)
            .chain(&p.members)
            .find(|m| m.name.to_lowercase() == wanted)
            .map(|m| (m.key.clone(), m.name.clone()))
            .with_context(|| format!("Kein gekoppeltes Gerät heißt „{name}“."))
    }

    /// Sperrt ein Gerät dauerhaft; die Sperre verteilt sich an alle anderen.
    pub async fn revoke(&self, name: &str) -> Result<String> {
        let (key, shown) = self.member_key(name)?;
        if key == self.public_key() {
            bail!("Das Hauptgerät kann sich nicht selbst sperren.");
        }
        self.edit_policy(|p| {
            p.members.retain(|m| m.key != key);
            p.revoked.push(key.clone());
            Ok(())
        })?;
        Ok(format!(
            "{shown} ist gesperrt und kann sich nicht mehr verbinden."
        ))
    }

    /// Rechte eines Geräts ändern (nur am Hauptgerät).
    pub fn set_permissions(
        &self,
        name: &str,
        control: Option<bool>,
        files: Option<bool>,
        control_main: Option<bool>,
    ) -> Result<String> {
        let (key, shown) = self.member_key(name)?;
        if key == self.public_key() {
            bail!("Das Hauptgerät darf immer alles.");
        }
        let mut now = (false, false, false);
        self.edit_policy(|p| {
            let m = p
                .members
                .iter_mut()
                .find(|m| m.key == key)
                .context("Gerät nicht gefunden")?;
            m.control = control.unwrap_or(m.control);
            m.files = files.unwrap_or(m.files);
            m.control_main = control_main.unwrap_or(m.control_main);
            now = (m.control, m.files, m.control_main);
            Ok(())
        })?;
        let yes = |b: bool| if b { "ja" } else { "nein" };
        Ok(format!(
            "{shown}: andere Geräte steuern: {}, Hauptgerät steuern: {}, Dateien schicken: {}.",
            yes(now.0),
            yes(now.2),
            yes(now.1)
        ))
    }

    /// Gibt die Hauptrolle an ein anderes Gerät ab. Danach kann nur noch dieses
    /// Gerät koppeln, sperren und Rechte vergeben.
    pub fn make_admin(&self, name: &str) -> Result<String> {
        let (key, shown) = self.member_key(name)?;
        if key == self.public_key() {
            bail!("{shown} ist schon das Hauptgerät.");
        }
        self.edit_policy(|p| {
            let pos = p
                .members
                .iter()
                .position(|m| m.key == key)
                .context("Gerät nicht gefunden")?;
            let mut new_admin = p.members.remove(pos);
            if new_admin.sign_key.len() != 64 {
                bail!("{shown} hat eine zu alte Mira-Version – bitte erst aktualisieren.");
            }
            new_admin.control = true;
            new_admin.files = true;
            new_admin.control_main = true;
            let old = std::mem::replace(&mut p.admin, new_admin);
            p.members.push(old);
            Ok(())
        })?;
        Ok(format!("{shown} ist jetzt das Hauptgerät."))
    }

    /// Ist dieses Gerät das Hauptgerät?
    pub fn is_admin(&self) -> bool {
        self.trust.lock().unwrap().is_admin(&self.me.sign_key)
    }

    pub fn admin_name(&self) -> Option<String> {
        self.trust.lock().unwrap().admin_name()
    }

    /// Alle gekoppelten Geräte (auch offline) mit Rolle und Rechten.
    pub fn trusted_devices(&self) -> Vec<MemberStatus> {
        let connected: Vec<String> = self
            .peers
            .lock()
            .unwrap()
            .values()
            .map(|p| p.key.clone())
            .collect();
        let me = self.public_key();
        let t = self.trust.lock().unwrap();
        let Some(p) = t.policy() else { return vec![] };
        let mut list: Vec<MemberStatus> = std::iter::once(&p.admin)
            .chain(&p.members)
            .map(|m| MemberStatus {
                name: m.name.clone(),
                online: connected.contains(&m.key),
                this_device: m.key == me,
                admin: m.key == p.admin.key,
                control: m.control || m.key == p.admin.key,
                files: m.files || m.key == p.admin.key,
                control_main: m.control_main || m.key == p.admin.key,
            })
            .collect();
        list.sort_by(|a, b| b.admin.cmp(&a.admin).then(a.name.cmp(&b.name)));
        list
    }

    pub fn public_key(&self) -> String {
        self.identity.public_hex()
    }

    pub fn is_trusted_key(&self, key: &str) -> bool {
        self.trust.lock().unwrap().is_trusted(key)
    }

    pub fn pairing_problem(&self) -> Option<String> {
        self.pairing_problem.lock().unwrap().clone()
    }

    /// Ist dieses Gerät mit mindestens einem anderen gekoppelt bzw. dabei?
    pub fn pairing_pending(&self) -> bool {
        self.trust.lock().unwrap().joining.is_some()
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
        from: &str,
        from_key: &str,
    ) -> Option<Frame> {
        let denied = |what: Permission| {
            self.trust
                .lock()
                .unwrap()
                .allows(from_key, &self.identity.public_hex(), what)
                .err()
        };
        let fail = |id, e: anyhow::Error| {
            Some(Frame::Outcome {
                id,
                ok: false,
                message: format!("{e:#}"),
            })
        };
        match frame {
            Frame::Ping => Some(Frame::Pong),
            Frame::Pong
            | Frame::Hello { .. }
            | Frame::PairRequest { .. }
            | Frame::PairResult { .. } => None,
            Frame::Sync { policy, addrs } => {
                let (policy_changed, addrs_changed) = {
                    let mut t = self.trust.lock().unwrap();
                    let policy_changed = match policy.as_ref().map(|p| t.accept(p)) {
                        Some(Ok(c)) => c,
                        Some(Err(e)) => {
                            warn!("Geräteliste von {from} verworfen: {e:#}");
                            false
                        }
                        None => false,
                    };
                    let addrs_changed = t.merge_addrs(&self.identity.public_hex(), &addrs);
                    if policy_changed || addrs_changed {
                        let _ = t.save(&self.trust_path);
                    }
                    (policy_changed, addrs_changed)
                };
                if policy_changed {
                    self.drop_untrusted();
                }
                if policy_changed || addrs_changed {
                    // Weitergeben – nur bei echter Änderung, damit nichts endlos kreist.
                    self.broadcast_trust(None);
                }
                None
            }
            Frame::Interpret { id, text, here } => {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let answer = match self.interpret_with_brain(&text, &here).await {
                        Ok(Interpretation::Command { command, reply }) => Frame::Interpreted {
                            id,
                            ok: true,
                            command: Some(command),
                            reply,
                        },
                        Ok(Interpretation::Answer(reply)) => Frame::Interpreted {
                            id,
                            ok: true,
                            command: None,
                            reply,
                        },
                        Err(e) => Frame::Interpreted {
                            id,
                            ok: false,
                            command: None,
                            reply: format!("{e:#}"),
                        },
                    };
                    let _ = tx.send(answer).await;
                });
                None
            }
            Frame::Interpreted {
                id,
                ok,
                command,
                reply,
            } => {
                if let Some(waiter) = self.interpreting.lock().unwrap().remove(&id) {
                    let _ = waiter.send(Interpreted { ok, command, reply });
                }
                None
            }
            Frame::Execute { id, action } => {
                if let Some(why) = denied(Permission::Control) {
                    warn!("Befehl von {from} abgelehnt: {why}");
                    return Some(Frame::Outcome {
                        id,
                        ok: false,
                        message: why,
                    });
                }
                let tx = tx.clone();
                self.emit(HudEvent::Heard {
                    text: describe(&action),
                    source: Some(from.to_string()),
                });
                tokio::spawn(async move {
                    let (ok, message) = match self.execute_local(&action).await {
                        Ok(m) => (true, m),
                        Err(e) => (false, format!("{e:#}")),
                    };
                    self.emit(HudEvent::Reply {
                        text: message.clone(),
                        ok,
                    });
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
            } => match denied(Permission::Files) {
                Some(why) => {
                    warn!("Datei von {from} abgelehnt: {why}");
                    fail(id, anyhow!(why))
                }
                None => match Incoming::start(&name, size, self.host.receive_dir(folder.as_ref()))
                    .await
                {
                    Ok(inc) => {
                        incoming.insert(id, inc);
                        None
                    }
                    Err(e) => fail(id, e),
                },
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
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        self.emit(HudEvent::File {
                            name,
                            from: from.to_string(),
                        });
                        self.host.file_received(&path);
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
    /// Ereignis an alle angemeldeten Oberflächen (HUD, Apps).
    pub fn emit(&self, event: HudEvent) {
        let _ = self.events.send(event);
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<HudEvent> {
        self.events.subscribe()
    }

    pub async fn handle_text(&self, text: &str) -> Reply {
        self.handle_text_from(text, None).await
    }

    /// Wie [`Self::handle_text`]; `source` erscheint im HUD („Sprache", Gerätename).
    pub async fn handle_text_from(&self, text: &str, source: Option<&str>) -> Reply {
        self.emit(HudEvent::Heard {
            text: text.to_string(),
            source: source.map(str::to_string),
        });
        self.emit(HudEvent::Thinking);
        let reply = self.handle_text_inner(text).await;
        self.emit(HudEvent::Reply {
            text: reply.text.clone(),
            ok: reply.ok,
        });
        reply
    }

    async fn handle_text_inner(&self, text: &str) -> Reply {
        let started = std::time::Instant::now();
        let reply = match mira_intent::parse(text, &self.spoken_names()) {
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

    /// Deutet einen freien Satz: mit dem eigenen Sprachmodell oder – etwa auf
    /// dem Handy – mit dem eines verbundenen Geräts (z. B. Ollama auf dem PC).
    async fn ask_brain(&self, text: &str) -> Reply {
        let interpreted = if self.brain.is_some() {
            match self.interpret_with_brain(text, &self.me.name).await {
                Ok(Interpretation::Command { command, reply }) => Interpreted {
                    ok: true,
                    command: Some(command),
                    reply,
                },
                Ok(Interpretation::Answer(reply)) => Interpreted {
                    ok: true,
                    command: None,
                    reply,
                },
                // Eigenes Modell nicht erreichbar (z. B. kein Ollama auf dem Laptop):
                // das Sprachmodell eines anderen Geräts fragen.
                Err(e) => match self.interpret_remote(text).await {
                    Ok(i) => i,
                    Err(_) => Interpreted {
                        ok: false,
                        command: None,
                        reply: format!("{e}"),
                    },
                },
            }
        } else {
            match self.interpret_remote(text).await {
                Ok(i) => i,
                Err(e) => return Reply::err(format!("Das habe ich nicht verstanden. ({e})")),
            }
        };
        match interpreted {
            Interpreted {
                ok: false, reply, ..
            } => Reply::err(format!("Das habe ich nicht verstanden: {reply}")),
            Interpreted {
                command: None,
                reply,
                ..
            } => Reply::ok(reply),
            Interpreted {
                command: Some(command),
                reply,
                ..
            } => match self.run(&command).await {
                Ok(msg) if reply.is_empty() => Reply::ok(msg),
                Ok(_) => Reply::ok(reply),
                Err(e) => Reply::err(format!("{e:#}")),
            },
        }
    }

    async fn interpret_with_brain(&self, text: &str, here: &str) -> Result<Interpretation> {
        let brain = self
            .brain
            .as_ref()
            .context("kein Sprachmodell auf diesem Gerät")?;
        let devices: Vec<(String, String)> = self
            .devices()
            .into_iter()
            .map(|d| (d.name, format!("{:?}", d.platform)))
            .collect();
        brain
            .interpret(text, here, &devices)
            .await
            .with_context(|| format!("{} hilft gerade nicht weiter", brain.backend().describe()))
    }

    async fn interpret_remote(&self, text: &str) -> Result<Interpreted> {
        let peer = self
            .peers
            .lock()
            .unwrap()
            .values()
            .find(|p| p.info.capabilities.contains(&Capability::Brain))
            .cloned()
            .context("kein verbundenes Gerät mit Sprachmodell")?;
        let id = Uuid::new_v4();
        let (tx, rx) = oneshot::channel();
        self.interpreting.lock().unwrap().insert(id, tx);
        let sent = peer
            .tx
            .send(Frame::Interpret {
                id,
                text: text.to_string(),
                here: self.me.name.clone(),
            })
            .await;
        let result = match sent {
            Ok(()) => tokio::time::timeout(Duration::from_secs(90), rx).await,
            Err(_) => {
                self.interpreting.lock().unwrap().remove(&id);
                bail!("{} ist nicht mehr verbunden", peer.info.name);
            }
        };
        self.interpreting.lock().unwrap().remove(&id);
        match result {
            Ok(Ok(i)) => Ok(i),
            Ok(Err(_)) => bail!("Verbindung zu {} unterbrochen", peer.info.name),
            Err(_) => bail!("{} antwortet nicht", peer.info.name),
        }
    }

    /// Führt einen erkannten Befehl auf dem richtigen Gerät aus.
    pub async fn run(&self, cmd: &Command) -> Result<String> {
        let target = self.resolve(&cmd.target)?;
        match (&cmd.action, target) {
            (Action::SendFile { file, folder }, Resolved::Remote(peer)) => {
                let path = tokio::task::spawn_blocking({
                    let (file, host) = (file.clone(), self.host.clone());
                    move || host.locate_file(&file)
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
        // Name enthält das Wort („am PC" -> „Gaming-PC", „am iPad" -> „iPad Büro"):
        // bei genau einem Treffer gewinnt dieses Gerät vor den Gattungsnamen.
        let words = |n: &str| -> Vec<String> {
            n.to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .map(String::from)
                .collect()
        };
        let named: Vec<&Peer> = peers
            .iter()
            .filter(|p| words(&p.info.name).contains(&name))
            .collect();
        let me_named = words(&self.me.name).contains(&name);
        match (named.as_slice(), me_named) {
            ([one], false) => return Ok(Resolved::Remote((*one).clone())),
            ([], true) => return Ok(Resolved::Here),
            _ => {}
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
                let (app, host) = (app.clone(), self.host.clone());
                tokio::task::spawn_blocking(move || host.open_app(&app)).await?
            }
            Action::CloseApp { app } => {
                let (app, host) = (app.clone(), self.host.clone());
                tokio::task::spawn_blocking(move || host.close_app(&app)).await?
            }
            Action::Say { text } => {
                let (text, host) = (text.clone(), self.host.clone());
                tokio::task::spawn_blocking(move || host.say(&text)).await??;
                Ok("Gesagt.".into())
            }
            Action::ComputerUse { task } => {
                bail!("Computer Use ist noch nicht eingebaut (Aufgabe: „{task}“).")
            }
            Action::WebSearch { query } => {
                let (url, host) = (crate::web::google(query), self.host.clone());
                tokio::task::spawn_blocking(move || host.open_url(&url)).await??;
                Ok(format!("Ich suche nach „{query}“."))
            }
            Action::PlayMusic { query, service } => {
                let (query, service, host) = (query.clone(), service.clone(), self.host.clone());
                tokio::task::spawn_blocking(move || host.play_music(&query, service.as_deref()))
                    .await?
            }
            Action::PairDevice => {
                if !self.is_admin() {
                    bail!(
                        "Neue Geräte koppelst du am Hauptgerät ({}).",
                        self.admin_name().unwrap_or_else(|| "unbekannt".into())
                    );
                }
                let host = self.host.clone();
                tokio::task::spawn_blocking(move || host.show_pairing()).await?
            }
            Action::Media { command } => {
                let (command, host) = (*command, self.host.clone());
                tokio::task::spawn_blocking(move || host.media(command)).await?
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

/// Eine Datei, die gerade empfangen wird. Sie landet erst in `*.mira-part`
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
    async fn start(name: &str, size: u64, dir: Result<PathBuf>) -> Result<Self> {
        if size > files::MAX_RECEIVE_BYTES {
            bail!(
                "Datei zu groß ({} MB, höchstens {} MB)",
                size >> 20,
                files::MAX_RECEIVE_BYTES >> 20
            );
        }
        let name = files::safe_file_name(name)?;
        let dir = dir?;
        tokio::fs::create_dir_all(&dir).await?;
        let part = dir.join(format!(".{name}.{}.mira-part", Uuid::new_v4().simple()));
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
        files::mark_as_downloaded(&target);
        Ok(target)
    }
}
