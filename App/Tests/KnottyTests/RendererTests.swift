import Foundation
import Testing

import KnottyRender
import KnottySession

/// The metrics every golden below is held to.
///
/// Injected rather than measured, and that is the whole of why a golden
/// travels between machines: the development machine and the CI runner are on
/// different macOS versions, and neither a font's advance nor its raster is
/// promised to be the same across them. Metrics are already an input to the
/// renderer, so pinning them costs nothing.
private let metrics = CellMetrics(width: 16, height: 34, fontPixelSize: 26)

private let goldensDirectory = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appending(path: "goldens")

/// The format the goldens are written in. Bump it when the encoding changes,
/// so a stale golden fails loudly rather than diffing line by line.
private let format = "knotty-render-golden 1"

/// The environment variable that turns a check into a rewrite. Its own, not
/// the harness's: updating a screen must not quietly update a drawing too.
private let updateVariable = "KNOTTY_UPDATE_RENDER_GOLDENS"

/// Everything the renderer decided, and nothing it was told.
///
/// The atlas coordinate is left out — where a glyph was packed is the
/// packer's answer, not a judgement about the screen — and the glyph is named
/// by the codepoint the frame says was baked there instead. What is left is a
/// rectangle and a colour for every cell, a rectangle for the cursor, and for
/// every glyph which one it is, where it sits and what tints it.
private func describe(_ frame: Frame, at metrics: CellMetrics) -> String {
    var out = "\(format)\n"
    out += "cell \(metrics.width) \(metrics.height)\n"

    let baked = Dictionary(
        uniqueKeysWithValues: frame.atlasUpdates.map { ([$0.x, $0.y], $0.codepoint) }
    )

    // The cursor is the last rectangle of the background pass, and it is
    // named so that it does not read as one more cell.
    let cells = frame.backgrounds.count - (frame.cursor == nil ? 0 : 1)
    out += "backgrounds \(cells)\n"
    for instance in frame.backgrounds.prefix(cells) {
        out += "background \(instance.x) \(instance.y) \(instance.width) \(instance.height)"
        out += " \(hex(instance.color))\n"
    }
    if let cursor = frame.cursor {
        out += "cursor \(cursor.x) \(cursor.y) \(cursor.width) \(cursor.height)"
        out += " \(hex(cursor.color))\n"
    } else {
        out += "cursor none\n"
    }

    out += "glyphs \(frame.glyphs.count)\n"
    for glyph in frame.glyphs {
        // A quad whose slot no update named would be one drawn from an atlas
        // this frame never filled, so it is written down rather than skipped.
        let codepoint = baked[[glyph.atlasX, glyph.atlasY]]
            .map { String(format: "%04X", $0) } ?? "unbaked"
        out += "glyph \(glyph.x) \(glyph.y) \(codepoint) \(hex(glyph.color))\n"
    }
    return out
}

private func hex(_ color: Rgb) -> String {
    String(format: "%02x%02x%02x", color.r, color.g, color.b)
}

/// Where two descriptions part company, in terms of the line rather than the
/// byte. Every line names its own place on the grid, so quoting the pair is
/// already the answer to what changed where.
private func difference(_ golden: String, _ produced: String) -> String? {
    guard golden != produced else { return nil }

    let expected = golden.split(separator: "\n", omittingEmptySubsequences: false)
    let actual = produced.split(separator: "\n", omittingEmptySubsequences: false)
    for (number, pair) in zip(expected, actual).enumerated() where pair.0 != pair.1 {
        return "line \(number + 1)\n  golden   \(pair.0)\n  produced \(pair.1)"
    }
    return "golden has \(expected.count) lines, produced has \(actual.count)"
}

