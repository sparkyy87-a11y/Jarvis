//! Kopplungs-Link `jarvis://pair?…`, den `jarvisd pair` als QR-Code zeigt.
//! Die Handykamera öffnet damit direkt die Jarvis-App.

use anyhow::{bail, Context, Result};

use crate::config::Config;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pairing {
    pub key: String,
    /// Name des Geräts, das den Code gezeigt hat.
    pub name: String,
    /// Adressen, unter denen es erreichbar ist ("192.168.1.20:7879").
    pub addrs: Vec<String>,
}

/// Link für dieses Gerät mit allen LAN-Adressen, die wir herausfinden können.
pub fn link_for(cfg: &Config) -> String {
    let mut url = format!(
        "jarvis://pair?v=1&key={}&name={}",
        cfg.mesh.pairing_key,
        encode(&cfg.device.name)
    );
    for ip in lan_ips() {
        url.push_str(&format!(
            "&addr={}",
            encode(&format!("{ip}:{}", cfg.mesh.port))
        ));
    }
    url
}

pub fn parse(link: &str) -> Result<Pairing> {
    let query = link
        .trim()
        .strip_prefix("jarvis://pair?")
        .context("Das ist kein Jarvis-Kopplungscode")?;
    let (mut key, mut name, mut addrs) = (None, String::from("Jarvis"), Vec::new());
    for part in query.split('&') {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        let v = decode(v);
        match k {
            "key" => key = Some(v),
            "name" => name = v,
            "addr" => addrs.push(v),
            _ => {}
        }
    }
    let key = key.context("Kopplungscode ohne Schlüssel")?;
    if key.len() != 64 || !key.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("Schlüssel im Kopplungscode ist ungültig");
    }
    Ok(Pairing { key, name, addrs })
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
        let mut cfg = Config::new("Gaming-PC Ü".into(), None).unwrap();
        cfg.mesh.port = 7879;
        let link = link_for(&cfg);
        let p = parse(&link).unwrap();
        assert_eq!(p.key, cfg.mesh.pairing_key);
        assert_eq!(p.name, "Gaming-PC Ü");
        assert!(p.addrs.iter().all(|a| a.ends_with(":7879")));
    }

    #[test]
    fn parse_explicit() {
        let key = "ab".repeat(32);
        let p = parse(&format!(
            "jarvis://pair?v=1&key={key}&name=PC&addr=192.168.1.20%3A7879&addr=10.0.0.5:7879"
        ))
        .unwrap();
        assert_eq!(p.addrs, vec!["192.168.1.20:7879", "10.0.0.5:7879"]);
        assert!(parse("https://example.com").is_err());
        assert!(parse("jarvis://pair?key=123").is_err());
    }
}
