//! Kopplungs-Link `jarvis://pair?…`, den `jarvisd pair` als QR-Code zeigt.
//! Die Handykamera öffnet damit direkt die Jarvis-App.

use anyhow::{bail, Context, Result};

use crate::config::Config;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pairing {
    /// Öffentlicher Schlüssel des Geräts, das den Code zeigt (Fingerabdruck).
    pub key: String,
    /// Öffentlicher Signaturschlüssel des Hauptgeräts – nur damit unterschriebene
    /// Gerätelisten werden übernommen.
    pub sign_key: String,
    /// Einmal-Code (10 Minuten gültig, nur einmal nutzbar).
    pub token: String,
    /// Name des Geräts, das den Code gezeigt hat.
    pub name: String,
    /// Adressen, unter denen es erreichbar ist ("192.168.1.20:7879").
    pub addrs: Vec<String>,
}

/// Kopplungslink. Enthält kein dauerhaftes Geheimnis: nur den öffentlichen
/// Schlüssel (damit sich niemand dazwischenschalten kann) und einen Einmal-Code.
pub fn link_for(cfg: &Config, token: &str, addrs: Option<Vec<String>>) -> String {
    let addrs = addrs.unwrap_or_else(|| {
        lan_ips()
            .into_iter()
            .map(|ip| format!("{ip}:{}", cfg.mesh.port))
            .collect()
    });
    let mut url = format!(
        "jarvis://pair?v=3&key={}&skey={}&token={}&name={}",
        cfg.device.public_key,
        cfg.sign_public().unwrap_or_default(),
        token,
        encode(&cfg.device.name)
    );
    for a in addrs {
        url.push_str(&format!("&addr={}", encode(&a)));
    }
    url
}

pub fn parse(link: &str) -> Result<Pairing> {
    let query = link
        .trim()
        .strip_prefix("jarvis://pair?")
        .context("Das ist kein Jarvis-Kopplungscode")?;
    let (mut key, mut skey, mut token, mut name, mut addrs) =
        (None, None, None, String::from("Jarvis"), Vec::new());
    for part in query.split('&') {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        let v = decode(v);
        match k {
            "key" => key = Some(v),
            "skey" => skey = Some(v),
            "token" => token = Some(v),
            "name" => name = v,
            "addr" => addrs.push(v),
            _ => {}
        }
    }
    let hexlen = |s: &str, n: usize| s.len() == n && s.chars().all(|c| c.is_ascii_hexdigit());
    let key = key
        .filter(|k| hexlen(k, 64))
        .context("Kopplungscode ist unvollständig (Schlüssel fehlt)")?;
    let sign_key = skey
        .filter(|k| hexlen(k, 64))
        .context("Kopplungscode ist veraltet – bitte Jarvis auf dem Hauptgerät aktualisieren")?;
    let token = token
        .filter(|t| hexlen(t, 32))
        .context("Kopplungscode ist veraltet – am Hauptgerät einen neuen Code anzeigen")?;
    if addrs.is_empty() {
        bail!("Kopplungscode enthält keine Adresse");
    }
    Ok(Pairing {
        key: key.to_lowercase(),
        sign_key: sign_key.to_lowercase(),
        token,
        name,
        addrs,
    })
}

/// Die Adresse, über die dieses Gerät ins Netz geht. Ein UDP-"connect" schickt
/// nichts ab, verrät aber die passende lokale IP.
fn lan_ips() -> Vec<std::net::IpAddr> {
    let probe = std::net::UdpSocket::bind("0.0.0.0:0").and_then(|s| {
        s.connect("192.168.0.1:9")?;
        s.local_addr()
    });
    probe
        .map(|a| a.ip())
        .into_iter()
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .collect()
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(b) => {
                    out.push(b);
                    i += 3;
                    continue;
                }
                Err(_) => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let cfg = Config::new("Gaming-PC Ü".into()).unwrap();
        let token = "ab".repeat(16);
        let link = link_for(&cfg, &token, Some(vec!["192.168.1.20:7879".into()]));
        assert!(
            !link.contains(&cfg.device.private_key),
            "geheimer Schlüssel gehört nie in den Link"
        );
        let p = parse(&link).unwrap();
        assert_eq!(p.key, cfg.device.public_key);
        assert_eq!(p.sign_key, cfg.sign_public().unwrap());
        assert!(!link.contains(&cfg.device.signing_key));
        assert_eq!(p.token, token);
        assert_eq!(p.name, "Gaming-PC Ü");
        assert_eq!(p.addrs, vec!["192.168.1.20:7879"]);
    }

    #[test]
    fn rejects_broken_codes() {
        assert!(parse("https://example.com").is_err());
        assert!(parse("jarvis://pair?key=123").is_err());
        let key = "ab".repeat(32);
        // alter Link ohne Einmal-Code
        assert!(parse(&format!("jarvis://pair?v=1&key={key}&addr=1.2.3.4:7879")).is_err());
    }
}
