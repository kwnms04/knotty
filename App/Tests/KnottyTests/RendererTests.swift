import CoreText
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

/// The face every golden below is held to, pinned for the same reason the
/// metrics are: the ligature path a face takes is derived from that face's own
/// GSUB, so a machine with a ligature font installed would draw these
/// recordings by a different set of judgements than a runner without one. The
/// system's fixed-pitch face carries no ligature feature anywhere, which is
/// what makes it the one both agree on.
private func pinned() -> Faces { Faces(metrics: metrics, name: nil) }

private let goldensDirectory = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appending(path: "goldens")

/// The format the goldens are written in. Bump it when the encoding changes,
/// so a stale golden fails loudly rather than diffing line by line.
private let format = "knotty-render-golden 4"

/// The environment variable that turns a check into a rewrite. Its own, not
/// the harness's: updating a screen must not quietly update a drawing too.
private let updateVariable = "KNOTTY_UPDATE_RENDER_GOLDENS"

/// Everything the renderer decided, and nothing it was told.
///
/// A glyph is named by what the renderer shaped to arrive at it: the
/// codepoints of the run, which of that run's cells this one is, and the path
/// that chose it. Not by its glyph id, which moves with the font's version and
/// so would say one thing on a development machine and another on a runner —
/// the same problem the injected metrics answer. cf. 04-renderer R3.
///
/// A cell of the run whose cluster did not fit in one contributes every
/// codepoint of that cluster, resolved through the grapheme table: the cell
/// itself holds an index into it, and an index is the packer's answer rather
/// than a judgement about the screen. The trailing cell of a wide character
/// contributes nothing, because nothing was shaped for it. Which page a cluster was baked into is
/// left out for a different reason — it follows from whether the cascade
/// answered with a colour font, which is the system's property and not this
/// code's. ``aColorEmojiIsBakedOnItsOwnPage()`` holds that instead.
///
/// Left out for the same reason are the atlas coordinate, which is the
/// packer's answer rather than a judgement about the screen, and the quad's
/// width and offset, which are the font's ink and not this code's decision.
/// What is left is a rectangle and a colour for every cell, a rectangle for
/// the cursor, and for every glyph what it draws, where it sits and what tints
/// it.
private func describe(_ frame: Frame, at metrics: CellMetrics, of snapshot: Snapshot) -> String {
    var out = "\(format)\n"
    out += "cell \(metrics.width) \(metrics.height)\n"

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
        let row = Int(glyph.y / metrics.height)
        let first = Int(glyph.x / metrics.width - glyph.cellIndex)
        let run = (first..<(first + Int(glyph.cluster))).flatMap { column -> [String] in
            // The trailing cell of a wide character holds no text of its own,
            // so it contributes none: what a run is named by is the codepoints
            // that were shaped, and how many cells they were given is already
            // said beside it.
            let cell = snapshot.cells[row * Int(snapshot.cols) + column]
            guard !cell.isWideTail else { return [] }
            return snapshot.codepoints(of: cell).map { String(format: "%04X", $0) }
        }
        out += "glyph \(glyph.x) \(glyph.y) \(glyph.path.rawValue) \(glyph.style.name)"
        out += " \(glyph.cellIndex)/\(glyph.cluster) \(run.joined(separator: "+"))"
        out += " \(hex(glyph.color))\n"
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
    let renderer = Renderer(metrics: metrics, faces: pinned())

    let produced = try #require(
        try session.withSnapshot { describe(renderer.frame(for: $0), at: metrics, of: $0) }
    )
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
/// not: `unicode` pins what the slow path draws — Hangul over two cells from
/// a font the primary is not, emoji in colour, and a combining mark on the
/// base it belongs to rather than beside it.
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
    let renderer = Renderer(metrics: metrics, faces: pinned())

    try session.withSnapshot { snapshot in
        let first = renderer.frame(for: snapshot)
        // Two letters on the screen and three cells showing one.
        #expect(first.atlasUpdates.count == 2)
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
    let measured = CellMetrics.system(pointSize: 13, scale: 2, name: nil)
    #expect(measured.width > 0)
    #expect(measured.height > 0)

    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(recording("vim"))
    let renderer = Renderer(metrics: measured, faces: Faces(metrics: measured, name: nil))

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
    return try #require(try session.withSnapshot { Renderer(metrics: metrics, faces: pinned()).frame(for: $0) })
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
        let renderer = Renderer(metrics: metrics, faces: pinned())

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
    let measured = CellMetrics.system(pointSize: 13, scale: 2, name: nil)
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("M.".utf8))
    let renderer = Renderer(metrics: measured, faces: Faces(metrics: measured, name: nil))

    let frame = try #require(try session.withSnapshot { renderer.frame(for: $0) })
    // The page is what ties the two together: a quad names the slot it
    // samples, and a slot was filled by exactly one update.
    let coverage = Dictionary(
        uniqueKeysWithValues: frame.atlasUpdates.map { ([$0.x, $0.y], $0.pixels) }
    )
    let ink = try frame.glyphs.map { try #require(coverage[[$0.atlasX, $0.atlasY]]) }
    #expect(ink.count == 2)

    #expect(ink[0].contains { $0 > 0 })

    // A full stop sits on the baseline, so all of its ink is in the lower
    // half of the cell — which holds only if the copy off the context turned
    // the bitmap over.
    let half = ink[1].count / 2
    #expect(ink[1][..<half].allSatisfy { $0 == 0 })
    #expect(ink[1][half...].contains { $0 > 0 })
}


