//! Handy und iPad sprechen direkt miteinander, nachdem der PC aus ist.

use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mira_mobile::{pair, Mira, PlatformBridge, Reply};
use mira_node::config::{BrainProvider, Config};
use mira_node::node::Node;

#[derive(Default)]
struct Bridge {
    opened: Mutex<Vec<String>>,
}

impl PlatformBridge for Bridge {
    fn open_app(&self, app: String) -> Reply {
        self.opened.lock().unwrap().push(app.clone());
        Reply {
            ok: true,
            text: format!("{app} geöffnet"),
        }
    }
    fn speak(&self, _text: String) {}
    fn file_received(&self, _path: String) {}
    fn open_url(&self, _url: String) -> Reply {
        Reply {
            ok: true,
            text: String::new(),
        }
    }
    fn play_music(&self, _q: String, _s: String) -> Reply {
        Reply {
            ok: true,
            text: String::new(),
        }
    }
    fn media(&self, c: String) -> Reply {
        Reply { ok: true, text: c }
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let t = Instant::now();
    while !ok() {
        assert!(
            t.elapsed() < Duration::from_secs(20),
            "Zeitüberschreitung: {what}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Koppelt ein Gerät wie nach dem QR-Scan und startet es.
fn device(
    pc: &Node,
    pc_port: u16,
    name: &str,
    platform: &str,
    bridge: Arc<Bridge>,
) -> (Arc<Mira>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().to_string_lossy().to_string();
    let code = pc
        .create_invite(Some(vec![format!("127.0.0.1:{pc_port}")]))
        .unwrap();
    pair(d.clone(), name.into(), platform.into(), code).unwrap();
    let path = dir.path().join("mira.toml");
    let mut cfg = Config::load_from(&path).unwrap();
    cfg.mesh.mdns = false; // wie auf dem iPhone: nur gelernte Adressen
    cfg.mesh.port = free_port();
    cfg.save_to(&path).unwrap();
    (Mira::start(d, None, bridge).unwrap(), dir)
}

#[test]
fn ipad_and_android_without_pc() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());

    let mut pc_cfg = Config::new("Gaming-PC".into()).unwrap();
    pc_cfg.mesh.port = free_port();
    pc_cfg.mesh.mdns = false;
    pc_cfg.brain.provider = BrainProvider::Off;
    pc_cfg.save_to(&home.path().join("pc.toml")).unwrap();
    let pc_rt = tokio::runtime::Runtime::new().unwrap();
    let pc = Node::new(pc_cfg.clone()).unwrap();
    pc_rt.spawn(pc.clone().listen());

    let handy_bridge = Arc::new(Bridge::default());
    let (handy, _h) = device(
        &pc,
        pc_cfg.mesh.port,
        "Handy",
        "android",
        handy_bridge.clone(),
    );
    wait("Handy mit PC verbunden", || {
        handy.devices().iter().any(|d| d.name == "Gaming-PC")
    });
    let (ipad, _i) = device(
        &pc,
        pc_cfg.mesh.port,
        "iPad Büro",
        "ipados",
        Arc::new(Bridge::default()),
    );
    wait("iPad mit PC verbunden", || {
        ipad.devices().iter().any(|d| d.name == "Gaming-PC")
    });

    // PC aus.
    pc_rt.shutdown_background();
    wait("PC ist weg", || {
        ipad.devices().iter().all(|d| d.name != "Gaming-PC")
    });

    // Das iPad hat die Adresse des Handys vom PC gelernt und verbindet sich direkt.
    wait("iPad findet das Handy ohne PC", || {
        ipad.devices().iter().any(|d| d.name == "Handy")
    });
    let r = ipad.ask("öffne Spotify am Handy".into());
    assert!(r.ok, "{}", r.text);
    assert_eq!(
        *handy_bridge.opened.lock().unwrap(),
        vec!["Spotify".to_string()]
    );
}
