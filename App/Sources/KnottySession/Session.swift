import CKnotty

/// What the boundary answered when it refused.
public struct SessionError: Error, CustomStringConvertible {
    /// The call that refused.
    public let call: StaticString
    /// The `KtStatus` it refused with, as the header numbers them.
    public let status: Int32

    public var description: String { "\(call) returned status \(status)" }

    /// What a key naming no physical key is refused with, which is a mapping
    /// to fill in rather than a key that has no bytes.
    public static let unidentifiedKey = Int32(KT_STATUS_UNIDENTIFIED_KEY.rawValue)
}

/// A session, and the only way to reach the handle behind one.
///
/// The boundary is written for calls that are serialized per session. This is
/// half of what makes that structure rather than discipline: the handle is
/// private and every call on it is a method here. The other half is the app
/// holding one of these in one place — the `SessionHost` of the ownership
/// tree, which arrives with the window that needs it. cf. 05-swift-app 4.
///
/// One of these has a child behind a pseudoterminal and a thread of its own
/// reading it; the other has neither, and takes its bytes from ``feed(_:)`` on
/// the calling thread. The second is what lets a test drive the whole path
/// without a shell, a window or a GPU, and everything past the parser is the
/// same code either way. cf. adr/0008.
public final class Session {
    private let handle: OpaquePointer
    /// What was last registered with ``onWake(_:)``, held here because the
    /// core keeps a pointer to it rather than a reference.
    private var wake: Wake?

    /// Create a session with no PTY behind it.
    public init(cols: UInt16, rows: UInt16, scrollback: Int) throws {
        var handle: OpaquePointer?
        let status = kt_session_new_detached(cols, rows, scrollback, &handle)
        guard status == KT_STATUS_OK.rawValue, let handle else {
            throw SessionError(call: "kt_session_new_detached", status: status)
        }
        self.handle = handle
    }

    /// Create a session with a child process behind a pseudoterminal.
    ///
    /// `command` is what to run: the first element is the program and the
    /// rest are its arguments. The child starts knowing the size given here,
    /// so its first frame is already the right shape, and it inherits this
    /// process's working directory — the boundary has no argument for one.
    ///
    /// The session gets a thread of its own that reads the terminal and
    /// publishes, which is why ``feed(_:)`` is refused on one of these.
    public init(command: [String], cols: UInt16, rows: UInt16, scrollback: Int) throws {
        var handle: OpaquePointer?
        let status = Self.withArgv(command) { argv in
            kt_session_new_pty(cols, rows, scrollback, argv.baseAddress, argv.count, &handle)
        }
        guard status == KT_STATUS_OK.rawValue, let handle else {
            throw SessionError(call: "kt_session_new_pty", status: status)
        }
        self.handle = handle
    }

    /// Releasing the session is the one place it happens, and it is the
    /// session that stops the thread and collects the child of one that has
    /// them.
    deinit {
        kt_session_free(handle)
    }

    /// Register what the session calls when it has something new to be taken,
    /// replacing whatever was registered before.
    ///
    /// `body` runs on the thread that drove the session — the I/O thread, for
    /// one with a PTY behind it — from inside the call that published. **It
    /// may do nothing but wake a thread of its own**: calling back into this
    /// session from there re-enters one the running call still holds.
    ///
    /// Wakes coalesce, so on each one take the snapshot and drain the queues
    /// until they are empty. What fell due before this call is paid before it
    /// returns, so registering late is told there is something to take.
    public func onWake(_ body: @escaping @Sendable () -> Void) throws {
        // Held on the stack for the whole call: registering pays what already
        // fell due, so the callback can fire before this returns.
        let wake = Wake(body)
        try check(
            "kt_session_set_wake",
            kt_session_set_wake(
                handle,
                { userdata in
                    guard let userdata else { return }
                    Unmanaged<Wake>.fromOpaque(userdata).takeUnretainedValue().body()
                },
                Unmanaged.passUnretained(wake).toOpaque()
            )
        )
        self.wake = wake
    }

    /// Feed bytes to the session.
    ///
    /// The whole buffer is processed on this thread before the call returns,
    /// and at most one snapshot comes out of it.
    ///
    /// A writer queue too full to hold what the terminal answered comes back
    /// as a refusal, the way it does for the Rust harness. The frame is
    /// published either way: what the child missed hearing does not make the
    /// screen wrong, and the next take still has it.
    public func feed(_ bytes: [UInt8]) throws {
        try bytes.withUnsafeBufferPointer { bytes in
            try check("kt_session_feed", kt_session_feed(handle, bytes.baseAddress, bytes.count))
        }
    }

