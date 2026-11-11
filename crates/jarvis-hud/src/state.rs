//! Was das HUD gerade zeigt – reine Logik, ohne Zeichnen (testbar).

use std::time::{Duration, Instant};

use jarvis_node::events::HudEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Hidden,
    Listening,
    Thinking,
    Answer,
}

pub struct HudState {
    pub mode: Mode,
    /// Deckkraft 0..1 (weiches Ein- und Ausblenden).
    pub alpha: f32,
    /// Geglätteter Pegel 0..1 (Stimme oder Sprechen).
    pub level: f32,
    target_level: f32,
    pub heard: String,
    pub source: Option<String>,
    pub reply: String,
    pub ok: bool,
    reply_since: Instant,
    pub speaking: bool,
    visible_until: Instant,
    last_tick: Instant,
    /// Laufende Animationszeit in Sekunden.
    pub t: f32,
    /// Aufsummierte Drehung (schneller beim Nachdenken).
    pub spin: f32,
}

/// Zeichen pro Sekunde für den Schreibmaschinen-Effekt.
const TYPE_RATE: f32 = 60.0;

impl HudState {
    pub fn new(now: Instant) -> Self {
        Self {
            mode: Mode::Hidden,
            alpha: 0.0,
            level: 0.0,
            target_level: 0.0,
            heard: String::new(),
            source: None,
            reply: String::new(),
            ok: true,
            reply_since: now,
            speaking: false,
            visible_until: now,
            last_tick: now,
            t: 0.0,
            spin: 0.0,
        }
    }

    fn keep_until(&mut self, t: Instant) {
        self.visible_until = self.visible_until.max(t);
    }

    fn reading_time(&self) -> Duration {
        Duration::from_millis(2500 + self.reply.chars().count() as u64 * 45)
    }

    pub fn apply(&mut self, event: HudEvent, now: Instant) {
        match event {
            HudEvent::Wake => {
                self.mode = Mode::Listening;
                self.heard.clear();
                self.reply.clear();
                self.source = None;
                self.keep_until(now + Duration::from_secs(8));
            }
            HudEvent::Level { value } => {
                self.target_level = self.target_level.max(value.clamp(0.0, 1.0));
                if self.mode == Mode::Hidden {
                    self.mode = Mode::Listening;
                }
                self.keep_until(now + Duration::from_secs(3));
            }
            HudEvent::Heard { text, source } => {
                self.heard = text;
                self.source = source;
                self.reply.clear();
                self.mode = Mode::Thinking;
                self.keep_until(now + Duration::from_secs(30));
            }
            HudEvent::Thinking => {
                self.mode = Mode::Thinking;
                self.keep_until(now + Duration::from_secs(30));
            }
            HudEvent::Reply { text, ok } => {
                self.reply = text;
                self.ok = ok;
                self.mode = Mode::Answer;
                self.reply_since = now;
                // Nach dem Nachdenken nicht länger als nötig stehen bleiben.
                self.visible_until = now + self.reading_time();
            }
            HudEvent::Speaking { active } => {
                self.speaking = active;
                if active {
                    if self.mode == Mode::Hidden {
                        self.mode = Mode::Answer;
                    }
                    self.keep_until(now + Duration::from_secs(60));
                } else {
                    let typed_end = self.reply_since + self.reading_time();
                    self.visible_until = (now + Duration::from_millis(2500)).max(typed_end);
                }
            }
            HudEvent::File { name, from } => {
                self.heard = format!("Datei von {from}");
                self.source = None;
                self.reply = name;
                self.ok = true;
                self.mode = Mode::Answer;
                self.reply_since = now;
                self.visible_until = now + Duration::from_secs(5);
            }
            HudEvent::Idle => {
                self.visible_until = self.visible_until.min(now + Duration::from_secs(1));
            }
        }
    }

