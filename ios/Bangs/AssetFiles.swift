import Foundation

/// Where downloaded shelf files live. The engine copies each CKAsset to
/// <state>/assets/<recordName with ':' replaced by '_'>. The folder of an iOS app container
/// changes between installs and updates, so the path is rebuilt from the key every time it is
/// needed instead of being read back from the stored record.
enum AssetFiles {
    static func url(forKey key: String, stateDirectory: URL) -> URL {
        var fileName = key.replacingOccurrences(of: ":", with: "_")
        fileName = fileName.replacingOccurrences(of: "/", with: "_")
        return stateDirectory
            .appendingPathComponent("assets", isDirectory: true)
            .appendingPathComponent(fileName)
    }

    static func exists(forKey key: String, stateDirectory: URL) -> Bool {
        let path = url(forKey: key, stateDirectory: stateDirectory).path
        return FileManager.default.fileExists(atPath: path)
    }

    /// QuickLook decides how to show a file from its extension, and the stored copy has none.
    /// Copies it to a temporary folder under its real name and returns that URL.
    static func previewCopy(for item: ShelfItem, stateDirectory: URL) -> URL? {
        let source = url(forKey: item.id, stateDirectory: stateDirectory)
        let fileManager = FileManager.default
        guard fileManager.fileExists(atPath: source.path) else { return nil }

        var name = item.name.replacingOccurrences(of: "/", with: "_")
        if name.isEmpty {
            name = "file"
        }
        if !item.fileExtension.isEmpty && !name.lowercased().hasSuffix("." + item.fileExtension.lowercased()) {
            name += "." + item.fileExtension
        }

        let folderName = item.id.replacingOccurrences(of: ":", with: "_").replacingOccurrences(of: "/", with: "_")
        let directory = fileManager.temporaryDirectory
            .appendingPathComponent("BangsPreview", isDirectory: true)
            .appendingPathComponent(folderName, isDirectory: true)
        let destination = directory.appendingPathComponent(name)
        do {
            try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
            if fileManager.fileExists(atPath: destination.path) {
                try fileManager.removeItem(at: destination)
            }
            try fileManager.copyItem(at: source, to: destination)
            return destination
        } catch {
            return nil
        }
    }
}
