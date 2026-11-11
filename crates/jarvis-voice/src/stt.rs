//! Spracherkennung mit whisper.cpp (lokal, optional auf der Grafikkarte).

use std::collections::VecDeque;
use std::path::Path;

use anyhow::{Context, Result};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
    WhisperVadContext, WhisperVadContextParams,
};

use crate::segment::{SpeechDetector, FRAME};

/// Schreibt mit, was gesagt wurde.
pub struct Transcriber {
    state: WhisperState,
    language: String,
    threads: i32,
}

impl Transcriber {
    pub fn load(model: &Path, language: &str) -> Result<Self> {
        silence_whisper_logs();
        let mut params = WhisperContextParameters::default();
        params.use_gpu(crate::GPU_BUILD);
        let ctx = WhisperContext::new_with_params(model, params)
            .with_context(|| format!("Whisper-Modell {} nicht ladbar", model.display()))?;
        let state = ctx
            .create_state()
            .context("Whisper-Zustand nicht anlegbar")?;
        let threads = std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(4) as i32;
        Ok(Self {
            state,
            language: language.to_string(),
            threads,
        })
    }

    /// 16-kHz-mono-Samples -> Text.
    pub fn transcribe(&mut self, samples: &[f32]) -> Result<String> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(if self.language == "auto" {
            "auto"
        } else {
            &self.language
        }));
        params.set_n_threads(self.threads);
        // Kurze Befehle: ein Segment, kein Kontext aus vorherigen Sätzen.
        params.set_single_segment(true);
        params.set_no_context(true);
        params.set_suppress_blank(true);
        params.set_no_timestamps(true);
        // Hilft Whisper, den Namen richtig zu schreiben.
        params.set_initial_prompt("Jarvis, öffne Minecraft. Jarvis, sende die Datei an den PC.");
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        // Whisper braucht mindestens 1 s Audio; kürzeres mit Stille auffüllen.
        let mut padded;
        let audio = if samples.len() < 16_000 + 1_600 {
            padded = samples.to_vec();
            padded.resize(16_000 + 1_600, 0.0);
            &padded[..]
        } else {
            samples
        };
        // Whisper rechnet sonst immer ein 30-s-Fenster. Für kurze Befehle nur so
        // viel Kontext wie nötig (50 Einheiten pro Sekunde, plus Reserve):
        // vielfach schneller bei kaum schlechterer Erkennung.
        let ctx = ((audio.len() as f32 / 16_000.0) * 50.0).ceil() as i32 + 64;
        params.set_audio_ctx(ctx.min(1500));
        self.state
            .full(params, audio)
            .context("Whisper fehlgeschlagen")?;
        let mut text = String::new();
        for seg in self.state.as_iter() {
            if seg.no_speech_probability() > 0.8 {
                continue;
            }
            text.push_str(&seg.to_str_lossy().unwrap_or_default());
        }
        Ok(text.trim().to_string())
    }
}

/// Sprachaktivität mit dem Silero-Modell (über whisper.cpp). Bewertet jeden
/// neuen Frame im Kontext der letzten ~0,5 s.
pub struct SileroDetector {
    vad: WhisperVadContext,
    window: VecDeque<f32>,
}

impl SileroDetector {
    const CONTEXT_FRAMES: usize = 16;

    pub fn load(model: &Path) -> Result<Self> {
        silence_whisper_logs();
        let mut p = WhisperVadContextParams::default();
        p.set_n_threads(1);
        p.set_use_gpu(false); // winzig, die CPU ist hier schneller
        let vad = WhisperVadContext::new(&model.to_string_lossy(), p)
            .with_context(|| format!("VAD-Modell {} nicht ladbar", model.display()))?;
        Ok(Self {
            vad,
            window: VecDeque::with_capacity(FRAME * Self::CONTEXT_FRAMES),
        })
    }
}

impl SpeechDetector for SileroDetector {
    fn speech_probability(&mut self, frame: &[f32]) -> f32 {
        self.window.extend(frame.iter().copied());
        while self.window.len() > FRAME * Self::CONTEXT_FRAMES {
            self.window.pop_front();
        }
        let samples: Vec<f32> = self.window.iter().copied().collect();
        if self.vad.detect_speech(&samples).is_err() {
            return 0.0;
        }
        self.vad.probabilities().last().copied().unwrap_or(0.0)
    }
}

fn silence_whisper_logs() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(whisper_rs::install_logging_hooks);
}