/// Packing is looked at on its own, because the goldens deliberately do not
/// look at it: every glyph gets a slot of its own, every slot is on the page,
/// and the slots walk shelves — a row of them across, then down by a cell.
/// A slot is as wide as the glyph's ink needs, so the walk is by that width
/// and not by a cell. cf. 04-renderer R6, adr/0016.
@Test func everyGlyphGetsAPlaceOfItsOwnOnTheShelves() throws {
    // Every printable ASCII character, and one past it: the range M2 baked,
    // and the boundary this ticket moves.
    let printable = (0x21...0x7E).map { Character(Unicode.Scalar($0)!) } + ["\u{276F}"]
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array(String(printable).utf8))
    let renderer = Renderer(metrics: metrics, faces: pinned())

    let updates = try #require(try session.withSnapshot { renderer.frame(for: $0).atlasUpdates })
    #expect(updates.count == printable.count)

    let slots = Set(updates.map { [$0.x, $0.y] })
    #expect(slots.count == updates.count)
    #expect(updates.allSatisfy { $0.x >= 0 && $0.x + $0.width <= 1024 })
    #expect(updates.allSatisfy { $0.y >= 0 && $0.y + metrics.height <= 1024 })
    #expect(updates.allSatisfy { $0.y % metrics.height == 0 })

    // The cursor walks: the next slot begins where the last one ended, or at
    // the left of the shelf below when it would have run off the page.
    #expect(updates[0].x == 0)
    for (last, next) in zip(updates, updates.dropFirst()) {
        if next.y == last.y {
            #expect(next.x == last.x + last.width)
        } else {
            #expect((next.x, next.y) == (0, last.y + metrics.height))
        }
    }
}

/// A codepoint outside ASCII is drawn rather than left blank. M2 baked
/// `0x21...0x7E` and nothing else, which is why the prompt's `❯` was a gap.
@Test func aCodepointOutsideAsciiIsDrawn() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("\u{276F} ".utf8))
    let renderer = Renderer(metrics: metrics, faces: pinned())

    let frame = try #require(try session.withSnapshot { renderer.frame(for: $0) })
    #expect(frame.glyphs.count == 1)
    #expect(frame.glyphs.first.map { ($0.x, $0.y) } ?? (-1, -1) == (0, 0))
    // The space beside it is not baked: it rasters to nothing, and a terminal
    // screen is mostly spaces.
    #expect(frame.atlasUpdates.count == 1)
}

