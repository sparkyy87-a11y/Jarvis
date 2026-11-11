//! Welche Geräte zu dir gehören.
//!
//! - Jedes Gerät hat ein eigenes Schlüsselpaar; vertraut wird dem öffentlichen
//!   Schlüssel, nicht einem gemeinsamen Passwort.
//! - Neue Geräte kommen nur mit einem **Einmal-Code** hinein (`jarvisd pair`,
//!   10 Minuten gültig, nur einmal nutzbar).
//! - Gesperrte Geräte (`jarvis entfernen <Gerät>`) kommen nie wieder hinein;
//!   die Sperre verbreitet sich an alle anderen Geräte.
//! - Vertrauenswürdige Geräte gleichen ihre Listen untereinander ab: koppelst
//!   du das Handy am PC, kennt es der Laptop beim nächsten Kontakt mit dem PC.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use jarvis_protocol::TrustEntry;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Gültigkeit eines Einmal-Codes.
pub const INVITE_SECONDS: u64 = 600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedDevice {
    pub id: Uuid,
    pub name: String,
    /// Zuletzt bekannte Adresse des Geräts.
    #[serde(default)]
    pub addr: Option<String>,
    #[serde(default)]
    pub seen: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub token: String,
    pub expires: u64,
}

/// Eine Kopplung, die dieses Gerät gerade mit einem anderen eingeht.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Joining {
    pub token: String,
    /// Erwarteter Schlüssel der Gegenstelle (aus dem QR-Code) – schützt vor
    /// einem Gerät, das sich dazwischenschaltet.
    pub key: String,
    pub name: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TrustStore {
    #[serde(default)]
    pub devices: BTreeMap<String, TrustedDevice>,
    /// Gesperrte Schlüssel -> Name (zur Anzeige).
    #[serde(default)]
    pub revoked: BTreeMap<String, String>,
    #[serde(default)]
    pub invite: Option<Invite>,
    #[serde(default)]
    pub joining: Option<Joining>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// Vergleich ohne Zeitunterschied je nach Übereinstimmung.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

impl TrustStore {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("{} ist ungültig", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("{} nicht lesbar", path.display())),
        }
    }

    /// Speichert atomar und nur für den eigenen Benutzer lesbar.
    pub fn save(&self, path: &Path) -> Result<()> {
        write_private(path, &toml::to_string_pretty(self)?)
    }

    pub fn is_trusted(&self, key: &str) -> bool {
        self.devices.contains_key(key) && !self.is_revoked(key)
    }

    pub fn is_revoked(&self, key: &str) -> bool {
        self.revoked.contains_key(key)
    }

    /// Nimmt ein Gerät auf (oder aktualisiert seinen Namen) – nie ein gesperrtes.
    pub fn add(&mut self, key: &str, id: Uuid, name: &str) -> bool {
        if self.is_revoked(key) {
            return false;
        }
        let d = self
            .devices
            .entry(key.to_string())
            .or_insert(TrustedDevice {
                id,
                name: name.to_string(),
                addr: None,
                seen: 0,
            });
        d.id = id;
        d.name = name.to_string();
        true
    }

    /// Adresse, unter der ein Gerät gerade direkt erreichbar war.
    pub fn observe(&mut self, key: &str, addr: String) {
        if let Some(d) = self.devices.get_mut(key) {
            d.addr = Some(addr);
            d.seen = now();
        }
    }

    /// Gekoppelte Geräte mit bekannter Adresse (zum Verbinden ohne PC).
    pub fn known_addresses(&self) -> Vec<(String, String)> {
        self.devices
            .iter()
            .filter(|(k, _)| !self.is_revoked(k))
            .filter_map(|(k, d)| d.addr.clone().map(|a| (k.clone(), a)))
            .collect()
    }

    pub fn revoke(&mut self, key: &str) -> Option<String> {
        let name = self
            .devices
            .remove(key)
            .map(|d| d.name)
            .unwrap_or_else(|| "Unbekannt".into());
        self.revoked.insert(key.to_string(), name.clone());
        Some(name)
    }

    /// Neuer Einmal-Code; ein älterer verfällt.
    pub fn new_invite(&mut self) -> Invite {
        let mut raw = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
        let invite = Invite {
            token: hex::encode(raw),
            expires: now() + INVITE_SECONDS,
        };
        self.invite = Some(invite.clone());
        invite
    }

    /// Prüft einen Einmal-Code und verbraucht ihn bei Erfolg.
    pub fn take_invite(&mut self, token: &str) -> bool {
        let ok = self
            .invite
            .as_ref()
            .is_some_and(|i| i.expires > now() && same(&i.token, token));
        if ok {
            self.invite = None;
        }
        ok
    }

    pub fn entries(&self) -> Vec<TrustEntry> {
        self.devices
            .iter()
            .map(|(key, d)| TrustEntry {
                key: key.clone(),
                id: d.id,
                name: d.name.clone(),
                addr: d.addr.clone(),
                seen: d.seen,
            })
            .collect()
    }

    pub fn revoked_keys(&self) -> Vec<String> {
        self.revoked.keys().cloned().collect()
    }

    /// Übernimmt Listen eines vertrauenswürdigen Geräts. Liefert die Schlüssel,
    /// die dabei neu gesperrt wurden (deren Verbindungen sind zu trennen).
    pub fn merge(
        &mut self,
        own_key: &str,
        trusted: &[TrustEntry],
        revoked: &[String],
    ) -> Vec<String> {
        let mut newly = Vec::new();
        for key in revoked {
            // Die eigene Sperre übernimmt ein Gerät nicht – es merkt sie beim Gegenüber.
            if key != own_key && !self.is_revoked(key) {
                self.revoke(key);
                newly.push(key.clone());
            }
        }
        for e in trusted {
            if e.key != own_key && !self.is_revoked(&e.key) {
                let d = self.devices.entry(e.key.clone()).or_insert(TrustedDevice {
                    id: e.id,
                    name: e.name.clone(),
                    addr: None,
                    seen: 0,
                });
                // Neuere Adresse übernehmen (das andere Gerät hat es später gesehen).
                if e.addr.is_some() && e.seen > d.seen {
                    d.addr = e.addr.clone();
                    d.seen = e.seen;
                }
            }
        }
        newly
    }
}

