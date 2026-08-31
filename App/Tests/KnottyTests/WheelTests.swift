import Testing

import KnottySession

/// A cell of the size the tests measure against: twenty points tall, ten wide.
private let cell = (width: 10.0, height: 20.0)

/// The half of the wheel that is the app's. What a flick costs is the number
/// of times the core is called for it, so a fraction of a line has to wait
/// here rather than round to nothing or to one.
@Test func aTrackpadCallsForALineOnlyOnceItHasScrolledOne() {
    var wheel = WheelLines()

    // A quarter of a line at a time. The first three are held, and the core
    // is not called at all for them.
    for _ in 0..<3 {
        #expect(
            wheel.lines(deltaX: 0, deltaY: 5, precise: true, cellSize: cell) == (x: 0, y: 0)
        )
    }
    // The fourth completes the line, and is the one event of the four that
    // reaches the core.
    #expect(wheel.lines(deltaX: 0, deltaY: 5, precise: true, cellSize: cell) == (x: 0, y: 1))
}

/// The remainder is kept rather than dropped, or a slow scroll would lose a
/// fraction of a line every time it crossed one.
@Test func whatIsLeftOverAfterALineIsKeptForTheNextOne() {
    var wheel = WheelLines()

    // Two and a half lines: two now, and half of one waiting.
    #expect(wheel.lines(deltaX: 0, deltaY: 50, precise: true, cellSize: cell) == (x: 0, y: 2))
    // Half a line more finishes the third.
    #expect(wheel.lines(deltaX: 0, deltaY: 10, precise: true, cellSize: cell) == (x: 0, y: 1))
}

/// Sideways is counted against the cell's width, and the two axes hold their
/// remainders apart — a horizontal flick must not push a vertical one over.
@Test func theTwoAxesAreCountedApart() {
    var wheel = WheelLines()

    #expect(wheel.lines(deltaX: 9, deltaY: 19, precise: true, cellSize: cell) == (x: 0, y: 0))
    #expect(wheel.lines(deltaX: 1, deltaY: 0, precise: true, cellSize: cell) == (x: 1, y: 0))
    #expect(wheel.lines(deltaX: 0, deltaY: 1, precise: true, cellSize: cell) == (x: 0, y: 1))
}

/// A wheel with detents reports lines already, and macOS reports the slowest
/// turn of one as a tenth of a line — so truncating is what would make a slow
/// wheel do nothing at all.
@Test func aWheelWithDetentsTurnsALineEvenWhenItReportsAFractionOfOne() {
    var wheel = WheelLines()

    #expect(wheel.lines(deltaX: 0, deltaY: 0.1, precise: false, cellSize: cell) == (x: 0, y: 1))
    #expect(wheel.lines(deltaX: 0, deltaY: -0.1, precise: false, cellSize: cell) == (x: 0, y: -1))
    #expect(wheel.lines(deltaX: 0, deltaY: 3, precise: false, cellSize: cell) == (x: 0, y: 3))
    #expect(wheel.lines(deltaX: 0, deltaY: 0, precise: false, cellSize: cell) == (x: 0, y: 0))
}

/// A view mid-layout has no cell size yet, and dividing by it would answer
/// with infinity — which is not a number of lines.
@Test func aCellOfNoSizeScrollsNothing() {
    var wheel = WheelLines()

    #expect(
        wheel.lines(deltaX: 100, deltaY: 100, precise: true, cellSize: (width: 0, height: 0))
            == (x: 0, y: 0)
    )
}