/// The system's fixed-pitch face carries no ligature feature at all, so there
/// is nothing to derive and no participating set to hold. That is what the
/// derivation answering an empty set means — not that it failed.
/// cf. adr/0016.
@Test func aFaceWithoutLigatureFeaturesHasAnEmptyParticipatingSet() {
    let ligatures = Faces(metrics: metrics, name: nil)[.regular].ligatures
    #expect(ligatures.participating.isEmpty)
    #expect(!ligatures.enabled)
    // Nothing folded a cell, which is a different fact from there being
    // nothing that could.
    #expect(ligatures.preservesGrid)
    #expect((ligatures.input, ligatures.backtrack, ligatures.lookahead) == (1, 0, 0))
    #expect((ligatures.leftOverhang, ligatures.rightOverhang) == (0, 0))
}

/// A face that does not return one glyph per character of the probe has its
/// ligature path turned off. A font that folds cells does not draw a glyph
/// wrongly — it moves every column after it, and with them every coordinate
/// the terminal keeps. cf. adr/0016.
///
/// Geeza Pro folding lam-alef is the fold this guards against, in the only
/// form a machine with no such monospace font can be shown one: two
/// characters, one glyph, and the face's own — not the cascade's. A face that
/// did that to `!=` would move every column after it.
@Test func aProbeThatDoesNotKeepItsCellsTurnsTheLigaturePathOff() {
    let folding = FontFace(metrics: metrics, name: "Geeza Pro", probe: "\u{0644}\u{0627}")
    #expect(!folding.ligatures.preservesGrid)
    #expect(!folding.ligatures.enabled)

    // The same face and two letters it does not join: what the probe answers
    // is the fold and not the alphabet.
    let keeping = FontFace(metrics: metrics, name: "Geeza Pro", probe: "\u{062F}\u{062F}")
    #expect(keeping.ligatures.preservesGrid)
}

/// A face carrying one of the two features derives from it, on any machine:
/// Courier New ships with macOS and carries `liga` and not `calt`, so a set
/// it fills is `liga` being read. The other half of the pair is
/// ``aLigatureFaceDerivesItsSetWindowAndOverhang()``, whose face carries
/// `calt` and not `liga` — which is what "both, because neither alone is
/// enough" comes to here. cf. adr/0016.
///
/// What is checked is the shape of the answer and not its numbers: those are
/// the font's property, not this code's.
@Test func theDerivationTellsAFaceWithTheFeaturesFromAFaceWithout() {
    let with = Faces(metrics: metrics, name: "Courier New")[.regular].ligatures
    #expect(!with.participating.isEmpty)
    #expect(with.enabled)

    let without = Faces(metrics: metrics, name: nil)[.regular].ligatures
    #expect(without.participating.isEmpty)
    #expect(!without.enabled)
}

/// A face that has them derives all three of the numbers the ligature path
/// needs, from the one GSUB walk. The values are the font's and not this
/// code's, so what is held here is their shape: a set that is neither empty
/// nor everything, a window that is bounded, and ink that leaves the cell.
/// cf. adr/0016.
///
/// This face carries `calt` and not `liga`, so everything below is `calt`
/// being read and honoured — the half of "both features" that
/// ``theDerivationTellsAFaceWithTheFeaturesFromAFaceWithout()`` cannot show.
@Test func aLigatureFaceDerivesItsSetWindowAndOverhang() {
    let faces = ligatureFaces()
    let face = faces[.regular]
    let ligatures = face.ligatures
    #expect(ligatures.enabled)

    // The fast path is what the set being small keeps: every codepoint
    // outside it is an atlas lookup and nothing more.
    #expect(!ligatures.participating.isEmpty)
    #expect(face.participates(codepoint: UInt32(UInt8(ascii: "!"))))
    #expect(!face.participates(codepoint: UInt32(UInt8(ascii: "a"))))

    #expect(ligatures.backtrack > 0)
    #expect(ligatures.lookahead > 0)
    // A ligature is drawn by one of the cells it spans and reaches across the
    // others, whichever way that face points.
    #expect(max(ligatures.leftOverhang, ligatures.rightOverhang) > 1)
}