    /// Queue bytes for the session's child.
    ///
    /// What is already text and has nothing left to decide: an input method's
    /// finished composition is the first of them. A composed syllable belongs
    /// to no place on the keyboard, so there is no key for the core to encode
    /// it from — the bytes are the whole of it. cf. 05-swift-app 7.
    ///
    /// Refused when the writer queue could not hold them, and in that case
    /// none of them were queued: a prefix of what the user typed reaching the
    /// child is worse than none of it.
    public func write(_ bytes: [UInt8]) throws {
        try bytes.withUnsafeBufferPointer { bytes in
            try check("kt_session_write", kt_session_write(handle, bytes.baseAddress, bytes.count))
        }
    }

    /// Sanitize `bytes`, wrap them the way the session's modes ask, and queue
    /// them for the child.
    ///
    /// The whole of what makes a clipboard safe to put in the input stream,
    /// and all of it the engine's: the control bytes that would be read as
    /// commands become spaces, and what is left is wrapped in the bracketed
    /// paste sequences when the child asked for them or has its newlines
    /// turned into carriage returns when it did not. cf. adr/0007.
    ///
    /// **Nothing on this side can skip that.** ``pasteIsSafe(_:)`` decides
    /// whether to warn first; a user who reads the warning and goes ahead
    /// arrives here all the same. ``write(_:)`` is not the way round it — it
    /// is for text the app already owns, an input method's finished
    /// composition — and putting a clipboard through it would be a decision
    /// somebody made, not an accident this API allows.
    public func paste(_ bytes: [UInt8]) throws {
        try bytes.withUnsafeBufferPointer { bytes in
            try check("kt_session_paste", kt_session_paste(handle, bytes.baseAddress, bytes.count))
        }
    }

    /// Whether `bytes` can be pasted without asking the user first.
    ///
    /// Unsafe means a newline, which a shell runs the moment it arrives, or
    /// the bracketed paste terminator, which would end the wrapping early and
    /// leave the rest being read as commands. The engine's judgement, and a
    /// conservative one — it looks at no terminal, which is what lets the
    /// question be asked before anything is pasted.
    ///
    /// A static function because it needs no session, and the warning has to
    /// come before one is used. It gates the warning and never the sanitizing.
    public static func pasteIsSafe(_ bytes: [UInt8]) -> Bool {
        bytes.withUnsafeBufferPointer { bytes in
            kt_paste_is_safe(bytes.baseAddress, bytes.count)
        }
    }

    /// Encode a key and queue what it comes to for the child.
    ///
    /// Which bytes that is belongs to the core, for the reason ``KeyEvent``
    /// gives.
    ///
    /// A key that comes to nothing queues nothing and is not a failure — a
    /// bare modifier, a release, and every key at all while an input method is
    /// composing. A key that names nothing is refused with
    /// ``SessionError/unidentifiedKey`` instead, so a hole in a mapping is
    /// heard about where it happens.
    public func key(_ event: KeyEvent) throws {
        try event.withRaw { event in
            try withUnsafePointer(to: event) { event in
                try check("kt_session_key", kt_session_key(handle, event))
            }
        }
    }

    /// Hand the session a mouse event over the cell at `x`, `y`.
    ///
    /// Cells rather than pixels: turning one into the other wants the
    /// metrics, and those belong on this side. Everything after that is the
    /// core's — whether the child hears about the event at all, and in which
    /// of the reporting formats. **A click at a shell prompt reaches nobody**,
    /// and that is the mode working rather than a failure. cf. adr/0017.
    /// `button` is nil for a motion with nothing held, which is the one
    /// event a button would be wrong for.
    public func mouse(
        _ action: MouseAction,
        button: MouseButton?,
        mods: Modifiers = [],
        x: UInt16,
        y: UInt16
    ) throws {
        try check(
            "kt_session_mouse",
            kt_session_mouse(handle, action.raw, MouseButton.raw(button), mods.rawValue, x, y)
        )
    }

