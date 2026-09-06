import Foundation
import Testing

import KnottySession

/// The configuration path end to end: a file the user wrote, through the
/// boundary, into the values a window opens with.
private func loading(_ text: String) throws -> Config.Loaded {
    let directory = URL(fileURLWithPath: NSTemporaryDirectory())
        .appending(path: "knotty-config-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }

    let path = directory.appending(path: "config.toml")
    try Data(text.utf8).write(to: path)
    return try Config.load(from: path)
}

@Test func aFontWrittenInTheFileIsWhatTheWindowOpensWith() throws {
    let loaded = try loading("[font]\nfamily = \"Menlo\"\nsize = 15.5\n")

    #expect(loaded.diagnostic == nil)
    #expect(loaded.config.font.family == "Menlo")
    #expect(loaded.config.font.size == 15.5)
}

@Test func noFileIsTheDefaultsAndNotAFailure() throws {
    let missing = URL(fileURLWithPath: NSTemporaryDirectory())
        .appending(path: "knotty-\(UUID().uuidString)/config.toml")
    let loaded = try Config.load(from: missing)

    #expect(loaded.diagnostic == nil)
    #expect(loaded.config.font.family == "JetBrains Mono")
    #expect(loaded.config.font.size == 13.0)
}

@Test func whatTheFileLeavesOutIsFilledIn() throws {
    let loaded = try loading("[font]\nsize = 16.0\n")

    #expect(loaded.diagnostic == nil)
    #expect(loaded.config.font.family == "JetBrains Mono")
    #expect(loaded.config.font.size == 16.0)
}

/// A typo is a diagnostic to show and not an app that will not start: the
/// configuration beside it is whole, because the first load has no previous
/// one to keep.
@Test func aBadValueComesBackAsADiagnosticBesideTheDefaults() throws {
    let loaded = try loading("[font]\nfamily = \"Menlo\"\nsize = -3.0\n")

    #expect(loaded.diagnostic?.contains("size") == true)
    #expect(loaded.config.font.family == "JetBrains Mono")
    #expect(loaded.config.font.size == 13.0)
}

/// The file the app reads without being told where, which is the one the
/// user is told to write.
@Test func theFileIsUnderTheUsersConfigDirectory() {
    #expect(Config.path.path(percentEncoded: false).hasSuffix("/.config/knotty/config.toml"))
}
