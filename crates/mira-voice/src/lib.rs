//! Spracheingabe für Mira.
//!
//! ```text
//! Mikrofon ─► audio (16 kHz mono) ─► segment (Silero-VAD) ─► stt (Whisper)
//!          ─► wake („Mira …") ─► mirad ─► Antwort vorlesen
//! ```
//!
//! Alles läuft lokal; mit Grafikkarte (Feature `cuda`/`vulkan`/`metal`)
//! dauert die Erkennung eines Befehls nur Bruchteile einer Sekunde.

pub mod audio;
pub mod models;
pub mod segment;
pub mod stt;
pub mod wake;

/// Whisper und Silero erwarten 16 kHz mono.
pub const SAMPLE_RATE: u32 = 16_000;

/// Wurde mit Grafikkarten-Unterstützung gebaut?
pub const GPU_BUILD: bool = cfg!(any(feature = "cuda", feature = "vulkan", feature = "metal"));

/// Standardmodell: groß und genau mit Grafikkarte, klein und flott ohne.
pub fn default_model() -> &'static str {
    if GPU_BUILD {
        "large-v3-turbo-q5_0"
    } else {
        "small-q5_1"
    }
}
