// swift-tools-version:5.9
import PackageDescription

// Bangs sync, Swift side. The wire format and merge rules live in docs/sync.md.
//
//   BangsSyncCore    Foundation only: SyncRecord, last-writer-wins, RecordStore. Builds on Linux.
//   BangsCloud       CloudKit transport (CloudEngine). Used by the iOS app.
//   BangsCloudBridge macOS only: the C ABI that the Rust app links as a static library.
let package = Package(
    name: "BangsCloud",
    platforms: [
        .macOS(.v12),
        .iOS(.v16),
    ],
    products: [
        .library(name: "BangsSyncCore", targets: ["BangsSyncCore"]),
        .library(name: "BangsCloud", targets: ["BangsCloud"]),
        .library(name: "BangsCloudBridge", type: .static, targets: ["BangsCloudBridge"]),
    ],
    targets: [
        .target(name: "BangsSyncCore"),
        .target(name: "BangsCloud", dependencies: ["BangsSyncCore"]),
        .target(name: "BangsCloudBridge", dependencies: ["BangsCloud", "BangsSyncCore"]),
        .testTarget(name: "BangsSyncCoreTests", dependencies: ["BangsSyncCore"]),
    ]
)