/// Replay a recording and hold the frame it draws as against its golden.
private func check(_ name: String) throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(recording(name))
    let renderer = Renderer(metrics: metrics)

    let produced = try #require(try session.withSnapshot { describe(renderer.frame(for: $0), at: metrics) })
    let golden = goldensDirectory.appending(path: "\(name).golden")

    if ProcessInfo.processInfo.environment[updateVariable] != nil {
        try FileManager.default.createDirectory(
            at: goldensDirectory, withIntermediateDirectories: true
        )
        try produced.write(to: golden, atomically: true, encoding: .utf8)
        return
    }

    let expected = try String(contentsOf: golden, encoding: .utf8)
    if let report = difference(expected, produced) {
        Issue.record(
            """
            \(name): \(report)
            if this change is meant, rerun with \(updateVariable)=1
            """
        )
    }
}

/// The four ASCII recordings the Rust harness replays, and the one that is
/// not: `unicode` pins what this milestone does with what it cannot draw —
/// a wide character and an overflowed cluster leave their background and
/// nothing else. That is the boundary M3 moves.
///
/// What these cannot pin is the letter under a block cursor: no recording
/// ends with the cursor on a cell that has one. That judgement is held by
/// ``onlyABlockCursorTurnsTheLetterUnderItOver()`` instead.
@Test(arguments: ["vim", "tmux", "htop", "synthetic", "unicode"])
func aRecordingDrawsWhatItsGoldenSays(name: String) throws {
    try check(name)
}

/// A screen the same letter twice over pays for it once. The atlas is the
/// renderer's only state, and this is what it is for.
@Test func aGlyphAskedForTwiceIsRasteredOnce() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("aa\u{1b}[Hbaa".utf8))
    let renderer = Renderer(metrics: metrics)

    try session.withSnapshot { snapshot in
        let first = renderer.frame(for: snapshot)
        #expect(first.atlasUpdates.map(\.codepoint) == [0x62, 0x61])
        #expect(first.glyphs.count == 3)

        // Same screen, same renderer: everything it needed is already baked.
        #expect(renderer.frame(for: snapshot).atlasUpdates.isEmpty)
    }
}

/// The cell origin lands on a device pixel, which is what lets one raster
/// serve every occurrence of a glyph. The metrics are integers so that the
/// grid cannot be laid out any other way; this checks that measuring a real
/// font yields such metrics, and that the grid is laid out from them.
/// cf. 04-renderer R5.
@Test func cellOriginsSitOnDevicePixels() throws {
    let measured = CellMetrics.system(pointSize: 13, scale: 2)
    #expect(measured.width > 0)
    #expect(measured.height > 0)

    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(recording("vim"))
    let renderer = Renderer(metrics: measured)

    try session.withSnapshot { snapshot in
        let frame = renderer.frame(for: snapshot)
        #expect(frame.backgrounds.allSatisfy {
            $0.x % measured.width == 0 && $0.y % measured.height == 0
        })
        #expect(frame.glyphs.allSatisfy {
            $0.x % measured.width == 0 && $0.y % measured.height == 0
        })
    }
}

/// A session with one letter on screen and the cursor sitting on it.
private func sessionWithCursor(style: Int) throws -> Session {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("A\u{1b}[\(style) q\u{1b}[H".utf8))
    return session
}

/// A frame of the one-letter screen, with the cursor in the shape asked for.
private func frame(cursorStyle: Int) throws -> Frame {
    let session = try sessionWithCursor(style: cursorStyle)
    return try #require(try session.withSnapshot { Renderer(metrics: metrics).frame(for: $0) })
}

/// The three shapes are told apart by the rectangle and by nothing else.
/// cf. 04-renderer R1.
@Test func theThreeCursorShapesDifferInTheirRectangle() throws {
    func cursor(style: Int) throws -> BackgroundInstance {
        try #require(try frame(cursorStyle: style).backgrounds.last)
    }

    // DECSCUSR: 2 is a block, 4 an underline, 6 a bar.
    let block = try cursor(style: 2)
    #expect((block.x, block.y, block.width, block.height) == (0, 0, 16, 34))

    let underline = try cursor(style: 4)
    #expect((underline.x, underline.y, underline.width, underline.height) == (0, 32, 16, 2))

    let bar = try cursor(style: 6)
    #expect((bar.x, bar.y, bar.width, bar.height) == (0, 0, 2, 34))
}

