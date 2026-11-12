//! `mira-listen` – hört auf „Mira …" und gibt Befehle an `mirad` weiter.
//!
//!   mira-listen                       Mikrofon
//!   mira-listen --mikrofone           verfügbare Mikrofone zeigen
//!   mira-listen --datei befehl.wav    Aufnahme statt Mikrofon (Test)
//!   mira-listen --trocken             nur anzeigen, nichts ausführen

use std::io::Write;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};
use mira_node::config::{Config, VoiceSection, DEFAULT_CONTROL_PORT};
use mira_node::control::{self, Request, Response};
use mira_node::events::HudEvent;
use mira_node::exec;
use mira_voice::audio::{downmix, Resampler};
use mira_voice::segment::{EnergyDetector, Segmenter, SegmenterConfig, SpeechDetector};
use mira_voice::stt::{SileroDetector, Transcriber};
use mira_voice::wake::{Conversation, Decision};
use mira_voice::{models, SAMPLE_RATE};
use tracing::{debug, info, warn};

#[derive(Parser)]
#[command(name = "mira-listen", about = "Spracheingabe für Mira")]
struct Cli {
    /// Verfügbare Mikrofone anzeigen.
    #[arg(long)]
    mikrofone: bool,
    /// WAV-Datei statt Mikrofon verarbeiten (zum Testen).
    #[arg(long)]
    datei: Option<PathBuf>,
    /// Erkannten Text nur anzeigen, nicht an mirad senden.
    #[arg(long)]
    trocken: bool,
    /// Jeder Satz ist ein Befehl, kein „Mira" nötig.
    #[arg(long)]
    ohne_weckwort: bool,
    /// Antworten nicht vorlesen.
    #[arg(long)]
    leise: bool,
    /// Whisper-Modell (Name oder Pfad), z. B. "large-v3-turbo-q5_0".
    #[arg(long)]
    modell: Option<String>,
    /// Nur die Modelle herunterladen und beenden (für den Installer).
    #[arg(long)]
    nur_modelle: bool,
}

type Detector = Box<dyn SpeechDetector + Send>;

struct App {
    voice: VoiceSection,
    control_port: u16,
    token_path: PathBuf,
    dry: bool,
    speak: bool,
    stt: Transcriber,
    conversation: Conversation,
    events: EventSink,
}

/// Schickt HUD-Ereignisse an mirad (ohne auf Antwort zu warten).
struct EventSink {
    port: u16,
    token_path: PathBuf,
    stream: Option<TcpStream>,
    last_level: Instant,
}

impl EventSink {
    fn new(port: u16, token_path: PathBuf) -> Self {
        Self {
            port,
            token_path,
            stream: None,
            last_level: Instant::now(),
        }
    }

    fn emit(&mut self, event: HudEvent) {
        let mut line = match serde_json::to_string(&Request::Event { event }) {
            Ok(l) => l,
            Err(_) => return,
        };
        line.push('\n');
        if self.stream.is_none() {
            self.stream = control::connect_blocking(self.port, &self.token_path)
                .ok()
                .map(|(s, _)| s);
        }
        if let Some(s) = &mut self.stream {
            if s.write_all(line.as_bytes()).is_err() {
                self.stream = None;
            }
        }
    }