/// `!=` and `=>` draw as one mark across two cells, and the cells stay where
/// they were: two quads, one per cell, on the grid the metrics laid out.
/// That is the whole of why the ligature path can avoid the shaper for
/// everything else. cf. 04-renderer R3.
@Test(arguments: ["!=", "=>"])
func aLigatureIsDrawnAcrossItsCells(text: String) throws {
    let faces = ligatureFaces()
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("a \(text) b".utf8))
    let renderer = Renderer(metrics: metrics, faces: faces)

    let frame = try #require(try session.withSnapshot { renderer.frame(for: $0) })
    let ligature = frame.glyphs.filter { $0.path == .ligature }
    #expect(ligature.count == 2)
    #expect(ligature.map(\.cellIndex) == [0, 1])
    #expect(ligature.allSatisfy { $0.cluster == 2 })

    // Columns 2 and 3, where they would be without a ligature at all.
    #expect(ligature.map(\.x) == [2 * metrics.width, 3 * metrics.width])
    #expect(frame.glyphs.allSatisfy { $0.x % metrics.width == 0 })

    // The mark is wider than the cell that draws it, and reaches out of it.
    let drawn = try #require(ligature.first { $0.width > metrics.width })
    #expect(drawn.offsetX < 0 || drawn.offsetX + drawn.width > metrics.width)

    // And the ink that left the cell was baked rather than clipped: the slot
    // has coverage in the part of it that is not the cell it belongs to.
    let slot = try #require(
        frame.atlasUpdates.first { ($0.x, $0.y) == (drawn.atlasX, drawn.atlasY) }
    )
    let outside = (0..<Int(metrics.height)).flatMap { row -> [UInt8] in
        let start = row * Int(slot.width)
        let cell = start + Int(-drawn.offsetX)
        return Array(slot.pixels[start..<cell])
            + Array(slot.pixels[(cell + Int(metrics.width))..<(start + Int(slot.width))])
    }
    #expect(outside.contains { $0 > 0 })

    // The letters either side are not part of it.
    #expect(frame.glyphs.filter { $0.path == .fast }.count == 2)
}

/// A row that is nothing but participating cells is one run, and not several.
///
/// What ends a run is a cell that does not participate, and nothing else. A
/// row of rules has no cell to save — every one of them has to be shaped — so
/// cutting it into windows would buy a shaper call per piece and no work back.
/// cf. adr/0018.
@Test func aRowOfNothingButParticipatingCellsIsOneRun() throws {
    let faces = ligatureFaces()
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array(String(repeating: "=", count: Int(cols)).utf8))
    let renderer = Renderer(metrics: metrics, faces: faces)

    let frame = try #require(try session.withSnapshot { renderer.frame(for: $0) })
    let ligature = frame.glyphs.filter { $0.path == .ligature }
    #expect(ligature.count == Int(cols))
    #expect(ligature.allSatisfy { $0.cluster == Int32(cols) })
    #expect(Set(ligature.map(\.cellIndex)) == Set(0..<Int32(cols)))
}

/// The one face this milestone draws with, read from the copy committed
/// beside the goldens.
///
/// Registered for this process rather than installed on the machine. Nothing
/// on a bare macOS carries a ligature feature, so without a copy of its own
/// the ligature path is a thing no runner can see — and a runner that fetched
/// one would hold whatever release it fetched, which is the version drift
/// `adr/0016` names. The goldens are untouched either way: they pin the
/// system's fixed-pitch face.
///
/// A machine with the font already installed resolves the name to that copy
/// instead, since a duplicate cannot register. What is asserted below is the
/// shape of the answer rather than its numbers, which is what makes the two
/// agree anyway.
/// Registering a face that is registered already answers false and changes
/// nothing, so the three calls this file makes need nothing to hold them to
/// one.
private func ligatureFaces() -> Faces {
    CTFontManagerRegisterFontsForURL(
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appending(path: "fonts/JetBrainsMono-Regular.ttf") as CFURL,
        .process,
        nil
    )
    return Faces(metrics: metrics)
}

