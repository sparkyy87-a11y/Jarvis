import SwiftUI

@main
struct JarvisMain: App {
    @StateObject private var model = JarvisModel()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(model)
                // QR-Code von `jarvisd pair`: jarvis://pair?…
                .onOpenURL { url in model.connect(code: url.absoluteString) }
        }
    }
}
