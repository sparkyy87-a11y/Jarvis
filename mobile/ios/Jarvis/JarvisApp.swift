import SwiftUI

@main
struct JarvisMain: App {
    @StateObject private var model = JarvisModel()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(model)
                // QR-Code von `jarvisd pair`: jarvis://pair?… – füllt nur das Formular vor.
                // Gekoppelt wird erst nach Bestätigung (Schutz vor fremden Links).
                .onOpenURL { url in if !model.paired { model.pendingCode = url.absoluteString } }
        }
    }
}
