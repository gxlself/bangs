import SwiftUI
import WatchKit

/// Keys in user defaults. Kept outside the store, which belongs to the main
/// actor, so a view's property initializer can read them too.
enum Defaults {
    /// The address last typed, so pairing again does not mean typing it again.
    static let lastAddress = "lastAddress"
}

/// Everything the watch knows about the computer, refreshed every two
/// seconds while the app is on screen.
@MainActor
final class WatchStore: ObservableObject {
    enum Connection: Equatable {
        case connecting
        case online
        case offline
    }

    @Published private(set) var endpoint: Endpoint?
    @Published private(set) var snapshot: Snapshot?
    @Published private(set) var connection = Connection.connecting
    @Published private(set) var artwork: UIImage?
    @Published private(set) var lyrics: [LyricLine] = []
    /// A session that just finished, shown for a few seconds on any page.
    @Published private(set) var banner: String?
    /// Ticked off on the watch and still open in the last snapshot.
    @Published private(set) var removing: Set<String> = []

    /// The computer's clock minus the watch's, in milliseconds, so playback
    /// positions sampled over there can be carried forward over here.
    private var clockOffset: Double = 0
    private var artworkID: String?
    private var lyricsID: String?
    private var statuses: [String: String] = [:]
    private var poller: Task<Void, Never>?
    private var bannerTask: Task<Void, Never>?

    private static let endpointKey = "endpoint"
    private static let interval: Duration = .seconds(2)

    init() {
        #if DEBUG
        if DemoData.isOn {
            endpoint = DemoData.endpoint
            snapshot = DemoData.snapshot()
            connection = .online
            lyrics = DemoData.lyrics
            artwork = DemoData.artwork()
            return
        }
        #endif
        if let data = Keychain.load(Self.endpointKey) {
            endpoint = try? JSONDecoder().decode(Endpoint.self, from: data)
        }
    }

    /// The screenshot demo: nothing goes over the network, edits stay here.
    private var isDemo: Bool {
        #if DEBUG
        return DemoData.isOn
        #else
        return false
        #endif
    }

    private var client: BangsClient? {
        endpoint.flatMap(BangsClient.init(endpoint:))
    }

    var todos: [Todo] {
        (snapshot?.todos ?? []).filter { !removing.contains($0.id) }
    }

    // MARK: - Polling