/// A wide character the primary face has no glyph for is drawn across the two
/// cells the engine gave it, by whatever font the cascade answered with.
///
/// Both halves of the slow path in one screen: the grid is the primary font's
/// and a fallback does not widen it, so the quad is the two cells and nothing
/// more — which is also what keeps it out of the cell beside it.
/// cf. 04-renderer R4.
@Test func aWideFallbackCharacterFillsTheTwoCellsItWasGiven() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("한A".utf8))
    let renderer = Renderer(metrics: metrics, faces: pinned())

    let frame = try #require(try session.withSnapshot { renderer.frame(for: $0) })
    // Two cells of Hangul and one of a letter, and the trailing cell of the
    // Hangul draws nothing of its own.
    #expect(frame.glyphs.count == 2)
    let hangul = try #require(frame.glyphs.first)
    #expect(hangul.path == .slow)
    #expect((hangul.cellIndex, hangul.cluster) == (0, 2))
    #expect((hangul.x, hangul.offsetX, hangul.width) == (0, 0, 2 * metrics.width))
    #expect(frame.glyphs.last?.x == 2 * metrics.width)

    // Fitted rather than dropped: the slot the cascade filled has ink in it.
    let slot = try #require(
        frame.atlasUpdates.first { ($0.x, $0.y) == (hangul.atlasX, hangul.atlasY) }
    )
    #expect(slot.width == 2 * metrics.width)
    #expect(slot.pixels.contains { $0 > 0 })
}

/// A colour emoji is baked on the page of its own that R6 asks for, and the
/// letters beside it stay on the coverage page. Mixing them would make every
/// letter on the screen cost four times what it does.
///
/// The pass stays one buffer either way: which page a quad samples rides on
/// the instance rather than splitting the list, which is what keeps a screen
/// with an emoji on it one draw call. cf. 04-renderer R1, R6.
@Test func aColorEmojiIsBakedOnItsOwnPage() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("A\u{1F600}B".utf8))
    let renderer = Renderer(metrics: metrics, faces: pinned())

    let frame = try #require(try session.withSnapshot { renderer.frame(for: $0) })
    #expect(frame.glyphs.map(\.page) == [.coverage, .color, .coverage])
    // Still one list, in the order the cells run.
    #expect(frame.glyphs.map(\.x) == [0, metrics.width, 3 * metrics.width])

    // Four bytes a pixel on the colour page and one on the other, which is
    // the whole of why they are two pages.
    for update in frame.atlasUpdates {
        #expect(
            update.pixels.count
                == Int(update.width) * Int(metrics.height) * update.page.bytesPerPixel
        )
    }
    let emoji = try #require(frame.atlasUpdates.first { $0.page == .color })
    #expect(emoji.pixels.contains { $0 > 0 })
    // Premultiplied, which is the shape the shader divides back out: no
    // channel can carry more than the alpha beside it.
    #expect(
        stride(from: 0, to: emoji.pixels.count, by: 4).allSatisfy { pixel in
            emoji.pixels[pixel..<(pixel + 3)].allSatisfy { $0 <= emoji.pixels[pixel + 3] }
        }
    )
}

/// A sequence of codepoints the terminal calls one grapheme is drawn as one
/// glyph, over the cells that one grapheme was given.
///
/// The terminal is what decides where a cluster ends, and it says so by
/// putting the whole run in one cell: mode 2027 is what makes a flag one cell
/// rather than two regional indicators, and knotty holds it on. The renderer's
/// half is to draw what arrived in a cell as one thing rather than as its
/// pieces. cf. 04-renderer R3.
@Test(arguments: [
    "\u{1F1F0}\u{1F1F7}",  // a flag: two regional indicators
    "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",  // a family: three emoji and two joiners
    "\u{1F44D}\u{1F3FD}",  // a thumb and the modifier that colours it
])
func aJoinedSequenceIsDrawnAsOneGlyph(text: String) throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array(text.utf8))
    let renderer = Renderer(metrics: metrics, faces: pinned())

    let frame = try #require(try session.withSnapshot { renderer.frame(for: $0) })
    #expect(frame.glyphs.count == 1)
    let joined = try #require(frame.glyphs.first)
    #expect(joined.path == .slow)
    #expect((joined.cellIndex, joined.cluster) == (0, 2))
    #expect(joined.page == .color)
    #expect((joined.x, joined.offsetX, joined.width) == (0, 0, 2 * metrics.width))
}

