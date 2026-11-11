//! Lokale, regelbasierte Erkennung von Befehlen.
//!
//! Ziel: Die häufigen Befehle ("öffne Minecraft", "sende Datei a an den PC in
//! Dokumente") ohne Netz und in Mikrosekunden verstehen. Was hier nicht
//! erkannt wird (`None`), geht an `jarvis-brain` (Claude).

use std::sync::OnceLock;

use jarvis_protocol::{Action, Command, Folder, Target};
use regex::Regex;

/// Versteht `text` als Befehl.
///
/// `device_names` sind alle Namen, unter denen bekannte Geräte angesprochen
/// werden können (klein geschrieben), z. B. `["pc", "ipad", "gaming-pc"]`.
pub fn parse(text: &str, device_names: &[String]) -> Option<Command> {
    let text = normalize(text);
    let (text, target) = extract_target(&text, device_names);

    if let Some(c) = re(&SAY).captures(&text) {
        return Some(Command {
            action: Action::Say {
                text: c["text"].trim().to_string(),
            },
            target,
        });
    }

    let text = strip_fillers(&text);

    if let Some(c) = re(&SEND).captures(&text) {
        // Eine Datei ohne Zielgerät zu senden ergibt keinen Sinn -> Claude fragen lassen.
        if target == Target::Here {
            return None;
        }
        let rest = c["rest"].trim();
        let (file, folder) = match re(&INTO_FOLDER).captures(rest) {
            Some(f) => (
                f["file"].to_string(),
                Some(Folder::from_spoken(&clean_folder(&f["folder"]))),
            ),
            None => (rest.to_string(), None),
        };
        let file = unquote(&file);
        if file.is_empty() {
            return None;
        }
        return Some(Command {
            action: Action::SendFile { file, folder },
            target,
        });
    }

    for (pattern, close) in [
        (&CLOSE_PREFIX, true),
        (&CLOSE_SUFFIX, true),
        (&OPEN_PREFIX, false),
        (&OPEN_SUFFIX, false),
    ] {
        if let Some(c) = re(pattern).captures(&text) {
            let app = unquote(&c["app"]);
            if app.is_empty() {
                return None;
            }
            let action = if close {
                Action::CloseApp { app }
            } else {
                Action::OpenApp { app }
            };
            return Some(Command { action, target });
        }
    }

    None
}

