import MediaPlayer
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

    /// Öffnet eine Adresse; `true`, wenn iOS sie annehmen konnte.
    private func open(_ url: URL) -> Bool {
        let done = DispatchSemaphore(value: 0)
        var opened = false
        DispatchQueue.main.async {
            UIApplication.shared.open(url) { ok in
                opened = ok
                done.signal()
            }
        }
        return done.wait(timeout: .now() + 5) == .success && opened
    }

    func openUrl(url: String) -> Reply {
        guard let u = URL(string: url), open(u) else {
            return Reply(ok: false, text: "Die Seite ließ sich nicht öffnen – ist Jarvis gerade offen?")
        }
        return Reply(ok: true, text: "Ich öffne die Suche.")
    }

    func playMusic(query: String, service: String) -> Reply {
        let term = query.replacingOccurrences(of: " von ", with: " ")
        let encoded = term.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) ?? term
        switch service {
        case "spotify":
            return openOrFail("spotify:search:\(encoded)", ok: "Spotify zeigt „\(query)“.")
        case "youtube-music":
            return openOrFail("https://music.youtube.com/search?q=\(encoded)", ok: "YouTube Music zeigt „\(query)“.")
        default:
            if let title = playFromLibrary(query) { return Reply(ok: true, text: "Ich spiele \(title).") }
            return openOrFail("music://music.apple.com/search?term=\(encoded)", ok: "Apple Music zeigt „\(query)“ – zum Abspielen antippen.")
        }
    }

    private func openOrFail(_ url: String, ok: String) -> Reply {
        guard let u = URL(string: url), open(u) else {
            return Reply(ok: false, text: "Die Musik-App ließ sich nicht öffnen.")
        }
        return Reply(ok: true, text: ok)
    }

    /// Sucht in der eigenen Mediathek (Titel, Künstler, Album) und spielt direkt ab.
    private func playFromLibrary(_ query: String) -> String? {
        guard MPMediaLibrary.authorizationStatus() == .authorized else {
            MPMediaLibrary.requestAuthorization { _ in }
            return nil
        }
        let parts = query.components(separatedBy: " von ")
        let title = parts[0].trimmingCharacters(in: .whitespaces)
        let artist = parts.count > 1 ? parts[1].trimmingCharacters(in: .whitespaces) : nil
        var items: [MPMediaItem] = []
        if let artist {
            let q = MPMediaQuery.songs()
            q.addFilterPredicate(MPMediaPropertyPredicate(value: title, forProperty: MPMediaItemPropertyTitle, comparisonType: .contains))
            q.addFilterPredicate(MPMediaPropertyPredicate(value: artist, forProperty: MPMediaItemPropertyArtist, comparisonType: .contains))
            items = q.items ?? []
        } else {
            for property in [MPMediaItemPropertyTitle, MPMediaItemPropertyArtist, MPMediaItemPropertyAlbumTitle] where items.isEmpty {
                let q = MPMediaQuery.songs()
                q.addFilterPredicate(MPMediaPropertyPredicate(value: title, forProperty: property, comparisonType: .contains))
                items = q.items ?? []
            }
        }
        guard let first = items.first else { return nil }
        DispatchQueue.main.async {
            let player = MPMusicPlayerController.systemMusicPlayer
            player.setQueue(with: MPMediaItemCollection(items: items))
            player.play()
        }
        return [first.title, first.artist].compactMap { $0 }.joined(separator: " von ")
    }

    func media(command: String) -> Reply {
        let player = MPMusicPlayerController.systemMusicPlayer
        let text: String
        switch command {
        case "play": text = "Weiter geht's."
        case "pause": text = "Pausiert."
        case "next": text = "Nächster Titel."
        default: text = "Vorheriger Titel."
        }
        DispatchQueue.main.async {
            switch command {
            case "play": player.play()
            case "pause": player.pause()
            case "next": player.skipToNextItem()
            default: player.skipToPreviousItem()
            }
        }
        return Reply(ok: true, text: text)
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
