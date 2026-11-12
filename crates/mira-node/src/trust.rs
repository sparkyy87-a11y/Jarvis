//! Wer zu dir gehört – und wer was darf.
//!
//! - Ein Gerät ist **Hauptgerät** (z. B. dein Laptop). Nur es darf neue Geräte
//!   koppeln, Geräte sperren, Rechte vergeben und die Hauptrolle weitergeben.
//! - Die Liste „wer gehört dazu, wer darf was, wer ist gesperrt" ist eine
//!   **Richtlinie, die das Hauptgerät signiert** (Ed25519). Jedes Gerät merkt
//!   sich beim Koppeln den Signaturschlüssel des Hauptgeräts und übernimmt nur
//!   Fassungen, die damit unterschrieben und neuer sind. Ein anderes Gerät kann
//!   sich also keine Rechte geben – auch nicht durch Ändern seiner Dateien.
//! - Neue Geräte kommen nur mit einem **Einmal-Code** vom Hauptgerät hinein
//!   (10 Minuten gültig, nur einmal nutzbar).
//! - Bekannte Adressen der Geräte werden zusätzlich ausgetauscht, damit sich
//!   z. B. iPad und Handy auch ohne PC finden.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, ensure, Context, Result};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use mira_protocol::{AddrHint, DeviceInfo, Member, Policy, SignedPolicy};
use serde::{Deserialize, Serialize};

/// Gültigkeit eines Einmal-Codes.
pub const INVITE_SECONDS: u64 = 600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub token: String,
    pub expires: u64,
}

