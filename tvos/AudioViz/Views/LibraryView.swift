import SwiftUI
import MusicKit

// ── LibraryView ───────────────────────────────────────────────────────────────
//
// Full-screen sheet for browsing and playing music.
//
// Tabs: Songs · Albums · Playlists · Search
//
// Selecting an item starts playback via MusicPlayer and dismisses the sheet.
// The visualizer continues running behind the sheet (idle animation).

struct LibraryView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.dismiss) private var dismiss

    @State private var selectedTab  = 0
    @State private var songs:      [Song]     = []
    @State private var albums:     [Album]    = []
    @State private var playlists:  [Playlist] = []
    @State private var isLoading   = false
    @State private var loadError:  String?

    // Search
    @State private var searchQuery     = ""
    @State private var searchSongs:    [Song]  = []
    @State private var searchAlbums:   [Album] = []
    @State private var isSearching     = false

    var body: some View {
        NavigationStack {
            Group {
                if appState.musicPlayer.authStatus != .authorized {
                    authPrompt
                } else if isLoading {
                    ProgressView("Loading library…")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if let err = loadError {
                    ContentUnavailableView("Could not load library",
                                           systemImage: "exclamationmark.triangle",
                                           description: Text(err))
                } else {
                    tabContent
                }
            }
            .navigationTitle("Library")
        }
        .task {
            if appState.musicPlayer.authStatus == .authorized {
                await loadLibrary()
            }
        }
    }

    // MARK: - Auth prompt

    private var authPrompt: some View {
        VStack(spacing: 32) {
            Image(systemName: "music.note")
                .font(.system(size: 80))
                .foregroundStyle(.secondary)
            Text("Connect Apple Music")
                .font(.title.bold())
            Text("Allow access to browse your library and play music.")
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Button("Allow Access") {
                Task { await appState.requestMusicAuthAndStart() }
            }
            .buttonStyle(.borderedProminent)
        }
        .padding(80)
    }

    // MARK: - Tab content

    private var tabContent: some View {
        VStack(spacing: 0) {
            Picker("Browse", selection: $selectedTab) {
                Text("Songs").tag(0)
                Text("Albums").tag(1)
                Text("Playlists").tag(2)
                Text("Search").tag(3)
            }
            .pickerStyle(.segmented)
            .padding(.horizontal, 80)
            .padding(.vertical, 24)

            switch selectedTab {
            case 0:  songsTab
            case 1:  albumsTab
            case 2:  playlistsTab
            default: searchTab
            }
        }
    }

    // MARK: - Songs tab

    private var songsTab: some View {
        List {
            ForEach(Array(songs.enumerated()), id: \.element.id) { index, song in
                Button {
                    play { try await appState.musicPlayer.play(songs: songs, startingAt: index) }
                } label: {
                    SongRow(song: song)
                }
                .buttonStyle(.plain)
            }
        }
        .listStyle(.plain)
    }

    // MARK: - Albums tab

    private var albumsTab: some View {
        let columns = Array(repeating: GridItem(.flexible(), spacing: 40), count: 4)
        return ScrollView {
            LazyVGrid(columns: columns, spacing: 40) {
                ForEach(albums) { album in
                    Button {
                        play { try await appState.musicPlayer.play(album: album) }
                    } label: {
                        AlbumCard(album: album)
                    }
                    .buttonStyle(.plain)
                    .hoverEffect()
                }
            }
            .padding(80)
        }
    }

    // MARK: - Playlists tab

    private var playlistsTab: some View {
        List(playlists) { playlist in
            Button {
                play { try await appState.musicPlayer.play(playlist: playlist) }
            } label: {
                PlaylistRow(playlist: playlist)
            }
            .buttonStyle(.plain)
        }
        .listStyle(.plain)
    }

    // MARK: - Search tab

    private var searchTab: some View {
        VStack(spacing: 0) {
            TextField("Search Apple Music…", text: $searchQuery)
                .textFieldStyle(.plain)
                .padding(.horizontal, 80)
                .padding(.vertical, 16)
                .submitLabel(.search)
                .onSubmit { Task { await performSearch() } }
                .onChange(of: searchQuery) { _, q in
                    if q.isEmpty { searchSongs = []; searchAlbums = [] }
                }

            if isSearching {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if searchSongs.isEmpty && searchAlbums.isEmpty && !searchQuery.isEmpty {
                ContentUnavailableView.search(text: searchQuery)
            } else {
                searchResults
            }
        }
    }

    private var searchResults: some View {
        List {
            if !searchSongs.isEmpty {
                Section("Songs") {
                    ForEach(Array(searchSongs.enumerated()), id: \.element.id) { idx, song in
                        Button {
                            play { try await appState.musicPlayer.play(songs: searchSongs, startingAt: idx) }
                        } label: { SongRow(song: song) }
                        .buttonStyle(.plain)
                    }
                }
            }
            if !searchAlbums.isEmpty {
                Section("Albums") {
                    ForEach(searchAlbums) { album in
                        Button {
                            play { try await appState.musicPlayer.play(album: album) }
                        } label: { AlbumRow(album: album) }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
        .listStyle(.plain)
    }

    // MARK: - Helpers

    /// Run a throwing async action that starts playback, then dismiss.
    private func play(_ action: @escaping () async throws -> Void) {
        Task {
            try? await action()
            dismiss()
        }
    }

    private func loadLibrary() async {
        isLoading  = true
        loadError  = nil
        async let s = appState.musicPlayer.librarySongs()
        async let a = appState.musicPlayer.libraryAlbums()
        async let p = appState.musicPlayer.libraryPlaylists()
        do {
            songs     = try await s
            albums    = try await a
            playlists = try await p
        } catch {
            loadError = error.localizedDescription
        }
        isLoading = false
    }

    private func performSearch() async {
        guard !searchQuery.isEmpty else { return }
        isSearching = true
        let result = try? await appState.musicPlayer.search(query: searchQuery)
        searchSongs  = result?.songs  ?? []
        searchAlbums = result?.albums ?? []
        isSearching  = false
    }
}

// MARK: - Row / card sub-views

private struct SongRow: View {
    let song: Song
    var body: some View {
        HStack(spacing: 16) {
            artThumb(song.artwork?.url(width: 80, height: 80))
            VStack(alignment: .leading, spacing: 4) {
                Text(song.title).font(.headline).lineLimit(1)
                Text(song.artistName).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer()
            if let dur = song.duration {
                Text(formatDuration(dur)).font(.caption).foregroundStyle(.tertiary).monospacedDigit()
            }
        }
        .padding(.vertical, 6)
    }
}

private struct AlbumCard: View {
    let album: Album
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            artThumb(album.artwork?.url(width: 300, height: 300))
                .frame(width: 200, height: 200)
                .clipShape(RoundedRectangle(cornerRadius: 10))
            Text(album.title).font(.headline).lineLimit(2).frame(width: 200, alignment: .leading)
            Text(album.artistName).font(.subheadline).foregroundStyle(.secondary).lineLimit(1).frame(width: 200, alignment: .leading)
        }
    }
}

private struct AlbumRow: View {
    let album: Album
    var body: some View {
        HStack(spacing: 16) {
            artThumb(album.artwork?.url(width: 80, height: 80))
            VStack(alignment: .leading, spacing: 4) {
                Text(album.title).font(.headline).lineLimit(1)
                Text(album.artistName).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
            }
        }
        .padding(.vertical, 6)
    }
}

private struct PlaylistRow: View {
    let playlist: Playlist
    var body: some View {
        HStack(spacing: 16) {
            artThumb(playlist.artwork?.url(width: 80, height: 80))
            Text(playlist.name).font(.headline).lineLimit(1)
        }
        .padding(.vertical, 6)
    }
}

// MARK: - Shared helpers (file-private)

@ViewBuilder
private func artThumb(_ url: URL?) -> some View {
    if let url {
        AsyncImage(url: url) { phase in
            if let img = phase.image {
                img.resizable().aspectRatio(contentMode: .fill)
            } else {
                artFallback
            }
        }
        .frame(width: 60, height: 60)
        .clipShape(RoundedRectangle(cornerRadius: 6))
    } else {
        artFallback.frame(width: 60, height: 60)
    }
}

private var artFallback: some View {
    RoundedRectangle(cornerRadius: 6)
        .fill(.secondary.opacity(0.2))
        .overlay { Image(systemName: "music.note").foregroundStyle(.tertiary) }
}

private func formatDuration(_ secs: TimeInterval) -> String {
    let s = Int(secs)
    return String(format: "%d:%02d", s / 60, s % 60)
}
