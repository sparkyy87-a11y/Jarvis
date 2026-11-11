import UIKit
import UserNotifications

/// Was Jarvis auf dem iPhone/iPad selbst tun kann – aufgerufen vom Rust-Kern
/// (aus einem Hintergrund-Thread).
final class IOSBridge: PlatformBridge, @unchecked Sendable {
    private weak var model: JarvisModel?

    init(model: JarvisModel) { self.model = model }

    /// iOS lässt Apps nur über ihre Links öffnen. Bekannte Apps:
    private static let schemes: [String: String] = [
        "spotify": "spotify:", "youtube": "youtube://", "youtubemusic": "youtubemusic://",
        "whatsapp": "whatsapp://", "telegram": "tg://", "signal": "sgnl://", "instagram": "instagram://",
        "netflix": "nflx://", "primevideo": "aiv://", "disney": "disneyplus://", "twitch": "twitch://",
        "discord": "discord://", "steam": "steam://", "minecraft": "minecraft://", "zoom": "zoomus://",
        "teams": "msteams://", "outlook": "ms-outlook://", "googlemaps": "comgooglemaps://",
        "karten": "maps://", "maps": "maps://", "musik": "music://", "applemusic": "music://",
        "podcasts": "podcasts://", "kalender": "calshow://", "notizen": "mobilenotes://",
        "nachrichten": "sms:", "mail": "message://", "fotos": "photos-redirect://",
        "appstore": "itms-apps://", "shortcuts": "shortcuts://", "kurzbefehle": "shortcuts://",
        "einstellungen": UIApplication.openSettingsURLString,
    ]

    private static func squash(_ s: String) -> String {
        s.lowercased().filter { $0.isLetter || $0.isNumber }
    }

    func openApp(app: String) -> Reply {
        let wanted = Self.squash(app)
        let scheme = Self.schemes[wanted]
            ?? Self.schemes.first(where: { wanted.contains($0.key) || $0.key.contains(wanted) && wanted.count >= 3 })?.value
        guard let scheme, let url = URL(string: scheme) else {
            return Reply(ok: false, text: "„\(app)“ kann ich auf dem \(UIDevice.current.model) nicht öffnen – iOS erlaubt das nur über bekannte App-Links.")
        }
        let done = DispatchSemaphore(value: 0)
        var opened = false
        DispatchQueue.main.async {
            UIApplication.shared.open(url) { ok in
                opened = ok
                done.signal()
            }
        }
        guard done.wait(timeout: .now() + 5) == .success, opened else {
            return Reply(ok: false, text: "\(app) ließ sich nicht öffnen – ist die App installiert und Jarvis gerade offen?")
        }
        return Reply(ok: true, text: "\(app) wird geöffnet.")
    }

    func speak(text: String) {
        Task { @MainActor in self.model?.say(text) }
    }

    func fileReceived(path: String) {
        let url = URL(fileURLWithPath: path)
        Task { @MainActor in
            self.model?.add(Entry(fromUser: false, text: "Datei empfangen: \(url.lastPathComponent)", file: url))
        }
        let content = UNMutableNotificationContent()
        content.title = "Datei empfangen"
        content.body = url.lastPathComponent
        UNUserNotificationCenter.current().add(
            UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        )
    }
}
