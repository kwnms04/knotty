import Darwin
import Foundation
import Testing

import KnottySession

/// The size the recordings were made at, and the scrollback the Rust harness
/// replays them with. Replaying at another size would show a screen no
/// application ever drew.
private let cols: UInt16 = 80
private let rows: UInt16 = 24
private let scrollback = 1000

/// A recording, read from where it lives rather than copied in beside these
/// tests. The point of a detached session being public is that both layers
/// look at the same stream. cf. adr/0008.
private func recording(_ name: String) throws -> [UInt8] {
    // This file sits four levels under the repository root:
    // App/Tests/KnottyTests/SessionTests.swift.
    let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
    let file = root.appending(path: "crates/knotty-harness/recordings/\(name).vt")
    return try [UInt8](Data(contentsOf: file))
}

/// One round of everything a consumer does with a session.
private func replaySynthetic() throws -> Session {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(recording("synthetic"))
    return session
}

/// A row read back as text, with a cell holding nothing read as a space.
private func text(of snapshot: Snapshot, row: Int) -> String {
    let start = row * Int(snapshot.cols)
    let characters = (0..<Int(snapshot.cols)).map { column -> Character in
        let codepoint = snapshot.cells[start + column].codepoint
        guard codepoint != 0, let scalar = Unicode.Scalar(codepoint) else { return " " }
        return Character(scalar)
    }
    return String(characters)
}

/// The whole of the path M2 rests on: bytes in, a screen out, and the screen
/// says what the Rust golden says it should.
@Test func aRecordingFedToASessionComesBackAsAScreen() throws {
    let session = try replaySynthetic()

    let drawn = try session.withSnapshot { snapshot in
        #expect(snapshot.cols == cols)
        #expect(snapshot.rows == rows)
        // By the end of the recording the screen has scrolled: what stands on
        // the top row is the eighth line of padding.
        #expect(text(of: snapshot, row: 0).hasPrefix("pad 07 ---"))
        #expect(snapshot.cursor.x == 0)
        #expect(snapshot.cursor.y == 23)
        #expect(snapshot.cursor.visible)
        return true
    }

    #expect(drawn == true)
}

/// Anything that outlives the frame is copied out of it, which is what the
/// borrowed pointers leave a consumer no choice about.
@Test func aTitleKeptPastTheFrameIsACopy() throws {
    let session = try replaySynthetic()

    let title = try session.withSnapshot { String(decoding: $0.title, as: UTF8.self) }

    #expect(title == "knotty synthetic")
}

/// The two states are read off the frame that was taken, not asked of the
/// session afterwards.
@Test func theFrameSaysWhatTheChildAndTheSessionAre() throws {
    let session = try replaySynthetic()

    let states = try #require(try session.withSnapshot { ($0.childState, $0.sessionState) })

    // Nothing stands behind a detached session, which is a different fact
    // from the session itself being well.
    #expect(states.0 == ChildState.none)
    #expect(states.1 == .ok)
}

/// A session that has published nothing hands back nothing, and says so
/// without it being a failure.
@Test func aSessionWithNothingPublishedHandsBackNothing() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)

    #expect(try session.withSnapshot { _ in true } == nil)
}

/// The queue empties and says what overflowed it, which is all M2 owes it.
@Test func theEventQueueEmptiesAndSaysWhatItDropped() throws {
    let session = try replaySynthetic()

    // The recording rings the bell and copies to the clipboard, in that
    // order, and nothing else it does is an event.
    let drained = try session.drainEvents()
    #expect(drained.taken == 2)
    #expect(drained.dropped == 0)

    #expect(try session.drainEvents().taken == 0)
}

/// Sessions come and go without leaving anything behind.
///
/// Nothing a consumer can see says a handle leaked, so what is watched is the
/// allocator. A leaked session or a leaked frame is tens of kilobytes, and a
/// hundred of either overshoots this bound by an order of magnitude — which
/// is what keeps a bound this loose from being a bound that never fails. It
/// has to be loose: the tests run alongside each other in one process, so
/// what the others are holding lands in the same measurement.
@Test func sessionsComeAndGoWithoutGrowingTheHeap() throws {
    let bytes = try recording("synthetic")
    func round() throws {
        let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
        try session.feed(bytes)
        try session.drainEvents()
        _ = try session.withSnapshot { $0.cols }
    }

    // The first round pays for whatever the first session sets up once.
    try round()
    let before = Int(mstats().bytes_used)
    for _ in 0..<100 {
        try round()
    }
    let grown = Int(mstats().bytes_used) - before

    #expect(grown < 1 << 20, "the heap grew by \(grown) bytes over 100 sessions")
}
