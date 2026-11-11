//! Zerlegt den Mikrofonstrom in einzelne Äußerungen.
//!
//! Ein [`SpeechDetector`] bewertet jeden 32-ms-Frame; der [`Segmenter`]
//! startet eine Äußerung nach kurzer Sprache, hängt etwas Vorlauf an (damit
//! das „J" von „Jarvis" nicht fehlt) und beendet sie nach einer Pause.

use std::collections::VecDeque;

/// Silero arbeitet mit 512 Samples bei 16 kHz = 32 ms.
pub const FRAME: usize = 512;
const FRAME_MS: u32 = 32;

pub trait SpeechDetector {
    /// Wahrscheinlichkeit 0..1, dass in diesem Frame gesprochen wird.
    fn speech_probability(&mut self, frame: &[f32]) -> f32;
}

#[derive(Debug, Clone)]
pub struct SegmenterConfig {
    pub threshold: f32,
    /// So viele Sprach-Frames am Stück starten eine Äußerung.
    pub start_frames: usize,
    /// Pause, nach der eine Äußerung endet.
    pub end_silence_ms: u32,
    /// Vorlauf vor dem erkannten Sprachbeginn.
    pub preroll_ms: u32,
    /// Kürzere Geräusche werden verworfen.
    pub min_speech_ms: u32,
    pub max_ms: u32,
}

impl Default for SegmenterConfig {
    fn default() -> Self {
        Self {
            threshold: 0.5,
            start_frames: 3,
            end_silence_ms: 700,
            preroll_ms: 300,
            min_speech_ms: 250,
            max_ms: 15_000,
        }
    }
}

struct Active {
    samples: Vec<f32>,
    speech_frames: usize,
    silence_frames: usize,
}

pub struct Segmenter<D> {
    detector: D,
    cfg: SegmenterConfig,
    pending: Vec<f32>,
    preroll: VecDeque<Vec<f32>>,
    speech_run: usize,
    active: Option<Active>,
}

impl<D: SpeechDetector> Segmenter<D> {
    pub fn new(detector: D, cfg: SegmenterConfig) -> Self {
        Self {
            detector,
            cfg,
            pending: Vec::new(),
            preroll: VecDeque::new(),
            speech_run: 0,
            active: None,
        }
    }

    /// Verwirft alles Angefangene, z. B. nachdem Jarvis selbst gesprochen hat.
    pub fn reset(&mut self) {
        self.pending.clear();
        self.preroll.clear();
        self.speech_run = 0;
        self.active = None;
    }

    pub fn is_listening_to_speech(&self) -> bool {
        self.active.is_some()
    }

    /// 16-kHz-Samples hinein, fertige Äußerungen heraus.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        self.pending.extend_from_slice(samples);
        let mut done = Vec::new();
        let frames = self.pending.len() / FRAME;
        let pending = std::mem::take(&mut self.pending);
        for frame in pending.chunks_exact(FRAME).take(frames) {
            if let Some(seg) = self.on_frame(frame) {
                done.push(seg);
            }
        }
        self.pending = pending[frames * FRAME..].to_vec();
        done
    }

    /// Beendet eine laufende Äußerung (z. B. am Dateiende).
    pub fn flush(&mut self) -> Option<Vec<f32>> {
        let a = self.active.take()?;
        self.reset();
        (a.speech_frames >= self.frames(self.cfg.min_speech_ms)).then_some(a.samples)
    }

    fn frames(&self, ms: u32) -> usize {
        (ms / FRAME_MS).max(1) as usize
    }

    fn on_frame(&mut self, frame: &[f32]) -> Option<Vec<f32>> {
        let speech = self.detector.speech_probability(frame) >= self.cfg.threshold;
        let (end_silence, max_frames, min_speech) = (
            self.frames(self.cfg.end_silence_ms),
            self.frames(self.cfg.max_ms),
            self.frames(self.cfg.min_speech_ms),
        );

        if let Some(a) = &mut self.active {
            a.samples.extend_from_slice(frame);
            if speech {
                a.speech_frames += 1;
                a.silence_frames = 0;
            } else {
                a.silence_frames += 1;
            }
            if a.silence_frames >= end_silence || a.samples.len() >= max_frames * FRAME {
                let a = self.active.take().expect("aktiv");
                self.speech_run = 0;
                return (a.speech_frames >= min_speech).then_some(a.samples);
            }
            return None;
        }

        self.preroll.push_back(frame.to_vec());
        while self.preroll.len() > self.frames(self.cfg.preroll_ms) + self.cfg.start_frames {
            self.preroll.pop_front();
        }
        if !speech {
            self.speech_run = 0;
            return None;
        }
        self.speech_run += 1;
        if self.speech_run >= self.cfg.start_frames {
            self.active = Some(Active {
                samples: self.preroll.drain(..).flatten().collect(),
                speech_frames: self.speech_run,
                silence_frames: 0,
            });
        }
        None
    }
}

