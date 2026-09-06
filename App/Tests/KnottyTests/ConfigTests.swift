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
