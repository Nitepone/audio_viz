import Foundation
import MusicKit
import Combine

// ── MusicPlayer ───────────────────────────────────────────────────────────────
//
// Wraps MusicKit's ApplicationMusicPlayer for library browsing and playback.
//
// ⚠️  Audio capture note
// ──────────────────────
// ApplicationMusicPlayer routes audio through the system session, not through
// our AVAudioEngine.  This means the visualizer runs on the audio it captures
// from *our* engine (test tone, or future local-file playback), not on Apple
// Music streams.  The visualizer will show idle animation while Apple Music
// is playing; full synchronised visualisation requires audio to flow through
// AudioEngine (see PLAN.md for the companion-app stretch goal).
//
// Threading
// ─────────
// All properties and methods are @MainActor.  Background MusicKit requests
// are awaited inside async functions; results are published on the main actor.

@MainActor
final class MusicPlayer: ObservableObject {

    // MARK: - Published

    @Published var authStatus: MusicAuthorization.Status = .notDetermined
    @Published var isPlaying   = false
    @Published var currentSong: Song?
    @Published var isMusicKitAvailable = false

    // MARK: - Private

    private var pollTask: Task<Void, Never>?

    // MARK: - Init

    init() {
        authStatus = MusicAuthorization.currentStatus
        isMusicKitAvailable = authStatus == .authorized
        startObservingPlayer()
    }

    // MARK: - Authorization

    /// Request access to Apple Music.  Call once on first launch (or when the
    /// user taps the "Connect Apple Music" button in the library view).
    func requestAuthorization() async {
        authStatus = await MusicAuthorization.request()
        isMusicKitAvailable = authStatus == .authorized
    }

    // MARK: - Library queries

    /// Fetch the user's library songs, sorted by title.
    func librarySongs(limit: Int = 100) async throws -> [Song] {
        var req = MusicLibraryRequest<Song>()
        req.limit = limit
        req.sort(by: \.title, ascending: true)
        return Array(try await req.response().items)
    }

    /// Fetch the user's library albums, sorted by title.
    func libraryAlbums(limit: Int = 100) async throws -> [Album] {
        var req = MusicLibraryRequest<Album>()
        req.limit = limit
        req.sort(by: \.title, ascending: true)
        return Array(try await req.response().items)
    }

    /// Fetch the user's playlists.
    func libraryPlaylists() async throws -> [Playlist] {
        var req = MusicLibraryRequest<Playlist>()
        return Array(try await req.response().items)
    }

    // MARK: - Catalog search

    /// Search the Apple Music catalog.  Returns up to 25 songs and 10 albums.
    func search(query: String) async throws -> (songs: [Song], albums: [Album]) {
        guard !query.isEmpty else { return ([], []) }
        var req = MusicCatalogSearchRequest(term: query, types: [Song.self, Album.self])
        req.limit = 25
        let res = try await req.response()
        return (Array(res.songs), Array(res.albums))
    }

    // MARK: - Playback

    /// Play a list of songs starting at `index`.
    func play(songs: [Song], startingAt index: Int = 0) async throws {
        guard index < songs.count else { return }
        ApplicationMusicPlayer.shared.queue = .init(for: songs, startingAt: songs[index])
        try await ApplicationMusicPlayer.shared.play()
    }

    /// Play an album (loads tracks first).
    func play(album: Album) async throws {
        let detailed = try await album.with([.tracks])
        guard let tracks = detailed.tracks else { return }
        ApplicationMusicPlayer.shared.queue = .init(for: tracks)
        try await ApplicationMusicPlayer.shared.play()
    }

    /// Play a playlist (loads tracks first).
    func play(playlist: Playlist) async throws {
        let detailed = try await playlist.with([.tracks])
        guard let tracks = detailed.tracks else { return }
        ApplicationMusicPlayer.shared.queue = .init(for: tracks)
        try await ApplicationMusicPlayer.shared.play()
    }

    func pause() {
        ApplicationMusicPlayer.shared.pause()
    }

    func togglePlayPause() {
        let player = ApplicationMusicPlayer.shared
        if player.state.playbackStatus == .playing {
            player.pause()
        } else {
            Task { try? await player.play() }
        }
    }

    func skipToNext() {
        Task { try? await ApplicationMusicPlayer.shared.skipToNextEntry() }
    }

    func skipToPrevious() {
        Task { try? await ApplicationMusicPlayer.shared.skipToPreviousEntry() }
    }

    // MARK: - Private

    private func startObservingPlayer() {
        // Poll the player state every 0.5 s.  MusicKit does not currently
        // expose a stable Combine/AsyncSequence publisher for playback state
        // on tvOS, so polling is the simplest reliable approach.
        pollTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                self?.syncPlayerState()
                try? await Task.sleep(for: .milliseconds(500))
            }
        }
    }

    private func syncPlayerState() {
        let player = ApplicationMusicPlayer.shared
        isPlaying = player.state.playbackStatus == .playing

        if let entry = player.queue.currentEntry,
           case .song(let song) = entry.item {
            currentSong = song
        } else {
            currentSong = nil
        }
    }
}