    /// Turn the wheel over the cell at `x`, `y`.
    ///
    /// **Both deltas are in lines**, and up and right are positive — see
    /// ``WheelLines`` for what turns a trackpad's pixels into them, and call
    /// only when that count changed.
    ///
    /// What the child hears is one of three things and the terminal is what
    /// says which: a mouse code, the cursor keys, or nothing at all because
    /// the viewport moved instead. The last of those publishes a frame, so
    /// the scroll position stays the core's rather than being kept here too.
    public func wheel(
        deltaX: Int32,
        deltaY: Int32,
        x: UInt16,
        y: UInt16,
        mods: Modifiers = []
    ) throws {
        try check(
            "kt_session_wheel",
            kt_session_wheel(handle, deltaX, deltaY, x, y, mods.rawValue)
        )
    }

    /// Tell the session the window gained or lost focus.
    ///
    /// Nothing reaches the child unless it asked to hear, which is the usual
    /// case. vim's `autoread` is what asks: a file changed by something else
    /// is re-read when the window comes back.
    public func focus(gained: Bool) throws {
        try check("kt_session_focus", kt_session_focus(handle, gained))
    }

    /// Resize the grid, and say how big one cell now is in pixels.
    ///
    /// The primary screen reflows: a line longer than the new width folds
    /// rather than losing its tail, and widening unfolds it again. The pixel
    /// size is a cell's, and a session with a PTY behind it tells its child
    /// both — which is the `SIGWINCH` that makes an editor redraw.
    ///
    /// **Only when one of them has changed**, for the reason
    /// `kt_session_resize` gives: a window being dragged must not reach the
    /// reflow on every pixel.
    public func resize(cols: UInt16, rows: UInt16, cellWidth: UInt32, cellHeight: UInt32) throws {
        try check(
            "kt_session_resize",
            kt_session_resize(handle, cols, rows, cellWidth, cellHeight)
        )
    }

    /// Select a range of the viewport, or clear the selection with nil.
    ///
    /// Both ends are inclusive and either may come first: the pair records
    /// which way the selection was made, not which end is topmost. What comes
    /// back on the snapshot is per row — the flag and the columns — because a
    /// selection carried inside a cell would empty the renderer's line cache
    /// on every mouse move. cf. 02-ffi, 04-renderer R2.
    public func setSelection(_ range: SelectionRange?) throws {
        guard var range else {
            return try check("kt_session_set_selection", kt_session_set_selection(handle, nil))
        }
        try check(
            "kt_session_set_selection",
            withUnsafePointer(to: &range) { kt_session_set_selection(handle, $0) }
        )
    }

    /// Select from the cell a gesture began on out to the cell it is over
    /// now, measured in `unit`.
    ///
    /// **Both ends together.** A word or a line is widened from each end, so
    /// a call naming only the cell under the pointer has nothing to widen
    /// from and the selection collapses the moment the pointer crosses a
    /// space. The anchor, the click count `unit` comes from and whether a
    /// drag is under way are the view's three pieces of gesture state; where
    /// the boundaries fall is the engine's, and nothing on this side counts
    /// one. cf. 05-swift-app 4, adr/0017.
    ///
    /// The pair also records which way the drag went, so dragging back past
    /// the anchor reverses the selection. `rectangle` makes the two ends
    /// opposite corners of a block, which is what ⌥ asks for.
    public func select(
        anchor: (x: UInt16, y: UInt16),
        to cell: (x: UInt16, y: UInt16),
        unit: SelectionUnit,
        rectangle: Bool = false
    ) throws {
        try check(
            "kt_session_select",
            kt_session_select(handle, anchor.x, anchor.y, cell.x, cell.y, unit.raw, rectangle)
        )
    }

    /// The selection as plain text, or nil when nothing is selected.
    ///
    /// Folded lines come back as the one line they were typed as, which is
    /// what makes a copied paragraph paste as the paragraph. Plain text and
    /// nothing else: v1's clipboard carries `text/plain`.
    ///
    /// Copied out rather than lent on: the run the boundary answers with is
    /// the session's until the next copy, and what this is for is a
    /// pasteboard that outlives the call.
    public func copySelection() throws -> String? {
        var text = KtBytes()
        let status = kt_session_copy_selection(handle, &text)
        if status == KT_STATUS_NO_VALUE.rawValue {
            return nil
        }
        try check("kt_session_copy_selection", status)
        guard let bytes = text.bytes else { return "" }
        return String(decoding: UnsafeBufferPointer(start: bytes, count: text.len), as: UTF8.self)
    }

