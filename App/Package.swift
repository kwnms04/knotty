// swift-tools-version: 6.0

import Foundation
import PackageDescription

// SwiftPM knows nothing about cargo, so the staticlib the boundary lives in is
// named by path. The assembly script builds it before it calls `swift build`;
// skipping that order fails at link time rather than silently. cf. adr/0014.
let repositoryRoot = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .deletingLastPathComponent()
    .path
// Named as a file rather than with `-l`: cargo emits a dylib beside the
// staticlib and the linker prefers it, which linked the core dynamically
// against an absolute path in the build tree. cf. 05-swift-app 1.
let coreLibrary = "\(repositoryRoot)/target/release/libknotty_ffi.a"

// One direction only: CKnotty -> KnottySession -> KnottyRender -> knotty.
// The graph is the shape of the contracts in 05-swift-app, but it does not
// enforce them — what does is `scripts/build-app.sh`. cf. adr/0015.
let package = Package(
    name: "knotty",
    // Nothing here asks for more, and the floor moves when something does.
    platforms: [.macOS(.v14)],
    targets: [
        .systemLibrary(name: "CKnotty", path: "Sources/CKnotty"),
        .target(
            name: "KnottySession",
            dependencies: ["CKnotty"],
            linkerSettings: [.unsafeFlags([coreLibrary])]
        ),
        .target(name: "KnottyRender", dependencies: ["KnottySession"]),
        .executableTarget(
            name: "knotty",
            dependencies: ["KnottySession", "KnottyRender"],
            // SwiftPM copies Metal sources rather than compiling them, so the
            // assembly script owns this file.
            exclude: ["Shaders.metal"]
        ),
        .testTarget(
            name: "KnottyTests",
            dependencies: ["KnottySession", "KnottyRender"],
            // The renderer goldens are read by path, the way the recordings
            // they are made from are. SwiftPM would otherwise ask to be told
            // what they are.
            exclude: ["goldens"]
        ),
    ]
)
