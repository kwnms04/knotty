import Foundation
import Testing

import KnottyRender
import KnottySession

/// The metrics the drawing below is held to, pinned the way the renderer
/// goldens' are and for the same reason: a rectangle measured against a
/// machine's own font says one thing here and another on a runner.
private let linkMetrics = CellMetrics(width: 16, height: 34, fontPixelSize: 26)

/// One screen, fed and scanned.
private func scanned(_ text: String) throws -> [Link] {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array(text.utf8))
    return try #require(try session.withSnapshot { Link.scan($0) })
}

/// **`http://` and `https://` and nothing else.** A terminal screen is full of
/// dotted tokens, and a scan that took a scheme-less domain would underline
/// half of one — which is noise where the point was knowing what is about to
/// be clicked. cf. adr/0006.
@Test func nothingWithoutASchemeIsALink() throws {
    let links = try scanned("Cargo.toml main.rs v1.2.3 example.com 192.168.0.1:8080")

    #expect(links.isEmpty)
}

/// A scheme has to start a token. Otherwise the rule that keeps `Cargo.toml`
/// off the screen lets `shttp://example.com` on it, which is the same false
/// positive wearing a scheme.
@Test func aSchemeInTheMiddleOfAWordIsNotALink() throws {
    let links = try scanned("shttp://example.com and xhttps://example.com")

    #expect(links.isEmpty)
}

/// The address, and the cells it is drawn over — which is what the underline
/// is put on and what the click is answered from.
@Test func aURLIsFoundWithTheCellsItCovers() throws {
    let links = try scanned("see https://example.com/x here")

    #expect(links.count == 1)
    let link = try #require(links.first)
    #expect(link.url == URL(string: "https://example.com/x"))
    // "see " is four cells, and the address is twenty-one.
    #expect(link.runs == [Link.Run(row: 0, columns: 4..<25)])
    #expect(link.covers(row: 0, column: 4))
    #expect(link.covers(row: 0, column: 24))
    #expect(!link.covers(row: 0, column: 25))
    #expect(!link.covers(row: 1, column: 4))
}

/// A line that ended by running out of columns runs on into the next one, and
/// the address that crossed the two is one address. `KT_ROW_FLAG_WRAPPED` is
/// what says so, and joining the rows before the scan is what keeps a long
/// URL from being read as two halves that neither of them parses.
@Test func aURLThatWrappedIsOneLink() throws {
    let padding = String(repeating: "-", count: 70)
    let address = "https://example.com/abcdefghij"

    let links = try scanned("\(padding) \(address)")

    #expect(links.count == 1)
    let link = try #require(links.first)
    #expect(link.url == URL(string: address))
    // The padding and its space take 71 cells, so nine of the address are on
    // the first row and the remaining twenty-one are on the second.
    #expect(link.runs == [Link.Run(row: 0, columns: 71..<80), Link.Run(row: 1, columns: 0..<21)])
}

/// Trailing punctuation belongs to the sentence rather than to the address.
/// Every one of these is legal in a URL, so this is a guess — and the guess
/// that a full stop ended a sentence is right far more often than the one
/// that it ended a path.
@Test func punctuationAfterAURLIsNotPartOfIt() throws {
    let links = try scanned("see https://example.com/a, or https://example.com/b.")

    #expect(
        links.map(\.url) == [
            URL(string: "https://example.com/a"), URL(string: "https://example.com/b"),
        ]
    )
}

/// The underline a link is drawn with is the one 05 built for a cell's own:
/// the same rectangle, at the same weight, in the same pass. A decoration is
/// a rectangle whoever asked for it. cf. 04-renderer R1.
@Test func aLinkIsDrawnInTheUnderlinePass() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("see https://example.com/x here".utf8))
    let renderer = Renderer(metrics: linkMetrics)

    let (without, with) = try #require(
        try session.withSnapshot { snapshot -> ([BackgroundInstance], [BackgroundInstance]) in
            let links = Link.scan(snapshot)
            return (
                renderer.frame(for: snapshot).underlines,
                renderer.frame(for: snapshot, links: links).underlines
            )
        }
    )

    // Nothing on this screen carries an underline of its own, so the ⌘ is the
    // whole difference between the two frames.
    #expect(without.isEmpty)
    #expect(with.count == 1)
    let line = try #require(with.first)
    #expect(line.x == 4 * linkMetrics.width)
    #expect(line.width == 21 * linkMetrics.width)
    // At the foot of the row, at the weight every decoration is drawn at.
    #expect(line.height == max(1, linkMetrics.width / 8))
    #expect(line.y == linkMetrics.height - line.height)
}
