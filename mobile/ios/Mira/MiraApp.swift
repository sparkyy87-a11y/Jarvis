import SwiftUI

@main
struct MiraMain: App {
    @StateObject private var model = MiraModel()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(model)
                // QR-Code von `mirad pair`: mira://pair?… – füllt nur das Formular vor.
                // Gekoppelt wird erst nach Bestätigung (Schutz vor fremden Links).
                .onOpenURL { url in if !model.paired { model.pendingCode = url.absoluteString } }
        }
    }
}
