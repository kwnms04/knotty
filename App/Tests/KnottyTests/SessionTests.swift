import Darwin
import Foundation
import Testing

import KnottySession

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
        // A cell crosses whole, not just its text: the golden has this one
        // white on black with nothing else set.
        let first = snapshot.cells[0]
        #expect((first.foreground.r, first.foreground.g, first.foreground.b) == (255, 255, 255))
        #expect((first.background.r, first.background.g, first.background.b) == (0, 0, 0))
        #expect(first.attributes == 0)
        #expect(snapshot.rowStates.count == Int(rows))
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
/// allocator. It has to be a loose bound: the tests run alongside each other
/// in one process, so what the others are holding lands in the same
/// measurement — a few megabytes of it. What keeps a bound this loose from
/// being a bound that never fails is the count. A leaked session or a leaked
/// frame is tens of kilobytes, so a thousand of either is tens of megabytes,
/// which the neighbours cannot be mistaken for.
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
    for _ in 0..<1000 {
        try round()
    }
    let grown = Int(mstats().bytes_used) - before

    #expect(grown < 8 << 20, "the heap grew by \(grown) bytes over 1000 sessions")
}

/// A real child, and a way to be told when it has done something.
///
/// `/bin/echo` rather than a shell: what is under test is the session, and a
/// child that prints one line and stops is the smallest one that makes it
/// publish. The shell itself is looked at by eye, as M2 said it would be.
private func spawnEcho() throws -> (session: Session, woken: DispatchSemaphore) {
    let woken = DispatchSemaphore(value: 0)
    let session = try Session(
        command: ["/bin/echo", "knotty"], cols: cols, rows: rows, scrollback: scrollback
    )
    // Registering settles what already fell due, so a child quick enough to
    // have finished by this line still wakes us.
    try session.onWake { woken.signal() }
    return (session, woken)
}

/// Take frames as the session wakes us, until one answers `read` with
/// something or the wait runs out.
///
/// A child is a live thing: what it has printed by the first wake is not
/// promised, only that what it prints arrives on some wake. Waiting on the
/// wake rather than on the clock is what the app does too, and the deadline is
/// long because what it is there for is a machine that stopped, not one that
/// is busy.
private func settle<Value>(
    _ session: Session,
    wokenBy woken: DispatchSemaphore,
    reading read: (Snapshot) -> Value?
) throws -> Value? {
    let deadline = Date().addingTimeInterval(10)
    while Date() < deadline {
        guard woken.wait(timeout: .now() + 1) == .success else { continue }
        if let taken = try session.withSnapshot(read), let value = taken {
            return value
        }
    }
    // The clock ran out. A bare `nil` says only that nothing answered, and a
    // child that was slow and one whose output was lost look the same from
    // there — so what the screen held at the deadline goes in the record.
    let held = try session.withSnapshot { snapshot in
        "child \(String(describing: snapshot.childState)), "
            + "top row \(text(of: snapshot, row: 0).trimmingCharacters(in: .whitespaces).debugDescription)"
    }
    Issue.record("settle ran out of patience: \(held ?? "no frame was ever published")")
    return nil
}

/// The whole of the beat the app runs on, one storey down from the window:
/// a real child prints, the session wakes, and the screen the wake was about
/// says what the child printed.
@Test func aChildsOutputWakesTheSessionAndLandsOnTheScreen() throws {
    let (session, woken) = try spawnEcho()

    let top = try settle(session, wokenBy: woken) { snapshot -> String? in
        let line = text(of: snapshot, row: 0).trimmingCharacters(in: .whitespaces)
        return line.isEmpty ? nil : line
    }

    #expect(top == "knotty", "the top row came back as \(top ?? "no frame at all")")
}

/// The child's end reaches the app as part of a frame, which is the reading
/// that cannot be dropped.
@Test func theFrameSaysTheChildIsGone() throws {
    let (session, woken) = try spawnEcho()

    let state = try settle(session, wokenBy: woken) { snapshot -> ChildState? in
        snapshot.childState == .running ? nil : snapshot.childState
    }

    #expect(
        state == ChildState.exited(code: 0),
        "the child came back as \(state.map(String.init(describing:)) ?? "still running")"
    )
}

/// The shell comes from the user record. An app the window server started
/// has no environment worth reading, and `chsh` writes to the record.
@Test func theLoginShellIsAnExecutableTheUserRecordNames() {
    #expect(LoginShell.path.hasPrefix("/"))
    #expect(FileManager.default.isExecutableFile(atPath: LoginShell.path))
    // The conventional `-` before `argv[0]` is not sayable across this
    // boundary, so the argument does that work instead.
    #expect(LoginShell.command == [LoginShell.path, "-l"])
}
