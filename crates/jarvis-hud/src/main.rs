//! `jarvis-hud` – leuchtendes Overlay, das sich einblendet, sobald Jarvis
//! zuhört, nachdenkt oder antwortet. Durchklickbar, randlos, immer im Vordergrund.
//!
//!   jarvis-hud            mit jarvisd verbinden
//!   jarvis-hud --demo     Vorführung ohne jarvisd

#![cfg_attr(windows, windows_subsystem = "windows")]

mod draw;
mod state;

use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;
use jarvis_node::config::{Config, DEFAULT_CONTROL_PORT};
use jarvis_node::events::HudEvent;

struct HudApp {
    state: state::HudState,
    events: mpsc::Receiver<HudEvent>,
    position: String,
    placed: bool,
}

impl eframe::App for HudApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.placed {
            if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
                let m = 24.0;
                let (w, h) = (draw::WIDTH, draw::HEIGHT);
                let pos = match self.position.as_str() {
                    "links-unten" => egui::pos2(m, monitor.y - h - m - 40.0),
                    "rechts-oben" => egui::pos2(monitor.x - w - m, m),
                    "links-oben" => egui::pos2(m, m),
                    "mitte" => egui::pos2((monitor.x - w) / 2.0, (monitor.y - h) / 2.0),
                    _ => egui::pos2(monitor.x - w - m, monitor.y - h - m - 40.0),
                };
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
                self.placed = true;
            }
        }
        let now = Instant::now();
        while let Ok(e) = self.events.try_recv() {
            self.state.apply(e, now);
        }
        let visible = self.state.tick(now);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                draw::paint(ui, &self.state, now);
            });
        if visible {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

/// Hält die Verbindung zu jarvisd und reicht Ereignisse weiter.
fn listen(
    port: u16,
    token_path: std::path::PathBuf,
    tx: mpsc::Sender<HudEvent>,
    ctx: egui::Context,
) {
    loop {
        if let Ok((mut s, reader)) = jarvis_node::control::connect_blocking(port, &token_path) {
            if s.write_all(b"{\"cmd\":\"subscribe\"}\n").is_ok() {
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if let Ok(e) = serde_json::from_str::<HudEvent>(&line) {
                        if tx.send(e).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Vorführung: ein typisches Gespräch in Schleife.
fn demo(tx: mpsc::Sender<HudEvent>, ctx: egui::Context) {
    let send = |e: HudEvent| {
        let _ = tx.send(e);
        ctx.request_repaint();
    };
    let wait = |ms: u64| std::thread::sleep(Duration::from_millis(ms));
    std::thread::sleep(Duration::from_millis(600));
    loop {
        send(HudEvent::Wake);
        for i in 0..45 {
            let v = 0.25 + 0.6 * ((i as f32 * 0.45).sin().abs());
            send(HudEvent::Level { value: v });
            wait(50);
        }
        send(HudEvent::Heard {
            text: "öffne Minecraft am PC".into(),
            source: Some("Sprache".into()),
        });
        send(HudEvent::Thinking);
        wait(1300);
        send(HudEvent::Reply {
            text: "Gaming-PC: Minecraft wird gestartet.".into(),
            ok: true,
        });
        wait(600);
        send(HudEvent::Speaking { active: true });
        wait(2200);
        send(HudEvent::Speaking { active: false });
        wait(5000);
        send(HudEvent::File {
            name: "Urlaub.jpg".into(),
            from: "iPhone".into(),
        });
        wait(6000);
    }
}

fn main() -> eframe::Result {
    let demo_mode = std::env::args().any(|a| a == "--demo");
    let cfg = Config::load().ok();
    let port = cfg
        .as_ref()
        .map(|c| c.mesh.control_port)
        .unwrap_or(DEFAULT_CONTROL_PORT);
    let position = cfg
        .as_ref()
        .map(|c| c.hud.position.clone())
        .unwrap_or_default();
    let token_path = cfg
        .as_ref()
        .map(|c| c.control_token_path())
        .unwrap_or_else(Config::default_control_token_path);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Jarvis HUD")
            .with_inner_size([draw::WIDTH, draw::HEIGHT])
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            .with_taskbar(false)
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(
        "Jarvis HUD",
        options,
        Box::new(move |cc| {
            let (tx, rx) = mpsc::channel();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                if demo_mode {
                    demo(tx, ctx)
                } else {
                    listen(port, token_path, tx, ctx)
                }
            });
            Ok(Box::new(HudApp {
                state: state::HudState::new(Instant::now()),
                events: rx,
                position,
                placed: false,
            }))
        }),
    )
}