    /// Fortschreiben der Animation; `true`, solange etwas zu sehen ist.
    pub fn tick(&mut self, now: Instant) -> bool {
        let dt = now
            .saturating_duration_since(self.last_tick)
            .as_secs_f32()
            .min(0.1);
        self.last_tick = now;
        self.t += dt;
        self.spin += dt
            * if self.mode == Mode::Thinking {
                2.4
            } else {
                0.35
            };

        // Beim Vorlesen pulsiert der Ring künstlich mit (kein Zugriff auf den Ton).
        if self.speaking {
            let s = self.t;
            self.target_level = self
                .target_level
                .max(0.35 + 0.3 * (s * 9.0).sin().abs() * (s * 2.3).sin().abs());
        }
        self.level += (self.target_level - self.level) * (dt * 14.0).min(1.0);
        self.target_level *= (-dt * 4.0).exp();

        let target_alpha = if now < self.visible_until { 1.0 } else { 0.0 };
        let step = dt * 4.0;
        self.alpha = if target_alpha > self.alpha {
            (self.alpha + step).min(1.0)
        } else {
            (self.alpha - step).max(0.0)
        };
        if self.alpha == 0.0 && now >= self.visible_until {
            self.mode = Mode::Hidden;
            self.speaking = false;
        }
        self.alpha > 0.0
    }

    /// Antwort mit Schreibmaschinen-Effekt.
    pub fn typed_reply(&self, now: Instant) -> String {
        let n = (now
            .saturating_duration_since(self.reply_since)
            .as_secs_f32()
            * TYPE_RATE) as usize;
        self.reply.chars().take(n).collect()
    }

    pub fn status(&self) -> &'static str {
        match self.mode {
            Mode::Hidden => "",
            Mode::Listening => "HÖRE ZU",
            Mode::Thinking => "VERARBEITE",
            Mode::Answer if !self.ok => "PROBLEM",
            Mode::Answer if self.speaking => "SPRECHE",
            Mode::Answer => "BEREIT",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &mut HudState, from: Instant, secs: f32) -> Instant {
        let mut t = from;
        for _ in 0..(secs * 60.0) as usize {
            t += Duration::from_millis(16);
            s.tick(t);
        }
        t
    }

    #[test]
    fn full_conversation_fades_in_and_out() {
        let t0 = Instant::now();
        let mut s = HudState::new(t0);
        assert!(!s.tick(t0));

        s.apply(HudEvent::Wake, t0);
        let t = run(&mut s, t0, 0.5);
        assert_eq!(s.mode, Mode::Listening);
        assert!(s.alpha > 0.99);

        s.apply(
            HudEvent::Heard {
                text: "öffne Minecraft".into(),
                source: Some("Sprache".into()),
            },
            t,
        );
        assert_eq!(s.status(), "VERARBEITE");
        s.apply(
            HudEvent::Reply {
                text: "Minecraft wird gestartet.".into(),
                ok: true,
            },
            t,
        );
        assert_eq!(s.typed_reply(t), "");
        let t = run(&mut s, t, 0.2);
        assert!(!s.typed_reply(t).is_empty() && s.typed_reply(t).len() < s.reply.len());
        let t = run(&mut s, t, 1.0);
        assert_eq!(s.typed_reply(t), "Minecraft wird gestartet.");

        // Spricht: bleibt sichtbar, Ring pulsiert
        s.apply(HudEvent::Speaking { active: true }, t);
        let t = run(&mut s, t, 10.0);
        assert!(s.alpha > 0.99 && s.level > 0.2);
        s.apply(HudEvent::Speaking { active: false }, t);
        let _ = run(&mut s, t, 4.0);
        assert_eq!(s.mode, Mode::Hidden);
        assert_eq!(s.alpha, 0.0);
    }

    #[test]
    fn level_raises_ring_and_decays() {
        let t0 = Instant::now();
        let mut s = HudState::new(t0);
        s.apply(HudEvent::Wake, t0);
        s.apply(HudEvent::Level { value: 0.9 }, t0);
        let t = run(&mut s, t0, 0.15);
        assert!(s.level > 0.4, "{}", s.level);
        let _ = run(&mut s, t, 1.5);
        assert!(s.level < 0.1, "{}", s.level);
    }

    #[test]
    fn errors_and_files() {
        let t0 = Instant::now();
        let mut s = HudState::new(t0);
        s.apply(
            HudEvent::Reply {
                text: "Ich kenne kein Gerät".into(),
                ok: false,
            },
            t0,
        );
        assert_eq!(s.status(), "PROBLEM");
        s.apply(
            HudEvent::File {
                name: "Urlaub.jpg".into(),
                from: "iPhone".into(),
            },
            t0,
        );
        assert_eq!(s.heard, "Datei von iPhone");
        assert_eq!(s.status(), "BEREIT");
    }
}
