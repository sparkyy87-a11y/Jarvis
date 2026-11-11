//! `jarvisd` – der Jarvis-Dienst. Läuft auf jedem Gerät im Hintergrund.

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use jarvis_node::config::Config;
use jarvis_node::control::Request;
use jarvis_node::node::Node;
use jarvis_node::trust::{Joining, TrustStore};
use jarvis_node::{control, discovery};
use tracing::error;

#[derive(Parser)]
#[command(name = "jarvisd", about = "Jarvis-Dienst für dieses Gerät")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Dienst starten (Standard).
    Run,
    /// Erstes Gerät einrichten (eigener Geräteschlüssel wird erzeugt).
    Init {
        /// Name dieses Geräts, z. B. "Gaming-PC" oder "Laptop".
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        opts: SetupOpts,
    },
    /// Dieses Gerät mit einem vorhandenen Jarvis koppeln (Code aus `jarvisd pair`).
    Join {
        code: String,
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        opts: SetupOpts,
    },
    /// Einmal-Code und QR-Code zum Koppeln eines weiteren Geräts anzeigen.
    Pair,
    /// Alle gekoppelten Geräte anzeigen (auch offline).
    Geraete,
    /// Ein Gerät dauerhaft sperren (verloren, verkauft …).
    Entfernen { name: String },
    /// Jarvis beim Anmelden automatisch starten: an, aus oder status.
    Autostart {
        #[arg(value_enum, default_value = "status")]
        mode: AutostartMode,
    },
}

/// Auswahl aus dem Installer.
#[derive(clap::Args, Clone)]
struct SetupOpts {
    /// Spracheingabe nicht automatisch starten.
    #[arg(long)]
    ohne_sprache: bool,
    /// HUD nicht automatisch starten.
    #[arg(long)]
    ohne_hud: bool,
    /// Lokales Sprachmodell (Ollama), z. B. "qwen2.5:7b".
    #[arg(long)]
    sprachmodell: Option<String>,
    /// Kein eigenes Sprachmodell – freie Sätze deutet ein anderes Gerät (z. B. der PC).
    #[arg(long, conflicts_with = "sprachmodell")]
    ohne_sprachmodell: bool,
}

