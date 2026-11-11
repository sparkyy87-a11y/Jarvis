//! `jarvis-einstellungen` – Jarvis einrichten, ohne Konfigurationsdateien anzufassen.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod form;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};
use form::Form;
use jarvis_node::autostart;
use jarvis_node::config::{BrainProvider, Config};
use jarvis_node::control::{self, Request};

const CYAN: Color32 = Color32::from_rgb(79, 209, 255);
const AMBER: Color32 = Color32::from_rgb(255, 170, 70);
const GREEN: Color32 = Color32::from_rgb(110, 220, 140);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Devices,
    Voice,
    Brain,
    MusicApps,
    Hud,
}

impl Page {
    const ALL: [(Page, &'static str); 6] = [
        (Page::Overview, "Übersicht"),
        (Page::Devices, "Geräte"),
        (Page::Voice, "Sprache"),
        (Page::Brain, "Sprachmodell"),
        (Page::MusicApps, "Musik & Apps"),
        (Page::Hud, "HUD"),
    ];
}

/// Ergebnis einer Hintergrundaufgabe (damit das Fenster nie hängt).
struct Task<T>(Option<mpsc::Receiver<T>>);

impl<T: Send + 'static> Task<T> {
    fn none() -> Self {
        Task(None)
    }
    fn spawn(ctx: &egui::Context, f: impl FnOnce() -> T + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f());
            ctx.request_repaint();
        });
        Task(Some(rx))
    }
    fn running(&self) -> bool {
        self.0.is_some()
    }
    fn poll(&mut self) -> Option<T> {
        let v = self.0.as_ref()?.try_recv().ok()?;
        self.0 = None;
        Some(v)
    }
}

struct App {
    cfg: Option<Config>,
    form: Form,
    saved: Form,
    page: Page,
    message: Option<(bool, String)>,

    running: bool,
    autostart_on: bool,
    trusted: Vec<(String, bool)>,
    trusted_task: Task<Result<Vec<(String, bool)>, String>>,
    last_refresh: Instant,
    pairing: Option<(String, egui::TextureHandle, Instant)>,
    pair_task: Task<Result<String, String>>,
    revoke_confirm: Option<String>,
    revoke_task: Task<Result<String, String>>,
    microphones: Vec<String>,
    mic_task: Task<Vec<String>>,
    ollama: Option<Result<Vec<String>, String>>,
    ollama_task: Task<Result<Vec<String>, String>>,
    save_task: Task<Result<String, String>>,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        let cfg = Config::load().ok();
        let form = cfg.as_ref().map(Form::from_config).unwrap_or_else(|| {
            Form::from_config(&Config::new("Mein PC".into()).expect("Schlüssel erzeugen"))
        });
        let mut app = Self {
            saved: form.clone(),
            form,
            cfg,
            page: Page::Overview,
            message: None,
            running: autostart::is_running(),
            autostart_on: autostart::status().map(|s| s.enabled).unwrap_or(false),
            trusted: Vec::new(),
            trusted_task: Task::none(),
            last_refresh: Instant::now() - Duration::from_secs(60),
            pairing: None,
            pair_task: Task::none(),
            revoke_confirm: None,
            revoke_task: Task::none(),
            microphones: Vec::new(),
            mic_task: Task::spawn(ctx, list_microphones),
            ollama: None,
            ollama_task: Task::none(),
            save_task: Task::none(),
        };
        app.check_ollama(ctx);
        // `--koppeln` (z. B. von „Jarvis, verbinde dich mit einem neuen Gerät“):
        // direkt die Geräte-Seite mit frischem QR-Code, im Vordergrund.
        if std::env::args().any(|a| a == "--koppeln") {
            app.page = Page::Devices;
            app.request_pairing(ctx);
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        app
    }

    fn control(&self) -> Option<(u16, std::path::PathBuf)> {
        self.cfg
            .as_ref()
            .map(|c| (c.mesh.control_port, c.control_token_path()))
    }

    fn request_pairing(&mut self, ctx: &egui::Context) {
        if let Some((port, token)) = self.control() {
            self.pair_task = Task::spawn(ctx, move || {
                let r = control::request_blocking(port, &token, &Request::Pair { addrs: None })
                    .map_err(|e| format!("{e:#}"))?;
                r.link
                    .ok_or_else(|| r.text.unwrap_or_else(|| "Kein Code erhalten".into()))
            });
        }
    }

