//! `jarvisd` – der Jarvis-Dienst. Läuft auf jedem Gerät im Hintergrund.

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use jarvis_node::config::Config;
use jarvis_node::node::Node;
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
    /// Erstes Gerät einrichten: erzeugt Konfiguration und Pairing-Schlüssel.
    Init {
        /// Name dieses Geräts, z. B. "Gaming-PC".
        #[arg(long)]
        name: Option<String>,
    },
    /// Weiteres Gerät einrichten mit dem Schlüssel des ersten Geräts.
    Join {
        key: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Pairing-Schlüssel anzeigen (für `jarvisd join` auf anderen Geräten).
    Key,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    match Cli::parse().cmd.unwrap_or(Cmd::Run) {
        Cmd::Init { name } => setup(name, None),
        Cmd::Join { key, name } => setup(name, Some(key)),
        Cmd::Key => {
            println!("{}", Config::load()?.mesh.pairing_key);
            Ok(())
        }
        Cmd::Run => run().await,
    }
}

fn setup(name: Option<String>, key: Option<String>) -> Result<()> {
    let path = Config::path();
    if path.exists() {
        bail!(
            "{} existiert schon – zum Neu-Einrichten löschen",
            path.display()
        );
    }
    let name = name.unwrap_or_else(|| gethostname::gethostname().to_string_lossy().to_string());
    let joining = key.is_some();
    let cfg = Config::new(name, key)?;
    cfg.save()?;
    println!("Eingerichtet: „{}“ ({})", cfg.device.name, path.display());
    if !joining {
        println!(
            "\nAuf jedem weiteren Gerät ausführen:\n  jarvisd join {}\n",
            cfg.mesh.pairing_key
        );
        println!("Den Schlüssel wie ein Passwort behandeln.");
    }
    Ok(())
}

async fn run() -> Result<()> {
    let cfg = Config::load()?;
    let node = Node::new(cfg.clone())?;

    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(node.clone().listen());
    tasks.spawn(control::serve(node.clone(), cfg.mesh.control_port));
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
