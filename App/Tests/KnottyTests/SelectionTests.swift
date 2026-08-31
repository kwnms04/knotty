import Testing

import KnottySession

/// The half of a selection gesture that is the app's: a click count is what
/// says which unit the gesture measures in, and it is the only judgement about
/// a selection made on this side. Where a word or a line ends is the engine's,
/// and the harness goldens are what hold that. cf. adr/0017.
@Test func aClickCountSaysWhichUnitAGestureMeasuresIn() {
    #expect(SelectionUnit(clickCount: 1) == .cell)
    #expect(SelectionUnit(clickCount: 2) == .word)
    #expect(SelectionUnit(clickCount: 3) == .line)
    // Past three AppKit keeps counting, and a fourth click starts the round
    // again — which is what every other application on the platform does.
    #expect(SelectionUnit(clickCount: 4) == .cell)
    #expect(SelectionUnit(clickCount: 5) == .word)
    // A drag with no click behind it is one cell to the next.
    #expect(SelectionUnit(clickCount: 0) == .cell)
}

/// The other half that is the app's: which way a drag held outside the window
/// asks the screen to keep coming. Up is positive, as everywhere else the
/// boundary counts lines, and a pointer inside the window asks for nothing.
@Test func aDragOutsideTheWindowSaysWhichWayTheScreenComes() {
    // A view forty points tall: the top edge is 40 because a view counts from
    // the bottom, and the row under the pointer is what the drag is over.
    #expect(Autoscroll.lines(pointerY: 41, viewHeight: 40) == 1)
    #expect(Autoscroll.lines(pointerY: -1, viewHeight: 40) == -1)
    // Inside, and on either edge, which is still inside.
    #expect(Autoscroll.lines(pointerY: 20, viewHeight: 40) == 0)
    #expect(Autoscroll.lines(pointerY: 0, viewHeight: 40) == 0)
    #expect(Autoscroll.lines(pointerY: 40, viewHeight: 40) == 0)
}

/// What ⌘C puts on the pasteboard, taken across the boundary and decoded.
@Test func aGesturedWordComesBackAsTheTextItCovers() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("one two three".utf8))

    try session.select(anchor: (x: 5, y: 0), to: (x: 5, y: 0), unit: .word)

    #expect(try session.copySelection() == "two")
}

/// Nothing selected is an answer and not a failure, and the one a ⌘C with no
/// selection behind it gets: the pasteboard is left holding what it held.
@Test func copyingWithNothingSelectedAnswersWithNothing() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("one two three".utf8))

    #expect(try session.copySelection() == nil)
}

/// The selection is the engine's to keep track of, so output that pushes it
/// out of the viewport leaves it over the same text. Nothing on this side
/// holds a coordinate that could have gone stale with it.
@Test func aSelectionPushedIntoTheScrollbackStillCopiesItsOwnText() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("keep me\r\n".utf8))
    try session.select(anchor: (x: 0, y: 0), to: (x: 6, y: 0), unit: .cell)

    // More lines than the viewport is tall, so what was selected is well back
    // in the history by the end of it.
    let flood = (0..<Int(rows) + 5).map { "line \($0)" }.joined(separator: "\r\n")
    try session.feed(Array(flood.utf8))

    #expect(try session.copySelection() == "keep me")
}

/// A drag inside a program that asked to hear about the mouse is that
/// program's, and the terminal selects nothing for it — the branch the core
/// makes rather than the app, since the mode arrives as output. cf. adr/0017.
@Test func aGestureSelectsNothingWhileTheChildIsHearingAboutTheMouse() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("one two three\u{1b}[?1000h".utf8))

    try session.select(anchor: (x: 0, y: 0), to: (x: 6, y: 0), unit: .cell)

    #expect(try session.copySelection() == nil)
}

/// What the autoscroll timer calls while a drag is held outside the window.
/// The core moves the viewport and publishes, so the app keeps no scroll
/// position of its own — the next frame is already scrolled.
@Test func scrollingTheViewportWalksBackIntoTheScrollback() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    let lines = (0..<Int(rows) + 3).map { "line \($0)" }.joined(separator: "\r\n")
    try session.feed(Array(lines.utf8))
    // Take the frame the output published, so that what the next take answers
    // is the scroll's own.
    _ = try session.withSnapshot { $0.cols }

    try session.scrollViewport(lines: 3)

    let top = try #require(
        try session.withSnapshot { snapshot -> String in
            let cells = (0..<Int(snapshot.cols)).compactMap { snapshot.text(of: snapshot.cells[$0]) }
            return cells.joined()
        }
    )
    #expect(top == "line0")
}
