//! Weckwort im erkannten Text finden und Whisper-Halluzinationen aussortieren.

use std::time::{Duration, Instant};

use jarvis_node::files::levenshtein;

/// Schreibweisen, die Whisper für „Jarvis" erfahrungsgemäß wählt.
const JARVIS_VARIANTS: [&str; 6] = [
    "charvis", "tschavis", "dschawis", "dschavis", "scharvis", "jarviss",
];

/// Prüft, ob `text` mit dem Weckwort beginnt (oder endet) und gibt den Befehl
/// zurück. `Some("")` heißt: nur das Weckwort, Befehl folgt.
///
/// Unscharf, weil Whisper Namen gern leicht anders schreibt („Javis", „Jervis").
pub fn split_wake(text: &str, wake_words: &[String]) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let is_wake = |w: &str| {
        let w = normalize(w);
        !w.is_empty()
            && wake_words.iter().any(|ww| {
                let ww = normalize(ww);
                // 1 Tippfehler ist ok („Javis", „Jervis"); „Paris" (2) nicht.
                levenshtein(&w, &ww) <= if ww.chars().count() >= 9 { 2 } else { 1 }
                    || (ww == "jarvis" && JARVIS_VARIANTS.contains(&w.as_str()))
            })
    };
    const LEAD: [&str; 10] = [
        "hey", "hei", "hallo", "ok", "okay", "hi", "he", "ey", "ja", "na",
    ];

    // „Hey Jarvis, öffne …"
    let mut i = 0;
    while i < words.len() && i < 2 && LEAD.contains(&normalize(words[i]).as_str()) {
        i += 1;
    }
    if i < words.len() && is_wake(words[i]) {
        return Some(clean_command(&words[i + 1..].join(" ")));
    }
    // „Öffne Minecraft, Jarvis."
    if words.len() >= 2 && is_wake(words[words.len() - 1]) {
        return Some(clean_command(&words[..words.len() - 1].join(" ")));
    }
    None
}

/// Satzzeichen am Rand entfernen, die Whisper gern setzt.
pub fn clean_command(s: &str) -> String {
    s.trim()
        .trim_matches(|c: char| {
            matches!(c, ',' | '.' | '!' | '?' | ':' | ';' | '-' | '–' | '…' | '"')
        })
        .trim()
        .to_string()
}

fn normalize(w: &str) -> String {
    w.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Texte, die Whisper bei Stille oder Rauschen gern erfindet.
pub fn is_hallucination(text: &str) -> bool {
    let t = text.to_lowercase();
    let t = t.trim();
    const PATTERNS: [&str; 10] = [
        "untertitel",
        "amara.org",
        "copyright",
        "fürs zuschauen",
        "für's zuschauen",
        "thanks for watching",
        "thank you for watching",
        "swr 20",
        "wdr 20",
        "zdf",
    ];
    t.is_empty()
        || PATTERNS.iter().any(|p| t.contains(p))
        || (t.starts_with(['[', '(', '*']) && t.ends_with([']', ')', '*']))
        || t.chars().all(|c| !c.is_alphanumeric())
}

/// Was mit einem erkannten Satz passieren soll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Kein Weckwort – nicht an Jarvis gerichtet.
    Ignore,
    /// Nur „Jarvis" – kurz bestätigen und auf den Befehl warten.
    Acknowledge,
    Command(String),
}

/// Merkt sich, ob Jarvis gerade nach „Jarvis …" auf den Befehl wartet.
pub struct Conversation {
    wake_words: Vec<String>,
    follow_up: Duration,
    always_listening: bool,
    armed_until: Option<Instant>,
}

impl Conversation {
    /// `always_listening`: jeder Satz ist ein Befehl (zum Testen, Push-to-Talk).
    pub fn new(wake_words: Vec<String>, follow_up: Duration, always_listening: bool) -> Self {
        Self {
            wake_words,
            follow_up,
            always_listening,
            armed_until: None,
        }
    }

    pub fn on_text(&mut self, text: &str, now: Instant) -> Decision {
        if is_hallucination(text) {
            return Decision::Ignore;
        }
        let armed = self.armed_until.take().is_some_and(|t| now <= t);
        match split_wake(text, &self.wake_words) {
            Some(cmd) if cmd.is_empty() => {
                self.armed_until = Some(now + self.follow_up);
                Decision::Acknowledge
            }
            Some(cmd) => Decision::Command(cmd),
            None if armed || self.always_listening => {
                let cmd = clean_command(text);
                if cmd.is_empty() {
                    Decision::Ignore
                } else {
                    Decision::Command(cmd)
                }
            }
            None => Decision::Ignore,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_flow() {
        let t0 = Instant::now();
        let mut c = Conversation::new(w(), Duration::from_secs(8), false);
        assert_eq!(c.on_text("Wie spät ist es?", t0), Decision::Ignore);
        assert_eq!(
            c.on_text("Jarvis, öffne Minecraft.", t0),
            Decision::Command("öffne Minecraft".into())
        );
        assert_eq!(c.on_text("Jarvis?", t0), Decision::Acknowledge);
        assert_eq!(
            c.on_text("Sende Datei a an den PC.", t0 + Duration::from_secs(3)),
            Decision::Command("Sende Datei a an den PC".into())
        );
        // Danach wieder nur mit Weckwort.
        assert_eq!(
            c.on_text("Öffne Steam.", t0 + Duration::from_secs(4)),
            Decision::Ignore
        );
        // Zu spät nach „Jarvis".
        c.on_text("Jarvis", t0);
        assert_eq!(
            c.on_text("Öffne Steam.", t0 + Duration::from_secs(20)),
            Decision::Ignore
        );
        // Halluzination entschärft nicht die Bereitschaft.
        assert_eq!(
            c.on_text("Untertitel im Auftrag des ZDF", t0),
            Decision::Ignore
        );
    }

    fn w() -> Vec<String> {
        vec!["jarvis".into()]
    }

    #[test]
    fn wake_word_variants() {
        assert_eq!(
            split_wake("Jarvis, öffne Minecraft.", &w()),
            Some("öffne Minecraft".into())
        );
        assert_eq!(
            split_wake("Hey Jarvis öffne Minecraft", &w()),
            Some("öffne Minecraft".into())
        );
        assert_eq!(
            split_wake("Javis, sende Datei a an den PC.", &w()),
            Some("sende Datei a an den PC".into())
        );
        assert_eq!(
            split_wake("Öffne Minecraft, Jarvis!", &w()),
            Some("Öffne Minecraft".into())
        );
        assert_eq!(split_wake("Jarvis?", &w()), Some(String::new()));
        assert_eq!(
            split_wake("Ja, Jarvis, öffne Minecraft.", &w()),
            Some("öffne Minecraft".into())
        );
    }

    #[test]
    fn no_wake_word() {
        assert_eq!(split_wake("Öffne Minecraft", &w()), None);
        assert_eq!(split_wake("Wir waren in Paris.", &w()), None);
        assert_eq!(split_wake("", &w()), None);
    }

    #[test]
    fn hallucinations() {
        assert!(is_hallucination("Untertitel der Amara.org-Community"));
        assert!(is_hallucination("[Musik]"));
        assert!(is_hallucination(" ... "));
        assert!(!is_hallucination("Jarvis, öffne Minecraft."));
    }
}
