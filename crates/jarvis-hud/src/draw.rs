//! Zeichnet den Holo-Ring und das Textfeld (eigener Entwurf, Cyan auf Transparent).

use std::f32::consts::TAU;
use std::time::Instant;

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Shape, Stroke, Vec2};

use crate::state::{HudState, Mode};

pub const WIDTH: f32 = 420.0;
pub const HEIGHT: f32 = 640.0;
const ORB_Y: f32 = 190.0;

fn cyan(a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(79, 209, 255, (a.clamp(0.0, 1.0) * 255.0) as u8)
}

fn amber(a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(255, 170, 70, (a.clamp(0.0, 1.0) * 255.0) as u8)
}

fn dir(angle: f32) -> Vec2 {
    Vec2::new(angle.cos(), angle.sin())
}

fn arc(painter: &egui::Painter, c: Pos2, r: f32, start: f32, sweep: f32, stroke: Stroke) {
    let n = ((sweep.abs() * r) / 6.0).ceil().max(2.0) as usize;
    let pts: Vec<Pos2> = (0..=n)
        .map(|k| c + dir(start + sweep * k as f32 / n as f32) * r)
        .collect();
    painter.add(Shape::line(pts, stroke));
}

pub fn paint(ui: &egui::Ui, s: &HudState, now: Instant) {
    let painter = ui.painter();
    let rect = ui.max_rect();
    let a = s.alpha;
    if a <= 0.0 {
        return;
    }
    let c = Pos2::new(rect.center().x, rect.top() + ORB_Y);
    let lvl = s.level;
    let tint = |x: f32| {
        if s.ok || s.mode != Mode::Answer {
            cyan(x * a)
        } else {
            amber(x * a)
        }
    };

    // Leuchten
    for i in 0..8 {
        painter.circle_filled(c, 64.0 + i as f32 * 13.0 + lvl * 22.0, tint(0.03));
    }

    // Äußerer Skalenring mit 72 Strichen
    let r_ticks = 168.0;
    for i in 0..72 {
        let ang = i as f32 / 72.0 * TAU + s.spin * 0.25;
        let major = i % 6 == 0;
        let len = if major { 13.0 } else { 5.0 };
        let d = dir(ang);
        painter.line_segment(
            [c + d * (r_ticks - len), c + d * r_ticks],
            Stroke::new(
                if major { 2.0_f32 } else { 1.2 },
                tint(if major { 0.9 } else { 0.4 }),
            ),
        );
    }
    painter.circle_stroke(c, 152.0, Stroke::new(1.0_f32, tint(0.25)));

    // Drei breite Segmentbögen, gegenläufig
    for k in 0..3 {
        let start = -s.spin * 1.3 + k as f32 * TAU / 3.0;
        arc(
            painter,
            c,
            140.0,
            start,
            TAU / 3.0 * 0.62,
            Stroke::new(4.0_f32, tint(0.85)),
        );
    }
    // Gestrichelter Ring
    for k in 0..36 {
        let start = s.spin * 0.8 + k as f32 * TAU / 36.0;
        arc(
            painter,
            c,
            121.0,
            start,
            TAU / 36.0 * 0.45,
            Stroke::new(2.0_f32, tint(0.5)),
        );
    }
    // Zwei kleine Marker, die außen umlaufen
    for k in 0..2 {
        let ang = s.t * 1.7 + k as f32 * std::f32::consts::PI;
        painter.circle_filled(c + dir(ang) * 152.0, 3.5, tint(1.0));
    }

    // Pegel-Strahlen: reagieren auf Stimme bzw. Sprechen
    for i in 0..64 {
        let ang = i as f32 / 64.0 * TAU - std::f32::consts::FRAC_PI_2;
        let wobble = 0.55 + 0.45 * (i as f32 * 2.3 + s.t * 9.0).sin();
        let amp = match s.mode {
            Mode::Thinking => 0.15 + 0.12 * (i as f32 * 0.5 - s.t * 7.0).sin().abs(),
            _ => lvl * wobble,
        };
        let r0 = 80.0;
        let len = 4.0 + amp * 30.0;
        let d = dir(ang);
        painter.line_segment(
            [c + d * r0, c + d * (r0 + len)],
            Stroke::new(2.4_f32, tint(0.55 + 0.45 * amp)),
        );
    }

    // Kern
    let core = 46.0 + lvl * 10.0;
    painter.circle_filled(
        c,
        core,
        Color32::from_rgba_unmultiplied(6, 22, 34, (a * 230.0) as u8),
    );
    painter.circle_stroke(c, core, Stroke::new(2.0_f32, tint(1.0)));
    painter.circle_filled(c, 20.0 + lvl * 14.0, tint(0.30));
    painter.circle_filled(
        c,
        7.0 + lvl * 4.0,
        Color32::from_rgba_unmultiplied(220, 250, 255, (a * 255.0) as u8),
    );
    painter.text(
        c + Vec2::new(0.0, core + 70.0 - lvl * 10.0),
        egui::Align2::CENTER_CENTER,
        s.status(),
        FontId::monospace(13.0),
        tint(0.95),
    );

    // Textfeld mit Eckklammern
    let heard = match (&s.source, s.heard.is_empty()) {
        (_, true) => String::new(),
        (Some(src), false) => format!("▸ {src}: {}", s.heard),
        (None, false) => format!("▸ {}", s.heard),
    };
    let reply = s.typed_reply(now);
    if heard.is_empty() && reply.is_empty() {
        return;
    }
    let wrap = WIDTH - 72.0;
    let g1 = painter.layout(heard, FontId::monospace(13.0), tint(0.8), wrap);
    let reply_color = if s.ok {
        Color32::from_rgba_unmultiplied(225, 248, 255, (a * 255.0) as u8)
    } else {
        Color32::from_rgba_unmultiplied(255, 205, 160, (a * 255.0) as u8)
    };
    let g2 = painter.layout(reply, FontId::proportional(18.0), reply_color, wrap);
    let gap = if g1.size().y > 0.0 && g2.size().y > 0.0 {
        10.0
    } else {
        0.0
    };
    let h = 32.0 + g1.size().y + gap + g2.size().y;
    let top = c.y + 205.0;
    let panel = Rect::from_min_size(
        Pos2::new(rect.left() + 20.0, top),
        Vec2::new(WIDTH - 40.0, h),
    );
    painter.rect_filled(
        panel,
        6.0,
        Color32::from_rgba_unmultiplied(4, 14, 24, (a * 200.0) as u8),
    );
    painter.rect_stroke(
        panel,
        6.0,
        Stroke::new(1.0_f32, tint(0.35)),
        egui::StrokeKind::Inside,
    );
    let l = 14.0;
    let bracket = Stroke::new(2.0_f32, tint(0.95));
    for (corner, sx, sy) in [
        (panel.left_top(), 1.0, 1.0),
        (panel.right_top(), -1.0, 1.0),
        (panel.left_bottom(), 1.0, -1.0),
        (panel.right_bottom(), -1.0, -1.0),
    ] {
        painter.line_segment([corner, corner + Vec2::new(l * sx, 0.0)], bracket);
        painter.line_segment([corner, corner + Vec2::new(0.0, l * sy)], bracket);
    }
    let mut y = panel.top() + 16.0;
    let x = panel.left() + 16.0;
    let g1h = g1.size().y;
    painter.galley(Pos2::new(x, y), g1, tint(0.8));
    y += g1h + gap;
    painter.galley(Pos2::new(x, y), g2, reply_color);
}
