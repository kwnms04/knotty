import Foundation

/// The size the recordings were made at, and the scrollback the Rust harness
/// replays them with. Replaying at another size would show a screen no
/// application ever drew.
let cols: UInt16 = 80
let rows: UInt16 = 24
let scrollback = 1000

/// The repository root, four levels up from a file in this directory.
let repositoryRoot = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .deletingLastPathComponent()
    .deletingLastPathComponent()
    .deletingLastPathComponent()

/// A recording, read from where it lives rather than copied in beside these
/// tests. The point of a detached session being public is that both layers
/// look at the same stream. cf. adr/0008.
func recording(_ name: String) throws -> [UInt8] {
    let file = repositoryRoot.appending(path: "crates/knotty-harness/recordings/\(name).vt")
    return try [UInt8](Data(contentsOf: file))
}