/// A combining mark is drawn on the base it belongs to, which is one glyph
/// over one cell — not two glyphs stacked on the same origin.
///
/// The mark and its base cross as one cluster, so Core Text places the mark
/// and the atlas keeps the pair. What says they are not overlaid is that they
/// are not two: the cell asks for one quad, and what it holds is not what the
/// bare base holds.
@Test func aCombiningMarkIsDrawnOnItsBaseRatherThanOverIt() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("e\u{0301}".utf8))
    let renderer = Renderer(metrics: metrics, faces: pinned())

    let frame = try #require(try session.withSnapshot { renderer.frame(for: $0) })
    #expect(frame.glyphs.count == 1)
    let accented = try #require(frame.glyphs.first)
    #expect(accented.path == .slow)
    #expect((accented.cellIndex, accented.cluster) == (0, 1))
    #expect(accented.width == metrics.width)

    // The bare letter, on a renderer of its own so that the page is empty
    // before it is asked. The mark sits above the base, so the cluster's ink
    // starts higher than the letter's own — a comparison rather than a row
    // number, since where a face puts its ink is the face's business.
    let alone = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try alone.feed(Array("e".utf8))
    let baked = try #require(
        try alone.withSnapshot {
            Renderer(metrics: metrics, faces: pinned()).frame(for: $0).atlasUpdates
        }
    )
    let bare = try #require(baked.first)
    let cluster = try #require(
        frame.atlasUpdates.first { ($0.x, $0.y) == (accented.atlasX, accented.atlasY) }
    )
    #expect(cluster.pixels != bare.pixels)
    #expect(highestInk(of: cluster) < highestInk(of: bare))
}

/// The topmost row of a coverage slot that has any ink in it, counted from the
/// top of the cell — which is how the rows are stored.
private func highestInk(of update: AtlasUpdate) -> Int {
    let width = Int(update.width)
    return (0..<(update.pixels.count / width)).first { row in
        update.pixels[(row * width)..<((row + 1) * width)].contains { $0 > 0 }
    } ?? Int.max
}

/// The cascade is asked once per cluster, however often the cluster is on the
/// screen. The atlas keys a slow-path slot by the cluster for this reason: a
/// fallback lookup and a shaping call are the same call, and neither is worth
/// making twice. cf. 04-renderer R4.
@Test func aClusterAskedForTwiceIsShapedOnce() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("한한글".utf8))
    let renderer = Renderer(metrics: metrics, faces: pinned())

    try session.withSnapshot { snapshot in
        let first = renderer.frame(for: snapshot)
        // Three syllables on the screen and two of them the same.
        #expect(first.glyphs.count == 3)
        #expect(first.atlasUpdates.count == 2)

        // Same screen, same renderer: everything it needed is already baked.
        #expect(renderer.frame(for: snapshot).atlasUpdates.isEmpty)
    }
}

/// The four attribute combinations draw in the four faces, and each is a
/// raster of its own on the pages.
///
/// This is the axis M2 deferred. A bold cell drawn in the regular face and
/// coloured to match is what daily use did not survive: a prompt and a search
/// highlight are mostly bold. cf. 04-renderer R3.
@Test func theFourAttributeCombinationsDrawInTheirOwnFaces() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    // SGR 1 is bold and SGR 3 italic; 0 puts both down again.
    try session.feed(Array("A\u{1b}[1mA\u{1b}[0;3mA\u{1b}[1;3mA".utf8))
    let renderer = Renderer(metrics: metrics, faces: pinned())

    let frame = try #require(try session.withSnapshot { renderer.frame(for: $0) })
    #expect(frame.glyphs.map(\.style) == [.regular, .bold, .italic, .boldItalic])

    // One letter, four slots: the atlas key has a style axis, so the same
    // codepoint in another face is another entry rather than the same one.
    #expect(frame.atlasUpdates.count == 4)
    #expect(Set(frame.glyphs.map { [$0.atlasX, $0.atlasY] }).count == 4)
}

