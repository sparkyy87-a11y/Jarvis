//! `jarvis-hintergrund` – startet `jarvisd` und `jarvis-listen` unsichtbar und
//! startet sie nach einem Absturz neu. Wird von `jarvisd autostart an`
//! eingetragen; Ausgaben landen in Logdateien.

// Unter Windows ohne Konsolenfenster.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use jarvis_node::autostart::{logs_dir, quiet, sibling_exe, SUPERVISOR_PORT};
use jarvis_node::config::Config;

const MAX_LOG: u64 = 5 * 1024 * 1024;

struct Supervised {
    name: &'static str,
    args: &'static [&'static str],
    exe: PathBuf,
    child: Option<Child>,
    started: Instant,
    next_start: Instant,
    backoff: Duration,
}

impl Supervised {
    fn new(name: &'static str, args: &'static [&'static str]) -> Option<Self> {
        let exe = sibling_exe(name).ok().filter(|p| p.is_file())?;
        let now = Instant::now();
        Some(Self {
            name,
            args,
            exe,
            child: None,
            started: now,
            next_start: now,
            backoff: Duration::from_secs(2),
        })
    }

    fn tick(&mut self, log: &mut File) {
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(None) => return,
                Ok(Some(status)) => note(log, &format!("{} beendet ({status})", self.name)),
                Err(e) => note(log, &format!("{}: {e}", self.name)),
            }
            self.child = None;
            // Lief es eine Weile stabil, schnell neu starten; sonst immer länger warten.
            self.backoff = if self.started.elapsed() > Duration::from_secs(60) {
                Duration::from_secs(2)
            } else {
                (self.backoff * 2).min(Duration::from_secs(60))
            };
            self.next_start = Instant::now() + self.backoff;
        }
        if Instant::now() < self.next_start {
            return;
        }
        match open_log(&logs_dir().join(format!("{}.log", self.name))) {
            Ok(out) => {
                let err = out.try_clone();
                let mut cmd = Command::new(&self.exe);
                quiet(cmd.args(self.args).stdin(Stdio::null()).stdout(out));
                if let Ok(err) = err {
                    cmd.stderr(err);
                }
                match cmd.spawn() {
                    Ok(c) => {
                        note(log, &format!("{} gestartet (PID {})", self.name, c.id()));
                        self.child = Some(c);
                        self.started = Instant::now();
                    }
                    Err(e) => {
                        note(log, &format!("{} startet nicht: {e}", self.name));
                        self.next_start = Instant::now() + Duration::from_secs(60);
                    }
                }
            }
            Err(e) => note(log, &format!("Log für {} nicht anlegbar: {e}", self.name)),
        }
    }

    fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Öffnet eine Logdatei zum Anhängen; ab 5 MB wird die alte zu `.old`.
fn open_log(path: &Path) -> std::io::Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if path.metadata().map(|m| m.len() > MAX_LOG).unwrap_or(false) {
        let _ = std::fs::rename(path, path.with_extension("log.old"));
    }
    OpenOptions::new().create(true).append(true).open(path)
}

fn note(log: &mut File, msg: &str) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let _ = writeln!(log, "[{secs}] {msg}");
}

fn main() {
    // Läuft schon eine Instanz? Dann still beenden.
    let Ok(lock) = TcpListener::bind(("127.0.0.1", SUPERVISOR_PORT)) else {
        return;
    };
    let Ok(mut log) = open_log(&logs_dir().join("jarvis-hintergrund.log")) else {
        return;
    };
    note(&mut log, "jarvis-hintergrund gestartet");

    // `jarvisd autostart aus` schickt "stop".
    let stop = Arc::new(AtomicBool::new(false));
    let (stop_flag, stop_ack) = (stop.clone(), Arc::new(std::sync::Mutex::new(Vec::new())));
    let acks = stop_ack.clone();
    std::thread::spawn(move || {
        for conn in lock.incoming().flatten() {
            let mut line = String::new();
            let _ = conn.set_read_timeout(Some(Duration::from_secs(2)));
            if BufReader::new(&conn).read_line(&mut line).is_ok() && line.trim() == "stop" {
                stop_flag.store(true, Ordering::SeqCst);
                acks.lock().unwrap().push(conn);
            }
        }
    });

    let voice = Config::load().map(|c| c.voice.enabled).unwrap_or(true);
    let mut services: Vec<Supervised> = [Supervised::new("jarvisd", &["run"])]
        .into_iter()
        .chain(voice.then(|| Supervised::new("jarvis-listen", &[])))
        .flatten()
        .collect();
    if services.is_empty() {
        note(&mut log, "jarvisd nicht gefunden – nichts zu tun");
        return;
    }

    while !stop.load(Ordering::SeqCst) {
        for s in &mut services {
            s.tick(&mut log);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    for s in &mut services {
        s.stop();
    }
    note(&mut log, "beendet auf Wunsch");
    for mut conn in stop_ack.lock().unwrap().drain(..) {
        let _ = conn.write_all(b"ok\n");
    }
}
