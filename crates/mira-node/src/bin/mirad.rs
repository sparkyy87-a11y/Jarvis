//! `mirad` – der Mira-Dienst. Läuft auf jedem Gerät im Hintergrund.

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use mira_node::config::Config;
use mira_node::control::Request;
use mira_node::node::Node;
use mira_node::trust::{Joining, TrustStore};
use mira_node::{control, discovery};
use tracing::error;

#[derive(Parser)]
#[command(name = "mirad", about = "Mira-Dienst für dieses Gerät")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Dienst starten (Standard).
    Run,
    /// Erstes Gerät einrichten – es wird das Hauptgerät (koppelt, sperrt, vergibt Rechte).
    Init {
        /// Name dieses Geräts, z. B. "Gaming-PC" oder "Laptop".
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        opts: SetupOpts,
    },
    /// Dieses Gerät mit einem vorhandenen Mira koppeln (Code aus `mirad pair`).
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
    /// Ein Gerät dauerhaft sperren (verloren, verkauft …). Nur am Hauptgerät.
    Entfernen { name: String },
    /// Rechte eines Geräts ändern (nur am Hauptgerät), z. B.
    /// `mirad rechte PC --steuern nein --dateien ja` oder
    /// `mirad rechte iPhone --hauptgeraet ja`.
    Rechte {
        name: String,
        /// Darf dieses Gerät andere Geräte steuern?
        #[arg(long, value_enum)]
        steuern: Option<JaNein>,
        /// Darf dieses Gerät Dateien an andere schicken?
        #[arg(long, value_enum)]
        dateien: Option<JaNein>,
        /// Darf dieses Gerät auch das Hauptgerät steuern? (für deine eigenen Geräte)
        #[arg(long, value_enum)]
        hauptgeraet: Option<JaNein>,
    },
    /// Hauptrolle an ein anderes Gerät abgeben (nur am Hauptgerät).
    Hauptgeraet { name: String },
    /// Mira beim Anmelden automatisch starten: an, aus oder status.
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
            cfg.brain.provider = mira_node::config::BrainProvider::Off;
        }
    }
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum JaNein {
    Ja,
    Nein,
}

impl From<JaNein> for bool {
    fn from(v: JaNein) -> bool {
        matches!(v, JaNein::Ja)
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
                let role = if d.admin {
                    "Hauptgerät".to_string()
                } else {
                    let mut r = vec![];
                    r.push(if d.control {
                        "steuert andere"
                    } else {
                        "steuert nichts"
                    });
                    if d.control_main {
                        r.push("auch das Hauptgerät");
                    }
                    r.push(if d.files {
                        "schickt Dateien"
                    } else {
                        "keine Dateien"
                    });
                    r.join(", ")
                };
                println!(
                    "• {}{} – {role}{}",
                    d.name,
                    if d.this_device {
                        " (dieses Gerät)"
                    } else {
                        ""
                    },
                    if d.online { "  · verbunden" } else { "" }
                );
            }
            Ok(())
        }
        Cmd::Entfernen { name } => answer(&Request::Revoke { name }),
        Cmd::Rechte {
            name,
            steuern,
            dateien,
            hauptgeraet,
        } => {
            if steuern.is_none() && dateien.is_none() && hauptgeraet.is_none() {
                bail!("Bitte --steuern, --hauptgeraet und/oder --dateien mit ja|nein angeben.");
            }
            answer(&Request::Permissions {
                name,
                control: steuern.map(Into::into),
                files: dateien.map(Into::into),
                main: hauptgeraet.map(Into::into),
            })
        }
        Cmd::Hauptgeraet { name } => answer(&Request::MakeAdmin { name }),
    }
}

/// Anfrage stellen und Antwort ausgeben; Fehler als Fehler melden.
fn answer(req: &Request) -> Result<()> {
    let resp = ask(req)?;
    let text = resp.text.unwrap_or_default();
    if !resp.ok {
        bail!("{text}");
    }
    println!("{text}");
    Ok(())
}

/// Anfrage an den laufenden Dienst.
fn ask(req: &Request) -> Result<mira_node::control::Response> {
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
        "Mit der Handykamera scannen – die Mira-App öffnet sich und koppelt sich mit „{}“.",
        cfg.device.name
    );
    println!("Auf einem PC/Laptop stattdessen ausführen:\n  mirad join \"{link}\"\n");
    println!("Der Code gilt 10 Minuten und nur für ein Gerät.");
    Ok(())
}

fn autostart_cmd(mode: AutostartMode) -> Result<()> {
    use mira_node::autostart;
    match mode {
        AutostartMode::An => {
            autostart::enable()?;
            println!("Mira startet jetzt und künftig bei jeder Anmeldung (ohne Fenster).");
        }
        AutostartMode::Aus => {
            autostart::disable()?;
            println!("Autostart entfernt, Mira im Hintergrund beendet.");
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
    println!("„{}“ ist das Hauptgerät: nur hier koppelst du neue Geräte, sperrst sie und vergibst Rechte.", cfg.device.name);
    println!("\nWeitere Geräte koppeln: Dienst starten (`mirad` oder `mirad autostart an`),");
    println!("dann `mirad pair` – zeigt einen QR-Code mit Einmal-Code.");
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
    let p = mira_node::pairing::parse(code)?;
    let name = name.unwrap_or_else(|| gethostname::gethostname().to_string_lossy().to_string());
    let mut cfg = Config::new(name)?;
    opts.apply(&mut cfg);
    cfg.mesh.static_peers = p.addrs.clone();
    cfg.save()?;
    let trust = TrustStore {
        joining: Some(Joining {
            token: p.token,
            key: p.key,
            sign_key: p.sign_key,
            name: p.name.clone(),
        }),
        ..Default::default()
    };
    trust.save(&cfg.trust_path())?;
    println!(
        "Eingerichtet: „{}“. Beim nächsten Start von mirad koppelt es sich mit „{}“.",
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
