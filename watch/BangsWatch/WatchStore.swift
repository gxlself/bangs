import SwiftUI
import WatchKit

/// The pairing, the latest state from the Mac and the loop that keeps it fresh.
@MainActor
final class WatchStore: ObservableObject {
    enum Connection: Equatable {
        case idle
        case connecting
        case online
        case offline(String)
    }

    @Published private(set) var pairing: Pairing?
    @Published private(set) var state: WatchState?
    @Published private(set) var connection: Connection = .idle
    @Published private(set) var artwork: UIImage?
    /// Milliseconds to add to this watch's clock to get the Mac's.
    @Published private(set) var clockOffset: Double = 0

    private var loop: Task<Void, Never>?
    private var artworkId: String?
    private static let pairingKey = "pairing"

    init() {
        if let data = UserDefaults.standard.data(forKey: Self.pairingKey),
           let saved = try? JSONDecoder().decode(Pairing.self, from: data) {
            pairing = saved
        }
    }

    private var client: BangsClient? {
        pairing.map { BangsClient(pairing: $0) }
    }

    // MARK: Pairing

    func pair(address: String, code: String) async throws {
        let pairing = try await BangsClient.pair(address: address, code: code)
        UserDefaults.standard.set(try JSONEncoder().encode(pairing), forKey: Self.pairingKey)
        self.pairing = pairing
        start()
    }

    /// Back to the pairing screen. `tellMac` also asks the Mac to drop the token.
    func forget(tellMac: Bool) {
        let client = self.client
        stop()
        UserDefaults.standard.removeObject(forKey: Self.pairingKey)
        pairing = nil
        state = nil
        artwork = nil
        artworkId = nil
        connection = .idle
        if tellMac, let client {
            Task { try? await client.unpair() }
        }
    }

    // MARK: Staying in sync

    /// Starts long polling, unless it is already running. Called whenever the
    /// app comes to the front.
    func start() {
        guard loop == nil, let client else { return }
        connection = .connecting
        loop = Task { [weak self] in
            await self?.run(client)
        }
    }

    /// Called when the app goes to the background: watchOS would suspend the
    /// request anyway.
    func stop() {
        loop?.cancel()
        loop = nil
    }

    private func run(_ client: BangsClient) async {
        // No `since` on the first request, so it answers at once.
        var since: String?
        while !Task.isCancelled {
            do {
                let next = try await client.state(since: since)
                since = next.rev
                apply(next)
                connection = .online
            } catch BangsError.unauthorized {
                forget(tellMac: false)
                return
            } catch {
                if Task.isCancelled || (error as? URLError)?.code == .cancelled {
                    return
                }
                connection = .offline(error.localizedDescription)
                since = nil
                try? await Task.sleep(nanoseconds: 3_000_000_000)
            }
        }
    }

    private func apply(_ next: WatchState) {
        clockOffset = next.serverTime - Date().timeIntervalSince1970 * 1000
        if let previous = state {
            tapIfSessionStopped(before: previous.sessions, after: next.sessions)
        }
        state = next

        let id = next.media?.artworkId
        guard id != artworkId else { return }
        artworkId = id
        artwork = nil
        guard id != nil, let client else { return }
        Task { [weak self] in
            let data = try? await client.artwork()
            guard let self, self.artworkId == id, let data else { return }
            self.artwork = UIImage(data: data)
        }
    }

    /// The same moment the notch opens on the Mac: a session that was working
    /// has stopped, to wait for an answer or because it is done.
    private func tapIfSessionStopped(before: [Session], after: [Session]) {
        let wasBusy = Set(before.filter { $0.status == .busy }.map(\.id))
        let stopped = after.filter { wasBusy.contains($0.id) && $0.status != .busy }
        guard let first = stopped.first else { return }
        WKInterfaceDevice.current().play(first.status == .waiting ? .notification : .success)
    }

    // MARK: Actions

    /// "toggle", "next" or "previous".
    func media(_ action: String) {
        guard let client else { return }
        WKInterfaceDevice.current().play(.click)
        if action == "toggle", var media = state?.media {
            // Show the new state now; the Mac's answer follows within a poll.
            let now = Date()
            media.elapsed = media.position(at: now, clockOffset: clockOffset)
            media.elapsedAt = now.timeIntervalSince1970 * 1000 + clockOffset
            media.playing.toggle()
            state?.media = media
        }
        Task { try? await client.media(action) }
    }

    func addTodo(_ text: String) {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, let client else { return }
        Task { try? await client.addTodo(text) }
    }

    func removeTodo(_ id: String) {
        guard let client else { return }
        WKInterfaceDevice.current().play(.success)
        state?.todos.removeAll { $0.id == id }
        Task { try? await client.removeTodo(id) }
    }
}