    fn check_ollama(&mut self, ctx: &egui::Context) {
        let url = if self.form.ollama_url.trim().is_empty() {
            "http://127.0.0.1:11434".to_string()
        } else {
            self.form
                .ollama_url
                .trim()
                .trim_end_matches('/')
                .to_string()
        };
        self.ollama_task = Task::spawn(ctx, move || ollama_models(&url));
    }

    fn poll_tasks(&mut self, ctx: &egui::Context) {
        if let Some(r) = self.trusted_task.poll() {
            self.running = r.is_ok();
            if let Ok(list) = r {
                self.trusted = list;
            }
        }
        if let Some(r) = self.pair_task.poll() {
            match r {
                Ok(link) => {
                    let tex = qr_texture(ctx, &link);
                    self.pairing = Some((link, tex, Instant::now()));
                }
                Err(e) => self.message = Some((false, e)),
            }
        }
        if let Some(r) = self.revoke_task.poll() {
            self.message = Some(match r {
                Ok(t) => (true, t),
                Err(e) => (false, e),
            });
            self.last_refresh = Instant::now() - Duration::from_secs(60);
        }
        if let Some(m) = self.mic_task.poll() {
            self.microphones = m;
        }
        if let Some(o) = self.ollama_task.poll() {
            self.ollama = Some(o);
        }
        if let Some(r) = self.save_task.poll() {
            match r {
                Ok(t) => {
                    self.saved = self.form.clone();
                    self.cfg = Config::load().ok();
                    self.running = autostart::is_running();
                    self.message = Some((true, t));
                }
                Err(e) => self.message = Some((false, e)),
            }
        }
        // Geräteliste regelmäßig auffrischen
        if self.last_refresh.elapsed() > Duration::from_secs(3) && !self.trusted_task.running() {
            self.last_refresh = Instant::now();
            if let Some((port, token)) = self.control() {
                self.trusted_task = Task::spawn(ctx, move || {
                    control::request_blocking(port, &token, &Request::Trusted)
                        .map(|r| {
                            r.trusted
                                .unwrap_or_default()
                                .into_iter()
                                .map(|t| (t.name, t.online))
                                .collect()
                        })
                        .map_err(|e| format!("{e:#}"))
                });
            }
        }
    }

    fn save(&mut self, ctx: &egui::Context) {
        let problems = self.form.problems();
        if !problems.is_empty() {
            self.message = Some((false, problems.join(" ")));
            return;
        }
        let form = self.form.clone();
        let existing = self.cfg.clone();
        self.save_task = Task::spawn(ctx, move || {
            let first = existing.is_none();
            let mut cfg = match existing {
                Some(c) => c,
                None => Config::new(form.name.clone()).map_err(|e| format!("{e:#}"))?,
            };
            form.apply(&mut cfg);
            let path = cfg.source.clone().unwrap_or_else(Config::path);
            cfg.save_to(&path)
                .map_err(|e| format!("Speichern fehlgeschlagen: {e:#}"))?;
            if first {
                autostart::enable().map_err(|e| format!("{e:#}"))?;
                return Ok("Jarvis ist eingerichtet und läuft.".into());
            }
            match autostart::restart() {
                Ok(true) => Ok(
                    "Gespeichert – Jarvis wurde mit den neuen Einstellungen neu gestartet.".into(),
                ),
                Ok(false) => Ok("Gespeichert – gilt beim nächsten Start von Jarvis.".into()),
                Err(e) => Err(format!("Gespeichert, aber Neustart fehlgeschlagen: {e:#}")),
            }
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_tasks(ctx);
        ctx.request_repaint_after(Duration::from_secs(1));

        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(190.0)
            .show(ctx, |ui| {
                ui.add_space(14.0);
                ui.label(RichText::new("JARVIS").size(24.0).strong().color(CYAN));
                ui.label(RichText::new("Einstellungen").weak());
                ui.add_space(18.0);
                for (page, label) in Page::ALL {
                    let selected = self.page == page;
                    let text = RichText::new(label).size(15.0);
                    if ui
                        .add_sized([170.0, 32.0], egui::Button::selectable(selected, text))
                        .clicked()
                    {
                        self.page = page;
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.add_space(10.0);
                    let (dot, text) = if self.running {
                        (GREEN, "Jarvis läuft")
                    } else {
                        (AMBER, "Jarvis läuft nicht")
                    };
                    ui.horizontal(|ui| {
                        self::dot(ui, dot);
                        ui.label(text);
                    });
                });
            });

        egui::TopBottomPanel::bottom("actions").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let dirty = self.form != self.saved || self.cfg.is_none();
                let label = if self.cfg.is_none() {
                    "Einrichten & starten"
                } else {
                    "Speichern & übernehmen"
                };
                let busy = self.save_task.running();
                if ui
                    .add_enabled(
                        dirty && !busy,
                        egui::Button::new(RichText::new(label).strong())
                            .min_size([200.0, 32.0].into()),
                    )
                    .clicked()
                {
                    self.save(ctx);
                }
                if ui
                    .add_enabled(dirty && !busy, egui::Button::new("Verwerfen"))
                    .clicked()
                {
                    self.form = self.saved.clone();
                    self.message = None;
                }
                if busy {
                    ui.spinner();
                }
                if let Some((ok, msg)) = &self.message {
                    ui.label(RichText::new(msg).color(if *ok { GREEN } else { AMBER }));
                }
            });
            ui.add_space(8.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(6.0);
                match self.page {
                    Page::Overview => self.page_overview(ui),
                    Page::Devices => self.page_devices(ui, ctx),
                    Page::Voice => self.page_voice(ui),
                    Page::Brain => self.page_brain(ui, ctx),
                    Page::MusicApps => self.page_music_apps(ui),
                    Page::Hud => self.page_hud(ui),
                }
            });
        });

        self.revoke_dialog(ctx);
    }
}

