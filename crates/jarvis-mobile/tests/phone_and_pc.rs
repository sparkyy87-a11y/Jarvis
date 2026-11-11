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
    let mut pc_cfg = Config::new("Gaming-PC".into()).unwrap();
    pc_cfg.mesh.port = free_port();
    pc_cfg.mesh.mdns = false;
    pc_cfg.brain.provider = BrainProvider::Ollama;
    pc_cfg.brain.ollama_url = Some(fake_ollama());
    pc_cfg.save_to(&home.path().join("pc.toml")).unwrap();
    let pc_rt = tokio::runtime::Runtime::new().unwrap();
    let pc = Node::new(pc_cfg.clone()).unwrap();
    pc_rt.spawn(pc.clone().listen());

    // Handy koppeln wie nach dem QR-Scan
    let phone_dir = tempfile::tempdir().unwrap();
    let d = phone_dir.path().to_string_lossy().to_string();
    // Wie `jarvisd pair` am PC: Einmal-Code erzeugen
    let code = pc
        .create_invite(Some(vec![format!("127.0.0.1:{}", pc_cfg.mesh.port)]))
        .unwrap();
    pair(d.clone(), "Handy".into(), "android".into(), code.clone()).unwrap();
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
    // Genauer Name: auf dem Mac-Runner hört der „PC" nicht auf „pc".
    let photo_name = format!("jarvis-test-{}.jpg", std::process::id());
    let photo = phone_dir.path().join(&photo_name);
    std::fs::write(&photo, b"JPEG...").unwrap();
    let r = phone.send_file(
        photo.to_string_lossy().into(),
        "Gaming-PC".into(),
        Some("Bilder".into()),
    );
    assert!(r.ok, "{}", r.text);
    // Windows/macOS ermitteln „Bilder" über das System, nicht über HOME.
    let pictures =
        jarvis_node::files::resolve_folder(Some(&jarvis_protocol::Folder::Pictures)).unwrap();
    let received = pictures.join(&photo_name);
    let content = std::fs::read(&received);
    let _ = std::fs::remove_file(&received);
    assert_eq!(content.unwrap(), b"JPEG...");

    // Vom PC aus Musik auf dem Handy starten und dort pausieren.
    let r = pc_rt.block_on(pc.handle_text("spiel Musik von Adele auf dem Handy"));
    assert!(r.ok, "{}", r.text);
    assert!(bridge
        .opened
        .lock()
        .unwrap()
        .contains(&"apple-music:Adele".to_string()));
    let r = pc_rt.block_on(pc.handle_text("Pause am Handy"));
    assert!(r.ok && r.text.ends_with("pause"), "{}", r.text);

    // --- Hauptgerät und Rechte ---------------------------------------------
    // Der PC hat Jarvis zuerst eingerichtet und ist Hauptgerät: das Handy darf
    // ihn erst steuern, wenn er es ausdrücklich erlaubt.
    let r = phone.ask("öffne Steam am Gaming-PC".into());
    assert!(!r.ok && r.text.contains("Hauptgerät"), "{}", r.text);
    pc.set_permissions("Handy", None, None, Some(true)).unwrap();
    let r = phone.ask("öffne Steam am Gaming-PC".into());
    assert!(!r.text.contains("darf"), "{}", r.text);
    // Dateien schicken verbieten -> wird am PC abgelehnt.
    pc.set_permissions("Handy", None, Some(false), None)
        .unwrap();
    let r = phone.send_file(
        photo.to_string_lossy().into(),
        "Gaming-PC".into(),
        Some("Bilder".into()),
    );
    assert!(!r.ok && r.text.contains("keine Dateien"), "{}", r.text);
    let _ = std::fs::remove_file(pictures.join(&photo_name));
    // Das Handy selbst kann keine Rechte ändern oder Geräte aufnehmen.
    let t = Instant::now();
    loop {
        let members = phone.members();
        let me = members.iter().find(|m| m.this_device);
        if members[0].name == "Gaming-PC"
            && members[0].admin
            && me.is_some_and(|m| m.control_main && !m.files && !m.admin)
        {
            break;
        }
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "Handy kennt die neuen Rechte nicht: {members:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    // --- Sicherheit ---------------------------------------------------------
    // Ein zweites Gerät mit demselben (schon verbrauchten) Code kommt nicht hinein.
    let thief_dir = tempfile::tempdir().unwrap();
    let td = thief_dir.path().to_string_lossy().to_string();
    pair(td.clone(), "Fremd".into(), "android".into(), code).unwrap();
    let tpath = thief_dir.path().join("jarvis.toml");
    let mut tcfg = Config::load_from(&tpath).unwrap();
    tcfg.mesh.mdns = false;
    tcfg.mesh.port = free_port();
    tcfg.save_to(&tpath).unwrap();
    let thief = Jarvis::start(td, None, Arc::new(Bridge::default())).unwrap();
    let t = Instant::now();
    while thief.status().is_none_or(|s| !s.contains("ungültig")) {
        assert!(
            t.elapsed() < Duration::from_secs(10),
            "zweite Kopplung mit demselben Code muss scheitern: {:?}",
            thief.status()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(thief.devices().iter().all(|d| d.name != "Gaming-PC"));
    assert_eq!(
        phone.status(),
        None,
        "das echte Handy ist ordentlich gekoppelt"
    );

    // Handy verloren: am PC sperren -> Verbindung weg, und es kommt nicht wieder.
    let r = pc_rt.block_on(pc.revoke("Handy")).unwrap();
    assert!(r.contains("gesperrt"), "{r}");
    let t = Instant::now();
    while phone.devices().iter().any(|d| d.name == "Gaming-PC") {
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "gesperrtes Handy ist noch verbunden"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_secs(4)); // mehrere Wiederverbindungsversuche
    assert!(
        phone.devices().iter().all(|d| d.name != "Gaming-PC"),
        "gesperrtes Handy hat sich wieder verbunden"
    );
    assert!(pc.trusted_devices().iter().all(|d| d.name != "Handy"));
}