/// Einfache, lautstärkebasierte Erkennung mit mitlaufendem Grundrauschen.
/// Rückfall, wenn das Silero-Modell fehlt, und für Tests.
pub struct EnergyDetector {
    floor: f32,
}

impl Default for EnergyDetector {
    fn default() -> Self {
        Self { floor: 0.003 }
    }
}

impl SpeechDetector for EnergyDetector {
    fn speech_probability(&mut self, frame: &[f32]) -> f32 {
        let rms = (frame.iter().map(|v| v * v).sum::<f32>() / frame.len() as f32).sqrt();
        if rms < self.floor {
            self.floor = 0.9 * self.floor + 0.1 * rms;
        } else {
            self.floor *= 1.002;
        }
        self.floor = self.floor.max(1e-4);
        ((rms / self.floor - 2.0) / 4.0).clamp(0.0, 1.0)
    }
}

impl SpeechDetector for Box<dyn SpeechDetector + Send> {
    fn speech_probability(&mut self, frame: &[f32]) -> f32 {
        (**self).speech_probability(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Erkennt "Sprache" an der Amplitude – deterministisch für Tests.
    struct Loud;
    impl SpeechDetector for Loud {
        fn speech_probability(&mut self, f: &[f32]) -> f32 {
            if f[0].abs() > 0.1 {
                1.0
            } else {
                0.0
            }
        }
    }

    fn ms(level: f32, ms: usize) -> Vec<f32> {
        vec![level; ms * 16]
    }

    #[test]
    fn one_utterance_with_preroll() {
        let mut s = Segmenter::new(Loud, SegmenterConfig::default());
        let mut audio = ms(0.0, 1000);
        audio.extend(ms(0.5, 1000));
        audio.extend(ms(0.0, 1000));
        let segs: Vec<_> = audio.chunks(1234).flat_map(|c| s.push(c)).collect();
        assert_eq!(segs.len(), 1);
        let len_ms = segs[0].len() / 16;
        // 1 s Sprache + ~300 ms Vorlauf + ~700 ms Pause
        assert!((1900..=2200).contains(&len_ms), "{len_ms}");
        assert!(segs[0][0].abs() < 0.01, "Vorlauf fehlt");
    }

    #[test]
    fn short_click_is_ignored_and_pause_splits() {
        let mut s = Segmenter::new(Loud, SegmenterConfig::default());
        let mut audio = ms(0.5, 150); // Klick
        audio.extend(ms(0.0, 1000));
        audio.extend(ms(0.5, 800)); // „Jarvis"
        audio.extend(ms(0.0, 200)); // kurze Atempause – gehört dazu
        audio.extend(ms(0.5, 800)); // „öffne Minecraft"
        audio.extend(ms(0.0, 1000));
        let segs = s.push(&audio);
        assert_eq!(segs.len(), 1, "Klick verworfen, Atempause trennt nicht");
    }

    #[test]
    fn long_speech_is_cut() {
        let mut s = Segmenter::new(
            Loud,
            SegmenterConfig {
                max_ms: 2000,
                ..Default::default()
            },
        );
        let segs = s.push(&ms(0.5, 5000));
        assert_eq!(segs.len(), 2);
    }

    #[test]
    fn energy_detector_adapts_to_noise() {
        let mut d = EnergyDetector::default();
        let noise: Vec<f32> = (0..FRAME)
            .map(|i| if i % 2 == 0 { 0.01 } else { -0.01 })
            .collect();
        for _ in 0..100 {
            d.speech_probability(&noise);
        }
        assert!(d.speech_probability(&noise) < 0.5);
        let voice: Vec<f32> = noise.iter().map(|v| v * 20.0).collect();
        assert!(d.speech_probability(&voice) >= 0.5);
    }
}