/// Schreibt eine Datei atomar; unter Unix nur für den Besitzer lesbar (0600).
/// Windows: Dateien unter %APPDATA% sind ohnehin nur für den Benutzer zugänglich.
pub fn write_private(path: &Path, content: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp: PathBuf = path.with_extension("tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts
            .open(&tmp)
            .with_context(|| format!("{} nicht schreibbar", tmp.display()))?;
        std::io::Write::write_all(&mut f, content.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, name: &str) -> TrustEntry {
        TrustEntry {
            key: key.into(),
            id: Uuid::new_v4(),
            name: name.into(),
            addr: None,
            seen: 0,
        }
    }

    #[test]
    fn invite_is_single_use() {
        let mut t = TrustStore::default();
        let inv = t.new_invite();
        assert!(!t.take_invite("falsch"));
        assert!(t.take_invite(&inv.token));
        assert!(
            !t.take_invite(&inv.token),
            "Einmal-Code darf nur einmal gelten"
        );
    }

    #[test]
    fn expired_invite_is_rejected() {
        let mut t = TrustStore::default();
        let inv = t.new_invite();
        t.invite.as_mut().unwrap().expires = now() - 1;
        assert!(!t.take_invite(&inv.token));
    }

    #[test]
    fn revoked_devices_stay_out_and_spread() {
        let mut pc = TrustStore::default();
        pc.add("handy", Uuid::new_v4(), "Handy");
        pc.add("laptop", Uuid::new_v4(), "Laptop");
        pc.revoke("handy");
        assert!(!pc.is_trusted("handy"));
        assert!(
            !pc.add("handy", Uuid::new_v4(), "Handy"),
            "gesperrt bleibt gesperrt"
        );

        // Der Laptop kennt das Handy noch und erfährt die Sperre beim Abgleich.
        let mut laptop = TrustStore::default();
        laptop.add("handy", Uuid::new_v4(), "Handy");
        let newly = laptop.merge("laptop", &pc.entries(), &pc.revoked_keys());
        assert_eq!(newly, vec!["handy".to_string()]);
        assert!(!laptop.is_trusted("handy"));
        // Ein gesperrtes Gerät kann sich nicht über den Abgleich wieder einschleusen.
        laptop.merge("laptop", &[entry("handy", "Handy")], &[]);
        assert!(!laptop.is_trusted("handy"));
    }

    #[test]
    fn own_key_is_never_revoked_or_added() {
        let mut t = TrustStore::default();
        t.merge("ich", &[entry("ich", "Ich")], &["ich".into()]);
        assert!(t.devices.is_empty() && t.revoked.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn saved_privately() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("trust.toml");
        TrustStore::default().save(&p).unwrap();
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn addresses_spread_and_fresher_wins() {
        // PC kennt Handy (frisch) – iPad lernt die Adresse vom PC.
        let mut pc = TrustStore::default();
        pc.add("handy", Uuid::new_v4(), "Handy");
        pc.observe("handy", "192.168.1.42:7879".into());
        let mut ipad = TrustStore::default();
        ipad.merge("ipad", &pc.entries(), &[]);
        assert_eq!(
            ipad.known_addresses(),
            vec![("handy".to_string(), "192.168.1.42:7879".to_string())]
        );
        // Ältere Info überschreibt keine neuere.
        let mut old = pc.entries();
        old[0].addr = Some("10.0.0.9:7879".into());
        old[0].seen -= 100;
        ipad.merge("ipad", &old, &[]);
        assert_eq!(
            ipad.devices["handy"].addr.as_deref(),
            Some("192.168.1.42:7879")
        );
        // Name ändern behält die Adresse.
        ipad.add("handy", ipad.devices["handy"].id, "Pixel");
        assert!(ipad.devices["handy"].addr.is_some());
    }
}
