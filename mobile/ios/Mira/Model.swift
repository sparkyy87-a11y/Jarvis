import AVFoundation
import Foundation
import SwiftUI
import UIKit
import UserNotifications

/// Eine Zeile im Gesprächsverlauf.
struct Entry: Identifiable {
    let id = UUID()
    let fromUser: Bool
    let text: String
    var ok = true
    var file: URL? = nil
}

/// Fehlermeldung aus dem Rust-Kern lesbar machen.
func describe(_ error: Error) -> String {
    if let e = error as? MiraError, case let .Failed(message) = e { return message }
    return error.localizedDescription
}

@MainActor
final class MiraModel: ObservableObject {
    @Published var paired = false
    @Published var entries: [Entry] = []
    @Published var devices: [Device] = []
    /// Name des Hauptgeräts (verwaltet Kopplung, Sperren und Rechte).
    @Published var adminName: String?
    @Published var message: String?
    /// Code aus einem mira://-Link (wird nur vorausgefüllt).
    @Published var pendingCode = ""
    /// Hinweis, solange die Kopplung nicht abgeschlossen ist.
    @Published var status: String?
    /// Zustand des Rings (Zuhören setzt die Ansicht selbst).
    @Published var orb: OrbMode = .idle

    private(set) var mira: Mira?
    private let synthesizer = AVSpeechSynthesizer()
    private let speechDelegate = SpeechDelegate()
    private var pollTask: Task<Void, Never>?

    /// Kopplung (mit Schlüssel) – privat, nicht in der Dateien-App sichtbar.
    nonisolated static var dataDir: String {
        let url = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Mira")
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url.path
    }

    /// Empfangene Dateien – sichtbar in Dateien › Auf meinem iPhone › Mira.
    nonisolated static var filesDir: String {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].path
    }

    var platform: String { UIDevice.current.userInterfaceIdiom == .pad ? "ipados" : "ios" }
    var deviceName: String { UIDevice.current.userInterfaceIdiom == .pad ? "iPad" : "iPhone" }

    init() {
        synthesizer.delegate = speechDelegate
        speechDelegate.onChange = { [weak self] speaking in
            Task { @MainActor in
                guard let self else { return }
                if speaking { self.orb = .speaking } else if self.orb == .speaking { self.orb = .idle }
            }
        }
        paired = isPaired(dataDir: Self.dataDir)
        if paired { start() }
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    func start() {
        guard mira == nil else { return }
        let bridge = IOSBridge(model: self)
        Task.detached {
            do {
                let j = try Mira.start(dataDir: Self.dataDir, filesDir: Self.filesDir, bridge: bridge)
                await MainActor.run {
                    self.mira = j
                    self.paired = true
                    self.startPolling()
                }
            } catch {
                await MainActor.run { self.message = describe(error) }
            }
        }
    }

    /// Kopplung mit dem Code aus `mirad pair` (QR-Code oder eingefügt).
    func connect(code: String, name: String) {
        let chosen = name.trimmingCharacters(in: .whitespaces)
        let (dir, platform) = (Self.dataDir, platform)
        let deviceName = chosen.isEmpty ? self.deviceName : chosen
        Task.detached {
            do {
                let info = try pair(dataDir: dir, deviceName: deviceName, platform: platform, code: code.trimmingCharacters(in: .whitespacesAndNewlines))
                await MainActor.run {
                    self.message = "Kopplung mit \(info.name) läuft …"
                    self.start()
                }
            } catch {
                await MainActor.run { self.message = describe(error) }
            }
        }
    }

    func forget() {
        pollTask?.cancel()
        mira = nil
        unpair(dataDir: Self.dataDir)
        paired = false
        entries = []
        devices = []
    }

    func add(_ entry: Entry) {
        entries.append(entry)
        if entries.count > 200 { entries.removeFirst(entries.count - 200) }
    }

    /// Text an Mira; Antwort in den Verlauf und auf Wunsch vorlesen.
    func ask(_ text: String, speak: Bool) {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        add(Entry(fromUser: true, text: t))
        guard let mira else {
            add(Entry(fromUser: false, text: "Noch nicht verbunden.", ok: false))
            return
        }
        orb = .thinking
        Task.detached {
            let r = mira.ask(text: t)
            await MainActor.run {
                self.add(Entry(fromUser: false, text: r.text, ok: r.ok))
                if !r.ok {
                    self.orb = .error
                    Task {
                        try? await Task.sleep(for: .seconds(2.5))
                        if self.orb == .error { self.orb = .idle }
                    }
                } else if !speak {
                    self.orb = .idle
                }
                if speak { self.say(r.text) }
            }
        }
    }

    /// Datei aus der Auswahl an ein Gerät schicken.
    func send(file: URL, to device: String) {
        guard let mira else { return }
        add(Entry(fromUser: true, text: "„\(file.lastPathComponent)“ an \(device) senden"))
        Task.detached {
            // Ausgewählte Dateien liegen oft außerhalb der App: erst kopieren.
            let scoped = file.startAccessingSecurityScopedResource()
            defer { if scoped { file.stopAccessingSecurityScopedResource() } }
            let copy = FileManager.default.temporaryDirectory.appendingPathComponent(file.lastPathComponent)
            try? FileManager.default.removeItem(at: copy)
            let reply: Reply
            do {
                try FileManager.default.copyItem(at: file, to: copy)
                reply = mira.sendFile(path: copy.path, device: device, folder: nil)
            } catch {
                reply = Reply(ok: false, text: "Datei nicht lesbar: \(error.localizedDescription)")
            }
            try? FileManager.default.removeItem(at: copy)
            await MainActor.run { self.add(Entry(fromUser: false, text: reply.text, ok: reply.ok)) }
        }
    }

    func say(_ text: String) {
        guard !text.isEmpty else { return }
        let utterance = AVSpeechUtterance(string: text)
        utterance.voice = AVSpeechSynthesisVoice(language: "de-DE")
        synthesizer.speak(utterance)
    }

    private func startPolling() {
        pollTask?.cancel()
        pollTask = Task {
            while !Task.isCancelled {
                if let j = mira {
                    let (list, state, members) = await Task.detached { (j.devices(), j.status(), j.members()) }.value
                    devices = list.filter { !$0.thisDevice }
                    adminName = members.first(where: { $0.admin })?.name
                    status = state
                }
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }
}

/// Meldet, wann die Sprachausgabe beginnt und endet (für den Ring).
final class SpeechDelegate: NSObject, AVSpeechSynthesizerDelegate {
    var onChange: ((Bool) -> Void)?
    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didStart utterance: AVSpeechUtterance) { onChange?(true) }
    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) { onChange?(false) }
    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) { onChange?(false) }
}
