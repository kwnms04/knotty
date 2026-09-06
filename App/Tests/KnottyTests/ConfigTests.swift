import Foundation
import Testing

import KnottySession

/// A directory of this test's own, for a file that is written more than once.
private func temporaryDirectory() throws -> URL {
    let directory = URL(fileURLWithPath: NSTemporaryDirectory())
        .appending(path: "knotty-config-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    return directory
}

/// The configuration path end to end: a file the user wrote, through the
/// boundary, into the values a window opens with.
private func loading(_ text: String) throws -> Config.Loaded {
    let directory = try temporaryDirectory()
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

/// The colours a screen is drawn in, straight out of the file.
@Test func aThemeWrittenInTheFileIsWhatTheWindowOpensWith() throws {
    let loaded = try loading(
        """
        [theme]
        background = "#1e1e1e"
        foreground = "#d4d4d4"
        cursor = "#ff8800"
        palette = [
          "#000000", "#cc6666", "#b5bd68", "#f0c674",
          "#81a2be", "#b294bb", "#8abeb7", "#c5c8c6",
          "#666666", "#d54e53", "#b9ca4a", "#e7c547",
          "#7aa6da", "#c397d8", "#70c0b1", "#eaeaea",
        ]
        """
    )

    #expect(loaded.diagnostic == nil)
    let theme = loaded.config.theme
    #expect((theme.background.r, theme.background.g, theme.background.b) == (0x1e, 0x1e, 0x1e))
    #expect((theme.foreground.r, theme.foreground.g, theme.foreground.b) == (0xd4, 0xd4, 0xd4))
    #expect(theme.cursor.map { ($0.r, $0.g, $0.b) }.map { $0 == (0xff, 0x88, 0x00) } == true)
    #expect(theme.palette.count == 16)
    #expect((theme.palette[1].r, theme.palette[1].g, theme.palette[1].b) == (0xcc, 0x66, 0x66))
}

/// Terminal.app's Basic is what a screen nobody themed opens in, and the
/// cursor is the one colour with no default — unwritten means it takes the
/// colour of the text it stands on. cf. 04-renderer R1.
@Test func aThemeNobodyWroteIsTerminalsOwnColours() throws {
    let theme = try loading("[font]\nsize = 16.0\n").config.theme

    #expect((theme.background.r, theme.background.g, theme.background.b) == (255, 255, 255))
    #expect((theme.foreground.r, theme.foreground.g, theme.foreground.b) == (0, 0, 0))
    #expect(theme.cursor == nil)
    #expect(theme.palette.count == 16)
    #expect((theme.palette[1].r, theme.palette[1].g, theme.palette[1].b) == (0x99, 0, 0))
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

/// A typo made while the app is running costs the banner and nothing else.
///
/// The difference from the first load, which has the defaults to fall back
/// on: here there is a configuration the user is working in, and taking their
/// colours away halfway through a line they are still typing would be the
/// editing losing them the terminal they are editing in.
@Test func aTypoOnReloadKeepsWhatIsInForce() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let path = directory.appending(path: "config.toml")

    try Data("[font]\nfamily = \"Menlo\"\nsize = 15.5\n".utf8).write(to: path)
    let inForce = try Config.load(from: path).config

    try Data("[font]\nfamily = \"Menlo\"\nsize = -3.0\n".utf8).write(to: path)
    let reloaded = try Config.reload(from: path, keeping: inForce)

    #expect(reloaded.diagnostic?.contains("size") == true)
    #expect(reloaded.config == inForce)
}

/// A file the user fixed is a file that applies, which is the other half of
/// the same call.
@Test func aFileThatParsesOnReloadIsWhatComesBack() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let path = directory.appending(path: "config.toml")

    try Data("[font]\nfamily = \"Menlo\"\nsize = 15.5\n".utf8).write(to: path)
    let inForce = try Config.load(from: path).config

    try Data("[font]\nfamily = \"Menlo\"\nsize = 18.0\n".utf8).write(to: path)
    let reloaded = try Config.reload(from: path, keeping: inForce)

    #expect(reloaded.diagnostic == nil)
    #expect(reloaded.config.font.size == 18.0)
}

/// Saves in quick succession are one reload.
///
/// What a hand on ⌘S twice comes to, and what one save comes to as well: an
/// editor writing a file touches its directory more than once, and every one
/// of those would otherwise be a reload of its own — half of them reading a
/// file that is not finished being written.
@MainActor @Test func consecutiveSavesAreOneReload() async throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let path = directory.appending(path: "config.toml")
    try Data("[font]\nsize = 13.0\n".utf8).write(to: path)

    let reloads = Reloads()
    let watch = Config.Watch(path: path) { reloads.count += 1 }

    // What making the directory left in flight is not what this is about, and
    // a stream is not watching the moment it is asked to.
    try await Task.sleep(for: .milliseconds(400))
    reloads.count = 0

    // No tolerance, because this gap is the thing being tested: a sleep the
    // runtime is free to round up could put the two saves in windows of their
    // own and prove nothing.
    try Data("[font]\nsize = 14.0\n".utf8).write(to: path)
    try await Task.sleep(for: .milliseconds(30), tolerance: .zero)
    try Data("[font]\nsize = 15.0\n".utf8).write(to: path)

    // Comfortably past the latency the stream gathers for, so that a second
    // callback would have arrived by now if there were going to be one.
    try await Task.sleep(for: .milliseconds(800))
    #expect(reloads.count == 1)

    withExtendedLifetime(watch) {}
}

/// The first configuration a user ever writes is seen without a restart.
///
/// Writing it makes `~/.config/knotty` as well as the file in it, so the
/// directory the watch was given did not exist when the watch began. A stream
/// that resolved its path once would stay silent through exactly the save
/// that matters most.
@MainActor @Test func aConfigurationWrittenForTheFirstTimeIsSeen() async throws {
    let parent = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: parent) }
    // Named but not made, the way `~/.config/knotty` is before the first save.
    let directory = parent.appending(path: "knotty")
    let path = directory.appending(path: "config.toml")

    let reloads = Reloads()
    let watch = Config.Watch(path: path) { reloads.count += 1 }

    try await Task.sleep(for: .milliseconds(400))
    reloads.count = 0

    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    try Data("[font]\nsize = 14.0\n".utf8).write(to: path)

    try await Task.sleep(for: .seconds(1))
    #expect(reloads.count >= 1)

    withExtendedLifetime(watch) {}
}

/// How often the watch called back. A reference, because the closure it is
/// counted in outlives the call that made it.
@MainActor private final class Reloads {
    var count = 0
}