    /// Move the viewport `lines` lines into the scrollback, up positive.
    ///
    /// What a selection drag out of the window asks for. The core moves the
    /// viewport and publishes, so no scroll position is kept on this side.
    public func scrollViewport(lines: Int32) throws {
        try check("kt_session_scroll_viewport", kt_session_scroll_viewport(handle, lines))
    }

    /// Take the bytes a detached session has queued for its child, emptying
    /// the queue.
    ///
    /// What the terminal answered and what was typed into it, in the order
    /// they were queued. A session with a PTY behind it has its own reader
    /// draining this, and refuses the call.
    public func takeWrites() throws -> [UInt8] {
        var bytes = KtBytes()
        try check("kt_session_take_writes", kt_session_take_writes(handle, &bytes))
        guard let queued = bytes.bytes else { return [] }
        return [UInt8](UnsafeBufferPointer(start: queued, count: bytes.len))
    }

    /// Take the latest frame and lend a view of it to `body`, or answer nil
    /// when nothing has been published since the last take.
    ///
    /// The view is borrowed, not copied. The core has already made the copy,
    /// and making a second one is what would put the grid back into the cost
    /// of taking a frame — the very thing that lets the main thread do this
    /// work. cf. adr/0005.
    ///
    /// The scope is the lifetime: the pointers in the view are good until
    /// `body` returns, and the release on the way out is the only release
    /// there is.
    public func withSnapshot<Value>(_ body: (Snapshot) throws -> Value) throws -> Value? {
        var snapshot: OpaquePointer?
        let status = kt_session_take_snapshot(handle, &snapshot)
        if status == KT_STATUS_NO_VALUE.rawValue {
            return nil
        }
        try check("kt_session_take_snapshot", status)
        // A status of OK is the boundary promising an owned handle. Nothing
        // good comes of reading a frame that is not there.
        guard let snapshot else {
            preconditionFailure("kt_session_take_snapshot succeeded with no snapshot")
        }
        defer { kt_snapshot_free(snapshot) }

        var view = KtSnapshotView()
        try check("kt_snapshot_view", kt_snapshot_view(snapshot, &view))
        return try body(Snapshot(view))
    }

    /// Empty the event queue, answering how many events came out of it and
    /// how many had been dropped for want of room since the last time.
    ///
    /// One call takes the whole queue, so this is the drain-until-empty the
    /// boundary asks for on every wake. What to do with an event is a policy
    /// M4 writes; what M2 owes is an emptied queue, and events are dropped
    /// here rather than kept.
    @discardableResult
    public func drainEvents() throws -> (taken: Int, dropped: UInt64) {
        var events = KtEvents()
        try check("kt_session_take_events", kt_session_take_events(handle, &events))
        return (events.len, events.dropped)
    }

    /// Lend `command` to `body` as the run of texts the boundary spawns from.
    ///
    /// One buffer holds all of it and the texts point into that, so every
    /// pointer stays good for the whole call — which is the only span the
    /// boundary borrows them for.
    private static func withArgv<Value>(
        _ command: [String],
        _ body: (UnsafeBufferPointer<KtText>) -> Value
    ) -> Value {
        let arguments = command.map { Array($0.utf8) }
        let lengths = arguments.map(\.count)
        return arguments.flatMap { $0 }.withUnsafeBufferPointer { bytes in
            var offset = 0
            let texts = lengths.map { length -> KtText in
                defer { offset += length }
                // Null only where the length is 0, which is what the boundary
                // allows and what an empty argument comes out as.
                return KtText(bytes: bytes.baseAddress.map { $0 + offset }, len: length)
            }
            return texts.withUnsafeBufferPointer(body)
        }
    }

    /// The header's status type is ambiguous in Swift — the C enum and the
    /// typedef beside it arrive under one name — so what a call answers is
    /// held as the integer it is and compared against the constants.
    private func check(_ call: StaticString, _ status: Int32) throws {
        guard status == KT_STATUS_OK.rawValue else {
            throw SessionError(call: call, status: status)
        }
    }
}

/// A closure the core can be handed a pointer to.
///
/// The boundary takes an untyped pointer and hands it back untouched, so what
/// crosses is this box; the session holds it for as long as the core has the
/// pointer, and lets go only after the free that stops the thread.
final class Wake {
    let body: @Sendable () -> Void

    init(_ body: @escaping @Sendable () -> Void) {
        self.body = body
    }
}
