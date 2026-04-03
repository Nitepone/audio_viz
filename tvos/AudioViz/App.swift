import SwiftUI

@main
struct AudioVizApp: App {
    @StateObject private var appState = AppState()

    var body: some Scene {
        WindowGroup {
            NowPlayingView()
                .environmentObject(appState)
                .onAppear {
                    // Start the audio engine immediately so the visualizer is
                    // live as soon as the app opens.  MusicKit auth is requested
                    // lazily when the user opens the Library.
                    appState.start()
                }
        }
    }
}