    /// Mikrofonpegel, höchstens 20-mal pro Sekunde.
    fn level(&mut self, samples: &[f32]) {
        if samples.is_empty() || self.last_level.elapsed() < Duration::from_millis(50) {
            return;
        }
        self.last_level = Instant::now();
        let rms = (samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32).sqrt();
        self.emit(HudEvent::Level {
            value: (rms * 8.0).clamp(0.0, 1.0),
        });
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cli = Cli::parse();

    if cli.mikrofone {
        return list_microphones();
    }

    let cfg = Config::load().ok();
    let voice = cfg.as_ref().map(|c| c.voice.clone()).unwrap_or_default();
    let control_port = cfg
        .as_ref()
        .map(|c| c.mesh.control_port)
        .unwrap_or(DEFAULT_CONTROL_PORT);
    let token_path = cfg
        .as_ref()
        .map(|c| c.control_token_path())
        .unwrap_or_else(Config::default_control_token_path);

    let model_name = cli
        .modell
        .clone()
        .or(voice.model.clone())
        .unwrap_or_else(|| mira_voice::default_model().into());
    let model = models::ensure(&model_name)?;
    if cli.nur_modelle {
        models::ensure(models::VAD_MODEL)?;
        println!("Modelle bereit: {model_name}, {}", models::VAD_MODEL);
        return Ok(());
    }
    let started = Instant::now();
    let stt = Transcriber::load(&model, &voice.language)?;
    info!(
        "Whisper {model_name} geladen in {:.1} s ({})",
        started.elapsed().as_secs_f32(),
        if mira_voice::GPU_BUILD {
            "Grafikkarte"
        } else {
            "nur Prozessor"
        }
    );

    let detector: Detector =
        match models::ensure(models::VAD_MODEL).and_then(|p| SileroDetector::load(&p)) {
            Ok(d) => Box::new(d),
            Err(e) => {
                warn!("Silero-VAD nicht verfügbar ({e:#}) – nutze Lautstärke-Erkennung");
                Box::new(EnergyDetector::default())
            }
        };
    let segmenter = Segmenter::new(detector, SegmenterConfig::default());

    let mut app = App {
        conversation: Conversation::new(
            voice.wake_words.clone(),
            Duration::from_secs(voice.follow_up_seconds),
            cli.ohne_weckwort,
        ),
        speak: voice.speak_replies && !cli.leise,
        voice,
        control_port,
        dry: cli.trocken,
        events: EventSink::new(control_port, token_path.clone()),
        token_path,
        stt,
    };

    match cli.datei {
        Some(path) => app.run_file(&path, segmenter),
        None => app.run_microphone(segmenter),
    }
}

impl App {
    fn run_file(&mut self, path: &PathBuf, mut segmenter: Segmenter<Detector>) -> Result<()> {
        let reader = hound::WavReader::open(path)
            .with_context(|| format!("{} nicht lesbar", path.display()))?;
        let spec = reader.spec();
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader.into_samples::<f32>().collect::<Result<_, _>>()?,
            hound::SampleFormat::Int => {
                let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .into_samples::<i32>()
                    .map(|s| s.map(|v| v as f32 / scale))
                    .collect::<Result<_, _>>()?
            }
        };
        let mono = downmix(&samples, spec.channels as usize);
        let mut resampler = Resampler::new(spec.sample_rate, SAMPLE_RATE);
        // In 100-ms-Häppchen, wie vom Mikrofon.
        for chunk in mono.chunks((spec.sample_rate / 10) as usize) {
            for seg in segmenter.push(&resampler.process(chunk)) {
                self.on_segment(&seg);
            }
        }
        // Eine Sekunde Stille hinterher, damit die letzte Äußerung endet.
        for seg in segmenter.push(&vec![0.0; SAMPLE_RATE as usize]) {
            self.on_segment(&seg);
        }
        if let Some(seg) = segmenter.flush() {
            self.on_segment(&seg);
        }
        Ok(())
    }

    fn run_microphone(&mut self, mut segmenter: Segmenter<Detector>) -> Result<()> {
        let host = cpal::default_host();
        let device = match &self.voice.input_device {
            Some(wanted) => host
                .input_devices()?
                .find(|d| {
                    d.description()
                        .map(|n| {
                            n.to_string()
                                .to_lowercase()
                                .contains(&wanted.to_lowercase())
                        })
                        .unwrap_or(false)
                })
                .with_context(|| {
                    format!("Kein Mikrofon passt zu „{wanted}“ (siehe --mikrofone)")
                })?,
            None => host
                .default_input_device()
                .context("Kein Mikrofon gefunden")?,
        };
        let supported = device
            .default_input_config()
            .context("Mikrofon liefert keine Konfiguration")?;
        let rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let (tx, rx) = mpsc::channel::<Vec<f32>>();
        let config = supported.config();
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => input_stream::<f32>(&device, config, channels, tx)?,
            cpal::SampleFormat::I16 => input_stream::<i16>(&device, config, channels, tx)?,
            cpal::SampleFormat::U16 => input_stream::<u16>(&device, config, channels, tx)?,
            cpal::SampleFormat::I32 => input_stream::<i32>(&device, config, channels, tx)?,
            other => bail!("Mikrofon-Format {other} wird nicht unterstützt"),
        };
        stream.play()?;
        let name = device
            .description()
            .map(|d| d.to_string())
            .unwrap_or_else(|_| "Mikrofon".into());
        info!("Höre zu über „{name}“ ({rate} Hz) – sag „Mira, …“");

