import Foundation

/// What `GET /watch/state` returns; see `src-tauri/src/watch.rs`.
struct WatchState: Decodable, Equatable {
    /// Hash of everything below. Sent back as `since` to wait for the next change.
    var rev: String
    /// The Mac's name.
    var host: String
    /// Unix milliseconds on the Mac's clock when this was sent.
    var serverTime: Double
    var media: Media?
    /// Timed lines of the track playing; empty when there are none.
    var lyrics: [LyricLine]
    var sessions: [Session]
    var todos: [Todo]
}

struct Media: Decodable, Equatable {
    var title: String
    var artist: String
    var appName: String
    var playing: Bool
    /// Seconds.
    var duration: Double?
    /// Seconds into the track as of `elapsedAt`.
    var elapsed: Double?
    /// Unix milliseconds on the Mac's clock.
    var elapsedAt: Double
    /// Changes with the picture; `GET /watch/artwork` has the bytes.
    var artworkId: String?

    /// Seconds into the track at `date`. `clockOffset` is what to add to this
    /// watch's clock, in milliseconds, to get the Mac's.
    func position(at date: Date, clockOffset: Double) -> Double? {
        guard var position = elapsed, position.isFinite else { return nil }
        if playing && elapsedAt > 0 {
            let now = date.timeIntervalSince1970 * 1000 + clockOffset
            position += max(0, (now - elapsedAt) / 1000)
        }
        if let duration, duration > 0 {
            position = min(position, duration)
        }
        return max(0, position)
    }
}

struct LyricLine: Decodable, Equatable {
    /// Seconds into the track.
    var at: Double
    var text: String
    var translation: String?
}

extension Array where Element == LyricLine {
    /// The line being sung at `position`.
    func line(at position: Double) -> LyricLine? {
        last(where: { $0.at <= position })
    }
}

struct Session: Decodable, Equatable, Identifiable {
    enum Status: String, Decodable {
        /// Working.
        case busy
        /// Stopped and wants an answer.
        case waiting
        case idle
    }

    var id: String
    /// "claude" or "codex".
    var agent: String
    var project: String
    var name: String
    var status: Status
    /// What it is waiting for, when the session says so.
    var detail: String?
    /// Unix milliseconds of the last status change.
    var updatedAt: Double
}

struct Todo: Decodable, Equatable, Identifiable {
    var id: String
    var text: String
    /// Unix milliseconds.
    var createdAt: Double
}
