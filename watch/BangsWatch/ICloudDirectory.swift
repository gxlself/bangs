import CloudKit

/// A Mac signed in to the same Apple ID with watch access on, as Bangs
/// describes itself in iCloud (`src-tauri/src/watch.rs`, `mod icloud`).
struct MacRecord: Decodable, Identifiable, Equatable {
    var id: String
    var host: String
    /// Tried in order: the LAN address, then the Bonjour name.
    var addresses: [String]
    var port: Int
    var token: String

    /// One pairing per address, in the order to try them.
    var pairings: [Pairing] {
        addresses.compactMap { address in
            var components = URLComponents()
            components.scheme = "http"
            components.host = address
            components.port = port
            return components.url.map { Pairing(baseURL: $0, token: token, host: host, macID: id) }
        }
    }
}

/// Reads the records Bangs keeps in the private CloudKit database. Only
/// devices signed in to the same Apple ID can see them.
enum ICloudDirectory {
    static let containerID = "iCloud.com.gxlself.bangs"
    private static let zoneID = CKRecordZone.ID(zoneName: "BangsWatch", ownerName: CKCurrentUserDefaultName)

    static func macs() async throws -> [MacRecord] {
        let database = CKContainer(identifier: containerID).privateCloudDatabase
        do {
            let changes = try await database.recordZoneChanges(inZoneWith: zoneID, since: nil)
            return changes.modificationResultsByID.values
                .compactMap { result -> MacRecord? in
                    guard let record = try? result.get().record,
                          let text = record.encryptedValues["payload"] as? String,
                          let data = text.data(using: .utf8)
                    else { return nil }
                    return try? JSONDecoder().decode(MacRecord.self, from: data)
                }
                .sorted { $0.host < $1.host }
        } catch let error as CKError where error.code == .zoneNotFound || error.code == .userDeletedZone {
            // No Mac on this Apple ID has turned watch access on yet.
            return []
        }
    }
}