        let mut resampler = Resampler::new(rate, SAMPLE_RATE);
        while let Ok(chunk) = rx.recv() {
            let samples = resampler.process(&chunk);
            if self.conversation.is_armed(Instant::now()) {
                self.events.level(&samples);
            }
            for seg in segmenter.push(&samples) {
                if self.on_segment(&seg) {
                    // Mira hat gesprochen: das eigene Echo nicht als Befehl verstehen.
                    while rx.try_recv().is_ok() {}
                    segmenter.reset();
                }
            }
        }
        bail!("Mikrofon-Strom beendet")
    }

    /// Verarbeitet eine Äußerung. `true`, wenn Mira etwas vorgelesen hat.
    fn on_segment(&mut self, samples: &[f32]) -> bool {
        let t = Instant::now();
        let text = match self.stt.transcribe(samples) {
            Ok(t) => t,
            Err(e) => {
                warn!("{e:#}");
                return false;
            }
        };
        let secs = samples.len() as f32 / SAMPLE_RATE as f32;
        info!(
            "„{text}“ ({secs:.1} s Audio, erkannt in {} ms)",
            t.elapsed().as_millis()
        );

        match self.conversation.on_text(&text, Instant::now()) {
            Decision::Ignore => {
                debug!("kein Weckwort – ignoriert");
                false
            }
            Decision::Acknowledge => {
                self.events.emit(HudEvent::Wake);
                self.say("Ja?")
            }
            Decision::Command(cmd) => {
                println!("» {cmd}");
                self.events.emit(HudEvent::Wake);
                if self.dry {
                    return false;
                }
                let reply = match ask_mirad(self.control_port, &self.token_path, &cmd) {
                    Ok(r) => r.text.unwrap_or_default(),
                    Err(e) => format!("{e:#}"),
                };
                println!("« {reply}");
                self.say(&reply)
            }
        }
    }

    fn say(&mut self, text: &str) -> bool {
        if !self.speak || text.is_empty() {
            return false;
        }
        self.events.emit(HudEvent::Speaking { active: true });
        if let Err(e) = exec::say_blocking(text) {
            warn!("{e:#}");
        }
        self.events.emit(HudEvent::Speaking { active: false });
        true
    }
}

fn input_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    tx: mpsc::Sender<Vec<f32>>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    Ok(device.build_input_stream(
        config,
        move |data: &[T], _: &_| {
            let samples: Vec<f32> = data.iter().map(|s| f32::from_sample(*s)).collect();
            let _ = tx.send(downmix(&samples, channels));
        },
        |e| warn!("Mikrofon: {e}"),
        None,
    )?)
}

fn ask_mirad(port: u16, token_path: &std::path::Path, text: &str) -> Result<Response> {
    control::request_blocking(
        port,
        token_path,
        &Request::Text {
            text: text.to_string(),
            source: Some("Sprache".into()),
        },
    )
}

fn list_microphones() -> Result<()> {
    let host = cpal::default_host();
    let default = host
        .default_input_device()
        .and_then(|d| d.description().ok())
        .map(|d| d.to_string());
    for d in host.input_devices()? {
        let name = d
            .description()
            .map(|n| n.to_string())
            .unwrap_or_else(|_| "?".into());
        let marker = if Some(&name) == default.as_ref() {
            "  (Standard)"
        } else {
            ""
        };
        println!("• {name}{marker}");
    }
    Ok(())
}
