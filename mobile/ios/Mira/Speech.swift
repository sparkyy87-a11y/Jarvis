import AVFoundation
import Speech

/// Mikrofon-Taste: Apple-Spracherkennung (auf dem Gerät, wenn verfügbar).
/// Hört auf, sobald 1,5 s nichts Neues kommt.
@MainActor
final class SpeechInput: ObservableObject {
    @Published var listening = false
    @Published var partial = ""
    /// Mikrofonpegel 0…1 für den Ring.
    @Published var level: Float = 0

    private let recognizer = SFSpeechRecognizer(locale: Locale(identifier: "de-DE"))
    private let engine = AVAudioEngine()
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var task: SFSpeechRecognitionTask?
    private var silenceTimer: Timer?
    private var onResult: ((String) -> Void)?

    func toggle(onResult: @escaping (String) -> Void) {
        if listening {
            stop(send: true)
        } else {
            self.onResult = onResult
            requestPermissions()
        }
    }

    private func requestPermissions() {
        SFSpeechRecognizer.requestAuthorization { status in
            Task { @MainActor in
                guard status == .authorized else {
                    self.partial = "Spracherkennung ist nicht erlaubt (Einstellungen › Mira)."
                    return
                }
                AVAudioApplication.requestRecordPermission { granted in
                    Task { @MainActor in
                        if granted { self.begin() } else { self.partial = "Mikrofon ist nicht erlaubt (Einstellungen › Mira)." }
                    }
                }
            }
        }
    }

    private func begin() {
        guard let recognizer, recognizer.isAvailable else {
            partial = "Spracherkennung gerade nicht verfügbar."
            return
        }
        do {
            let session = AVAudioSession.sharedInstance()
            try session.setCategory(.playAndRecord, mode: .measurement, options: [.duckOthers, .defaultToSpeaker])
            try session.setActive(true, options: .notifyOthersOnDeactivation)

            let request = SFSpeechAudioBufferRecognitionRequest()
            request.shouldReportPartialResults = true
            if recognizer.supportsOnDeviceRecognition { request.requiresOnDeviceRecognition = true }
            self.request = request

            let input = engine.inputNode
            input.removeTap(onBus: 0)
            input.installTap(onBus: 0, bufferSize: 1024, format: input.outputFormat(forBus: 0)) { [weak self] buffer, _ in
                request.append(buffer)
                guard let samples = buffer.floatChannelData?[0], buffer.frameLength > 0 else { return }
                var sum: Float = 0
                for i in 0..<Int(buffer.frameLength) { sum += samples[i] * samples[i] }
                let rms = (sum / Float(buffer.frameLength)).squareRoot()
                let value = min(1, rms * 8)
                Task { @MainActor in self?.level = value }
            }
            engine.prepare()
            try engine.start()
            partial = ""
            listening = true
            task = recognizer.recognitionTask(with: request) { result, error in
                Task { @MainActor in
                    if let result {
                        self.partial = result.bestTranscription.formattedString
                        self.armTimer(1.5)
                        if result.isFinal { self.stop(send: true) }
                    } else if error != nil {
                        self.stop(send: true)
                    }
                }
            }
            armTimer(6) // Zeit, um anzufangen
        } catch {
            partial = "Mikrofon: \(error.localizedDescription)"
            stop(send: false)
        }
    }

    private func armTimer(_ seconds: TimeInterval) {
        silenceTimer?.invalidate()
        silenceTimer = Timer.scheduledTimer(withTimeInterval: seconds, repeats: false) { _ in
            Task { @MainActor in self.stop(send: true) }
        }
    }

    func stop(send: Bool) {
        guard listening else { return }
        listening = false
        level = 0
        silenceTimer?.invalidate()
        engine.stop()
        engine.inputNode.removeTap(onBus: 0)
        request?.endAudio()
        task?.cancel()
        task = nil
        request = nil
        try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .spokenAudio)
        let text = partial.trimmingCharacters(in: .whitespacesAndNewlines)
        if send, !text.isEmpty { onResult?(text) }
    }
}
