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

/// The other way into the writer queue: bytes that are already the bytes,
/// which is what an input method's finished composition is. Nothing encodes
/// them, so what comes out is what went in. cf. 05-swift-app 7.
@Test func writtenTextReachesTheChildAsTheBytesItAlreadyIs() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)

    try session.write(Array("한".utf8))

    #expect(try session.takeWrites() == Array("한".utf8))
}

/// That the app reaches the call at all, and reaches the right one: an
/// emptied screen is what says which symbol was bound. What emptying means —
/// the history, the alternate screen, the cursor — is `abi.rs`'s.
@Test func clearingASessionEmptiesTheScreenItCameBackWith() throws {
    let session = try replaySynthetic()
    _ = try session.withSnapshot { _ in true }
    // What the recording's queries were answered with, so that whatever is in
    // the queue afterwards is the clear's doing.
    _ = try session.takeWrites()

    try session.clear()

    let emptied = try session.withSnapshot { snapshot in
        (0..<Int(snapshot.rows)).allSatisfy { text(of: snapshot, row: $0).allSatisfy { $0 == " " } }
            && snapshot.cursor.x == 0 && snapshot.cursor.y == 0
    }

    #expect(emptied == true)
    #expect(try session.takeWrites().isEmpty)
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
private func spawn(
    _ command: [String], in directory: String? = nil
) throws -> (session: Session, woken: DispatchSemaphore) {
    let woken = DispatchSemaphore(value: 0)
    let session = try Session(
        command: command, cols: cols, rows: rows, scrollback: scrollback, directory: directory
    )
    // Registering settles what already fell due, so a child quick enough to
    // have finished by this line still wakes us.
    try session.onWake { woken.signal() }
    return (session, woken)
}

/// The child every test that only needs one takes.
///
/// `/bin/echo` rather than a shell: what is under test is the session, and a
/// child that prints one line and stops is the smallest one that makes it
/// publish. The shell itself is looked at by eye, as M2 said it would be.
private func spawnEcho() throws -> (session: Session, woken: DispatchSemaphore) {
    try spawn(["/bin/echo", "knotty"])
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

/// What a window is called comes from the child, and falls back to the app's
/// own name only where the child named nothing.
///
/// Both halves through a real PTY, because both are what a window meets: a
/// child that sets no title is every login shell out of the box, and the
/// programs that do set one, tmux above all, do it with `OSC 2`. Read through
/// the property the window is named from, so that what is checked is the path
/// the app takes. cf. 05-swift-app 3, 06-integration.
@Test func aWindowIsNamedByItsChildAndOtherwiseByTheApp() throws {
    let (quiet, quietWoken) = try spawnEcho()
    let untold = try settle(quiet, wokenBy: quietWoken) { snapshot -> String? in
        // Read off the frame the output landed in, so that "named nothing" is
        // a frame that came rather than a frame that never did.
        text(of: snapshot, row: 0).trimmingCharacters(in: .whitespaces).isEmpty
            ? nil : snapshot.windowTitle
    }
    #expect(untold == ProcessInfo.processInfo.processName)

    let (naming, namingWoken) = try spawn(["/bin/echo", "-n", "\u{1b}]2;made\u{7}"])
    let named = try settle(naming, wokenBy: namingWoken) { snapshot -> String? in
        snapshot.title.isEmpty ? nil : snapshot.windowTitle
    }

    #expect(named == "made")
}

/// What this binding does that no test below it can: hand a path across as a
/// pointer and a length, and read the field a restored window would be opened
/// from back off a frame. The child reports no directory of its own — `/bin/sh`
/// sends no OSC 7 — so the path that comes back is the one read off the
/// process. cf. adr/0020.
@Test func aSessionStartsWhereItWasToldAndSaysWhereThatIs() throws {
    // A directory that is its own real path, so that what comes back can be
    // compared with what was asked for. A temporary one would not do: `/tmp`
    // is a symlink, and the system answers with what it points at.
    let directory = "/usr/lib"
    // A child that stays, so the frame under test is not one from a session
    // already on its way out.
    let (session, woken) = try spawn(
        ["/bin/sh", "-c", "printf ready; read line"], in: directory
    )

    let reported = try settle(session, wokenBy: woken) { snapshot -> String? in
        let pwd = String(decoding: snapshot.pwd, as: UTF8.self)
        return pwd.isEmpty ? nil : pwd
    }

    #expect(reported == directory)
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

/// Keep asking until the answer comes back true, or give up.
///
/// A poll rather than a wait on a wake, because what is being waited for
/// leaves no mark: a shell handing its terminal to a job writes nothing, so
/// no frame is published and no wake is paid. That is the whole reason
/// ``Session/foregroundBusy()`` is asked rather than read off a frame — and
/// asking in a loop is what a test does about it. The app never does this: it
/// asks once, when a window is being closed.
private func waitUntilBusy(_ session: Session) throws -> Bool {
    let deadline = Date().addingTimeInterval(10)
    while Date() < deadline {
        if try session.foregroundBusy() { return true }
        Thread.sleep(forTimeInterval: 0.02)
    }
    return false
}

/// What a window asks before closing, both ways round.
///
/// The same command twice, told apart by job control alone. Without it the
/// `sleep` shares the shell's process group and nothing is in front of
/// anything; with it — which is how a login shell runs — the `sleep` gets a
/// process group of its own and the terminal's foreground with it. So the
/// pair is exactly the distinction the warning is made of: a prompt with
/// nothing at it against a shell with a job in front of it.
/// cf. 05-swift-app 8.
@Test func aSessionSaysWhetherAProgramIsRunningInFrontOfTheShell() throws {
    let (quiet, quietWoken) = try spawn(["/bin/sh", "-c", "echo ready; sleep 300"])
    // Waiting for the line is what says the shell got as far as running the
    // `sleep`. Without it, "nothing is running" would also be the answer for
    // a shell that had not started yet, which is not the same fact.
    let printed = try settle(quiet, wokenBy: quietWoken) { snapshot -> Bool? in
        text(of: snapshot, row: 0).hasPrefix("ready") ? true : nil
    }
    #expect(printed == true)
    #expect(try quiet.foregroundBusy() == false)

    let (busy, _) = try spawn(["/bin/sh", "-c", "set -m; sleep 300"])
    #expect(
        try waitUntilBusy(busy),
        "nothing ever took the terminal in front of the shell"
    )
}

/// The third answer, which is neither of the two above: a window whose shell
/// has already gone has nothing running in it. Nothing is left that closing
/// the window would take down, so a warning would be one nobody could act on.
///
/// A terminal in that state may have no foreground group at all, which the
/// system reports as 0 rather than as a failure. Whether that number is read
/// as a process group is the core's own test to catch — the round after a
/// short-lived child's output is where it turns up. This one is the app's
/// side of the answer.
@Test func aSessionWhoseChildHasGoneSaysNothingIsRunning() throws {
    let (session, woken) = try spawn(["/bin/sh", "-c", "exit 0"])
    let gone = try settle(session, wokenBy: woken) { snapshot -> Bool? in
        snapshot.childState == .exited(code: 0) ? true : nil
    }
    #expect(gone == true)

    #expect(try session.foregroundBusy() == false)
}

/// What a child is told about the terminal it was started in.
///
/// All three, because tmux reads all three: terminfo is looked up under
/// `TERM`, and the `terminal-features` autodetection that decides what tmux
/// will send is keyed on `TERM_PROGRAM` and `COLORTERM`. Read back out of the
/// child's own environment rather than out of the spawn code, which is the
/// only reading that says the child really got them. cf. 06-integration.
@Test func aChildIsToldWhichTerminalItWasStartedIn() throws {
    let (session, woken) = try spawn([
        "/bin/sh", "-c", #"printf "%s|%s|%s\n" "$TERM" "$COLORTERM" "$TERM_PROGRAM""#,
    ])

    let told = try settle(session, wokenBy: woken) { snapshot -> String? in
        let line = text(of: snapshot, row: 0).trimmingCharacters(in: .whitespaces)
        return line.isEmpty ? nil : line
    }

    #expect(told == "xterm-256color|truecolor|knotty")
}