/// And the four rasters are four different pictures of the letter — which is
/// what "bold looks bold" comes to once the face has been chosen.
///
/// Bold carries more ink than regular, which holds whether the family shipped
/// a bold face or the system emboldened one for it. Italic is held to being
/// different rather than to leaning a measured amount: how far a face slants
/// is the face's business and not this code's. cf. adr/0016.
@Test func aBoldCellIsHeavierThanTheSameLetterInRegular() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("A\u{1b}[1mA\u{1b}[0;3mA".utf8))
    let renderer = Renderer(metrics: metrics, faces: pinned())

    let frame = try #require(try session.withSnapshot { renderer.frame(for: $0) })
    let baked = Dictionary(
        uniqueKeysWithValues: frame.atlasUpdates.map { ([$0.x, $0.y], $0.pixels) }
    )
    let ink = try frame.glyphs.map { try #require(baked[[$0.atlasX, $0.atlasY]]) }
    #expect(ink.count == 3)

    let coverage = ink.map { $0.reduce(0) { $0 + Int($1) } }
    #expect(coverage[1] > coverage[0])
    #expect(ink[2] != ink[0])
}

/// Each face is the face its style names, where the family has one.
///
/// A family that shipped three faces answers the fourth with what it has, and
/// drawing those cells in the regular is what is left — so this asks what came
/// back rather than assuming the ask was met.
@Test func eachFaceCarriesTheTraitsItsStyleNames() {
    let faces = Faces(metrics: metrics, name: nil)
    #expect(!faces[.regular].traits.contains(.traitBold))
    #expect(!faces[.regular].traits.contains(.traitItalic))
    #expect(faces[.bold].traits.contains(.traitBold))
    #expect(faces[.italic].traits.contains(.traitItalic))
    #expect(faces[.boldItalic].traits.isSuperset(of: [.traitBold, .traitItalic]))
}

/// The GSUB walk runs once per face, and the four answers are the four faces'
/// own rather than the regular's copied across.
///
/// Courier New is the family that shows it on a bare macOS: its roman faces
/// carry a substitution table and its italics do not, so a derivation shared
/// from the regular would give the italic a participating set it has no rules
/// for. That is the Cascadia Italic case — a set of 34 where its own Regular's
/// is 85 — in the form every runner has. The numbers themselves are the font's
/// property and are not asserted. cf. adr/0016.
@Test func everyFaceDerivesItsOwnLigaturesRatherThanTheRegularsAgain() {
    let faces = Faces(metrics: metrics, name: "Courier New")

    // Four faces and not one asked four times.
    let names = FontStyle.allCases.map { faces[$0].postScriptName }
    #expect(Set(names).count == 4)

    #expect(faces[.regular].ligatures.participating != faces[.italic].ligatures.participating)
}

/// A run is shaped by one face, so a style boundary ends it.
///
/// Two faces have two sets of rules and two numbering of glyphs; a window
/// spanning both would let one face's rules decide on the other's characters.
/// cf. 04-renderer R3.
@Test func aRunStopsWhereTheFaceChanges() throws {
    let faces = ligatureFaces()
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    // `!=` with the `=` in bold: two participating cells, two faces.
    try session.feed(Array("!\u{1b}[1m=".utf8))
    let renderer = Renderer(metrics: metrics, faces: faces)

    let frame = try #require(try session.withSnapshot { renderer.frame(for: $0) })
    #expect(frame.glyphs.map(\.style) == [.regular, .bold])
    // Each cell is a run of its own, so neither joined the other.
    #expect(frame.glyphs.allSatisfy { $0.cluster == 1 })
}

