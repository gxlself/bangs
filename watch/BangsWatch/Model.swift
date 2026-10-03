import Foundation

// What Bangs on the computer answers with; see docs/watch.md. Enum-like
// fields stay strings so a newer Bangs with a new value does not break an
// older watch.

struct Snapshot: Decodable {
    var host: String
    /// Unix milliseconds on the computer.
    var now: Double
    var media: Media?
    var sessions: [Session]
    var todos: [Todo]
    var activities: [Activity]
}

struct Media: Decodable, Equatable {
    var title: String
    var artist: String
    var album: String
    var appName: String
    var playing: Bool
    /// Seconds, when the player reports a length.
    var duration: Double?
    /// Seconds into the track as of `elapsedAt`.
    var elapsed: Double?
    /// Unix milliseconds on the computer.
    var elapsedAt: Double
    /// Changes with the picture; fetch /v1/artwork when it does.
    var artwork: String?
    /// Changes with the lines; fetch /v1/lyrics when it does.
    var lyrics: String?
}

struct Session: Decodable, Identifiable, Equatable {
    var id: String
    /// "claude" or "codex".
    var agent: String
    var name: String
    var project: String
    /// "busy", "waiting" or "idle".
    var status: String
    var detail: String?
    /// Unix milliseconds on the computer.
    var updatedAt: Double

    var agentName: String {
        switch agent {
        case "claude": return "Claude"
        case "codex": return "Codex"
        default: return agent.capitalized
        }
    }
}

struct Todo: Decodable, Identifiable, Equatable {
    var id: String
    var text: String
    var createdAt: Double
}

struct Activity: Decodable, Identifiable, Equatable {
    var id: String
    var title: String
    var subtitle: String?
    var icon: String?
    /// 0…1.
    var progress: Double?
    var updatedAt: Double
}

struct LyricLine: Decodable, Equatable {
    /// Seconds into the track.
    var at: Double
    var text: String
    var translation: String?
}

struct LyricsPayload: Decodable {
    var lines: [LyricLine]
}

struct Hello: Decodable {
    var app: String
    var version: String
    var host: String
}

struct Paired: Decodable {
    var token: String
    var host: String
}

struct ServerError: Decodable {
    var error: String
}

/// The computer this watch is paired with, kept in the keychain.
struct Endpoint: Codable, Equatable {
    /// `http://192.168.1.20:17651`
    var base: String
    var token: String
    var host: String
}