/// A block covers the letter under it, so that letter is drawn in the colour
/// the cell would have been. A bar and an underline stand clear of it and
/// leave it alone. cf. 04-renderer R1.
@Test func onlyABlockCursorTurnsTheLetterUnderItOver() throws {
    func letter(style: Int) throws -> GlyphInstance {
        try #require(try frame(cursorStyle: style).glyphs.first)
    }

    // The screen is white on black, so the cell's background is black.
    #expect(try hex(letter(style: 2).color) == "000000")
    #expect(try hex(letter(style: 6).color) == "ffffff")
}

/// The cursor takes the colour the theme named, and the colour of the text it
/// stands on when the theme named none. cf. 04-renderer R1.
@Test func theCursorTakesTheThemesColourOrTheCellsForeground() throws {
    let session = try sessionWithCursor(style: 2)
    let asked = Rgb(r: 200, g: 100, b: 50)

    try session.withSnapshot { snapshot in
        let renderer = Renderer(metrics: metrics)

        let themed = renderer.frame(for: snapshot, cursorColor: asked).backgrounds.last
        #expect(themed.map(\.color).map(hex) == "c86432")

        let bare = renderer.frame(for: snapshot).backgrounds.last
        #expect(bare.map(\.color).map(hex) == "ffffff")
    }
}

/// The bake is real: coverage comes out, and it comes out the right way up.
///
/// Nothing else here would notice a rasterizer that drew nothing or a copy
/// that left the bitmap upside down — the goldens name glyphs rather than
/// their pixels, which is exactly what lets them travel between machines. So
/// the pixels are looked at once, here, for the two things about them that do
/// not depend on which machine drew them.
@Test func aBakedGlyphHasInkAndSitsOnItsBaseline() throws {
    let measured = CellMetrics.system(pointSize: 13, scale: 2)
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("M.".utf8))
    let renderer = Renderer(metrics: measured)

    let updates = try #require(try session.withSnapshot { renderer.frame(for: $0).atlasUpdates })
    let ink = Dictionary(uniqueKeysWithValues: updates.map { ($0.codepoint, $0.coverage) })

    #expect(try #require(ink[0x4D]).contains { $0 > 0 })

    // A full stop sits on the baseline, so all of its ink is in the lower
    // half of the cell — which holds only if the copy off the context turned
    // the bitmap over.
    let period = try #require(ink[0x2E])
    let half = period.count / 2
    #expect(period[..<half].allSatisfy { $0 == 0 })
    #expect(period[half...].contains { $0 > 0 })
}


/// Packing is looked at on its own, because the goldens deliberately do not
/// look at it: every glyph gets a slot of its own, every slot is on the page,
/// and the slots walk shelves — a row of them across, then down by a cell.
/// cf. 04-renderer R6.
@Test func everyGlyphGetsAPlaceOfItsOwnOnTheShelves() throws {
    // Every printable ASCII character, which is the whole of what M2 bakes.
    let printable = (0x21...0x7E).map { Character(Unicode.Scalar($0)!) }
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array(String(printable).utf8))
    let renderer = Renderer(metrics: metrics)

    let updates = try #require(try session.withSnapshot { renderer.frame(for: $0).atlasUpdates })
    #expect(updates.count == printable.count)

    let slots = Set(updates.map { [$0.x, $0.y] })
    #expect(slots.count == updates.count)
    #expect(updates.allSatisfy { $0.x >= 0 && $0.x + metrics.width <= 1024 })
    #expect(updates.allSatisfy { $0.y >= 0 && $0.y + metrics.height <= 1024 })

    // A shelf holds 1024 / 16 = 64 of these, so the sixty-fifth starts the
    // next one.
    #expect(updates.prefix(64).allSatisfy { $0.y == 0 })
    #expect((updates[0].x, updates[63].x) == (0, 63 * metrics.width))
    #expect((updates[64].x, updates[64].y) == (0, metrics.height))
}
