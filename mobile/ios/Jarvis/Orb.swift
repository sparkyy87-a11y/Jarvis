import SwiftUI

enum OrbMode { case idle, listening, thinking, speaking, error }

/// Hält Drehung und geglätteten Pegel zwischen zwei Bildern (kein SwiftUI-Zustand,
/// damit das Zeichnen keine neue Auswertung der Ansicht auslöst).
final class OrbAnimator {
    var spin = 0.0
    var level = 0.0
    var last: Double?
}

/// Der Jarvis-Ring – zugleich die Mikrofon-Taste. Gleicher Entwurf wie das PC-HUD.
struct JarvisOrb: View {
    let mode: OrbMode
    let level: Float
    var size: CGFloat = 260
    @State private var anim = OrbAnimator()

    private var tint: Color { mode == .error ? Color(red: 1, green: 0.67, blue: 0.27) : Color(red: 0.31, green: 0.82, blue: 1) }

    private var label: String {
        switch mode {
        case .idle: return "ANTIPPEN ZUM SPRECHEN"
        case .listening: return "HÖRE ZU"
        case .thinking: return "VERARBEITE"
        case .speaking: return "SPRECHE"
        case .error: return "PROBLEM"
        }
    }

    var body: some View {
        VStack(spacing: 4) {
            TimelineView(.animation) { timeline in
                Canvas { g, canvasSize in
                    draw(g, canvasSize, t: timeline.date.timeIntervalSinceReferenceDate)
                }
            }
            .frame(width: size, height: size)
            .contentShape(Circle())
            Text(label)
                .font(.system(size: 12, design: .monospaced))
                .foregroundStyle(tint)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(mode == .listening ? "Zuhören beenden" : "Sprechen")
        .accessibilityAddTraits(.isButton)
    }

    private func draw(_ g: GraphicsContext, _ canvas: CGSize, t: Double) {
        let dt = min(0.1, anim.last.map { t - $0 } ?? 0)
        anim.last = t
        anim.spin += dt * (mode == .thinking ? 2.4 : 0.35)
        let target: Double
        switch mode {
        case .listening: target = Double(level)
        case .speaking: target = 0.35 + 0.3 * abs(sin(t * 9)) * abs(sin(t * 2.3))
        default: target = 0
        }
        anim.level += (target - anim.level) * min(1, dt * 12)
        let lvl = anim.level, spin = anim.spin

        let dim = mode == .idle ? 0.6 : 1.0
        func c(_ a: Double) -> GraphicsContext.Shading { .color(tint.opacity(min(1, a * dim))) }
        let s = min(canvas.width, canvas.height) / 2 / 172
        let center = CGPoint(x: canvas.width / 2, y: canvas.height / 2)
        func at(_ angle: Double, _ r: Double) -> CGPoint {
            CGPoint(x: center.x + cos(angle) * r * s, y: center.y + sin(angle) * r * s)
        }
        func circle(_ r: Double) -> Path {
            Path(ellipseIn: CGRect(x: center.x - r * s, y: center.y - r * s, width: 2 * r * s, height: 2 * r * s))
        }
        func line(_ a: CGPoint, _ b: CGPoint, _ width: Double, _ alpha: Double) {
            var p = Path()
            p.move(to: a)
            p.addLine(to: b)
            g.stroke(p, with: c(alpha), style: StrokeStyle(lineWidth: width * s, lineCap: .round))
        }
        func arc(_ r: Double, _ start: Double, _ sweep: Double, _ width: Double, _ alpha: Double) {
            var p = Path()
            p.addArc(center: center, radius: r * s, startAngle: .radians(start), endAngle: .radians(start + sweep), clockwise: false)
            g.stroke(p, with: c(alpha), style: StrokeStyle(lineWidth: width * s, lineCap: .round))
        }
        let tau = 2 * Double.pi

        for i in 0..<8 { g.fill(circle(64 + Double(i) * 13 + lvl * 22), with: c(0.03)) }
        for i in 0..<72 {
            let ang = Double(i) / 72 * tau + spin * 0.25
            let major = i % 6 == 0
            line(at(ang, 168 - (major ? 13 : 5)), at(ang, 168), major ? 2 : 1.2, major ? 0.9 : 0.4)
        }
        g.stroke(circle(152), with: c(0.25), lineWidth: 1 * s)
        for k in 0..<3 { arc(140, -spin * 1.3 + Double(k) * tau / 3, tau / 3 * 0.62, 4, 0.85) }
        for k in 0..<36 { arc(121, spin * 0.8 + Double(k) * tau / 36, tau / 36 * 0.45, 2, 0.5) }
        for k in 0..<2 {
            let p = at(t * 1.7 + Double(k) * .pi, 152)
            g.fill(Path(ellipseIn: CGRect(x: p.x - 3.5 * s, y: p.y - 3.5 * s, width: 7 * s, height: 7 * s)), with: c(1))
        }
        for i in 0..<64 {
            let ang = Double(i) / 64 * tau - tau / 4
            let wobble = 0.55 + 0.45 * sin(Double(i) * 2.3 + t * 9)
            let amp = mode == .thinking ? 0.15 + 0.12 * abs(sin(Double(i) * 0.5 - t * 7)) : lvl * wobble
            line(at(ang, 80), at(ang, 84 + amp * 30), 2.4, 0.55 + 0.45 * amp)
        }
        let core = 46 + lvl * 10
        g.fill(circle(core), with: .color(Color(red: 0.02, green: 0.09, blue: 0.13).opacity(0.9)))
        g.stroke(circle(core), with: c(1), lineWidth: 2 * s)
        g.fill(circle(20 + lvl * 14), with: c(0.3))
        g.fill(circle(7 + lvl * 4), with: .color(Color(red: 0.86, green: 0.98, blue: 1).opacity(dim)))
    }
}
