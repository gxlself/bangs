import Foundation

enum ClientError: LocalizedError {
    case badAddress
    /// The computer does not know this watch (or the pairing code was wrong).
    case unauthorized(String)
    case tooSoon
    case notBangs
    case server(String)
    case unreachable

    var errorDescription: String? {
        switch self {
        case .badAddress:
            return t("地址不对，填 Mac 菜单里显示的那个", "That address won't work — use the one in the Bangs menu")
        case .unauthorized:
            return t("配对码不对", "Wrong pairing code")
        case .tooSoon:
            return t("太快了，等两秒再试", "Too fast — wait a moment and try again")
        case .notBangs:
            return t("那个地址上不是 Bangs", "That address isn't Bangs")
        case .server(let message):
            return message
        case .unreachable:
            return t("连不上 Mac", "Can't reach the Mac")
        }
    }
}

/// Talks to Bangs on the computer over the local network.
struct BangsClient {
    static let defaultPort = 17651

    let base: URL
    let token: String?

    private static let session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 6
        configuration.timeoutIntervalForResource = 15
        configuration.waitsForConnectivity = false
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        return URLSession(configuration: configuration)
    }()

    private static let decoder = JSONDecoder()

    /// Accepts `192.168.1.20`, `192.168.1.20:17651`, `mac.local` or a full
    /// `http://` URL, as typed or scribbled on a watch.
    static func baseURL(_ typed: String) -> URL? {
        var text = typed
            .replacingOccurrences(of: "。", with: ".")
            .replacingOccurrences(of: "：", with: ":")
            .replacingOccurrences(of: " ", with: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if text.lowercased().hasPrefix("http://") {
            text = String(text.dropFirst("http://".count))
        }
        text = text.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        guard !text.isEmpty, !text.contains("/") else { return nil }
        if !text.contains(":") {
            text += ":\(defaultPort)"
        }
        guard let url = URL(string: "http://\(text)"), url.host != nil else { return nil }
        return url
    }

    init(base: URL, token: String? = nil) {
        self.base = base
        self.token = token
    }

    init?(endpoint: Endpoint) {
        guard let base = URL(string: endpoint.base) else { return nil }
        self.init(base: base, token: endpoint.token)
    }

    // MARK: - Endpoints

    func hello() async throws -> Hello {
        do {
            let data = try await send("GET", "/v1/hello")
            let hello: Hello = try decode(data)
            guard hello.app == "bangs" else { throw ClientError.notBangs }
            return hello
        } catch is DecodingError {
            throw ClientError.notBangs
        }
    }

    func pair(code: String, name: String) async throws -> Paired {
        let data = try await send("POST", "/v1/pair", body: ["code": code, "name": name])
        return try decode(data)
    }

    func unpair() async throws {
        _ = try await send("DELETE", "/v1/pair")
    }

    func state() async throws -> Snapshot {
        let data = try await send("GET", "/v1/state")
        return try decode(data)
    }

    func artwork() async throws -> Data {
        try await send("GET", "/v1/artwork")
    }

    func lyrics() async throws -> [LyricLine] {
        let data = try await send("GET", "/v1/lyrics")
        let payload: LyricsPayload = try decode(data)
        return payload.lines
    }

    /// `toggle`, `next` or `previous`.
    func media(_ action: String) async throws {
        _ = try await send("POST", "/v1/media", body: ["action": action])
    }

    func addTodo(_ text: String) async throws {
        _ = try await send("POST", "/v1/todos", body: ["text": text])
    }

    func removeTodo(_ id: String) async throws {
        let escaped = id.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? id
        _ = try await send("DELETE", "/v1/todos/\(escaped)")
    }

    // MARK: - Plumbing

    private func decode<T: Decodable>(_ data: Data) throws -> T {
        try Self.decoder.decode(T.self, from: data)
    }

    private func send(_ method: String, _ path: String, body: [String: String]? = nil) async throws -> Data {
        guard let url = URL(string: base.absoluteString + path) else { throw ClientError.badAddress }
        var request = URLRequest(url: url)
        request.httpMethod = method
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
        }
        let result: (Data, URLResponse)
        do {
            result = try await Self.session.data(for: request)
        } catch {
            throw ClientError.unreachable
        }
        let (data, response) = result
        guard let http = response as? HTTPURLResponse else { throw ClientError.unreachable }
        let message = (try? Self.decoder.decode(ServerError.self, from: data))?.error
            ?? HTTPURLResponse.localizedString(forStatusCode: http.statusCode)
        switch http.statusCode {
        case 200..<300: return data
        case 401: throw ClientError.unauthorized(message)
        case 429: throw ClientError.tooSoon
        default: throw ClientError.server(message)
        }
    }
}
