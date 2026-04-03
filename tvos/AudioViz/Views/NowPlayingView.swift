import SwiftUI
import MusicKit

// ── NowPlayingView ────────────────────────────────────────────────────────────
//
// Root view of the app.  The fullscreen visualizer fills the background; a
// translucent control strip is anchored to the bottom edge.
//
// Siri Remote
// ───────────
// • Play/Pause button  → toggle playback
// • D-pad left/right   → skip prev/next (when transport buttons are focused)
// • Menu button        → system back (handled by tvOS)

struct NowPlayingView: View {
    @EnvironmentObject var appState: AppState

    @State private var showLibrary  = false
    @State private var showPicker   = false
    @State private var showSettings = false

    var body: some View {
        ZStack(alignment: .bottom) {

            // ── Visualizer (full screen) ──────────────────────────────────────
            VisualizerView(bridge: appState.bridge)
                .ignoresSafeArea()

            // ── Bottom scrim ──────────────────────────────────────────────────
            LinearGradient(
                colors: [.clear, .black.opacity(0.92)],
                startPoint: .init(x: 0.5, y: 0.0),
                endPoint:   .init(x: 0.5, y: 1.0)
            )
            .frame(height: 360)
            .ignoresSafeArea(edges: .bottom)

            // ── Controls ──────────────────────────────────────────────────────
            VStack(spacing: 20) {
                trackInfoRow
                transportRow
                    .focusSection()
                actionRow
                    .focusSection()
            }
            .padding(.horizontal, 80)
            .padding(.bottom, 60)
        }
        .ignoresSafeArea()
        .onPlayPauseCommand {
            appState.musicPlayer.togglePlayPause()
        }
        .sheet(isPresented: $showLibrary)  {
            LibraryView().environmentObject(appState)
        }
        .sheet(isPresented: $showPicker) {
            VisualizerPickerView().environmentObject(appState)
        }
        .sheet(isPresented: $showSettings) {
            SettingsView().environmentObject(appState)
        }
    }

    // MARK: - Track info row

    private var trackInfoRow: some View {
        HStack(alignment: .center, spacing: 28) {

            // Album artwork
            albumArt
                .frame(width: 110, height: 110)

            // Title / artist / album
            VStack(alignment: .leading, spacing: 5) {
                if let song = appState.musicPlayer.currentSong {
                    Text(song.title)
                        .font(.title2.bold())
                        .lineLimit(1)
                    Text(song.artistName)
                        .font(.headline)
                        .foregroundStyle(.secondary)
                    if let album = song.albumTitle {
                        Text(album)
                            .font(.subheadline)
                            .foregroundStyle(.tertiary)
                    }
                } else {
                    Text("Nothing playing")
                        .font(.title2.bold())
                    Text("Open the Library to choose music")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            }
            .foregroundStyle(.white)

            Spacer()

            // Progress + time
            if let song = appState.musicPlayer.currentSong {
                progressBlock(song: song)
            }
        }
    }

    @ViewBuilder
    private var albumArt: some View {
        if let url = appState.musicPlayer.currentSong?.artwork?.url(width: 220, height: 220) {
            AsyncImage(url: url) { phase in
                if let img = phase.image {
                    img.resizable().aspectRatio(contentMode: .fill)
                } else {
                    artPlaceholder
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 10))
        } else {
            artPlaceholder
        }
    }

    private var artPlaceholder: some View {
        RoundedRectangle(cornerRadius: 10)
            .fill(.white.opacity(0.12))
            .overlay {
                Image(systemName: "music.note")
                    .font(.system(size: 36))
                    .foregroundStyle(.secondary)
            }
    }

    private func progressBlock(song: Song) -> some View {
        // TimelineView gives us continuous updates for the progress bar without
        // needing a separate timer or polling.
        TimelineView(.periodic(from: .now, by: 0.5)) { _ in
            let elapsed  = ApplicationMusicPlayer.shared.playbackTime
            let duration = song.duration ?? 1
            let fraction = max(0, min(1, elapsed / duration))

            VStack(alignment: .trailing, spacing: 6) {
                ProgressView(value: fraction)
                    .frame(width: 220)
                    .tint(.white)
                HStack {
                    Text(formatTime(elapsed))
                    Spacer()
                    Text(formatTime(duration))
                }
                .font(.caption.monospacedDigit())
                .foregroundStyle(.secondary)
                .frame(width: 220)
            }
        }
    }

    // MARK: - Transport row

    private var transportRow: some View {
        HStack(spacing: 64) {
            transportButton(systemImage: "backward.fill", size: 30) {
                appState.musicPlayer.skipToPrevious()
            }
            transportButton(
                systemImage: appState.musicPlayer.isPlaying ? "pause.fill" : "play.fill",
                size: 44
            ) {
                appState.musicPlayer.togglePlayPause()
            }
            transportButton(systemImage: "forward.fill", size: 30) {
                appState.musicPlayer.skipToNext()
            }
        }
    }

    private func transportButton(systemImage: String, size: CGFloat, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.system(size: size, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 60, height: 60)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .hoverEffect()
    }

    // MARK: - Action row

    private var actionRow: some View {
        HStack(spacing: 40) {
            actionButton("Library",    icon: "music.note.list") { showLibrary  = true }
            actionButton("Visualizers",icon: "waveform")        { showPicker   = true }
            actionButton("Settings",   icon: "slider.horizontal.3") { showSettings = true }

            Spacer()

            // Visualizer badge
            Text(appState.currentVizName.isEmpty ? "" : "▸ \(appState.currentVizName)")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
    }

    private func actionButton(_ label: String, icon: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Label(label, systemImage: icon)
                .font(.subheadline)
                .foregroundStyle(.white)
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
                .background(.white.opacity(0.12), in: Capsule())
        }
        .buttonStyle(.plain)
        .hoverEffect()
    }

    // MARK: - Helpers

    private func formatTime(_ secs: TimeInterval) -> String {
        let total = Int(max(0, secs))
        return String(format: "%d:%02d", total / 60, total % 60)
    }
}