    func resume() {
        guard endpoint != nil, poller == nil, !isDemo else { return }
        poller = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refresh()
                try? await Task.sleep(for: WatchStore.interval)
            }
        }
    }

    func pause() {
        poller?.cancel()
        poller = nil
    }

    func refresh() async {
        guard let client else { return }
        do {
            let next = try await client.state()
            clockOffset = next.now - Date().timeIntervalSince1970 * 1000
            noticeFinished(next.sessions)
            snapshot = next
            removing.formIntersection(next.todos.map(\.id))
            connection = .online
            await follow(next.media, with: client)
        } catch ClientError.unauthorized {
            // The computer forgot this watch (from its menu); pair again.
            forget()
        } catch {
            connection = .offline
        }
    }

    /// Fetches the picture and the lyrics only when they changed.
    private func follow(_ media: Media?, with client: BangsClient) async {
        if media?.artwork != artworkID {
            artworkID = media?.artwork
            if artworkID == nil {
                artwork = nil
            } else if let data = try? await client.artwork() {
                artwork = UIImage(data: data)
            } else {
                // Try again on the next refresh.
                artworkID = nil
            }
        }
        if media?.lyrics != lyricsID {
            lyricsID = media?.lyrics
            if lyricsID == nil {
                lyrics = []
            } else if let lines = try? await client.lyrics() {
                lyrics = lines
            } else {
                lyricsID = nil
            }
        }
    }

    /// A tap on the wrist when a session stops working, like the alert that
    /// opens the notch on the computer.
    private func noticeFinished(_ sessions: [Session]) {
        let previous = statuses
        statuses = Dictionary(sessions.map { ($0.id, $0.status) }, uniquingKeysWith: { first, _ in first })
        // The first answer after launch is not news.
        guard snapshot != nil else { return }
        guard let done = sessions.first(where: { previous[$0.id] == "busy" && $0.status != "busy" }) else { return }
        let what = done.status == "waiting" ? t("等你回复", "is waiting for you") : t("忙完了", "is done")
        show("\(done.agentName) · \(done.project) \(what)")
        WKInterfaceDevice.current().play(.notification)
    }

    private func show(_ text: String) {
        banner = text
        bannerTask?.cancel()
        bannerTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(4))
            guard !Task.isCancelled else { return }
            self?.banner = nil
        }
    }

    // MARK: - Now playing

    /// Where the track is now, carried forward from the computer's sample.
    func position(of media: Media, at date: Date) -> Double? {
        guard let elapsed = media.elapsed else { return nil }
        var position = elapsed
        if media.playing {
            let computerNow = date.timeIntervalSince1970 * 1000 + clockOffset
            position += max(0, computerNow - media.elapsedAt) / 1000
        }
        if let duration = media.duration, duration > 0 {
            position = min(position, duration)
        }
        return position
    }

    /// The line being sung at `position`.
    func lyric(at position: Double?) -> LyricLine? {
        guard let position, !lyrics.isEmpty else { return nil }
        // Lines come sorted by time; find the last one that has started.
        var low = 0
        var high = lyrics.count
        while low < high {
            let middle = (low + high) / 2
            if lyrics[middle].at <= position {
                low = middle + 1
            } else {
                high = middle
            }
        }
        return low == 0 ? nil : lyrics[low - 1]
    }

    func media(_ action: String) {
        if isDemo, action == "toggle", var media = snapshot?.media {
            let now = Date().timeIntervalSince1970 * 1000
            media.elapsed = position(of: media, at: Date())
            media.elapsedAt = now
            media.playing.toggle()
            snapshot?.media = media
            snapshot?.now = now
            return
        }
        guard let client, !isDemo else { return }
        WKInterfaceDevice.current().play(.click)
        Task {
            do {
                try await client.media(action)
            } catch {
                WKInterfaceDevice.current().play(.failure)
            }
            // Give the player a moment to say what it is doing now.
            try? await Task.sleep(for: .milliseconds(400))
            await refresh()
        }
    }

    // MARK: - To-do

    func addTodo(_ text: String) {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if isDemo, !text.isEmpty {
            snapshot?.todos.insert(Todo(id: UUID().uuidString, text: text, createdAt: Date().timeIntervalSince1970 * 1000), at: 0)
            return
        }
        guard let client, !text.isEmpty else { return }
        Task {
            do {
                try await client.addTodo(text)
            } catch {
                WKInterfaceDevice.current().play(.failure)
                show(error.localizedDescription)
            }
            await refresh()
        }
    }

    func complete(_ todo: Todo) {
        if isDemo {
            snapshot?.todos.removeAll { $0.id == todo.id }
            return
        }
        guard let client else { return }
        WKInterfaceDevice.current().play(.success)
        removing.insert(todo.id)
        Task {
            do {
                try await client.completeTodo(todo.id)
            } catch {
                removing.remove(todo.id)
                WKInterfaceDevice.current().play(.failure)
            }
            await refresh()
        }
    }

    // MARK: - Pairing

    func pair(address: String, code: String) async throws {
        guard let base = BangsClient.baseURL(address) else { throw ClientError.badAddress }
        UserDefaults.standard.set(address, forKey: Defaults.lastAddress)
        let client = BangsClient(base: base)
        _ = try await client.hello()
        let digits = code.filter(\.isNumber)
        let paired = try await client.pair(code: digits, name: WKInterfaceDevice.current().name)
        reset()
        keep(Endpoint(base: base.absoluteString, token: paired.token, host: paired.host))
        WKInterfaceDevice.current().play(.success)
        resume()
    }

    /// The computer is at a new address (another network, a new DHCP lease);
    /// the pairing itself still holds.
    func move(to address: String) async throws {
        guard let current = endpoint, let base = BangsClient.baseURL(address) else { throw ClientError.badAddress }
        let client = BangsClient(base: base, token: current.token)
        let hello = try await client.hello()
        do {
            _ = try await client.state()
        } catch ClientError.unauthorized {
            throw ClientError.server(t("那台电脑没和这只手表配对过", "That computer isn't paired with this watch"))
        }
        UserDefaults.standard.set(address, forKey: Defaults.lastAddress)
        keep(Endpoint(base: base.absoluteString, token: current.token, host: hello.host))
        connection = .connecting
        await refresh()
    }

    private func keep(_ endpoint: Endpoint) {
        if let data = try? JSONEncoder().encode(endpoint) {
            Keychain.save(data, Self.endpointKey)
        }
        self.endpoint = endpoint
    }

    /// Unpairs on both ends; the computer drops this watch's token.
    func unpair() async {
        try? await client?.unpair()
        forget()
    }

    /// Unpairs on this end only.
    private func forget() {
        Keychain.delete(Self.endpointKey)
        endpoint = nil
        reset()
    }

    private func reset() {
        pause()
        snapshot = nil
        connection = .connecting
        artwork = nil
        lyrics = []
        artworkID = nil
        lyricsID = nil
        statuses = [:]
        removing = []
        banner = nil
    }
}
