//! Links für Websuche und Musikdienste.

/// Prozentkodierung für Such-Parameter.
pub fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn google(query: &str) -> String {
    format!("https://www.google.com/search?q={}", encode(query))
}

/// Suchbegriff für Musikdienste: „Thunderstruck von AC/DC" -> „Thunderstruck AC/DC".
pub fn music_term(query: &str) -> String {
    query.replace(" von ", " ").replace(" by ", " ")
}

pub fn apple_music_web(query: &str) -> String {
    format!(
        "https://music.apple.com/search?term={}",
        encode(&music_term(query))
    )
}

/// Öffnet die Suche direkt in der Musik-App (macOS/iOS).
pub fn apple_music_app(query: &str) -> String {
    format!(
        "music://music.apple.com/search?term={}",
        encode(&music_term(query))
    )
}

pub fn spotify(query: &str) -> String {
    format!(
        "spotify:search:{}",
        encode(&music_term(query)).replace('+', "%20")
    )
}

pub fn youtube_music(query: &str) -> String {
    format!(
        "https://music.youtube.com/search?q={}",
        encode(&music_term(query))
    )
}

/// Anzeigename eines Dienstes.
pub fn service_name(service: &str) -> &'static str {
    match service {
        "spotify" => "Spotify",
        "youtube-music" => "YouTube Music",
        _ => "Apple Music",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links() {
        assert_eq!(
            google("Wetter Berlin"),
            "https://www.google.com/search?q=Wetter+Berlin"
        );
        assert_eq!(
            google("C++ & Rust?"),
            "https://www.google.com/search?q=C%2B%2B+%26+Rust%3F"
        );
        assert_eq!(
            apple_music_web("Thunderstruck von AC/DC"),
            "https://music.apple.com/search?term=Thunderstruck+AC%2FDC"
        );
        assert_eq!(
            spotify("Bohemian Rhapsody"),
            "spotify:search:Bohemian%20Rhapsody"
        );
        assert_eq!(
            google("Größe"),
            "https://www.google.com/search?q=Gr%C3%B6%C3%9Fe"
        );
    }
}
