//! Handy (über die Mobil-Schnittstelle) und PC (echter Knoten mit Ollama-Attrappe)
//! im selben Prozess: Kopplung, freier Satz über das Sprachmodell des PCs,
//! Ausführung auf dem Handy, Datei vom Handy an den PC.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jarvis_mobile::{is_paired, pair, Jarvis, PlatformBridge, Reply};
use jarvis_node::config::{BrainProvider, Config};
use jarvis_node::node::Node;

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
}

/// Antwortet auf jede Anfrage wie Ollama mit „öffne Spotify“.
fn fake_ollama() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let mut reader = BufReader::new(s.try_clone().unwrap());
            let mut len = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            let content = r#"{"action":"open_app","target_device":"","app":"Spotify","file":"","folder":"","text":"","reply":"Ich mache dir Spotify an."}"#;
            let json = format!(
                r#"{{"message":{{"role":"assistant","content":{}}},"done":true}}"#,
                serde_json_string(content)
            );
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{json}", json.len());
        }
    });
    format!("http://{addr}")
}

fn serde_json_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn phone_uses_pc_brain_and_sends_files() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());

    // PC mit Sprachmodell
    let mut pc_cfg = Config::new("Gaming-PC".into(), None).unwrap();
    pc_cfg.mesh.port = free_port();
    pc_cfg.mesh.mdns = false;
    pc_cfg.brain.provider = BrainProvider::Ollama;
    pc_cfg.brain.ollama_url = Some(fake_ollama());
    let pc_rt = tokio::runtime::Runtime::new().unwrap();
    let pc = Node::new(pc_cfg.clone()).unwrap();
    pc_rt.spawn(pc.clone().listen());

    // Handy koppeln wie nach dem QR-Scan
    let phone_dir = tempfile::tempdir().unwrap();
    let d = phone_dir.path().to_string_lossy().to_string();
    let code = format!(
        "jarvis://pair?v=1&key={}&name=Gaming-PC&addr=127.0.0.1:{}",
        pc_cfg.mesh.pairing_key, pc_cfg.mesh.port
    );
    pair(d.clone(), "Handy".into(), "android".into(), code).unwrap();
    assert!(is_paired(d.clone()));
    let cfg_path = phone_dir.path().join("jarvis.toml");
    let mut phone_cfg = Config::load_from(&cfg_path).unwrap();
    phone_cfg.mesh.mdns = false;
    phone_cfg.mesh.port = free_port();
    phone_cfg.save_to(&cfg_path).unwrap();

    let bridge = Arc::new(Bridge::default());
    let phone = Jarvis::start(d, None, bridge.clone()).unwrap();
    let t = Instant::now();
    while !phone.devices().iter().any(|d| d.name == "Gaming-PC") {
        assert!(
            t.elapsed() < Duration::from_secs(10),
            "Handy findet den PC nicht"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    // Freier Satz: das Handy versteht ihn nicht selbst, fragt den PC, führt selbst aus.
    let r = phone.ask("Mach mir Musik zum Arbeiten an".into());
    assert!(r.ok, "{}", r.text);
    assert_eq!(r.text, "Ich mache dir Spotify an.");
    assert_eq!(*bridge.opened.lock().unwrap(), vec!["Spotify".to_string()]);

    // Datei aus dem Teilen-Menü an den PC
    let photo = phone_dir.path().join("urlaub.jpg");
    std::fs::write(&photo, b"JPEG...").unwrap();
    let r = phone.send_file(
        photo.to_string_lossy().into(),
        "pc".into(),
        Some("Bilder".into()),
    );
    assert!(r.ok, "{}", r.text);
    let received = home.path().join("Pictures").join("urlaub.jpg");
    assert_eq!(std::fs::read(received).unwrap(), b"JPEG...");
}