impl SetupOpts {
    fn apply(self, cfg: &mut Config) {
        cfg.voice.enabled = !self.ohne_sprache;
        cfg.hud.enabled = !self.ohne_hud;
        if let Some(m) = self.sprachmodell {
            cfg.brain.model = Some(m);
        }
        if self.ohne_sprachmodell {
            cfg.brain.provider = jarvis_node::config::BrainProvider::Off;
        }
    }
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum AutostartMode {
    An,
    Aus,
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    match Cli::parse().cmd.unwrap_or(Cmd::Run) {
        Cmd::Init { name, opts } => init(name, opts),
        Cmd::Join { code, name, opts } => join(&code, name, opts),
        Cmd::Run => run().await,
        Cmd::Autostart { mode } => autostart_cmd(mode),
        Cmd::Pair => pair(),
        Cmd::Geraete => {
            let resp = ask(&Request::Trusted)?;
            for d in resp.trusted.unwrap_or_default() {
                println!(
                    "• {}{}",
                    d.name,
                    if d.online { "  (verbunden)" } else { "" }
                );
            }
            Ok(())
        }
        Cmd::Entfernen { name } => {
            let resp = ask(&Request::Revoke { name })?;
            println!("{}", resp.text.unwrap_or_default());
            Ok(())
        }
    }
}

/// Anfrage an den laufenden Dienst.
fn ask(req: &Request) -> Result<jarvis_node::control::Response> {
    let cfg = Config::load()?;
    control::request_blocking(cfg.mesh.control_port, &cfg.control_token_path(), req)
}

fn pair() -> Result<()> {
    let cfg = Config::load()?;
    let resp = ask(&Request::Pair { addrs: None })?;
    let Some(link) = resp.link else {
        bail!(
            "{}",
            resp.text
                .unwrap_or_else(|| "Kein Kopplungscode erhalten".into())
        );
    };
    let qr = qrcode::QrCode::new(link.as_bytes())?;
    let art = qr
        .render::<qrcode::render::unicode::Dense1x2>()
        .dark_color(qrcode::render::unicode::Dense1x2::Light)
        .light_color(qrcode::render::unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build();
    println!("{art}");
    println!(
        "Mit der Handykamera scannen – die Jarvis-App öffnet sich und koppelt sich mit „{}“.",
        cfg.device.name
    );
    println!("Auf einem PC/Laptop stattdessen ausführen:\n  jarvisd join \"{link}\"\n");
    println!("Der Code gilt 10 Minuten und nur für ein Gerät.");
    Ok(())
}

fn autostart_cmd(mode: AutostartMode) -> Result<()> {
    use jarvis_node::autostart;
    match mode {
        AutostartMode::An => {
            autostart::enable()?;
            println!("Jarvis startet jetzt und künftig bei jeder Anmeldung (ohne Fenster).");
        }
        AutostartMode::Aus => {
            autostart::disable()?;
            println!("Autostart entfernt, Jarvis im Hintergrund beendet.");
        }
        AutostartMode::Status => {}
    }
    let s = autostart::status()?;
    println!(
        "Autostart: {} · läuft: {} · Logs: {}",
        if s.enabled { "an" } else { "aus" },
        if s.running { "ja" } else { "nein" },
        s.logs.display()
    );
    Ok(())
}

fn init(name: Option<String>, opts: SetupOpts) -> Result<()> {
    let path = Config::path();
    if path.exists() {
        bail!(
            "{} existiert schon – zum Neu-Einrichten löschen",
            path.display()
        );
    }
    let name = name.unwrap_or_else(|| gethostname::gethostname().to_string_lossy().to_string());
    let mut cfg = Config::new(name)?;
    opts.apply(&mut cfg);
    cfg.save()?;
    println!("Eingerichtet: „{}“ ({})", cfg.device.name, path.display());
    println!("\nWeitere Geräte koppeln: Dienst starten (`jarvisd` oder `jarvisd autostart an`),");
    println!("dann `jarvisd pair` – zeigt einen QR-Code mit Einmal-Code.");
    Ok(())
}

fn join(code: &str, name: Option<String>, opts: SetupOpts) -> Result<()> {
    let path = Config::path();
    if path.exists() {
        bail!(
            "{} existiert schon – zum Neu-Einrichten löschen",
            path.display()
        );
    }
    let p = jarvis_node::pairing::parse(code)?;
    let name = name.unwrap_or_else(|| gethostname::gethostname().to_string_lossy().to_string());
    let mut cfg = Config::new(name)?;
    opts.apply(&mut cfg);
    cfg.mesh.static_peers = p.addrs.clone();
    cfg.save()?;
    let trust = TrustStore {
        joining: Some(Joining {
            token: p.token,
            key: p.key,
            name: p.name.clone(),
        }),
        ..Default::default()
    };
    trust.save(&cfg.trust_path())?;
    println!(
        "Eingerichtet: „{}“. Beim nächsten Start von jarvisd koppelt es sich mit „{}“.",
        cfg.device.name, p.name
    );
    Ok(())
}

async fn run() -> Result<()> {
    let cfg = Config::load()?;
    let node = Node::new(cfg.clone())?;

    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(node.clone().listen());
    let token = control::load_or_create_token(&cfg.control_token_path())?;
    tasks.spawn(control::serve(node.clone(), cfg.mesh.control_port, token));
    if cfg.mesh.mdns {
        tasks.spawn(discovery::run(node.clone(), cfg.mesh.port));
    }
    let n = node.clone();
    tasks.spawn(async move {
        n.maintain_static_peers().await;
        Ok(())
    });

    tokio::select! {
        Some(res) = tasks.join_next() => {
            match res {
                Ok(Err(e)) => error!("{e:#}"),
                Err(e) => error!("{e}"),
                Ok(Ok(())) => {}
            }
            bail!("Ein Teil des Dienstes ist ausgefallen");
        }
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}
