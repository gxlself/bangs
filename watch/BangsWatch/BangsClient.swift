import Foundation

/// A watch paired with one Mac.
struct Pairing: Codable, Equatable {
    /// `http://<address>:17651`
    var baseURL: URL
    var token: String
    /// The Mac's name, as it said when pairing.
    var host: String
    /// Set when the pairing came from iCloud: names the Mac's record, so a new
    /// address or token can be picked up from there.
    var macID: String?
}

enum BangsError: LocalizedError {
    case badAddress
    /// The Mac no longer knows this watch's token.
    case unauthorized
    case server(status: Int, message: String)
    /// None of the addresses a Mac put in iCloud answered.
    case unreachable(host: String)

    var errorDescription: String? {
        switch self {
        case .badAddress:
            return "地址看不懂，填 Mac 的 IP，比如 192.168.1.8"
        case .unauthorized:
            return "Mac 已经取消了这块手表的配对"
        case .unreachable(let host):
            return "连不上 \(host)，手表和 Mac 要在同一个 Wi‑Fi 下"
        case .server(let status, let message):
            if status == 403 && message.contains("too many") {
                return "错太多次了，托盘里已经换了新的配对码"
            }
            if status == 403 {
                return "配对码不对"
            }
            if status == 503 {
                return "Mac 上没打开「允许手表连接」"
            }
            return message.isEmpty ? "Mac 返回了 \(status)" : message
        }
    }
}

/// Talks to the watch API Bangs serves on the Mac (`src-tauri/src/watch.rs`).
struct BangsClient {
    static let port = 17651

    let pairing: Pairing

    private static let session: URLSession = {
        let configuration = URLSessionConfiguration.default
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.urlCache = nil
        configuration.waitsForConnectivity = false
        return URLSession(configuration: configuration)
    }()

    /// Turns what was typed or dictated into `http://host:17651`.
    static func baseURL(from address: String) -> URL? {
        var text = address.trimmingCharacters(in: .whitespacesAndNewlines)
        // Dictation in Chinese writes full-width punctuation.
        text = text
            .replacingOccurrences(of: "：", with: ":")
            .replacingOccurrences(of: "。", with: ".")
            .replacingOccurrences(of: "．", with: ".")
            .replacingOccurrences(of: " ", with: "")
        if !text.contains("://") {
            text = "http://" + text
        }
        guard var components = URLComponents(string: text),
              let host = components.host, !host.isEmpty
        else { return nil }
        if components.port == nil {
            components.port = port
        }
        components.path = ""
        components.query = nil
        components.fragment = nil
        return components.url
    }

    /// Trades the code shown in the Mac's tray for a token.
    static func pair(address: String, code: String) async throws -> Pairing {
        guard let base = baseURL(from: address) else { throw BangsError.badAddress }
        var request = URLRequest(url: base.appendingPathComponent("watch/pair"), timeoutInterval: 8)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(["code": code])
        struct Reply: Decodable {
            var token: String
            var host: String
        }
        let reply = try JSONDecoder().decode(Reply.self, from: try await send(request))
        return Pairing(baseURL: base, token: reply.token, host: reply.host)
    }

    /// Everything on the watch's screens. With `since`, the Mac holds the
    /// request until something changes (or about 20 seconds pass).
    func state(since rev: String?, timeout: TimeInterval = 30) async throws -> WatchState {
        let query = rev.map { [URLQueryItem(name: "since", value: $0)] } ?? []
        let request = makeRequest("watch/state", query: query, timeout: timeout)
        return try JSONDecoder().decode(WatchState.self, from: try await Self.send(request))
    }

    func artwork() async throws -> Data {
        try await Self.send(makeRequest("watch/artwork", timeout: 10))
    }

    /// "toggle", "next" or "previous".
    func media(_ action: String) async throws {
        _ = try await Self.send(makeRequest("watch/media", method: "POST", body: ["action": action]))
    }

    func addTodo(_ text: String) async throws {
        _ = try await Self.send(makeRequest("watch/todos", method: "POST", body: ["text": text]))
    }

    func removeTodo(_ id: String) async throws {
        _ = try await Self.send(makeRequest("watch/todos/\(id)", method: "DELETE"))
    }

    /// Tells the Mac to forget this watch.
    func unpair() async throws {
        _ = try await Self.send(makeRequest("watch/pair", method: "DELETE"))
    }

    private func makeRequest(
        _ path: String,
        method: String = "GET",
        query: [URLQueryItem] = [],
        body: [String: String]? = nil,
        timeout: TimeInterval = 8
    ) -> URLRequest {
        var components = URLComponents(
            url: pairing.baseURL.appendingPathComponent(path),
            resolvingAgainstBaseURL: false
        )!
        components.queryItems = query.isEmpty ? nil : query
        var request = URLRequest(url: components.url!, timeoutInterval: timeout)
        request.httpMethod = method
        request.setValue("Bearer \(pairing.token)", forHTTPHeaderField: "Authorization")
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try? JSONEncoder().encode(body)
        }
        return request
    }

    private static func send(_ request: URLRequest) async throws -> Data {
        let (data, response) = try await session.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        switch status {
        case 200..<300:
            return data
        case 401:
            throw BangsError.unauthorized
        default:
            let message = (try? JSONDecoder().decode([String: String].self, from: data))?["error"] ?? ""
            throw BangsError.server(status: status, message: message)
        }
    }
}