type Lazy = (OnceLock<Regex>, &'static str);

fn re(l: &'static Lazy) -> &'static Regex {
    l.0.get_or_init(|| Regex::new(l.1).expect("ungültige Regex"))
}

static SAY: Lazy = (
    OnceLock::new(),
    r#"(?i)^(?:sag|sage|say|sprich)\s*[:„"“']\s*(?P<text>.+?)["“”']?$"#,
);
static SEND: Lazy = (
    OnceLock::new(),
    r"(?i)^(?:sende|sender|send|schicke|schick|übertrage|übertrag|kopiere|kopier|copy|transfer|teile|share)\s+(?:mir\s+)?(?:die\s+|das\s+|the\s+)?(?:datei\s+|file\s+)?(?P<rest>.+)$",
);
static INTO_FOLDER: Lazy = (
    OnceLock::new(),
    r"(?i)^(?P<file>.+?)\s+(?:in|im|ins|nach|into|to)\s+(?:den\s+|die\s+|das\s+|dem\s+|the\s+|my\s+)?(?:ordner\s+|folder\s+|verzeichnis\s+)?(?P<folder>.+)$",
);
static HYPHEN_NOUN: Lazy = (
    OnceLock::new(),
    r"(?i)\b(datei|ordner|verzeichnis|file|folder)-",
);
static OPEN_PREFIX: Lazy = (
    OnceLock::new(),
    r"(?i)^(?:öffne|oeffne|starte|start|open|launch|run|spiele|spiel|play)\s+(?:mir\s+)?(?:die\s+app\s+|das\s+programm\s+|das\s+spiel\s+|the\s+app\s+)?(?P<app>.+)$",
);
static OPEN_SUFFIX: Lazy = (
    OnceLock::new(),
    r"(?i)^(?:mach\s+|mache\s+)?(?P<app>.+?)\s+(?:öffnen|starten|aufmachen|auf|spielen)$",
);
static CLOSE_PREFIX: Lazy = (
    OnceLock::new(),
    r"(?i)^(?:schließe|schließ|schliesse|beende|beend|close|quit|exit|kill|stoppe|stopp|stop)\s+(?P<app>.+)$",
);
static CLOSE_SUFFIX: Lazy = (
    OnceLock::new(),
    r"(?i)^(?:mach\s+|mache\s+)?(?P<app>.+?)\s+(?:schließen|schliessen|beenden|zumachen|zu)$",
);

/// Entfernt Anrede, Höflichkeitsfloskeln am Anfang und Satzzeichen am Ende.
fn normalize(text: &str) -> String {
    // Spracherkennung schreibt gern „Datei-Bericht" oder „Ordner-Dokumente".
    let text = re(&HYPHEN_NOUN).replace_all(text, "$1 ");
    let mut t = text
        .trim()
        .trim_end_matches(['.', '!', '?'])
        .trim()
        .to_string();
    for prefix in ["hey jarvis", "hallo jarvis", "ok jarvis", "jarvis"] {
        if t.to_lowercase().starts_with(prefix) {
            t = t[prefix.len()..]
                .trim_start_matches([',', ':', ' '])
                .to_string();
            break;
        }
    }
    for prefix in [
        "kannst du",
        "könntest du",
        "würdest du",
        "can you",
        "could you",
        "would you",
    ] {
        if t.to_lowercase().starts_with(prefix) {
            t = t[prefix.len()..].trim_start().to_string();
            break;
        }
    }
    collapse_ws(&t)
}

/// Sucht "am PC", "auf dem iPad", "an den PC", "on my laptop" … und entfernt
/// die Phrase aus dem Text. Ohne Präposition wird ein Gerätename *nicht* als
/// Ziel gewertet ("öffne Computer" öffnet die App "Computer").
fn extract_target(text: &str, device_names: &[String]) -> (String, Target) {
    let mut names: Vec<&String> = device_names
        .iter()
        .filter(|n| !n.trim().is_empty())
        .collect();
    // Längere Namen zuerst, damit "gaming-pc" vor "pc" gewinnt.
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    for name in names {
        let pattern = format!(
            r"(?i)(?:^|\s)(?:am|aufs|ans|zum|zur|auf|an|on|to)(?:\s+(?:dem|der|den|das|die|mein|meinem|meinen|meiner|meine|the|my))?\s+{}(?:[\s:,]|$)",
            regex::escape(name)
        );
        let r = Regex::new(&pattern).expect("ungültige Ziel-Regex");
        if let Some(m) = r.find(text) {
            let rest = format!("{} {}", &text[..m.start()], &text[m.end()..]);
            return (collapse_ws(&rest), Target::Named(name.to_lowercase()));
        }
    }
    (text.to_string(), Target::Here)
}

fn strip_fillers(text: &str) -> String {
    const FILLERS: [&str; 6] = ["bitte", "mal", "please", "doch", "jetzt", "eben"];
    text.split_whitespace()
        .filter(|w| !FILLERS.contains(&w.to_lowercase().as_str()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn clean_folder(f: &str) -> String {
    let f = f.trim();
    let lower = f.to_lowercase();
    for suffix in ["-ordner", " ordner", " folder", "-folder"] {
        if lower.ends_with(suffix) {
            return f[..f.len() - suffix.len()].trim().to_string();
        }
    }
    f.to_string()
}

fn unquote(s: &str) -> String {
    s.trim()
        .trim_matches(|c| matches!(c, '"' | '\'' | '„' | '“' | '”'))
        .trim()
        .to_string()
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        [
            "pc",
            "computer",
            "ipad",
            "tablet",
            "handy",
            "gaming-pc",
            "laptop",
        ]
        .map(String::from)
        .to_vec()
    }

    fn p(t: &str) -> Option<Command> {
        parse(t, &names())
    }

    fn open(app: &str, target: Target) -> Option<Command> {
        Some(Command {
            action: Action::OpenApp { app: app.into() },
            target,
        })
    }

    #[test]
    fn open_here() {
        assert_eq!(p("öffne Minecraft"), open("Minecraft", Target::Here));
        assert_eq!(
            p("Jarvis, starte bitte Minecraft!"),
            open("Minecraft", Target::Here)
        );
        assert_eq!(p("Minecraft öffnen"), open("Minecraft", Target::Here));
        assert_eq!(p("mach mal Spotify auf"), open("Spotify", Target::Here));
        assert_eq!(
            p("kannst du Discord starten?"),
            open("Discord", Target::Here)
        );
        assert_eq!(
            p("open Visual Studio Code"),
            open("Visual Studio Code", Target::Here)
        );
    }

    #[test]
    fn open_remote() {
        assert_eq!(
            p("öffne Minecraft am PC"),
            open("Minecraft", Target::Named("pc".into()))
        );
        assert_eq!(
            p("starte auf dem Gaming-PC Steam"),
            open("Steam", Target::Named("gaming-pc".into()))
        );
        assert_eq!(
            p("open Netflix on my iPad"),
            open("Netflix", Target::Named("ipad".into()))
        );
    }

    #[test]
    fn device_name_without_preposition_is_an_app() {
        assert_eq!(p("öffne Computer"), open("Computer", Target::Here));
    }

    #[test]
    fn close() {
        assert_eq!(
            p("beende Minecraft am PC"),
            Some(Command {
                action: Action::CloseApp {
                    app: "Minecraft".into()
                },
                target: Target::Named("pc".into())
            })
        );
        assert_eq!(
            p("mach Spotify zu"),
            Some(Command {
                action: Action::CloseApp {
                    app: "Spotify".into()
                },
                target: Target::Here
            })
        );
    }

    #[test]
    fn send_file() {
        let expected = Some(Command {
            action: Action::SendFile {
                file: "a".into(),
                folder: Some(Folder::Documents),
            },
            target: Target::Named("pc".into()),
        });
        assert_eq!(p("sende Datei a an den PC in Ordner Dokumente"), expected);
        assert_eq!(
            p("schick die Datei a in den Dokumente-Ordner an den PC"),
            expected
        );
        assert_eq!(
            p("Jarvis, sende Datei a an den PC in meine Dokumente."),
            expected
        );
        // So schreibt Whisper es gern:
        assert_eq!(
            p("Sende die Datei-a an den PC in den Ordner-Dokumenten."),
            expected
        );
    }

    #[test]
    fn send_file_variants() {
        assert_eq!(
            p("send report.pdf to my laptop into downloads"),
            Some(Command {
                action: Action::SendFile {
                    file: "report.pdf".into(),
                    folder: Some(Folder::Downloads)
                },
                target: Target::Named("laptop".into()),
            })
        );
        assert_eq!(
            p("schick \"Urlaub 2026.zip\" aufs Handy"),
            Some(Command {
                action: Action::SendFile {
                    file: "Urlaub 2026.zip".into(),
                    folder: None
                },
                target: Target::Named("handy".into()),
            })
        );
        // Ohne Zielgerät -> nicht eindeutig.
        assert_eq!(p("sende Datei a"), None);
    }

    #[test]
    fn say() {
        assert_eq!(
            p("sag auf dem iPad: \"Essen ist fertig\""),
            Some(Command {
                action: Action::Say {
                    text: "Essen ist fertig".into()
                },
                target: Target::Named("ipad".into())
            })
        );
    }

    #[test]
    fn unknown_goes_to_brain() {
        assert_eq!(p("wie wird das Wetter morgen"), None);
        assert_eq!(p(""), None);
    }
}