/// Eine Kopplung, die dieses Gerät gerade mit dem Hauptgerät eingeht.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Joining {
    pub token: String,
    /// Erwarteter Geräteschlüssel des Hauptgeräts (aus dem QR-Code).
    pub key: String,
    /// Erwarteter Signaturschlüssel des Hauptgeräts (aus dem QR-Code).
    pub sign_key: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownAddr {
    pub addr: String,
    pub seen: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TrustStore {
    /// Signaturschlüssel des Hauptgeräts, dem dieses Gerät vertraut (hex).
    #[serde(default)]
    pub pinned: Option<String>,
    /// Aktuell gültige, signierte Richtlinie.
    #[serde(default)]
    pub policy: Option<SignedPolicy>,
    /// Gelernte Adressen: Geräteschlüssel -> Adresse.
    #[serde(default)]
    pub addrs: BTreeMap<String, KnownAddr>,
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

/// Neuer Ed25519-Signaturschlüssel (geheim, hex).
pub fn new_signing_key() -> String {
    let mut raw = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
    hex::encode(raw)
}

pub fn signing_key(secret_hex: &str) -> Result<SigningKey> {
    let bytes: [u8; 32] = hex::decode(secret_hex)
        .context("signing_key ist kein Hex")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("signing_key muss 32 Byte lang sein"))?;
    Ok(SigningKey::from_bytes(&bytes))
}

pub fn public_sign_key(secret_hex: &str) -> Result<String> {
    Ok(hex::encode(
        signing_key(secret_hex)?.verifying_key().to_bytes(),
    ))
}

fn verify(policy: &SignedPolicy, signer_hex: &str) -> Result<()> {
    let key: [u8; 32] = hex::decode(signer_hex)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Schlüssellänge"))?;
    let sig: [u8; 64] = hex::decode(&policy.signature)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Signaturlänge"))?;
    VerifyingKey::from_bytes(&key)?
        .verify(
            &policy.policy.signing_bytes(),
            &ed25519_dalek::Signature::from_bytes(&sig),
        )
        .context("Signatur des Hauptgeräts ungültig")
}

fn sign(policy: Policy, key: &SigningKey) -> SignedPolicy {
    let signature = hex::encode(key.sign(&policy.signing_bytes()).to_bytes());
    SignedPolicy { policy, signature }
}

/// Mitglied aus den Angaben eines Geräts.
pub fn member_from(key: &str, d: &DeviceInfo, control: bool, files: bool) -> Member {
    let control_main = false;
    Member {
        key: key.to_string(),
        id: d.id,
        name: d.name.clone(),
        sign_key: d.sign_key.clone(),
        control,
        files,
        control_main,
    }
}

/// Was ein Gerät auf einem anderen tun möchte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Control,
    Files,
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

    pub fn policy(&self) -> Option<&Policy> {
        self.policy.as_ref().map(|p| &p.policy)
    }

    pub fn is_revoked(&self, key: &str) -> bool {
        self.policy()
            .is_some_and(|p| p.revoked.iter().any(|r| r == key))
    }

    pub fn is_trusted(&self, key: &str) -> bool {
        !self.is_revoked(key) && self.policy().and_then(|p| p.member(key)).is_some()
    }

    pub fn is_admin_key(&self, key: &str) -> bool {
        self.policy().is_some_and(|p| p.admin.key == key)
    }

    /// Ist dieses Gerät (mit diesem öffentlichen Signaturschlüssel) das Hauptgerät?
    pub fn is_admin(&self, own_sign_pub: &str) -> bool {
        self.pinned.as_deref() == Some(own_sign_pub)
            && self
                .policy()
                .is_some_and(|p| p.admin.sign_key == own_sign_pub)
    }

    pub fn admin_name(&self) -> Option<String> {
        self.policy().map(|p| p.admin.name.clone())
    }

    /// Prüft, ob das Gerät mit Schlüssel `from` hier (`me`) `what` darf.
    pub fn allows(
        &self,
        from: &str,
        me: &str,
        what: Permission,
    ) -> std::result::Result<(), String> {
        let Some(p) = self.policy() else {
            return Err("keine gültige Richtlinie".into());
        };
        if self.is_revoked(from) {
            return Err("Gerät ist gesperrt".into());
        }
        if p.admin.key == from {
            return Ok(());
        }
        let Some(m) = p.members.iter().find(|m| m.key == from) else {
            return Err("Gerät gehört nicht dazu".into());
        };
        match what {
            Permission::Control if p.admin.key == me => {
                if m.control_main {
                    Ok(())
                } else {
                    Err(format!(
                        "„{}“ darf das Hauptgerät „{}“ nicht steuern.",
                        m.name, p.admin.name
                    ))
                }
            }
            Permission::Control if !m.control => {
                Err(format!("„{}“ darf keine anderen Geräte steuern.", m.name))
            }
            Permission::Files if !m.files => {
                Err(format!("„{}“ darf keine Dateien schicken.", m.name))
            }
            _ => Ok(()),
        }
    }

    /// Übernimmt eine Richtlinie, wenn sie vom gemerkten Hauptgerät signiert und
    /// neuer ist. `Ok(true)` bei Änderung. Gibt das Hauptgerät seine Rolle ab,
    /// gilt danach der Schlüssel des neuen Hauptgeräts.
    pub fn accept(&mut self, sp: &SignedPolicy) -> Result<bool> {
        let Some(pinned) = self.pinned.clone() else {
            bail!("noch kein Hauptgerät bekannt")
        };
        if self
            .policy()
            .is_some_and(|p| p.version >= sp.policy.version)
        {
            return Ok(false);
        }
        verify(sp, &pinned)?;
        self.pinned = Some(sp.policy.admin.sign_key.clone());
        self.policy = Some(sp.clone());
        Ok(true)
    }

    /// Erstes Gerät: wird Hauptgerät.
    pub fn init_admin(&mut self, me: Member, sign_secret: &str) -> Result<()> {
        let key = signing_key(sign_secret)?;
        ensure!(
            hex::encode(key.verifying_key().to_bytes()) == me.sign_key,
            "Signaturschlüssel passt nicht"
        );
        self.pinned = Some(me.sign_key.clone());
        self.policy = Some(sign(
            Policy {
                version: 1,
                admin: me,
                members: vec![],
                revoked: vec![],
            },
            &key,
        ));
        Ok(())
    }

    /// Ändert die Richtlinie – nur das Hauptgerät kann das.
    pub fn edit(
        &mut self,
        sign_secret: &str,
        change: impl FnOnce(&mut Policy) -> Result<()>,
    ) -> Result<SignedPolicy> {
        let key = signing_key(sign_secret)?;
        let own = hex::encode(key.verifying_key().to_bytes());
        if !self.is_admin(&own) {
            bail!(
                "Das darf nur das Hauptgerät ({}).",
                self.admin_name().unwrap_or_else(|| "unbekannt".into())
            );
        }
        let mut p = self.policy().cloned().context("keine Richtlinie")?;
        change(&mut p)?;
        p.version += 1;
        p.members.sort_by(|a, b| a.key.cmp(&b.key));
        p.revoked.sort();
        p.revoked.dedup();
        let sp = sign(p, &key);
        // Bei Übergabe der Hauptrolle zeigt `pinned` danach auf das neue Hauptgerät.
        self.pinned = Some(sp.policy.admin.sign_key.clone());
        self.policy = Some(sp.clone());
        Ok(sp)
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

    // ---------------------------------------------------------------- Adressen

    /// Adresse, unter der ein Gerät gerade direkt erreichbar war.
    pub fn observe(&mut self, key: &str, addr: String) {
        self.addrs
            .insert(key.to_string(), KnownAddr { addr, seen: now() });
    }

    pub fn addr_hints(&self) -> Vec<AddrHint> {
        self.addrs
            .iter()
            .filter(|(k, _)| self.is_trusted(k))
            .map(|(k, a)| AddrHint {
                key: k.clone(),
                addr: a.addr.clone(),
                seen: a.seen,
            })
            .collect()
    }

    /// Übernimmt neuere Adressen; `true` bei Änderung.
    pub fn merge_addrs(&mut self, own_key: &str, hints: &[AddrHint]) -> bool {
        let mut changed = false;
        for h in hints {
            if h.key == own_key || !self.is_trusted(&h.key) {
                continue;
            }
            let newer = self.addrs.get(&h.key).is_none_or(|a| h.seen > a.seen);
            if newer {
                self.addrs.insert(
                    h.key.clone(),
                    KnownAddr {
                        addr: h.addr.clone(),
                        seen: h.seen,
                    },
                );
                changed = true;
            }
        }
        changed
    }

    /// Gekoppelte Geräte mit bekannter Adresse (zum Verbinden ohne PC).
    pub fn known_addresses(&self) -> Vec<(String, String)> {
        self.addrs
            .iter()
            .filter(|(k, _)| self.is_trusted(k))
            .map(|(k, a)| (k.clone(), a.addr.clone()))
            .collect()
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
    use uuid::Uuid;

    struct Dev {
        key: String,
        secret: String,
        member: Member,
    }

    fn dev(name: &str) -> Dev {
        let secret = new_signing_key();
        let key = format!("{name}-key");
        let member = Member {
            key: key.clone(),
            id: Uuid::new_v4(),
            name: name.into(),
            sign_key: public_sign_key(&secret).unwrap(),
            control: true,
            files: true,
            control_main: false,
        };
        Dev {
            key,
            secret,
            member,
        }
    }

    /// Laptop ist Hauptgerät, PC (Sohn) und Handy sind Mitglieder.
    fn family() -> (TrustStore, Dev, Dev, Dev) {
        let (laptop, pc, handy) = (dev("Laptop"), dev("PC"), dev("Handy"));
        let mut t = TrustStore::default();
        t.init_admin(laptop.member.clone(), &laptop.secret).unwrap();
        let (m1, m2) = (pc.member.clone(), handy.member.clone());
        t.edit(&laptop.secret, |p| {
            p.members.push(m1);
            p.members.push(m2);
            Ok(())
        })
        .unwrap();
        (t, laptop, pc, handy)
    }

    /// Ein Gerät, das beim Koppeln den Schlüssel des Hauptgeräts gemerkt hat.
    fn follower(admin: &TrustStore) -> TrustStore {
        let mut t = TrustStore {
            pinned: Some(admin.policy().unwrap().admin.sign_key.clone()),
            ..Default::default()
        };
        t.accept(admin.policy.as_ref().unwrap()).unwrap();
        t
    }

    #[test]
    fn nobody_controls_the_main_device_except_itself() {
        let (t, laptop, pc, handy) = family();
        assert!(t.allows(&pc.key, &laptop.key, Permission::Control).is_err());
        assert!(t
            .allows(&handy.key, &laptop.key, Permission::Control)
            .is_err());
        // Eigenes Handy ausdrücklich freigeben: dann darf es auch den Laptop steuern.
        let mut t = t;
        let hk = handy.key.clone();
        t.edit(&laptop.secret, |p| {
            p.members
                .iter_mut()
                .find(|m| m.key == hk)
                .unwrap()
                .control_main = true;
            Ok(())
        })
        .unwrap();
        assert!(t
            .allows(&handy.key, &laptop.key, Permission::Control)
            .is_ok());
        assert!(t.allows(&pc.key, &laptop.key, Permission::Control).is_err());
        assert!(t.allows(&laptop.key, &pc.key, Permission::Control).is_ok());
        assert!(
            t.allows(&pc.key, &handy.key, Permission::Control).is_ok(),
            "Mitglieder untereinander dürfen, solange erlaubt"
        );
        assert!(t.allows("fremd", &pc.key, Permission::Files).is_err());
    }

    #[test]
    fn permissions_are_enforced_and_only_admin_can_change_them() {
        let (mut t, laptop, pc, handy) = family();
        t.edit(&laptop.secret, |p| {
            p.members
                .iter_mut()
                .find(|m| m.name == "PC")
                .unwrap()
                .control = false;
            Ok(())
        })
        .unwrap();
        assert!(t
            .allows(&pc.key, &handy.key, Permission::Control)
            .unwrap_err()
            .contains("darf keine anderen Geräte steuern"));
        assert!(t.allows(&pc.key, &handy.key, Permission::Files).is_ok());
        // Der PC selbst kann die Richtlinie nicht ändern.
        let mut sons_copy = follower(&t);
        assert!(sons_copy
            .edit(&pc.secret, |_| Ok(()))
            .unwrap_err()
            .to_string()
            .contains("nur das Hauptgerät"));
    }

    #[test]
    fn forged_or_old_policies_are_rejected() {
        let (t, _laptop, pc, _handy) = family();
        let mut handy_store = follower(&t);

        // Der PC baut sich eine eigene Richtlinie, in der er Hauptgerät ist.
        let mut forged = t.policy().unwrap().clone();
        forged.version += 10;
        forged.admin = pc.member.clone();
        let forged = sign(forged, &signing_key(&pc.secret).unwrap());
        assert!(handy_store.accept(&forged).is_err());

        // Veränderte Rechte bei alter Signatur fallen auf.
        let mut tampered = t.policy.clone().unwrap();
        tampered.policy.version += 1;
        tampered.policy.members.retain(|m| m.name != "Handy");
        assert!(handy_store.accept(&tampered).is_err());

        // Ältere Fassung ändert nichts.
        let mut old = t.policy.clone().unwrap();
        old.policy.version = 0;
        assert!(!handy_store.accept(&old).unwrap());
        // Ohne gemerktes Hauptgerät wird gar nichts übernommen.
        assert!(TrustStore::default()
            .accept(t.policy.as_ref().unwrap())
            .is_err());
    }

    #[test]
    fn revocation_and_handover() {
        let (mut t, laptop, pc, handy) = family();
        let pc_key = pc.key.clone();
        t.edit(&laptop.secret, |p| {
            p.members.retain(|m| m.key != pc_key);
            p.revoked.push(pc_key.clone());
            Ok(())
        })
        .unwrap();
        assert!(!t.is_trusted(&pc.key) && t.is_revoked(&pc.key));
        assert!(t.allows(&pc.key, &handy.key, Permission::Files).is_err());

        // Hauptrolle an das Handy übergeben: danach gilt dessen Signatur.
        let mut other = follower(&t);
        let (hm, lm) = (handy.member.clone(), laptop.member.clone());
        let handed = t
            .edit(&laptop.secret, |p| {
                p.members.retain(|m| m.key != hm.key);
                p.members.push(lm);
                p.admin = hm;
                Ok(())
            })
            .unwrap();
        assert!(other.accept(&handed).unwrap());
        assert!(other.is_admin(&handy.member.sign_key));
        assert!(t.edit(&laptop.secret, |_| Ok(())).is_err());
        assert!(other
            .allows(&laptop.key, &handy.key, Permission::Control)
            .is_err());
    }

    #[test]
    fn invite_is_single_use_and_expires() {
        let mut t = TrustStore::default();
        let inv = t.new_invite();
        assert!(!t.take_invite("falsch"));
        assert!(t.take_invite(&inv.token));
        assert!(!t.take_invite(&inv.token));
        let inv = t.new_invite();
        t.invite.as_mut().unwrap().expires = now() - 1;
        assert!(!t.take_invite(&inv.token));
    }

    #[test]
    fn addresses_only_for_members_and_fresher_wins() {
        let (t, laptop, _pc, handy) = family();
        let mut ipad = follower(&t);
        let hints = vec![
            AddrHint {
                key: handy.key.clone(),
                addr: "192.168.1.42:7879".into(),
                seen: 100,
            },
            AddrHint {
                key: "fremd".into(),
                addr: "6.6.6.6:7879".into(),
                seen: 100,
            },
        ];
        assert!(ipad.merge_addrs(&laptop.key, &hints));
        assert_eq!(
            ipad.known_addresses(),
            vec![(handy.key.clone(), "192.168.1.42:7879".to_string())]
        );
        let old = vec![AddrHint {
            key: handy.key.clone(),
            addr: "10.0.0.9:7879".into(),
            seen: 50,
        }];
        assert!(!ipad.merge_addrs(&laptop.key, &old));
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
}
