//! Was ein Gerät *kann*: Apps öffnen, sprechen, Dateien ablegen.
//!
//! Der Desktop nutzt [`DesktopHost`]; die Handy-Apps liefern eine eigene
//! Umsetzung (Android-Intents, iOS-URL-Schemata, native Sprachausgabe).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use jarvis_protocol::Folder;

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
}

pub struct DesktopHost {
    apps: BTreeMap<String, String>,
}

impl DesktopHost {
    pub fn new(apps: BTreeMap<String, String>) -> Self {
        Self { apps }
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
}