/// A screen of rows the renderer has already placed is placed again for free.
///
/// The first frame of a fresh screen is two rows and not twenty-four: every
/// blank row holds what every other blank row holds, and the cache is keyed on
/// that rather than on where the row sits. cf. 04-renderer R2.
@Test func aRowThatDidNotChangeIsNotPlacedAgain() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    let renderer = Renderer(metrics: metrics, faces: pinned())

    try session.feed(Array("hello".utf8))
    try session.withSnapshot { _ = renderer.frame(for: $0) }
    // One row of letters and one blank row, asked for twenty-four times.
    #expect(renderer.lineCache.misses == 2)
    #expect(renderer.lineCache.hits == Int(rows) - 2)

    // One row changes. The other twenty-three are the rows they were, and the
    // shaping that placed them is not done again.
    try session.feed(Array("\u{1b}[2;1Hworld".utf8))
    try session.withSnapshot { _ = renderer.frame(for: $0) }
    #expect(renderer.lineCache.misses == 3)
    #expect(renderer.lineCache.hits == Int(rows) - 2 + Int(rows) - 1)
}

/// A row that scrolled is the row it was. The cache is keyed on what the row
/// holds and not on where it sits, so moving every line up by one costs the
/// one line that is new. cf. 04-renderer R2.
@Test func aRowThatScrolledIsFoundByWhatItHolds() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    let renderer = Renderer(metrics: metrics, faces: pinned())

    // A full screen of rows that are all different, so that nothing below can
    // hit by being blank.
    let screen = (0..<Int(rows)).map { "row \($0)" }.joined(separator: "\r\n")
    try session.feed(Array(screen.utf8))
    try session.withSnapshot { _ = renderer.frame(for: $0) }
    let before = renderer.lineCache
    #expect(before.misses == Int(rows))
    #expect(before.hits == 0)

    // One more line scrolls the screen: every row is at a new coordinate and
    // only the last of them holds anything new.
    try session.feed(Array("\r\nrow \(rows)".utf8))
    try session.withSnapshot { _ = renderer.frame(for: $0) }
    #expect(renderer.lineCache.misses == before.misses + 1)
    #expect(renderer.lineCache.hits == Int(rows) - 1)
}

/// Selecting text does not empty the cache.
///
/// This is what the boundary carries a selection on the row for rather than in
/// the cell: a drag is a new selection every mouse move, and a selection the
/// key could see would throw away every row of the screen on each one.
/// cf. 02-ffi, 04-renderer R2.
@Test func aSelectionDoesNotEmptyTheLineCache() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    let renderer = Renderer(metrics: metrics, faces: pinned())

    let screen = (0..<Int(rows)).map { "row \($0)" }.joined(separator: "\r\n")
    try session.feed(Array(screen.utf8))
    try session.withSnapshot { _ = renderer.frame(for: $0) }
    let before = renderer.lineCache

    try session.setSelection(
        SelectionRange(start_x: 0, start_y: 0, end_x: 4, end_y: 2, rectangle: false)
    )
    let selected = try #require(
        try session.withSnapshot { snapshot -> Bool in
            _ = renderer.frame(for: snapshot)
            return snapshot.rowStates.contains { $0.isSelected }
        }
    )

    // The selection really arrived, and not one row was placed again for it.
    #expect(selected)
    #expect(renderer.lineCache.misses == before.misses)
    #expect(renderer.lineCache.hits == before.hits + Int(rows))
}

/// The hit rate B3 is measured against is read off the counter, not counted by
/// the caller. Measuring it is M5's; carrying it is M3's. cf. 04-renderer R3.
@Test func theHitRateIsReadableAndStartsAtOne() throws {
    let renderer = Renderer(metrics: metrics, faces: pinned())
    #expect((renderer.lineCache.hits, renderer.lineCache.misses) == (0, 0))
    #expect(renderer.lineCache.hitRate == 1)

    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("hello".utf8))
    try session.withSnapshot { _ = renderer.frame(for: $0) }
    #expect(renderer.lineCache.hitRate == Double(Int(rows) - 2) / Double(rows))
}
