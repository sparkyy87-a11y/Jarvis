//! Was ein Gerät *kann*: Apps öffnen, sprechen, Dateien ablegen.
//!
//! Der Desktop nutzt [`DesktopHost`]; die Handy-Apps liefern eine eigene
//! Umsetzung (Android-Intents, iOS-URL-Schemata, native Sprachausgabe).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use jarvis_protocol::{Folder, MediaCommand};

use crate::{exec, files};

pub trait Host: Send + Sync + 'static {
    fn open_app(&self, app: &str) -> Result<String>;
    fn close_app(&self, app: &str) -> Result<String>;
    /// Vorlesen; darf blockieren, bis gesprochen ist.
    fn say(&self, text: &str) -> Result<()>;
    /// Wohin eine empfangene Datei soll.
    fn receive_dir(&self, folder: Option<&Folder>) -> Result<PathBuf>;
    /// Datei finden, die der Nutzer beim Namen genannt hat.
    fn locate_file(&self, spoken: &str) -> Result<PathBuf>;
    /// Eine Datei ist vollständig angekommen.
    fn file_received(&self, _path: &Path) {}
    /// Adresse im Browser bzw. der zuständigen App öffnen.
    fn open_url(&self, url: &str) -> Result<String>;
    /// Musik suchen und – wo das System es erlaubt – abspielen.
    /// `service`: "apple-music", "spotify", "youtube-music" oder Standard.
    fn play_music(&self, query: &str, service: Option<&str>) -> Result<String>;
    fn media(&self, command: MediaCommand) -> Result<String>;
}

pub struct DesktopHost {
    apps: BTreeMap<String, String>,
    music_service: String,
}

impl DesktopHost {
    pub fn new(apps: BTreeMap<String, String>, music_service: String) -> Self {
        Self {
            apps,
            music_service,
        }
    }
}

impl Host for DesktopHost {
    fn open_app(&self, app: &str) -> Result<String> {
        exec::open_app(app, &self.apps)
    }
    fn close_app(&self, app: &str) -> Result<String> {
        exec::close_app(app)
    }
    fn say(&self, text: &str) -> Result<()> {
        exec::say_blocking(text)
    }
    fn receive_dir(&self, folder: Option<&Folder>) -> Result<PathBuf> {
        files::resolve_folder(folder)
    }
    fn locate_file(&self, spoken: &str) -> Result<PathBuf> {
        files::locate(spoken)
    }
    fn open_url(&self, url: &str) -> Result<String> {
        exec::open_url(url)?;
        Ok("Geöffnet.".into())
    }
    fn play_music(&self, query: &str, service: Option<&str>) -> Result<String> {
        exec::play_music(query, service.unwrap_or(&self.music_service))
    }
    fn media(&self, command: MediaCommand) -> Result<String> {
        exec::media(command, &self.music_service)
    }
}
