import Testing

import KnottySession

@testable import KnottyRender

/// A page 96 device pixels square holds twelve slots at the metrics the
/// goldens are pinned to — six to a shelf at a cell 16 wide, two shelves at a
/// cell 34 tall. Small enough to fill in a test, which is the whole of why
/// the side is injectable: filling a page the size the app packs into takes a
/// year of a screen's letters. cf. 04-renderer R7.
private let smallPage: Int32 = 96

/// A full page is emptied and built again, and the letters keep coming.
///
/// What must not happen is that frame going out half-baked: every glyph it
/// draws has to come from a slot the same frame filled, because the slots the
/// pass that ran the page out chose belong to a page that no longer exists.
/// cf. 04-renderer R7, R8.
@Test func aFullAtlasIsEmptiedAndTheLettersKeepDrawing() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    let renderer = Renderer(metrics: metrics, faces: pinned(), atlasSide: smallPage)

    try session.feed(Array("abcdefgh".utf8))
    try session.withSnapshot { snapshot in
        #expect(renderer.frame(for: snapshot).atlasUpdates.count == 8)
    }

    // Eight letters none of the first eight were, onto a page with room for
    // four. The thirteenth is where it runs out.
    try session.feed(Array("\u{1b}[2J\u{1b}[Hijkl\r\nmnop".utf8))
    let placements = renderer.lineCache.misses
    try session.withSnapshot { snapshot in
        let reset = renderer.frame(for: snapshot)

        #expect(reset.glyphs.count == 8, "a page that filled up stopped answering")
        #expect(reset.atlasUpdates.count == 8, "the reset frame did not bake the screen again")
        #expect(
            reset.atlasUpdates.contains { $0.x == 0 && $0.y == 0 },
            "the shelves did not start over"
        )
        let baked = Set(reset.atlasUpdates.map { [$0.x, $0.y] })
        #expect(
            reset.glyphs.allSatisfy { baked.contains([$0.atlasX, $0.atlasY]) },
            "a glyph was drawn out of a slot the same frame had already written over"
        )
        // Three rows as this screen reads them — the two with letters on
        // them and the blank one the other twenty-two are — and every one of
        // them placed again, which is the shaping cache having gone with the
        // atlas. The pass that was thrown away is not in here with them.
        #expect(
            renderer.lineCache.misses == placements + 3,
            "the shaping cache outlived the atlas it was emptied with"
        )

        // And what it rebuilt is a page that answers: the same screen again
        // asks nothing of the rasterizer.
        let again = renderer.frame(for: snapshot)
        #expect(again.glyphs.count == 8)
        #expect(again.atlasUpdates.isEmpty)
    }
}

/// A screen holding more letters than a page has slots is drawn as far as the
/// page goes, and draws the same letters every frame.
///
/// The reset is once a frame, so a screen that was never going to fit cannot
/// take the renderer round again inside one. The cells past the end of the
/// page keep their background, and the ones before it draw what they drew
/// last frame — a screen that did not change is placed in the order it was
/// placed in before.
@Test func aScreenLargerThanThePageDrawsAsFarAsThePageGoes() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    let renderer = Renderer(metrics: metrics, faces: pinned(), atlasSide: smallPage)

    // Sixteen letters against twelve slots.
    try session.feed(Array("abcdefghijklmnop".utf8))
    try session.withSnapshot { snapshot in
        let frame = renderer.frame(for: snapshot)
        #expect(frame.glyphs.count == 12, "the page answered for more cells than it has slots")

        let next = renderer.frame(for: snapshot)
        #expect(
            next.glyphs.map(\.x) == frame.glyphs.map(\.x),
            "the cells that drew a letter moved between two frames of one screen"
        )
        let baked = Set(next.atlasUpdates.map { [$0.x, $0.y] })
        #expect(next.glyphs.allSatisfy { baked.contains([$0.atlasX, $0.atlasY]) })
    }
}
