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

    enum CloudSearch: Equatable {
        case idle
        case searching
        case done
        case failed(String)
    }

    @Published private(set) var pairing: Pairing?
    @Published private(set) var state: WatchState?
    @Published private(set) var connection: Connection = .idle
    @Published private(set) var artwork: UIImage?
    /// Milliseconds to add to this watch's clock to get the Mac's.
    @Published private(set) var clockOffset: Double = 0
    /// Macs on this Apple ID that offer themselves in iCloud.
    @Published private(set) var cloudMacs: [MacRecord] = []
    @Published private(set) var cloudSearch: CloudSearch = .idle

    /// Connect by itself when iCloud finds exactly one Mac. Off once the user
    /// unpaired on purpose, so the pairing screen does not snap straight back.
    private var autoConnect = true

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
        save(try await BangsClient.pair(address: address, code: code))
    }

    /// Looks for Macs on this Apple ID, and connects to the only one there is.
    func searchICloud() async {
        cloudSearch = .searching
        do {
            cloudMacs = try await ICloudDirectory.macs()
            cloudSearch = .done
        } catch {
            cloudSearch = .failed(error.localizedDescription)
            return
        }
        if autoConnect, pairing == nil, cloudMacs.count == 1 {
            try? await connect(cloudMacs[0])
        }
    }

    /// Pairs with a Mac found in iCloud: no code, its record carries a token.
    func connect(_ mac: MacRecord) async throws {
        guard let reachable = await firstReachable(mac) else {
            throw BangsError.unreachable(host: mac.host)
        }
        save(reachable)
    }

    /// The first of the Mac's addresses that answers.
    private func firstReachable(_ mac: MacRecord) async -> Pairing? {
        for candidate in mac.pairings {
            if (try? await BangsClient(pairing: candidate).state(since: nil, timeout: 4)) != nil {
                return candidate
            }
        }
        return nil
    }

    /// When a Mac paired through iCloud stops answering, its record may say
    /// why: a new address from DHCP, or a new token after "取消所有手表配对".
    /// Returns whether it found a pairing that works.
    private func refreshFromICloud() async -> Bool {
        guard let id = pairing?.macID,
              let mac = (try? await ICloudDirectory.macs())?.first(where: { $0.id == id }),
              let reachable = await firstReachable(mac)
        else { return false }
        if reachable != pairing {
            save(reachable)
        }
        return true
    }

    private func save(_ pairing: Pairing) {
        if let data = try? JSONEncoder().encode(pairing) {
            UserDefaults.standard.set(data, forKey: Self.pairingKey)
        }
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
        if tellMac {
            autoConnect = false
            if let client {
                Task { try? await client.unpair() }
            }
        }
    }

    // MARK: Staying in sync

    /// Starts long polling, unless it is already running. Called whenever the
    /// app comes to the front.
    func start() {
        guard loop == nil, pairing != nil else { return }
        connection = .connecting
        loop = Task { [weak self] in
            await self?.run()
        }
    }

    /// Called when the app goes to the background: watchOS would suspend the
    /// request anyway.
    func stop() {
        loop?.cancel()
        loop = nil
    }

    private func run() async {
        // No `since` on the first request, so it answers at once.
        var since: String?
        var failures = 0
        // Read the client every time round: iCloud may have moved the pairing.
        while !Task.isCancelled, let client {
            do {
                let next = try await client.state(since: since)
                since = next.rev
                failures = 0
                apply(next)
                connection = .online
            } catch BangsError.unauthorized {
                if await refreshFromICloud() {
                    since = nil
                    continue
                }
                forget(tellMac: false)
                return
            } catch {
                if Task.isCancelled || (error as? URLError)?.code == .cancelled {
                    return
                }
                connection = .offline(error.localizedDescription)
                since = nil
                failures += 1
                if failures % 10 == 2, await refreshFromICloud() {
                    continue
                }
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
