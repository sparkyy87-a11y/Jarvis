//! Der Jarvis-Dienst, der auf jedem Gerät läuft.
//!
//! - [`transport`]: Noise-verschlüsselte Verbindung zwischen zwei Geräten
//! - [`node`]: Geräteliste, Befehls-Routing, Dateiübertragung
//! - [`exec`]: lokale Ausführung (Apps öffnen/schließen, sprechen)
//! - [`files`]: Dateien finden und sicher empfangen
//! - [`discovery`]: Geräte im LAN per mDNS finden
//! - [`control`]: lokale Schnittstelle für CLI, Sprach- und App-Oberflächen

pub mod config;
pub mod control;
pub mod discovery;
pub mod exec;
pub mod files;
pub mod node;
pub mod transport;