/// Farbiger Statuspunkt (gezeichnet – nicht jede Schrift hat „●“).
fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.5, color);
}

fn status(ui: &mut egui::Ui, ok: bool, text: &str) {
    ui.horizontal(|ui| {
        dot(ui, if ok { GREEN } else { AMBER });
        ui.label(RichText::new(text).color(if ok { GREEN } else { AMBER }));
    });
}

fn heading(ui: &mut egui::Ui, title: &str, sub: &str) {
    ui.label(RichText::new(title).size(22.0).strong());
    ui.label(RichText::new(sub).weak());
    ui.add_space(12.0);
}

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style())
        .inner_margin(12.0)
        .corner_radius(8.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().color(CYAN));
            ui.add_space(6.0);
            add(ui);
        });
    ui.add_space(10.0);
}

impl App {
    fn page_overview(&mut self, ui: &mut egui::Ui) {
        heading(
            ui,
            "Übersicht",
            "Wie dieses Gerät heißt und ob Jarvis läuft.",
        );
        if self.cfg.is_none() {
            section(ui, "Willkommen", |ui| {
                ui.label("Jarvis ist auf diesem Gerät noch nicht eingerichtet. Gib ihm einen Namen und klicke unten auf „Einrichten & starten“.");
                ui.label(RichText::new("Soll es mit einem vorhandenen Jarvis gekoppelt werden, nutze stattdessen den Installer oder `jarvisd join`.").weak());
            });
        }
        section(ui, "Dieses Gerät", |ui| {
            egui::Grid::new("dev")
                .num_columns(2)
                .spacing([16.0, 10.0])
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut self.form.name).desired_width(280.0));
                    ui.end_row();
                    ui.label("Weitere Namen");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.form.aliases)
                            .hint_text("z. B. großer Rechner, Zocker-PC")
                            .desired_width(280.0),
                    );
                    ui.end_row();
                });
            ui.label(RichText::new("Tipp: Ein Name mit dem Wort, das du sagst (Gaming-PC, Laptop, iPad Büro), macht „am PC“ eindeutig.").weak());
        });
        section(ui, "Start", |ui| {
            status(
                ui,
                self.running,
                if self.running {
                    "Jarvis läuft"
                } else {
                    "Jarvis läuft nicht"
                },
            );
            let mut on = self.autostart_on;
            if ui
                .checkbox(&mut on, "Bei jeder Anmeldung automatisch starten")
                .changed()
                && self.cfg.is_some()
            {
                let r = if on {
                    autostart::enable()
                } else {
                    autostart::disable()
                };
                match r {
                    Ok(()) => {
                        self.autostart_on = on;
                        self.running = autostart::is_running();
                    }
                    Err(e) => self.message = Some((false, format!("{e:#}"))),
                }
            }
            if self.cfg.is_some() && ui.button("Jarvis neu starten").clicked() {
                self.message = Some(match autostart::restart() {
                    Ok(true) => (true, "Jarvis wurde neu gestartet.".into()),
                    Ok(false) => match autostart::enable() {
                        Ok(()) => (true, "Jarvis wurde gestartet.".into()),
                        Err(e) => (false, format!("{e:#}")),
                    },
                    Err(e) => (false, format!("{e:#}")),
                });
                self.running = autostart::is_running();
            }
            if let Ok(s) = autostart::status() {
                ui.label(
                    RichText::new(format!("Protokolle: {}", s.logs.display()))
                        .weak()
                        .small(),
                );
            }
        });
    }

    fn page_devices(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        heading(
            ui,
            "Geräte",
            "Deine gekoppelten Geräte – PC, Laptop, Handy, Tablets.",
        );
        section(ui, "Gekoppelt", |ui| {
            if !self.running {
                ui.label(
                    RichText::new("Jarvis läuft nicht – die Liste erscheint, sobald Jarvis läuft.")
                        .color(AMBER),
                );
            } else if self.trusted.is_empty() {
                ui.label("Noch keine weiteren Geräte. Koppel unten das erste.");
            }
            for (name, online) in self.trusted.clone() {
                ui.horizontal(|ui| {
                    dot(ui, if online { GREEN } else { Color32::GRAY });
                    ui.label(RichText::new(&name).size(15.0));
                    ui.label(
                        RichText::new(if online {
                            "verbunden"
                        } else {
                            "nicht verbunden"
                        })
                        .weak(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Sperren …").clicked() {
                            self.revoke_confirm = Some(name.clone());
                        }
                    });
                });
            }
        });
        section(ui, "Neues Gerät koppeln", |ui| {
            if let Some((link, tex, since)) = &self.pairing {
                let left = 600i64 - since.elapsed().as_secs() as i64;
                if left <= 0 {
                    ui.label(RichText::new("Der Code ist abgelaufen.").color(AMBER));
                } else {
                    ui.horizontal(|ui| {
                        ui.image((tex.id(), egui::vec2(220.0, 220.0)));
                        ui.vertical(|ui| {
                            ui.label("Handy/Tablet: QR-Code mit der Kamera scannen, Namen eingeben, bestätigen.");
                            ui.add_space(6.0);
                            ui.label("PC/Laptop: Code in den Installer einfügen.");
                            if ui.button("Code kopieren").clicked() {
                                ui.ctx().copy_text(link.clone());
                            }
                            ui.add_space(6.0);
                            ui.label(RichText::new(format!("Gilt noch {}:{:02} Minuten, nur für ein Gerät.", left / 60, left % 60)).weak());
                        });
                    });
                }
            }
            let label = if self.pairing.is_some() {
                "Neuen Code erzeugen"
            } else {
                "Code erzeugen"
            };
            if ui
                .add_enabled(
                    self.running && !self.pair_task.running(),
                    egui::Button::new(label),
                )
                .clicked()
            {
                self.request_pairing(ctx);
            }
        });
        section(ui, "Erweitert", |ui| {
            ui.checkbox(
                &mut self.form.mdns,
                "Eigene Geräte im WLAN automatisch finden (mDNS)",
            );
            ui.label(
                "Feste Adressen (eine pro Zeile), falls die automatische Suche im Netz nicht geht:",
            );
            ui.add(
                egui::TextEdit::multiline(&mut self.form.static_peers)
                    .hint_text("192.168.1.20:7879")
                    .desired_rows(3)
                    .desired_width(320.0),
            );
        });
    }

    fn revoke_dialog(&mut self, ctx: &egui::Context) {
        let Some(name) = self.revoke_confirm.clone() else {
            return;
        };
        let modal = egui::Modal::new(egui::Id::new("revoke")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(format!("„{name}“ sperren?")).size(18.0).strong());
            ui.add_space(6.0);
            ui.label("Das Gerät wird sofort getrennt und kann sich nicht mehr verbinden – auch nicht mit deinen anderen Geräten. Gedacht für verlorene oder verkaufte Geräte.");
            ui.label(RichText::new("Zum erneuten Koppeln braucht es einen neuen Code und eine neue Installation der App.").weak());
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Sperren").color(AMBER)).clicked() {
                    if let Some((port, token)) = self.control() {
                        let n = name.clone();
                        self.revoke_task = Task::spawn(ctx, move || {
                            let r = control::request_blocking(port, &token, &Request::Revoke { name: n }).map_err(|e| format!("{e:#}"))?;
                            let text = r.text.unwrap_or_default();
                            if r.ok { Ok(text) } else { Err(text) }
                        });
                    }
                    self.revoke_confirm = None;
                }
                if ui.button("Abbrechen").clicked() {
                    self.revoke_confirm = None;
                }
            });
        });
        if modal.should_close() {
            self.revoke_confirm = None;
        }
    }

    fn page_voice(&mut self, ui: &mut egui::Ui) {
        heading(
            ui,
            "Sprache",
            "„Jarvis, …“ – Spracheingabe und Vorlesen auf diesem Gerät.",
        );
        section(ui, "Spracheingabe", |ui| {
            ui.checkbox(&mut self.form.voice_enabled, "Auf „Jarvis“ hören");
            ui.add_enabled_ui(self.form.voice_enabled, |ui| {
                egui::Grid::new("voice")
                    .num_columns(2)
                    .spacing([16.0, 10.0])
                    .show(ui, |ui| {
                        ui.label("Mikrofon");
                        let current = self
                            .form
                            .microphone
                            .clone()
                            .unwrap_or_else(|| "Standardmikrofon".into());
                        egui::ComboBox::from_id_salt("mic")
                            .selected_text(current)
                            .width(300.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.form.microphone,
                                    None,
                                    "Standardmikrofon",
                                );
                                for m in &self.microphones {
                                    ui.selectable_value(
                                        &mut self.form.microphone,
                                        Some(m.clone()),
                                        m,
                                    );
                                }
                            });
                        ui.end_row();
                        ui.label("Sprache");
                        egui::ComboBox::from_id_salt("lang")
                            .selected_text(match self.form.language.as_str() {
                                "en" => "Englisch",
                                "auto" => "Automatisch erkennen",
                                _ => "Deutsch",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.form.language,
                                    "de".into(),
                                    "Deutsch",
                                );
                                ui.selectable_value(
                                    &mut self.form.language,
                                    "en".into(),
                                    "Englisch",
                                );
                                ui.selectable_value(
                                    &mut self.form.language,
                                    "auto".into(),
                                    "Automatisch erkennen",
                                );
                            });
                        ui.end_row();
                        ui.label("Erkennung");
                        let models: [(Option<String>, &str); 5] = [
                            (None, "Automatisch (passend zum Gerät)"),
                            (Some("base-q5_1".into()), "Schnell – für schwache Rechner"),
                            (Some("small-q5_1".into()), "Ausgewogen – ohne Grafikkarte"),
                            (
                                Some("large-v3-turbo-q5_0".into()),
                                "Genau – mit Grafikkarte",
                            ),
                            (
                                Some("large-v3-turbo".into()),
                                "Sehr genau – starke Grafikkarte",
                            ),
                        ];
                        let shown = models
                            .iter()
                            .find(|(m, _)| *m == self.form.whisper_model)
                            .map(|(_, l)| *l)
                            .unwrap_or("Eigenes Modell");
                        egui::ComboBox::from_id_salt("whisper")
                            .selected_text(shown)
                            .width(300.0)
                            .show_ui(ui, |ui| {
                                for (m, l) in models {
                                    ui.selectable_value(&mut self.form.whisper_model, m, l);
                                }
                            });
                        ui.end_row();
                        ui.label("Weckwort");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.wake_words)
                                .desired_width(300.0),
                        );
                        ui.end_row();
                        ui.label("Nach „Jarvis.“ warten");
                        ui.add(
                            egui::Slider::new(&mut self.form.follow_up_seconds, 3..=20)
                                .suffix(" s"),
                        );
                        ui.end_row();
                    });
            });
        });
        section(ui, "Antworten", |ui| {
            ui.checkbox(&mut self.form.speak_replies, "Antworten vorlesen");
        });
    }

    fn page_brain(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        heading(
            ui,
            "Sprachmodell",
            "Versteht freie Sätze wie „Mach mir Musik zum Arbeiten an“.",
        );
        section(ui, "Anbieter", |ui| {
            ui.radio_value(
                &mut self.form.provider,
                BrainProvider::Auto,
                "Automatisch – Claude, wenn ein Schlüssel da ist, sonst Ollama",
            );
            ui.radio_value(
                &mut self.form.provider,
                BrainProvider::Ollama,
                "Ollama – kostenlos, läuft auf diesem Rechner",
            );
            ui.radio_value(
                &mut self.form.provider,
                BrainProvider::Claude,
                "Claude – klüger, kostet je Anfrage (API-Schlüssel)",
            );
            ui.radio_value(
                &mut self.form.provider,
                BrainProvider::Off,
                "Keins – andere Geräte (z. B. der PC) übernehmen",
            );
        });
        if matches!(
            self.form.provider,
            BrainProvider::Auto | BrainProvider::Ollama
        ) {
            section(ui, "Ollama", |ui| {
                match &self.ollama {
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Prüfe Ollama …");
                        });
                    }
                    Some(Ok(models)) => {
                        status(
                            ui,
                            true,
                            &format!("Ollama läuft – {} Modell(e) installiert", models.len()),
                        );
                    }
                    Some(Err(e)) => {
                        status(ui, false, e);
                        ui.hyperlink_to("Ollama herunterladen", "https://ollama.com/download");
                    }
                }
                egui::Grid::new("ollama")
                    .num_columns(2)
                    .spacing([16.0, 10.0])
                    .show(ui, |ui| {
                        ui.label("Modell");
                        let models = match &self.ollama {
                            Some(Ok(m)) => m.clone(),
                            _ => Vec::new(),
                        };
                        let shown = if self.form.model.is_empty() {
                            "qwen2.5:3b (Standard)".to_string()
                        } else {
                            self.form.model.clone()
                        };
                        egui::ComboBox::from_id_salt("model")
                            .selected_text(shown)
                            .width(260.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.form.model,
                                    String::new(),
                                    "qwen2.5:3b (Standard)",
                                );
                                for m in models {
                                    ui.selectable_value(&mut self.form.model, m.clone(), m);
                                }
                            });
                        ui.end_row();
                        ui.label("Adresse");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.ollama_url)
                                .hint_text("http://127.0.0.1:11434")
                                .desired_width(260.0),
                        );
                        ui.end_row();
                    });
                if ui.button("Erneut prüfen").clicked() {
                    self.ollama = None;
                    self.check_ollama(ctx);
                }
            });
        }
        if matches!(
            self.form.provider,
            BrainProvider::Auto | BrainProvider::Claude
        ) {
            section(ui, "Claude", |ui| {
                ui.label("API-Schlüssel von console.anthropic.com (wird nur auf diesem Gerät gespeichert):");
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.api_key)
                        .password(true)
                        .hint_text("sk-ant-…")
                        .desired_width(360.0),
                );
                ui.label(RichText::new("Das kostenlose claude.ai-Abo gilt hierfür nicht; die API wird nach Verbrauch abgerechnet.").weak());
            });
        }
    }

    fn page_music_apps(&mut self, ui: &mut egui::Ui) {
        heading(
            ui,
            "Musik & Apps",
            "Womit „spiel …“ abspielt und eigene Kurzbefehle.",
        );
        section(ui, "Musikdienst", |ui| {
            ui.radio_value(
                &mut self.form.music_service,
                "apple-music".into(),
                "Apple Music",
            );
            ui.radio_value(&mut self.form.music_service, "spotify".into(), "Spotify");
            ui.radio_value(
                &mut self.form.music_service,
                "youtube-music".into(),
                "YouTube Music",
            );
        });
        section(ui, "Eigene Kurzbefehle", |ui| {
            ui.label(
                "„öffne <Name>“ startet dieses Programm – Vorrang vor der automatischen Suche.",
            );
            let mut remove = None;
            egui::Grid::new("apps")
                .num_columns(3)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Name").weak());
                    ui.label(RichText::new("Programm oder Befehl").weak());
                    ui.end_row();
                    for (i, (name, cmd)) in self.form.apps.iter_mut().enumerate() {
                        ui.add_sized(
                            [150.0, 24.0],
                            egui::TextEdit::singleline(name).hint_text("minecraft"),
                        );
                        ui.add_sized(
                            [360.0, 24.0],
                            egui::TextEdit::singleline(cmd).hint_text(r"C:\Spiele\Minecraft.exe"),
                        );
                        if ui.button("Entfernen").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
            if let Some(i) = remove {
                self.form.apps.remove(i);
            }
            if ui.button("+ Kurzbefehl").clicked() {
                self.form.apps.push((String::new(), String::new()));
            }
        });
    }

    fn page_hud(&mut self, ui: &mut egui::Ui) {
        heading(
            ui,
            "HUD",
            "Der leuchtende Ring, der sich beim Sprechen mit Jarvis einblendet.",
        );
        section(ui, "Anzeige", |ui| {
            ui.checkbox(&mut self.form.hud_enabled, "HUD einblenden");
            ui.add_enabled_ui(self.form.hud_enabled, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Position");
                    let positions = [
                        ("rechts-unten", "Rechts unten"),
                        ("links-unten", "Links unten"),
                        ("rechts-oben", "Rechts oben"),
                        ("links-oben", "Links oben"),
                        ("mitte", "Bildschirmmitte"),
                    ];
                    let shown = positions
                        .iter()
                        .find(|(k, _)| *k == self.form.hud_position)
                        .map(|(_, l)| *l)
                        .unwrap_or("Rechts unten");
                    egui::ComboBox::from_id_salt("pos")
                        .selected_text(shown)
                        .show_ui(ui, |ui| {
                            for (k, l) in positions {
                                ui.selectable_value(&mut self.form.hud_position, k.to_string(), l);
                            }
                        });
                });
            });
            if ui.button("Vorschau (15 Sekunden)").clicked() {
                if let Ok(exe) = autostart::sibling_exe("jarvis-hud") {
                    if let Ok(mut child) = std::process::Command::new(exe).arg("--demo").spawn() {
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_secs(15));
                            let _ = child.kill();
                            let _ = child.wait();
                        });
                    }
                }
            }
        });
    }
}

