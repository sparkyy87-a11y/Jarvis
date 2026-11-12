import QuickLook
import SwiftUI
import UniformTypeIdentifiers

struct ContentView: View {
    @EnvironmentObject private var model: MiraModel

    var body: some View {
        if model.paired { ChatView() } else { PairingView() }
    }
}

// MARK: - Kopplung

struct PairingView: View {
    @EnvironmentObject private var model: MiraModel
    @State private var code = ""
    @State private var name = ""

    /// Wohin gekoppelt wird – nur bei gültigem Code.
    private var target: String? { try? parsePairingCode(code: code.trimmingCharacters(in: .whitespacesAndNewlines)).name }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Mira").font(.largeTitle.bold())
            Text("""
            Mit deinem PC koppeln:

            1. Am PC im Terminal: mirad pair
            2. Den QR-Code mit der Kamera-App scannen – Mira öffnet sich mit dem Code.

            Oder den Code hier einfügen:
            """)
            TextField("mira://pair?…", text: $code)
                .textFieldStyle(.roundedBorder)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            TextField("Name dieses Geräts, z. B. iPad Wohnzimmer", text: $name)
                .textFieldStyle(.roundedBorder)
            Button(target.map { "Mit „\($0)“ koppeln" } ?? "Koppeln") { model.connect(code: code, name: name) }
                .buttonStyle(.borderedProminent)
                .disabled(target == nil)
            if let message = model.message {
                Text(message).foregroundStyle(.red)
            }
        }
        .padding(24)
        .frame(maxWidth: 560)
        .onAppear {
            if name.isEmpty { name = model.deviceName }
            if code.isEmpty { code = model.pendingCode }
        }
        .onChange(of: model.pendingCode) { code = model.pendingCode }
    }
}

// MARK: - Gespräch

struct ChatView: View {
    @EnvironmentObject private var model: MiraModel
    @StateObject private var speech = SpeechInput()
    @State private var input = ""
    @State private var picking = false
    @State private var fileToSend: URL?
    @State private var preview: URL?

    var body: some View {
        VStack(spacing: 0) {
            header
            deviceChips
            MiraOrb(
                mode: speech.listening ? .listening : model.orb,
                level: speech.level,
                size: model.entries.isEmpty ? 260 : 170
            )
            .onTapGesture { speech.toggle { text in model.ask(text, speak: true) } }
            .padding(.vertical, 6)
            .animation(.easeInOut(duration: 0.3), value: model.entries.isEmpty)
            messages
            if speech.listening || !speech.partial.isEmpty {
                Text(speech.partial.isEmpty ? "Ich höre zu …" : speech.partial)
                    .font(.callout).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal)
            }
            inputBar
        }
        .fileImporter(isPresented: $picking, allowedContentTypes: [.item]) { result in
            if case let .success(url) = result { fileToSend = url }
        }
        .confirmationDialog(
            "An welches Gerät?",
            isPresented: Binding(get: { fileToSend != nil }, set: { if !$0 { fileToSend = nil } }),
            titleVisibility: .visible
        ) {
            ForEach(model.devices, id: \.name) { device in
                Button(device.name) {
                    if let file = fileToSend { model.send(file: file, to: device.name) }
                    fileToSend = nil
                }
            }
        } message: {
            if model.devices.isEmpty { Text("Gerade ist kein anderes Gerät verbunden.") }
        }
        .quickLookPreview($preview)
    }

    private var header: some View {
        HStack {
            VStack(alignment: .leading) {
                Text("Mira").font(.title2.bold())
                Text(model.deviceName).font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            Menu {
                Button("Kopplung aufheben", role: .destructive) { model.forget() }
            } label: {
                Image(systemName: "ellipsis.circle").font(.title2)
            }
        }
        .padding()
    }

    private var deviceChips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack {
                if model.devices.isEmpty {
                    Label(model.status ?? "Suche deine Geräte …", systemImage: "antenna.radiowaves.left.and.right")
                        .font(.footnote).padding(8)
                        .background(.thinMaterial, in: Capsule())
                }
                ForEach(model.devices, id: \.name) { device in
                    Label(device.name == model.adminName ? "\(device.name) · Hauptgerät" : device.name,
                          systemImage: icon(for: device.platform))
                        .font(.footnote).padding(8)
                        .background(.thinMaterial, in: Capsule())
                }
            }
            .padding(.horizontal)
        }
    }

    private var messages: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 8) {
                    if model.entries.isEmpty {
                        Text("Tippe auf den Ring und sag zum Beispiel:\n„Öffne Minecraft am PC“\n„Google Wetter Berlin“\n„Spiel Musik von Adele“")
                            .foregroundStyle(.secondary)
                            .padding()
                    }
                    ForEach(model.entries) { entry in
                        bubble(entry).id(entry.id)
                    }
                }
                .padding(.horizontal)
            }
            .onChange(of: model.entries.count) {
                if let last = model.entries.last { withAnimation { proxy.scrollTo(last.id, anchor: .bottom) } }
            }
        }
    }

    private func bubble(_ entry: Entry) -> some View {
        HStack {
            if entry.fromUser { Spacer(minLength: 40) }
            Text(entry.text)
                .padding(.horizontal, 14).padding(.vertical, 10)
                .background(
                    entry.fromUser ? Color.accentColor.opacity(0.2) : (entry.ok ? Color.secondary.opacity(0.15) : Color.red.opacity(0.2)),
                    in: RoundedRectangle(cornerRadius: 16)
                )
                .onTapGesture { if let file = entry.file { preview = file } }
            if !entry.fromUser { Spacer(minLength: 40) }
        }
    }

    private var inputBar: some View {
        HStack(spacing: 10) {
            Button { picking = true } label: { Image(systemName: "paperclip").font(.title3) }
            TextField("Befehl tippen …", text: $input)
                .textFieldStyle(.roundedBorder)
                .onSubmit(sendTyped)
            Button(action: sendTyped) { Image(systemName: "paperplane.fill") }
                .disabled(input.isEmpty)
        }
        .padding()
    }

    private func sendTyped() {
        model.ask(input, speak: false)
        input = ""
    }

    private func icon(for platform: String) -> String {
        switch platform.lowercased() {
        case "windows", "linux": return "desktopcomputer"
        case "macos": return "laptopcomputer"
        case "ipados": return "ipad"
        default: return "iphone"
        }
    }
}