fn qr_texture(ctx: &egui::Context, link: &str) -> egui::TextureHandle {
    let code = qrcode::QrCode::new(link.as_bytes()).expect("Link passt in einen QR-Code");
    let w = code.width();
    let (scale, quiet) = (6usize, 4usize);
    let size = (w + 2 * quiet) * scale;
    let colors = code.to_colors();
    let mut img = egui::ColorImage::new([size, size], vec![Color32::WHITE; size * size]);
    for y in 0..w {
        for x in 0..w {
            if colors[y * w + x] == qrcode::Color::Dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let px = (x + quiet) * scale + dx;
                        let py = (y + quiet) * scale + dy;
                        img.pixels[py * size + px] = Color32::BLACK;
                    }
                }
            }
        }
    }
    ctx.load_texture("pair-qr", img, egui::TextureOptions::NEAREST)
}

fn list_microphones() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    let mut names: Vec<String> = devices
        .filter_map(|d| d.description().ok().map(|n| n.to_string()))
        .collect();
    names.sort();
    names.dedup();
    names
}

fn ollama_models(url: &str) -> Result<Vec<String>, String> {
    #[derive(serde::Deserialize)]
    struct Tags {
        models: Vec<Tag>,
    }
    #[derive(serde::Deserialize)]
    struct Tag {
        name: String,
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| e.to_string())?;
    let tags: Tags = client
        .get(format!("{url}/api/tags"))
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json())
        .map_err(|_| format!("Ollama ist unter {url} nicht erreichbar"))?;
    Ok(tags.models.into_iter().map(|m| m.name).collect())
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Jarvis – Einstellungen")
            .with_inner_size([860.0, 620.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Jarvis – Einstellungen",
        options,
        Box::new(|cc| {
            let mut visuals = egui::Visuals::dark();
            visuals.selection.bg_fill = Color32::from_rgb(20, 90, 120);
            visuals.hyperlink_color = CYAN;
            visuals.panel_fill = Color32::from_rgb(12, 18, 26);
            visuals.window_fill = Color32::from_rgb(16, 24, 34);
            visuals.extreme_bg_color = Color32::from_rgb(8, 12, 18);
            cc.egui_ctx.set_visuals(visuals);
            Ok(Box::new(App::new(&cc.egui_ctx)))
        }),
    )
}
